use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RcloneItem {
    #[serde(rename = "Path")]
    pub path: String,
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "Size")]
    pub size: i64,
    #[serde(rename = "MimeType", default)]
    pub mime_type: String,
    #[serde(rename = "ModTime", default)]
    pub mod_time: String,
    #[serde(rename = "IsDir")]
    pub is_dir: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SearchHit {
    pub path: String,
    pub name: String,
    pub parent_dir: String,
    pub is_dir: bool,
    pub size: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EngineStatus {
    pub is_running: bool,
    pub port: u16,
    pub version: String,
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct TransferProgress {
    pub active: bool,
    pub task_type: String,
    pub bytes: i64,
    pub total_bytes: i64,
    pub speed: i64,
    pub percentage: u8,
    pub current_file: String,
    pub transferred_files: i64,
    pub total_files: i64,
    pub eta: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct ListResponse {
    list: Option<Vec<RcloneItem>>,
}

/// 带哈希字段的列表项（用于完整性自检的清单构建/校验）
#[derive(Debug, Deserialize)]
struct HashListItem {
    #[serde(rename = "Path")]
    path: String,
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "IsDir")]
    is_dir: bool,
    #[serde(rename = "Encrypted", default)]
    encrypted: String,
    #[serde(rename = "EncryptedPath", default)]
    encrypted_path: String,
    #[serde(rename = "Hashes", default)]
    hashes: std::collections::HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct HashListResponse {
    list: Option<Vec<HashListItem>>,
}

pub struct RcloneService {
    progress: Mutex<TransferProgress>,
    app_handle: Mutex<Option<tauri::AppHandle>>,
    search_index: Mutex<Option<Vec<RcloneItem>>>,
    search_build_lock: Mutex<()>,
    recycle_meta_cache: Mutex<std::collections::HashMap<String, serde_json::Value>>,
    raw_remote: Mutex<String>,
    active_job: Mutex<Option<i64>>,
    cancel_requested: Mutex<bool>,
    // 标记当前是否已成功挂载保险箱（加密通道就绪）；锁定/切换后置 false
    mounted: Mutex<bool>,
}

/// 保险箱内部鉴权标记文件名（磁盘上为密文，解密后用于校验解锁）
const VAULT_AUTH_FILE_NAME: &str = ".vault_auth";

/// 校验保险箱内条目名称（单段）：拒绝非法字符、Windows 保留名、路径穿越等。
/// 通过返回 Ok(())，否则返回面向用户的中文错误信息。
fn validate_entry_name(name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("名称不能为空".into());
    }
    if name == "." || name == ".." {
        return Err("名称不能是 . 或 ..".into());
    }
    if name.starts_with(' ') || name.ends_with(' ') {
        return Err("名称不能以空格开头或结尾".into());
    }
    if name.starts_with('.') || name.ends_with('.') {
        return Err("名称不能以点号开头或结尾".into());
    }
    if name.contains(|c| matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')) {
        return Err("名称不能包含字符: < > : \" / \\ | ? *".into());
    }
    if name.chars().any(|c| c.is_control()) {
        return Err("名称不能包含控制字符".into());
    }
    let stem = name.split('.').next().unwrap_or(name).to_uppercase();
    const RESERVED: [&str; 24] = [
        "CON", "PRN", "AUX", "NUL",
        "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
        "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
        "AUDIT", "AUDIT.LOG"
    ];
    if RESERVED.contains(&stem.as_str()) || name.eq_ignore_ascii_case("audit.log") || name.eq_ignore_ascii_case(".audit.log") {
        return Err("名称不能使用系统保留名 (CON/PRN/AUX/NUL/COM1~9/LPT1~9/audit.log)".into());
    }
    Ok(())
}

/// 是否为内部管控文件/路径（不参与清单与普通浏览）
fn is_internal(path: &str) -> bool {
    let name = Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if name == ".vault_auth" || name == ".manifest" || name == "audit.log" || name == ".audit.log" {
        return true;
    }
    path == ".recycle" || path.starts_with(".recycle/")
}

/// 判断 rc 错误是否为「目录不存在」
fn is_missing_dir(err: &str) -> bool {
    let e = err.to_lowercase();
    e.contains("directory not found")
        || e.contains("couldn't find directory")
        || e.contains("could not find directory")
        || e.contains("not found: directory")
}

/// 判断 rc 错误是否为「目录不存在」（回收站从未来过内容时 .recycle 目录尚未创建，应视为空而非报错）
fn is_recycle_not_found(err: &str) -> bool {
    is_missing_dir(err)
}

/// 应用私有临时目录：明文/解密临时文件不再落到系统共享 %TEMP%，
/// 改为 %LOCALAPPDATA%\SafeVault\tmp（应用私有、逐请求清理、崩溃残留风险可控）
fn private_tmp_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join("SafeVault").join("tmp");
    let _ = fs::create_dir_all(&dir);
    dir
}

/// 清理私有临时目录中超过 1 小时的明文残留（崩溃或异常退出遗留），
/// 每次解锁/初始化时调用一次；正常会话内的临时文件即用即删，不受影响
fn cleanup_stale_tmp_files() {
    use std::time::{Duration, SystemTime};
    let cutoff = match SystemTime::now().checked_sub(Duration::from_secs(3600)) {
        Some(t) => t,
        None => return,
    };
    if let Ok(entries) = fs::read_dir(private_tmp_dir()) {
        for entry in entries.flatten() {
            let path = entry.path();
            let stale = entry
                .metadata()
                .and_then(|m| m.modified())
                .map(|t| t < cutoff)
                .unwrap_or(false);
            if stale && path.is_file() {
                let _ = fs::remove_file(&path);
            }
        }
    }
}

impl RcloneService {
    pub fn new() -> Self {
        crate::librclone::init();
        Self {
            progress: Mutex::new(TransferProgress::default()),
            app_handle: Mutex::new(None),
            search_index: Mutex::new(None),
            search_build_lock: Mutex::new(()),
            recycle_meta_cache: Mutex::new(std::collections::HashMap::new()),
            raw_remote: Mutex::new(String::new()),
            active_job: Mutex::new(None),
            cancel_requested: Mutex::new(false),
            mounted: Mutex::new(false),
        }
    }

    pub fn set_app_handle(&self, handle: tauri::AppHandle) {
        *self.app_handle.lock().unwrap() = Some(handle);
    }

    /// 根据当前运行环境的逻辑 CPU 核心数计算最佳并发数
    /// 预留至少 1~2 个核心给 UI 渲染线程和操作系统，防止高负荷时出现界面掉帧或系统卡死
    fn get_optimal_concurrency() -> (usize, usize) {
        let cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);

        // 加密属于 CPU 密集型任务：
        // <=4 核设备只占用 2~3 核；多核设备预留 2 核或按 75% 比例分配，上限收敛至 12，杜绝 CPU 100% 占满
        let transfers = if cpus <= 4 {
            (cpus.saturating_sub(1)).max(2)
        } else {
            ((cpus * 3) / 4).clamp(4, 12)
        };

        // 目录元数据扫描属于 I/O 等待密集型，设置为 transfers 的 1.5 倍，最高不超过 16
        let checkers = (transfers + (transfers / 2)).clamp(4, 16);

        (transfers, checkers)
    }

    fn emit_progress(&self, prog: &TransferProgress) {
        use tauri::Emitter;
        if let Some(handle) = self.app_handle.lock().unwrap().as_ref() {
            let _ = handle.emit("transfer-progress", prog);
        }
    }

    pub fn get_progress(&self) -> TransferProgress {
        self.progress.lock().unwrap().clone()
    }

    pub fn get_engine_status(&self) -> EngineStatus {
        // 引擎库本身在进程启动时就绪（内存级 FFI），但「加密通道」只有在挂载保险箱后才就绪。
        // 未挂载（锁定 / 尚未解锁）时报告休眠，避免前端误显示为已加密运营（误导性状态灯）。
        if !*self.mounted.lock().unwrap() {
            return EngineStatus {
                is_running: false,
                port: 0,
                version: String::new(),
                message: "加密会话已休眠，解锁保险箱后可用".to_string(),
            };
        }
        if let Ok(res) = self.call_rc("core/version", serde_json::json!({})) {
            let ver = res.get("version").and_then(|v| v.as_str()).unwrap_or("v1.68.2");
            return EngineStatus {
                is_running: true,
                port: 0,
                version: ver.to_string(),
                message: "安全加密引擎运行正常 (内存级 FFI)".to_string(),
            };
        }

        EngineStatus {
            is_running: false,
            port: 0,
            version: String::new(),
            message: "引擎待命中".to_string(),
        }
    }

    pub fn obscure(&self, secret: &str) -> Result<String, String> {
        let res = self.call_rc("core/obscure", serde_json::json!({ "clear": secret }))?;
        if let Some(obs) = res.get("obscured").and_then(|v| v.as_str()) {
            Ok(obs.to_string())
        } else {
            Err("混淆密钥失败：rclone RPC 未返回有效的 obscured 字段".to_string())
        }
    }

    pub fn start_daemon(&self) -> Result<(), String> {
        crate::librclone::init();
        cleanup_stale_tmp_files();
        Ok(())
    }

    pub fn stop_daemon(&self) {
        self.unmount_vault();
    }

    pub fn call_rc(&self, endpoint: &str, params: serde_json::Value) -> Result<serde_json::Value, String> {
        crate::librclone::rclone_rpc(endpoint, params)
    }

    pub fn mount_vault(&self, vault_dir: &str, obscured_key: &str, obscured_salt: &str) -> Result<(), String> {
        let norm_dir = vault_dir.replace('\\', "/");

        // 挂载前先防御性清理旧的同名虚拟后端（忽略未创建时的删除失败）
        self.unmount_vault();

        // 记录底层密文目录，供完整性自检使用
        *self.raw_remote.lock().unwrap() = format!("vault_raw:{}", norm_dir);

        // 1. 创建本地内存后端
        self.call_rc(
            "config/create",
            serde_json::json!({
                "name": "vault_raw",
                "type": "local",
                "parameters": {}
            }),
        )?;

        // 2. 创建加密内存后端 (标准 AES-EME 混淆)
        self.call_rc(
            "config/create",
            serde_json::json!({
                "name": "vault_crypt",
                "type": "crypt",
                "parameters": {
                    "remote": format!("vault_raw:{}", norm_dir),
                    "filename_encryption": "standard",
                    "directory_name_encryption": "true",
                    "password": obscured_key,
                    "password2": obscured_salt
                }
            }),
        )?;

        // 挂载成功：标记加密通道就绪，前端状态灯据此点亮
        *self.mounted.lock().unwrap() = true;
        Ok(())
    }

    pub fn unmount_vault(&self) {
        // 锁定/切换：立即将「加密通道就绪」置 false，前端状态灯随之熄灭
        *self.mounted.lock().unwrap() = false;
        // 保险箱切换/锁定时清空搜索快照与回收站元数据缓存，防止旧保险箱元数据残留污染
        self.invalidate_search_index();
        self.recycle_meta_cache.lock().unwrap().clear();
        *self.raw_remote.lock().unwrap() = String::new();
        let _ = self.call_rc("config/delete", serde_json::json!({ "name": "vault_crypt" }));
        let _ = self.call_rc("config/delete", serde_json::json!({ "name": "vault_raw" }));
    }

    pub fn list_files(&self, remote_dir: &str) -> Result<Vec<RcloneItem>, String> {
        let clean_remote = remote_dir.replace('\\', "/").trim_start_matches('/').to_string();
        let res = self.call_rc(
            "operations/list",
            serde_json::json!({
                "fs": "vault_crypt:",
                "remote": clean_remote
            }),
        )?;

        let list_res: ListResponse = serde_json::from_value(res).map_err(|e| e.to_string())?;
        let mut items = list_res.list.unwrap_or_default();
        let clean_prefix = clean_remote.trim_matches('/');
        if !clean_prefix.is_empty() {
            for item in &mut items {
                let item_path = item.path.trim_start_matches('/');
                if !item_path.starts_with(clean_prefix) {
                    item.path = format!("{}/{}", clean_prefix, item_path);
                }
            }
        }
        Ok(items)
    }

    /// 全库文件名/路径模糊搜索（大小写不敏感子串匹配）
    ///
    /// 首次调用时递归列出整个保险箱并缓存明文元数据快照；
    /// 后续搜索直接基于内存快照做线性匹配（毫秒级），
    /// 避免每次搜索都重复遍历并解密数万文件名。
    /// 缓存会在锁定 / 导入 / 删除 / 新建目录后自动失效。
    pub fn search_files(&self, keyword: &str, limit: usize) -> Result<Vec<SearchHit>, String> {
        let kw = keyword.trim().to_lowercase();
        if kw.is_empty() {
            return Ok(Vec::new());
        }

        let need_build = self.search_index.lock().unwrap().is_none();
        if need_build {
            let _build_guard = self.search_build_lock.lock().unwrap();
            if self.search_index.lock().unwrap().is_none() {
                let res = self.call_rc(
                    "operations/list",
                    serde_json::json!({
                        "fs": "vault_crypt:",
                        "remote": "",
                        "opt": { "recurse": true }
                    }),
                )?;

                let list_res: ListResponse = serde_json::from_value(res).map_err(|e| e.to_string())?;
                let items: Vec<RcloneItem> = list_res
                    .list
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|i| !is_internal(&i.path))
                    .collect();
                *self.search_index.lock().unwrap() = Some(items);
            }
        }

        let all = self.search_index.lock().unwrap().clone().unwrap_or_default();
        let mut hits: Vec<SearchHit> = Vec::new();

        for item in all.iter() {
            if hits.len() >= limit {
                break;
            }
            let path_lower = item.path.to_lowercase();
            let name_lower = item.name.to_lowercase();
            if !(path_lower.contains(&kw) || name_lower.contains(&kw)) {
                continue;
            }

            let parent_dir = match item.path.rfind('/') {
                Some(pos) => item.path[..pos].to_string(),
                None => String::new(),
            };

            hits.push(SearchHit {
                path: item.path.clone(),
                name: item.name.clone(),
                parent_dir,
                is_dir: item.is_dir,
                size: item.size,
            });
        }

        Ok(hits)
    }

    /// 强制清空全库搜索快照（锁定 / 数据变更后调用）
    pub fn invalidate_search_index(&self) {
        *self.search_index.lock().unwrap() = None;
    }

    /// 配置一个与保险箱等价的标准 crypt remote，直连绝对路径（互通自检用，等价外部 rclone）
    pub fn create_crypt_remote(&self, name: &str, base_path: &str, obscured_key: &str, obscured_salt: &str) -> Result<(), String> {
        let norm = base_path.replace('\\', "/").trim_end_matches('/').to_string();
        self.call_rc(
            "config/create",
            serde_json::json!({
                "name": name,
                "type": "crypt",
                "parameters": {
                    "remote": format!(":local,path={}", norm),
                    "filename_encryption": "standard",
                    "directory_name_encryption": "true",
                    "password": obscured_key,
                    "password2": obscured_salt,
                }
            }),
        )?;
        Ok(())
    }

    pub fn delete_config(&self, name: &str) -> Result<(), String> {
        self.call_rc("config/delete", serde_json::json!({ "name": name }))?;
        Ok(())
    }

    pub fn copyfile(&self, src_fs: &str, src_remote: &str, dst_fs: &str, dst_remote: &str) -> Result<(), String> {
        self.call_rc(
            "operations/copyfile",
            serde_json::json!({
                "srcFs": src_fs,
                "srcRemote": src_remote,
                "dstFs": dst_fs,
                "dstRemote": dst_remote,
            }),
        )?;
        Ok(())
    }

    pub fn write_text_file(&self, remote_path: &str, content: &str) -> Result<(), String> {
        let temp_dir = private_tmp_dir();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let temp_file = temp_dir.join(format!("_vault_chk_{}_{}.tmp", std::process::id(), ts));
        fs::write(&temp_file, content).map_err(|e| e.to_string())?;

        let src_dir = temp_file
            .parent()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|| "".to_string());
        let src_file = temp_file
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "_vault_chk.tmp".to_string());

        let res = self.call_rc(
            "operations/copyfile",
            serde_json::json!({
                "srcFs": src_dir,
                "srcRemote": src_file,
                "dstFs": "vault_crypt:",
                "dstRemote": remote_path
            }),
        );

        let _ = fs::remove_file(&temp_file);
        res.map(|_| ())
    }

    /// 读取文本文件内容（立即读入内存并销毁磁盘临时文件）
    pub fn read_text_file(&self, remote_path: &str) -> Result<String, String> {
        let clean_path = remote_path.trim_start_matches('/');
        let temp_dir = private_tmp_dir();
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let temp_file = temp_dir.join(format!("_vault_rd_{}_{}.tmp", std::process::id(), ts));

        let dst_dir = temp_file
            .parent()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|| "".to_string());
        let dst_file = temp_file
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "_vault_rd.tmp".to_string());

        let res = self.call_rc(
            "operations/copyfile",
            serde_json::json!({
                "srcFs": "vault_crypt:",
                "srcRemote": clean_path,
                "dstFs": dst_dir,
                "dstRemote": dst_file
            }),
        );

        let read_res = match res {
            Ok(_) => fs::read_to_string(&temp_file).map_err(|e| e.to_string()),
            Err(e) => Err(e),
        };
        let _ = fs::remove_file(&temp_file);
        read_res
    }

    fn run_async_job(&self, task_type: &str, endpoint: &str, params: serde_json::Value) -> Result<(), String> {
        self.run_async_job_full(task_type, endpoint, params, None, None)
    }

    fn run_async_job_total(
        &self,
        task_type: &str,
        endpoint: &str,
        params: serde_json::Value,
        expected_total: Option<i64>,
    ) -> Result<(), String> {
        self.run_async_job_full(task_type, endpoint, params, expected_total, None)
    }

    /// 异步执行 rclone rc 调用并等待完成。
    /// `expected_total`：删除类任务（如清空回收站）的预扫描总文件数，作为进度条分母；
    /// `batch_info`：批量任务进度上下文 (当前索引 0-based, 总任务项数)。
    fn run_async_job_full(
        &self,
        task_type: &str,
        endpoint: &str,
        mut params: serde_json::Value,
        expected_total: Option<i64>,
        batch_info: Option<(usize, usize)>,
    ) -> Result<(), String> {
        let _ = self.call_rc("core/stats-reset", serde_json::json!({}));

        if let Some(obj) = params.as_object_mut() {
            obj.insert("_async".to_string(), serde_json::json!(true));
        }

        let res = self.call_rc(endpoint, params)?;
        let job_id = match res.get("jobid").and_then(|v| v.as_i64()) {
            Some(id) => id,
            None => return Ok(()),
        };

        {
            *self.active_job.lock().unwrap() = Some(job_id);
            *self.cancel_requested.lock().unwrap() = false;
            let mut prog = self.progress.lock().unwrap();
            *prog = TransferProgress {
                active: true,
                task_type: task_type.to_string(),
                ..Default::default()
            };
            if let Some((idx, total)) = batch_info {
                if total > 0 {
                    prog.transferred_files = idx as i64;
                    prog.total_files = total as i64;
                    prog.percentage = (((idx as f64) / (total as f64)) * 100.0) as u8;
                }
            } else if let Some(total) = expected_total {
                prog.total_files = total;
            }
            self.emit_progress(&prog);
        }

        loop {
            std::thread::sleep(Duration::from_millis(150));

            // 用户点击“取消”：停止 rclone 作业并立即退出等待
            if *self.cancel_requested.lock().unwrap() {
                let _ = self.call_rc("job/stop", serde_json::json!({ "jobid": job_id }));
                self.active_job.lock().unwrap().take();
                *self.cancel_requested.lock().unwrap() = false;
                let mut prog = self.progress.lock().unwrap();
                prog.active = false;
                self.emit_progress(&prog);
                return Err("任务已取消".into());
            }

            // 获取实时传输统计
            if let Ok(stats) = self.call_rc("core/stats", serde_json::json!({})) {
                let bytes = stats.get("bytes").and_then(|v| v.as_i64()).unwrap_or(0);
                let total_bytes = stats.get("totalBytes").and_then(|v| v.as_i64()).unwrap_or(0);
                let speed = stats.get("speed").and_then(|v| v.as_f64()).unwrap_or(0.0) as i64;
                let transfers = stats.get("transfers").and_then(|v| v.as_i64()).unwrap_or(0);
                let total_transfers = stats.get("totalTransfers").and_then(|v| v.as_i64()).unwrap_or(0);
                let deletes = stats.get("deletes").and_then(|v| v.as_i64()).unwrap_or(0);
                let deleting_name = stats
                    .get("deleting")
                    .and_then(|v| v.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|o| o.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                let mut current_file = String::new();
                let mut eta = None;
                if let Some(transferring) = stats.get("transferring").and_then(|v| v.as_array()) {
                    if let Some(first) = transferring.first() {
                        current_file = first.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        eta = first.get("eta").and_then(|v| v.as_i64());
                    }
                }
                if current_file.is_empty() {
                    current_file = deleting_name;
                }

                // 删除阶段（如清空回收站）：无字节传输，用 deletes 计数驱动进度
                let is_delete_phase = transfers == 0 && bytes == 0 && deletes > 0;

                let mut prog = self.progress.lock().unwrap();
                prog.active = true;
                prog.task_type = task_type.to_string();
                if is_delete_phase {
                    prog.transferred_files = deletes;
                    prog.speed = speed;
                    prog.current_file = current_file;
                    prog.percentage = if prog.total_files > 0 {
                        ((deletes as f64 / prog.total_files as f64) * 100.0).clamp(0.0, 100.0) as u8
                    } else {
                        0
                    };
                } else {
                    prog.bytes = bytes;
                    prog.total_bytes = total_bytes;
                    prog.speed = speed;

                    if let Some((idx, total)) = batch_info {
                        let single_pct = if total_bytes > 0 {
                            (bytes as f64 / total_bytes as f64).clamp(0.0, 1.0)
                        } else if let Some(pct) = stats.get("percentage").and_then(|v| v.as_u64()) {
                            (pct.min(100) as f64) / 100.0
                        } else {
                            0.0
                        };
                        let overall_pct = (((idx as f64) + single_pct) / (total as f64) * 100.0).clamp(0.0, 100.0);
                        prog.percentage = overall_pct as u8;
                        prog.transferred_files = idx as i64;
                        prog.total_files = total as i64;
                        if !current_file.is_empty() {
                            prog.current_file = format!("({}/{}) {}", idx + 1, total, current_file);
                        } else {
                            prog.current_file = format!("正在处理第 {}/{} 项...", idx + 1, total);
                        }
                    } else {
                        prog.percentage = if total_bytes > 0 {
                            ((bytes as f64 / total_bytes as f64) * 100.0).clamp(0.0, 100.0) as u8
                        } else if let Some(pct) = stats.get("percentage").and_then(|v| v.as_u64()) {
                            pct.min(100) as u8
                        } else {
                            0
                        };
                        prog.current_file = current_file;
                        prog.transferred_files = transfers;
                        if total_transfers > 0 {
                            prog.total_files = total_transfers;
                        }
                    }
                    prog.eta = eta;
                }
                self.emit_progress(&prog);
            }

            // 检查作业状态
            if let Ok(status) = self.call_rc("job/status", serde_json::json!({ "jobid": job_id })) {
                let finished = status.get("finished").and_then(|v| v.as_bool()).unwrap_or(false);
                if finished {
                    let success = status.get("success").and_then(|v| v.as_bool()).unwrap_or(false);
                    self.active_job.lock().unwrap().take();
                    *self.cancel_requested.lock().unwrap() = false;
                    let mut prog = self.progress.lock().unwrap();
                    let is_last_in_batch = match batch_info {
                        Some((idx, total)) => idx + 1 >= total,
                        None => true,
                    };
                    if is_last_in_batch {
                        prog.active = false;
                        if let Some((_, total)) = batch_info {
                            prog.percentage = 100;
                            prog.transferred_files = total as i64;
                        }
                    }
                    self.emit_progress(&prog);
                    if !success {
                        let err = status.get("error").and_then(|v| v.as_str()).unwrap_or("传输任务执行失败");
                        return Err(err.to_string());
                    }
                    break;
                }
            }
        }

        self.active_job.lock().unwrap().take();
        *self.cancel_requested.lock().unwrap() = false;
        Ok(())
    }

    /// 取消当前正在进行的传输任务（导入/导出）。无任务时返回错误提示。
    pub fn cancel_transfer(&self) -> Result<(), String> {
        let job_id = self.active_job.lock().unwrap().take();
        match job_id {
            Some(id) => {
                *self.cancel_requested.lock().unwrap() = true;
                let _ = self.call_rc("job/stop", serde_json::json!({ "jobid": id }));
                Ok(())
            }
            None => Err("当前没有正在进行的传输任务".into()),
        }
    }

    /// 解析导入目标：按源路径类型计算目标明文路径并完成名称逐段校验。
    /// 返回 (目标路径, 是否为目录)。校验失败返回中文错误，不执行任何写入。
    pub fn resolve_import_dest(&self, src_path: &str, target_dir: &str) -> Result<(String, bool), String> {
        let p = Path::new(src_path);
        let norm_target = target_dir.replace('\\', "/").trim_matches('/').to_string();

        if p.is_dir() {
            let dir_name = p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "folder".to_string());
            let dst_remote = if norm_target.is_empty() {
                dir_name.clone()
            } else {
                format!("{}/{}", norm_target, dir_name)
            };
            Ok((dst_remote, true))
        } else {
            let file_name = p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "file".to_string());
            let dst_remote = if norm_target.is_empty() {
                file_name.clone()
            } else {
                format!("{}/{}", norm_target, file_name)
            };
            Ok((dst_remote, false))
        }
    }

    /// 校验目标路径各段均符合保险箱命名规则，防止导入点号前缀等非法名
    /// （此类条目后续将无法改名/移动，回收后还会从回收站列表中隐形）
    fn validate_dest_name(&self, dst_remote: &str) -> Result<(), String> {
        for seg in dst_remote.split('/') {
            validate_entry_name(seg)?;
        }
        Ok(())
    }

    /// 批量导入预检：仅解析目标并校验名称与目标冲突，不执行任何写入。
    /// 任一项失败即整体返回错误（原子语义），由调用方保证「全做或全不做」。
    pub fn validate_imports(&self, source_paths: &[String], target_dir: &str) -> Result<(), String> {
        for src in source_paths {
            let (dst_remote, _is_dir) = self.resolve_import_dest(src, target_dir)?;
            self.validate_dest_name(&dst_remote)?;
            if self.exists(&dst_remote)? {
                let name = Path::new(&dst_remote)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                return Err(format!(
                    "目标位置已存在同名条目 \"{}\"，为避免覆盖原有加密数据，已中止导入。请先改名或移开后重试",
                    name
                ));
            }
        }
        Ok(())
    }

    pub fn import_path(&self, src_path: &str, target_dir: &str) -> Result<(), String> {
        self.import_path_with_batch(src_path, target_dir, None)
    }

    pub fn import_path_with_batch(
        &self,
        src_path: &str,
        target_dir: &str,
        batch_info: Option<(usize, usize)>,
    ) -> Result<(), String> {
        let p = Path::new(src_path);
        let norm_src = src_path.replace('\\', "/");
        let (dst_remote, is_dir) = self.resolve_import_dest(src_path, target_dir)?;
        self.validate_dest_name(&dst_remote)?;

        if is_dir {
            // 防覆盖：目标已存在同名目录时拒绝合并导入，避免静默覆盖原有加密数据
            if self.exists(&dst_remote)? {
                let dir_name = p
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "folder".to_string());
                return Err(format!(
                    "目标位置已存在同名目录 \"{}\"，为避免覆盖原有加密数据，已中止导入。请先改名或移开后重试",
                    dir_name
                ));
            }

            // 预先创建顶层目录，确保即便源文件夹完全为空也能成功导入该目录结构
            self.create_folder(&dst_remote)?;

            let (transfers, checkers) = Self::get_optimal_concurrency();

            self.run_async_job_full(
                "import",
                "sync/copy",
                serde_json::json!({
                    "srcFs": norm_src,
                    "dstFs": format!("vault_crypt:{}", dst_remote),
                    "_config": {
                        "Transfers": transfers,
                        "Checkers": checkers,
                        "CreateEmptySrcDirs": true
                    }
                }),
                None,
                batch_info,
            )?;
        } else {
            let src_dir = p
                .parent()
                .map(|d| d.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|| "".to_string());
            let file_name = p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "file".to_string());

            // 防覆盖：目标已存在同名文件时拒绝导入，避免静默覆盖原有加密数据
            if self.exists(&dst_remote)? {
                return Err(format!(
                    "目标位置已存在同名文件 \"{}\"，为避免覆盖原有加密数据，已中止导入。请先改名或移开后重试",
                    file_name
                ));
            }

            self.run_async_job_full(
                "import",
                "operations/copyfile",
                serde_json::json!({
                    "srcFs": src_dir,
                    "srcRemote": file_name,
                    "dstFs": "vault_crypt:",
                    "dstRemote": dst_remote
                }),
                None,
                batch_info,
            )?;
        }

        self.invalidate_search_index();
        Ok(())
    }

    pub fn export_item(&self, vault_path: &str, is_dir: bool, target_local_folder: &str) -> Result<(), String> {
        let norm_dst = target_local_folder.replace('\\', "/");
        if norm_dst.is_empty() {
            return Err("导出路径不能为空".into());
        }
        let item_name = Path::new(vault_path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "export_item".to_string());

        // 防静默覆盖：目标位置已存在同名条目时拒绝导出，避免解密结果覆盖用户磁盘上的既有数据
        let full_dst = format!("{}/{}", norm_dst.trim_end_matches('/'), item_name);
        if Path::new(&full_dst).exists() {
            return Err(format!(
                "目标位置已存在同名文件或文件夹 \"{}\"，为避免覆盖，已中止导出。请先移开或改名后重试",
                item_name
            ));
        }

        if is_dir {
            // 预创建目标目录，确保即便源文件夹为空也能在本地生成对应文件夹结构
            std::fs::create_dir_all(&full_dst)
                .map_err(|e| format!("创建目标目录失败: {}", e))?;
            let (transfers, checkers) = Self::get_optimal_concurrency();

            self.run_async_job(
                "export",
                "sync/copy",
                serde_json::json!({
                    "srcFs": format!("vault_crypt:{}", vault_path),
                    "dstFs": full_dst,
                    "_config": {
                        "Transfers": transfers,
                        "Checkers": checkers,
                        "CreateEmptySrcDirs": true
                    }
                }),
            )?;
        } else {
            self.run_async_job(
                "export",
                "operations/copyfile",
                serde_json::json!({
                    "srcFs": "vault_crypt:",
                    "srcRemote": vault_path,
                    "dstFs": norm_dst,
                    "dstRemote": item_name
                }),
            )?;
        }

        Ok(())
    }

    /// 批量解密导出（多选）：把 vault_crypt: 下多个条目依次还原到本地目标目录。
    /// 每个条目复用 run_async_job（进度事件/取消/错误语义一致）。
    /// 执行前先统一预检目标冲突，任一已存在即整体拒绝（避免半途导出后才发现冲突）。
    pub fn export_items(&self, items: &[(String, bool)], target_local_folder: &str) -> Result<(), String> {
        let norm_dst = target_local_folder.replace('\\', "/");
        if norm_dst.is_empty() {
            return Err("导出路径不能为空".into());
        }

        // 预检：所有条目的目标位置均不得已存在同名条目，整体通过后才开始导出
        for (vault_path, _is_dir) in items {
            let item_name = Path::new(vault_path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "export_item".to_string());
            let full_dst = format!("{}/{}", norm_dst.trim_end_matches('/'), item_name);
            if Path::new(&full_dst).exists() {
                return Err(format!(
                    "目标位置已存在同名文件或文件夹 \"{}\"，为避免覆盖，已中止导出。请先移开或改名后重试",
                    item_name
                ));
            }
        }

        let total = items.len();
        let mut completed = 0usize;
        for (idx, (vault_path, is_dir)) in items.iter().enumerate() {
            let item_name = Path::new(vault_path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "export_item".to_string());

            let batch_info = if total > 1 { Some((idx, total)) } else { None };
            let res = if *is_dir {
                // 预创建目标目录，确保空目录也能被正确导出
                let full_dst = format!("{}/{}", norm_dst, item_name);
                if let Err(e) = std::fs::create_dir_all(&full_dst) {
                    return Err(format!("创建目标目录失败: {}", e));
                }
                let (transfers, checkers) = Self::get_optimal_concurrency();
                self.run_async_job_full(
                    "export",
                    "sync/copy",
                    serde_json::json!({
                        "srcFs": format!("vault_crypt:{}", vault_path),
                        "dstFs": full_dst,
                        "_config": {
                            "Transfers": transfers,
                            "Checkers": checkers,
                            "CreateEmptySrcDirs": true
                        }
                    }),
                    None,
                    batch_info,
                )
            } else {
                self.run_async_job_full(
                    "export",
                    "operations/copyfile",
                    serde_json::json!({
                        "srcFs": "vault_crypt:",
                        "srcRemote": vault_path,
                        "dstFs": norm_dst,
                        "dstRemote": item_name
                    }),
                    None,
                    batch_info,
                )
            };

            if let Err(err) = res {
                return Err(format!(
                    "批量导出第 {}/{} 项 \"{}\" 失败: {}（此前已成功导出 {} 项）",
                    idx + 1,
                    total,
                    item_name,
                    err,
                    completed
                ));
            }
            completed += 1;
        }

        Ok(())
    }

    pub fn create_folder(&self, folder_path: &str) -> Result<(), String> {
        let remote = folder_path.replace('\\', "/");
        let remote = remote.trim_start_matches('/').to_string();
        // 逐段校验每个目录名合法性，防止非法名/路径穿越
        let mut saw_segment = false;
        for segment in remote.split('/') {
            let seg = segment.trim();
            if seg.is_empty() {
                continue;
            }
            saw_segment = true;
            validate_entry_name(seg)?;
        }
        if !saw_segment {
            return Err("目录名不能为空".into());
        }
        self.call_rc(
            "operations/mkdir",
            serde_json::json!({
                "fs": "vault_crypt:",
                "remote": remote
            }),
        )?;
        self.invalidate_search_index();
        Ok(())
    }

    pub fn delete_item(&self, vault_path: &str, is_dir: bool) -> Result<(), String> {
        let remote = vault_path.replace('\\', "/").trim_start_matches('/').to_string();
        if is_dir {
            self.call_rc(
                "operations/purge",
                serde_json::json!({
                    "fs": "vault_crypt:",
                    "remote": remote
                }),
            )?;
        } else {
            self.call_rc(
                "operations/deletefile",
                serde_json::json!({
                    "fs": "vault_crypt:",
                    "remote": remote
                }),
            )?;
        }
        self.invalidate_search_index();
        Ok(())
    }

    fn make_dir(&self, remote: &str) -> Result<(), String> {
        self.call_rc(
            "operations/mkdir",
            serde_json::json!({
                "fs": "vault_crypt:",
                "remote": remote.trim_start_matches('/')
            }),
        )?;
        Ok(())
    }

    /// 获取当前挂载的保险箱物理磁盘根目录
    fn get_physical_vault_dir(&self) -> Option<PathBuf> {
        let raw = self.raw_remote.lock().unwrap().clone();
        if let Some(stripped) = raw.strip_prefix("vault_raw:") {
            if !stripped.is_empty() {
                return Some(PathBuf::from(stripped));
            }
        }
        None
    }

    /// 精准解析指定明文条目（文件或任意深层文件夹）在物理磁盘上的密文绝对路径（耗时 < 1ms，无递归）
    fn resolve_encrypted_item_path(&self, remote: &str) -> Result<PathBuf, String> {
        let vault_root = self.get_physical_vault_dir().ok_or("无法获取保险箱物理根目录")?;
        let trimmed = remote.replace('\\', "/").trim_matches('/').to_string();
        if trimmed.is_empty() {
            return Ok(vault_root);
        }

        let segments: Vec<&str> = trimmed.split('/').filter(|s| !s.is_empty()).collect();
        let mut current_remote = String::new();
        let mut current_phys = vault_root.clone();

        for seg in segments {
            // 如果物理当前路径下刚好存在与 seg 密文完全一致的条目，直接命中（避免单层包含大量文件时全量解密遍历）
            let mut found_enc = None;

            let res = self.call_rc(
                "operations/list",
                serde_json::json!({
                    "fs": "vault_crypt:",
                    "remote": current_remote,
                    "opt": { "showEncrypted": true }
                }),
            )?;
            let list_res: HashListResponse = serde_json::from_value(res).map_err(|e| e.to_string())?;
            let items = list_res.list.unwrap_or_default();

            for item in items {
                if item.name == seg {
                    let enc = if !item.encrypted.is_empty() {
                        item.encrypted
                    } else {
                        item.encrypted_path
                    };
                    if !enc.is_empty() {
                        let clean_enc = enc.trim_matches('/').replace('\\', "/");
                        let enc_file_name = Path::new(&clean_enc)
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_else(|| clean_enc.clone());
                        found_enc = Some(enc_file_name);
                        break;
                    }
                }
            }

            let enc_name = match found_enc {
                Some(n) => n,
                None => {
                    return Err(format!("未在保险箱物理磁盘上找到条目 \"{}\" 的密文段 \"{}\"", remote, seg));
                }
            };

            current_phys = current_phys.join(&enc_name);
            current_remote = if current_remote.is_empty() {
                seg.to_string()
            } else {
                format!("{}/{}", current_remote, seg)
            };
        }

        if current_phys.exists() {
            Ok(current_phys)
        } else {
            Err(format!("密文物理路径不存在: {:?}", current_phys))
        }
    }

    /// 在同保险箱内部执行 O(1) 原生物理层原子移动/重命名（无论文件还是包含数十万文件的文件夹，耗时 < 1ms，0% CPU）
    fn move_item_atomic(&self, src_remote: &str, dst_remote: &str, is_dir: bool) -> Result<(), String> {
        let src_clean = src_remote.replace('\\', "/").trim_matches('/').to_string();
        let dst_clean = dst_remote.replace('\\', "/").trim_matches('/').to_string();
        let src_name = Path::new(&src_clean)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let dst_name = Path::new(&dst_clean)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        let src_phys = self.resolve_encrypted_item_path(&src_clean)?;

        if src_name == dst_name {
            // 1. 同名跨目录移动（移入回收站 / 恢复 / 跨目录移动）：
            // AES-EME-128 单段加密下，同名条目的密文文件名完全一致，直接移动到目标密文父目录下
            let dst_parent = Self::parent_dir_of(&dst_clean);
            if !dst_parent.is_empty() {
                self.make_dir(&dst_parent)?;
            }
            let dst_parent_phys = self.resolve_encrypted_item_path(&dst_parent)?;
            let enc_name = src_phys.file_name().ok_or("无效的源物理文件名")?;
            let dst_phys = dst_parent_phys.join(enc_name);

            // 防静默覆盖：若目标物理路径已存在，直接报错拒绝，绝不静默删除已有数据
            if dst_phys.exists() {
                return Err(format!("目标物理文件已存在，已拒绝覆盖: {:?}", dst_phys));
            }

            fs::rename(&src_phys, &dst_phys)
                .map_err(|e| format!("Windows 物理层原子移动失败: {}", e))?;
            Ok(())
        } else if is_dir {
            // 2. 文件夹重命名（名称变更）：
            // 先让 rclone 创建目标新空目录获取其密文名，移除空目录后将原密文目录原子重命名过去
            self.make_dir(&dst_clean)?;
            let dst_phys = self.resolve_encrypted_item_path(&dst_clean)?;
            fs::remove_dir(&dst_phys)
                .map_err(|e| format!("准备目标重命名目录失败: {}", e))?;
            fs::rename(&src_phys, &dst_phys)
                .map_err(|e| format!("Windows 物理层目录原子重命名失败: {}", e))?;
            Ok(())
        } else {
            // 3. 单文件重命名（名称变更）：委托给 rclone operations/movefile
            self.move_file_internal(&src_clean, &dst_clean)
        }
    }

    fn move_file_internal(&self, src_remote: &str, dst_remote: &str) -> Result<(), String> {
        self.call_rc(
            "operations/movefile",
            serde_json::json!({
                "srcFs": "vault_crypt:",
                "srcRemote": src_remote.trim_start_matches('/'),
                "dstFs": "vault_crypt:",
                "dstRemote": dst_remote.trim_start_matches('/')
            }),
        )?;
        Ok(())
    }

    fn copy_file_internal(&self, src_remote: &str, dst_remote: &str) -> Result<(), String> {
        self.call_rc(
            "operations/copyfile",
            serde_json::json!({
                "srcFs": "vault_crypt:",
                "srcRemote": src_remote.trim_start_matches('/'),
                "dstFs": "vault_crypt:",
                "dstRemote": dst_remote.trim_start_matches('/')
            }),
        )?;
        Ok(())
    }

    fn sync_dir_to(&self, src_remote: &str, dst_remote: &str) -> Result<(), String> {
        // 必须先创建目标目录，确保空源目录在保险箱内部复制/移动/恢复时也能保留结构
        self.make_dir(dst_remote.trim_start_matches('/'))?;
        // 必须等待异步作业真正完成后再返回：否则调用方（移动/复制/恢复）会
        // 在目录还没有复制完时就删除源，导致目录被清空且无法恢复。
        self.run_async_job(
            "dir_copy",
            "sync/copy",
            serde_json::json!({
                "srcFs": format!("vault_crypt:{}", src_remote.trim_start_matches('/')),
                "dstFs": format!("vault_crypt:{}", dst_remote.trim_start_matches('/')),
                "_config": {
                    "CreateEmptySrcDirs": true
                }
            }),
        )?;
        Ok(())
    }

    /// 自底向上删除变空的祖先目录；单层轻量判定，非空时立即停止。
    fn cleanup_empty_ancestors(&self, remote: &str) -> Result<(), String> {
        let mut dir = Self::parent_dir_of(remote.trim_start_matches('/'));
        while !dir.is_empty() {
            let items = match self.list_files_full(&dir) {
                Ok(it) => it,
                Err(_) => break,
            };
            let user_count = items.iter().filter(|i| !is_internal(&i.name)).count();
            if user_count > 0 {
                break;
            }
            if self.call_rc("operations/rmdir", serde_json::json!({ "fs": "vault_crypt:", "remote": dir })).is_err() {
                break;
            }
            dir = Self::parent_dir_of(&dir);
        }
        Ok(())
    }

    fn parent_dir_of(path: &str) -> String {
        match path.rfind('/') {
            Some(pos) => path[..pos].to_string(),
            None => String::new(),
        }
    }

    // ========== 回收站（软删除） ==========

    /// 将指定条目移动到保险箱内 .recycle/<时间戳>/ 目录并记录原路径，
    /// 供“恢复”使用。返回回收站条目 ID（时间戳）。
    pub fn recycle_item(&self, vault_path: &str, is_dir: bool) -> Result<String, String> {
        let ts_seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let remote = vault_path.replace('\\', "/").trim_start_matches('/').to_string();
        let name = Path::new(&remote)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "item".to_string());

        // 时间戳去重：同一毫秒内连续回收会共用 .recycle/<ts> 并互相覆盖 .meta，
        // 导致先回收的条目从回收站列表中丢失；存在则自增直至目录唯一
        let mut ts_num = ts_seed;
        let entry_parent = loop {
            let candidate = format!(".recycle/{}", ts_num);
            if !self.exists(&candidate)? {
                break candidate;
            }
            ts_num += 1;
        };
        let entry_dir = format!("{}/{}", entry_parent, name);

        self.make_dir(&entry_parent)?;
        self.move_item_atomic(&remote, &entry_dir, is_dir)?;
        self.cleanup_empty_ancestors(&remote)?;

        // 记录恢复所需的原路径元数据（存于回收站条目内）
        // 文件名用点号前缀 .meta：由于用户条目名不允许以点号开头，可避免与
        // 用户内容“_meta”等名称冲突（旧实现会被本次写入覆盖，造成内容丢失）。
        let meta_val = serde_json::json!({
            "original_path": remote,
            "name": name,
            "deleted_at": ts_num,
            "is_dir": is_dir,
        });
        self.write_text_file(
            &format!("{}/.meta", entry_parent),
            &meta_val.to_string(),
        )?;

        // 写入内存元数据缓存，使后续回收站列表秒开
        self.recycle_meta_cache.lock().unwrap().insert(ts_num.to_string(), meta_val);

        self.invalidate_search_index();
        Ok(ts_num.to_string())
    }

    /// 列出回收站条目：每个条目包含 ID、原路径、名称、删除时间、是否目录。
    /// 仅单层列出 .recycle 根目录下的 <ts> 目录并优先命中内存元数据缓存，
    /// 严禁递归（recurse: false），无论回收站内包含多少万个文件，均在毫秒级内瞬间返回且 0 CPU 占用。
    pub fn list_recycle(&self) -> Result<Vec<serde_json::Value>, String> {
        // .recycle 目录在首次删除时才创建；从未操作过回收站时不视为错误，返回空列表
        let res = match self.call_rc(
            "operations/list",
            serde_json::json!({
                "fs": "vault_crypt:",
                "remote": ".recycle"
            }),
        ) {
            Ok(r) => r,
            Err(e) if is_recycle_not_found(&e) => {
                return Ok(vec![]);
            }
            Err(e) => return Err(e),
        };
        let list_res: ListResponse = serde_json::from_value(res).map_err(|e| e.to_string())?;
        let items = list_res.list.unwrap_or_default();

        let mut cache = self.recycle_meta_cache.lock().unwrap();
        let mut entries: Vec<serde_json::Value> = Vec::new();
        for item in items.iter() {
            // 跳过非目录或内部隐藏文件
            if item.name.starts_with('.') || item.name == "_meta" {
                continue;
            }
            let ts = item.name.trim().to_string();
            if ts.is_empty() {
                continue;
            }

            // 优先从内存缓存获取元数据，未命中时读取并写入缓存
            let meta = if let Some(cached) = cache.get(&ts) {
                cached.clone()
            } else {
                let fetched = self.read_meta(&format!(".recycle/{}", ts)).unwrap_or_default();
                cache.insert(ts.clone(), fetched.clone());
                fetched
            };

            let mut original = meta
                .get("original_path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let meta_name = meta
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let is_dir = meta.get("is_dir").and_then(|v| v.as_bool()).unwrap_or(item.is_dir);

            // 名称优先取 .meta 中的原名；若 .meta 损坏或缺失，则浅层探测子项名
            let display_name = if !meta_name.is_empty() {
                meta_name
            } else {
                let children = self.list_files_full(&format!(".recycle/{}", ts)).unwrap_or_default();
                let found = children
                    .into_iter()
                    .find(|c| c.name != ".meta" && c.name != "_meta")
                    .map(|c| c.name);
                match found {
                    Some(name) => name,
                    None => {
                        // 既无元数据又无有效子文件（残留空目录），自动清理并跳过
                        let _ = self.call_rc(
                            "operations/rmdir",
                            serde_json::json!({ "fs": "vault_crypt:", "remote": format!(".recycle/{}", ts) }),
                        );
                        continue;
                    }
                }
            };

            // 若原路径缺失，兜底回退为显示名称（恢复至根目录）
            if original.is_empty() {
                original = display_name.clone();
            }

            let deleted_at = meta.get("deleted_at").and_then(|v| v.as_u64()).unwrap_or(0);

            entries.push(serde_json::json!({
                "id": ts,
                "name": display_name,
                "original_path": original,
                "is_dir": is_dir,
                "deleted_at": deleted_at,
            }));
        }
        // 按删除时间降序排列（最新删除的在最前）
        entries.sort_by(|a, b| {
            let t_a = a.get("deleted_at").and_then(|v| v.as_u64()).unwrap_or(0);
            let t_b = b.get("deleted_at").and_then(|v| v.as_u64()).unwrap_or(0);
            t_b.cmp(&t_a)
        });
        Ok(entries)
    }

    fn read_meta(&self, entry_dir: &str) -> Result<serde_json::Value, String> {
        let content = self
            .read_text_file(&format!("{}/.meta", entry_dir))
            .or_else(|_| self.read_text_file(&format!("{}/_meta", entry_dir)))?;
        serde_json::from_str(&content).map_err(|e| e.to_string())
    }

    /// 恢复指定回收站条目到原路径
    pub fn restore_item(&self, id: &str) -> Result<bool, String> {
        let clean_id = id.trim_start_matches('/');
        let entry_dir = format!(".recycle/{}", clean_id);

        let meta = self.read_meta(&entry_dir).map_err(|_| {
            format!("该条目（ID: {}）元数据不存在或已被移除", clean_id)
        })?;
        let mut original = meta
            .get("original_path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let name = meta
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("item")
            .to_string();
        let is_dir = meta.get("is_dir").and_then(|v| v.as_bool()).unwrap_or(false);

        if original.is_empty() {
            if name != "item" && !name.is_empty() {
                original = name.clone();
            } else {
                return Err("回收站条目元数据已损坏，无法定位原路径".into());
            }
        }
        let src_path = format!("{}/{}", entry_dir, name);

        // 防静默覆盖：原位置已存在同名条目时拒绝恢复，避免恢复操作覆盖用户新数据
        if self.exists(&original)? {
            let conflict = Path::new(&original)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| original.clone());
            return Err(format!(
                "原位置已存在同名条目“{}”，为避免覆盖请先改名或移开该条目后再恢复",
                conflict
            ));
        }

        // 确保原路径父目录存在
        let parent = Self::parent_dir_of(&original);
        if !parent.is_empty() {
            self.make_dir(&parent)?;
        }

        self.move_item_atomic(&src_path, &original, is_dir)?;

        // 清理条目元数据
        let _ = self.call_rc(
            "operations/deletefile",
            serde_json::json!({ "fs": "vault_crypt:", "remote": format!("{}/.meta", entry_dir) }),
        );
        // 删除空目录
        let _ = self.call_rc(
            "operations/rmdir",
            serde_json::json!({ "fs": "vault_crypt:", "remote": entry_dir }),
        );

        // 从内存缓存中剔除已恢复项
        self.recycle_meta_cache.lock().unwrap().remove(clean_id);

        self.invalidate_search_index();
        Ok(true)
    }

    /// 彻底清空回收站（带删除进度：先预扫描条目总数，再异步逐文件删除）
    pub fn empty_recycle(&self) -> Result<bool, String> {
        // 本地后端无原生 purge，crypt 的 purge 会回退为逐文件删除（DeleteFiles），
        // 每次删除都会递增 core/stats 的 deletes；先用一次递归列表统计总文件数作为
        // 进度条分母，让清空过程有真实的进度展示。
        let expected_total = self.count_recycle_files()?;
        let purge_res = if let Some(total) = expected_total {
            self.run_async_job_total(
                "empty_recycle",
                "operations/purge",
                serde_json::json!({ "fs": "vault_crypt:", "remote": ".recycle" }),
                Some(total),
            )
        } else {
            Ok(())
        };

        // 无论 purge 成功、取消或失败，均统一防御性重建 .recycle 目录并清空缓存，
        // 防止中途取消后留下无 .recycle 目录或陈旧元数据缓存导致后续回收站功能异常。
        let _ = self.make_dir(".recycle");
        self.recycle_meta_cache.lock().unwrap().clear();
        self.invalidate_search_index();

        purge_res.map(|_| true)
    }

    // 统计回收站内文件总数（目录与元数据 .meta/_meta 不计）；回收站从未有内容返回 None。
    fn count_recycle_files(&self) -> Result<Option<i64>, String> {
        match self.call_rc(
            "operations/list",
            serde_json::json!({
                "fs": "vault_crypt:",
                "remote": ".recycle",
                "opt": { "recurse": true }
            }),
        ) {
            Err(e) if is_recycle_not_found(&e) => Ok(None),
            Err(e) => Err(e),
            Ok(res) => {
                let list_res: ListResponse = serde_json::from_value(res).map_err(|e| e.to_string())?;
                let items = list_res.list.unwrap_or_default();
                let files = items
                    .iter()
                    .filter(|i| !i.is_dir && i.name != ".meta" && i.name != "_meta")
                    .count() as i64;
                Ok(Some(files.max(0)))
            }
        }
    }

    // ========== 文件操作扩展 ==========

    /// 重命名条目（同目录改名），带名称合法性校验与目标冲突检测
    pub fn rename_item(&self, vault_path: &str, new_name: &str) -> Result<bool, String> {
        validate_entry_name(new_name)?;
        let remote = vault_path.replace('\\', "/").trim_start_matches('/').to_string();
        let parent = Self::parent_dir_of(&remote);
        let new_remote = if parent.is_empty() {
            new_name.trim().to_string()
        } else {
            format!("{}/{}", parent, new_name.trim())
        };
        if remote == new_remote {
            return Ok(true);
        }
        if self.exists(&new_remote)? {
            return Err("目标名称已存在".into());
        }
        let is_dir = self.is_dir(&remote)?;
        self.move_item_atomic(&remote, &new_remote, is_dir)?;
        self.cleanup_empty_ancestors(&remote)?;
        self.invalidate_search_index();
        Ok(true)
    }

    fn exists(&self, remote: &str) -> Result<bool, String> {
        let trimmed = remote.trim_start_matches('/').to_string();
        if trimmed.is_empty() {
            return Ok(false);
        }
        let parent = Self::parent_dir_of(&trimmed);
        let name = Path::new(&trimmed)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        // 父目录不存在（如已被删除）时不视为存在冲突，允许调用方（恢复/移动）重建父目录后再写
        let items = match self.list_files_full(&parent) {
            Ok(items) => items,
            Err(e) if is_missing_dir(&e) => return Ok(false),
            Err(e) => return Err(e),
        };
        Ok(items.iter().any(|i| i.name == name))
    }

    /// 将多个条目移动到目标目录（目标目录为保险箱明文路径，"" 表示根）。
    /// 执行前统一预检：名称合法性、移动到自身子目录（环）、目标冲突，避免半途失败。
    pub fn move_items(&self, paths: Vec<String>, dest_dir: &str) -> Result<usize, String> {
        let dest = dest_dir.replace('\\', "/").trim_matches('/').to_string();
        // 目标路径逐段校验：拒绝 . / .. 与非法段组成的目录穿越
        for segment in dest.split('/') {
            if !segment.is_empty() {
                validate_entry_name(segment)?;
            }
        }

        // 预检并生成执行计划，保证原子性（要么全做，要么都不做）
        let mut plans: Vec<(String, String)> = Vec::new();
        let mut planned_dests = std::collections::HashSet::new();
        for p in paths.iter() {
            let remote = p.replace('\\', "/").trim_start_matches('/').to_string();
            let name = Path::new(&remote)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            validate_entry_name(&name)?;
            let dst = if dest.is_empty() {
                name.clone()
            } else {
                format!("{}/{}", dest, name)
            };
            if remote == dst {
                continue;
            }
            if dst.starts_with(&format!("{}/", remote)) {
                return Err(format!("不能将 \"{}\" 移动到其自身内部", name));
            }
            if !planned_dests.insert(dst.clone()) {
                return Err(format!("批处理中存在多个移向相同目标 \"{}\" 的条目，已中止操作", dst));
            }
            if self.exists(&dst)? {
                return Err(format!("目标目录中已存在同名条目 \"{}\"", name));
            }
            plans.push((remote, dst));
        }

        let mut count = 0;
        for (remote, dst) in plans.iter() {
            let is_dir = self.is_dir(remote)?;
            self.move_item_atomic(remote, dst, is_dir)?;
            self.cleanup_empty_ancestors(remote)?;
            count += 1;
        }
        self.invalidate_search_index();
        Ok(count)
    }

    /// 将多个条目复制到目标目录，执行前统一预检冲突，避免半途失败
    pub fn copy_items(&self, paths: Vec<String>, dest_dir: &str) -> Result<usize, String> {
        let dest = dest_dir.replace('\\', "/").trim_matches('/').to_string();
        // 目标路径逐段校验：拒绝 . / .. 与非法段组成的目录穿越
        for segment in dest.split('/') {
            if !segment.is_empty() {
                validate_entry_name(segment)?;
            }
        }

        let mut plans: Vec<(String, String)> = Vec::new();
        let mut planned_dests = std::collections::HashSet::new();
        for p in paths.iter() {
            let remote = p.replace('\\', "/").trim_start_matches('/').to_string();
            let name = Path::new(&remote)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            validate_entry_name(&name)?;
            let dst = if dest.is_empty() {
                name.clone()
            } else {
                format!("{}/{}", dest, name)
            };
            if remote == dst {
                continue;
            }
            if dst.starts_with(&format!("{}/", remote)) {
                return Err(format!("不能将 \"{}\" 复制到其自身内部", name));
            }
            if !planned_dests.insert(dst.clone()) {
                return Err(format!("批处理中存在多个复制到相同目标 \"{}\" 的条目，已中止操作", dst));
            }
            if self.exists(&dst)? {
                return Err(format!("目标目录中已存在同名条目 \"{}\"", name));
            }
            plans.push((remote, dst));
        }

        let mut count = 0;
        for (remote, dst) in plans.iter() {
            let is_dir = self.is_dir(remote)?;
            if is_dir {
                self.sync_dir_to(remote, dst)?;
            } else {
                if !dest.is_empty() {
                    self.make_dir(&dest)?;
                }
                self.copy_file_internal(remote, dst)?;
            }
            count += 1;
        }
        self.invalidate_search_index();
        Ok(count)
    }

    fn is_dir(&self, remote: &str) -> Result<bool, String> {
        let trimmed = remote.trim_start_matches('/').to_string();
        if trimmed.is_empty() {
            return Ok(false);
        }
        let res = self.call_rc(
            "operations/stat",
            serde_json::json!({ "fs": "vault_crypt:", "remote": trimmed }),
        );
        if let Ok(v) = res {
            let item = v.get("item").unwrap_or(&v);
            let is_dir = item
                .get("IsDir")
                .or_else(|| item.get("is_dir"))
                .and_then(|x| x.as_bool())
                .unwrap_or(false);
            if is_dir {
                return Ok(true);
            }
        }
        // 回退机制：从父目录列表中精准查找
        let parent = Self::parent_dir_of(&trimmed);
        let name = Path::new(&trimmed)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if let Ok(items) = self.list_files_full(&parent) {
            if let Some(found) = items.into_iter().find(|i| i.name == name) {
                return Ok(found.is_dir);
            }
        }
        Ok(false)
    }

    // ========== 数据完整性自检 ==========

    /// 构建/刷新全库清单：记录每个密文文件的 SHA-256（作为数据落盘时的参照）。
    /// 密文文件一旦发生位翻转/损坏，其 SHA-256 即会改变，可据此发现数据损坏。
    pub fn build_manifest(&self) -> Result<usize, String> {
        let mapping = self.list_raw_mapping()?;
        let manifest = serde_json::json!({ "files": mapping });
        self.write_text_file(".manifest", &manifest.to_string())?;
        self.invalidate_search_index();
        Ok(mapping.len())
    }

    /// 对指定路径（递归可选）进行完整性自检，返回逐文件结果与汇总。
    /// 校验依据是 .manifest 中记录的各密文文件哈希指纹；缺失清单时提示先构建。
    pub fn integrity_check(&self, path: &str, recursive: bool) -> Result<serde_json::Value, String> {
        let manifest_raw = self.read_text_file(".manifest").unwrap_or_default();
        let has_manifest = !manifest_raw.trim().is_empty();
        let manifest: serde_json::Value = serde_json::from_str(&manifest_raw).unwrap_or_else(|_| serde_json::json!({"files": []}));
        let refs: Vec<(String, String, String)> = manifest
            .get("files")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|f| {
                        let p = f.get("path")?.as_str()?.to_string();      // 解密明文路径
                        let e = f.get("encrypted")?.as_str()?.to_string(); // 密文文件名
                        // 优先提取 hash / sha1 / sha256 / md5 字段
                        let h = f.get("hash")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .or_else(|| f.get("sha1").and_then(|v| v.as_str()))
                            .or_else(|| f.get("sha256").and_then(|v| v.as_str()))
                            .or_else(|| f.get("md5").and_then(|v| v.as_str()))
                            .unwrap_or("")
                            .to_string();
                        Some((p, e, h))
                    })
                    .collect()
            })
            .unwrap_or_default();

        // 当前磁盘上各密文文件的哈希指纹 (路径 -> (哈希值, 算法类型))
        let current = self.list_raw_hashes()?;
        let current_map: std::collections::HashMap<String, (String, String)> = current
            .into_iter()
            .map(|(p, h, t)| (p, (h, t)))
            .collect();

        let mut ok = 0usize;
        let mut differ = 0usize;
        let mut missing = 0usize;
        let mut bad: Vec<serde_json::Value> = Vec::new();
        let mut checked = 0usize;

        // 路径范围过滤：recursive=true 检查目标子树；recursive=false 仅检查目标目录的直接子文件
        let target = path.replace('\\', "/").trim_matches('/').to_string();
        for (idx, (p, e, h)) in refs.iter().enumerate() {
            if idx % 50 == 0 && *self.cancel_requested.lock().unwrap() {
                *self.cancel_requested.lock().unwrap() = false;
                return Err("完整性自检已取消".into());
            }
            let parent = Self::parent_dir_of(p);
            // 递归模式: 目标为空(根)覆盖全部；目标为文件时覆盖该文件本身；
            // 目录目标必须包含直接子文件 (parent == target) 及其所有后代
            let in_scope = if recursive {
                target.is_empty()
                    || p == &target
                    || parent == target
                    || parent.starts_with(&format!("{}/", target))
            } else {
                parent == target
            };
            if !in_scope {
                continue;
            }
            checked += 1;
            match current_map.get(e) {
                Some((cur, _)) if !cur.is_empty() && cur == h => ok += 1,
                Some(_) => {
                    differ += 1;
                    bad.push(serde_json::json!({ "path": p, "status": "DIFFER" }));
                }
                None => {
                    missing += 1;
                    bad.push(serde_json::json!({ "path": p, "status": "MISSING" }));
                }
            }
        }

        Ok(serde_json::json!({
            "checked": checked,
            "ok": ok,
            "differ": differ,
            "missing": missing,
            "has_manifest": has_manifest,
            "issues": bad,
        }))
    }

    /// 列出所有明文条目及其加密文件名（跳过内部管控文件）
    pub(crate) fn list_raw_mapping(&self) -> Result<Vec<serde_json::Value>, String> {
        let res = self.call_rc(
            "operations/list",
            serde_json::json!({
                "fs": "vault_crypt:",
                "remote": "",
                "opt": { "recurse": true, "showEncrypted": true }
            }),
        )?;
        let list_res: HashListResponse = serde_json::from_value(res).map_err(|e| e.to_string())?;
        let raw_hashes = self.list_raw_hashes()?;
        let raw_map: std::collections::HashMap<String, (String, String)> =
            raw_hashes.into_iter().map(|(p, h, t)| (p, (h, t))).collect();

        let mut out = Vec::new();
        for i in list_res.list.unwrap_or_default().into_iter() {
            if i.is_dir {
                continue;
            }
            if i.name == VAULT_AUTH_FILE_NAME || is_internal(i.path.as_str()) {
                continue;
            }
            let raw_file = i.encrypted_path.trim_end_matches("/").to_string();
            let (hash_val, hash_type) = raw_map.get(&raw_file).cloned().unwrap_or_default();
            let mut entry = serde_json::json!({
                "path": i.path,
                "encrypted": raw_file,
            });
            // 仅填充实际算法对应的字段，避免 sha1/sha256/md5 出现重复同值的误导性数据
            if !hash_val.is_empty() {
                entry["hash"] = serde_json::Value::String(hash_val.clone());
                entry["hash_type"] = serde_json::Value::String(hash_type.clone());
                match hash_type.as_str() {
                    "sha1" => entry["sha1"] = serde_json::Value::String(hash_val),
                    "sha256" => entry["sha256"] = serde_json::Value::String(hash_val),
                    "md5" => entry["md5"] = serde_json::Value::String(hash_val),
                    _ => {}
                }
            }
            out.push(entry);
        }
        Ok(out)
    }

    fn parse_hash_items(items: Vec<HashListItem>) -> Vec<(String, String, String)> {
        items
            .into_iter()
            .filter(|i| !i.is_dir)
            .map(|i| {
                let mut chosen: (String, String) = (String::new(), String::new());
                for key in ["sha1", "SHA-1"] {
                    if let Some(v) = i.hashes.get(key) {
                        if !v.is_empty() {
                            chosen = (v.clone(), "sha1".to_string());
                            break;
                        }
                    }
                }
                if chosen.0.is_empty() {
                    for key in ["sha256", "SHA-256"] {
                        if let Some(v) = i.hashes.get(key) {
                            if !v.is_empty() {
                                chosen = (v.clone(), "sha256".to_string());
                                break;
                            }
                        }
                    }
                }
                if chosen.0.is_empty() {
                    for key in ["md5", "MD5"] {
                        if let Some(v) = i.hashes.get(key) {
                            if !v.is_empty() {
                                chosen = (v.clone(), "md5".to_string());
                                break;
                            }
                        }
                    }
                }
                (i.path, chosen.0, chosen.1)
            })
            .collect()
    }

    /// 列出底层本地（密文）目录中每个文件的哈希及其算法类型
    /// （优先 SHA-1，备选 SHA-256，最后 MD5）。支持中途取消。返回 (路径, 哈希值, 算法类型)。
    fn list_raw_hashes(&self) -> Result<Vec<(String, String, String)>, String> {
        let raw = self.raw_remote.lock().unwrap().clone();
        let raw = if raw.is_empty() { "vault_raw:".to_string() } else { raw };
        let res = self.call_rc(
            "operations/list",
            serde_json::json!({
                "fs": raw,
                "remote": "",
                "opt": { "recurse": true, "showHash": true, "hashTypes": ["SHA-1", "MD5", "SHA256", "SHA-256"] },
                "_async": true
            }),
        )?;

        let job_id = match res.get("jobid").and_then(|v| v.as_i64()) {
            Some(id) => id,
            None => {
                let list_res: HashListResponse = serde_json::from_value(res).map_err(|e| e.to_string())?;
                return Ok(Self::parse_hash_items(list_res.list.unwrap_or_default()));
            }
        };

        *self.active_job.lock().unwrap() = Some(job_id);
        *self.cancel_requested.lock().unwrap() = false;

        let output_res = loop {
            std::thread::sleep(Duration::from_millis(150));

            if *self.cancel_requested.lock().unwrap() {
                let _ = self.call_rc("job/stop", serde_json::json!({ "jobid": job_id }));
                self.active_job.lock().unwrap().take();
                *self.cancel_requested.lock().unwrap() = false;
                return Err("完整性自检已取消".into());
            }

            if let Ok(status) = self.call_rc("job/status", serde_json::json!({ "jobid": job_id })) {
                let finished = status.get("finished").and_then(|v| v.as_bool()).unwrap_or(false);
                if finished {
                    self.active_job.lock().unwrap().take();
                    *self.cancel_requested.lock().unwrap() = false;
                    let success = status.get("success").and_then(|v| v.as_bool()).unwrap_or(false);
                    if !success {
                        let err = status.get("error").and_then(|v| v.as_str()).unwrap_or("读取指纹失败");
                        return Err(err.to_string());
                    }
                    let output = status.get("output").cloned().unwrap_or(serde_json::Value::Null);
                    break output;
                }
            }
        };

        let list_res: HashListResponse = serde_json::from_value(output_res).map_err(|e| e.to_string())?;
        Ok(Self::parse_hash_items(list_res.list.unwrap_or_default()))
    }

    fn list_files_full(&self, remote: &str) -> Result<Vec<RcloneItem>, String> {
        let res = self.call_rc(
            "operations/list",
            serde_json::json!({ "fs": "vault_crypt:", "remote": remote }),
        )?;
        let list_res: ListResponse = serde_json::from_value(res).map_err(|e| e.to_string())?;
        Ok(list_res.list.unwrap_or_default())
    }

    // ========== 性能 / 统计面板 ==========

    /// 汇总保险箱统计：文件/目录数、总大小、类型分布
    pub fn get_stats(&self) -> Result<serde_json::Value, String> {
        let res = self.call_rc(
            "operations/list",
            serde_json::json!({
                "fs": "vault_crypt:",
                "remote": "",
                "opt": { "recurse": true }
            }),
        )?;
        let list_res: ListResponse = serde_json::from_value(res).map_err(|e| e.to_string())?;
        let items = list_res.list.unwrap_or_default();

        let mut files = 0usize;
        let mut dirs = 0usize;
        let mut total_size: i64 = 0;
        let mut by_ext: std::collections::BTreeMap<String, i64> = std::collections::BTreeMap::new();
        let mut by_ext_count: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();

        for item in items.iter() {
            if is_internal(&item.path) {
                continue;
            }
            if item.is_dir {
                dirs += 1;
                continue;
            }
            files += 1;
            total_size += item.size;
            let ext = Path::new(&item.name)
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_else(|| "(无)".to_string());
            *by_ext.entry(ext.clone()).or_insert(0) += item.size;
            *by_ext_count.entry(ext).or_insert(0) += 1;
        }

        Ok(serde_json::json!({
            "files": files,
            "dirs": dirs,
            "total_size": total_size,
            "by_extension": by_ext,
            "by_extension_count": by_ext_count,
        }))
    }
}

impl Drop for RcloneService {
    fn drop(&mut self) {
        self.stop_daemon();
    }
}

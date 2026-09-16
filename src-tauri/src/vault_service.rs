use crate::audit;
use crate::crypto::derive_keys;
use crate::rclone_service::{EngineStatus, RcloneItem, RcloneService, SearchHit, TransferProgress};
use crate::stream_server::MediaStreamServer;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const AUTH_FILE_NAME: &str = ".vault_auth";
const AUTH_FILE_CONTENT: &str = "SAFE_VAULT_AUTHORIZED_2026";

/// 内部管控文件/目录：仅在保险箱根层下过滤，不在主文件浏览中显示
fn is_internal_control(path_or_name: &str) -> bool {
    let clean = path_or_name.replace('\\', "/").trim_matches('/').to_string();
    if clean.is_empty() {
        return false;
    }
    // 根目录下的管控文件/目录或 .recycle 整个子树
    matches!(
        clean.as_str(),
        ".vault_auth" | ".recycle" | ".manifest" | "audit.log" | ".audit.log"
    ) || clean.starts_with(".recycle/")
}

/// 由保险箱路径派生的稳定短命名空间（仅用于生成互探文件名，非安全凭据）。
/// 不同保险箱使用不同探针名，避免同一固定名 interop_probe.txt 在不同保险箱间互相干扰。
fn vault_namespace(id: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    id.hash(&mut h);
    h.finish()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct VaultStatus {
    pub is_unlocked: bool,
    pub vault_path: String,
    pub username: String,
}

/// 供外部 rclone 双向互通的等价 crypt 配置
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct InteropConfig {
    pub snippet: String,
    pub probe_file: String,
    pub instructions: String,
    pub vault_path: String,
}

/// 互通一键自测结果
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct InteropSelfVerify {
    /// 写入的明文内容（模拟外部 rclone 通过 [interop_crypt] 配置写入）
    pub written: String,
    /// 本保险箱解密读回的内容
    pub read_back: String,
    /// 内容是否完全一致
    pub ok: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VaultDirKind {
    /// 目录不存在
    NotExist,
    /// 目录为空（或仅有系统隐藏文件），尚未初始化
    Empty,
    /// 检测到已有保险箱密文特征或已初始化
    InitializedVault,
    /// 目录非空且看起来是普通文件目录
    NonVaultNotEmpty,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultDirInspection {
    pub kind: VaultDirKind,
    pub path: String,
    pub file_count: usize,
    pub message: String,
}

pub struct VaultService {
    rclone: Arc<RcloneService>,
    status: Mutex<VaultStatus>,
    // 派生后的 rclone crypt 密钥材料（key_hex, salt_hex），仅驻留内存，锁定即清除
    crypt_keys: Mutex<Option<(String, String)>>,
    // 本地流媒体微服务句柄（仅在解锁状态下运行，锁定即物理销毁）
    stream_server: Mutex<Option<MediaStreamServer>>,
}

impl VaultService {
    pub fn new() -> Self {
        Self {
            rclone: Arc::new(RcloneService::new()),
            status: Mutex::new(VaultStatus {
                is_unlocked: false,
                vault_path: String::new(),
                username: String::new(),
            }),
            crypt_keys: Mutex::new(None),
            stream_server: Mutex::new(None),
        }
    }

    pub fn set_app_handle(&self, handle: tauri::AppHandle) {
        self.rclone.set_app_handle(handle);
    }

    pub fn get_status(&self) -> VaultStatus {
        self.status.lock().unwrap().clone()
    }

    fn audit(&self, action: &str, detail: &str) {
        let username = {
            let st = self.status.lock().unwrap();
            st.username.clone()
        };
        let line = audit::new_line(&username, action, detail);
        // 日志以加密文件形式保存在保险箱内（remote: audit.log）
        let existing = self.rclone.read_text_file("audit.log").unwrap_or_default();
        let updated = audit::append_line(&existing, &line);
        let _ = self.rclone.write_text_file("audit.log", &updated);
    }

    pub fn get_audit_log(&self, limit: usize) -> Vec<audit::AuditEntry> {
        if !self.status.lock().unwrap().is_unlocked {
            return Vec::new();
        }
        let content = self.rclone.read_text_file("audit.log").unwrap_or_default();
        audit::read(&content, limit.max(1))
    }

    pub fn get_engine_status(&self) -> EngineStatus {
        self.rclone.get_engine_status()
    }

    pub fn get_transfer_progress(&self) -> TransferProgress {
        self.rclone.get_progress()
    }

    pub fn inspect_vault_dir(&self, vault_path: &str) -> VaultDirInspection {
        let p = Path::new(vault_path);
        if !p.exists() {
            return VaultDirInspection {
                kind: VaultDirKind::NotExist,
                path: vault_path.to_string(),
                file_count: 0,
                message: "目录路径不存在".to_string(),
            };
        }
        if !p.is_dir() {
            return VaultDirInspection {
                kind: VaultDirKind::NotExist,
                path: vault_path.to_string(),
                file_count: 0,
                message: "指定路径不是有效文件夹".to_string(),
            };
        }

        let entries = match fs::read_dir(p) {
            Ok(e) => e.flatten().collect::<Vec<_>>(),
            Err(err) => {
                return VaultDirInspection {
                    kind: VaultDirKind::NotExist,
                    path: vault_path.to_string(),
                    file_count: 0,
                    message: format!("无法读取目录: {}", err),
                };
            }
        };

        // 过滤掉系统忽略文件（如 Desktop.ini, Thumbs.db, .DS_Store）及明文审计日志
        let user_entries: Vec<_> = entries
            .into_iter()
            .filter(|e| {
                let name = e.file_name().to_string_lossy().to_lowercase();
                name != "desktop.ini" && name != "thumbs.db" && name != ".ds_store" && name != ".audit.log"
            })
            .collect();

        let count = user_entries.len();
        if count == 0 {
            return VaultDirInspection {
                kind: VaultDirKind::Empty,
                path: vault_path.to_string(),
                file_count: 0,
                message: "目录为空，尚未初始化为保险箱".to_string(),
            };
        }

        // 检查是否存在 rclone crypt standard 密文特征：小写 base32 变体（仅 0-9 与 a-v，无大写/下划线/连字符/点后缀）。
        // 真实保险箱在 standard 模式下每个可见条目均为纯密文字符集。为避免被恰好混入的少数
        // 普通文件（如 README.txt）误判，采用 ≥80% 且至少 1 个相符的启发式：
        // 密文特征条目数 matches 满足 matches*5 >= 总数*4 即判定为保险箱。
        let total = user_entries.len();
        let crypto_like = user_entries
            .iter()
            .filter(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                name.len() >= 10
                    && name.chars().all(|c| c.is_ascii_digit() || (c >= 'a' && c <= 'v'))
            })
            .count();
        let looks_like_vault = crypto_like > 0 && crypto_like * 5 >= total * 4;

        if looks_like_vault {
            VaultDirInspection {
                kind: VaultDirKind::InitializedVault,
                path: vault_path.to_string(),
                file_count: count,
                message: "检测到已有保险箱数据".to_string(),
            }
        } else {
            VaultDirInspection {
                kind: VaultDirKind::NonVaultNotEmpty,
                path: vault_path.to_string(),
                file_count: count,
                message: "目录非空且未检测到保险箱特征（包含普通文件）".to_string(),
            }
        }
    }

    pub fn check_initialized(&self, vault_path: &str) -> bool {
        let inspection = self.inspect_vault_dir(vault_path);
        inspection.kind == VaultDirKind::InitializedVault
    }

    pub fn init_vault(&self, vault_path: &str, username: &str, master_pass: &str, force: bool) -> Result<VaultStatus, String> {
        // 拒绝在含普通文件的目录上建库：避免明文文件与加密数据混杂在磁盘上掩盖安全风险
        let inspection = self.inspect_vault_dir(vault_path);
        if inspection.kind == VaultDirKind::NonVaultNotEmpty {
            return Err(format!(
                "目录“{}”包含 {} 个普通文件。为避免明文文件与加密数据混杂在磁盘上，请先清空该目录或另选空目录作为保险箱。",
                vault_path, inspection.file_count
            ));
        }
        // 目录已是保险箱时必须显式确认覆盖：重初始化会用新密钥改写 .vault_auth，
        // 现有密文将全部变为不可读孤儿且物理残留不清理，属于不可逆的破坏性操作
        if inspection.kind == VaultDirKind::InitializedVault && !force {
            return Err(format!(
                "目录“{}”已经是保险箱，覆盖创建将导致原有加密数据永久不可读。如确需重置，请先备份旧保险箱，再确认“仍要覆盖创建”后重试。",
                vault_path
            ));
        }

        // 密码强度后端兜底（与前端策略一致）：拒绝空/纯空白/过短/单类字符密码，
        // 防止经 derive_keys 裁剪后的空白密码派生空密钥导致保险箱可被离线爆破。
        let pass = master_pass.trim();
        let has_letter = pass.chars().any(|c| c.is_ascii_alphabetic());
        let has_digit = pass.chars().any(|c| c.is_ascii_digit());
        if pass.len() < 8 || !has_letter || !has_digit {
            return Err("密码强度不足：至少 8 位，且需同时包含字母与数字".into());
        }

        let p = Path::new(vault_path);
        if !p.exists() {
            fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }

        let (key, salt) = derive_keys(username, master_pass);
        let obs_key = self.rclone.obscure(&key)?;
        let obs_salt = self.rclone.obscure(&salt)?;

        self.rclone.start_daemon()?;
        self.rclone.mount_vault(vault_path, &obs_key, &obs_salt)?;

        // 写入鉴权标记文件（若写入失败立即回滚卸载，防止状态机残留半挂载假象）
        if let Err(err) = self.rclone.write_text_file(AUTH_FILE_NAME, AUTH_FILE_CONTENT) {
            self.rclone.unmount_vault();
            return Err(format!("初始化写入鉴权文件失败: {}", err));
        }

        *self.crypt_keys.lock().unwrap() = Some((key, salt));
        // 启动本地流媒体微服务（127.0.0.1 随机端口 + Token 鉴权）
        self.restart_stream_server();

        let mut st = self.status.lock().unwrap();
        st.is_unlocked = true;
        st.vault_path = vault_path.to_string();
        st.username = username.to_string();

        let st_clone = st.clone();
        drop(st);
        self.audit("vault_init", &format!("创建并解锁保险箱: {}", vault_path));
        Ok(st_clone)
    }

    pub fn unlock_vault(&self, vault_path: &str, username: &str, master_pass: &str) -> Result<VaultStatus, String> {
        if !Path::new(vault_path).exists() {
            return Err("保险箱目录不存在".into());
        }

        let (key, salt) = derive_keys(username, master_pass);
        let obs_key = self.rclone.obscure(&key)?;
        let obs_salt = self.rclone.obscure(&salt)?;

        self.rclone.start_daemon()?;
        self.rclone.mount_vault(vault_path, &obs_key, &obs_salt)?;

        // 校验读取
        match self.rclone.read_text_file(AUTH_FILE_NAME) {
            Ok(content) if content.trim() == AUTH_FILE_CONTENT => {
                *self.crypt_keys.lock().unwrap() = Some((key, salt));
                // 启动本地流媒体微服务
                self.restart_stream_server();

                let mut st = self.status.lock().unwrap();
                st.is_unlocked = true;
                st.vault_path = vault_path.to_string();
                st.username = username.to_string();
                let st_clone = st.clone();
                drop(st);
                self.audit("unlock", &format!("解锁成功 (用户: {})", username));
                Ok(st_clone)
            }
            _ => {
                // 解锁失败：密码错误，无法通过加密通道写入。
                // 失败审计仅驻留内存（锁定后 get_audit_log 返回空），绝不写盘明文日志，
                // 避免用户账号名等信息以明文残留磁盘。
                *self.crypt_keys.lock().unwrap() = None;
                self.stop_stream_server();
                self.rclone.unmount_vault();
                Err("账号或主密码错误，无法解密该保险箱！".into())
            }
        }
    }

    pub fn lock_vault(&self) -> VaultStatus {
        self.audit("lock", "锁定保险箱");
        *self.crypt_keys.lock().unwrap() = None;
        self.stop_stream_server();
        self.rclone.unmount_vault();
        let mut st = self.status.lock().unwrap();
        st.is_unlocked = false;
        st.vault_path.clear();
        st.username.clear();
        st.clone()
    }

    /// 导出与当前保险箱完全等价的标准 rclone crypt 配置段（供外部 rclone 双向互通）
    pub fn get_interop_config(&self) -> Result<InteropConfig, String> {
        let st = self.status.lock().unwrap();
        if !st.is_unlocked {
            return Err("保险箱尚未解锁".into());
        }
        let vault_path = st.vault_path.clone();
        drop(st);

        let keys = self.crypt_keys.lock().unwrap().clone().ok_or("加密密钥未加载，请重新解锁")?;
        let (key_hex, salt_hex) = keys;
        let obs_key = self.rclone.obscure(&key_hex)?;
        let obs_salt = self.rclone.obscure(&salt_hex)?;

        let norm = vault_path.replace('\\', "/").trim_end_matches('/').to_string();
        // 探针名带本保险箱派生的命名空间：既避免跨保险箱的固定名互探冲突，
        // 也保证与 self 自测（独立唯一名）彻底区分，防止自测清理误删用户外部探针。
        let probe_file = format!("interop_probe_{:08x}.txt", vault_namespace(&norm));

        let snippet = format!(
r#"[interop_crypt]
type = crypt
remote = :local,path={}
filename_encryption = standard
directory_name_encryption = true
password = {}
password2 = {}"#,
            norm, obs_key, obs_salt
        );

        let instructions = format!(
r#"1) 在外部 rclone 的 rclone.conf 中粘贴上面的 [interop_crypt] 段（remote 已指向本保险箱原始目录 {}）。
2) 外部侧执行： rclone copy 你的测试文件 interop_crypt:{} （例如 rclone copy test.txt interop_crypt:{} ）
3) 回到本应用点击「校验互通」，确认能解密读出外部写入的内容。

说明：此配置与当前保险箱等价（standard 文件名加密 + 目录名加密），同一账户密码在任何机器派生密钥一致；请妥善保管 password/password2，泄露即等同交出保险箱钥匙。"#,
            norm, probe_file, probe_file
        );

        Ok(InteropConfig {
            snippet,
            probe_file,
            instructions,
            vault_path: norm,
        })
    }

    /// 校验互通：读取外部 rclone 用导出配置写入保险箱的探针文件并解密返回其内容
    pub fn verify_interop(&self, probe: &str) -> Result<String, String> {
        let st = self.status.lock().unwrap();
        if !st.is_unlocked {
            return Err("保险箱尚未解锁".into());
        }
        drop(st);

        let name = probe.trim();
        if name.is_empty()
            || name == "."
            || name == ".."
            || name.contains('/')
            || name.contains('\\')
            || name.contains("..")
        {
            return Err("互探文件名为非法路径".into());
        }

        let items = self.rclone.list_files("")?;
        if !items.iter().any(|i| !i.is_dir && i.name == name) {
            return Err(format!(
                "未在保险箱根目录找到互探文件 {}。\n请先用外部 rclone 的 [interop_crypt] 配置把测试文件写入保险箱（rclone copy test.txt interop_crypt:{}），再重试校验。",
                name, name
            ));
        }
        let content = self.rclone.read_text_file(name)?;
        // 外部 rclone 可能刚刚写入新文件，失效搜索快照保证后续能检索到
        self.rclone.invalidate_search_index();
        self.rclone.invalidate_stream_cache();
        Ok(content)
    }

    /// 自检/互通验证辅助：以完全等价的标准 rclone crypt 独立配置把本地明文写入保险箱，模拟外部 rclone
    pub fn interop_write_plain(&self, local_plain: &str, name: &str) -> Result<(), String> {
        let st = self.status.lock().unwrap();
        let vault_path = st.vault_path.clone();
        if !st.is_unlocked || vault_path.is_empty() {
            return Err("保险箱尚未解锁".into());
        }
        drop(st);

        let keys = self.crypt_keys.lock().unwrap().clone().ok_or("加密密钥未加载")?;
        let (key_hex, salt_hex) = keys;
        let obs_key = self.rclone.obscure(&key_hex)?;
        let obs_salt = self.rclone.obscure(&salt_hex)?;

        const EXT: &str = "interop_ext";
        let _ = self.rclone.delete_config(EXT);
        self.rclone.create_crypt_remote(EXT, &vault_path, &obs_key, &obs_salt)?;

        let p = Path::new(local_plain);
        let src_dir = p
            .parent()
            .map(|d| d.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        let src_file = p
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();

        let res = self.rclone.copyfile(&src_dir, &src_file, &format!("{}:", EXT), name);
        let _ = self.rclone.delete_config(EXT);
        if res.is_ok() {
            // 外部写入即保险箱数据变化，失效搜索快照避免旧结果残留
            self.rclone.invalidate_search_index();
            // 同名文件可能已被外部覆盖，流媒体解密缓存一并失效，防止读到旧明文
            self.rclone.invalidate_stream_cache();
        }
        res
    }

    /// 一键互通自测：用与导出配置等价的标准 crypt 独立后端（与外部 rclone 完全相同的
    /// 写入路径）把探针内容写入保险箱，再立即通过本保险箱解密读回，以此证明
    /// [interop_crypt] 配置与本保险箱真正互通；自测结束自动清理探针文件，不污染清单。
    pub fn interop_self_verify(&self) -> Result<InteropSelfVerify, String> {
        let st = self.status.lock().unwrap();
        if !st.is_unlocked {
            return Err("保险箱尚未解锁".into());
        }
        drop(st);

        // 生成唯一明文内容，确保每轮自测都代表一次全新写入而非旧缓存
        let ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let content = format!("INTEROP_SELF_CHECK_{}", ns);

        let tmp_dir = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("SafeVault")
            .join("tmp");
        let _ = std::fs::create_dir_all(&tmp_dir);
        let tmp_file = tmp_dir.join(format!("_interop_self_{}_{}.txt", std::process::id(), ns));
        std::fs::write(&tmp_file, &content).map_err(|e| format!("写入自测明文失败: {}", e))?;

        // 本自测使用独立唯一探针名（含 PID + 时间戳），与外部互通使用的
        // interop_probe_<ns>.txt 完全区分，绝不会因自测清理误删外部探针。
        let probe = format!("interop_self_{}_{}.txt", std::process::id(), ns);
        let result = (|| {
            self.interop_write_plain(&tmp_file.to_string_lossy(), &probe)?;
            let read_back = self.verify_interop(&probe)?;
            let ok = read_back.trim() == content;
            // 仅当读回内容与本轮自测写入内容完全一致（证明该条目确为本自测创建）才清理，
            // 避免误删恰好同名的外部真实条目。
            if ok {
                let _ = self.rclone.delete_item(&probe, false);
            }
            Ok(InteropSelfVerify {
                written: content.clone(),
                read_back: read_back.trim().to_string(),
                ok,
            })
        })();
        let _ = std::fs::remove_file(&tmp_file);
        result
    }

    pub fn list_files(&self, remote_dir: &str) -> Result<Vec<RcloneItem>, String> {
        let is_unlocked = self.status.lock().unwrap().is_unlocked;
        if !is_unlocked {
            return Err("保险箱尚未解锁".into());
        }

        let items = self.rclone.list_files(remote_dir)?;
        Ok(items.into_iter().filter(|i| !is_internal_control(&i.name)).collect())
    }

    pub fn search_files(&self, keyword: &str, limit: usize) -> Result<Vec<SearchHit>, String> {
        let is_unlocked = self.status.lock().unwrap().is_unlocked;
        if !is_unlocked {
            return Err("保险箱尚未解锁".into());
        }

        Ok(self
            .rclone
            .search_files(keyword, limit)?
            .into_iter()
            .filter(|h| !is_internal_control(&h.name) && !h.path.starts_with(".recycle/"))
            .collect())
    }

    pub fn import_paths(&self, source_paths: Vec<String>, target_dir: &str) -> Result<Vec<String>, String> {
        let (is_unlocked, current_vault_path) = {
            let st = self.status.lock().unwrap();
            (st.is_unlocked, st.vault_path.clone())
        };
        if !is_unlocked {
            return Err("保险箱尚未解锁".into());
        }

        // 防自我递归导入：禁止将保险箱自身的密文存放目录或其内部文件拖拽导入自身
        let norm_vault = Path::new(&current_vault_path)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(&current_vault_path));
        for src in &source_paths {
            let src_p = Path::new(src);
            let norm_src = src_p.canonicalize().unwrap_or_else(|_| PathBuf::from(src));
            if norm_src.starts_with(&norm_vault) || norm_vault.starts_with(&norm_src) {
                return Err("禁止将当前保险箱密文目录或其内部文件导入保险箱自身".into());
            }
        }

        // 原子预检：统一校验全部源路径的目标名称与目标冲突，任一失败即整体拒绝，
        // 避免批量导入时前面的条目已写入、后面的条目失败导致的半途状态。
        self.rclone.validate_imports(&source_paths, target_dir)?;

        let total = source_paths.len();
        let mut imported = Vec::new();
        for (idx, src) in source_paths.into_iter().enumerate() {
            let batch_info = if total > 1 { Some((idx, total)) } else { None };
            if let Err(err) = self.rclone.import_path_with_batch(&src, target_dir, batch_info) {
                if !imported.is_empty() {
                    self.audit("import", &format!("部分导入 {}/{} 个项目到: /{}", imported.len(), total, target_dir));
                }
                return Err(format!(
                    "批量导入第 {}/{} 个项目 \"{}\" 失败: {}（此前已成功导入 {} 项）",
                    idx + 1,
                    total,
                    src,
                    err,
                    imported.len()
                ));
            }
            imported.push(src);
        }
        self.audit("import", &format!("导入 {} 个项目到: /{}", imported.len(), target_dir));
        Ok(imported)
    }

    pub fn export_item(&self, vault_path: &str, is_dir: bool, target_dir: &str) -> Result<(), String> {
        let is_unlocked = self.status.lock().unwrap().is_unlocked;
        if !is_unlocked {
            return Err("保险箱尚未解锁".into());
        }
        self.rclone.export_item(vault_path, is_dir, target_dir)?;
        self.audit("export", &format!("解密导出: {} -> {}", vault_path, target_dir));
        Ok(())
    }

    pub fn export_items(&self, items: &[(String, bool)], target_dir: &str) -> Result<(), String> {
        let is_unlocked = self.status.lock().unwrap().is_unlocked;
        if !is_unlocked {
            return Err("保险箱尚未解锁".into());
        }
        self.rclone.export_items(items, target_dir)?;
        let names: Vec<String> = items.iter().map(|(p, _)| p.clone()).collect();
        self.audit("export", &format!("批量解密导出 {} 个条目 -> {}: {}", items.len(), target_dir, names.join(", ")));
        Ok(())
    }

    pub fn create_folder(&self, folder_path: &str) -> Result<(), String> {
        let is_unlocked = self.status.lock().unwrap().is_unlocked;
        if !is_unlocked {
            return Err("保险箱尚未解锁".into());
        }
        self.rclone.create_folder(folder_path)?;
        self.audit("create_dir", &format!("新建目录: /{}", folder_path));
        Ok(())
    }

    pub fn delete_item(&self, vault_path: &str, is_dir: bool) -> Result<(), String> {
        let is_unlocked = self.status.lock().unwrap().is_unlocked;
        if !is_unlocked {
            return Err("保险箱尚未解锁".into());
        }
        self.rclone.delete_item(vault_path, is_dir)?;
        self.audit("delete", &format!("永久删除: /{}", vault_path));
        Ok(())
    }

    // ===== 回收站/软删除 =====
    pub fn recycle_item(&self, vault_path: &str, is_dir: bool) -> Result<String, String> {
        let is_unlocked = self.status.lock().unwrap().is_unlocked;
        if !is_unlocked {
            return Err("保险箱尚未解锁".into());
        }
        let id = self.rclone.recycle_item(vault_path, is_dir)?;
        self.audit("recycle", &format!("移入回收站: /{} (条目 {})", vault_path, id));
        Ok(id)
    }

    pub fn list_recycle(&self) -> Result<Vec<serde_json::Value>, String> {
        let is_unlocked = self.status.lock().unwrap().is_unlocked;
        if !is_unlocked {
            return Err("保险箱尚未解锁".into());
        }
        self.rclone.list_recycle()
    }

    pub fn restore_item(&self, id: &str) -> Result<bool, String> {
        let is_unlocked = self.status.lock().unwrap().is_unlocked;
        if !is_unlocked {
            return Err("保险箱尚未解锁".into());
        }
        self.rclone.restore_item(id)?;
        self.audit("restore", &format!("从回收站恢复条目: {}", id));
        Ok(true)
    }

    pub fn empty_recycle(&self) -> Result<bool, String> {
        let is_unlocked = self.status.lock().unwrap().is_unlocked;
        if !is_unlocked {
            return Err("保险箱尚未解锁".into());
        }
        self.rclone.empty_recycle()?;
        self.audit("empty_recycle", "清空回收站");
        Ok(true)
    }

    // ===== 文件操作扩展 =====
    pub fn rename_item(&self, vault_path: &str, new_name: &str) -> Result<bool, String> {
        let is_unlocked = self.status.lock().unwrap().is_unlocked;
        if !is_unlocked {
            return Err("保险箱尚未解锁".into());
        }
        self.rclone.rename_item(vault_path, new_name)?;
        self.audit("rename", &format!("重命名: /{} -> {}", vault_path, new_name));
        Ok(true)
    }

    pub fn move_items(&self, paths: Vec<String>, dest_dir: &str) -> Result<usize, String> {
        let is_unlocked = self.status.lock().unwrap().is_unlocked;
        if !is_unlocked {
            return Err("保险箱尚未解锁".into());
        }
        let n = self.rclone.move_items(paths.clone(), dest_dir)?;
        self.audit("move", &format!("移动 {} 个项目到 /{}", paths.len(), dest_dir));
        Ok(n)
    }

    pub fn copy_items(&self, paths: Vec<String>, dest_dir: &str) -> Result<usize, String> {
        let is_unlocked = self.status.lock().unwrap().is_unlocked;
        if !is_unlocked {
            return Err("保险箱尚未解锁".into());
        }
        let n = self.rclone.copy_items(paths.clone(), dest_dir)?;
        self.audit("copy", &format!("复制 {} 个项目到 /{}", paths.len(), dest_dir));
        Ok(n)
    }

    // ===== 数据完整性自检 =====
    pub fn build_manifest(&self) -> Result<usize, String> {
        let is_unlocked = self.status.lock().unwrap().is_unlocked;
        if !is_unlocked {
            return Err("保险箱尚未解锁".into());
        }
        let n = self.rclone.build_manifest()?;
        self.audit("manifest_build", &format!("构建完整性清单，共 {} 个文件", n));
        Ok(n)
    }

    pub fn integrity_check(&self, path: &str, recursive: bool) -> Result<serde_json::Value, String> {
        let is_unlocked = self.status.lock().unwrap().is_unlocked;
        if !is_unlocked {
            return Err("保险箱尚未解锁".into());
        }
        self.rclone.integrity_check(path, recursive)
    }

    /// 列出明文条目的密文文件名（内部管控文件已过滤）——供 E2E 测试与诊断使用
    pub fn list_raw_mapping(&self) -> Result<Vec<serde_json::Value>, String> {
        let is_unlocked = self.status.lock().unwrap().is_unlocked;
        if !is_unlocked {
            return Err("保险箱尚未解锁".into());
        }
        self.rclone.list_raw_mapping()
    }

    // ===== 统计面板 =====
    pub fn get_stats(&self) -> Result<serde_json::Value, String> {
        let is_unlocked = self.status.lock().unwrap().is_unlocked;
        if !is_unlocked {
            return Err("保险箱尚未解锁".into());
        }
        self.rclone.get_stats()
    }

    /// 读取文件二进制用于安全预览（带体积熔断限制）
    pub fn read_file_preview(&self, vault_item_path: &str, max_bytes: usize) -> Result<Vec<u8>, String> {
        let is_unlocked = self.status.lock().unwrap().is_unlocked;
        if !is_unlocked {
            return Err("保险箱尚未解锁".into());
        }
        if is_internal_control(vault_item_path) {
            return Err("内部管控文件不可预览".into());
        }
        // 体积熔断：无论调用方传多大，预览最多只缓存 200MB 明文到内存，
        // 超过阈值直接拒绝而非截断返回，杜绝误读大文件导致内存暴涨
        const PREVIEW_HARD_CAP: usize = 200 * 1024 * 1024;
        if max_bytes > PREVIEW_HARD_CAP {
            return Err(format!(
                "预览体积已超过 {} MB 熔断上限，请使用流媒体模式播放",
                PREVIEW_HARD_CAP / 1024 / 1024
            ));
        }
        self.rclone.read_file_range_bytes(vault_item_path, 0, max_bytes as u64)
    }

    /// 分块流式读取（用于视频、音频 Range 头分片播放）
    pub fn read_file_stream_range(&self, vault_item_path: &str, start: u64, length: u64) -> Result<Vec<u8>, String> {
        let is_unlocked = self.status.lock().unwrap().is_unlocked;
        if !is_unlocked {
            return Err("保险箱尚未解锁".into());
        }
        if is_internal_control(vault_item_path) {
            return Err("内部管控文件不可预览".into());
        }
        self.rclone.read_file_range_bytes(vault_item_path, start, length)
    }

    /// 检查并获取指定文件的元信息（大小、MIME 等）
    pub fn stat_item(&self, remote_path: &str) -> Result<RcloneItem, String> {
        let is_unlocked = self.status.lock().unwrap().is_unlocked;
        if !is_unlocked {
            return Err("保险箱尚未解锁".into());
        }
        let clean = remote_path.trim_start_matches('/').to_string();
        let parent = match clean.rfind('/') {
            Some(idx) => &clean[..idx],
            None => "",
        };
        let items = self.rclone.list_files(parent)?;
        let name = Path::new(&clean)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        items
            .into_iter()
            .find(|i| i.name == name)
            .ok_or_else(|| format!("未找到文件: {}", remote_path))
    }

    /// 获取底层 Rclone 服务的只读引用（用于自定义协议流式处理）
    pub fn rclone_ref(&self) -> &RcloneService {
        &self.rclone
    }

    /// 重启/启动本地流媒体微服务
    fn restart_stream_server(&self) {
        let _ = self.restart_stream_server_with_lan(false);
    }

    /// 根据局域网开关启动或切换流媒体微服务 (allow_lan: true 监听 0.0.0.0, false 仅监听 127.0.0.1)
    pub fn restart_stream_server_with_lan(&self, allow_lan: bool) -> Result<(), String> {
        self.stop_stream_server();
        // 清空解密缓存：流媒体服务重启意味着可能切换了加密通道或锁定/解锁
        self.rclone.invalidate_stream_cache();
        let rclone_size = self.rclone.clone();
        let rclone_read = self.rclone.clone();
        match MediaStreamServer::start(
            allow_lan,
            move |path: &str| -> Result<u64, String> { rclone_size.stat_stream_total(path) },
            move |path: &str, start: u64, len: u64| -> Result<crate::stream_server::StreamReadResult, String> {
                rclone_read.read_file_stream_range(path, start, len)
            },
        ) {
            Ok(server) => {
                *self.stream_server.lock().unwrap() = Some(server);
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// 停止并销毁本地流媒体微服务
    fn stop_stream_server(&self) {
        if let Some(server) = self.stream_server.lock().unwrap().take() {
            server.stop();
        }
    }

    /// 获取文件的本地安全流媒体 URL（带 127.0.0.1 或 0.0.0.0 + Token 鉴权）
    pub fn get_stream_url(&self, remote_path: &str, use_lan_ip: bool) -> Result<String, String> {
        let is_unlocked = self.status.lock().unwrap().is_unlocked;
        if !is_unlocked {
            return Err("保险箱尚未解锁".into());
        }
        let lock = self.stream_server.lock().unwrap();
        match lock.as_ref() {
            Some(srv) => {
                if use_lan_ip {
                    Ok(srv.get_lan_stream_url(remote_path))
                } else {
                    Ok(srv.get_stream_url(remote_path))
                }
            }
            None => Err("流媒体服务未就绪".into()),
        }
    }

    /// 查询当前流媒体服务状态及局域网 IP
    pub fn get_stream_server_status(&self) -> serde_json::Value {
        let is_unlocked = self.status.lock().unwrap().is_unlocked;
        let lock = self.stream_server.lock().unwrap();
        let (running, port, is_lan) = match lock.as_ref() {
            Some(srv) => (true, srv.port(), srv.is_lan_enabled()),
            None => (false, 0, false),
        };
        let lan_ip = crate::stream_server::get_local_lan_ip();
        serde_json::json!({
            "is_unlocked": is_unlocked,
            "running": running,
            "port": port,
            "is_lan": is_lan,
            "lan_ip": lan_ip,
        })
    }

    /// 取消当前正在进行的传输任务（导入/导出）
    pub fn cancel_transfer(&self) -> Result<(), String> {
        self.rclone.cancel_transfer()
    }

    pub fn stop(&self) {
        self.stop_stream_server();
        // 先取消并等待在跑的传输任务退出后再终结 rclone Go 运行时，
        // 避免轮询线程在 finalize 后仍调用已终结的运行时（未定义行为/退出崩溃）。
        self.rclone.request_cancel_and_wait();
        self.rclone.stop_daemon();
        crate::librclone::finalize();
    }
}

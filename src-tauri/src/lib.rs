pub mod audit;
pub mod crypto;
pub mod librclone;
pub mod rclone_service;
pub mod stream_server;
pub mod vault_service;

use rclone_service::{EngineStatus, RcloneItem, SearchHit, TransferProgress};
use std::process::Command;
use std::sync::Arc;
use tauri::State;
use vault_service::{InteropConfig, InteropSelfVerify, VaultDirInspection, VaultService, VaultStatus};

type SharedVault = Arc<VaultService>;

#[tauri::command]
async fn get_engine_status(state: State<'_, SharedVault>) -> Result<EngineStatus, String> {
    let service = state.inner().clone();
    tokio::task::spawn_blocking(move || service.get_engine_status())
        .await
        .map_err(|e| format!("获取引擎状态失败: {}", e))
}

#[tauri::command]
fn get_transfer_progress(state: State<'_, SharedVault>) -> TransferProgress {
    state.get_transfer_progress()
}

#[tauri::command]
fn inspect_vault_dir(state: State<'_, SharedVault>, vault_path: String) -> VaultDirInspection {
    state.inspect_vault_dir(&vault_path)
}

#[tauri::command]
fn check_initialized(state: State<'_, SharedVault>, vault_path: String) -> bool {
    state.check_initialized(&vault_path)
}

#[tauri::command]
fn get_status(state: State<'_, SharedVault>) -> VaultStatus {
    state.get_status()
}

#[tauri::command]
async fn init_vault(
    state: State<'_, SharedVault>,
    vault_path: String,
    username: String,
    password: String,
    force: bool,
) -> Result<VaultStatus, String> {
    let service = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        service.init_vault(&vault_path, &username, &password, force)
    })
    .await
    .map_err(|e| format!("执行初始化任务失败: {}", e))?
}

#[tauri::command]
async fn unlock_vault(
    state: State<'_, SharedVault>,
    vault_path: String,
    username: String,
    password: String,
) -> Result<VaultStatus, String> {
    let service = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        service.unlock_vault(&vault_path, &username, &password)
    })
    .await
    .map_err(|e| format!("执行解锁任务失败: {}", e))?
}

#[tauri::command]
async fn lock_vault(state: State<'_, SharedVault>) -> Result<VaultStatus, String> {
    let service = state.inner().clone();
    tokio::task::spawn_blocking(move || service.lock_vault())
        .await
        .map_err(|e| format!("执行锁定任务失败: {}", e))
}

#[tauri::command]
async fn list_files(
    state: State<'_, SharedVault>,
    dir_path: String,
) -> Result<Vec<RcloneItem>, String> {
    let service = state.inner().clone();
    tokio::task::spawn_blocking(move || service.list_files(&dir_path))
        .await
        .map_err(|e| format!("读取文件列表任务失败: {}", e))?
}

#[tauri::command]
async fn import_paths(
    state: State<'_, SharedVault>,
    source_paths: Vec<String>,
    target_dir: String,
) -> Result<Vec<String>, String> {
    let service = state.inner().clone();
    tokio::task::spawn_blocking(move || service.import_paths(source_paths, &target_dir))
        .await
        .map_err(|e| format!("执行导入任务失败: {}", e))?
}

#[tauri::command]
async fn get_interop_config(state: State<'_, SharedVault>) -> Result<InteropConfig, String> {
    let service = state.inner().clone();
    tokio::task::spawn_blocking(move || service.get_interop_config())
        .await
        .map_err(|e| format!("生成互通配置失败: {}", e))?
}

#[tauri::command]
async fn verify_interop(state: State<'_, SharedVault>, probe: String) -> Result<String, String> {
    let service = state.inner().clone();
    tokio::task::spawn_blocking(move || service.verify_interop(&probe))
        .await
        .map_err(|e| format!("校验互通失败: {}", e))?
}

#[tauri::command]
async fn interop_self_verify(state: State<'_, SharedVault>) -> Result<InteropSelfVerify, String> {
    let service = state.inner().clone();
    tokio::task::spawn_blocking(move || service.interop_self_verify())
        .await
        .map_err(|e| format!("互通自测失败: {}", e))?
}

#[tauri::command]
async fn export_item(
    state: State<'_, SharedVault>,
    vault_item_path: String,
    is_dir: bool,
    target_dir: String,
) -> Result<bool, String> {
    let service = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        service.export_item(&vault_item_path, is_dir, &target_dir)?;
        Ok::<bool, String>(true)
    })
    .await
    .map_err(|e| format!("执行导出任务失败: {}", e))?
}

#[tauri::command]
async fn export_items(
    state: State<'_, SharedVault>,
    vault_item_paths: Vec<String>,
    is_dirs: Vec<bool>,
    target_dir: String,
) -> Result<bool, String> {
    let service = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        let items: Vec<(String, bool)> = vault_item_paths.into_iter().zip(is_dirs.into_iter()).collect();
        service.export_items(&items, &target_dir)?;
        Ok::<bool, String>(true)
    })
    .await
    .map_err(|e| format!("执行批量导出任务失败: {}", e))?
}

#[tauri::command]
async fn create_folder(
    state: State<'_, SharedVault>,
    folder_path: String,
) -> Result<bool, String> {
    let service = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        service.create_folder(&folder_path)?;
        Ok::<bool, String>(true)
    })
    .await
    .map_err(|e| format!("创建目录任务失败: {}", e))?
}

#[tauri::command]
async fn delete_item(
    state: State<'_, SharedVault>,
    vault_item_path: String,
    is_dir: bool,
) -> Result<bool, String> {
    let service = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        service.delete_item(&vault_item_path, is_dir)?;
        Ok::<bool, String>(true)
    })
    .await
    .map_err(|e| format!("删除任务失败: {}", e))?
}

#[tauri::command]
async fn search_files(
    state: State<'_, SharedVault>,
    keyword: String,
    limit: Option<usize>,
) -> Result<Vec<SearchHit>, String> {
    let service = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        service.search_files(&keyword, limit.unwrap_or(200))
    })
    .await
    .map_err(|e| format!("搜索任务失败: {}", e))?
}

#[tauri::command]
fn show_item_in_folder(full_path: String) -> bool {
    // Windows 专属 Explorer 高亮定位
    #[cfg(target_os = "windows")]
    {
        let win_path = full_path.replace('/', "\\");
        let path_obj = std::path::Path::new(&win_path);
        if !path_obj.exists() {
            return false;
        }
        // /select, 必须与路径紧邻为单一参数，拆分为两段会导致 explorer 忽略 /select, 而仅打开默认目录
        let _ = Command::new("explorer.exe")
            .arg(format!("/select,{}", win_path))
            .spawn();
        true
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = full_path;
        false
    }
}

// ===== 回收站/软删除 =====
#[tauri::command]
async fn recycle_item(
    state: State<'_, SharedVault>,
    vault_item_path: String,
    is_dir: bool,
) -> Result<String, String> {
    let service = state.inner().clone();
    tokio::task::spawn_blocking(move || service.recycle_item(&vault_item_path, is_dir))
        .await
        .map_err(|e| format!("移入回收站失败: {}", e))?
}

#[tauri::command]
async fn list_recycle(state: State<'_, SharedVault>) -> Result<Vec<serde_json::Value>, String> {
    let service = state.inner().clone();
    tokio::task::spawn_blocking(move || service.list_recycle())
        .await
        .map_err(|e| format!("读取回收站失败: {}", e))?
}

#[tauri::command]
async fn restore_item(state: State<'_, SharedVault>, id: String) -> Result<bool, String> {
    let service = state.inner().clone();
    tokio::task::spawn_blocking(move || service.restore_item(&id))
        .await
        .map_err(|e| format!("恢复失败: {}", e))?
}

#[tauri::command]
async fn empty_recycle(state: State<'_, SharedVault>) -> Result<bool, String> {
    let service = state.inner().clone();
    tokio::task::spawn_blocking(move || service.empty_recycle())
        .await
        .map_err(|e| format!("清空回收站失败: {}", e))?
}

// ===== 文件操作扩展 =====
#[tauri::command]
async fn rename_item(
    state: State<'_, SharedVault>,
    vault_item_path: String,
    new_name: String,
) -> Result<bool, String> {
    let service = state.inner().clone();
    tokio::task::spawn_blocking(move || service.rename_item(&vault_item_path, &new_name))
        .await
        .map_err(|e| format!("重命名失败: {}", e))?
}

#[tauri::command]
async fn move_items(
    state: State<'_, SharedVault>,
    source_paths: Vec<String>,
    dest_dir: String,
) -> Result<usize, String> {
    let service = state.inner().clone();
    tokio::task::spawn_blocking(move || service.move_items(source_paths, &dest_dir))
        .await
        .map_err(|e| format!("移动失败: {}", e))?
}

#[tauri::command]
async fn copy_items(
    state: State<'_, SharedVault>,
    source_paths: Vec<String>,
    dest_dir: String,
) -> Result<usize, String> {
    let service = state.inner().clone();
    tokio::task::spawn_blocking(move || service.copy_items(source_paths, &dest_dir))
        .await
        .map_err(|e| format!("复制失败: {}", e))?
}

// ===== 数据完整性自检 =====
#[tauri::command]
async fn build_manifest(state: State<'_, SharedVault>) -> Result<usize, String> {
    let service = state.inner().clone();
    tokio::task::spawn_blocking(move || service.build_manifest())
        .await
        .map_err(|e| format!("构建清单失败: {}", e))?
}

#[tauri::command]
async fn integrity_check(
    state: State<'_, SharedVault>,
    path: String,
    recursive: bool,
) -> Result<serde_json::Value, String> {
    let service = state.inner().clone();
    tokio::task::spawn_blocking(move || service.integrity_check(&path, recursive))
        .await
        .map_err(|e| format!("完整性自检失败: {}", e))?
}

// ===== 统计面板 / 审计日志 =====
#[tauri::command]
async fn get_stats(state: State<'_, SharedVault>) -> Result<serde_json::Value, String> {
    let service = state.inner().clone();
    tokio::task::spawn_blocking(move || service.get_stats())
        .await
        .map_err(|e| format!("读取统计失败: {}", e))?
}

#[tauri::command]
async fn get_audit_log(
    state: State<'_, SharedVault>,
    limit: Option<usize>,
) -> Result<Vec<audit::AuditEntry>, String> {
    let service = state.inner().clone();
    let limit = limit.unwrap_or(200);
    Ok(tokio::task::spawn_blocking(move || service.get_audit_log(limit))
        .await
        .map_err(|e| format!("读取审计日志任务失败: {}", e))?)
}

#[tauri::command]
async fn cancel_transfer(state: State<'_, SharedVault>) -> Result<(), String> {
    let service = state.inner().clone();
    tokio::task::spawn_blocking(move || service.cancel_transfer())
        .await
        .map_err(|e| format!("取消失败: {}", e))?
}

#[tauri::command]
async fn read_file_preview(
    state: State<'_, SharedVault>,
    vault_item_path: String,
    max_bytes: Option<usize>,
) -> Result<Vec<u8>, String> {
    let limit = max_bytes.unwrap_or(200 * 1024 * 1024); // 放宽至 200MB，支持绝大部分常见音频/视频/文档
    let service = state.inner().clone();
    tokio::task::spawn_blocking(move || service.read_file_preview(&vault_item_path, limit))
        .await
        .map_err(|e| format!("预览任务异常: {}", e))?
}

#[tauri::command]
async fn get_stream_url(
    state: State<'_, SharedVault>,
    vault_item_path: String,
    use_lan_ip: Option<bool>,
) -> Result<String, String> {
    let service = state.inner().clone();
    let lan = use_lan_ip.unwrap_or(false);
    tokio::task::spawn_blocking(move || service.get_stream_url(&vault_item_path, lan))
        .await
        .map_err(|e| format!("获取流媒体地址失败: {}", e))?
}

#[tauri::command]
async fn set_stream_lan_mode(
    state: State<'_, SharedVault>,
    allow_lan: bool,
) -> Result<serde_json::Value, String> {
    let service = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        service.restart_stream_server_with_lan(allow_lan)?;
        Ok::<serde_json::Value, String>(service.get_stream_server_status())
    })
    .await
    .map_err(|e| format!("切换流媒体模式失败: {}", e))?
}

#[tauri::command]
async fn get_stream_server_status(
    state: State<'_, SharedVault>,
) -> Result<serde_json::Value, String> {
    let service = state.inner().clone();
    Ok(tokio::task::spawn_blocking(move || service.get_stream_server_status())
        .await
        .map_err(|e| format!("查询流媒体状态失败: {}", e))?)
}

pub fn run() {
    let vault_service = Arc::new(VaultService::new());
    let vs_clone = vault_service.clone();
    let vs_setup = vault_service.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(vault_service)
        .setup(move |app| {
            vs_setup.set_app_handle(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_engine_status,
            get_transfer_progress,
            inspect_vault_dir,
            check_initialized,
            get_status,
            init_vault,
            unlock_vault,
            lock_vault,
            list_files,
            import_paths,
            export_item,
            export_items,
            create_folder,
            delete_item,
            search_files,
            show_item_in_folder,
            recycle_item,
            list_recycle,
            restore_item,
            empty_recycle,
            rename_item,
            move_items,
            copy_items,
            build_manifest,
            integrity_check,
            get_stats,
            get_audit_log,
            cancel_transfer,
            read_file_preview,
            get_stream_url,
            set_stream_lan_mode,
            get_stream_server_status,
            get_interop_config,
            verify_interop,
            interop_self_verify
        ])
        .build(tauri::generate_context!())
        .expect("运行 Tauri 应用程序时发生错误")
        .run(move |_app_handle, event| {
            if let tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit = event {
                vs_clone.stop();
            }
        });
}

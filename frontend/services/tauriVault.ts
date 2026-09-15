import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import type {
  AuditEntry,
  EngineStatus,
  IntegrityResult,
  InteropConfig,
  InteropSelfVerify,
  RecycleEntry,
  RcloneItem,
  SearchHit,
  TransferProgress,
  VaultDirInspection,
  VaultStats,
  VaultStatus
} from '@/types'

export const tauriVault = {
  async getEngineStatus(): Promise<EngineStatus> {
    return await invoke<EngineStatus>('get_engine_status')
  },

  async getTransferProgress(): Promise<TransferProgress> {
    return await invoke<TransferProgress>('get_transfer_progress')
  },
  async selectDirectory(): Promise<string | null> {
    const selected = await open({
      directory: true,
      multiple: false,
      title: '选择保险箱密文存放目录'
    })
    return typeof selected === 'string' ? selected : null
  },

  async selectExportDirectory(): Promise<string | null> {
    const selected = await open({
      directory: true,
      multiple: false,
      title: '选择解密导出的目标目录'
    })
    return typeof selected === 'string' ? selected : null
  },

  async selectImportFiles(): Promise<string[]> {
    const selected = await open({
      directory: false,
      multiple: true,
      title: '选择需要加密导入的文件'
    })
    if (!selected) return []
    return Array.isArray(selected) ? selected : [selected]
  },

  async selectImportFolder(): Promise<string | null> {
    const selected = await open({
      directory: true,
      multiple: false,
      title: '选择需要加密导入的文件夹'
    })
    return typeof selected === 'string' ? selected : null
  },

  async inspectVaultDir(vaultPath: string): Promise<VaultDirInspection> {
    return await invoke<VaultDirInspection>('inspect_vault_dir', { vaultPath })
  },

  async checkInitialized(vaultPath: string): Promise<boolean> {
    return await invoke<boolean>('check_initialized', { vaultPath })
  },

  async initVault(data: { vaultPath: string; username: string; password: string; force: boolean }): Promise<VaultStatus> {
    return await invoke<VaultStatus>('init_vault', data)
  },

  async unlockVault(data: { vaultPath: string; username: string; password: string }): Promise<VaultStatus> {
    return await invoke<VaultStatus>('unlock_vault', data)
  },

  async lockVault(): Promise<VaultStatus> {
    return await invoke<VaultStatus>('lock_vault')
  },

  async getStatus(): Promise<VaultStatus> {
    return await invoke<VaultStatus>('get_status')
  },

  async listFiles(dirPath: string): Promise<RcloneItem[]> {
    return await invoke<RcloneItem[]>('list_files', { dirPath })
  },

  async importPaths(data: { sourcePaths: string[]; targetDir: string }): Promise<string[]> {
    return await invoke<string[]>('import_paths', data)
  },

  async exportItem(data: { vaultItemPath: string; isDir: boolean; targetDir: string }): Promise<boolean> {
    return await invoke<boolean>('export_item', data)
  },

  async exportItems(data: { vaultItemPaths: string[]; isDirs: boolean[]; targetDir: string }): Promise<boolean> {
    return await invoke<boolean>('export_items', data)
  },

  async createFolder(folderPath: string): Promise<boolean> {
    return await invoke<boolean>('create_folder', { folderPath })
  },

  async deleteItem(data: { vaultItemPath: string; isDir: boolean }): Promise<boolean> {
    return await invoke<boolean>('delete_item', data)
  },

  async searchFiles(keyword: string, limit?: number): Promise<SearchHit[]> {
    return await invoke<SearchHit[]>('search_files', { keyword, limit })
  },

  async showItemInFolder(fullPath: string): Promise<boolean> {
    return await invoke<boolean>('show_item_in_folder', { fullPath })
  },

  // ===== 回收站 / 软删除 =====
  async recycleItem(data: { vaultItemPath: string; isDir: boolean }): Promise<string> {
    return await invoke<string>('recycle_item', data)
  },

  async listRecycle(): Promise<RecycleEntry[]> {
    return await invoke<RecycleEntry[]>('list_recycle')
  },

  async restoreItem(id: string): Promise<boolean> {
    return await invoke<boolean>('restore_item', { id })
  },

  async emptyRecycle(): Promise<boolean> {
    return await invoke<boolean>('empty_recycle')
  },

  // ===== 文件操作扩展 =====
  async renameItem(data: { vaultItemPath: string; newName: string }): Promise<boolean> {
    return await invoke<boolean>('rename_item', data)
  },

  async moveItems(data: { sourcePaths: string[]; destDir: string }): Promise<number> {
    return await invoke<number>('move_items', data)
  },

  async copyItems(data: { sourcePaths: string[]; destDir: string }): Promise<number> {
    return await invoke<number>('copy_items', data)
  },

  // ===== 数据完整性自检 =====
  async buildManifest(): Promise<number> {
    return await invoke<number>('build_manifest')
  },

  async integrityCheck(data: { path: string; recursive: boolean }): Promise<IntegrityResult> {
    return await invoke<IntegrityResult>('integrity_check', data)
  },

  // ===== 统计面板 / 审计日志 =====
  async getStats(): Promise<VaultStats> {
    return await invoke<VaultStats>('get_stats')
  },

  async getAuditLog(limit?: number): Promise<AuditEntry[]> {
    return await invoke<AuditEntry[]>('get_audit_log', { limit })
  },

  async cancelTransfer(): Promise<void> {
    return await invoke<void>('cancel_transfer')
  },

  // ===== 外部 rclone 互通 =====
  async getInteropConfig(): Promise<InteropConfig> {
    return await invoke<InteropConfig>('get_interop_config')
  },

  async verifyInterop(probe: string): Promise<string> {
    return await invoke<string>('verify_interop', { probe })
  },

  async interopSelfVerify(): Promise<InteropSelfVerify> {
    return await invoke<InteropSelfVerify>('interop_self_verify')
  }
}

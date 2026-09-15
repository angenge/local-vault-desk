export interface RcloneItem {
  Path: string
  Name: string
  Size: number
  MimeType?: string
  ModTime?: string
  IsDir: boolean
}

export interface SearchHit {
  path: string
  name: string
  parent_dir: string
  is_dir: boolean
  size: number
}

export interface TransferProgress {
  active: boolean
  task_type: string // "import" | "export"
  bytes: number
  total_bytes: number
  speed: number
  percentage: number
  current_file: string
  transferred_files: number
  total_files: number
  eta: number | null
}

export interface EngineStatus {
  is_running: boolean
  port: number
  version: string
  message: string
}

export interface VaultStatus {
  is_unlocked: boolean
  vault_path: string
  username: string
}

export interface RecycleEntry {
  id: string
  name: string
  original_path: string
  is_dir: boolean
  deleted_at?: number
}

export interface IntegrityResult {
  checked: number
  ok: number
  differ: number
  missing: number
  has_manifest: boolean
  issues: IntegrityIssue[]
}

export interface IntegrityIssue {
  path: string
  status: string
}

export interface VaultStats {
  files: number
  dirs: number
  total_size: number
  by_extension: Record<string, number>
  by_extension_count: Record<string, number>
}

export interface AuditEntry {
  ts: string
  username: string
  action: string
  detail: string
}

export interface InteropConfig {
  snippet: string
  probe_file: string
  instructions: string
  vault_path: string
}

export interface InteropSelfVerify {
  written: string
  read_back: string
  ok: boolean
}

export interface ClipboardState {
  mode: 'cut' | 'copy'
  paths: string[]
}

export type VaultDirKind = 'not_exist' | 'empty' | 'initialized_vault' | 'non_vault_not_empty'

export interface VaultDirInspection {
  kind: VaultDirKind
  path: string
  file_count: number
  message: string
}

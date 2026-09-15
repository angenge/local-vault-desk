<script setup lang="ts">
import { ref, watch, onMounted, onUnmounted } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { confirm, message } from '@tauri-apps/plugin-dialog'
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import { useVaultStore } from '@/stores/vault'
import { tauriVault } from '@/services/tauriVault'
import Breadcrumb from '@/components/Breadcrumb.vue'
import ImportNoticeModal from '@/components/ImportNoticeModal.vue'
import TransferProgressModal from '@/components/TransferProgressModal.vue'
import VaultToolsPanel from '@/components/VaultToolsPanel.vue'
import {
  Lock,
  Upload,
  FolderPlus,
  RefreshCw,
  Search,
  X,
  FolderSearch,
  Folder,
  File,
  Download,
  HardDrive,
  ShieldCheck,
  Activity,
  Recycle,
  Pencil,
  Scissors,
  Copy,
  ClipboardPaste,
  Wrench
} from 'lucide-vue-next'
import type { RcloneItem, SearchHit, TransferProgress } from '@/types'

const vaultStore = useVaultStore()

const showNoticeModal = ref(false)
const recentlyImported = ref<string[]>([])
const selectedItems = ref<RcloneItem[]>([])
const isExporting = ref(false)

const searchKeyword = ref('')
const isSearching = ref(false)
const searchResults = ref<SearchHit[]>([])
const searchError = ref('')
const isSearchMode = ref(false)
let searchTimer: any = null

const showTools = ref(false)
const recycleCount = ref(0)
const clipboard = ref<{ mode: 'cut' | 'copy'; paths: string[] } | null>(null)
const lastSelectedIdx = ref<number>(-1)

function hitToItem(hit: SearchHit): RcloneItem {
  return {
    Path: hit.path,
    Name: hit.name,
    Size: hit.size,
    IsDir: hit.is_dir,
    ModTime: ''
  }
}

function getActiveItems(): RcloneItem[] {
  return isSearchMode.value ? searchResults.value.map(hitToItem) : vaultStore.files
}

// ===== 右键快捷菜单 =====
interface ContextMenuState {
  show: boolean
  x: number
  y: number
  target: 'item' | 'blank'
}

const contextMenu = ref<ContextMenuState>({
  show: false,
  x: 0,
  y: 0,
  target: 'blank'
})

function closeContextMenu() {
  contextMenu.value.show = false
}

function calculateMenuPosition(clientX: number, clientY: number, menuWidth = 200, menuHeight = 240) {
  const windowWidth = window.innerWidth
  const windowHeight = window.innerHeight
  const x = clientX + menuWidth > windowWidth ? windowWidth - menuWidth - 10 : clientX
  const y = clientY + menuHeight > windowHeight ? windowHeight - menuHeight - 10 : clientY
  return { x: Math.max(10, x), y: Math.max(10, y) }
}

function handleItemContextMenu(item: RcloneItem, event: MouseEvent) {
  if (isTransferring.value || vaultStore.loading) return
  if (!isSelected(item)) {
    selectedItems.value = [item]
  }
  const pos = calculateMenuPosition(event.clientX, event.clientY, 200, 230)
  contextMenu.value = {
    show: true,
    x: pos.x,
    y: pos.y,
    target: 'item'
  }
}

function handleBlankContextMenu(event: MouseEvent) {
  if (isTransferring.value || vaultStore.loading) return
  const pos = calculateMenuPosition(event.clientX, event.clientY, 200, 210)
  contextMenu.value = {
    show: true,
    x: pos.x,
    y: pos.y,
    target: 'blank'
  }
}

function handleGlobalKeyDown(event: KeyboardEvent) {
  markActivity()
  if (contextMenu.value.show && event.key === 'Escape') {
    closeContextMenu()
    return
  }
  // 如果焦点在输入框（例如密码、搜索、新建目录名），不拦截快捷键
  const tag = (event.target as HTMLElement)?.tagName?.toLowerCase()
  if (tag === 'input' || tag === 'textarea') return

  if (isTransferring.value || vaultStore.loading) return

  // Ctrl+A / Cmd+A 全选
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'a') {
    const activeList = getActiveItems()
    if (activeList.length > 0) {
      event.preventDefault()
      selectedItems.value = [...activeList]
    }
    return
  }

  // Ctrl+C / Cmd+C 复制
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'c') {
    if (selectedItems.value.length > 0) {
      event.preventDefault()
      copyToClipboard('copy')
    }
    return
  }

  // Ctrl+X / Cmd+X 剪切
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'x') {
    if (selectedItems.value.length > 0) {
      event.preventDefault()
      copyToClipboard('cut')
    }
    return
  }

  // Ctrl+V / Cmd+V 粘贴
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'v') {
    if (canPaste()) {
      event.preventDefault()
      handlePaste()
    }
    return
  }

  // F2 重命名
  if (event.key === 'F2') {
    if (selectedItems.value.length === 1) {
      event.preventDefault()
      handleRename()
    }
    return
  }

  // Delete 移入回收站
  if (event.key === 'Delete') {
    if (selectedItems.value.length > 0) {
      event.preventDefault()
      handleRecycle()
    }
    return
  }
}

// ===== 名称输入弹窗（替代原生 prompt，避免出现 tauri.localhost 标题） =====
const nameModal = ref<{
  mode: 'create' | 'rename'
  title: string
  confirmLabel: string
  initial?: string
} | null>(null)
const nameInput = ref('')
const nameError = ref('')

function uiMessage(text: string) {
  return message(text, { title: '本地文件保险箱' })
}

// ===== 安全增强：自动锁定（闲置超时） =====
const AUTO_LOCK_OPTIONS = [
  { value: 0, label: '关闭' },
  { value: 5, label: '5 分钟' },
  { value: 15, label: '15 分钟' },
  { value: 30, label: '30 分钟' }
]
const autoLockMinutes = ref<number>(Number(localStorage.getItem('auto_lock_minutes')) || 0)
let lastActivity = Date.now()
let autoLockTimer: any = null
// 工具面板执行长时间任务（完整性自检/回收站清空等）时抑制自动锁定，避免任务被中途打断
const toolsBusy = ref(false)

function markActivity() {
  lastActivity = Date.now()
}

function updateAutoLockTimer() {
  if (autoLockTimer) clearInterval(autoLockTimer)
  autoLockTimer = null
  const minutes = autoLockMinutes.value
  if (minutes <= 0) return
  autoLockTimer = setInterval(() => {
    if (vaultStore.isUnlocked && !isTransferring.value && !toolsBusy.value) {
      if (Date.now() - lastActivity >= minutes * 60 * 1000) {
        vaultStore.lock()
      }
    }
  }, 1000)
}

function onAutoLockChange() {
  localStorage.setItem('auto_lock_minutes', String(autoLockMinutes.value))
  updateAutoLockTimer()
}

// 锁定后立即清除明文名称驻留（选中项/剪贴板/搜索结果/工具抽屉）
watch(
  () => vaultStore.isUnlocked,
  (unlocked) => {
    if (!unlocked) {
      selectedItems.value = []
      clipboard.value = null
      showTools.value = false
      clearSearch()
    } else {
      lastActivity = Date.now()
    }
  }
)

// 切换目录时自动清空选区，防止跨目录误操作
watch(
  () => vaultStore.currentDir,
  () => {
    selectedItems.value = []
  }
)

// ===== 前端名称合法性校验（与后端规则一致） =====
const RESERVED_NAMES = ['CON', 'PRN', 'AUX', 'NUL',
  'COM1', 'COM2', 'COM3', 'COM4', 'COM5', 'COM6', 'COM7', 'COM8', 'COM9',
  'LPT1', 'LPT2', 'LPT3', 'LPT4', 'LPT5', 'LPT6', 'LPT7', 'LPT8', 'LPT9',
  'AUDIT', 'AUDIT.LOG']

function validateVaultName(name: string): string | null {
  const t = name.trim()
  if (!t) return '名称不能为空'
  if (t === '.' || t === '..') return '名称不能是 . 或 ..'
  if (/^[ .]|[ .]$/.test(t)) return '名称不能以空格或点号开头/结尾'
  if (/[<>:"/\\|?*]/.test(t)) return '名称不能包含字符: < > : " / \\ | ? *'
  if (/[\u0000-\u001F]/.test(t)) return '名称不能包含控制字符'
  const upper = t.toUpperCase()
  const stem = upper.split('.')[0]
  if (RESERVED_NAMES.includes(upper) || RESERVED_NAMES.includes(stem)) {
    return '名称不能使用系统保留名 (CON/PRN/AUX/NUL/COM1~9/LPT1~9/audit.log)'
  }
  return null
}

function extractError(err: any, fallback: string): string {
  if (typeof err === 'string') return err
  return err?.message || String(err || fallback)
}

function isCancelledError(err: any): boolean {
  const msg = extractError(err, '')
  return msg.includes('已取消') || msg.toLowerCase().includes('cancel')
}

const isTransferring = ref(false)
const transferProgress = ref<TransferProgress>({
  active: false,
  task_type: 'import',
  bytes: 0,
  total_bytes: 0,
  speed: 0,
  percentage: 0,
  current_file: '',
  transferred_files: 0,
  total_files: 0,
  eta: null
})

let timer: any = null
let unlistenProgress: UnlistenFn | null = null
let unlistenDrop: UnlistenFn | null = null
let transferCloseTimer: any = null

function initProgressState(taskType: 'import' | 'export') {
  transferProgress.value = {
    active: true,
    task_type: taskType,
    bytes: 0,
    total_bytes: 0,
    speed: 0,
    percentage: 0,
    current_file: '准备传输...',
    transferred_files: 0,
    total_files: 0,
    eta: null
  }
  isTransferring.value = true
}

onMounted(async () => {
  await vaultStore.checkEngine()

  // 采用 Tauri 事件推送接收实时进度，彻底消除每秒多次的 IPC 轮询开销
  unlistenProgress = await listen<TransferProgress>('transfer-progress', (event) => {
    const payload = event.payload
    if (!payload) return
    transferProgress.value = payload
    markActivity()
    // 后端主动推送任务的弹窗开关（如清空回收站）：
    // - active=true 立即打开弹窗；
    // - active=false 延迟关闭：批量导出各子任务间会连续出现 false 事件，
    //   若立即关闭会导致弹窗闪烁，仅在一段时间内无新任务时真正关闭。
    if (payload.active) {
      isTransferring.value = true
      clearTimeout(transferCloseTimer)
    } else if (isTransferring.value) {
      clearTimeout(transferCloseTimer)
      transferCloseTimer = setTimeout(() => {
        if (!transferProgress.value.active) {
          isTransferring.value = false
          markActivity()
        }
      }, 600)
    }
  })

  // 原生拖拽文件到窗口 → 自动加密导入当前目录
  unlistenDrop = await getCurrentWebviewWindow().onDragDropEvent((event) => {
    if (event.payload.type === 'drop' && event.payload.paths.length > 0) {
      handleDroppedPaths(event.payload.paths)
    }
  })

  await refreshRecycleBadge()

  // 用户活动监听与全局快捷键
  window.addEventListener('pointerdown', handleGlobalPointerDown)
  window.addEventListener('keydown', handleGlobalKeyDown)
  window.addEventListener('mousemove', markActivity)
  window.addEventListener('wheel', markActivity, { passive: true })
  window.addEventListener('scroll', closeContextMenu, true)

  updateAutoLockTimer()

  timer = setInterval(() => {
    if (vaultStore.isUnlocked && !isTransferring.value) {
      vaultStore.checkEngine()
    }
  }, 4000)
})

function handleGlobalPointerDown(event: PointerEvent) {
  markActivity()
  if (contextMenu.value.show) {
    // 如果点击在右键菜单自身内部，不立即关闭
    const menuEl = document.getElementById('vault-context-menu')
    if (menuEl && menuEl.contains(event.target as Node)) {
      return
    }
    closeContextMenu()
  }
}

onUnmounted(() => {
  if (timer) clearInterval(timer)
  if (searchTimer) clearTimeout(searchTimer)
  if (autoLockTimer) clearInterval(autoLockTimer)
  window.removeEventListener('pointerdown', handleGlobalPointerDown)
  window.removeEventListener('keydown', handleGlobalKeyDown)
  window.removeEventListener('mousemove', markActivity)
  window.removeEventListener('wheel', markActivity)
  window.removeEventListener('scroll', closeContextMenu, true)
  if (unlistenProgress) {
    unlistenProgress()
    unlistenProgress = null
  }
  if (unlistenDrop) {
    unlistenDrop()
    unlistenDrop = null
  }
})

async function refreshRecycleBadge() {
  if (!vaultStore.isUnlocked) return
  try {
    const entries = await tauriVault.listRecycle()
    recycleCount.value = entries.length
  } catch {
    recycleCount.value = 0
  }
}

async function onToolsPanelRefresh() {
  await Promise.all([refreshRecycleBadge(), vaultStore.refreshFiles()])
}

async function handleDroppedPaths(paths: string[]) {
  if (isTransferring.value || vaultStore.loading) return

  // 前端防御：禁止拖拽导入当前保险箱自身密文目录或其内部文件
  const vPath = vaultStore.vaultPath.replace(/\\/g, '/').toLowerCase().replace(/\/+$/, '')
  for (const p of paths) {
    const normP = p.replace(/\\/g, '/').toLowerCase().replace(/\/+$/, '')
    if (normP === vPath || normP.startsWith(vPath + '/') || vPath.startsWith(normP + '/')) {
      uiMessage('禁止将当前保险箱密文目录或其内部文件导入保险箱自身')
      return
    }
  }

  initProgressState('import')
  vaultStore.loading = true
  try {
    const imported = await tauriVault.importPaths({
      sourcePaths: paths,
      targetDir: vaultStore.currentDir
    })
    await vaultStore.refreshFiles()
    recentlyImported.value = imported
    showNoticeModal.value = true
  } catch (err: any) {
    if (!isCancelledError(err)) uiMessage(`拖拽导入失败: ${extractError(err, '导入失败')}`)
  } finally {
    isTransferring.value = false
    vaultStore.loading = false
    markActivity()
  }
}

async function refresh() {
  await vaultStore.refreshFiles()
}

function formatSize(bytes: number): string {
  if (bytes === -1 || bytes === undefined || isNaN(bytes)) return '-'
  if (bytes <= 0) return '0 B'
  const k = 1024
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB']
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(k)), sizes.length - 1)
  return (bytes / Math.pow(k, i)).toFixed(1) + ' ' + sizes[i]
}

function formatTime(iso?: string): string {
  if (!iso) return '-'
  try {
    const d = new Date(iso)
    return d.toLocaleString()
  } catch {
    return iso
  }
}

async function handleImportFiles() {
  const filePaths = await tauriVault.selectImportFiles()
  if (!filePaths || filePaths.length === 0) return

  initProgressState('import')
  vaultStore.loading = true
  try {
    const imported = await tauriVault.importPaths({
      sourcePaths: filePaths,
      targetDir: vaultStore.currentDir
    })
    await vaultStore.refreshFiles()

    recentlyImported.value = imported
    showNoticeModal.value = true
  } catch (err: any) {
    if (!isCancelledError(err)) uiMessage(`导入失败: ${extractError(err, '导入失败')}`)
  } finally {
    isTransferring.value = false
    vaultStore.loading = false
    markActivity()
  }
}

async function handleCancelTransfer() {
  try {
    await tauriVault.cancelTransfer()
  } catch (err: any) {
    if (!isCancelledError(err)) uiMessage(`取消失败: ${extractError(err, '取消失败')}`)
  }
}

async function handleImportFolder() {
  const folderPath = await tauriVault.selectImportFolder()
  if (!folderPath) return

  initProgressState('import')
  vaultStore.loading = true
  try {
    const imported = await tauriVault.importPaths({
      sourcePaths: [folderPath],
      targetDir: vaultStore.currentDir
    })
    await vaultStore.refreshFiles()

    recentlyImported.value = imported
    showNoticeModal.value = true
  } catch (err: any) {
    if (!isCancelledError(err)) uiMessage(`导入失败: ${extractError(err, '导入失败')}`)
  } finally {
    isTransferring.value = false
    vaultStore.loading = false
    markActivity()
  }
}

async function handleCreateFolder() {
  nameError.value = ''
  nameInput.value = ''
  nameModal.value = { mode: 'create', title: '新建目录', confirmLabel: '新建' }
}

async function submitNameModal() {
  if (!nameModal.value) return
  const name = nameInput.value.trim()
  const invalid = validateVaultName(name)
  if (invalid) {
    nameError.value = invalid
    return
  }
  const { mode, initial } = nameModal.value
  nameModal.value = null
  try {
    if (mode === 'create') {
      const targetPath = vaultStore.currentDir ? `${vaultStore.currentDir}/${name}` : name
      await tauriVault.createFolder(targetPath)
    } else {
      const item = selectedItems.value[0]
      if (!item || name === initial) return
      await tauriVault.renameItem({
        vaultItemPath: item.Path,
        newName: name
      })
    }
    await vaultStore.refreshFiles()
    // 重命名/新建后旧路径已失效，清空选区避免后续操作使用陈旧路径
    selectedItems.value = []
    lastSelectedIdx.value = -1
    if (isSearchMode.value && searchKeyword.value.trim()) {
      await runSearch(searchKeyword.value.trim())
    }
  } catch (err: any) {
    uiMessage(mode === 'create' ? `创建失败: ${extractError(err, '创建失败')}` : `重命名失败: ${extractError(err, '重命名失败')}`)
  }
}

function handleItemClick(item: RcloneItem, event: MouseEvent, index?: number) {
  const activeList = getActiveItems()
  const curIdx = index !== undefined ? index : activeList.findIndex((i) => i.Path === item.Path)

  if (event.shiftKey && lastSelectedIdx.value >= 0 && lastSelectedIdx.value < activeList.length) {
    const start = Math.min(lastSelectedIdx.value, curIdx)
    const end = Math.max(lastSelectedIdx.value, curIdx)
    const range = activeList.slice(start, end + 1)
    if (event.ctrlKey || event.metaKey) {
      const existingPaths = new Set(selectedItems.value.map((i) => i.Path))
      for (const it of range) {
        if (!existingPaths.has(it.Path)) {
          selectedItems.value.push(it)
          existingPaths.add(it.Path)
        }
      }
    } else {
      selectedItems.value = range
    }
  } else if (event.ctrlKey || event.metaKey) {
    const idx = selectedItems.value.findIndex((i) => i.Path === item.Path)
    if (idx >= 0) {
      selectedItems.value.splice(idx, 1)
    } else {
      selectedItems.value.push(item)
    }
    lastSelectedIdx.value = curIdx
  } else {
    selectedItems.value = [item]
    lastSelectedIdx.value = curIdx
  }
}

function isSelected(item: RcloneItem): boolean {
  return selectedItems.value.some((i) => i.Path === item.Path)
}

function handleItemDblClick(item: RcloneItem) {
  if (item.IsDir) {
    vaultStore.navigateTo(item.Path)
    selectedItems.value = []
    lastSelectedIdx.value = -1
  }
}

async function handleExport() {
  if (selectedItems.value.length === 0) {
    uiMessage('请先选中要解密导出的文件或文件夹')
    return
  }
  const targetDir = await tauriVault.selectExportDirectory()
  if (!targetDir) return

  const paths = selectedItems.value.map((i) => i.Path)
  const isDirs = selectedItems.value.map((i) => i.IsDir)

  initProgressState('export')
  isExporting.value = true
  try {
    if (paths.length === 1) {
      await tauriVault.exportItem({
        vaultItemPath: paths[0],
        isDir: isDirs[0],
        targetDir
      })
    } else {
      await tauriVault.exportItems({ vaultItemPaths: paths, isDirs, targetDir })
    }
    uiMessage(`解密导出成功！已还原 ${paths.length} 个条目至:\n${targetDir}`)
  } catch (err: any) {
    if (!isCancelledError(err)) uiMessage(`导出失败: ${extractError(err, '导出失败')}`)
  } finally {
    isTransferring.value = false
    isExporting.value = false
    markActivity()
  }
}

async function handleRename() {
  if (selectedItems.value.length !== 1) return
  const item = selectedItems.value[0]
  nameError.value = ''
  nameInput.value = item.Name
  nameModal.value = { mode: 'rename', title: '重命名', confirmLabel: '确定', initial: item.Name }
}

function copyToClipboard(mode: 'cut' | 'copy') {
  if (selectedItems.value.length === 0) return
  clipboard.value = {
    mode,
    paths: selectedItems.value.map((i) => i.Path)
  }
}

const canPaste = () => clipboard.value !== null && clipboard.value.paths.length > 0

async function handlePaste() {
  if (!clipboard.value || clipboard.value.paths.length === 0) return
  const { mode, paths } = clipboard.value
  try {
    if (mode === 'cut') {
      await tauriVault.moveItems({
        sourcePaths: paths,
        destDir: vaultStore.currentDir
      })
      clipboard.value = null
    } else {
      await tauriVault.copyItems({
        sourcePaths: paths,
        destDir: vaultStore.currentDir
      })
    }
    await vaultStore.refreshFiles()
    if (isSearchMode.value && searchKeyword.value.trim()) {
      await runSearch(searchKeyword.value.trim())
    }
  } catch (err: any) {
    const errorText = extractError(err, mode === 'cut' ? '移动失败' : '复制失败')
    uiMessage(mode === 'cut' ? `移动失败: ${errorText}` : `复制失败: ${errorText}`)
    // 若是源文件已不存在导致失败，自动清除剪贴板以避免重复报错
    if (errorText.includes('不存在') || errorText.toLowerCase().includes('not found')) {
      clipboard.value = null
    }
  }
}

async function handleRecycle() {
  if (selectedItems.value.length === 0) return
  const names = selectedItems.value.map((i) => i.Name).join('、')
  const ok = await confirm(
    `确定将 ${selectedItems.value.length} 项（${names}）移入回收站吗？可在回收站中恢复。`,
    {
      title: '移入回收站',
      kind: 'warning',
      okLabel: '移入回收站',
      cancelLabel: '取消'
    }
  )
  if (!ok) return

  let failedCount = 0
  for (const item of [...selectedItems.value]) {
    try {
      await tauriVault.recycleItem({
        vaultItemPath: item.Path,
        isDir: item.IsDir
      })
      // 逐个移除已成功回收的条目，避免中途失败后残留已失效路径的选区
      const idx = selectedItems.value.findIndex((i) => i.Path === item.Path)
      if (idx >= 0) selectedItems.value.splice(idx, 1)
    } catch {
      failedCount++
    }
  }

  if (failedCount > 0) {
    uiMessage(`有 ${failedCount} 项未能移入回收站（可能已被删除或路径已变更），其余已成功`)
  }
  lastSelectedIdx.value = -1
  await vaultStore.refreshFiles()
  await refreshRecycleBadge()
  if (isSearchMode.value && searchKeyword.value.trim()) {
    await runSearch(searchKeyword.value.trim())
  }
}

let currentSearchSeq = 0

function onSearchInput() {
  if (searchTimer) clearTimeout(searchTimer)
  const keyword = searchKeyword.value.trim()
  if (!keyword) {
    clearSearch()
    return
  }
  searchTimer = setTimeout(() => {
    runSearch(keyword)
  }, 300)
}

async function runSearch(keyword: string) {
  const seq = ++currentSearchSeq
  isSearching.value = true
  searchError.value = ''
  try {
    const results = await tauriVault.searchFiles(keyword, 200)
    if (seq !== currentSearchSeq) return
    searchResults.value = results
    isSearchMode.value = true
  } catch (err: any) {
    if (seq !== currentSearchSeq) return
    searchError.value = extractError(err, '搜索失败')
  } finally {
    if (seq === currentSearchSeq) {
      isSearching.value = false
    }
  }
}

function clearSearch() {
  currentSearchSeq++
  if (searchTimer) clearTimeout(searchTimer)
  searchKeyword.value = ''
  isSearchMode.value = false
  searchResults.value = []
  searchError.value = ''
  isSearching.value = false
}

async function jumpToHit(hit: SearchHit) {
  if (hit.is_dir) {
    vaultStore.navigateTo(hit.path)
  } else {
    vaultStore.navigateTo(hit.parent_dir || '')
  }
  clearSearch()
}

function onEnterSearch() {
  if (searchTimer) clearTimeout(searchTimer)
  const keyword = searchKeyword.value.trim()
  if (!keyword) return
  runSearch(keyword)
}
</script>

<template>
  <div class="h-screen flex flex-col bg-slate-900 text-slate-100 overflow-hidden">
    <!-- 顶部导航栏 -->
    <header class="h-14 bg-slate-950 border-b border-slate-800 px-5 flex items-center justify-between shrink-0">
      <div class="flex items-center gap-3 min-w-0 flex-1">
        <div class="w-8 h-8 rounded-lg bg-blue-600/20 text-blue-400 flex items-center justify-center font-bold shrink-0">
          <HardDrive class="w-4 h-4" />
        </div>
        <div class="min-w-0">
          <div class="text-xs font-semibold text-white flex items-center gap-1.5 whitespace-nowrap min-w-0">
            <span class="truncate">安全保险箱已就绪</span>
            <span class="inline-flex items-center px-1.5 py-0.5 rounded text-[10px] font-medium bg-emerald-500/20 text-emerald-400 shrink-0">
              本地安全加密
            </span>
          </div>
          <div class="text-[10px] text-slate-400 truncate max-w-sm" :title="vaultStore.vaultPath">
            {{ vaultStore.vaultPath }}
          </div>
        </div>
      </div>

      <!-- 右侧操作 -->
      <div class="flex items-center gap-3 shrink-0">
        <!-- 保险箱工具（回收站/自检/统计/审计） -->
        <button
          @click="showTools = !showTools"
          :disabled="!vaultStore.isUnlocked"
          class="relative flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-slate-800 hover:bg-slate-700 text-slate-300 text-xs font-medium transition disabled:opacity-40 disabled:cursor-not-allowed shrink-0 whitespace-nowrap"
          title="保险箱管理工具"
        >
          <Wrench class="w-3.5 h-3.5" />
          <span>保险箱工具</span>
          <span
            v-if="recycleCount > 0"
            class="flex items-center justify-center min-w-4 h-4 px-1 rounded-full bg-red-500 text-white text-[10px] font-bold"
          >
            {{ recycleCount }}
          </span>
        </button>

        <!-- 加密引擎运行状态指示灯 -->
        <div
          class="flex items-center gap-2 px-2.5 py-1.5 rounded-lg border text-xs font-medium transition cursor-default select-none shadow-inner shrink-0 whitespace-nowrap"
          :class="vaultStore.engineStatus.is_running
            ? 'bg-emerald-500/10 border-emerald-500/30 text-emerald-300'
            : 'bg-amber-500/10 border-amber-500/30 text-amber-300'"
          :title="`安全加密引擎: ${vaultStore.engineStatus.message}`"
        >
          <span class="relative flex h-2 w-2">
            <span
              v-if="vaultStore.engineStatus.is_running"
              class="animate-ping absolute inline-flex h-full w-full rounded-full bg-emerald-400 opacity-75"
            ></span>
            <span
              class="relative inline-flex rounded-full h-2 w-2"
              :class="vaultStore.engineStatus.is_running ? 'bg-emerald-500' : 'bg-amber-500'"
            ></span>
          </span>
          <span class="text-[11px] font-mono flex items-center gap-1">
            <Activity class="w-3 h-3 text-emerald-400" v-if="vaultStore.engineStatus.is_running" />
            <span>安全引擎: {{ vaultStore.engineStatus.is_running ? '正常运行' : '待命中' }}</span>
          </span>
        </div>

        <!-- 自动锁定档位 -->
        <div class="flex items-center gap-1.5 pl-1 shrink-0 whitespace-nowrap">
          <span class="text-[10px] text-slate-500 select-none">自动锁定</span>
          <select
            v-model.number="autoLockMinutes"
            @change="onAutoLockChange"
            :disabled="isTransferring || vaultStore.loading"
            class="bg-slate-800 border border-slate-700 rounded-lg px-2 py-1.5 text-[11px] text-slate-300 outline-none focus:border-blue-500/60 transition disabled:opacity-50"
            title="闲置超时后自动锁定保险箱"
          >
            <option v-for="opt in AUTO_LOCK_OPTIONS" :key="opt.value" :value="opt.value">
              {{ opt.label }}
            </option>
          </select>
        </div>

        <button
          @click="vaultStore.lock"
          :disabled="isTransferring || vaultStore.loading"
          class="flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-slate-800 hover:bg-red-500/20 hover:text-red-300 text-slate-300 text-xs font-medium transition disabled:opacity-50 disabled:cursor-not-allowed shrink-0 whitespace-nowrap"
        >
          <Lock class="w-3.5 h-3.5" />
          <span>立即锁定</span>
        </button>
      </div>
    </header>

    <!-- 工具栏 -->
    <div class="h-12 bg-slate-900/90 border-b border-slate-800 px-5 flex items-center justify-between shrink-0">
      <!-- 左侧操作组 -->
      <div class="flex items-center gap-2 shrink-0">
        <button
          @click="handleImportFiles"
          :disabled="isTransferring || vaultStore.loading"
          class="flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-blue-600 hover:bg-blue-500 text-white text-xs font-medium transition shadow-sm disabled:opacity-50 disabled:cursor-not-allowed shrink-0 whitespace-nowrap"
        >
          <Upload class="w-3.5 h-3.5" />
          <span>导入文件</span>
        </button>
        <button
          @click="handleImportFolder"
          :disabled="isTransferring || vaultStore.loading"
          class="flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-slate-800 hover:bg-slate-700 text-slate-200 text-xs font-medium transition disabled:opacity-50 disabled:cursor-not-allowed shrink-0 whitespace-nowrap"
        >
          <FolderPlus class="w-3.5 h-3.5" />
          <span>导入文件夹</span>
        </button>
        <button
          @click="handleCreateFolder"
          :disabled="isTransferring || vaultStore.loading"
          class="flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-slate-800 hover:bg-slate-700 text-slate-200 text-xs font-medium transition disabled:opacity-50 disabled:cursor-not-allowed shrink-0 whitespace-nowrap"
        >
          <span>新建目录</span>
        </button>
      </div>

      <!-- 搜索框 -->
      <div class="flex-1 flex justify-center px-4 min-w-0">
        <div
          class="relative w-full max-w-sm flex items-center bg-slate-950/70 border border-slate-800 rounded-lg focus-within:border-blue-500/60 transition min-w-0"
        >
          <Search class="w-3.5 h-3.5 text-slate-500 absolute left-2.5 shrink-0" />
          <input
            v-model="searchKeyword"
            type="text"
            placeholder="搜索保险箱内所有文件 / 目录..."
            class="flex-1 bg-transparent outline-none pl-8 pr-8 py-1.5 text-xs text-slate-200 placeholder:text-slate-600 min-w-0"
            @input="onSearchInput"
            @keydown.enter="onEnterSearch"
            @keydown.esc="clearSearch"
          />
          <button
            v-if="searchKeyword"
            @click="clearSearch"
            class="absolute right-2 p-0.5 rounded hover:bg-slate-800 text-slate-500 hover:text-white transition"
            title="清空搜索"
          >
            <X class="w-3.5 h-3.5" />
          </button>
        </div>
      </div>

      <!-- 右侧选中项操作 -->
      <div class="flex items-center gap-2 shrink-0">
        <div
          v-if="clipboard"
          class="flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg bg-blue-500/10 border border-blue-500/30 text-blue-300 text-[11px] shrink-0 whitespace-nowrap"
        >
          <Scissors v-if="clipboard.mode === 'cut'" class="w-3.5 h-3.5" />
          <Copy v-else class="w-3.5 h-3.5" />
          <span>{{ clipboard.paths.length }} 项{{ clipboard.mode === 'cut' ? '剪切中' : '已复制' }}</span>
          <button @click="clipboard = null" class="ml-1 p-0.5 rounded hover:bg-blue-500/20 text-blue-300" title="取消">
            <X class="w-3 h-3" />
          </button>
        </div>
        <template v-if="selectedItems.length > 0">
          <button
            v-if="selectedItems.length === 1"
            @click="handleRename"
            :disabled="isTransferring || vaultStore.loading"
            class="flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg bg-slate-800 hover:bg-slate-700 text-slate-300 text-xs font-medium transition disabled:opacity-50 disabled:cursor-not-allowed shrink-0 whitespace-nowrap"
            title="重命名"
          >
            <Pencil class="w-3.5 h-3.5" />
          </button>
          <button
            @click="copyToClipboard('cut')"
            :disabled="isTransferring || vaultStore.loading"
            class="flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg bg-slate-800 hover:bg-slate-700 text-slate-300 text-xs font-medium transition disabled:opacity-50 disabled:cursor-not-allowed shrink-0 whitespace-nowrap"
            title="剪切（粘贴后移动）"
          >
            <Scissors class="w-3.5 h-3.5" />
          </button>
          <button
            @click="copyToClipboard('copy')"
            :disabled="isTransferring || vaultStore.loading"
            class="flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg bg-slate-800 hover:bg-slate-700 text-slate-300 text-xs font-medium transition disabled:opacity-50 disabled:cursor-not-allowed shrink-0 whitespace-nowrap"
            title="复制"
          >
            <Copy class="w-3.5 h-3.5" />
          </button>
          <button
            v-if="selectedItems.length >= 1"
            @click="handleExport"
            :disabled="isExporting || isTransferring || vaultStore.loading"
            class="flex items-center gap-1.5 px-3 py-1.5 rounded-lg bg-emerald-600 hover:bg-emerald-500 text-white text-xs font-medium transition disabled:opacity-50 disabled:cursor-not-allowed shrink-0 whitespace-nowrap"
          >
            <Download class="w-3.5 h-3.5" />
            <span>{{ isExporting ? '正在解密导出...' : (selectedItems.length > 1 ? `导出 (${selectedItems.length})` : '导出') }}</span>
          </button>
          <button
            @click="handleRecycle"
            :disabled="isTransferring || vaultStore.loading"
            class="flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg bg-slate-800 hover:bg-red-500/20 text-slate-300 hover:text-red-400 text-xs font-medium transition disabled:opacity-50 disabled:cursor-not-allowed"
            title="移入回收站"
          >
            <Recycle class="w-3.5 h-3.5" />
          </button>
        </template>
        <button
          @click="handlePaste"
:disabled="!canPaste() || isTransferring || vaultStore.loading"
          class="flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg bg-slate-800 hover:bg-slate-700 text-slate-300 text-xs font-medium transition disabled:opacity-50 disabled:cursor-not-allowed shrink-0 whitespace-nowrap"
          title="粘贴到当前目录"
        >
          <ClipboardPaste class="w-3.5 h-3.5" />
          <span>粘贴</span>
        </button>
        <button
          @click="refresh"
          :disabled="isTransferring || vaultStore.loading"
          class="p-1.5 rounded-lg hover:bg-slate-800 text-slate-400 hover:text-white transition disabled:opacity-50 disabled:cursor-not-allowed shrink-0"
          title="刷新列表"
        >
          <RefreshCw class="w-4 h-4" :class="{ 'animate-spin': vaultStore.loading }" />
        </button>
      </div>
    </div>

    <!-- 路径面包屑导航 -->
    <div class="px-5 py-2 bg-slate-950/40 border-b border-slate-800/60">
      <Breadcrumb :currentDir="vaultStore.currentDir" @navigate="vaultStore.navigateTo" />
    </div>

    <!-- 文件管理主视图 -->
    <div
      class="flex-1 overflow-y-auto p-5"
      @contextmenu.prevent="handleBlankContextMenu($event)"
    >
      <!-- 全库搜索模式 -->
      <template v-if="isSearchMode">
        <div v-if="searchError" class="py-16 flex flex-col items-center justify-center text-slate-500">
          <FolderSearch class="w-12 h-12 text-slate-700 mb-3" />
          <div class="text-sm text-slate-400">{{ searchError }}</div>
        </div>
        <div
          v-else-if="!isSearching && searchResults.length === 0"
          class="py-16 flex flex-col items-center justify-center text-slate-500"
        >
          <FolderSearch class="w-12 h-12 text-slate-700 mb-3" />
          <div class="text-sm font-medium text-slate-400">未找到与 “{{ searchKeyword }}” 匹配的项目</div>
          <p class="text-xs text-slate-600 mt-1">仅搜索文件名 / 目录名，加密内容无法全文检索</p>
        </div>
        <table v-else class="w-full text-left text-xs text-slate-300 border-collapse">
          <thead>
            <tr class="text-slate-500 border-b border-slate-800 pb-2">
              <th class="py-2.5 px-3 font-semibold">名称</th>
              <th class="py-2.5 px-3 font-semibold w-28">大小</th>
              <th class="py-2.5 px-3 font-semibold w-64">所在位置</th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-for="(hit, idx) in searchResults"
              :key="hit.path"
              @click="handleItemClick(hitToItem(hit), $event, idx)"
              @dblclick="jumpToHit(hit)"
              @contextmenu.prevent.stop="handleItemContextMenu(hitToItem(hit), $event)"
              class="hover:bg-slate-800/70 cursor-pointer border-b border-slate-800/40 transition select-none"
              :class="isSelected(hitToItem(hit)) ? 'bg-blue-600/20 text-white font-medium' : ''"
              title="单击选中，双击跳转至所在目录"
            >
              <td class="py-2.5 px-3 truncate max-w-md">
                <div class="flex items-center gap-2.5">
                  <Folder v-if="hit.is_dir" class="w-4 h-4 text-amber-400 shrink-0" />
                  <File v-else class="w-4 h-4 text-slate-400 shrink-0" />
                  <span class="truncate">{{ hit.name }}</span>
                </div>
              </td>
              <td class="py-2.5 px-3 text-slate-400">
                {{ hit.is_dir ? '-' : formatSize(hit.size) }}
              </td>
              <td class="py-2.5 px-3 text-slate-500 truncate">
                {{ hit.parent_dir || '/' }}
              </td>
            </tr>
          </tbody>
        </table>
      </template>

      <!-- 普通目录浏览模式 -->
      <template v-else>
        <table v-if="vaultStore.currentDir || vaultStore.files.length > 0" class="w-full text-left text-xs text-slate-300 border-collapse">
          <thead>
            <tr class="text-slate-500 border-b border-slate-800 pb-2">
              <th class="py-2.5 px-3 font-semibold">名称</th>
              <th class="py-2.5 px-3 font-semibold w-28">大小</th>
              <th class="py-2.5 px-3 font-semibold w-40">修改时间</th>
            </tr>
          </thead>
          <tbody>
            <tr
              v-if="vaultStore.currentDir"
              @dblclick="vaultStore.goUp"
              class="hover:bg-slate-800/60 cursor-pointer border-b border-slate-800/40 transition select-none"
              title="双击返回上一级目录"
            >
              <td colspan="3" class="py-2.5 px-3 flex items-center gap-2 text-blue-400 font-medium">
                <Folder class="w-4 h-4" />
                <span>.. (返回上一级)</span>
              </td>
            </tr>

            <tr
              v-for="(item, idx) in vaultStore.files"
              :key="item.Path"
              @click="handleItemClick(item, $event, idx)"
              @dblclick="handleItemDblClick(item)"
              @contextmenu.prevent.stop="handleItemContextMenu(item, $event)"
              class="hover:bg-slate-800/70 cursor-pointer border-b border-slate-800/40 transition select-none"
              :class="isSelected(item) ? 'bg-blue-600/20 text-white font-medium' : ''"
            >
              <td class="py-2.5 px-3 flex items-center gap-2.5 truncate max-w-md">
                <Folder v-if="item.IsDir" class="w-4 h-4 text-amber-400 shrink-0" />
                <File v-else class="w-4 h-4 text-slate-400 shrink-0" />
                <span class="truncate">{{ item.Name }}</span>
              </td>
              <td class="py-2.5 px-3 text-slate-400">
                {{ item.IsDir ? '-' : formatSize(item.Size) }}
              </td>
              <td class="py-2.5 px-3 text-slate-400">
                {{ formatTime(item.ModTime) }}
              </td>
            </tr>
          </tbody>
        </table>

        <!-- 空目录提示 -->
        <div
          v-if="!vaultStore.loading && vaultStore.files.length === 0"
          class="flex flex-col items-center justify-center text-slate-500 py-16"
        >
          <ShieldCheck class="w-16 h-16 text-slate-700 mb-3" />
          <div class="text-sm font-medium text-slate-400">
            {{ vaultStore.currentDir ? '当前子目录为空' : '保险箱当前目录为空' }}
          </div>
          <p class="text-xs text-slate-600 mt-1">点击上方“导入文件/文件夹”，右键空白区域或将文件拖入本窗口</p>
          <button
            v-if="vaultStore.currentDir"
            type="button"
            @click="vaultStore.goUp"
            class="mt-4 px-3.5 py-1.5 bg-slate-800 hover:bg-slate-700 text-blue-400 text-xs font-medium rounded-lg transition flex items-center gap-1.5"
          >
            <Folder class="w-3.5 h-3.5" />
            返回上一级
          </button>
        </div>

        <!-- 空白右键/点击区域：列表占满整屏时仍可呼出空白上下文菜单（新建目录 / 粘贴） -->
        <div
          class="h-24 flex items-center justify-center text-[11px] text-slate-700 select-none"
          @contextmenu.prevent="handleBlankContextMenu($event)"
        >
          可在此空白处右键呼出目录操作菜单
        </div>
      </template>
    </div>

    <!-- 底部状态条 -->
    <footer class="h-8 bg-slate-950 border-t border-slate-800 px-5 flex items-center justify-between text-[11px] text-slate-500 shrink-0 select-none">
      <div class="whitespace-nowrap shrink-0">
        {{ isSearchMode
          ? (isSearching ? '正在搜索整个保险箱...' : `共找到 ${searchResults.length} 个结果`)
          : `共 ${vaultStore.files.length} 个项目` }}
      </div>
      <div class="flex items-center gap-2 min-w-0">
        <span class="relative flex h-2 w-2 shrink-0">
          <span
            v-if="vaultStore.engineStatus.is_running"
            class="animate-ping absolute inline-flex h-full w-full rounded-full bg-emerald-400 opacity-75"
          ></span>
          <span
            class="relative inline-flex rounded-full h-2 w-2"
            :class="vaultStore.engineStatus.is_running ? 'bg-emerald-500' : 'bg-slate-600'"
          ></span>
        </span>
        <span
          class="truncate whitespace-nowrap"
          :class="vaultStore.engineStatus.is_running ? 'text-emerald-400/90 font-medium' : 'text-slate-500'"
          :title="vaultStore.engineStatus.is_running ? '安全加密通道正常运行 (纯内存映射/零磁盘残留)' : '加密会话已休眠'"
        >
          {{ vaultStore.engineStatus.is_running ? '安全加密通道正常运行 (纯内存映射/零磁盘残留)' : '加密会话已休眠' }}
        </span>
      </div>
    </footer>

    <!-- 传输进度弹窗（导入/导出进度条与速度监控） -->
    <TransferProgressModal
      :show="isTransferring"
      :progress="transferProgress"
      @cancel="handleCancelTransfer"
    />

    <!-- 导入成功后的安全提醒弹窗 -->
    <ImportNoticeModal
      :show="showNoticeModal"
      :importedPaths="recentlyImported"
      @close="showNoticeModal = false"
    />

    <!-- 保险箱管理工具抽屉（回收站 / 完整性自检 / 统计 / 审计） -->
    <VaultToolsPanel
      :show="showTools"
      :currentDir="vaultStore.currentDir"
      @close="showTools = false"
      @refresh="onToolsPanelRefresh"
      @busy="toolsBusy = $event"
    />

    <!-- 右键上下文快捷菜单 -->
    <div
      v-if="contextMenu.show"
      id="vault-context-menu"
      class="fixed z-[70] min-w-[180px] bg-slate-900/95 border border-slate-700/80 rounded-xl p-1.5 shadow-2xl backdrop-blur-md animate-fade-in text-xs select-none"
      :style="{ left: `${contextMenu.x}px`, top: `${contextMenu.y}px` }"
      @click="closeContextMenu"
    >
      <!-- 1. 针对条目（单选 / 多选）的右键菜单 -->
      <template v-if="contextMenu.target === 'item'">
        <button
          @click="handleExport"
          :disabled="isExporting || isTransferring || vaultStore.loading"
          class="w-full flex items-center justify-between px-2.5 py-1.5 rounded-lg text-slate-200 hover:bg-blue-600 hover:text-white transition group disabled:opacity-40"
        >
          <span class="flex items-center gap-2">
            <Download class="w-3.5 h-3.5 text-emerald-400 group-hover:text-white" />
            <span>解密导出{{ selectedItems.length > 1 ? ` (${selectedItems.length})` : '' }}</span>
          </span>
        </button>

        <div class="h-px bg-slate-800 my-1"></div>

        <button
          @click="copyToClipboard('cut')"
          :disabled="isTransferring || vaultStore.loading"
          class="w-full flex items-center justify-between px-2.5 py-1.5 rounded-lg text-slate-200 hover:bg-blue-600 hover:text-white transition group disabled:opacity-40"
        >
          <span class="flex items-center gap-2">
            <Scissors class="w-3.5 h-3.5 text-slate-400 group-hover:text-white" />
            <span>剪切</span>
          </span>
          <span class="text-[10px] text-slate-500 group-hover:text-blue-200 font-mono">Ctrl+X</span>
        </button>

        <button
          @click="copyToClipboard('copy')"
          :disabled="isTransferring || vaultStore.loading"
          class="w-full flex items-center justify-between px-2.5 py-1.5 rounded-lg text-slate-200 hover:bg-blue-600 hover:text-white transition group disabled:opacity-40"
        >
          <span class="flex items-center gap-2">
            <Copy class="w-3.5 h-3.5 text-slate-400 group-hover:text-white" />
            <span>复制</span>
          </span>
          <span class="text-[10px] text-slate-500 group-hover:text-blue-200 font-mono">Ctrl+C</span>
        </button>

        <button
          v-if="selectedItems.length === 1"
          @click="handleRename"
          :disabled="isTransferring || vaultStore.loading"
          class="w-full flex items-center justify-between px-2.5 py-1.5 rounded-lg text-slate-200 hover:bg-blue-600 hover:text-white transition group disabled:opacity-40"
        >
          <span class="flex items-center gap-2">
            <Pencil class="w-3.5 h-3.5 text-slate-400 group-hover:text-white" />
            <span>重命名</span>
          </span>
          <span class="text-[10px] text-slate-500 group-hover:text-blue-200 font-mono">F2</span>
        </button>

        <div class="h-px bg-slate-800 my-1"></div>

        <button
          @click="handleRecycle"
          :disabled="isTransferring || vaultStore.loading"
          class="w-full flex items-center justify-between px-2.5 py-1.5 rounded-lg text-red-400 hover:bg-red-600 hover:text-white transition group disabled:opacity-40"
        >
          <span class="flex items-center gap-2">
            <Recycle class="w-3.5 h-3.5 text-red-400 group-hover:text-white" />
            <span>移入回收站</span>
          </span>
          <span class="text-[10px] text-red-400/70 group-hover:text-red-200 font-mono">Delete</span>
        </button>
      </template>

      <!-- 2. 针对空白区域的右键菜单 -->
      <template v-else>
        <button
          @click="handleCreateFolder"
          :disabled="isTransferring || vaultStore.loading"
          class="w-full flex items-center justify-between px-2.5 py-1.5 rounded-lg text-slate-200 hover:bg-blue-600 hover:text-white transition group disabled:opacity-40"
        >
          <span class="flex items-center gap-2">
            <FolderPlus class="w-3.5 h-3.5 text-amber-400 group-hover:text-white" />
            <span>新建目录</span>
          </span>
        </button>

        <button
          @click="handlePaste"
          :disabled="!canPaste() || isTransferring || vaultStore.loading"
          class="w-full flex items-center justify-between px-2.5 py-1.5 rounded-lg text-slate-200 hover:bg-blue-600 hover:text-white transition group disabled:opacity-40"
        >
          <span class="flex items-center gap-2">
            <ClipboardPaste class="w-3.5 h-3.5 text-blue-400 group-hover:text-white" />
            <span>粘贴</span>
          </span>
          <span class="text-[10px] text-slate-500 group-hover:text-blue-200 font-mono">Ctrl+V</span>
        </button>

        <div class="h-px bg-slate-800 my-1"></div>

        <button
          @click="handleImportFiles"
          :disabled="isTransferring || vaultStore.loading"
          class="w-full flex items-center justify-between px-2.5 py-1.5 rounded-lg text-slate-200 hover:bg-blue-600 hover:text-white transition group disabled:opacity-40"
        >
          <span class="flex items-center gap-2">
            <Upload class="w-3.5 h-3.5 text-blue-400 group-hover:text-white" />
            <span>导入文件...</span>
          </span>
        </button>

        <button
          @click="handleImportFolder"
          :disabled="isTransferring || vaultStore.loading"
          class="w-full flex items-center justify-between px-2.5 py-1.5 rounded-lg text-slate-200 hover:bg-blue-600 hover:text-white transition group disabled:opacity-40"
        >
          <span class="flex items-center gap-2">
            <FolderPlus class="w-3.5 h-3.5 text-indigo-400 group-hover:text-white" />
            <span>导入文件夹...</span>
          </span>
        </button>

        <div class="h-px bg-slate-800 my-1"></div>

        <button
          @click="refresh"
          :disabled="isTransferring || vaultStore.loading"
          class="w-full flex items-center justify-between px-2.5 py-1.5 rounded-lg text-slate-200 hover:bg-blue-600 hover:text-white transition group disabled:opacity-40"
        >
          <span class="flex items-center gap-2">
            <RefreshCw class="w-3.5 h-3.5 text-slate-400 group-hover:text-white" />
            <span>刷新</span>
          </span>
        </button>
      </template>
    </div>

    <!-- 名称输入弹窗（新建目录 / 重命名，替代原生 prompt） -->
    <div
      v-if="nameModal"
      class="fixed inset-0 z-[60] flex items-center justify-center bg-black/60 backdrop-blur-sm"
      @click.self="nameModal = null"
    >
      <div class="w-80 rounded-xl border border-slate-700 bg-slate-800 p-5 shadow-2xl">
        <h3 class="text-sm font-semibold text-slate-100">{{ nameModal.title }}</h3>
        <input
          v-model="nameInput"
          autofocus
          class="mt-3 w-full rounded-lg border border-slate-600 bg-slate-900 px-3 py-2 text-sm text-slate-100 outline-none focus:border-cyan-500"
          placeholder="请输入名称"
          @keydown.enter.prevent="submitNameModal"
        />
        <p v-if="nameError" class="mt-1 text-xs text-red-400">{{ nameError }}</p>
        <div class="mt-4 flex justify-end gap-2">
          <button
            class="rounded-lg px-3 py-1.5 text-sm text-slate-300 hover:bg-slate-700"
            @click="nameModal = null"
          >
            取消
          </button>
          <button
            class="rounded-lg bg-cyan-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-cyan-500"
            @click="submitNameModal"
          >
            {{ nameModal.confirmLabel }}
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

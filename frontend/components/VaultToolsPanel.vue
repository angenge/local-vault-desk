<script setup lang="ts">
import { ref, watch, onUnmounted } from 'vue'
import { confirm, message } from '@tauri-apps/plugin-dialog'
import {
  X,
  Recycle,
  ShieldCheck,
  BarChart3,
  ClipboardList,
  RotateCcw,
  Trash2,
  FolderOpen,
  FileWarning,
  RefreshCw,
  HardDriveDownload,
  FileStack,
  CheckCircle2,
  AlertTriangle,
  Folder,
  Cable,
  Copy,
  KeyRound
} from 'lucide-vue-next'
import type { AuditEntry, IntegrityResult, InteropConfig, InteropSelfVerify, RecycleEntry, VaultStats } from '@/types'
import { tauriVault } from '@/services/tauriVault'

const props = defineProps<{
  show: boolean
  currentDir: string
}>()

function uiMessage(text: string) {
  return message(text, { title: '本地文件保险箱' })
}

const emit = defineEmits<{
  (e: 'close'): void
  (e: 'refresh'): void
  (e: 'busy', busy: boolean): void
}>()

const activeTab = ref<'recycle' | 'integrity' | 'stats' | 'audit' | 'interop'>('recycle')

const recycleEntries = ref<RecycleEntry[]>([])
const recycleLoading = ref(false)
const recycleError = ref('')
const emptying = ref(false)
const restoringMap = ref<Record<string, boolean>>({})

const integrityResult = ref<IntegrityResult | null>(null)
const integrityLoading = ref(false)
const integrityError = ref('')

const stats = ref<VaultStats | null>(null)
const statsLoading = ref(false)
const statsError = ref('')

const auditEntries = ref<AuditEntry[]>([])
const auditLoading = ref(false)
const auditError = ref('')

const interopConfig = ref<InteropConfig | null>(null)
const interopLoading = ref(false)
const interopError = ref('')
const interopResult = ref('')
const interopVerifyLoading = ref(false)
const interopVerifyError = ref('')
const interopCopied = ref(false)
const interopSelfLoading = ref(false)
const interopSelfError = ref('')
const interopSelfResult = ref<InteropSelfVerify | null>(null)

watch(
  () => props.show,
  (open) => {
    if (open) {
      loadRecycle()
      loadStats()
      loadAudit()
      startRefreshTimer()
    } else {
      stopRefreshTimer()
    }
  }
)

// 切换选项卡时立即刷新该页数据，避免长时间停留后切回看到陈旧内容
watch(activeTab, (tab) => {
  if (!props.show) return
  if (tab === 'recycle') loadRecycle()
  else if (tab === 'stats') loadStats()
  else if (tab === 'audit') loadAudit()
})

// 抽屉打开期间轻量轮询刷新：回收站（单层列表）与审计日志（读单文件）开销可忽略；
// 统计面板需递归全库，开销较大，仅通过切换页/手动刷新/操作后即时刷新更新。
let refreshTimer: ReturnType<typeof setInterval> | null = null

function startRefreshTimer() {
  stopRefreshTimer()
  refreshTimer = setInterval(() => {
    if (!props.show) return
    if (activeTab.value === 'recycle') loadRecycle()
    else if (activeTab.value === 'audit') loadAudit()
  }, 5000)
}

function stopRefreshTimer() {
  if (refreshTimer) {
    clearInterval(refreshTimer)
    refreshTimer = null
  }
}

onUnmounted(stopRefreshTimer)

async function refreshAll() {
  loadRecycle()
  loadStats()
  loadAudit()
}

function formatSize(bytes: number): string {
  if (bytes === -1 || bytes === undefined) return '-'
  if (bytes === 0) return '0 B'
  const k = 1024
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB']
  const i = Math.floor(Math.log(bytes) / Math.log(k))
  return (bytes / Math.pow(k, i)).toFixed(1) + ' ' + sizes[i]
}

function formatTime(t?: string | number): string {
  if (!t && t !== 0) return '-'
  try {
    return new Date(t).toLocaleString()
  } catch {
    return String(t)
  }
}

function extractError(err: any, fallback: string): string {
  if (typeof err === 'string') return err
  return err?.message || String(err || fallback)
}

async function loadRecycle() {
  recycleLoading.value = true
  recycleError.value = ''
  try {
    recycleEntries.value = await tauriVault.listRecycle()
  } catch (err: any) {
    recycleError.value = extractError(err, '读取回收站失败')
  } finally {
    recycleLoading.value = false
  }
}

async function handleRestore(entry: RecycleEntry) {
  if (restoringMap.value[entry.id]) return
  restoringMap.value[entry.id] = true
  emit('busy', true)
  try {
    await tauriVault.restoreItem(entry.id)
    await loadRecycle()
    loadStats()
    loadAudit()
    emit('refresh')
  } catch (err: any) {
    uiMessage(`恢复失败: ${extractError(err, '恢复失败')}`)
  } finally {
    delete restoringMap.value[entry.id]
    emit('busy', false)
  }
}

async function handleEmptyRecycle() {
  const ok = await confirm('确定要彻底清空回收站吗？所有条目将永久删除且无法恢复！', {
    title: '清空回收站',
    kind: 'warning',
    okLabel: '彻底删除',
    cancelLabel: '取消'
  })
  if (!ok) return
  emptying.value = true
  emit('busy', true)
  try {
    await tauriVault.emptyRecycle()
    await loadRecycle()
    loadStats()
    loadAudit()
    emit('refresh')
  } catch (err: any) {
    const msg = extractError(err, '清空失败')
    if (!msg.includes('取消') && !msg.toLowerCase().includes('cancel')) {
      uiMessage(`清空失败: ${msg}`)
    }
  } finally {
    emptying.value = false
    emit('busy', false)
  }
}

async function runIntegrityCheck(recursive: boolean) {
  integrityLoading.value = true
  integrityError.value = ''
  emit('busy', true)
  try {
    integrityResult.value = await tauriVault.integrityCheck({
      path: recursive ? '' : props.currentDir,
      recursive
    })
  } catch (err: any) {
    const msg = extractError(err, '完整性自检失败')
    if (!msg.includes('取消') && !msg.toLowerCase().includes('cancel')) {
      integrityError.value = msg
    }
  } finally {
    integrityLoading.value = false
    emit('busy', false)
  }
}

async function handleCancelIntegrity() {
  try {
    await tauriVault.cancelTransfer()
  } catch {}
}

async function handleBuildManifest() {
  integrityLoading.value = true
  integrityError.value = ''
  emit('busy', true)
  try {
    const n = await tauriVault.buildManifest()
    await runIntegrityCheck(false)
    uiMessage(`完整性清单已重建，共记录 ${n} 个文件。`)
  } catch (err: any) {
    integrityError.value = extractError(err, '构建清单失败')
  } finally {
    integrityLoading.value = false
    emit('busy', false)
  }
}

async function loadStats() {
  statsLoading.value = true
  statsError.value = ''
  try {
    stats.value = await tauriVault.getStats()
  } catch (err: any) {
    statsError.value = extractError(err, '读取统计失败')
  } finally {
    statsLoading.value = false
  }
}

async function loadAudit() {
  auditLoading.value = true
  auditError.value = ''
  try {
    auditEntries.value = await tauriVault.getAuditLog(100)
  } catch (err: any) {
    auditError.value = extractError(err, '读取审计日志失败')
  } finally {
    auditLoading.value = false
  }
}

async function generateInteropConfig() {
  interopLoading.value = true
  interopError.value = ''
  interopVerifyError.value = ''
  try {
    interopConfig.value = await tauriVault.getInteropConfig()
  } catch (err: any) {
    interopError.value = extractError(err, '生成互通配置失败')
  } finally {
    interopLoading.value = false
  }
}

async function handleCopySnippet() {
  if (!interopConfig.value) return
  try {
    await navigator.clipboard.writeText(interopConfig.value.snippet)
    interopCopied.value = true
    setTimeout(() => (interopCopied.value = false), 1500)
  } catch {
    uiMessage('复制失败，请手动选择复制')
  }
}

async function handleVerifyInterop() {
  if (!interopConfig.value) return
  interopVerifyLoading.value = true
  interopVerifyError.value = ''
  interopResult.value = ''
  try {
    const probe = interopConfig.value.probe_file
    interopResult.value = await tauriVault.verifyInterop(probe)
  } catch (err: any) {
    interopVerifyError.value = extractError(err, '校验失败')
  } finally {
    interopVerifyLoading.value = false
  }
}

async function handleInteropSelfVerify() {
  interopSelfLoading.value = true
  interopSelfError.value = ''
  interopSelfResult.value = null
  try {
    interopSelfResult.value = await tauriVault.interopSelfVerify()
  } catch (err: any) {
    interopSelfError.value = extractError(err, '互通自测失败')
  } finally {
    interopSelfLoading.value = false
  }
}

const ACTION_LABELS: Record<string, string> = {
  vault_init: '创建保险箱',
  unlock: '解锁',
  lock: '锁定',
  import: '导入',
  export: '导出',
  create_dir: '新建目录',
  delete: '删除',
  recycle: '移入回收站',
  restore: '从回收站恢复',
  empty_recycle: '清空回收站',
  rename: '重命名',
  move: '移动',
  copy: '复制',
  manifest_build: '构建完整性清单',
  integrity_check: '完整性自检',
  unlock_fail: '解锁失败'
}

const maxExt = (ext: string) => {
  if (!stats.value) return 0
  const total = totalSize() || 1
  return Math.round(((stats.value.by_extension[ext] || 0) / total) * 100)
}

const totalSize = () => stats.value?.total_size || 0
</script>

<template>
  <div
    v-if="show"
    class="fixed inset-0 z-40 bg-black/50 backdrop-blur-[2px] animate-fade-in"
    @click.self="emit('close')"
  >
    <aside
      class="absolute inset-y-0 right-0 w-[400px] max-w-[90vw] bg-slate-900 border-l border-slate-800 shadow-2xl flex flex-col animate-slide-in-right"
    >
      <!-- 抽屉头部 -->
      <header class="h-14 bg-slate-950 border-b border-slate-800 px-5 flex items-center justify-between shrink-0">
        <div class="flex items-center gap-2.5 min-w-0 flex-1">
          <div class="w-7 h-7 rounded-lg bg-blue-600/20 text-blue-400 flex items-center justify-center shrink-0">
            <ShieldCheck class="w-4 h-4" />
          </div>
          <div class="min-w-0">
            <div class="text-sm font-bold text-white truncate">保险箱管理工具</div>
            <div class="text-[10px] text-slate-500 whitespace-nowrap truncate">回收站 / 自检 / 统计 / 审计 / 互通</div>
          </div>
        </div>
        <button
          @click="refreshAll"
          class="p-1.5 rounded-lg hover:bg-slate-800 text-slate-400 hover:text-white transition shrink-0"
          title="刷新面板数据"
        >
          <RefreshCw class="w-3.5 h-3.5" />
        </button>
        <button
          @click="emit('close')"
          class="p-1.5 rounded-lg hover:bg-slate-800 text-slate-400 hover:text-white transition shrink-0"
        >
          <X class="w-4 h-4" />
        </button>
      </header>

      <!-- 选项卡 -->
      <nav class="flex border-b border-slate-800 text-xs shrink-0 overflow-x-auto">
        <button
          v-for="tab in [
            { key: 'recycle', label: '回收站', icon: Recycle },
            { key: 'integrity', label: '完整性自检', icon: ShieldCheck },
            { key: 'stats', label: '统计面板', icon: BarChart3 },
            { key: 'audit', label: '审计日志', icon: ClipboardList },
            { key: 'interop', label: '互通', icon: Cable }
          ] as const"
          :key="tab.key"
          @click="activeTab = tab.key"
          class="flex-1 flex items-center justify-center gap-1.5 py-2.5 border-b-2 font-medium transition whitespace-nowrap"
          :class="activeTab === tab.key
            ? 'border-blue-500 text-blue-400'
            : 'border-transparent text-slate-500 hover:text-slate-300 hover:bg-slate-800/50'"
        >
          <component :is="tab.icon" class="w-3.5 h-3.5" />
          {{ tab.label }}
        </button>
      </nav>

      <!-- 内容 -->
      <div class="flex-1 overflow-y-auto p-4 space-y-4">
        <!-- ===== 回收站 ===== -->
        <div v-if="activeTab === 'recycle'" class="space-y-3">
          <div class="flex items-center justify-between">
            <span class="text-xs text-slate-400">
              共 {{ recycleEntries.length }} 条
            </span>
            <button
              v-if="recycleEntries.length > 0"
              @click="handleEmptyRecycle"
              :disabled="emptying"
              class="flex items-center gap-1.5 px-2.5 py-1.5 rounded-lg bg-red-500/10 border border-red-500/30 text-red-400 hover:bg-red-500/20 text-[11px] font-medium transition whitespace-nowrap shrink-0 disabled:opacity-50 disabled:cursor-not-allowed"
            >
              <Trash2 class="w-3 h-3" />
              {{ emptying ? '清空中...' : '清空回收站' }}
            </button>
          </div>

          <div v-if="recycleError" class="text-xs text-red-400 bg-red-500/10 border border-red-500/30 rounded-lg p-3">
            {{ recycleError }}
          </div>

          <div
            v-if="!recycleLoading && recycleEntries.length === 0 && !recycleError"
            class="py-10 flex flex-col items-center text-slate-600"
          >
            <Recycle class="w-10 h-10 mb-2" />
            <span class="text-xs">回收站为空</span>
          </div>

          <div v-else class="space-y-2">
            <div
              v-for="entry in recycleEntries"
              :key="entry.id"
              class="bg-slate-800/60 border border-slate-700/50 rounded-xl p-3 hover:border-slate-600 transition"
            >
              <div class="flex items-start justify-between gap-2">
                <div class="flex items-center gap-2.5 min-w-0">
                  <FolderOpen v-if="entry.is_dir" class="w-4 h-4 text-amber-400 shrink-0" />
                  <FileStack v-else class="w-4 h-4 text-slate-400 shrink-0" />
                  <div class="min-w-0">
                    <div class="text-xs font-medium text-white truncate">{{ entry.name }}</div>
                    <div class="text-[10px] text-slate-500 truncate">原位置: {{ entry.original_path || '/' }}</div>
                    <div v-if="entry.deleted_at" class="text-[10px] text-slate-600">{{ formatTime(entry.deleted_at) }}</div>
                  </div>
                </div>
                <button
                  @click="handleRestore(entry)"
                  :disabled="restoringMap[entry.id]"
                  class="flex items-center gap-1 px-2.5 py-1.5 rounded-lg bg-emerald-500/10 border border-emerald-500/30 text-emerald-400 hover:bg-emerald-500/20 text-[11px] font-medium transition shrink-0 disabled:opacity-50 disabled:cursor-not-allowed"
                >
                  <RotateCcw class="w-3 h-3" :class="{ 'animate-spin': restoringMap[entry.id] }" />
                  {{ restoringMap[entry.id] ? '恢复中...' : '恢复' }}
                </button>
              </div>
            </div>
          </div>
        </div>

        <!-- ===== 完整性自检 ===== -->
        <div v-if="activeTab === 'integrity'" class="space-y-3">
          <div class="grid grid-cols-2 gap-2">
            <button
              @click="runIntegrityCheck(false)"
              :disabled="integrityLoading"
              class="flex flex-col items-center gap-1 py-3 rounded-xl bg-slate-800 hover:bg-slate-700 border border-slate-700 text-slate-200 text-xs font-medium transition disabled:opacity-50"
            >
              <Folder class="w-4 h-4 text-blue-400" />
              检查当前目录
              <span class="text-[10px] text-slate-500 font-normal">{{ props.currentDir || '（保险箱根目录）' }}</span>
            </button>
            <button
              @click="runIntegrityCheck(true)"
              :disabled="integrityLoading"
              class="flex flex-col items-center gap-1 py-3 rounded-xl bg-slate-800 hover:bg-slate-700 border border-slate-700 text-slate-200 text-xs font-medium transition disabled:opacity-50"
            >
              <HardDriveDownload class="w-4 h-4 text-blue-400" />
              检查全库
              <span class="text-[10px] text-slate-500 font-normal">递归全部文件</span>
            </button>
          </div>

          <button
            @click="handleBuildManifest"
            :disabled="integrityLoading"
            class="w-full flex items-center justify-center gap-1.5 py-2 rounded-lg bg-slate-800 hover:bg-slate-700 border border-slate-700 text-slate-300 text-xs font-medium transition disabled:opacity-50"
          >
            <RefreshCw class="w-3.5 h-3.5" :class="{ 'animate-spin': integrityLoading }" />
            重建完整性清单（记录当前所有文件指纹）
          </button>

          <div v-if="integrityError" class="text-xs text-red-400 bg-red-500/10 border border-red-500/30 rounded-lg p-3">
            {{ integrityError }}
          </div>

          <div v-if="integrityLoading" class="flex flex-col items-center justify-center py-4 space-y-3">
            <div class="flex items-center gap-2 text-xs text-slate-400">
              <RefreshCw class="w-3.5 h-3.5 animate-spin text-blue-400" />
              <span>正在逐文件校验 SHA1 指纹...</span>
            </div>
            <button
              type="button"
              @click="handleCancelIntegrity"
              class="px-3 py-1.5 rounded-lg bg-slate-800 hover:bg-red-500/20 border border-slate-700 hover:border-red-500/40 text-slate-300 hover:text-red-400 text-xs font-medium transition"
            >
              <X class="w-3 h-3 inline mr-1" />
              取消自检
            </button>
          </div>

          <div v-if="integrityResult" class="space-y-3">
            <div
              class="rounded-xl border p-4"
              :class="integrityResult.differ === 0 && integrityResult.missing === 0
                ? 'bg-emerald-500/10 border-emerald-500/30'
                : 'bg-red-500/10 border-red-500/30'"
            >
              <div class="flex items-center gap-2 text-sm font-semibold mb-3"
                :class="integrityResult.differ === 0 && integrityResult.missing === 0 ? 'text-emerald-400' : 'text-red-400'">
                <CheckCircle2 v-if="integrityResult.differ === 0 && integrityResult.missing === 0" class="w-4 h-4" />
                <AlertTriangle v-else class="w-4 h-4" />
                {{
                  integrityResult.differ === 0 && integrityResult.missing === 0
                    ? '数据完整，未发现损坏'
                    : '发现数据异常！'
                }}
              </div>
              <div class="grid grid-cols-4 gap-2 text-center">
                <div class="bg-slate-900/60 rounded-lg py-2">
                  <div class="text-lg font-bold text-white">{{ integrityResult.checked }}</div>
                  <div class="text-[10px] text-slate-500">已检查</div>
                </div>
                <div class="bg-slate-900/60 rounded-lg py-2">
                  <div class="text-lg font-bold text-emerald-400">{{ integrityResult.ok }}</div>
                  <div class="text-[10px] text-slate-500">正常</div>
                </div>
                <div class="bg-slate-900/60 rounded-lg py-2">
                  <div class="text-lg font-bold text-red-400">{{ integrityResult.differ }}</div>
                  <div class="text-[10px] text-slate-500">差异</div>
                </div>
                <div class="bg-slate-900/60 rounded-lg py-2">
                  <div class="text-lg font-bold text-amber-400">{{ integrityResult.missing }}</div>
                  <div class="text-[10px] text-slate-500">缺失</div>
                </div>
              </div>
              <div v-if="!integrityResult.has_manifest" class="mt-2 text-center text-[11px] text-slate-500">
                尚未建立完整性清单，请先“重建完整性清单”
              </div>
            </div>

            <div v-if="integrityResult.issues.length > 0" class="space-y-2">
              <div class="text-xs font-semibold text-slate-400">异常文件：</div>
              <div
                v-for="(issue, idx) in integrityResult.issues"
                :key="idx"
                class="flex items-center gap-2 bg-slate-800/60 border border-slate-700/50 rounded-lg px-3 py-2"
              >
                <FileWarning class="w-4 h-4 text-red-400 shrink-0" />
                <span class="text-xs text-slate-300 truncate flex-1">{{ issue.path }}</span>
                <span
                  class="text-[10px] font-bold px-2 py-0.5 rounded"
                  :class="issue.status === 'MISSING' ? 'bg-amber-500/20 text-amber-400' : 'bg-red-500/20 text-red-400'"
                >
                  {{ issue.status }}
                </span>
              </div>
            </div>
          </div>
        </div>

        <!-- ===== 统计面板 ===== -->
        <div v-if="activeTab === 'stats'" class="space-y-3">
          <div v-if="statsError" class="text-xs text-red-400 bg-red-500/10 border border-red-500/30 rounded-lg p-3">
            {{ statsError }}
          </div>

          <div v-if="stats" class="grid grid-cols-3 gap-2">
            <div class="bg-slate-800/60 border border-slate-700/50 rounded-xl p-3 text-center">
              <div class="text-xl font-bold text-white">{{ stats.files }}</div>
              <div class="text-[10px] text-slate-500 mt-0.5">文件数</div>
            </div>
            <div class="bg-slate-800/60 border border-slate-700/50 rounded-xl p-3 text-center">
              <div class="text-xl font-bold text-amber-400">{{ stats.dirs }}</div>
              <div class="text-[10px] text-slate-500 mt-0.5">目录数</div>
            </div>
            <div class="bg-slate-800/60 border border-slate-700/50 rounded-xl p-3 text-center">
              <div class="text-xl font-bold text-blue-400">{{ formatSize(stats.total_size) }}</div>
              <div class="text-[10px] text-slate-500 mt-0.5">总大小</div>
            </div>
          </div>

          <div v-if="stats && Object.keys(stats.by_extension).length > 0" class="space-y-2">
            <div class="text-xs font-semibold text-slate-400">文件类型分布：</div>
            <div
              v-for="ext in Object.keys(stats.by_extension)"
              :key="ext"
              class="bg-slate-800/60 border border-slate-700/50 rounded-xl p-3"
            >
              <div class="flex items-center justify-between text-xs mb-1.5">
                <span class="text-slate-300 font-medium flex items-center gap-1.5">
                  <span class="px-1.5 py-0.5 rounded bg-slate-700/70 text-slate-300 font-mono text-[10px]">{{ ext }}</span>
                  <span class="text-[10px] text-slate-500">{{ stats.by_extension_count[ext] }} 个文件</span>
                </span>
                <span class="text-slate-400">{{ formatSize(stats.by_extension[ext]) }}</span>
              </div>
              <div class="h-1.5 bg-slate-900 rounded-full overflow-hidden">
                <div
                  class="h-full bg-blue-500 rounded-full transition-all"
                  :style="{ width: maxExt(ext) + '%' }"
                ></div>
              </div>
              <div class="text-right text-[10px] text-slate-600 mt-1">{{ maxExt(ext) }}%</div>
            </div>
          </div>
        </div>

        <!-- ===== 审计日志 ===== -->
        <div v-if="activeTab === 'audit'" class="space-y-3">
          <div v-if="auditError" class="text-xs text-red-400 bg-red-500/10 border border-red-500/30 rounded-lg p-3">
            {{ auditError }}
          </div>

          <div v-if="!auditLoading && auditEntries.length === 0 && !auditError" class="py-10 flex flex-col items-center text-slate-600">
            <ClipboardList class="w-10 h-10 mb-2" />
            <span class="text-xs">暂无审计记录</span>
          </div>

          <div v-else class="space-y-2">
            <div
              v-for="(entry, idx) in auditEntries"
              :key="idx"
              class="bg-slate-800/60 border border-slate-700/50 rounded-xl p-3"
            >
              <div class="flex items-center justify-between mb-1">
                <span class="text-[11px] font-medium text-blue-400">
                  {{ ACTION_LABELS[entry.action] || entry.action }}
                </span>
                <span class="text-[10px] text-slate-500 font-mono">{{ formatTime(entry.ts) }}</span>
              </div>
              <div class="text-[11px] text-slate-400">{{ entry.detail }}</div>
              <div class="text-[10px] text-slate-600 mt-1">{{ entry.username }}</div>
            </div>
          </div>
        </div>

        <!-- ===== 外部 rclone 互通 ===== -->
        <div v-if="activeTab === 'interop'" class="space-y-3">
          <div class="bg-blue-500/10 border border-blue-500/30 rounded-xl p-3 text-[11px] text-slate-300 leading-relaxed">
            用同一账户密码在任意机器上派生的加密密钥是一致的。把下面生成的 <span class="text-blue-400 font-mono">[interop_crypt]</span> 配置贴到外部 <span class="font-mono">rclone.conf</span> 后，标准 rclone 即可直接读写本保险箱；也可在本页校验「外部 rclone 写入的文件能否被本保险箱解密」。
          </div>

          <button
            @click="generateInteropConfig"
            :disabled="interopLoading"
            class="w-full flex items-center justify-center gap-1.5 px-3 py-2 rounded-lg bg-cyan-600 hover:bg-cyan-500 text-white text-xs font-medium transition disabled:opacity-50 whitespace-nowrap"
          >
            <KeyRound class="w-3.5 h-3.5" />
            {{ interopConfig ? '重新生成互通配置' : '生成互通配置' }}
          </button>

          <div v-if="interopError" class="text-xs text-red-400 bg-red-500/10 border border-red-500/30 rounded-lg p-3">
            {{ interopError }}
          </div>

          <div v-if="interopConfig" class="space-y-3">
            <div class="bg-slate-800/60 border border-slate-700/50 rounded-xl p-3 space-y-2">
              <div class="flex items-center justify-between">
                <span class="text-xs font-semibold text-slate-400">rclone 配置段（含密钥，请勿外泄）</span>
                <button
                  @click="handleCopySnippet"
                  class="flex items-center gap-1 px-2 py-1 rounded-lg bg-slate-700/60 hover:bg-slate-700 text-slate-300 hover:text-white text-[10px] font-medium transition shrink-0 whitespace-nowrap"
                >
                  <Copy class="w-3 h-3" />
                  {{ interopCopied ? '已复制' : '复制' }}
                </button>
              </div>
              <pre class="text-[10px] font-mono text-slate-200 bg-slate-950/80 border border-slate-700/50 rounded-lg p-3 whitespace-pre-wrap break-all overflow-x-auto max-h-56 overflow-y-auto">{{ interopConfig.snippet }}</pre>
            </div>

            <div class="bg-slate-800/60 border border-slate-700/50 rounded-xl p-3 text-[11px] text-slate-400 leading-relaxed whitespace-pre-line">
              {{ interopConfig.instructions }}
            </div>

            <div class="border-t border-slate-800 pt-3 space-y-2">
              <div class="text-xs font-semibold text-slate-300">一键自测互通</div>
              <p class="text-[11px] text-slate-400 leading-relaxed">
                无需外部 rclone：本应用会用与导出配置完全等价的标准 crypt 独立后端模拟一次外部写入，再解密读回，直接验证导出配置与本保险箱互通可用。
              </p>
              <button
                @click="handleInteropSelfVerify"
                :disabled="interopSelfLoading"
                class="w-full flex items-center justify-center gap-1.5 px-3 py-2 rounded-lg bg-cyan-700 hover:bg-cyan-600 text-white text-xs font-medium transition disabled:opacity-50 whitespace-nowrap"
              >
                <Cable class="w-3.5 h-3.5" />
                {{ interopSelfLoading ? '自测中...' : '一键自测互通' }}
              </button>

              <div v-if="interopSelfError" class="text-xs text-red-400 bg-red-500/10 border border-red-500/30 rounded-lg p-3 whitespace-pre-line">
                {{ interopSelfError }}
              </div>

              <div
                v-if="interopSelfResult"
                class="flex items-start gap-2 text-xs text-emerald-400 bg-emerald-500/10 border border-emerald-500/30 rounded-lg p-3"
              >
                <CheckCircle2 class="w-4 h-4 mt-0.5 shrink-0" />
                <div class="space-y-1">
                  <div class="font-medium">自测通过：模拟外部写入的内容已被本保险箱成功解密读回。</div>
                  <div class="text-slate-300 break-all">写入：{{ interopSelfResult.written }}</div>
                  <div class="text-slate-300 break-all">读回：{{ interopSelfResult.read_back }}</div>
                </div>
              </div>
            </div>

            <div class="border-t border-slate-800 pt-3 space-y-2">
              <div class="text-xs font-semibold text-slate-400">校验互通</div>
              <button
                @click="handleVerifyInterop"
                :disabled="interopVerifyLoading"
                class="w-full flex items-center justify-center gap-1.5 px-3 py-2 rounded-lg bg-emerald-600 hover:bg-emerald-500 text-white text-xs font-medium transition disabled:opacity-50 whitespace-nowrap"
              >
                <Cable class="w-3.5 h-3.5" />
                {{ interopVerifyLoading ? '正在校验...' : '校验互通' }}
              </button>

              <div v-if="interopVerifyError" class="text-xs text-amber-400 bg-amber-500/10 border border-amber-500/30 rounded-lg p-3 whitespace-pre-line">
                {{ interopVerifyError }}
              </div>

              <div
                v-if="interopResult"
                class="flex items-start gap-2 text-xs text-emerald-400 bg-emerald-500/10 border border-emerald-500/30 rounded-lg p-3"
              >
                <CheckCircle2 class="w-4 h-4 mt-0.5 shrink-0" />
                <div class="space-y-1">
                  <div class="font-medium">互通验证通过：外部 rclone 写入的文件可被本保险箱解密。</div>
                  <div class="text-slate-300 break-all">解密内容：{{ interopResult }}</div>
                </div>
              </div>
            </div>
          </div>
        </div>
      </div>
    </aside>
  </div>
</template>

<style scoped>
@keyframes slide-in-right {
  from {
    transform: translateX(100%);
  }
  to {
    transform: translateX(0);
  }
}
.animate-slide-in-right {
  animation: slide-in-right 0.25s ease-out;
}
</style>
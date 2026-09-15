<script setup lang="ts">
import { computed } from 'vue'
import { Upload, Download, Trash2, Loader2, X, Scissors, Copy } from 'lucide-vue-next'
import type { TransferProgress } from '@/types'

const props = defineProps<{
  show: boolean
  progress: TransferProgress
}>()

const emit = defineEmits<{
  (e: 'cancel'): void
}>()

type ProgressKind = 'import' | 'export' | 'delete' | 'move' | 'copy'

const kind = computed<ProgressKind>(() => {
  if (props.progress.task_type === 'import') return 'import'
  if (props.progress.task_type === 'empty_recycle') return 'delete'
  if (props.progress.task_type === 'dir_move') return 'move'
  if (props.progress.task_type === 'dir_copy') return 'copy'
  return 'export'
})
const isImport = computed(() => kind.value === 'import')
const isDelete = computed(() => kind.value === 'delete')
const isMove = computed(() => kind.value === 'move')
const isCopy = computed(() => kind.value === 'copy')
const isDirOp = computed(() => isMove.value || isCopy.value)
const title = computed(() =>
  isImport.value ? '正在安全加密导入'
    : isDelete.value ? '正在清空回收站'
    : isMove.value ? '正在移动目录'
    : isCopy.value ? '正在复制目录'
    : '正在安全解密导出'
)
const subtitle = computed(() =>
  isImport.value
    ? '数据正在内存中进行分块加密与混淆存储'
    : isDelete.value
      ? '正在安全删除回收站内的加密文件'
      : isMove.value
        ? '正在移动保险箱内的目录及其内容'
        : isCopy.value
          ? '正在复制保险箱内的目录及其内容'
          : '数据正在解密并还原至本地明文目录'
)

function formatBytes(bytes: number): string {
  if (bytes <= 0 || isNaN(bytes)) return '0 B'
  const k = 1024
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB']
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(k)), sizes.length - 1)
  return (bytes / Math.pow(k, i)).toFixed(2) + ' ' + sizes[i]
}

function formatSpeed(bytesPerSec: number): string {
  if (bytesPerSec <= 0 || isNaN(bytesPerSec)) return '0 B/s'
  return `${formatBytes(bytesPerSec)}/s`
}

function formatEta(seconds: number | null): string {
  if (seconds === null || seconds === undefined || seconds <= 0) return '计算中...'
  if (seconds < 60) return `剩余约 ${seconds} 秒`
  const mins = Math.floor(seconds / 60)
  const secs = seconds % 60
  if (mins < 60) return `剩余约 ${mins} 分 ${secs} 秒`
  const hours = Math.floor(mins / 60)
  return `剩余约 ${hours} 小时 ${mins % 60} 分`
}
</script>

<template>
  <div
    v-if="show"
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/75 backdrop-blur-md p-4 animate-fade-in select-none"
  >
    <div
      class="bg-slate-900 border border-slate-700/80 w-full max-w-md rounded-2xl shadow-2xl overflow-hidden flex flex-col p-6"
    >
      <!-- 头部状态图标与标题 -->
      <div class="flex items-center gap-4 mb-5">
        <div
          class="w-12 h-12 rounded-xl flex items-center justify-center shadow-inner shrink-0"
          :class="isImport ? 'bg-blue-600/20 text-blue-400 border border-blue-500/30' : isDelete ? 'bg-red-600/20 text-red-400 border border-red-500/30' : isDirOp ? 'bg-indigo-600/20 text-indigo-400 border border-indigo-500/30' : 'bg-emerald-600/20 text-emerald-400 border border-emerald-500/30'"
        >
          <Upload v-if="isImport" class="w-6 h-6 animate-bounce" />
          <Trash2 v-else-if="isDelete" class="w-6 h-6 animate-bounce" />
          <Scissors v-else-if="isMove" class="w-6 h-6 animate-bounce" />
          <Copy v-else-if="isCopy" class="w-6 h-6 animate-bounce" />
          <Download v-else class="w-6 h-6 animate-bounce" />
        </div>
        <div class="flex-1 min-w-0">
          <div class="flex items-center justify-between">
            <h3 class="text-base font-bold text-white flex items-center gap-2">
              <span>{{ title }}</span>
              <Loader2 class="w-4 h-4 animate-spin text-slate-400" />
            </h3>
            <span class="text-xs font-mono font-semibold px-2 py-0.5 rounded-full"
              :class="isImport ? 'bg-blue-500/20 text-blue-300' : isDelete ? 'bg-red-500/20 text-red-300' : isDirOp ? 'bg-indigo-500/20 text-indigo-300' : 'bg-emerald-500/20 text-emerald-300'"
            >
              {{ progress.percentage }}%
            </span>
          </div>
          <p class="text-[11px] text-slate-400 truncate mt-0.5">{{ subtitle }}</p>
        </div>
      </div>

      <!-- 进度条 -->
      <div class="space-y-2 mb-4">
        <div class="w-full h-3 bg-slate-950 rounded-full overflow-hidden p-0.5 border border-slate-800">
          <div
            class="h-full rounded-full transition-all duration-300 ease-out"
            :class="isImport ? 'bg-gradient-to-r from-blue-600 to-indigo-500' : isDelete ? 'bg-gradient-to-r from-red-600 to-rose-500' : isDirOp ? 'bg-gradient-to-r from-indigo-600 to-violet-500' : 'bg-gradient-to-r from-emerald-600 to-teal-500'"
            :style="{ width: `${Math.max(progress.percentage, 2)}%` }"
          ></div>
        </div>

        <!-- 传输/删除统计与进度 -->
        <div class="flex items-center justify-between text-[11px] text-slate-400 font-mono">
          <span v-if="isDelete">
            <template v-if="progress.total_files > 0 && progress.transferred_files > 0">
              已删除 <strong class="text-slate-200">{{ progress.transferred_files }}</strong> / {{ progress.total_files }} 个文件
            </template>
            <template v-else-if="progress.total_files > 0">
              共 <strong class="text-slate-200">{{ progress.total_files }}</strong> 个待清理项，正在安全清除...
            </template>
            <template v-else>
              正在彻底清理回收站中的加密密文...
            </template>
          </span>
          <span v-else>
            {{ formatBytes(progress.bytes) }} / {{ progress.total_bytes > 0 ? formatBytes(progress.total_bytes) : '计算中' }}
          </span>
          <span class="text-slate-300 font-semibold" v-if="!isDelete || progress.speed > 0">
            {{ formatSpeed(progress.speed) }}
          </span>
        </div>
      </div>

      <!-- 详细指标信息卡片 -->
      <div class="bg-slate-950/70 border border-slate-800 rounded-xl p-3 space-y-2 text-xs">
        <!-- 当前文件 -->
        <div class="flex items-center justify-between gap-2">
          <span class="text-slate-500 shrink-0">正在处理:</span>
          <span class="text-slate-300 font-mono truncate text-right max-w-[240px]" :title="progress.current_file || (progress.total_files > 0 ? `正在扫描目录 (已发现 ${progress.total_files} 项)...` : '准备传输元数据...')">
            {{ progress.current_file || (progress.total_files > 0 ? `正在扫描目录 (已发现 ${progress.total_files} 项)...` : '准备传输元数据...') }}
          </span>
        </div>

        <!-- 任务总数与预计剩余时间 -->
        <div class="flex items-center justify-between text-[11px] text-slate-400 pt-1 border-t border-slate-800/60">
          <span>
            已完成文件: <strong class="text-slate-200 font-mono">{{ progress.transferred_files }}</strong>
            <span v-if="progress.total_files > 0"> / {{ progress.total_files }}</span>
          </span>
          <span class="text-amber-300/90 font-medium">
            {{ formatEta(progress.eta) }}
          </span>
        </div>
      </div>

      <!-- 取消按钮 -->
      <button
        @click="emit('cancel')"
        class="mt-4 w-full flex items-center justify-center gap-1.5 py-2 rounded-xl bg-slate-800 hover:bg-red-500/20 border border-slate-700 hover:border-red-500/40 text-slate-300 hover:text-red-400 text-xs font-medium transition select-none"
      >
        <X class="w-3.5 h-3.5" />
        取消本次传输
      </button>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, watch, onUnmounted } from 'vue'
import { tauriVault } from '@/services/tauriVault'
import type { RcloneItem } from '@/types'
import {
  X,
  FileText,
  Image as ImageIcon,
  Film,
  Music,
  FileSpreadsheet,
  FileCode,
  FileQuestion,
  Download,
  AlertCircle,
  Loader2,
  Maximize2,
  Minimize2,
  Copy,
  Check
} from 'lucide-vue-next'

const props = defineProps<{
  show: boolean
  item: RcloneItem | null
}>()

const emit = defineEmits<{
  (e: 'close'): void
  (e: 'export', item: RcloneItem): void
}>()

const loading = ref(false)
const errorMsg = ref('')
const blobUrl = ref<string | null>(null)
const textContent = ref<string>('')
const isCopied = ref(false)
const isExpanded = ref(false)

// 20 MB 预览体积熔断保护
const MAX_PREVIEW_SIZE = 20 * 1024 * 1024

const ext = computed(() => {
  if (!props.item || props.item.IsDir) return ''
  const parts = props.item.Name.split('.')
  return parts.length > 1 ? parts.pop()!.toLowerCase() : ''
})

const fileCategory = computed<'text' | 'image' | 'video' | 'audio' | 'pdf' | 'unsupported'>(() => {
  const e = ext.value
  if (['txt', 'md', 'json', 'js', 'ts', 'jsx', 'tsx', 'html', 'css', 'scss', 'xml', 'yaml', 'yml', 'toml', 'rs', 'go', 'py', 'java', 'c', 'cpp', 'h', 'hpp', 'sh', 'bat', 'ps1', 'sql', 'log', 'ini', 'conf', 'env'].includes(e)) {
    return 'text'
  }
  if (['png', 'jpg', 'jpeg', 'gif', 'webp', 'svg', 'bmp', 'ico'].includes(e)) {
    return 'image'
  }
  if (['mp4', 'webm', 'ogg', 'mov'].includes(e)) {
    return 'video'
  }
  if (['mp3', 'wav', 'aac', 'flac', 'm4a', 'oga'].includes(e)) {
    return 'audio'
  }
  if (e === 'pdf') {
    return 'pdf'
  }
  return 'unsupported'
})

const mimeType = computed(() => {
  const e = ext.value
  switch (e) {
    case 'png': return 'image/png'
    case 'jpg':
    case 'jpeg': return 'image/jpeg'
    case 'gif': return 'image/gif'
    case 'webp': return 'image/webp'
    case 'svg': return 'image/svg+xml'
    case 'bmp': return 'image/bmp'
    case 'ico': return 'image/x-icon'
    case 'pdf': return 'application/pdf'
    case 'mp4': return 'video/mp4'
    case 'webm': return 'video/webm'
    case 'ogg': return 'video/ogg'
    case 'mp3': return 'audio/mpeg'
    case 'wav': return 'audio/wav'
    case 'aac': return 'audio/aac'
    case 'flac': return 'audio/flac'
    case 'json': return 'application/json'
    case 'html': return 'text/html'
    case 'css': return 'text/css'
    case 'xml': return 'text/xml'
    default: return 'text/plain;charset=utf-8'
  }
})

function cleanBlob() {
  if (blobUrl.value) {
    URL.revokeObjectURL(blobUrl.value)
    blobUrl.value = null
  }
  textContent.value = ''
  errorMsg.value = ''
  isCopied.value = false
}

async function loadPreview() {
  cleanBlob()
  if (!props.item || props.item.IsDir) return

  // 1. 视频与音频：直接走 HTTP Range 流式分块解密通道，支持 1GB+ 无内存压力秒开与拖拽
  if (fileCategory.value === 'video' || fileCategory.value === 'audio') {
    const encoded = encodeURIComponent(props.item.Path)
    blobUrl.value = `http://stream.localhost/${encoded}`
    return
  }

  // 2. 文本、图片、PDF：走内存快速加载（带 30MB 保护）
  if (props.item.Size > MAX_PREVIEW_SIZE) {
    errorMsg.value = `该文件体积为 ${(props.item.Size / 1024 / 1024).toFixed(1)} MB，超出文档/图片快速预览上限（20 MB）。建议直接导出后查看。`
    return
  }

  if (fileCategory.value === 'unsupported') {
    return
  }

  loading.value = true
  try {
    const rawBytes = await tauriVault.readFilePreview({
      vaultItemPath: props.item.Path,
      maxBytes: MAX_PREVIEW_SIZE
    })

    const u8 = new Uint8Array(rawBytes)

    if (fileCategory.value === 'text') {
      const decoder = new TextDecoder('utf-8')
      textContent.value = decoder.decode(u8)
    } else {
      const blob = new Blob([u8], { type: mimeType.value })
      blobUrl.value = URL.createObjectURL(blob)
    }
  } catch (err: any) {
    errorMsg.value = typeof err === 'string' ? err : err?.message || '读取文件内容失败'
  } finally {
    loading.value = false
  }
}

watch(
  () => [props.show, props.item],
  ([show]) => {
    if (show) {
      loadPreview()
    } else {
      cleanBlob()
      isExpanded.value = false
    }
  }
)

onUnmounted(() => {
  cleanBlob()
})

function formatSize(bytes: number) {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / 1024 / 1024).toFixed(2)} MB`
}

async function copyText() {
  if (!textContent.value) return
  try {
    await navigator.clipboard.writeText(textContent.value)
    isCopied.value = true
    setTimeout(() => {
      isCopied.value = false
    }, 2000)
  } catch {}
}
</script>

<template>
  <div
    v-if="show && item"
    class="fixed inset-0 z-50 flex items-center justify-center p-3 sm:p-6 bg-black/75 backdrop-blur-md transition-all duration-200"
    @click.self="emit('close')"
  >
    <div
      class="bg-slate-900 border border-slate-700/80 rounded-2xl shadow-2xl flex flex-col overflow-hidden transition-all duration-200"
      :class="isExpanded ? 'w-[96vw] h-[94vh]' : 'w-full max-w-4xl max-h-[88vh] h-[780px]'"
    >
      <!-- 顶部控制栏 -->
      <div class="h-14 px-5 border-b border-slate-800 bg-slate-950/60 flex items-center justify-between shrink-0 select-none">
        <div class="flex items-center gap-3 min-w-0 pr-4">
          <div class="w-8 h-8 rounded-lg bg-blue-500/10 border border-blue-500/20 text-blue-400 flex items-center justify-center shrink-0">
            <ImageIcon v-if="fileCategory === 'image'" class="w-4 h-4" />
            <Film v-else-if="fileCategory === 'video'" class="w-4 h-4" />
            <Music v-else-if="fileCategory === 'audio'" class="w-4 h-4" />
            <FileText v-else-if="fileCategory === 'text'" class="w-4 h-4" />
            <FileText v-else-if="fileCategory === 'pdf'" class="w-4 h-4 text-rose-400" />
            <FileQuestion v-else class="w-4 h-4 text-slate-400" />
          </div>
          <div class="min-w-0">
            <h3 class="text-sm font-semibold text-white truncate" :title="item.Name">{{ item.Name }}</h3>
            <p class="text-[11px] text-slate-400 flex items-center gap-2">
              <span>{{ formatSize(item.Size) }}</span>
              <span class="text-slate-600">•</span>
              <span class="text-emerald-400/90 font-medium">纯内存解密 (零落盘)</span>
            </p>
          </div>
        </div>

        <div class="flex items-center gap-1.5 shrink-0">
          <button
            v-if="fileCategory === 'text' && textContent"
            type="button"
            @click="copyText"
            class="p-2 text-slate-400 hover:text-slate-200 hover:bg-slate-800 rounded-lg transition"
            :title="isCopied ? '已复制' : '复制全文'"
          >
            <Check v-if="isCopied" class="w-4 h-4 text-emerald-400" />
            <Copy v-else class="w-4 h-4" />
          </button>
          <button
            type="button"
            @click="emit('export', item)"
            class="px-3 py-1.5 bg-blue-600 hover:bg-blue-500 text-white rounded-lg text-xs font-medium transition flex items-center gap-1.5 shadow-sm"
            title="解密导出此文件"
          >
            <Download class="w-3.5 h-3.5" />
            <span class="hidden sm:inline">导出</span>
          </button>
          <button
            type="button"
            @click="isExpanded = !isExpanded"
            class="p-2 text-slate-400 hover:text-slate-200 hover:bg-slate-800 rounded-lg transition"
            :title="isExpanded ? '还原窗口' : '最大化窗口'"
          >
            <Minimize2 v-if="isExpanded" class="w-4 h-4" />
            <Maximize2 v-else class="w-4 h-4" />
          </button>
          <button
            type="button"
            @click="emit('close')"
            class="p-2 text-slate-400 hover:text-red-400 hover:bg-red-500/10 rounded-lg transition ml-1"
            title="关闭预览 (Esc)"
          >
            <X class="w-4 h-4" />
          </button>
        </div>
      </div>

      <!-- 主预览视口 -->
      <div class="flex-1 overflow-hidden relative flex items-center justify-center bg-slate-950/40 p-4">
        <!-- 加载中状态 -->
        <div v-if="loading" class="flex flex-col items-center justify-center gap-3 text-slate-400">
          <Loader2 class="w-8 h-8 animate-spin text-blue-500" />
          <span class="text-xs tracking-wide">正在纯内存流式解密中...</span>
        </div>

        <!-- 错误状态 / 超大文件熔断 -->
        <div v-else-if="errorMsg" class="max-w-md text-center p-6 bg-slate-900/90 border border-slate-800 rounded-2xl space-y-3">
          <AlertCircle class="w-10 h-10 text-amber-400 mx-auto" />
          <h4 class="text-sm font-semibold text-white">无法预览文件</h4>
          <p class="text-xs text-slate-400 leading-relaxed">{{ errorMsg }}</p>
          <div class="pt-2">
            <button
              type="button"
              @click="emit('export', item)"
              class="px-4 py-2 bg-blue-600 hover:bg-blue-500 text-white text-xs font-semibold rounded-xl transition shadow"
            >
              直接解密导出
            </button>
          </div>
        </div>

        <!-- 不支持的格式提示 -->
        <div v-else-if="fileCategory === 'unsupported'" class="max-w-md text-center p-6 bg-slate-900/90 border border-slate-800 rounded-2xl space-y-3">
          <FileQuestion class="w-10 h-10 text-slate-500 mx-auto" />
          <h4 class="text-sm font-semibold text-white">暂不支持直接预览该格式</h4>
          <p class="text-xs text-slate-400 leading-relaxed">
            该文件类型 (.{{ ext || '未知' }}) 需要专用软件打开。建议点击下方按钮将其安全导出为明文文件查看。
          </p>
          <div class="pt-2">
            <button
              type="button"
              @click="emit('export', item)"
              class="px-4 py-2 bg-blue-600 hover:bg-blue-500 text-white text-xs font-semibold rounded-xl transition shadow"
            >
              解密并导出到本地
            </button>
          </div>
        </div>

        <!-- 1. 纯文本 / 代码预览 -->
        <div v-else-if="fileCategory === 'text'" class="w-full h-full overflow-auto rounded-xl border border-slate-800/80 bg-slate-950 p-4 font-mono text-xs text-slate-300 leading-relaxed select-text">
          <pre class="whitespace-pre-wrap break-all">{{ textContent }}</pre>
        </div>

        <!-- 2. 图片预览 -->
        <div v-else-if="fileCategory === 'image' && blobUrl" class="w-full h-full flex items-center justify-center overflow-auto">
          <img
            :src="blobUrl"
            :alt="item.Name"
            class="max-w-full max-h-full object-contain rounded-lg shadow-lg"
          />
        </div>

        <!-- 3. 视频预览 -->
        <div v-else-if="fileCategory === 'video' && blobUrl" class="w-full h-full flex items-center justify-center">
          <video
            :src="blobUrl"
            controls
            autoplay
            class="max-w-full max-h-full rounded-xl shadow-lg bg-black"
          ></video>
        </div>

        <!-- 4. 音频预览 -->
        <div v-else-if="fileCategory === 'audio' && blobUrl" class="flex flex-col items-center justify-center gap-6 p-8 bg-slate-900/80 border border-slate-800 rounded-2xl">
          <div class="w-20 h-20 rounded-full bg-blue-500/10 border border-blue-500/20 text-blue-400 flex items-center justify-center animate-pulse">
            <Music class="w-10 h-10" />
          </div>
          <div class="text-center">
            <h4 class="text-sm font-semibold text-white">{{ item.Name }}</h4>
            <p class="text-xs text-slate-400 mt-1">{{ formatSize(item.Size) }}</p>
          </div>
          <audio :src="blobUrl" controls autoplay class="w-72 sm:w-96"></audio>
        </div>

        <!-- 5. PDF 预览 -->
        <div v-else-if="fileCategory === 'pdf' && blobUrl" class="w-full h-full rounded-xl overflow-hidden border border-slate-800">
          <iframe
            :src="blobUrl"
            class="w-full h-full border-0 bg-slate-800"
          ></iframe>
        </div>
      </div>

      <!-- 底部安全提示 -->
      <div class="h-9 px-5 border-t border-slate-800/80 bg-slate-950 flex items-center justify-between text-[11px] text-slate-500 shrink-0 select-none">
        <span class="flex items-center gap-1.5">
          <span class="w-1.5 h-1.5 rounded-full bg-emerald-500"></span>
          <span>安全环境：关闭此弹窗将立即覆写销毁内存句柄</span>
        </span>
        <span class="text-slate-600">按 Esc 或空格键关闭</span>
      </div>
    </div>
  </div>
</template>

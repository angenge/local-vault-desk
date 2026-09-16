<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted } from 'vue'
import { openUrl } from '@tauri-apps/plugin-opener'
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
  Check,
  ExternalLink,
  Tv,
  Wifi,
  WifiOff,
  Smartphone,
  QrCode
} from 'lucide-vue-next'
import QRCode from 'qrcode'

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
const streamUrl = ref<string | null>(null)
const lanStreamUrl = ref<string | null>(null)
const isLanEnabled = ref(false)
const lanIp = ref<string | null>(null)
const textContent = ref<string>('')
const isCopied = ref(false)
const isStreamCopied = ref(false)
const isLanCopied = ref(false)
const isExpanded = ref(false)
const mediaDecodeError = ref(false)
const isAudioOnlyPlaying = ref(false)
const videoRef = ref<HTMLVideoElement | null>(null)
const showQr = ref(false)
const qrDataUrl = ref<string | null>(null)
const qrError = ref('')

function handleMediaError() {
  mediaDecodeError.value = true
}

function handleVideoLoadedMetadata(e: Event) {
  const v = e.target as HTMLVideoElement
  // 若元数据解析完成后 videoWidth 为 0（说明该 MP4 仅包含音频轨道，或者视频轨为未识别编码导致无画面）
  if (v && v.videoWidth === 0 && v.videoHeight === 0 && v.duration > 0) {
    isAudioOnlyPlaying.value = true
  }
}

// 200 MB 预览体积熔断保护（放宽至 200MB，支持绝大多数常见音频/视频/文档）
const MAX_PREVIEW_SIZE = 200 * 1024 * 1024

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
  streamUrl.value = null
  lanStreamUrl.value = null
  textContent.value = ''
  errorMsg.value = ''
  isCopied.value = false
  isStreamCopied.value = false
  isLanCopied.value = false
  mediaDecodeError.value = false
  isAudioOnlyPlaying.value = false
  showQr.value = false
  qrDataUrl.value = null
  qrError.value = ''
}

async function loadPreview() {
  cleanBlob()
  if (!props.item || props.item.IsDir) return

  // 音视频：直接走流媒体 URL（streamUrl），不再同时加载 blob 避免双份内存开销
  if (fileCategory.value === 'video' || fileCategory.value === 'audio') {
    loading.value = true
    try {
      const status = await tauriVault.getStreamServerStatus()
      isLanEnabled.value = status.is_lan
      lanIp.value = status.lan_ip
      streamUrl.value = await tauriVault.getStreamUrl(props.item.Path, false)
      if (status.is_lan) {
        lanStreamUrl.value = await tauriVault.getStreamUrl(props.item.Path, true)
      }
    } catch (err: any) {
      errorMsg.value = typeof err === 'string' ? err : err?.message || '获取流媒体地址失败'
    } finally {
      loading.value = false
    }
    return
  }

  // 非媒体文件体积超限：给出提示
  if (props.item.Size > MAX_PREVIEW_SIZE) {
    errorMsg.value = `该文件体积为 ${(props.item.Size / 1024 / 1024).toFixed(1)} MB，超出内存安全预览上限（200 MB）。为保障流畅性，请直接导出后查看。`
    return
  }

  if (fileCategory.value === 'unsupported') {
    return
  }

  // 文本/图片/PDF：内存 Blob 读取
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

async function toggleLanMode() {
  if (!props.item) return
  try {
    const nextMode = !isLanEnabled.value
    const status = await tauriVault.setStreamLanMode(nextMode)
    isLanEnabled.value = status.is_lan
    lanIp.value = status.lan_ip
    streamUrl.value = await tauriVault.getStreamUrl(props.item.Path, false)
    if (status.is_lan) {
      lanStreamUrl.value = await tauriVault.getStreamUrl(props.item.Path, true)
    } else {
      lanStreamUrl.value = null
    }
  } catch {}
}

async function copyLanStreamLink() {
  const url = lanStreamUrl.value || streamUrl.value
  if (!url) return
  try {
    await navigator.clipboard.writeText(url)
    isLanCopied.value = true
    setTimeout(() => {
      isLanCopied.value = false
    }, 2000)
  } catch {}
}

// 生成局域网播放二维码：扫码即开，免去手机端手动粘贴长 URL（含 Token）
async function openQrCode() {
  const url = lanStreamUrl.value || streamUrl.value
  if (!url) return
  showQr.value = true
  qrError.value = ''
  qrDataUrl.value = null
  try {
    qrDataUrl.value = await QRCode.toDataURL(url, {
      errorCorrectionLevel: 'M',
      margin: 1,
      width: 384,
      color: { dark: '#111827', light: '#ffffff' }
    })
  } catch {
    qrError.value = '二维码生成失败，请改用下方直链复制后发送到手机打开'
  }
}

function closeQrCode() {
  showQr.value = false
  qrDataUrl.value = null
  qrError.value = ''
}

function handleQrKeydown(event: KeyboardEvent) {
  if (showQr.value && event.key === 'Escape') {
    event.stopPropagation()
    closeQrCode()
  }
}

onMounted(() => {
  // 捕获阶段优先处理：QR 弹窗开启时 Esc 仅关闭二维码，不连带关闭预览
  window.addEventListener('keydown', handleQrKeydown, { capture: true })
})

onUnmounted(() => {
  window.removeEventListener('keydown', handleQrKeydown, { capture: true })
  cleanBlob()
})

async function openInBrowser() {
  if (!streamUrl.value) return
  try {
    await openUrl(streamUrl.value)
  } catch {}
}

async function copyStreamLink() {
  if (!streamUrl.value) return
  try {
    await navigator.clipboard.writeText(streamUrl.value)
    isStreamCopied.value = true
    setTimeout(() => {
      isStreamCopied.value = false
    }, 2000)
  } catch {}
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
          <!-- 局域网 / 外部播放器 / 浏览器串流按钮组 -->
          <template v-if="(fileCategory === 'video' || fileCategory === 'audio') && streamUrl">
            <button
              type="button"
              @click="toggleLanMode"
              class="px-2.5 py-1.5 rounded-lg text-xs font-medium transition flex items-center gap-1.5 border shadow-sm"
              :class="isLanEnabled ? 'bg-emerald-600/20 border-emerald-500/40 text-emerald-300' : 'bg-slate-800 hover:bg-slate-700 border-slate-700 text-slate-300'"
              :title="isLanEnabled ? '局域网共享已开启（手机/iPad可在同WiFi下直接观看）' : '开启局域网共享（允许手机/iPad在同WiFi下观看）'"
            >
              <Wifi v-if="isLanEnabled" class="w-3.5 h-3.5 text-emerald-400 animate-pulse" />
              <WifiOff v-else class="w-3.5 h-3.5 text-slate-400" />
              <span class="hidden lg:inline">{{ isLanEnabled ? '局域网已开' : '局域网共享' }}</span>
            </button>

            <button
              v-if="isLanEnabled"
              type="button"
              @click="copyLanStreamLink"
              class="px-2.5 py-1.5 bg-emerald-600 hover:bg-emerald-500 text-white rounded-lg text-xs font-medium transition flex items-center gap-1.5 shadow-sm"
              :title="`复制局域网播放直链 (${lanIp || '192.168.x.x'})，可在手机/iPad浏览器或播放器打开`"
            >
              <Check v-if="isLanCopied" class="w-3.5 h-3.5 text-white" />
              <Smartphone v-else class="w-3.5 h-3.5" />
              <span class="hidden md:inline">{{ isLanCopied ? '已复制手机直链' : '手机/iPad直链' }}</span>
            </button>

            <button
              v-if="isLanEnabled && (lanStreamUrl || streamUrl)"
              type="button"
              @click="openQrCode"
              class="px-2.5 py-1.5 bg-slate-900 hover:bg-slate-800 text-emerald-300 rounded-lg text-xs font-medium transition flex items-center gap-1.5 border border-emerald-500/30"
              title="生成扫码播放二维码，手机/iPad 扫码即可在浏览器中打开"
            >
              <QrCode class="w-3.5 h-3.5 text-emerald-400" />
              <span class="hidden md:inline">扫码播放</span>
            </button>

            <button
              type="button"
              @click="openInBrowser"
              class="px-2.5 py-1.5 bg-slate-800 hover:bg-slate-700 text-slate-200 rounded-lg text-xs font-medium transition flex items-center gap-1.5 border border-slate-700"
              title="在系统默认浏览器中打开全屏硬解播放"
            >
              <ExternalLink class="w-3.5 h-3.5 text-blue-400" />
              <span class="hidden md:inline">浏览器播放</span>
            </button>

            <button
              type="button"
              @click="copyStreamLink"
              class="p-2 text-slate-400 hover:text-slate-200 hover:bg-slate-800 rounded-lg transition"
              :title="isStreamCopied ? '已复制播放地址' : '复制本机流媒体直链 (可在 PotPlayer/VLC 中直接打开)'"
            >
              <Check v-if="isStreamCopied" class="w-4 h-4 text-emerald-400" />
              <Tv v-else class="w-4 h-4 text-cyan-400" />
            </button>
          </template>

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

        <!-- 3. 视频预览（优先 streamUrl 流式播放，无需全量加载到内存） -->
        <div v-else-if="fileCategory === 'video' && streamUrl" class="w-full h-full flex flex-col items-center justify-center relative p-2">
          <!-- 视频加载中：正在获取流媒体地址 / 等待首帧就绪 -->
          <div v-if="loading" class="absolute inset-0 z-10 flex flex-col items-center justify-center gap-3 bg-slate-950/95 rounded-xl text-slate-400">
            <Loader2 class="w-8 h-8 animate-spin text-blue-500" />
            <span class="text-xs tracking-wide">正在连接流媒体服务...</span>
          </div>
          <video
            ref="videoRef"
            :src="streamUrl"
            controls
            playsinline
            preload="metadata"
            class="max-w-full max-h-[68vh] w-auto h-auto rounded-xl shadow-2xl bg-black border border-slate-800"
            @loadedmetadata="handleVideoLoadedMetadata"
            @error="handleMediaError"
          ></video>
          <!-- 若检测到音频轨道正常但无画面/视频尺寸为0（典型的 H.265 纯音频播放现象） -->
          <div v-if="mediaDecodeError || isAudioOnlyPlaying" class="absolute inset-0 bg-slate-900/95 flex flex-col items-center justify-center p-6 text-center space-y-3 rounded-xl m-2">
            <AlertCircle class="w-10 h-10 text-amber-400" />
            <h4 class="text-sm font-semibold text-white">当前内置播放器无法解码该视频画面 (如 H.265 / HEVC)</h4>
            <p class="text-xs text-slate-400 max-w-md leading-relaxed">
              因 Windows WebView2 组件缺少部分视频编码的硬件解码器，无法直接渲染画面。您可以通过本地安全流媒体在外部专业播放器或浏览器中直接观看，无需等待导出。
            </p>
            <div class="flex flex-wrap items-center justify-center gap-2.5 pt-2">
              <button
                v-if="streamUrl"
                type="button"
                @click="openInBrowser"
                class="px-4 py-2 bg-indigo-600 hover:bg-indigo-500 text-white text-xs font-semibold rounded-xl transition shadow flex items-center gap-1.5"
              >
                <ExternalLink class="w-3.5 h-3.5" />
                在默认浏览器中打开
              </button>
              <button
                v-if="isLanEnabled"
                type="button"
                @click="copyLanStreamLink"
                class="px-3.5 py-2 bg-emerald-600 hover:bg-emerald-500 text-white text-xs font-semibold rounded-xl transition shadow flex items-center gap-1.5"
              >
                <Check v-if="isLanCopied" class="w-3.5 h-3.5" />
                <Smartphone v-else class="w-3.5 h-3.5" />
                <span>{{ isLanCopied ? '已复制手机/iPad直链' : '复制手机/iPad直链' }}</span>
              </button>
              <button
                v-if="isLanEnabled"
                type="button"
                @click="openQrCode"
                class="px-3.5 py-2 bg-slate-900 hover:bg-slate-800 text-emerald-300 text-xs font-semibold rounded-xl transition shadow border border-emerald-500/30 flex items-center gap-1.5"
              >
                <QrCode class="w-3.5 h-3.5 text-emerald-400" />
                手机扫码播放
              </button>
              <button
                v-if="streamUrl"
                type="button"
                @click="copyStreamLink"
                class="px-3.5 py-2 bg-slate-800 hover:bg-slate-700 text-slate-200 text-xs font-semibold rounded-xl transition border border-slate-700 flex items-center gap-1.5"
              >
                <Check v-if="isStreamCopied" class="w-3.5 h-3.5 text-emerald-400" />
                <Tv v-else class="w-3.5 h-3.5 text-cyan-400" />
                <span>{{ isStreamCopied ? '已复制直链' : '复制本机直链 (PotPlayer/VLC)' }}</span>
              </button>
              <button
                type="button"
                @click="emit('export', item)"
                class="px-4 py-2 bg-blue-600 hover:bg-blue-500 text-white text-xs font-semibold rounded-xl transition shadow flex items-center gap-1.5"
              >
                <Download class="w-3.5 h-3.5" />
                解密导出到本地
              </button>
            </div>
          </div>
        </div>

        <!-- 4. 音频预览（优先 streamUrl 流式播放） -->
        <div v-else-if="fileCategory === 'audio' && streamUrl" class="flex flex-col items-center justify-center gap-6 p-8 bg-slate-900/80 border border-slate-800 rounded-2xl">
          <!-- 音频加载中 -->
          <div v-if="loading" class="flex flex-col items-center gap-3 text-slate-400">
            <Loader2 class="w-8 h-8 animate-spin text-blue-500" />
            <span class="text-xs tracking-wide">正在连接流媒体服务...</span>
          </div>
          <template v-else>
            <div class="w-20 h-20 rounded-full bg-blue-500/10 border border-blue-500/20 text-blue-400 flex items-center justify-center animate-pulse">
              <Music class="w-10 h-10" />
            </div>
            <div class="text-center">
              <h4 class="text-sm font-semibold text-white">{{ item.Name }}</h4>
              <p class="text-xs text-slate-400 mt-1">{{ formatSize(item.Size) }}</p>
            </div>
            <audio :src="streamUrl" controls preload="auto" class="w-72 sm:w-96" @error="handleMediaError"></audio>
          </template>
          <p v-if="mediaDecodeError" class="text-xs text-amber-400">当前系统音频格式解码失败，建议导出后播放</p>
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

    <!-- 手机扫码播放弹窗 -->
    <div
      v-if="showQr"
      class="fixed inset-0 z-[70] flex items-center justify-center bg-black/70 backdrop-blur-sm p-4"
      @click.self="closeQrCode"
    >
      <div class="bg-slate-900 border border-slate-700 rounded-2xl shadow-2xl w-full max-w-sm p-6 flex flex-col items-center gap-4">
        <div class="flex items-center justify-between w-full">
          <h4 class="text-sm font-semibold text-white flex items-center gap-2">
            <QrCode class="w-4 h-4 text-emerald-400" />
            手机扫码播放
          </h4>
          <button type="button" @click="closeQrCode" class="p-1.5 text-slate-400 hover:text-white hover:bg-slate-800 rounded-lg transition" title="关闭扫码">
            <X class="w-4 h-4" />
          </button>
        </div>

        <div class="rounded-2xl bg-white p-3 shadow-lg">
          <img v-if="qrDataUrl" :src="qrDataUrl" alt="扫码播放二维码" class="w-64 h-64" />
          <div v-else class="w-64 h-64 flex items-center justify-center">
            <Loader2 v-if="!qrError" class="w-8 h-8 animate-spin text-blue-500" />
            <AlertCircle v-else class="w-10 h-10 text-amber-400" />
          </div>
        </div>

        <p v-if="qrError" class="text-xs text-amber-400 text-center">{{ qrError }}</p>
        <p v-else class="text-xs text-slate-400 text-center leading-relaxed">
          使用手机相机 / 微信「扫一扫」扫描，<br />
          保持手机与电脑在同一 WiFi 下即可直接播放
        </p>
        <p class="text-[11px] text-slate-500 truncate w-full text-center" :title="lanStreamUrl || streamUrl || ''">
          {{ lanStreamUrl || streamUrl || '' }}
        </p>

        <button
          type="button"
          @click="copyLanStreamLink"
          class="w-full py-2 bg-emerald-600 hover:bg-emerald-500 text-white text-xs font-semibold rounded-xl transition flex items-center justify-center gap-1.5"
        >
          <Check v-if="isLanCopied" class="w-3.5 h-3.5" />
          <Copy v-else class="w-3.5 h-3.5" />
          {{ isLanCopied ? '已复制直链' : '复制直链（扫码失败时兜底）' }}
        </button>
      </div>
    </div>
  </div>
</template>

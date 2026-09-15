<script setup lang="ts">
import { AlertTriangle, FolderOpen, Check } from 'lucide-vue-next'
import { tauriVault } from '@/services/tauriVault'

const props = defineProps<{
  show: boolean
  importedPaths: string[]
}>()

const emit = defineEmits<{
  (e: 'close'): void
}>()

function reveal(path: string) {
  tauriVault.showItemInFolder(path)
}
</script>

<template>
  <div
    v-if="show"
    class="fixed inset-0 z-50 flex items-center justify-center bg-black/70 backdrop-blur-sm p-4 animate-fade-in"
  >
    <div
      class="bg-slate-800 border border-slate-700 w-full max-w-lg rounded-2xl shadow-2xl overflow-hidden flex flex-col"
    >
      <!-- 弹窗头部 -->
      <div class="px-6 pt-6 pb-4 flex items-center gap-3 border-b border-slate-700/60 bg-slate-900/80">
        <div class="w-10 h-10 rounded-full bg-emerald-500/20 text-emerald-400 flex items-center justify-center">
          <Check class="w-6 h-6" />
        </div>
        <div>
          <h3 class="text-lg font-bold text-white">文件已加密入库</h3>
          <p class="text-xs text-slate-400">已通过高强度算法加密安全保存</p>
        </div>
      </div>

      <!-- 弹窗主体 -->
      <div class="p-6 space-y-4">
        <!-- 醒目重要安全提示 -->
        <div class="bg-amber-500/10 border border-amber-500/30 rounded-xl p-4 flex gap-3">
          <AlertTriangle class="w-6 h-6 text-amber-400 shrink-0 mt-0.5" />
          <div class="text-sm space-y-1">
            <h4 class="font-semibold text-amber-300">安全准则：原文件未被删除</h4>
            <p class="text-xs text-amber-200/80 leading-relaxed">
              为防止误操作造成数据丢失，保险箱<span class="font-bold underline text-amber-100">不会触碰或删除</span>您本地磁盘中的原始明文文件。
              为确保隐私不泄露，请在确认加密入库无误后，<span class="font-bold text-amber-100">手动彻底删除（Shift+Delete）</span>原始文件。
            </p>
          </div>
        </div>

        <!-- 导入的文件列表及定位按钮 -->
        <div>
          <div class="text-xs font-semibold text-slate-400 mb-2">本次入库项目（可点击快速定位源文件）：</div>
          <div class="max-h-40 overflow-y-auto space-y-2 pr-1">
            <div
              v-for="item in importedPaths"
              :key="item"
              class="flex items-center justify-between bg-slate-900/60 border border-slate-700/50 rounded-lg p-2.5 text-xs text-slate-300 hover:border-slate-600 transition"
            >
              <span class="truncate max-w-[320px]" :title="item">{{ item }}</span>
              <button
                @click="reveal(item)"
                class="flex items-center gap-1.5 px-2.5 py-1 rounded bg-slate-800 hover:bg-slate-700 text-blue-400 hover:text-blue-300 font-medium transition shrink-0 ml-2"
              >
                <FolderOpen class="w-3.5 h-3.5" />
                定位原文件
              </button>
            </div>
          </div>
        </div>
      </div>

      <!-- 弹窗底部操作 -->
      <div class="px-6 py-4 bg-slate-900/80 border-t border-slate-700/60 flex justify-end">
        <button
          @click="emit('close')"
          class="px-5 py-2 rounded-xl bg-blue-600 hover:bg-blue-500 text-white text-sm font-medium transition shadow-lg shadow-blue-500/20"
        >
          我知道了
        </button>
      </div>
    </div>
  </div>
</template>

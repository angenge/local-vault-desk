<script setup lang="ts">
import { ref, onMounted, nextTick } from 'vue'
import { useVaultStore } from '@/stores/vault'
import { tauriVault } from '@/services/tauriVault'
import type { VaultDirInspection } from '@/types'
import {
  FolderLock,
  FolderOpen,
  FolderPlus,
  KeyRound,
  User,
  ArrowRight,
  AlertTriangle,
  CheckCircle2,
  Sparkles,
  Info,
  Eye,
  EyeOff
} from 'lucide-vue-next'

const vaultStore = useVaultStore()

const mode = ref<'unlock' | 'create'>('unlock')
const vaultPath = ref(vaultStore.vaultPath)
const username = ref(vaultStore.username || '')
const password = ref('')
const confirmPassword = ref('')
const showPassword = ref(false)
const showConfirmPassword = ref(false)
const loading = ref(false)
const errorMsg = ref('')

// 目录探测状态
const dirInspection = ref<VaultDirInspection | null>(null)
// 是否显示“是否创建新保险箱”引导弹窗
const showCreatePromptDialog = ref(false)
// 是否显示“是否切换为解锁”引导弹窗
const showUnlockPromptDialog = ref(false)
// 用户是否显式确认“仍要覆盖创建”（初始化后端需 force=true 才放行覆盖已有保险箱）
const forceOverride = ref(false)

const passwordInputRef = ref<HTMLInputElement | null>(null)

onMounted(async () => {
  if (vaultPath.value) {
    await inspectPath(vaultPath.value, false)
  }
})

async function chooseDir() {
  forceOverride.value = false
  const selected = await tauriVault.selectDirectory()
  if (selected) {
    vaultPath.value = selected
    await inspectPath(selected, true)
  }
}

async function inspectPath(path: string, isUserAction = false) {
  errorMsg.value = ''
  try {
    const inspection = await tauriVault.inspectVaultDir(path)
    dirInspection.value = inspection

    if (mode.value === 'unlock') {
      if (inspection.kind === 'empty' || inspection.kind === 'not_exist') {
        if (isUserAction) {
          // 用户在解锁界面选了一个空目录，弹出提示引导是否创建保险箱
          showCreatePromptDialog.value = true
        }
      }
    } else if (mode.value === 'create') {
      if (inspection.kind === 'initialized_vault' && isUserAction) {
        // 用户在创建界面选了一个已有的保险箱，提示是否切换为解锁
        showUnlockPromptDialog.value = true
      }
    }
  } catch (e: any) {
    dirInspection.value = null
  }
}

function handleAcceptCreatePrompt() {
  showCreatePromptDialog.value = false
  forceOverride.value = false
  mode.value = 'create'
  errorMsg.value = ''
  nextTick(() => {
    passwordInputRef.value?.focus()
  })
}

async function handleForceUnlock() {
  if (!password.value.trim()) {
    showCreatePromptDialog.value = false
    errorMsg.value = '请先输入主密码'
    nextTick(() => {
      passwordInputRef.value?.focus()
    })
    return
  }
  showCreatePromptDialog.value = false
  loading.value = true
  errorMsg.value = ''
  try {
    const status = await tauriVault.unlockVault({
      vaultPath: vaultPath.value,
      username: username.value.trim(),
      password: password.value
    })
    vaultStore.setStatus(status)
    await vaultStore.refreshFiles()
  } catch (err: any) {
    errorMsg.value = typeof err === 'string' ? err : err?.message || '操作失败，请检查密码是否正确'
  } finally {
    loading.value = false
  }
}

function handleAcceptUnlockPrompt() {
  showUnlockPromptDialog.value = false
  forceOverride.value = false
  mode.value = 'unlock'
  errorMsg.value = ''
  nextTick(() => {
    passwordInputRef.value?.focus()
  })
}

function handleAcceptOverwrite() {
  // 在已有保险箱目录上确认“仍要覆盖创建”：需透传 force=true，其余路径一律保持校验
  showUnlockPromptDialog.value = false
  forceOverride.value = true
  errorMsg.value = ''
  nextTick(() => {
    passwordInputRef.value?.focus()
  })
}

function switchMode(target: 'unlock' | 'create') {
  mode.value = target
  forceOverride.value = false
  password.value = ''
  confirmPassword.value = ''
  showPassword.value = false
  showConfirmPassword.value = false
  errorMsg.value = ''
  showCreatePromptDialog.value = false
  showUnlockPromptDialog.value = false
}

// 密码强度策略：至少 8 位（不含首尾空格），且同时包含字母与数字；
// 须基于 trim 后结果判定，避免“纯空格”密码经 derive_keys 裁剪后派生空密钥。
function validatePassword(pw: string): string | null {
  const t = pw.trim()
  if (t.length < 8) return '密码长度至少需 8 位（不含首尾空格）'
  if (!/[a-zA-Z]/.test(t) || !/\d/.test(t)) return '密码需同时包含字母与数字'
  return null
}

async function handleSubmit() {
  errorMsg.value = ''
  if (!vaultPath.value) {
    errorMsg.value = '请选择保险箱目录路径'
    return
  }
  if (!username.value.trim()) {
    errorMsg.value = '请输入账号名称'
    return
  }
  if (!password.value) {
    errorMsg.value = '请输入主密码'
    return
  }

  if (mode.value === 'create') {
    if (password.value !== confirmPassword.value) {
      errorMsg.value = '两次输入的密码不一致'
      return
    }
    const pwErr = validatePassword(password.value)
    if (pwErr) {
      errorMsg.value = pwErr
      return
    }
    if (dirInspection.value?.kind === 'non_vault_not_empty') {
      errorMsg.value = '该目录包含普通文件，为避免明文文件与加密数据混杂在磁盘上，请先清空该目录或另选空目录'
      return
    }
  } else {
    // 解锁模式下，如果检测到是空目录 / 不存在 / 非保险箱普通目录，拦截并弹出友好引导
    if (
      dirInspection.value?.kind === 'empty' ||
      dirInspection.value?.kind === 'not_exist' ||
      dirInspection.value?.kind === 'non_vault_not_empty'
    ) {
      showCreatePromptDialog.value = true
      return
    }
  }

  loading.value = true
  try {
    let status
    if (mode.value === 'create') {
      status = await tauriVault.initVault({
        vaultPath: vaultPath.value,
        username: username.value.trim(),
        password: password.value,
        force: dirInspection.value?.kind === 'initialized_vault' && forceOverride.value
      })
    } else {
      status = await tauriVault.unlockVault({
        vaultPath: vaultPath.value,
        username: username.value.trim(),
        password: password.value
      })
    }
    vaultStore.setStatus(status)
    await vaultStore.refreshFiles()
  } catch (err: any) {
    errorMsg.value = typeof err === 'string' ? err : err?.message || '操作失败，请检查密码是否正确'
  } finally {
    loading.value = false
  }
}
</script>

<template>
  <div class="h-screen w-screen overflow-y-auto flex items-center justify-center p-4 sm:p-6 bg-gradient-to-br from-slate-950 via-slate-900 to-slate-950 relative">
    <div class="w-full max-w-md bg-slate-900/90 border border-slate-800 rounded-3xl p-6 sm:p-7 shadow-2xl backdrop-blur-md relative z-10 my-auto">
      <!-- 标志与标题 -->
      <div class="flex flex-col items-center text-center mb-5">
        <div class="w-12 h-12 rounded-2xl bg-blue-600/10 border border-blue-500/20 flex items-center justify-center text-blue-500 mb-2.5 shadow-inner">
          <FolderLock class="w-6 h-6" />
        </div>
        <h1 class="text-xl font-bold text-white tracking-tight">本地文件保险箱</h1>
        <p class="text-xs text-slate-400 mt-0.5">金融级端到端本地加密存储</p>
      </div>

      <!-- 模式切换标签 -->
      <div class="flex bg-slate-950/60 p-1 rounded-xl border border-slate-800/80 mb-4">
        <button
          type="button"
          @click="switchMode('unlock')"
          class="flex-1 py-1.5 text-xs font-semibold rounded-lg transition whitespace-nowrap"
          :class="mode === 'unlock' ? 'bg-blue-600 text-white shadow' : 'text-slate-400 hover:text-white'"
        >
          解锁保险箱
        </button>
        <button
          type="button"
          @click="switchMode('create')"
          class="flex-1 py-1.5 text-xs font-semibold rounded-lg transition whitespace-nowrap"
          :class="mode === 'create' ? 'bg-blue-600 text-white shadow' : 'text-slate-400 hover:text-white'"
        >
          创建新保险箱
        </button>
      </div>

      <!-- 表单区域 -->
      <form @submit.prevent="handleSubmit" class="space-y-3">
        <!-- 保险箱物理路径 -->
        <div>
          <div class="flex items-center justify-between mb-1.5">
            <label class="block text-xs font-medium text-slate-300">密文存放目录</label>
            <!-- 目录特征感知标签 -->
            <span
              v-if="dirInspection && vaultPath"
              class="text-[11px] flex items-center gap-1 font-medium transition-colors"
              :class="{
                'text-emerald-400': dirInspection.kind === 'initialized_vault',
                'text-amber-400': dirInspection.kind === 'empty',
                'text-orange-400': dirInspection.kind === 'non_vault_not_empty',
                'text-slate-500': dirInspection.kind === 'not_exist'
              }"
            >
              <template v-if="dirInspection.kind === 'initialized_vault'">
                <CheckCircle2 class="w-3.5 h-3.5" /> 已有保险箱
              </template>
              <template v-else-if="dirInspection.kind === 'empty'">
                <Sparkles class="w-3.5 h-3.5" /> 空目录 (待创建)
              </template>
              <template v-else-if="dirInspection.kind === 'non_vault_not_empty'">
                <AlertTriangle class="w-3.5 h-3.5" /> 普通目录 (非保险箱)
              </template>
            </span>
          </div>

          <div class="flex gap-2">
            <input
              v-model="vaultPath"
              type="text"
              placeholder="选择本地目录..."
              readonly
              class="flex-1 bg-slate-950/60 border border-slate-800 rounded-xl px-3.5 py-2.5 text-xs text-slate-200 focus:outline-none focus:border-blue-500 transition cursor-pointer"
              @click="chooseDir"
            />
            <button
              type="button"
              @click="chooseDir"
              class="px-3.5 py-2.5 bg-slate-800 hover:bg-slate-700 text-slate-300 rounded-xl text-xs font-medium transition flex items-center gap-1.5 shrink-0"
            >
              <FolderOpen class="w-4 h-4" />
              浏览
            </button>
          </div>

          <!-- 解锁模式下选择空目录的内嵌快捷提示条 -->
          <div
            v-if="mode === 'unlock' && dirInspection?.kind === 'empty'"
            class="mt-2 p-2.5 bg-amber-500/10 border border-amber-500/20 rounded-xl flex items-center justify-between text-xs text-amber-300"
          >
            <span class="flex items-center gap-1.5">
              <Info class="w-3.5 h-3.5 shrink-0" />
              该目录为空，尚未创建保险箱
            </span>
            <button
              type="button"
              @click="handleAcceptCreatePrompt"
              class="px-2.5 py-1 bg-amber-500 hover:bg-amber-400 text-slate-950 font-semibold rounded-lg text-[11px] transition shrink-0 flex items-center gap-1"
            >
              <FolderPlus class="w-3 h-3" />
              立即创建
            </button>
          </div>

          <!-- 解锁模式下选择非空且非保险箱目录的提示条 -->
          <div
            v-else-if="mode === 'unlock' && dirInspection?.kind === 'non_vault_not_empty'"
            class="mt-2 p-2.5 bg-orange-500/10 border border-orange-500/20 rounded-xl flex items-center justify-between text-xs text-orange-300"
          >
            <span class="flex items-center gap-1.5">
              <AlertTriangle class="w-3.5 h-3.5 shrink-0" />
              包含普通文件，未检测到保险箱；创建保险箱需使用空目录
            </span>
            <button
              type="button"
              @click="chooseDir"
              class="px-2.5 py-1 bg-orange-500 hover:bg-orange-400 text-slate-950 font-semibold rounded-lg text-[11px] transition shrink-0 flex items-center gap-1"
            >
              <FolderOpen class="w-3 h-3" />
              重新选择空目录
            </button>
          </div>
        </div>

        <!-- 用户名 -->
        <div>
          <label class="block text-xs font-medium text-slate-300 mb-1.5">账号名称</label>
          <div class="relative">
            <User class="w-4 h-4 text-slate-500 absolute left-3.5 top-1/2 -translate-y-1/2" />
            <input
              v-model="username"
              type="text"
              placeholder="例如: Bob / Alice"
              class="w-full bg-slate-950/60 border border-slate-800 rounded-xl pl-10 pr-3.5 py-2.5 text-xs text-slate-200 focus:outline-none focus:border-blue-500 transition"
            />
          </div>
        </div>

        <!-- 主密码 -->
        <div>
          <label class="block text-xs font-medium text-slate-300 mb-1.5">主密码</label>
          <div class="relative">
            <KeyRound class="w-4 h-4 text-slate-500 absolute left-3.5 top-1/2 -translate-y-1/2" />
            <input
              ref="passwordInputRef"
              v-model="password"
              :type="showPassword ? 'text' : 'password'"
              placeholder="输入主密码"
              class="w-full bg-slate-950/60 border border-slate-800 rounded-xl pl-10 pr-10 py-2.5 text-xs text-slate-200 focus:outline-none focus:border-blue-500 transition"
            />
            <button
              type="button"
              tabindex="-1"
              @click="showPassword = !showPassword"
              class="absolute right-3 top-1/2 -translate-y-1/2 text-slate-500 hover:text-slate-300 transition p-0.5"
              :title="showPassword ? '隐藏密码' : '显示密码'"
            >
              <EyeOff v-if="showPassword" class="w-4 h-4" />
              <Eye v-else class="w-4 h-4" />
            </button>
          </div>
        </div>

        <!-- 确认密码（仅创建模式） -->
        <div v-if="mode === 'create'">
          <label class="block text-xs font-medium text-slate-300 mb-1.5">再次确认主密码</label>
          <div class="relative">
            <KeyRound class="w-4 h-4 text-slate-500 absolute left-3.5 top-1/2 -translate-y-1/2" />
            <input
              v-model="confirmPassword"
              :type="showConfirmPassword ? 'text' : 'password'"
              placeholder="再次输入相同密码"
              class="w-full bg-slate-950/60 border border-slate-800 rounded-xl pl-10 pr-10 py-2.5 text-xs text-slate-200 focus:outline-none focus:border-blue-500 transition"
            />
            <button
              type="button"
              tabindex="-1"
              @click="showConfirmPassword = !showConfirmPassword"
              class="absolute right-3 top-1/2 -translate-y-1/2 text-slate-500 hover:text-slate-300 transition p-0.5"
              :title="showConfirmPassword ? '隐藏密码' : '显示密码'"
            >
              <EyeOff v-if="showConfirmPassword" class="w-4 h-4" />
              <Eye v-else class="w-4 h-4" />
            </button>
          </div>
        </div>

        <!-- 错误提示 -->
        <div v-if="errorMsg" class="p-3 bg-red-500/10 border border-red-500/30 rounded-xl text-xs text-red-400">
          {{ errorMsg }}
        </div>

        <!-- 创建模式下主密码不可找回的安全告示 -->
        <div
          v-if="mode === 'create'"
          class="p-2.5 bg-amber-500/10 border border-amber-500/30 rounded-xl text-xs text-amber-300 space-y-0.5"
        >
          <p class="font-semibold flex items-center gap-1.5 text-[11px]">
            <AlertTriangle class="w-3.5 h-3.5 shrink-0" />
            主密码无法找回，请务必妥善保管
          </p>
          <p class="text-amber-200/80 leading-relaxed text-[11px]">
            本保险箱使用不可逆密钥派生加密，不设任何找回通道。忘记主密码将导致数据永久无法解密。
          </p>
        </div>

        <!-- 提交按钮 -->
        <button
          type="submit"
          :disabled="loading"
          class="w-full py-2.5 px-4 bg-blue-600 hover:bg-blue-500 active:bg-blue-700 disabled:opacity-50 text-white rounded-xl text-sm font-semibold transition shadow-lg shadow-blue-600/25 flex items-center justify-center gap-2 mt-4"
        >
          <span v-if="loading">正在初始化加密通道...</span>
          <template v-else>
            <span>{{ mode === 'unlock' ? '安全解锁' : '初始化并创建保险箱' }}</span>
            <ArrowRight class="w-4 h-4" />
          </template>
        </button>
      </form>

      <!-- 底层引擎指示灯 -->
      <div class="mt-4 pt-3 border-t border-slate-800/80 flex items-center justify-between text-[11px] text-slate-500 select-none">
        <span class="flex items-center gap-2">
          <span class="relative flex h-2 w-2">
            <span
              v-if="vaultStore.engineStatus.is_running"
              class="animate-ping absolute inline-flex h-full w-full rounded-full bg-emerald-400 opacity-60"
            ></span>
            <span
              class="relative inline-flex rounded-full h-2 w-2"
              :class="vaultStore.engineStatus.is_running ? 'bg-emerald-500' : 'bg-slate-500'"
            ></span>
          </span>
          <span class="text-slate-400">
            安全加密引擎:
            <span :class="vaultStore.engineStatus.is_running ? 'text-emerald-400 font-medium' : 'text-slate-400 font-medium'">
              {{ vaultStore.engineStatus.is_running ? (vaultStore.engineStatus.version ? `运行中 (${vaultStore.engineStatus.version})` : '运行中') : '待命中 (解锁后连接)' }}
            </span>
          </span>
        </span>
        <span class="text-slate-500 text-[10px]">纯本地零云端</span>
      </div>
    </div>

    <!-- 弹窗 1：解锁模式下选择非保险箱目录，提示创建新保险箱或尝试解锁 -->
    <div
      v-if="showCreatePromptDialog"
      class="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-sm animate-fade-in"
    >
      <div class="bg-slate-900 border border-slate-700 rounded-2xl p-6 max-w-sm w-full shadow-2xl space-y-4">
        <div class="flex items-start gap-3">
          <div
            class="w-10 h-10 rounded-xl border flex items-center justify-center shrink-0"
            :class="dirInspection?.kind === 'non_vault_not_empty' ? 'bg-amber-500/10 border-amber-500/20 text-amber-400' : 'bg-blue-500/10 border-blue-500/20 text-blue-400'"
          >
            <FolderLock v-if="dirInspection?.kind === 'non_vault_not_empty'" class="w-5 h-5" />
            <FolderPlus v-else class="w-5 h-5" />
          </div>
          <div>
            <h3 class="text-sm font-bold text-white">
              {{ dirInspection?.kind === 'non_vault_not_empty' ? '目录未完全符合保险箱特征' : '未检测到保险箱数据' }}
            </h3>
            <p class="text-xs text-slate-300 mt-1 leading-relaxed">
              <template v-if="dirInspection?.kind === 'non_vault_not_empty'">
                该目录包含部分普通文件。若您确信这是既有保险箱（如混入了外部同步文件），可点击<span class="text-emerald-400 font-semibold">仍尝试解锁</span>验证主密码。
              </template>
              <template v-else>
                所选目录尚未初始化为保险箱。是否立即在此目录<span class="text-blue-400 font-semibold">创建新保险箱</span>？
              </template>
            </p>
          </div>
        </div>

        <div class="bg-slate-950/70 p-2.5 rounded-xl text-[11px] text-slate-400 break-all border border-slate-800">
          <span class="text-slate-500">路径：</span>{{ vaultPath }}
        </div>

        <div class="flex gap-2 pt-1">
          <button
            type="button"
            @click="showCreatePromptDialog = false"
            class="flex-1 py-2 px-3 bg-slate-800 hover:bg-slate-700 text-slate-300 rounded-xl text-xs font-medium transition"
          >
            取消 / 重选
          </button>
          <button
            v-if="dirInspection?.kind === 'non_vault_not_empty'"
            type="button"
            @click="handleForceUnlock"
            class="flex-1 py-2 px-3 bg-emerald-600 hover:bg-emerald-500 text-white rounded-xl text-xs font-semibold transition shadow flex items-center justify-center gap-1"
          >
            <FolderLock class="w-3.5 h-3.5" />
            仍尝试解锁
          </button>
          <button
            v-else
            type="button"
            @click="handleAcceptCreatePrompt"
            class="flex-1 py-2 px-3 bg-blue-600 hover:bg-blue-500 text-white rounded-xl text-xs font-semibold transition shadow flex items-center justify-center gap-1"
          >
            <FolderPlus class="w-3.5 h-3.5" />
            前往创建
          </button>
        </div>
      </div>
    </div>

    <!-- 弹窗 2：创建模式下选择已有保险箱，提示切换解锁 -->
    <div
      v-if="showUnlockPromptDialog"
      class="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-sm animate-fade-in"
    >
      <div class="bg-slate-900 border border-slate-700 rounded-2xl p-6 max-w-sm w-full shadow-2xl space-y-4">
        <div class="flex items-start gap-3">
          <div class="w-10 h-10 rounded-xl bg-emerald-500/10 border border-emerald-500/20 flex items-center justify-center text-emerald-400 shrink-0">
            <FolderLock class="w-5 h-5" />
          </div>
          <div>
            <h3 class="text-sm font-bold text-white">检测到已有保险箱</h3>
            <p class="text-xs text-slate-300 mt-1 leading-relaxed">
              该目录已包含已初始化的保险箱数据。是否切换至<span class="text-emerald-400 font-semibold">安全解锁</span>模式？
            </p>
          </div>
        </div>

        <div class="bg-slate-950/70 p-2.5 rounded-xl text-[11px] text-slate-400 break-all border border-slate-800">
          <span class="text-slate-500">路径：</span>{{ vaultPath }}
        </div>

        <div class="flex gap-2 pt-1">
          <button
            type="button"
            @click="handleAcceptOverwrite"
            class="flex-1 py-2 px-3 bg-slate-800 hover:bg-slate-700 text-slate-300 rounded-xl text-xs font-medium transition"
          >
            仍要覆盖创建
          </button>
          <button
            type="button"
            @click="handleAcceptUnlockPrompt"
            class="flex-1 py-2 px-3 bg-emerald-600 hover:bg-emerald-500 text-white rounded-xl text-xs font-semibold transition shadow flex items-center justify-center gap-1"
          >
            <FolderLock class="w-3.5 h-3.5" />
            切换至解锁
          </button>
        </div>
      </div>
    </div>
  </div>
</template>

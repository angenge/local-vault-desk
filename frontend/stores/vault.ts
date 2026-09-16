import { defineStore } from 'pinia'
import { ref } from 'vue'
import type { EngineStatus, RcloneItem, VaultStatus } from '@/types'
import { tauriVault } from '@/services/tauriVault'

export const useVaultStore = defineStore('vault', () => {
  const isUnlocked = ref(false)
  const vaultPath = ref(localStorage.getItem('last_vault_path') || '')
  // 账号名仅驻留当前会话（sessionStorage），不写入 localStorage（落盘明文），避免泄露
  const username = ref(sessionStorage.getItem('last_vault_user') || '')
  const currentDir = ref('')
  const files = ref<RcloneItem[]>([])
  const loading = ref(false)
  const errorMsg = ref('')
  const engineStatus = ref<EngineStatus>({
    is_running: false,
    port: 0,
    version: '',
    message: '待命中'
  })

  async function checkEngine() {
    try {
      engineStatus.value = await tauriVault.getEngineStatus()
    } catch {}
  }

  function setStatus(status: VaultStatus) {
    isUnlocked.value = status.is_unlocked
    if (status.vault_path) {
      vaultPath.value = status.vault_path
      localStorage.setItem('last_vault_path', status.vault_path)
    }
    if (status.username) {
      username.value = status.username
      sessionStorage.setItem('last_vault_user', status.username)
    }
    checkEngine()
  }

  // 请求序号：丢弃过期响应。快速导航/重复刷新时，较慢的旧请求后到会把
  // files 覆盖成上一个目录的内容（面包屑与列表错位）。
  let filesSeq = 0

  async function refreshFiles() {
    if (!isUnlocked.value) return
    const seq = ++filesSeq
    loading.value = true
    errorMsg.value = ''
    try {
      files.value = await tauriVault.listFiles(currentDir.value)
      if (seq !== filesSeq) return // 已有更新的请求，本次结果已过期
      await checkEngine()
    } catch (err: any) {
      if (seq !== filesSeq) return
      errorMsg.value = typeof err === 'string' ? err : err?.message || '加载文件失败'
    } finally {
      if (seq === filesSeq) loading.value = false
    }
  }

  function navigateTo(dir: string) {
    currentDir.value = dir.replace(/\\/g, '/').replace(/^\/+|\/+$/g, '')
    refreshFiles()
  }

  function goUp() {
    if (!currentDir.value) return
    const parts = currentDir.value.split('/')
    parts.pop()
    currentDir.value = parts.join('/')
    refreshFiles()
  }

  async function lock() {
    await tauriVault.lockVault()
    isUnlocked.value = false
    files.value = []
    currentDir.value = ''
    await checkEngine()
  }

  return {
    isUnlocked,
    vaultPath,
    username,
    currentDir,
    files,
    loading,
    errorMsg,
    engineStatus,
    checkEngine,
    setStatus,
    refreshFiles,
    navigateTo,
    goUp,
    lock
  }
})

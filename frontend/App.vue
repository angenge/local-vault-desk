<script setup lang="ts">
import { onMounted } from 'vue'
import { useVaultStore } from '@/stores/vault'
import { tauriVault } from '@/services/tauriVault'
import LoginView from '@/views/LoginView.vue'
import VaultMainView from '@/views/VaultMainView.vue'

const vaultStore = useVaultStore()

onMounted(async () => {
  try {
    const status = await tauriVault.getStatus()
    vaultStore.setStatus(status)
    if (status.is_unlocked) {
      await vaultStore.refreshFiles()
    }
  } catch {}
})
</script>

<template>
  <div class="w-full h-screen overflow-hidden bg-slate-900 font-sans">
    <VaultMainView v-if="vaultStore.isUnlocked" />
    <LoginView v-else />
  </div>
</template>

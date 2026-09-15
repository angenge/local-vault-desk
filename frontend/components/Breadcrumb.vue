<script setup lang="ts">
import { ChevronRight, Home } from 'lucide-vue-next'
import { computed } from 'vue'

const props = defineProps<{
  currentDir: string
}>()

const emit = defineEmits<{
  (e: 'navigate', path: string): void
}>()

const breadcrumbs = computed(() => {
  if (!props.currentDir) return []
  const parts = props.currentDir.split('/').filter(Boolean)
  return parts.map((part, index) => {
    return {
      name: part,
      path: parts.slice(0, index + 1).join('/')
    }
  })
})
</script>

<template>
  <div class="flex items-center gap-1 text-sm text-slate-400 overflow-x-auto py-1">
    <button
      @click="emit('navigate', '')"
      class="flex items-center gap-1 hover:text-white px-2 py-1 rounded hover:bg-slate-800 transition"
      :class="{ 'text-blue-400 font-semibold': !currentDir }"
    >
      <Home class="w-4 h-4" />
      <span>根目录</span>
    </button>

    <template v-for="item in breadcrumbs" :key="item.path">
      <ChevronRight class="w-4 h-4 text-slate-600 shrink-0" />
      <button
        @click="emit('navigate', item.path)"
        class="hover:text-white px-2 py-1 rounded hover:bg-slate-800 transition whitespace-nowrap"
        :class="{ 'text-blue-400 font-semibold': item.path === currentDir }"
      >
        {{ item.name }}
      </button>
    </template>
  </div>
</template>

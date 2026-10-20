<script setup lang="ts">
import { computed } from 'vue'

import { formatElapsed } from '../lib/format'
import { useSessionStore } from '../stores/session'

const store = useSessionStore()

const RESULT_LABELS: Record<string, string> = {
  completed: '完成',
  early: '提前出关',
  emergency: '急事破戒',
  accident: '显示变化',
  interrupted: '异常中断',
}

const label = computed(() => {
  const key = store.result ?? 'completed'
  return RESULT_LABELS[key] ?? key
})
</script>

<template>
  <main class="settled">
    <p class="settled__label">本次闭关</p>
    <p class="settled__duration num">{{ formatElapsed(store.elapsedMs) }}</p>
    <p class="settled__result">{{ label }}</p>
    <p class="settled__note">结算页的完整内容（评语、累计、连续天数）在 M2、M5 补齐。</p>
    <button class="btn" type="button" @click="store.backToSetup()">再入一关</button>
  </main>
</template>

<style scoped>
.settled {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 12px;
  height: 100%;
}

.settled__label {
  color: var(--fg-muted);
  font-size: 12px;
  letter-spacing: 0.3em;
}

.settled__duration {
  font-size: 56px;
}

.settled__result {
  color: var(--accent);
  font-size: 15px;
}

.settled__note {
  color: var(--fg-muted);
  font-size: 11px;
}
</style>

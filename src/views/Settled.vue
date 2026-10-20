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

const RESULT_NOTES: Record<string, string> = {
  completed: '此关已过。钟鸣三声，道友辛苦。',
  early: '提前出关，已记入流水。',
  emergency: '急事破戒，本次不计入累计。',
  accident: '检测到显示配置变化，已为你出关。',
  interrupted: '上次闭关异常中断，本次已自动解除。',
}

const key = computed(() => store.result ?? 'completed')
const label = computed(() => RESULT_LABELS[key.value] ?? key.value)
const note = computed(() => RESULT_NOTES[key.value] ?? '')

// 「再入一关」只回配置页并预填上次时长，不自动开始（PRD F-7.6）
function again(): void {
  store.backToSetup()
}
</script>

<template>
  <main class="settled">
    <p class="settled__label">本次闭关</p>
    <p class="settled__duration num">{{ formatElapsed(store.elapsedMs) }}</p>
    <p class="settled__result">{{ label }}</p>
    <p class="settled__note">{{ note }}</p>

    <div class="settled__actions">
      <button class="btn btn--primary" type="button" @click="again">再入一关</button>
    </div>

    <p class="settled__todo">累计时长、连续天数与长老评语将在 M5 补齐。</p>
  </main>
</template>

<style scoped>
.settled {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 10px;
  height: 100%;
  padding: 24px;
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
  letter-spacing: 0.1em;
}

.settled__note {
  max-width: 320px;
  color: var(--fg-muted);
  font-size: 12px;
  line-height: 1.7;
  text-align: center;
}

.settled__actions {
  display: flex;
  gap: 12px;
  margin-top: 12px;
}

.btn {
  padding: 10px 22px;
  border: 1px solid var(--dialog-border);
  border-radius: var(--radius-btn);
  background: transparent;
  color: var(--fg);
  font-size: 13px;
}

.btn--primary {
  border-color: var(--accent);
  background: var(--accent);
  color: #14110e;
}

.settled__todo {
  margin-top: 10px;
  color: var(--fg-muted);
  font-size: 11px;
}
</style>

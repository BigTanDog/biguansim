<script setup lang="ts">
// 急事破戒的二次确认（PRD F-6.2 ~ F-6.4）：
// 必须完整输入 9 个字、代价先公示、Esc 或「再坐一会」可退回。
import { computed, onMounted, onUnmounted, ref } from 'vue'

import { formatElapsed } from '../lib/format'
import { useSessionStore } from '../stores/session'

const props = defineProps<{ sentence: string; elapsedMs: number; faults: number }>()

const store = useSessionStore()

const text = ref('')
const input = ref<HTMLInputElement | null>(null)
const busy = ref(false)

const matched = computed(() => text.value === props.sentence)
const expected = computed(() => Array.from(props.sentence).length)

async function confirm(): Promise<void> {
  if (!matched.value || busy.value) {
    return
  }
  busy.value = true
  await store.confirmEmergency(text.value)
  busy.value = false
}

function cancel(): void {
  void store.cancelEmergency()
}

function onKeydown(event: KeyboardEvent): void {
  if (event.key === 'Escape') {
    event.preventDefault()
    cancel()
  }
}

onMounted(() => {
  input.value?.focus()
  window.addEventListener('keydown', onKeydown)
})
onUnmounted(() => window.removeEventListener('keydown', onKeydown))
</script>

<template>
  <div class="confirm">
    <p class="confirm__title">破戒需谨慎</p>

    <ul class="confirm__cost">
      <li>本次已坐 {{ formatElapsed(elapsedMs) }} 将不计入累计</li>
      <li>今日将不计为有效日</li>
      <li>破戒累计 {{ faults }} 次</li>
    </ul>

    <p class="confirm__ask">若仍要走，请逐字抄下：</p>
    <p class="confirm__sentence">{{ sentence }}</p>

    <input
      ref="input"
      v-model="text"
      class="confirm__input"
      type="text"
      spellcheck="false"
      autocomplete="off"
      :placeholder="`请输入 ${expected} 个字`"
    />

    <div class="confirm__actions">
      <button class="confirm__cancel" type="button" @click="cancel">再坐一会</button>
      <button class="confirm__ok" type="button" :disabled="!matched || busy" @click="confirm">
        确认破戒
      </button>
    </div>
  </div>
</template>

<style scoped>
.confirm {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.confirm__title {
  color: #d4695f;
  font-size: 13px;
  letter-spacing: 0.15em;
}

.confirm__cost {
  display: flex;
  gap: 18px;
  color: var(--fg-muted);
  font-size: 12px;
  list-style: none;
}

.confirm__ask {
  font-size: 13px;
}

.confirm__sentence {
  color: var(--accent);
  font-size: 15px;
  letter-spacing: 0.1em;
}

.confirm__input {
  width: 100%;
  padding: 9px 12px;
  border: 1px solid var(--dialog-border);
  border-radius: var(--radius-btn);
  background: rgba(0, 0, 0, 0.25);
  color: var(--fg);
  font: inherit;
  font-size: 14px;
}

.confirm__input:focus {
  border-color: var(--accent);
  outline: none;
}

.confirm__actions {
  display: flex;
  gap: 10px;
  justify-content: flex-end;
}

.confirm__cancel,
.confirm__ok {
  padding: 7px 16px;
  border-radius: var(--radius-btn);
  background: transparent;
  cursor: pointer;
  font: inherit;
  font-size: 13px;
}

.confirm__cancel {
  border: 1px solid var(--dialog-border);
  color: var(--fg-muted);
}

.confirm__ok {
  border: 1px solid #d4695f;
  color: #d4695f;
}

.confirm__ok:disabled {
  border-color: var(--dialog-border);
  color: var(--fg-muted);
  cursor: default;
}
</style>

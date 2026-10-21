<script setup lang="ts">
// 抄写（PRD F-5.8 / A-23）：原文全程可见，逐字校验，错字标红、可退格。
// 展示层比对只在本地做高亮，**通过与否只认 Rust 的 submit_copy**。
import { computed, onMounted, ref } from 'vue'

import { copyProgress, firstMismatch, typedCount } from '../lib/copyText'
import { useSessionStore } from '../stores/session'

const props = defineProps<{ text: string; chars: number }>()

const store = useSessionStore()

const typed = ref('')
const area = ref<HTMLTextAreaElement | null>(null)
const busy = ref(false)
/** 服务端返回的首个错字位置（本地比对只用于即时提示） */
const serverMismatch = ref<number | null>(null)

const localMismatch = computed(() => firstMismatch(props.text, typed.value))
const mismatch = computed(() => serverMismatch.value ?? localMismatch.value)
const typedChars = computed(() => Array.from(typed.value))
/** 出错位置上"应该是什么"；多字时为空 */
const expectedChar = computed(() => {
  if (mismatch.value === null) {
    return ''
  }
  return Array.from(props.text)[mismatch.value] ?? ''
})
const typedTotal = computed(() => typedCount(typed.value))
const progress = computed(() => copyProgress(props.text, typed.value))

function onInput(): void {
  serverMismatch.value = null
}

/** 抄写不接受换行：Enter 直接吞掉，避免"多一个不可见字符"被判错 */
function onKeydown(event: KeyboardEvent): void {
  if (event.key === 'Enter') {
    event.preventDefault()
  }
}

async function submit(): Promise<void> {
  if (busy.value) {
    return
  }
  busy.value = true
  serverMismatch.value = await store.submitCopy(typed.value)
  busy.value = false
}

onMounted(() => area.value?.focus())
</script>

<template>
  <div class="copy">
    <p class="copy__title">逐字抄一遍（共 {{ chars }} 字）</p>

    <blockquote class="copy__source">{{ text }}</blockquote>

    <textarea
      ref="area"
      v-model="typed"
      class="copy__input"
      rows="3"
      spellcheck="false"
      autocomplete="off"
      placeholder="在此抄写，不要漏字、不要多字"
      @input="onInput"
      @keydown="onKeydown"
    />

    <p class="copy__preview">
      <span
        v-for="(char, index) in typedChars"
        :key="index"
        :class="{ 'copy__char--bad': index === mismatch }"
        >{{ char }}</span
      >
      <span v-if="typedChars.length === 0" class="copy__placeholder">（尚未动笔）</span>
    </p>

    <div class="copy__foot">
      <span class="copy__count num">{{ typedTotal }} / {{ chars }}</span>
      <span v-if="mismatch !== null" class="copy__bad">
        第 {{ mismatch + 1 }} 字不符：应为「{{ expectedChar || '（多出的字）' }}」
      </span>
      <progress class="copy__bar" :value="progress" max="1" />
      <button class="copy__submit" type="button" :disabled="busy" @click="submit">已抄完</button>
    </div>
  </div>
</template>

<style scoped>
.copy {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.copy__title {
  color: var(--accent);
  font-size: 12px;
  letter-spacing: 0.1em;
}

.copy__source {
  max-height: 22vh;
  overflow: auto;
  padding: 10px 12px;
  border-left: 2px solid var(--accent);
  color: var(--fg-muted);
  font-size: 13px;
  line-height: 1.9;
}

.copy__input {
  width: 100%;
  padding: 10px 12px;
  border: 1px solid var(--dialog-border);
  border-radius: var(--radius-btn);
  background: rgba(0, 0, 0, 0.25);
  color: var(--fg);
  font-family: inherit;
  font-size: 14px;
  line-height: 1.8;
  resize: none;
}

.copy__input:focus {
  border-color: var(--accent);
  outline: none;
}

.copy__preview {
  min-height: 20px;
  color: var(--fg);
  font-size: 14px;
  line-height: 1.8;
  word-break: break-all;
}

.copy__char--bad {
  border-bottom: 1px solid #d4695f;
  color: #d4695f;
}

.copy__placeholder {
  color: var(--fg-muted);
  font-size: 12px;
}

.copy__foot {
  display: flex;
  align-items: center;
  gap: 12px;
}

.copy__count {
  color: var(--fg-muted);
  font-size: 12px;
}

.copy__bad {
  color: #d4695f;
  font-size: 12px;
}

.copy__bar {
  width: 120px;
  height: 3px;
  margin-left: auto;
  appearance: none;
  background: rgba(255, 255, 255, 0.1);
  border: none;
}

.copy__bar::-webkit-progress-value {
  background: var(--accent);
}

.copy__submit {
  padding: 7px 16px;
  border: 1px solid var(--accent);
  border-radius: var(--radius-btn);
  background: transparent;
  color: var(--accent);
  cursor: pointer;
  font: inherit;
  font-size: 13px;
}

.copy__submit:disabled {
  border-color: var(--fg-muted);
  color: var(--fg-muted);
  cursor: default;
}
</style>

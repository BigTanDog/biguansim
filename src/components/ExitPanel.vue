<script setup lang="ts">
// 冷却（Cool）与问阵（Challenge）两块投影（PRD F-4.3 / F-5.6）。
// 冷却：固定语 + 分钟粒度，不给任何跳过入口（F-4.2）；
// 问阵：题干 + 四个选项，键盘 1–4 与鼠标都可选，**不做 Enter 提交**（F-5.6）。
import { computed, onMounted, onUnmounted } from 'vue'

import { formatElapsed } from '../lib/format'
import type { PhasePayload } from '../lib/ipc'

const props = defineProps<{
  payload: PhasePayload
  coolRemainMs: number
  elapsedMs: number
  notice: string | null
}>()

const emit = defineEmits<{ choose: [index: number] }>()

const challenge = computed(() => (props.payload.kind === 'challenge' ? props.payload : null))
const cool = computed(() => (props.payload.kind === 'cool' ? props.payload : null))

/** 分钟粒度、向上取整：显示"还剩 4 分钟"时实际不会少于 4 分钟 */
const coolMinutes = computed(() => Math.ceil(Math.max(0, props.coolRemainMs) / 60_000))

function onKeydown(event: KeyboardEvent): void {
  if (event.isComposing || event.altKey || event.ctrlKey || event.metaKey) {
    return
  }

  const current = challenge.value
  if (!current) {
    return
  }

  const picked = Number.parseInt(event.key, 10)
  if (Number.isNaN(picked) || picked < 1 || picked > current.options.length) {
    return
  }

  event.preventDefault()
  emit('choose', picked - 1)
}

onMounted(() => window.addEventListener('keydown', onKeydown))
onUnmounted(() => window.removeEventListener('keydown', onKeydown))
</script>

<template>
  <div class="panel">
    <template v-if="cool">
      <p class="panel__line">正在通知长老，请稍候。</p>
      <p class="panel__sub">还剩 {{ coolMinutes }} 分钟 · 已坚持 {{ formatElapsed(elapsedMs) }}</p>
      <p v-if="notice" class="panel__notice">{{ notice }}</p>
    </template>

    <template v-else-if="challenge">
      <p class="panel__meta">第 {{ challenge.index }} / {{ challenge.total }} 题</p>
      <p class="panel__stem">{{ challenge.stem }}</p>
      <ul class="options">
        <li v-for="(option, index) in challenge.options" :key="option">
          <button class="option" type="button" @click="emit('choose', index)">
            <span class="option__badge num">{{ index + 1 }}</span>
            <span class="option__text">{{ option }}</span>
          </button>
        </li>
      </ul>
      <p v-if="notice" class="panel__notice">{{ notice }}</p>
    </template>
  </div>
</template>

<style scoped>
.panel {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.panel__line {
  font-size: 15px;
  line-height: 1.9;
}

.panel__sub {
  color: var(--fg-muted);
  font-size: 12px;
}

.panel__meta {
  color: var(--accent);
  font-size: 12px;
  letter-spacing: 0.1em;
}

.panel__stem {
  font-size: 15px;
  line-height: 1.8;
}

.panel__notice {
  color: #d4695f;
  font-size: 12px;
}

.options {
  display: grid;
  gap: 8px;
  margin-top: 4px;
  grid-template-columns: repeat(2, minmax(0, 1fr));
}

.option {
  display: flex;
  width: 100%;
  align-items: center;
  gap: 10px;
  padding: 9px 12px;
  border: 1px solid var(--dialog-border);
  border-radius: var(--radius-btn);
  background: transparent;
  color: var(--fg);
  cursor: pointer;
  font: inherit;
  text-align: left;
}

.option:hover {
  border-color: var(--accent);
}

.option__badge {
  color: var(--fg-muted);
  font-size: 12px;
}

.option__text {
  font-size: 14px;
}
</style>

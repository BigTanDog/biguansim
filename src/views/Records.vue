<script setup lang="ts">
// 记录页（PRD F-9.8 / F-9.9）：四个数字 + 最近 50 条流水。
// 所有数字都来自 Rust 的 `core::stats`，这里只做显示。
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

const stats = computed(() => store.recordsPage?.stats ?? store.stats)
const records = computed(() => store.recordsPage?.records ?? [])
const skipped = computed(() => store.recordsPage?.skipped ?? 0)

const cards = computed(() => {
  const value = stats.value
  if (!value) {
    return []
  }
  return [
    { label: '累计', text: formatElapsed(value.totalMs) },
    { label: '连续', text: `${value.streakDays} 天` },
    { label: '有效日', text: `${value.validDays} 天` },
    { label: '破戒', text: `${value.faults} 次` },
  ]
})

/** 2026-10-21 21:03 */
function formatWhen(ms: number): string {
  const date = new Date(ms)
  const pad = (value: number): string => String(value).padStart(2, '0')
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`
}
</script>

<template>
  <main class="records">
    <header class="records__head">
      <button class="records__back" type="button" @click="store.closeSubPage()">‹ 返回</button>
      <h1 class="records__title">闭关记录</h1>
    </header>

    <section v-if="cards.length" class="cards">
      <div v-for="card in cards" :key="card.label" class="card">
        <p class="card__label">{{ card.label }}</p>
        <p class="card__value num">{{ card.text }}</p>
      </div>
    </section>

    <p v-if="!stats?.todayValid && stats" class="hint">
      今日尚未达标（≥25 分钟且完整坐满才算一个有效日）
    </p>

    <p v-if="skipped > 0" class="warn">{{ skipped }} 行记录无法解析，已跳过（文件可能被手改坏）</p>

    <ul v-if="records.length" class="list">
      <li v-for="record in records" :key="record.startedAtMs" class="row">
        <span class="row__when num">{{ formatWhen(record.startedAtMs) }}</span>
        <span class="row__actual num">{{ formatElapsed(record.actualMs) }}</span>
        <span class="row__result">{{ RESULT_LABELS[record.result] ?? record.result }}</span>
        <span class="row__meta">
          <template v-if="record.attempts > 0">尝试 {{ record.attempts }} 次</template>
        </span>
      </li>
    </ul>

    <p v-else class="empty">尚无闭关记录。第一次闭关，从 25 分钟开始。</p>
  </main>
</template>

<style scoped>
.records {
  display: flex;
  flex-direction: column;
  gap: 14px;
  width: min(760px, 100%);
  height: 100%;
  margin: 0 auto;
  padding: 26px 24px;
  overflow-y: auto;
}

.records__head {
  display: flex;
  align-items: center;
  gap: 12px;
}

.records__back {
  padding: 4px 10px;
  border: 1px solid var(--dialog-border);
  border-radius: var(--radius-btn);
  background: transparent;
  color: var(--fg-muted);
  cursor: pointer;
  font: inherit;
  font-size: 12px;
}

.records__title {
  font-size: 18px;
  font-weight: 500;
  letter-spacing: 0.06em;
}

.cards {
  display: grid;
  gap: 10px;
  grid-template-columns: repeat(4, 1fr);
}

.card {
  padding: 12px 14px;
  border: 1px solid var(--dialog-border);
  border-radius: var(--radius-card);
  background: var(--dialog-bg);
}

.card__label {
  color: var(--fg-muted);
  font-size: 11px;
  letter-spacing: 0.1em;
}

.card__value {
  margin-top: 6px;
  font-size: 18px;
}

.hint {
  color: var(--fg-muted);
  font-size: 11px;
}

.warn {
  color: #c9a227;
  font-size: 11px;
}

.list {
  display: flex;
  flex-direction: column;
  gap: 2px;
  list-style: none;
}

.row {
  display: grid;
  align-items: baseline;
  gap: 12px;
  padding: 8px 4px;
  border-bottom: 1px solid rgba(255, 255, 255, 0.06);
  font-size: 12px;
  grid-template-columns: 150px 90px 90px 1fr;
}

.row__when {
  color: var(--fg-muted);
}

.row__actual {
  color: var(--fg);
}

.row__result {
  color: var(--accent);
}

.row__meta {
  color: var(--fg-muted);
}

.empty {
  padding: 24px 0;
  color: var(--fg-muted);
  font-size: 12px;
  line-height: 1.9;
}
</style>

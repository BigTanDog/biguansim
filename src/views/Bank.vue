<script setup lang="ts">
// 题库页（PRD F-8.6）：只读，显示题面与出现/对错统计，**不显示答案**。
import { computed } from 'vue'

import { useSessionStore } from '../stores/session'

const store = useSessionStore()

const total = computed(() => store.questions.length)
const cooling = computed(() => store.questions.filter((item) => item.cooling).length)
</script>

<template>
  <main class="bank">
    <header class="bank__head">
      <button class="bank__back" type="button" @click="store.closeSubPage()">‹ 返回</button>
      <h1 class="bank__title">题库</h1>
      <span class="bank__meta num">{{ total }} 题 · 冷却中 {{ cooling }}</span>
    </header>

    <p class="bank__note">
      答对过的题三十天内不再出现，答错的题三十分钟内不再出现；全部在冷却时会按"最久未出现"补齐。
    </p>

    <ul class="list">
      <li v-for="question in store.questions" :key="question.id" class="row">
        <span class="row__id num">{{ question.id }}</span>
        <span class="row__stem">{{ question.stem }}</span>
        <span class="row__stat num"
          >对 {{ question.rightCount }} · 错 {{ question.wrongCount }}</span
        >
        <span class="row__cool" :class="{ 'row__cool--on': question.cooling }">
          {{ question.cooling ? '冷却中' : '可抽' }}
        </span>
      </li>
    </ul>
  </main>
</template>

<style scoped>
.bank {
  display: flex;
  flex-direction: column;
  gap: 12px;
  width: min(820px, 100%);
  height: 100%;
  margin: 0 auto;
  padding: 26px 24px;
  overflow-y: auto;
}

.bank__head {
  display: flex;
  align-items: center;
  gap: 12px;
}

.bank__back {
  padding: 4px 10px;
  border: 1px solid var(--dialog-border);
  border-radius: var(--radius-btn);
  background: transparent;
  color: var(--fg-muted);
  cursor: pointer;
  font: inherit;
  font-size: 12px;
}

.bank__title {
  font-size: 18px;
  font-weight: 500;
  letter-spacing: 0.06em;
}

.bank__meta {
  margin-left: auto;
  color: var(--fg-muted);
  font-size: 11px;
}

.bank__note {
  color: var(--fg-muted);
  font-size: 11px;
  line-height: 1.7;
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
  gap: 10px;
  padding: 8px 4px;
  border-bottom: 1px solid rgba(255, 255, 255, 0.06);
  font-size: 12px;
  grid-template-columns: 52px 1fr 110px 52px;
}

.row__id {
  color: var(--fg-muted);
}

.row__stat {
  color: var(--fg-muted);
  text-align: right;
}

.row__cool {
  color: var(--accent);
  text-align: right;
}

.row__cool--on {
  color: var(--fg-muted);
}
</style>

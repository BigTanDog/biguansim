<script setup lang="ts">
import { computed, ref, watch } from 'vue'

import { useSessionStore } from '../stores/session'

const PRESETS = [25, 45, 60, 90, 120, 180] as const
const MIN_MINUTES = 15
const MAX_MINUTES = 480
const GOAL_MAX_LEN = 24
/** 题库总数低于这个数才出"阵法简陋"黄字（PRD F-8.3 / A-16） */
const THIN_BANK_THRESHOLD = 30

const store = useSessionStore()
const duration = ref(120)
const goal = ref('')

// 「再入一关」预填上次时长（PRD F-7.6）：配置到达或被更新时同步
watch(
  () => store.lastDurationMin,
  (value) => {
    if (value >= MIN_MINUTES && value <= MAX_MINUTES) {
      duration.value = value
    }
  },
  { immediate: true },
)

const validDuration = computed(() => duration.value >= MIN_MINUTES && duration.value <= MAX_MINUTES)
const thinBank = computed(() => store.bankTotal > 0 && store.bankTotal < THIN_BANK_THRESHOLD)

async function submit(): Promise<void> {
  if (!validDuration.value || store.busy) {
    return
  }
  await store.start(duration.value, goal.value)
}

function pick(preset: number): void {
  duration.value = preset
}
</script>

<template>
  <main class="setup">
    <header class="setup__head">
      <h1 class="setup__title">闭关模拟器</h1>
      <p class="setup__hint">选定闭关时长，入关后电脑将被全屏接管。</p>
    </header>

    <section v-if="store.lastSession" class="notice">
      <p class="notice__title">上次闭关异常中断，已自动解除。</p>
      <p class="notice__body">
        本次时长照常计入累计，但不计为有效日。如仍有残留的暗屏，请在设置里恢复默认亮度。
      </p>
    </section>

    <section class="card">
      <h2 class="card__title">闭关时长</h2>
      <div class="presets">
        <button
          v-for="preset in PRESETS"
          :key="preset"
          class="preset"
          :class="{ 'preset--active': duration === preset }"
          type="button"
          @click="pick(preset)"
        >
          {{ preset }} 分钟
        </button>
      </div>

      <label class="custom">
        <span class="custom__label"
          >自定义（{{ MIN_MINUTES }}–{{ MAX_MINUTES }} 分钟，步进 5）</span
        >
        <input v-model.number="duration" class="input" type="number" min="15" max="480" step="5" />
      </label>

      <p v-if="duration < 25" class="tip">本次不计为有效日（当日累计 ≥25 分钟仍算）</p>
    </section>

    <section class="card">
      <h2 class="card__title">阵法</h2>
      <p class="bank">
        {{ store.bankName }} · 可抽 {{ store.bankTotal }} 题 / 共 {{ store.bankTotal }} 题
      </p>
      <p v-if="thinBank" class="bank-warn">阵法简陋（共 {{ store.bankTotal }} 题），易被强破</p>
      <label class="custom">
        <span class="custom__label">本次目标（非必填，最多 {{ GOAL_MAX_LEN }} 字）</span>
        <input
          v-model="goal"
          class="input"
          type="text"
          :maxlength="GOAL_MAX_LEN"
          placeholder="例如：背完 List 5–7"
        />
      </label>
    </section>

    <p v-if="store.error" class="error">入关失败：{{ store.error }}</p>
    <p v-if="store.lastError" class="error">{{ store.lastError }}</p>

    <button class="enter" type="button" :disabled="!validDuration || store.busy" @click="submit">
      入关
    </button>

    <p class="footnote">
      闭关中可点右下角「解开阵法」退出：先冷却，再答问阵，全对方可出关；第 3
      次起还要抄写。遇急事可长按 Esc 八秒走破戒通道（记为一次破戒）。Ctrl+Alt+Del 与 Win
      键拦不住，详见「关于」页的已知局限。
    </p>
  </main>
</template>

<style scoped>
.setup {
  display: flex;
  flex-direction: column;
  gap: 16px;
  width: min(560px, 100%);
  height: 100%;
  margin: 0 auto;
  padding: 28px 24px;
  overflow-y: auto;
}

.setup__title {
  font-size: 20px;
  font-weight: 500;
  letter-spacing: 0.06em;
}

.setup__hint {
  margin-top: 6px;
  color: var(--fg-muted);
  font-size: 12px;
}

.card {
  padding: 16px;
  border: 1px solid var(--dialog-border);
  border-radius: var(--radius-card);
  background: var(--dialog-bg);
}

.card__title {
  margin-bottom: 12px;
  color: var(--fg-muted);
  font-size: 12px;
  font-weight: 400;
  letter-spacing: 0.1em;
}

.presets {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 8px;
}

.preset {
  padding: 10px 0;
  border: 1px solid var(--dialog-border);
  border-radius: var(--radius-btn);
  background: transparent;
  font-size: 13px;
}

.preset--active {
  border-color: var(--accent);
  color: var(--accent);
}

.custom {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-top: 14px;
}

.custom__label {
  color: var(--fg-muted);
  font-size: 11px;
}

.input {
  padding: 8px 10px;
  border: 1px solid var(--dialog-border);
  border-radius: var(--radius-btn);
  background: transparent;
  color: var(--fg);
  font-family: inherit;
  font-size: 13px;
}

.input:focus {
  border-color: var(--accent);
  outline: none;
}

.tip {
  margin-top: 10px;
  color: var(--fg-muted);
  font-size: 11px;
}

.bank {
  font-size: 13px;
}

/* 黄字只在"题库总数 < 30"时出现；可抽数少是勤奋的结果，不算简陋（F-8.3） */
.bank-warn {
  margin-top: 6px;
  color: #c9a227;
  font-size: 11px;
}

.enter {
  padding: 12px;
  border: 1px solid var(--accent);
  border-radius: var(--radius-btn);
  background: var(--accent);
  color: #14110e;
  font-size: 15px;
  letter-spacing: 0.2em;
}

.enter:disabled {
  border-color: var(--dialog-border);
  background: transparent;
  color: var(--fg-muted);
  cursor: not-allowed;
}

.notice {
  padding: 12px 14px;
  border-left: 2px solid var(--accent);
  background: var(--dialog-bg);
}

.notice__title {
  font-size: 13px;
}

.notice__body {
  margin-top: 4px;
  color: var(--fg-muted);
  font-size: 12px;
  line-height: 1.6;
}

.error {
  color: #d4695f;
  font-size: 12px;
}

.footnote {
  color: var(--fg-muted);
  font-size: 11px;
  line-height: 1.6;
}
</style>

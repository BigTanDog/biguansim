<script setup lang="ts">
import { getCurrentWindow } from '@tauri-apps/api/window'
import { computed, onUnmounted, ref } from 'vue'

import { formatClockLabel, formatCountdown, formatElapsed, SKEW_THRESHOLD_MIN } from '../lib/format'
import { useSessionStore } from '../stores/session'

const store = useSessionStore()

// 副屏窗口 label 是 display-N：只渲染环与倒计时，不承载任何交互（TDD §4.3）
const isMirror = getCurrentWindow().label !== 'main'

const RADIUS = 92
const CIRCUMFERENCE = 2 * Math.PI * RADIUS

const now = ref(new Date())
const clockTimer = window.setInterval(() => {
  now.value = new Date()
}, 1_000)
onUnmounted(() => window.clearInterval(clockTimer))

const dash = computed(() => store.progress * CIRCUMFERENCE)
const clockLabel = computed(() => formatClockLabel(now.value, store.clockSkewMin))
const clockSuspect = computed(() => Math.abs(store.clockSkewMin) > SKEW_THRESHOLD_MIN)
</script>

<template>
  <main class="lock" :class="{ 'lock--mirror': isMirror }">
    <header v-if="!isMirror" class="lock__head">
      <span>静修 · 阵法：基础阵法</span>
      <span v-if="store.goal" class="lock__goal">目标：{{ store.goal }}</span>
    </header>

    <section class="stage">
      <svg class="ring" viewBox="0 0 200 200" aria-hidden="true">
        <circle class="ring__track" cx="100" cy="100" :r="RADIUS" />
        <circle
          class="ring__arc"
          cx="100"
          cy="100"
          :r="RADIUS"
          :stroke-dasharray="`${dash} ${CIRCUMFERENCE - dash}`"
        />
      </svg>

      <div class="readout">
        <p class="readout__label">剩余</p>
        <p class="readout__value num">{{ formatCountdown(store.remainingMs) }}</p>
        <p class="readout__sub">已坚持 {{ formatElapsed(store.elapsedMs) }}</p>
      </div>
    </section>

    <section v-if="!isMirror" class="dialog">
      <p class="dialog__speaker">长老</p>
      <p class="dialog__line">道友，且坐。此关已开，莫问还有几时——问，便是想走。</p>
      <p class="dialog__line dialog__line--muted">
        本版破戒通道尚未启用：如需退出，请用任务管理器结束进程，本次将记为「异常中断」，时长仍计入累计。
      </p>
    </section>

    <p
      v-if="!isMirror"
      class="clock num"
      :class="{ 'clock--warn': clockSuspect }"
      :title="clockSuspect ? '检测到系统时间被调整；剩余时间以单调计时为准' : ''"
    >
      {{ clockLabel }}
    </p>
  </main>
</template>

<style scoped>
.lock {
  position: relative;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: space-between;
  height: 100%;
  padding: 3vh 4vw;
  /* 视觉压暗：纯 UI 层，无系统副作用（F-3.10） */
  background: color-mix(in srgb, var(--bg) 88%, #000);
}

.lock--mirror {
  justify-content: center;
  padding: 0;
}

.lock__head {
  display: flex;
  gap: 24px;
  align-self: stretch;
  color: var(--fg-muted);
  font-size: 12px;
}

.stage {
  position: relative;
  display: grid;
  flex: 1;
  place-items: center;
  align-self: stretch;
}

.ring {
  width: min(46vh, 46vw);
  height: min(46vh, 46vw);
  transform: rotate(-90deg);
}

.ring__track {
  fill: none;
  stroke: rgba(255, 255, 255, 0.08);
  stroke-width: 1.5;
}

.ring__arc {
  fill: none;
  stroke: var(--accent);
  stroke-linecap: round;
  stroke-width: 1.5;
  transition: stroke-dasharray 250ms linear;
}

.readout {
  position: absolute;
  text-align: center;
}

.readout__label {
  color: var(--fg-muted);
  font-size: 12px;
  letter-spacing: 0.3em;
}

.readout__value {
  margin-top: 4px;
  font-size: min(11vh, 88px);
  letter-spacing: 0.04em;
  line-height: 1.15;
}

.readout__sub {
  margin-top: 6px;
  color: var(--fg-muted);
  font-size: 12px;
}

.dialog {
  position: relative;
  align-self: stretch;
  min-height: 120px;
  padding: 18px 20px;
  border: 1px solid var(--dialog-border);
  border-radius: var(--radius-card);
  background: var(--dialog-bg);
}

.dialog__speaker {
  position: absolute;
  top: -10px;
  left: 18px;
  padding: 0 6px;
  background: var(--dialog-bg);
  color: var(--accent);
  font-size: 12px;
}

.dialog__line {
  font-size: 14px;
  line-height: 1.9;
}

.dialog__line--muted {
  margin-top: 8px;
  color: var(--fg-muted);
  font-size: 12px;
}

.clock {
  position: absolute;
  right: 16px;
  bottom: 12px;
  color: var(--fg-muted);
  font-size: 11px;
}

.clock--warn {
  color: #d4695f;
}
</style>

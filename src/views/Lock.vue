<script setup lang="ts">
import { getCurrentWindow } from '@tauri-apps/api/window'
import { computed, onUnmounted, ref } from 'vue'

import ConfirmPanel from '../components/ConfirmPanel.vue'
import CopyPanel from '../components/CopyPanel.vue'
import EscRing from '../components/EscRing.vue'
import ExitPanel from '../components/ExitPanel.vue'
import { formatClockLabel, formatCountdown, formatElapsed, SKEW_THRESHOLD_MIN } from '../lib/format'
import * as ipc from '../lib/ipc'
import { useSessionStore } from '../stores/session'

const store = useSessionStore()

// 副屏窗口 label 是 display-N：只渲染环与倒计时，不承载任何交互（TDD §4.3）
const isMirror = getCurrentWindow().label !== 'main'

/** 开发期逃生口：由 Rust 告知是否 debug 构建（release 里既无按钮也无命令） */
const devMode = computed(() => store.debugBuild)

function devExit(): void {
  ipc.devAbort().catch((err: unknown) => {
    store.notify('error', `DEV 退出失败：${String(err)}`)
  })
}

const RADIUS = 92
const CIRCUMFERENCE = 2 * Math.PI * RADIUS

const now = ref(new Date())
const clockTimer = window.setInterval(() => {
  now.value = new Date()
}, 1_000)
onUnmounted(() => window.clearInterval(clockTimer))

const unlocking = ref(false)

async function unlock(): Promise<void> {
  if (unlocking.value) {
    return
  }
  unlocking.value = true
  await store.requestUnlock()
  unlocking.value = false
}

const dash = computed(() => store.progress * CIRCUMFERENCE)
const clockLabel = computed(() => formatClockLabel(now.value, store.clockSkewMin))
const clockSuspect = computed(() => Math.abs(store.clockSkewMin) > SKEW_THRESHOLD_MIN)

/**
 * 「正在通知长老，请稍候。」是**看守弟子**去通报，不是长老自己说自己。
 * 其余场景（叙述/问阵/抄写/破戒确认）都是长老在讲。
 */
const speaker = computed(() => (store.state === 'Cool' ? '看守弟子' : '长老'))

const exitPayload = computed(() =>
  store.payload && (store.payload.kind === 'cool' || store.payload.kind === 'challenge')
    ? store.payload
    : null,
)
const copyPayload = computed(() => (store.payload?.kind === 'copy' ? store.payload : null))
const confirmPayload = computed(() => (store.payload?.kind === 'confirm' ? store.payload : null))
</script>

<template>
  <main class="lock" :class="{ 'lock--mirror': isMirror }">
    <header v-if="!isMirror" class="lock__head">
      <span>静修 · 阵法：基础阵法</span>
      <span v-if="store.goal" class="lock__goal">目标：{{ store.goal }}</span>
      <span v-if="devMode" class="lock__dev">测试版：正确项固定第一项 · 冷却 ×1/10</span>
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
      <p class="dialog__speaker">{{ speaker }}</p>

      <div class="dialog__body">
        <!-- Locked：叙述 + 常驻的「解开阵法」（U-5：不隐藏、不淡化） -->
        <template v-if="store.state === 'Locked'">
          <p class="dialog__line">已入关。若遇急事，长按 Esc 八秒可破戒。</p>
          <p class="dialog__line dialog__line--muted">
            道友，此关已开，莫问还有几时——问，便是想走。
          </p>
          <div class="dialog__actions">
            <button class="dialog__unlock" type="button" :disabled="unlocking" @click="unlock">
              解开阵法
            </button>
          </div>
        </template>

        <!-- Cool / Challenge -->
        <ExitPanel
          v-else-if="exitPayload"
          :payload="exitPayload"
          :cool-remain-ms="store.coolRemainMs"
          :elapsed-ms="store.elapsedMs"
          :notice="store.notice"
          @choose="store.choose"
        />

        <!-- Copying -->
        <CopyPanel v-else-if="copyPayload" :text="copyPayload.text" :chars="copyPayload.chars" />

        <!-- Confirming -->
        <ConfirmPanel
          v-else-if="confirmPayload"
          :sentence="confirmPayload.sentence"
          :elapsed-ms="confirmPayload.elapsedMs"
          :faults="confirmPayload.faults"
        />
      </div>
    </section>

    <EscRing
      v-if="!isMirror && store.state === 'Locked' && store.escHoldMs > 0"
      :held-ms="store.escHoldMs"
    />

    <button v-if="devMode && !isMirror" class="dev-exit" type="button" @click="devExit">
      DEV 退出（不记录）
    </button>

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

.lock__dev {
  margin-left: auto;
  color: #8c8375;
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
  min-height: 132px;
  padding: 20px 20px 16px;
  border: 1px solid var(--dialog-border);
  border-radius: var(--radius-card);
  background: var(--dialog-bg);
  /* 不能在这里滚动：说话人标签是 top:-10px 的绝对定位元素，
     父级一旦 overflow:auto 就会把它裁掉（2026-10-21 实测"长老看不全"）。
     滚动交给内层 .dialog__body。 */
  overflow: visible;
}

.dialog__body {
  max-height: 38vh;
  overflow: auto;
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

.dialog__actions {
  display: flex;
  justify-content: flex-end;
  margin-top: 10px;
}

.dialog__unlock {
  padding: 7px 18px;
  border: 1px solid var(--dialog-border);
  border-radius: var(--radius-btn);
  background: transparent;
  color: var(--fg-muted);
  cursor: pointer;
  font: inherit;
  font-size: 13px;
}

.dialog__unlock:hover:enabled {
  border-color: var(--accent);
  color: var(--accent);
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

/* 开发期唯一可靠的退出口：鼠标路径不经过键盘钩子。
   2026-10-20 实测键盘组合键可能被系统热键吃掉，所以这里必须显眼、可点。 */
.dev-exit {
  position: absolute;
  right: 16px;
  bottom: 34px;
  padding: 8px 16px;
  border: 1px solid #8c8375;
  border-radius: var(--radius-btn);
  background: rgba(20, 17, 14, 0.9);
  color: #c9b99a;
  font-size: 12px;
  cursor: pointer;
  z-index: 5;
}

.dev-exit:hover {
  border-color: var(--accent);
  color: var(--accent);
}
</style>

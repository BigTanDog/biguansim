<script setup lang="ts">
// 破戒进度环（PRD F-6.1）。**只渲染**：8 秒判定在 Rust 的钩子里，
// 前端连"有没有满"都不判断，只把 heldMs 画成弧长。
import { computed } from 'vue'

const props = defineProps<{ heldMs: number }>()

/** 与 Rust 的 ESC_HOLD_TRIGGER_MS 一致，仅用于画满环 */
const TOTAL_MS = 8_000
const RADIUS = 54
const CIRCUMFERENCE = 2 * Math.PI * RADIUS

const ratio = computed(() => Math.min(1, Math.max(0, props.heldMs / TOTAL_MS)))
const dash = computed(() => CIRCUMFERENCE * ratio.value)
const seconds = computed(() => Math.min(8, Math.floor(props.heldMs / 1_000)))
</script>

<template>
  <div class="esc" role="status" aria-live="polite">
    <svg class="esc__ring" viewBox="0 0 120 120" aria-hidden="true">
      <circle class="esc__track" cx="60" cy="60" :r="RADIUS" />
      <circle
        class="esc__arc"
        cx="60"
        cy="60"
        :r="RADIUS"
        :stroke-dasharray="`${dash} ${CIRCUMFERENCE - dash}`"
      />
    </svg>
    <div class="esc__text">
      <p class="esc__title">破戒需谨慎</p>
      <p class="esc__hint">继续按住 Esc 满八秒（{{ seconds }}/8）</p>
    </div>
  </div>
</template>

<style scoped>
.esc {
  position: fixed;
  inset: 0;
  display: grid;
  place-items: center;
  background: rgba(10, 8, 6, 0.55);
  pointer-events: none;
  z-index: 8;
}

.esc__ring {
  width: min(34vh, 34vw);
  height: min(34vh, 34vw);
  transform: rotate(-90deg);
}

.esc__track {
  fill: none;
  stroke: rgba(255, 255, 255, 0.1);
  stroke-width: 2;
}

.esc__arc {
  fill: none;
  stroke: #d4695f;
  stroke-linecap: round;
  stroke-width: 2;
  transition: stroke-dasharray 250ms linear;
}

.esc__text {
  position: absolute;
  text-align: center;
}

.esc__title {
  color: #e8c9a0;
  font-size: 15px;
  letter-spacing: 0.2em;
}

.esc__hint {
  margin-top: 8px;
  color: var(--fg-muted);
  font-size: 12px;
}
</style>

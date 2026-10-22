<script setup lang="ts">
import { getCurrentWindow } from '@tauri-apps/api/window'
import { computed, onMounted, onUnmounted } from 'vue'

import { useSessionStore } from '../stores/session'

const store = useSessionStore()

// 副屏镜像（label = display-N）不可聚焦，也收不到按键：只显示数字，不显示 Esc 提示
const isMirror = getCurrentWindow().label !== 'main'

// 3 → 2 → 1：剩余毫秒由 Rust 推送，这里只做向上取整展示
const seconds = computed(() => Math.max(1, Math.ceil(store.armingRemainingMs / 1000)))

function onKeydown(event: KeyboardEvent): void {
  if (event.key !== 'Escape') {
    return
  }
  event.preventDefault()
  void store.cancelArming()
}

onMounted(() => {
  if (!isMirror) {
    window.addEventListener('keydown', onKeydown)
  }
})

onUnmounted(() => {
  window.removeEventListener('keydown', onKeydown)
})
</script>

<template>
  <main class="arming">
    <p class="arming__count num">{{ seconds }}</p>
    <template v-if="!isMirror">
      <p class="arming__hint">此时取消还来得及（Esc）</p>
      <p class="arming__note">三秒之后全屏接管，键盘也归它管。</p>
    </template>
  </main>
</template>

<style scoped>
.arming {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 18px;
  height: 100%;
  background: #0b0906;
}

.arming__count {
  color: var(--fg);
  font-size: min(28vh, 240px);
  line-height: 1;
}

.arming__hint {
  color: var(--accent);
  font-size: 14px;
  letter-spacing: 0.2em;
}

.arming__note {
  color: var(--fg-muted);
  font-size: 11px;
}
</style>

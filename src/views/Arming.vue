<script setup lang="ts">
import { computed, onMounted, onUnmounted } from 'vue'

import { useSessionStore } from '../stores/session'

const store = useSessionStore()

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
  window.addEventListener('keydown', onKeydown)
})

onUnmounted(() => {
  window.removeEventListener('keydown', onKeydown)
})
</script>

<template>
  <main class="arming">
    <p class="arming__count num">{{ seconds }}</p>
    <p class="arming__hint">此时取消还来得及（Esc）</p>
    <p class="arming__note">切走窗口后 Esc 就无效了——全局拦截要到 M3 才生效。</p>
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

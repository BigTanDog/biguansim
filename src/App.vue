<script setup lang="ts">
import { onMounted, onUnmounted } from 'vue'

import * as ipc from './lib/ipc'
import { useSessionStore } from './stores/session'
import Arming from './views/Arming.vue'
import Lock from './views/Lock.vue'
import Settled from './views/Settled.vue'
import Setup from './views/Setup.vue'

const store = useSessionStore()

/** 开发期逃生口：只有 dev 构建才挂，正式产物里既没有命令也没有这个监听 */
const DEV = import.meta.env.DEV

/**
 * 两条组合键：`Ctrl+Shift+F12` 与 `Ctrl+Alt+Shift+X`。
 * 后者的 Alt+Shift 是系统输入法切换热键，可能先被系统吃掉（2026-10-20 实测失效），
 * 所以前者是主用、后者兼容旧习惯；闭关页还常驻一个可见按钮作为最后手段。
 */
function isDevExitCombo(event: KeyboardEvent): boolean {
  const f12 = event.ctrlKey && event.shiftKey && !event.altKey && event.code === 'F12'
  const x = event.ctrlKey && event.altKey && event.shiftKey && event.code === 'KeyX'
  return f12 || x
}

function onDevKeydown(event: KeyboardEvent): void {
  if (!DEV || !isDevExitCombo(event)) {
    return
  }
  event.preventDefault()
  ipc.devAbort().catch((err: unknown) => {
    // 静默失败过一次（2026-10-20 按键退出无效却没有任何提示），必须看得见
    store.notify('error', `DEV 退出失败：${String(err)}`)
  })
}

/** 闭关期间屏蔽刷新类快捷键与右键菜单（TDD §5.4） */
function onBlockedKeydown(event: KeyboardEvent): void {
  if (store.phase !== 'lock' && store.phase !== 'arming') {
    return
  }

  const refresh = event.key === 'F5' || (event.ctrlKey && event.key.toLowerCase() === 'r')
  if (refresh) {
    event.preventDefault()
  }
}

function onContextMenu(event: MouseEvent): void {
  if (store.phase === 'lock' || store.phase === 'arming') {
    event.preventDefault()
  }
}

onMounted(() => {
  void store.init()
  window.addEventListener('keydown', onDevKeydown)
  window.addEventListener('keydown', onBlockedKeydown)
  window.addEventListener('contextmenu', onContextMenu)
})

onUnmounted(() => {
  window.removeEventListener('keydown', onDevKeydown)
  window.removeEventListener('keydown', onBlockedKeydown)
  window.removeEventListener('contextmenu', onContextMenu)
})
</script>

<template>
  <Setup v-if="store.phase === 'setup'" />
  <Arming v-else-if="store.phase === 'arming'" />
  <Lock v-else-if="store.phase === 'lock'" />
  <Settled v-else />

  <Transition name="toast">
    <div v-if="store.toast" class="toast" :class="`toast--${store.toast.level}`">
      {{ store.toast.text }}
    </div>
  </Transition>
</template>

<style scoped>
.toast {
  position: fixed;
  top: 16px;
  left: 50%;
  padding: 8px 16px;
  border: 1px solid var(--dialog-border);
  border-radius: var(--radius-btn);
  background: var(--dialog-bg);
  color: var(--fg);
  font-size: 12px;
  transform: translateX(-50%);
  z-index: 10;
}

.toast--error {
  border-color: #d4695f;
  color: #d4695f;
}

.toast--warn {
  border-color: var(--accent);
  color: var(--accent);
}

.toast-enter-active,
.toast-leave-active {
  transition: opacity 200ms ease;
}

.toast-enter-from,
.toast-leave-to {
  opacity: 0;
}
</style>

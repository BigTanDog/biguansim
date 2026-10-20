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

function onDevKeydown(event: KeyboardEvent): void {
  if (!DEV) {
    return
  }
  if (event.ctrlKey && event.altKey && event.shiftKey && event.code === 'KeyX') {
    event.preventDefault()
    void ipc.devAbort()
  }
}

onMounted(() => {
  void store.init()
  if (DEV) {
    window.addEventListener('keydown', onDevKeydown)
  }
})

onUnmounted(() => {
  window.removeEventListener('keydown', onDevKeydown)
})
</script>

<template>
  <Setup v-if="store.phase === 'setup'" />
  <Arming v-else-if="store.phase === 'arming'" />
  <Lock v-else-if="store.phase === 'lock'" />
  <Settled v-else />
</template>

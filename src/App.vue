<script setup lang="ts">
import { onMounted } from 'vue'

import { useSessionStore } from './stores/session'
import Arming from './views/Arming.vue'
import Lock from './views/Lock.vue'
import Settled from './views/Settled.vue'
import Setup from './views/Setup.vue'

const store = useSessionStore()

onMounted(() => {
  void store.init()
})
</script>

<template>
  <Setup v-if="store.phase === 'setup'" />
  <Arming v-else-if="store.phase === 'arming'" />
  <Lock v-else-if="store.phase === 'lock'" />
  <Settled v-else />
</template>

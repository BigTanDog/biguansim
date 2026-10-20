<script setup lang="ts">
import { onMounted } from 'vue'

import { useSessionStore } from './stores/session'
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
  <Lock v-else-if="store.phase === 'lock'" />
  <Settled v-else />
</template>

// 只保存"最近一次 Rust 推送的投影"，不做业务派生（TDD §3.2）。
import { defineStore } from 'pinia'
import { computed, ref } from 'vue'

import * as ipc from '../lib/ipc'

export type Phase = 'setup' | 'lock' | 'settled'

export const useSessionStore = defineStore('session', () => {
  const phase = ref<Phase>('setup')
  const ready = ref(false)
  const busy = ref(false)
  const error = ref<string | null>(null)

  const targetMs = ref(0)
  const elapsedMs = ref(0)
  const remainingMs = ref(0)
  const clockSkewMin = ref(0)
  const goal = ref<string | null>(null)
  const bank = ref('basic')
  const result = ref<string | null>(null)
  const lastSession = ref<ipc.LastSession | null>(null)
  const lastError = ref<string | null>(null)

  const progress = computed(() => {
    if (targetMs.value <= 0) {
      return 0
    }
    return Math.min(1, Math.max(0, elapsedMs.value / targetMs.value))
  })

  function applyState(state: ipc.StateView): void {
    targetMs.value = state.targetMs
    elapsedMs.value = state.elapsedMs
    remainingMs.value = state.remainingMs
    clockSkewMin.value = state.clockSkewMin
    goal.value = state.goal
    bank.value = state.bank
    phase.value =
      state.state === 'Locked' ? 'lock' : state.state === 'Settled' ? 'settled' : 'setup'
  }

  async function init(): Promise<void> {
    if (ready.value) {
      return
    }

    try {
      const boot = await ipc.bootstrap()
      lastSession.value = boot.lastSession
      applyState(await ipc.getState())

      await ipc.onTick((tick) => {
        elapsedMs.value = tick.elapsedMs
        remainingMs.value = tick.remainingMs
        clockSkewMin.value = tick.clockSkewMin
      })

      await ipc.onStateChanged((changed) => {
        if (changed.state === 'Locked') {
          phase.value = 'lock'
          targetMs.value = changed.targetMs
          goal.value = changed.goal
        } else if (changed.state === 'Settled') {
          phase.value = 'settled'
          result.value = changed.result
          elapsedMs.value = changed.elapsedMs
        }
      })
    } catch (err) {
      lastError.value = String(err)
    } finally {
      ready.value = true
    }
  }

  async function start(durationMin: number, goalText: string): Promise<boolean> {
    busy.value = true
    error.value = null

    try {
      const trimmed = goalText.trim()
      await ipc.startSession(durationMin, trimmed.length > 0 ? trimmed : null, 'basic')
      applyState(await ipc.getState())
      return true
    } catch (err) {
      error.value = String(err)
      return false
    } finally {
      busy.value = false
    }
  }

  function backToSetup(): void {
    phase.value = 'setup'
    result.value = null
    elapsedMs.value = 0
    remainingMs.value = 0
    targetMs.value = 0
  }

  return {
    phase,
    ready,
    busy,
    error,
    lastError,
    targetMs,
    elapsedMs,
    remainingMs,
    clockSkewMin,
    goal,
    bank,
    result,
    lastSession,
    progress,
    init,
    start,
    backToSetup,
  }
})

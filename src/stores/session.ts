// 只保存"最近一次 Rust 推送的投影"，不做业务派生（TDD §3.2）。
import { defineStore } from 'pinia'
import { computed, ref } from 'vue'

import { ensureAudio, playChime } from '../lib/chime'
import * as ipc from '../lib/ipc'

export type Phase = 'setup' | 'arming' | 'lock' | 'settled'

export const useSessionStore = defineStore('session', () => {
  const phase = ref<Phase>('setup')
  const ready = ref(false)
  const busy = ref(false)
  const error = ref<string | null>(null)

  const config = ref<ipc.Config | null>(null)
  const targetMs = ref(0)
  const elapsedMs = ref(0)
  const remainingMs = ref(0)
  const armingRemainingMs = ref(0)
  const clockSkewMin = ref(0)
  const goal = ref<string | null>(null)
  const bank = ref('basic')
  const result = ref<string | null>(null)
  const lastSession = ref<ipc.LastSession | null>(null)
  const lastError = ref<string | null>(null)
  const toast = ref<ipc.Toast | null>(null)
  let toastTimer: number | undefined

  const lastDurationMin = computed(() => config.value?.lastDurationMin ?? 120)

  /** 顶部提示条：Rust 推来的 toast 与前端自身的失败提示共用一条通道 */
  function notify(level: string, text: string): void {
    toast.value = { level, text }
    window.clearTimeout(toastTimer)
    toastTimer = window.setTimeout(() => {
      toast.value = null
    }, 4_000)
  }

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
      state.state === 'Arming'
        ? 'arming'
        : state.state === 'Locked'
          ? 'lock'
          : state.state === 'Settled'
            ? 'settled'
            : 'setup'
  }

  async function init(): Promise<void> {
    if (ready.value) {
      return
    }

    try {
      const boot = await ipc.bootstrap()
      config.value = boot.config
      lastSession.value = boot.lastSession
      applyState(await ipc.getState())

      await ipc.onTick((tick) => {
        elapsedMs.value = tick.elapsedMs
        remainingMs.value = tick.remainingMs
        clockSkewMin.value = tick.clockSkewMin
      })

      await ipc.onArmingTick((tick) => {
        armingRemainingMs.value = tick.remainingMs
      })

      await ipc.onToast((incoming) => {
        notify(incoming.level, incoming.text)
      })

      await ipc.onChime((chime) => {
        // 音量取配置（PRD F-10.3），上限 60%
        const volume = Math.min(0.6, Math.max(0, config.value?.chimeVolume ?? 0.6))
        playChime(chime, volume)
      })

      await ipc.onStateChanged((changed) => {
        switch (changed.state) {
          case 'Arming':
            phase.value = 'arming'
            targetMs.value = changed.targetMs
            goal.value = changed.goal
            break
          case 'Locked':
            phase.value = 'lock'
            targetMs.value = changed.targetMs
            goal.value = changed.goal
            // 首帧就要有数字：否则要等第一次 tick（≤250 ms）才从 00:00 跳过来
            remainingMs.value = changed.targetMs
            elapsedMs.value = 0
            clockSkewMin.value = 0
            break
          case 'Settled':
            phase.value = 'settled'
            result.value = changed.result
            elapsedMs.value = changed.elapsedMs
            break
          default:
            phase.value = 'setup'
            result.value = null
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
      // 自动播放策略要求音频上下文在用户手势里创建
      ensureAudio()

      const trimmed = goalText.trim()
      await ipc.startSession(durationMin, trimmed.length > 0 ? trimmed : null, 'basic')

      if (config.value) {
        config.value.lastDurationMin = durationMin
      }
      armingRemainingMs.value = 3_000
      phase.value = 'arming'
      return true
    } catch (err) {
      error.value = String(err)
      return false
    } finally {
      busy.value = false
    }
  }

  async function cancelArming(): Promise<void> {
    try {
      await ipc.cancelArming()
      phase.value = 'setup'
    } catch (err) {
      error.value = String(err)
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
    config,
    targetMs,
    elapsedMs,
    remainingMs,
    armingRemainingMs,
    clockSkewMin,
    goal,
    bank,
    result,
    lastSession,
    lastDurationMin,
    toast,
    progress,
    notify,
    init,
    start,
    cancelArming,
    backToSetup,
  }
})

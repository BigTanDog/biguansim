// 只保存"最近一次 Rust 推送的投影"，不做业务派生（TDD §3.2）。
import { defineStore } from 'pinia'
import { computed, ref } from 'vue'

import { ensureAudio, playChime } from '../lib/chime'
import * as ipc from '../lib/ipc'

/** 视图路由只有四个：闭关中的五种投影共用 lock */
export type Phase = 'setup' | 'arming' | 'lock' | 'settled'

/** 错误码 → 人话（PRD §12 / F-4.5）。未收录的码原样显示，便于排障 */
const ERROR_TEXT: Record<string, string> = {
  E_REMAIN_TOO_SHORT: '距出关不足一次冷却，请稍候到点。',
  E_CONFIRM_MISMATCH: '抄写不一致，请逐字核对后再确认。',
  E_INVALID_CHOICE: '这一题已失效，界面已刷新。',
  E_BANK_INVALID: '题库不可用，无法出题。',
  E_STATE_ILLEGAL: '状态已变化，请重试。',
  E_HOOK_FAILED: '结界未能生效，已取消入关。',
  E_OVERLAY_FAILED: '窗口未能接管屏幕。',
  E_STORE_FAILED: '记录未能保存。',
}

export function errorText(err: unknown): string {
  const raw = String(err)
  const code = raw.split(':')[0]?.trim() ?? raw
  return ERROR_TEXT[code] ?? raw
}

export const useSessionStore = defineStore('session', () => {
  const phase = ref<Phase>('setup')
  const state = ref<ipc.SessionStateName>('Idle')
  const payload = ref<ipc.PhasePayload | null>(null)
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
  /** 冷却剩余（Rust 每 250 ms 给一次事实，界面只显示分钟粒度） */
  const coolRemainMs = ref(0)
  /** Esc 已按住毫秒数（只用于画进度环，判定在 Rust） */
  const escHoldMs = ref(0)
  /** 对话框内的一次性提示，例如"剩余不足一次冷却" */
  const notice = ref<string | null>(null)

  let toastTimer: number | undefined
  let noticeTimer: number | undefined

  const lastDurationMin = computed(() => config.value?.lastDurationMin ?? 120)

  const progress = computed(() => {
    if (targetMs.value <= 0) {
      return 0
    }
    return Math.min(1, Math.max(0, elapsedMs.value / targetMs.value))
  })

  /** 顶部提示条：Rust 推来的 toast 与前端自身的失败提示共用一条通道 */
  function notify(level: string, text: string): void {
    toast.value = { level, text }
    window.clearTimeout(toastTimer)
    toastTimer = window.setTimeout(() => {
      toast.value = null
    }, 4_000)
  }

  function showNotice(text: string): void {
    notice.value = text
    window.clearTimeout(noticeTimer)
    noticeTimer = window.setTimeout(() => {
      notice.value = null
    }, 6_000)
  }

  function setState(next: ipc.SessionStateName): void {
    state.value = next
    phase.value =
      next === 'Arming'
        ? 'arming'
        : next === 'Settled'
          ? 'settled'
          : next === 'Idle'
            ? 'setup'
            : 'lock'

    if (next !== 'Cool') {
      coolRemainMs.value = 0
    }
    if (next !== 'Locked') {
      escHoldMs.value = 0
    }
  }

  function applyState(view: ipc.StateView): void {
    targetMs.value = view.targetMs
    elapsedMs.value = view.elapsedMs
    remainingMs.value = view.remainingMs
    clockSkewMin.value = view.clockSkewMin
    goal.value = view.goal
    bank.value = view.bank
    payload.value = view.payload
    if (view.payload?.kind === 'cool') {
      coolRemainMs.value = view.payload.remainMs
    }
    setState(view.state)
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

      await ipc.onCoolTick((tick) => {
        coolRemainMs.value = tick.remainMs
      })

      await ipc.onEscHold((hold) => {
        escHoldMs.value = hold.heldMs
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
        payload.value = changed.payload
        switch (changed.state) {
          case 'Arming':
            setState('Arming')
            targetMs.value = changed.targetMs
            goal.value = changed.goal
            break
          case 'Locked':
          case 'Cool':
          case 'Challenge':
          case 'Copying':
          case 'Confirming':
            setState(changed.state)
            targetMs.value = changed.targetMs
            goal.value = changed.goal
            if (changed.state === 'Locked') {
              remainingMs.value = changed.targetMs
              clockSkewMin.value = 0
            }
            if (changed.payload?.kind === 'cool') {
              coolRemainMs.value = changed.payload.remainMs
            }
            break
          case 'Settled':
            setState('Settled')
            result.value = changed.result
            elapsedMs.value = changed.elapsedMs
            payload.value = null
            break
          default:
            setState('Idle')
            result.value = null
            payload.value = null
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
      setState('Arming')
      return true
    } catch (err) {
      error.value = errorText(err)
      return false
    } finally {
      busy.value = false
    }
  }

  async function cancelArming(): Promise<void> {
    try {
      await ipc.cancelArming()
      setState('Idle')
    } catch (err) {
      error.value = errorText(err)
    }
  }

  /** 点「解开阵法」：被拒绝时把 F-4.5 的原句显示在对话框里 */
  async function requestUnlock(): Promise<void> {
    try {
      await ipc.requestUnlock()
      notice.value = null
    } catch (err) {
      showNotice(errorText(err))
    }
  }

  async function choose(choice: number): Promise<void> {
    const current = payload.value
    if (current?.kind !== 'challenge') {
      return
    }

    try {
      await ipc.submitChoice(current.id, choice)
    } catch (err) {
      notify('warn', errorText(err))
    }
  }

  /** 返回服务端判定出的首个错字位置；null 表示通过或已失败 */
  async function submitCopy(text: string): Promise<number | null> {
    try {
      const result = await ipc.submitCopy(text)
      return result.ok ? null : result.mismatchIndex
    } catch (err) {
      notify('warn', errorText(err))
      return null
    }
  }

  async function confirmEmergency(text: string): Promise<boolean> {
    try {
      await ipc.confirmEmergency(text)
      return true
    } catch (err) {
      showNotice(errorText(err))
      return false
    }
  }

  async function cancelEmergency(): Promise<void> {
    try {
      await ipc.cancelEmergency()
    } catch (err) {
      notify('warn', errorText(err))
    }
  }

  function backToSetup(): void {
    setState('Idle')
    result.value = null
    payload.value = null
    elapsedMs.value = 0
    remainingMs.value = 0
    targetMs.value = 0
  }

  return {
    phase,
    state,
    payload,
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
    notice,
    coolRemainMs,
    escHoldMs,
    progress,
    notify,
    init,
    start,
    cancelArming,
    requestUnlock,
    choose,
    submitCopy,
    confirmEmergency,
    cancelEmergency,
    backToSetup,
  }
})

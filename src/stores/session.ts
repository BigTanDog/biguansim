// 只保存"最近一次 Rust 推送的投影"，不做业务派生（TDD §3.2）。
import { defineStore } from 'pinia'
import { computed, ref } from 'vue'

import { ensureAudio, playChime } from '../lib/chime'
import * as ipc from '../lib/ipc'

/** 视图路由只有四个：闭关中的五种投影共用 lock */
export type Phase = 'setup' | 'arming' | 'lock' | 'settled'

/** Idle 下的二级页面（F-1.6：不建独立窗口） */
export type View = 'setup' | 'records' | 'bank' | 'settings'

/** 错误码 → 人话（PRD §12 / F-4.5）。未收录的码原样显示，便于排障 */
const ERROR_TEXT: Record<string, string> = {
  E_REMAIN_TOO_SHORT: '距出关不足一次冷却，请稍候到点。',
  E_CONFIRM_MISMATCH: '抄写不一致，请逐字核对后再确认。',
  E_INVALID_CHOICE: '这一题已失效。',
  E_BANK_INVALID: '题库不可用，无法出题。',
  E_STATE_ILLEGAL: '状态已变化，请重试。',
  E_HOOK_FAILED: '结界未能生效，已取消入关。',
  E_OVERLAY_FAILED: '窗口未能接管屏幕。',
  E_STORE_FAILED: '记录未能保存。',
}

export function errorText(err: unknown): string {
  const raw = String(err)
  const [head, ...rest] = raw.split(':')
  const code = head?.trim() ?? raw
  const detail = rest.join(':').trim()

  const mapped = ERROR_TEXT[code]
  if (!mapped) {
    return raw
  }
  // Rust 有时会在错误码后面补具体原因（如"题库至少需要 3 题，当前 2 题"），
  // 直接吞掉会让人看不出到底哪不对，所以把它接在固定文案后面。
  return detail ? `${mapped}（${detail}）` : mapped
}

export const useSessionStore = defineStore('session', () => {
  const phase = ref<Phase>('setup')
  const view = ref<View>('setup')
  /** 累计 / 连续 / 破戒 / 有效日：全部由 Rust 算，前端只显示 */
  const stats = ref<ipc.StatsView | null>(null)
  const recordsPage = ref<ipc.RecordPage | null>(null)
  const questions = ref<ipc.QuestionSummary[]>([])
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
  /** 本次闭关的说话人（阵灵/前辈/冥冥之中的声音/长老）：由 Rust 在入关时随机 */
  const speaker = ref<string | null>(null)
  const bank = ref('basic')
  const bankName = ref('基础阵法')
  const bankTotal = ref(0)
  /** 可抽题数（总数 − 冷却中），F-1.2 要求与总数分开显示 */
  const bankDrawable = ref(0)
  /** debug 构建标记：由 Rust 的 cfg!(debug_assertions) 告知（release 里恒为 false） */
  const debugBuild = ref(false)
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

  /** 首次引导是否还没看（F-11.1）：配置到手且 onboarded 为 false 才算 */
  const needsOnboarding = computed(() => config.value !== null && !config.value.onboarded)

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
    speaker.value = view.speaker
    bankTotal.value = view.bankTotal
    bankDrawable.value = view.bankDrawable
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
      bank.value = boot.bank.id || 'basic'
      bankName.value = boot.bank.name || '基础阵法'
      bankTotal.value = boot.bank.total
      bankDrawable.value = boot.bank.drawable
      debugBuild.value = boot.debugBuild
      stats.value = boot.stats
      lastSession.value = boot.lastSession
      applyTheme(boot.config.theme)
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
        speaker.value = changed.speaker
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
            // 结算页要显示累计与连续天数：出关时重取一次统计
            void refreshStats()
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

  /** 重取一次全量状态：用于"陈旧点击"这类静默收口的场景 */
  async function refresh(): Promise<void> {
    try {
      applyState(await ipc.getState())
    } catch (err) {
      lastError.value = String(err)
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
      // 连点两下时第二下带的是旧题号，后端会正确拒绝：这种情况静默重取，
      // 不要把"界面已刷新"这种承诺了却没做的事弹给用户（2026-10-21 审阅意见）
      if (String(err).startsWith('E_INVALID_CHOICE')) {
        await refresh()
        return
      }
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
    view.value = 'setup'
    result.value = null
    payload.value = null
    elapsedMs.value = 0
    remainingMs.value = 0
    targetMs.value = 0
  }

  async function refreshStats(): Promise<void> {
    try {
      const page = await ipc.listRecords(1)
      stats.value = page.stats
      // 可抽题数会随答题变化，顺手同步一次（否则配置页的数字会停在启动那一刻）
      const boot = await ipc.getState()
      applyState(boot)
    } catch (err) {
      lastError.value = String(err)
    }
  }

  /** Idle 下打开记录页（F-1.6 的二级入口） */
  async function openRecords(): Promise<void> {
    view.value = 'records'
    try {
      recordsPage.value = await ipc.listRecords(50)
      stats.value = recordsPage.value.stats
    } catch (err) {
      lastError.value = String(err)
    }
  }

  async function openBank(): Promise<void> {
    view.value = 'bank'
    try {
      questions.value = await ipc.listQuestions()
    } catch (err) {
      lastError.value = String(err)
    }
  }

  function openSettings(): void {
    view.value = 'settings'
  }

  function closeSubPage(): void {
    view.value = 'setup'
  }

  /** 设置页写配置：成功后把 Rust 返回的整份配置换掉（含 extra 保留字段） */
  async function saveConfig(patch: Partial<ipc.Config>): Promise<boolean> {
    try {
      const next = await ipc.updateConfig(patch)
      config.value = next
      applyTheme(next.theme)
      return true
    } catch (err) {
      notify('error', errorText(err))
      return false
    }
  }

  /** 暖白/暗色：theme.css 用 `[data-theme='light']` 覆盖变量 */
  function applyTheme(theme: string): void {
    if (theme === 'light') {
      document.documentElement.dataset.theme = 'light'
    } else {
      delete document.documentElement.dataset.theme
    }
  }

  /** 首次引导（F-11.4）：三步都点过才写 onboarded */
  async function finishOnboarding(): Promise<void> {
    await saveConfig({ onboarded: true })
    view.value = 'setup'
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
    speaker,
    bank,
    bankName,
    bankTotal,
    bankDrawable,
    debugBuild,
    result,
    lastSession,
    lastDurationMin,
    toast,
    notice,
    coolRemainMs,
    escHoldMs,
    progress,
    view,
    stats,
    recordsPage,
    questions,
    notify,
    refresh,
    refreshStats,
    openRecords,
    openBank,
    openSettings,
    closeSubPage,
    saveConfig,
    finishOnboarding,
    needsOnboarding,
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

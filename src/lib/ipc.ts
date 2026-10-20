// 唯一允许直接调用 Tauri 的文件（TDD §3.2）。其余代码只依赖这里的类型化封装。
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

export interface Config {
  version: number
  dim: boolean
  physicalBrightness: boolean
  chimeVolume: number
  cooldownBase: number
  questionBase: number
  theme: string
  lastDurationMin: number
  recentQuotes: string[]
}

export interface LastSession {
  actualMs: number
  startedAtMs: number
}

export interface BootstrapView {
  config: Config
  /** 上次异常中断的会话；正常启动为 null */
  lastSession: LastSession | null
}

export interface StartResult {
  ok: boolean
  sessionId: string
}

export type SessionStateName = 'Idle' | 'Arming' | 'Locked' | 'Settled'

export interface StateView {
  state: SessionStateName
  targetMs: number
  elapsedMs: number
  remainingMs: number
  goal: string | null
  bank: string
  attempts: number
  clockSkewMin: number
}

export interface TimerTick {
  elapsedMs: number
  remainingMs: number
  clockSkewMin: number
}

export interface StateChanged {
  state: string
  result: string | null
  goal: string | null
  elapsedMs: number
  targetMs: number
}

/** 起势倒计时：时间由 Rust 的 tick 线程给出 */
export interface ArmingTick {
  remainingMs: number
}

/** 钟鸣：kind = tick | bell */
export interface Chime {
  kind: string
  index: number
}

export const bootstrap = (): Promise<BootstrapView> => invoke('bootstrap')

export const startSession = (
  durationMin: number,
  goal: string | null,
  bankId: string | null,
): Promise<StartResult> => invoke('start_session', { durationMin, goal, bankId })

export const getState = (): Promise<StateView> => invoke('get_state')

export const onTick = (handler: (tick: TimerTick) => void): Promise<UnlistenFn> =>
  listen<TimerTick>('timer://tick', (event) => handler(event.payload))

export const onStateChanged = (handler: (state: StateChanged) => void): Promise<UnlistenFn> =>
  listen<StateChanged>('state://changed', (event) => handler(event.payload))

export const onArmingTick = (handler: (tick: ArmingTick) => void): Promise<UnlistenFn> =>
  listen<ArmingTick>('arming://tick', (event) => handler(event.payload))

export const onChime = (handler: (chime: Chime) => void): Promise<UnlistenFn> =>
  listen<Chime>('chime://play', (event) => handler(event.payload))

/** 起势 3 秒内取消；非 Arming 状态会返回 E_STATE_ILLEGAL */
export const cancelArming = (): Promise<void> => invoke('cancel_arming')

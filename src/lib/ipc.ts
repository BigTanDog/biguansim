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
  onboarded: boolean
}

export interface LastSession {
  actualMs: number
  startedAtMs: number
}

export interface BankInfo {
  id: string
  name: string
  /** 题库总题数（记忆衰减接入后，可抽题数会小于它） */
  total: number
  /** 当前可抽题数：总数减去冷却中的 */
  drawable: number
}

/** 统计口径全在 Rust（`core::stats`），前端只负责显示 */
export interface StatsView {
  totalMs: number
  sessions: number
  faults: number
  validDays: number
  streakDays: number
  todayValid: boolean
}

export interface RecordView {
  startedAtMs: number
  actualMs: number
  plannedMs: number
  result: string
  attempts: number
}

export interface RecordPage {
  records: RecordView[]
  /** 损坏行数：>0 时界面必须提示（PRD §12） */
  skipped: number
  stats: StatsView
}

export interface QuestionSummary {
  id: string
  stem: string
  tags: string[]
  rightCount: number
  wrongCount: number
  cooling: boolean
}

export interface BootstrapView {
  config: Config
  bank: BankInfo
  stats: StatsView
  /** 上次异常中断的会话；正常启动为 null */
  lastSession: LastSession | null
  /** Rust 侧 CARGO_PKG_VERSION，设置页「关于」显示 */
  version: string
  /** debug 构建才为 true：前端据此显示 DEV 退出按钮与测试版提示 */
  debugBuild: boolean
}

export interface StartResult {
  ok: boolean
  sessionId: string
}

/** 八态（PRD §5.1）。Recovered 不是状态，是启动弹窗 */
export type SessionStateName =
  'Idle' | 'Arming' | 'Locked' | 'Cool' | 'Challenge' | 'Copying' | 'Confirming' | 'Settled'

/**
 * 活跃态投影。`kind` 由 Rust 决定，前端只按它分支渲染。
 * 注意：Challenge 里**没有答案**，只有打乱后的选项。
 */
export type PhasePayload =
  | { kind: 'cool'; attempt: number; cooldownMs: number; remainMs: number }
  | {
      kind: 'challenge'
      id: string
      index: number
      total: number
      stem: string
      options: string[]
      attempt: number
    }
  | { kind: 'copy'; text: string; chars: number }
  | { kind: 'confirm'; sentence: string; elapsedMs: number; faults: number }

export interface StateView {
  state: SessionStateName
  targetMs: number
  elapsedMs: number
  remainingMs: number
  goal: string | null
  bank: string
  attempts: number
  clockSkewMin: number
  payload: PhasePayload | null
  speaker: string | null
  bankTotal: number
  bankDrawable: number
}

export interface TimerTick {
  elapsedMs: number
  remainingMs: number
  clockSkewMin: number
}

export interface StateChanged {
  state: SessionStateName
  result: string | null
  goal: string | null
  elapsedMs: number
  targetMs: number
  payload: PhasePayload | null
  speaker: string | null
}

/** 起势倒计时：时间由 Rust 的 tick 线程给出 */
export interface ArmingTick {
  remainingMs: number
}

/** 冷却剩余：界面只显示分钟粒度（PRD F-4.3） */
export interface CoolTick {
  remainMs: number
}

/** Esc 已按住毫秒数：进度环只渲染，不参与判定 */
export interface EscHold {
  heldMs: number
}

/** 钟鸣：kind = tick | bell */
export interface Chime {
  kind: string
  index: number
}

export interface Toast {
  level: string
  text: string
}

export interface UnlockResult {
  accepted: boolean
  attempt: number
  cooldownMs: number
}

export interface ChoiceResult {
  accepted: boolean
  correct: boolean
}

export interface CopyResult {
  ok: boolean
  mismatchIndex: number | null
}

export const bootstrap = (): Promise<BootstrapView> => invoke('bootstrap')

export const startSession = (
  durationMin: number,
  goal: string | null,
  bankId: string | null,
): Promise<StartResult> => invoke('start_session', { durationMin, goal, bankId })

export const getState = (): Promise<StateView> => invoke('get_state')

/** 记录页（按开始时间倒序） */
export const listRecords = (limit: number): Promise<RecordPage> => invoke('list_records', { limit })

/** 题库页：只有题面与统计，没有答案 */
export const listQuestions = (): Promise<QuestionSummary[]> => invoke('list_questions')

/** 设置页：浅合并 patch，非法值返回 E_INVALID_CONFIG 且不落盘 */
export const updateConfig = (patch: Partial<Config>): Promise<Config> =>
  invoke('update_config', { patch })

export const openDataDir = (): Promise<void> => invoke('open_data_dir')

/** 起势 3 秒内取消；非 Arming 状态会返回 E_STATE_ILLEGAL */
export const cancelArming = (): Promise<void> => invoke('cancel_arming')

/**
 * 点「解开阵法」：Locked → Cool。
 * 剩余不足一次冷却时抛 `E_REMAIN_TOO_SHORT`（F-4.5），状态保持不变。
 */
export const requestUnlock = (): Promise<UnlockResult> => invoke('request_unlock')

/** `choice` 是展示索引（0–3）；判定与答案都在 Rust */
export const submitChoice = (questionId: string, choice: number): Promise<ChoiceResult> =>
  invoke('submit_choice', { questionId, choice })

export const submitCopy = (text: string): Promise<CopyResult> => invoke('submit_copy', { text })

export const confirmEmergency = (text: string): Promise<void> =>
  invoke('confirm_emergency', { text })

export const cancelEmergency = (): Promise<void> => invoke('cancel_emergency')

/**
 * 界面侧 Esc 长按满 8 秒后调用（D-45）。真机上 `WH_KEYBOARD_LL` 收不到 Esc，
 * 所以计时改由 WebView 负责；`Locked` 之外调用会返回 `E_STATE_ILLEGAL`。
 */
export const requestEmergency = (): Promise<void> => invoke('request_emergency')

export const onTick = (handler: (tick: TimerTick) => void): Promise<UnlistenFn> =>
  listen<TimerTick>('timer://tick', (event) => handler(event.payload))

export const onStateChanged = (handler: (state: StateChanged) => void): Promise<UnlistenFn> =>
  listen<StateChanged>('state://changed', (event) => handler(event.payload))

export const onToast = (handler: (toast: Toast) => void): Promise<UnlistenFn> =>
  listen<Toast>('notice://toast', (event) => handler(event.payload))

export const onArmingTick = (handler: (tick: ArmingTick) => void): Promise<UnlistenFn> =>
  listen<ArmingTick>('arming://tick', (event) => handler(event.payload))

export const onCoolTick = (handler: (tick: CoolTick) => void): Promise<UnlistenFn> =>
  listen<CoolTick>('cool://tick', (event) => handler(event.payload))

export const onEscHold = (handler: (hold: EscHold) => void): Promise<UnlistenFn> =>
  listen<EscHold>('esc://hold', (event) => handler(event.payload))

export const onChime = (handler: (chime: Chime) => void): Promise<UnlistenFn> =>
  listen<Chime>('chime://play', (event) => handler(event.payload))

/**
 * 开发期逃生口：仅 debug 构建存在（release 里命令未注册，调用会报错）。
 * 不写记录、不计入统计。
 */
export const devAbort = (): Promise<void> => invoke('dev_abort')

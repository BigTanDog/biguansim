// 纯函数：倒计时/已坚持/校验时钟的显示格式（PRD F-3.17、F-3.18）。
// 只做展示，不参与任何判定——判定在 Rust 侧。

/** 偏差超过这个分钟数才提示"不一致" */
export const SKEW_THRESHOLD_MIN = 3

/**
 * 倒计时：≥1 小时显示 `HH:MM`，<1 小时显示 `MM:SS`。
 * 不足 1 秒按 1 秒显示，避免"到点前先显示 00:00"的错觉。
 */
export function formatCountdown(ms: number): string {
  const totalSeconds = Math.max(0, Math.ceil(ms / 1000))
  const hours = Math.floor(totalSeconds / 3600)
  const minutes = Math.floor((totalSeconds % 3600) / 60)
  const seconds = totalSeconds % 60

  if (hours > 0) {
    return `${pad(hours)}:${pad(minutes)}`
  }
  return `${pad(minutes)}:${pad(seconds)}`
}

/** 已坚持时长：始终显示到分钟，≥1 小时用 `H:MM` */
export function formatElapsed(ms: number): string {
  const safe = Math.max(0, ms)
  const totalMinutes = Math.floor(safe / 60000)

  // 不足 1 分钟显示秒：否则"提前出关 40 秒"会显示成 `00 分`，看着像坏掉了
  if (totalMinutes === 0) {
    return `${Math.max(1, Math.round(safe / 1000))} 秒`
  }

  const hours = Math.floor(totalMinutes / 60)
  const minutes = totalMinutes % 60
  return hours > 0 ? `${hours}:${pad(minutes)}` : `${pad(minutes)} 分`
}

/** 右下角校验时钟：`系统 21:03 · 一致` / `不一致 (+12 分)` */
export function formatClockLabel(now: Date, skewMin: number): string {
  if (Math.abs(skewMin) <= SKEW_THRESHOLD_MIN) {
    return `系统 ${pad(now.getHours())}:${pad(now.getMinutes())} · 一致`
  }
  const sign = skewMin > 0 ? '+' : '-'
  return `不一致 (${sign}${Math.abs(skewMin)} 分)`
}

function pad(value: number): string {
  return String(value).padStart(2, '0')
}

import { describe, expect, it } from 'vitest'

import { formatClockLabel, formatCountdown, formatElapsed } from './format'

describe('formatCountdown', () => {
  it('不足一小时用 MM:SS', () => {
    expect(formatCountdown(0)).toBe('00:00')
    expect(formatCountdown(1_000)).toBe('00:01')
    expect(formatCountdown(59_999)).toBe('01:00')
    expect(formatCountdown(59 * 60_000)).toBe('59:00')
  })

  it('满一小时用 HH:MM', () => {
    expect(formatCountdown(3_600_000)).toBe('01:00')
    expect(formatCountdown(7_200_000)).toBe('02:00')
    expect(formatCountdown(7_260_000)).toBe('02:01')
  })

  it('负数按 0 处理', () => {
    expect(formatCountdown(-5_000)).toBe('00:00')
  })
})

describe('formatElapsed', () => {
  it('不足一小时显示分钟', () => {
    expect(formatElapsed(0)).toBe('00 分')
    expect(formatElapsed(32 * 60_000)).toBe('32 分')
  })

  it('满一小时显示 H:MM', () => {
    expect(formatElapsed(2 * 3_600_000 + 5 * 60_000)).toBe('2:05')
  })
})

describe('formatClockLabel', () => {
  const now = new Date(2026, 9, 20, 21, 3, 0)

  it('偏差在阈值内显示一致', () => {
    expect(formatClockLabel(now, 0)).toBe('系统 21:03 · 一致')
    expect(formatClockLabel(now, 3)).toBe('系统 21:03 · 一致')
    expect(formatClockLabel(now, -3)).toBe('系统 21:03 · 一致')
  })

  it('超过阈值显示不一致并带方向', () => {
    expect(formatClockLabel(now, 12)).toBe('不一致 (+12 分)')
    expect(formatClockLabel(now, -10)).toBe('不一致 (-10 分)')
  })
})

// 钟鸣：用 Web Audio 合成，不引音频文件、不引依赖（M2 S3）。
// 音量由 `config.chimeVolume` 决定，上限 0.6（PRD F-10.3），最终响度仍由系统混音决定。

import type { Chime } from './ipc'

/** 钟鸣的默认音量上限 */
export const MAX_VOLUME = 0.6

let context: AudioContext | null = null

/**
 * 在用户手势里调用（点「入关」时），否则浏览器的自动播放策略会把第一声吞掉。
 */
export function ensureAudio(): void {
  if (!context) {
    context = new AudioContext()
  }
  if (context.state === 'suspended') {
    void context.resume()
  }
}

export function playChime(chime: Chime, volume: number = MAX_VOLUME): void {
  // 长时间锁屏后 WebView 可能把音频上下文挂起，播之前再恢复一次
  if (context && context.state === 'suspended') {
    void context.resume()
  }

  const level = Math.min(MAX_VOLUME, Math.max(0, volume))

  if (chime.kind === 'bell') {
    playBell(level)
  } else {
    // 单声滴答：一个振荡器，不会削波
    tone(660, 0.09, level)
  }
}

/**
 * 到点钟鸣：三声，基频 + 五度泛音。
 * 每声的两个振荡器分别 0.55 / 0.25 倍音量，三声重叠时峰值仍 < 1.0，不会硬削波。
 */
function playBell(level: number): void {
  for (let index = 0; index < 3; index += 1) {
    window.setTimeout(() => {
      tone(440, 1.1, level * 0.55)
      tone(660, 0.9, level * 0.25)
    }, index * 650)
  }
}

function tone(frequency: number, durationSec: number, peak: number): void {
  const audio = context
  if (!audio || peak <= 0) {
    return
  }

  const oscillator = audio.createOscillator()
  const gain = audio.createGain()
  const now = audio.currentTime

  oscillator.type = 'sine'
  oscillator.frequency.value = frequency

  gain.gain.setValueAtTime(0, now)
  gain.gain.linearRampToValueAtTime(peak, now + 0.01)
  gain.gain.exponentialRampToValueAtTime(0.001, now + durationSec)

  oscillator.connect(gain)
  gain.connect(audio.destination)
  oscillator.start(now)
  oscillator.stop(now + durationSec + 0.02)
}

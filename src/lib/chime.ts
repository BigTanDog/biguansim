// 钟鸣：用 Web Audio 合成，不引音频文件、不引依赖（M2 S3）。
// 音量固定 0.6（PRD F-10.3 的上限），最终响度仍由系统混音决定。

import type { Chime } from './ipc'

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

/** 到点前的轻滴答：短促、略低 */
export function playTick(): void {
  tone(660, 0.09, 0.6)
}

/** 到点钟鸣：三声，基频 + 五度泛音，带回声感 */
export function playBell(): void {
  for (let index = 0; index < 3; index += 1) {
    window.setTimeout(() => {
      tone(440, 1.1, 0.6)
      tone(660, 0.9, 0.28)
    }, index * 650)
  }
}

export function playChime(chime: Chime): void {
  if (chime.kind === 'bell') {
    playBell()
  } else {
    playTick()
  }
}

function tone(frequency: number, durationSec: number, peak: number): void {
  const audio = context
  if (!audio) {
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

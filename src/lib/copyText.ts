// 抄写的**展示层**比对（TDD §8.3）。真正决定通过与否的是 Rust 的 submit_copy，
// 这里只负责"标红第几个字"，两边口径一致：按字符比对、不做全角半角归一化。

/** 首个不一致的字符下标；完全一致返回 null。少字报在缺失处，多字报在原文长度处。 */
export function firstMismatch(target: string, typed: string): number | null {
  const expected = Array.from(target)
  const actual = Array.from(typed)

  for (let index = 0; index < expected.length; index += 1) {
    if (actual[index] !== expected[index]) {
      return index
    }
  }

  if (actual.length > expected.length) {
    return expected.length
  }

  return null
}

/** 已输入字数（按字符计，一个汉字算一个） */
export function typedCount(typed: string): number {
  return Array.from(typed).length
}

/** 抄写进度：0–1，用于界面上的细进度条 */
export function copyProgress(target: string, typed: string): number {
  const total = Array.from(target).length
  if (total === 0) {
    return 0
  }
  return Math.min(1, typedCount(typed) / total)
}

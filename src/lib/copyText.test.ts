import { describe, expect, it } from 'vitest'

import { copyProgress, firstMismatch, typedCount } from './copyText'

const TARGET = '道友且坐，莫问几时。'

describe('firstMismatch', () => {
  it('完全一致返回 null', () => {
    expect(firstMismatch(TARGET, TARGET)).toBeNull()
  })

  it('错一个字报出下标', () => {
    expect(firstMismatch(TARGET, '道友且座，莫问几时。')).toBe(3)
  })

  it('少字报在缺失处', () => {
    expect(firstMismatch(TARGET, '道友且坐莫问几时。')).toBe(4)
  })

  it('多字报在原文长度处', () => {
    expect(firstMismatch(TARGET, `${TARGET}。`)).toBe(Array.from(TARGET).length)
  })

  it('半角标点算错（不归一化）', () => {
    expect(firstMismatch(TARGET, '道友且坐,莫问几时。')).toBe(4)
  })

  it('空输入报第 0 个字', () => {
    expect(firstMismatch(TARGET, '')).toBe(0)
  })

  it('按字符而不是字节计数', () => {
    expect(firstMismatch('一二三', '一二四')).toBe(2)
  })
})

describe('typedCount / copyProgress', () => {
  it('汉字按字符计数', () => {
    expect(typedCount('道友且坐')).toBe(4)
  })

  it('进度封顶 1', () => {
    expect(copyProgress(TARGET, `${TARGET}多出来的字`)).toBe(1)
  })

  it('空原文进度为 0', () => {
    expect(copyProgress('', '任意')).toBe(0)
  })
})

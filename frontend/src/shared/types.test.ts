import { describe, expect, it } from 'vitest'
import {
  compareSourcesForSearch,
  sourceMatchLabel,
  sourceMatchStrength,
  sourceMatchTier,
  speedTagFromRespondMs,
  type BookSource
} from './types'

function source(patch: Partial<BookSource> = {}): BookSource {
  return {
    bookSourceUrl: 'https://example.com',
    bookSourceName: '示例源',
    ...patch
  }
}

describe('types 比较器', () => {
  it('speedTagFromRespondMs 按延迟分档，无效输入返回 null', () => {
    expect(speedTagFromRespondMs(100)).toBe('turbo')
    expect(speedTagFromRespondMs(1000)).toBe('excellent')
    expect(speedTagFromRespondMs(3000)).toBe('slow')
    expect(speedTagFromRespondMs(undefined)).toBeNull()
    expect(speedTagFromRespondMs(-1)).toBeNull()
  })

  it('sourceMatchStrength 书架使用加成有上限 75', () => {
    const shelf = new Map([['https://example.com', 10]])
    expect(sourceMatchStrength(source(), shelf)).toBe(75)
    expect(sourceMatchStrength(source(), new Map())).toBe(0)
  })

  it('compareSourcesForSearch 匹配强度优先于速度', () => {
    const fast = source({ bookSourceUrl: 'https://fast.example', flyRespondMs: 100 })
    const used = source({
      bookSourceUrl: 'https://used.example',
      flyRespondMs: 3000,
      flyMatchScore: 90
    })
    const shelf = new Map([['https://used.example', 1]])
    // used 源匹配强度更高，应排在前面（返回负值）
    expect(compareSourcesForSearch(used, fast, shelf)).toBeLessThan(0)
  })

  it('compareSourcesForSearch 同分时速度快的在前', () => {
    const a = source({ bookSourceUrl: 'https://a.example', flyRespondMs: 100 })
    const b = source({ bookSourceUrl: 'https://b.example', flyRespondMs: 3000 })
    expect(compareSourcesForSearch(a, b)).toBeLessThan(0)
  })

  it('sourceMatchLabel / sourceMatchTier 档位一致', () => {
    expect(sourceMatchLabel(80)).toBe('高匹配')
    expect(sourceMatchTier(80)).toBe('high')
    expect(sourceMatchLabel(50)).toBe('中匹配')
    expect(sourceMatchTier(50)).toBe('mid')
    expect(sourceMatchLabel(10)).toBe('低匹配')
    expect(sourceMatchTier(0)).toBeNull()
  })
})

import { describe, expect, it } from 'vitest'
import {
  dedupeSearchBooks,
  filterChangeSourceCandidates,
  isSearchHitRelevant,
  normalizeSearchText,
  titleSimilarity
} from './searchRelevance'

describe('searchRelevance', () => {
  it('normalizeSearchText 去空白标点与版本文案', () => {
    expect(normalizeSearchText('《剑来》精校版')).toBe('剑来')
  })

  it('titleSimilarity 完全相同为 1，包含关系按长度比计分', () => {
    expect(titleSimilarity('剑来', '剑来')).toBe(1)
    expect(titleSimilarity('剑来', '剑来新编')).toBeCloseTo(2 / 4)
  })

  it('isSearchHitRelevant 短关键词须完整出现在书名或作者里', () => {
    expect(isSearchHitRelevant('剑来', { name: '剑来', author: '烽火戏诸侯' })).toBe(true)
    expect(isSearchHitRelevant('剑来', { name: '狮舞者', author: '佚名' })).toBe(false)
  })

  it('dedupeSearchBooks 按 origin + bookUrl 去重，保留首次出现', () => {
    const books = [
      { origin: 'a', bookUrl: '/1', name: '剑来' },
      { origin: 'a', bookUrl: '/1', name: '剑来' },
      { origin: 'b', bookUrl: '/1', name: '剑来' }
    ]
    expect(dedupeSearchBooks(books)).toHaveLength(2)
  })

  it('filterChangeSourceCandidates 短书名要求互相包含', () => {
    const cands = filterChangeSourceCandidates(
      { name: '剑来', author: '烽火戏诸侯' },
      [
        { name: '剑来', author: '烽火戏诸侯' },
        { name: '狮舞者', author: '佚名' }
      ]
    )
    expect(cands).toHaveLength(1)
    expect(cands[0].name).toBe('剑来')
  })
})

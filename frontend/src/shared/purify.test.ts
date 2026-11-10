import { describe, expect, it } from 'vitest'
import { applyPurify } from './purify'

describe('purify', () => {
  it('applyPurify 剔除正文中的广告链接', () => {
    expect(applyPurify('正文 https://ad.example.com/x 更多', [])).toBe('正文  更多')
  })

  it('applyPurify 按用户规则做字面删除', () => {
    expect(applyPurify('abc 广告 xyz', ['广告'])).toBe('abc  xyz')
  })

  it('applyPurify 空规则原样返回（除内置广告外）', () => {
    expect(applyPurify('干净正文', [])).toBe('干净正文')
  })
})

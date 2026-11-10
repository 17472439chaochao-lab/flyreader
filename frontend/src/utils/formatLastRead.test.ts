import { describe, expect, it } from 'vitest'
import { formatLastReadLabel } from './formatLastRead'

describe('formatLastRead', () => {
  const now = new Date('2026-11-10T12:00:00+08:00').getTime()

  it('未读过返回暂未阅读', () => {
    expect(formatLastReadLabel(0, now)).toBe('暂未阅读')
    expect(formatLastReadLabel(undefined, now)).toBe('暂未阅读')
  })

  it('1 分钟内返回刚刚', () => {
    expect(formatLastReadLabel(now - 30 * 1000, now)).toBe('上次阅读：刚刚')
  })

  it('3 分钟前', () => {
    expect(formatLastReadLabel(now - 3 * 60 * 1000, now)).toBe('上次阅读：3分钟前')
  })

  it('2 小时前', () => {
    expect(formatLastReadLabel(now - 2 * 60 * 60 * 1000, now)).toBe('上次阅读：2小时前')
  })

  it('5 天前', () => {
    expect(formatLastReadLabel(now - 5 * 24 * 60 * 60 * 1000, now)).toBe('上次阅读：5天前')
  })
})

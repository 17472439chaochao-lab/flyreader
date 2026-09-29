/**
 * 书架「上次阅读」相对时间文案。
 * @param lastReadAt 上次阅读时间戳（毫秒）；缺省或 ≤0 视为未读
 * @param now 当前时间（便于测试）；默认 Date.now()
 * @returns 如「暂未阅读」「上次阅读：3分钟前」
 */
export function formatLastReadLabel(lastReadAt?: number, now = Date.now()): string {
  const t = lastReadAt || 0
  if (t <= 0) return '暂未阅读'

  const diffMs = Math.max(0, now - t)
  const minuteMs = 60 * 1000
  const hourMs = 60 * minuteMs
  const dayMs = 24 * hourMs
  const yearMs = 365 * dayMs

  if (diffMs < minuteMs) return '上次阅读：刚刚'

  if (diffMs < hourMs) {
    const mins = Math.max(1, Math.floor(diffMs / minuteMs))
    return `上次阅读：${mins}分钟前`
  }

  if (diffMs < dayMs) {
    const hours = Math.max(1, Math.floor(diffMs / hourMs))
    return `上次阅读：${hours}小时前`
  }

  if (diffMs < yearMs) {
    const days = Math.max(1, Math.floor(diffMs / dayMs))
    return `上次阅读：${days}天前`
  }

  const years = Math.max(1, Math.floor(diffMs / yearMs))
  return `上次阅读：${years}年前`
}

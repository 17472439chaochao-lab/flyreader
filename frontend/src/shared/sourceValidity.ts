import type { BookSource } from './types'

/**
 * 检查可搜索文本书源的结构是否完整（名称、搜索 URL、规则字段等）。
 * @param source 书源对象
 * @returns 结构有效返回 true，否则 false
 */
export function isSourceStructuralOk(source: BookSource): boolean {
  const rs = source.ruleSearch
  return !!(
    source.bookSourceUrl &&
    source.bookSourceName &&
    source.searchUrl &&
    rs?.bookList &&
    rs?.name &&
    rs?.bookUrl
  )
}

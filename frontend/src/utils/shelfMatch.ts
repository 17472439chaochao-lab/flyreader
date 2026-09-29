import { authorName } from '@shared/author'
import { isLocalBook } from '@shared/bookLocal'
import { normalizeSearchText } from '@shared/searchRelevance'
import type { SearchBook, ShelfBook } from '@shared/types'

/** 小说身份：书名 + 作者（用于判重/换源） */
export type NovelIdentity = {
  name?: string | null
  author?: string | null
}

/**
 * 判断两本是否为同一小说（规范化书名且作者一致）。
 * @param a 身份 A
 * @param b 身份 B
 * @returns 同一小说则 true
 */
export function sameNovel(a: NovelIdentity, b: NovelIdentity): boolean {
  const nameA = normalizeSearchText(a.name)
  const nameB = normalizeSearchText(b.name)
  if (!nameA || !nameB || nameA !== nameB) return false
  return authorName(a.author) === authorName(b.author)
}

/**
 * 生成搜索书籍的稳定 id（origin + bookUrl）。
 * @param book 至少含 origin、bookUrl 的搜索书
 * @returns 形如 origin::bookUrl 的唯一键
 */
export function searchBookId(book: Pick<SearchBook, 'origin' | 'bookUrl'>): string {
  return `${book.origin}::${book.bookUrl}`
}

/**
 * 在书架中查找与目标像同一本小说的在线书（排除本地书）。
 * @param shelf 当前书架列表
 * @param book 目标小说身份
 * @returns 匹配的在线书架条目
 */
export function findShelfDuplicates(shelf: ShelfBook[], book: NovelIdentity): ShelfBook[] {
  return shelf.filter((b) => !isLocalBook(b) && sameNovel(b, book))
}

/**
 * 在搜索结果中找同一小说的其他书源命中（排除自身 origin+url）。
 * @param results 搜索结果列表
 * @param book 当前选中的搜索书
 * @returns 同书异源的其他命中
 */
export function findSameNovelSearchAlts(
  results: SearchBook[],
  book: SearchBook
): SearchBook[] {
  const selfId = searchBookId(book)
  return results.filter((b) => sameNovel(b, book) && searchBookId(b) !== selfId)
}

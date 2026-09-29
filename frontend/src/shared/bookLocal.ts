import type { ShelfBook } from './types'

/**
 * 判断书籍是否为本地书（txt/epub/md），本地书无在线缓存与换源。
 * @param book 至少包含 isLocal、origin 的书架书籍信息
 * @returns 是本地书则返回 true，否则 false
 */
export function isLocalBook(book: Pick<ShelfBook, 'isLocal' | 'origin'>): boolean {
  return book.isLocal === true || book.origin === 'local'
}

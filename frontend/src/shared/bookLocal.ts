import type { ShelfBook } from './types'

/**
 * 判断书籍是否为本地书（txt/epub/md），本地书无在线缓存与换源。
 * @param book 至少包含 isLocal、origin 的书架书籍信息
 * @returns 是本地书则返回 true，否则 false
 */
export function isLocalBook(book: Pick<ShelfBook, 'isLocal' | 'origin'>): boolean {
  return book.isLocal === true || book.origin === 'local'
}

/**
 * 判断是否为 Markdown 本地书（全文模式：目录切换只改锚点，不重复拉取正文）。
 * App.loadChapter 与 ReaderView 共用，避免两处各自写一份格式判断。
 */
export function isMdBook(book: Pick<ShelfBook, 'localFormat' | 'kind'>): boolean {
  const fmt = (book.localFormat || '').toLowerCase()
  const kind = (book.kind || '').toUpperCase()
  return fmt === 'md' || kind === 'MD' || kind === 'MARKDOWN'
}

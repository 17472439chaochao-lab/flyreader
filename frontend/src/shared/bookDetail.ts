import { pickBookAuthor, pickBookName } from './bookMeta'
import type { BookInfo, Chapter, SearchBook, ShelfBook } from './types'

/** 书架唯一 ID：origin + 书籍 URL */
export function bookId(origin: string, bookUrl: string): string {
  return `${origin}::${bookUrl}`
}

/**
 * 两段式拉取书籍详情与目录（books.info + books.toc）。
 * 详情失败时仍用 bookUrl 尝试拉目录（详情规则损坏时兜底），两者都失败才抛错。
 * @param origin 书源标识
 * @param bookUrl 书籍详情 URL
 * @param tocUrlHint 已知的目录 URL（优先于 info.tocUrl）
 */
export async function fetchBookDetail(
  origin: string,
  bookUrl: string,
  tocUrlHint?: string
): Promise<{ info: BookInfo | null; tocUrl: string; chapters: Chapter[] }> {
  let info: BookInfo | null = null
  try {
    info = await window.fly.books.info(origin, bookUrl)
  } catch {
    info = null
  }
  const tocUrl = tocUrlHint || info?.tocUrl || bookUrl
  try {
    const chapters = await window.fly.books.toc(origin, tocUrl)
    return { info, tocUrl, chapters }
  } catch (e) {
    // 目录 URL 失败且与 bookUrl 不同时，再用 bookUrl 兜底一次
    if (tocUrl !== bookUrl) {
      const chapters = await window.fly.books.toc(origin, bookUrl)
      return { info, tocUrl: bookUrl, chapters }
    }
    throw e
  }
}

/**
 * 由搜索命中 + 详情构造可入库的书架书籍。
 * @param book 搜索命中
 * @param detail fetchBookDetail 的返回值
 * @param base 可选：复用旧书架条目的进度/时间戳等字段（如换源时传旧书）
 */
export function buildShelfBook(
  book: SearchBook,
  detail: { info: BookInfo | null; tocUrl: string; chapters: Chapter[] },
  base?: ShelfBook
): ShelfBook {
  const now = Date.now()
  const info = detail.info
  return {
    ...(base ? { ...base } : {}),
    id: bookId(book.origin, book.bookUrl),
    name: pickBookName(info?.name, book.name, base?.name),
    author: pickBookAuthor(info?.author, book.author, base?.author),
    bookUrl: book.bookUrl,
    coverUrl: info?.coverUrl || book.coverUrl || base?.coverUrl,
    intro: info?.intro || book.intro || base?.intro,
    kind: info?.kind || book.kind || base?.kind,
    lastChapter: info?.lastChapter || book.lastChapter || base?.lastChapter,
    origin: book.origin,
    originName: book.originName,
    tocUrl: detail.tocUrl,
    chapters: detail.chapters,
    chapterIndex: base ? base.chapterIndex : 0,
    scrollTop: base ? base.scrollTop : 0,
    addedAt: base?.addedAt ?? now,
    updatedAt: now,
    lastReadAt: base?.lastReadAt ?? now
  }
}

import type { Chapter, ShelfBook } from './types'

/**
 * 可读章节（有 URL、非分卷标题）。
 * @param chapters 目录列表
 */
export function readableChapters(chapters: Chapter[] | undefined): Chapter[] {
  return (chapters || []).filter((c) => c.url && !c.isVolume)
}

/**
 * 相对旧目录，新目录新增了多少可读章节。
 * 旧目录为空时返回 0，避免把「首次拉取整本目录」误报成「新增 N 章」。
 * @param prev 书架上已有目录
 * @param next 书源新拉到的目录
 */
export function countAddedChapters(prev: Chapter[] | undefined, next: Chapter[]): number {
  const a = readableChapters(prev)
  const b = readableChapters(next)
  if (!a.length) return 0
  return Math.max(0, b.length - a.length)
}

/**
 * 目录更新类型：无变化 / 首次同步 / 有新增章节。
 * @param prev 书架上已有目录
 * @param next 书源新拉到的目录
 */
export function catalogUpdateKind(
  prev: Chapter[] | undefined,
  next: Chapter[]
): 'none' | 'first' | 'added' {
  const a = readableChapters(prev)
  const b = readableChapters(next)
  if (!a.length) return b.length ? 'first' : 'none'
  if (b.length > a.length) return 'added'
  const prevLast = a[a.length - 1]?.url || ''
  const nextLast = b[b.length - 1]?.url || ''
  if (prevLast && nextLast && prevLast !== nextLast) return 'added'
  return 'none'
}

/**
 * 目录刷新或换源后，尽量保持原阅读章节位置。
 * 优先按章节 URL 匹配，其次按标题包含关系，最后钳制到有效索引。
 * @param prev 换源/刷新前的书架书籍（含 chapters、chapterIndex）
 * @param chapters 新的章节列表
 * @returns 应对齐到的章节下标（从 0 起）
 */
export function matchChapterIndex(prev: ShelfBook, chapters: Chapter[]): number {
  const current = prev.chapters?.[prev.chapterIndex]
  if (!current || !chapters.length) return 0
  const byUrl = chapters.findIndex((c) => c.url && c.url === current.url)
  if (byUrl >= 0) return byUrl
  const byTitle = chapters.findIndex(
    (c) =>
      c.title &&
      (c.title === current.title ||
        c.title.includes(current.title) ||
        current.title.includes(c.title))
  )
  if (byTitle >= 0) return byTitle
  return Math.min(prev.chapterIndex, chapters.length - 1)
}

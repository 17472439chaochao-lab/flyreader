import { isLocalBook } from '@shared/bookLocal'
import type { ShelfBook } from '@shared/types'

/**
 * 生成书架条目上的缓存状态徽章文案与样式类名。
 * @param b 书架书籍
 * @param cacheBusyIds 当前正在缓存的书籍 id 列表
 * @param cacheProgressMap 按书籍 id 记录的缓存进度展示字符串
 * @returns text 为展示文案，cls 为 CSS 类名
 */
export function cacheLabel(
  b: ShelfBook,
  cacheBusyIds: string[],
  cacheProgressMap: Record<string, string>
): {
  text: string
  cls: string
} {
  if (isLocalBook(b)) {
    const fmt = (b.localFormat || b.kind || '本地').toString().toUpperCase()
    return { text: `本地 · ${fmt}`, cls: 'cache-badge local' }
  }
  const caching = cacheBusyIds.includes(b.id) || b.cache?.status === 'caching'
  if (caching) {
    const progress = cacheProgressMap[b.id] || ''
    return {
      text: progress ? `缓存中 ${progress}` : '缓存中…',
      cls: 'cache-badge caching'
    }
  }
  const status = b.cache?.status || 'none'
  const cached = b.cache?.cached ?? 0
  const total = b.cache?.total ?? 0
  if (status === 'full') return { text: '已缓存', cls: 'cache-badge full' }
  if (status === 'partial') return { text: `部分缓存 ${cached}/${total}`, cls: 'cache-badge partial' }
  return { text: '未缓存', cls: 'cache-badge none' }
}

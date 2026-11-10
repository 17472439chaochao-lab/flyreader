import { isLocalBook } from '@shared/bookLocal'
import type { ShelfBook } from '@shared/types'
import { SafeCover } from './SafeCover'

/**
 * 书架封面展示组件：优先显示网络封面图；
 * 本地 TXT/EPUB/MD 显示格式徽章；否则用书名首字占位。
 */
export function ShelfCover({ book }: { book: ShelfBook }) {
  if (book.coverUrl) {
    // 网络封面走 SafeCover：加载失败回落首字占位，不留破图
    return <SafeCover src={book.coverUrl} title={book.name} />
  }
  const local = isLocalBook(book)
  const fmt = (book.localFormat || '').toLowerCase()
  const kind = (book.kind || '').toUpperCase()
  if (local && (fmt === 'txt' || kind === 'TXT')) {
    return <div className="cover format-cover txt">TXT</div>
  }
  if (local && (fmt === 'epub' || kind === 'EPUB')) {
    return <div className="cover format-cover epub">EPUB</div>
  }
  if (local && (fmt === 'md' || kind === 'MD' || kind === 'MARKDOWN')) {
    return <div className="cover format-cover md">MD</div>
  }
  return <div className="cover placeholder">{book.name.slice(0, 1)}</div>
}

import { useEffect, useRef, useState } from 'react'
import { Loader } from '@mantine/core'
import { authorLabel } from '@shared/author'
import type { SearchBook, ShelfBook } from '@shared/types'
import { useBookSearch } from '../hooks/useBookSearch'
import { SafeCover } from './SafeCover'

/**
 * 换源对话框（独立组件，不复用 Mantine Modal）。
 *
 * 设计目标：面板尺寸在「搜索中 / 结果列表 / 正在换源」三个阶段保持完全一致，
 * 阶段切换只换内部内容，不改变面板高度——从根上消除竖向跳动。
 * 搜索由 useBookSearch（与 App 搜书共用）负责：代际守卫 + 流式 rAF 合批；
 * 自身只负责候选展示与应用，对外只暴露 book + 回调。
 */

export type ChangeSourceDialogProps = {
  book: ShelfBook
  /** 过滤候选（排除当前源等），由外部提供以保持单一实现 */
  filterCandidates: (book: ShelfBook, hits: SearchBook[]) => SearchBook[]
  /** 把候选应用到书架（替换原书），成功后调用 onClose */
  apply: (candidate: SearchBook) => Promise<void>
  onClose: () => void
}

/** 面板固定尺寸：三个阶段共用，杜绝高度跳变 */
const PANEL_W = 460
const PANEL_H = 440

export function ChangeSourceDialog({
  book,
  filterCandidates,
  apply,
  onClose
}: ChangeSourceDialogProps) {
  const [applying, setApplying] = useState<SearchBook | null>(null)
  const [error, setError] = useState<string | null>(null)
  const filterRef = useRef(filterCandidates)
  filterRef.current = filterCandidates
  const bookRef = useRef(book)
  bookRef.current = book
  /** 搜索由共享 Hook 负责（代际守卫 + rAF 队列合批，不再双份实现） */
  const { results: candidates, searching, progress, runSearch } = useBookSearch({
    filterHits: (_key, hits) => filterRef.current(bookRef.current, hits)
  })

  useEffect(() => {
    setError(null)
    void runSearch(book.name).catch((e: Error) => {
      setError(`搜索失败：${e.message}`)
    })
    // 只在对话框打开（book.id 变化）时触发一次搜索
  }, [book.id]) // eslint-disable-line react-hooks/exhaustive-deps

  /** 选中候选应用：面板保持，内部切到「正在换源…」状态 */
  async function pick(candidate: SearchBook) {
    if (applying) return
    setApplying(candidate)
    setError(null)
    try {
      await apply(candidate)
    } catch (e) {
      setError(`换源失败：${(e as Error).message}`)
    } finally {
      setApplying(null)
    }
  }

  /** 阶段判定：应用 > 搜索 > 结果/空/错误 */
  const phase: 'applying' | 'searching' | 'results' = applying
    ? 'applying'
    : searching
      ? 'searching'
      : 'results'

  const progressText =
    progress && progress.total > 0
      ? `正在搜索书源 ${progress.done}/${progress.total}${
          progress.current ? ` · ${progress.current}` : ''
        }`
      : '正在搜索书源…'

  return (
    <div className="csd-overlay" role="presentation" onClick={applying ? undefined : onClose}>
      <div
        className="csd-panel"
        role="dialog"
        aria-modal="true"
        aria-label={`换源 · ${book.name}`}
        style={{ width: PANEL_W, height: PANEL_H }}
        onClick={(e) => e.stopPropagation()}
      >
        <header className="csd-head">
          <div className="csd-title">
            <strong>换源 · {book.name}</strong>
            <span>当前源：{book.originName || '未知'}</span>
          </div>
          <button className="csd-close" aria-label="关闭" onClick={onClose} disabled={applying !== null}>
            ×
          </button>
        </header>

        <div className="csd-body">
          {phase === 'searching' ? (
            <>
              <div className="csd-status" role="status" aria-live="polite">
                <Loader size={14} />
                <span>{progressText}</span>
              </div>
              {candidates.length ? (
                <div className="csd-list">
                  {candidates.map((c) => (
                    <CsdRow key={`${c.origin}-${c.bookUrl}`} c={c} onClick={() => void pick(c)} />
                  ))}
                </div>
              ) : null}
            </>
          ) : null}

          {phase === 'results' ? (
            candidates.length ? (
              <div className="csd-list">
                {candidates.map((c) => (
                  <CsdRow key={`${c.origin}-${c.bookUrl}`} c={c} onClick={() => void pick(c)} />
                ))}
              </div>
            ) : error ? (
              <div className="csd-empty">{error}</div>
            ) : (
              <div className="csd-empty">未找到可换的其他书源</div>
            )
          ) : null}

          {phase === 'applying' ? (
            <div className="csd-applying">
              <Loader size={16} />
              <span>正在换源到「{applying!.originName || applying!.name}」…</span>
              <p>将抓取目录并匹配原阅读章节，可能需要几秒</p>
              {error ? <p className="csd-error">{error}</p> : null}
            </div>
          ) : null}
        </div>

        <footer className="csd-foot">
          <span>{candidates.length ? `共 ${candidates.length} 个候选` : ' '}</span>
          <span>点击候选即可换源</span>
        </footer>
      </div>
    </div>
  )
}

/** 单个候选行：封面 + 元信息 */
function CsdRow({ c, onClick }: { c: SearchBook; onClick: () => void }) {
  return (
    <button className="csd-row" onClick={onClick}>
      <SafeCover src={c.coverUrl} title={c.name} className="csd-cover" placeholderClassName="csd-cover placeholder" />
      <div className="csd-meta">
        <strong>{c.name}</strong>
        <span>{authorLabel(c.author)}</span>
        <span>书源：{c.originName || '未知'}</span>
        <span className="muted">最新：{c.lastChapter?.trim() || '暂无'}</span>
      </div>
    </button>
  )
}

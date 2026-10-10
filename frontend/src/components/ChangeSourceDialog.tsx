import { useEffect, useRef, useState } from 'react'
import { Loader } from '@mantine/core'
import { authorLabel } from '@shared/author'
import type { SearchBook, SearchProgress, ShelfBook } from '@shared/types'

/**
 * 换源对话框（独立组件，不复用 Mantine Modal）。
 *
 * 设计目标：面板尺寸在「搜索中 / 结果列表 / 正在换源」三个阶段保持完全一致，
 * 阶段切换只换内部内容，不改变面板高度——从根上消除竖向跳动。
 * 自身负责搜索、进度订阅、候选合并与应用，对外只暴露 book + 回调。
 */

export type ChangeSourceDialogProps = {
  book: ShelfBook
  /** 可用的书源数量（决定进度条总数） */
  sourceCount: number
  /** 过滤候选（排除当前源等），由外部提供以保持单一实现 */
  filterCandidates: (book: ShelfBook, hits: SearchBook[]) => SearchBook[]
  /** 把候选应用到书架（替换原书），成功后调用 onClose */
  apply: (candidate: SearchBook) => Promise<void>
  onClose: () => void
}

/** 面板固定尺寸：三个阶段共用，杜绝高度跳变 */
const PANEL_W = 460
const PANEL_H = 440
const LIST_MAX_H = 300

export function ChangeSourceDialog({
  book,
  sourceCount,
  filterCandidates,
  apply,
  onClose
}: ChangeSourceDialogProps) {
  const [candidates, setCandidates] = useState<SearchBook[]>([])
  const [progress, setProgress] = useState<SearchProgress | null>(null)
  const [searching, setSearching] = useState(true)
  const [applying, setApplying] = useState<SearchBook | null>(null)
  const [error, setError] = useState<string | null>(null)

  /** 对话框实例代号：忽略过期事件（搜索被新对话框或关闭取代） */
  const genRef = useRef(0)
  /** 候选去重池 */
  const seenRef = useRef<Set<string>>(new Set())

  useEffect(() => {
    const gen = ++genRef.current
    seenRef.current = new Set()
    setCandidates([])
    setProgress({ done: 0, total: sourceCount, found: 0, current: '', phase: 'start' })
    setSearching(true)

    let unlistProgress: (() => void) | null = null
    let unlistPartial: (() => void) | null = null

    const isStale = () => gen !== genRef.current

    /** 合并中间命中：去重 + 排除当前源，rAF 拆帧避免同帧批量 DOM 插入 */
    let pendingFrame: number | null = null
    const mergeHits = (hits: SearchBook[]) => {
      const filtered = filterCandidates(book, hits)
      if (!filtered.length) return
      if (pendingFrame !== null) return
      pendingFrame = requestAnimationFrame(() => {
        pendingFrame = null
        if (isStale()) return
        setCandidates((prev) => {
          const next = prev.slice()
          for (const h of filtered) {
            const id = `${h.origin}::${h.bookUrl}`
            if (seenRef.current.has(id)) continue
            seenRef.current.add(id)
            next.push(h)
          }
          return next.length === prev.length ? prev : next
        })
      })
    }

    unlistProgress = window.fly.books.onSearchProgress((p) => {
      if (isStale()) return
      setProgress(p)
    })
    unlistPartial = window.fly.books.onSearchPartial((hits) => {
      if (isStale()) return
      mergeHits(hits)
    })

    window.fly.books
      .search(book.name)
      .then(({ books }) => {
        if (isStale()) return
        const final = filterCandidates(book, books)
        setCandidates((prev) => {
          const seen = new Set(prev.map((c) => `${c.origin}::${c.bookUrl}`))
          const extra = final.filter((c) => !seen.has(`${c.origin}::${c.bookUrl}`))
          if (!extra.length) return prev
          const next = prev.slice()
          for (const c of extra) {
            const id = `${c.origin}::${c.bookUrl}`
            if (!seen.has(id)) {
              seen.add(id)
              next.push(c)
            }
          }
          return next
        })
        setProgress(null)
        setSearching(false)
      })
      .catch((e: Error) => {
        if (isStale()) return
        setError(`搜索失败：${e.message}`)
        setProgress(null)
        setSearching(false)
      })

    return () => {
      genRef.current += 1
      unlistProgress?.()
      unlistPartial?.()
      if (pendingFrame !== null) cancelAnimationFrame(pendingFrame)
    }
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
      {c.coverUrl ? (
        <img className="csd-cover" src={c.coverUrl} alt="" loading="lazy" />
      ) : (
        <div className="csd-cover placeholder">{c.name.slice(0, 1)}</div>
      )}
      <div className="csd-meta">
        <strong>{c.name}</strong>
        <span>{authorLabel(c.author)}</span>
        <span>书源：{c.originName || '未知'}</span>
        <span className="muted">最新：{c.lastChapter?.trim() || '暂无'}</span>
      </div>
    </button>
  )
}

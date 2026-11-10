import { useCallback, useEffect, useRef, useState } from 'react'
import { dedupeSearchBooks } from '@shared/searchRelevance'
import type { SearchBook, SearchProgress } from '@shared/types'
import { runWithProgress } from '../utils/progress'

export type BookSearchResult = {
  books: SearchBook[]
  history: string[]
  sources?: import('@shared/types').BookSource[]
}

export type UseBookSearchOptions = {
  /** 流式命中过滤（如相关度过滤 / 换源候选过滤），缺省不过滤 */
  filterHits?: (keyword: string, hits: SearchBook[]) => SearchBook[]
}

/**
 * 多书源搜索共享 Hook（App 搜书与换源对话框共用）：
 * 代际守卫丢弃过期搜索的事件与结果；流式命中经 rAF 合批后合并去重。
 * rAF 队列会累积同帧到达的多批命中，帧回调里一次性合并，不丢弃后到的批次。
 */
export function useBookSearch(options: UseBookSearchOptions = {}) {
  const [results, setResults] = useState<SearchBook[]>([])
  const [searching, setSearching] = useState(false)
  const [progress, setProgress] = useState<SearchProgress | null>(null)
  const genRef = useRef(0)
  const filterRef = useRef(options.filterHits)
  filterRef.current = options.filterHits
  /** 待合批的流式命中队列 + rAF 句柄（useRef 存储，卸载时取消） */
  const pendingRef = useRef<SearchBook[]>([])
  const rafRef = useRef(0)

  useEffect(
    () => () => {
      if (rafRef.current) cancelAnimationFrame(rafRef.current)
    },
    []
  )

  /** 把一批命中追加进队列，同一帧内只排一个 rAF，回调里一次性合并全部 */
  const queueHits = useCallback((hits: SearchBook[]) => {
    if (!hits.length) return
    pendingRef.current.push(...hits)
    if (rafRef.current) return
    rafRef.current = requestAnimationFrame(() => {
      rafRef.current = 0
      const batch = pendingRef.current
      pendingRef.current = []
      if (!batch.length) return
      setResults((prev) => dedupeSearchBooks(prev.concat(batch)))
    })
  }, [])

  const runSearch = useCallback(
    async (keyword: string): Promise<BookSearchResult | null> => {
      const key = keyword.trim()
      if (!key) return null
      const gen = ++genRef.current
      // 新一轮搜索开始：丢弃上一轮未合并的队列，避免旧命中混入新结果
      pendingRef.current = []
      if (rafRef.current) {
        cancelAnimationFrame(rafRef.current)
        rafRef.current = 0
      }
      setSearching(true)
      setResults([])
      setProgress({ done: 0, total: 0, found: 0, current: '', phase: 'start' })
      try {
        const res = await runWithProgress(
          (track) => {
            track(
              window.fly.books.onSearchProgress((p) => {
                if (gen !== genRef.current) return
                setProgress(p)
              })
            )
            track(
              window.fly.books.onSearchPartial((hits) => {
                if (gen !== genRef.current) return
                const filtered = filterRef.current ? filterRef.current(key, hits) : hits
                if (filtered.length) queueHits(filtered)
              })
            )
          },
          () => window.fly.books.search(key)
        )
        if (gen !== genRef.current) return null
        // 帧队列里可能还有未刷新的命中：先取消 rAF 并直接合并，避免最终结果缺失
        if (rafRef.current) {
          cancelAnimationFrame(rafRef.current)
          rafRef.current = 0
        }
        const queued = pendingRef.current
        pendingRef.current = []
        const cleaned = dedupeSearchBooks(
          filterRef.current ? filterRef.current(key, res.books) : res.books
        )
        setResults(dedupeSearchBooks(queued.concat(cleaned)))
        return res
      } finally {
        if (gen === genRef.current) {
          setSearching(false)
          setProgress(null)
        }
      }
    },
    [queueHits]
  )

  const reset = useCallback(() => {
    genRef.current += 1
    pendingRef.current = []
    if (rafRef.current) {
      cancelAnimationFrame(rafRef.current)
      rafRef.current = 0
    }
    setResults([])
    setSearching(false)
    setProgress(null)
  }, [])

  return { results, searching, progress, runSearch, reset, setResults }
}

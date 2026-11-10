import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Button, Notification, useMantineColorScheme } from '@mantine/core'
import { filterChangeSourceCandidates, filterRelevantSearchHits, dedupeSearchBooks } from '@shared/searchRelevance'
import { pickBookAuthor, pickBookName } from '@shared/bookMeta'
import { isLocalBook, isMdBook } from '@shared/bookLocal'
import { bookId, buildShelfBook, fetchBookDetail } from '@shared/bookDetail'
import { catalogUpdateKind, countAddedChapters, matchChapterIndex, readableChapters } from '@shared/matchChapter'
import {
  DEFAULT_PREFS,
  DEFAULT_SETTINGS,
  type AppPrefs,
  type BookSource,
  type Chapter,
  type ReaderSettings,
  type SearchBook,
  type ShelfBook,
  type ShelfSort,
  type ViewName
} from '@shared/types'
import { ConfirmDialog, type ConfirmOutcome, type ConfirmRequest } from './components/ConfirmDialog'
import { ChangeSourceDialog } from './components/ChangeSourceDialog'
import { SidebarMotto } from './components/SidebarMotto'
import { WindowChrome } from './components/WindowChrome'
import { useBookSearch } from './hooks/useBookSearch'
import { useEyeCareReminder } from './hooks/useEyeCareReminder'
import { AboutView } from './views/AboutView'
import { RankView } from './views/RankView'
import { ReaderView } from './views/ReaderView'
import { SearchView } from './views/SearchView'
import { SettingsView } from './views/SettingsView'
import { ShelfView } from './views/ShelfView'
import { SourcesView } from './views/SourcesView'
import {
  findSameNovelSearchAlts,
  findShelfDuplicates
} from './utils/shelfMatch'
import { formatProgressLabel, runWithProgress } from './utils/progress'
import type { LegacyDataStatus, LegacyImportResult } from './tauri/shim'

/**
 * 各视图对外暴露的回调集合（扁平命名，供 ref 镜像与稳定包装复用）。
 * 每次渲染都会把最新闭包写入 viewCbRef.current，稳定包装则持有不变的引用。
 */
interface ViewCallbacks {
  // ShelfView
  onOpen: (b: ShelfBook) => void
  onImportLocal: () => void
  onSortChange: (sort: ShelfSort) => void
  onUpdate: (b: ShelfBook) => void
  onUpdateAll: () => void
  onChangeSource: (b: ShelfBook) => void
  onCache: (b: ShelfBook) => void
  onCancelCache: (id: string) => void
  onExportTxt: (b: ShelfBook) => void
  onRemove: (id: string) => void
  onSearch: () => void
  // SearchView（onSearch 与 ShelfView 同名，这里用 onSearchRun 区分）
  onSearchRun: () => void
  onSearchKeyword: (k: string) => void
  onClearHistory: () => void
  onRemoveHistory: (k: string) => void
  onRead: (b: SearchBook) => void
  // RankView
  onRankOpenBook: (name: string) => void
  // SourcesView
  onTestKeywordChange: (v: string) => void
  onTestKeywordCommit: (v: string) => void
  onImportFile: () => void
  // SettingsView
  onPrefsChange: (patch: Partial<AppPrefs>) => void
  onRemovePurify: (rule: string) => void
  onLegacyStatus: () => Promise<LegacyDataStatus>
  onImportLegacy: () => Promise<LegacyImportResult>
  // ReaderView
  onBack: () => void
  onSelectChapter: (i: number) => void
  onPrev: () => void
  onNext: () => void
  onScrollSave: (scrollTop: number) => void
  /** Markdown 全文滚动时同步左侧目录高亮（不重置滚动） */
  onActiveChapter: (i: number) => void
  onSettingsChange: (s: ReaderSettings) => void
  onAddPurify: (text: string) => void
  /** 强制重新加载当前章节正文（跳过缓存） */
  onReloadChapter: () => void
}

/**
 * 应用根组件：管理导航、书架、搜索、书源、设置、阅读与换源等全局状态与 IPC 交互。
 */
export default function App() {
  const [view, setView] = useState<ViewName>('shelf')
  const [sources, setSources] = useState<BookSource[]>([])
  const [shelf, setShelf] = useState<ShelfBook[]>([])
  const [settings, setSettings] = useState<ReaderSettings>(DEFAULT_SETTINGS)
  const [prefs, setPrefs] = useState<AppPrefs>(DEFAULT_PREFS)
  /** Toast 队列：并发提示依次堆叠展示，各自到期后独立消失 */
  const [toasts, setToasts] = useState<{ id: number; text: string }[]>([])
  const toastIdRef = useRef(0)
  /** 搜索命中书籍校验期间的持久加载指示（toast 会过期消失，此遮罩保持到校验结束） */
  const [probeState, setProbeState] = useState<string | null>(null)
  const [keyword, setKeyword] = useState('')
  /** 搜索状态（结果/进行中/进度）由 useBookSearch 统一管理，与换源对话框共用同一实现 */
  const {
    results,
    searching,
    progress: searchProgress,
    runSearch: runBookSearch
  } = useBookSearch({ filterHits: (key, hits) => filterRelevantSearchHits(key, hits) })
  const [searchHistory, setSearchHistory] = useState<string[]>([])
  const [reading, setReading] = useState<ShelfBook | null>(null)
  const readingRef = useRef<ShelfBook | null>(null)
  readingRef.current = reading
  const viewRef = useRef<ViewName>('shelf')
  viewRef.current = view
  const chapterLoadGenRef = useRef(0)
  const [chapters, setChapters] = useState<Chapter[]>([])
  const [content, setContent] = useState('')
  const contentRef = useRef('')
  contentRef.current = content
  const [loadingContent, setLoadingContent] = useState(false)
  const [shelfBusyIds, setShelfBusyIds] = useState<string[]>([])
  const [shelfUpdatingAll, setShelfUpdatingAll] = useState(false)
  const [shelfUpdateProgress, setShelfUpdateProgress] = useState('')
  const [cacheBusyIds, setCacheBusyIds] = useState<string[]>([])
  const [cacheProgressMap, setCacheProgressMap] = useState<Record<string, string>>({})
  const pendingExportsRef = useRef<Map<string, string>>(new Map())
  const [readerCacheTick, setReaderCacheTick] = useState(0)
  /** 换源对话框：只保留要换哪本书 + 应用结果后的关闭标记 */
  const [changeSource, setChangeSource] = useState<ShelfBook | null>(null)

  const [confirmReq, setConfirmReq] = useState<ConfirmRequest | null>(null)

  // 最新视图回调镜像：每次渲染刷新，供下方稳定包装读取，避免闭包陈旧与全树重渲染
  const viewCbRef = useRef<ViewCallbacks | null>(null)

  /**
   * 视图回调守卫：统一收敛 viewCbRef.current! 的 33 处调用点。
   * ref 为空时静默跳过（只在挂载前理论上发生），调用方不再写非空断言。
   */
  const callView = useCallback(
    <T,>(fn: (cb: ViewCallbacks) => T): T => {
      const cb = viewCbRef.current
      if (!cb) return undefined as T
      return fn(cb)
    },
    []
  )

  /** 安全提取错误文案：Tauri invoke 抛出的值未必是标准 Error，message 可能缺失 */
  const errMsg = useCallback((e: unknown, fallback: string): string => {
    if (e instanceof Error && e.message) return e.message
    if (typeof e === 'string' && e.trim()) return e
    if (e && typeof e === 'object') {
      const m = (e as { message?: unknown }).message
      if (typeof m === 'string' && m.trim()) return m
    }
    return fallback
  }, [])

  /** 显示短暂 Toast 提示（约 2.8 秒后自动消失；多条并发时依次堆叠互不覆盖） */
  const showToast = useCallback((msg: string) => {
    const id = ++toastIdRef.current
    setToasts((prev) => [...prev, { id, text: msg }])
    setTimeout(() => {
      setToasts((prev) => prev.filter((t) => t.id !== id))
    }, 2800)
  }, [])

  /** 弹出确认对话框并返回用户选择结果 */
  const askConfirm = useCallback(
    (opts: {
      title: string
      message: string
      confirmText?: string
      cancelText?: string
      extraText?: string
      danger?: boolean
    }) =>
      new Promise<ConfirmOutcome>((resolve) => {
        setConfirmReq({ ...opts, resolve })
      }),
    []
  )

  /**
   * 关闭确认对话框并 resolve 结果
   * @param result 用户选择：确认 / 取消 / 额外操作
   */
  const closeConfirm = useCallback((result: ConfirmOutcome) => {
    setConfirmReq((prev: ConfirmRequest | null) => {
      prev?.resolve(result)
      return null
    })
  }, [])

  /** 护眼提醒：阅读累计时长，老板键隐藏时不计时也不弹窗 */
  useEyeCareReminder({
    enabled: Boolean(prefs.eyeCareEnabled),
    intervalMinutes: prefs.eyeCareIntervalMinutes ?? 120,
    readingActive: view === 'reader' && !!reading,
    askConfirm,
    dismissConfirm: () => closeConfirm('cancel')
  })

  /**
   * 切换主界面视图；非阅读页时持久化 lastView
   * @param next 目标视图名
   */
  const goView = useCallback((next: ViewName) => {
    if (viewRef.current === 'reader' && next !== 'reader' && readingRef.current) {
      void window.fly.books.cancelPreload(readingRef.current.id)
    }
    setView(next)
    if (next !== 'reader') {
      void window.fly.prefs.save({ lastView: next }).then((r) => setPrefs(r.prefs))
    }
  }, [])

  const { setColorScheme } = useMantineColorScheme()

  useEffect(() => {
    const theme = settings.theme || 'paper'
    document.documentElement.classList.remove('theme-paper', 'theme-green', 'theme-night')
    document.documentElement.classList.add(`theme-${theme}`)
    setColorScheme(theme === 'night' ? 'dark' : 'light')
  }, [settings.theme, setColorScheme])

  /** 从后端重取全量状态（首屏与旧版数据导入后共用） */
  async function reloadAllState() {
    const state = await window.fly.getState()
    setSources(state.sources)
    setShelf(state.shelf)
    setSettings(state.settings)
    setPrefs(state.prefs || DEFAULT_PREFS)
    setSearchHistory(await window.fly.books.history())
    return state
  }

  useEffect(() => {
    void reloadAllState().then((state) => {
      if (state.prefs?.lastView) setView(state.prefs.lastView)
    })
  }, [])

  /** 启动后静默检查更新：仅在有新版本时提示 */
  useEffect(() => {
    let cancelled = false
    const timer = setTimeout(() => {
      void (async () => {
        try {
          const result = await window.fly.checkUpdate()
          if (cancelled || result.status !== 'available') return
          const action = await askConfirm({
            title: '发现新版本',
            message: `当前 v${result.currentVersion}，最新 v${result.latestVersion}。是否打开发行版页面下载？`,
            confirmText: '前往下载',
            cancelText: '稍后'
          })
          if (action === 'confirm') {
            const open = await window.fly.openExternal(result.releaseUrl)
            if (!open.ok) showToast(open.message || '无法打开链接')
          }
        } catch {
          /* 启动检查失败时静默忽略 */
        }
      })()
    }, 2800)
    return () => {
      cancelled = true
      clearTimeout(timer)
    }
  }, [askConfirm, showToast])

  /**
   * 导出指定书籍为 TXT 文件并刷新书架
   * @param bookId 书籍 ID
   * @param bookName 可选书名（用于 Toast）
   */
  const runExportTxt = useCallback(async (bookId: string, bookName?: string) => {
    showToast(`正在导出${bookName ? `《${bookName}》` : ''}…`)
    try {
      const r = await window.fly.shelf.exportTxt(bookId)
      if (r.ok) {
        setShelf(await window.fly.shelf.list())
        showToast(r.message)
      } else if (r.message !== '已取消') {
        showToast(r.message)
      }
    } catch (e) {
      showToast(`导出失败：${(e as Error).message}`)
    }
  }, [showToast])

  useEffect(() => {
    const off = window.fly.shelf.onCacheProgress((p) => {
      if (p.phase === 'start' || p.phase === 'progress') {
        // 进度阶段只更新轻量状态（busy ids + 进度文案），不触碰整本 shelf
        // 的最终缓存状态交给 phase !== 'progress' 时的 shelf.list() 统一刷新
        setCacheBusyIds((prev) => (prev.includes(p.bookId) ? prev : [...prev, p.bookId]))
        setCacheProgressMap((prev) => ({
          ...prev,
          [p.bookId]: p.total > 0 ? `${p.cached ?? p.done}/${p.total}` : `${p.done}/${p.total}`
        }))
      } else {
        setCacheBusyIds((prev) => prev.filter((id) => id !== p.bookId))
        setCacheProgressMap((prev) => {
          const next = { ...prev }
          delete next[p.bookId]
          return next
        })
        const pendingName = pendingExportsRef.current.get(p.bookId)
        if (pendingName) {
          pendingExportsRef.current.delete(p.bookId)
          if (p.phase === 'cancelled') {
            showToast('已取消缓存，导出已中止')
          } else {
            void runExportTxt(p.bookId, pendingName)
          }
        }
        void window.fly.shelf.list().then(setShelf)
      }
    })
    return () => {
      off()
    }
  }, [showToast, runExportTxt])

  const enabledCount = useMemo(
    () => sources.filter((s) => s.enabled !== false).length,
    [sources]
  )

  /** 通过系统文件选择器导入书源 */
  async function onImportFile() {
    const res = await window.fly.sources.importFile()
    showToast(res.message)
    if (res.ok && res.sources) setSources(res.sources)
  }

  /**
   * 执行多书源并发搜索：流式合并/去重/代际守卫由 useBookSearch 负责，
   * 这里只做启用校验、历史与书源刷新、结果提示。
   * @param raw 可选关键词；缺省使用当前输入框内容
   */
  async function runSearch(raw?: string) {
    const key = (raw ?? keyword).trim()
    if (!key || searching) return
    if (!enabledCount) {
      showToast('请先导入并启用书源')
      goView('sources')
      return
    }
    setKeyword(key)
    try {
      const res = await runBookSearch(key)
      if (!res) return
      setSearchHistory(res.history)
      if (res.sources?.length) setSources(res.sources)
      else {
        const latest = await window.fly.sources.list()
        setSources(latest)
      }
      const cleaned = dedupeSearchBooks(filterRelevantSearchHits(key, res.books))
      if (!cleaned.length) showToast('未搜索到结果，可换个书源或关键词')
      else showToast(`搜索完成，共 ${cleaned.length} 条结果`)
    } catch (e) {
      showToast(`搜索失败：${(e as Error).message}`)
    }
  }

  /** 确认后清空全部搜索历史 */
  async function clearSearchHistory() {
    if (!searchHistory.length) return
    const result = await askConfirm({
      title: '清空搜索历史',
      message: `将清除全部 ${searchHistory.length} 条搜索记录，此操作不可撤销。`,
      confirmText: '清空',
      danger: true
    })
    if (result !== 'confirm') return
    setSearchHistory(await window.fly.books.clearHistory())
    showToast('已清空搜索历史')
  }

  /**
   * 直接删除单条搜索历史（无需确认）
   * @param keyword 要删除的关键词
   */
  async function removeSearchHistoryItem(keyword: string) {
    setSearchHistory(await window.fly.books.removeHistory(keyword))
  }

  /**
   * 将搜索结果加入书架；若存在同名书可选择换源或仍添加
   * @param book 搜索命中的书籍
   */
  async function addToShelf(book: SearchBook) {
    const id = bookId(book.origin, book.bookUrl)
    if (shelf.some((b) => b.id === id)) {
      showToast(`《${book.name}》已在书架`)
      return
    }

    const dups = findShelfDuplicates(shelf, book)
      .filter((b) => b.id !== id)
      .slice()
      .sort((a, b) => (b.lastReadAt || 0) - (a.lastReadAt || 0))

    if (dups.length) {
      const existing = dups[0]
      const more =
        dups.length > 1 ? `\n（书架上另有 ${dups.length - 1} 个同名源）` : ''
      const action = await askConfirm({
        title: '已在书架',
        message: `《${book.name}》已在书架（当前源：${existing.originName || '未知'}）。\n是否换到「${book.originName || '未知'}」？${more}`,
        confirmText: '仍然添加',
        cancelText: '取消',
        extraText: '换到此源'
      })
      if (action === 'cancel') return
      if (action === 'extra') {
        showToast('正在换源…')
        try {
          await replaceShelfWithSearchBook(existing, book)
          showToast(`已换源到「${book.originName}」`)
        } catch (e) {
          showToast(`换源失败：${(e as Error).message}`)
        }
        return
      }
      // 确认 → 仍作为独立书架条目添加
    }

    // 仅加书架不拉详情：用空详情构造，lastReadAt 保持 0（未读过）
    const item: ShelfBook = {
      ...buildShelfBook(book, { info: null, tocUrl: book.bookUrl, chapters: [] }),
      lastReadAt: 0,
      chapters: undefined
    }
    const next = await window.fly.shelf.upsert(item)
    setShelf(next)
    showToast(`已加入书架：${book.name}`)
  }

  /**
   * 用搜索命中替换书架已有书籍（换源），尽量匹配原阅读章节
   * @param old 原书架书籍
   * @param candidate 新书源搜索命中
   */
  async function replaceShelfWithSearchBook(old: ShelfBook, candidate: SearchBook) {
    setShelfBusyIds((prev) => (prev.includes(old.id) ? prev : [...prev, old.id]))
    try {
      const detail = await fetchBookDetail(candidate.origin, candidate.bookUrl)
      if (!detail.chapters.filter((c: Chapter) => c.url && !c.isVolume).length) {
        throw new Error('无法获取目录')
      }
      const newBook: ShelfBook = {
        ...buildShelfBook(candidate, detail, old),
        chapterIndex: matchChapterIndex(old, detail.chapters)
      }
      await window.fly.shelf.remove(old.id)
      const next = await window.fly.shelf.upsert(newBook)
      setShelf(next)
      if (candidate.origin) {
        setSources(await window.fly.sources.list())
      }
      return newBook
    } finally {
      setShelfBusyIds((prev) => prev.filter((id) => id !== old.id))
    }
  }

  /**
   * 更新单本在线书籍的详情与目录
   * @param book 书架书籍
   */
  async function updateShelfBook(book: ShelfBook) {
    if (isLocalBook(book)) {
      showToast('本地书籍无需更新')
      return
    }
    if (shelfBusyIds.includes(book.id) || shelfUpdatingAll) return
    setShelfBusyIds((prev) => (prev.includes(book.id) ? prev : [...prev, book.id]))
    try {
      const detail = await fetchBookDetail(book.origin, book.bookUrl, book.tocUrl)
      const list = detail.chapters
      const kind = catalogUpdateKind(book.chapters, list)
      const added = countAddedChapters(book.chapters, list)
      const total = readableChapters(list).length
      const updated: ShelfBook = {
        ...buildShelfBook(book, detail, book),
        lastChapter:
          detail.info?.lastChapter || list[list.length - 1]?.title || book.lastChapter,
        chapterIndex: matchChapterIndex(book, list)
      }
      const next = await window.fly.shelf.upsert(updated)
      setShelf(next)
      if (kind === 'added' && added > 0) {
        showToast(`《${updated.name}》已更新，新增 ${added} 章（共 ${total} 章）`)
      } else if (kind === 'first') {
        showToast(`《${updated.name}》已同步目录，共 ${total} 章`)
      } else if (kind === 'added') {
        showToast(`《${updated.name}》目录已刷新（共 ${total} 章）`)
      } else {
        showToast(`《${updated.name}》暂无更新`)
      }
    } catch (e) {
      showToast(`更新失败：${(e as Error).message}`)
    } finally {
      setShelfBusyIds((prev) => prev.filter((id) => id !== book.id))
    }
  }

  /** 批量更新书架上全部在线书籍 */
  async function updateAllShelfBooks() {
    if (!shelf.length || shelfUpdatingAll) return
    setShelfUpdatingAll(true)
    setShelfUpdateProgress(`0/${shelf.length}`)
    const beforeLens = new Map(
      shelf.map((b) => [b.id, readableChapters(b.chapters).length] as const)
    )
    try {
      const next = await runWithProgress(
        (track) => {
          track(window.fly.shelf.onUpdateProgress((p) => setShelfUpdateProgress(formatProgressLabel(p))))
        },
        () => window.fly.shelf.updateAll()
      )
      setShelf(next)
      let booksWithNew = 0
      let chaptersAdded = 0
      let firstSynced = 0
      for (const b of next) {
        if (isLocalBook(b)) continue
        const prev = beforeLens.get(b.id) || 0
        const now = readableChapters(b.chapters).length
        if (prev === 0 && now > 0) {
          firstSynced += 1
        } else if (now > prev) {
          booksWithNew += 1
          chaptersAdded += now - prev
        }
      }
      if (booksWithNew > 0) {
        showToast(`书架更新完成：${booksWithNew} 本有更新，共新增 ${chaptersAdded} 章`)
      } else if (firstSynced > 0) {
        showToast(`书架已同步 ${firstSynced} 本目录，暂无新章节`)
      } else {
        showToast('书架暂无更新')
      }
    } catch (e) {
      showToast(`全部更新失败：${(e as Error).message}`)
    } finally {
      setShelfUpdatingAll(false)
      setShelfUpdateProgress('')
    }
  }

  /**
   * 打开换源对话框（对话框内部完成搜索与候选展示；这里只负责挂起/收起面板）
   * @param book 当前书架书籍
   */
  function openChangeSource(book: ShelfBook) {
    if (isLocalBook(book)) {
      showToast('本地书籍不支持换源')
      return
    }
    if (!enabledCount) {
      showToast('请先导入并启用书源')
      goView('sources')
      return
    }
    setChangeSource(book)
  }

  /** 对话框选中候选后：替换书架条目并关闭面板 */
  async function applyChangeSource(candidate: SearchBook) {
    if (!changeSource) return
    const old = changeSource
    try {
      await replaceShelfWithSearchBook(old, candidate)
      setChangeSource(null)
      showToast(`已换源到「${candidate.originName}」`)
    } catch (e) {
      const msg = errMsg(e, '网络或书源异常')
      showToast(`换源失败：${msg}`)
      throw new Error(msg)
    }
  }

  /** 关闭换源对话框 */
  function closeChangeSource() {
    setChangeSource(null)
  }

  /**
   * 打开书架书籍进入阅读：拉取目录与正文，失败时可移除或换源
   * @param book 书架书籍
   */
  async function openBook(book: ShelfBook) {
    const openedAt = Date.now()
    const withRead: ShelfBook = { ...book, lastReadAt: openedAt, updatedAt: openedAt }
    setReading(withRead)
    setView('reader')
    setContent('')
    setChapters(book.chapters || [])
    setLoadingContent(true)
    try {
      let infoChapters = book.chapters
      const isLocal = isLocalBook(book)
      if (!infoChapters?.length && !isLocal) {
        // 详情失败时 fetchBookDetail 内部已用 bookUrl 兜底拉目录，两者都失败才抛错
        const detail = await fetchBookDetail(book.origin, book.bookUrl)
        infoChapters = detail.chapters
        const updated: ShelfBook = buildShelfBook(book, detail, withRead)
        const next = await window.fly.shelf.upsert(updated)
        setShelf(next)
        setReading(updated)
        setChapters(infoChapters || [])
        book = updated
      } else {
        const next = await window.fly.shelf.upsert(withRead)
        setShelf(next)
        book = withRead
      }
      if (!book.chapters?.length) {
        throw new Error(isLocal ? '本地书籍没有章节，请重新导入' : '无法获取目录')
      }

      const idx = Math.min(book.chapterIndex, Math.max((book.chapters?.length || 1) - 1, 0))
      await loadChapter(book, idx, book.chapters || infoChapters || [], { resetScroll: false })
    } catch (e) {
      const msg = (e as Error).message || '未知错误'
      setLoadingContent(false)
      setReading(null)
      setChapters([])
      setContent('')
      goView('shelf')
      showToast(`打开失败：${msg}`)
      const action = await askConfirm({
        title: '打开失败',
        message: `《${withRead.name}》打开失败：\n${msg}`,
        confirmText: '移除',
        cancelText: '保留',
        extraText: isLocalBook(withRead) ? undefined : '换源',
        danger: true
      })
      if (action === 'extra') {
        void openChangeSource(withRead)
        return
      }
      if (action !== 'confirm') return
      try {
        const next = await window.fly.shelf.remove(withRead.id)
        setShelf(next)
        showToast(`已移除《${withRead.name}》`)
      } catch (err) {
        showToast(`移除失败：${(err as Error).message}`)
      }
    }
  }

  /**
   * 探测搜索命中的详情与目录；可读时返回可入库的书架书籍
   * @param b 搜索命中
   */
  async function probeSearchBook(
    b: SearchBook,
    onStage?: (stage: string) => void
  ): Promise<
    | { ok: true; item: ShelfBook }
    | { ok: false; message: string }
  > {
    try {
      onStage?.('正在获取书籍详情…')
      const detail = await fetchBookDetail(b.origin, b.bookUrl)
      onStage?.('正在获取目录…')
      const chapters = detail.chapters
      const info = detail.info
      const infoName = pickBookName(info?.name, b.name)
      const infoAuthor = pickBookAuthor(info?.author, b.author)
      const infoCover = info?.coverUrl || b.coverUrl
      const infoIntro = info?.intro || b.intro
      const infoLast = info?.lastChapter || b.lastChapter
      const infoKind = info?.kind || b.kind
      const readable = chapters.filter((c) => c.url && !c.isVolume)
      if (!readable.length) {
        return { ok: false, message: '无法获取目录，书源可能失效或规则不兼容' }
      }
      const item: ShelfBook = {
        ...buildShelfBook(b, detail, undefined),
        name: infoName,
        author: infoAuthor,
        coverUrl: infoCover,
        intro: infoIntro,
        kind: infoKind,
        lastChapter: infoLast
      }
      return { ok: true, item }
    } catch (e) {
      return { ok: false, message: (e as Error).message || '书籍不可用' }
    }
  }

  /**
   * 阅读前轻量校验搜索命中（详情 + 非空目录）；失败时可尝试同名其他源
   * @param b 搜索命中
   */
  async function readSearchBook(b: SearchBook) {
    // 用持久遮罩代替短 toast：详情+目录可能耗时数十秒，遮罩保持到校验结束并分阶段提示
    setProbeState('正在校验书籍…')
    try {
      const first = await probeSearchBook(b, setProbeState)
      if (first.ok) {
        setProbeState(null)
        const next = await window.fly.shelf.upsert(first.item)
        setShelf(next)
        setSources(await window.fly.sources.list())
        const saved = next.find((x) => x.id === first.item.id) || first.item
        void openBook(saved)
        return
      }

      setProbeState(null)
      const alts = findSameNovelSearchAlts(results, b)
      const action = await askConfirm({
        title: '无法阅读',
        message: `《${b.name}》（${b.originName || '未知源'}）\n${first.message}`,
        confirmText: '仅加书架',
        cancelText: '取消',
        extraText: alts.length ? `尝试其他源（${alts.length}）` : undefined
      })

      if (action === 'cancel') return

      if (action === 'confirm') {
        await addToShelf(b)
        return
      }

      // 尝试当前搜索结果中的同名其他源
      for (let i = 0; i < alts.length; i++) {
        const alt = alts[i]
        setProbeState(`正在尝试其他源（${i + 1}/${alts.length}）：${alt.originName || '未知'}…`)
        const probed = await probeSearchBook(alt, setProbeState)
        if (!probed.ok) continue
        setProbeState(null)
        const next = await window.fly.shelf.upsert(probed.item)
        setShelf(next)
        setSources(await window.fly.sources.list())
        const saved = next.find((x) => x.id === probed.item.id) || probed.item
        showToast(`已改用「${alt.originName}」`)
        void openBook(saved)
        return
      }
      setProbeState(null)
      showToast('同名其他源均无法获取目录')
    } catch (e) {
      setProbeState(null)
      showToast(`校验失败：${(e as Error).message || '未知错误'}`)
    }
  }

  /** 通过系统文件选择器导入本地书籍并可选立即打开 */
  async function importLocalBook() {
    const r = await window.fly.shelf.importLocal()
    if (!r.ok) {
      if (r.message !== '已取消') showToast(r.message)
      return
    }
    if (r.shelf) setShelf(r.shelf)
    showToast(r.message)
    if (r.book) void openBook(r.book)
  }

  /**
   * 加载指定章节正文，更新阅读进度，并按偏好预加载后续章节
   * @param book 当前阅读书籍
   * @param index 章节索引
   * @param list 目录列表
   * @param options.resetScroll 是否重置滚动位置
   */
  async function loadChapter(
    book: ShelfBook,
    index: number,
    list: Chapter[],
    options?: { resetScroll?: boolean; refresh?: boolean }
  ) {
    const chapter = list[index]
    if (!chapter?.url) {
      setContent('（分卷或无效章节）')
      setLoadingContent(false)
      return
    }
    const refresh = options?.refresh ?? false
    const resetScroll = options?.resetScroll ?? index !== book.chapterIndex
    const mdDoc = isMdBook(book)

    // Markdown 全文模式：目录切换只改锚点，正文已在内存则不再重复拉取
    if (
      !refresh &&
      mdDoc &&
      readingRef.current?.id === book.id &&
      contentRef.current.trim().length > 0 &&
      !contentRef.current.startsWith('加载失败') &&
      index !== book.chapterIndex
    ) {
      const updated: ShelfBook = {
        ...book,
        chapterIndex: index,
        chapters: list,
        scrollTop: resetScroll ? 0 : book.scrollTop || 0,
        lastReadAt: Date.now(),
        updatedAt: Date.now()
      }
      setReading(updated)
      setLoadingContent(false)
      void window.fly.shelf.upsert(updated).then(setShelf)
      return
    }

    const gen = ++chapterLoadGenRef.current
    setLoadingContent(true)
    try {
      const text = await window.fly.books.content(
        book.origin,
        chapter.url,
        book.id,
        refresh
      )
      if (gen !== chapterLoadGenRef.current) return
      setContent(text || '（正文为空，可能是书源规则不兼容）')
      setReaderCacheTick((n) => n + 1)
      const updated: ShelfBook = {
        ...book,
        chapterIndex: index,
        chapters: list,
        scrollTop: resetScroll ? 0 : book.scrollTop || 0,
        lastReadAt: Date.now(),
        updatedAt: Date.now()
      }
      setReading(updated)
      const next = await window.fly.shelf.upsert(updated)
      if (gen !== chapterLoadGenRef.current) return
      setShelf(next)
      if (refresh) showToast('已重新加载本章正文')

      const preloadN = Math.max(0, prefs.preloadCount ?? 3)
      if (preloadN > 0 && !isLocalBook(book)) {
        const upcoming = list
          .slice(index + 1)
          .filter((c) => c.url && !c.isVolume)
          .slice(0, preloadN)
          .map((c) => c.url)
        if (upcoming.length) {
          void window.fly.books.preload(book.origin, book.id, upcoming).then(() => {
            if (gen !== chapterLoadGenRef.current) return
            setReaderCacheTick((n) => n + 1)
            void window.fly.shelf.list().then(setShelf)
          })
        }
      }
    } catch (e) {
      if (gen !== chapterLoadGenRef.current) return
      if (refresh) {
        // 刷新失败保留当前正文，只提示错误
        showToast((e as Error).message || '重新加载失败')
      } else {
        setContent(`加载失败：${(e as Error).message}`)
      }
    } finally {
      if (gen === chapterLoadGenRef.current) {
        setLoadingContent(false)
      }
    }
  }

  /**
   * 持久化当前阅读滚动位置（变化过小时跳过）
   * @param scrollTop 滚动像素
   */
  async function saveReadingScroll(scrollTop: number) {
    const current = readingRef.current
    if (!current) return
    const top = Math.max(0, Math.round(scrollTop))
    if (Math.abs((current.scrollTop || 0) - top) < 2) return
    const updated: ShelfBook = {
      ...current,
      scrollTop: top,
      updatedAt: Date.now()
    }
    readingRef.current = updated
    setReading(updated)
    try {
      const patched = await window.fly.shelf.patchProgress(current.id, { scrollTop: top })
      if (patched) {
        setShelf((prev) =>
          prev.map((b) =>
            b.id === patched.id
              ? { ...b, scrollTop: patched.scrollTop, updatedAt: patched.updatedAt }
              : b
          )
        )
      }
    } catch {
      /* 持久化失败时仍保留本地阅读位置 */
    }
  }

  /**
   * Markdown 滚动时同步目录高亮：只改 chapterIndex，不碰 scrollTop。
   */
  async function saveActiveChapter(index: number) {
    const current = readingRef.current
    if (!current) return
    if (current.chapterIndex === index) return
    if (index < 0 || index >= (current.chapters?.length || chapters.length)) return
    const updated: ShelfBook = {
      ...current,
      chapterIndex: index,
      updatedAt: Date.now()
    }
    readingRef.current = updated
    setReading(updated)
    try {
      const patched = await window.fly.shelf.patchProgress(current.id, { chapterIndex: index })
      if (patched) {
        setShelf((prev) =>
          prev.map((b) =>
            b.id === patched.id
              ? { ...b, chapterIndex: patched.chapterIndex, updatedAt: patched.updatedAt }
              : b
          )
        )
      }
    } catch {
      /* ignore */
    }
  }

  /**
   * 保存阅读器设置到主进程
   * @param next 新的阅读设置
   */
  async function saveSettings(next: ReaderSettings) {
    setSettings(next)
    await window.fly.settings.save(next)
  }

  // 将本次渲染的最新回调实现写入 ref（渲染期同步赋值，保证事件触发时调用的是最新闭包）
  viewCbRef.current = {
    onOpen: (b) => void openBook(b),
    onImportLocal: () => void importLocalBook(),
    onSortChange: (sort) => {
      setPrefs((p) => ({ ...p, shelfSort: sort }))
      void window.fly.prefs.save({ shelfSort: sort }).then((r) => setPrefs(r.prefs))
    },
    onUpdate: (b) => void updateShelfBook(b),
    onUpdateAll: () => void updateAllShelfBooks(),
    onChangeSource: (b) => void openChangeSource(b),
    onCache: async (b) => {
      if (isLocalBook(b)) {
        showToast('本地书籍无需缓存')
        return
      }
      if (cacheBusyIds.includes(b.id)) return
      setCacheBusyIds((prev) => [...prev, b.id])
      try {
        const next = await window.fly.shelf.cacheBook(b.id)
        setShelf(next)
        const info = next.find((x) => x.id === b.id)?.cache
        if (info?.status === 'full') showToast(`《${b.name}》已全部缓存`)
        else showToast(`《${b.name}》缓存 ${info?.cached ?? 0}/${info?.total ?? 0}`)
      } catch (e) {
        showToast(`缓存失败：${(e as Error).message}`)
        setShelf(await window.fly.shelf.list())
      } finally {
        setCacheBusyIds((prev) => prev.filter((id) => id !== b.id))
        setCacheProgressMap((prev) => {
          const n = { ...prev }
          delete n[b.id]
          return n
        })
      }
    },
    onCancelCache: async (id) => {
      pendingExportsRef.current.delete(id)
      const next = await window.fly.shelf.cancelCache(id)
      setShelf(next)
      setCacheBusyIds((prev) => prev.filter((x) => x !== id))
      setCacheProgressMap((prev) => {
        const n = { ...prev }
        delete n[id]
        return n
      })
      showToast('已取消缓存')
    },
    onExportTxt: async (b) => {
      if (isLocalBook(b)) {
        showToast('本地书籍请直接使用原文件')
        return
      }
      const info =
        b.cache ||
        (await window.fly.shelf.cacheStatus(b.id)) || {
          bookId: b.id,
          total: 0,
          cached: 0,
          status: 'none' as const
        }
      const fullyCached =
        info.status === 'full' || (info.total > 0 && info.cached >= info.total)

      if (fullyCached) {
        await runExportTxt(b.id, b.name)
        return
      }

      const cachedLabel = info.total > 0 ? `（当前 ${info.cached}/${info.total}）` : ''
      const result = await askConfirm({
        title: '需要先缓存',
        message: `《${b.name}》尚未全部缓存${cachedLabel}。确认后将先缓存全书，缓存完成后自动导出为 TXT。`,
        confirmText: '开始缓存并导出',
        cancelText: '取消'
      })
      if (result !== 'confirm') return

      pendingExportsRef.current.set(b.id, b.name)
      showToast('缓存完成后将自动导出')

      if (cacheBusyIds.includes(b.id) || info.status === 'caching') {
        return
      }

      setCacheBusyIds((prev) => [...prev, b.id])
      try {
        const next = await window.fly.shelf.cacheBook(b.id)
        setShelf(next)
        if (pendingExportsRef.current.has(b.id)) {
          pendingExportsRef.current.delete(b.id)
          await runExportTxt(b.id, b.name)
        }
      } catch (e) {
        pendingExportsRef.current.delete(b.id)
        showToast(`缓存失败：${(e as Error).message}`)
        setShelf(await window.fly.shelf.list())
      } finally {
        setCacheBusyIds((prev) => prev.filter((id) => id !== b.id))
        setCacheProgressMap((prev) => {
          const n = { ...prev }
          delete n[b.id]
          return n
        })
      }
    },
    onRemove: async (id) => {
      const book = shelf.find((b) => b.id === id)
      const result = await askConfirm({
        title: '移出书架',
        message: book
          ? `确定从书架移除《${book.name}》？本地缓存也会一并删除。`
          : '确定从书架移除这本书？',
        confirmText: '移除',
        danger: true
      })
      if (result !== 'confirm') return
      const next = await window.fly.shelf.remove(id)
      setShelf(next)
      showToast(book ? `已移除《${book.name}》` : '已移除')
    },
    onSearch: () => goView('search'),
    onSearchRun: () => void runSearch(),
    onSearchKeyword: (k) => void runSearch(k),
    onClearHistory: () => void clearSearchHistory(),
    onRemoveHistory: (k) => void removeSearchHistoryItem(k),
    onRead: (b) => void readSearchBook(b),
    onRankOpenBook: (name) => {
      const key = name.trim()
      if (!key) return
      setKeyword(key)
      goView('search')
      void runSearch(key)
    },
    onTestKeywordChange: (v) => {
      setPrefs((p) => ({ ...p, sourceTestKeyword: v }))
    },
    onTestKeywordCommit: (v) => {
      void window.fly.prefs.save({ sourceTestKeyword: v }).then((r) => setPrefs(r.prefs))
    },
    onImportFile: () => void onImportFile(),
    onPrefsChange: async (patch) => {
      const r = await window.fly.prefs.save(patch)
      setPrefs(r.prefs)
      if (patch.bossKey !== undefined || patch.bossKeyEnabled !== undefined) {
        showToast(r.bossKey.message)
      }
    },
    onRemovePurify: async (rule) => {
      const next = await window.fly.settings.removePurify(rule)
      setSettings(next)
      showToast('已移除净化规则')
    },
    onLegacyStatus: () => window.fly.data.legacyStatus(),
    onImportLegacy: async () => {
      const r = await window.fly.data.importLegacy()
      await reloadAllState()
      showToast(
        `已导入旧版数据：${r.merged.shelf} 本书 / ${r.merged.sources} 个书源 / ${r.merged.chapters} 章缓存`
      )
      return r
    },
    onBack: () => goView('shelf'),
    onSelectChapter: (i) => {
      if (reading) void loadChapter(reading, i, chapters, { resetScroll: true })
    },
    onPrev: () => {
      if (!reading) return
      if (reading.chapterIndex > 0)
        void loadChapter(reading, reading.chapterIndex - 1, chapters, { resetScroll: true })
    },
    onNext: () => {
      if (!reading) return
      if (reading.chapterIndex < chapters.length - 1)
        void loadChapter(reading, reading.chapterIndex + 1, chapters, { resetScroll: true })
    },
    onScrollSave: (top) => void saveReadingScroll(top),
    onActiveChapter: (i) => void saveActiveChapter(i),
    onSettingsChange: (s) => void saveSettings(s),
    onAddPurify: async (text) => {
      const next = await window.fly.settings.addPurify(text)
      setSettings(next)
      showToast(`已加入净化：${text.slice(0, 24)}${text.length > 24 ? '…' : ''}`)
    },
    onReloadChapter: () => {
      if (!reading) return
      if (isLocalBook(reading)) {
        showToast('本地书籍无需重新加载')
        return
      }
      if (loadingContent) return
      void loadChapter(reading, reading.chapterIndex, chapters, {
        resetScroll: false,
        refresh: true
      })
    }
  }

  // 身份稳定的视图回调包装：仅创建一次，转发到 viewCbRef.current，使 React.memo 真正生效
  const shelfCb = useMemo<Pick<ViewCallbacks, 'onOpen' | 'onImportLocal' | 'onSortChange' | 'onUpdate' | 'onUpdateAll' | 'onChangeSource' | 'onCache' | 'onCancelCache' | 'onExportTxt' | 'onRemove' | 'onSearch'>>(
    () => ({
      onOpen: (b) => callView((cb) => void cb.onOpen(b)),
      onImportLocal: () => callView((cb) => cb.onImportLocal()),
      onSortChange: (s) => callView((cb) => cb.onSortChange(s)),
      onUpdate: (b) => callView((cb) => void cb.onUpdate(b)),
      onUpdateAll: () => callView((cb) => cb.onUpdateAll()),
      onChangeSource: (b) => callView((cb) => void cb.onChangeSource(b)),
      onCache: (b) => callView((cb) => void cb.onCache(b)),
      onCancelCache: (id) => callView((cb) => cb.onCancelCache(id)),
      onExportTxt: (b) => callView((cb) => void cb.onExportTxt(b)),
      onRemove: (id) => callView((cb) => void cb.onRemove(id)),
      onSearch: () => callView((cb) => cb.onSearch())
    }),
    [callView]
  )

  const searchCb = useMemo<Pick<ViewCallbacks, 'onSearch' | 'onSearchKeyword' | 'onClearHistory' | 'onRemoveHistory' | 'onRead'>>(
    () => ({
      onSearch: () => callView((cb) => cb.onSearchRun()),
      onSearchKeyword: (k) => callView((cb) => cb.onSearchKeyword(k)),
      onClearHistory: () => callView((cb) => cb.onClearHistory()),
      onRemoveHistory: (k) => callView((cb) => cb.onRemoveHistory(k)),
      onRead: (b) => callView((cb) => void cb.onRead(b))
    }),
    [callView]
  )

  const rankCb = useMemo<Pick<ViewCallbacks, 'onRankOpenBook'>>(
    () => ({
      onRankOpenBook: (name) => callView((cb) => cb.onRankOpenBook(name))
    }),
    [callView]
  )

  const sourcesCb = useMemo<Pick<ViewCallbacks, 'onTestKeywordChange' | 'onTestKeywordCommit' | 'onImportFile'>>(
    () => ({
      onTestKeywordChange: (v) => callView((cb) => cb.onTestKeywordChange(v)),
      onTestKeywordCommit: (v) => callView((cb) => cb.onTestKeywordCommit(v)),
      onImportFile: () => callView((cb) => cb.onImportFile())
    }),
    [callView]
  )

  const settingsCb = useMemo<
    Pick<
      ViewCallbacks,
      'onPrefsChange' | 'onRemovePurify' | 'onLegacyStatus' | 'onImportLegacy'
    >
  >(
    () => ({
      onPrefsChange: (p) => callView((cb) => void cb.onPrefsChange(p)),
      onRemovePurify: (r) => callView((cb) => void cb.onRemovePurify(r)),
      onLegacyStatus: () => callView((cb) => cb.onLegacyStatus()),
      onImportLegacy: () => callView((cb) => cb.onImportLegacy())
    }),
    [callView]
  )

  const readerCb = useMemo<
    Pick<
      ViewCallbacks,
      | 'onBack'
      | 'onSelectChapter'
      | 'onPrev'
      | 'onNext'
      | 'onScrollSave'
      | 'onActiveChapter'
      | 'onSettingsChange'
      | 'onAddPurify'
      | 'onReloadChapter'
    >
  >(
    () => ({
      onBack: () => callView((cb) => cb.onBack()),
      onSelectChapter: (i) => callView((cb) => cb.onSelectChapter(i)),
      onPrev: () => callView((cb) => cb.onPrev()),
      onNext: () => callView((cb) => cb.onNext()),
      onScrollSave: (t) => callView((cb) => cb.onScrollSave(t)),
      onActiveChapter: (i) => callView((cb) => cb.onActiveChapter(i)),
      onSettingsChange: (s) => callView((cb) => cb.onSettingsChange(s)),
      onAddPurify: (t) => callView((cb) => void cb.onAddPurify(t)),
      onReloadChapter: () => callView((cb) => cb.onReloadChapter())
    }),
    [callView]
  )

  const toastStack = toasts.length ? (
    <div className="toast-stack" role="status" aria-live="polite">
      {toasts.map((t) => (
        <Notification key={t.id} withCloseButton={false} color="accent" className="toast-note">
          {t.text}
        </Notification>
      ))}
    </div>
  ) : null
  const confirmOverlay = confirmReq ? (
    <ConfirmDialog request={confirmReq} onClose={closeConfirm} />
  ) : null

  if (view === 'reader' && reading) {
    return (
      <>
        <WindowChrome />
        <ReaderView
          book={reading}
          chapters={chapters}
          content={content}
          loading={loadingContent}
          settings={settings}
          cacheTick={readerCacheTick}
          onBack={readerCb.onBack}
          onSelectChapter={readerCb.onSelectChapter}
          onPrev={readerCb.onPrev}
          onNext={readerCb.onNext}
          onScrollSave={readerCb.onScrollSave}
          onActiveChapter={readerCb.onActiveChapter}
          onSettingsChange={readerCb.onSettingsChange}
          onAddPurify={readerCb.onAddPurify}
          onReloadChapter={readerCb.onReloadChapter}
        />
        {toastStack}
        {confirmOverlay}
      </>
    )
  }

  return (
    <div className="app">
      <WindowChrome />
      <aside className="sidebar" data-tauri-drag-region="deep">
        <div className="brand">
          <h1>FlyReader</h1>
          <p>轻量阅读 · Legado 书源</p>
        </div>
        <nav className="nav">
          {(
            [
              ['shelf', '书架'],
              ['search', '搜书'],
              ['rank', '排行'],
              ['sources', '书源'],
              ['settings', '设置'],
              ['about', '关于']
            ] as const
          ).map(([id, label]) => (
            <Button
              key={id}
              className="nav-item"
              mod={{ active: view === id }}
              variant="subtle"
              radius="md"
              size="sm"
              justify="flex-start"
              onClick={() => goView(id)}
            >
              {label}
            </Button>
          ))}
        </nav>
        <SidebarMotto />
      </aside>

      <section className="main">
        <div
          className="topbar"
          aria-hidden="true"
          data-tauri-drag-region=""
          onDoubleClick={() => {
            if (document.documentElement.dataset.os === 'macos') return
            void import('@tauri-apps/api/window').then(({ getCurrentWindow }) =>
              getCurrentWindow().toggleMaximize()
            )
          }}
        />
        <div className="content">
          {view === 'shelf' && (
            <ShelfView
              shelf={shelf}
              sort={prefs.shelfSort || 'lastRead'}
              busyIds={shelfBusyIds}
              cacheBusyIds={cacheBusyIds}
              cacheProgressMap={cacheProgressMap}
              updatingAll={shelfUpdatingAll}
              updateProgress={shelfUpdateProgress}
              onOpen={shelfCb.onOpen}
              onImportLocal={shelfCb.onImportLocal}
              onSortChange={shelfCb.onSortChange}
              onUpdate={shelfCb.onUpdate}
              onUpdateAll={shelfCb.onUpdateAll}
              onChangeSource={shelfCb.onChangeSource}
              onCache={shelfCb.onCache}
              onCancelCache={shelfCb.onCancelCache}
              onExportTxt={shelfCb.onExportTxt}
              onRemove={shelfCb.onRemove}
              onSearch={shelfCb.onSearch}
            />
          )}
          {view === 'search' && (
            <SearchView
              keyword={keyword}
              setKeyword={setKeyword}
              searching={searching}
              progress={searchProgress}
              results={results}
              history={searchHistory}
              onSearch={searchCb.onSearch}
              onSearchKeyword={searchCb.onSearchKeyword}
              onClearHistory={searchCb.onClearHistory}
              onRemoveHistory={searchCb.onRemoveHistory}
              onRead={searchCb.onRead}
            />
          )}
          {view === 'rank' && <RankView onOpenBook={rankCb.onRankOpenBook} />}
          {view === 'sources' && (
            <SourcesView
              sources={sources}
              shelf={shelf}
              testKeyword={prefs.sourceTestKeyword}
              onTestKeywordChange={sourcesCb.onTestKeywordChange}
              onTestKeywordCommit={sourcesCb.onTestKeywordCommit}
              askConfirm={askConfirm}
              showToast={showToast}
              onImportFile={sourcesCb.onImportFile}
              onSourcesChange={setSources}
            />
          )}
          {view === 'settings' && (
            <SettingsView
              settings={settings}
              prefs={prefs}
              askConfirm={askConfirm}
              onPrefsChange={settingsCb.onPrefsChange}
              onRemovePurify={settingsCb.onRemovePurify}
              onLegacyStatus={settingsCb.onLegacyStatus}
              onImportLegacy={settingsCb.onImportLegacy}
            />
          )}
          {view === 'about' && (
            <AboutView showToast={showToast} askConfirm={askConfirm} />
          )}
        </div>
      </section>
      {toastStack}
      {confirmOverlay}
      {probeState ? (
        <div className="probe-overlay" role="status" aria-live="polite">
          <div className="probe-card">
            <span className="probe-spinner" aria-hidden="true" />
            <span>{probeState}</span>
          </div>
        </div>
      ) : null}
      {changeSource ? (
        <ChangeSourceDialog
          book={changeSource}
          filterCandidates={(book, hits) =>
            filterChangeSourceCandidates(book, hits).filter(
              (b) => !(b.origin === book.origin && b.bookUrl === book.bookUrl)
            )
          }
          apply={(c) => applyChangeSource(c)}
          onClose={closeChangeSource}
        />
      ) : null}
    </div>
  )
}


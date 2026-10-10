import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Button, Loader, Modal, Notification, Text, useMantineColorScheme } from '@mantine/core'
import { filterChangeSourceCandidates, filterRelevantSearchHits, dedupeSearchBooks } from '@shared/searchRelevance'
import { authorLabel } from '@shared/author'
import { pickBookAuthor, pickBookName } from '@shared/bookMeta'
import { isLocalBook } from '@shared/bookLocal'
import { catalogUpdateKind, countAddedChapters, matchChapterIndex, readableChapters } from '@shared/matchChapter'
import {
  DEFAULT_PREFS,
  DEFAULT_SETTINGS,
  type AppPrefs,
  type BookSource,
  type Chapter,
  type ReaderSettings,
  type SearchBook,
  type SearchProgress,
  type ShelfBook,
  type ShelfSort,
  type ViewName
} from '@shared/types'
import { ConfirmDialog, type ConfirmOutcome, type ConfirmRequest } from './components/ConfirmDialog'
import { ChangeSourceDialog } from './components/ChangeSourceDialog'
import { SidebarMotto } from './components/SidebarMotto'
import { WindowChrome } from './components/WindowChrome'
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
 * 由书源 origin 与书籍 URL 生成书架唯一 ID
 * @param origin 书源标识
 * @param bookUrl 书籍详情 URL
 */
function bookId(origin: string, bookUrl: string): string {
  return `${origin}::${bookUrl}`
}

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
  const [searching, setSearching] = useState(false)
  const [searchProgress, setSearchProgress] = useState<SearchProgress | null>(null)
  const [results, setResults] = useState<SearchBook[]>([])
  const [searchHistory, setSearchHistory] = useState<string[]>([])
  const [reading, setReading] = useState<ShelfBook | null>(null)
  const readingRef = useRef<ShelfBook | null>(null)
  readingRef.current = reading
  const viewRef = useRef<ViewName>('shelf')
  viewRef.current = view
  const searchGenRef = useRef(0)
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
    // eslint-disable-next-line react-hooks/exhaustive-deps
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
   * 执行多书源并发搜索（支持进度与中间结果流式更新）
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
    const gen = ++searchGenRef.current
    setKeyword(key)
    setSearching(true)
    setResults([])
    setSearchProgress({
      done: 0,
      total: enabledCount,
      found: 0,
      current: '',
      phase: 'start'
    })
    try {
      const { books, history, sources: nextSources } = await runWithProgress(
        (track) => {
          track(
            window.fly.books.onSearchProgress((p) => {
              if (gen !== searchGenRef.current) return
              setSearchProgress(p)
            })
          )
          track(
            window.fly.books.onSearchPartial((books) => {
              if (gen !== searchGenRef.current) return
              const filtered = filterRelevantSearchHits(key, books)
              if (!filtered.length) return
              setResults((prev) => dedupeSearchBooks(prev.concat(filtered)))
            })
          )
        },
        () => window.fly.books.search(key)
      )
      if (gen !== searchGenRef.current) return
      const cleaned = dedupeSearchBooks(filterRelevantSearchHits(key, books))
      setResults(cleaned)
      setSearchHistory(history)
      if (nextSources?.length) setSources(nextSources)
      else {
        const latest = await window.fly.sources.list()
        setSources(latest)
      }
      if (!cleaned.length) showToast('未搜索到结果，可换个书源或关键词')
      else showToast(`搜索完成，共 ${cleaned.length} 条结果`)
    } catch (e) {
      if (gen !== searchGenRef.current) return
      showToast(`搜索失败：${(e as Error).message}`)
    } finally {
      if (gen === searchGenRef.current) {
        setSearching(false)
        setSearchProgress(null)
      }
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

    const now = Date.now()
    const item: ShelfBook = {
      id,
      name: book.name,
      author: book.author,
      bookUrl: book.bookUrl,
      coverUrl: book.coverUrl,
      intro: book.intro,
      kind: book.kind,
      lastChapter: book.lastChapter,
      origin: book.origin,
      originName: book.originName,
      chapterIndex: 0,
      scrollTop: 0,
      addedAt: now,
      updatedAt: now,
      lastReadAt: 0
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
      const info = await window.fly.books.info(candidate.origin, candidate.bookUrl)
      const tocUrl = info.tocUrl || candidate.bookUrl
      const list = await window.fly.books.toc(candidate.origin, tocUrl)
      if (!list.filter((c: Chapter) => c.url && !c.isVolume).length) {
        throw new Error('无法获取目录')
      }
      const newBook: ShelfBook = {
        ...old,
        id: bookId(candidate.origin, candidate.bookUrl),
        name: pickBookName(info.name, candidate.name, old.name),
        author: pickBookAuthor(info.author, candidate.author, old.author),
        bookUrl: candidate.bookUrl,
        coverUrl: info.coverUrl || candidate.coverUrl || old.coverUrl,
        intro: info.intro || candidate.intro || old.intro,
        kind: info.kind || candidate.kind || old.kind,
        lastChapter: info.lastChapter || candidate.lastChapter,
        origin: candidate.origin,
        originName: candidate.originName,
        tocUrl,
        chapters: list,
        chapterIndex: matchChapterIndex(old, list),
        updatedAt: Date.now()
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
      const info = await window.fly.books.info(book.origin, book.bookUrl)
      const tocUrl = info.tocUrl || book.tocUrl || book.bookUrl
      const list = await window.fly.books.toc(book.origin, tocUrl)
      const kind = catalogUpdateKind(book.chapters, list)
      const added = countAddedChapters(book.chapters, list)
      const total = readableChapters(list).length
      const updated: ShelfBook = {
        ...book,
        name: pickBookName(info.name, book.name),
        author: pickBookAuthor(info.author, book.author),
        coverUrl: info.coverUrl || book.coverUrl,
        intro: info.intro || book.intro,
        lastChapter: info.lastChapter || list[list.length - 1]?.title || book.lastChapter,
        tocUrl,
        chapters: list,
        chapterIndex: matchChapterIndex(book, list),
        updatedAt: Date.now()
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
      let tocUrl = book.tocUrl || book.bookUrl
      let infoChapters = book.chapters
      const isLocal = isLocalBook(book)
      if (!infoChapters?.length && !isLocal) {
        try {
          const info = await window.fly.books.info(book.origin, book.bookUrl)
          tocUrl = info.tocUrl || book.bookUrl
          infoChapters = await window.fly.books.toc(book.origin, tocUrl)
          const updated: ShelfBook = {
            ...withRead,
            name: pickBookName(info.name, book.name),
            author: pickBookAuthor(info.author, book.author),
            coverUrl: info.coverUrl || book.coverUrl,
            intro: info.intro || book.intro,
            tocUrl,
            chapters: infoChapters,
            lastReadAt: openedAt,
            updatedAt: Date.now()
          }
          const next = await window.fly.shelf.upsert(updated)
          setShelf(next)
          setReading(updated)
          setChapters(infoChapters || [])
          book = updated
        } catch (infoErr) {
          // 详情规则可能损坏；仍用详情 URL 尝试拉目录
          try {
            infoChapters = await window.fly.books.toc(book.origin, book.bookUrl)
            if (!infoChapters?.length) throw infoErr
            const updated: ShelfBook = {
              ...withRead,
              tocUrl: book.bookUrl,
              chapters: infoChapters,
              lastReadAt: openedAt,
              updatedAt: Date.now()
            }
            const next = await window.fly.shelf.upsert(updated)
            setShelf(next)
            setReading(updated)
            setChapters(infoChapters)
            book = updated
          } catch {
            throw infoErr
          }
        }
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
      let tocUrl = b.bookUrl
      let chapters: Chapter[] = []
      let infoName = b.name
      let infoAuthor = b.author
      let infoCover = b.coverUrl
      let infoIntro = b.intro
      let infoLast = b.lastChapter
      let infoKind = b.kind
      try {
        onStage?.('正在获取书籍详情…')
        const info = await window.fly.books.info(b.origin, b.bookUrl)
        tocUrl = info.tocUrl || b.bookUrl
        infoName = pickBookName(info.name, b.name)
        infoAuthor = pickBookAuthor(info.author, b.author)
        infoCover = info.coverUrl || b.coverUrl
        infoIntro = info.intro || b.intro
        infoLast = info.lastChapter || b.lastChapter
        infoKind = info.kind || b.kind
        onStage?.('正在获取目录…')
        chapters = await window.fly.books.toc(b.origin, tocUrl)
      } catch {
        onStage?.('正在获取目录…')
        chapters = await window.fly.books.toc(b.origin, b.bookUrl)
        tocUrl = b.bookUrl
      }
      const readable = chapters.filter((c) => c.url && !c.isVolume)
      if (!readable.length) {
        return { ok: false, message: '无法获取目录，书源可能失效或规则不兼容' }
      }
      const now = Date.now()
      const item: ShelfBook = {
        id: bookId(b.origin, b.bookUrl),
        name: infoName,
        author: infoAuthor,
        bookUrl: b.bookUrl,
        coverUrl: infoCover,
        intro: infoIntro,
        kind: infoKind,
        lastChapter: infoLast,
        origin: b.origin,
        originName: b.originName,
        tocUrl,
        chapters,
        chapterIndex: 0,
        scrollTop: 0,
        addedAt: now,
        updatedAt: now,
        lastReadAt: now
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
    const mdDoc =
      (book.localFormat || '').toLowerCase() === 'md' ||
      (book.kind || '').toUpperCase() === 'MD' ||
      (book.kind || '').toUpperCase() === 'MARKDOWN'

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
      onOpen: (b) => void viewCbRef.current!.onOpen(b),
      onImportLocal: () => viewCbRef.current!.onImportLocal(),
      onSortChange: (s) => viewCbRef.current!.onSortChange(s),
      onUpdate: (b) => void viewCbRef.current!.onUpdate(b),
      onUpdateAll: () => viewCbRef.current!.onUpdateAll(),
      onChangeSource: (b) => void viewCbRef.current!.onChangeSource(b),
      onCache: (b) => void viewCbRef.current!.onCache(b),
      onCancelCache: (id) => viewCbRef.current!.onCancelCache(id),
      onExportTxt: (b) => void viewCbRef.current!.onExportTxt(b),
      onRemove: (id) => void viewCbRef.current!.onRemove(id),
      onSearch: () => viewCbRef.current!.onSearch()
    }),
    []
  )

  const searchCb = useMemo<Pick<ViewCallbacks, 'onSearch' | 'onSearchKeyword' | 'onClearHistory' | 'onRemoveHistory' | 'onRead'>>(
    () => ({
      onSearch: () => viewCbRef.current!.onSearchRun(),
      onSearchKeyword: (k) => viewCbRef.current!.onSearchKeyword(k),
      onClearHistory: () => viewCbRef.current!.onClearHistory(),
      onRemoveHistory: (k) => viewCbRef.current!.onRemoveHistory(k),
      onRead: (b) => void viewCbRef.current!.onRead(b)
    }),
    []
  )

  const rankCb = useMemo<Pick<ViewCallbacks, 'onRankOpenBook'>>(
    () => ({
      onRankOpenBook: (name) => viewCbRef.current!.onRankOpenBook(name)
    }),
    []
  )

  const sourcesCb = useMemo<Pick<ViewCallbacks, 'onTestKeywordChange' | 'onTestKeywordCommit' | 'onImportFile'>>(
    () => ({
      onTestKeywordChange: (v) => viewCbRef.current!.onTestKeywordChange(v),
      onTestKeywordCommit: (v) => viewCbRef.current!.onTestKeywordCommit(v),
      onImportFile: () => viewCbRef.current!.onImportFile()
    }),
    []
  )

  const settingsCb = useMemo<
    Pick<
      ViewCallbacks,
      'onPrefsChange' | 'onRemovePurify' | 'onLegacyStatus' | 'onImportLegacy'
    >
  >(
    () => ({
      onPrefsChange: (p) => void viewCbRef.current!.onPrefsChange(p),
      onRemovePurify: (r) => void viewCbRef.current!.onRemovePurify(r),
      onLegacyStatus: () => viewCbRef.current!.onLegacyStatus(),
      onImportLegacy: () => viewCbRef.current!.onImportLegacy()
    }),
    []
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
      onBack: () => viewCbRef.current!.onBack(),
      onSelectChapter: (i) => viewCbRef.current!.onSelectChapter(i),
      onPrev: () => viewCbRef.current!.onPrev(),
      onNext: () => viewCbRef.current!.onNext(),
      onScrollSave: (t) => viewCbRef.current!.onScrollSave(t),
      onActiveChapter: (i) => viewCbRef.current!.onActiveChapter(i),
      onSettingsChange: (s) => viewCbRef.current!.onSettingsChange(s),
      onAddPurify: (t) => void viewCbRef.current!.onAddPurify(t),
      onReloadChapter: () => viewCbRef.current!.onReloadChapter()
    }),
    []
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
          sourceCount={enabledCount}
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


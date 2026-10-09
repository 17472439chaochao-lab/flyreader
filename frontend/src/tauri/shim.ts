import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import type {
  AppPrefs,
  AppState,
  BookCacheInfo,
  BookCacheProgress,
  BookInfo,
  BookSource,
  Chapter,
  OnlineFetchProgress,
  OnlineRepoRequest,
  RankBoard,
  RankChannel,
  RankGender,
  RankListResult,
  ReaderSettings,
  SearchBook,
  SearchProgress,
  ShelfBook,
  SourceRepoMeta,
  SourceTestResult
} from '@shared/types'
import type { UpdateCheckResult } from '@shared/updateCheck'

/** 书源批量测试进度事件载荷 */
export type SourceTestProgress = {
  done: number
  total: number
  current: string
  phase: 'start' | 'progress' | 'done'
  ok?: boolean
}

/** 后端事件订阅：返回同步的取消函数（渲染层的 runWithProgress 会立刻收集它） */
function onEvent<T>(event: string, cb: (payload: T) => void): () => void {
  let unlisten: UnlistenFn | null = null
  let disposed = false
  void listen<T>(event, (e) => cb(e.payload)).then((off) => {
    if (disposed) off()
    else unlisten = off
  })
  return () => {
    disposed = true
    unlisten?.()
  }
}

/** 书架书籍：后端在返回前统一附加运行时 cache 字段（对应原版 enrichShelf） */
type ShelfBookWithCache = ShelfBook & { cache?: BookCacheInfo }

/** 旧版 FlyNovel 数据位置与体量 */
export type LegacyDataStatus = {
  available: boolean
  path: string | null
  bytes: number
}

/** 旧版数据导入结果（各表合并行数） */
export type LegacyImportResult = {
  merged: {
    settings: number
    sources: number
    shelf: number
    history: number
    covers: number
    chapters: number
  }
  path: string
}

export const fly = {
  getState: () => invoke<AppState & { shelf: ShelfBookWithCache[] }>('app_get_state'),
  getVersion: () => invoke<string>('app_get_version'),
  checkUpdate: () => invoke<UpdateCheckResult>('app_check_update'),
  openExternal: (url: string) =>
    invoke<{ ok: boolean; message?: string }>('app_open_external', { url }),
  sources: {
    list: () => invoke<BookSource[]>('sources_list'),
    importFile: () =>
      invoke<{ ok: boolean; message: string; sources?: BookSource[] }>('sources_import_file'),
    importUrl: (url: string) =>
      invoke<{ ok: boolean; message: string; sources?: BookSource[] }>('sources_import_url', { url }),
    exportFile: (urls?: string[]) =>
      invoke<{ ok: boolean; message: string; count?: number; path?: string }>('sources_export_file', {
        urls
      }),
    repoList: () => invoke<SourceRepoMeta[]>('sources_repo_list'),
    fetchOnline: (repos?: OnlineRepoRequest[]) =>
      invoke<{
        ok: boolean
        message: string
        sources?: BookSource[]
        added?: number
        skipped?: number
      }>('sources_fetch_online', { repos }),
    onFetchOnlineProgress: (cb: (p: OnlineFetchProgress) => void) =>
      onEvent<OnlineFetchProgress>('sources_fetch_online_progress', cb),
    toggle: (url: string, enabled: boolean) =>
      invoke<BookSource[]>('sources_toggle', { url, enabled }),
    remove: (url: string) => invoke<BookSource[]>('sources_remove', { url }),
    removeMany: (urls: string[]) =>
      invoke<{ sources: BookSource[]; removed: number }>('sources_remove_many', { urls }),
    update: (oldUrl: string, source: BookSource) =>
      invoke<BookSource[]>('sources_update', { oldUrl, source }),
    test: (url: string, keyword?: string) =>
      invoke<{ result: SourceTestResult; sources: BookSource[] }>('sources_test', { url, keyword }),
    testAll: (keyword?: string, urls?: string[]) =>
      invoke<BookSource[]>('sources_test_all', { keyword, urls }),
    onTestProgress: (cb: (p: SourceTestProgress) => void) =>
      onEvent<SourceTestProgress>('sources_test_progress', cb),
    removeInvalid: () => invoke<{ sources: BookSource[]; removed: number }>('sources_remove_invalid')
  },
  books: {
    search: (keyword: string) =>
      invoke<{ books: SearchBook[]; history: string[]; sources?: BookSource[] }>('books_search', { keyword }),
    onSearchProgress: (cb: (p: SearchProgress) => void) =>
      onEvent<SearchProgress>('books_search_progress', cb),
    onSearchPartial: (cb: (books: SearchBook[]) => void) =>
      onEvent<SearchBook[]>('books_search_partial', cb),
    history: () => invoke<string[]>('search_history'),
    clearHistory: () => invoke<string[]>('search_clear_history'),
    removeHistory: (keyword: string) => invoke<string[]>('search_remove_history', { keyword }),
    info: (origin: string, bookUrl: string) =>
      invoke<BookInfo>('books_info', { origin, bookUrl }),
    toc: (origin: string, tocUrl: string) => invoke<Chapter[]>('books_toc', { origin, tocUrl }),
    content: (origin: string, chapterUrl: string, bookId?: string, refresh?: boolean) =>
      invoke<string>('books_content', {
        origin,
        chapterUrl,
        bookId,
        refresh: refresh ?? false
      }),
    cachedUrls: (bookId: string) => invoke<string[]>('books_cached_urls', { bookId }),
    preload: (origin: string, bookId: string, chapterUrls: string[]) =>
      invoke<string[]>('books_preload', { origin, bookId, chapterUrls }),
    cancelPreload: (bookId: string) => invoke<void>('books_cancel_preload', { bookId })
  },
  shelf: {
    list: () => invoke<ShelfBookWithCache[]>('shelf_list'),
    upsert: (book: ShelfBook) => invoke<ShelfBookWithCache[]>('shelf_upsert', { book }),
    patchProgress: (
      id: string,
      patch: { scrollTop?: number; chapterIndex?: number; lastReadAt?: number }
    ) => invoke<ShelfBook | null>('shelf_patch_progress', { id, patch }),
    remove: (id: string) => invoke<ShelfBook[]>('shelf_remove', { id }),
    importLocal: () =>
      invoke<{ ok: boolean; message: string; book?: ShelfBookWithCache; shelf?: ShelfBookWithCache[] }>(
        'shelf_import_local'
      ),
    updateAll: () => invoke<ShelfBookWithCache[]>('shelf_update_all'),
    cacheBook: (id: string) => invoke<ShelfBookWithCache[]>('shelf_cache_book', { id }),
    cancelCache: (id: string) => invoke<ShelfBookWithCache[]>('shelf_cancel_cache', { id }),
    exportTxt: (id: string) =>
      invoke<{ ok: boolean; message: string; path?: string; exported?: number; total?: number }>(
        'shelf_export_txt',
        { id }
      ),
    cacheStatus: (id: string) => invoke<BookCacheInfo | null>('shelf_cache_status', { id }),
    onUpdateProgress: (cb: (p: SourceTestProgress) => void) =>
      onEvent<SourceTestProgress>('shelf_update_progress', cb),
    onCacheProgress: (cb: (p: BookCacheProgress) => void) =>
      onEvent<BookCacheProgress>('shelf_cache_progress', cb)
  },
  settings: {
    get: () => invoke<ReaderSettings>('settings_get'),
    save: (settings: ReaderSettings) => invoke<ReaderSettings>('settings_save', { settings }),
    addPurify: (rule: string) => invoke<ReaderSettings>('settings_add_purify', { rule }),
    removePurify: (rule: string) => invoke<ReaderSettings>('settings_remove_purify', { rule })
  },
  fonts: {
    list: () => invoke<string[]>('fonts_list')
  },
  prefs: {
    get: () => invoke<AppPrefs>('prefs_get'),
    save: (patch: Partial<AppPrefs>) =>
      invoke<{ prefs: AppPrefs; bossKey: { ok: boolean; message: string } }>('prefs_save', { patch })
  },
  /** Tauri 版新增：旧版 Electron FlyNovel 数据继承（原 preload 无此通道） */
  data: {
    legacyStatus: () => invoke<LegacyDataStatus>('data_legacy_status'),
    importLegacy: () => invoke<LegacyImportResult>('data_import_legacy')
  },
  /** 起点移动站排行 / 完本 */
  rank: {
    qidianList: (args: {
      channel: RankChannel
      board?: RankBoard
      gender?: RankGender
      page?: number
      /** 起点分类 ID；-1 表示全站 */
      catId?: number
    }) => invoke<RankListResult>('rank_qidian_list', args)
  },
  /** 老板键显隐事件：由 Rust 侧全局快捷键回调推送 */
  onBossVisibility: (cb: (payload: { hidden: boolean }) => void) =>
    onEvent<{ hidden: boolean }>('app_boss_visibility', cb)
}

/** 暴露给渲染进程的 FlyReader API 类型（与原 preload FlyApi 对齐） */
export type FlyApi = typeof fly

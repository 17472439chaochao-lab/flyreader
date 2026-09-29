/** 书源搜索规则字段 */
export interface SearchRule {
  bookList?: string
  name?: string
  author?: string
  intro?: string
  kind?: string
  lastChapter?: string
  updateTime?: string
  bookUrl?: string
  coverUrl?: string
  wordCount?: string
  checkKeyWord?: string
}

/** 书籍详情页解析规则 */
export interface BookInfoRule {
  init?: string
  name?: string
  author?: string
  intro?: string
  kind?: string
  lastChapter?: string
  updateTime?: string
  coverUrl?: string
  tocUrl?: string
  wordCount?: string
  canReName?: string
  downloadUrls?: string
}

/** 目录解析规则 */
export interface TocRule {
  preUpdateJs?: string
  chapterList?: string
  chapterName?: string
  chapterUrl?: string
  isVolume?: string
  isVip?: string
  isPay?: string
  updateTime?: string
  nextTocUrl?: string
}

/** 正文解析规则 */
export interface ContentRule {
  content?: string
  title?: string
  nextContentUrl?: string
  webJs?: string
  sourceRegex?: string
  replaceRegex?: string
  imageStyle?: string
  payAction?: string
}

/** 书源测速标签 */
export type SourceSpeedTag = 'turbo' | 'excellent' | 'good' | 'slow'

/** Legado 风格书源定义（含应用本地测速/匹配元数据） */
export interface BookSource {
  bookSourceUrl: string
  bookSourceName: string
  bookSourceGroup?: string
  bookSourceType?: number
  bookUrlPattern?: string
  customOrder?: number
  enabled?: boolean
  enabledExplore?: boolean
  header?: string
  loginUrl?: string
  searchUrl?: string
  exploreUrl?: string
  ruleSearch?: SearchRule
  ruleBookInfo?: BookInfoRule
  ruleToc?: TocRule
  ruleContent?: ContentRule
  weight?: number
  lastUpdateTime?: number
  respondTime?: number
  /** 应用本地测试状态 */
  flyTestStatus?: 'ok' | 'fail' | 'untested'
  flyTestMessage?: string
  flyTestAt?: number
  flyRespondMs?: number
  /** 由测试响应时间自动得出的速度标签 */
  flySpeedTag?: SourceSpeedTag | null
  /**
   * 学习得到的搜索匹配质量 0～100（书名相关度的 EMA）。
   * 越高则搜索时越靠前，即使不是最快。
   */
  flyMatchScore?: number
  /** 参与计算 flyMatchScore 的搜索样本数 */
  flyMatchSamples?: number
}

/** 内置在线书源仓库元数据（面板展示与请求回传共用） */
export interface SourceRepoMeta {
  id: string
  name: string
  desc: string
  /**
   * json：响应直接是书源 JSON（数组或单个对象）；
   * sub：订阅列表（每行一个书源 JSON 文件的 URL，递归拉取）；
   * auto：先按 JSON 解析，失败再按订阅列表展开（自定义 URL 用）。
   */
  kind: 'json' | 'sub' | 'auto'
  url: string
  /** 备用地址：主地址失败时自动尝试 */
  alt?: string
}

/** 在线获取书源的仓库请求（前端勾选内置仓库或追加自定义 URL 后回传主进程） */
export interface OnlineRepoRequest {
  id: string
  name?: string
  kind?: 'json' | 'sub' | 'auto'
  url?: string
  alt?: string
}

/** 在线获取书源进度事件载荷 */
export interface OnlineFetchProgress {
  repoName: string
  done: number
  total: number
  /** 目前已累计命中的去重书源数 */
  found: number
  phase: 'start' | 'done'
  ok?: boolean
}

/** 速度标签选项（界面展示用） */
export const SOURCE_SPEED_TAG_OPTIONS: { id: SourceSpeedTag; label: string }[] = [
  { id: 'turbo', label: '极速' },
  { id: 'excellent', label: '优秀' },
  { id: 'good', label: '良好' },
  { id: 'slow', label: '慢' }
]

/**
 * 根据实测响应时间（毫秒）推导速度标签。
 * @param ms 响应毫秒数，可为空
 * @returns 速度标签；无效输入返回 null
 */
export function speedTagFromRespondMs(ms: number | null | undefined): SourceSpeedTag | null {
  if (ms == null || !Number.isFinite(ms) || ms < 0) return null
  if (ms < 500) return 'turbo'
  if (ms < 1500) return 'excellent'
  if (ms < 2000) return 'good'
  return 'slow'
}

/**
 * 读取书源当前有效速度标签（优先实测，其次已存标签）。
 * @param s 书源
 * @returns 速度标签或 null
 */
export function sourceSpeedTag(s: BookSource): SourceSpeedTag | null {
  return speedTagFromRespondMs(s.flyRespondMs ?? s.respondTime) || s.flySpeedTag || null
}

/**
 * 将书源速度标签转为中文展示文案。
 * @param s 书源
 * @returns 如「极速」「优秀」等；无标签则 null
 */
export function sourceSpeedLabel(s: BookSource): string | null {
  const tag = sourceSpeedTag(s)
  if (!tag) return null
  return SOURCE_SPEED_TAG_OPTIONS.find((t) => t.id === tag)?.label || null
}

/**
 * 仅按速度计算搜索排序权重，数值越小越优先。
 * @param s 书源
 * @returns 排序秩：极速 0 … 慢 4，未知为 3
 */
export function sourceSearchRank(s: BookSource): number {
  const tag = sourceSpeedTag(s)
  if (tag === 'turbo') return 0
  if (tag === 'excellent') return 1
  if (tag === 'good') return 2
  if (tag === 'slow') return 4
  return 3
}

/**
 * 有效匹配强度：学习分 + 书架使用加成。
 * @param s 书源
 * @param shelfCountByOrigin 可选，书源 URL → 书架上使用该源的在线书数量
 * @returns 0～100 的匹配强度
 */
export function sourceMatchStrength(
  s: BookSource,
  shelfCountByOrigin?: Map<string, number>
): number {
  const learned =
    typeof s.flyMatchScore === 'number' && Number.isFinite(s.flyMatchScore)
      ? Math.max(0, Math.min(100, s.flyMatchScore))
      : 0
  const n = shelfCountByOrigin?.get(s.bookSourceUrl) ?? 0
  // 书架使用是强信号——相对「仅快」更偏好已验证书源。
  // 1 本 ≈ 18，4 本 ≈ 72（上限 75）。
  const shelfBoost = Math.min(75, Math.max(0, n) * 18)
  return Math.min(100, learned + shelfBoost)
}

/**
 * 由书架上使用该源的书籍数量推导匹配基线分。
 * @param shelfCount 书架中来自该源的在线书数量
 * @returns 0～88 的基线分
 */
export function shelfMatchBaseline(shelfCount: number): number {
  const n = Math.max(0, Math.floor(shelfCount))
  if (n <= 0) return 0
  return Math.min(88, 40 + n * 12)
}

/**
 * 将匹配分数转为中文档位文案。
 * @param score 匹配分
 * @returns 「高匹配」「中匹配」「低匹配」或 null
 */
export function sourceMatchLabel(score: number | null | undefined): string | null {
  if (score == null || !Number.isFinite(score) || score <= 0) return null
  if (score >= 70) return '高匹配'
  if (score >= 40) return '中匹配'
  return '低匹配'
}

/**
 * 将匹配分数映射为档位枚举。
 * @param score 匹配分
 * @returns high / mid / low，无效则 null
 */
export function sourceMatchTier(score: number | null | undefined): 'high' | 'mid' | 'low' | null {
  if (score == null || !Number.isFinite(score) || score <= 0) return null
  if (score >= 70) return 'high'
  if (score >= 40) return 'mid'
  return 'low'
}

/**
 * 搜索书源排序比较器：先比匹配强度，再比速度标签/延迟。
 * @param a 书源 A
 * @param b 书源 B
 * @param shelfCountByOrigin 可选，bookSourceUrl → 书架在线书数量
 * @returns 负值表示 a 更靠前，正值表示 b 更靠前
 */
export function compareSourcesForSearch(
  a: BookSource,
  b: BookSource,
  shelfCountByOrigin?: Map<string, number>
): number {
  const sa = sourceMatchStrength(a, shelfCountByOrigin)
  const sb = sourceMatchStrength(b, shelfCountByOrigin)
  if (sa !== sb) return sb - sa

  const ra = sourceSearchRank(a)
  const rb = sourceSearchRank(b)
  if (ra !== rb) return ra - rb
  const ma = a.flyRespondMs ?? a.respondTime ?? Number.POSITIVE_INFINITY
  const mb = b.flyRespondMs ?? b.respondTime ?? Number.POSITIVE_INFINITY
  return ma - mb
}

/** 书源连通/搜索测试结果 */
export type SourceTestResult = {
  ok: boolean
  url: string
  name: string
  message: string
  found: number
  respondMs: number
  structuralOk: boolean
}

/** 搜索命中的书籍摘要 */
export interface SearchBook {
  name: string
  author: string
  bookUrl: string
  coverUrl?: string
  intro?: string
  kind?: string
  lastChapter?: string
  wordCount?: string
  origin: string
  originName: string
}

/** 书籍详情（在搜索结果基础上可含目录 URL） */
export interface BookInfo extends SearchBook {
  tocUrl?: string
}

/** 章节项 */
export interface Chapter {
  title: string
  url: string
  index: number
  isVolume?: boolean
  /** Markdown 文档标题锚点，选中目录时滚动到对应标题 */
  mdAnchor?: string
}

/** 书籍缓存状态 */
export type BookCacheStatus = 'none' | 'partial' | 'full' | 'caching'

/** 书架书籍的缓存统计信息 */
export interface BookCacheInfo {
  bookId: string
  total: number
  cached: number
  status: BookCacheStatus
}

/** 缓存进度事件载荷 */
export type BookCacheProgress = {
  bookId: string
  done: number
  total: number
  current: string
  phase: 'start' | 'progress' | 'done' | 'cancelled'
  ok?: boolean
  cached?: number
}

/** 书架上的一本书（含阅读进度与可选本地文件信息） */
export interface ShelfBook {
  id: string
  name: string
  author: string
  bookUrl: string
  coverUrl?: string
  intro?: string
  kind?: string
  lastChapter?: string
  origin: string
  originName: string
  tocUrl?: string
  chapters?: Chapter[]
  chapterIndex: number
  scrollTop: number
  addedAt: number
  updatedAt: number
  /** 用户最近打开/阅读时间（毫秒），用于书架排序 */
  lastReadAt?: number
  /** 本地文件书（txt / epub / md）；无换源与在线缓存 */
  isLocal?: boolean
  localPath?: string
  localFormat?: 'txt' | 'epub' | 'md'
  /** 主进程列出书架时填充；不落库 */
  cache?: BookCacheInfo
}

/** 阅读器外观与净化设置 */
export interface ReaderSettings {
  fontSize: number
  lineHeight: number
  pageWidth: number
  theme: 'paper' | 'night' | 'green'
  fontFamily: string
  purifyRules: string[]
}

/** 窗口位置与尺寸 */
export interface WindowBounds {
  x?: number
  y?: number
  width: number
  height: number
  isMaximized?: boolean
}

/** 主界面视图名 */
export type ViewName = 'shelf' | 'search' | 'sources' | 'reader' | 'settings' | 'about'

/** 书架排序方式 */
export type ShelfSort = 'lastRead' | 'added'

/** 应用级偏好（持久化于 SQLite settings.key = 'app'） */
export interface AppPrefs {
  sourceTestKeyword: string
  lastView: Exclude<ViewName, 'reader'>
  windowBounds: WindowBounds
  /** 全局热键显示/隐藏应用（Electron accelerator） */
  bossKeyEnabled: boolean
  bossKey: string
  /** 阅读时预取后续章节数 */
  preloadCount: number
  /**
   * 最大并行网络任务数（搜索 / 测源 / 缓存 / 书架更新 / 导出）。
   * 钳制在 REQUEST_CONCURRENCY_MIN..MAX。
   */
  requestConcurrency: number
  /** 书架列表排序（均为降序）。默认：最近阅读 */
  shelfSort: ShelfSort
  /** 是否启用护眼 / 久坐提醒（仅阅读页累计时长） */
  eyeCareEnabled: boolean
  /** 连续阅读多久后提醒（分钟） */
  eyeCareIntervalMinutes: number
}

/** 护眼提醒可选间隔（分钟） */
export const EYE_CARE_INTERVAL_OPTIONS = [
  { value: 30, label: '30 分钟' },
  { value: 60, label: '1 小时' },
  { value: 120, label: '2 小时' },
  { value: 300, label: '5 小时' },
  { value: 600, label: '10 小时' }
] as const

/**
 * 将护眼间隔钳制为可选分钟数之一。
 * @param value 原始分钟数
 */
export function clampEyeCareIntervalMinutes(value: unknown): number {
  const n = typeof value === 'number' && Number.isFinite(value) ? Math.round(value) : 120
  const allowed: number[] = EYE_CARE_INTERVAL_OPTIONS.map((o) => o.value)
  if (allowed.includes(n)) return n
  let best = allowed[0]
  let bestDist = Math.abs(n - best)
  for (const v of allowed) {
    const d = Math.abs(n - v)
    if (d < bestDist) {
      best = v
      bestDist = d
    }
  }
  return best
}

/**
 * 护眼间隔对应的毫秒数。
 * @param minutes 偏好中的分钟数
 */
export function eyeCareIntervalMs(minutes: number): number {
  return Math.max(1, minutes) * 60 * 1000
}

/** 全局请求并发安全上下限（避免站点限流） */
export const REQUEST_CONCURRENCY_MIN = 1
export const REQUEST_CONCURRENCY_MAX = 16
export const REQUEST_CONCURRENCY_DEFAULT = 8

/**
 * 将任意输入钳制为合法请求并发数。
 * @param value 原始值
 * @returns REQUEST_CONCURRENCY_MIN～MAX 之间的整数
 */
export function clampRequestConcurrency(value: unknown): number {
  const n = typeof value === 'number' && Number.isFinite(value) ? Math.round(value) : REQUEST_CONCURRENCY_DEFAULT
  return Math.max(REQUEST_CONCURRENCY_MIN, Math.min(REQUEST_CONCURRENCY_MAX, n))
}

/** 应用内存态：书源、书架、阅读设置与偏好 */
export interface AppState {
  sources: BookSource[]
  shelf: ShelfBook[]
  settings: ReaderSettings
  prefs: AppPrefs
}

/** 默认阅读器设置 */
export const DEFAULT_SETTINGS: ReaderSettings = {
  fontSize: 18,
  lineHeight: 1.8,
  pageWidth: 720,
  theme: 'paper',
  /** 首次启动/迁移时填入平台默认字体 */
  fontFamily: '',
  purifyRules: []
}

/** 默认应用偏好 */
export const DEFAULT_PREFS: AppPrefs = {
  sourceTestKeyword: '剑来',
  lastView: 'shelf',
  windowBounds: { width: 1100, height: 740 },
  bossKeyEnabled: true,
  bossKey: 'CommandOrControl+Shift+H',
  preloadCount: 3,
  requestConcurrency: REQUEST_CONCURRENCY_DEFAULT,
  shelfSort: 'lastRead',
  eyeCareEnabled: true,
  eyeCareIntervalMinutes: 120
}

/** 多源搜索进度事件 */
export type SearchProgress = {
  done: number
  total: number
  found: number
  current: string
  phase: 'start' | 'progress' | 'done'
}

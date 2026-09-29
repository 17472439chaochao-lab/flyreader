import { memo, useEffect, useMemo, useState } from 'react'
import { isLocalBook } from '@shared/bookLocal'
import { isSourceStructuralOk } from '@shared/sourceValidity'
import {
  sourceMatchLabel,
  sourceMatchStrength,
  sourceMatchTier,
  sourceSpeedLabel,
  sourceSpeedTag,
  type BookSource,
  type OnlineFetchProgress,
  type OnlineRepoRequest,
  type ShelfBook,
  type SourceRepoMeta
} from '@shared/types'
import {
  IconBroom,
  IconCheck,
  IconClose,
  IconDownload,
  IconEdit,
  IconExport,
  IconImport,
  IconLink,
  IconSelect,
  IconTest,
  IconTrash,
  LoadingIcon,
  SearchIcon
} from '../components/icons'
import { VirtualList } from '../components/VirtualList'
import { formatProgressLabel, runWithProgress } from '../utils/progress'

/**
 * 书源管理页：搜索/筛选/统计书源，支持文件与 URL 导入、
 * 启用切换、单测/批量测试、编辑 JSON、多选批量删除、删除无效源等。
 */
export const SourcesView = memo(function SourcesView({
  sources,
  shelf,
  testKeyword,
  onTestKeywordChange,
  onTestKeywordCommit,
  askConfirm,
  showToast,
  onImportFile,
  onSourcesChange
}: {
  sources: BookSource[]
  shelf: ShelfBook[]
  testKeyword: string
  onTestKeywordChange: (v: string) => void
  onTestKeywordCommit: (v: string) => void
  askConfirm: (opts: {
    title: string
    message: string
    confirmText?: string
    cancelText?: string
    extraText?: string
    danger?: boolean
  }) => Promise<'confirm' | 'cancel' | 'extra'>
  showToast: (msg: string) => void
  onImportFile: () => void
  onSourcesChange: (s: BookSource[]) => void
}) {
  const [testingUrl, setTestingUrl] = useState<string | null>(null)
  const [testingAll, setTestingAll] = useState(false)
  const [testProgress, setTestProgress] = useState('')
  const [editing, setEditing] = useState<BookSource | null>(null)
  const [editName, setEditName] = useState('')
  const [editText, setEditText] = useState('')
  const [importUrl, setImportUrl] = useState('')
  const [importingUrl, setImportingUrl] = useState(false)
  const [onlineOpen, setOnlineOpen] = useState(false)
  const [onlineRepos, setOnlineRepos] = useState<SourceRepoMeta[]>([])
  const [selectedRepoIds, setSelectedRepoIds] = useState<string[]>([])
  const [customUrl, setCustomUrl] = useState('')
  const [onlineFetching, setOnlineFetching] = useState(false)
  const [onlineProgress, setOnlineProgress] = useState<OnlineFetchProgress | null>(null)
  const [filter, setFilter] = useState<
    'all' | 'enabled' | 'disabled' | 'ok' | 'fail' | 'structuralBad' | 'untested'
  >('all')
  /** 书源搜索关键词（匹配名称 / 地址 / 分组 / 测试信息） */
  const [query, setQuery] = useState('')
  /** 是否处于多选模式 */
  const [selectMode, setSelectMode] = useState(false)
  /** 已勾选的书源 URL */
  const [picked, setPicked] = useState<string[]>([])

  const pickedSet = useMemo(() => new Set(picked), [picked])

  /** 书源被删除 / 导入后清理已失效的勾选项 */
  useEffect(() => {
    setPicked((prev) => {
      if (!prev.length) return prev
      const alive = new Set(sources.map((s) => s.bookSourceUrl))
      const next = prev.filter((u) => alive.has(u))
      return next.length === prev.length ? prev : next
    })
  }, [sources])

  const shelfCountByOrigin = useMemo(() => {
    const map = new Map<string, number>()
    for (const b of shelf) {
      if (isLocalBook(b) || !b.origin) continue
      map.set(b.origin, (map.get(b.origin) || 0) + 1)
    }
    return map
  }, [shelf])

  const stats = useMemo(() => {
    let total = 0
    let enabled = 0
    let ok = 0
    let fail = 0
    let structuralBad = 0
    let invalid = 0
    let untested = 0
    for (const s of sources) {
      total += 1
      if (s.enabled !== false) enabled += 1
      if (s.flyTestStatus === 'ok') ok += 1
      else if (s.flyTestStatus === 'fail') fail += 1
      if (!isSourceStructuralOk(s)) {
        structuralBad += 1
        invalid += 1
      } else if (s.flyTestStatus === 'fail') {
        invalid += 1
      }
      if (!s.flyTestStatus || s.flyTestStatus === 'untested') untested += 1
    }
    return { total, enabled, disabled: total - enabled, ok, fail, structuralBad, invalid, untested }
  }, [sources])

  const filteredSources = useMemo(() => {
    const keyword = query.trim().toLowerCase()
    const list = sources.filter((s) => {
      if (keyword) {
        const hay = [s.bookSourceName, s.bookSourceUrl, s.bookSourceGroup, s.flyTestMessage]
          .filter(Boolean)
          .join('\n')
          .toLowerCase()
        if (!hay.includes(keyword)) return false
      }
      switch (filter) {
        case 'enabled':
          return s.enabled !== false
        case 'disabled':
          return s.enabled === false
        case 'ok':
          return s.flyTestStatus === 'ok'
        case 'fail':
          return s.flyTestStatus === 'fail'
        case 'structuralBad':
          return !isSourceStructuralOk(s)
        case 'untested':
          return !s.flyTestStatus || s.flyTestStatus === 'untested'
        default:
          return true
      }
    })
    return list.slice().sort((a, b) => {
      const d =
        sourceMatchStrength(b, shelfCountByOrigin) - sourceMatchStrength(a, shelfCountByOrigin)
      if (d !== 0) return d
      return (a.bookSourceName || '').localeCompare(b.bookSourceName || '', 'zh')
    })
  }, [sources, filter, shelfCountByOrigin, query])

  /** 当前显示（搜索 + 分类筛选后）的书源 URL，作为「全选」的作用范围 */
  const visibleUrls = useMemo(
    () => filteredSources.map((s) => s.bookSourceUrl),
    [filteredSources]
  )
  const allVisiblePicked = visibleUrls.length > 0 && visibleUrls.every((u) => pickedSet.has(u))

  /**
   * 切换书源启用状态
   * @param url 书源 URL
   * @param enabled 目标启用状态
   */
  async function onToggle(url: string, enabled: boolean) {
    onSourcesChange(await window.fly.sources.toggle(url, enabled))
  }

  /** 通过订阅 URL 导入书源 */
  async function onImportFromUrl() {
    if (!importUrl.trim() || importingUrl) return
    setImportingUrl(true)
    try {
      const res = await window.fly.sources.importUrl(importUrl.trim())
      showToast(res.message)
      if (res.ok && res.sources) {
        onSourcesChange(res.sources)
        if (res.message.includes('新增')) setImportUrl('')
      }
    } catch (e) {
      showToast(`URL 导入失败：${(e as Error).message}`)
    } finally {
      setImportingUrl(false)
    }
  }

  /** 打开在线获取面板：首次拉取内置仓库列表并默认全选 */
  async function openOnlinePanel() {
    setOnlineProgress(null)
    setOnlineOpen(true)
    if (onlineRepos.length) return
    try {
      const repos = await window.fly.sources.repoList()
      setOnlineRepos(repos)
      setSelectedRepoIds(repos.map((r) => r.id))
    } catch {
      /* 仓库列表拉取失败时仍可打开面板，用自定义 URL 获取 */
    }
  }

  /** 勾选或取消勾选内置仓库 */
  function toggleRepo(id: string) {
    setSelectedRepoIds((prev) =>
      prev.includes(id) ? prev.filter((x) => x !== id) : [...prev, id]
    )
  }

  /** 把自定义 URL 追加为 auto 类型仓库并默认选中 */
  function addCustomRepo() {
    const url = customUrl.trim()
    if (!/^https?:\/\//i.test(url)) {
      showToast('请输入以 http:// 或 https:// 开头的书源地址')
      return
    }
    if (onlineRepos.some((r) => r.url === url)) {
      showToast('该地址已在列表中')
      return
    }
    let host = 'URL'
    try {
      host = new URL(url).host || 'URL'
    } catch {
      /* 保持默认 */
    }
    const repo: SourceRepoMeta = {
      id: `custom-${onlineRepos.length + 1}`,
      name: `自定义源 ${host}`,
      desc: '用户自定义书源地址（自动识别 JSON 或订阅列表）',
      kind: 'auto',
      url
    }
    setOnlineRepos((prev) => [...prev, repo])
    setSelectedRepoIds((prev) => [...prev, repo.id])
    setCustomUrl('')
  }

  /** 从选中的仓库在线获取并自动导入书源 */
  async function onFetchOnline() {
    if (onlineFetching) return
    const requests: OnlineRepoRequest[] = onlineRepos
      .filter((r) => selectedRepoIds.includes(r.id))
      .map((r) => ({ id: r.id, name: r.name, kind: r.kind, url: r.url, alt: r.alt }))
    if (!requests.length) {
      showToast('请至少选择一个书源仓库')
      return
    }
    setOnlineFetching(true)
    setOnlineProgress(null)
    try {
      const res = await runWithProgress(
        (track) => {
          track(window.fly.sources.onFetchOnlineProgress((p) => setOnlineProgress(p)))
        },
        () => window.fly.sources.fetchOnline(requests)
      )
      if (res.sources) onSourcesChange(res.sources)
      showToast(res.message)
      if (res.ok) setOnlineOpen(false)
    } catch (e) {
      showToast(`在线获取失败：${(e as Error).message}`)
    } finally {
      setOnlineFetching(false)
      setOnlineProgress(null)
    }
  }

  /**
   * 确认后删除单个书源
   * @param url 书源 URL
   */
  async function onRemove(url: string) {
    const target = sources.find((s) => s.bookSourceUrl === url)
    const name = target?.bookSourceName || url
    const result = await askConfirm({
      title: '删除书源',
      message: `确定删除书源「${name}」？删除后不可恢复。`,
      confirmText: '删除',
      danger: true
    })
    if (result !== 'confirm') return
    onSourcesChange(await window.fly.sources.remove(url))
    showToast('已删除书源')
  }

  /**
   * 测试单个书源可用性
   * @param url 书源 URL
   */
  async function onTestOne(url: string) {
    setTestingUrl(url)
    try {
      const { result, sources: next } = await window.fly.sources.test(url, testKeyword)
      onSourcesChange(next)
      showToast(result.ok ? `可用：${result.message}` : `失效：${result.message}`)
    } catch (e) {
      showToast(`测试失败：${(e as Error).message}`)
    } finally {
      setTestingUrl(null)
    }
  }

  /** 批量测试当前筛选条件下的全部书源 */
  async function onTestCurrent() {
    if (!filteredSources.length || testingAll) return
    const urls = filteredSources.map((s) => s.bookSourceUrl)
    const filterLabel = filterItems.find((f) => f.id === filter)?.label || '当前'
    setTestingAll(true)
    setTestProgress(`0/${urls.length}`)
    try {
      const next = await runWithProgress(
        (track) => {
          track(window.fly.sources.onTestProgress((p) => setTestProgress(formatProgressLabel(p))))
        },
        () => window.fly.sources.testAll(testKeyword, urls)
      )
      onSourcesChange(next)
      const tested = next.filter((s) => urls.includes(s.bookSourceUrl))
      const ok = tested.filter((s) => s.flyTestStatus === 'ok').length
      const fail = tested.filter((s) => s.flyTestStatus === 'fail').length
      showToast(`「${filterLabel}」测试完成：可用 ${ok}，失效 ${fail}`)
    } catch (e) {
      showToast(`批量测试失败：${(e as Error).message}`)
    } finally {
      setTestingAll(false)
      setTestProgress('')
    }
  }

  /** 确认后删除规则残缺或测试失败的无效书源 */
  async function onRemoveInvalid() {
    if (!stats.invalid) {
      showToast('当前没有可清理的无效书源（需先测试，或存在规则残缺）')
      return
    }
    const result = await askConfirm({
      title: '删除无效书源',
      message: `将删除 ${stats.invalid} 个无效书源（规则残缺或测试失败），此操作不可撤销。`,
      confirmText: '全部删除',
      danger: true
    })
    if (result !== 'confirm') return
    const { sources: next, removed } = await window.fly.sources.removeInvalid()
    onSourcesChange(next)
    showToast(removed ? `已删除 ${removed} 个无效书源` : '没有删除任何书源')
  }

  /** 进入 / 退出多选模式（退出时清空已勾选） */
  function toggleSelectMode() {
    if (selectMode) setPicked([])
    setSelectMode(!selectMode)
  }

  /**
   * 勾选或取消勾选单个书源
   * @param url 书源 URL
   */
  function togglePick(url: string) {
    setPicked((prev) => (prev.includes(url) ? prev.filter((u) => u !== url) : [...prev, url]))
  }

  /** 全选 / 取消全选「当前显示」的书源（不影响已勾选的未显示项） */
  function togglePickAll() {
    const urls = visibleUrls
    setPicked((prev) => {
      if (urls.length && urls.every((u) => prev.includes(u))) {
        const drop = new Set(urls)
        return prev.filter((u) => !drop.has(u))
      }
      const next = new Set(prev)
      urls.forEach((u) => next.add(u))
      return Array.from(next)
    })
  }

  /** 确认后批量删除已勾选的书源 */
  async function onRemovePicked() {
    const urls = picked
    if (!urls.length) {
      showToast('请先勾选要删除的书源')
      return
    }
    const result = await askConfirm({
      title: '批量删除书源',
      message: `确定删除勾选的 ${urls.length} 个书源？删除后不可恢复。`,
      confirmText: `删除 ${urls.length} 个`,
      danger: true
    })
    if (result !== 'confirm') return
    try {
      const { sources: next, removed } = await window.fly.sources.removeMany(urls)
      onSourcesChange(next)
      setPicked([])
      showToast(removed ? `已删除 ${removed} 个书源` : '没有删除任何书源')
    } catch (e) {
      showToast(`批量删除失败：${(e as Error).message}`)
    }
  }

  /** 将当前筛选下的书源导出为 JSON 文件 */
  async function onExportCurrent() {
    if (!filteredSources.length) {
      showToast('当前没有可导出的书源')
      return
    }
    const res = await window.fly.sources.exportFile(filteredSources.map((s) => s.bookSourceUrl))
    if (res.message !== '已取消') showToast(res.message)
  }

  /**
   * 打开书源编辑弹窗并填充名称与 JSON
   * @param s 待编辑书源
   */
  function openEdit(s: BookSource) {
    setEditing(s)
    setEditName(s.bookSourceName || '')
    const { bookSourceName: _name, ...rest } = s
    setEditText(JSON.stringify(rest, null, 2))
  }

  /** 校验并保存编辑中的书源 JSON */
  async function saveEdit() {
    if (!editing) return
    const name = editName.trim()
    if (!name) {
      showToast('书源名称不能为空')
      return
    }
    try {
      const parsed = JSON.parse(editText) as BookSource
      parsed.bookSourceName = name
      if (!parsed.bookSourceUrl?.trim()) {
        showToast('书源 URL 不能为空')
        return
      }
      const next = await window.fly.sources.update(editing.bookSourceUrl, parsed)
      onSourcesChange(next)
      setEditing(null)
      showToast('书源已保存')
    } catch (e) {
      showToast(`保存失败：${(e as Error).message}`)
    }
  }

  /**
   * 根据结构完整性与测试状态生成状态徽章文案与样式类
   * @param s 书源对象
   */
  function statusLabel(s: BookSource) {
    if (!isSourceStructuralOk(s)) {
      return { text: '规则残缺', cls: 'badge bad' }
    }
    if (s.flyTestStatus === 'ok') return { text: '可用', cls: 'badge ok' }
    if (s.flyTestStatus === 'fail') return { text: '失效', cls: 'badge bad' }
    return { text: '未测试', cls: 'badge mute' }
  }

  const filterItems = [
    { id: 'all' as const, label: '全部', count: stats.total },
    { id: 'enabled' as const, label: '启用', count: stats.enabled },
    { id: 'disabled' as const, label: '禁用', count: stats.disabled },
    { id: 'ok' as const, label: '可用', count: stats.ok },
    { id: 'fail' as const, label: '失效', count: stats.fail },
    { id: 'structuralBad' as const, label: '残缺', count: stats.structuralBad },
    { id: 'untested' as const, label: '未测', count: stats.untested }
  ]

  return (
    <div>
      <div className="panel-head">
        <div>
          <h2>书源管理</h2>
          <p>匹配度高的书源会优先参与搜索（含书架常用源），不单看速度标签</p>
        </div>
        <div className="panel-actions">
          <button
            className={selectMode ? 'btn' : 'btn ghost'}
            onClick={toggleSelectMode}
            title={selectMode ? '退出多选模式' : '进入多选模式，可批量删除书源'}
          >
            <IconSelect />
            {selectMode ? '退出选择' : '选择'}
          </button>
          <button className="btn" onClick={() => void openOnlinePanel()}>
            <IconDownload />
            在线获取
          </button>
          <button className="btn ghost" onClick={onImportFile}>
            <IconImport />
            从文件导入
          </button>
        </div>
      </div>

      <div className="stats-bar" role="tablist" aria-label="书源筛选">
        {filterItems.map((item) => (
          <button
            key={item.id}
            type="button"
            role="tab"
            aria-selected={filter === item.id}
            className={`stats-chip${filter === item.id ? ' active' : ''}`}
            onClick={() => setFilter(item.id)}
          >
            {item.label} <b>{item.count}</b>
          </button>
        ))}
      </div>

      <div className="source-search-row">
        <SearchIcon />
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="搜索书源名称 / 地址 / 分组"
          aria-label="搜索书源"
          onKeyDown={(e) => {
            if (e.key === 'Escape') setQuery('')
          }}
        />
        {query ? (
          <>
            <span className="search-count">
              {filteredSources.length
                ? `匹配 ${filteredSources.length}`
                : '无匹配'}
            </span>
            <button
              type="button"
              className="btn ghost icon-btn"
              title="清除搜索"
              onClick={() => setQuery('')}
            >
              <IconClose />
            </button>
          </>
        ) : null}
      </div>

      <div className="source-toolbar">
        <input
          className="test-keyword"
          value={testKeyword}
          onChange={(e) => onTestKeywordChange(e.target.value)}
          onBlur={(e) => onTestKeywordCommit(e.target.value)}
          placeholder="测试关键词"
          disabled={testingAll || !!testingUrl}
        />
        <button
          className="btn ghost"
          disabled={!filteredSources.length || testingAll}
          title="测试当前顶部筛选分类中的书源"
          onClick={() => void onTestCurrent()}
        >
          {testingAll ? (
            <>
              <LoadingIcon /> 测试中 {testProgress}
            </>
          ) : (
            <>
              <IconTest />
              测试当前（{filteredSources.length}）
            </>
          )}
        </button>
        <button
          className="btn ghost"
          disabled={!stats.invalid || testingAll}
          onClick={() => void onRemoveInvalid()}
        >
          <IconBroom />
          删除无效（{stats.invalid}）
        </button>
        <button
          className="btn ghost"
          disabled={!filteredSources.length || testingAll}
          title="导出当前顶部筛选分类中的书源为 JSON"
          onClick={() => void onExportCurrent()}
        >
          <IconExport />
          导出（{filteredSources.length}）
        </button>
      </div>

      <div className="import-url-row">
        <input
          value={importUrl}
          onChange={(e) => setImportUrl(e.target.value)}
          placeholder="书源订阅 URL，例如 https://example.com/sources.json"
          disabled={importingUrl}
          onKeyDown={(e) => {
            if (e.key === 'Enter') void onImportFromUrl()
          }}
        />
        <button
          className="btn ghost"
          disabled={!importUrl.trim() || importingUrl}
          onClick={() => void onImportFromUrl()}
        >
          {importingUrl ? <LoadingIcon /> : <IconLink />}
          {importingUrl ? '导入中' : '从 URL 导入'}
        </button>
      </div>

      {selectMode ? (
        <div className="source-select-bar" role="group" aria-label="书源批量操作">
          <span className="select-summary">
            已选 <b>{picked.length}</b> 个 · 当前显示 <b>{filteredSources.length}</b> 个
          </span>
          <button
            type="button"
            className="btn ghost"
            disabled={!visibleUrls.length}
            onClick={togglePickAll}
          >
            <IconCheck />
            {allVisiblePicked ? '取消全选' : '全选显示'}
          </button>
          <button
            type="button"
            className="btn danger"
            disabled={!picked.length}
            onClick={() => void onRemovePicked()}
          >
            <IconTrash />
            删除选中（{picked.length}）
          </button>
          <button type="button" className="btn ghost" onClick={toggleSelectMode}>
            <IconClose />
            退出选择
          </button>
        </div>
      ) : null}

      {!sources.length ? (
        <div className="empty-guide">
          <h3>还没有书源</h3>
          <p>
            书源是搜索小说的「数据源」。可以直接从公开维护的书源仓库在线获取，
            也可以导入自己收藏的书源文件（兼容 Legado 格式）。
          </p>
          <div className="empty-actions">
            <button className="btn" onClick={() => void openOnlinePanel()}>
              <IconDownload />
              在线获取书源
            </button>
            <button className="btn ghost" onClick={onImportFile}>
              <IconImport />
              从文件导入
            </button>
          </div>
        </div>
      ) : !filteredSources.length ? (
        <div className="empty">
          {query.trim() ? `没有匹配「${query.trim()}」的书源` : '当前筛选下没有书源'}
        </div>
      ) : (
        <VirtualList
          className="source-list source-virtual"
          style={{ height: 560, width: '100%' }}
          count={filteredSources.length}
          estimateSize={92}
          gap={8}
          overscan={6}
          renderItem={(i) => {
            const s = filteredSources[i]
            const st = statusLabel(s)
            const busy = testingUrl === s.bookSourceUrl
            const speedTag = sourceSpeedTag(s)
            const speedLabel = sourceSpeedLabel(s)
            const matchScore = sourceMatchStrength(s, shelfCountByOrigin)
            const matchTier = sourceMatchTier(matchScore)
            const matchLabel = sourceMatchLabel(matchScore)
            const isPicked = pickedSet.has(s.bookSourceUrl)
            return (
              <div
                className={`source-row${selectMode ? ' selectable' : ''}${
                  isPicked ? ' picked' : ''
                }`}
                key={s.bookSourceUrl}
                onClick={selectMode ? () => togglePick(s.bookSourceUrl) : undefined}
                onKeyDown={
                  selectMode
                    ? (e) => {
                        if (e.key === 'Enter' || e.key === ' ') {
                          e.preventDefault()
                          togglePick(s.bookSourceUrl)
                        }
                      }
                    : undefined
                }
                role={selectMode ? 'checkbox' : undefined}
                aria-checked={selectMode ? isPicked : undefined}
                aria-label={selectMode ? s.bookSourceName : undefined}
                tabIndex={selectMode ? 0 : undefined}
              >
                {/* 占位列在两种模式下保持同宽，避免虚拟列表行高变化导致重叠 */}
                <span className="source-pick">
                  {selectMode ? (
                    <input
                      type="checkbox"
                      checked={isPicked}
                      onClick={(e) => e.stopPropagation()}
                      onChange={() => togglePick(s.bookSourceUrl)}
                      aria-label={`选择 ${s.bookSourceName}`}
                    />
                  ) : null}
                </span>
                <div className="info">
                  <strong>
                    {s.bookSourceName} <span className={st.cls}>{st.text}</span>
                    {speedLabel && speedTag ? (
                      <span className={`badge speed-${speedTag}`} title="根据测试响应速度自动标记">
                        {speedLabel}
                      </span>
                    ) : null}
                    {matchLabel && matchTier ? (
                      <span
                        className={`badge match-${matchTier}`}
                        title={`搜索匹配度 ${Math.round(matchScore)}（含书架使用加权），越高搜索越优先`}
                      >
                        {matchLabel}
                        {matchScore >= 1 ? ` ${Math.round(matchScore)}` : ''}
                      </span>
                    ) : null}
                  </strong>
                  <span>
                    {s.bookSourceGroup ? `${s.bookSourceGroup} · ` : ''}
                    {s.bookSourceUrl}
                  </span>
                  {s.flyTestMessage ? (
                    <span className="test-msg">
                      {s.flyRespondMs != null ? `${s.flyRespondMs}ms · ` : ''}
                      {s.flyTestMessage}
                    </span>
                  ) : null}
                </div>
                <div className="row-actions">
                  <button
                    className={`switch ${s.enabled !== false ? 'on' : ''}`}
                    onClick={() => void onToggle(s.bookSourceUrl, s.enabled === false)}
                    aria-label="启用"
                    disabled={testingAll || selectMode}
                  >
                    <i />
                  </button>
                  <button
                    className="btn ghost"
                    disabled={testingAll || busy || selectMode}
                    onClick={() => void onTestOne(s.bookSourceUrl)}
                  >
                    {busy ? <LoadingIcon /> : <IconTest />}
                    {busy ? '测试中' : '测试'}
                  </button>
                  <button
                    className="btn ghost"
                    disabled={testingAll || selectMode}
                    onClick={() => openEdit(s)}
                  >
                    <IconEdit />
                    编辑
                  </button>
                  <button
                    className="btn ghost"
                    disabled={testingAll || selectMode}
                    onClick={() => void onRemove(s.bookSourceUrl)}
                  >
                    <IconTrash />
                    删除
                  </button>
                </div>
              </div>
            )
          }}
        />
      )}

      {onlineOpen ? (
        <div className="modal-backdrop" onClick={() => !onlineFetching && setOnlineOpen(false)}>
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-head">
              <h3>在线获取书源</h3>
              <button
                className="btn ghost icon-btn"
                title="关闭"
                disabled={onlineFetching}
                onClick={() => setOnlineOpen(false)}
              >
                <IconClose />
              </button>
            </div>

            <div className="repo-list">
              {onlineRepos.length ? (
                onlineRepos.map((r) => (
                  <label
                    className={`repo-row${selectedRepoIds.includes(r.id) ? ' on' : ''}`}
                    key={r.id}
                  >
                    <input
                      type="checkbox"
                      checked={selectedRepoIds.includes(r.id)}
                      onChange={() => toggleRepo(r.id)}
                      disabled={onlineFetching}
                    />
                    <span className="repo-info">
                      <strong>{r.name}</strong>
                      <small>{r.desc}</small>
                      <code>{r.url}</code>
                    </span>
                  </label>
                ))
              ) : (
                <div className="repo-empty">内置仓库列表加载失败，可添加自定义地址获取</div>
              )}
            </div>

            <div className="import-url-row">
              <input
                value={customUrl}
                onChange={(e) => setCustomUrl(e.target.value)}
                placeholder="自定义仓库 URL（书源 JSON 或订阅列表）"
                disabled={onlineFetching}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') addCustomRepo()
                }}
              />
              <button
                className="btn ghost"
                disabled={!customUrl.trim() || onlineFetching}
                onClick={addCustomRepo}
              >
                <IconLink />
                添加
              </button>
            </div>

            {onlineProgress ? (
              <div className="online-progress">
                <LoadingIcon />
                <span>
                  {onlineProgress.phase === 'start'
                    ? `正在获取「${onlineProgress.repoName}」…`
                    : onlineProgress.ok
                      ? `「${onlineProgress.repoName}」完成`
                      : `「${onlineProgress.repoName}」获取失败`}
                  （{onlineProgress.done}/{onlineProgress.total}，已命中 {onlineProgress.found} 个）
                </span>
              </div>
            ) : null}

            <div className="modal-actions">
              <button
                className="btn ghost"
                disabled={onlineFetching}
                onClick={() => setOnlineOpen(false)}
              >
                <IconClose />
                取消
              </button>
              <button
                className="btn"
                disabled={onlineFetching || !selectedRepoIds.length}
                onClick={() => void onFetchOnline()}
              >
                {onlineFetching ? <LoadingIcon /> : <IconDownload />}
                {onlineFetching ? '获取中…' : `获取并导入（${selectedRepoIds.length}）`}
              </button>
            </div>
          </div>
        </div>
      ) : null}

      {editing ? (
        <div className="modal-backdrop" onClick={() => setEditing(null)}>
          <div className="modal" onClick={(e) => e.stopPropagation()}>
            <div className="modal-head">
              <h3>编辑书源</h3>
              <button className="btn ghost icon-btn" title="关闭" onClick={() => setEditing(null)}>
                <IconClose />
              </button>
            </div>
            <div className="edit-name-field">
              <label htmlFor="edit-source-name">书源名称</label>
              <input
                id="edit-source-name"
                value={editName}
                onChange={(e) => setEditName(e.target.value)}
                placeholder="输入书源名称"
                autoFocus
              />
            </div>
            <textarea
              className="modal-editor"
              value={editText}
              onChange={(e) => setEditText(e.target.value)}
              spellCheck={false}
            />
            <div className="modal-actions">
              <button className="btn ghost" onClick={() => setEditing(null)}>
                <IconClose />
                取消
              </button>
              <button className="btn" onClick={() => void saveEdit()}>
                <IconCheck />
                保存
              </button>
            </div>
          </div>
        </div>
      ) : null}
    </div>
  )
})

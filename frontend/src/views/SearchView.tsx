import { Button, Progress, Text, TextInput } from '@mantine/core'
import { memo } from 'react'
import { authorLabel } from '@shared/author'
import type { SearchBook, SearchProgress } from '@shared/types'
import { IconClose, IconTrash, SearchIcon } from '../components/icons'
import { VirtualList } from '../components/VirtualList'

/**
 * 搜书页：多书源并发搜索、展示进度与结果，
 * 并管理搜索历史（点击重搜、单条删除、全部清空）。
 */
export const SearchView = memo(function SearchView({
  keyword,
  setKeyword,
  searching,
  progress,
  results,
  history,
  onSearch,
  onSearchKeyword,
  onClearHistory,
  onRemoveHistory,
  onRead
}: {
  keyword: string
  setKeyword: (v: string) => void
  searching: boolean
  progress: SearchProgress | null
  results: SearchBook[]
  history: string[]
  onSearch: () => void
  onSearchKeyword: (k: string) => void
  onClearHistory: () => void
  onRemoveHistory: (k: string) => void
  onRead: (b: SearchBook) => void
}) {
  const percent =
    progress && progress.total > 0
      ? Math.min(100, Math.round((progress.done / progress.total) * 100))
      : searching
        ? 8
        : 0

  const statusText = searching
    ? progress?.current
      ? `正在搜索「${progress.current}」· ${progress.done}/${progress.total} · 已找到 ${progress.found} 本`
      : progress
        ? `正在并发请求 ${progress.total} 个书源…`
        : '正在搜索…'
    : null

  return (
    <div className="search-page">
      <div className="search-page-top">
        <div className="panel-head">
          <div>
            <h2>搜索小说</h2>
            <p>多书源并发搜索</p>
          </div>
        </div>
        <div className="search-row">
          <TextInput
            value={keyword}
            placeholder="输入书名或作者"
            disabled={searching}
            size="sm"
            style={{ flex: 1, minWidth: 0 }}
            onChange={(e) => setKeyword(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') onSearch()
            }}
          />
          <Button
            size="sm"
            className="search-btn"
            loading={searching}
            disabled={searching}
            leftSection={searching ? undefined : <SearchIcon />}
            onClick={onSearch}
          >
            {searching ? '搜索中' : '搜索'}
          </Button>
        </div>
        {searching ? (
          <div className="search-progress" role="status" aria-live="polite">
            {progress && progress.total > 0 ? (
              <Progress value={percent} size="sm" radius="999px" />
            ) : (
              <div className="search-progress-track">
                <div className="search-progress-bar indeterminate" />
              </div>
            )}
            <Text size="xs" c="dimmed" mt={8}>
              {statusText}
            </Text>
          </div>
        ) : null}
      </div>

      <div className="search-page-body">
        {history.length ? (
          <div className="search-history">
            <div className="search-history-head">
              <span>搜索历史</span>
              <Button
                variant="subtle"
                size="compact-xs"
                disabled={searching}
                leftSection={<IconTrash />}
                onClick={onClearHistory}
              >
                清空
              </Button>
            </div>
            <div className="search-history-tags">
              {history.map((item) => (
                <div className="history-tag-wrap" key={item}>
                  <button
                    type="button"
                    className="history-tag"
                    disabled={searching}
                    onClick={() => onSearchKeyword(item)}
                  >
                    {item}
                  </button>
                  <button
                    type="button"
                    className="history-tag-remove"
                    title="删除这条历史"
                    aria-label={`删除历史 ${item}`}
                    disabled={searching}
                    onClick={(e) => {
                      e.stopPropagation()
                      onRemoveHistory(item)
                    }}
                  >
                    <IconClose />
                  </button>
                </div>
              ))}
            </div>
          </div>
        ) : null}

        {results.length > 0 ? (
          <VirtualList
            grid
            minColumnWidth={300}
            gap={12}
            estimateSize={104}
            overscan={4}
            style={{ flex: 1, minHeight: 0 }}
            className="search-results"
            count={results.length}
            getItemKey={(i) => `${results[i].origin}-${results[i].bookUrl}`}
            renderItem={(i) => {
              const b = results[i]
              return (
                <button type="button" className="book-card" onClick={() => onRead(b)}>
                  {b.coverUrl ? (
                    <img className="cover" src={b.coverUrl} alt="" />
                  ) : (
                    <div className="cover placeholder">{b.name.slice(0, 1)}</div>
                  )}
                  <div className="book-meta">
                    <h3>{b.name}</h3>
                    <p>{authorLabel(b.author)}</p>
                    <p>{b.originName}</p>
                    {b.lastChapter ? <p className="muted">{b.lastChapter}</p> : null}
                  </div>
                </button>
              )
            }}
          />
        ) : (
          <div className="empty">
            {searching ? '正在从各书源拉取结果，有结果会立即显示…' : '输入关键词开始搜索'}
          </div>
        )}
      </div>
    </div>
  )
})

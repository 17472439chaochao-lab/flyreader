import { memo, useMemo } from 'react'
import { Button, Select } from '@mantine/core'
import { authorLabel, authorName } from '@shared/author'
import { isLocalBook } from '@shared/bookLocal'
import type { ShelfBook, ShelfSort } from '@shared/types'
import { ShelfCover } from '../components/ShelfCover'
import { VirtualList } from '../components/VirtualList'
import {
  IconDownload,
  IconExport,
  IconImport,
  IconRefresh,
  IconStop,
  IconSwap,
  IconTrash,
  SearchIcon
} from '../components/icons'
import { cacheLabel } from '../utils/cacheLabel'
import { formatLastReadLabel } from '../utils/formatLastRead'

/**
 * 书架页：展示书籍列表（可排序），并提供打开、导入本地、
 * 更新、换源、缓存、导出 TXT、移除等操作入口。
 */
export const ShelfView = memo(function ShelfView({
  shelf,
  sort,
  busyIds,
  cacheBusyIds,
  cacheProgressMap,
  updatingAll,
  updateProgress,
  onOpen,
  onImportLocal,
  onSortChange,
  onUpdate,
  onUpdateAll,
  onChangeSource,
  onCache,
  onCancelCache,
  onExportTxt,
  onRemove,
  onSearch
}: {
  shelf: ShelfBook[]
  sort: ShelfSort
  busyIds: string[]
  cacheBusyIds: string[]
  cacheProgressMap: Record<string, string>
  updatingAll: boolean
  updateProgress: string
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
}) {
  const onlineCount = useMemo(() => shelf.filter((b) => !isLocalBook(b)).length, [shelf])
  const sortedShelf = useMemo(() => {
    const list = shelf.slice()
    if (sort === 'added') {
      list.sort((a, b) => (b.addedAt || 0) - (a.addedAt || 0))
    } else {
      list.sort((a, b) => {
        const ta = a.lastReadAt || a.updatedAt || 0
        const tb = b.lastReadAt || b.updatedAt || 0
        return tb - ta
      })
    }
    return list
  }, [shelf, sort])

  return (
    <div className="shelf-view">
      <div className="panel-head">
        <div>
          <h2>我的书架</h2>
          <p>{shelf.length ? `${shelf.length} 本书` : '还没有书，去搜一本或打开本地文件吧'}</p>
        </div>
        <div className="panel-actions">
          {shelf.length ? (
            <div className="shelf-sort">
              <span className="shelf-sort-label">排序</span>
              <Select
                size="xs"
                w={112}
                aria-label="书架排序"
                value={sort}
                onChange={(value) => value && onSortChange(value as ShelfSort)}
                data={[
                  { value: 'lastRead', label: '最近阅读' },
                  { value: 'added', label: '加入时间' }
                ]}
              />
            </div>
          ) : null}
          <Button size="sm" variant="default" leftSection={<IconImport />} onClick={onImportLocal}>
            打开本地书
          </Button>
          <Button
            size="sm"
            variant="default"
            disabled={!onlineCount || updatingAll}
            loading={updatingAll}
            leftSection={<IconRefresh />}
            onClick={onUpdateAll}
          >
            {updatingAll ? `更新中 ${updateProgress}` : '全部更新'}
          </Button>
          <Button size="sm" leftSection={<SearchIcon />} onClick={onSearch}>
            去搜书
          </Button>
        </div>
      </div>
      {!shelf.length ? (
        <div className="empty">书架空空如也</div>
      ) : (
        <VirtualList
          className="shelf-virtual"
          style={{ flex: 1, minHeight: 0 }}
          count={sortedShelf.length}
          estimateSize={96}
          gap={8}
          overscan={8}
          getItemKey={(i) => sortedShelf[i].id}
          renderItem={(i) => {
            const b = sortedShelf[i]
            const local = isLocalBook(b)
            const busy = busyIds.includes(b.id)
            const caching = !local && (cacheBusyIds.includes(b.id) || b.cache?.status === 'caching')
            const locked = busy || updatingAll
            const badge = cacheLabel(b, cacheBusyIds, cacheProgressMap)
            return (
              <div className="shelf-row">
                <button className="book-item" onClick={() => onOpen(b)}>
                  <ShelfCover book={b} />
                  <div className="book-meta">
                    <h3>
                      {b.name}
                      <span className={badge.cls}>{badge.text}</span>
                    </h3>
                    <p>
                      {authorLabel(
                        local &&
                          (b.localFormat === 'txt' ||
                            b.localFormat === 'md' ||
                            b.kind?.toUpperCase() === 'TXT' ||
                            b.kind?.toUpperCase() === 'MD') &&
                          authorName(b.author) === '本地'
                          ? '佚名'
                          : b.author
                      )}
                    </p>
                    <p>
                      {local ? '本地文件' : `当前源：${b.originName || '未知'}`}
                      {b.chapters?.length
                        ? ` · 读到 ${Math.min(b.chapterIndex + 1, b.chapters.length)}/${b.chapters.length}`
                        : ''}
                    </p>
                    <p className="shelf-last-read">{formatLastReadLabel(b.lastReadAt)}</p>
                    {b.lastChapter ? <p className="muted">最新：{b.lastChapter}</p> : null}
                  </div>
                </button>
                <div className="row-actions shelf-actions">
                  {!local ? (
                    <>
                      {caching ? (
                        <Button
                          size="compact-sm"
                          variant="default"
                          leftSection={<IconStop />}
                          onClick={() => onCancelCache(b.id)}
                        >
                          取消缓存
                        </Button>
                      ) : (
                        <Button
                          size="compact-sm"
                          variant="default"
                          disabled={locked}
                          leftSection={<IconDownload />}
                          onClick={() => onCache(b)}
                        >
                          {b.cache?.status === 'full' ? '重新缓存' : '缓存'}
                        </Button>
                      )}
                      <Button
                        size="compact-sm"
                        variant="default"
                        disabled={locked || caching}
                        leftSection={<IconExport />}
                        onClick={() => onExportTxt(b)}
                      >
                        导出TXT
                      </Button>
                      <Button
                        size="compact-sm"
                        variant="default"
                        disabled={locked || caching}
                        loading={busy}
                        leftSection={<IconRefresh />}
                        onClick={() => onUpdate(b)}
                      >
                        {busy ? '更新中' : '更新'}
                      </Button>
                      <Button
                        size="compact-sm"
                        variant="default"
                        disabled={locked || caching}
                        leftSection={<IconSwap />}
                        onClick={() => onChangeSource(b)}
                      >
                        换源
                      </Button>
                    </>
                  ) : null}
                  <Button
                    size="compact-sm"
                    variant="light"
                    color="danger"
                    disabled={locked || caching}
                    leftSection={<IconTrash />}
                    onClick={() => onRemove(b.id)}
                  >
                    移除
                  </Button>
                </div>
              </div>
            )
          }}
        />
      )}
    </div>
  )
})

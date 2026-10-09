import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { authorLabel } from '@shared/author'
import type { RankBoard, RankBook, RankChannel, RankGender } from '@shared/types'
import { LoadingIcon } from '../components/icons'
import { VirtualList, type VirtualListHandle } from '../components/VirtualList'

const CHANNELS: { id: RankChannel; label: string }[] = [
  { id: 'rank', label: '排行榜' },
  { id: 'finish', label: '完本' }
]

const BOARDS: { id: RankBoard; label: string }[] = [
  { id: 'yuepiao', label: '月票' },
  { id: 'hotsales', label: '畅销' },
  { id: 'recom', label: '推荐' },
  { id: 'collect', label: '收藏' }
]

const GENDERS: { id: RankGender; label: string }[] = [
  { id: 'male', label: '男频' },
  { id: 'female', label: '女频' }
]

/** 男频小说分类（排行 / 完本共用，对齐起点 catId） */
const MALE_CATS: { id: number; label: string }[] = [
  { id: -1, label: '全站' },
  { id: 21, label: '玄幻' },
  { id: 1, label: '奇幻' },
  { id: 2, label: '武侠' },
  { id: 22, label: '仙侠' },
  { id: 4, label: '都市' },
  { id: 15, label: '现实' },
  { id: 6, label: '军事' },
  { id: 5, label: '历史' },
  { id: 7, label: '游戏' },
  { id: 8, label: '体育' },
  { id: 9, label: '科幻' },
  { id: 10, label: '悬疑' },
  { id: 20109, label: '诸天' },
  { id: 12, label: '轻小说' }
]

/** 女频排行榜专用筛选（起点女频榜 catId 语义与完本分类不同） */
const FEMALE_RANK_CATS: { id: number; label: string }[] = [
  { id: -1, label: '全站' },
  { id: 21, label: '古言' },
  { id: 22, label: '现言' },
  { id: 23, label: '幻言' },
  { id: 0, label: 'VIP新作' }
]

/** 女频完本 / 分类列表 */
const FEMALE_FINISH_CATS: { id: number; label: string }[] = [
  { id: -1, label: '全站' },
  { id: 80, label: '古代言情' },
  { id: 81, label: '仙侠奇缘' },
  { id: 82, label: '现代言情' },
  { id: 83, label: '浪漫青春' },
  { id: 84, label: '玄幻言情' },
  { id: 85, label: '悬疑推理' },
  { id: 86, label: '科幻空间' },
  { id: 88, label: '游戏竞技' },
  { id: 87, label: '轻小说' },
  { id: 30120, label: '现实生活' }
]

function categoriesFor(channel: RankChannel, gender: RankGender) {
  if (gender === 'male') return MALE_CATS
  return channel === 'finish' ? FEMALE_FINISH_CATS : FEMALE_RANK_CATS
}

/**
 * 起点排行 / 完本页：拉取移动站榜单，点击书名跳转本机搜书。
 */
export const RankView = memo(function RankView({
  onOpenBook
}: {
  onOpenBook: (name: string) => void
}) {
  const [channel, setChannel] = useState<RankChannel>('rank')
  const [board, setBoard] = useState<RankBoard>('yuepiao')
  const [gender, setGender] = useState<RankGender>('male')
  const [catId, setCatId] = useState(-1)
  const [page, setPage] = useState(1)
  const [reloadKey, setReloadKey] = useState(0)
  const [books, setBooks] = useState<RankBook[]>([])
  const [hasMore, setHasMore] = useState(false)
  const [boardLabel, setBoardLabel] = useState('排行')
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const genRef = useRef(0)
  const listRef = useRef<VirtualListHandle>(null)
  const bodyRef = useRef<HTMLDivElement>(null)

  const categories = useMemo(() => categoriesFor(channel, gender), [channel, gender])
  const catLabel = categories.find((c) => c.id === catId)?.label || '全站'

  useEffect(() => {
    const gen = ++genRef.current
    setLoading(true)
    setError(null)
    void window.fly.rank
      .qidianList({
        channel,
        board: channel === 'rank' ? board : undefined,
        gender,
        page,
        catId
      })
      .then((r) => {
        if (gen !== genRef.current) return
        setBooks(r.books || [])
        setHasMore(!!r.hasMore)
        setBoardLabel(r.boardLabel || (channel === 'finish' ? '完本' : '排行'))
        setLoading(false)
      })
      .catch((e: unknown) => {
        if (gen !== genRef.current) return
        setBooks([])
        setHasMore(false)
        setError(e instanceof Error ? e.message : String(e || '起点暂时无法抓取'))
        setLoading(false)
      })
  }, [channel, board, gender, catId, page, reloadKey])

  // 翻页 / 切榜后回到列表顶部，避免沿用上一页的滚动位置
  useLayoutEffect(() => {
    listRef.current?.scrollToIndex(0, 'start')
    if (bodyRef.current) bodyRef.current.scrollTop = 0
  }, [channel, board, gender, catId, page, reloadKey, books])

  function resetCatIfNeeded(nextChannel: RankChannel, nextGender: RankGender) {
    const nextCats = categoriesFor(nextChannel, nextGender)
    if (!nextCats.some((c) => c.id === catId)) {
      setCatId(-1)
    }
  }

  function switchChannel(next: RankChannel) {
    if (next === channel) return
    resetCatIfNeeded(next, gender)
    setChannel(next)
    setPage(1)
  }

  function switchBoard(next: RankBoard) {
    if (next === board) return
    setBoard(next)
    setPage(1)
  }

  function switchGender(next: RankGender) {
    if (next === gender) return
    resetCatIfNeeded(channel, next)
    setGender(next)
    setPage(1)
  }

  function switchCat(next: number) {
    if (next === catId) return
    setCatId(next)
    setPage(1)
  }

  const subtitle =
    channel === 'finish'
      ? `${GENDERS.find((g) => g.id === gender)?.label || ''} · ${catLabel} · 第 ${page} 页`
      : `${boardLabel} · ${GENDERS.find((g) => g.id === gender)?.label || ''} · ${catLabel} · 第 ${page} 页`

  return (
    <div className="rank-page">
      <div className="rank-page-top">
        <div className="panel-head">
          <div>
            <h2>排行</h2>
            <p>起点中文网 · {subtitle}</p>
          </div>
          <div className="panel-actions">
            <button
              type="button"
              className="btn ghost"
              disabled={loading}
              onClick={() => setReloadKey((k) => k + 1)}
              title="重新加载"
            >
              {loading ? <LoadingIcon /> : null}
              {loading ? '加载中' : '刷新'}
            </button>
          </div>
        </div>

        <div className="rank-filters" role="toolbar" aria-label="排行筛选">
          <div className="rank-seg" role="group" aria-label="频道">
            {CHANNELS.map((c) => (
              <button
                key={c.id}
                type="button"
                className={channel === c.id ? 'active' : ''}
                aria-pressed={channel === c.id}
                onClick={() => switchChannel(c.id)}
              >
                {c.label}
              </button>
            ))}
          </div>
          {channel === 'rank' ? (
            <div className="rank-seg" role="group" aria-label="榜单">
              {BOARDS.map((b) => (
                <button
                  key={b.id}
                  type="button"
                  className={board === b.id ? 'active' : ''}
                  aria-pressed={board === b.id}
                  onClick={() => switchBoard(b.id)}
                >
                  {b.label}
                </button>
              ))}
            </div>
          ) : null}
          <div className="rank-seg" role="group" aria-label="频道性别">
            {GENDERS.map((g) => (
              <button
                key={g.id}
                type="button"
                className={gender === g.id ? 'active' : ''}
                aria-pressed={gender === g.id}
                onClick={() => switchGender(g.id)}
              >
                {g.label}
              </button>
            ))}
          </div>
          <div className="rank-seg rank-seg-wrap" role="group" aria-label="小说分类">
            {categories.map((c) => (
              <button
                key={c.id}
                type="button"
                className={catId === c.id ? 'active' : ''}
                aria-pressed={catId === c.id}
                onClick={() => switchCat(c.id)}
              >
                {c.label}
              </button>
            ))}
          </div>
        </div>
      </div>

      <div className="rank-page-body" ref={bodyRef}>
        {error ? (
          <div className="empty empty-guide">
            <h3>起点暂时无法抓取</h3>
            <p>{error}</p>
            <div className="empty-actions">
              <button type="button" className="btn" onClick={() => setReloadKey((k) => k + 1)}>
                重试
              </button>
            </div>
          </div>
        ) : books.length > 0 ? (
          <VirtualList
            ref={listRef}
            key={`${channel}-${board}-${gender}-${catId}-${page}-${reloadKey}`}
            grid
            minColumnWidth={300}
            gap={12}
            estimateSize={104}
            overscan={4}
            style={{ flex: 1, minHeight: 0 }}
            className="rank-results"
            count={books.length}
            getItemKey={(i) => `${books[i].bid || books[i].name}-${books[i].rank ?? i}`}
            renderItem={(i) => {
              const b = books[i]
              const rankNo = b.rank ?? (page - 1) * 20 + i + 1
              return (
                <button
                  type="button"
                  className="book-card rank-card"
                  onClick={() => onOpenBook(b.name)}
                  title={`在本软件搜索「${b.name}」`}
                >
                  <span className="rank-no" aria-hidden="true">
                    {rankNo}
                  </span>
                  {b.coverUrl ? (
                    <img className="cover" src={b.coverUrl} alt="" loading="lazy" />
                  ) : (
                    <div className="cover placeholder">{b.name.slice(0, 1)}</div>
                  )}
                  <div className="book-meta">
                    <h3>{b.name}</h3>
                    <p>{authorLabel(b.author)}</p>
                    <p className="muted">
                      {[b.category, b.status].filter(Boolean).join(' · ') || '点击搜索'}
                    </p>
                  </div>
                </button>
              )
            }}
          />
        ) : (
          <div className="empty">{loading ? '正在拉取起点榜单…' : '暂无数据'}</div>
        )}
      </div>

      <div className="rank-pager">
        <button
          type="button"
          className="btn ghost"
          disabled={loading || page <= 1}
          onClick={() => setPage((p) => Math.max(1, p - 1))}
        >
          上一页
        </button>
        <span className="rank-page-num">第 {page} 页</span>
        <button
          type="button"
          className="btn ghost"
          disabled={loading || !hasMore}
          onClick={() => setPage((p) => p + 1)}
        >
          下一页
        </button>
      </div>
    </div>
  )
})

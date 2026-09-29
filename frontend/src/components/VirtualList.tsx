import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode
} from 'react'

/** 虚拟列表对外暴露的命令式句柄 */
export type VirtualListHandle = {
  scrollToIndex: (index: number, align?: 'start' | 'center' | 'end') => void
}

/** 虚拟列表组件属性 */
export type VirtualListProps = {
  count: number
  /** 估算/统一行高；列表模式会按实测高度校正，网格模式作为统一行高 */
  estimateSize: number
  overscan?: number
  className?: string
  style?: CSSProperties
  getItemKey?: (index: number) => string | number
  renderItem: (index: number) => ReactNode
  /** 网格模式：按 minColumnWidth 自动计算列数并虚拟化（适合等高卡片） */
  grid?: boolean
  /** 网格列最小宽度（像素），默认 300 */
  minColumnWidth?: number
  /** 单元间距（像素）；列表为行间距，网格为纵向行间距，默认 0 */
  gap?: number
}

/** 单个虚拟单元：绝对定位并按内容实测高度，避免换行内容重叠 */
function MeasuredUnit({
  unitKey,
  offset,
  children,
  onMeasure
}: {
  unitKey: string | number
  offset: number
  children: ReactNode
  onMeasure: (key: string | number, height: number) => void
}) {
  const ref = useRef<HTMLDivElement | null>(null)
  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    const h = el.getBoundingClientRect().height
    if (h > 0) onMeasure(unitKey, h)
  }, [unitKey, onMeasure])
  return (
    <div
      ref={ref}
      className="virtual-list-item"
      style={{
        position: 'absolute',
        top: 0,
        left: 0,
        right: 0,
        transform: `translateY(${offset}px)`
      }}
    >
      {children}
    </div>
  )
}

/**
 * 虚拟化列表 / 网格：
 * - 仅渲染可视区域及 overscan 范围内的单元，通过绝对定位 + translateY 放置。
 * - 列表模式（grid 缺省）按内容实测高度（动态行高），长标题换行不会溢出重叠。
 * - 网格模式按固定列宽自动计算列数，按行带（band）虚拟化，适合等高卡片。
 * 暴露 scrollToIndex 句柄（列表按条目、网格按条目换算到所在行带）。
 */
export const VirtualList = forwardRef<VirtualListHandle, VirtualListProps>(function VirtualList(
  {
    count,
    estimateSize,
    overscan = 6,
    className,
    style,
    getItemKey,
    renderItem,
    grid = false,
    minColumnWidth = 300,
    gap = 0
  },
  ref
) {
  const scrollerRef = useRef<HTMLDivElement | null>(null)
  const [scrollTop, setScrollTop] = useState(0)
  const [viewportHeight, setViewportHeight] = useState(0)
  const [contentWidth, setContentWidth] = useState(0)
  /** 各单元实测高度；缺测时回退 estimateSize */
  const heightsRef = useRef<Map<string | number, number>>(new Map())
  const [heightsTick, setHeightsTick] = useState(0)
  /** 稳定化回调引用：getItemKey 通常由调用方内联传箭头函数，
   * 若直接进 useMemo 依赖会导致 offsets 每次父组件渲染都重算。 */
  const getItemKeyRef = useRef(getItemKey)
  getItemKeyRef.current = getItemKey

  useEffect(() => {
    const el = scrollerRef.current
    if (!el) return
    const sync = () => {
      setViewportHeight(el.clientHeight)
      setContentWidth(el.clientWidth)
    }
    sync()
    const ro = typeof ResizeObserver !== 'undefined' ? new ResizeObserver(sync) : null
    ro?.observe(el)
    window.addEventListener('resize', sync)
    return () => {
      ro?.disconnect()
      window.removeEventListener('resize', sync)
    }
  }, [])

  /** 滚动时通过 requestAnimationFrame 节流更新 scrollTop，避免整棵 VirtualList 每帧重渲染 */
  let scrollRaf = 0
  const onScroll = useCallback(() => {
    const el = scrollerRef.current
    if (!el) return
    if (scrollRaf) cancelAnimationFrame(scrollRaf)
    scrollRaf = requestAnimationFrame(() => setScrollTop(el.scrollTop))
  }, [])

  /** 实测高度回调：仅在高度变化（>0.5px）时更新，避免无谓重渲染 */
  const onMeasure = useCallback((key: string | number, h: number) => {
    const prev = heightsRef.current.get(key)
    if (prev === undefined || Math.abs(prev - h) > 0.5) {
      heightsRef.current.set(key, h)
      setHeightsTick((n) => n + 1)
    }
  }, [])

  const columns = grid
    ? Math.max(1, Math.floor((contentWidth + gap) / (minColumnWidth + gap)) || 1)
    : 1
  const unitCount = grid ? Math.ceil(count / columns) : count

  /** 前缀和高度（offsets），用于二分定位可见区间 */
  const offsets = useMemo(() => {
    const arr = new Array(unitCount + 1)
    arr[0] = 0
    const gk = getItemKeyRef.current
    for (let i = 0; i < unitCount; i++) {
      const key = grid ? `band-${i}` : gk ? gk(i) : i
      const h = heightsRef.current.get(key) ?? estimateSize
      arr[i + 1] = arr[i] + h + (i < unitCount - 1 ? gap : 0)
    }
    return arr
    // heightsTick 触发重算；getItemKey 通过 ref 稳定化，不再作为依赖
  }, [unitCount, estimateSize, gap, grid, heightsTick])

  const totalHeight = offsets[unitCount] ?? 0

  const { start, end } = useMemo(() => {
    if (unitCount <= 0 || viewportHeight <= 0) {
      return { start: 0, end: 0 }
    }
    // 二分：首个满足 offsets[i] <= scrollTop 的单元索引
    let lo = 0
    let hi = unitCount - 1
    let first = 0
    while (lo <= hi) {
      const mid = (lo + hi) >> 1
      if (offsets[mid] <= scrollTop) {
        first = mid
        lo = mid + 1
      } else {
        hi = mid - 1
      }
    }
    const s = Math.max(0, first - overscan)
    let e = first
    while (e < unitCount && offsets[e] < scrollTop + viewportHeight) e++
    e = Math.min(unitCount, e + overscan)
    return { start: s, end: e }
  }, [unitCount, offsets, overscan, scrollTop, viewportHeight])

  useImperativeHandle(
    ref,
    () => ({
      /**
       * 滚动到指定索引（条目）行；网格模式会换算到所在行带
       * @param index 目标条目索引
       * @param align 对齐方式：顶部 / 居中 / 底部
       */
      scrollToIndex(index: number, align: 'start' | 'center' | 'end' = 'center') {
        const el = scrollerRef.current
        if (!el || count <= 0) return
        const unitIndex = grid ? Math.floor(index / columns) : index
        const i = Math.max(0, Math.min(unitCount - 1, unitIndex))
        const unitTop = offsets[i]
        const unitH = offsets[i + 1] - offsets[i]
        let next = unitTop
        if (align === 'center') {
          next = unitTop - el.clientHeight / 2 + unitH / 2
        } else if (align === 'end') {
          next = unitTop - el.clientHeight + unitH
        }
        el.scrollTop = Math.max(0, Math.min(totalHeight - el.clientHeight, next))
      }
    }),
    [count, columns, grid, unitCount, offsets, totalHeight]
  )

  const units: ReactNode[] = []
  for (let i = start; i < end; i++) {
    const unitKey = grid ? `band-${i}` : getItemKey ? getItemKey(i) : i
    const offset = offsets[i]
    units.push(
      <MeasuredUnit key={unitKey} unitKey={unitKey} offset={offset} onMeasure={onMeasure}>
        {grid ? (
          <div
            className="virtual-list-grid"
            style={{
              display: 'grid',
              gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))`,
              gap: `${gap}px`
            }}
          >
            {Array.from({ length: Math.min(columns, count - i * columns) }, (_, k) => {
              const itemIndex = i * columns + k
              const itemKey = getItemKey ? getItemKey(itemIndex) : itemIndex
              return (
                <div key={itemKey} className="virtual-list-grid-cell">
                  {renderItem(itemIndex)}
                </div>
              )
            })}
          </div>
        ) : (
          renderItem(i)
        )}
      </MeasuredUnit>
    )
  }

  return (
    <div
      ref={scrollerRef}
      className={className ? `virtual-list ${className}` : 'virtual-list'}
      style={style}
      onScroll={onScroll}
    >
      <div className="virtual-list-spacer" style={{ height: totalHeight, position: 'relative' }}>
        {units}
      </div>
    </div>
  )
})

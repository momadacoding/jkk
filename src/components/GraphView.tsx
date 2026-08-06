import { useMemo, useRef, useState, useCallback, useEffect } from 'react'
import type { CommitNode, RefPointer } from '../engine/types'
import { computeLayout } from '../engine/layout'
import { computePaths, filterPathsToBand, laneColor } from '../engine/paths'
import { createSvgRenderer } from '../engine/svgRenderer'
import { WORKDIR_ID, withWorkdirRow } from '../engine/workdir'
import { CommitRow } from './CommitRow'

const ROW_HEIGHT = 28
const LANE_WIDTH = 20
const OVERSCAN = 5

interface GraphViewProps {
  commits: CommitNode[]
  refs: RefPointer[]
  selectedId: string | null
  onSelect: (id: string) => void
  /** 当前 HEAD 的完整 oid（不是分支名），用来挂工作区伪行。 */
  headHash: string
  /** 未提交改动数，显示在工作区伪行上。 */
  changeCount: number
  /** 点击工作区伪行时切到 Local Changes 视图。 */
  onOpenChanges: () => void
}

const renderer = createSvgRenderer()

export function GraphView({
  commits, refs, selectedId, onSelect, headHash, changeCount, onOpenChanges,
}: GraphViewProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const [scrollTop, setScrollTop] = useState(0)
  const [viewportHeight, setViewportHeight] = useState(600)

  // 布局与路径对全量历史只算一次：父 commit 可能在视口外，线段必须完整生成，
  // 渲染阶段才按条带过滤（见下方 svg memo）。颜色按 lane 分配，与视口无关。
  const layout = useMemo(() => {
    const withWorkdir = withWorkdirRow(commits, headHash || null)
    const rows = computeLayout(withWorkdir, refs)
    const paths = computePaths(rows, ROW_HEIGHT, LANE_WIDTH)
    return {
      rows,
      paths,
      laneCount: Math.max(1, ...rows.map((r) => r.lane)) + 1,
      rowHeight: ROW_HEIGHT,
      laneWidth: LANE_WIDTH,
      offsetY: 0,
    }
  }, [commits, refs, headHash])

  const startIndex = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN)
  const endIndex = Math.min(
    layout.rows.length - 1,
    Math.floor((scrollTop + viewportHeight) / ROW_HEIGHT) + OVERSCAN,
  )
  const visibleRows = layout.rows.slice(startIndex, endIndex + 1)

  // SVG 只画条带内的线段与节点：元素高度 = 条带高度，容器放在条带顶端，
  // 内容整体上移 offsetY。跨边界的线段保留下来，超出的部分由 SVG 边界裁掉，
  // 视觉上是「延伸到边缘」而不是断头。
  const bandTop = startIndex * ROW_HEIGHT
  const svg = useMemo(() => {
    const band = {
      ...layout,
      rows: visibleRows,
      paths: filterPathsToBand(layout.paths, bandTop, (endIndex + 1) * ROW_HEIGHT),
      offsetY: bandTop,
    }
    return renderer.render(band, refs, selectedId)
  }, [layout, visibleRows, bandTop, endIndex, refs, selectedId])

  // 标签用 display:none 隐藏时浏览器会把 scrollTop 清零，
  // 重新显示后要按记住的值还原，所以这里留一份 ref 供观察者读取。
  const scrollTopRef = useRef(0)

  // 挂载后立即量一次真实高度，并跟随面板拖动 / 标签显隐更新——
  // 否则首次滚动前一直按初始值估算可见行数。
  useEffect(() => {
    const el = containerRef.current
    if (!el) return
    const update = () => {
      const h = el.clientHeight
      if (h === 0) return // 标签被隐藏，别把高度记成 0
      setViewportHeight(h)
      if (el.scrollTop !== scrollTopRef.current) el.scrollTop = scrollTopRef.current
    }
    update()
    const observer = new ResizeObserver(update)
    observer.observe(el)
    return () => observer.disconnect()
  }, [])

  // 选中的提交不在可见范围内时把它滚到中间；已经可见就不动，避免点击行时画面乱跳
  useEffect(() => {
    const el = containerRef.current
    if (!el || !selectedId || el.clientHeight === 0) return
    const idx = commits.findIndex((c) => c.id === selectedId)
    if (idx < 0) return
    // 插了工作区伪行后真实提交整体下移一行
    const rowOffset = headHash ? 1 : 0
    const rowTop = (idx + rowOffset) * ROW_HEIGHT
    if (rowTop >= el.scrollTop && rowTop + ROW_HEIGHT <= el.scrollTop + el.clientHeight) return
    el.scrollTop = Math.max(0, rowTop - el.clientHeight / 2 + ROW_HEIGHT / 2)
  }, [selectedId, commits, headHash])

  const handleScroll = useCallback((e: React.UIEvent<HTMLDivElement>) => {
    scrollTopRef.current = e.currentTarget.scrollTop
    setScrollTop(e.currentTarget.scrollTop)
    setViewportHeight(e.currentTarget.clientHeight)
  }, [])

  const handleClick = useCallback((e: React.MouseEvent) => {
    const target = e.target as HTMLElement
    const id = target.closest('[data-id]')?.getAttribute('data-id')
    if (!id) return
    // 工作区伪行不是提交，选不了；点它等于进 Local Changes
    if (id === WORKDIR_ID) onOpenChanges()
    else onSelect(id)
  }, [onSelect, onOpenChanges])

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (!selectedId) return
      // commits 不含伪行，键盘导航天然跳过它
      const idx = commits.findIndex((c) => c.id === selectedId)
      if (idx < 0) return
      if (e.key === 'ArrowUp' && idx > 0) onSelect(commits[idx - 1].id)
      if (e.key === 'ArrowDown' && idx < commits.length - 1) onSelect(commits[idx + 1].id)
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [commits, selectedId, onSelect])

  const totalHeight = layout.rows.length * ROW_HEIGHT
  const layoutWidth = layout.laneCount * LANE_WIDTH
  const refByHash = useMemo(() => {
    const map: Record<string, RefPointer[]> = {}
    for (const ref of refs) {
      ;(map[ref.hash] ??= []).push(ref)
    }
    return map
  }, [refs])

  return (
    <div
      ref={containerRef}
      className="graph-view"
      onScroll={handleScroll}
      onClick={handleClick}
      style={{ overflow: 'auto', height: '100%' }}
    >
      <div style={{ position: 'relative', height: totalHeight, minWidth: layoutWidth + 300 }}>
        <div style={{ position: 'absolute', top: bandTop, left: 0, zIndex: 1 }}>
          {svg}
        </div>
        <div style={{ position: 'absolute', top: bandTop, left: 0, right: 0, zIndex: 2 }}>
          {visibleRows.map((node) => (
            <CommitRow
              key={node.commit.id}
              node={node}
              refs={refByHash[node.commit.id] ?? []}
              selected={selectedId === node.commit.id}
              layoutWidth={layoutWidth}
              laneColor={laneColor(node.lane)}
              changeCount={changeCount}
              isHead={node.commit.id === headHash}
            />
          ))}
        </div>
      </div>
    </div>
  )
}

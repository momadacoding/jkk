import { useMemo, useRef, useState, useCallback, useEffect } from 'react'
import type { CommitNode, RefPointer } from '../engine/types'
import { computeLayout } from '../engine/layout'
import { computePaths } from '../engine/paths'
import { createSvgRenderer } from '../engine/svgRenderer'
import { CommitRow } from './CommitRow'

const ROW_HEIGHT = 28
const LANE_WIDTH = 20
const OVERSCAN = 5

interface GraphViewProps {
  commits: CommitNode[]
  refs: RefPointer[]
  selectedId: string | null
  onSelect: (id: string) => void
}

const renderer = createSvgRenderer()

export function GraphView({ commits, refs, selectedId, onSelect }: GraphViewProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const [scrollTop, setScrollTop] = useState(0)
  const [viewportHeight, setViewportHeight] = useState(600)

  const layout = useMemo(() => {
    const rows = computeLayout(commits)
    const paths = computePaths(rows, ROW_HEIGHT, LANE_WIDTH)
    return {
      rows,
      paths,
      laneCount: Math.max(1, ...rows.map((r) => r.lane)) + 1,
      rowHeight: ROW_HEIGHT,
      laneWidth: LANE_WIDTH,
    }
  }, [commits])

  const startIndex = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN)
  const endIndex = Math.min(
    layout.rows.length - 1,
    Math.floor((scrollTop + viewportHeight) / ROW_HEIGHT) + OVERSCAN,
  )
  const visibleRows = layout.rows.slice(startIndex, endIndex + 1)

  const visibleLayout = useMemo(() => {
    const paths = computePaths(visibleRows, ROW_HEIGHT, LANE_WIDTH)
    return {
      ...layout,
      rows: visibleRows,
      paths,
    }
  }, [visibleRows, layout])

  const handleScroll = useCallback((e: React.UIEvent<HTMLDivElement>) => {
    setScrollTop(e.currentTarget.scrollTop)
    setViewportHeight(e.currentTarget.clientHeight)
  }, [])

  const handleClick = useCallback((e: React.MouseEvent) => {
    const target = e.target as HTMLElement
    const id = target.closest('[data-id]')?.getAttribute('data-id')
    if (id) onSelect(id)
  }, [onSelect])

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (!selectedId) return
      const idx = commits.findIndex((c) => c.id === selectedId)
      if (e.key === 'ArrowUp' && idx > 0) onSelect(commits[idx - 1].id)
      if (e.key === 'ArrowDown' && idx < commits.length - 1) onSelect(commits[idx + 1].id)
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [commits, selectedId, onSelect])

  const totalHeight = layout.rows.length * ROW_HEIGHT
  const layoutWidth = layout.laneCount * LANE_WIDTH
  const svg = renderer.render(visibleLayout, refs, selectedId)
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
        <div style={{ position: 'absolute', top: startIndex * ROW_HEIGHT, left: 0, zIndex: 1 }}>
          {svg}
        </div>
        <div style={{ position: 'absolute', top: startIndex * ROW_HEIGHT, left: 0, right: 0, zIndex: 2 }}>
          {visibleRows.map((node) => (
            <CommitRow
              key={node.commit.id}
              node={node}
              refs={refByHash[node.commit.id] ?? []}
              selected={selectedId === node.commit.id}
              layoutWidth={layoutWidth}
            />
          ))}
        </div>
      </div>
    </div>
  )
}

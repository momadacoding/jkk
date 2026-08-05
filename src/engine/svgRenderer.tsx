import type { ReactNode } from 'react'
import type { GraphLayout, LanePath, LayoutNode, RefPointer } from './graphTypes'
import type { GraphRenderer } from './renderer'
import { laneColor } from './paths'

export function createSvgRenderer(): GraphRenderer {
  return {
    render(layout, refs, selectedId): ReactNode {
      const width = Math.max(1, layout.laneCount * layout.laneWidth + 24)
      const height = layout.rows.length * layout.rowHeight
      const refByHash = groupRefs(refs)

      return (
        <svg width={width} height={height} className="graph-svg">
          {/* 坐标是整条历史的绝对值，整体上移把条带对齐到 SVG 原点 */}
          <g transform={`translate(0, ${-layout.offsetY})`}>
            {layout.paths.map((path) => renderPath(path))}
            {layout.rows.map((node) => renderNode(node, layout, refByHash[node.commit.id] ?? [], selectedId === node.commit.id))}
          </g>
        </svg>
      )
    },
  }
}

function groupRefs(refs: RefPointer[]): Record<string, RefPointer[]> {
  const map: Record<string, RefPointer[]> = {}
  for (const ref of refs) {
    ;(map[ref.hash] ??= []).push(ref)
  }
  return map
}

function renderPath(path: LanePath): ReactNode {
  let d = ''
  for (const s of path.segments) {
    if (s.curve) {
      const midY = (s.y1 + s.y2) / 2
      d += `M ${s.x1} ${s.y1} C ${s.x1} ${midY}, ${s.x2} ${midY}, ${s.x2} ${s.y2} `
    } else {
      d += `M ${s.x1} ${s.y1} L ${s.x2} ${s.y2} `
    }
  }
  return <path key={path.id} d={d} stroke={path.color} fill="none" strokeWidth={2} />
}

function renderNode(node: LayoutNode, layout: GraphLayout, refs: RefPointer[], selected: boolean): ReactNode {
  const cx = node.lane * layout.laneWidth + layout.laneWidth / 2
  const cy = node.row * layout.rowHeight + layout.rowHeight / 2
  const isHead = refs.some((r) => r.kind === 'head')
  // 中空圆：分支色填充 + 白边；HEAD 所在 commit 加大加粗；选中行高亮由 DOM 行负责，圆点不变形
  const r = isHead ? 6.5 : 5
  const strokeWidth = isHead || selected ? 2.5 : 1.5

  return (
    <g key={node.commit.id} className={selected ? 'commit-node selected' : 'commit-node'} data-id={node.commit.id}>
      <circle cx={cx} cy={cy} r={r} fill={laneColor(node.lane)} stroke="#fff" strokeWidth={strokeWidth} />
    </g>
  )
}

import type { ReactNode } from 'react'
import type { GraphLayout, LanePath, LayoutNode, RefPointer } from './graphTypes'
import type { GraphRenderer } from './renderer'

export function createSvgRenderer(): GraphRenderer {
  return {
    render(layout, refs, selectedId): ReactNode {
      const width = Math.max(1, layout.laneCount * layout.laneWidth + 24)
      const height = layout.rows.length * layout.rowHeight
      const refByHash = groupRefs(refs)

      return (
        <svg width={width} height={height} className="graph-svg">
          {layout.paths.map((path) => renderPath(path))}
          {layout.rows.map((node) => renderNode(node, layout, refByHash[node.commit.id] ?? [], selectedId === node.commit.id))}
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

function renderNode(node: LayoutNode, layout: GraphLayout, _refs: RefPointer[], selected: boolean): ReactNode {
  const cx = node.lane * layout.laneWidth + layout.laneWidth / 2
  const cy = node.row * layout.rowHeight + layout.rowHeight / 2
  const r = 5

  return (
    <g key={node.commit.id} className={selected ? 'commit-node selected' : 'commit-node'} data-id={node.commit.id}>
      {selected && <rect x={cx - 10} y={node.row * layout.rowHeight + 2} width={200} height={layout.rowHeight - 4} rx={4} className="selection-bg" />}
      <circle cx={cx} cy={cy} r={r} stroke={selected ? '#fff' : '#888'} strokeWidth={2} fill={selected ? '#fff' : '#222'} />
    </g>
  )
}

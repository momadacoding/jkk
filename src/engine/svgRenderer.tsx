import type { ReactNode } from 'react'
import type { GraphLayout, LanePath, LayoutNode, PathSegment, RefPointer } from './graphTypes'
import type { GraphRenderer } from './renderer'
import { laneColor } from './paths'
import { WORKDIR_ID } from './workdir'

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

function toD(segments: PathSegment[]): string {
  let d = ''
  for (const s of segments) {
    if (s.curve) {
      const midY = (s.y1 + s.y2) / 2
      d += `M ${s.x1} ${s.y1} C ${s.x1} ${midY}, ${s.x2} ${midY}, ${s.x2} ${s.y2} `
    } else {
      d += `M ${s.x1} ${s.y1} L ${s.x2} ${s.y2} `
    }
  }
  return d
}

function renderPath(path: LanePath): ReactNode {
  // 一个 path 元素只能有一种 dasharray，实线和虚线必须拆开画
  const solid = path.segments.filter((s) => !s.dashed)
  const dashed = path.segments.filter((s) => s.dashed)
  return (
    <g key={path.id}>
      {solid.length > 0 && (
        <path d={toD(solid)} stroke={path.color} fill="none" strokeWidth={2} />
      )}
      {dashed.length > 0 && (
        <path d={toD(dashed)} stroke={path.color} fill="none" strokeWidth={2} strokeDasharray="3 3" />
      )}
    </g>
  )
}

function renderNode(node: LayoutNode, layout: GraphLayout, refs: RefPointer[], selected: boolean): ReactNode {
  const cx = node.lane * layout.laneWidth + layout.laneWidth / 2
  const cy = node.row * layout.rowHeight + layout.rowHeight / 2
  const color = laneColor(node.lane)

  // 工作区伪行：空心虚线圆，和真实提交的实心圆明确区分
  if (node.commit.id === WORKDIR_ID) {
    return (
      <g key={node.commit.id} className="commit-node workdir" data-id={node.commit.id}>
        <circle cx={cx} cy={cy} r={5} fill="none" stroke={color} strokeWidth={1.5} strokeDasharray="2 2" />
      </g>
    )
  }

  const isHead = refs.some((r) => r.kind === 'head')
  // 中空圆：分支色填充 + 白边；HEAD 所在 commit 加大加粗；选中行高亮由 DOM 行负责，圆点不变形
  const r = isHead ? 6.5 : 5
  const strokeWidth = isHead || selected ? 2.5 : 1.5

  return (
    <g key={node.commit.id} className={selected ? 'commit-node selected' : 'commit-node'} data-id={node.commit.id}>
      <circle cx={cx} cy={cy} r={r} fill={color} stroke="#fff" strokeWidth={strokeWidth} />
    </g>
  )
}

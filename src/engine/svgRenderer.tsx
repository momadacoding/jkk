import type { ReactNode } from 'react'
import type { GraphLayout, LanePath, LayoutNode, RefPointer } from './graphTypes'
import type { GraphRenderer } from './renderer'
import { laneColor } from './paths'
import { toPathData } from './pathData'
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
            {layout.paths.map((path) => renderPath(path, layout.rowHeight))}
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

function renderPath(path: LanePath, rowHeight: number): ReactNode {
  // 一个 path 元素只能有一种 dasharray，实线和虚线必须拆开画
  const solid = path.segments.filter((s) => !s.dashed)
  const dashed = path.segments.filter((s) => s.dashed)
  return (
    <g key={path.id}>
      {solid.length > 0 && (
        <path d={toPathData(solid, rowHeight)} stroke={path.color} fill="none" strokeWidth={2} />
      )}
      {dashed.length > 0 && (
        <path d={toPathData(dashed, rowHeight)} stroke={path.color} fill="none" strokeWidth={2} strokeDasharray="3 3" />
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
  const isMerge = node.commit.parents.length > 1
  const className = selected ? 'commit-node selected' : 'commit-node'

  // 合并提交画成空心环。被并入的分支常常和主干共用车道与颜色，
  // 光看线分不出哪一行是合并；环形标记与实心圆一眼可分。
  // 填充色要和背景一致才成其为「环」，故与 --bg 保持同值。
  if (isMerge) {
    return (
      <g key={node.commit.id} className={`${className} merge`} data-id={node.commit.id}>
        <circle cx={cx} cy={cy} r={isHead ? 6.5 : 6} fill="#1e1e1e" stroke={color} strokeWidth={3} />
      </g>
    )
  }

  // 中空圆：分支色填充 + 白边；HEAD 所在 commit 加大加粗；选中行高亮由 DOM 行负责，圆点不变形
  const r = isHead ? 6.5 : 5
  const strokeWidth = isHead || selected ? 2.5 : 1.5

  return (
    <g key={node.commit.id} className={className} data-id={node.commit.id}>
      <circle cx={cx} cy={cy} r={r} fill={color} stroke="#fff" strokeWidth={strokeWidth} />
    </g>
  )
}

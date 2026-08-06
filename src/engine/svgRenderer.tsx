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

/** 拐角半径（像素）。 */
const CORNER = 6

/**
 * 把线段转成 path 数据。
 *
 * 跨车道的边画成「直线 + 小圆角」而不是跨越整段高度的贝塞尔：
 * 后者在相邻两行之间还算好看，一旦跨十几行就变成一道横扫几百像素的巨大弧线，
 * 完全看不出它连的是哪两个提交。线段始终笔直地待在它自己所属的车道里，
 * 只在另一端拐一次弯。
 */
function toD(segments: PathSegment[]): string {
  let d = ''
  for (const s of segments) {
    if (!s.curve) {
      d += `M ${s.x1} ${s.y1} L ${s.x2} ${s.y2} `
      continue
    }
    // 拐角不能大于它要占用的横向/纵向距离，否则会画出反向的钩子
    const r = Math.min(CORNER, Math.abs(s.x2 - s.x1), Math.abs(s.y2 - s.y1))
    const dir = s.x2 > s.x1 ? 1 : -1
    if (s.cornerAtTop) {
      // 从子节点横出去，拐进父节点的车道，再一路竖下去
      d += `M ${s.x1} ${s.y1} L ${s.x2 - dir * r} ${s.y1} Q ${s.x2} ${s.y1}, ${s.x2} ${s.y1 + r} L ${s.x2} ${s.y2} `
    } else {
      // 在子节点的车道里竖下去，到父节点那一行才拐过去
      d += `M ${s.x1} ${s.y1} L ${s.x1} ${s.y2 - r} Q ${s.x1} ${s.y2}, ${s.x1 + dir * r} ${s.y2} L ${s.x2} ${s.y2} `
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

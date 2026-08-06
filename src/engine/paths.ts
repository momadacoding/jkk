import type { LayoutNode, LanePath, PathSegment } from './graphTypes'
import { WORKDIR_ID } from './owners'

/** 按 lane 分配的高区分度调色板，12 色循环。 */
export const LANE_PALETTE = [
  '#4fc3f7', '#81c784', '#ffb74d', '#ba68c8',
  '#e57373', '#64b5f6', '#fff176', '#4db6ac',
  '#ff8a65', '#90a4ae', '#a1887f', '#7986cb',
]

/**
 * lane 的确定性颜色。
 *
 * 曾经的实现按 path 的插入顺序取色（`PALETTE[paths.size % N]`），
 * 可见切片一变颜色就重排。颜色只由 lane 决定，与计算顺序、视口无关。
 */
export function laneColor(lane: number): string {
  return LANE_PALETTE[Math.abs(lane) % LANE_PALETTE.length]
}

/**
 * 把车道色转成低透明度底色，给 ref 标签用。
 *
 * 标签底色跟着车道走，眼睛才能把「这个分支名」和「图上哪条线」连起来；
 * 统一的灰底做不到这一点。
 */
export function laneTint(color: string, alpha: number): string {
  const n = Number.parseInt(color.slice(1), 16)
  return `rgba(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}, ${alpha})`
}

/**
 * 为整组行计算分支线。
 *
 * 必须传入完整的 rows（整条历史），不能传可见切片：
 * 父 commit 在视口外时线段仍然要生成，渲染阶段再按条带过滤。
 * 内部用 hash → 行号索引，整体 O(n)。
 */
export function computePaths(rows: LayoutNode[], rowHeight: number, laneWidth: number): LanePath[] {
  const byHash = new Map<string, LayoutNode>()
  for (const row of rows) byHash.set(row.commit.id, row)

  const paths = new Map<number, LanePath>()

  function getPath(lane: number): LanePath {
    if (!paths.has(lane)) {
      paths.set(lane, {
        id: `lane-${lane}`,
        color: laneColor(lane),
        segments: [],
      })
    }
    return paths.get(lane)!
  }

  for (const node of rows) {
    const x = node.lane * laneWidth + laneWidth / 2
    const y = node.row * rowHeight + rowHeight / 2

    for (const parent of node.parents) {
      const parentNode = byHash.get(parent.hash)
      if (!parentNode) continue // 数据缺失（如 5000 条上限截断）时画不了

      // 必须用父提交**实际落到**的 lane，而不是 `parent.lane`。
      // 后者只是布局遍历子节点时的预留，父提交真正被处理时可能认领了别的 lane
      // （比如它同时是某个合并提交的第一父，早就被主干占住了）。
      // 用预留值会把线画成本车道内一段悬空的竖线，接不到父提交的圆点上。
      const px = parentNode.lane * laneWidth + laneWidth / 2
      const py = parentNode.row * rowHeight + rowHeight / 2

      // 第一父是本分支自身的延续，线归子节点的 lane；
      // 其余父是被并入的分支，线归那个父所在的 lane。两者都保证一条拓扑线同色。
      const path = getPath(parent.isMain ? node.lane : parentNode.lane)

      const segment: PathSegment = {
        x1: x,
        y1: y,
        x2: px,
        y2: py,
        curve: parentNode.lane !== node.lane,
        // 线归谁的车道，拐角就在另一端：第一父的线在子节点车道里往下走，
        // 到父节点那行才拐；第二父的线属于父节点车道，从子节点横出去后就竖下去。
        cornerAtTop: !parent.isMain,
        // 伪行连到 HEAD 的边不是真实的父子关系，画成虚线
        dashed: node.commit.id === WORKDIR_ID,
      }
      path.segments.push(segment)
    }
  }

  return Array.from(paths.values())
}

/**
 * 只保留与视口条带相交的线段。
 *
 * 线段只要有一个端点在条带内就保留——父 commit 在条带下方时，
 * 线段跨越边界，由 SVG 自身边界裁掉，视觉上「延伸到边缘」而不是断头。
 */
export function filterPathsToBand(paths: LanePath[], top: number, bottom: number): LanePath[] {
  return paths
    .map((p) => ({
      ...p,
      segments: p.segments.filter(
        (s) => Math.max(s.y1, s.y2) >= top && Math.min(s.y1, s.y2) <= bottom,
      ),
    }))
    .filter((p) => p.segments.length > 0)
}

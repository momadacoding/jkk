import type { CommitNode } from './types'
export type { RefPointer } from './types'

export interface LayoutNode {
  commit: CommitNode
  row: number
  lane: number
  parents: LayoutParent[]
}

export interface LayoutParent {
  hash: string
  lane: number // lane assigned to this parent by the layout pass
  isMain: boolean
}

export interface LanePath {
  id: string
  color: string
  segments: PathSegment[]
}

export interface PathSegment {
  x1: number
  y1: number
  x2: number
  y2: number
  curve: boolean // true for merge/fork curves
  /** 工作区伪行连到 HEAD 的那条线画成虚线，与真实提交边区分。 */
  dashed?: boolean
}

export interface GraphLayout {
  rows: LayoutNode[]
  paths: LanePath[]
  laneCount: number
  rowHeight: number
  laneWidth: number
  /**
   * 条带顶端的绝对 y（像素）。
   *
   * rows / paths 里的坐标都是相对整条历史的绝对值，而 SVG 元素只有条带那么高。
   * 渲染时整体上移 offsetY，条带内容才落在 SVG 自己的坐标系里。
   */
  offsetY: number
}

export interface ViewportConfig {
  top: number
  visibleCount: number
  rowHeight: number
}

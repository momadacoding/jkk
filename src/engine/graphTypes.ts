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
}

export interface GraphLayout {
  rows: LayoutNode[]
  paths: LanePath[]
  laneCount: number
  rowHeight: number
  laneWidth: number
}

export interface ViewportConfig {
  top: number
  visibleCount: number
  rowHeight: number
}

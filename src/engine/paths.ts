import type { LayoutNode, LanePath, PathSegment } from './graphTypes'

const PALETTE = [
  '#4fc3f7', '#81c784', '#ffb74d', '#ba68c8',
  '#e57373', '#64b5f6', '#fff176', '#4db6ac',
  '#ff8a65', '#90a4ae', '#a1887f', '#7986cb',
]

export function computePaths(rows: LayoutNode[], rowHeight: number, laneWidth: number): LanePath[] {
  const paths = new Map<string, LanePath>()

  function getPath(id: string): LanePath {
    if (!paths.has(id)) {
      paths.set(id, {
        id,
        color: PALETTE[paths.size % PALETTE.length],
        segments: [],
      })
    }
    return paths.get(id)!
  }

  for (const node of rows) {
    const x = node.lane * laneWidth + laneWidth / 2
    const y = node.row * rowHeight + rowHeight / 2

    for (const parent of node.parents) {
      const parentNode = rows.find((r) => r.commit.id === parent.hash)
      if (!parentNode) continue

      const px = parent.lane * laneWidth + laneWidth / 2
      const py = parentNode.row * rowHeight + rowHeight / 2
      const sameLane = parent.lane === node.lane
      const pathId = `${parent.lane}-${node.lane}`
      const path = getPath(pathId)

      const segment: PathSegment = {
        x1: x,
        y1: y,
        x2: px,
        y2: py,
        curve: !sameLane,
      }
      path.segments.push(segment)
    }
  }

  return Array.from(paths.values())
}

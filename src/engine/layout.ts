import type { CommitNode } from './types'
import type { LayoutNode, LayoutParent } from './graphTypes'

export function computeLayout(commits: CommitNode[]): LayoutNode[] {
  const rows: LayoutNode[] = []
  const laneHeads: (string | null)[] = [] // lane index -> current commit hash at top
  const commitToLane = new Map<string, number>()

  for (let row = 0; row < commits.length; row++) {
    const commit = commits[row]
    let lane = commitToLane.get(commit.id)

    // Try to claim a lane whose head is this commit
    if (lane === undefined) {
      lane = laneHeads.findIndex((head) => head === commit.id)
      if (lane === -1) {
        // New branch: take first free lane or expand
        lane = laneHeads.indexOf(null)
        if (lane === -1) {
          lane = laneHeads.length
          laneHeads.push(null)
        }
      }
    }

    // Mark this lane as occupied by this commit (will be released by parents)
    laneHeads[lane] = commit.id
    commitToLane.set(commit.id, lane)

    // Assign parents to lanes
    const parents: LayoutParent[] = commit.parents.map((parentHash, idx) => {
      const isMain = idx === 0
      let parentLane: number

      if (isMain) {
        parentLane = lane
      } else {
        // Secondary parent: try existing lane head, else new/free lane
        parentLane = laneHeads.findIndex((head) => head === parentHash)
        if (parentLane === -1) {
          parentLane = laneHeads.indexOf(null)
          if (parentLane === -1) {
            parentLane = laneHeads.length
            laneHeads.push(null)
          }
        }
      }

      // Record that this parent continues at parentLane
      if (!commitToLane.has(parentHash)) {
        laneHeads[parentLane] = parentHash
      }

      return { hash: parentHash, lane: parentLane, isMain }
    })

    rows.push({ commit, row, lane, parents })

    // Release this commit's lane for the next iteration unless a parent continues here
    const continuesHere = parents.some((p) => p.lane === lane)
    if (!continuesHere) {
      laneHeads[lane] = null
    }
  }

  return rows
}

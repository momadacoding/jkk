import { computeOwners, WORKDIR_ID } from './owners'
import { buildRefPills } from './refPills'
import type { CommitNode, RefPointer } from './types'

/**
 * 一个提交在图上「画在哪条分支的车道上」。
 *
 * 刻意只给一个答案，不是 `git branch --contains`：master 上的老提交被所有从它
 * 切出去的分支包含，列一长串反而认不出它在哪条线上（2026-10-17 用户选定）。
 */
export type LaneBranch =
  /** 车道顶端挂着的分支名；本地与同提交上的同名远程合成一个，和图上标签一致 */
  | { kind: 'branch'; names: string[] }
  /** 车道顶端没名字（合并后删了），报合并进去的那条分支；离它最近的那次合并排前面 */
  | { kind: 'merged'; into: string[] }
  /** SVN 混合图的主干 */
  | { kind: 'svn' }
  /** 游离 HEAD、只挂 tag 等：既没名字也没被合并过 */
  | { kind: 'none' }

/**
 * 算出图上每个提交所在车道的分支归属。
 *
 * 车道划分直接复用布局的 `computeOwners`，输入与 `computeLayout` 相同，所以这里报的
 * 分支就是图上那条线——不会出现面板说 A、图上画在 B 车道的情况。
 * 工作区伪行与图外提交（收进 svn 节点的本地提交、stash）查不到，返回 undefined。
 */
export function computeLaneBranches(commits: CommitNode[], refs: RefPointer[]): Map<string, LaneBranch> {
  const real = commits.filter((c) => c.id !== WORKDIR_ID)
  const { ownerOf, owners } = computeOwners(real, refs)

  // owner 的起点就是它行序最靠前的自有提交：认领沿第一父链只往下走。
  // 不能用 OwnerInfo.minRow——第二父边会把它顶到合并那一行。
  const tipOf: string[] = new Array(owners.length)
  // 已删分支要顺着「谁把我的 tip 当第二父合并了」往上找。按行序收完再整体反转，
  // 离 tip 最近（最早）的那次合并排前面；不逐个 unshift，同一 tip 被合并很多次时会退化成平方级。
  const mergersOf = new Map<string, string[]>()
  for (const c of real) {
    const owner = ownerOf.get(c.id)
    if (owner !== undefined && tipOf[owner] === undefined) tipOf[owner] = c.id
    for (const p of c.parents.slice(1)) {
      const list = mergersOf.get(p) ?? []
      list.push(c.id)
      mergersOf.set(p, list)
    }
  }
  for (const list of mergersOf.values()) list.reverse()

  const refsAt = new Map<string, RefPointer[]>()
  for (const r of refs) {
    if (r.kind !== 'branch' && r.kind !== 'remote') continue
    const list = refsAt.get(r.hash) ?? []
    list.push(r)
    refsAt.set(r.hash, list)
  }

  const memo = new Map<number, LaneBranch>()
  const resolve = (owner: number, visiting: Set<number>): LaneBranch => {
    const cached = memo.get(owner)
    if (cached) return cached
    const tip = tipOf[owner]
    let result: LaneBranch
    if (tip.startsWith('svn:')) {
      result = { kind: 'svn' }
    } else {
      const names = buildRefPills(refsAt.get(tip) ?? []).map((p) => p.label)
      if (names.length > 0) {
        result = { kind: 'branch', names }
      } else {
        // 图是 DAG，按理不会成环；visiting 只是防御，不让脏数据把递归跑飞
        visiting.add(owner)
        // Set 保持插入顺序；用数组 includes 去重时，名字一多、嵌套一深就是平方级（2026-10-17 Codex review）
        const into = new Set<string>()
        for (const merger of mergersOf.get(tip) ?? []) {
          const next = ownerOf.get(merger)
          if (next === undefined || visiting.has(next)) continue
          const up = resolve(next, visiting)
          const found =
            up.kind === 'branch' ? up.names
              : up.kind === 'merged' ? up.into
                : up.kind === 'svn' ? [formatLaneBranch(up)]
                  : []
          for (const name of found) into.add(name)
        }
        visiting.delete(owner)
        result = into.size > 0 ? { kind: 'merged', into: [...into] } : { kind: 'none' }
      }
    }
    memo.set(owner, result)
    return result
  }

  const out = new Map<string, LaneBranch>()
  for (const [id, owner] of ownerOf) out.set(id, resolve(owner, new Set()))
  return out
}

/** 详情面板「分支」一栏的文字。 */
export function formatLaneBranch(lane: LaneBranch): string {
  switch (lane.kind) {
    case 'branch':
      return lane.names.join(', ')
    case 'merged':
      return `已合并进 ${lane.into.join(', ')}`
    case 'svn':
      return 'SVN 主线'
    case 'none':
      return '无分支'
  }
}

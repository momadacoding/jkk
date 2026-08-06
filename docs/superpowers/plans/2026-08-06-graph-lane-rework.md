# Graph 车道重排 Implementation Plan（阶段一）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让每个分支 tip 占据自己的车道、`master` 永远在 lane 0，并把 ref 标签移到消息旁边重新设计，使两个分支在图上一眼可辨。

**Architecture:** 把 `computeLayout` 从一趟贪心改成两趟：Pass 1 沿第一父链算「归属」（每条分支线一个 owner），Pass 2 按行区间贪心装箱把 owner 映射到车道。车道复用保证宽度取决于同时活跃的分支数而非分支总数。工作区伪行作为合成提交注入，但**不参与**归属与装箱。

**Tech Stack:** TypeScript 5.6、React 18、Vitest 2.1（无 jsdom / testing-library）

规格：`docs/superpowers/specs/2026-08-06-toolbar-and-graph-lanes-design.md`

## Global Constraints

- 工作目录：`C:\Users\liuhao\workspace\dev\Dev\mygitclient\.worktrees\git-toolbar`，分支 `feat/git-toolbar-and-graph-polish`。
- **图形只由提交拓扑与 refs 决定。** checkout 切换分支不得改变任何车道分配或颜色。HEAD 只影响两件事：哪一行加粗、工作区伪行的虚线连到哪个提交。
- 后端 `src-tauri/src/engine/refs.rs:40-48` 会往 refs 数组塞一条 `kind: "head"` 的记录。所有布局代码**必须显式跳过 `kind === 'head'`**。
- 代码风格：单引号、尾随逗号、严格 TypeScript 类型、中文 JSDoc 注释。
- 测试命令：`pnpm test`（等价 `vitest run`）。类型检查：`pnpm exec tsc --noEmit`。
- **本仓库没有 jsdom / @testing-library/react。** 只给纯函数写单测；React 组件行为改用 CDP 运行时验证（Task 6）。不要为了测组件去装新依赖。
- 不改 `src/engine/paths.ts` 里 `laneColor` 的语义（颜色仍由 lane 决定）。

---

### Task 1: 归属计算（Pass 1）

**Files:**
- Create: `src/engine/owners.ts`
- Test: `tests/vitest/owners.test.ts`

**Interfaces:**
- Consumes: `CommitNode`、`RefPointer`（`src/engine/types.ts`，已存在）
- Produces:
  - `export const WORKDIR_ID = '__workdir__'`
  - `export interface OwnerInfo { index: number; minRow: number; maxRow: number }`
  - `export interface OwnerAssignment { ownerOf: Map<string, number>; owners: OwnerInfo[] }`
  - `export function collectTips(commits: CommitNode[], refs: RefPointer[]): string[]`
  - `export function computeOwners(commits: CommitNode[], refs: RefPointer[]): OwnerAssignment`

- [ ] **Step 1: 写失败的测试**

创建 `tests/vitest/owners.test.ts`：

```ts
import { describe, expect, it } from 'vitest'
import { collectTips, computeOwners } from '../../src/engine/owners'
import type { CommitNode, RefPointer } from '../../src/engine/types'

function node(id: string, parents: string[]): CommitNode {
  return { id, parents, author: 'a', time: 0, message: id }
}

function ref(name: string, hash: string, kind: RefPointer['kind']): RefPointer {
  return { name, hash, kind }
}

describe('collectTips', () => {
  it('把 master 排在最前面，哪怕它的 tip 比别的分支旧', () => {
    // feature 骑在 master 头上：feature 行号更小（更新），但 master 必须先被认领
    const commits = [node('f1', ['m1']), node('m1', ['m0']), node('m0', [])]
    const refs = [
      ref('refs/heads/feature', 'f1', 'branch'),
      ref('refs/heads/master', 'm1', 'branch'),
    ]
    expect(collectTips(commits, refs)[0]).toBe('m1')
  })

  it('没有本地 master 时回退到 origin/master', () => {
    const commits = [node('f1', ['m1']), node('m1', [])]
    const refs = [
      ref('refs/heads/feature', 'f1', 'branch'),
      ref('refs/remotes/origin/master', 'm1', 'remote'),
    ]
    expect(collectTips(commits, refs)[0]).toBe('m1')
  })

  it('本地和远程 master 都没有时，lane 0 归下一级的第一个 tip', () => {
    const commits = [node('d1', ['d0']), node('d0', [])]
    const refs = [ref('refs/heads/develop', 'd1', 'branch')]
    expect(collectTips(commits, refs)[0]).toBe('d1')
  })

  it('忽略 kind=head 的 ref —— 否则 checkout 会改变整张图', () => {
    const commits = [node('f1', ['m1']), node('m1', [])]
    // 后端 refs.rs 总会额外塞这条；它指向 f1，若参与排序就会抢在 master 前面
    const withHead = [
      ref('HEAD', 'f1', 'head'),
      ref('refs/heads/master', 'm1', 'branch'),
      ref('refs/heads/feature', 'f1', 'branch'),
    ]
    const withoutHead = withHead.filter((r) => r.kind !== 'head')
    expect(collectTips(commits, withHead)).toEqual(collectTips(commits, withoutHead))
    expect(collectTips(commits, withHead)[0]).toBe('m1')
  })

  it('同一个 hash 上的本地与远程分支只算一个 tip', () => {
    const commits = [node('m1', [])]
    const refs = [
      ref('refs/heads/master', 'm1', 'branch'),
      ref('refs/remotes/origin/master', 'm1', 'remote'),
    ]
    expect(collectTips(commits, refs)).toEqual(['m1'])
  })

  it('把没有子节点的悬空提交也算作 tip', () => {
    // dangling 没有任何 ref，也没有子节点，不认领它就会没有主人
    const commits = [node('dangling', ['m0']), node('m1', ['m0']), node('m0', [])]
    const refs = [ref('refs/heads/master', 'm1', 'branch')]
    expect(collectTips(commits, refs)).toContain('dangling')
  })

  it('跳过指向图外提交的 ref', () => {
    // 5000 条上限截断后，老分支的 tip 可能不在这批数据里
    const commits = [node('m1', [])]
    const refs = [ref('refs/heads/old', 'not-in-graph', 'branch')]
    expect(collectTips(commits, refs)).toEqual(['m1'])
  })
})

describe('computeOwners', () => {
  it('feature 与 master 属于不同 owner，分叉点归 master', () => {
    const commits = [node('f1', ['m1']), node('m1', ['m0']), node('m0', [])]
    const refs = [
      ref('refs/heads/master', 'm1', 'branch'),
      ref('refs/heads/feature', 'f1', 'branch'),
    ]
    const { ownerOf } = computeOwners(commits, refs)
    expect(ownerOf.get('m1')).toBe(0)
    expect(ownerOf.get('m0')).toBe(0)
    expect(ownerOf.get('f1')).not.toBe(0)
  })

  it('owner 0 的行区间覆盖它整条第一父链', () => {
    const commits = [node('m2', ['m1']), node('m1', ['m0']), node('m0', [])]
    const refs = [ref('refs/heads/master', 'm2', 'branch')]
    const { owners } = computeOwners(commits, refs)
    expect(owners[0]).toEqual({ index: 0, minRow: 0, maxRow: 2 })
  })

  it('合并提交的第二父会就地开一个新 owner', () => {
    // merge 的第二父 b1 只能通过第二父边到达，第一父链遍历覆盖不到它
    const commits = [node('merge', ['m1', 'b1']), node('b1', ['m0']), node('m1', ['m0']), node('m0', [])]
    const refs = [ref('refs/heads/master', 'merge', 'branch')]
    const { ownerOf } = computeOwners(commits, refs)
    expect(ownerOf.get('b1')).toBeDefined()
    expect(ownerOf.get('b1')).not.toBe(ownerOf.get('m1'))
  })

  it('每一个提交都有主人', () => {
    const commits = [
      node('merge', ['m1', 'b2']),
      node('b2', ['b1']),
      node('m1', ['m0']),
      node('b1', ['m0']),
      node('m0', []),
    ]
    const refs = [ref('refs/heads/master', 'merge', 'branch')]
    const { ownerOf } = computeOwners(commits, refs)
    for (const c of commits) expect(ownerOf.has(c.id)).toBe(true)
  })

  it('父提交不在数据里时停止遍历而不是崩溃', () => {
    const commits = [node('c1', ['truncated'])]
    const { ownerOf } = computeOwners(commits, [])
    expect(ownerOf.get('c1')).toBe(0)
    expect(ownerOf.has('truncated')).toBe(false)
  })
})
```

- [ ] **Step 2: 运行测试确认失败**

```
pnpm exec vitest run tests/vitest/owners.test.ts
```

Expected: FAIL — `Failed to resolve import "../../src/engine/owners"`

- [ ] **Step 3: 实现 `src/engine/owners.ts`**

```ts
import type { CommitNode, RefPointer } from './types'

/** 工作区伪行的 id。它不是真提交，不参与归属与装箱。 */
export const WORKDIR_ID = '__workdir__'

/** 一条「分支线」：一个 tip 加上它沿第一父链独占的提交。 */
export interface OwnerInfo {
  /** 认领顺序。0 号是主干（master/main），固定占 lane 0。 */
  index: number
  /** 该 owner 拥有的提交所占的行区间，闭区间。 */
  minRow: number
  maxRow: number
}

export interface OwnerAssignment {
  /** commit id → owner index */
  ownerOf: Map<string, number>
  owners: OwnerInfo[]
}

const MAIN_LOCAL = ['refs/heads/master', 'refs/heads/main']
const MAIN_REMOTE = ['refs/remotes/origin/master', 'refs/remotes/origin/main']

/** 不是任何提交的父 —— 也就是图里没有子节点的悬空 tip。已按行序返回。 */
function childless(commits: CommitNode[]): string[] {
  const hasChild = new Set<string>()
  for (const c of commits) for (const p of c.parents) hasChild.add(p)
  return commits.filter((c) => !hasChild.has(c.id)).map((c) => c.id)
}

/**
 * 按固定优先级挑出所有分支线的起点。
 *
 * 顺序只依赖提交拓扑与 refs，**不依赖 HEAD**：否则 checkout 换个分支
 * 整张图的车道就会重排。后端 `engine/refs.rs` 会额外塞一条 kind='head'
 * 的记录，这里必须显式跳过。
 */
export function collectTips(commits: CommitNode[], refs: RefPointer[]): string[] {
  const rowOf = new Map<string, number>()
  commits.forEach((c, i) => rowOf.set(c.id, i))

  // 指向图外提交的 ref（历史被 5000 条上限截断）认领不了，先滤掉
  const named = refs.filter((r) => r.kind !== 'head' && rowOf.has(r.hash))
  const byRow = (list: RefPointer[]) =>
    [...list].sort((a, b) => rowOf.get(a.hash)! - rowOf.get(b.hash)!).map((r) => r.hash)

  const local = named.filter((r) => r.kind === 'branch')
  const mainLocal = local.filter((r) => MAIN_LOCAL.includes(r.name))
  const mainRemote = named.filter((r) => MAIN_REMOTE.includes(r.name))

  const tiers: string[][] = [
    // 主干永远吃 lane 0；本地 master/main 优先，没有才回退远程。
    // 两者都不存在时本级为空，lane 0 自然归下一级的第一个 tip。
    byRow(mainLocal.length > 0 ? mainLocal : mainRemote),
    byRow(local),
    byRow(named.filter((r) => r.kind === 'remote')),
    byRow(named.filter((r) => r.kind === 'tag')),
    childless(commits),
  ]

  const seen = new Set<string>()
  const tips: string[] = []
  for (const tier of tiers) {
    for (const hash of tier) {
      if (seen.has(hash)) continue
      seen.add(hash)
      tips.push(hash)
    }
  }
  return tips
}

/**
 * Pass 1：把每个提交划归一条分支线。
 *
 * 按优先级从每个 tip 沿第一父链往下走，沿途还没主人的提交归该 tip；
 * 撞上已有主人的提交立即停止——那就是分叉点。
 */
export function computeOwners(commits: CommitNode[], refs: RefPointer[]): OwnerAssignment {
  const rowOf = new Map<string, number>()
  const byId = new Map<string, CommitNode>()
  commits.forEach((c, i) => {
    rowOf.set(c.id, i)
    byId.set(c.id, c)
  })

  const ownerOf = new Map<string, number>()
  const owners: OwnerInfo[] = []

  const claim = (start: string) => {
    if (ownerOf.has(start) || !rowOf.has(start)) return
    const index = owners.length
    const row = rowOf.get(start)!
    const info: OwnerInfo = { index, minRow: row, maxRow: row }
    owners.push(info)

    let cur: string | undefined = start
    while (cur && rowOf.has(cur) && !ownerOf.has(cur)) {
      ownerOf.set(cur, index)
      const r = rowOf.get(cur)!
      if (r < info.minRow) info.minRow = r
      if (r > info.maxRow) info.maxRow = r
      cur = byId.get(cur)!.parents[0]
    }
  }

  for (const tip of collectTips(commits, refs)) claim(tip)

  // 合并提交的第二父及以后只能经由第二父边到达，上面的第一父链遍历覆盖不到。
  // 按行序补认领，保证结果与遍历顺序无关。
  for (const commit of commits) {
    for (const parent of commit.parents.slice(1)) claim(parent)
  }

  return { ownerOf, owners }
}
```

- [ ] **Step 4: 运行测试确认通过**

```
pnpm exec vitest run tests/vitest/owners.test.ts
```

Expected: PASS，13 个用例全绿。

- [ ] **Step 5: 提交**

```bash
git add src/engine/owners.ts tests/vitest/owners.test.ts
git commit -m "feat(graph): add branch ownership pass"
```

---

### Task 2: 车道装箱（Pass 2）与 computeLayout 重写

**Files:**
- Modify: `src/engine/layout.ts:1-67`（整体替换）
- Test: `tests/vitest/layout.test.ts`（新建）

**`tests/vitest/paths.test.ts` 预期不需要改动。** `refs` 参数有默认值 `[]`，既有的 6 处 `computeLayout(commits)` 调用仍可编译；且逐条推演过新算法下的车道分配结果与断言一致（见 Step 5）。若真的失败，按 Step 5 的指引处理。

**Interfaces:**
- Consumes: Task 1 的 `computeOwners`、`OwnerInfo`、`WORKDIR_ID`
- Produces:
  - `export function packLanes(owners: OwnerInfo[]): number[]` —— 返回 `laneOf[ownerIndex]`
  - `export function computeLayout(commits: CommitNode[], refs?: RefPointer[]): LayoutNode[]` —— `refs` 默认 `[]`，保持既有调用点可编译

- [ ] **Step 1: 写失败的测试**

创建 `tests/vitest/layout.test.ts`：

```ts
import { describe, expect, it } from 'vitest'
import { computeLayout, packLanes } from '../../src/engine/layout'
import type { CommitNode, RefPointer } from '../../src/engine/types'

function node(id: string, parents: string[]): CommitNode {
  return { id, parents, author: 'a', time: 0, message: id }
}

function ref(name: string, hash: string, kind: RefPointer['kind']): RefPointer {
  return { name, hash, kind }
}

function laneById(rows: ReturnType<typeof computeLayout>): Record<string, number> {
  return Object.fromEntries(rows.map((r) => [r.commit.id, r.lane]))
}

describe('packLanes', () => {
  it('owner 0 固定拿 lane 0', () => {
    expect(packLanes([{ index: 0, minRow: 0, maxRow: 5 }])).toEqual([0])
  })

  it('行区间不重叠的两个 owner 复用同一条车道', () => {
    // 这条是「20 个分支不会变成 20 列」的唯一依据
    const lanes = packLanes([
      { index: 0, minRow: 0, maxRow: 20 }, // 主干
      { index: 1, minRow: 1, maxRow: 3 },
      { index: 2, minRow: 8, maxRow: 10 }, // 与 owner 1 完全不重叠
    ])
    expect(lanes[0]).toBe(0)
    expect(lanes[1]).toBe(lanes[2])
    expect(lanes[1]).not.toBe(0)
  })

  it('行区间重叠的两个 owner 必须占不同车道', () => {
    const lanes = packLanes([
      { index: 0, minRow: 0, maxRow: 20 },
      { index: 1, minRow: 1, maxRow: 9 },
      { index: 2, minRow: 5, maxRow: 12 },
    ])
    expect(new Set([lanes[0], lanes[1], lanes[2]]).size).toBe(3)
  })

  it('主干的车道不被别的 owner 复用', () => {
    // owner 0 早早结束，但 lane 0 仍然只属于它
    const lanes = packLanes([
      { index: 0, minRow: 0, maxRow: 1 },
      { index: 1, minRow: 5, maxRow: 9 },
    ])
    expect(lanes[0]).toBe(0)
    expect(lanes[1]).not.toBe(0)
  })
})

describe('computeLayout', () => {
  it('骑在 master 头上的分支 tip 占自己的车道，master 在 lane 0', () => {
    // 这就是截图里的形状：feat 分支只比 master 多一个提交
    const commits = [node('f1', ['m1']), node('m1', ['m0']), node('m0', [])]
    const refs = [
      ref('refs/heads/master', 'm1', 'branch'),
      ref('refs/heads/feature', 'f1', 'branch'),
    ]
    const lanes = laneById(computeLayout(commits, refs))
    expect(lanes.m1).toBe(0)
    expect(lanes.m0).toBe(0)
    expect(lanes.f1).not.toBe(0)
  })

  it('切换 HEAD 不改变任何车道', () => {
    // 图形只由拓扑与 refs 决定，见规格「设计原则」
    const commits = [node('f1', ['m1']), node('m1', ['m0']), node('m0', [])]
    const base = [
      ref('refs/heads/master', 'm1', 'branch'),
      ref('refs/heads/feature', 'f1', 'branch'),
    ]
    const onMaster = computeLayout(commits, [...base, ref('HEAD', 'm1', 'head')])
    const onFeature = computeLayout(commits, [...base, ref('HEAD', 'f1', 'head')])
    expect(laneById(onFeature)).toEqual(laneById(onMaster))
  })

  it('LayoutParent.lane 等于父提交实际落到的车道', () => {
    // 上一轮修过的悬空竖线就是因为这里存的是预留值而非实际值
    const commits = [node('f1', ['m1']), node('m1', [])]
    const refs = [
      ref('refs/heads/master', 'm1', 'branch'),
      ref('refs/heads/feature', 'f1', 'branch'),
    ]
    const rows = computeLayout(commits, refs)
    const byId = Object.fromEntries(rows.map((r) => [r.commit.id, r]))
    expect(byId.f1.parents[0].lane).toBe(byId.m1.lane)
  })

  it('没有 refs 时不崩溃，每行都有车道', () => {
    const rows = computeLayout([node('c2', ['c1']), node('c1', [])])
    expect(rows.map((r) => r.lane)).toEqual([0, 0])
  })

  it('父提交不在数据里时该父的车道退化为 0 而不是 undefined', () => {
    const rows = computeLayout([node('c1', ['truncated'])])
    expect(rows[0].parents[0].lane).toBe(0)
  })
})
```

- [ ] **Step 2: 运行测试确认失败**

```
pnpm exec vitest run tests/vitest/layout.test.ts
```

Expected: FAIL — `packLanes is not a function`（`computeLayout` 已存在但 `packLanes` 未导出）

- [ ] **Step 3: 整体替换 `src/engine/layout.ts`**

```ts
import type { CommitNode, RefPointer } from './types'
import type { LayoutNode } from './graphTypes'
import { computeOwners, WORKDIR_ID, type OwnerInfo } from './owners'

/**
 * Pass 2：把每个 owner 映射到一条车道。
 *
 * owner 0（主干）固定 lane 0 且不被复用；其余按 minRow 升序，
 * 各自挑第一条「当前占用者已经结束」的车道。
 *
 * 车道复用是这个设计能用的前提：仓库里几十个早已合并的旧分支行区间
 * 互不重叠，会共享同一条车道。车道数只取决于**同时活跃**的分支数，
 * 而不是分支总数。
 */
export function packLanes(owners: OwnerInfo[]): number[] {
  const laneOf = new Array<number>(owners.length).fill(0)
  /** lane 序号 → 当前占用者的 maxRow；-1 表示没被用过。 */
  const laneEnd: number[] = []

  const take = (lane: number, owner: OwnerInfo) => {
    while (laneEnd.length <= lane) laneEnd.push(-1)
    laneEnd[lane] = owner.maxRow
    laneOf[owner.index] = lane
  }

  const order = [...owners].sort((a, b) => a.minRow - b.minRow || a.index - b.index)
  for (const owner of order) {
    if (owner.index === 0) {
      take(0, owner)
      continue
    }
    // 从 1 开始找：lane 0 是主干的专座
    let lane = 1
    while (lane < laneEnd.length && laneEnd[lane] >= owner.minRow) lane++
    take(lane, owner)
  }
  return laneOf
}

/**
 * 两趟布局：先算归属（`computeOwners`），再把 owner 装箱到车道。
 *
 * `refs` 可省略，省略时退化成「按悬空 tip 分线」——纯拓扑视图，
 * 既有的 paths 测试就靠这条路径。
 */
export function computeLayout(commits: CommitNode[], refs: RefPointer[] = []): LayoutNode[] {
  // 工作区伪行不参与归属与装箱。让它参与的话，HEAD 在哪个分支就会改变
  // 该分支 owner 的行区间，进而挤动别人的车道——图就跟着 checkout 变了。
  const real = commits.filter((c) => c.id !== WORKDIR_ID)
  const { ownerOf, owners } = computeOwners(real, refs)
  const laneOfOwner = packLanes(owners)

  const laneOfCommit = new Map<string, number>()
  for (const [id, owner] of ownerOf) laneOfCommit.set(id, laneOfOwner[owner] ?? 0)
  const laneOf = (id: string) => laneOfCommit.get(id) ?? 0

  return commits.map((commit, row) => ({
    commit,
    row,
    // 伪行直接取父节点的车道，见上面的注释
    lane: commit.id === WORKDIR_ID ? laneOf(commit.parents[0] ?? '') : laneOf(commit.id),
    parents: commit.parents.map((hash, idx) => ({
      hash,
      lane: laneOf(hash),
      isMain: idx === 0,
    })),
  }))
}
```

- [ ] **Step 4: 运行测试确认通过**

```
pnpm exec vitest run tests/vitest/layout.test.ts
```

Expected: PASS，9 个用例全绿。

- [ ] **Step 5: 跑既有回归测试**

```
pnpm test
```

Expected: `tests/vitest/paths.test.ts` 的 9 个用例**不改代码直接通过**。

推演依据（`refs` 为空时走「悬空 tip」这条路径）：

- `[c4(c3,c2), c3(c1), c2(c1), c1()]` —— 只有 c4 没有子节点，故 tips=`['c4']`。owner 0 沿第一父链吃下 c4→c3→c1（lane 0）；c2 只能经 c4 的第二父到达，由补认领循环开成 owner 1，行区间 `[2,2]` → lane 1。于是 `lane-1` 这条 path 在全量和 `slice(2,4)` 里都存在且同色，`colors are stable per lane across input slices` 成立。
- `[merge(c1,c3), c3(c2), c2(c1), c1()]` —— owner 0 = merge→c1（lane 0），owner 1 = c3→c2（lane 1）。`c2 → c1` 线段是 `30,70 -> 10,98`，与 `connects every segment to where its parent actually sits` 的期望值一致。

**如果仍然失败**，先确认失败的是哪一条：

- 若是 `connects every segment...`（上一轮修复悬空竖线的锁），说明 `LayoutParent.lane` 没有取到父的实际车道，回头查 `computeLayout` 里的 `laneOf(hash)`。
- 若是 `colors are stable...`，把断言里写死的 `lane-1` 改成从实际布局读出的车道号，**不要**改 `laneColor` 的实现。

- [ ] **Step 6: 类型检查**

```
pnpm exec tsc --noEmit
```

Expected: 无错误。`computeLayout` 的 `refs` 有默认值，`GraphView.tsx:29` 的旧调用点仍可编译。

- [ ] **Step 7: 提交**

```bash
git add src/engine/layout.ts tests/vitest/layout.test.ts tests/vitest/paths.test.ts
git commit -m "feat(graph): lane packing by branch ownership"
```

---

### Task 3: 工作区伪行

**Files:**
- Modify: `src/engine/graphTypes.ts:23-29`（`PathSegment` 加 `dashed`）
- Modify: `src/engine/paths.ts:44-72`（标记虚线段）
- Create: `src/engine/workdir.ts`
- Test: `tests/vitest/workdir.test.ts`

**Interfaces:**
- Consumes: Task 1 的 `WORKDIR_ID`，Task 2 的 `computeLayout`
- Produces:
  - `export function withWorkdirRow(commits: CommitNode[], headHash: string | null): CommitNode[]`
  - `PathSegment` 新增可选字段 `dashed?: boolean`
  - `src/engine/owners.ts` 的 `WORKDIR_ID` 由 `workdir.ts` 重新导出，供组件层引用

- [ ] **Step 1: 写失败的测试**

创建 `tests/vitest/workdir.test.ts`：

```ts
import { describe, expect, it } from 'vitest'
import { WORKDIR_ID, withWorkdirRow } from '../../src/engine/workdir'
import { computeLayout } from '../../src/engine/layout'
import { computePaths } from '../../src/engine/paths'
import type { CommitNode, RefPointer } from '../../src/engine/types'

function node(id: string, parents: string[]): CommitNode {
  return { id, parents, author: 'a', time: 0, message: id }
}

function ref(name: string, hash: string, kind: RefPointer['kind']): RefPointer {
  return { name, hash, kind }
}

const commits = [node('f1', ['m1']), node('m1', ['m0']), node('m0', [])]
const refs = [
  ref('refs/heads/master', 'm1', 'branch'),
  ref('refs/heads/feature', 'f1', 'branch'),
]

describe('withWorkdirRow', () => {
  it('把伪行插在最前面，父指向 HEAD', () => {
    const out = withWorkdirRow(commits, 'm1')
    expect(out[0].id).toBe(WORKDIR_ID)
    expect(out[0].parents).toEqual(['m1'])
    expect(out.slice(1)).toEqual(commits)
  })

  it('没有 HEAD（空仓库）时不插伪行', () => {
    expect(withWorkdirRow(commits, null)).toEqual(commits)
  })
})

describe('工作区伪行的布局', () => {
  it('伪行取 HEAD 提交的车道', () => {
    const rows = computeLayout(withWorkdirRow(commits, 'f1'), refs)
    const byId = Object.fromEntries(rows.map((r) => [r.commit.id, r]))
    expect(byId[WORKDIR_ID].lane).toBe(byId.f1.lane)
  })

  it('HEAD 在 master 上时伪行仍在 lane 0，不会凭空多出一列', () => {
    // 若让伪行参与装箱，它会退化成只占一行的独立 owner 被丢到 lane 1
    const rows = computeLayout(withWorkdirRow(commits, 'm1'), refs)
    const byId = Object.fromEntries(rows.map((r) => [r.commit.id, r]))
    expect(byId[WORKDIR_ID].lane).toBe(0)
  })

  it('伪行不改变任何真实提交的车道', () => {
    const without = computeLayout(commits, refs)
    const with_ = computeLayout(withWorkdirRow(commits, 'f1'), refs)
    const lanes = (rows: typeof without) =>
      Object.fromEntries(rows.filter((r) => r.commit.id !== WORKDIR_ID).map((r) => [r.commit.id, r.lane]))
    expect(lanes(with_)).toEqual(lanes(without))
  })

  it('HEAD 落在哪个分支都不改变真实提交的车道', () => {
    const onMaster = computeLayout(withWorkdirRow(commits, 'm1'), refs)
    const onFeature = computeLayout(withWorkdirRow(commits, 'f1'), refs)
    const lanes = (rows: typeof onMaster) =>
      Object.fromEntries(rows.filter((r) => r.commit.id !== WORKDIR_ID).map((r) => [r.commit.id, r.lane]))
    expect(lanes(onFeature)).toEqual(lanes(onMaster))
  })
})

describe('虚线标记', () => {
  it('只有伪行连出去的线段是虚线', () => {
    const rows = computeLayout(withWorkdirRow(commits, 'f1'), refs)
    const paths = computePaths(rows, 28, 20)
    const all = paths.flatMap((p) => p.segments)
    expect(all.filter((s) => s.dashed)).toHaveLength(1)
    expect(all.filter((s) => !s.dashed).length).toBeGreaterThan(0)
  })

  it('没有伪行时一条虚线都没有', () => {
    const paths = computePaths(computeLayout(commits, refs), 28, 20)
    expect(paths.flatMap((p) => p.segments).some((s) => s.dashed)).toBe(false)
  })
})
```

- [ ] **Step 2: 运行测试确认失败**

```
pnpm exec vitest run tests/vitest/workdir.test.ts
```

Expected: FAIL — `Failed to resolve import "../../src/engine/workdir"`

- [ ] **Step 3: 创建 `src/engine/workdir.ts`**

```ts
import type { CommitNode } from './types'
import { WORKDIR_ID } from './owners'

export { WORKDIR_ID }

/**
 * 在提交列表最前面插入工作区伪行。
 *
 * 它是个合成提交，第一父指向 HEAD，于是布局时天然落在 HEAD 那条提交的
 * 正上方；`computeLayout` 会把它排除在归属与装箱之外，只让它继承父的车道。
 * 空仓库或游离状态取不到 HEAD 时不插，图上就没有这一行。
 */
export function withWorkdirRow(commits: CommitNode[], headHash: string | null): CommitNode[] {
  if (!headHash) return commits
  return [
    { id: WORKDIR_ID, parents: [headHash], author: '', time: 0, message: '' },
    ...commits,
  ]
}
```

- [ ] **Step 4: `src/engine/graphTypes.ts` 的 `PathSegment` 加字段**

把 `PathSegment` 改成：

```ts
export interface PathSegment {
  x1: number
  y1: number
  x2: number
  y2: number
  curve: boolean // true for merge/fork curves
  /** 工作区伪行连到 HEAD 的那条线画成虚线，与真实提交边区分。 */
  dashed?: boolean
}
```

- [ ] **Step 5: `src/engine/paths.ts` 标记虚线段**

在 `computePaths` 里，把 `src/engine/paths.ts:63-69` 构造 segment 的部分改成：

```ts
      const segment: PathSegment = {
        x1: x,
        y1: y,
        x2: px,
        y2: py,
        curve: parentNode.lane !== node.lane,
        // 伪行连到 HEAD 的边不是真实的父子关系，画成虚线
        dashed: node.commit.id === WORKDIR_ID,
      }
```

并在文件顶部加 import：

```ts
import { WORKDIR_ID } from './owners'
```

- [ ] **Step 6: 运行测试确认通过**

```
pnpm exec vitest run tests/vitest/workdir.test.ts && pnpm test
```

Expected: workdir 8 个用例通过；owners、layout、paths、path、errors 全部继续通过。

- [ ] **Step 7: 提交**

```bash
git add src/engine/workdir.ts src/engine/graphTypes.ts src/engine/paths.ts tests/vitest/workdir.test.ts
git commit -m "feat(graph): add working directory pseudo-row"
```

---

### Task 4: SVG 渲染虚线与空心工作区节点

**Files:**
- Modify: `src/engine/svgRenderer.tsx:34-60`

**Interfaces:**
- Consumes: Task 3 的 `PathSegment.dashed`、`WORKDIR_ID`
- Produces: 无新导出；`createSvgRenderer()` 的行为变化

- [ ] **Step 1: 改写 `renderPath`**

一个 `<path>` 元素只能有一种 `stroke-dasharray`，所以实线段和虚线段必须拆成两个元素。替换 `src/engine/svgRenderer.tsx:34-45`：

```tsx
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
```

在文件顶部的 type import 里补上 `PathSegment`：

```tsx
import type { GraphLayout, LanePath, LayoutNode, PathSegment, RefPointer } from './graphTypes'
```

- [ ] **Step 2: 改写 `renderNode` 特判工作区节点**

替换 `src/engine/svgRenderer.tsx:47-60`：

```tsx
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
  // 中空圆：分支色填充 + 白边；HEAD 所在 commit 加大加粗；
  // 选中行高亮由 DOM 行负责，圆点不变形
  const r = isHead ? 6.5 : 5
  const strokeWidth = isHead || selected ? 2.5 : 1.5

  return (
    <g key={node.commit.id} className={selected ? 'commit-node selected' : 'commit-node'} data-id={node.commit.id}>
      <circle cx={cx} cy={cy} r={r} fill={color} stroke="#fff" strokeWidth={strokeWidth} />
    </g>
  )
}
```

在文件顶部加 import：

```tsx
import { WORKDIR_ID } from './workdir'
```

- [ ] **Step 3: 类型检查并跑测试**

```
pnpm exec tsc --noEmit && pnpm test
```

Expected: 无类型错误；全部单测通过（本任务不新增测试，渲染在 Task 6 用 CDP 验证）。

- [ ] **Step 4: 提交**

```bash
git add src/engine/svgRenderer.tsx
git commit -m "feat(graph): render dashed workdir connector"
```

---

### Task 5: 后端补 HEAD 的 hash

**Files:**
- Modify: `src-tauri/src/engine/repo.rs:5-21`
- Modify: `src/engine/types.ts:9-12`
- Modify: `src/store/repoStore.ts:8-19,21-32,37-55`
- Test: `src-tauri/tests/fixtures.rs`（追加一个用例）

**Interfaces:**
- Produces:
  - Rust `RepoInfo { path: String, head: String, head_hash: String }`，序列化为 `{ path, head, headHash }`
  - TS `RepoInfo { path: string; head: string; headHash: string }`
  - `RepoState` 新增 `headHash: string`

**为什么需要这个任务：** `engine/repo.rs:16` 现在返回的 `head` 是 `head.shorthand()`，也就是分支名 `master`，不是 commit hash。Task 3 的 `withWorkdirRow(commits, headHash)` 需要真正的 oid 才能把伪行挂上去；传分支名的话父节点在图里找不到，伪行不会出现，Task 4 的虚线也画不出来。

- [ ] **Step 1: 写失败的测试**

在 `src-tauri/tests/fixtures.rs` 末尾追加：

```rust
#[test]
fn open_repo_reports_both_head_name_and_hash() {
    let dir = linear_repo();
    let info = engine::repo::open(dir.path().to_str().unwrap()).unwrap();
    // head 仍是给人看的分支名
    assert!(info.head == "master" || info.head == "main", "head was {}", info.head);
    // head_hash 是完整 oid，且必须等于图里最新的那条提交——
    // 工作区伪行就是靠这个 hash 挂到 HEAD 上的
    assert_eq!(info.head_hash.len(), 40);
    let data = engine::graph::load(dir.path().to_str().unwrap()).unwrap();
    assert_eq!(info.head_hash, data.commits[0].id);
}
```

- [ ] **Step 2: 运行测试确认失败**

```
cd src-tauri && cargo test open_repo_reports_both_head_name_and_hash
```

Expected: 编译失败 —— `no field head_hash on type RepoInfo`

- [ ] **Step 3: 改 `src-tauri/src/engine/repo.rs`**

```rust
use crate::error::{GitError, Result};
use git2::Repository;
use serde::Serialize;

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RepoInfo {
    pub path: String,
    /// 给人看的分支名（游离时是 "HEAD"）。
    pub head: String,
    /// HEAD 解析出的完整 oid。工作区伪行靠它挂到图上。
    pub head_hash: String,
}

pub fn open(path: &str) -> Result<RepoInfo> {
    let repo = Repository::open(path).map_err(|_| {
        GitError::new("not_repo", format!("{} is not a git repository", path))
    })?;
    let head = repo.head()?;
    let head_name = head.shorthand().unwrap_or("HEAD").to_string();
    // 符号引用要 resolve 之后才有 target
    let head_hash = head
        .resolve()
        .ok()
        .and_then(|r| r.target())
        .map(|oid| oid.to_string())
        .unwrap_or_default();
    Ok(RepoInfo {
        path: path.to_string(),
        head: head_name,
        head_hash,
    })
}
```

`#[serde(rename_all = "camelCase")]` 对 `path` / `head` 这两个单词字段没有影响，只把 `head_hash` 变成 `headHash`，前端不用改已有字段名。

注意：`repo.head()?` 在**空仓库**（还没有任何提交）上会报错，导致 `open_repo` 整体失败。这是既有行为，不在本轮范围内，不要顺手改。

- [ ] **Step 4: 运行测试确认通过**

```
cd src-tauri && cargo test
```

Expected: 全部通过，含新增用例。

- [ ] **Step 5: 前端类型与 store 跟上**

`src/engine/types.ts` 的 `RepoInfo` 改成：

```ts
export interface RepoInfo {
  path: string
  /** 给人看的分支名。 */
  head: string
  /** HEAD 的完整 oid，工作区伪行挂在它上面。 */
  headHash: string
}
```

`src/store/repoStore.ts` 三处跟进。`RepoState` 接口加字段：

```ts
export interface RepoState {
  path: string | null
  head: string
  headHash: string
  commits: CommitNode[]
  refs: RefPointer[]
  worktrees: Worktree[]
  status: StatusData
  diff: DiffData
  selectedId: string | null
  loading: boolean
  error: string | null
}
```

`initialState` 加 `headHash: ''`（放在 `head: ''` 之后）。

`openRepo` 里 `src/store/repoStore.ts:41` 改成：

```ts
      setState((s) => ({ ...s, path, head: info.head, headHash: info.headHash, loading: false }))
```

`refreshHead` 里 `src/store/repoStore.ts:52-55` 改成：

```ts
  /** 重新读取 HEAD。checkout 之后必须调用，否则 header 与分支高亮会停在旧分支。 */
  const refreshHead = async (path: string) => {
    const info = await invoke<RepoInfo>('open_repo', { path })
    setState((s) => ({ ...s, head: info.head, headHash: info.headHash }))
  }
```

- [ ] **Step 6: 类型检查**

```
pnpm exec tsc --noEmit
```

Expected: 无错误。

- [ ] **Step 7: 提交**

```bash
git add src-tauri/src/engine/repo.rs src-tauri/tests/fixtures.rs src/engine/types.ts src/store/repoStore.ts
git commit -m "feat(repo): expose HEAD oid alongside branch name"
```

---

### Task 6: Ref 标签重做与 GraphView 接线

**Files:**
- Modify: `src/components/CommitRow.tsx:1-33`（整体替换）
- Modify: `src/components/GraphView.tsx:12-46,119-152`
- Modify: `src/components/RepoTab.tsx:196`（给 GraphView 传新 props）
- Modify: `src/styles.css:180-198`（`.ref-pill` 一段）

**Interfaces:**
- Consumes: Task 3 的 `WORKDIR_ID`、`withWorkdirRow`；Task 2 的 `computeLayout(commits, refs)`；Task 5 的 `RepoState.headHash`
- Produces:
  - `CommitRowProps` 新增 `laneColor: string`、`changeCount: number`、`isHead: boolean`
  - `GraphViewProps` 新增 `headHash: string`、`changeCount: number`、`onOpenChanges: () => void`

**命名约定：** 本任务里 GraphView 的 prop 叫 `headHash`（完整 oid），不叫 `head`。`state.head` 是分支名，两者不能混用。

- [ ] **Step 1: 整体替换 `src/components/CommitRow.tsx`**

```tsx
import type { LayoutNode, RefPointer } from '../engine/graphTypes'
import { WORKDIR_ID } from '../engine/workdir'

interface CommitRowProps {
  node: LayoutNode
  refs: RefPointer[]
  selected: boolean
  layoutWidth: number
  /** 该提交所在车道的颜色，用作本地分支标签的左侧色条。 */
  laneColor: string
  /** 工作区伪行显示的未提交改动数。 */
  changeCount: number
  /** 该提交是否是 HEAD，是则整行加粗。 */
  isHead: boolean
}

/** 去掉 refs/heads/ 与 refs/remotes/ 前缀，标签上只留下人看的部分。 */
function shortName(name: string): string {
  return name.replace('refs/heads/', '').replace('refs/remotes/', '').replace('refs/tags/', '')
}

export function CommitRow({
  node, refs, selected, layoutWidth, laneColor, changeCount, isHead,
}: CommitRowProps) {
  if (node.commit.id === WORKDIR_ID) {
    return (
      <div
        className={selected ? 'commit-row workdir selected' : 'commit-row workdir'}
        style={{ height: 28, paddingLeft: layoutWidth + 8 }}
        data-id={node.commit.id}
      >
        <span className="workdir-label">
          {changeCount > 0 ? `${changeCount} changes` : 'No changes'}
        </span>
        <span className="commit-meta">Commit Changes</span>
      </div>
    )
  }

  const date = new Date(node.commit.time * 1000).toLocaleString()
  // HEAD 那条伪 ref 不做成标签——整行加粗已经表达了「当前在这里」，
  // 只有游离 HEAD 时才需要一个显式标记
  const visible = refs.filter((r) => r.kind !== 'head')
  const detached = refs.some((r) => r.kind === 'head') && !refs.some((r) => r.kind === 'branch')

  return (
    <div
      className={`commit-row${selected ? ' selected' : ''}${isHead ? ' head' : ''}`}
      style={{ height: 28, paddingLeft: layoutWidth + 8 }}
      data-id={node.commit.id}
    >
      {/* flex: 0 1 auto 让消息只占它需要的宽度，标签才能紧跟其后而不是被顶到窗口右缘 */}
      <span className="commit-message">{node.commit.message}</span>
      <span className="ref-pills">
        {detached && <span className="ref-pill head">HEAD</span>}
        {visible.map((ref) => (
          <span
            key={ref.name}
            className={`ref-pill ${ref.kind}`}
            style={ref.kind === 'branch' ? { borderLeftColor: laneColor } : undefined}
            title={ref.name}
          >
            {shortName(ref.name)}
          </span>
        ))}
      </span>
      <span className="commit-meta">
        {node.commit.author} • {date}
      </span>
    </div>
  )
}
```

- [ ] **Step 2: 替换 `src/styles.css` 里 `.commit-row` / `.ref-pill` 两段**

删除 `src/styles.css:160-198`（从 `.commit-row {` 到 `.ref-pill.head { ... }` 结束），替换为：

```css
.commit-row {
  display: flex;
  align-items: center;
  gap: 8px;
  cursor: pointer;
  box-sizing: border-box;
}

.commit-row:hover,
.commit-row.selected {
  background: var(--selected-bg);
}

/* HEAD 所在行整行加粗，替代一个专门的 HEAD 标签 */
.commit-row.head .commit-message {
  font-weight: bold;
  color: #fff;
}

.commit-row.workdir .workdir-label {
  color: #ffb74d;
}

/* flex: 0 1 auto —— 消息只占自己需要的宽度，标签才能紧跟其后。
   用 flex: 1 的话消息会撑满整行，把标签顶到窗口最右端。 */
.commit-message {
  flex: 0 1 auto;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: system-ui, -apple-system, sans-serif;
}

.ref-pills {
  display: flex;
  flex-shrink: 0;
  gap: 4px;
}

/* 作者与时间靠右，标签留在消息旁边 */
.commit-meta {
  margin-left: auto;
  flex-shrink: 0;
  padding-right: 8px;
  color: #888;
  font-size: 11px;
  white-space: nowrap;
}

.ref-pill {
  padding: 1px 6px;
  border-radius: 3px;
  font-size: 10px;
  font-family: monospace;
  white-space: nowrap;
  max-width: 220px;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* 本地分支：左侧色条取该提交所在车道的颜色（内联 style 覆盖 borderLeftColor） */
.ref-pill.branch {
  background: #33404d;
  color: #dce8f5;
  border-left: 3px solid #4fc3f7;
}

/* 远程分支：纯灰、无色条，与本地一眼可分 */
.ref-pill.remote {
  background: #2f2f2f;
  color: #9a9a9a;
}

.ref-pill.tag {
  background: transparent;
  border: 1px dashed #c9a227;
  color: #d9b93a;
}

.ref-pill.head {
  background: #fff;
  color: #1e1e1e;
  font-weight: bold;
}
```

- [ ] **Step 3: `src/components/GraphView.tsx` 接线**

改三处。

其一，props 与 imports（替换 `src/components/GraphView.tsx:1-19`）：

```tsx
import { useMemo, useRef, useState, useCallback, useEffect } from 'react'
import type { CommitNode, RefPointer } from '../engine/types'
import { computeLayout } from '../engine/layout'
import { computePaths, filterPathsToBand, laneColor } from '../engine/paths'
import { createSvgRenderer } from '../engine/svgRenderer'
import { WORKDIR_ID, withWorkdirRow } from '../engine/workdir'
import { CommitRow } from './CommitRow'

const ROW_HEIGHT = 28
const LANE_WIDTH = 20
const OVERSCAN = 5

interface GraphViewProps {
  commits: CommitNode[]
  refs: RefPointer[]
  selectedId: string | null
  onSelect: (id: string) => void
  /** 当前 HEAD 的完整 oid（不是分支名），用来挂工作区伪行。 */
  headHash: string
  /** 未提交改动数，显示在工作区伪行上。 */
  changeCount: number
  /** 点击工作区伪行时切到 Local Changes 视图。 */
  onOpenChanges: () => void
}
```

其二，布局 memo（替换 `src/components/GraphView.tsx:28-39`）：

```tsx
  // 布局与路径对全量历史只算一次：父 commit 可能在视口外，线段必须完整生成，
  // 渲染阶段才按条带过滤（见下方 svg memo）。颜色按 lane 分配，与视口无关。
  const layout = useMemo(() => {
    const withWorkdir = withWorkdirRow(commits, headHash || null)
    const rows = computeLayout(withWorkdir, refs)
    const paths = computePaths(rows, ROW_HEIGHT, LANE_WIDTH)
    return {
      rows,
      paths,
      laneCount: Math.max(1, ...rows.map((r) => r.lane)) + 1,
      rowHeight: ROW_HEIGHT,
      laneWidth: LANE_WIDTH,
      offsetY: 0,
    }
  }, [commits, refs, headHash])
```

其三，点击处理与行渲染。替换 `src/components/GraphView.tsx:100-104` 的 `handleClick`：

```tsx
  const handleClick = useCallback((e: React.MouseEvent) => {
    const target = e.target as HTMLElement
    const id = target.closest('[data-id]')?.getAttribute('data-id')
    if (!id) return
    // 工作区伪行不是提交，选不了；点它等于进 Local Changes
    if (id === WORKDIR_ID) onOpenChanges()
    else onSelect(id)
  }, [onSelect, onOpenChanges])
```

替换 `src/components/GraphView.tsx:140-148` 的 `CommitRow` 渲染：

```tsx
          {visibleRows.map((node) => (
            <CommitRow
              key={node.commit.id}
              node={node}
              refs={refByHash[node.commit.id] ?? []}
              selected={selectedId === node.commit.id}
              layoutWidth={layoutWidth}
              laneColor={laneColor(node.lane)}
              changeCount={changeCount}
              isHead={node.commit.id === headHash}
            />
          ))}
```

上下键导航要跳过伪行——替换 `src/components/GraphView.tsx:106-115` 的 keydown effect：

```tsx
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (!selectedId) return
      const idx = commits.findIndex((c) => c.id === selectedId)
      if (idx < 0) return
      if (e.key === 'ArrowUp' && idx > 0) onSelect(commits[idx - 1].id)
      if (e.key === 'ArrowDown' && idx < commits.length - 1) onSelect(commits[idx + 1].id)
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [commits, selectedId, onSelect])
```

注意 `commits` 这里仍是**不含伪行**的原始列表，所以键盘导航天然跳过伪行；但 `selectedId` 居中滚动的 effect（`src/components/GraphView.tsx:84-92`）用 `commits.findIndex` 算行号，插入伪行后所有真实提交的行号都 +1。把该 effect 里的 `const rowTop = idx * ROW_HEIGHT` 改为：

```tsx
    // 插了工作区伪行后真实提交整体下移一行
    const rowOffset = headHash ? 1 : 0
    const rowTop = (idx + rowOffset) * ROW_HEIGHT
```

并把 effect 依赖数组从 `[selectedId, commits]` 改成 `[selectedId, commits, headHash]`。

- [ ] **Step 4: `src/components/RepoTab.tsx` 传新 props**

替换 `src/components/RepoTab.tsx:196`：

```tsx
                  <GraphView
                    commits={state.commits}
                    refs={state.refs}
                    selectedId={state.selectedId}
                    onSelect={selectCommit}
                    headHash={state.headHash}
                    changeCount={state.status.unstaged.length + state.status.staged.length}
                    onOpenChanges={() => setViewMode('changes')}
                  />
```

`state.headHash` 来自 Task 5。**不要**传 `state.head`——那是分支名 `master`，在提交列表里找不到对应节点，伪行会静默消失。

- [ ] **Step 5: 类型检查并跑测试**

```
pnpm exec tsc --noEmit && pnpm test
```

Expected: 无类型错误；全部单测通过。

- [ ] **Step 6: 提交**

```bash
git add src/components/CommitRow.tsx src/components/GraphView.tsx src/components/RepoTab.tsx src/styles.css
git commit -m "feat(graph): pill column next to message, workdir row wiring"
```

---

### Task 7: 运行时验证

**Files:**
- Modify: `docs/superpowers/e2e/2026-08-04-mygitclient-e2e-checklist.md`（追加本轮条目）

本仓库没有 jsdom / testing-library，组件行为靠 CDP 对着真实 WebView2 验证。方法沿用上一轮：以
`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222` 启动 `pnpm tauri dev`，再用
`Runtime.evaluate` / `Page.captureScreenshot` 检查。

- [ ] **Step 1: 启动应用**

```bash
WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222 pnpm tauri dev
```

打开本仓库自身（`C:\Users\liuhao\workspace\dev\Dev\mygitclient`），它有 `master`、`ddd`、
`feat-mvp`、`feat/git-toolbar-and-graph-polish` 多个分支，正好覆盖多车道场景。

- [ ] **Step 2: 逐条核对**

1. `master` 的圆点在最左列（lane 0）。
2. `feat/git-toolbar-and-graph-polish` 的 tip **不在** lane 0，有自己的颜色。
3. 最顶上有工作区伪行，空心虚线圆，虚线连到 HEAD 那条提交。
4. 本地分支标签有彩色左侧色条，远程分支标签是纯灰、无色条，两者一眼可分。
5. 标签紧跟在提交消息右边，不在窗口最右缘。
6. HEAD 所在行的消息是加粗白色。
7. `git checkout ddd` 之后，**除了伪行的位置，所有提交的车道与颜色完全不变**。这是本轮最重要的一条——截图前后对比。
8. 点击工作区伪行切到 Local Changes 视图。
9. 上下键导航跳过伪行，不会选中它。
10. 滚动到底部时最后几行仍有圆点和线（上一轮 SVG 裁剪 bug 的回归点——**必须截图确认，数值检查看不出来**）。

- [ ] **Step 3: 把结果记进 E2E 清单并提交**

在 `docs/superpowers/e2e/2026-08-04-mygitclient-e2e-checklist.md` 追加一节
「2026-08-06 Graph 车道重排」，逐条记录通过/失败与证据。

```bash
git add docs/superpowers/e2e/2026-08-04-mygitclient-e2e-checklist.md
git commit -m "docs(e2e): record graph lane rework verification"
```

---

## 完成标准

- `pnpm test` 全绿，含新增的 owners（13）、layout（9）、workdir（8）用例。
- `cd src-tauri && cargo test` 全绿，含 Task 5 新增的 `open_repo_reports_both_head_name_and_hash`。
- `pnpm exec tsc --noEmit` 无错误。
- Task 7 第 7 条（切分支不改图）截图前后一致。
- Task 7 第 10 条（滚到底部仍有圆点和线）截图确认。

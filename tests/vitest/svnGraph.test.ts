import { describe, expect, it } from 'vitest'
import { baselineMap, remapRefs, spliceHistory, type SvnRevNode } from '@/engine/svnGraph'
import type { CommitNode } from '@/engine/types'

const svn = (rev: number): SvnRevNode => ({
  rev,
  author: 'someone',
  date: '2026-09-15T15:21:22.337313Z',
  message: `r${rev}`,
})

/**
 * 后端 `load_graph` 给的一条 git 提交。
 *
 * ⚠️ `message` **只有标题**（后端取的是 `commit.summary()`）。
 * 这里绝不能手写一条带 trailer 的多行 message —— 2026-09-16 之前这个 helper
 * 就是那么写的，于是全部测试在「前端根本拿不到 trailer」的情况下照样全绿
 * （`AGENTS.md` §13 第 9 条：测试的输入不是实际场景的形态 = 虚假信心）。
 */
const git = (id: string, parents: string[], extra: Partial<CommitNode> = {}): CommitNode => ({
  id,
  parents,
  author: 'me',
  time: 1_700_000_000,
  message: 'work',
  ...extra,
})

/**
 * 一条 baseline commit，形状与后端真实产物一致：标题是 `svn: rN`，
 * trailer 已经由 Rust 侧解析成 `baselineRev`。
 */
const baseline = (id: string, parents: string[], rev: number): CommitNode =>
  git(id, parents, { message: `svn: r${rev}`, baselineRev: rev })

describe('spliceHistory', () => {
  it('SVN 序列连成一条链，parent 指向下一个更小的 revision', () => {
    const out = spliceHistory([svn(3), svn(2), svn(1)], [])
    expect(out.map((c) => c.id)).toEqual(['svn:3', 'svn:2', 'svn:1'])
    expect(out[0].parents).toEqual(['svn:2'])
    expect(out[1].parents).toEqual(['svn:1'])
    expect(out[2].parents).toEqual([])
  })

  it('svn 节点带上 vcs 与 svnRev', () => {
    const out = spliceHistory([svn(3)], [])
    expect(out[0].vcs).toBe('svn')
    expect(out[0].svnRev).toBe(3)
    expect(out[0].author).toBe('someone')
  })

  it('revision 不连号时照样接成链', () => {
    // 白名单路径的历史天然跳号：只动了别的目录的 revision 不会出现在这里。
    const out = spliceHistory([svn(9), svn(4), svn(1)], [])
    expect(out[0].parents).toEqual(['svn:4'])
    expect(out[1].parents).toEqual(['svn:1'])
  })

  it('baseline commit 被替换掉，指向它的 git commit 改指 svn 节点', () => {
    // spec §10.3 ③：baseline commit 自身被替换掉，它在图上的位置由 SVN 序列决定。
    const out = spliceHistory([svn(3), svn(2)], [git('aaa', ['bbb']), baseline('bbb', ['ccc'], 3)])
    expect(out.find((c) => c.id === 'bbb')).toBeUndefined()
    expect(out.find((c) => c.id === 'aaa')!.parents).toEqual(['svn:3'])
  })

  it('孤儿 baseline —— 锚点没拉到时不留悬空父', () => {
    // 悬空父不会画出线（computePaths 里 `if (!parentNode) continue`），
    // 但 computeOwners 仍然拿它算归属，会把车道分配算歪。
    //
    // 锚点补拉走网络，断网时拿不到，所以这**不是**理论情况。
    const out = spliceHistory([svn(2)], [git('aaa', ['bbb']), baseline('bbb', [], 99)])
    const aaa = out.find((c) => c.id === 'aaa')!
    expect(aaa.parents).toEqual([])
    expect(out.every((c) => c.parents.every((p) => out.some((n) => n.id === p)))).toBe(true)
  })

  it('普通 git commit 之间的 parent 关系原样保留', () => {
    const out = spliceHistory([], [git('aaa', ['bbb']), git('bbb', [])])
    expect(out.find((c) => c.id === 'aaa')!.parents).toEqual(['bbb'])
  })

  it('合并提交的多个父各自替换', () => {
    const out = spliceHistory(
      [svn(5)],
      [git('merge', ['aaa', 'bbb']), git('aaa', []), baseline('bbb', [], 5)],
    )
    expect(out.find((c) => c.id === 'merge')!.parents).toEqual(['aaa', 'svn:5'])
  })

  it('localOnly 原样带过来，后端没标的归一成 false', () => {
    // 判定本身在 Rust 侧（`graph.rs` 的 parse_local_only，契约见
    // `src-tauri/tests/svn_graph_contract.rs`）。这里只保证接合不会把它弄丢，
    // 以及缺省归一成 false —— 角标按 falsy 渲染，undefined 虽然也不渲染，
    // 但它会让「后端没说」和「后端说了不是」在下游混成一谈。
    const out = spliceHistory([], [git('aaa', [], { localOnly: true }), git('bbb', [])])
    expect(out.find((c) => c.id === 'aaa')!.localOnly).toBe(true)
    expect(out.find((c) => c.id === 'bbb')!.localOnly).toBe(false)
  })

  it('git 节点标成 vcs=git', () => {
    const out = spliceHistory([], [git('aaa', [])])
    expect(out[0].vcs).toBe('git')
  })

  it('纯 git 仓库（svnRevs 为空）时拓扑与输入等价', () => {
    // spec §10.4：现有 Git 模式零影响。
    const input = [git('aaa', ['bbb']), git('bbb', ['ccc']), git('ccc', [])]
    const out = spliceHistory([], input)
    expect(out.map((c) => ({ id: c.id, parents: c.parents }))).toEqual(
      input.map((c) => ({ id: c.id, parents: c.parents })),
    )
  })

  it('svn 节点排在 git 节点之后，整体仍是「新的在前」', () => {
    // 工作分支的提交比基线新，图上要在上面。
    const out = spliceHistory([svn(2), svn(1)], [git('aaa', ['bbb']), baseline('bbb', [], 2)])
    expect(out.map((c) => c.id)).toEqual(['aaa', 'svn:2', 'svn:1'])
  })

  it('两个 baseline 指向同一个 revision 时都被替换', () => {
    // 重建基线会在 svn/base 上落新 commit，旧的仍在历史里。
    const out = spliceHistory(
      [svn(4)],
      [git('aaa', ['b1']), baseline('b1', ['b0'], 4), baseline('b0', [], 4)],
    )
    expect(out.find((c) => c.id === 'b1')).toBeUndefined()
    expect(out.find((c) => c.id === 'b0')).toBeUndefined()
    expect(out.find((c) => c.id === 'aaa')!.parents).toEqual(['svn:4'])
  })

  it('空输入返回空数组', () => {
    expect(spliceHistory([], [])).toEqual([])
  })
})

describe('baselineMap / remapRefs', () => {
  const refs = (items: Array<{ name: string; hash: string }>) =>
    items.map((r) => ({
      name: r.name,
      hash: r.hash,
      kind: 'branch' as const,
      ahead: null,
      behind: null,
    }))

  it('指向 baseline commit 的 ref 改指 svn 节点', () => {
    // **独立 review 的发现。** spliceHistory 把 baseline commit 从节点列表里删掉、
    // 并重写了 parents，但 refs 仍带着那个已经不存在的 oid。
    // GraphView 按 `refByHash[node.commit.id]` 渲染标签，于是 `svn/base`
    // 匹配不到任何一行、**整个看不见**。
    const commits = [git('aaa', ['bbb']), baseline('bbb', [], 4)]
    const map = baselineMap(commits)
    const out = remapRefs(refs([{ name: 'refs/heads/svn/base', hash: 'bbb' }]), map)
    expect(out[0].hash).toBe('svn:4')
  })

  it('指向普通 commit 的 ref 原样不动', () => {
    const map = baselineMap([git('aaa', []), baseline('bbb', [], 4)])
    const out = remapRefs(refs([{ name: 'refs/heads/work', hash: 'aaa' }]), map)
    expect(out[0].hash).toBe('aaa')
  })

  it('重复映射是幂等的', () => {
    // refs 只在 refreshGraph 里映射一次，但 svnRevs 变化会重接 commits。
    // 万一哪天有人在别处又调一次，不能把已经映射过的再映射坏。
    const map = baselineMap([baseline('bbb', [], 4)])
    const once = remapRefs(refs([{ name: 'x', hash: 'bbb' }]), map)
    expect(remapRefs(once, map)[0].hash).toBe('svn:4')
  })

  it('没有 baselineRev 的提交不进映射表', () => {
    // 「哪些 commit 算 baseline」由后端判（trailer 要逐行完整匹配，正文里顺口
    // 提到不算，见 `svn_graph_contract.rs`）。这里保证前端不自己再猜一遍 ——
    // 猜错的代价是一个普通提交连同它的分支线从图上凭空消失。
    const map = baselineMap([git('aaa', []), baseline('bbb', [], 7)])
    expect(map.get('aaa')).toBeUndefined()
    expect(map.get('bbb')).toBe('svn:7')
  })

  it('baselineRev 为 0 也算 baseline，不能被真值判断吃掉', () => {
    // `if (c.baselineRev)` 会把 0 当成「不是 baseline」。r0 在 SVN 里是空仓库的
    // 初始 revision，实际拿不到基线，但这条守的是**写法**：用真值判断读可选数字
    // 字段迟早会在别处咬人。
    const map = baselineMap([git('zero', [], { baselineRev: 0 })])
    expect(map.get('zero')).toBe('svn:0')
  })

  it('HEAD 落在 baseline 上时也能换算出图上的节点 id', () => {
    // 用户可以 checkout 到 svn/base（Task 13 明确放行）。不换算的话
    // headHash 在 commits 里找不到对应行 —— HEAD 标记不画、定位滚不过去。
    const map = baselineMap([baseline('bbb', [], 4)])
    expect(map.get('bbb') ?? 'bbb').toBe('svn:4')
  })
})

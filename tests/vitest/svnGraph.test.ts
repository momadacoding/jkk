import { describe, expect, it } from 'vitest'
import {
  baselineMap,
  commitDiffScope,
  remapRefs,
  selectionDiffScope,
  spliceHistory,
  type SvnRevNode,
} from '@/engine/svnGraph'
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

  it('git 与 svn 两条序列按时间交错，子仍在父之上', () => {
    // 2026-10-11 带分支的 fixture 实测：keep-a 指着已交付的 class A（06:26:30），它比 r2、r3
    // 都旧，却被 `[...git, ...svn]` 排在所有 svn 行上面，看起来像是在 r3 之后才提交的。
    const at = (hms: string) => Date.parse(`2026-10-11T${hms}Z`) / 1000
    const svnAt = (rev: number, hms: string): SvnRevNode => ({
      ...svn(rev),
      date: `2026-10-11T${hms}Z`,
    })
    const out = spliceHistory(
      [svnAt(3, '06:35:30'), svnAt(2, '06:30:47'), svnAt(1, '06:21:18')],
      // 后端 --date-order 的真实顺序
      [
        git('grow', ['many'], { time: at('23:10:51') }),
        git('feature', ['B3'], { time: at('23:10:51') }),
        git('local', ['B3'], { time: at('06:35:32') }),
        { ...baseline('B3', ['many'], 3), time: at('06:35:31') },
        git('many', ['B2'], { time: at('06:35:29') }),
        { ...baseline('B2', ['classA'], 2), time: at('06:30:48') },
        git('classA', ['B1'], { time: at('06:26:30') }),
        { ...baseline('B1', [], 1), time: at('06:21:19') },
      ],
      ['grow', 'feature', 'local', 'classA', 'B3'],
    )
    expect(out.map((c) => c.id)).toEqual([
      'grow', 'feature', 'local', 'svn:3', 'many', 'svn:2', 'classA', 'svn:1',
    ])
  })

  it('时间与拓扑冲突时拓扑优先：子提交的时钟比父还早也排在父上面', () => {
    // 本地时钟偏慢、或 SVN 服务器时钟偏快都会出现；子在父下面线就得往上画
    const out = spliceHistory(
      [{ ...svn(2), date: '2026-10-11T08:00:00Z' }],
      [
        git('w', ['B2'], { time: Date.parse('2026-10-11T07:00:00Z') / 1000 }),
        baseline('B2', [], 2),
      ],
    )
    expect(out.map((c) => c.id)).toEqual(['w', 'svn:2'])
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

describe('spliceHistory：已交付的本地提交（Phase 3a）', () => {
  // 真实形状（2026-10-11 fixture 实测 `git log --graph`）：交付录的新基线以交付树为父
  // （spec §8.2.2），所以交付过的提交夹在两条基线之间：
  //
  //   local(仅本地) → B3(svn: r3) → many → B2(svn: r2) → classA → B1(svn: r1)
  const delivered = () => [
    git('local', ['B3'], { localOnly: true }),
    baseline('B3', ['many'], 3),
    git('many', ['B2'], { message: 'many files' }),
    baseline('B2', ['classA'], 2),
    git('classA', ['B1'], { message: 'class A' }),
    baseline('B1', [], 1),
  ]

  it('只能经由基线才能到达的提交不进图——它们已经是 SVN 历史的一部分', () => {
    // 不隐藏的话，基线换成 svn 节点后它们没有子节点，被画成从旧 revision 分出、悬在
    // 那里的分支，还排在所有 svn 行的上面，看上去像「还没交付」（2026-10-11 真实窗口）
    const out = spliceHistory([svn(3), svn(2), svn(1)], delivered(), ['local'])
    expect(out.map((c) => c.id)).toEqual(['local', 'svn:3', 'svn:2', 'svn:1'])
  })

  it('svn 节点记下录这条基线之前交付的本地提交', () => {
    const out = spliceHistory([svn(3), svn(2), svn(1)], delivered(), ['local'])
    const at = (id: string) => out.find((c) => c.id === id)!
    expect(at('svn:3').deliveredFrom).toEqual([{ id: 'many', message: 'many files' }])
    expect(at('svn:2').deliveredFrom).toEqual([{ id: 'classA', message: 'class A' }])
    expect(at('svn:1').deliveredFrom).toBeUndefined()
  })

  it('一次交付了一串提交：整串都隐藏、都记在同一个 svn 节点上', () => {
    const out = spliceHistory(
      [svn(2), svn(1)],
      [
        git('top', ['B2']),
        baseline('B2', ['c2'], 2),
        git('c2', ['c1'], { message: 'second' }),
        git('c1', ['B1'], { message: 'first' }),
        baseline('B1', [], 1),
      ],
      ['top'],
    )
    expect(out.map((c) => c.id)).toEqual(['top', 'svn:2', 'svn:1'])
    expect(out.find((c) => c.id === 'svn:2')!.deliveredFrom!.map((d) => d.id)).toEqual(['c2', 'c1'])
  })

  it('还有分支（或 HEAD）指着的交付提交照常显示——不能把用户的分支藏掉', () => {
    const out = spliceHistory([svn(3), svn(2), svn(1)], delivered(), ['local', 'classA'])
    expect(out.some((c) => c.id === 'classA')).toBe(true)
    expect(out.some((c) => c.id === 'many')).toBe(false)
    expect(out.find((c) => c.id === 'svn:2')!.deliveredFrom).toBeUndefined()
  })

  it('另一条分支从交付提交上长出来：它和它的祖先都照常显示', () => {
    const out = spliceHistory(
      [svn(2), svn(1)],
      [
        git('top', ['B2']),
        git('side', ['c1']),
        baseline('B2', ['c1'], 2),
        git('c1', ['B1']),
        baseline('B1', [], 1),
      ],
      ['top', 'side'],
    )
    expect(out.some((c) => c.id === 'c1')).toBe(true)
    expect(out.find((c) => c.id === 'side')!.parents).toEqual(['c1'])
  })

  it('基线对应的 revision 还没拉进来时不收起：收进一个不画的节点等于整个看不见', () => {
    // 最终 review 第 6 条：首次打开 svn log 缓存为空、或只翻了一页时 r2 不在 svnRevs 里
    const out = spliceHistory([svn(3), svn(1)], delivered(), ['local'])
    expect(out.some((c) => c.id === 'classA')).toBe(true)
    expect(out.some((c) => c.id === 'many')).toBe(false) // r3 在，照常收起
    expect(out.find((c) => c.id === 'svn:3')!.deliveredFrom).toEqual([{ id: 'many', message: 'many files' }])
  })

  it('「更新」录的基线以旧基线为父，什么都不隐藏', () => {
    const out = spliceHistory(
      [svn(2), svn(1)],
      [git('top', ['B2']), baseline('B2', ['B1'], 2), baseline('B1', [], 1)],
      ['top'],
    )
    expect(out.map((c) => c.id)).toEqual(['top', 'svn:2', 'svn:1'])
    expect(out.every((c) => c.deliveredFrom === undefined)).toBe(true)
  })
})

describe('spliceHistory：svn 节点的本地基线（用来本地算文件变化）', () => {
  it('带上自己的基线 oid，以及图上下一个节点的基线 oid', () => {
    const out = spliceHistory(
      [svn(3), svn(2), svn(1)],
      [git('top', ['B3']), baseline('B3', ['B2'], 3), baseline('B2', ['B1'], 2), baseline('B1', [], 1)],
      ['top'],
    )
    const at = (id: string) => out.find((c) => c.id === id)!
    expect([at('svn:3').baselineOid, at('svn:3').diffBase]).toEqual(['B3', 'B2'])
    expect([at('svn:2').baselineOid, at('svn:2').diffBase]).toEqual(['B2', 'B1'])
    // 最早的节点没有可比的上一版
    expect([at('svn:1').baselineOid, at('svn:1').diffBase]).toEqual(['B1', undefined])
  })

  it('上一个节点没有本地基线（同事的 revision）：不给 diffBase，不能拿更早的基线冒充', () => {
    // r1 → r3 之间夹着同事的 r2：B1..B3 的差异是 r2 + r3 两次的，不是 r3 一次的
    const out = spliceHistory(
      [svn(3), svn(2), svn(1)],
      [git('top', ['B3']), baseline('B3', ['B1'], 3), baseline('B1', [], 1)],
      ['top'],
    )
    const at = (id: string) => out.find((c) => c.id === id)!
    expect(at('svn:3').baselineOid).toBe('B3')
    expect(at('svn:3').diffBase).toBeUndefined()
    expect(at('svn:2').baselineOid).toBeUndefined()
  })

  it('同一个 revision 有两条基线（重建过）时取最新那条', () => {
    const out = spliceHistory(
      [svn(4)],
      [git('aaa', ['b1']), baseline('b1', ['b0'], 4), baseline('b0', [], 4)],
      ['aaa'],
    )
    expect(out.find((c) => c.id === 'svn:4')!.baselineOid).toBe('b1')
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

describe('commitDiffScope', () => {
  it('git 节点照旧读这个 commit 的 diff', () => {
    expect(commitDiffScope(git('abc', ['p']))).toBe('commit:abc')
  })

  it('有前后两版本地基线的 svn 节点：比两版基线', () => {
    const node = { ...git('svn:2', ['svn:1']), vcs: 'svn' as const, baselineOid: 'B2', diffBase: 'B1' }
    expect(commitDiffScope(node)).toBe('range:B1..B2')
  })

  it('没有可比基线的 svn 节点不请求 diff（id 不是 oid，发出去只会报错）', () => {
    expect(commitDiffScope({ ...git('svn:2', []), vcs: 'svn' as const })).toBeNull()
    expect(commitDiffScope({ ...git('svn:2', []), vcs: 'svn' as const, baselineOid: 'B2' })).toBeNull()
  })

  it('没选中任何节点时不请求', () => {
    expect(commitDiffScope(undefined)).toBeNull()
  })
})

describe('selectionDiffScope', () => {
  const commits = [
    git('abc', ['svn:2']),
    { ...git('svn:2', []), vcs: 'svn' as const, baselineOid: 'B2', diffBase: 'B1' },
  ]

  it('图上的节点按 commitDiffScope 读', () => {
    expect(selectionDiffScope(commits, 'abc')).toBe('commit:abc')
    expect(selectionDiffScope(commits, 'svn:2')).toBe('range:B1..B2')
  })

  it('不在图上的选中（stash、5000 条之外的提交）照旧按 oid 读——普通仓库也走这里', () => {
    // 最终 review 第 1 条：只认图上节点的话，点 stash 不再读 diff，详情停在上一条提交上
    expect(selectionDiffScope(commits, 'stash0')).toBe('commit:stash0')
    expect(selectionDiffScope([], 'stash0')).toBe('commit:stash0')
  })

  it('不在图上的 svn 节点 id 不是 oid，不请求', () => {
    expect(selectionDiffScope(commits, 'svn:9')).toBeNull()
  })

  it('没有选中时不请求', () => {
    expect(selectionDiffScope(commits, null)).toBeNull()
  })
})

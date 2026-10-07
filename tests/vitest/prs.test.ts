import { describe, expect, it } from 'vitest'
import { indexPullRequestsByBranch, prBadge, pullRequestForRef } from '../../src/engine/prs'
import type { PullRequest, RefPointer } from '../../src/engine/types'

const pr = (number: number, headRefName: string, extra: Partial<PullRequest> = {}): PullRequest => ({
  number,
  headRefName,
  title: `PR ${number}`,
  isDraft: false,
  state: 'OPEN',
  headRefOid: 'f'.repeat(40),
  url: `https://github.com/o/r/pull/${number}`,
  ...extra,
})
const merged = (number: number, headRefName: string, headRefOid = 'f'.repeat(40)): PullRequest =>
  pr(number, headRefName, { state: 'MERGED', headRefOid })
const closed = (number: number, headRefName: string): PullRequest =>
  pr(number, headRefName, { state: 'CLOSED' })

describe('indexPullRequestsByBranch', () => {
  it('空数组给出空索引，不抛异常', () => {
    expect(indexPullRequestsByBranch([]).size).toBe(0)
  })

  it('按 headRefName 建索引', () => {
    const index = indexPullRequestsByBranch([pr(36, 'fix/watch'), pr(35, 'feat/tab')])
    expect(index.get('fix/watch')?.number).toBe(36)
    expect(index.get('feat/tab')?.number).toBe(35)
  })

  it('分支名里的斜杠原样保留，不做任何切分', () => {
    const index = indexPullRequestsByBranch([pr(7, 'feat/a/b/c')])
    expect(index.get('feat/a/b/c')?.number).toBe(7)
    expect(index.get('feat')).toBeUndefined()
  })

  it('同一分支有多个 open PR 时取编号最大的那个', () => {
    // GitHub 允许同一个 head 分支同时开多个 PR（指向不同 base）。
    // 徽章只有一个位置，取最新的那条最贴近「我刚开的 PR」。
    const index = indexPullRequestsByBranch([pr(10, 'feat/x'), pr(42, 'feat/x'), pr(31, 'feat/x')])
    expect(index.get('feat/x')?.number).toBe(42)
  })

  it('顺序颠倒也取编号最大的，不依赖 gh 的返回顺序', () => {
    const index = indexPullRequestsByBranch([pr(42, 'feat/x'), pr(10, 'feat/x')])
    expect(index.get('feat/x')?.number).toBe(42)
  })

  it('大小写不同视为不同分支，不做大小写折叠', () => {
    // GitHub 的分支名是大小写敏感的，折叠会把 Feat/X 的 PR 错挂到 feat/x 上。
    // 宁可少显示一个徽章，也不能指着错的 PR。
    const index = indexPullRequestsByBranch([pr(5, 'Feat/X')])
    expect(index.get('feat/x')).toBeUndefined()
    expect(index.get('Feat/X')?.number).toBe(5)
  })

  it('headRefName 为空或只有空白的条目直接丢掉', () => {
    // 空键会和「分支名为空」的查询意外命中，而后者本身就是坏数据
    const index = indexPullRequestsByBranch([pr(1, ''), pr(2, '   '), pr(3, 'ok')])
    expect(index.size).toBe(1)
    expect(index.get('ok')?.number).toBe(3)
  })

  it('保留 draft 标记和 url，徽章要用', () => {
    const index = indexPullRequestsByBranch([pr(9, 'feat/d', { isDraft: true })])
    expect(index.get('feat/d')?.isDraft).toBe(true)
    expect(index.get('feat/d')?.url).toBe('https://github.com/o/r/pull/9')
  })

  it('已合并的 PR 也进索引——分支还在但内容已进主干，这正是要标出来的情况', () => {
    const index = indexPullRequestsByBranch([merged(33, 'feat/portable-zip-signing')])
    expect(index.get('feat/portable-zip-signing')?.number).toBe(33)
  })

  it('关闭但未合并的 PR 不进索引', () => {
    // 被否掉的 PR 对「这条分支现在什么状态」没有信息量，标出来只会误导
    expect(indexPullRequestsByBranch([closed(12, 'feat/abandoned')]).size).toBe(0)
  })

  it('同一分支同时有 open 和 merged 时，open 优先——哪怕它编号更小', () => {
    // 老分支合过一次又重开 PR：当下正在进行的那个才是用户关心的
    const index = indexPullRequestsByBranch([merged(50, 'feat/x'), pr(12, 'feat/x')])
    expect(index.get('feat/x')?.number).toBe(12)
    expect(index.get('feat/x')?.state).toBe('OPEN')
  })

  it('顺序颠倒也是 open 优先，不依赖 gh 的返回顺序', () => {
    const index = indexPullRequestsByBranch([pr(12, 'feat/x'), merged(50, 'feat/x')])
    expect(index.get('feat/x')?.number).toBe(12)
  })

  it('全是 merged 时取编号最大的那个', () => {
    const index = indexPullRequestsByBranch([merged(10, 'feat/x'), merged(42, 'feat/x')])
    expect(index.get('feat/x')?.number).toBe(42)
  })

  it('closed 不会顶掉同分支的 merged', () => {
    const index = indexPullRequestsByBranch([closed(99, 'feat/x'), merged(10, 'feat/x')])
    expect(index.get('feat/x')?.number).toBe(10)
  })
})

describe('prBadge', () => {
  const MERGED_AT = 'ac6e785eb547aadf74ff83993fe44b231aaf9b8a'
  const MOVED_ON = '6e6f014513cd3cc520cfa3838228517af47ad3ef'

  it('open PR：绿色，只显示编号', () => {
    const badge = prBadge(pr(37, 'a'), MERGED_AT)
    expect(badge.variant).toBe('open')
    expect(badge.label).toBe('#37')
  })

  it('draft PR：灰色，只显示编号', () => {
    expect(prBadge(pr(37, 'a', { isDraft: true }), MERGED_AT).variant).toBe('draft')
  })

  it('merged 压过 isDraft——已合并就不再是草稿了', () => {
    // gh 不该给出这种组合，但真给了也不能显示成灰色草稿
    expect(prBadge(pr(1, 'a', { state: 'MERGED', isDraft: true })).variant).toBe('merged')
  })

  it('分支头就停在合并点上：✓ 前缀，表示内容全在主干', () => {
    // 颜色不能是唯一的区分手段：pill 底色是车道色，紫字在黄色车道上淡到
    // 与 draft 灰分不开（实测最高 4.41）；符号这一路对色盲也成立
    const badge = prBadge(merged(33, 'a', MERGED_AT), MERGED_AT)
    expect(badge.variant).toBe('merged')
    expect(badge.label).toBe('✓#33')
  })

  /** 后端对着 `tip` 算出的本地分支与合并点的位置。 */
  const relatedAt = (tip: string, ahead: number, behind: number): Partial<PullRequest> =>
    ({ localRelation: { tip, ahead, behind } })
  const mergedWith = (number: number, rel: Partial<PullRequest>): PullRequest =>
    pr(number, 'a', { state: 'MERGED', headRefOid: MERGED_AT, ...rel })

  it('PR 合并后分支又长了提交：必须区别对待，否则照着 ✓ 删分支会丢东西', () => {
    // 真实案例：PR #20 合并于 ac6e785，之后分支上又加了 6e6f014，
    // 而那个提交不在 master 里。画成 ✓ 等于告诉用户「可以删了」。
    const badge = prBadge(mergedWith(20, relatedAt(MOVED_ON, 1, 0)), MOVED_ON)
    expect(badge.variant).toBe('merged-ahead')
    expect(badge.label).toBe('✓#20↑')
  })

  it('merged-ahead 的提示要把两个 SHA 和多出的提交数都说出来', () => {
    const tip = prBadge(mergedWith(20, relatedAt(MOVED_ON, 3, 0)), MOVED_ON).tooltip
    expect(tip).toContain('ac6e785')
    expect(tip).toContain('6e6f014')
    expect(tip).toContain('3 个提交')
  })

  it('本地落后于合并点：不能画成 ↑，内容其实全在 PR 里', () => {
    // 真实案例：本地 worktree-ios-kickoff 停在 736890d，PR #321 合并的 93e3d22 是它的子提交
    // （在别处推上去、本地没拉）。旧徽章显示 ✓#321↑「本地已经走到 736890d」，正好反了。
    const local = '736890db9c9e750180a974f2a90a82f02e6cc483'
    const badge = prBadge(
      pr(321, 'a', { state: 'MERGED', headRefOid: '93e3d222de3626f1f95b6e5de682b6cc22c48278', ...relatedAt(local, 0, 1) }),
      local,
    )
    expect(badge.variant).toBe('merged-behind')
    expect(badge.label).toBe('✓#321↓')
    expect(badge.tooltip).toContain('93e3d22')
    expect(badge.tooltip).toContain('736890d')
    expect(badge.tooltip).not.toContain('删分支前先确认')
  })

  it('两边分叉：本地有 PR 外的提交，PR 里也有本地没有的', () => {
    const badge = prBadge(mergedWith(20, relatedAt(MOVED_ON, 2, 1)), MOVED_ON)
    expect(badge.variant).toBe('merged-diverged')
    expect(badge.label).toBe('✓#20↑↓')
    expect(badge.tooltip).toContain('删分支前先确认')
  })

  it('后端没给结论（合并点对象本地没有）：无法判断，仍然提醒确认', () => {
    // 不能退回普通 ✓——SHA 对不上时，「合并后又提交了」是可能的，那会丢东西
    const badge = prBadge(mergedWith(20, { localRelation: null }), MOVED_ON)
    expect(badge.variant).toBe('merged-unknown')
    expect(badge.label).toBe('✓#20?')
    expect(badge.tooltip).toContain('删分支前先确认')
  })

  it('字段整个缺失（老后端）同样是无法判断', () => {
    expect(prBadge(merged(20, 'a', MERGED_AT), MOVED_ON).variant).toBe('merged-unknown')
  })

  it('结论是对着旧的分支头算的：分支已经动过，不再采信', () => {
    // PR 列表只在 fetch/pull/push 后刷新，本地分支却随时会动。
    // 后端算的时候本地落后，之后本地又提交了——这时再显示 ↓ 就是在说谎。
    const badge = prBadge(mergedWith(20, relatedAt(MERGED_AT.replace('a', 'b'), 0, 1)), MOVED_ON)
    expect(badge.variant).toBe('merged-unknown')
  })

  it('比对分支头时不分大小写', () => {
    expect(prBadge(mergedWith(20, relatedAt(MOVED_ON.toUpperCase(), 0, 1)), MOVED_ON).variant)
      .toBe('merged-behind')
  })

  it('SHA 已相等时不看 localRelation，直接是 merged', () => {
    expect(prBadge(mergedWith(33, { localRelation: null }), MERGED_AT).variant).toBe('merged')
  })

  it('拿不到分支头时退回普通 merged，不瞎猜', () => {
    expect(prBadge(merged(33, 'a', MERGED_AT)).variant).toBe('merged')
  })

  it('gh 没给 headRefOid 时退回普通 merged', () => {
    // 老版本 gh 或字段缺失：宁可少一个提示，也不能凭空报警
    expect(prBadge(merged(33, 'a', ''), MOVED_ON).variant).toBe('merged')
  })

  it('SHA 比较不分大小写', () => {
    expect(prBadge(merged(33, 'a', MERGED_AT.toUpperCase()), MERGED_AT).variant).toBe('merged')
  })

  it('open PR 的分支头对不上也还是 open', () => {
    // 那种情况已经由分支行上的 ahead/behind 徽章表达了，不重复报警
    expect(prBadge(pr(37, 'a', { headRefOid: MERGED_AT }), MOVED_ON).variant).toBe('open')
  })
})

describe('pullRequestForRef', () => {
  const index = indexPullRequestsByBranch([pr(37, 'feat/x'), pr(9, 'master')])
  const ref = (name: string, kind: RefPointer['kind']): RefPointer =>
    ({ name, kind, hash: 'a'.repeat(40), ahead: null, behind: null })

  it('本地分支剥掉前缀后精确命中', () => {
    expect(pullRequestForRef(index, ref('refs/heads/feat/x', 'branch'))?.number).toBe(37)
  })

  it('远程分支不给编号——同一行上本地 pill 已经标过了，再标一次是重复噪音', () => {
    // origin/feat/x 和 feat/x 常常并排出现在同一个提交上
    expect(pullRequestForRef(index, ref('refs/remotes/origin/feat/x', 'remote'))).toBeUndefined()
  })

  it('tag 不给编号', () => {
    expect(pullRequestForRef(index, ref('refs/tags/v1.0.0', 'tag'))).toBeUndefined()
  })

  it('HEAD 伪 ref 不给编号', () => {
    expect(pullRequestForRef(index, ref('HEAD', 'head'))).toBeUndefined()
  })

  it('没有对应 PR 的分支返回 undefined', () => {
    expect(pullRequestForRef(index, ref('refs/heads/chore/no-pr', 'branch'))).toBeUndefined()
  })

  it('前缀只剥一次，分支名里再出现同样的字样也原样保留', () => {
    // 病态但合法的分支名，不能被多剥一层去撞到别的分支
    const weird = indexPullRequestsByBranch([pr(1, 'refs/heads/x')])
    expect(pullRequestForRef(weird, ref('refs/heads/refs/heads/x', 'branch'))?.number).toBe(1)
  })

  it('空索引上安全返回 undefined', () => {
    expect(pullRequestForRef(new Map(), ref('refs/heads/feat/x', 'branch'))).toBeUndefined()
  })
})

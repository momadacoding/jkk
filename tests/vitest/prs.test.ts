import { describe, expect, it } from 'vitest'
import {
  groupPullRequestsByHead,
  indexPullRequestsByBranch,
  prBadge,
  pullRequestForRef,
} from '../../src/engine/prs'
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

describe('prBadge：叠 PR 合进了已经合并的父分支', () => {
  // 真实案例（readwise-alternative）：#397 叠在 #396 上，base 是 #396 的分支。
  // #396 在 17:08 合进 main，#397 在 17:20 才合——合进的是那条已经没用的分支，改动没有进 main，
  // 可徽章照样是 ✓。
  const PARENT_HEAD = 'worktree-fix-mobile-api-failure-kind'
  const CHILD_HEAD = 'worktree-fix-refresh-truncated-body'
  const CHILD_OID = 'bb9bb65ad2b3b43bd7a24e343641f2f24a8bc752'
  const parent = (extra: Partial<PullRequest> = {}): PullRequest =>
    pr(396, PARENT_HEAD, {
      state: 'MERGED',
      baseRefName: 'main',
      mergedAt: '2026-11-02T17:08:07Z',
      ...extra,
    })
  const child = (extra: Partial<PullRequest> = {}): PullRequest =>
    pr(397, CHILD_HEAD, {
      state: 'MERGED',
      headRefOid: CHILD_OID,
      baseRefName: PARENT_HEAD,
      mergedAt: '2026-11-02T17:20:38Z',
      ...extra,
    })
  const badgeOf = (c: PullRequest, p: PullRequest, tip = CHILD_OID) =>
    prBadge(c, tip, groupPullRequestsByHead([p, c]))
  const at = (hhmm: string) => `2026-11-02T${hhmm}:00Z`

  it('父 PR 比它先合并：改动困在父分支上，不能再画成普通 ✓', () => {
    const badge = badgeOf(child(), parent())
    expect(badge.variant).toBe('merged-stranded')
    expect(badge.label).toBe('✓#397!')
  })

  it('提示要说出合进了哪条分支、是哪个 PR 的、没进的是哪条分支，并提醒删分支前确认', () => {
    const tip = badgeOf(child(), parent()).tooltip
    expect(tip).toContain(PARENT_HEAD)
    // 父 PR 合进的不一定是默认分支，要说出它实际的 base，而不是笼统的「主干」
    expect(tip).toContain('没有随 #396 进入 main')
    expect(tip).toContain('删分支前先确认')
  })

  it('父 PR 自己的徽章不受影响', () => {
    const p = parent({ headRefOid: CHILD_OID })
    expect(prBadge(p, CHILD_OID, groupPullRequestsByHead([p, child()])).variant).toBe('merged')
  })

  it('父分支上有多个 PR 时，只要有一个在它之后合并就算进去了——不能只看显示用的那一个', () => {
    // 显示索引会挑编号最大的 #150（合进 release、比子 PR 早），据此会误报
    const toMain = pr(100, 'x', { state: 'MERGED', baseRefName: 'main', mergedAt: at('18:00') })
    const toRelease = pr(150, 'x', { state: 'MERGED', baseRefName: 'release', mergedAt: at('10:00') })
    const c = child({ baseRefName: 'x', mergedAt: at('12:00') })
    expect(prBadge(c, CHILD_OID, groupPullRequestsByHead([toMain, toRelease, c])).variant)
      .toBe('merged')
  })

  it('父分支上还有开着的 PR：改动会随它合进去，即使那条分支更早合过一次', () => {
    const earlier = pr(100, 'x', { state: 'MERGED', baseRefName: 'main', mergedAt: at('09:00') })
    const reopened = pr(200, 'x', { state: 'OPEN', baseRefName: 'main', mergedAt: null })
    const c = child({ baseRefName: 'x', mergedAt: at('12:00') })
    expect(prBadge(c, CHILD_OID, groupPullRequestsByHead([earlier, reopened, c])).variant)
      .toBe('merged')
  })

  it('多层叠 PR：父 PR 晚于它合并，但父 PR 自己也困住了，它跟着一起困住', () => {
    // A→main 10:00 合并；C→B 11:00 合并；B→A 12:00 合并（A 已经合过了，B 被困住）
    const a = pr(1, 'a', { state: 'MERGED', baseRefName: 'main', mergedAt: at('10:00') })
    const b = pr(2, 'b', { state: 'MERGED', baseRefName: 'a', mergedAt: at('12:00') })
    const c = pr(3, 'c', { state: 'MERGED', baseRefName: 'b', mergedAt: at('11:00'), headRefOid: CHILD_OID })
    const badge = prBadge(c, CHILD_OID, groupPullRequestsByHead([a, b, c]))
    expect(badge.variant).toBe('merged-stranded')
    expect(badge.tooltip).toContain('#2')
    expect(badge.tooltip).toContain('没有随 #1 进入 main')
  })

  it('多层叠 PR 一路按顺序合并：全都进去了', () => {
    const a = pr(1, 'a', { state: 'MERGED', baseRefName: 'main', mergedAt: at('12:00') })
    const b = pr(2, 'b', { state: 'MERGED', baseRefName: 'a', mergedAt: at('11:00') })
    const c = pr(3, 'c', { state: 'MERGED', baseRefName: 'b', mergedAt: at('10:00'), headRefOid: CHILD_OID })
    expect(prBadge(c, CHILD_OID, groupPullRequestsByHead([a, b, c])).variant).toBe('merged')
  })

  it('base 互相指向的坏数据不会死循环', () => {
    const a = pr(1, 'a', { state: 'MERGED', baseRefName: 'b', mergedAt: at('12:00') })
    const b = pr(2, 'b', { state: 'MERGED', baseRefName: 'a', mergedAt: at('11:00'), headRefOid: CHILD_OID })
    expect(prBadge(b, CHILD_OID, groupPullRequestsByHead([a, b])).variant).toBe('merged')
  })

  it('同一秒合并分不出先后，不报警', () => {
    expect(badgeOf(child(), parent({ mergedAt: '2026-11-02T17:20:38Z' })).variant).toBe('merged')
  })

  it('困住的同时本地分支又有 PR 外的提交：两件事都要说', () => {
    const moved = '6e6f014513cd3cc520cfa3838228517af47ad3ef'
    const c = child({ localRelation: { tip: moved, ahead: 3, behind: 0 } })
    const tip = badgeOf(c, parent(), moved).tooltip
    expect(tip).toContain('没有随 #396 进入 main')
    expect(tip).toContain('3 个提交不在这个 PR 里')
  })

  it('关闭未合并的 PR 不当父 PR', () => {
    expect(badgeOf(child(), parent({ state: 'CLOSED' })).variant).toBe('merged')
  })

  it('正常的叠 PR 流程：子 PR 先合进父分支、父 PR 后合进主干，内容都进去了', () => {
    const badge = badgeOf(child(), parent({ mergedAt: '2026-11-02T18:00:00Z' }))
    expect(badge.variant).toBe('merged')
  })

  it('父 PR 还开着：改动会随它一起合进去，不报警', () => {
    expect(badgeOf(child(), parent({ state: 'OPEN', mergedAt: null })).variant).toBe('merged')
  })

  it('fork 的 PR 不算父 PR——它的 head 常常也叫 main', () => {
    const forkMain = pr(500, 'main', {
      state: 'MERGED',
      isCrossRepository: true,
      mergedAt: '2026-01-01T00:00:00Z',
    })
    const normal = child({ baseRefName: 'main' })
    expect(badgeOf(normal, forkMain).variant).toBe('merged')
  })

  it('任一边缺 mergedAt（老版本 gh）时不判断，不凭空报警', () => {
    expect(badgeOf(child({ mergedAt: undefined }), parent()).variant).toBe('merged')
    expect(badgeOf(child(), parent({ mergedAt: null })).variant).toBe('merged')
  })

  it('mergedAt 解析不了时同样不判断', () => {
    expect(badgeOf(child({ mergedAt: 'not a date' }), parent()).variant).toBe('merged')
  })

  it('base 是自己的 head 分支（坏数据）时不把自己当父 PR', () => {
    const self = child({ baseRefName: CHILD_HEAD, mergedAt: '2026-11-02T17:20:38Z' })
    expect(prBadge(self, CHILD_OID, groupPullRequestsByHead([self])).variant).toBe('merged')
  })

  it('困在父分支上压过 ↑ 等形态：最要紧的是改动不在主干', () => {
    const moved = '6e6f014513cd3cc520cfa3838228517af47ad3ef'
    const c = child({ localRelation: { tip: moved, ahead: 1, behind: 0 } })
    expect(badgeOf(c, parent(), moved).variant).toBe('merged-stranded')
  })

  it('不传索引时保持原来的判定', () => {
    expect(prBadge(child(), CHILD_OID).variant).toBe('merged')
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

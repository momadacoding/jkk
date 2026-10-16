import { describe, expect, it } from 'vitest'
import {
  describeUnfinishedSync,
  needsLockPolling,
  otherBannersAreSuppressed,
  sinceText,
  SYNC_LOCK_POLL_MS,
  type SyncPhase,
  type SyncState,
  type SyncStateUnreadable,
  type UnfinishedSync,
} from '@/engine/svnJournal'

const clean: SyncState = { locked: false, holder: null, unfinished: null }

describe('describeUnfinishedSync', () => {
  it('还没读到状态时不挂任何东西', () => {
    // 打开仓库那一瞬间它是 null。挂一条横幅等于每次打开都闪一下。
    expect(describeUnfinishedSync(null)).toBeNull()
  })

  it('干净时不挂', () => {
    expect(describeUnfinishedSync(clean)).toBeNull()
  })

  it('正在同步时说的是「进行中」，不是「未完成」', () => {
    // 这两件事的出口完全相反：进行中要等，未完成要用户决定继续还是放弃。
    // 说错了会引导用户去「放弃」一次正在正常跑的同步。
    const b = describeUnfinishedSync({
      locked: true,
      holder: { pid: 4242, startedAt: 1757000000, opid: 'abc' },
      unfinished: { flow: 'sync_in', phase: 'rebase', opid: 'abc', startedAt: 1757000000 },
    })!
    expect(b).not.toBeNull()
    expect(b.title).toContain('正在进行')
    expect(b.actions).toEqual([])
    expect(b.detail).toContain('4242')
  })

  it('锁没人拿着但 journal 还在 ⇒ 上次没收尾', () => {
    const b = describeUnfinishedSync({
      locked: false,
      holder: null,
      unfinished: { flow: 'sync_in', phase: 'stashed', opid: 'abc', startedAt: 1757000000 },
    })!
    expect(b.tone).toBe('danger')
    expect(b.title).toContain('没有正常结束')
  })

  it('stashed 阶段必须告诉用户改动被压进了 stash', () => {
    // spec §7.3.1：这正是写前日志存在的理由。用户看到工作区空了，
    // 界面必须立刻说清东西在哪，而不是让他以为改动没了。
    const b = describeUnfinishedSync({
      locked: false,
      holder: null,
      unfinished: { flow: 'sync_in', phase: 'stashed', opid: 'abc', startedAt: 1757000000 },
    })!
    expect(b.detail).toContain('stash')
  })

  it('sync_in 未收尾时给出继续 / 放弃按钮', () => {
    const b = describeUnfinishedSync({
      locked: false,
      holder: null,
      unfinished: { flow: 'sync_in', phase: 'stashed', opid: 'abc', startedAt: 1757000000 },
    })!
    expect(b.actions.map((a) => a.id)).toEqual(['syncContinue', 'syncAbort'])
  })

  it('sync_out 留下的记录挂交付那一组按钮，绝不挂 sync_in 的继续 / 放弃', () => {
    // `publishing-baseline` 是两条流程共用的阶段名（spec §7.6）：只看 phase 的话，
    // sync_out 的记录会挂上 sync_in 的继续 / 放弃按钮——那两个命令会拒绝交付记录。
    // Phase 3a 之前这里断言「不给按钮」，交付的恢复入口落地后改为按 flow 分流
    const at = (phase: string) =>
      describeUnfinishedSync({
        locked: false,
        holder: null,
        unfinished: { flow: 'sync_out', phase: phase as SyncPhase, opid: 'x', startedAt: 1 },
      })!
    expect(at('publishing-baseline').actions.map((a) => a.id)).toEqual(['syncOutContinue', 'syncOutAbort'])
    expect(at('svn-staged').actions.map((a) => a.id)).toEqual(['syncOutCleanup'])
    // 「不会重复提交」只对已经提交的 phase 有意义；提交之前讲它是跑题（2026-10-14 用户反馈）
    expect(at('publishing-baseline').detail).toContain('不会重复提交')
    expect(at('svn-staged').note).not.toContain('重复提交')
  })

  it('交付留下的记录：标题、正文、按钮都按 sync_out 的 phase 给', () => {
    const b = describeUnfinishedSync({
      locked: false,
      holder: null,
      unfinished: { flow: 'sync_out', phase: 'svn-staged', opid: 'op', startedAt: 1 },
    })!
    expect(b.title).toBe('交付被打断了，还没提交')
    expect(b.detail).toContain('登记')
    expect(b.actions.map((a) => a.id)).toEqual(['syncOutCleanup'])
  })

  // 2026-10-14 E2E 场景 7b 用户反馈「这个提示好难理解」：原文是实现层的说法（「新基线可能已经
  // 发布」「分支还没接上去」），第二行是所有 phase 共用的说明（提交之后停下时也讲「提交之前
  // 停下的…」，还把按钮叫成「继续」），两个按钮点了会怎样没说。
  // 改成：标题回答「交上去没有」，正文回答「安全吗、还差什么」，note 写每个按钮的后果。
  describe('交付横幅说人话', () => {
    const banner = (phase: string, extra: Partial<UnfinishedSync> = {}) =>
      describeUnfinishedSync({
        locked: false,
        holder: null,
        unfinished: { flow: 'sync_out', phase: phase as SyncPhase, opid: 'op', startedAt: 1, ...extra },
      })!
    const all = (b: ReturnType<typeof banner>) => `${b.title}\n${b.detail}\n${b.note ?? ''}`

    it('提交之前：标题说还没提交，note 只讲「清理」', () => {
      for (const phase of ['prepare', 'building-deliver', 'deliver-built', 'checked-out', 'svn-staged']) {
        const b = banner(phase)
        expect(b.title, phase).toBe('交付被打断了，还没提交')
        expect(b.detail, phase).toContain('SVN 上')
        expect(b.note, phase).toContain('「清理」')
        expect(all(b), phase).not.toContain('「继续')
        expect(all(b), phase).not.toContain('提交之后')
      }
      expect(banner('checked-out').detail).toContain('本地补丁暂时不在磁盘上')
    })

    it('提交之后：标题带 revision，正文说不会丢、不会重复提交，note 讲两个按钮各自的后果', () => {
      for (const phase of ['post-commit', 'publishing-baseline', 'restoring-branch']) {
        const b = banner(phase, { committedRev: 11 })
        expect(b.title, phase).toContain('交付已提交（r11）')
        expect(b.detail, phase).toContain('不会丢')
        expect(b.detail, phase).toContain('不会重复提交')
        expect(b.note, phase).toContain('「停止收尾」')
        expect(b.note, phase).toContain('「更新」')
        // 不再出现与本 phase 无关的「提交之前停下的…」，也不说实现层的「基线」「发布」
        expect(all(b), phase).not.toContain('提交之前')
        expect(all(b), phase).not.toContain('基线')
        expect(all(b), phase).not.toContain('发布')
      }
      expect(banner('publishing-baseline', { committedRev: 11 }).note).toContain('「继续交付」')
      expect(banner('post-commit', { committedRev: 11 }).note).toContain('需要网络')
      expect(banner('restoring-branch', { committedRev: 11 }).note).toContain('「我已解决，继续」')
    })

    it('note 里点名的按钮正是横幅上那几个', () => {
      for (const phase of ['svn-staged', 'post-commit', 'publishing-baseline', 'restoring-branch']) {
        const b = banner(phase, { committedRev: 11 })
        for (const a of b.actions) expect(b.note, `${phase} ${a.label}`).toContain(`「${a.label}」`)
      }
    })

    it('点名原分支，说清还差的是「切回它、把它更新到最新」，中间位置别在上面改代码', () => {
      // 2026-10-14 E2E 场景 8：从 e2e8 交付、停在 publishing-baseline，工具栏显示 svn/base。
      // 用户先问「现在的工作分支是 svn/base 吗？」，看了「把分支 e2e8 接到最新版本上」又问
      // 「e2e8 不是已经提交到 SVN 了么？接上去是干啥？」——「接上去」是实现层的说法
      for (const phase of ['post-commit', 'publishing-baseline', 'restoring-branch']) {
        const b = banner(phase, { committedRev: 12, origBranch: 'e2e8' })
        expect(b.detail, phase).toContain('e2e8 的改动已经在 SVN 上了')
        expect(b.detail, phase).toContain('不要在这里改代码')
        expect(b.note, phase).toContain('切回 e2e8')
        expect(all(b), phase).toContain('更新到最新版本')
        expect(all(b), phase).not.toMatch(/接到|接上|接回/)
      }
      for (const phase of ['checked-out', 'svn-staged']) {
        const b = banner(phase, { origBranch: 'e2e8' })
        expect(b.note, phase).toContain('切回 e2e8')
        expect(b.detail, phase).toContain('不要在这里改代码')
      }
      // 「停止收尾」之后为什么要点「更新」：不更新，下次交付会被拒
      expect(banner('publishing-baseline', { committedRev: 12, origBranch: 'e2e8' }).note).toContain(
        '下次交付会被拒',
      )
    })

    it('拿不到分支名（旧记录）时退回「你的」，不出现 undefined', () => {
      const b = banner('publishing-baseline', { committedRev: 12 })
      expect(b.detail).toContain('你的改动已经在 SVN 上了')
      expect(`${b.title}${b.detail}${b.note}`).not.toContain('undefined')
    })

    it('拿不到 revision 号（旧记录）时标题照样成立，不出现「rundefined」', () => {
      const b = banner('publishing-baseline')
      expect(b.title).toBe('交付已提交，还差收尾')
      expect(all(b)).not.toContain('undefined')
    })
  })

  it('提交状态未知：换标题，不给按钮', () => {
    const b = describeUnfinishedSync({
      locked: false,
      holder: null,
      unfinished: {
        flow: 'sync_out',
        phase: 'committing',
        opid: 'op',
        startedAt: 1,
        expectedStartRev: 7,
        commitMessage: 'm',
      },
    })!
    expect(b.title).toBe('交付状态未知')
    expect(b.actions).toEqual([])
    expect(b.detail).toContain('r8')
  })

  it('未收尾或进行中时，其余横幅一律让位', () => {
    // 那两种状态下仓库停在中间态，别的横幅的结论可能是错的，而它们的出口
    // （重建基线）此刻是危险的——会移动正被依赖的基线指针。
    expect(otherBannersAreSuppressed(null)).toBe(false)
    expect(otherBannersAreSuppressed(clean)).toBe(false)
    expect(
      otherBannersAreSuppressed({
        locked: false,
        holder: null,
        unfinished: { flow: 'sync_in', phase: 'stashed', opid: 'x', startedAt: 1 },
      }),
    ).toBe(true)
    expect(
      otherBannersAreSuppressed({ locked: true, holder: null, unfinished: null }),
    ).toBe(true)
  })

  it('每个 phase 都有自己的说明，不能落到一句通用话上', () => {
    // 通用文案等于没说。用户要知道的恰恰是「我的东西现在在哪」。
    const phases = [
      'start',
      'stashing',
      'stashed',
      'rebase',
      'capturing-baseline',
      'publishing-baseline',
      'baseline-ready',
      'applying-stash',
    ] as const
    const seen = new Set<string>()
    for (const phase of phases) {
      const b = describeUnfinishedSync({
        locked: false,
        holder: null,
        unfinished: { flow: 'sync_in', phase, opid: 'x', startedAt: 1 },
      })!
      expect(b.detail, `phase ${phase} 没有说明`).toBeTruthy()
      seen.add(b.detail)
    }
    expect(seen.size, '有 phase 共用了同一句说明').toBe(phases.length)
  })

  it('回退途中停在放回改动那一步：不能说「基线已经更新」（PR #49 review #3）', () => {
    const at = (rollingBack: boolean) =>
      describeUnfinishedSync({
        locked: false,
        holder: null,
        unfinished: { flow: 'sync_in', phase: 'applying-stash', opid: 'x', startedAt: 1, rollingBack },
      })!.detail
    expect(at(true)).not.toContain('基线已经更新')
    expect(at(true)).toContain('更新前')
    expect(at(true)).toContain('stash')
    expect(at(false)).toContain('基线已经更新')
  })

  it('认不出的 phase 也要有话说，不能渲染一条没有正文的红条', () => {
    // 后端的 Phase 枚举比任一流程会写的都宽：手写的 journal、别的版本写的记录，
    // 都能让一条流程的记录里出现另一条流程的阶段名。
    // `PHASE_DETAIL[未知]` 是 undefined，会渲染出只有标题的红条。
    // （Phase 3a 之前这里用 sync_out 的 `svn-staged` 当「认不出」，现在它认得了。）
    for (const [flow, phase] of [
      ['sync_in', 'svn-staged'],
      ['sync_out', 'stashed'],
    ] as const) {
      const b = describeUnfinishedSync({
        locked: false,
        holder: null,
        unfinished: { flow, phase: phase as SyncPhase, opid: 'x', startedAt: 1 },
      })!
      expect(b.detail, flow).toBeTruthy()
      expect(b.detail, flow).toContain(phase)
      expect(b.actions, flow).toEqual([])
    }
  })

  it('stashing 阶段必须明确叫用户不要自己去 apply', () => {
    // spec §7.6 的表：这个阶段 push 可能落地也可能没有，「不静默 apply」。
    // 用户自己挑一条 stash 去 apply 极可能挑到别人的 WIP（§7.6.1）。
    const b = describeUnfinishedSync({
      locked: false,
      holder: null,
      unfinished: { flow: 'sync_in', phase: 'stashing', opid: 'x', startedAt: 1 },
    })!
    expect(b.detail).toContain('不要')
  })

  it('读不出同步状态 ≠ 干净：必须挂红条，且其余横幅让位', () => {
    // 后端把「journal 坏了」与「没有 journal」严格分开（B3），前端若在 catch 里
    // 写回 null，这两者又在界面上合成了「一切正常」——不挂横幅、也不报错。
    // 判不出来时仓库可能正停在中间态，与守卫同一方向：保守地当成有问题。
    const unreadable: SyncStateUnreadable = {
      readError: '同步记录 .git/mygitclient-sync.json 读不出来：缺字段 phase',
    }
    const b = describeUnfinishedSync(unreadable)!
    expect(b).not.toBeNull()
    expect(b.tone).toBe('danger')
    expect(b.detail).toContain('缺字段 phase')
    expect(b.actions).toEqual([])
    expect(otherBannersAreSuppressed(unreadable)).toBe(true)
  })

  it('只在锁被持有时轮询：释放不产生任何文件事件，只能靠问', () => {
    // 持锁进程崩溃或关掉句柄时锁文件原样留着，watcher 收不到任何东西。
    // 不轮询的话黄条会一直挂着、一直压着其余横幅，直到工作区碰巧有别的变动。
    expect(needsLockPolling({ locked: true, holder: null, unfinished: null })).toBe(true)
    expect(
      needsLockPolling({
        locked: true,
        holder: { pid: 1, startedAt: 1, opid: 'x' },
        unfinished: { flow: 'sync_in', phase: 'rebase', opid: 'x', startedAt: 1 },
      }),
    ).toBe(true)
    // 没锁就不轮询：绝大多数时间都是这个状态，不该有任何定时开销。
    // 「开始同步」由 watcher 负责（锁文件与 journal 的出现都会发 repo-changed）。
    expect(needsLockPolling(null)).toBe(false)
    expect(needsLockPolling(clean)).toBe(false)
    expect(
      needsLockPolling({
        locked: false,
        holder: null,
        unfinished: { flow: 'sync_in', phase: 'stashed', opid: 'x', startedAt: 1 },
      }),
    ).toBe(false)
    // 读不出来时锁状态未知，但坏的是 journal——轮询不会让它自己变好，
    // 修好文件本身会发 repo-changed。
    expect(needsLockPolling({ readError: 'x' })).toBe(false)
  })

  it('轮询间隔足够快到「释放后很快消失」，又不至于高频', () => {
    // 每一轮是一次独占打开 + 读一个小文件；E2E 清单承诺「最多 2 秒」。
    expect(SYNC_LOCK_POLL_MS).toBeGreaterThanOrEqual(1000)
    expect(SYNC_LOCK_POLL_MS).toBeLessThanOrEqual(2000)
  })

  it('横幅按纯文本渲染，文案里不能带 Markdown 记号', () => {
    // BaselineBanner 用 `<p>{detail}</p>` 渲染，`**不要**` 会原样显示两对星号。
    const phases: SyncPhase[] = [
      'start',
      'stashing',
      'stashed',
      'rebase',
      'capturing-baseline',
      'publishing-baseline',
      'baseline-ready',
      'applying-stash',
    ]
    const states: (SyncState | SyncStateUnreadable)[] = [
      ...phases.map((phase) => ({
        locked: false,
        holder: null,
        unfinished: { flow: 'sync_in' as const, phase, opid: 'x', startedAt: 1 },
      })),
      ...[
        'prepare',
        'building-deliver',
        'deliver-built',
        'checked-out',
        'svn-staged',
        'committing',
        'post-commit',
        'publishing-baseline',
        'restoring-branch',
      ].map((phase) => ({
        locked: false,
        holder: null,
        unfinished: { flow: 'sync_out' as const, phase: phase as SyncPhase, opid: 'x', startedAt: 1 },
      })),
      { locked: true, holder: { pid: 1, startedAt: 1, opid: 'x' }, unfinished: null },
      { locked: true, holder: null, unfinished: null },
      { readError: 'x' },
    ]
    for (const s of states) {
      const b = describeUnfinishedSync(s)!
      for (const text of [b.title, b.detail, b.note ?? '']) {
        expect(text, JSON.stringify(s)).not.toContain('**')
      }
    }
  })
})

describe('sinceText', () => {
  it('与后端 describe_age 同一套说法', () => {
    expect(sinceText(1000, 1030)).toBe('刚刚开始')
    expect(sinceText(1000, 1000 + 5 * 60)).toBe('已持续 5 分钟')
    expect(sinceText(1000, 1000 + 2 * 3600 + 60)).toBe('已持续 2 小时')
    expect(sinceText(2000, 1000)).toBe('刚刚开始')
  })

  it('正在同步的横幅带上持续时长（spec §13：pid 与开始时间）', () => {
    const b = describeUnfinishedSync(
      { locked: true, holder: { pid: 4242, startedAt: 1000, opid: 'x' }, unfinished: null },
      1000 + 7 * 60,
    )!
    expect(b.detail).toContain('4242')
    expect(b.detail).toContain('已持续 7 分钟')
  })

  it('重建基线 / 行尾迁移拿着锁时不说「同步」，也不劝人重开应用', () => {
    // 那两条命令全程持同步锁，opid 带 `baseline-` 前缀（`commands/svn.rs::hold_for_baseline_move`）。
    // 用户知道自己没点过「更新」，照着「重开应用即可」去做，就把正在跑的重建拦腰杀掉
    const b = describeUnfinishedSync(
      {
        locked: true,
        holder: { pid: 4242, startedAt: 1000, opid: 'baseline-op-1-2' },
        unfinished: null,
      },
      1000 + 60,
    )!
    expect(b.title).toContain('基线')
    expect(b.title).not.toContain('同步')
    expect(b.detail).toContain('4242')
    expect(b.note ?? '').not.toContain('重开应用')

    // 普通的同步照旧
    const s = describeUnfinishedSync(
      { locked: true, holder: { pid: 1, startedAt: 1000, opid: 'op-1-2' }, unfinished: null },
      1000,
    )!
    expect(s.title).toBe('这个工作副本正在进行一次同步')
  })
})

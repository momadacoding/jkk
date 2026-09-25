import { describe, expect, it } from 'vitest'
import {
  describeUnfinishedSync,
  otherBannersAreSuppressed,
  type SyncPhase,
  type SyncState,
  type SyncStateUnreadable,
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

  it('本期还没有继续/放弃按钮，但必须说清这一点', () => {
    // 半个功能比没有功能更糟的地方在于让用户干等。这一期只能检测和展示，
    // 那就明说，别挂两颗点不动的按钮。
    const b = describeUnfinishedSync({
      locked: false,
      holder: null,
      unfinished: { flow: 'sync_in', phase: 'start', opid: 'abc', startedAt: 1757000000 },
    })!
    expect(b.actions).toEqual([])
    expect(b.note).toBeTruthy()
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

  it('认不出的 phase 也要有话说，不能渲染一条没有正文的红条', () => {
    // 后端的 Phase 枚举比前端这个联合类型宽（含 sync_out 的全部阶段），
    // 手写的 journal 或 Phase 3 的流程都能让它们出现。
    // `PHASE_DETAIL[未知]` 是 undefined，会渲染出只有标题的红条。
    const b = describeUnfinishedSync({
      locked: false,
      holder: null,
      unfinished: {
        flow: 'sync_out',
        phase: 'svn-staged' as SyncPhase,
        opid: 'x',
        startedAt: 1,
      },
    })!
    expect(b.detail).toBeTruthy()
    expect(b.detail).toContain('svn-staged')
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
    ]
    const states: (SyncState | SyncStateUnreadable)[] = [
      ...phases.map((phase) => ({
        locked: false,
        holder: null,
        unfinished: { flow: 'sync_in' as const, phase, opid: 'x', startedAt: 1 },
      })),
      {
        locked: false,
        holder: null,
        unfinished: { flow: 'sync_out', phase: 'svn-staged' as SyncPhase, opid: 'x', startedAt: 1 },
      },
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

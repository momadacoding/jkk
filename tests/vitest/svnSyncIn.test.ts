import { describe, expect, it } from 'vitest'
import {
  describeSyncAction,
  describeSyncInPreview,
  describeSyncOutcome,
  expectFromPreview,
  syncActions,
  type SyncInPreview,
} from '@/engine/svnSyncIn'

const preview: SyncInPreview = {
  origBranch: 'work',
  origHead: 'h'.repeat(40),
  baselineOid: 'b'.repeat(40),
  baselineRev: 1,
  targetRev: 3,
  catchUpRev: null,
  outsideChanges: [],
}

describe('expectFromPreview', () => {
  it('把预览里的事实原样带回去做锁内复核', () => {
    const p = { ...preview, outsideChanges: ['Art/x.psd'] }
    const e = expectFromPreview(p)
    expect(e).toEqual({
      origHead: p.origHead,
      baselineOid: p.baselineOid,
      confirmedOutside: ['Art/x.psd'],
    })
    // 拷贝而不是共享：确认框之后预览对象被改，不能悄悄改掉已确认的清单
    expect(e.confirmedOutside).not.toBe(p.outsideChanges)
  })
})

describe('describeSyncInPreview', () => {
  it('说清从哪个 revision 更新到哪个、会重放哪个分支', () => {
    const m = describeSyncInPreview(preview)
    expect(m.message).toContain('r1')
    expect(m.message).toContain('r3')
    expect(m.message).toContain('work')
    expect(m.danger).toBeFalsy()
  })

  it('服务器没有新提交时照实说，不假装有更新', () => {
    const m = describeSyncInPreview({ ...preview, targetRev: 1 })
    expect(m.message).toContain('没有比 r1 更新的提交')
  })

  it('白名单外会被碰到的文件必须列出来，并用危险色', () => {
    // spec §7.2.1：git 对它们没有任何恢复手段，确认只授权 update
    const m = describeSyncInPreview({ ...preview, outsideChanges: ['Art/boss.psd'] })
    expect(m.warning).toContain('Art/boss.psd')
    expect(m.warning).toContain('TortoiseSVN')
    expect(m.danger).toBe(true)
  })

  it('总是说明「已暂存」状态不会保留（计划 A8）', () => {
    expect(describeSyncInPreview(preview).warning).toContain('重新暂存')
  })

  it('工作副本被外部整体更新过：先说会把基线跟上、服务器改动与自己的改动会重新分开（计划 A16）', () => {
    const m = describeSyncInPreview({ ...preview, baselineRev: 1, catchUpRev: 2, targetRev: 3 })
    expect(m.message).toContain('TortoiseSVN')
    expect(m.message).toContain('r2')
    expect(m.message).toContain('重新分开')
    expect(m.message).toContain('把 r2 更新到 r3')
    expect(m.danger).toBeFalsy()
  })

  it('接管之后服务器没有更新的内容：只整理，不说「仍会走一遍流程」那句空话', () => {
    const m = describeSyncInPreview({ ...preview, baselineRev: 1, catchUpRev: 3, targetRev: 3 })
    expect(m.message).toContain('没有比 r3 更新的提交')
    expect(m.message).not.toContain('仍会走一遍流程')
  })
})

describe('describeSyncOutcome', () => {
  it('成功不打扰', () => {
    expect(describeSyncOutcome({ kind: 'done', newRev: 3 })).toBeNull()
  })

  it('rebase 冲突：列出文件，并指向横幅上的继续 / 放弃', () => {
    const n = describeSyncOutcome({ kind: 'rebaseConflict', files: ['Code/A.cs'] })!
    expect(n.detail).toContain('Code/A.cs')
    expect(n.detail).toContain('继续')
    expect(n.detail).toContain('放弃')
  })

  it('撞名：列出文件并说明一个字节都没放回、备份在哪', () => {
    const n = describeSyncOutcome({
      kind: 'stashBlocked',
      collisions: ['Code/N.cs'],
      stashOid: 'f2208125aad7d35d63ee9495ad0a6cc3c1ce21be',
    })!
    expect(n.detail).toContain('Code/N.cs')
    expect(n.detail).toContain('f2208125aa')
  })

  it('在放回改动那一步放弃：不能说「已放回」，要说备份在 stash 里、分支已在新基线上（2c review I1）', () => {
    const n = describeSyncOutcome({
      kind: 'released',
      stashOid: 'f2208125aad7d35d63ee9495ad0a6cc3c1ce21be',
      rolledBack: false,
    })!
    expect(n.detail).not.toContain('已放回')
    expect(n.detail).toContain('f2208125aa')
    expect(n.detail).toContain('stash')
    expect(n.detail).toContain('新基线')
  })

  it('回退途中放弃放回：分支在更新前的位置，不能说在新基线上（PR #49 review #3）', () => {
    const n = describeSyncOutcome({
      kind: 'released',
      stashOid: 'f2208125aad7d35d63ee9495ad0a6cc3c1ce21be',
      rolledBack: true,
    })!
    expect(n.detail).not.toContain('新基线')
    expect(n.detail).toContain('更新前')
    expect(n.detail).not.toContain('已放回')
    expect(n.detail).toContain('f2208125aa')
  })

  it('stash 冲突给出备份 OID', () => {
    const n = describeSyncOutcome({
      kind: 'stashConflict',
      files: ['Code/A.cs'],
      stashOid: 'f2208125aad7d35d63ee9495ad0a6cc3c1ce21be',
    })!
    expect(n.detail).toContain('Code/A.cs')
    expect(n.detail).toContain('f2208125aa')
  })
})

describe('syncActions', () => {
  it('start 只有「清除记录」：它的定义就是还没动任何东西（计划 A10）', () => {
    const a = syncActions('start')
    expect(a.map((x) => x.id)).toEqual(['syncAbort'])
    expect(a[0].label).toContain('清除')
  })

  it('中间 phase 都有继续与放弃', () => {
    for (const p of ['stashing', 'stashed', 'capturing-baseline', 'publishing-baseline', 'baseline-ready', 'rebase', 'applying-stash']) {
      expect(syncActions(p).map((x) => x.id), p).toEqual(['syncContinue', 'syncAbort'])
    }
  })

  it('认不出的 phase 不给按钮（多半是 sync_out，Phase 3 才有）', () => {
    expect(syncActions('svn-staged')).toEqual([])
  })
})

describe('describeSyncAction', () => {
  it('rebase 阶段放弃要说清：基线保持新的，分支会落后于它', () => {
    const m = describeSyncAction('syncAbort', 'rebase')
    expect(m.message).toContain('基线')
    expect(m.danger).toBe(true)
  })

  it('applying-stash 阶段放弃要说清备份留在 stash 列表里', () => {
    expect(describeSyncAction('syncAbort', 'applying-stash').message).toContain('stash')
  })

  it('S5 之后放弃要说清 SVN 侧不退回、可能要重建基线（计划 A5）', () => {
    const m = describeSyncAction('syncAbort', 'capturing-baseline')
    expect(m.message).toContain('重建基线')
  })

  it('继续不是危险操作', () => {
    expect(describeSyncAction('syncContinue', 'rebase').danger).toBeFalsy()
  })
})

describe('文案按纯文本渲染', () => {
  it('任何一段都不能带 Markdown 记号（2b 第二轮 review R4 的教训）', () => {
    const texts: string[] = []
    const push = (m: { title: string; message: string; warning?: string }) =>
      texts.push(m.title, m.message, m.warning ?? '')
    push(describeSyncInPreview({ ...preview, outsideChanges: ['Art/boss.psd'] }))
    push(describeSyncInPreview({ ...preview, targetRev: 1 }))
    push(describeSyncInPreview({ ...preview, catchUpRev: 2 }))
    for (const p of ['start', 'stashed', 'capturing-baseline', 'rebase', 'applying-stash']) {
      push(describeSyncAction('syncContinue', p))
      push(describeSyncAction('syncAbort', p))
    }
    for (const o of [
      { kind: 'aborted' as const },
      { kind: 'rebaseConflict' as const, files: ['A'] },
      { kind: 'stashConflict' as const, files: ['A'], stashOid: 'x'.repeat(40) },
      { kind: 'stashBlocked' as const, collisions: ['N'], stashOid: 'x'.repeat(40) },
      { kind: 'released' as const, stashOid: 'x'.repeat(40), rolledBack: false },
      { kind: 'released' as const, stashOid: 'x'.repeat(40), rolledBack: true },
    ]) {
      const n = describeSyncOutcome(o)!
      texts.push(n.title, n.detail)
    }
    for (const t of texts) expect(t).not.toContain('**')
  })
})

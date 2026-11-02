import { describe, expect, it } from 'vitest'
import {
  describeCheckoutBehind,
  describeFollowContinue,
  describeFollowConflict,
  describeFollowOutcome,
  describeFollowReport,
  describeManualFollow,
  describeManualFollowAction,
  describeManualFollowOutcome,
  needsSvnUserPrompt,
  type FollowOutcome,
  type FollowReport,
  type Leak,
} from '@/engine/svnFollow'
import type { SyncInOutcome } from '@/engine/svnSyncIn'

const report: FollowReport = {
  newFloor: 3,
  verified: true,
  changes: [],
  removed: [],
  dropped: [],
  rewritten: [],
  stage: { decisions: {}, uncertain: [], lost: [], discarded: [] },
  movedBranches: [],
  behindBranches: [],
  leaks: [],
  pinnedRef: null,
  evidence: 'heuristic',
  evidenceCutoffMs: null,
}

describe('describeFollowReport 的判定依据（C1h）', () => {
  it('写明按什么判定的暂存与 commit', () => {
    expect(describeFollowReport(report).basis).toBe('判定依据：作者 + 失焦快照')
    expect(describeFollowReport({ ...report, evidence: 'hooks', evidenceCutoffMs: 1 }).basis).toBe(
      '判定依据：TortoiseSVN 钩子事件',
    )
  })
})

const leak: Leak = {
  branch: 'work',
  commit: 'l'.repeat(40),
  subject: '加日志',
  path: 'Code/L.cs',
  hunks: [{ oldStart: 3, lines: 1, verdict: 'unknown' }],
}

describe('describeFollowReport', () => {
  const section = (r: FollowReport, id: string) =>
    describeFollowReport(r).sections.find((s) => s.id === id)
  const text = (r: FollowReport) => {
    const m = describeFollowReport(r)
    return [
      m.title,
      m.summary,
      m.warning ?? '',
      ...m.sections.flatMap((s) => [
        s.title,
        s.note ?? '',
        ...s.groups.flatMap((g) => [g.label, ...g.items]),
      ]),
    ].join('\n')
  }

  it('follow report groups files by author', () => {
    const groups = section(
      {
        ...report,
        changes: [
          { path: 'Code/B.cs', rev: 12, author: 'zhang' },
          { path: 'Code/A.cs', rev: 13, author: 'li' },
          { path: 'Code/C.cs', rev: 12, author: 'zhang' },
          { path: 'Code/N.cs', rev: 14, author: null },
        ],
        removed: ['Code/Gone.cs'],
      },
      'changes',
    )!.groups

    expect(groups.map((g) => g.label)).toEqual(['li', 'zhang', '（没有作者）', '已在 SVN 上删除'])
    expect(groups[1].items).toEqual(['Code/B.cs  r12', 'Code/C.cs  r12'])
    expect(groups[3].items).toEqual(['Code/Gone.cs'])
  })

  it('uncertain stages are listed with a keep note', () => {
    const s = section(
      { ...report, stage: { ...report.stage, uncertain: ['Code/U.cs'] } },
      'uncertain',
    )!
    expect(s.groups[0].items).toEqual(['Code/U.cs'])
    expect(s.note).toContain('保留')
    expect(describeFollowReport({ ...report, stage: { ...report.stage, uncertain: ['x'] } }).tone).toBe(
      'attention',
    )
  })

  it('leaks are shown as a warning section', () => {
    const m = describeFollowReport({ ...report, leaks: [leak] })
    const s = m.sections.find((x) => x.id === 'leaks')!
    expect(s.warn).toBe(true)
    expect(s.note).toContain('Local-Only')
    expect(s.groups[0].items[0]).toContain('Code/L.cs')
    expect(s.groups[0].items[0]).toContain('work')
    expect(s.groups[0].items[0]).toContain('加日志')
    expect(m.tone).toBe('attention')
  })

  it('要用户留意的节排在最前：列表很长时不能被文件清单压到看不见', () => {
    const m = describeFollowReport({
      ...report,
      changes: [{ path: 'Code/A.cs', rev: 2, author: 'zhang' }],
      leaks: [leak],
      stage: { ...report.stage, lost: ['Code/L.cs'] },
    })
    expect(m.sections.map((s) => s.id)).toEqual(['leaks', 'lost', 'changes'])
  })

  it('干净完成只是告知，没发生的节一个都不渲染', () => {
    const m = describeFollowReport(report)
    expect(m.tone).toBe('ok')
    expect(m.title).toBe('跟进完成')
    expect(m.sections).toEqual([])
    expect(m.warning).toBeUndefined()
    expect(m.summary).toContain('r3')
  })

  it('丢失的暂存点名钉住的 ref 与取回的办法', () => {
    const s = section(
      {
        ...report,
        stage: { ...report.stage, lost: ['Code/A.cs'] },
        pinnedRef: 'refs/mygitclient/follow/op9',
      },
      'lost',
    )!
    expect(s.warn).toBe(true)
    expect(s.groups[0].items).toEqual(['Code/A.cs'])
    expect(s.note).toContain('refs/mygitclient/follow/op9')
    expect(s.note).toContain('git show refs/mygitclient/follow/op9:blobs/')
  })

  it('丢弃中间版本的暂存时也点名钉住的 ref', () => {
    const s = section(
      {
        ...report,
        stage: { ...report.stage, discarded: ['Code/A.cs'] },
        pinnedRef: 'refs/mygitclient/follow/op9',
      },
      'discarded',
    )!
    expect(s.note).toContain('refs/mygitclient/follow/op9')
    expect(s.note).toContain('git show')
  })

  it('没有钉住的 ref 时不编一个具体的名字出来', () => {
    const s = section({ ...report, stage: { ...report.stage, lost: ['Code/A.cs'] } }, 'lost')!
    expect(s.note).toContain('refs/mygitclient/follow/')
    expect(s.note).not.toContain('null')
  })

  it('离线没验证要明确提示：基线照样发布了', () => {
    const m = describeFollowReport({ ...report, verified: false })
    expect(m.tone).toBe('attention')
    expect(m.warning).toContain('服务器')
    expect(m.warning).toContain('体检')
  })

  it('合并的暂存只列三方合并过的路径', () => {
    const s = section(
      {
        ...report,
        stage: {
          ...report.stage,
          decisions: {
            'Code/A.cs': { merged: 'a'.repeat(40) },
            'Code/B.cs': 'keepStaged',
            'Code/C.cs': 'takeNew',
            'Code/D.cs': 'lostToNew',
          },
        },
      },
      'merged',
    )!
    expect(s.groups[0].items).toEqual(['Code/A.cs'])
  })

  it('丢弃的 commit 与换成 SVN 版本的路径', () => {
    const r: FollowReport = {
      ...report,
      dropped: ['d'.repeat(40)],
      rewritten: [['e'.repeat(40), ['Code/A.cs', 'Code/B.cs']]],
    }
    expect(section(r, 'dropped')!.groups[0].items).toEqual(['d'.repeat(10)])
    const rewritten = section(r, 'rewritten')!.groups[0]
    expect(rewritten.label).toContain('e'.repeat(10))
    expect(rewritten.items).toEqual(['Code/A.cs', 'Code/B.cs'])
    // 这两节只是告知
    expect(describeFollowReport(r).tone).toBe('ok')
  })

  it('移动的分支只是告知，落后的分支要说原因', () => {
    const r: FollowReport = {
      ...report,
      movedBranches: ['feature'],
      behindBranches: [['old', '重放 abc 时冲突']],
    }
    expect(section(r, 'moved')!.groups[0].items).toEqual(['feature'])
    expect(section(r, 'behind')!.groups[0].items).toEqual(['old：重放 abc 时冲突'])
    expect(describeFollowReport(r).tone).toBe('attention')
    expect(describeFollowReport({ ...report, movedBranches: ['feature'] }).tone).toBe('ok')
  })

  it('特别长的清单截断，并说还有多少', () => {
    const changes = Array.from({ length: 205 }, (_, i) => ({
      path: `Code/${i}.cs`,
      rev: 2,
      author: 'zhang',
    }))
    const items = section({ ...report, changes }, 'changes')!.groups[0].items
    expect(items).toHaveLength(201)
    expect(items.at(-1)).toContain('5')
  })

  it('文案里不能带 Markdown 记号（按纯文本渲染）', () => {
    const t = text({
      ...report,
      verified: false,
      changes: [{ path: 'A', rev: 2, author: 'x' }],
      removed: ['R'],
      dropped: ['d'],
      rewritten: [['e', ['A']]],
      stage: {
        decisions: { M: { merged: 'm' } },
        uncertain: ['U'],
        lost: ['L'],
        discarded: ['D'],
      },
      movedBranches: ['m'],
      behindBranches: [['b', 'why']],
      leaks: [leak],
      pinnedRef: 'refs/mygitclient/follow/op9',
    })
    expect(t).not.toContain('**')
  })
})

describe('needsSvnUserPrompt', () => {
  const none = { configured: null, detected: null }

  it('取不到用户时问', () => {
    expect(needsSvnUserPrompt(report, none)).toBe(true)
  })

  it('配了或探测到就不问', () => {
    expect(needsSvnUserPrompt(report, { configured: 'zhang', detected: null })).toBe(false)
    expect(needsSvnUserPrompt(report, { configured: null, detected: 'zhang' })).toBe(false)
  })

  it('有拿不准的暂存时总要问：探测到的名字可能不是提交时用的那个', () => {
    const r = { ...report, stage: { ...report.stage, uncertain: ['U'] } }
    expect(needsSvnUserPrompt(r, { configured: 'zhang', detected: 'zhang' })).toBe(true)
  })
})

describe('describeCheckoutBehind', () => {
  it('默认是切换并手动跟进，「仅切换」写明风险', () => {
    const m = describeCheckoutBehind('old')
    expect(m.confirmLabel).toBe('切换并手动跟进')
    expect(m.alternativeLabel).toBe('仅切换')
    expect(m.message).toContain('old')
    expect(m.warning).toContain('仅切换')
    expect(m.warning).toContain('写回磁盘')
    expect(m.warning).toContain('回退')
  })

  it('没有 busyLabel：切分支不走这个框的 action，忙碌态不会出现（Review 3 第 4 条）', () => {
    expect('busyLabel' in describeCheckoutBehind('old')).toBe(false)
  })
})

describe('describeFollowOutcome', () => {
  it('冲突：说清基线已发布、分支没动，以及是哪些文件', () => {
    const n = describeFollowOutcome({
      kind: 'needsManual',
      at: 'c'.repeat(40),
      files: ['Code/A.cs'],
      baselinePublished: true,
      leaks: [],
      staleDisk: false,
    })
    expect(n.tone).toBe('attention')
    expect(n.detail).toContain('Code/A.cs')
    expect(n.detail).toContain('基线')
    expect(n.detail).toContain('c'.repeat(10))
  })

  it('冲突的出路是手动跟进，不再让用户去点「更新」（日志 D10）', () => {
    const n = describeFollowOutcome({
      kind: 'needsManual',
      at: 'c'.repeat(40),
      files: ['Code/A.cs'],
      baselinePublished: true,
      leaks: [],
      staleDisk: false,
    })
    expect(n.detail).toContain('手动跟进')
    expect(n.detail).not.toContain('「更新」')
    expect(n.detail).toContain('写工作区')
  })

  it('冲突时也要把泄漏告警带上：用户要在动手之前知道', () => {
    const n = describeFollowOutcome({
      kind: 'needsManual',
      at: 'c'.repeat(40),
      files: ['Code/A.cs'],
      baselinePublished: true,
      leaks: [leak],
      staleDisk: false,
    })
    expect(n.detail).toContain('Local-Only')
    expect(n.detail).toContain('Code/L.cs')
    expect(n.detail).toContain('加日志')
  })

  it('磁盘上还是旧分支内容（Review 3 第 2 条）：不说冲突，说清只移分支会造出回退', () => {
    const n = describeFollowOutcome({
      kind: 'needsManual',
      at: 'c'.repeat(40),
      files: ['Code/A.cs'],
      baselinePublished: true,
      leaks: [],
      staleDisk: true,
    })
    expect(n.title).not.toContain('冲突')
    expect(n.detail).not.toContain('冲突')
    expect(n.detail).toContain('Code/A.cs')
    expect(n.detail).toContain('旧内容')
    expect(n.detail).toContain('回退')
    expect(n.detail).toContain('手动跟进')
    expect(n.detail).toContain('写回磁盘')
  })

  it('拒绝：把后端的原话交给用户，并说明什么都没动', () => {
    const n = describeFollowOutcome({ kind: 'refused', code: 'svn_index_busy', message: '索引被占着' })
    expect(n.tone).toBe('attention')
    expect(n.detail).toContain('索引被占着')
    expect(n.detail).toContain('没有改动')
  })

  it('文案里不能带 Markdown 记号（按纯文本渲染）', () => {
    for (const n of [
      describeFollowOutcome({ kind: 'refused', code: 'x', message: 'm' }),
      describeFollowOutcome({
        kind: 'needsManual',
        at: 'a',
        files: [],
        baselinePublished: true,
        leaks: [],
        staleDisk: false,
      }),
      { ...describeFollowContinue(), detail: describeFollowContinue().message },
    ]) {
      expect(n.title).not.toContain('**')
      expect(n.detail).not.toContain('**')
    }
  })
})

describe('describeFollowConflict', () => {
  const conflict: Extract<FollowOutcome, { kind: 'needsManual' }> = {
    kind: 'needsManual',
    at: 'c'.repeat(40),
    files: ['Code/A.cs'],
    baselinePublished: true,
    leaks: [leak],
    staleDisk: false,
  }

  it('把冲突的事实放在手动跟进的确认框上：按钮就是出路', () => {
    const m = describeFollowConflict(conflict)
    expect(m.title).toBe('跟进遇到冲突')
    expect(m.message).toContain('Code/A.cs')
    expect(m.message).toContain('Code/L.cs')
    expect(m.confirmLabel).toBe('手动跟进')
    expect(m.busyLabel).toBe(describeManualFollow().busyLabel)
    expect(m.warning).toBe(describeManualFollow().warning)
  })
})

describe('手动跟进', () => {
  const stash = 'x'.repeat(40)

  it('确认框说清它会写工作区，且不切基线、不跑 svn update', () => {
    const m = describeManualFollow()
    expect(m.confirmLabel).toBe('手动跟进')
    expect(`${m.message}${m.warning}`).toContain('写工作区')
    expect(m.message).toContain('svn update')
  })

  it('成功不打扰', () => {
    expect(describeManualFollowOutcome({ kind: 'done', newRev: 3 })).toBeNull()
  })

  it('暂存没保住要说出来，并分清是完成还是放弃', () => {
    const forward = describeManualFollowOutcome({
      kind: 'stageLost',
      newRev: 3,
      stashOid: stash,
      files: [],
    })!
    expect(forward.tone).toBe('attention')
    expect(forward.title).toContain('手动跟进完成')
    expect(forward.detail).toContain('内容都放回来了')
    expect(forward.detail).toContain(stash)

    const back = describeManualFollowOutcome({
      kind: 'stageLost',
      newRev: null,
      stashOid: stash,
      files: [],
    })!
    expect(back.title).toContain('放弃')
    expect(back.detail).toContain('之前的位置')
  })

  it('暂存与磁盘不同（Review 3 第 1 条）：如实说暂存的那一份还在 stash 里，点名文件、给出取回办法', () => {
    const n = describeManualFollowOutcome({
      kind: 'stageLost',
      newRev: 3,
      stashOid: stash,
      files: ['Code/B.cs'],
    })!
    expect(n.detail).toContain('Code/B.cs')
    expect(n.detail).toContain('与磁盘上的不同')
    expect(n.detail).toContain(`stash 列表里的 mygitclient-sync:…（${stash}）`)
    expect(n.detail).toContain(`git show ${stash}^2:<文件>`)
    expect(n.detail).toContain(`git restore --source=${stash}^2 --staged -- <文件>`)
    expect(n.detail).not.toContain('需要的话重新暂存一次')
  })

  it('放弃：分支回去了，基线不退', () => {
    const n = describeManualFollowOutcome({ kind: 'aborted' })!
    expect(n.tone).toBe('ok')
    expect(n.detail).toContain('基线')
    expect(n.detail).not.toContain('更新前')
  })

  it('放回冲突时暂存必然没保住，要说出来', () => {
    const n = describeManualFollowOutcome({ kind: 'stashConflict', files: ['A'], stashOid: stash })!
    expect(n.tone).toBe('attention')
    expect(n.detail).toContain('已暂存')
    expect(n.detail).toContain(stash.slice(0, 10))
  })

  it('任何结局都不能把这次说成「更新」', () => {
    const all: SyncInOutcome[] = [
      { kind: 'aborted' },
      { kind: 'stageLost', newRev: 2, stashOid: stash, files: ['B'] },
      { kind: 'stageLost', newRev: null, stashOid: null, files: [] },
      { kind: 'rebaseConflict', files: ['A'] },
      { kind: 'stashConflict', files: ['A'], stashOid: stash },
      { kind: 'stashBlocked', collisions: ['N'], stashOid: stash },
      { kind: 'released', stashOid: stash, rolledBack: false },
      { kind: 'released', stashOid: stash, rolledBack: true },
      { kind: 'settled', stashOid: stash, rolledBack: false },
      { kind: 'settled', stashOid: stash, rolledBack: true },
    ]
    for (const o of all) {
      const n = describeManualFollowOutcome(o)!
      expect(`${n.title}${n.detail}`, o.kind).not.toContain('更新')
      expect(`${n.title}${n.detail}`, o.kind).not.toContain('**')
    }
  })

  it('继续 / 放弃的确认框也说手动跟进，放弃 start 之外都标危险', () => {
    for (const p of ['start', 'stashing', 'stashed', 'rebase', 'applying-stash']) {
      for (const id of ['syncContinue', 'syncAbort'] as const) {
        const m = describeManualFollowAction(id, p)
        expect(m.title, p).toContain('手动跟进')
        expect(`${m.title}${m.message}`, p).not.toContain('更新')
      }
      expect(describeManualFollowAction('syncAbort', p).danger, p).toBe(p !== 'start')
    }
  })

  it('在放回那一步放弃不回滚：分支留在新基线上，备份留在 stash', () => {
    const m = describeManualFollowAction('syncAbort', 'applying-stash')
    expect(m.message).toContain('新基线')
    expect(m.message).toContain('stash')
  })
})

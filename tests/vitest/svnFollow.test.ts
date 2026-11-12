import { describe, expect, it } from 'vitest'
import {
  autoFollowKey,
  describeAutoFollowDone,
  describeAutoFollowSkip,
  describeCheckoutBehind,
  describeFollowContinue,
  describeFollowConflict,
  describeFollowOutcome,
  describeFollowReport,
  describeManualFollow,
  describeManualFollowAction,
  describeManualFollowOutcome,
  describeRestoreOutcome,
  needsSvnUserPrompt,
  restorablePaths,
  type FollowOutcome,
  type FollowReport,
  type Leak,
  type ReportItem,
} from '@/engine/svnFollow'
import type { SyncInOutcome } from '@/engine/svnSyncIn'

const report: FollowReport = {
  newFloor: 3,
  verified: true,
  changes: [],
  removed: [],
  dropped: [],
  rewritten: [],
  snapshot: [],
  drift: [],
  stage: { decisions: {}, uncertain: [], lost: [], discarded: [] },
  movedBranches: [],
  behindBranches: [],
  leaks: [],
  pinnedRef: null,
  evidence: 'heuristic',
  evidenceCutoffMs: null,
}

describe('自动跟进（事故分析 6.12）', () => {
  const advanced = {
    verdict: {
      kind: 'externallyAdvanced',
      changed: [{ path: 'Code/A.cs', rev: 4, author: 'me' }],
      added: [],
      removed: [],
    },
  }

  it('只有 SVN 侧有新内容时才有现场键；同步、分支落后、读不到判定都没有', () => {
    expect(autoFollowKey(advanced, 'h1')).not.toBeNull()
    expect(autoFollowKey({ verdict: { kind: 'inSync' } }, 'h1')).toBeNull()
    expect(autoFollowKey({ verdict: { kind: 'wcLocked' } }, 'h1')).toBeNull()
    expect(autoFollowKey(null, 'h1')).toBeNull()
  })

  it('同一个现场键相同；SVN 侧又有新内容、或 HEAD 动了，键就变了', () => {
    const again = JSON.parse(JSON.stringify(advanced))
    expect(autoFollowKey(again, 'h1')).toBe(autoFollowKey(advanced, 'h1'))
    expect(autoFollowKey(advanced, 'h2')).not.toBe(autoFollowKey(advanced, 'h1'))
    const more = {
      verdict: { ...advanced.verdict, added: [{ path: 'Code/B.cs', rev: 5, author: 'zhang' }] },
    }
    expect(autoFollowKey(more, 'h1')).not.toBe(autoFollowKey(advanced, 'h1'))
  })

  it('没自动的原因前面写明「没有自动跟进」', () => {
    expect(describeAutoFollowSkip('有冲突。')).toBe('没有自动跟进：有冲突。')
  })

  it('完成提示：一切顺利是灰色；有要留意的内容时变黄，并让人点开看', () => {
    const ok = describeAutoFollowDone({
      ...report,
      changes: [{ path: 'Code/A.cs', rev: 4, author: 'me' }],
    })
    expect(ok.tone).toBe('quiet')
    expect(ok.title).toBe('已自动跟进 SVN 侧的新内容')
    expect(ok.detail).toContain('跟进了 1 个文件')
    expect(ok.detail).not.toContain('查看详情')

    const behind = describeAutoFollowDone({ ...report, behindBranches: [['old', '有冲突']] })
    expect(behind.tone).toBe('warn')
    expect(behind.detail).toContain('「查看详情」')
  })
})

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

/** 只比路径：附注另有用例。 */
const texts = (items: ReportItem[]) => items.map((i) => i.text)

describe('describeFollowReport', () => {
  const section = (r: FollowReport, id: string) =>
    describeFollowReport(r).sections.find((s) => s.id === id)
  const text = (r: FollowReport) => {
    const m = describeFollowReport(r)
    return [
      m.title,
      m.summary,
      ...m.sections.flatMap((s) => [
        s.title,
        s.note ?? '',
        ...s.groups.flatMap((g) => [g.label, ...g.items.flatMap((i) => [i.text, i.meta ?? ''])]),
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
    // 路径与 revision 分开：路径对齐，revision 退到一边
    expect(groups[1].items).toEqual([
      { text: 'Code/B.cs', meta: 'r12' },
      { text: 'Code/C.cs', meta: 'r12' },
    ])
    expect(groups[3].items).toEqual([{ text: 'Code/Gone.cs' }])
  })

  it('uncertain stages are listed with a keep note', () => {
    const s = section(
      { ...report, stage: { ...report.stage, uncertain: ['Code/U.cs'] } },
      'uncertain',
    )!
    expect(texts(s.groups[0].items)).toEqual(['Code/U.cs'])
    // 只要确认，不是红色告警
    expect(s.level).toBe('attention')
    expect(s.note).toContain('保留')
    expect(describeFollowReport({ ...report, stage: { ...report.stage, uncertain: ['x'] } }).tone).toBe(
      'attention',
    )
  })

  it('leaks are shown as a warning section', () => {
    const m = describeFollowReport({ ...report, leaks: [leak] })
    const s = m.sections.find((x) => x.id === 'leaks')!
    expect(s.level).toBe('danger')
    expect(s.note).toContain('Local-Only')
    expect(s.groups[0].items[0].text).toBe('Code/L.cs')
    expect(s.groups[0].items[0].meta).toContain('work')
    expect(s.groups[0].items[0].meta).toContain('加日志')
    expect(m.tone).toBe('attention')
  })

  it('6.8：泄漏按判定分两档，并写明不影响这次跟进', () => {
    const sure: Leak = { ...leak, path: 'Code/S.cs', hunks: [{ oldStart: 1, lines: 1, verdict: 'present' }] }
    const m = describeFollowReport({ ...report, leaks: [sure, leak] })
    const s = m.sections.find((x) => x.id === 'leaks')!
    expect(s.groups.map((g) => g.label)).toEqual(['这些改动已确认在 SVN 上', '拿不准'])
    expect(s.groups[0].items[0].text).toBe('Code/S.cs')
    expect(s.groups[1].items[0].text).toBe('Code/L.cs')
    expect(s.note).toContain('不影响这次跟进')
    expect(s.note).toContain('其他分支')
    expect(s.note).not.toContain('请检查')
  })

  it('6.8：冲突框里的泄漏同样分两档，说清不拦手动跟进', () => {
    const sure: Leak = { ...leak, path: 'Code/S.cs', hunks: [{ oldStart: 1, lines: 1, verdict: 'present' }] }
    const n = describeFollowOutcome({
      kind: 'needsManual',
      at: 'c'.repeat(40),
      files: ['Code/A.cs'],
      baselinePublished: true,
      leaks: [sure, leak],
      staleDisk: false,
    })
    expect(n.detail).toContain('已确认在 SVN 上')
    expect(n.detail).toContain('拿不准')
    expect(n.detail).toContain('不拦手动跟进')
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

  it('先 danger、再 attention、最后 info', () => {
    const m = describeFollowReport({
      ...report,
      changes: [{ path: 'Code/A.cs', rev: 2, author: 'zhang' }],
      movedBranches: ['feature'],
      behindBranches: [['old', '有冲突']],
      snapshot: [['f'.repeat(40), ['Code/B.cs']]],
      stage: { ...report.stage, uncertain: ['Code/U.cs'] },
      drift: ['Code/D.cs'],
    })
    expect(m.sections.map((s) => s.id)).toEqual([
      'drift',
      'uncertain',
      'snapshot',
      'behind',
      'changes',
      'moved',
    ])
  })

  it('标题写出要处理几项：有 danger 说「需要处理」，只有 attention 说「请确认」', () => {
    const confirm = describeFollowReport({ ...report, behindBranches: [['old', '有冲突']] })
    expect(confirm.title).toBe('跟进完成，有 1 项请确认')
    expect(confirm.level).toBe('attention')
    expect(confirm.todo).toBe(1)

    // 没体检本身也算一项，而且是 danger
    const handle = describeFollowReport({
      ...report,
      verified: false,
      behindBranches: [['old', '有冲突']],
    })
    expect(handle.title).toBe('跟进完成，有 2 项需要处理')
    expect(handle.level).toBe('danger')

    // 只是告知的节不算
    const info = describeFollowReport({ ...report, movedBranches: ['feature'] })
    expect(info.title).toBe('跟进完成')
    expect(info.level).toBe('ok')
    expect(info.todo).toBe(0)
  })

  it('摘要带上一起移动的分支数，「工作区没改」单独一行', () => {
    const m = describeFollowReport({
      ...report,
      changes: [{ path: 'Code/A.cs', rev: 3, author: 'zhang' }],
      movedBranches: ['a', 'b'],
    })
    expect(m.summary).toBe('跟进了 1 个文件，2 条分支一起移动，基线最低在 r3。')
    expect(m.safe).toBe('工作区里的文件没有改动')
  })

  it('干净完成只是告知，没发生的节一个都不渲染', () => {
    const m = describeFollowReport(report)
    expect(m.tone).toBe('ok')
    expect(m.title).toBe('跟进完成')
    expect(m.sections).toEqual([])
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
    expect(s.level).toBe('danger')
    expect(texts(s.groups[0].items)).toEqual(['Code/A.cs'])
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
    const m = describeFollowReport({
      ...report,
      verified: false,
      changes: [{ path: 'Code/A.cs', rev: 2, author: 'zhang' }],
      drift: ['Code/D.cs'],
    })
    expect(m.tone).toBe('attention')
    // 是一节 danger，而且排在最前：整份报告的前提有问题
    const s = m.sections[0]
    expect(s.id).toBe('unverified')
    expect(s.level).toBe('danger')
    expect(s.note).toContain('服务器')
    expect(s.note).toContain('体检')
    expect(m.sections.map((x) => x.id)).toEqual(['unverified', 'drift', 'changes'])
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
    expect(texts(s.groups[0].items)).toEqual(['Code/A.cs'])
  })

  it('丢弃的 commit 与换成 SVN 版本的路径', () => {
    const r: FollowReport = {
      ...report,
      dropped: ['d'.repeat(40)],
      rewritten: [['e'.repeat(40), ['Code/A.cs', 'Code/B.cs']]],
    }
    expect(texts(section(r, 'dropped')!.groups[0].items)).toEqual(['d'.repeat(10)])
    const rewritten = section(r, 'rewritten')!.groups[0]
    expect(rewritten.label).toContain('e'.repeat(10))
    expect(texts(rewritten.items)).toEqual(['Code/A.cs', 'Code/B.cs'])
    // 这两节只是告知
    expect(describeFollowReport(r).tone).toBe('ok')
  })

  it('6.10：快照进来的 SVN 内容单独成节，要用户看一眼', () => {
    const r: FollowReport = { ...report, snapshot: [['f'.repeat(40), ['Code/B.cs']]] }
    const s = section(r, 'snapshot')!
    // 会让图标变黄，节本身也得是黄的、排在文件清单前面（以前是灰的、排在后面）
    expect(s.level).toBe('attention')
    expect(s.groups[0].label).toContain('f'.repeat(10))
    expect(texts(s.groups[0].items)).toEqual(['Code/B.cs'])
    expect(s.note).toContain('逐字节相同')
    expect(s.note).toContain('有意')
    // 可能把用户故意停住的旧版本换掉了：不能只是「完成」
    expect(describeFollowReport(r).tone).toBe('attention')
  })

  it('PR 119 遗留：重放后与磁盘对不上的文件是警告，排在文件清单前面', () => {
    const r: FollowReport = {
      ...report,
      drift: ['Code/A.cs'],
      changes: [{ path: 'Code/B.cs', rev: 4, author: 'x' }],
    }
    const m = describeFollowReport(r)
    const s = section(r, 'drift')!
    expect(s.level).toBe('danger')
    expect(texts(s.groups[0].items)).toEqual(['Code/A.cs'])
    expect(s.note).toContain('磁盘')
    expect(m.tone).toBe('attention')
    expect(m.sections.findIndex((x) => x.id === 'drift')).toBeLessThan(
      m.sections.findIndex((x) => x.id === 'changes'),
    )
  })

  it('条件 11 的重叠文件逐个标出与磁盘是否一致；不一致的要确认，一致的只告知，drift 不重复列', () => {
    const r: FollowReport = {
      ...report,
      drift: ['Code/D.cs'],
      overlap: [
        { path: 'Code/A.cs', commits: ['e381033 add @x'], matchesDisk: true },
        { path: 'Code/B.cs', commits: ['e381033 add @x', '1234567 more'], matchesDisk: false },
        { path: 'Code/D.cs', commits: ['e381033 add @x'], matchesDisk: false },
      ],
      changes: [{ path: 'Code/A.cs', rev: 4, author: 'x' }],
    }
    const m = describeFollowReport(r)
    const differs = section(r, 'overlapDiffers')!
    expect(differs.level).toBe('attention')
    expect(differs.groups[0].items).toEqual([
      { text: 'Code/B.cs', meta: 'e381033 add @x、1234567 more' },
    ])
    const matches = section(r, 'overlapMatches')!
    expect(matches.level).toBe('info')
    expect(texts(matches.groups[0].items)).toEqual(['Code/A.cs'])
    expect(m.sections.map((s) => s.id)).toEqual([
      'drift',
      'overlapDiffers',
      'changes',
      'overlapMatches',
    ])
  })

  it('保不住的暂存分三档：都不在是红色，部分在是琥珀并给计数，全在是灰色', () => {
    const r: FollowReport = {
      ...report,
      stage: {
        ...report.stage,
        lost: ['Code/L.cs', 'Code/P.cs'],
        partial: { 'Code/P.cs': { present: 24, total: 25 } },
        superseded: ['Code/S.cs'],
      },
    }
    const m = describeFollowReport(r)
    expect(texts(section(r, 'lost')!.groups[0].items)).toEqual(['Code/L.cs'])
    const part = section(r, 'lostPartial')!
    expect(part.level).toBe('attention')
    expect(part.groups[0].items).toEqual([
      { text: 'Code/P.cs', meta: '25 处改动里 24 处已在 SVN 上' },
    ])
    const sup = section(r, 'superseded')!
    expect(sup.level).toBe('info')
    expect(texts(sup.groups[0].items)).toEqual(['Code/S.cs'])
    expect(sup.note).toContain('refs/mygitclient/follow/')
    expect(m.sections.map((s) => s.id)).toEqual(['lost', 'lostPartial', 'superseded'])
  })

  it('丢失的文件名与对象原型上的属性同名时，照旧是红色', () => {
    const r: FollowReport = {
      ...report,
      stage: { ...report.stage, lost: ['constructor', 'toString'], partial: {} },
    }
    expect(texts(section(r, 'lost')!.groups[0].items)).toEqual(['constructor', 'toString'])
    expect(section(r, 'lostPartial')).toBeUndefined()
  })

  it('只有已被取代的暂存时，跟进照常算「完成」', () => {
    const m = describeFollowReport({
      ...report,
      stage: { ...report.stage, superseded: ['Code/S.cs'] },
    })
    expect(m.todo).toBe(0)
    expect(m.title).toBe('跟进完成')
  })

  it('旧后端没有 overlap 时不成节', () => {
    const m = describeFollowReport({ ...report, changes: [{ path: 'Code/A.cs', rev: 4, author: 'x' }] })
    expect(m.sections.map((s) => s.id)).toEqual(['changes'])
  })

  it('旧后端没有这两个字段时照常描述', () => {
    const old = { ...report } as Partial<FollowReport>
    delete old.snapshot
    delete old.drift
    expect(() => describeFollowReport(old as FollowReport)).not.toThrow()
  })

  it('移动的分支只是告知，落后的分支要说原因', () => {
    const r: FollowReport = {
      ...report,
      movedBranches: ['feature'],
      behindBranches: [['old', '重放 abc 时冲突']],
    }
    expect(section(r, 'moved')!.level).toBe('info')
    expect(texts(section(r, 'moved')!.groups[0].items)).toEqual(['feature'])
    expect(section(r, 'behind')!.level).toBe('attention')
    expect(section(r, 'behind')!.groups[0].items).toEqual([{ text: 'old', meta: '重放 abc 时冲突' }])
    expect(describeFollowReport(r).tone).toBe('attention')
    expect(describeFollowReport({ ...report, movedBranches: ['feature'] }).tone).toBe('ok')
  })

  it('特别长的清单截断，并说还有多少', () => {
    const changes = Array.from({ length: 205 }, (_, i) => ({
      path: `Code/${i}.cs`,
      rev: 2,
      author: 'zhang',
    }))
    const s = section({ ...report, changes }, 'changes')!
    const items = s.groups[0].items
    expect(items).toHaveLength(201)
    expect(items.at(-1)).toEqual({ text: '…还有 5 个', more: true })
    // 条数是截断之前的
    expect(s.count).toBe(205)
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

  it('6.5：冲突框给出每个文件在 SVN 上是谁、哪个 revision 改的，以及冲突那几行', () => {
    const n = describeFollowOutcome({
      kind: 'needsManual',
      at: 'c'.repeat(40),
      files: ['Code/A.cs', 'Code/G.cs', 'Code/X.cs'],
      baselinePublished: true,
      leaks: [],
      staleDisk: false,
      details: [
        {
          path: 'Code/A.cs',
          svnRev: 66464,
          svnAuthor: 'liuhao',
          svnRemoved: false,
          preview: '<<<<<<< 新基线（SVN）\nnew\n=======\nmine\n>>>>>>> 你的提交',
        },
        { path: 'Code/G.cs', svnRev: null, svnAuthor: null, svnRemoved: true, preview: null },
        { path: 'Code/X.cs', svnRev: null, svnAuthor: null, svnRemoved: false, preview: null },
      ],
    })
    expect(n.detail).toContain('Code/A.cs（SVN 上 r66464 liuhao 改过）')
    expect(n.detail).toContain('Code/G.cs（SVN 上已删除）')
    expect(n.detail).toMatch(/· Code\/X\.cs\n/)
    expect(n.detail).toContain('<<<<<<< 新基线（SVN）\nnew\n=======\nmine\n>>>>>>> 你的提交')
    // 旧后端没有 details：照原样列文件名
    const old = describeFollowOutcome({
      kind: 'needsManual',
      at: 'c'.repeat(40),
      files: ['Code/A.cs'],
      baselinePublished: true,
      leaks: [],
      staleDisk: false,
    })
    expect(old.detail).toContain('Code/A.cs')
    expect(old.detail).not.toContain('SVN 上 r')
  })

  it('6.7：冲突框说清点了取消之后从哪里回来，按钮名照实写', () => {
    for (const staleDisk of [false, true]) {
      const n = describeFollowOutcome({
        kind: 'needsManual',
        at: 'c'.repeat(40),
        files: ['Code/A.cs'],
        baselinePublished: true,
        leaks: [],
        staleDisk,
      })
      // 确认框的取消按钮写的是 Cancel（ConfirmDialog），文案要对得上
      expect(n.detail).toContain('「Cancel」')
      expect(n.detail).toContain('横幅上的「跟进」')
    }
  })

  it('6.10：手动跟进会直接采用基线的快照内容，动手之前列出来', () => {
    const base = {
      kind: 'needsManual' as const,
      at: 'c'.repeat(40),
      files: ['Code/A.cs'],
      baselinePublished: true,
      leaks: [],
      staleDisk: false,
    }
    const n = describeFollowOutcome({ ...base, snapshot: [['f'.repeat(40), ['Code/B.cs']]] })
    expect(n.detail).toContain('Code/B.cs（ffffffffff）')
    expect(n.detail).toContain('直接采用新基线的版本')
    expect(n.detail).toContain('有意')
    // 旧后端没有这个字段、或者没有命中：不出这一段
    expect(describeFollowOutcome(base).detail).not.toContain('逐字节相同')
    expect(describeFollowOutcome({ ...base, snapshot: [] }).detail).not.toContain('逐字节相同')
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

  describe('收尾核对有发现（事故分析 6.9）', () => {
    const drift = (o: Partial<Extract<SyncInOutcome, { kind: 'followDrift' }>>) =>
      describeManualFollowOutcome({
        kind: 'followDrift',
        newRev: 3,
        changed: [],
        resurrected: [],
        before: stash,
        stashOid: stash,
        stageLost: null,
        auditError: null,
        ...o,
      })!

    it('不能报成功：要打扰，点名被合错的文件，给出取回跟进前版本的完整命令', () => {
      const n = drift({ changed: ['Code/Dbs.cpp'] })
      expect(n.tone).toBe('attention')
      expect(n.title).toContain('和跟进前不一样')
      expect(n.detail).toContain('Code/Dbs.cpp')
      expect(n.detail).toContain(`git show "${stash}:<文件>"`)
      expect(n.detail).toContain('有意改的，可以忽略')
    })

    it('取回命令只写磁盘、不动暂存区，并说清跟进前未跟踪的文件怎么取（独立 review）', () => {
      const n = drift({ changed: ['Code/Dbs.cpp'] })
      expect(n.detail).toContain(`git restore --source="${stash}" --worktree -- <文件>`)
      expect(n.detail).not.toContain('git checkout')
      // cmd.exe 里 `^` 是转义符：必须在引号里
      expect(n.detail).toContain(`--source="${stash}^3"`)
      expect(n.detail).toContain('没有旧版本可取')
    })

    it('核对没跑成时照实说，不能装作核对过了', () => {
      const n = drift({ auditError: 'hash-object 读不了 Code/Locked.cs' })
      expect(n.tone).toBe('attention')
      expect(n.title).toContain('核对没跑成')
      expect(n.detail).toContain('没法确认')
      expect(n.detail).toContain('hash-object 读不了 Code/Locked.cs')
      expect(n.detail).toContain(`mygitclient-sync:…，${stash}`)
    })

    it('备份留着要说清：它可能是正确内容唯一的副本', () => {
      const n = drift({ changed: ['Code/Dbs.cpp'] })
      expect(n.detail).toContain(`mygitclient-sync:…，${stash}`)
      expect(n.detail).toContain('唯一的副本')
    })

    it('没有未提交的改动时不提备份，取回命令指向原分支 HEAD，也不提 ^3', () => {
      const head = 'h'.repeat(40)
      const n = drift({ changed: ['Code/Dbs.cpp'], before: head, stashOid: null })
      expect(n.detail).not.toContain('mygitclient-sync')
      expect(n.detail).toContain(`git restore --source="${head}" --worktree -- <文件>`)
      expect(n.detail).not.toContain('^3')
    })

    it('被带回的 SVN 已删文件单独成段；磁盘上还在的只从 git 里删，不动 SVN 工作副本里那份', () => {
      const n = drift({ resurrected: ['Tools/sim_lowmem.lua'] })
      expect(n.detail).toContain('Tools/sim_lowmem.lua')
      expect(n.detail).toContain('显示为删除')
      expect(n.detail).toContain('git rm --cached -- <文件>')
      expect(n.detail).not.toContain('git restore')
    })

    it('暂存也没保住时一并说出来', () => {
      const n = drift({ changed: ['Code/Dbs.cpp'], stageLost: ['Code/B.cs'] })
      expect(n.detail).toContain('「已暂存」的状态没保住')
      expect(n.detail).toContain('Code/B.cs')
    })
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

describe('放回 Local-Only 补丁（使用报告 9.1）', () => {
  it('旧后端没有 restorable 字段时为空', () => {
    expect(restorablePaths(report)).toEqual([])
    expect(restorablePaths({ ...report, restorable: ['Code/A.cs'] })).toEqual(['Code/A.cs'])
  })

  it('全部放回：成功，说明 SVN 里会显示已修改', () => {
    const d = describeRestoreOutcome({ restored: ['a', 'b'], skipped: [] })
    expect(d.ok).toBe(true)
    expect(d.text).toContain('2 个文件')
    expect(d.text).toContain('SVN 里')
  })

  it('有跳过的：要留意，写清放回与跳过各几个', () => {
    const skip = { path: 'b', reason: '磁盘上有你的编辑' }
    expect(describeRestoreOutcome({ restored: ['a'], skipped: [skip] })).toEqual({
      ok: false,
      text: '已放回 1 个文件，1 个跳过了：',
    })
    expect(describeRestoreOutcome({ restored: [], skipped: [skip] }).text).toBe(
      '一个文件都没放回，1 个跳过了：',
    )
  })
})

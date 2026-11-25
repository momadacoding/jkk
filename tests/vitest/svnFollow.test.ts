import { describe, expect, it } from 'vitest'
import {
  autoFollowKey,
  describeAutoFollowDone,
  describeAutoFollowConflict,
  describeAutoFollowSkip,
  describeCheckoutBehind,
  describeFollowContinue,
  describeFollowConflict,
  describeFollowConflictModel,
  describeFollowOutcome,
  describeFollowReport,
  driftDiffTarget,
  describeManualFollow,
  describeManualFollowAction,
  describeManualFollowOutcome,
  describePendingRestores,
  describeRestoreOutcome,
  describeRestoreSkips,
  describeHeldBack,
  heldBackFor,
  heldBackScene,
  keepOnNewFollow,
  pinHeldBack,
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

  it('重放冲突的文件组：文件行与新内容列表同一种写法，分支落后时说清它们不一定在新内容里', () => {
    const m = describeAutoFollowConflict({
      files: [
        { path: 'Code/A.cs', rev: 69007, author: 'xiebin', svnRemoved: false },
        { path: 'Code/C.cs', rev: 5, author: null, svnRemoved: false },
        { path: 'Code/D.cs', rev: null, author: null, svnRemoved: false },
        { path: 'Code/E.cs', rev: 6, author: 'li', svnRemoved: true },
      ],
      lagging: true,
    })
    expect(m.title).toBe('没有自动跟进：当前分支的提交重放到新基线上，有 4 个文件冲突')
    expect(m.files).toEqual([
      'Code/A.cs  r69007  xiebin',
      'Code/C.cs  r5',
      'Code/D.cs',
      'Code/E.cs  SVN 上已删除',
    ])
    expect(m.note).toContain('不一定在下面的新内容里')
    expect(m.note).toContain('点「跟进」会弹出冲突的详情')

    const fresh = describeAutoFollowConflict({
      files: [{ path: 'Code/A.cs', rev: 2, author: 'li', svnRemoved: false }],
      lagging: false,
    })
    expect(fresh.note).not.toContain('新内容')
  })

  describe('部分跟进之后只剩留下的文件：这个现场不必再自动试一次', () => {
    const held = [{ path: 'Code/A.cs', reason: '重放有冲突，要你来判断。' }]

    it('判定里只有留下的文件：给出现场键和横幅上的原因', () => {
      const scene = heldBackScene(held, advanced, 'h2')
      expect(scene?.key).toBe(autoFollowKey(advanced, 'h2'))
      // 横幅凭它不再写这一行：原因由留下的文件那一组显示（使用报告 2026-11-23）
      expect(scene?.held).toBe(true)
      expect(scene?.message).toBe(
        '还有 1 个文件留在旧基线上（上一次自动跟进留下的），要你来处理。' +
          'Code/A.cs：重放有冲突，要你来判断。点「跟进」来处理。',
      )
    })

    it('原因没带句号时补上，不和后面的话粘在一起', () => {
      const bare = [{ path: 'Code/A.cs', reason: '有冲突' }]
      expect(heldBackScene(bare, advanced, 'h2')?.message).toBe(
        '还有 1 个文件留在旧基线上（上一次自动跟进留下的），要你来处理。' +
          'Code/A.cs：有冲突。点「跟进」来处理。',
      )
    })

    it('原因不止一种时按原因分开写，文件多了只点名前几个', () => {
      const many = [
        ...['A', 'B', 'C', 'D', 'E', 'F'].map((n) => ({ path: `Code/${n}.cs`, reason: '有冲突。' })),
        { path: 'Code/Z.cs', reason: '和磁盘对不上。' },
      ]
      const sync = {
        verdict: {
          kind: 'externallyAdvanced',
          changed: many.map((h) => ({ path: h.path, rev: 4, author: 'zhang' })),
          added: [],
          removed: [],
        },
      }
      expect(heldBackScene(many, sync, 'h2')?.message).toBe(
        '还有 7 个文件留在旧基线上（上一次自动跟进留下的），要你来处理。' +
          'Code/A.cs、Code/B.cs、Code/C.cs、Code/D.cs、Code/E.cs 等 6 个文件：有冲突。' +
          'Code/Z.cs：和磁盘对不上。点「跟进」来处理。',
      )
    })

    it('SVN 侧还有别的新内容（多了文件、新增、删除）：照常自动试', () => {
      const more = {
        verdict: {
          ...advanced.verdict,
          changed: [...advanced.verdict.changed, { path: 'Code/B.cs', rev: 5, author: 'zhang' }],
        },
      }
      expect(heldBackScene(held, more, 'h2')).toBeNull()
      const added = {
        verdict: { ...advanced.verdict, added: [{ path: 'Code/N.cs', rev: 5, author: 'zhang' }] },
      }
      expect(heldBackScene(held, added, 'h2')).toBeNull()
      const removed = { verdict: { ...advanced.verdict, removed: ['Code/Gone.cs'] } }
      expect(heldBackScene(held, removed, 'h2')).toBeNull()
    })

    it('留下的文件有一个不在判定里了：现场变了，照常自动试', () => {
      const two = [...held, { path: 'Code/B.cs', reason: '有冲突。' }]
      expect(heldBackScene(two, advanced, 'h2')).toBeNull()
    })

    it('没有留下文件、读不到判定、判定不是「SVN 侧有新内容」时没有', () => {
      expect(heldBackScene([], advanced, 'h2')).toBeNull()
      expect(heldBackScene(undefined, advanced, 'h2')).toBeNull()
      expect(heldBackScene(held, null, 'h2')).toBeNull()
      expect(heldBackScene(held, { verdict: { kind: 'inSync' } }, 'h2')).toBeNull()
    })
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

describe('keepOnNewFollow：新的跟进开始时旧提示留不留（E2E 2026-11-13，独立 review 中-1）', () => {
  it('安静的提示收起：只说「做完了」，留着会和新横幅打架', () => {
    const quiet = { ...report, changes: [{ path: 'Code/A.cs', rev: 4, author: 'me' }] }
    expect(describeAutoFollowDone(quiet).tone).toBe('quiet')
    expect(keepOnNewFollow(quiet)).toBeNull()
  })

  it('要你留意的报告留着：「查看详情」是唯一入口，没点「知道了」不替用户扔', () => {
    const warn = { ...report, behindBranches: [['old', '有冲突']] as [string, string][] }
    expect(describeAutoFollowDone(warn).tone).toBe('warn')
    expect(keepOnNewFollow(warn)).toBe(warn)
  })

  it('本来就没有提示：还是没有', () => {
    expect(keepOnNewFollow(null)).toBeNull()
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

describe('describeFollowReport 的用时（2026-11-11 使用报告 10.5）', () => {
  it('流程补了用时就写出来，口径同初始化报告（不到一分钟带一位小数）', () => {
    expect(describeFollowReport({ ...report, elapsedMs: 12_340 }).elapsed).toBe('跟进用时 12.3 秒')
    expect(describeFollowReport({ ...report, elapsedMs: 70_400 }).elapsed).toBe('跟进用时 1 分 10 秒')
    expect(describeFollowReport({ ...report, elapsedMs: 0 }).elapsed).toBe('跟进用时 0.0 秒')
  })

  it('没有用时（崩溃后续上的、旧流程）就不写', () => {
    expect(describeFollowReport(report).elapsed).toBeNull()
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

  describe('提交时已确认的 Local-Only 改动（设计 2026-11-16-tsvn-acknowledged-leaks）', () => {
    const acked: Leak = { ...leak, path: 'Code/K.cs', hunks: [{ oldStart: 2, lines: 1, verdict: 'present' }] }

    it('是留档用的 info 节，不是红色告警，也不算要处理的一项', () => {
      const m = describeFollowReport({ ...report, acknowledgedLeaks: [acked] })
      const s = section({ ...report, acknowledgedLeaks: [acked] }, 'acknowledgedLeaks')!
      expect(s.level).toBe('info')
      expect(s.title).toBe('提交时已确认的 Local-Only 改动')
      expect(s.note).toContain('TortoiseSVN')
      expect(s.note).toContain('Proceed anyway')
      expect(texts(s.groups[0].items)).toEqual(['Code/K.cs'])
      expect(s.groups[0].items[0].meta).toContain('加日志')
      expect(m.sections.some((x) => x.id === 'leaks')).toBe(false)
      expect(m.todo).toBe(0)
      expect(m.level).toBe('ok')
    })

    it('与没确认的泄漏同时出现时各列各的', () => {
      const m = describeFollowReport({ ...report, leaks: [leak], acknowledgedLeaks: [acked] })
      expect(texts(m.sections.find((x) => x.id === 'leaks')!.groups.flatMap((g) => g.items))).toEqual([
        'Code/L.cs',
      ])
      expect(
        texts(m.sections.find((x) => x.id === 'acknowledgedLeaks')!.groups.flatMap((g) => g.items)),
      ).toEqual(['Code/K.cs'])
    })

    it('重放后变空、已丢弃的提交单列一组，不写成还在分支上', () => {
      const s = section({ ...report, dropped: [acked.commit], acknowledgedLeaks: [acked] }, 'acknowledgedLeaks')!
      expect(s.groups.map((g) => g.label)).toEqual(['已从分支移除：内容都在 SVN 上了'])
      expect(s.groups[0].items[0].meta).toContain('原在分支 work')
    })

    it('旧后端没有这个字段：不出这一节', () => {
      expect(section(report, 'acknowledgedLeaks')).toBeUndefined()
    })

    it('冲突框里也列出来，没有未确认的泄漏时不出泄漏那段', () => {
      const n = describeFollowOutcome({
        kind: 'needsManual',
        at: 'c'.repeat(40),
        files: ['Code/A.cs'],
        baselinePublished: true,
        leaks: [],
        acknowledgedLeaks: [acked],
        staleDisk: false,
      })
      expect(n.detail).toContain('提交时已确认')
      expect(n.detail).toContain('Code/K.cs')
      expect(n.detail).not.toContain('拿不准')
    })
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

  it('带了对比的两个提交时，对不上的每个文件都能「对比」；旧后端没带就不能', () => {
    const r: FollowReport = { ...report, drift: ['Code/A.cs', 'Code/B.cs'] }
    expect(section(r, 'drift')!.groups[0].items.map((i) => i.compare)).toEqual([
      undefined,
      undefined,
    ])
    const withRange = { ...r, driftRange: { from: 'b'.repeat(40), to: 't'.repeat(40) } }
    expect(section(withRange, 'drift')!.groups[0].items.map((i) => i.compare)).toEqual([
      'Code/A.cs',
      'Code/B.cs',
    ])
  })

  it('「对比」左边是 SVN 的版本（from），右边是分支的版本（to）', () => {
    expect(driftDiffTarget({ from: 'svn-base', to: 'branch-tip' }, 'Code/A.cs')).toEqual({
      kind: 'range',
      from: 'svn-base',
      to: 'branch-tip',
      file: 'Code/A.cs',
      oldFile: null,
    })
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

  it('按钮只给还没放回的：跟进时自动放回了的不再列', () => {
    const r: FollowReport = {
      ...report,
      restorable: ['Code/A.cs', 'Code/B.cs'],
      restore: { restored: ['Code/A.cs'], skipped: [{ path: 'Code/B.cs', reason: '不是你' }] },
    }
    expect(restorablePaths(r)).toEqual(['Code/B.cs'])
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

describe('跟进后自动放回 Local-Only 补丁（计划 2026-11-12-auto-restore-local-only）', () => {
  // 后端的真实形状：`overlap_files` 把整个 drift 并进 overlap，能放回的文件磁盘 ≠ tip，
  // matchesDisk 一定是 false（`svn_follow_auto.rs` 的现场断言过）。不带 overlap 的 fixture 是虚假信心
  const overlapOf = (paths: string[]) =>
    paths.map((path) => ({ path, commits: ['fae50a8 add @avatarclass'], matchesDisk: false }))
  const restoredReport: FollowReport = {
    ...report,
    drift: ['Code/A.cs', 'Code/B.cs', 'Code/C.cs'],
    overlap: overlapOf(['Code/A.cs', 'Code/B.cs', 'Code/C.cs']),
    restorable: ['Code/A.cs', 'Code/B.cs'],
    restore: {
      restored: ['Code/A.cs'],
      skipped: [{ path: 'Code/B.cs', reason: 'SVN 上最后改它的不是你' }],
      commits: { 'Code/A.cs': ['fae50a8 add @avatarclass'], 'Code/B.cs': ['1234567 补丁'] },
    },
  }
  const section = (id: string) => describeFollowReport(restoredReport).sections.find((s) => s.id === id)

  it('能放回的不留在红色「分支与磁盘对不上」里（缺陷 5）', () => {
    const drift = section('drift')!
    expect(drift.groups.flatMap((g) => g.items.map((i) => i.text))).toEqual(['Code/C.cs'])
    // 例外那句留着：restorable_paths 出错时能放回的文件会落进这一节
    expect(drift.note).toContain('Local-Only 补丁时别提交')
  })

  it('已放回的单列一节，写明路径和提交（缺陷 6）', () => {
    const s = section('restored')!
    expect(s.level).toBe('info')
    expect(s.groups[0].items).toEqual([{ text: 'Code/A.cs', meta: 'fae50a8 add @avatarclass' }])
  })

  it('没放回的列成要留意的一节，写明提交与原因', () => {
    const s = section('restorePending')!
    expect(s.level).toBe('attention')
    // 只有一种原因：写进说明一次，不在每个文件后面重复
    expect(s.groups[0].items).toEqual([{ text: 'Code/B.cs', meta: '补丁来自 1234567 补丁' }])
    expect(s.note).toContain('SVN 上最后改它的不是你')
    expect(s.note).toContain('顶部横幅')
  })

  it('旧后端没有 restore：能放回的全部算没放回，不写附注', () => {
    const r: FollowReport = { ...report, drift: ['Code/A.cs'], restorable: ['Code/A.cs'] }
    const m = describeFollowReport(r)
    expect(m.sections.find((s) => s.id === 'drift')).toBeUndefined()
    expect(m.sections.find((s) => s.id === 'restorePending')!.groups[0].items).toEqual([
      { text: 'Code/A.cs' },
    ])
    expect(m.safe).toBe('工作区里的文件没有改动')
  })

  it('放回了文件时不再说「工作区没有改动」', () => {
    expect(describeFollowReport(restoredReport).safe).toBe(
      '工作区里只放回了 1 个文件的 Local-Only 补丁，其余文件没有改动',
    )
  })

  it('能放回的文件不落进「和磁盘不一致」那一节（review H1：那里也建议以磁盘为准提交）', () => {
    const differs = section('overlapDiffers')
    const listed = differs?.groups.flatMap((g) => g.items.map((i) => i.text)) ?? []
    expect(listed).not.toContain('Code/A.cs')
    expect(listed).not.toContain('Code/B.cs')
    // C 已经在红色「对不上」里，不重复列
    expect(listed).not.toContain('Code/C.cs')
  })

  it('全部放回时自动跟进的提示是安静的，说明放回了', () => {
    const r: FollowReport = {
      ...report,
      drift: ['Code/A.cs'],
      overlap: overlapOf(['Code/A.cs']),
      restorable: ['Code/A.cs'],
      restore: { restored: ['Code/A.cs'], skipped: [] },
    }
    const m = describeFollowReport(r)
    expect(m.sections.map((s) => s.id)).not.toContain('overlapDiffers')
    const n = describeAutoFollowDone(r)
    expect(n.tone).toBe('quiet')
    expect(n.detail).toContain('放回了 1 个文件的 Local-Only 补丁')
  })

  it('有没放回的：自动跟进的提示要人看', () => {
    expect(describeAutoFollowDone(restoredReport).tone).toBe('warn')
  })
})

describe('部分自动跟进（计划 2026-11-13-partial-auto-follow）', () => {
  const partial: FollowReport = {
    ...report,
    changes: [{ path: 'Code/A.cs', rev: 5, author: 'me' }],
    heldBack: [
      { path: 'Code/B.cs', reason: '重放出来的结果和磁盘对不上' },
      { path: 'Code/C.cs', reason: '有冲突' },
    ],
  }

  it('留在旧基线上的文件单列一节，写明原因；要人看', () => {
    const m = describeFollowReport(partial)
    const s = m.sections.find((x) => x.id === 'heldBack')
    expect(s?.level).toBe('attention')
    expect(s?.count).toBe(2)
    expect(s?.groups[0].items).toEqual([
      { text: 'Code/B.cs', meta: '重放出来的结果和磁盘对不上' },
      { text: 'Code/C.cs', meta: '有冲突' },
    ])
    expect(s?.note).toContain('顶部横幅')
    expect(m.summary).toContain('2 个文件留在旧基线上')
    // 报告框里照旧算一项（非目标：不改报告）
    expect(m.todo).toBe(1)
  })

  describe('完成提示不再把留下的文件算成「要你留意」（使用报告 2026-11-23）', () => {
    // 下方横幅一直列着它们和原因，提示再说一遍就是同一件事说两次
    it('只留下了文件：提示是安静的，指向下方横幅，不叫人点详情', () => {
      const n = describeAutoFollowDone(partial)
      expect(n.tone).toBe('quiet')
      expect(n.detail).toContain('2 个文件留在旧基线上')
      // 红框可能不在（横幅被同步中间态压着、刷新后判定变了）：措辞同时指向「查看详情」（独立 review 低 1）
      expect(n.detail).toContain('留下的文件和原因见下方横幅或「查看详情」')
      expect(n.detail).not.toContain('要你留意')
      expect(keepOnNewFollow(partial)).toBeNull()
    })

    it('另有要留意的内容：照旧变黄，项数不含留下的文件', () => {
      const n = describeAutoFollowDone({ ...partial, behindBranches: [['old', '有冲突']] })
      expect(n.tone).toBe('warn')
      expect(n.detail).toContain('报告里有 1 项要你留意')
    })

    it('没留下文件：不提下方横幅', () => {
      const n = describeAutoFollowDone({ ...partial, heldBack: [] })
      expect(n.detail).not.toContain('下方横幅')
    })
  })
})

describe('留下的文件钉在当时的 revision 上（使用报告 2026-11-23）', () => {
  const held = [
    { path: 'Code/A.cs', reason: '重放有冲突。' },
    { path: 'Code/B.cs', reason: '和磁盘对不上。' },
  ]
  const sync = {
    verdict: {
      kind: 'externallyAdvanced',
      changed: [
        { path: 'Code/A.cs', rev: 4, author: 'zhang' },
        { path: 'Code/B.cs', rev: 5, author: null },
        { path: 'Code/C.cs', rev: 6, author: 'li' },
      ],
      added: [],
      removed: [],
    },
  }

  it('钉住刷新之后的 revision 与 HEAD；判定里已经没有的文件不钉', () => {
    const pin = pinHeldBack([...held, { path: 'Code/Gone.cs', reason: '有冲突。' }], {
      sync,
      head: 'h2',
    })
    expect(pin).toEqual({
      head: 'h2',
      files: [
        { path: 'Code/A.cs', reason: '重放有冲突。', rev: 4 },
        { path: 'Code/B.cs', reason: '和磁盘对不上。', rev: 5 },
      ],
    })
  })

  it('读不出刷新之后的现场：照样记下，不按 revision 与 HEAD 比', () => {
    expect(pinHeldBack(held, null)).toEqual({
      head: null,
      files: held.map((h) => ({ ...h, rev: null })),
    })
  })

  it('没留下文件、刷新之后已不是「SVN 侧有新内容」：不记', () => {
    expect(pinHeldBack([], { sync, head: 'h2' })).toBeNull()
    expect(pinHeldBack(undefined, { sync, head: 'h2' })).toBeNull()
    expect(pinHeldBack(held, { sync: { verdict: { kind: 'inSync' } }, head: 'h2' })).toBeNull()
    expect(pinHeldBack(held, { sync: null, head: 'h2' })).toBeNull()
  })

  it('HEAD 动了（提交、切分支）照样列着，但标成「之前的提交上判定的」；没记 HEAD 时不比', () => {
    // 不能藏起来：完成提示已经是灰色、会被下一次跟进收起，藏了就哪里都看不到原因（独立 review 中 1）
    const pin = pinHeldBack(held, { sync, head: 'h2' })
    expect(heldBackFor(pin, 'h2')).toEqual({ files: pin?.files, stale: false })
    expect(heldBackFor(pin, 'h3')).toEqual({ files: pin?.files, stale: true })
    expect(heldBackFor(null, 'h2')).toBeNull()
    const loose = pinHeldBack(held, null)
    expect(heldBackFor(loose, 'h9')).toEqual({ files: loose?.files, stale: false })
  })
})

describe('横幅上留下的文件那一组（使用报告 2026-11-23）', () => {
  const sync = {
    verdict: {
      kind: 'externallyAdvanced' as const,
      changed: [
        { path: 'Code/A.cs', rev: 4, author: 'zhang' },
        { path: 'Code/B.cs', rev: 5, author: null },
        { path: 'Code/C.cs', rev: 6, author: 'li' },
        { path: 'Code/D.cs', rev: 7, author: 'li' },
        { path: 'Code/E.cs', rev: 8, author: 'li' },
      ],
      added: [],
      removed: [],
    },
  }
  // 后端 `svn_follow::HELD_*` 四种原因
  const reasons = {
    conflict: '分支上的提交重放到 SVN 的新内容上有冲突，要你来判断。',
    leak: '分支的 Local-Only 补丁可能已经被带上了 SVN 服务器，要你先看一眼。',
    overlap: '重放出来的结果和磁盘对不上：可能是文件里还有没提交的改动，也可能是重放合错了，要你看一眼。',
    notMine: '分支只比磁盘多 Local-Only 补丁，但 SVN 上最后改它的不是你（或认不出是谁），可能是你有意去掉的补丁。',
  }

  it('四种原因各成一组，原因就是组的标题；文件行与新内容列表同一种写法', () => {
    const m = describeHeldBack(
      [
        { path: 'Code/A.cs', reason: reasons.conflict, rev: 4 },
        { path: 'Code/B.cs', reason: reasons.leak, rev: 5 },
        { path: 'Code/C.cs', reason: reasons.overlap, rev: 6 },
        { path: 'Code/D.cs', reason: reasons.notMine, rev: 7 },
        { path: 'Code/E.cs', reason: reasons.conflict, rev: 8 },
      ],
      sync,
    )
    expect(m?.title).toBe('上一次自动跟进留下了 5 个文件，还停在旧基线上')
    expect(m?.groups).toEqual([
      { reason: reasons.conflict, files: ['Code/A.cs  r4  zhang', 'Code/E.cs  r8  li'] },
      { reason: reasons.leak, files: ['Code/B.cs  r5'] },
      { reason: reasons.overlap, files: ['Code/C.cs  r6  li'] },
      { reason: reasons.notMine, files: ['Code/D.cs  r7  li'] },
    ])
    expect(m?.lines.has('Code/A.cs  r4  zhang')).toBe(true)
    // 「跟进」只在这一处说明里出现一次，路径不在说明里重复
    expect(m?.note.match(/跟进/g)?.length).toBe(1)
    expect(m?.note).not.toContain('Code/')
  })

  it('SVN 侧又有了更新的 revision、或文件已经跟上：那个文件不算留下的', () => {
    const m = describeHeldBack(
      [
        { path: 'Code/A.cs', reason: reasons.conflict, rev: 3 },
        { path: 'Code/Gone.cs', reason: reasons.conflict, rev: 1 },
        { path: 'Code/B.cs', reason: reasons.leak, rev: 5 },
      ],
      sync,
    )
    expect(m?.groups).toEqual([{ reason: reasons.leak, files: ['Code/B.cs  r5'] }])
    expect(m?.title).toContain('1 个文件')
  })

  it('没钉 revision 时只按路径认', () => {
    const m = describeHeldBack([{ path: 'Code/A.cs', reason: reasons.conflict, rev: null }], sync)
    expect(m?.groups[0].files).toEqual(['Code/A.cs  r4  zhang'])
  })

  it('HEAD 变了之后：说明换成「之前的提交上判定的，点跟进重新判断」，「跟进」仍只出现一次', () => {
    const one = [{ path: 'Code/A.cs', reason: reasons.conflict, rev: 4 }]
    const m = describeHeldBack(one, sync, null, true)
    expect(m?.groups[0].reason).toBe(reasons.conflict)
    expect(m?.note).toContain('之前的提交上判定的')
    expect(m?.note).toContain('点「跟进」重新判断')
    expect(m?.note.match(/跟进/g)?.length).toBe(1)
    expect(describeHeldBack(one, sync)?.note).not.toContain('之前的提交')
  })

  it('已经在整次冲突那一组里的文件不重复列', () => {
    const files = [
      { path: 'Code/A.cs', reason: reasons.conflict, rev: 4 },
      { path: 'Code/B.cs', reason: reasons.leak, rev: 5 },
    ]
    const conflict = {
      files: [{ path: 'Code/A.cs', rev: 4, author: 'zhang', svnRemoved: false }],
      lagging: false,
    }
    expect(describeHeldBack(files, sync, conflict)?.groups).toEqual([
      { reason: reasons.leak, files: ['Code/B.cs  r5'] },
    ])
    expect(describeHeldBack(files.slice(0, 1), sync, conflict)).toBeNull()
  })

  it('没有留下的文件、判定不是「SVN 侧有新内容」时没有这一组', () => {
    const one = [{ path: 'Code/A.cs', reason: reasons.conflict, rev: 4 }]
    expect(describeHeldBack(null, sync)).toBeNull()
    expect(describeHeldBack([], sync)).toBeNull()
    expect(describeHeldBack(one, null)).toBeNull()
    expect(describeHeldBack(one, { verdict: { kind: 'inSync' } })).toBeNull()
  })

  it('整次跟进、旧后端：没有这一节，摘要不提', () => {
    for (const r of [report, { ...report, heldBack: [] }]) {
      const m = describeFollowReport(r)
      expect(m.sections.some((x) => x.id === 'heldBack')).toBe(false)
      expect(m.summary).not.toContain('留在旧基线上')
    }
  })
})

describe('「缺 Local-Only 补丁」横幅（计划的 C）', () => {
  it('放回后没写成的原因交给错误条：全部写成时不说话', () => {
    expect(describeRestoreSkips({ restored: ['Code/A.cs'], skipped: [] })).toBeNull()
    expect(
      describeRestoreSkips({
        restored: [],
        skipped: [
          { path: 'Code/A.cs', reason: '被占用' },
          { path: 'Code/B.cs', reason: '磁盘有你的编辑' },
        ],
      }),
    ).toBe('有 2 个文件没放回 Local-Only 补丁：Code/A.cs（被占用）；Code/B.cs（磁盘有你的编辑）')
  })

  it('空清单不挂', () => {
    expect(describePendingRestores([])).toBeNull()
  })

  it('写明个数、文件和补丁', () => {
    const m = describePendingRestores([
      { path: 'Code/A.cs', commits: ['fae50a8 add @avatarclass'] },
      { path: 'Code/B.cs', commits: [] },
    ])!
    expect(m.title).toBe('2 个文件缺 Local-Only 补丁')
    expect(m.files).toEqual(['Code/A.cs ← fae50a8 add @avatarclass', 'Code/B.cs'])
    expect(m.detail).toContain('不要把它提交进分支')
  })
})

describe('报告三节的写法（计划 2026-11-16-follow-report-usability 的 A）', () => {
  // 2026-11-16 Client 仓库那次跟进的形状：后端把整个 drift 并进 overlap（带上改过它的提交），
  // 能放回的两个文件 SVN 最后提交者是同事，没自动放回；「lua worktree」重放后变空被丢弃，
  // 它的泄漏仍在 leaks 里
  const NOT_MINE = 'SVN 上最后改它的不是你（或认不出是谁），可能是你有意去掉的补丁，没有自动放回。'
  const luaWorktree = 'a'.repeat(40)
  const editorTweak = 'b'.repeat(40)
  const real: FollowReport = {
    ...report,
    changes: [
      { path: 'Lua/ImpChat.lua', rev: 68460, author: 'xiebin' },
      { path: 'Lua/ImpUnionParade.lua', rev: 68461, author: 'xiebin' },
      { path: 'Lua/UnionMainBaseUI.lua', rev: 68463, author: 'xiebin' },
      { path: 'Editor/LuaSourceRoot.cs', rev: 68339, author: 'liuhao' },
    ],
    dropped: [luaWorktree],
    drift: ['Lua/ImpChat.lua', 'Lua/ImpUnionParade.lua', 'Lua/UnionMainBaseUI.lua'],
    overlap: [
      { path: 'Lua/ImpChat.lua', commits: ['9f1e2d3 [Local-Only] 调试日志'], matchesDisk: false },
      { path: 'Lua/ImpUnionParade.lua', commits: ['9f1e2d3 [Local-Only] 调试日志'], matchesDisk: false },
      { path: 'Lua/UnionMainBaseUI.lua', commits: ['c028e264 小婉问答'], matchesDisk: false },
    ],
    restorable: ['Lua/ImpChat.lua', 'Lua/ImpUnionParade.lua'],
    restore: {
      restored: [],
      skipped: [
        { path: 'Lua/ImpChat.lua', reason: NOT_MINE },
        { path: 'Lua/ImpUnionParade.lua', reason: NOT_MINE },
      ],
      commits: {
        'Lua/ImpChat.lua': ['9f1e2d3 [Local-Only] 调试日志'],
        'Lua/ImpUnionParade.lua': ['9f1e2d3 [Local-Only] 调试日志'],
      },
    },
    leaks: [
      {
        branch: 'work',
        commit: luaWorktree,
        subject: 'lua worktree',
        path: 'Editor/LuaSourceRoot.cs',
        hunks: [{ oldStart: 0, lines: 0, verdict: 'present' }],
      },
      {
        branch: 'work',
        commit: editorTweak,
        subject: 'editor 便利性优化',
        path: 'Editor/RuntimePrefabSourceTracker.cs',
        hunks: [{ oldStart: 4, lines: 2, verdict: 'unknown' }],
      },
    ],
  }
  const m = describeFollowReport(real)
  const section = (id: string) => m.sections.find((s) => s.id === id)!

  describe('没放回的补丁：问一句要不要，原因只说一次', () => {
    const s = section('restorePending')

    it('标题就是要用户做的决定，带上文件数', () => {
      expect(s.title).toBe('要不要放回这 2 个文件的 Local-Only 补丁？')
    })

    it('原因写在说明里一次，每个文件后面只写补丁来自哪个提交、SVN 最后提交者', () => {
      expect(s.note!.split(NOT_MINE)).toHaveLength(2)
      expect(s.groups).toEqual([
        {
          label: '',
          items: [
            {
              text: 'Lua/ImpChat.lua',
              meta: '补丁来自 9f1e2d3 [Local-Only] 调试日志 · SVN 最后提交：xiebin（r68460）',
            },
            {
              text: 'Lua/ImpUnionParade.lua',
              meta: '补丁来自 9f1e2d3 [Local-Only] 调试日志 · SVN 最后提交：xiebin（r68461）',
            },
          ],
        },
      ])
    })

    it('两条路都写清，和顶部横幅说的一样：要补丁就放回，有意去掉的就点「不放回」', () => {
      expect(s.note).toContain('要补丁就放回')
      expect(s.note).toContain('有意去掉的')
      expect(s.note).toContain('「不放回」')
      // 旧文案「不要把磁盘版本提交进分支」把「不要补丁」这条路堵死了；也不叫人提交：没放回的原因可能是
      // 磁盘或暂存区里另有内容，提交会把不知道是什么的东西带进分支（独立 review 第 2、3 条）
      expect(s.note).not.toContain('提交进分支')
      // 「磁盘上是 SVN 的版本」只在跟进那一刻成立
      expect(s.note).toMatch(/^跟进时/)
    })

    it('原因各不相同时按原因分组，说明里不再写某一条', () => {
      const r: FollowReport = {
        ...real,
        restore: {
          ...real.restore!,
          skipped: [
            { path: 'Lua/ImpChat.lua', reason: NOT_MINE },
            { path: 'Lua/ImpUnionParade.lua', reason: '文件被别的程序占用' },
          ],
        },
      }
      const g = describeFollowReport(r).sections.find((x) => x.id === 'restorePending')!
      expect(g.groups.map((x) => x.label)).toEqual([NOT_MINE, '文件被别的程序占用'])
      expect(g.note).not.toContain(NOT_MINE)
      expect(g.count).toBe(2)
    })

    it('SVN 最后提交者没有作者、或这次没跟进到它：照实写或不写', () => {
      const r: FollowReport = {
        ...real,
        changes: [{ path: 'Lua/ImpChat.lua', rev: 68460, author: null }],
      }
      const items = describeFollowReport(r).sections.find((x) => x.id === 'restorePending')!
        .groups[0].items
      expect(items[0].meta).toBe(
        '补丁来自 9f1e2d3 [Local-Only] 调试日志 · SVN 最后提交：没有作者（r68460）',
      )
      expect(items[1].meta).toBe('补丁来自 9f1e2d3 [Local-Only] 调试日志')
    })
  })

  describe('分支与磁盘对不上：三句话，带上改过它的提交', () => {
    const s = section('drift')

    it('只列不能放回的文件，后面写上重放后仍改过它的提交', () => {
      expect(s.groups[0].items).toEqual([
        { text: 'Lua/UnionMainBaseUI.lua', meta: '改过它的提交：c028e264 小婉问答' },
      ])
    })

    it('说明只有三句：是什么、怎么办、例外', () => {
      const sentences = s.note!.split('。').filter((t) => t !== '')
      expect(sentences).toHaveLength(3)
      expect(sentences[0]).toContain('本地改动')
      expect(sentences[1]).toContain('提交进分支')
      expect(sentences[2]).toContain('Local-Only')
    })

    it('旧后端没有 overlap：照原样只列文件名', () => {
      const r: FollowReport = { ...real, overlap: undefined }
      const d = describeFollowReport(r).sections.find((x) => x.id === 'drift')!
      expect(d.groups[0].items).toEqual([{ text: 'Lua/UnionMainBaseUI.lua' }])
    })
  })

  describe('分支与磁盘对不上：写出分支比 SVN 差几行（计划 2026-11-16 的 B）', () => {
    const drift = (lines: FollowReport['driftLines']) =>
      describeFollowReport({ ...real, driftLines: lines }).sections.find((x) => x.id === 'drift')!
        .groups[0].items[0]

    it('2026-11-16 那次：分支比 SVN 少 7 行，后面跟着造成差异的提交', () => {
      expect(
        drift({ 'Lua/UnionMainBaseUI.lua': { added: 0, removed: 7, binary: false } }),
      ).toEqual({
        text: 'Lua/UnionMainBaseUI.lua',
        meta: '分支比 SVN 少 7 行 · 改过它的提交：c028e264 小婉问答',
      })
    })

    it('多出来的、两边都有的、二进制的各有说法', () => {
      const meta = (added: number, removed: number, binary = false) =>
        drift({ 'Lua/UnionMainBaseUI.lua': { added, removed, binary } }).meta
      expect(meta(3, 0)).toBe('分支比 SVN 多 3 行 · 改过它的提交：c028e264 小婉问答')
      expect(meta(2, 7)).toBe('分支比 SVN 多 2 行、少 7 行 · 改过它的提交：c028e264 小婉问答')
      expect(meta(0, 0, true)).toBe('二进制文件，内容不同 · 改过它的提交：c028e264 小婉问答')
    })

    it('SVN 上已删、分支把它带回来的文件：说清是整个文件，不只是「多 N 行」（独立 review nit 5）', () => {
      const r: FollowReport = {
        ...real,
        removed: ['Lua/UnionMainBaseUI.lua'],
        driftLines: { 'Lua/UnionMainBaseUI.lua': { added: 120, removed: 0, binary: false } },
      }
      const item = describeFollowReport(r).sections.find((x) => x.id === 'drift')!.groups[0].items[0]
      expect(item.meta).toBe(
        'SVN 上已删除，分支里还有这个文件（120 行） · 改过它的提交：c028e264 小婉问答',
      )
      const bin: FollowReport = {
        ...r,
        driftLines: { 'Lua/UnionMainBaseUI.lua': { added: 0, removed: 0, binary: true } },
      }
      expect(
        describeFollowReport(bin).sections.find((x) => x.id === 'drift')!.groups[0].items[0].meta,
      ).toBe('SVN 上已删除，分支里还有这个文件 · 改过它的提交：c028e264 小婉问答')
    })

    it('没有行数（旧后端、后端算不出来）：照原样只写提交', () => {
      expect(drift(undefined).meta).toBe('改过它的提交：c028e264 小婉问答')
      expect(drift({}).meta).toBe('改过它的提交：c028e264 小婉问答')
    })

    it('有行数、没有提交时只写行数', () => {
      const r: FollowReport = {
        ...real,
        overlap: undefined,
        driftLines: { 'Lua/UnionMainBaseUI.lua': { added: 0, removed: 7, binary: false } },
      }
      const d = describeFollowReport(r).sections.find((x) => x.id === 'drift')!
      expect(d.groups[0].items).toEqual([{ text: 'Lua/UnionMainBaseUI.lua', meta: '分支比 SVN 少 7 行' }])
    })
  })

  describe('泄漏告警：重放后被丢弃的提交标成已从分支移除', () => {
    const s = section('leaks')

    it('丢弃的提交单列一组，不再写成「分支 work：…」', () => {
      expect(s.groups.map((g) => g.label)).toEqual(['拿不准', '已从分支移除：内容都在 SVN 上了'])
      expect(s.groups[1].items).toEqual([
        { text: 'Editor/LuaSourceRoot.cs', meta: 'lua worktree，原在分支 work' },
      ])
      expect(s.groups[0].items[0].meta).toBe('分支 work：editor 便利性优化')
      expect(s.note).toContain('已从分支移除')
    })

    it('还是红色：补丁内容上了服务器这件事本身要你知道', () => {
      expect(s.level).toBe('danger')
      const only = describeFollowReport({ ...real, leaks: [real.leaks[0]] })
      expect(only.sections.find((x) => x.id === 'leaks')!.level).toBe('danger')
    })

    it('别的分支带着同一个提交：它没重放、没移动（plan_other 见泄漏就停），不能标成已移除', () => {
      const other = { ...real.leaks[0], branch: 'work2' }
      const r: FollowReport = {
        ...real,
        leaks: [real.leaks[0], other],
        behindBranches: [['work2', '分支上的 Local-Only 补丁里有改动已经或可能出现在 SVN 上，不自动移动']],
      }
      const l = describeFollowReport(r).sections.find((x) => x.id === 'leaks')!
      expect(l.groups).toEqual([
        {
          label: '这些改动已确认在 SVN 上',
          items: [{ text: 'Editor/LuaSourceRoot.cs', meta: '分支 work2：lua worktree' }],
        },
        {
          label: '已从分支移除：内容都在 SVN 上了',
          items: [{ text: 'Editor/LuaSourceRoot.cs', meta: 'lua worktree，原在分支 work' }],
        },
      ])
    })

    it('没有丢弃的提交时说明不提「已从分支移除」', () => {
      const r: FollowReport = { ...real, dropped: [] }
      const l = describeFollowReport(r).sections.find((x) => x.id === 'leaks')!
      expect(l.groups.map((g) => g.label)).toEqual(['这些改动已确认在 SVN 上', '拿不准'])
      expect(l.note).not.toContain('已从分支移除')
    })
  })
})

describe('describeFollowConflictModel（设计 2026-11-25-follow-resolve-with-disk §5）', () => {
  const base = 'b'.repeat(40)
  const at = 'c'.repeat(40)
  const o = (
    disk: { kind: 'usable' } | { kind: 'refused'; reason: string } | null,
    diskReady: boolean,
  ): Extract<FollowOutcome, { kind: 'needsManual' }> => ({
    kind: 'needsManual',
    at,
    files: ['LuaScripts/Client/SettingMgr.lua'],
    baselinePublished: true,
    leaks: [],
    staleDisk: false,
    base,
    diskReady,
    details: [
      {
        path: 'LuaScripts/Client/SettingMgr.lua',
        svnRev: 70760,
        svnAuthor: 'li',
        svnRemoved: false,
        preview: '<<<<<<< 新基线（SVN）\nnew\n=======\nmine\n>>>>>>> 你的提交',
        disk,
      },
    ],
  })

  it('能用磁盘内容：✓ 一行，对比 SVN 新版本与磁盘上的真实文件，没有红色警告', () => {
    const m = describeFollowConflictModel(o({ kind: 'usable' }, true))
    expect(m.title).toBe('跟进遇到冲突')
    expect(m.intro).toContain(at.slice(0, 10))
    expect(m.intro).toContain('1 个文件冲突')
    expect(m.diskReady).toBe(true)
    expect(m.warning).toBeNull()
    expect(m.action).toContain('不写工作区')
    const f = m.files[0]
    expect(f.origin).toBe('SVN 上 r70760 li 改过，你的提交也改了同一处。')
    expect(f.diskLine).toBe('✓ 你已经在 SVN 里合并好了，磁盘上就是合并结果。')
    expect(f.compare).toEqual({ kind: 'worktree', from: base, file: f.path })
    expect(f.sides).toBe('左：SVN 新版本 r70760　右：磁盘上的合并结果')
    expect(m.confirmed).toEqual([f.path])
  })

  it('不能用：✗ 写明原因，对比 SVN 新版本与你的提交，只剩手动跟进时才有红色警告', () => {
    const m = describeFollowConflictModel(o({ kind: 'refused', reason: '磁盘上还有冲突标记' }, false))
    expect(m.diskReady).toBe(false)
    expect(m.warning).toContain('会写工作区')
    const f = m.files[0]
    expect(f.diskLine).toBe('✗ 不能直接用磁盘上的内容：磁盘上还有冲突标记')
    expect(f.compare).toEqual({ kind: 'range', from: base, to: at, file: f.path, oldFile: null })
    expect(f.sides).toBe('左：SVN 新版本 r70760　右：你的提交')
  })

  it('不内嵌冲突标记：预览只作为没有对比工具时的退路交给框', () => {
    const m = describeFollowConflictModel(o({ kind: 'usable' }, true))
    expect([m.intro, m.action, ...m.notes].join('\n')).not.toContain('<<<<<<<')
    expect(m.files[0].preview).toContain('新基线（SVN）')
  })

  it('冲突分属几个 commit：对比你的提交时右边取文件自己的 commit，说明里写清从哪个起', () => {
    const second = 'd'.repeat(40)
    const one = o({ kind: 'refused', reason: 'x' }, false)
    const m = describeFollowConflictModel({
      ...one,
      files: [...one.files, 'Code/G.cs'],
      details: [
        { ...one.details![0], commit: at },
        {
          path: 'Code/G.cs',
          svnRev: 70761,
          svnAuthor: null,
          svnRemoved: false,
          preview: null,
          disk: { kind: 'refused', reason: '冲突处以外还有别的改动' },
          commit: second,
        },
      ],
    })
    expect(m.files[1].compare).toEqual({
      kind: 'range',
      from: base,
      to: second,
      file: 'Code/G.cs',
      oldFile: null,
    })
    expect(m.intro).toContain('2 个提交')
    expect(m.intro).toContain('2 个文件冲突')
  })

  it('有磁盘结论但没有新基线：不给磁盘按钮（后端没法核对冲突框对比的是哪个基线）', () => {
    const m = describeFollowConflictModel({ ...o({ kind: 'usable' }, true), base: undefined })
    expect(m.diskReady).toBe(false)
    expect(m.base).toBeNull()
  })

  it('旧后端（没有 base / disk）：不给对比，不显示磁盘按钮', () => {
    const old = { ...o(null, false), base: undefined, diskReady: undefined }
    const m = describeFollowConflictModel(old)
    expect(m.diskReady).toBe(false)
    expect(m.files[0].compare).toBeNull()
    expect(m.files[0].diskLine).toBeNull()
  })

  it('跟进报告单列按磁盘内容解决的文件', () => {
    const r = describeFollowReport({
      ...report,
      diskResolved: [[at, ['LuaScripts/Client/SettingMgr.lua']]],
    })
    const s = r.sections.find((x) => x.id === 'diskResolved')
    expect(s?.title).toBe('按磁盘内容解决的冲突')
    expect(s?.count).toBe(1)
    expect(describeFollowReport(report).sections.some((x) => x.id === 'diskResolved')).toBe(false)
  })
})

import { describe, expect, it } from 'vitest'
import {
  canDeliver,
  describeDeliverPreview,
  describeSyncOutAction,
  describeSyncOutOutcome,
  describeSyncOutBanner,
  expectFromDeliverPreview,
  syncOutActions,
  type SyncOutPreview,
} from '@/engine/svnSyncOut'

const preview: SyncOutPreview = {
  origBranch: 'work',
  origHead: 'h'.repeat(40),
  baselineOid: 'b'.repeat(40),
  baselineRev: 7,
  remoteRev: 7,
  deliverOid: 'd'.repeat(40),
  deliverTree: 't'.repeat(40),
  changes: [
    { status: 'M', path: 'Code/A.cs' },
    { status: 'D', path: 'Code/B.cs' },
    { status: 'A', path: 'Code/Deep/New.cs' },
  ],
  localOnly: [],
  outsideChanges: [],
}

describe('describeDeliverPreview', () => {
  it('按新增 / 修改 / 删除分组并标出数量，空组不出现', () => {
    const m = describeDeliverPreview(preview)
    expect(m.groups).toEqual([
      { label: '新增 1', files: ['Code/Deep/New.cs'] },
      { label: '修改 1', files: ['Code/A.cs'] },
      { label: '删除 1', files: ['Code/B.cs'] },
    ])
    expect(m.summary).toContain('work')
    expect(m.summary).toContain('r7')
    expect(m.summary).toContain('不能撤回')
  })

  it('没有本地补丁时也要明说（spec §8.2.3 防漏标）', () => {
    const m = describeDeliverPreview(preview)
    expect(m.localOnly).toEqual([])
    expect(m.localOnlyNote).toContain('本次没有本地补丁被排除')
  })

  it('列出被摘掉的本地补丁', () => {
    const m = describeDeliverPreview({
      ...preview,
      localOnly: [{ oid: 'abcdef1234567890', subject: 'mute log' }],
    })
    expect(m.localOnly).toEqual(['abcdef1234 mute log'])
    expect(m.localOnlyNote).toContain('1 个本地补丁')
  })

  it('白名单里 git 管不到的修改：要警告，且必须勾选才能交付', () => {
    const m = describeDeliverPreview({ ...preview, outsideChanges: ['Code/x.psd'] })
    expect(m.outsideFiles).toEqual(['Code/x.psd'])
    expect(m.outsideWarning).toContain('TortoiseSVN')
    expect(canDeliver('修复', false, m)).toBe(false)
    expect(canDeliver('修复', true, m)).toBe(true)
  })

  it('横幅与对话框按纯文本渲染，文案里不能有 **', () => {
    const m = describeDeliverPreview({
      ...preview,
      localOnly: [{ oid: 'abcdef1234567890', subject: 'mute log' }],
      outsideChanges: ['Code/x.psd'],
    })
    for (const text of [m.title, m.summary, m.localOnlyNote, m.outsideWarning ?? '']) {
      expect(text).not.toContain('**')
    }
  })
})

describe('canDeliver', () => {
  it('提交信息不能是空的或只有空白', () => {
    const m = describeDeliverPreview(preview)
    expect(canDeliver('', true, m)).toBe(false)
    expect(canDeliver('  \n ', true, m)).toBe(false)
    expect(canDeliver('修复', false, m)).toBe(true)
  })
})

describe('expectFromDeliverPreview', () => {
  it('预览里的事实原样带回去做锁内复核，提交信息原样（后端 trim）', () => {
    const e = expectFromDeliverPreview({ ...preview, outsideChanges: ['Code/x.psd'] }, ' 修复\n')
    expect(e).toEqual({
      origBranch: 'work',
      origHead: preview.origHead,
      baselineOid: preview.baselineOid,
      deliverTree: preview.deliverTree,
      changes: preview.changes,
      confirmedOutside: ['Code/x.psd'],
      message: ' 修复\n',
    })
  })
})

describe('describeSyncOutOutcome', () => {
  it('D9：还有本地补丁时绝不说「可以删除分支」', () => {
    const n = describeSyncOutOutcome({ kind: 'delivered', rev: 8, branch: 'work', localOnlyLeft: 2 })!
    expect(n.title).toContain('r8')
    expect(n.detail).toContain('2 个本地补丁')
    expect(n.detail).not.toContain('可以删除')
  })

  it('D9：没有本地补丁时提示可删除，但不自动删', () => {
    const n = describeSyncOutOutcome({ kind: 'delivered', rev: 8, branch: 'work', localOnlyLeft: 0 })!
    expect(n.detail).toContain('可以删除')
    expect(n.detail).toContain('不会自动删')
  })

  it('rebase 冲突：说清提交已成功、只剩补丁', () => {
    const n = describeSyncOutOutcome({ kind: 'rebaseConflict', rev: 8, files: ['Code/A.cs'] })!
    expect(n.detail).toContain('r8')
    expect(n.detail).toContain('Code/A.cs')
    // 点名的按钮必须是 restoring-branch 横幅上真有的（2026-10-13 E2E 场景 6 实测原来写的是「继续」）
    expect(n.detail).toContain('「我已解决，继续」')
    expect(n.detail).toContain('「停止收尾」')
    expect(n.detail).not.toContain('「继续」')
    const labels = syncOutActions('restoring-branch').map((a) => a.label)
    expect(labels).toEqual(['我已解决，继续', '停止收尾'])
  })

  it('放弃的两种结局说法不同', () => {
    expect(describeSyncOutOutcome({ kind: 'abandoned', committed: false })!.detail).toContain('SVN 侧没有任何改动')
    expect(describeSyncOutOutcome({ kind: 'abandoned', committed: true })!.detail).toContain('没有撤回')
  })
})

describe('横幅', () => {
  it('D5 之前只给「清理」；committing 不给任何按钮；D5 之后给继续与停止', () => {
    for (const p of ['prepare', 'building-deliver', 'deliver-built', 'checked-out', 'svn-staged']) {
      expect(syncOutActions(p).map((a) => a.id)).toEqual(['syncOutCleanup'])
    }
    expect(syncOutActions('committing')).toEqual([])
    for (const p of ['post-commit', 'publishing-baseline', 'restoring-branch']) {
      expect(syncOutActions(p).map((a) => a.id)).toEqual(['syncOutContinue', 'syncOutAbort'])
    }
    expect(syncOutActions('stashed')).toEqual([]) // 认不出的 phase 不给按钮
  })

  it('按钮名与后端报错里引用的一致（「清理」「继续交付」「停止收尾」）', () => {
    // 后端的报错文案会叫用户去点这些按钮（2026-10-11 里程碑 review 第 4 条）
    expect(syncOutActions('svn-staged')[0].label).toBe('清理')
    expect(syncOutActions('post-commit').map((a) => a.label)).toEqual(['继续交付', '停止收尾'])
    expect(syncOutActions('restoring-branch')[1].label).toBe('停止收尾')
  })

  it('committing 的正文告诉用户去 log 里从哪个 revision 之后找什么（计划 B4）', () => {
    const d = describeSyncOutBanner({
      phase: 'committing',
      opid: 'op-1',
      expectedStartRev: 7,
      commitMessage: '修复跳跃',
    })!.detail
    // 提交若落地，第一个可能的位置就是 r8。「r8 之后」按字面会跳过它（最终 review 第 5 条）
    expect(d).toContain('从 r8 起')
    expect(d).not.toContain('r8 之后')
    expect(d).toContain('修复跳跃')
    expect(d).toContain('mygitclient:opid=op-1')
  })

  it('D5 之后的「继续」明说不会重复提交，「停止」明说不撤回', () => {
    expect(describeSyncOutAction('syncOutContinue', 'post-commit').message).toContain('不会重复提交')
    expect(describeSyncOutAction('syncOutAbort', 'post-commit').message).toContain('不会撤回')
    expect(describeSyncOutAction('syncOutCleanup', 'svn-staged').message).toContain('撤销')
  })

  it('确认框与横幅说同一套人话：不提基线、接上去、mixed-revision（2026-10-14 E2E 用户反馈）', () => {
    for (const phase of ['post-commit', 'publishing-baseline', 'restoring-branch']) {
      for (const id of ['syncOutContinue', 'syncOutAbort'] as const) {
        const m = describeSyncOutAction(id, phase).message
        expect(m, `${id} ${phase}`).not.toMatch(/基线|接上|接到|mixed-revision/)
        expect(m, `${id} ${phase}`).toContain('更新到最新版本')
      }
    }
    // 结局提示框也一样
    for (const o of [
      { kind: 'rebaseConflict', rev: 8, files: ['Code/P.cs'] },
      { kind: 'abandoned', committed: true },
    ] as const) {
      expect(describeSyncOutOutcome(o)!.detail, o.kind).not.toMatch(/基线|接上|接到|mixed-revision/)
    }
    // 「停止收尾」之后为什么要点「更新」
    expect(describeSyncOutAction('syncOutAbort', 'publishing-baseline').message).toContain('下次交付会被拒')
    expect(describeSyncOutAction('syncOutContinue', 'restoring-branch').message).toContain('本地补丁')
  })

  it('post-commit 的「停止」说明它也要联网（后端会先把白名单更新到最新）', () => {
    expect(describeSyncOutAction('syncOutAbort', 'post-commit').message).toContain('网络')
    expect(describeSyncOutAction('syncOutAbort', 'restoring-branch').message).not.toContain('网络')
  })
})

import { describe, expect, it } from 'vitest'
import {
  auditIsDue,
  SVN_AUDIT_MIN_GAP_MS,
  SVN_AUDIT_POLL_MS,
  describeAudit,
  describeSync,
  needsStandaloneCheck,
  type SyncStatus,
} from '@/engine/svnSync'

const inSync: SyncStatus = { baselineRev: 4, nodeRevs: [4], verdict: { kind: 'inSync' } }

describe('describeSync', () => {
  it('同步时不挂横幅', () => {
    expect(describeSync(inSync)).toBeNull()
  })

  it('还没读到状态时也不挂横幅', () => {
    // 打开仓库的那一瞬间 status 是 null。此时挂一个「状态未知」的横幅，
    // 等于每次打开仓库都闪一下红。
    expect(describeSync(null)).toBeNull()
  })

  it('mixed revision 列出每一个值，出口是重建基线', () => {
    const m = describeSync({
      baselineRev: 4,
      nodeRevs: [4, 7],
      verdict: { kind: 'mixedRevision', revs: [4, 7] },
    })!
    expect(m.tone).toBe('danger')
    expect(m.detail).toContain('4')
    expect(m.detail).toContain('7')
    expect(m.actions.map((a) => a.id)).toContain('rebuild')
  })

  it('pinned reset 给的是 svn update 那条出路，不是重建基线', () => {
    // spec §6.6.2：这两种情况的修复完全相反。
    // 对 pinned 用「重建基线」会把被钉死的**旧内容**录成一个标着新 revision 的
    // 基线 —— 自己造出 §9.2 的基线撒谎。
    const m = describeSync({
      baselineRev: 7,
      nodeRevs: [5],
      verdict: { kind: 'pinnedReset', nodeRev: 5, baselineRev: 7, external: 'Code' },
    })!
    const ids = m.actions.map((a) => a.id)
    expect(ids).toContain('updateWhitelistDir')
    expect(ids).not.toContain('rebuild')
    expect(m.detail).toContain('Code')
  })

  it('pinned reset 给出可以照抄的命令文本', () => {
    // 本期一条 SVN 写命令都不发，所以出口是「把命令给用户，让他自己去跑」。
    const m = describeSync({
      baselineRev: 7,
      nodeRevs: [5],
      verdict: { kind: 'pinnedReset', nodeRev: 5, baselineRev: 7, external: 'Code' },
    })!
    expect(m.command).toBe('svn update Code')
  })

  it('diverged 时分清是被推进还是被回退', () => {
    const forward = describeSync({
      baselineRev: 4,
      nodeRevs: [9],
      verdict: { kind: 'diverged', nodeRev: 9, baselineRev: 4 },
    })!
    const backward = describeSync({
      baselineRev: 9,
      nodeRevs: [4],
      verdict: { kind: 'diverged', nodeRev: 4, baselineRev: 9 },
    })!
    expect(forward.detail).not.toEqual(backward.detail)
    expect(forward.detail).toContain('前进')
    expect(backward.detail).toContain('回退')
  })

  it('所有 danger 文案都写明重建基线不碰工作目录、也不推进工作分支', () => {
    // spec §16：「代价是工作分支不会自动跟进新基线」。不写明的话，用户会以为
    // 点完就万事大吉，下次提交时才发现分支还挂在旧基线上。
    const cases: SyncStatus[] = [
      { baselineRev: 4, nodeRevs: [4, 7], verdict: { kind: 'mixedRevision', revs: [4, 7] } },
      { baselineRev: 4, nodeRevs: [9], verdict: { kind: 'diverged', nodeRev: 9, baselineRev: 4 } },
    ]
    for (const c of cases) {
      const m = describeSync(c)!
      expect(m.actions.map((a) => a.id)).toContain('rebuild')
      expect(m.note).toContain('工作目录')
      expect(m.note).toContain('工作分支')
    }
  })

  it('每种失同步都提供手动体检入口', () => {
    // NB-6：`check_current_baseline` 是 Lying 唯一真正可达的入口。
    const m = describeSync({
      baselineRev: 4,
      nodeRevs: [9],
      verdict: { kind: 'diverged', nodeRev: 9, baselineRev: 4 },
    })!
    expect(m.actions.map((a) => a.id)).toContain('check')
  })
})

describe('describeAudit', () => {
  it('对得上时不挂提示', () => {
    expect(describeAudit({ status: 'matched' })).toBeNull()
    expect(describeAudit(null)).toBeNull()
  })

  it('没验成时不挂提示——但也绝不说「没问题」', () => {
    // NotChecked 与 Matched 必须分开，但界面上都不打扰用户：
    // 这条检查是锦上添花，断网时天天弹一条黄条只会教会用户忽略它。
    expect(describeAudit({ status: 'notChecked', reason: '断网' })).toBeNull()
  })

  it('漏收时点名文件，并说清「git 没在保护它们」', () => {
    const m = describeAudit({
      status: 'missing',
      files: ['Code/Deep/D.cs'],
      total: 1,
    })!
    expect(m.tone).toBe('warn')
    expect(m.files).toEqual(['Code/Deep/D.cs'])
    expect(m.detail).toContain('保护')
  })

  it('漏收很多时说总数，不只说列出来的那几条', () => {
    // 只列 20 条而不说总数，用户会以为只漏了 20 个。
    const files = Array.from({ length: 20 }, (_, i) => `Code/F${i}.cs`)
    const m = describeAudit({ status: 'missing', files, total: 137 })!
    expect(m.detail).toContain('137')
  })
})

describe('needsStandaloneCheck', () => {
  // 2026-09-16 独立 review：体检入口原先只挂在失同步横幅里，而 describeSync
  // 对 InSync 返回 null，于是**正常仓库根本点不到体检**。
  // 偏偏 `Lying`（基线记的内容与服务器不符）恰恰发生在 revision 完全对得上的
  // 时候 —— 等于只在不需要它的时候提供它。

  it('同步的 SVN 仓库要有常驻入口', () => {
    expect(needsStandaloneCheck(true, inSync)).toBe(true)
  })

  it('还没读到状态时也要有——离线打开仓库照样得能体检', () => {
    expect(needsStandaloneCheck(true, null)).toBe(true)
  })

  it('普通 git 仓库不渲染', () => {
    expect(needsStandaloneCheck(false, null)).toBe(false)
    expect(needsStandaloneCheck(false, inSync)).toBe(false)
  })

  it('失同步横幅在时不重复渲染——任何时刻恰好一个入口', () => {
    const diverged: SyncStatus = {
      baselineRev: 4,
      nodeRevs: [9],
      verdict: { kind: 'diverged', nodeRev: 9, baselineRev: 4 },
    }
    expect(needsStandaloneCheck(true, diverged)).toBe(false)
    // 前提：那条横幅自己带着体检按钮
    expect(describeSync(diverged)!.actions.map((a) => a.id)).toContain('check')
  })

  it('pinned reset 的横幅也带体检，所以同样不重复', () => {
    const pinned: SyncStatus = {
      baselineRev: 9,
      nodeRevs: [4],
      verdict: { kind: 'pinnedReset', nodeRev: 4, baselineRev: 9, external: 'Code/Vendor' },
    }
    expect(needsStandaloneCheck(true, pinned)).toBe(false)
    expect(describeSync(pinned)!.actions.map((a) => a.id)).toContain('check')
  })
})

describe('auditIsDue', () => {
  // `now` 的真实形态是 `Date.now()`，也就是一个 1.7e12 量级的数，不是 0。
  // 拿 0 当 now 测出来的结论对不上实际行为（`AGENTS.md` §13 #9）。
  const NOW = 1_770_000_000_000

  it('从来没跑过时立刻放行', () => {
    // 打开仓库那一次会把 lastAt 写成当时的时刻，所以 0 只出现在「这个仓库还
    // 一次都没核对过」的情况下——那时不该等满一分钟。
    expect(auditIsDue(NOW, 0, false)).toBe(true)
  })

  it('刚跑完就不再跑', () => {
    const now = NOW
    expect(auditIsDue(now, now, false)).toBe(false)
    expect(auditIsDue(now, now - 1, false)).toBe(false)
    expect(auditIsDue(now, now - (SVN_AUDIT_MIN_GAP_MS - 1), false)).toBe(false)
  })

  it('刚好到间隔就放行（边界取等号，不是严格大于）', () => {
    const now = NOW
    expect(auditIsDue(now, now - SVN_AUDIT_MIN_GAP_MS, false)).toBe(true)
  })

  it('上一次还没回来就一律不跑，哪怕已经过了很久', () => {
    // 慢仓库上一次核对可能比间隔还长。不挡的话请求会一层层叠起来，
    // 而这条检查本来就是「锦上添花」，没有任何理由把服务器压垮。
    const now = NOW
    expect(auditIsDue(now, 0, true)).toBe(false)
    expect(auditIsDue(now, now - SVN_AUDIT_POLL_MS, true)).toBe(false)
  })

  it('定时器的间隔比闸宽，所以每一次滴答都真的会核对', () => {
    // 这两个常数的关系是有意的：闸是用来限制**窗口焦点**那个触发点的，
    // 不该把定时器自己也挡掉。反过来配的话定时器会变成隔一次跑一次。
    expect(SVN_AUDIT_POLL_MS).toBeGreaterThan(SVN_AUDIT_MIN_GAP_MS)
    expect(auditIsDue(NOW, NOW - SVN_AUDIT_POLL_MS, false)).toBe(true)
  })
})

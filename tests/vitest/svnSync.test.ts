import { describe, expect, it } from 'vitest'
import {
  auditIsDue,
  SVN_AUDIT_MIN_GAP_MS,
  SVN_AUDIT_POLL_MS,
  describeAudit,
  describeEolMigration,
  describeSync,
  MIGRATE_EOL_WARNING,
  needsStandaloneCheck,
  recordAfterSuccess,
  syncBannerIsSuppressed,
  syncIfRootRevChanged,
  type EolStatus,
  type RootRevMemo,
  type SyncStatus,
} from '@/engine/svnSync'

const inSync: SyncStatus = { baselineRev: 4, nodeRevs: [4], verdict: { kind: 'inSync' } }
const eolNeeds: EolStatus = { ruleInstalled: false, needsMigration: true }
const eolDone: EolStatus = { ruleInstalled: true, needsMigration: false }

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

  // 第三个参数是行尾迁移状态。下面这组传 `eolDone`（规则已装）＝ 迁移不介入，
  // 于是这组断言测的仍然是原来那条判据。迁移介入时的行为单独一组，见文件末尾。

  it('同步的 SVN 仓库要有常驻入口', () => {
    expect(needsStandaloneCheck(true, inSync, eolDone)).toBe(true)
  })

  it('还没读到状态时也要有——离线打开仓库照样得能体检', () => {
    expect(needsStandaloneCheck(true, null, eolDone)).toBe(true)
  })

  it('普通 git 仓库不渲染', () => {
    expect(needsStandaloneCheck(false, null, eolDone)).toBe(false)
    expect(needsStandaloneCheck(false, inSync, eolDone)).toBe(false)
  })

  it('失同步横幅在时不重复渲染——任何时刻恰好一个入口', () => {
    const diverged: SyncStatus = {
      baselineRev: 4,
      nodeRevs: [9],
      verdict: { kind: 'diverged', nodeRev: 9, baselineRev: 4 },
    }
    expect(needsStandaloneCheck(true, diverged, eolDone)).toBe(false)
    // 前提：那条横幅自己带着体检按钮
    expect(describeSync(diverged)!.actions.map((a) => a.id)).toContain('check')
  })

  it('pinned reset 的横幅也带体检，所以同样不重复', () => {
    const pinned: SyncStatus = {
      baselineRev: 9,
      nodeRevs: [4],
      verdict: { kind: 'pinnedReset', nodeRev: 4, baselineRev: 9, external: 'Code/Vendor' },
    }
    expect(needsStandaloneCheck(true, pinned, eolDone)).toBe(false)
    expect(describeSync(pinned)!.actions.map((a) => a.id)).toContain('check')
  })

  it('行尾状态还没读到时按「不需要迁移」处理，常驻入口照常', () => {
    // 打开仓库的那一瞬间它是 null。此时把体检入口藏起来，等于每次打开都闪一下。
    expect(needsStandaloneCheck(true, inSync, null)).toBe(true)
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

describe('§6.6.3 两档同步检测：什么时候才允许把观测值记下来', () => {
  /** 一个可控的第二档：可以让它成功，也可以让它抛错，并记下被调了几次。 */
  function fakeFullCheck() {
    const calls: number[] = []
    let fail = false
    return {
      calls,
      failNext: (v: boolean) => {
        fail = v
      },
      run: async () => {
        calls.push(calls.length)
        if (fail) throw new Error('svn status 撞上工作副本锁')
      },
    }
  }

  it('rootRevs 没变就不跑第二档', async () => {
    const memo: RootRevMemo = { value: 'wc=7' }
    const full = fakeFullCheck()
    const ran = await syncIfRootRevChanged(memo, async () => 'wc=7', full.run)
    expect(ran).toBe(false)
    expect(full.calls).toHaveLength(0)
    expect(memo.value).toBe('wc=7')
  })

  it('rootRevs 变了就跑第二档，成功后记下新值', async () => {
    const memo: RootRevMemo = { value: 'wc=7' }
    const full = fakeFullCheck()
    const ran = await syncIfRootRevChanged(memo, async () => 'wc=8', full.run)
    expect(ran).toBe(true)
    expect(full.calls).toHaveLength(1)
    expect(memo.value).toBe('wc=8')
  })

  it('第一次启动时 memo 是 null，第一档一律判「变了」', async () => {
    // 进程刚起来没有基准，这一次全量正是打开仓库本来就要做的事。
    const memo: RootRevMemo = { value: null }
    const full = fakeFullCheck()
    expect(await syncIfRootRevChanged(memo, async () => 'wc=1', full.run)).toBe(true)
    expect(memo.value).toBe('wc=1')
  })

  it('第二档失败时不记观测值，**同一个 revision 第二次必须重试**', async () => {
    // 这一条是这组测试存在的理由（2026-09-21 独立 review）。
    // 改之前：观测值在 await 之前就推进了，失败之后下一次事件看到同一个
    // revision 直接返回，失同步横幅永远停在旧值，直到 revision 再变或重开仓库。
    const memo: RootRevMemo = { value: 'wc=7' }
    const full = fakeFullCheck()
    const read = async () => 'wc=8'

    full.failNext(true)
    await expect(syncIfRootRevChanged(memo, read, full.run)).rejects.toThrow()
    expect(full.calls).toHaveLength(1)
    expect(memo.value).toBe('wc=7') // ← 没推进

    // 下一次文件变动事件：revision 还是 wc=8，但必须真的再跑一遍
    full.failNext(false)
    expect(await syncIfRootRevChanged(memo, read, full.run)).toBe(true)
    expect(full.calls).toHaveLength(2)
    expect(memo.value).toBe('wc=8')
  })

  it('第一档自己读不到 rootRevs 时，既不跑第二档也不动观测值', async () => {
    const memo: RootRevMemo = { value: 'wc=7' }
    const full = fakeFullCheck()
    const boom = async (): Promise<string> => {
      throw new Error('svn info 失败')
    }
    await expect(syncIfRootRevChanged(memo, boom, full.run)).rejects.toThrow()
    expect(full.calls).toHaveLength(0)
    expect(memo.value).toBe('wc=7')
  })

  it('无条件入口（打开仓库 / 重建之后）同样是成功才记', async () => {
    // 这两个入口不经过第一档，但「失败就不记」这条规则必须共用一份实现，
    // 否则迟早有一份会和另一份分叉。
    const memo: RootRevMemo = { value: null }
    const full = fakeFullCheck()

    full.failNext(true)
    await expect(recordAfterSuccess(memo, 'wc=9', full.run)).rejects.toThrow()
    expect(memo.value).toBeNull()

    full.failNext(false)
    await recordAfterSuccess(memo, 'wc=9', full.run)
    expect(memo.value).toBe('wc=9')
  })
})

describe('describeEolMigration', () => {
  it('还没读到状态时不挂任何东西', () => {
    // 打开仓库的那一瞬间它是 null。挂一条红条等于每次打开都闪一下。
    expect(describeEolMigration(null, inSync)).toBeNull()
  })

  it('规则已装就不挂', () => {
    expect(describeEolMigration(eolDone, inSync)).toBeNull()
  })

  it('需要迁移时给出重建入口', () => {
    const b = describeEolMigration(eolNeeds, inSync)
    expect(b).not.toBeNull()
    expect(b!.tone).toBe('danger')
    expect(b!.actions.map((a) => a.id)).toEqual(['migrateEol'])
  })

  it('revision 一致也照样挂——判据是规则在不在，不是同步与否', () => {
    // spec §7.3.0：翻完设置后 git status 会因为 stat cache 假报干净。
    // 任何基于「现在看着没事」的判据都会漏判，而漏判的表现正是用户最难
    // 自查的那种——文件在 mtime 被碰之后一个一个地无故变脏。
    expect(describeEolMigration(eolNeeds, inSync)).not.toBeNull()
  })

  it('pinnedReset 时让位——那种状态下重建基线是错的', () => {
    // 迁移的动作就是重建基线，而 pinnedReset 下重建会把被 svn:externals
    // 钉死的旧内容录成一个标着新 revision 的基线，自己造出 §9.2 的基线撒谎。
    const pinned: SyncStatus = {
      baselineRev: 9,
      nodeRevs: [4],
      verdict: { kind: 'pinnedReset', nodeRev: 4, baselineRev: 9, external: 'Code/Vendor' },
    }
    expect(describeEolMigration(eolNeeds, pinned)).toBeNull()
  })

  it('其余失同步状态下迁移横幅仍然出现', () => {
    const diverged: SyncStatus = {
      baselineRev: 4,
      nodeRevs: [9],
      verdict: { kind: 'diverged', nodeRev: 9, baselineRev: 4 },
    }
    expect(describeEolMigration(eolNeeds, diverged)).not.toBeNull()
  })
})

describe('迁移文案必须与后端真实行为一致', () => {
  // 2026-09-25 PR #45 review 抓到的。第一版后端只重录基线、确实不动工作分支，
  // 文案照实写了「不会推进你的工作分支」；后来修阻断时改成了「基线、工作分支、
  // 真实索引一起搬」，**文案没跟着改**，于是确认框对用户说了反话。
  //
  // 确认框是 git 状态变更前用户唯一的信息来源，这类矛盾必须由断言钉住：
  // 下一个改 `align_branch` 的人会在这里被拦下。
  const banner = describeEolMigration(eolNeeds, inSync)!

  it('两处都不得再声称「不会推进你的工作分支」', () => {
    for (const text of [banner.note, MIGRATE_EOL_WARNING]) {
      expect(text).not.toContain('不会推进')
    }
  })

  it('两处都要说明工作分支和暂存区会跟着一起前移', () => {
    for (const text of [banner.note, MIGRATE_EOL_WARNING]) {
      expect(text).toContain('工作分支和暂存区会跟着新基线一起前移')
    }
  })

  it('两处都要继续保证不动工作目录里的文件', () => {
    // 这一条是真的：`read-tree` 不带 `-u`，磁盘全程不碰。
    for (const text of [banner.note, MIGRATE_EOL_WARNING]) {
      expect(text).toContain('工作目录')
    }
  })

  it('确认框要披露会接管 .git/info/attributes', () => {
    // 迁移会把那个文件整体换成本工具生成的内容，用户原有的 filter / diff /
    // merge 规则随之失效（原件备份成 attributes.bak）。这是用户自己的配置被
    // 停用，不披露就是静默改了他的环境。
    expect(MIGRATE_EOL_WARNING).toContain('.git/info/attributes')
    expect(MIGRATE_EOL_WARNING).toContain('attributes.bak')
  })
})

describe('迁移横幅与失同步横幅互斥', () => {
  const diverged: SyncStatus = {
    baselineRev: 4,
    nodeRevs: [9],
    verdict: { kind: 'diverged', nodeRev: 9, baselineRev: 4 },
  }
  const pinned: SyncStatus = {
    baselineRev: 9,
    nodeRevs: [4],
    verdict: { kind: 'pinnedReset', nodeRev: 4, baselineRev: 9, external: 'Code/Vendor' },
  }

  it('需要迁移时失同步横幅让位，避免两条红条', () => {
    expect(syncBannerIsSuppressed(eolNeeds, diverged)).toBe(true)
  })

  it('不需要迁移时失同步横幅照常', () => {
    expect(syncBannerIsSuppressed(eolDone, diverged)).toBe(false)
  })

  it('pinnedReset 不让位——那条横幅指的是另一件必须先做的事', () => {
    expect(syncBannerIsSuppressed(eolNeeds, pinned)).toBe(false)
    expect(describeSync(pinned)).not.toBeNull()
  })

  it('迁移未完成时不挂常驻「体检基线」——基线本来就不可信', () => {
    expect(needsStandaloneCheck(true, inSync, eolNeeds)).toBe(false)
    expect(needsStandaloneCheck(true, inSync, eolDone)).toBe(true)
  })
})

import { describe, expect, it } from 'vitest'
import {
  WIZARD_STEPS,
  describeHealth,
  healthNeedsRebuild,
  HEALTH_MAX_LISTED,
  describePinned,
  describeTimings,
  formatDuration,
  nextStep,
  parseExtensions,
  splitErrorMessage,
  stepStatus,
  wizardErrorHint,
  describeGitignoreConflicts,
  describeGitignoreOutcome,
  gitignoreBlockReason,
  needsReconfirm,
} from '@/engine/svnInit'
import type { GitignoreConflict, GitignoreRule } from '@/engine/svnInit'

function rule(over: Partial<GitignoreRule> = {}): GitignoreRule {
  return {
    line: 1,
    pattern: '*.exe',
    effect: 'excludes',
    count: 2,
    samples: ['Assets/a.exe', 'Assets/b.exe'],
    ...over,
  }
}

function conflict(over: Partial<GitignoreConflict> = {}): GitignoreConflict {
  return {
    path: 'Assets/.gitignore',
    fullPath: 'C:\\w\\Assets\\.gitignore',
    versioned: false,
    activeAfterRename: false,
    rules: [rule()],
    ...over,
  }
}

describe('起作用的 .gitignore（2026-10-28 设计 §3）', () => {
  it('没有冲突时不显示这一块', () => {
    expect(describeGitignoreConflicts([])).toBeNull()
  })

  it('写完整路径与「第 N 行」，带数量和样例', () => {
    const n = describeGitignoreConflicts([conflict()])!
    expect(n.tone).toBe('warn')
    expect(n.title).toBe('这些 .gitignore 会影响交给 git 管的文件')
    expect(n.detail).toContain('C:\\w\\Assets\\.gitignore')
    expect(n.detail).toContain('第 1 行 `*.exe`：挡掉 2 个 SVN 里有的文件，例如 Assets/a.exe、Assets/b.exe')
    expect(n.detail).not.toContain('（受 SVN 版本控制）')
  })

  it('受版本控制的标出来；多收的另一种说法', () => {
    const n = describeGitignoreConflicts([
      conflict({
        versioned: true,
        rules: [rule({ line: 5, pattern: '!*.txt', effect: 'includes', count: 1, samples: ['Assets/n.txt'] })],
      }),
    ])!
    expect(n.detail).toContain('C:\\w\\Assets\\.gitignore（受 SVN 版本控制）')
    expect(n.detail).toContain('第 5 行 `!*.txt`：多收 1 个你没选的文件，例如 Assets/n.txt')
  })

  it('能不能点「建库」：受控 × 勾选、多收 × 勾选 四种组合', () => {
    const versioned = [conflict({ versioned: true })]
    const includes = [conflict({ rules: [rule({ effect: 'includes', pattern: '!*.txt' })] })]
    const plain = [conflict()]
    // 不勾 + 受控 → 拦（不改团队的规则）
    expect(gitignoreBlockReason(versioned, false)).toContain('受 SVN 版本控制')
    // 勾上 + 受控 → 放行（尊重它，不动它）
    expect(gitignoreBlockReason(versioned, true)).toBeNull()
    // 勾上 + 多收 → 拦（尊重只做减法）
    expect(gitignoreBlockReason(includes, true)).toContain('不允许多收')
    // 不勾 + 多收（未受控）→ 放行（改名之后多收自然消失）
    expect(gitignoreBlockReason(includes, false)).toBeNull()
    expect(gitignoreBlockReason(plain, false)).toBeNull()
    expect(gitignoreBlockReason(plain, true)).toBeNull()
    expect(gitignoreBlockReason([], false)).toBeNull()
  })

  it('藏在外层后面的标出来：外层停用之后才会起作用', () => {
    const n = describeGitignoreConflicts([
      conflict(),
      conflict({ path: 'Assets/Sub/.gitignore', fullPath: 'C:\\w\\Assets\\Sub\\.gitignore', activeAfterRename: true }),
    ])!
    expect(n.detail).toContain('C:\\w\\Assets\\Sub\\.gitignore（外层的 .gitignore 停用之后才会起作用）')
    expect(n.detail).not.toContain('C:\\w\\Assets\\.gitignore（外层')
  })

  it('勾上「尊重」时外层不停用，藏在后面的那些不起作用，不该拦', () => {
    const hiddenIncludes = conflict({
      activeAfterRename: true,
      rules: [rule({ effect: 'includes', pattern: '!*.txt' })],
    })
    expect(gitignoreBlockReason([conflict(), hiddenIncludes], true)).toBeNull()
    // 不勾时外层会被停用，藏在后面的受控文件会起作用——照样要拦
    const hiddenVersioned = conflict({ activeAfterRename: true, versioned: true })
    expect(gitignoreBlockReason([conflict(), hiddenVersioned], false)).toContain('受 SVN 版本控制')
  })

  it('完成页：既没尊重也没改名时不说', () => {
    expect(describeGitignoreOutcome({ respected: false, conflicts: [], disabled: [] })).toBeNull()
  })

  it('完成页：改过名的列出原路径与新路径，并说怎么恢复', () => {
    const n = describeGitignoreOutcome({
      respected: false,
      conflicts: [conflict()],
      disabled: [{ from: 'C:\\w\\Assets\\.gitignore', to: 'C:\\w\\Assets\\.gitignore.jkk-disabled' }],
    })!
    expect(n.title).toBe('停用了 1 个 .gitignore')
    expect(n.detail).toContain('C:\\w\\Assets\\.gitignore → C:\\w\\Assets\\.gitignore.jkk-disabled')
    expect(n.detail).toContain('想恢复就把它改回原名。')
  })

  it('完成页：尊重时列出没交给 git 管的文件', () => {
    const n = describeGitignoreOutcome({ respected: true, conflicts: [conflict()], disabled: [] })!
    expect(n.title).toBe('按 .gitignore 的规则，这些文件没有交给 git 管')
    expect(n.detail).toContain('C:\\w\\Assets\\.gitignore')
    expect(n.detail).toContain('第 1 行 `*.exe`：2 个，例如 Assets/a.exe、Assets/b.exe')
  })

  it('只有「出现了没看过的 .gitignore」需要刷新清单重新确认', () => {
    expect(needsReconfirm('svn_gitignore_unconfirmed')).toBe(true)
    expect(needsReconfirm('svn_gitignore_versioned')).toBe(false)
    expect(needsReconfirm('include_set_mismatch')).toBe(false)
  })

  it('三个新错误码都有引导语', () => {
    expect(wizardErrorHint('svn_gitignore_unconfirmed')).toContain('重新点「建库」')
    expect(wizardErrorHint('svn_gitignore_versioned')).toContain('尊重')
    expect(wizardErrorHint('svn_gitignore_includes')).toContain('改名停用')
  })
})

describe('耗时的显示', () => {
  it('与 Rust 侧 human_duration 同一口径：一分钟内保留一位小数，之后按分秒', () => {
    expect(formatDuration(0)).toBe('0.0 秒')
    expect(formatDuration(1234)).toBe('1.2 秒')
    expect(formatDuration(60_000)).toBe('1 分 0 秒')
    expect(formatDuration(95_000)).toBe('1 分 35 秒')
    expect(formatDuration(31 * 60_000 + 12_400)).toBe('31 分 12 秒')
  })

  it('取整只用整数运算、四舍五入，与 Rust 侧逐字一致（review 指出 toFixed 与 {:.1} 对 .x5 取整方向不同）', () => {
    // 1250：Rust `{:.1}` 按银行家舍入给 1.2，`toFixed(1)` 给 1.3——两边都改成整数四舍五入
    expect(formatDuration(1250)).toBe('1.3 秒')
    expect(formatDuration(1249)).toBe('1.2 秒')
    // 59_950 四舍五入到一位小数是 60.0 秒，应当进到分钟那一档
    expect(formatDuration(59_949)).toBe('59.9 秒')
    expect(formatDuration(59_950)).toBe('1 分 0 秒')
    expect(formatDuration(89_500)).toBe('1 分 30 秒')
  })

  it('成功页按执行顺序列出每个阶段，标题给总耗时', () => {
    const n = describeTimings([
      { phase: '前置校验 P1–P4（svn status）', millis: 1200 },
      { phase: '录入磁盘内容（git add）', millis: 95_000 },
    ])
    expect(n).not.toBeNull()
    expect(n!.title).toContain('1 分 36 秒')
    const lines = n!.detail.split('\n')
    expect(lines[0]).toContain('前置校验')
    expect(lines[0]).toContain('1.2 秒')
    expect(lines[1]).toContain('git add')
    expect(lines[1]).toContain('1 分 35 秒')
  })

  it('没有耗时（旧后端或空数组）时不显示', () => {
    expect(describeTimings([])).toBeNull()
    expect(describeTimings(undefined)).toBeNull()
  })
})

describe('错误正文的拆分', () => {
  it('第一行作标题，其余原样作详情——几百行的不一致报告不能被挤成一段', () => {
    const msg = 'git 实际跟踪的文件集合与配置声明的不一致。\n\n应当被跟踪、实际没有：共 3 个\n  · 3 个被 .gitignore:1:*.pyc 挡掉'
    const { headline, detail } = splitErrorMessage(msg)
    expect(headline).toBe('git 实际跟踪的文件集合与配置声明的不一致。')
    expect(detail).toBe('应当被跟踪、实际没有：共 3 个\n  · 3 个被 .gitignore:1:*.pyc 挡掉')
  })

  it('单行错误没有详情', () => {
    expect(splitErrorMessage('svn 不在 PATH 里')).toEqual({
      headline: 'svn 不在 PATH 里',
      detail: '',
    })
  })
})

describe('向导步骤状态机', () => {
  it('按固定顺序推进，最后一步没有下一步', () => {
    // 步骤顺序就是 spec §6.5 的序列，UI 不该自己排
    expect(WIZARD_STEPS.map((s) => s.id)).toEqual(['choose', 'preflight', 'confirm', 'done'])
    expect(nextStep('choose')).toBe('preflight')
    expect(nextStep('preflight')).toBe('confirm')
    expect(nextStep('confirm')).toBe('done')
    expect(nextStep('done')).toBeNull()
  })

  it('当前步之前的都算已完成，之后的都算未开始', () => {
    expect(stepStatus('choose', 'confirm')).toBe('done')
    expect(stepStatus('confirm', 'confirm')).toBe('current')
    expect(stepStatus('done', 'confirm')).toBe('pending')
  })
})

describe('扩展名输入的解析', () => {
  it('补上缺失的点，去空白，去重，保留顺序', () => {
    // 用户会输入 `cs, .ts ,cs`。配置校验要求每一条以 `.` 开头（Rust 侧 svn_config），
    // 让用户为此报错是没必要的——这里能自动修的就修掉。
    expect(parseExtensions('cs, .ts ,cs')).toEqual(['.cs', '.ts'])
  })

  it('`*` 原样保留，不被当成缺点的扩展名', () => {
    // `*` 是 spec §6.3 明文给出的全纳入写法，Rust 侧单独放行。
    // 给它补成 `.*` 会让配置校验失败，而用户完全不知道自己哪里写错了。
    expect(parseExtensions('*')).toEqual(['*'])
  })

  it('空输入得到空数组，不是 [""]', () => {
    expect(parseExtensions('')).toEqual([])
    expect(parseExtensions('  ,  ')).toEqual([])
  })
})

describe('体检三态的显示分类', () => {
  it('verified 是成功，不需要用户做任何事', () => {
    const d = describeHealth({ status: 'verified', rev: 12346, unverified: [] })
    expect(d.tone).toBe('ok')
    expect(d.blocking).toBe(false)
    expect(d.title).toContain('12346')
  })

  it('lying 是阻断，要列出差异文件', () => {
    const d = describeHealth({ status: 'lying', files: ['Code/A.cs', 'Code/B.cs'] })
    expect(d.tone).toBe('error')
    expect(d.blocking).toBe(true)
    expect(d.detail).toContain('Code/A.cs')
  })

  it('notChecked 既不是成功也不是阻断——这是 spec §6.8.6 的第三行', () => {
    // 把「没能验证」当成「验证不通过」会让离线场景寸步难行，
    // 当成「验证通过」则等于这条检查不存在。所以它必须是独立的第三种。
    const d = describeHealth({ status: 'notChecked', reason: '连不上服务器' })
    expect(d.tone).toBe('warn')
    expect(d.blocking).toBe(false)
    expect(d.title).toContain('未经体检')
    expect(d.detail).toContain('连不上服务器')
  })

  it('verified 但有未覆盖路径时，文案必须说清楚覆盖不全', () => {
    // 2026-09-21 独立 review。判据 `D \ S == ∅` 比的是**磁盘与服务器**，而 S 里
    // 那些路径上的磁盘内容正是用户改的——这条检查对它们什么也证明不了。
    // 改之前这里无条件显示「磁盘内容与服务器上该 revision 一致」，是越权。
    const d = describeHealth({
      status: 'verified',
      rev: 9,
      unverified: ['Code/A.cs', 'Code/Deep/D.cs'],
    })
    expect(d.tone).toBe('ok')
    expect(d.blocking).toBe(false)
    expect(d.title).toContain('2')
    expect(d.detail).toContain('Code/A.cs')
    expect(d.detail).toContain('Code/Deep/D.cs')
    // 不能再出现那句无保留的断言
    expect(d.detail).not.toBe('磁盘内容与服务器上该 revision 一致。')
  })

  it('未覆盖清单为空时文案一个字都不多', () => {
    // 另一半：干净工作副本上平白多一句「有 0 个文件未覆盖」是纯噪音。
    // 缺了它，一个「永远显示未覆盖」的实现照样能让上面那条通过。
    const d = describeHealth({ status: 'verified', rev: 9, unverified: [] })
    expect(d.detail).toBe('磁盘内容与服务器上该 revision 一致。')
    expect(d.title).toBe('基线已通过体检（r9）')
  })

  it('未覆盖路径很多时只列前 HEALTH_MAX_LISTED 条，其余报个数', () => {
    // 真实工程里白名单目录底下随便就有几千个未版本控制文件，全列出来会把
    // 对话框撑爆，而用户一条也读不完。
    const many = Array.from({ length: HEALTH_MAX_LISTED + 7 }, (_, i) => `Code/F${i}.cs`)
    const d = describeHealth({ status: 'verified', rev: 9, unverified: many })
    expect(d.detail).toContain('Code/F0.cs')
    expect(d.detail).toContain('…还有 7 条')
    expect(d.detail).not.toContain(`Code/F${HEALTH_MAX_LISTED}.cs`)
  })

  it('三种状态的 tone 互不相同', () => {
    // 防的是「都返回 warn 也能让上面三条通过」那种实现
    const tones = [
      describeHealth({ status: 'verified', rev: 1, unverified: [] }).tone,
      describeHealth({ status: 'lying', files: [] }).tone,
      describeHealth({ status: 'notChecked', reason: 'x' }).tone,
    ]
    expect(new Set(tones).size).toBe(3)
  })
})

describe('pinned 模式的说明', () => {
  it('没有钉死的 external 时不显示任何东西', () => {
    expect(describePinned([])).toBeNull()
  })

  it('有钉死的 external 时说明残余风险，但不阻断', () => {
    // spec §6.5.3：命中不阻断，改为进入 pinned 模式，并**常驻说明**残余风险
    // ——外部工具在项目根跑递归 update 会把代码退回 peg。
    const d = describePinned([{ definedIn: 'Code', localPath: 'Code/pinA', pinnedRev: 1 }])
    expect(d).not.toBeNull()
    expect(d!.blocking).toBe(false)
    expect(d!.detail).toContain('Code/pinA')
    expect(d!.detail).toContain('递归')
  })
})

describe('错误码到引导语的映射', () => {
  it('obstructed 时明确说本应用不会替你动那些路径', () => {
    // spec §8.1 #2：revert / 删目录都是破坏性的，只拦住并引导，不自动修复。
    // 这句话必须出现在 UI 上，否则用户会以为点一下就能自动清理。
    const h = wizardErrorHint('svn_preflight_obstructed')
    expect(h).toContain('不会')
    // 同时要说清「只有它阻断」——否则用户看到一条 preflight 失败，
    // 会以为又回到了「工作区必须干净」那套（spec §16.1）。
    expect(h).toContain('不再阻断')
  })

  it('不再有 svn_preflight_dirty 这个错误码的条目', () => {
    // spec §16.1 把它整条删掉了：普通的本地修改不再阻断建库或重建基线。
    // 留着一条说「请先去 TortoiseSVN 提交或还原」的引导，会把用户推回那个
    // 已经不存在、而且从来就走不通的死胡同。
    expect(wizardErrorHint('svn_preflight_dirty')).toBe('')
  })

  it('重建期间被外部改动时说清指针没动、重试即可', () => {
    // R3c（spec §16 / §16.1 竞态表）。用户最关心的是「我的基线坏了吗」，
    // 所以第一句必须是「没坏」。
    const h = wizardErrorHint('svn_rebuild_revision_moved')
    expect(h).toContain('没有改动')
    expect(h).toContain('重新点')
  })

  it('体检不通过时先撇清「不是你的本地修改」', () => {
    // §6.8.7 之后本地修改已经被 S 排除在外。不说这一句的话，一个手上有改动的
    // 用户会立刻把两件事联系起来，然后去做一件完全没用的事（清理工作区）。
    const h = wizardErrorHint('svn_baseline_lying')
    expect(h).toContain('本地修改')
    expect(h).toContain('update')
  })

  it('mixed-revision 给的是 svn update', () => {
    expect(wizardErrorHint('svn_preflight_mixed_revision')).toContain('update')
  })

  it('mixed-revision 点名最常见的成因：提交过、没 update（2026-10-28 用户实测）', () => {
    // 初始化前在 TortoiseSVN 里提交过别的文件就会撞上。只说「多个 revision」时
    // 用户认不出那是自己刚才那次提交
    expect(wizardErrorHint('svn_preflight_mixed_revision')).toContain('提交过')
  })

  it('mixed-revision 的后端消息已列出文件与成因时，不再重复一遍', () => {
    // 后端消息的真实开头（svn_preflight::mixed_revision_error）
    const backend =
      '白名单内出现了多个 revision：{3, 4}。大多数文件停在 r3，下面这些不在：\n' +
      '  - r4  Code/B.cs\n'
    expect(wizardErrorHint('svn_preflight_mixed_revision', backend)).toBe('')
  })

  it('未完成的操作要区分 Cleanup 与完整 update', () => {
    // 2026-09-14 实测：svn cleanup 清掉写锁，但 item="incomplete" 原样留着。
    // 只说「跑一次 Cleanup」的引导会让用户以为做完了，而校验还是过不去。
    const h = wizardErrorHint('svn_preflight_operation_in_progress')
    expect(h).toContain('Cleanup')
    expect(h).toContain('update')
  })

  it('已有 .git 时明确说本应用不会碰它', () => {
    expect(wizardErrorHint('svn_unconfigured_git')).toContain('不会')
  })

  it('没有对应条目时返回空串，而不是 undefined', () => {
    // 组件会直接把它渲染出来，undefined 会变成字符串 "undefined"
    expect(wizardErrorHint('something_unknown')).toBe('')
  })

  it('后端消息已经说过的引导不再重复一遍', () => {
    // 2026-09-14 真实窗口验证时撞见的：preflight 拦截时后端消息本身就把引导
    // 说完了，而这张表又几乎逐字说了一遍，界面上并排两段一模一样的话
    // ——用户会以为那是两条不同的提示，然后花时间找区别。
    //
    // 判据是前 12 个字，所以这里给的后端消息必须与表里那条**开头一致**，
    // 否则测的就不是抑制逻辑而是别的东西。
    const backend =
      '白名单内有 obstructed（svn status 的 `~`）：\n  - Code/A.cs\n' +
      '本应用不会替你动这些路径，请先在 TortoiseSVN 里处理。'
    expect(wizardErrorHint('svn_preflight_obstructed', backend)).toBe('')
  })

  it('后端消息没说的引导仍然显示', () => {
    // 抑制只针对重复，不能把这张表整个变成摆设
    const terse = '核对失败'
    expect(wizardErrorHint('include_set_mismatch', terse)).toContain('.gitignore')
  })

  it('纳入集合不一致：预检与最终核对两种标题下都给出「怎么处理」', () => {
    // 后端报告只说「是哪条规则」，怎么处理由这条引导说——两道核对的标题不同，
    // 不能因为其中一个恰好与引导开头重合就被当成重复而吞掉。
    for (const headline of [
      'git 实际跟踪的文件集合与配置声明的不一致。',
      '录制基线前预检没通过：按当前的忽略规则，git 将要收进来的文件与配置声明的不一致。',
    ]) {
      const hint = wizardErrorHint('include_set_mismatch', headline)
      expect(hint).toContain('.gitignore')
      expect(hint).toContain('嵌套')
    }
  })
})

describe('healthNeedsRebuild', () => {
  it('基线在撒谎时给出重建的入口', () => {
    // 这是它存在的全部理由：`lying` 只在 revision 对得上时出现，而重建按钮
    // 挂在失同步横幅上——那时横幅不在。不给入口的话，体检等于只负责报丧。
    expect(healthNeedsRebuild({ status: 'lying', files: ['Code/A.cs'] })).toBe(true)
  })

  it('「没验成」不给——它不是「坏了」', () => {
    // 断网、URL 不可达都落在 notChecked。那时推用户去重建，等于拿一次网络抖动
    // 换一个新基线，而旧基线只能从 reflog 找回来。
    expect(healthNeedsRebuild({ status: 'notChecked', reason: '断网' })).toBe(false)
  })

  it('体检通过时不给', () => {
    expect(healthNeedsRebuild({ status: 'verified', rev: 7, unverified: [] })).toBe(false)
  })

  it('一处文件都没列出来的 lying 也照样给入口', () => {
    // `describeHealth` 对这种形态的文案是「这本身不该发生，请重建基线」——
    // 既然文案在劝重建，入口就不能反而消失。
    expect(healthNeedsRebuild({ status: 'lying', files: [] })).toBe(true)
  })
})

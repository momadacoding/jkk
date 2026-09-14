import { describe, expect, it } from 'vitest'
import {
  WIZARD_STEPS,
  describeHealth,
  describePinned,
  nextStep,
  parseExtensions,
  stepStatus,
  wizardErrorHint,
} from '@/engine/svnInit'

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
    const d = describeHealth({ status: 'verified', rev: 12346 })
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

  it('三种状态的 tone 互不相同', () => {
    // 防的是「都返回 warn 也能让上面三条通过」那种实现
    const tones = [
      describeHealth({ status: 'verified', rev: 1 }).tone,
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
  it('P1 不干净时明确说「不会替你 revert」', () => {
    // spec §8.1 #2：revert 用户的本地修改是破坏性的，只拦住并引导去 TortoiseSVN。
    // 这句话必须出现在 UI 上，否则用户会以为点一下就能自动清理。
    const h = wizardErrorHint('svn_preflight_dirty')
    expect(h).toContain('TortoiseSVN')
    expect(h).toContain('不会')
  })

  it('mixed-revision 给的是 svn update', () => {
    expect(wizardErrorHint('svn_preflight_mixed_revision')).toContain('update')
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
    // 2026-09-14 真实窗口验证时撞见的：P1 拦截时后端消息本身就以
    // 「白名单内的工作副本不干净，共 1 处…本应用不会替你 revert…」结尾，
    // 而这张表又几乎逐字说了一遍，界面上并排两段一模一样的话
    // ——用户会以为那是两条不同的提示，然后花时间找区别。
    const backend =
      '白名单内的工作副本不干净，共 1 处：\n  - Code/A.cs （modified）\n' +
      '本应用不会替你 revert（那是破坏性操作），请先在 TortoiseSVN 里提交或还原这些改动，再回来建库。'
    expect(wizardErrorHint('svn_preflight_dirty', backend)).toBe('')
  })

  it('后端消息没说的引导仍然显示', () => {
    // 抑制只针对重复，不能把这张表整个变成摆设
    const terse = '核对失败'
    expect(wizardErrorHint('include_set_mismatch', terse)).toContain('.gitignore')
  })
})

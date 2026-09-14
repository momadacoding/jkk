/**
 * SVN 初始化向导的纯逻辑：步骤状态机、输入解析、结果与错误的显示分类。
 *
 * 无 React、无 Tauri 依赖，可直接单测（AGENTS.md §7.2）。组件只做渲染与事件绑定。
 *
 * 1a 的 P2-2 已经证明过「判断散在组件里」会漏——同一个 `confirm()` 的两半，
 * 两轮 review 才抓齐。所以这里的规矩是：**任何「该显示什么 / 能不能继续」的判断
 * 都必须落在这个文件里**，组件不写 if。
 */

// ---------------------------------------------------------------------------
// 与 Rust 侧一一对应的类型
//
// Tauri 把 Rust 的 snake_case 转成 JS 的 camelCase，这层转换**没有编译期检查**。
// 下面每个字段名都由 `src-tauri/tests/svn_backend.rs` 里的 serde 断言钉住。
// ---------------------------------------------------------------------------

/** `engine::svn_preflight::Preflight` */
export interface Preflight {
  /** 记账单位 N，取自 nodeRevs（**不是** `svn info .` 的 revision）。 */
  nodeRev: number
  /** Repository Root。 */
  reposRoot: string
  uuid: string
}

/** `engine::svn_externals::PinnedExternal` */
export interface PinnedExternal {
  definedIn: string
  localPath: string
  pinnedRev: number
}

/**
 * `engine::svn_health::Health`，按 `status` 判别的联合类型。
 *
 * 三者必须分开（spec §6.8.6）：把「没能验证」当成「验证不通过」会让离线场景
 * 寸步难行，当成「验证通过」则等于这条检查不存在。
 */
export type Health =
  | { status: 'verified'; rev: number }
  | { status: 'lying'; files: string[] }
  | { status: 'notChecked'; reason: string }

/** `engine::svn_init::InitReport` */
export interface InitReport {
  nodeRev: number
  baseline: string
  health: Health
  pinned: PinnedExternal[]
}

/** `engine::svn_init::RebuildReport` */
export interface RebuildReport {
  nodeRev: number
  oldBaseline: string
  newBaseline: string
  health: Health
}

// ---------------------------------------------------------------------------
// 步骤状态机
// ---------------------------------------------------------------------------

export type StepId = 'choose' | 'preflight' | 'confirm' | 'done'

export interface WizardStep {
  id: StepId
  title: string
  /** 这一步在做什么，显示在标题下面。 */
  hint: string
}

/** 步骤顺序就是 spec §6.5 的序列，UI 不该自己排。 */
export const WIZARD_STEPS: readonly WizardStep[] = [
  { id: 'choose', title: '选择范围', hint: '哪些目录交给 git 管，收哪些扩展名' },
  { id: 'preflight', title: '检查工作副本', hint: 'P1–P5：干净、单一 revision、同一仓库' },
  { id: 'confirm', title: '确认并建库', hint: '建 .git、录首个基线、体检' },
  { id: 'done', title: '完成', hint: '已切到 work 分支' },
] as const

/** 下一步；最后一步返回 `null`。 */
export function nextStep(current: StepId): StepId | null {
  const i = WIZARD_STEPS.findIndex((s) => s.id === current)
  if (i < 0 || i === WIZARD_STEPS.length - 1) return null
  return WIZARD_STEPS[i + 1].id
}

export type StepState = 'done' | 'current' | 'pending'

/** `step` 相对于 `current` 处在什么状态。 */
export function stepStatus(step: StepId, current: StepId): StepState {
  const a = WIZARD_STEPS.findIndex((s) => s.id === step)
  const b = WIZARD_STEPS.findIndex((s) => s.id === current)
  if (a < b) return 'done'
  if (a === b) return 'current'
  return 'pending'
}

// ---------------------------------------------------------------------------
// 输入解析
// ---------------------------------------------------------------------------

/**
 * 把用户输入的扩展名串解析成配置需要的数组。
 *
 * Rust 侧 `svn_config::validate` 要求每一条以 `.` 开头，且 `*` 单独放行
 * （`*` 是 spec §6.3 明文给出的全纳入写法）。用户会输入 `cs, .ts ,cs`
 * ——能自动修的就修掉，不必为此弹一个他看不懂的校验错误。
 *
 * 给 `*` 补成 `.*` 会让校验失败，所以它必须原样放过。
 */
export function parseExtensions(raw: string): string[] {
  const out: string[] = []
  for (const piece of raw.split(',')) {
    const t = piece.trim()
    if (!t) continue
    const norm = t === '*' ? '*' : t.startsWith('.') ? t : `.${t}`
    if (!out.includes(norm)) out.push(norm)
  }
  return out
}

// ---------------------------------------------------------------------------
// 结果的显示分类
// ---------------------------------------------------------------------------

export type Tone = 'ok' | 'warn' | 'error'

export interface Notice {
  tone: Tone
  title: string
  detail: string
  /** 是否阻断流程。**`notChecked` 不阻断**，这是 spec §6.8.6 的要害。 */
  blocking: boolean
}

/** 把体检结果翻译成一条界面提示。 */
export function describeHealth(h: Health): Notice {
  switch (h.status) {
    case 'verified':
      return {
        tone: 'ok',
        title: `基线已通过体检（r${h.rev}）`,
        detail: '磁盘内容与服务器上该 revision 一致。',
        blocking: false,
      }
    case 'lying':
      return {
        tone: 'error',
        title: `基线与服务器不符，共 ${h.files.length} 处`,
        detail:
          h.files.length > 0
            ? h.files.join('\n')
            : '（没有列出具体文件，这本身不该发生，请重建基线）',
        blocking: true,
      }
    case 'notChecked':
      return {
        tone: 'warn',
        // 这句文案是 spec §6.8.6 第三行的直接体现：不阻断，但必须如实显示
        title: '基线未经体检',
        detail: `没能连上服务器核对，这**不代表基线有问题**。原因：${h.reason}`,
        blocking: false,
      }
  }
}

/**
 * pinned 模式的常驻说明（spec §6.5.3）。没有钉死的 external 时返回 `null`。
 *
 * 命中**不阻断**——改别人的 `svn:externals` 定义不是一个客户端能要求的事，
 * 而且客户端自己就能绕开（显式指名那个目录 `svn update` 不认 peg）。
 * 但残余风险必须常驻说明：外部工具在项目根跑递归 update 会把代码退回 peg。
 */
export function describePinned(pinned: PinnedExternal[]): Notice | null {
  if (pinned.length === 0) return null
  const list = pinned.map((p) => `${p.localPath} → r${p.pinnedRev}`).join('\n')
  return {
    tone: 'warn',
    title: `白名单覆盖到 ${pinned.length} 条被钉死的 external，已进入 pinned 模式`,
    detail:
      `${list}\n\n` +
      '本应用更新时会改用「项目根 --ignore-externals + 逐个显式更新」的方式，' +
      '你看到的行为不变。\n' +
      '残余风险：**其他工具**（TortoiseSVN 的 SVN Update、命令行 svn up）在项目根跑' +
      '递归 update 时，会把这些目录退回被钉住的 revision，而工作副本看起来是干净的。' +
      '出现这种情况时本应用会报出来，一条 svn update 就能拉回。',
    blocking: false,
  }
}

/**
 * 错误码 → 给用户的下一步引导。
 *
 * 没有对应条目时返回**空串**而不是 `undefined`：组件会直接渲染它，
 * `undefined` 会变成字符串 `"undefined"` 显示在界面上。
 */
export function wizardErrorHint(code: string): string {
  const table: Record<string, string> = {
    svn_preflight_dirty:
      '白名单内的工作副本不干净。本应用**不会**替你 revert（那是破坏性操作），' +
      '请先在 TortoiseSVN 里提交或还原这些改动，再回来建库。',
    svn_preflight_mixed_revision:
      '工作副本里有多个 revision。先跑一次完整的 svn update 把它对齐。' +
      '如果白名单落在 external 里，update 解决不了——错误详情里会说明。',
    svn_preflight_operation_in_progress:
      '上一次 svn 操作没跑完。先跑 Cleanup 释放写锁；' +
      '**只跑 Cleanup 往往不够**，还要再跑一次完整的 svn update 把没更新完的节点补齐。',
    svn_cross_repo:
      '白名单跨了两个 SVN 仓库。svn commit 对跨仓库的 target 列表会直接拒绝，' +
      '交付永远做不成——请把白名单收到同一个仓库内。',
    svn_unconfigured_git:
      '这个 SVN 工作副本里已经有一个 .git，但没有本应用的配置文件。' +
      '本应用**不会**碰它，请自行确认来源后处理。',
    svn_already_initialized: '这个目录已经建过库了。要重建基线请用「重建基线」，不是重新初始化。',
    svn_baseline_lying:
      '首个基线没通过体检，已经整个回退，刚建的 .git 也删掉了。' +
      '先确认工作副本确实与服务器一致（跑一次完整的 svn update），再重试。',
    include_set_mismatch:
      'git 实际跟踪的文件集合与配置声明的不一致。错误详情里指出了是哪条规则做的决定' +
      '——最常见的原因是工作树里有一个 .gitignore 盖掉了本应用生成的规则。',
    svn_baseline_no_trailer: '基线分支上没有本应用写的记账信息，它不是本应用录出来的基线。请重建基线。',
  }
  return table[code] ?? ''
}

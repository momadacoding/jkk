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
  /**
   * `unverified` 是这次检查**覆盖不到**的路径（§6.8.7 的 `S`）。空表示全覆盖。
   *
   * 判据 `D \ S == ∅` 比的是**磁盘与服务器**，而 `S` 里那些路径上的磁盘内容正是
   * 用户改过的——这条检查对它们什么也证明不了。工作副本干净时才 `S = ∅`，
   * 而「程序员的工作区 99% 的时间都是脏的」正是 §16.1 放松干净要求的理由。
   */
  | { status: 'verified'; rev: number; unverified: string[] }
  | { status: 'lying'; files: string[] }
  | { status: 'notChecked'; reason: string }

/** `engine::svn_init::PhaseTiming` */
export interface PhaseTiming {
  phase: string
  millis: number
}

/** `engine::svn_gitignore::GitignoreRule`：一个 `.gitignore` 里起作用的一行。 */
export interface GitignoreRule {
  line: number
  pattern: string
  /** `excludes` 挡住了 SVN 里有的文件；`includes` 用 `!` 把用户没选的文件拉进了 git。 */
  effect: 'excludes' | 'includes'
  count: number
  /** 最多 3 个，仓库相对路径。 */
  samples: string[]
}

/** `engine::svn_gitignore::GitignoreConflict`：一个起作用的 `.gitignore`。 */
export interface GitignoreConflict {
  /** 仓库相对路径。建库时原样作为 `acknowledged` 交回后端对账。 */
  path: string
  /** 完整的 Windows 路径，给人看的。 */
  fullPath: string
  /** 受 SVN 版本控制——团队的规则，不改名。 */
  versioned: boolean
  /**
   * 现在被外层的 `.gitignore` 挡着、git 根本读不到它；外层改名停用之后才开始起作用。
   * 勾上「尊重」时外层不停用，它也就不起作用。
   */
  activeAfterRename: boolean
  rules: GitignoreRule[]
}

/** `engine::svn_init::RenamedGitignore`，两端都是完整路径。 */
export interface RenamedGitignore {
  from: string
  to: string
}

/** `engine::svn_init::GitignoreOutcome`：建库对 `.gitignore` 做了什么。 */
export interface GitignoreOutcome {
  respected: boolean
  conflicts: GitignoreConflict[]
  disabled: RenamedGitignore[]
}

/** `engine::svn_init::InitPreflight`：第 2 步的结果。 */
export interface InitPreflight extends Preflight {
  /** 空表示没有冲突，第 3 步不显示勾选框。 */
  gitignoreConflicts: GitignoreConflict[]
}

/** `engine::svn_init::InitReport` */
export interface InitReport {
  nodeRev: number
  baseline: string
  health: Health
  pinned: PinnedExternal[]
  /** 各阶段耗时，按执行顺序。失败时后端把同样的内容附在错误正文末尾。 */
  timings: PhaseTiming[]
  gitignore: GitignoreOutcome
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
  { id: 'preflight', title: '检查工作副本', hint: 'P1–P5：单一 revision、同一仓库（本地修改不影响）' },
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

// 码点一律写成转义：零宽字符肉眼看不见，编辑器或格式化工具删掉它也不会有人发现。
//
// 先做 NFKC：全角的逗号、分号、句点、字母、数字、星号都折成 ASCII，半角片假名的
// `､` `｡` 折成 `、` `。`。之后要单独处理的只剩中文顿号、中文句号和零宽字符——
// 不用再逐个列全角字符，列不全的那个就是下一次静默失效（第三轮独立 review）。

/**
 * 分隔符：逗号、分号、中文顿号、空白，以及**两个以上连续的点**——`.cs。.lua` 换完句号
 * 是 `.cs..lua`，`.。lua` 是 `..lua`，按点切开才是本意。
 */
const EXTENSION_SEPARATORS = /(?:[,;\u{3001}\s]|\.{2,})+/u
/** 中文句号，当成 `.` */
const IDEOGRAPHIC_FULL_STOP = /\u{3002}/gu
/**
 * 复制粘贴带进来的零宽字符，切分之前去掉。U+200B–U+200D、U+2060 不属于 `\s`，
 * 留着会让扩展名静默匹配不到文件；U+FEFF 属于 `\s`，但它夹在扩展名中间是被粘进来的，
 * 去掉而不是当成分隔符切开。
 */
const ZERO_WIDTH = /[\u{200B}-\u{200D}\u{2060}\u{FEFF}]/gu

/**
 * 把用户输入的扩展名串解析成配置需要的数组。
 *
 * Rust 侧 `svn_config::validate` 要求每一条以 `.` 开头，且 `*` 单独放行
 * （`*` 是 spec §6.3 明文给出的全纳入写法）。用户会输入 `cs, .ts ,cs`
 * ——能自动修的就修掉，不必为此弹一个他看不懂的校验错误。
 *
 * 给 `*` 补成 `.*` 会让校验失败，所以它必须原样放过。
 *
 * 分隔符不只认 ASCII 逗号：中文输入法下顺手打出的 `、` `，` `；` 以及空白都切开。
 * 只认 `,` 时 `.cs、.lua` 会整串成为一个扩展名，生成的反选规则匹配不到任何文件，
 * 白名单整体失效却没有任何报错（2026-10-28 现场）。扩展名本身不会含这些字符。
 * 同理，全角字符折成 ASCII，中文句号换成 `.`，零宽字符直接去掉，连续的点当分隔，
 * `*.cs` 去掉前导 `*`，末尾的 `.` 去掉。
 *
 * Rust 侧 `svn_config::validate_new_extensions` 按白名单兜底（只收 ASCII 字母、数字与
 * `. _ - + ~`，不许空段）：这里漏掉的，建库时会在 `git init` 之前报错，不会静默建出
 * 一个什么都不管的仓库。
 */
export function parseExtensions(raw: string): string[] {
  const out: string[] = []
  const cleaned = raw
    .normalize('NFKC')
    .replace(ZERO_WIDTH, '')
    .replace(IDEOGRAPHIC_FULL_STOP, '.')
  for (const piece of cleaned.split(EXTENSION_SEPARATORS)) {
    // `*.cs` 是很顺手的 glob 写法，去掉 `*` 即是本意；末尾的 `.` 来自句末打的 `。`，
    // 留着就是 `.lua.`，匹配不到任何文件（Windows 上文件名不能以点结尾）
    const t = piece.trim().replace(/^\*(?=\.)/, '').replace(/\.+$/, '')
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
/**
 * 体检结论要不要顺带给一条「重建基线」的出路。
 *
 * # 只有 `lying` 给
 *
 * 「重建基线」在界面上**只有失同步横幅一个入口**，而 `lying` 恰恰发生在
 * **revision 完全对得上**的时候——也就是那条横幅不在的时候。于是体检告诉你
 * 基线坏了，界面上却没有任何地方能修它（2026-09-19 用户在手工测试中撞上）。
 *
 * 这与 2026-09-16 独立 review 修掉的那条是同一个毛病：体检入口当初也被绑在
 * 失同步横幅上，「等于只在不需要它的时候提供它」。那次只解开了体检。
 *
 * # ⚠️ `notChecked` 绝不能给
 *
 * 「没验成」不是「坏了」。断网、URL 不可达都会落到这里，那时推着用户去重建，
 * 等于拿一次网络抖动换一个新基线——而重建是要动 `svn/base` 指针的，旧基线只能
 * 从 reflog 找回来。`verified` 同理，没有任何理由重建。
 */
export function healthNeedsRebuild(h: Health): boolean {
  return h.status === 'lying'
}

/** 列给用户看的路径上限。几千条没人看，还会把对话框撑爆。 */
export const HEALTH_MAX_LISTED = 10

/** 把路径清单裁到 [`HEALTH_MAX_LISTED`] 条，多出来的只报个数。 */
function capPaths(paths: string[]): string[] {
  if (paths.length <= HEALTH_MAX_LISTED) return paths
  return [
    ...paths.slice(0, HEALTH_MAX_LISTED),
    `…还有 ${paths.length - HEALTH_MAX_LISTED} 条`,
  ]
}

export function describeHealth(h: Health): Notice {
  switch (h.status) {
    case 'verified':
      // ⚠️ **不能无条件说「一致」。** 2026-09-21 独立 review：`unverified` 里的
      // 路径被判据整批豁免了，这条检查对它们一无所知，而那正是用户此刻正在改的
      // 那些文件。原来的文案把「部分覆盖」说成了「全覆盖」——用户看到绿色的
      // 「已通过体检」，以为验的是全部。
      return {
        tone: 'ok',
        title:
          h.unverified.length > 0
            ? `基线已通过体检（r${h.rev}，${h.unverified.length} 个文件未覆盖）`
            : `基线已通过体检（r${h.rev}）`,
        detail:
          h.unverified.length > 0
            ? [
                `磁盘内容与服务器上该 revision 一致——**你本地改过的这 ${h.unverified.length} 个文件除外**，`,
                '这次检查对它们什么也证明不了（它比的是磁盘与服务器，而这些路径上的磁盘内容正是你改的）。',
                '',
                ...capPaths(h.unverified),
                '',
                '把这些改动提交或还原之后再体检一次，覆盖面才是完整的。',
              ].join('\n')
            : '磁盘内容与服务器上该 revision 一致。',
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
        detail: `没能连上服务器核对，这不代表基线有问题。原因：${h.reason}`,
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
      '残余风险：其他工具（TortoiseSVN 的 SVN Update、命令行 svn up）在项目根跑' +
      '递归 update 时，会把这些目录退回被钉住的 revision，而工作副本看起来是干净的。' +
      '出现这种情况时本应用会报出来，一条 svn update 就能拉回。',
    blocking: false,
  }
}

/**
 * `1234` → `1.2 秒`，`95000` → `1 分 35 秒`。
 *
 * 与 Rust 侧 `svn_init::human_duration` 同一口径：失败时耗时由后端拼进错误正文，
 * 成功时由这里格式化，两处写法不一致的话，用户拿两次结果对比会对不上号。
 *
 * 只用整数运算、四舍五入：`toFixed(1)` 与 Rust 的 `{:.1}` 对 `1.25` 这种恰好
 * 落在中间的值取整方向不同（一个给 1.3、一个给 1.2）。档位按**取整之后**的值切，
 * 否则 59.95 秒会显示成「60.0 秒」而不是「1 分 0 秒」。
 */
export function formatDuration(millis: number): string {
  const tenths = Math.floor((millis + 50) / 100)
  if (tenths < 600) return `${Math.floor(tenths / 10)}.${tenths % 10} 秒`
  const secs = Math.floor((millis + 500) / 1000)
  return `${Math.floor(secs / 60)} 分 ${secs % 60} 秒`
}

/**
 * 建库成功页的耗时说明。没有耗时时返回 `null`。
 *
 * 2026-10-27 用户在公司装机版上反馈「确认并建库」要半小时，但没法知道花在哪一步
 * ——装机版不落 Rust 日志，所以耗时只能放在界面上。
 */
export function describeTimings(timings: PhaseTiming[] | undefined): Notice | null {
  if (!timings || timings.length === 0) return null
  const total = timings.reduce((sum, t) => sum + t.millis, 0)
  return {
    tone: 'ok',
    title: `各阶段耗时（共 ${formatDuration(total)}）`,
    detail: timings.map((t) => `${t.phase}  ${formatDuration(t.millis)}`).join('\n'),
    blocking: false,
  }
}

// ---------------------------------------------------------------------------
// 起作用的 .gitignore（2026-10-28 设计 §3）
// ---------------------------------------------------------------------------

/** 一条规则写成一行：「第 N 行 `rule`：…，例如 a、b」。 */
function describeRule(r: GitignoreRule, what: 'conflict' | 'outcome'): string {
  const head = `第 ${r.line} 行 \`${r.pattern}\``
  const eg = r.samples.length > 0 ? `，例如 ${r.samples.join('、')}` : ''
  if (what === 'outcome') return `${head}：${r.count} 个${eg}`
  return r.effect === 'includes'
    ? `${head}：多收 ${r.count} 个你没选的文件${eg}`
    : `${head}：挡掉 ${r.count} 个 SVN 里有的文件${eg}`
}

/**
 * 第 3 步「确认并建库」按钮上方的冲突清单。没有冲突时返回 `null`——那时连勾选框都不显示。
 *
 * 写完整路径：git 报的是 `.gitignore` 的实际位置，可能在任意一层子目录，
 * 用户要能照着路径去打开它（用户反馈第 1 点）。
 */
export function describeGitignoreConflicts(conflicts: GitignoreConflict[]): Notice | null {
  if (conflicts.length === 0) return null
  const blocks = conflicts.map((c) =>
    [
      c.fullPath +
        (c.versioned ? '（受 SVN 版本控制）' : '') +
        (c.activeAfterRename ? '（外层的 .gitignore 停用之后才会起作用）' : ''),
      ...c.rules.map((r) => `  ${describeRule(r, 'conflict')}`),
    ].join('\n'),
  )
  return {
    tone: 'warn',
    title: '这些 .gitignore 会影响交给 git 管的文件',
    detail: blocks.join('\n\n'),
    blocking: false,
  }
}

/**
 * 按当前的勾选，「建库」能不能点；不能时返回原因（与后端的拦截同义）。
 *
 * 后端仍会再拦一次——这里只是让用户不必点了才知道。
 */
export function gitignoreBlockReason(conflicts: GitignoreConflict[], respect: boolean): string | null {
  if (!respect) {
    const versioned = conflicts.filter((c) => c.versioned)
    if (versioned.length > 0) {
      return (
        `${versioned.map((c) => c.fullPath).join('、')} 受 SVN 版本控制，是团队提交的规则，本应用不会改它。` +
        '请勾选「尊重已有的 .gitignore 规则」，或先和团队确认。'
      )
    }
    return null
  }
  // 尊重时外层不停用，藏在后面的那些不会被 git 读到，与这次建库无关
  for (const c of conflicts.filter((x) => !x.activeAfterRename)) {
    const r = c.rules.find((x) => x.effect === 'includes')
    if (r) {
      return (
        `${c.fullPath} 第 ${r.line} 行 \`${r.pattern}\` 会把你没选的文件拉进 git。` +
        '「尊重」只允许 .gitignore 少收文件、不允许多收。请取消勾选，或先修改这一行。'
      )
    }
  }
  return null
}

/**
 * 建库请求里实际带的「尊重」。
 *
 * 没有冲突时勾选框不渲染，用户看不到这个选项，它就不能生效：重新检查之后清单变空时，
 * 之前勾上的 `true` 还留在 state 里（/code-review 第 5 条）。
 */
export function effectiveRespect(conflicts: GitignoreConflict[], respect: boolean): boolean {
  return respect && conflicts.length > 0
}

/** 完成页：改了哪些名、或按尊重口径没收哪些文件。两样都没有时返回 `null`。 */
export function describeGitignoreOutcome(o: GitignoreOutcome | undefined): Notice | null {
  if (!o) return null
  if (o.disabled.length > 0) {
    return {
      tone: 'warn',
      title: `停用了 ${o.disabled.length} 个 .gitignore`,
      detail: [...o.disabled.map((d) => `${d.from} → ${d.to}`), '', '想恢复就把它改回原名。'].join('\n'),
      blocking: false,
    }
  }
  if (o.respected && o.conflicts.length > 0) {
    const blocks = o.conflicts.map((c) =>
      [
        c.fullPath,
        ...c.rules.filter((r) => r.effect === 'excludes').map((r) => `  ${describeRule(r, 'outcome')}`),
      ].join('\n'),
    )
    return {
      tone: 'ok',
      title: '按 .gitignore 的规则，这些文件没有交给 git 管',
      detail: blocks.join('\n\n'),
      blocking: false,
    }
  }
  return null
}

/**
 * 这个错误是不是「出现了你没看过的 .gitignore」：是的话向导要重新跑第 2 步、
 * 把新清单摆到第 3 步给用户看，而不是只报一个错。
 */
export function needsReconfirm(code: string): boolean {
  return code === 'svn_gitignore_unconfirmed'
}

/**
 * 把错误正文拆成「一行标题 + 详情」。
 *
 * 后端的多行错误（纳入集合不一致的分组报告、附在末尾的耗时）原来整段放在
 * `<strong>` 里，换行全被折叠，几十行挤成一大段，用户看不出每个文件后面跟的
 * 裁决规则（2026-10-27 用户报告）。详情交给保留换行的 `<pre>` 渲染。
 */
export function splitErrorMessage(message: string): { headline: string; detail: string } {
  const i = message.indexOf('\n')
  if (i < 0) return { headline: message, detail: '' }
  return {
    headline: message.slice(0, i),
    detail: message.slice(i + 1).replace(/^\n+/, '').replace(/\n+$/, ''),
  }
}

/**
 * 错误码 → 给用户的下一步引导。
 *
 * 没有对应条目时返回**空串**而不是 `undefined`：组件会直接渲染它，
 * `undefined` 会变成字符串 `"undefined"` 显示在界面上。
 *
 * # 为什么要收 `message`
 *
 * 2026-09-14 在真实窗口里跑到 P1 拦截时发现：后端的错误消息本身已经把
 * 「不会替你 revert，请去 TortoiseSVN」说完了，这张表又几乎逐字说了一遍，
 * 界面上并排显示两段一模一样的话——用户会以为那是两条不同的提示，
 * 然后花时间去找区别。
 *
 * 所以这张表仍然保留（它是 UI 这一侧的保证：后端消息哪天改短了，引导不会跟着消失），
 * 但**后端已经说过的就不再重复**。判据是前 12 个字——足够长到不会误伤，
 * 又不依赖整句逐字相同。
 */
export function wizardErrorHint(code: string, message = ''): string {
  const table: Record<string, string> = {
    svn_preflight_obstructed:
      '白名单内有 obstructed（svn status 的 `~`）——通常是一个受版本控制的文件被' +
      '换成了同名目录。这种位置上 svn 自己都说不清是什么，取不到它在服务器上的内容。' +
      '本应用不会替你动这些路径（删目录、revert 都是破坏性操作）。' +
      '除了 `~`，其余本地修改都不再阻断建库或重建基线。',
    // 开头与后端消息一致：后端列出了掉队文件与成因时这段整条不显示
    svn_preflight_mixed_revision:
      '白名单内出现了多个 revision——最常见的原因是在 TortoiseSVN 里提交过文件、' +
      '之后没有 update。先在 TortoiseSVN 里对工程根目录跑一次 Update 把它对齐。' +
      '如果白名单落在 external 里，update 解决不了——错误详情里会说明。',
    svn_preflight_operation_in_progress:
      '上一次 svn 操作没跑完。先跑 Cleanup 释放写锁；' +
      '只跑 Cleanup 往往不够，还要再跑一次完整的 svn update 把没更新完的节点补齐。',
    svn_cross_repo:
      '白名单跨了两个 SVN 仓库。svn commit 对跨仓库的 target 列表会直接拒绝，' +
      '交付永远做不成——请把白名单收到同一个仓库内。',
    svn_unconfigured_git:
      '这个 SVN 工作副本里已经有一个 .git，但没有本应用的配置文件。' +
      '本应用不会碰它，请自行确认来源后处理。',
    svn_already_initialized: '这个目录已经建过库了。要重建基线请用「重建基线」，不是重新初始化。',
    svn_baseline_lying:
      '首个基线没通过体检，已经整个回退，刚建的 .git 也删掉了。' +
      '注意这里说的**不是**你自己的本地修改——那些已经被排除在外了。' +
      '报出来的文件是「磁盘与服务器对不上、而 svn 又说它没被改过」，' +
      '最常见的原因是漏跑了一次完整的 svn update。',
    svn_rebuild_revision_moved:
      '重建过程中工作副本被外部改动了（多半是 TortoiseSVN 那边提交或更新了一次）。' +
      '基线指针没有改动，重新点一次「重建基线」即可。',
    svn_baseline_classification_conflict:
      'svn status 说某个文件在服务器上有，取它的内容时 svn 又说它还没提交过。' +
      '两个判断对不上，所以停下来了——多半是录制过程中工作副本被外部改动了，请重试一次。',
    include_set_mismatch:
      '上面的清单按「是哪条规则做的决定」分组，每组列几个样例；查不到规则的单独成组。' +
      '被某个 .gitignore 挡掉的：把那个 .gitignore 挪走或删掉对应的行——' +
      '最常见的来源是以前在这个目录里用过 git、只删了 .git 没删 .gitignore。' +
      '位于嵌套 git 仓库里的：把那个目录移出白名单，或者确认后删掉它里面的 .git。',
    svn_baseline_no_trailer: '基线分支上没有本应用写的记账信息，它不是本应用录出来的基线。请重建基线。',
    svn_gitignore_unconfirmed: '清单已经刷新，请看过之后重新点「建库」。',
    svn_gitignore_versioned:
      '受版本控制的 .gitignore 是团队规则。要按它来就勾选「尊重」；要改它请先和团队商量，用 TortoiseSVN 提交。',
    svn_gitignore_includes: '「尊重」只接受 .gitignore 少收文件。取消勾选会把这个 .gitignore 改名停用。',
  }
  const hint = table[code]
  if (!hint) return ''
  const head = hint.replace(/\*\*/g, '').slice(0, 12)
  return message.replace(/\*\*/g, '').includes(head) ? '' : hint
}

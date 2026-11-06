/**
 * 失同步状态 / 服务器侧核对结果 → 横幅要显示什么（spec §6.6、§6.3.1）。
 *
 * 纯逻辑，无 React 依赖（`AGENTS.md` §7.2）。组件只负责把这里算出来的东西渲染
 * 出来，不自己判断哪种情况给哪条出口——那种判断散进组件之后就再也测不到了。
 */

/**
 * 后端 `svn_sync::SyncVerdict` 的形状。
 *
 * Rust 侧是 `#[serde(tag = "kind", rename_all = "camelCase")]`，所以判别字段是
 * `kind`。⚠️ 与 `Health` / `ServerAudit` 的 `status` **不是同一个键**，
 * 两处各自在 Rust 的定义处写明了。
 */
export type SyncVerdict =
  | { kind: 'inSync' }
  | { kind: 'wcLocked'; dirs: string[] }
  | {
      kind: 'externallyAdvanced'
      changed: ExternalChange[]
      added: ExternalChange[]
      removed: string[]
    }
  /** 只表示回退：有文件的节点 revision 比基线记的还旧（计划 Task 3）。 */
  | { kind: 'diverged'; nodeRev: number; baselineRev: number }
  | { kind: 'pinnedReset'; nodeRev: number; baselineRev: number; external: string }

/** 后端 `svn_sync::ExternalChange`：SVN 侧有新内容的一个文件。 */
export interface ExternalChange {
  path: string
  /** 最后修改 revision；后端读不到时给的是节点 revision。 */
  rev: number
  /** 匿名提交为 `null`。 */
  author: string | null
}

/**
 * 后端 `svn_follow_mark::WcDbStamp`。
 *
 * `mtimeNs` 是十进制**字符串**：纳秒时间戳超过 2^53，当 number 收会被舍入。只拿它比相等。
 */
export interface WcDbStamp {
  wcRoot: string
  mtimeNs: string
  size: number
}

/** 后端 `svn_sync::BaseMapSummary`。 */
export interface BaseMapSummary {
  floor: number
  /** 单列在更新 revision 的文件数。 */
  newer: number
  /** 墓碑数。 */
  gone: number
}

export interface SyncStatus {
  /** 基线映射的 floor。 */
  baselineRev: number
  nodeRevs: number[]
  verdict: SyncVerdict
  base: BaseMapSummary
  /** 当前分支不包含 `svn/base`，且分叉点是一条基线 commit：基线发布了，分支没跟上。 */
  headBehindBase: boolean
  /** `headBehindBase` 时分支基线到 `svn/base` 改过的文件（事故分析 6.6 的提交前提醒用）。旧后端没有。 */
  behindPaths?: string[]
}

/** 后端 `svn_server_set::ServerAudit`，判别字段是 `status`。 */
export type ServerAudit =
  | { status: 'matched' }
  | { status: 'missing'; files: string[]; total: number }
  | { status: 'notChecked'; reason: string }

/** `quiet` 是灰色：只告知、不需要用户动手（与常驻体检入口同一套样式）。 */
export type BannerTone = 'warn' | 'danger' | 'quiet'

export interface BannerAction {
  id:
    | 'follow'
    | 'rebuild'
    | 'check'
    | 'updateWhitelistDir'
    | 'migrateEol'
    | 'syncContinue'
    | 'syncAbort'
    | 'followContinue'
    | 'svnUpdate'
    | 'syncOutCleanup'
    | 'syncOutContinue'
    | 'syncOutAbort'
  label: string
}

/** 行尾规则的安装状态（spec §7.3.0），对应 Rust 的 `EolStatus`。 */
export interface EolStatus {
  ruleInstalled: boolean
  needsMigration: boolean
}

export interface BannerModel {
  tone: BannerTone
  title: string
  detail: string
  actions: BannerAction[]
  /** 给用户照抄的命令（本期一条 SVN 写命令都不发，出口是把命令交给用户）。 */
  command?: string
  /** 补充说明，比如「重建基线不会碰工作目录」。 */
  note?: string
  /** 逐行列出的文件（已排好版的展示文本）。 */
  files?: string[]
}

export interface AuditModel {
  tone: BannerTone
  title: string
  detail: string
  files: string[]
}

/** 重建基线的代价，spec §16 明确要求写给用户看。 */
const REBUILD_NOTE =
  '「重建基线」只把当前磁盘内容录成新的基线 commit：不会碰工作目录里的任何文件，' +
  '也不会推进你的工作分支。分支要不要跟进新基线，由你之后自己决定。'

const REBUILD: BannerAction = { id: 'rebuild', label: '重建基线…' }
const CHECK: BannerAction = { id: 'check', label: '体检基线' }

/**
 * 失同步状态 → 横幅。同步（或还没读到状态）时返回 `null`，整个横幅不渲染。
 *
 * `null` 与「一切正常」的横幅是两回事：打开仓库的那一瞬间状态是 `null`，
 * 那时挂任何东西都等于每次打开都闪一下。
 */
export function describeSync(status: SyncStatus | null): BannerModel | null {
  if (!status) return null
  const v = status.verdict
  switch (v.kind) {
    case 'inSync':
      // 基线与工作副本一致，但当前分支还在更早的基线上：上次跟进遇到冲突，或者切到了一条旧分支
      // （分析 §6.11.3）。这时「本地改动」是相对旧内容算的，要给「分支跟进」的入口
      if (!status.headBehindBase) return null
      return {
        tone: 'warn',
        title: '当前分支落后基线',
        detail:
          '基线已经前进，当前分支还停在更早的基线上，「本地改动」里会多出基线带进来的文件。' +
          '点「跟进」把这条分支的提交重放到新基线上，不会改动工作目录里的任何文件。' +
          // Review 3 第 2 条：只移分支的话，磁盘上的旧内容会显示成把 SVN 的内容改回去
          '如果是用「仅切换」或别的工具切过来的，磁盘上还是这条分支的旧内容（TortoiseSVN 显示为已修改），' +
          '跟进会改为提示「手动跟进」：那一步会把新内容写回磁盘。',
        actions: [{ id: 'follow', label: '跟进' }],
      }

    case 'wcLocked':
      // E15：活锁（TortoiseSVN 正在提交 / 更新）与陈旧锁（进程被杀）分不出来。
      // 不给按钮：活锁等它做完就好，陈旧锁要用户自己在 TortoiseSVN 里 Clean up，
      // 本应用不替他跑 cleanup
      return {
        tone: 'quiet',
        title: 'SVN 正在更新…',
        detail:
          `工作副本被 SVN 锁着（${v.dirs.join('、')}），可能有 TortoiseSVN 操作正在进行。` +
          '等它做完这条提示会自己消失；如果一直不消失，说明上次操作中途断了，' +
          '请在 TortoiseSVN 里对该目录执行 Clean up。',
        actions: [],
      }

    case 'externallyAdvanced': {
      // 分析 §6.10：只给「跟进」。不给「体检基线」：此时体检用的还是旧基线，必然把这些文件
      // 报成「基线与服务器不符」，而跟进本身就包含体检。措辞不说「你提交了」：局部 update
      // 拉下的同事改动也是这个判定，作者列出来让用户自己分辨
      const n = v.changed.length + v.added.length + v.removed.length
      const by = (c: ExternalChange) => (c.author ? `  ${c.author}` : '')
      const files = [
        ...v.changed.map((c) => `${c.path}  r${c.rev}${by(c)}`),
        ...v.added.map((c) => `${c.path}  r${c.rev}${by(c)}（新增）`),
        // 不写「已在 SVN 上删除」：本地分不出提交删除与 `--set-depth=exclude` 漏收（E17），
        // 要跟进时向服务器确认才知道
        ...v.removed.map((p) => `${p}（工作副本里已没有）`),
      ]
      return {
        tone: 'warn',
        title: `SVN 侧有 ${n} 个文件有新内容`,
        detail:
          '这些文件在 SVN 上已经是更新的版本（在 TortoiseSVN 里提交过，或更新了部分目录），' +
          `基线还停在旧的内容上（r${status.baselineRev}）。` +
          '此刻「本地改动」里会把它们也算进去；点「跟进」把基线推进到各文件的真实 revision，' +
          '不会改动工作目录里的任何文件。',
        actions: [{ id: 'follow', label: '跟进' }],
        files,
      }
    }

    case 'pinnedReset':
      // spec §6.6.2 点名的那个「具体且常见」的成因。**不要给重建基线**：
      // 那会把被钉死的旧内容录成一个标着新 revision 的基线，自己造出基线撒谎。
      return {
        tone: 'warn',
        title: '白名单被拽回了 external 的固定 revision',
        detail:
          `白名单所在的 external \`${v.external}\` 被 \`svn:externals\` 钉在 ` +
          `r${v.nodeRev}，而基线记的是 r${v.baselineRev}。` +
          '最常见的成因是有人在项目根跑了一次递归 `svn update`，' +
          'SVN 按 externals 的定义把它拉回了固定 revision。' +
          '这不需要重建基线——把那个目录单独更新回来即可。',
        actions: [{ id: 'updateWhitelistDir', label: '复制命令' }, CHECK],
        command: `svn update ${v.external}`,
      }

    case 'diverged':
      // 计划 Task 3 起只表示回退：前进（含原 A16 的整体前进）一律是 externallyAdvanced。
      // 回退是红色，「更新」对它会拒绝
      return {
        tone: 'danger',
        title: '工作副本被回退到了基线之前',
        detail:
          `工作副本里有文件被回退到了 r${v.nodeRev}，而基线记着它在 r${v.baselineRev}。` +
          '基线比磁盘还新，任何基于它的比较都不成立。',
        actions: [REBUILD, CHECK],
        note: REBUILD_NOTE,
      }
  }
}

/**
 * 常驻的「体检基线」入口要不要渲染。
 *
 * # 为什么不能只把它挂在失同步横幅里
 *
 * `describeSync` 对 `InSync` 返回 `null`，整条横幅不渲染，连带把唯一的体检入口
 * 也带走了（2026-09-16 独立 review）。而 **revision 一致根本不能证明基线内容
 * 正确**——`check_current_baseline` 报的 `Lying`（基线记的内容与服务器不符）
 * 恰恰发生在 revision 完全对得上的时候，`svn revert` 那类外部操作也不动 revision。
 *
 * 换句话说，把体检入口绑在失同步状态上，等于**只在不需要它的时候提供它**。
 *
 * 失同步横幅自己带了一个体检按钮，所以那时不重复渲染，保证任何时刻恰好一个入口。
 */
export function needsStandaloneCheck(
  isSvn: boolean,
  sync: SyncStatus | null,
  eol: EolStatus | null,
  /** 基线分支不存在（第 70 条）：体检同样要读基线，入口点了只会报错，出口在恢复横幅上。 */
  baselineMissing = false,
): boolean {
  // 只看「丢了且还没有结论」：标记在判定成功时不清，重建之后有了结论，入口就该回来
  // （PR 115 review）
  if (baselineMissing && !sync) return false
  // 迁移没做完时基线本来就不可信，挂一个「体检基线」只会给出误导性的结论
  // ——体检比的是磁盘与服务器，而此刻的问题出在基线自己的形状上。
  //
  // 第三个参数**故意不给默认值**：给了默认值，漏传的调用点会被编译器放过，
  // 而漏传的后果正是这条规则要防的。让 tsc 把每个调用点都逼出来。
  if (describeEolMigration(eol, sync) !== null) return false
  return isSvn && describeSync(sync) === null
}

/**
 * 迁移到底会动什么——横幅上的简版。
 *
 * # 这段话的历史，以及为什么有测试钉着它
 *
 * 第一版后端只重录基线、确实不推进工作分支，这里照实写了「不会推进你的工作分支」。
 * 后来修 review 的阻断时改成了「基线、工作分支、真实索引一起搬到新基线」，
 * **文案没跟着改**，于是确认框在用户按下按钮之前对他说了反话
 * （2026-09-25 PR #45 review）。
 *
 * 确认框是 git 状态变更前用户唯一的信息来源，所以 `svnSync.test.ts` 里有一组
 * 断言钉住这两段话的关键句——下一个改 `align_branch` 的人会在那里被拦下。
 */
const MIGRATE_EOL_NOTE =
  '工作目录里的文件一个都不会动；工作分支和暂存区会跟着新基线一起前移一笔' +
  '（迁移前它们本来就停在基线上）。' +
  '不迁移的话，文件会在被编辑或 update 之后一个一个地无故显示成「已修改」。'

/**
 * 确认框里的警告条。与 [`MIGRATE_EOL_NOTE`] 说的是同一件事，多披露两点。
 *
 * 标题和正文不与横幅共用（那两处回答的是不同的问题：横幅说「这是什么」，
 * 确认框说「你按下去会发生什么」），但**「会发生什么」这一段必须是同一份**
 * ——同一个机械后果写两遍，迟早说出两种话，上面那段历史就是这么来的。
 */
export const MIGRATE_EOL_WARNING =
  '工作目录里的文件一个都不会动；工作分支和暂存区会跟着新基线一起前移一笔' +
  '（迁移前它们本来就停在基线上，不一起搬过去的话每个文本文件都会立刻显示成「已修改」）。' +
  '本应用还会接管 .git/info/attributes：那里原有的内容会先备份成同目录的 attributes.bak，' +
  '但其中的规则从此不再生效。' +
  '不做的话，文件会在被编辑或 update 之后一个一个地无故显示成「已修改」。'

/**
 * 行尾迁移横幅。规则没装 ⇒ 这个仓库的基线是在旧的行尾设置下录的。
 *
 * # 为什么判据是「规则在不在」，不是「看起来有没有问题」
 *
 * spec §7.3.0 实测：规则生效之后 `git status` 会因为 git 的 stat cache 假报
 * 0 个脏文件，要等某个文件的 mtime 被碰一下才单独冒出来。任何基于「现在看着
 * 没事」的判据都会漏判，而漏判的表现正是用户最难自查的那种——文件一个一个地
 * 无故变脏。
 *
 * # 为什么 `pinnedReset` 时让位
 *
 * 迁移的动作就是重建基线，而 `pinnedReset` 下重建是**明确错误**的：会把被
 * `svn:externals` 钉死的旧内容录成一个标着新 revision 的基线，自己造出 §9.2
 * 的基线撒谎。那种状态必须先把目录更新回来，迁移排在它后面。
 */
export function describeEolMigration(
  eol: EolStatus | null,
  sync: SyncStatus | null,
): BannerModel | null {
  if (!eol || !eol.needsMigration) return null
  if (sync?.verdict.kind === 'pinnedReset') return null
  return {
    tone: 'danger',
    title: '基线是在旧的行尾设置下录的',
    detail:
      '这个仓库建库时还没有关掉 git 的行尾转换，基线里记的内容与磁盘上的字节对不上。' +
      '现在重建一次基线就能对齐，之后不会再出现这个问题。',
    actions: [{ id: 'migrateEol', label: '重建基线…' }],
    note: MIGRATE_EOL_NOTE,
  }
}

/**
 * 失同步横幅要不要让位给迁移横幅。
 *
 * 两者的出口都是重建基线，同时挂两条红条只是噪音。`pinnedReset` 不在此列
 * ——那时 `describeEolMigration` 自己返回 `null`，失同步横幅照常渲染，因为
 * 它指的是另一件**必须先做**的事。
 */
/**
 * 基线分支 `svn/base` 不存在时的恢复横幅（第 70 条）。不用挂时返回 `null`。
 *
 * 读基线失败（`svn_no_baseline`）时失同步判定无从做起，原来界面上什么都不显示，
 * 用户在应用里没有出路：别的分支又不许改名成基线。后端的「重建基线」在基线不存在时
 * 从零录一条，走的是与平时重建相同的预检和体检。
 *
 * 已经有同步结论（重建成功、或者用户在别处把基线弄回来了）就不挂：标记可能还没来得及清，
 * 但结论本身说明基线是读得到的。
 */
export function describeMissingBaseline(
  missing: boolean,
  sync: SyncStatus | null,
): BannerModel | null {
  if (!missing || sync) return null
  return {
    tone: 'danger',
    title: '基线分支 svn/base 不见了',
    detail:
      '失同步判定、体检、更新和交付都以它为参照，现在都做不了。' +
      '重建基线会按服务器上当前的版本重新录一条，并接在你当前分支历史里最近的那条基线之后。',
    note: '不会修改工作目录里的任何文件，也不会推进你的工作分支。',
    actions: [{ id: 'rebuild', label: '重建基线' }],
  }
}

export function syncBannerIsSuppressed(
  eol: EolStatus | null,
  sync: SyncStatus | null,
): boolean {
  return describeEolMigration(eol, sync) !== null
}

/**
 * 服务器侧核对结果 → 提示。
 *
 * # `notChecked` 与 `matched` 一样不挂提示，但含义**完全不同**
 *
 * 这条检查走网络、是锦上添花。断网时天天弹一条黄条，只会教会用户忽略它。
 * 但界面不打扰**不等于**可以把它当成「没问题」——`ServerAudit` 那三态在数据层
 * 始终分开，将来要在别处（比如交付前置）用它时，不能拿这里的「不显示」当依据。
 */
export function describeAudit(audit: ServerAudit | null): AuditModel | null {
  if (!audit) return null
  if (audit.status !== 'missing') return null
  // 截断时才提列了几条；不截断时重复报一遍数字是噪音。
  const more =
    audit.total > audit.files.length ? `（以下只列出前 ${audit.files.length} 个）` : ''
  return {
    tone: 'warn',
    title: '服务器上有代码文件不在本地',
    detail:
      `服务器上白名单内有 ${audit.total} 个代码文件在你的磁盘上不存在${more}，` +
      'git 没有在保护它们。最常见的成因是这些目录被 ' +
      '`svn update --set-depth=exclude` 排除掉了——那种状态下 SVN 侧的所有检查' +
      '都显示正常，只有拿服务器清单对一遍才看得见。',
    files: audit.files,
  }
}

/**
 * 后台重跑服务器侧核对的间隔（毫秒）。
 *
 * # 为什么必须自动跑，不能只给一颗按钮
 *
 * 这条检查要抓的是 `svn update --set-depth=exclude` 那类**用户不知情**的漏收。
 * 「不知情」正是要害：不知道出了事的人不会去点一颗写着「检查一下有没有出事」的
 * 按钮。把它做成手动入口，等于只在用户已经怀疑的时候才提供——而那时它多半已经
 * 没用了（2026-09-19 用户原话：它不是工作流的一部分）。
 */
export const SVN_AUDIT_POLL_MS = 5 * 60_000

/**
 * 两次自动核对之间的最小间隔（毫秒）。
 *
 * 核对要发 `svn ls -R`，**走网络**，在几万文件的工作副本上是秒级。定时器本身
 * 已经稀疏，但窗口获得焦点也是一个触发点（用户刚从 TortoiseSVN 切回来，正是
 * 最可能出事的时刻），来回切窗口就会把它打成高频。这道闸把最坏情况钉在
 * 每分钟一次。
 */
export const SVN_AUDIT_MIN_GAP_MS = 60_000

/**
 * 这一次自动触发该不该真的去核对。
 *
 * 判据下沉到这里而不是写在 effect 里：effect 里的分支在 vitest 中测不到
 * （`AGENTS.md` §7.2），而这段逻辑恰恰是「会不会把服务器打爆」的那一段。
 *
 * ⚠️ `lastAt` 是**上一次核对开始**的时刻，由所有调用点共用——包括打开仓库和
 * 重建基线之后那两次显式核对。它们不受这道闸限制，但会把闸的计时重置：刚开完
 * 仓库就立刻再核对一遍是纯粹的浪费。
 */
export function auditIsDue(now: number, lastAt: number, inFlight: boolean): boolean {
  // 上一次还没回来就不再发。慢仓库上间隔可能短于一次核对的耗时，
  // 不挡的话请求会一层层叠起来。
  if (inFlight) return false
  return now - lastAt >= SVN_AUDIT_MIN_GAP_MS
}

/**
 * §6.6.3 第一档比对用的「上次观测值」。
 *
 * 用可变盒子而不是返回值：它在 `repoStore` 里的真身是一个 `useRef`，而这里要
 * 表达的恰恰是**什么时候允许写它**——那条规则写成纯函数才测得到（§7.2）。
 */
export interface RootRevMemo {
  value: string | null
}

/**
 * 跑一次第二档，**成功之后**才把这次观测值记进 memo。
 *
 * # 顺序就是这条函数存在的全部理由
 *
 * 2026-09-21 独立 review。原来的写法是「先把 `lastRootRevs` 更新成新值，再
 * `await` 完整校验」，而挂在 `repo-changed` 上的调用点又是
 * `.catch(ignore)`——完整校验一旦抛错（外部 `svn update` 中途工作副本被锁住
 * 就够了），观测值已经推进到新 revision，下一次事件看到的是**同一个**
 * revision，第一档直接返回，于是失同步状态**永远停在旧值**，直到 revision
 * 再变一次或用户重开仓库。
 *
 * 反过来「失败就不记」最坏只是下一次事件再跑一遍完整校验——那是本地读，
 * 而代价的另一头是横幅长期撒谎。
 *
 * `observed` 必须是**开跑之前**读到的值：校验期间工作副本要是又前进了，
 * 记下这个偏旧的值会让下一次事件判不等、再校验一遍。多跑一次远好过漏一次。
 */
export async function recordAfterSuccess(
  memo: RootRevMemo,
  observed: string,
  runFullCheck: () => Promise<void>,
): Promise<void> {
  await runFullCheck()
  memo.value = observed
}

/**
 * 第一档（O(1)）：rootRevs 与上次观测值比，变了才跑第二档。
 *
 * 返回是否真的跑了第二档。第二档抛错时本函数**一并抛出**，memo 不动——
 * 调用点可以继续吞掉这个错误（横幅是留给 git 操作的），但吞掉之后下一次
 * 事件仍然会重试。
 */
export async function syncIfRootRevChanged(
  memo: RootRevMemo,
  readRootRevs: () => Promise<string>,
  runFullCheck: () => Promise<void>,
): Promise<boolean> {
  const now = await readRootRevs()
  if (now === memo.value) return false
  await recordAfterSuccess(memo, now, runFullCheck)
  return true
}

/** 焦点检测的「上次观测值」。`null` = 还没成功校验过。放 ref 的理由同 {@link RootRevMemo}。 */
export interface WcDbStampMemo {
  value: WcDbStamp[] | null
}

/** 这次读到的戳与上次比。工作副本多了、少了、换了都算变（外部增删 external，日志 Task 9）。 */
export function wcDbStampsChanged(prev: WcDbStamp[] | null, next: WcDbStamp[]): boolean {
  if (!prev || prev.length !== next.length) return true
  const seen = new Map(prev.map((s) => [s.wcRoot, s]))
  return next.some((s) => {
    const old = seen.get(s.wcRoot)
    return !old || old.mtimeNs !== s.mtimeNs || old.size !== s.size
  })
}

/**
 * 窗口拿到焦点时的检测（分析 §6.6）：wc.db 变了才跑第二档校验。
 *
 * 来回切窗口很频繁，而第二档要跑 `svn status`，这里只有几次 `stat`。`svn status` 不写 wc.db
 * （E19），所以校验本身不会让下一次判成变了。
 *
 * 记录的规矩同 {@link recordAfterSuccess}：记开跑之前读到的值，校验成功之后才记。
 */
export async function syncIfWcDbChanged(
  memo: WcDbStampMemo,
  readStamps: () => Promise<WcDbStamp[]>,
  runFullCheck: () => Promise<void>,
): Promise<boolean> {
  const stamps = await readStamps()
  if (!wcDbStampsChanged(memo.value, stamps)) return false
  await runFullCheck()
  memo.value = stamps
  return true
}

/**
 * 窗口失焦时要不要记快照（日志 Task 0，W26）。
 *
 * 快照的价值在于「它一定早于接下来的外部提交」。所以只在确认过 InSync 时写，而且校验正在跑时
 * 不写：焦点刚回来、结论还没出，这时写下的快照可能晚于外部提交，却会被当成早于（Review 2 遗留）。
 * 不写只是用上一份更旧的快照或者走「拿不准」，方向是保守的。
 */
export function shouldMarkOnBlur(
  verdict: SyncVerdict['kind'] | null,
  checksRunning: number,
): boolean {
  return verdict === 'inSync' && checksRunning === 0
}

/**
 * 失焦时记快照。多看一眼 wc.db：TortoiseSVN 的进度框还在走时切回 jkk，提交在 jkk 前台时完成，
 * 焦点事件已经过去了，判定还是旧的 InSync，这时写下的快照同样晚于外部提交。
 *
 * 没有戳可比（打开仓库之后还没有过焦点检测）时按判定写，这是唯一会放过这种情形的窗口。
 *
 * @returns 是否写了
 */
export async function markOnBlur(
  verdict: SyncVerdict['kind'] | null,
  checksRunning: number,
  memo: WcDbStampMemo,
  readStamps: () => Promise<WcDbStamp[]>,
  mark: () => Promise<void>,
): Promise<boolean> {
  if (!shouldMarkOnBlur(verdict, checksRunning)) return false
  if (memo.value && wcDbStampsChanged(memo.value, await readStamps())) return false
  await mark()
  return true
}

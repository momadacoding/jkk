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
  | { kind: 'mixedRevision'; revs: number[] }
  | { kind: 'diverged'; nodeRev: number; baselineRev: number }
  | { kind: 'pinnedReset'; nodeRev: number; baselineRev: number; external: string }

export interface SyncStatus {
  baselineRev: number
  nodeRevs: number[]
  verdict: SyncVerdict
}

/** 后端 `svn_server_set::ServerAudit`，判别字段是 `status`。 */
export type ServerAudit =
  | { status: 'matched' }
  | { status: 'missing'; files: string[]; total: number }
  | { status: 'notChecked'; reason: string }

export type BannerTone = 'warn' | 'danger'

export interface BannerAction {
  id:
    | 'rebuild'
    | 'check'
    | 'updateWhitelistDir'
    | 'migrateEol'
    | 'syncContinue'
    | 'syncAbort'
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
      return null

    case 'mixedRevision':
      return {
        tone: 'danger',
        title: '工作副本处于多个 revision',
        detail:
          `白名单内的文件分别停在 r${v.revs.join('、r')} 上。` +
          '这通常是外部提交或只更新了一部分目录造成的——' +
          `基线记的是 r${status.baselineRev}，已经不能代表磁盘上的内容。`,
        actions: [REBUILD, CHECK],
        note: REBUILD_NOTE,
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

    case 'diverged': {
      // 计划 A16：前进是每天早上在 TortoiseSVN 里对项目根 update 的日常结果，
      // 给出口、降成黄色；回退照旧是红色，「更新」对它会拒绝
      if (v.nodeRev > v.baselineRev) {
        return {
          tone: 'warn',
          title: '工作副本已在外部更新',
          detail:
            `工作副本已经前进到 r${v.nodeRev}，基线还记着 r${v.baselineRev}——` +
            '在 TortoiseSVN 里对项目根点过 update 就会这样。' +
            '此刻「本地改动」里服务器的改动和你自己的改动混在一起；' +
            '点「更新」会先把基线跟上，再把两者重新分开。',
          actions: [{ id: 'svnUpdate', label: '更新…' }, REBUILD, CHECK],
          note: REBUILD_NOTE,
        }
      }
      return {
        tone: 'danger',
        title: '工作副本被回退到了基线之前',
        detail:
          `工作副本被回退到了 r${v.nodeRev}，而基线记着 r${v.baselineRev}。` +
          '基线比磁盘还新，任何基于它的比较都不成立。',
        actions: [REBUILD, CHECK],
        note: REBUILD_NOTE,
      }
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
): boolean {
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

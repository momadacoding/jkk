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
  id: 'rebuild' | 'check' | 'updateWhitelistDir'
  label: string
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
      const forward = v.nodeRev > v.baselineRev
      return {
        tone: 'danger',
        title: forward ? '基线落后于工作副本' : '工作副本被回退到了基线之前',
        detail: forward
          ? `工作副本已经前进到 r${v.nodeRev}，而基线还记着 r${v.baselineRev}。` +
            '「相对上次更新改了什么」现在算出来的是错的。'
          : `工作副本被回退到了 r${v.nodeRev}，而基线记着 r${v.baselineRev}。` +
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
export function needsStandaloneCheck(isSvn: boolean, sync: SyncStatus | null): boolean {
  return isSvn && describeSync(sync) === null
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

/**
 * 体检基线的两个入口：按症状主动提示（B）与工具栏菜单里的手动体检（C）。
 * 设计见 `docs/superpowers/specs/2026-11-25-svn-baseline-mismatch-design.md`。
 *
 * # 为什么不再有常驻入口
 *
 * 原来同步正常时顶部一直挂着「体检基线」，等用户怀疑时去点。用户的反馈：正常使用就不该怀疑，
 * 用户怎么知道什么时候该怀疑？问题不在措辞，在于把「何时该体检」交给了用户——这和后台服务器
 * 核对当初要做成自动的理由一样（`svnSync.ts` 的 `SVN_AUDIT_POLL_MS`）。现在由 jkk 自己发现
 * 症状：「本地改动」里有一个文件，TortoiseSVN 却说它没改过（后端 `svn_mismatch.rs`）。
 *
 * 纯逻辑，无 React 依赖（`AGENTS.md` §7.2）。
 */

import { otherBannersAreSuppressed, type SyncState, type SyncStateUnreadable } from './svnJournal'
import {
  describeEolMigration,
  type BannerModel,
  type EolStatus,
  type SyncStatus,
} from './svnSync'

/** 后端 `svn_mismatch::BaselineMismatch`。 */
export interface BaselineMismatch {
  /** 命中的文件（仓库相对、正斜杠、升序），后端最多给 50 个。 */
  files: string[]
  /** 命中的总数（截断前）。 */
  total: number
  /** 候选太多、只查了一部分：`total` 是「至少」。 */
  truncated: boolean
}

/**
 * 这一刻能不能按症状判定（设计 §6.1 第 1 条）。不能时既不查后端，也不挂横幅。
 *
 * # 为什么要这么严
 *
 * 判据是「git 说改了、SVN 说没改、分支提交没碰过」，下面每一种状态都会让正常的文件满足它：
 *
 * - **还没读到失同步结论**（`sync === null`）：打开仓库前 TortoiseSVN 可能已经 update 过，
 *   新下来的文件正是这个形状。等结论出来再说。
 * - **SVN 侧有新内容**（`externallyAdvanced`）：同上，而且那时顶部已经有「跟进」横幅。
 * - **分支落后基线**（`headBehindBase`）：「本地改动」是相对旧基线算的，基线带进来的文件都会
 *   出现在里面；重建基线之后也是这个状态，由「当前分支落后基线 → 跟进」接手。
 * - **回退 / 钉死 / 工作副本被锁**：顶部已经有对应的横幅。
 * - **行尾迁移没做或还没读到**：旧行尾设置下录的基线，与磁盘的字节逐个对不上，这正是迁移
 *   横幅要解决的事。`eol === null` 也不判：读到之前不知道是不是这种仓库。
 * - **同步进行中或没收尾**：基线停在中间态，其余横幅一律让位（`otherBannersAreSuppressed`）。
 */
export function mismatchCheckApplies(
  isSvn: boolean,
  sync: SyncStatus | null,
  eol: EolStatus | null,
  syncState: SyncState | SyncStateUnreadable | null,
): boolean {
  if (!isSvn || !sync || !eol) return false
  if (otherBannersAreSuppressed(syncState)) return false
  if (describeEolMigration(eol, sync) !== null) return false
  return sync.verdict.kind === 'inSync' && !sync.headBehindBase
}

/**
 * 一组命中文件的键。体检报「基线没问题」之后记下它，同一组文件不再提示（设计 §6.2）。
 *
 * 不这样的话，行尾 / 关键字展开这类体检查不出差异的不一致会让横幅一直挂着，用户每点一次体检
 * 都得到「没问题」，横幅却不走。
 */
export function mismatchKey(files: string[]): string {
  return files.join('\n')
}

/**
 * 按症状提示的横幅。没有命中、或这组文件已经体检过没问题时返回 `null`。
 *
 * 第一句是要用户做的事（设计 §4 目标第 2 条）：原来常驻那一行读起来像告警，用户的第一反应是
 * 「这是干啥？我需要做什么吗？」。
 */
export function describeBaselineMismatch(
  m: BaselineMismatch | null,
  ackKey: string | null,
): BannerModel | null {
  if (!m || m.files.length === 0) return null
  if (ackKey !== null && ackKey === mismatchKey(m.files)) return null
  const count = m.truncated ? `至少 ${m.total}` : `${m.total}`
  const more = m.total > m.files.length ? `（下面只列出前 ${m.files.length} 个）` : ''
  return {
    tone: 'warn',
    title: `点「体检」核对 ${count} 个文件：jkk 记的 SVN 内容可能不对`,
    detail:
      `TortoiseSVN 显示这些文件没有修改，你的分支提交也没碰过它们，但 jkk 的「本地改动」里有${more}。` +
      '体检会拿基线和 SVN 服务器逐个核对；确认基线记错了，可以就地重建基线。',
    actions: [{ id: 'check', label: '体检' }],
    files: m.files,
  }
}

/**
 * 工具栏「SVN ▾」里的「体检基线」该不该置灰；返回悬停说明，`null` 表示可点。
 *
 * 时机与原来常驻入口隐藏的时机一致，另加「SVN 侧有新内容」：那时体检用的还是旧基线，必然把
 * 新内容报成「基线与服务器不符」，而跟进本身就包含体检（`describeSync` 对它只给「跟进」）。
 * 回退、钉死这些失同步状态不拦：那几条横幅自己也给体检。
 */
export function manualCheckBlockedReason(
  sync: SyncStatus | null,
  eol: EolStatus | null,
  baselineMissing: boolean,
  syncState: SyncState | SyncStateUnreadable | null,
): string | null {
  if (otherBannersAreSuppressed(syncState)) {
    return '同步还没结束或没有收尾，先处理顶部的横幅'
  }
  // 只看「丢了且还没有结论」，与恢复横幅同一判据（`describeMissingBaseline`）
  if (baselineMissing && !sync) return '基线分支 svn/base 不见了，先用顶部横幅重建基线'
  if (describeEolMigration(eol, sync) !== null) {
    return '基线是在旧的行尾设置下录的，先按顶部横幅重建，体检的结论才可信'
  }
  if (sync?.verdict.kind === 'externallyAdvanced') {
    return 'SVN 侧有新内容，先点顶部横幅的「跟进」（跟进本身就会体检）'
  }
  return null
}

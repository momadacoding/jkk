/**
 * 同步状态 → 横幅（spec §7.6 末尾「打开仓库时发现它存在 → 界面顶部常驻
 * 『上次同步未完成』」）。
 *
 * 纯逻辑，无 React 依赖（`AGENTS.md` §7.2）。
 */
import type { BannerModel } from './svnSync'

/** 后端 `svn_lock::LockHolder`。**只用于显示**，不是存活判据。 */
export interface LockHolder {
  pid: number
  startedAt: number
  opid: string
}

/**
 * 后端 `svn_journal::Phase` 的线序名，逐字对应 spec §7.6 的两张表。
 *
 * 这里只列 `sync_in` 会停在的那些。`sync_out` 的阶段（`svn-staged`、
 * `committing`…）要等 Phase 3；本期的后端也写不出它们。
 */
export type SyncPhase =
  | 'start'
  | 'stashing'
  | 'stashed'
  | 'rebase'
  | 'capturing-baseline'
  | 'publishing-baseline'
  | 'baseline-ready'

export interface UnfinishedSync {
  flow: 'sync_in' | 'sync_out'
  phase: SyncPhase
  opid: string
  startedAt: number
}

export interface SyncState {
  locked: boolean
  holder: LockHolder | null
  unfinished: UnfinishedSync | null
}

/**
 * 每个 phase 回答同一个问题：**我的东西现在在哪。**
 *
 * 通用文案等于没说。用户在这一刻看到的是一个工作区可能空了的仓库，
 * 他要知道的不是「出错了」，是「改动还在不在」。
 */
const PHASE_DETAIL: Record<SyncPhase, string> = {
  start: '流程刚开始，还没有动过工作目录里的任何东西——可以直接当作没发生过。',
  stashing:
    '正要把未提交的改动压进 stash，但压没压成还不确定。' +
    '**不要**自己去 apply 任何 stash 条目：挑错条目会把别人的 WIP 覆盖到你的工作区上。',
  stashed:
    '未提交的改动已经压进 stash，它的 OID 已经记下来了，东西没丢。' +
    '工作目录看起来是空的，那是正常的。',
  'capturing-baseline': '正在录新基线。旧基线还在原处，磁盘上的文件一个都没动。',
  'publishing-baseline': '新基线可能已经发布，但工作分支还没恢复到原来那条。',
  'baseline-ready': '新基线已经就位，只差切回你原来的分支。',
  rebase:
    '停在 git 自己的 rebase 冲突态上。冲突文件可以在「本地改动」里看到，' +
    '这部分状态由 git 自己记着，没有丢失风险。',
}

const NOTE_NEXT_PHASE =
  '本期只能告诉你停在哪里；「继续」与「放弃」两个入口随更新流程一起到来。' +
  '在那之前请不要手工去动 stash 或分支指针——记录还在，等入口到了就能接着走。'

/**
 * `null`（还没读到）与「干净」都不挂横幅。
 *
 * 打开仓库那一瞬间状态是 `null`，那时挂任何东西都等于每次打开都闪一下。
 */
export function describeUnfinishedSync(s: SyncState | null): BannerModel | null {
  if (!s) return null

  // 「正在进行」与「未完成」的出口完全相反：前者要等，后者要用户决定。
  // 说反了会引导用户去「放弃」一次正在正常跑的同步。
  if (s.locked) {
    const who = s.holder
      ? `持有者是进程 ${s.holder.pid}。`
      : '还读不到持有它的进程信息（它可能刚刚开始）。'
    return {
      tone: 'warn',
      title: '这个工作副本正在进行一次同步',
      detail:
        `${who}同步期间暂存、提交、切换分支等写操作会被拒绝，` +
        '查看状态和 diff 不受影响。',
      actions: [],
      note: '如果你确定没有任何同步在跑，那是上次异常退出留下的痕迹，重开应用即可。',
    }
  }

  if (!s.unfinished) return null

  return {
    tone: 'danger',
    title: '上次同步没有正常结束',
    detail: PHASE_DETAIL[s.unfinished.phase],
    actions: [],
    note: NOTE_NEXT_PHASE,
  }
}

/**
 * 未收尾 / 同步进行中时，其余横幅一律让位。
 *
 * 那两种状态下仓库停在中间态：失同步横幅会拿一个半推进的基线去比 revision，
 * 行尾横幅会对着一个还没录完的基线报「需要迁移」。结论都可能是错的，而它们
 * 的出口（重建基线）在这种状态下是**危险**的——重建会移动基线指针，而此刻
 * 正有另一套流程（或它的残骸）依赖着那个指针。
 */
export function otherBannersAreSuppressed(s: SyncState | null): boolean {
  return describeUnfinishedSync(s) !== null
}

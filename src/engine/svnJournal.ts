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
 * 这里只列 `sync_in` 会停在的那些。**但后端的 `Phase` 枚举已经包含 `sync_out`
 * 的全部阶段**（`svn-staged`、`committing`…），一份手工写的 journal 或 Phase 3
 * 的流程都能让它们出现在这里——所以类型收窄不等于运行时收窄，
 * `describeUnfinishedSync` 必须对认不出的 phase 有兜底（2026-09-25 独立 review）。
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
 * `svn_sync_state` 读失败（典型是 journal 坏了，`svn_journal_corrupt`）。
 *
 * 必须与 `null`（还没读到）分开：后端把「坏了」与「没有」严格分开（B3），
 * 前端若把失败写回 `null`，两者会在界面上重新合成「一切正常」。
 */
export interface SyncStateUnreadable {
  readError: string
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
    '不要自己去 apply 任何 stash 条目：挑错条目会把别人的 WIP 覆盖到你的工作区上。',
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
export function describeUnfinishedSync(
  s: SyncState | SyncStateUnreadable | null,
): BannerModel | null {
  if (!s) return null

  // 判不出来与守卫同一方向：保守地当成有问题。仓库可能正停在中间态，
  // 此刻说「一切正常」并放出重建基线之类的出口，比多挂一条红条危险得多。
  if ('readError' in s) {
    return {
      tone: 'danger',
      title: '读不出上次同步的记录',
      // 后端的报错原文已经是完整句子（`svn_journal_corrupt` 带着路径和
      // 「请勿删除」），原样接在后面，不再套一层括号。
      detail: `判断不了上次同步有没有正常结束。${s.readError}`,
      actions: [],
      note: '在弄清楚之前，请不要手工去动 stash 或分支指针，也不要删掉那份记录——里面可能记着 stash 的 OID。',
    }
  }

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

  // 认不出的 phase 必须有话说。后端的 `Phase` 枚举比这里的联合类型宽
  // （含 `sync_out` 的全部阶段），而 `PHASE_DETAIL[未知]` 是 `undefined`
  // ——那会渲染出一条只有标题、没有正文的红条，比说「我不认识」更糟。
  const detail =
    PHASE_DETAIL[s.unfinished.phase] ??
    `流程停在「${s.unfinished.phase}」这一步，这个版本的界面还不认识它` +
      '（多半是更新之后留下的记录）。请不要手工去动 stash 或分支指针，' +
      '记录都还在。'

  return {
    tone: 'danger',
    title: '上次同步没有正常结束',
    detail,
    actions: [],
    note: NOTE_NEXT_PHASE,
  }
}

/**
 * 锁被持有期间的轮询间隔。每一轮是一次独占打开 + 读一个小文件。
 *
 * 为什么要轮询：锁的**释放**不一定产生任何文件事件——持锁进程崩溃、或只是关掉
 * 句柄时，锁文件原样留在磁盘上，watcher 什么都收不到（spec §5.1.2：「文件在」
 * 不等于「有人持有」）。
 */
export const SYNC_LOCK_POLL_MS = 2000

/**
 * 要不要轮询同步状态：**只在锁被持有时**。
 *
 * 「开始同步」不需要轮询：锁文件的出现与 journal 的写入都会让 watcher 发
 * `repo-changed`。需要靠问的只有「结束」这一侧。
 *
 * 读不出来（`readError`）时不轮询：坏的是 journal，轮询不会让它自己变好；
 * 修好文件本身就会发 `repo-changed`。
 */
export function needsLockPolling(s: SyncState | SyncStateUnreadable | null): boolean {
  return s !== null && !('readError' in s) && s.locked
}

/**
 * 未收尾 / 同步进行中时，其余横幅一律让位。
 *
 * 那两种状态下仓库停在中间态：失同步横幅会拿一个半推进的基线去比 revision，
 * 行尾横幅会对着一个还没录完的基线报「需要迁移」。结论都可能是错的，而它们
 * 的出口（重建基线）在这种状态下是**危险**的——重建会移动基线指针，而此刻
 * 正有另一套流程（或它的残骸）依赖着那个指针。
 */
export function otherBannersAreSuppressed(
  s: SyncState | SyncStateUnreadable | null,
): boolean {
  return describeUnfinishedSync(s) !== null
}

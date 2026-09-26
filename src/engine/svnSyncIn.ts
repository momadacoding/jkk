/**
 * 更新（sync_in）的纯展示逻辑：预览 → 确认框、结局 → 提示、继续 / 放弃 → 确认框。
 *
 * 纯函数，无 React / Tauri 依赖（`AGENTS.md` §7.2）。组件把返回值原样展开进
 * `ConfirmDialog` 的请求，所以每句文案只在这里写一份。
 *
 * ⚠️ 横幅与对话框都按**纯文本**渲染：不要写 `**`（2b 第二轮 review R4）。
 */
import type { BannerAction } from './svnSync'

/** 后端 `svn_sync_check::SyncInPreview`。 */
export interface SyncInPreview {
  origBranch: string
  origHead: string
  baselineOid: string
  baselineRev: number
  targetRev: number
  /** 计划 A16：工作副本已被外部整体 update 到的 revision；`null` 表示没有。 */
  catchUpRev: number | null
  outsideChanges: string[]
}

/** 后端 `svn_sync_in::SyncInExpect`。 */
export interface SyncInExpect {
  origHead: string
  baselineOid: string
  confirmedOutside: string[]
}

/** 后端 `svn_sync_in::SyncInOutcome`。 */
export type SyncInOutcome =
  | { kind: 'done'; newRev: number }
  | { kind: 'aborted' }
  | { kind: 'rebaseConflict'; files: string[] }
  | { kind: 'stashConflict'; files: string[]; stashOid: string }
  | { kind: 'stashBlocked'; collisions: string[]; stashOid: string }
  /**
   * 在放回改动那一步放弃：改动没有（完整）放回，备份留在 stash 里。
   * `rolledBack` 为真时停下的是一次回退，分支在更新前的位置；否则分支已在新基线上。
   */
  | { kind: 'released'; stashOid: string; rolledBack: boolean }

/** 与 `ConfirmRequest` 同名的那几个字段，组件直接展开。 */
export interface SyncConfirmModel {
  title: string
  message: string
  warning?: string
  confirmLabel: string
  busyLabel: string
  danger?: boolean
}

export interface OutcomeNotice {
  title: string
  detail: string
}

export type SyncActionId = 'syncContinue' | 'syncAbort'

/** 预览里的事实原样带回去做锁内复核（spec §7.2 第 7 条）。 */
export function expectFromPreview(p: SyncInPreview): SyncInExpect {
  return {
    origHead: p.origHead,
    baselineOid: p.baselineOid,
    confirmedOutside: [...p.outsideChanges],
  }
}

const MAX_LISTED = 20

function listFiles(files: string[]): string {
  const shown = files.slice(0, MAX_LISTED).map((f) => `· ${f}`)
  if (files.length > MAX_LISTED) shown.push(`…还有 ${files.length - MAX_LISTED} 个`)
  return shown.join('\n')
}

function shortOid(oid: string): string {
  return oid.slice(0, 10)
}

/** 计划 A8：S10 用不带 `--index` 的 stash apply。 */
export const STAGING_NOTE = '「已暂存」的状态不会保留：改动内容都会放回来，只是需要重新暂存。'

export function describeSyncInPreview(p: SyncInPreview): SyncConfirmModel {
  // 计划 A16：外部整体 update 过时，先跟上的那个 revision 才是这次更新的起点
  const from = p.catchUpRev ?? p.baselineRev
  const catchUp =
    p.catchUpRev === null
      ? ''
      : `工作副本已经在外部（TortoiseSVN）更新到了 r${p.catchUpRev}，基线还停在 r${p.baselineRev}，` +
        '所以「本地改动」里服务器的改动和你自己的改动现在混在一起。' +
        `这次会先按服务器 r${p.catchUpRev} 的原版把基线跟上（不碰你的文件），` +
        '再把你的提交和未提交改动整理到新基线上，两者会重新分开。\n\n'
  const update =
    p.targetRev <= from
      ? p.catchUpRev === null
        ? `服务器上没有比 r${from} 更新的提交。仍会走一遍流程，让基线记账与工作副本保持一致。`
        : `服务器上没有比 r${from} 更新的提交，这次只做上面的整理。`
      : `把 r${from} 更新到 r${p.targetRev}：未提交的改动先收起来，更新完之后把分支 ${p.origBranch} 重放到新基线上，再把改动放回来。`
  const message = catchUp + update
  const parts: string[] = []
  if (p.outsideChanges.length > 0) {
    parts.push(
      '下面这些文件 git 管不到、你本地改过、这次更新又会碰到它们。' +
        '出现冲突时只能回 TortoiseSVN 处理，本应用没有它们的备份：\n' +
        listFiles(p.outsideChanges),
    )
  }
  parts.push(STAGING_NOTE)
  return {
    title: '更新',
    message,
    warning: parts.join('\n\n'),
    confirmLabel: '更新',
    busyLabel: '正在更新…',
    danger: p.outsideChanges.length > 0,
  }
}

export function describeSyncOutcome(o: SyncInOutcome): OutcomeNotice | null {
  switch (o.kind) {
    case 'done':
      return null // 成功不打扰：界面刷新本身就是结果
    case 'aborted':
      return { title: '已放弃这次更新', detail: '已回到更新前的分支，未提交的改动已放回。' }
    case 'rebaseConflict':
      return {
        title: '更新遇到冲突',
        detail:
          `基线已经更新好了，把你的提交重放上去时有冲突：\n${listFiles(o.files)}\n\n` +
          '在「本地改动」里改好这些文件并暂存，然后点顶部横幅上的「继续」。想回到更新前，点「放弃」。',
      }
    case 'stashConflict':
      return {
        title: '放回未提交的改动时有冲突',
        detail:
          `更新已经完成，但你未提交的改动与新代码冲突了：\n${listFiles(o.files)}\n\n` +
          `改好并暂存之后点顶部横幅上的「继续」。改动的原始备份一直在 stash 里（${shortOid(o.stashOid)}）。`,
      }
    case 'stashBlocked':
      return {
        title: '未提交的改动还没放回来',
        detail:
          `服务器这次新增了与你的未跟踪文件同名的文件，直接放回会覆盖其中一边：\n${listFiles(o.collisions)}\n\n` +
          `把这些文件挪到别处（或改名）之后点顶部横幅上的「继续」。一个字节都还没放回，备份在 stash ${shortOid(o.stashOid)}。`,
      }
    case 'released':
      // 不能复用 aborted 那句「已放回」：这里恰恰没放回（2c 独立 review I1）
      // 回退途中停下的放回不能说「在新基线上」：基线没推进，分支回到了原处（PR #49 review #3）
      return {
        title: o.rolledBack ? '已放弃这次更新，改动没有自动放回' : '更新已结束，改动没有自动放回',
        detail:
          (o.rolledBack ? '分支已回到更新前的位置。' : '分支已经在新基线上。') +
          '未提交的改动没有（完整）放回来——如果刚才有冲突，' +
          '文件里可能还留着冲突标记，请自己处理。' +
          `原始备份一直在 stash 列表里（mygitclient-sync:…，${shortOid(o.stashOid)}），需要时自己取回，不要当成垃圾清掉。`,
      }
  }
}

const CONTINUE: BannerAction = { id: 'syncContinue', label: '继续' }
const ABORT: BannerAction = { id: 'syncAbort', label: '放弃，回到更新前' }

/** 横幅上的按钮。认不出的 phase（`sync_out` 的，Phase 3 才有）不给按钮。 */
export function syncActions(phase: string): BannerAction[] {
  switch (phase) {
    case 'start':
      return [{ id: 'syncAbort', label: '清除这条记录' }]
    case 'rebase':
      return [{ id: 'syncContinue', label: '我已解决，继续' }, ABORT]
    case 'applying-stash':
      return [{ id: 'syncContinue', label: '我已处理，继续' }, { id: 'syncAbort', label: '不再自动放回' }]
    case 'stashing':
    case 'stashed':
    case 'capturing-baseline':
    case 'publishing-baseline':
    case 'baseline-ready':
      return [CONTINUE, ABORT]
    default:
      return []
  }
}

const AFTER_UPDATE_ABORT =
  '回到更新前的分支，放回未提交的改动。SVN 那一侧已经更新的部分不会退回——' +
  '如果基线还没跟上，顶部会提示失同步，用「重建基线」即可。'

export function describeSyncAction(id: SyncActionId, phase: string): SyncConfirmModel {
  if (id === 'syncContinue') {
    const message =
      phase === 'rebase'
        ? '确认冲突文件都已改好并暂存。会接着把剩下的提交重放完，再放回未提交的改动。'
        : phase === 'applying-stash'
          ? '确认冲突已解决（或同名文件已挪走）。会结束这次更新并清理 stash 里的备份。'
          : '从中断的地方接着走完这次更新。'
    return { title: '继续更新', message, confirmLabel: '继续', busyLabel: '正在继续…' }
  }
  const message =
    phase === 'start'
      ? '上次更新在动手之前就中断了，清除这条记录即可，不会改动任何文件。'
      : phase === 'rebase'
        ? '中止重放，分支回到更新前的位置，放回未提交的改动。基线已经是新的，你的分支会落后于它，下次更新时再跟上。'
        : phase === 'applying-stash'
          ? '结束这次更新，不再自动放回改动。stash 里的备份（mygitclient-sync:…）会留在 stash 列表里，你可以之后自己取回。'
          : AFTER_UPDATE_ABORT
  return {
    title: '放弃这次更新',
    message,
    confirmLabel: '放弃',
    busyLabel: '正在放弃…',
    danger: phase !== 'start',
  }
}

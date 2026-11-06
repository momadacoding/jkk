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
  /**
   * 计划 A16：要先把基线跟上时，跟上之后的 floor；`null` 表示不需要。整体 update 时就是那个
   * revision；计划 Task 5 起只提交不更新、局部更新也接管，floor 可能等于 `baselineRev`。
   */
  catchUpRev: number | null
  outsideChanges: string[]
}

/** 后端 `svn_sync_in::SyncInExpect`。 */
export interface SyncInExpect {
  /** 确认框里看到的分支；锁内复核要比它（PR #49 review 第三轮 #6）。 */
  origBranch: string
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
  /**
   * 放回时停下过（冲突等），用户处理完点了「继续」：流程收尾，但原始备份**留在 stash 里**
   * ——磁盘上的改动经过了人手，证明不了还完整（PR #49 review 第四轮 P1-2）。
   */
  | { kind: 'settled'; stashOid: string; rolledBack: boolean }
  /**
   * 收尾了，改动内容都放回来了，但「已暂存」的状态没保住：手动跟进带 `--index` 放不回去，或者
   * 更新时有暂存版本与磁盘不同（Review 3 第 1 条）。`newRev` 为 null 时收尾的是一次放弃。
   *
   * 备份不删：`files` 的暂存版本只在 `stashOid` 那条 stash 里。`stashOid` 为 null 只出现在崩溃恢复、
   * stash 已被外部删掉时。
   */
  | { kind: 'stageLost'; newRev: number | null; stashOid: string | null; files: string[] }
  /**
   * 手动跟进收尾了，但收尾核对发现重放结果与跟进前不一致（事故分析 6.9）：`changed` 是跟进前后
   * 磁盘内容不同的文件，`resurrected` 是分支里有、新基线和磁盘上都没有的文件。只有手动跟进会报。
   *
   * `before` 是代表跟进前磁盘的提交（stash 或原分支 HEAD）。`stashOid` 为 null：这次没有未提交的
   * 改动，没有备份可留。`stageLost` 非 null：暂存状态也没保住，含义同 `stageLost.files`。
   * `auditError` 非 null：核对没跑成（这时两个清单为空），流程照常收尾、备份留着。
   */
  | {
      kind: 'followDrift'
      newRev: number
      changed: string[]
      resurrected: string[]
      before: string
      stashOid: string | null
      stageLost: string[] | null
      auditError: string | null
    }

/** 与 `ConfirmRequest` 同名的那几个字段，组件直接展开。 */
export interface SyncConfirmModel {
  title: string
  message: string
  warning?: string
  confirmLabel: string
  busyLabel: string
  danger?: boolean
}

/** 提示框的分量：`attention` 是还要用户动手（解决冲突、自己取回改动、补一次更新），`ok` 只是告知。 */
export type NoticeTone = 'ok' | 'attention'

export interface OutcomeNotice {
  title: string
  detail: string
  tone: NoticeTone
}

export type SyncActionId = 'syncContinue' | 'syncAbort'

/** 预览里的事实原样带回去做锁内复核（spec §7.2 第 7 条）。 */
export function expectFromPreview(p: SyncInPreview): SyncInExpect {
  return {
    origBranch: p.origBranch,
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

/**
 * 「已暂存」没保住时，暂存的那一份在哪、怎么取回（Review 3 第 1 条）。更新与手动跟进共用。
 *
 * 命令里给完整 OID：用户要照抄。`git restore --staged` 只写暂存区，不碰磁盘上现在的版本。
 */
export function stageLostDetail(o: Extract<SyncInOutcome, { kind: 'stageLost' }>): string {
  const head = '未提交的改动内容都放回来了，但「已暂存」的状态没保住，现在都显示为未暂存。'
  const oid = o.stashOid
  if (!oid) return `${head}需要的话重新暂存一次。`
  const where = `stash 列表里的 mygitclient-sync:…（${oid}）`
  if (o.files.length === 0) {
    return (
      `${head}暂存的内容磁盘上都有。为防万一，原始备份没有删，还在${where}，` +
      '确认没问题之后可以自己删掉。'
    )
  }
  return (
    `${head}下面这些文件暂存的版本与磁盘上的不同：\n${listFiles(o.files)}\n\n` +
    `暂存的那一份没有丢，原样留在${where}，不要当成垃圾清掉。取回办法：\n` +
    `· 查看：git show ${oid}^2:<文件>\n` +
    `· 放回暂存区（不动磁盘）：git restore --source=${oid}^2 --staged -- <文件>\n` +
    `也可以在侧栏的 STASHES 里找到这一条。另有 refs/mygitclient/sync/ 下的一个引用钉着它，确认不再需要时一并删掉。`
  )
}

/**
 * 手动跟进收尾核对有发现时的说明（事故分析 6.9）。命令里给完整 OID：用户要照抄。
 *
 * 不能说成「完成」了事：2026-11-05 磁盘上的文件被静默合错，界面报完成，正确内容唯一的副本
 * （stash 备份）也随之被删。
 */
export function describeFollowDrift(o: Extract<SyncInOutcome, { kind: 'followDrift' }>): OutcomeNotice {
  const parts: string[] = [
    o.auditError
      ? '分支已经在新基线上，但收尾核对没能跑完，没法确认磁盘上的内容都对。'
      : '分支已经在新基线上，但收尾核对发现有文件不对，请看一下。',
  ]
  if (o.auditError) {
    parts.push(`核对失败的原因：${o.auditError}`)
  }
  if (o.changed.length > 0) {
    // `git restore --worktree` 只写磁盘、不动暂存区；`git checkout <提交> -- <文件>` 会顺手把它暂存。
    // 引号不能省：cmd.exe 里 `^` 是转义符，不加引号 `<oid>^3` 会被吃成 `<oid>3`
    const untrackedNote = o.stashOid
      ? '\n跟进前还没被 git 跟踪的文件，把命令里的提交换成它的第三个父提交：\n' +
        `git restore --source="${o.before}^3" --worktree -- <文件>`
      : ''
    parts.push(
      `下面这些文件跟进后的内容和跟进前不一样：\n${listFiles(o.changed)}\n\n` +
        '如果是你刚才解决冲突时有意改的，可以忽略；否则多半是重放时被合错了（git 没有报冲突）。' +
        '跟进前的版本可以这样查看、取回（在仓库根目录执行）：\n' +
        `· 查看：git show "${o.before}:<文件>"\n` +
        `· 写回磁盘（不动暂存区）：git restore --source="${o.before}" --worktree -- <文件>` +
        untrackedNote +
        '\n跟进前根本不存在的文件（跟进后多出来的）没有旧版本可取，不需要的话直接删掉。',
    )
  }
  if (o.resurrected.length > 0) {
    parts.push(
      `下面这些文件还在重放后的分支里，但 SVN 新基线里已经没有了：\n${listFiles(o.resurrected)}\n\n` +
        '多半是 SVN 上已经删除或改名、又被旧的快照提交带了回来。确认不需要的话：\n' +
        '· 磁盘上没有的：「本地改动」里显示为删除，提交这条删除即可\n' +
        '· 磁盘上还在的：用 git rm --cached -- <文件> 只从 git 里删、不动磁盘上那份，再提交',
    )
  }
  if (o.stashOid) {
    parts.push(
      `跟进前的未提交改动没有删，原始备份还在 stash 列表里（mygitclient-sync:…，${o.stashOid}）。` +
        '它可能是正确内容唯一的副本，核对完之后再自己删掉。',
    )
  }
  if (o.stageLost) {
    parts.push(stageLostDetail({ kind: 'stageLost', newRev: o.newRev, stashOid: o.stashOid, files: o.stageLost }))
  }
  return {
    tone: 'attention',
    title: o.auditError ? '手动跟进完成，但收尾核对没跑成' : '手动跟进完成，但有文件和跟进前不一样',
    detail: parts.join('\n\n'),
  }
}

/** 计划 A8：S10 用不带 `--index` 的 stash apply。 */
export const STAGING_NOTE =
  '「已暂存」的状态不会保留：改动内容都会放回来，只是需要重新暂存。' +
  '暂存的版本与磁盘不同的文件，暂存的那一份会留在 stash 里，结束时会告诉你。'

export function describeSyncInPreview(p: SyncInPreview): SyncConfirmModel {
  // 计划 A16：外部整体 update 过时，先跟上的那个 revision 才是这次更新的起点
  const from = p.catchUpRev ?? p.baselineRev
  const catchUp =
    p.catchUpRev === null
      ? ''
      : p.catchUpRev > p.baselineRev
        ? `工作副本已经在外部（TortoiseSVN）更新到了 r${p.catchUpRev}，基线还停在 r${p.baselineRev}，` +
          '所以「本地改动」里服务器的改动和你自己的改动现在混在一起。' +
          `这次会先按服务器 r${p.catchUpRev} 的原版把基线跟上（不碰你的文件），` +
          '再把你的提交和未提交改动整理到新基线上，两者会重新分开。\n\n'
        : // 计划 Task 5：只提交不更新、或只更新了部分目录。floor 没动，但有文件已经在更新的
          // revision 上——说「更新到了 r1」是错的
          `工作副本里有文件在外部（TortoiseSVN）提交过或单独更新过，基线还停在 r${p.baselineRev}，` +
          '所以「本地改动」里已经进了 SVN 的内容和你还没提交的改动现在混在一起。' +
          '这次会先按这些文件在服务器上的原版把基线跟上（不碰你的文件），' +
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
      return { title: '已放弃这次更新', detail: '已回到更新前的分支，未提交的改动已放回。', tone: 'ok' }
    case 'rebaseConflict':
      return {
        tone: 'attention',
        title: '更新遇到冲突',
        detail:
          `基线已经更新好了，把你的提交重放上去时有冲突：\n${listFiles(o.files)}\n\n` +
          '在「本地改动」里改好这些文件并暂存，然后点顶部横幅上的「继续」。想回到更新前，点「放弃」。',
      }
    case 'stashConflict':
      return {
        tone: 'attention',
        title: '放回未提交的改动时有冲突',
        detail:
          `更新已经完成，但你未提交的改动与新代码冲突了：\n${listFiles(o.files)}\n\n` +
          `改好并暂存之后点顶部横幅上的「继续」。改动的原始备份一直在 stash 里（${shortOid(o.stashOid)}）。`,
      }
    case 'stashBlocked':
      return {
        tone: 'attention',
        title: '未提交的改动还没放回来',
        detail:
          `服务器这次新增了与你的未跟踪文件同名的文件，直接放回会覆盖其中一边：\n${listFiles(o.collisions)}\n\n` +
          `把这些文件挪到别处（或改名）之后点顶部横幅上的「继续」。一个字节都还没放回，备份在 stash ${shortOid(o.stashOid)}。`,
      }
    case 'released':
      // 不能复用 aborted 那句「已放回」：这里恰恰没放回（2c 独立 review I1）
      // 回退途中停下的放回不能说「在新基线上」：基线没推进，分支回到了原处（PR #49 review #3）
      return {
        tone: 'attention',
        title: o.rolledBack ? '已放弃这次更新，改动没有自动放回' : '更新已结束，改动没有自动放回',
        detail:
          (o.rolledBack ? '分支已回到更新前的位置。' : '分支已经在新基线上。') +
          '未提交的改动没有（完整）放回来——如果刚才有冲突，' +
          '文件里可能还留着冲突标记，请自己处理。' +
          `原始备份一直在 stash 列表里（mygitclient-sync:…，${shortOid(o.stashOid)}），需要时自己取回，不要当成垃圾清掉。`,
      }
    case 'settled':
      // stash 里的备份留着只是为防万一，删不删随用户，不算待办
      return {
        tone: 'ok',
        title: o.rolledBack ? '已放弃这次更新' : '更新完成',
        detail:
          (o.rolledBack
            ? '分支已回到更新前的位置，未提交的改动按你处理后的样子留在工作区里。'
            : '未提交的改动按你处理后的样子留在工作区里。') +
          `为防万一，改动的原始备份没有删，还在 stash 列表里（mygitclient-sync:…，${shortOid(o.stashOid)}）。` +
          '确认工作区里的内容没问题之后，可以自己把它删掉。',
      }
    case 'stageLost':
      // 更新不带 `--index`：有暂存版本与磁盘不同时备份留在 stash 里（Review 3 第 1 条）
      return {
        tone: 'attention',
        title: o.newRev === null ? '已放弃这次更新，暂存状态没保住' : '更新完成，暂存状态没保住',
        detail: (o.newRev === null ? '分支已回到更新前的位置。' : '') + stageLostDetail(o),
      }
    case 'followDrift':
      // 只有手动跟进会报；走到这里也照实说，不吞掉
      return describeFollowDrift(o)
  }
}

const CONTINUE: BannerAction = { id: 'syncContinue', label: '继续' }

/**
 * 横幅上的按钮。认不出的 phase（比如出现在更新记录里的交付阶段名）不给按钮。
 * 手动跟进沿用这组按钮，`abortLabel` 换掉那句「回到更新前」。
 */
export function syncActions(phase: string, abortLabel = '放弃，回到更新前'): BannerAction[] {
  const abort: BannerAction = { id: 'syncAbort', label: abortLabel }
  switch (phase) {
    case 'start':
      return [{ id: 'syncAbort', label: '清除这条记录' }]
    case 'rebase':
      return [{ id: 'syncContinue', label: '我已解决，继续' }, abort]
    case 'applying-stash':
      return [{ id: 'syncContinue', label: '我已处理，继续' }, { id: 'syncAbort', label: '不再自动放回' }]
    case 'stashing':
    case 'stashed':
    case 'capturing-baseline':
    case 'publishing-baseline':
    case 'baseline-ready':
      return [CONTINUE, abort]
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
          ? '确认冲突已解决（或同名文件已挪走）。会结束这次更新；如果刚才放回时有冲突，原始备份会留在 stash 列表里，由你确认后自己删。'
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

/**
 * 交付（sync_out）的纯展示逻辑：预览 → 交付对话框模型、结局 → 提示、横幅 → 按钮与确认框。
 *
 * 纯函数，无 React / Tauri 依赖（`AGENTS.md` §7.2）。每句文案只在这里写一份。
 *
 * ⚠️ 横幅与对话框都按纯文本渲染：不要写 `**`。
 */
import type { BannerAction } from './svnSync'
import type { OutcomeNotice, SyncConfirmModel } from './svnSyncIn'

/** 后端 `svn_sync_out_check::DeliverChange`。 */
export interface DeliverChange {
  status: 'A' | 'M' | 'D'
  path: string
}

/** 后端 `svn_deliver::LocalOnlyCommit`。 */
export interface LocalOnlyCommit {
  oid: string
  subject: string
}

/** 后端 `svn_sync_out_check::SyncOutPreview`。 */
export interface SyncOutPreview {
  origBranch: string
  origHead: string
  baselineOid: string
  baselineRev: number
  remoteRev: number
  deliverOid: string
  deliverTree: string
  changes: DeliverChange[]
  localOnly: LocalOnlyCommit[]
  outsideChanges: string[]
}

/** 后端 `svn_sync_out::SyncOutExpect`。字段名逐字对齐（`AGENTS.md` §7.3）。 */
export interface SyncOutExpect {
  origBranch: string
  origHead: string
  baselineOid: string
  deliverTree: string
  changes: DeliverChange[]
  confirmedOutside: string[]
  message: string
}

/** 后端 `svn_sync_out::SyncOutOutcome`。 */
export type SyncOutOutcome =
  | { kind: 'delivered'; rev: number; branch: string; localOnlyLeft: number }
  | { kind: 'rebaseConflict'; rev: number; files: string[] }
  | { kind: 'abandoned'; committed: boolean }

/** 后端 `svn_journal::Phase` 里交付流程的那几个（线序名，spec §7.6）。 */
export type SyncOutPhase =
  | 'prepare'
  | 'building-deliver'
  | 'deliver-built'
  | 'checked-out'
  | 'svn-staged'
  | 'committing'
  | 'post-commit'
  | 'publishing-baseline'
  | 'restoring-branch'

export type SyncOutActionId = 'syncOutCleanup' | 'syncOutContinue' | 'syncOutAbort'

export interface DeliverGroup {
  label: string
  files: string[]
}

export interface DeliverDialogModel {
  title: string
  summary: string
  groups: DeliverGroup[]
  /** 被摘掉的本地补丁，`<oid 前 10 位> <subject>`。 */
  localOnly: string[]
  /** 列表为空时也有话说（spec §8.2.3「漏标是主要人为风险」）。 */
  localOnlyNote: string
  /** 白名单里 git 管不到、本地改过、交付后会被更新碰到的文件。非空时必须勾选确认。 */
  outsideFiles: string[]
  outsideWarning?: string
  confirmLabel: string
  busyLabel: string
}

/** 预览里的事实原样带回去做锁内复核（计划 B1）。提交信息原样，trim 在后端做。 */
export function expectFromDeliverPreview(p: SyncOutPreview, message: string): SyncOutExpect {
  return {
    origBranch: p.origBranch,
    origHead: p.origHead,
    baselineOid: p.baselineOid,
    deliverTree: p.deliverTree,
    changes: [...p.changes],
    confirmedOutside: [...p.outsideChanges],
    message,
  }
}

const MAX_LISTED = 20

function listFiles(files: string[]): string {
  const shown = files.slice(0, MAX_LISTED).map((f) => `· ${f}`)
  if (files.length > MAX_LISTED) shown.push(`…还有 ${files.length - MAX_LISTED} 个`)
  return shown.join('\n')
}

export function describeDeliverPreview(p: SyncOutPreview): DeliverDialogModel {
  const pick = (s: DeliverChange['status']) => p.changes.filter((c) => c.status === s).map((c) => c.path)
  const groups: DeliverGroup[] = [
    { label: '新增', files: pick('A') },
    { label: '修改', files: pick('M') },
    { label: '删除', files: pick('D') },
  ]
    .filter((g) => g.files.length > 0)
    .map((g) => ({ label: `${g.label} ${g.files.length}`, files: g.files }))
  const n = p.localOnly.length
  return {
    title: '交付到 SVN',
    summary:
      `把分支 ${p.origBranch} 相对基线（r${p.baselineRev}）的 ${p.changes.length} 个文件变化提交到 SVN。` +
      '提交之后不能撤回，会直接影响全组。',
    groups,
    localOnly: p.localOnly.map((c) => `${c.oid.slice(0, 10)} ${c.subject}`),
    localOnlyNote:
      n === 0
        ? '本次没有本地补丁被排除。如果你有打算「仅本地」的改动却没在这里看到，说明它没有标记，会被一起交付。'
        : `下面 ${n} 个本地补丁（标了「仅本地，不交付」）不会被提交，交付之后它们仍留在你的分支上：`,
    outsideFiles: [...p.outsideChanges],
    outsideWarning:
      p.outsideChanges.length === 0
        ? undefined
        : '下面这些文件在白名单目录里，但 git 管不到（被扩展名排除），你本地改过。交付之后会把白名单目录' +
          '更新到最新，它们可能被合并；冲突时只能回 TortoiseSVN 处理，本应用没有它们的备份。' +
          '勾选只表示同意更新，不会还原它们：\n' +
          listFiles(p.outsideChanges),
    confirmLabel: '交付',
    busyLabel: '正在交付…',
  }
}

/** 交付按钮可不可以点。 */
export function canDeliver(message: string, outsideAcknowledged: boolean, m: DeliverDialogModel): boolean {
  return message.trim().length > 0 && (m.outsideFiles.length === 0 || outsideAcknowledged)
}

export function describeSyncOutOutcome(o: SyncOutOutcome): OutcomeNotice | null {
  switch (o.kind) {
    case 'delivered':
      // D9（spec §8.2.3）：分支上还有本地补丁时，它们是唯一副本，绝不能提示删除
      return {
        tone: 'ok',
        title: `已交付 r${o.rev}`,
        detail:
          o.localOnlyLeft > 0
            ? `分支 ${o.branch} 上还有 ${o.localOnlyLeft} 个本地补丁，没有随交付删除。它们只存在于这个分支上，请保留这个分支。`
            : `分支 ${o.branch} 的改动都已交付，可以删除这个分支了（不会自动删）。`,
      }
    case 'rebaseConflict':
      return {
        tone: 'attention',
        title: '交付成功，本地补丁落回时有冲突',
        detail:
          `改动已经在 SVN 上了（r${o.rev}），不会丢。只剩你的本地补丁放回到最新版本之上时有冲突：\n${listFiles(o.files)}\n\n` +
          '在「本地改动」里改好这些文件并暂存，然后点顶部横幅上的「我已解决，继续」。想先不管补丁，点「停止收尾」。',
      }
    case 'abandoned':
      return o.committed
        ? {
            // 还欠一次「更新」，否则下次交付会被拒
            tone: 'attention',
            title: '已停止收尾',
            detail:
              '改动已经在 SVN 上了，没有撤回。分支回到了交付前的状态；点「更新」把它更新到最新版本，否则下次交付会被拒。',
          }
        : {
            tone: 'ok',
            title: '已清理',
            detail: '这次交付在提交之前就停下了。已回到原分支，SVN 侧没有任何改动。',
          }
  }
}

/** 横幅正文需要的那部分未收尾记录。 */
export interface SyncOutUnfinished {
  phase: string
  opid: string
  expectedStartRev?: number
  commitMessage?: string
  /** 已经落地的 revision（D5 成功后才有）。 */
  committedRev?: number
  /** 发起交付的分支。 */
  origBranch?: string
}

/** 交付未收尾横幅的三样文字。 */
export interface SyncOutBanner {
  /** 回答「交上去没有」。 */
  title: string
  /** 回答「安全吗、还差什么」。 */
  detail: string
  /** 每个按钮点了会怎样。没有按钮时说该怎么办。 */
  note: string
}

const NOT_YET = '交付被打断了，还没提交'
/** 交付途中 HEAD 会临时停在交付树或 `svn/base` 上，工具栏显示的就是它（2026-10-14 E2E 场景 8）。 */
const PARKED_BEFORE = '现在暂时停在交付用的中间位置（工具栏会显示 HEAD），不要在这里改代码。'
const PARKED_AFTER =
  '现在暂时停在交付途中的中间位置（工具栏可能显示 svn/base 或 HEAD），不要在这里改代码。'

/**
 * 交付未收尾时横幅上的文字。认不出的 phase 返回 `undefined`。
 *
 * 2026-10-14 E2E 用户反馈「好难理解」：原先只有一段正文，用的是实现层的说法（「新基线可能已经
 * 发布」「分支还没接上去」），另配一条所有 phase 共用的说明（提交之后停下时也讲「提交之前停下的…」，
 * 还把按钮叫成「继续」）。现在每个 phase 只说自己的事，不提「基线」「发布」，note 里点名的按钮
 * 就是 `syncOutActions` 给的那几个（测试按完整按钮名断言）。
 *
 * 点名发起交付的分支：中途 HEAD 停在 `svn/base` 上时，用户以为工作分支变了（场景 8 实测）。
 * 提交之后还差的事说成用户的话——「切回它、把它更新到最新版本」——不说「接到新基线上」：
 * 用户的反问是「e2e8 不是已经提交到 SVN 了么？接上去是干啥？」。旧记录没有分支名时退回「你的」。
 */
export function describeSyncOutBanner(u: SyncOutUnfinished): SyncOutBanner | undefined {
  const done = u.committedRev === undefined ? '交付已提交' : `交付已提交（r${u.committedRev}）`
  const b = u.origBranch ?? '你的分支'
  const safe = `${u.origBranch ? `${u.origBranch} 的改动` : '你的改动'}已经在 SVN 上了，不会丢，也不会重复提交。`
  const update = '把它更新到最新版本（有本地补丁的话会放回去）'
  const stop = `「停止收尾」直接切回 ${b} 交付前的状态，之后点「更新」把它更新到最新版本，否则下次交付会被拒。`
  switch (u.phase) {
    case 'prepare':
    case 'building-deliver':
    case 'deliver-built':
      return {
        title: NOT_YET,
        detail: 'SVN 上什么都没发生，你的文件也没动过，只剩一个临时记录。',
        note: '「清理」删掉临时记录，之后可以重新交付。',
      }
    case 'checked-out':
      return {
        title: NOT_YET,
        detail:
          'SVN 上什么都没发生。为了交付，磁盘暂时换成了要提交的版本，本地补丁暂时不在磁盘上。' +
          PARKED_BEFORE,
        note: `「清理」切回 ${b}，本地补丁会回到磁盘上，之后可以重新交付。`,
      }
    case 'svn-staged':
      return {
        title: NOT_YET,
        detail:
          'SVN 上还没有提交，但已经登记了几个新增 / 删除。为了交付，磁盘暂时换成了要提交的版本，' +
          '本地补丁暂时不在磁盘上。' +
          PARKED_BEFORE,
        note: `「清理」撤掉这些登记、切回 ${b}，本地补丁会回到磁盘上，之后可以重新交付。`,
      }
    case 'committing': {
      // 提交若落地，第一个可能的位置就是 start+1，所以说「从它起」而不是「它之后」
      const from = u.expectedStartRev === undefined ? '' : `从 r${u.expectedStartRev + 1} 起`
      return {
        title: '交付状态未知',
        detail:
          '提交已经发出，但没有收到 SVN 的答复，不知道成没成功。为免重复提交，这里什么都不动。' +
          `请在 TortoiseSVN 的 Show log 里${from}找提交信息为「${u.commitMessage ?? ''}」的 revision` +
          `（它带着属性 mygitclient:opid=${u.opid}）。`,
        note: '找到了说明已经交上去，没找到说明没交上去。本版本还不能替你收尾，在那之前请不要再交付同样的改动。',
      }
    }
    case 'post-commit':
      return {
        title: `${done}，还差收尾`,
        detail: `${safe}还差收尾：把白名单目录更新到最新，再切回 ${b}，并${update}。${PARKED_AFTER}`,
        note:
          '「继续交付」完成收尾（需要网络）。' +
          stop +
          '（停止前也要先把白名单目录更新到最新，同样需要网络。）',
      }
    case 'publishing-baseline':
      return {
        title: `${done}，还差收尾`,
        detail: `${safe}还差收尾：切回 ${b}，并${update}。${PARKED_AFTER}`,
        note: `「继续交付」完成收尾（推荐）。${stop}`,
      }
    case 'restoring-branch':
      return {
        title: `${done}，本地补丁还没放回`,
        detail:
          `${safe}还差收尾：把本地补丁放回到最新版本之上（${b} 随之更新到最新版本）。` +
          `如果提示冲突，先在「本地改动」里改好冲突的文件并暂存。${PARKED_AFTER}`,
        note:
          '「我已解决，继续」放完补丁、完成收尾。' +
          `「停止收尾」不放了，直接切回 ${b} 交付前的状态，本地补丁不会丢；` +
          '之后点「更新」把它更新到最新版本，否则下次交付会被拒。',
      }
    default:
      return undefined
  }
}

/**
 * 按钮名要与后端报错、结局说明里引用的一致：会叫用户去点「清理」「继续交付」「我已解决，继续」
 * 「停止收尾」（`svn_sync_out.rs` 的 `early_exit` / `update_whitelist` / `unresolved`，以及
 * 上面的 `describeSyncOutOutcome`）。改名时两边一起改，测试按完整按钮名断言。
 */
const CLEANUP: BannerAction = { id: 'syncOutCleanup', label: '清理' }
const STOP: BannerAction = { id: 'syncOutAbort', label: '停止收尾' }

/** 横幅上的按钮（计划 B4 / B8 的表）。 */
export function syncOutActions(phase: string): BannerAction[] {
  switch (phase) {
    case 'prepare':
    case 'building-deliver':
    case 'deliver-built':
    case 'checked-out':
    case 'svn-staged':
      return [CLEANUP]
    case 'post-commit':
    case 'publishing-baseline':
      return [{ id: 'syncOutContinue', label: '继续交付' }, STOP]
    case 'restoring-branch':
      return [{ id: 'syncOutContinue', label: '我已解决，继续' }, STOP]
    default:
      return [] // committing（3b 才能查证）与认不出的 phase
  }
}

export function describeSyncOutAction(id: SyncOutActionId, phase: string): SyncConfirmModel {
  if (id === 'syncOutCleanup') {
    return {
      title: '清理这次交付',
      message:
        phase === 'svn-staged'
          ? '逐条撤销 SVN 里的新增 / 删除登记，切回你的分支，删掉临时的交付标记。服务器上没有任何东西。'
          : '切回你的分支，删掉临时的交付标记。服务器上没有任何东西。',
      confirmLabel: '清理',
      busyLabel: '正在清理…',
    }
  }
  // 与横幅同一套说法（`describeSyncOutBanner`）：不提「基线」「接上去」「mixed-revision」
  if (id === 'syncOutContinue') {
    return {
      title: '继续交付',
      message:
        phase === 'restoring-branch'
          ? '确认冲突的文件都已改好并暂存。会接着把本地补丁放回到最新版本之上，你的分支随之更新到最新版本，不会重复提交。'
          : '改动已经在 SVN 上了。会接着完成收尾：' +
            (phase === 'post-commit' ? '先把白名单目录更新到最新，再' : '') +
            '切回你的分支，并把它更新到最新版本，不会重复提交。',
      confirmLabel: '继续',
      busyLabel: '正在继续…',
    }
  }
  return {
    title: '停止收尾',
    message:
      '改动已经在 SVN 上了，不会撤回。只是不替你做收尾：直接切回你的分支交付前的状态' +
      (phase === 'restoring-branch' ? '（本地补丁不放了，不会丢）' : '') +
      '。之后点「更新」把它更新到最新版本，否则下次交付会被拒。' +
      // 后端在 post-commit 放弃时也要先 D6（2026-10-11 里程碑 review 第 4 条）：只提交不更新的
      // 工作副本是 mixed-revision，「更新」接不了
      (phase === 'post-commit' ? '停止之前也要先把白名单目录更新到最新，所以需要网络。' : ''),
    confirmLabel: '停止',
    busyLabel: '正在停止…',
    danger: true,
  }
}

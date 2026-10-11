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
        title: `已交付 r${o.rev}`,
        detail:
          o.localOnlyLeft > 0
            ? `分支 ${o.branch} 上还有 ${o.localOnlyLeft} 个本地补丁，没有随交付删除。它们只存在于这个分支上，请保留这个分支。`
            : `分支 ${o.branch} 的改动都已交付，可以删除这个分支了（不会自动删）。`,
      }
    case 'rebaseConflict':
      return {
        title: '交付成功，本地补丁落回时有冲突',
        detail:
          `提交已在服务器上（r${o.rev}），基线也已推进。只剩你的本地补丁没接到新基线上：\n${listFiles(o.files)}\n\n` +
          '在「本地改动」里改好这些文件并暂存，然后点顶部横幅上的「继续」。想先不管补丁，点「停止收尾」。',
      }
    case 'abandoned':
      return o.committed
        ? {
            title: '已停止收尾',
            detail:
              '提交已在服务器上，没有撤回。分支回到了交付前的位置；顶部会提示基线需要更新，点「更新」即可把分支接到服务器的最新内容上。',
          }
        : {
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
}

/** 每个 phase 回答同一个问题：我的提交现在在哪、我的文件现在在哪。认不出时返回 `undefined`。 */
export function describeSyncOutPhase(u: SyncOutUnfinished): string | undefined {
  switch (u.phase) {
    case 'prepare':
    case 'building-deliver':
    case 'deliver-built':
      return '交付在提交之前就停下了。磁盘上的文件和 SVN 都没有动过，只剩一个临时的交付标记要清掉。'
    case 'checked-out':
      return '交付在提交之前停下了。工作副本切到了交付用的版本（本地补丁暂时不在磁盘上），SVN 没有动过。清理会切回你的分支。'
    case 'svn-staged':
      return '交付在提交之前停下了。SVN 里登记了一些新增 / 删除，还没提交。清理会逐条撤销这些登记，再切回你的分支。'
    case 'committing': {
      const from = u.expectedStartRev === undefined ? '' : `r${u.expectedStartRev + 1} 之后`
      return (
        '提交已经发出，但没有拿到服务器的答复，可能成功也可能没有。本版本还不能自动查证，所以什么都没再动。' +
        `请在 TortoiseSVN 的 Show log 里看${from}有没有提交信息为「${u.commitMessage ?? ''}」的 revision` +
        `（它带着属性 mygitclient:opid=${u.opid}）。`
      )
    }
    case 'post-commit':
      return '提交已经成功，服务器上有了。还没把白名单更新到最新、也没推进基线。'
    case 'publishing-baseline':
      return '提交已经成功，新基线可能已经发布，你的分支还没接上去。'
    case 'restoring-branch':
      return '提交已经成功，基线已推进，正在把你的分支（和本地补丁）接到新基线上。如果提示冲突，改好并暂存后点继续。'
    default:
      return undefined
  }
}

/**
 * 按钮名要与后端报错里引用的一致：后端会叫用户去点「清理」「继续」「继续交付」
 * 「停止收尾」（`svn_sync_out.rs` 的 `early_exit` / `update_whitelist`）。
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
  if (id === 'syncOutContinue') {
    return {
      title: '继续交付',
      message:
        phase === 'restoring-branch'
          ? '确认冲突文件都已改好并暂存。会接着把本地补丁接到新基线上，不会重复提交。'
          : '提交已经在服务器上。会接着更新白名单、推进基线、把分支接上去，不会重复提交。',
      confirmLabel: '继续',
      busyLabel: '正在继续…',
    }
  }
  return {
    title: '停止收尾',
    message:
      '提交已经在服务器上，不会撤回。只是不再替你做后面的事：分支回到交付前的位置' +
      (phase === 'restoring-branch' ? '（中止补丁的重放）' : '') +
      '。之后顶部会提示基线需要更新，点「更新」即可。' +
      // 后端在 post-commit 放弃时也要先 D6（2026-10-11 里程碑 review 第 4 条）
      (phase === 'post-commit'
        ? '停止之前要先把白名单更新到最新（否则工作副本停在 mixed-revision），所以需要网络。'
        : ''),
    confirmLabel: '停止',
    busyLabel: '正在停止…',
    danger: true,
  }
}

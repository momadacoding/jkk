import { useEffect, useMemo, useRef, useState } from 'react'
import type { PullRequest, RefPointer } from '../engine/types'
import { indexPullRequestsByBranch, prBadge, pullRequestForRef } from '../engine/prs'
import { ContextMenu, useContextMenu } from './ContextMenu'
import { SidebarSection, SidebarGroup } from './SidebarSection'
import { usePrompt } from './PromptDialog'
import { DeleteBranchDialog, type DeleteTargets } from './DeleteBranchDialog'
import { useConfirm } from './ConfirmDialog'
import { isProtectedBaseline, type Capabilities } from '../engine/capabilities'
import { goneUpstreamName, goneUpstreamTooltip } from '../engine/refPills'
import { mergedPullRequestsFor } from '../engine/deleteBranchNote'

interface BranchViewProps {
  refs: RefPointer[]
  /**
   * 当前仓库的 open PR，用来在本地分支旁打编号徽章。
   *
   * 读不到（没装 gh、没登录、不是 GitHub 仓库）时是空数组，界面上就是没有徽章。
   */
  pullRequests: PullRequest[]
  head: string
  /** 当前选中的分支（仅用于高亮与定位，不改工作区）。 */
  selectedBranch: string | null
  onSelect: (name: string, hash: string) => void
  onCheckout: (name: string) => void
  onCreate: (name: string, start?: string) => void
  onRename: (oldName: string, newName: string) => void
  /**
   * 删除本地分支，`targets` 说明要不要连远程分支和 worktree 一起删。
   *
   * 返回的 Promise 要等删完才 resolve，确认框靠它显示进度；失败要抛出，
   * 确认框据此留在原地显示错误。
   */
  onDelete: (name: string, targets: DeleteTargets) => Promise<void>
  /** 检出远程分支：建一条跟踪它的本地分支并切过去。 */
  onCheckoutRemote: (remoteBranch: string) => void
  /**
   * 删除远程分支，参数是 `origin/feat/x` 这种完整简称。
   *
   * 同 `onDelete`：Promise 要等删完才 resolve，失败要抛出。
   */
  onDeleteRemote: (remoteBranch: string) => Promise<void>
  /** 仓库路径，删除确认框要用它去查上游和未合并提交数。 */
  repoPath: string | null
  /**
   * 后端能力。`Remote` 分组与 PR 徽章是 `BRANCHES` 分区**内部**的东西，
   * 上层的 `visibleSections` 裁不掉，只能由本组件自己按
   * `remotes` / `pullRequests` 决定。
   */
  capabilities: Capabilities
}

const shortName = (ref: RefPointer) =>
  ref.name.replace('refs/heads/', '').replace('refs/remotes/', '')

function BranchIcon() {
  return (
    <svg className="sb-icon" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.3">
      <circle cx="4.5" cy="3.5" r="1.7" />
      <circle cx="4.5" cy="12.5" r="1.7" />
      <circle cx="11.5" cy="6.5" r="1.7" />
      <path d="M4.5 5.2v5.6M11.5 8.2c0 2.2-2.6 2.4-4.4 3" />
    </svg>
  )
}

/** 领先/落后微徽章。没有上游（null）时什么都不画。 */
function SyncBadge({ ahead, behind }: { ahead: number | null; behind: number | null }) {
  if (!ahead && !behind) return null
  return (
    <span className="sb-sync">
      {/* 箭头由 CSS ::before 画，好在它和数字之间留出空隙——
          紧挨着的话 ↑10 会被读成 110 */}
      {behind ? <span className="sb-sync-behind">{behind}</span> : null}
      {ahead ? <span className="sb-sync-ahead">{ahead}</span> : null}
    </span>
  )
}

/**
 * 上游已删除徽章（`git branch -vv` 的 gone）。上游还在或没配上游时什么都不画。
 *
 * 这时 ↑↓ 必然是空的——没有上游可比——所以它和 SyncBadge 不会同时出现。
 */
function GoneBadge({ upstream }: { upstream: string | null }) {
  if (!upstream) return null
  return <span className="sb-gone" title={goneUpstreamTooltip(upstream)}>gone</span>
}

/**
 * PR 编号微徽章。分支上没有可显示的 PR（undefined）时什么都不画。
 *
 * `branchHash` 是本地分支的 tip，用来区分「已全部合并」「合并后又长了提交」「本地落后于合并点」等形态。
 */
function PrBadge({ pr, branchHash }: { pr: PullRequest | undefined; branchHash: string }) {
  if (!pr) return null
  const badge = prBadge(pr, branchHash)
  return (
    <span className={`sb-pr ${badge.variant}`} title={badge.tooltip}>
      {badge.label}
    </span>
  )
}

export function BranchView({
  refs, pullRequests, head, selectedBranch, onSelect, onCheckout, onCreate, onRename, onDelete,
  onCheckoutRemote, onDeleteRemote, repoPath, capabilities,
}: BranchViewProps) {
  const [creating, setCreating] = useState(false)
  const [newName, setNewName] = useState('')
  /** 正在确认删除的分支名；null 表示没开确认框。 */
  const [deleting, setDeleting] = useState<string | null>(null)
  const inputRef = useRef<HTMLInputElement>(null)
  const { menu, open, close } = useContextMenu()
  const { prompt, dialog } = usePrompt()
  const { confirm, dialog: confirmDialog } = useConfirm()
  const local = refs.filter((r) => r.kind === 'branch')
  const remote = refs.filter((r) => r.kind === 'remote')
  // 分支多的仓库里每行都遍历一遍 PR 列表是白费力气，建一次索引就够
  const prByBranch = useMemo(() => indexPullRequestsByBranch(pullRequests), [pullRequests])
  // 不能用上面的索引：它一个分支只留一条（open 优先），而删除框要的是全部已合并的合并点
  const deletingMergedPrs = useMemo(
    () => (deleting ? mergedPullRequestsFor(pullRequests, deleting) : []),
    [pullRequests, deleting],
  )

  // 展开输入框却不聚焦的话，用户得多点一次才能打字
  useEffect(() => { if (creating) inputRef.current?.focus() }, [creating])

  const submit = () => {
    const name = newName.trim()
    if (name) onCreate(name)
    setNewName('')
    setCreating(false)
  }

  const branchMenu = (name: string) => [
    {
      // **checkout 到基线分支是允许的**：spec §6.4 选一个可见分支，核心收益
      // 就是「我相对上次更新改了什么」这个 diff 要能看。禁止的只是在其上**提交**。
      label: `Checkout ${name}`,
      onSelect: () => onCheckout(name),
      disabled: name === head,
    },
    {
      label: 'Rename…',
      onSelect: () => void (async () => {
        const next = await prompt({
          title: 'Rename branch',
          message: `Rename branch "${name}" to:`,
          defaultValue: name,
          confirmLabel: 'Rename',
        })
        if (next && next !== name) onRename(name, next)
      })(),
      // 同下面的「删除」：改了名，基线就等于不在了（后端同样拒绝）
      disabled: isProtectedBaseline(name, capabilities),
    },
    {
      label: 'New branch from here…',
      onSelect: () => void (async () => {
        const child = await prompt({
          title: 'New branch',
          message: `New branch starting at "${name}":`,
          confirmLabel: 'Create',
        })
        if (child) onCreate(child, name)
      })(),
    },
    {
      label: 'Delete…',
      onSelect: () => setDeleting(name),
      // 基线分支不给删。spec §6.4 说真删了能从 reflog 恢复、最坏重建一个基线，
      // 但让它一键可删没有任何好处——删掉之后失同步检测、体检、重建全部失去
      // 参照，而用户通常不知道自己删的是什么。
      disabled: name === head || isProtectedBaseline(name, capabilities),
      danger: true,
    },
  ]

  /**
   * 远程分支的右键菜单。
   *
   * 这里**没有** Checkout：直接 `git checkout origin/x` 会进游离 HEAD，
   * 看着像切过去了，此后的提交却不属于任何分支。要建本地跟踪分支，
   * 所以措辞是 "Checkout as local branch"。
   */
  const remoteMenu = (full: string) => [
    {
      label: 'Checkout as local branch',
      onSelect: () => onCheckoutRemote(full),
    },
    {
      label: 'Copy branch name',
      onSelect: () => { void navigator.clipboard.writeText(full) },
    },
    {
      label: 'Delete remote branch…',
      onSelect: () => void confirm({
        title: 'Delete Remote Branch',
        message: 'Delete this branch from the remote repository',
        detail: { label: 'Branch:', value: full },
        warning: '远程分支是共享的，删掉之后其他人下次 fetch --prune 也会跟着少这一条。',
        confirmLabel: 'Delete',
        danger: true,
        // 交给弹框去跑：这一步要走网络，得让等待可见
        action: () => onDeleteRemote(full),
        busyLabel: '正在联系远程…',
      }),
      danger: true,
    },
  ]

  return (
    <SidebarSection
      title="BRANCHES"
      action={{ title: 'New branch', onClick: () => setCreating(true) }}
    >
      {creating && (
        <div className="sb-inline-form">
          <input
            ref={inputRef}
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') submit()
              // Esc 收起来，否则这个输入框会一直杵在那里
              if (e.key === 'Escape') { setNewName(''); setCreating(false) }
            }}
            onBlur={() => { if (!newName.trim()) setCreating(false) }}
            placeholder="branch name"
          />
        </div>
      )}

      <SidebarGroup label="Local" count={local.length}>
        {local.map((ref) => {
          const name = shortName(ref)
          const isBaseline = isProtectedBaseline(name, capabilities)
          const classes = ['sb-row']
          if (name === head) classes.push('current')
          if (name === selectedBranch) classes.push('selected')
          if (isBaseline) classes.push('baseline')
          return (
            <div
              key={ref.name}
              className={classes.join(' ')}
              title={
                isBaseline
                  ? `${name}\n\nSVN 基线分支：它记着「上次与服务器对齐时的内容」。\n` +
                    '可以 checkout 上去看 diff，但不能在它上面开发提交。'
                  : `${name}\n单击定位 · 双击 checkout · 右键更多操作`
              }
              onClick={() => onSelect(name, ref.hash)}
              onDoubleClick={() => { if (name !== head) onCheckout(name) }}
              onContextMenu={(e) => open(e, branchMenu(name))}
            >
              <BranchIcon />
              <span className="sb-label">{name}</span>
              {isBaseline && <span className="sb-baseline-tag">基线</span>}
              {capabilities.pullRequests && (
                <PrBadge pr={pullRequestForRef(prByBranch, ref)} branchHash={ref.hash} />
              )}
              <SyncBadge ahead={ref.ahead} behind={ref.behind} />
              <GoneBadge upstream={goneUpstreamName(ref)} />
              {name === head && <span className="sb-current-dot" title="当前分支" />}
            </div>
          )
        })}
      </SidebarGroup>

      {/* 远程分支在 SVN 混合仓库里没有对应物，而 checkout_remote_branch /
          delete_remote_branch 都还没进接缝，仍直接跑 git CLI。整组不渲染 */}
      {capabilities.remotes && (
      <SidebarGroup label="Remote" count={remote.length} defaultOpen={false}>
        {remote.map((ref) => (
          <div
            key={ref.name}
            className={shortName(ref) === selectedBranch ? 'sb-row remote selected' : 'sb-row remote'}
            title={`${shortName(ref)}\n单击定位 · 右键更多操作`}
            onClick={() => onSelect(shortName(ref), ref.hash)}
            onContextMenu={(e) => open(e, remoteMenu(shortName(ref)))}
          >
            <BranchIcon />
            <span className="sb-label">{shortName(ref)}</span>
          </div>
        ))}
      </SidebarGroup>
      )}

      <ContextMenu menu={menu} onClose={close} />
      {dialog}
      {confirmDialog}
      {deleting && (
        <DeleteBranchDialog
          repoPath={repoPath}
          name={deleting}
          capabilities={capabilities}
          mergedPullRequests={deletingMergedPrs}
          onCancel={() => setDeleting(null)}
          // 成功才关；失败时 onDelete 抛出，确认框自己接住并留在原地
          onConfirm={async (targets) => { await onDelete(deleting, targets); setDeleting(null) }}
        />
      )}
    </SidebarSection>
  )
}

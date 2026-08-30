import { useCallback, useEffect, useMemo, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { Panel, Group, Separator } from 'react-resizable-panels'
import { GraphView } from './GraphView'
import { BranchView } from './BranchView'
import { TagView } from './TagView'
import { WorktreeView } from './WorktreeView'
import { CommitDetailView } from './CommitDetailView'
import { LocalChangesView } from './LocalChangesView'
import { Toolbar } from './Toolbar'
import { StashView } from './StashView'
import { useRepoStore, useRepoEffects } from '../store/repoStore'
import { samePath } from '../engine/path'
import { askConfirm } from '../engine/confirm'
import { usePrompt } from './PromptDialog'
import { useConfirm } from './ConfirmDialog'
import { save } from '@tauri-apps/plugin-dialog'
import type { CommitNode } from '../engine/types'
import {
  commitMenu,
  formatCommitInfo,
  planCommitCommand,
  shortHash,
  type CommitAction,
} from '../engine/commitActions'

type ViewMode = 'commits' | 'changes'

/** 确认框上按钮的文字，照抄菜单项的动词，点下去做什么一目了然。 */
const CONFIRM_LABELS: Partial<Record<CommitAction, string>> = {
  checkout: 'Checkout',
  'cherry-pick': 'Cherry Pick',
  revert: 'Revert',
}

function LocalChangesIcon() {
  return (
    <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.2">
      <path d="M3.5 1.5h6l3 3v10h-9z" />
      <path d="M8 7v4M6 9h4" />
    </svg>
  )
}

function AllCommitsIcon() {
  return (
    <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.2">
      <circle cx="5" cy="4" r="1.6" />
      <circle cx="5" cy="12" r="1.6" />
      <path d="M5 5.6v4.8M5 4h6" />
    </svg>
  )
}

function SidebarNav({ mode, changeCount, onChange }: { mode: ViewMode; changeCount: number; onChange: (m: ViewMode) => void }) {
  return (
    <div className="sidebar-nav">
      <div
        className={mode === 'changes' ? 'sidebar-nav-item selected' : 'sidebar-nav-item'}
        onClick={() => onChange('changes')}
      >
        <LocalChangesIcon />
        <span>Local Changes{changeCount > 0 ? ` (${changeCount})` : ''}</span>
      </div>
      <div
        className={mode === 'commits' ? 'sidebar-nav-item selected' : 'sidebar-nav-item'}
        onClick={() => onChange('commits')}
      >
        <AllCommitsIcon />
        <span>All Commits</span>
      </div>
    </div>
  )
}

interface RepoTabProps {
  /** 本标签打开的仓库/worktree 路径。 */
  repoPath: string
  /** 所有标签已打开的路径，用于标记「已打开」。 */
  openPaths: string[]
  onOpenInNewTab: (path: string) => void
  /**
   * 这个目录已经被删掉了，开着它的标签该关掉；返回是否真有标签被关掉。
   *
   * 删 worktree 影响的是**别的**标签（当前标签自己的 worktree 删不了：侧边栏
   * 禁着按钮，删分支那条路也被「不能删 HEAD 所在分支」挡住），所以只能由
   * App 来关。
   */
  onRepoRemoved: (path: string) => void
}

/**
 * 一个标签页的全部内容。
 *
 * 每个实例持有自己的 `useRepoStore`，所以标签之间的 commits / 选中 / staging
 * 互不影响；`repo-changed` 事件在 store 内按路径过滤。
 */
export function RepoTab({
  repoPath, openPaths, onOpenInNewTab, onRepoRemoved,
}: RepoTabProps) {
  const {
    state, openRepo, refreshHead, refreshGraph, refreshStatus, refreshWorktrees,
    refreshDiff, refreshUpstream, refreshStashes, refreshPullRequests,
    selectCommit, clearError, reportError,
  } = useRepoStore()
  const [repoState, setRepoState] = useState<string>('normal')
  const [viewMode, setViewMode] = useState<ViewMode>('commits')
  const [selectedBranch, setSelectedBranch] = useState<string | null>(null)
  const [selectedTag, setSelectedTag] = useState<string | null>(null)
  const [selectedWorktree, setSelectedWorktree] = useState<string | null>(null)
  const { prompt, dialog: promptDialog } = usePrompt()
  const { confirm, dialog: confirmDialog } = useConfirm()

  useRepoEffects(
    state,
    { refreshHead, refreshGraph, refreshStatus, refreshWorktrees, refreshDiff, refreshUpstream, refreshStashes },
    viewMode,
  )

  useEffect(() => {
    openRepo(repoPath)
  }, [openRepo, repoPath])

  useEffect(() => {
    if (!state.path) return
    const path = state.path
    let cancelled = false
    // 先 await unwatch 再 watch，否则两个命令的先后顺序不确定
    void (async () => {
      await invoke('unwatch_repo', { path })
      if (cancelled) return
      await invoke('watch_repo', { path })
    })()
    return () => {
      cancelled = true
      void invoke('unwatch_repo', { path })
    }
  }, [state.path])

  // merge / rebase 横幅要跟着仓库变化走，否则冲突开始或解决后横幅会一直是旧值
  useEffect(() => {
    const path = state.path
    if (!path) return
    const read = () => {
      void invoke<string>('repo_state', { path }).then(setRepoState).catch(() => setRepoState('normal'))
    }
    read()
    let unlisten: (() => void) | undefined
    void listen<string>('repo-changed', (e) => { if (samePath(e.payload, path)) read() })
      .then((fn) => { unlisten = fn })
    return () => { unlisten?.() }
  }, [state.path])

  const handleCommit = useCallback(async (msg: string, amend: boolean) => {
    if (!state.path) return
    try {
      await invoke('commit', { path: state.path, message: msg, amend })
      await refreshGraph(state.path)
      await refreshStatus(state.path)
      await refreshDiff(state.path, 'workdir')
    } catch (err) {
      reportError(err)
    }
  }, [state.path, refreshGraph, refreshStatus, refreshDiff, reportError])

  const runAndRefresh = useCallback(async (cmd: string, args: Record<string, unknown>, refresh: () => Promise<void>) => {
    if (!state.path) return
    try {
      await invoke(cmd, { path: state.path, ...args })
      await refresh()
    } catch (err) {
      reportError(err)
    }
  }, [state.path, reportError])

  /** 工具栏动作：跑完把所有受影响的状态刷一遍。 */
  const runToolbar = useCallback(async (command: string, args: Record<string, unknown>) => {
    const path = state.path
    if (!path) return
    const before = state.headHash
    try {
      await invoke(command, { path, ...args })
      // fetch/pull/push/stash 会同时影响图、状态、上游计数和 stash 列表
      const after = await refreshHead(path)
      // HEAD 动了才把选中挪过去：pull 会动，fetch/push/stash 不会，
      // 不加这个判断的话点 Fetch 也会把你正在看的提交拽走
      await refreshGraph(path, after, after !== before)
      await refreshStatus(path)
      await refreshUpstream(path)
      await refreshStashes(path)
      // 只有这三个动作会改变「分支有没有 PR」的答案：push 常常紧跟着开 PR，
      // fetch/pull 则可能带回别人刚开的。stash / cherry-pick 之类不碰远端，
      // 让它们也发一次网络请求纯属浪费。不 await——徽章晚一两秒出现无所谓，
      // 但不能让 push 的反馈被 gh 拖住。
      if (command === 'fetch' || command === 'pull' || command === 'push') {
        void refreshPullRequests(path)
      }
    } catch (err) {
      reportError(err)
    }
  }, [state.path, state.headHash, refreshHead, refreshGraph, refreshStatus, refreshUpstream, refreshStashes, refreshPullRequests, reportError])

  /**
   * graph 行右键菜单的调度。
   *
   * 菜单长什么样由 `engine/commitActions` 决定，这里只负责补齐它拿不到的东西：
   * 输入框、存盘框、确认框和真正的 invoke。会动工作区的三项（checkout /
   * cherry-pick / revert）复用 `runToolbar` 的刷新语义——它们和 pull 一样会挪 HEAD。
   */
  const handleCommitAction = useCallback(async (action: CommitAction, commit: CommitNode) => {
    const path = state.path
    if (!path) return
    const short = shortHash(commit.id)
    try {
      switch (action) {
        case 'copy-hash':
          // 复制完整 hash：短 hash 贴到别的仓库或以后可能不再唯一
          await navigator.clipboard.writeText(commit.id)
          return
        case 'copy-info':
          await navigator.clipboard.writeText(formatCommitInfo(commit))
          return
        case 'new-branch': {
          const name = await prompt({
            title: 'New Branch',
            message: `在 ${short} 上创建分支`,
            confirmLabel: 'Create',
          })
          if (!name) return
          await invoke('create_branch', { path, name, startPoint: commit.id })
          await refreshGraph(path)
          return
        }
        case 'new-tag': {
          const name = await prompt({
            title: 'New Tag',
            message: `在 ${short} 上创建标签`,
            confirmLabel: 'Create',
          })
          if (!name) return
          await invoke('create_tag', { path, name, rev: commit.id })
          await refreshGraph(path)
          return
        }
        case 'save-patch': {
          const file = await save({
            title: 'Save as Patch',
            defaultPath: `${short}.patch`,
            filters: [{ name: 'Patch', extensions: ['patch'] }],
          })
          if (!file) return
          await invoke('save_patch', { path, rev: commit.id, out: file })
          return
        }
        default: {
          const plan = planCommitCommand(action, commit)
          const entry = commitMenu(commit, { isHead: commit.id === state.headHash })
            .find((e) => e.action === action)
          if (!plan || !entry?.confirm) return
          const ok = await confirm({
            title: entry.label.replace(/…$/, ''),
            message: entry.confirm,
            confirmLabel: CONFIRM_LABELS[action] ?? 'OK',
          })
          if (!ok) return
          await runToolbar(plan.command, plan.args)
        }
      }
    } catch (err) {
      reportError(err)
    }
  }, [state.path, state.headHash, prompt, confirm, runToolbar, refreshGraph, reportError])

  /**
   * 丢弃改动。这是界面上少数不可逆的操作之一，必须先确认。
   *
   * 未跟踪文件在 git 里没有任何副本，删了就找不回来了——确认框要把它们和
   * 「能从 git 恢复」的那些分开列出来，否则用户没法判断这次点下去的代价。
   */
  const handleDiscard = useCallback(async (paths: string[], staged: boolean) => {
    const path = state.path
    if (!path || paths.length === 0) return
    // Unstaged 区里 status 为 added 的就是未跟踪文件
    const gone = staged
      ? []
      : state.status.unstaged.filter((f) => paths.includes(f.path) && f.status === 'added').map((f) => f.path)
    const recoverable = paths.filter((p) => !gone.includes(p))

    const lines = ['丢弃以下改动？']
    if (recoverable.length > 0) {
      lines.push('', `恢复到${staged ? ' HEAD' : '暂存区的内容'}（${recoverable.length}）：`, ...recoverable.map((p) => `  ${p}`))
    }
    if (gone.length > 0) {
      lines.push('', `永久删除，无法恢复（${gone.length}）：`, ...gone.map((p) => `  ${p}`))
    }
    if (!(await askConfirm(lines.join('\n'), 'Discard changes'))) return

    try {
      await invoke('discard', { path, paths, staged })
      await refreshStatus(path)
      await refreshDiff(path, 'workdir')
    } catch (err) {
      reportError(err)
    }
  }, [state.path, state.status.unstaged, refreshStatus, refreshDiff, reportError])

  /**
   * 删除分支，可选连远程和 worktree 一起删。确认在 `DeleteBranchDialog` 里做过了。
   *
   * **顺序是 worktree → 远程 → 本地分支**，按「最容易失败的放最前」排：
   *
   * - worktree 那一步最常被拒——只要里面有改动或未跟踪文件，`git worktree
   *   remove` 就不干（这也意味着它不会偷偷毁掉未提交的东西）。它排第一，
   *   失败时远程和本地都还没动过，原地重试就行。
   * - 远程那一步会因为外部原因失败（断网、没权限、分支被保护），排在删本地
   *   引用之前；反过来的话本地引用已经没了，还得先去 reflog 里捞回来才能重试。
   * - 本地分支最后。它也**必须**在 worktree 之后：分支被 worktree 检出时
   *   `git branch -D` 会直接拒绝。
   *
   * **失败不在这里吞掉，抛给确认框。** 那个框会留在原地把错误显示出来，用户
   * 就地就能重试；丢进顶部横幅的话框已经关了，还得重新右键走一遍。
   */
  const handleDeleteBranch = useCallback(async (
    name: string,
    targets: { remote: { remote: string; branch: string } | null; worktreePath: string | null },
  ) => {
    const path = state.path
    if (!path) return
    const { remote, worktreePath } = targets
    if (worktreePath) {
      // 不带 force：worktree 里有未提交的东西时要拦下来，这层保护和从侧边栏
      // 删是一致的。错误会抛给确认框显示
      await invoke('delete_worktree', { path, worktreePath, force: false })
      // 就在这一步之后关，而不是等整套删完：后面的远程/本地分支还可能失败，
      // 那时候目录已经没了，标签不关照样是个死标签
      onRepoRemoved(worktreePath)
    }
    if (remote) {
      await invoke('delete_remote_branch', { path, remote: remote.remote, branch: remote.branch })
    }
    await invoke('delete_branch', { path, name })
    await refreshGraph(path)
    await refreshUpstream(path)
    // 删掉的 worktree 还挂在侧边栏上，得把那一栏也刷掉
    if (worktreePath) await refreshWorktrees(path)
  }, [state.path, refreshGraph, refreshUpstream, refreshWorktrees, onRepoRemoved])

  /**
   * 删除一个 worktree。确认在 `DeleteWorktreeDialog` 里做过了。
   *
   * **失败不在这里吞掉，抛给确认框**：它会留在原地显示错误，并把「有未提交
   * 改动」那种失败升级成「仍要删除」。丢进顶部横幅的话框已经关了，用户还得
   * 重新走一遍才能确认。
   */
  const handleDeleteWorktree = useCallback(async (worktreePath: string, force: boolean) => {
    const path = state.path
    if (!path) return
    await invoke('delete_worktree', { path, worktreePath, force })
    // 目录已经不在了，开着它的标签必须跟着关：留着的话文件监听挂在空处、
    // 刷新一路报错，那个死路径还会被存进会话，下次启动一直判成「还原不完整」
    onRepoRemoved(worktreePath)
    await refreshWorktrees(path)
  }, [state.path, refreshWorktrees, onRepoRemoved])

  /**
   * 检出一个远程分支。后端会建一条跟踪它的本地分支——直接检出远程引用会进
   * 游离 HEAD，那不是用户点这一下想要的。
   */
  const handleCheckoutRemote = useCallback(async (remoteBranch: string) => {
    const path = state.path
    if (!path) return
    const before = state.headHash
    try {
      await invoke('checkout_remote_branch', { path, remoteBranch })
      const after = await refreshHead(path)
      // HEAD 动了就把选中跟过去，否则图还停在原来的位置
      await refreshGraph(path, after, after !== before)
      await refreshStatus(path)
      await refreshUpstream(path)
    } catch (err) {
      reportError(err)
    }
  }, [state.path, state.headHash, refreshHead, refreshGraph, refreshStatus, refreshUpstream, reportError])

  /**
   * 删除远程分支。确认在 `BranchView` 的确认框里做过了。
   *
   * 同 `handleDeleteBranch`：失败抛给确认框，它会留在原地显示错误让用户重试。
   */
  const handleDeleteRemote = useCallback(async (remoteBranch: string) => {
    const path = state.path
    if (!path) return
    await invoke('delete_remote_ref', { path, remoteBranch })
    // 远程引用没了，图上的标签和上游计数都要跟着变
    await refreshGraph(path)
    await refreshUpstream(path)
  }, [state.path, refreshGraph, refreshUpstream])

  /** 单击分支：只选中并把 graph 定位到它的 tip，不碰工作区。 */
  const handleSelectBranch = useCallback((name: string, hash: string) => {
    setSelectedBranch(name)
    setViewMode('commits') // 否则在 Local Changes 视图下点分支毫无反馈
    selectCommit(hash)
  }, [selectCommit])

  /**
   * 单击 tag：只选中并把 graph 定位到它指向的提交，不碰工作区。
   *
   * load_graph 现在会把 tag 也当 revwalk 起点，正常情况下目标提交一定在图里；
   * 但提交数超过上限被截断时仍可能落空。同 `handleSelectWorktree`：宁可明说
   * 原因，也不要留一个悬空的 selectedId 让详情面板莫名空白。
   */
  const handleSelectTag = useCallback((name: string, hash: string) => {
    setSelectedTag(name)
    setViewMode('commits')
    if (!state.commits.some((c) => c.id === hash)) {
      reportError(`tag "${name}" 指向的提交 (${hash.slice(0, 7)}) 不在当前提交图中`)
      return
    }
    selectCommit(hash)
  }, [selectCommit, state.commits, reportError])

  /**
   * 双击 tag：检出到它指向的提交。这会进入游离 HEAD，是 git 的正常语义。
   *
   * rev 用全限定的 `refs/tags/<name>`：同时存在同名分支时短名有歧义，
   * git 会优先选分支，那就检出到了完全不同的地方。
   */
  const handleCheckoutTag = useCallback((name: string) => {
    // 双击一定先触发两次 onClick，所以 handleSelectTag 可能刚报过
    // 「不在当前提交图中」——那条提示对紧接着的 checkout 不成立（checkout 走
    // git，不受图的 5000 条上限影响），留着会变成一条自相矛盾的红条。
    clearError()
    void runAndRefresh('checkout', { rev: `refs/tags/${name}` }, async () => {
      const before = state.headHash
      const after = await refreshHead(state.path!)
      await refreshGraph(state.path!, after, after !== before)
      await refreshStatus(state.path!)
      // 检出 tag 会进游离 HEAD，此时不再有上游。不刷的话工具栏会继续显示
      // 原分支的 ↑↓，看着像这个游离 HEAD 还在跟踪某个远程分支。
      await refreshUpstream(state.path!)
    })
  }, [runAndRefresh, clearError, state.headHash, state.path, refreshHead, refreshGraph, refreshStatus, refreshUpstream])

  /** 单击 worktree：把 graph 定位到它的 HEAD，同样不碰工作区。 */
  const handleSelectWorktree = useCallback((headHash: string, name: string) => {
    setSelectedWorktree(name)
    setViewMode('commits')
    // 游离 HEAD 指向的提交可能不在任何分支上，也就不在这张图里。
    // 直接选中会留下一个悬空的 selectedId——详情面板空白且毫无解释，
    // 所以先确认它在图上，不在就明说原因。
    if (!state.commits.some((c) => c.id === headHash)) {
      reportError(`worktree "${name}" 的 HEAD (${headHash.slice(0, 7)}) 不在当前提交图中`)
      return
    }
    selectCommit(headHash)
  }, [selectCommit, state.commits, reportError])

  // stash 提交不被任何分支引用，所以不在 load_graph 的输出里。选中一条 stash 时
  // 从它自己的元数据现搭一个 CommitNode，否则详情面板只会显示 "Select a commit"——
  // diff 明明已经取回来了，却因为查不到提交而整片空白。
  const detailCommit = useMemo(() => {
    const fromGraph = state.commits.find((c) => c.id === state.selectedId)
    if (fromGraph) return fromGraph
    const stash = state.stashes.find((s) => s.hash === state.selectedId)
    if (!stash) return null
    return {
      id: stash.hash,
      parents: stash.parents,
      author: stash.author,
      time: stash.time,
      message: stash.message,
    }
  }, [state.commits, state.stashes, state.selectedId])

  if (state.loading && state.commits.length === 0) return <div style={{ padding: 16 }}>Loading...</div>

  return (
    <div className="repo-tab">
      {repoState !== 'normal' && <div className="banner-state">{repoState} in progress</div>}
      {state.error && (
        <div className="banner-error">
          <span>{state.error}</span>
          <button onClick={clearError}>×</button>
        </div>
      )}
      <Toolbar
        branch={state.head}
        upstream={state.upstream}
        onRun={runToolbar}
        commandContext={{ worktree: state.path, branch: state.head, headHash: state.headHash }}
      />
      <div className="repo-path-bar" title={state.path ?? ''}>{state.path} @ {state.head}</div>
      <div style={{ flex: 1, overflow: 'hidden' }}>
        <Group orientation="horizontal">
          {/* 22%：分支名普遍带 feat/ 前缀，20% 时连 worktree 名字都存不下 */}
          <Panel defaultSize="22%" minSize="15%" maxSize="40%" style={{ display: 'flex', flexDirection: 'column' }}>
            <div style={{ flex: 1, overflow: 'auto' }}>
              <SidebarNav
                mode={viewMode}
                changeCount={state.status.unstaged.length + state.status.staged.length}
                onChange={setViewMode}
              />
              <BranchView
                refs={state.refs}
                pullRequests={state.pullRequests}
                head={state.head}
                selectedBranch={selectedBranch}
                onSelect={handleSelectBranch}
                onCheckout={(name) => runAndRefresh('checkout', { rev: name }, async () => {
                  const before = state.headHash
                  const after = await refreshHead(state.path!)
                  // 切分支后选中要跟到新分支的 tip 上，否则图停在旧位置
                  await refreshGraph(state.path!, after, after !== before)
                  await refreshStatus(state.path!)
                })}
                onCreate={(name, start) => runAndRefresh('create_branch', { name, startPoint: start ?? null }, async () => await refreshGraph(state.path!))}
                onRename={(oldName, newName) => runAndRefresh('rename_branch', { old: oldName, new: newName }, async () => {
                  await refreshHead(state.path!)
                  await refreshGraph(state.path!)
                })}
                onDelete={handleDeleteBranch}
                onCheckoutRemote={(name) => void handleCheckoutRemote(name)}
                onDeleteRemote={handleDeleteRemote}
                repoPath={state.path}
              />
              <TagView
                tags={state.refs.filter((r) => r.kind === 'tag')}
                commits={state.commits}
                selectedTag={selectedTag}
                onSelect={handleSelectTag}
                onCheckout={handleCheckoutTag}
                onPush={(name) => void runToolbar('push_tag', { name })}
              />
              <WorktreeView
                worktrees={state.worktrees}
                currentPath={state.path ?? ''}
                openPaths={openPaths}
                onOpenInNewTab={onOpenInNewTab}
                onSelect={handleSelectWorktree}
                selectedPath={
                  state.worktrees.find((w) => w.name === selectedWorktree)?.path ?? null
                }
                onCreate={(name) => runAndRefresh('create_worktree', { name }, async () => await refreshWorktrees(state.path!))}
                onDelete={handleDeleteWorktree}
              />
              <StashView
                stashes={state.stashes}
                selectedId={state.selectedId}
                onSelect={(hash) => { setViewMode('commits'); selectCommit(hash) }}
                onApply={(index) => runAndRefresh('stash_apply', { index }, async () => {
                  await refreshStatus(state.path!)
                  await refreshStashes(state.path!)
                })}
                onPop={(index) => runAndRefresh('stash_pop', { index }, async () => {
                  await refreshStatus(state.path!)
                  await refreshStashes(state.path!)
                })}
                onDrop={(index) => runAndRefresh('stash_drop', { index }, async () => {
                  await refreshStashes(state.path!)
                })}
              />
            </div>
          </Panel>
          <Separator style={{ width: 4, background: '#333' }} />
          <Panel minSize="30%">
            {viewMode === 'commits' ? (
              <Group orientation="vertical">
                <Panel minSize="30%">
                  <GraphView
                    commits={state.commits}
                    refs={state.refs}
                    selectedId={state.selectedId}
                    onSelect={selectCommit}
                    headHash={state.headHash}
                    changeCount={state.status.unstaged.length + state.status.staged.length}
                    onOpenChanges={() => setViewMode('changes')}
                    onCommitAction={(action, commit) => void handleCommitAction(action, commit)}
                  />
                </Panel>
                <Separator style={{ height: 4, background: '#333' }} />
                <Panel defaultSize="35%" minSize="15%" maxSize="65%">
                  <CommitDetailView
                    commit={detailCommit}
                    diff={state.diff}
                    repoPath={state.path}
                  />
                </Panel>
              </Group>
            ) : (
              <LocalChangesView
                status={state.status}
                diff={state.diff}
                onStage={(paths) => runAndRefresh('stage', { paths }, async () => { await refreshStatus(state.path!); await refreshDiff(state.path!, 'workdir') })}
                onUnstage={(paths) => runAndRefresh('unstage', { paths }, async () => { await refreshStatus(state.path!); await refreshDiff(state.path!, 'workdir') })}
                onDiscard={(paths, staged) => void handleDiscard(paths, staged)}
                onCommit={handleCommit}
              />
            )}
          </Panel>
        </Group>
      </div>
      {promptDialog}
      {confirmDialog}
    </div>
  )
}

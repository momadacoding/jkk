import { useCallback, useEffect, useMemo, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { Panel, Group, Separator } from 'react-resizable-panels'
import { GraphView } from './GraphView'
import { BranchView } from './BranchView'
import { WorktreeView } from './WorktreeView'
import { CommitDetailView } from './CommitDetailView'
import { LocalChangesView } from './LocalChangesView'
import { Toolbar } from './Toolbar'
import { StashView } from './StashView'
import { useRepoStore, useRepoEffects } from '../store/repoStore'
import { samePath } from '../engine/path'
import { askConfirm } from '../engine/confirm'

type ViewMode = 'commits' | 'changes'

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
}

/**
 * 一个标签页的全部内容。
 *
 * 每个实例持有自己的 `useRepoStore`，所以标签之间的 commits / 选中 / staging
 * 互不影响；`repo-changed` 事件在 store 内按路径过滤。
 */
export function RepoTab({ repoPath, openPaths, onOpenInNewTab }: RepoTabProps) {
  const {
    state, openRepo, refreshHead, refreshGraph, refreshStatus, refreshWorktrees,
    refreshDiff, refreshUpstream, refreshStashes, selectCommit, clearError, reportError,
  } = useRepoStore()
  const [repoState, setRepoState] = useState<string>('normal')
  const [viewMode, setViewMode] = useState<ViewMode>('commits')
  const [selectedBranch, setSelectedBranch] = useState<string | null>(null)
  const [selectedWorktree, setSelectedWorktree] = useState<string | null>(null)
  /**
   * 刚删掉的 worktree，用来显示撤销提示条。
   *
   * 删除只是把目录搬进仓库内的回收站，所以撤销是真能撤的。提示条不自动消失：
   * 用户可能正盯着别处，几秒钟就撤不了的「撤销」等于没有。
   */
  const [undoDelete, setUndoDelete] = useState<{ name: string; trashPath: string } | null>(null)

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
    } catch (err) {
      reportError(err)
    }
  }, [state.path, state.headHash, refreshHead, refreshGraph, refreshStatus, refreshUpstream, refreshStashes, reportError])

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
    }
    if (remote) {
      await invoke('delete_remote_branch', { path, remote: remote.remote, branch: remote.branch })
    }
    await invoke('delete_branch', { path, name })
    await refreshGraph(path)
    await refreshUpstream(path)
    // 删掉的 worktree 还挂在侧边栏上，得把那一栏也刷掉
    if (worktreePath) await refreshWorktrees(path)
  }, [state.path, refreshGraph, refreshUpstream, refreshWorktrees])

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
    const deleted = await invoke<{ name: string; trashPath: string }>('delete_worktree', {
      path,
      worktreePath,
      force,
    })
    await refreshWorktrees(path)
    setUndoDelete(deleted)
  }, [state.path, refreshWorktrees])

  /** 撤销刚才那次删除，把目录和登记从回收站搬回原位。 */
  const handleUndoDelete = useCallback(async () => {
    const path = state.path
    if (!path || !undoDelete) return
    try {
      await invoke('restore_worktree', { path, trashPath: undoDelete.trashPath })
      await refreshWorktrees(path)
      setUndoDelete(null)
    } catch (err) {
      reportError(err)
    }
  }, [state.path, undoDelete, refreshWorktrees, reportError])

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
      {undoDelete && (
        <div className="banner-undo">
          <span>已删除 worktree「{undoDelete.name}」，目录还在仓库的回收站里。</span>
          <span className="banner-undo-actions">
            <button className="link" onClick={() => void handleUndoDelete()}>撤销</button>
            <button onClick={() => setUndoDelete(null)}>×</button>
          </span>
        </div>
      )}
      <Toolbar branch={state.head} upstream={state.upstream} onRun={runToolbar} />
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
    </div>
  )
}

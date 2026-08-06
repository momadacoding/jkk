import { useCallback, useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { Panel, Group, Separator } from 'react-resizable-panels'
import { GraphView } from './GraphView'
import { BranchView } from './BranchView'
import { WorktreeView } from './WorktreeView'
import { CommitDetailView } from './CommitDetailView'
import { LocalChangesView } from './LocalChangesView'
import { useRepoStore, useRepoEffects } from '../store/repoStore'
import { samePath } from '../engine/path'

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
    refreshDiff, selectCommit, clearError, reportError,
  } = useRepoStore()
  const [repoState, setRepoState] = useState<string>('normal')
  const [viewMode, setViewMode] = useState<ViewMode>('commits')
  const [selectedBranch, setSelectedBranch] = useState<string | null>(null)
  const [selectedWorktree, setSelectedWorktree] = useState<string | null>(null)

  useRepoEffects(state, { refreshHead, refreshGraph, refreshStatus, refreshWorktrees, refreshDiff }, viewMode)

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
      <div className="repo-path-bar" title={state.path ?? ''}>{state.path} @ {state.head}</div>
      <div style={{ flex: 1, overflow: 'hidden' }}>
        <Group orientation="horizontal">
          <Panel defaultSize="20%" minSize="15%" maxSize="40%" style={{ display: 'flex', flexDirection: 'column' }}>
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
                  await refreshHead(state.path!)
                  await refreshGraph(state.path!)
                  await refreshStatus(state.path!)
                })}
                onCreate={(name, start) => runAndRefresh('create_branch', { name, startPoint: start ?? null }, async () => await refreshGraph(state.path!))}
                onRename={(oldName, newName) => runAndRefresh('rename_branch', { old: oldName, new: newName }, async () => {
                  await refreshHead(state.path!)
                  await refreshGraph(state.path!)
                })}
                onDelete={(name) => runAndRefresh('delete_branch', { name }, async () => await refreshGraph(state.path!))}
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
                onDelete={(name) => runAndRefresh('delete_worktree', { name }, async () => await refreshWorktrees(state.path!))}
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
                    commit={state.commits.find((c) => c.id === state.selectedId) ?? null}
                    diff={state.diff}
                  />
                </Panel>
              </Group>
            ) : (
              <LocalChangesView
                status={state.status}
                diff={state.diff}
                onStage={(paths) => runAndRefresh('stage', { paths }, async () => { await refreshStatus(state.path!); await refreshDiff(state.path!, 'workdir') })}
                onUnstage={(paths) => runAndRefresh('unstage', { paths }, async () => { await refreshStatus(state.path!); await refreshDiff(state.path!, 'workdir') })}
                onCommit={handleCommit}
              />
            )}
          </Panel>
        </Group>
      </div>
    </div>
  )
}

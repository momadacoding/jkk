import { useCallback, useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import { Panel, Group, Separator } from 'react-resizable-panels'
import { GraphView } from './components/GraphView'
import { BranchView } from './components/BranchView'
import { WorktreeView } from './components/WorktreeView'
import { CommitDetailView } from './components/CommitDetailView'
import { LocalChangesView } from './components/LocalChangesView'
import { useRepoStore, useRepoEffects } from './store/repoStore'
import './styles.css'

const DEFAULT_REPO_PATH = 'C:\\Users\\liuhao\\workspace\\dev\\Dev\\mygitclient'
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

function App() {
  const { state, openRepo, refreshGraph, refreshStatus, refreshWorktrees, refreshDiff, selectCommit, clearError } = useRepoStore()
  const [repoState, setRepoState] = useState<string>('normal')
  const [viewMode, setViewMode] = useState<ViewMode>('commits')

  useRepoEffects(state, refreshGraph, refreshStatus, refreshDiff, viewMode)

  useEffect(() => {
    openRepo(DEFAULT_REPO_PATH)
  }, [openRepo])

  useEffect(() => {
    if (!state.path) return
    invoke('unwatch_repo')
    invoke('watch_repo', { path: state.path })
    invoke<string>('repo_state', { path: state.path }).then(setRepoState).catch(() => setRepoState('normal'))
  }, [state.path])

  const handleOpenRepo = useCallback(async () => {
    const selected = await open({ directory: true, multiple: false })
    if (typeof selected === 'string') {
      await openRepo(selected)
    }
  }, [openRepo])

  const handleCommit = useCallback(async (msg: string, amend: boolean) => {
    if (!state.path) return
    try {
      await invoke('commit', { path: state.path, message: msg, amend })
      await refreshGraph(state.path)
      await refreshStatus(state.path)
      await refreshDiff(state.path, 'workdir')
    } catch (err) {
      console.error(err)
    }
  }, [state.path, refreshGraph, refreshStatus, refreshDiff])

  const runAndRefresh = useCallback(async (cmd: string, args: Record<string, unknown>, refresh: () => Promise<void>) => {
    if (!state.path) return
    try {
      await invoke(cmd, { path: state.path, ...args })
      await refresh()
    } catch (err) {
      console.error(err)
    }
  }, [state.path])

  if (state.loading && state.commits.length === 0) return <div style={{ padding: 16 }}>Loading...</div>

  return (
    <div className="app" style={{ display: 'flex', flexDirection: 'column', height: '100vh' }}>
      {repoState !== 'normal' && <div style={{ padding: 8, background: '#ff8a65', color: '#000' }}>{repoState} in progress</div>}
      {state.error && <div style={{ padding: 8, background: '#e57373', color: '#000', display: 'flex', justifyContent: 'space-between' }}>
        <span>{state.error}</span>
        <button onClick={clearError}>×</button>
      </div>}
      <header style={{ padding: 4, borderBottom: '1px solid #333', display: 'flex', alignItems: 'center', gap: 8 }}>
        <button onClick={handleOpenRepo}>Open Repo</button>
        <span style={{ flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{state.path} @ {state.head}</span>
      </header>
      <div style={{ flex: 1, overflow: 'hidden' }}>
        <Group orientation="horizontal">
          <Panel defaultSize="20%" minSize="15%" maxSize="40%" style={{ display: 'flex', flexDirection: 'column' }}>
            <div style={{ flex: 1, overflow: 'auto' }}>
              <SidebarNav
                mode={viewMode}
                changeCount={state.status.unstaged.length + state.status.staged.length}
                onChange={setViewMode}
              />
              <BranchView refs={state.refs} head={state.head}
                onCheckout={(name) => runAndRefresh('checkout', { rev: name }, async () => { await refreshGraph(state.path!); await refreshStatus(state.path!) })}
                onCreate={(name) => runAndRefresh('create_branch', { name }, async () => await refreshGraph(state.path!))}
                onDelete={(name) => runAndRefresh('delete_branch', { name }, async () => await refreshGraph(state.path!))}
              />
              <WorktreeView worktrees={state.worktrees} currentPath={state.path ?? ''}
                onSwitch={async (path) => { await openRepo(path) }}
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
                  <GraphView commits={state.commits} refs={state.refs} selectedId={state.selectedId} onSelect={selectCommit} />
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

export default App

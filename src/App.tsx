import { useCallback, useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import { Panel, Group, Separator } from 'react-resizable-panels'
import { GraphView } from './components/GraphView'
import { BranchView } from './components/BranchView'
import { WorktreeView } from './components/WorktreeView'
import { StagingView } from './components/StagingView'
import { LocalChangesView } from './components/LocalChangesView'
import { useRepoStore, useRepoEffects } from './store/repoStore'
import './styles.css'

const DEFAULT_REPO_PATH = 'C:\\Users\\liuhao\\workspace\\dev\\Dev\\mygitclient'
type ViewMode = 'commits' | 'changes'

function ViewModeSwitch({ mode, onChange }: { mode: ViewMode; onChange: (m: ViewMode) => void }) {
  return (
    <div className="view-mode-switch">
      <button className={mode === 'commits' ? 'active' : ''} onClick={() => onChange('commits')}>All Commits</button>
      <button className={mode === 'changes' ? 'active' : ''} onClick={() => onChange('changes')}>Local Changes</button>
    </div>
  )
}

function App() {
  const { state, openRepo, refreshGraph, refreshStatus, refreshWorktrees, refreshDiff, selectCommit, clearError } = useRepoStore()
  const [selectedFile, setSelectedFile] = useState<string | null>(null)
  const [repoState, setRepoState] = useState<string>('normal')
  const [viewMode, setViewMode] = useState<ViewMode>('commits')

  useRepoEffects(state, refreshGraph, refreshStatus, refreshDiff)

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
    } catch (err) {
      console.error(err)
    }
  }, [state.path, refreshGraph, refreshStatus])

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
          <Panel defaultSize={20} minSize={15} maxSize={40} style={{ display: 'flex', flexDirection: 'column' }}>
            <ViewModeSwitch mode={viewMode} onChange={setViewMode} />
            <div style={{ flex: 1, overflow: 'auto' }}>
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
          <Panel minSize={30}>
            {viewMode === 'commits' ? (
              <Group orientation="vertical">
                <Panel minSize={30}>
                  <GraphView commits={state.commits} refs={state.refs} selectedId={state.selectedId} onSelect={selectCommit} />
                </Panel>
                <Separator style={{ height: 4, background: '#333' }} />
                <Panel defaultSize={35} minSize={15} maxSize={65}>
                  <StagingView
                    status={state.status}
                    diff={state.diff}
                    selectedFile={selectedFile}
                    onSelectFile={setSelectedFile}
                    onStage={(paths) => runAndRefresh('stage', { paths }, async () => await refreshStatus(state.path!))}
                    onUnstage={(paths) => runAndRefresh('unstage', { paths }, async () => await refreshStatus(state.path!))}
                    onCommit={handleCommit}
                  />
                </Panel>
              </Group>
            ) : (
              <LocalChangesView
                status={state.status}
                diff={state.diff}
                onStage={(paths) => runAndRefresh('stage', { paths }, async () => await refreshStatus(state.path!))}
                onUnstage={(paths) => runAndRefresh('unstage', { paths }, async () => await refreshStatus(state.path!))}
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

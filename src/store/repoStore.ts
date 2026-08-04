import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { useCallback, useEffect, useState } from 'react'
import type { CommitNode, DiffData, GraphData, RefPointer, RepoInfo, StatusData, Worktree } from '../engine/types'

export interface RepoState {
  path: string | null
  head: string
  commits: CommitNode[]
  refs: RefPointer[]
  worktrees: Worktree[]
  status: StatusData
  diff: DiffData
  selectedId: string | null
  loading: boolean
  error: string | null
}

const initialState: RepoState = {
  path: null,
  head: '',
  commits: [],
  refs: [],
  worktrees: [],
  status: { staged: [], unstaged: [] },
  diff: { files: [] },
  selectedId: null,
  loading: false,
  error: null,
}

export function useRepoStore() {
  const [state, setState] = useState<RepoState>(initialState)

  const openRepo = useCallback(async (path: string) => {
    setState((s) => ({ ...s, loading: true, error: null }))
    try {
      const info = await invoke<RepoInfo>('open_repo', { path })
      setState((s) => ({ ...s, path, head: info.head, loading: false }))
      await refreshGraph(path)
      await refreshStatus(path)
      const wts = await invoke<Worktree[]>('read_worktrees_cmd', { path })
      setState((s) => ({ ...s, worktrees: wts }))
    } catch (err) {
      setState((s) => ({ ...s, loading: false, error: err instanceof Error ? err.message : String(err) }))
    }
  }, [])

  const refreshGraph = async (path: string) => {
    const data = await invoke<GraphData>('load_graph', { path })
    setState((s) => ({
      ...s,
      commits: data.commits,
      refs: data.refs,
      selectedId: s.selectedId ?? data.commits[0]?.id ?? null,
    }))
  }

  const refreshStatus = async (path: string) => {
    const status = await invoke<StatusData>('read_status', { path })
    setState((s) => ({ ...s, status }))
  }

  const refreshWorktrees = async (path: string) => {
    const wts = await invoke<Worktree[]>('read_worktrees_cmd', { path })
    setState((s) => ({ ...s, worktrees: wts }))
  }

  const refreshDiff = async (path: string, scope: string) => {
    const diff = await invoke<DiffData>('read_diff', { path, scope })
    setState((s) => ({ ...s, diff }))
  }

  const selectCommit = (id: string) => {
    setState((s) => ({ ...s, selectedId: id }))
  }

  const clearError = () => setState((s) => ({ ...s, error: null }))

  return {
    state,
    openRepo,
    refreshGraph,
    refreshStatus,
    refreshWorktrees,
    refreshDiff,
    selectCommit,
    clearError,
  }
}

export function useRepoEffects(
  state: RepoState,
  refreshGraph: (path: string) => Promise<void>,
  refreshStatus: (path: string) => Promise<void>,
  refreshDiff: (path: string, scope: string) => Promise<void>,
) {
  useEffect(() => {
    if (!state.path || !state.selectedId) return
    refreshDiff(state.path, `commit:${state.selectedId}`)
  }, [state.selectedId, state.path])

  useEffect(() => {
    if (!state.path) return
    let unlisten: (() => void) | undefined
    listen<string>('repo-changed', () => {
      refreshGraph(state.path!)
      refreshStatus(state.path!)
    }).then((fn) => { unlisten = fn })
    return () => { unlisten?.() }
  }, [state.path])
}

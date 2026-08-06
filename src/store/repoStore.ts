import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { useCallback, useEffect, useState } from 'react'
import type { CommitNode, DiffData, GraphData, RefPointer, RepoInfo, StatusData, Worktree } from '../engine/types'
import { toErrorMessage } from '../engine/errors'
import { samePath } from '../engine/path'

export interface RepoState {
  path: string | null
  /** 分支名，显示用。 */
  head: string
  /** HEAD 的完整 oid，工作区伪行挂在它上面。别和 head 混用。 */
  headHash: string
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
  headHash: '',
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
      setState((s) => ({ ...s, path, head: info.head, headHash: info.headHash, loading: false }))
      // 打开一个 worktree 时想看的是「它在哪」，所以定位到它自己的 HEAD。
      // 这里必须把 headHash 显式传下去——上面那次 setState 是异步的，
      // refreshGraph 里的 updater 读不到刚写进去的值。
      await refreshGraph(path, info.headHash)
      await refreshStatus(path)
      const wts = await invoke<Worktree[]>('read_worktrees_cmd', { path })
      setState((s) => ({ ...s, worktrees: wts }))
    } catch (err) {
      setState((s) => ({ ...s, loading: false, error: toErrorMessage(err) }))
    }
  }, [])

  /** 重新读取 HEAD。checkout 之后必须调用，否则 header 与分支高亮会停在旧分支。 */
  const refreshHead = async (path: string) => {
    const info = await invoke<RepoInfo>('open_repo', { path })
    setState((s) => ({ ...s, head: info.head, headHash: info.headHash }))
  }

  /**
   * 重新加载提交图。
   *
   * `preferredId` 只在还没有任何选中时生效，用来把首次打开的仓库定位到它自己的
   * HEAD。不传的话会退回「图上第一条提交」——那是**全局最新**的提交，可能属于
   * 另一条分支，跟你刚打开的这个 worktree 毫无关系。
   */
  const refreshGraph = async (path: string, preferredId?: string) => {
    const data = await invoke<GraphData>('load_graph', { path })
    setState((s) => {
      // HEAD 万一不在这批数据里（5000 条上限截断）就退回最新一条
      const preferred = preferredId && data.commits.some((c) => c.id === preferredId)
        ? preferredId
        : undefined
      return {
        ...s,
        commits: data.commits,
        refs: data.refs,
        selectedId: s.selectedId ?? preferred ?? data.commits[0]?.id ?? null,
      }
    })
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

  const clearError = useCallback(() => setState((s) => ({ ...s, error: null })), [])

  /** 把一次失败呈现到顶部错误横幅。 */
  const reportError = useCallback(
    (err: unknown) => setState((s) => ({ ...s, error: toErrorMessage(err) })),
    [],
  )

  return {
    state,
    openRepo,
    refreshHead,
    refreshGraph,
    refreshStatus,
    refreshWorktrees,
    refreshDiff,
    selectCommit,
    clearError,
    reportError,
  }
}

/** `useRepoEffects` 需要的刷新函数集合。 */
export interface RepoRefreshers {
  refreshHead: (path: string) => Promise<void>
  refreshGraph: (path: string) => Promise<void>
  refreshStatus: (path: string) => Promise<void>
  refreshWorktrees: (path: string) => Promise<void>
  refreshDiff: (path: string, scope: string) => Promise<void>
}

export function useRepoEffects(
  state: RepoState,
  refreshers: RepoRefreshers,
  viewMode: string,
) {
  const { refreshHead, refreshGraph, refreshStatus, refreshWorktrees, refreshDiff } = refreshers
  const diffScope = viewMode === 'changes'
    ? 'workdir'
    : state.selectedId
      ? `commit:${state.selectedId}`
      : null

  useEffect(() => {
    if (!state.path || !diffScope) return
    refreshDiff(state.path, diffScope)
  }, [state.selectedId, state.path, diffScope])

  useEffect(() => {
    if (!state.path) return
    let unlisten: (() => void) | undefined
    listen<string>('repo-changed', (event) => {
      const path = state.path!
      // 事件是全局广播的，每个标签只认自己那个仓库的变更
      if (!samePath(event.payload, path)) return
      // 刷新期间仓库可能正处于中间态（比如 checkout 写到一半），失败就等下一次事件
      const ignore = () => undefined
      // 外部 checkout 同样会改 HEAD 和 worktree 归属，两者都要跟着刷新
      void refreshHead(path).catch(ignore)
      void refreshGraph(path).catch(ignore)
      void refreshStatus(path).catch(ignore)
      void refreshWorktrees(path).catch(ignore)
      if (diffScope) void refreshDiff(path, diffScope).catch(ignore)
    }).then((fn) => { unlisten = fn })
    return () => { unlisten?.() }
  }, [state.path, diffScope])
}

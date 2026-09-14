import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { useCallback, useEffect, useState } from 'react'
import type { CommitNode, DiffData, GraphData, PullRequest, RefPointer, RepoInfo, StashEntry, StatusData, UpstreamInfo, Worktree } from '../engine/types'
import { ALL_CAPABILITIES, type Capabilities } from '../engine/capabilities'
import { toErrorMessage } from '../engine/errors'
import type { Health, InitReport, Preflight, RebuildReport } from '../engine/svnInit'
import { samePath } from '../engine/path'
import { nextSelection } from '../engine/selection'

export interface RepoState {
  path: string | null
  /** 分支名，显示用。 */
  head: string
  /** HEAD 的完整 oid，工作区伪行挂在它上面。别和 head 混用。 */
  headHash: string
  commits: CommitNode[]
  refs: RefPointer[]
  worktrees: Worktree[]
  upstream: UpstreamInfo | null
  stashes: StashEntry[]
  /**
   * 当前仓库的 open PR。空数组既表示「没有 PR」也表示「这台机器读不到」——
   * 两者在界面上是同一件事：分支旁边不画徽章。
   */
  pullRequests: PullRequest[]
  /**
   * 当前后端支持哪些功能。组件据此**不渲染**对应入口。
   *
   * 默认（也是读取失败时的兜底）是全开：普通 git 仓库占绝大多数，
   * 把它们因为一次后端读取失败搞成半残，比在 SVN 仓库上多露一个入口更糟。
   */
  capabilities: Capabilities
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
  upstream: null,
  stashes: [],
  pullRequests: [],
  capabilities: ALL_CAPABILITIES,
  status: { staged: [], unstaged: [] },
  diff: { files: [] },
  selectedId: null,
  loading: false,
  error: null,
}

/**
 * 读取后端能力，失败时退回全开。
 *
 * 为什么不让失败冒泡：`read_capabilities` 走的是 `backend_for`，而打开仓库那条
 * 路径上 `open_repo` 已经先跑过同一个 `backend_for` 了——真的打不开的仓库在那里
 * 就报错了。能走到这里还失败的，只剩「命令没注册」这类本项目自己的 bug，
 * 把普通 git 仓库因此搞成半残不值当。
 */
async function readCapabilities(path: string): Promise<Capabilities> {
  try {
    return await invoke<Capabilities>('read_capabilities', { path })
  } catch (err) {
    console.warn('读取后端能力失败，按普通 git 仓库渲染全部入口：', err)
    return ALL_CAPABILITIES
  }
}

// ---------------------------------------------------------------------------
// SVN 混合仓库（Phase 1b）
//
// 这四个是**薄壳**：不吞错误、不重试、不做判断。三个会改磁盘的命令失败时必须
// 原样抛出去，让向导留在原地把错误显示给用户——`readCapabilities` 那种「失败就
// 退回默认值」的写法在这里是灾难性的（用户会以为建库成功了）。
//
// ⚠️ 参数名是 `snake_case` → `camelCase` 转换后的形态，没有编译期检查
// （AGENTS.md §7.3）。Rust 侧 `path` / `whitelist` / `extensions` 三个都是单词，
// 转换后不变，但别因此以为这层不存在。
// ---------------------------------------------------------------------------

/** 只跑 P1–P4，什么都不创建。 */
export function svnPreflightCheck(path: string, whitelist: string[]): Promise<Preflight> {
  return invoke<Preflight>('svn_preflight_check', { path, whitelist })
}

/** 走完初始化向导。**会在工作副本里创建 `.git`**；任何一步失败后端会自己删干净。 */
export function svnInitHybrid(
  path: string,
  whitelist: string[],
  extensions: string[],
): Promise<InitReport> {
  return invoke<InitReport>('svn_init_hybrid', { path, whitelist, extensions })
}

/** 重建基线。**会移动基线指针**；不动 HEAD、不动索引、不动磁盘。 */
export function svnRebuildBaseline(path: string): Promise<RebuildReport> {
  return invoke<RebuildReport>('svn_rebuild_baseline', { path })
}

/**
 * 体检当前基线。
 *
 * 三种结果都从 `Promise` 正常返回，**只有命令本身没跑起来才 reject**：
 * 「没能验证」与「验证不通过」必须分开（spec §6.8.6）。
 */
export function svnCheckBaseline(path: string): Promise<Health> {
  return invoke<Health>('svn_check_baseline', { path })
}

export function useRepoStore() {
  const [state, setState] = useState<RepoState>(initialState)

  const openRepo = useCallback(async (path: string) => {
    setState((s) => ({ ...s, loading: true, error: null }))
    try {
      const info = await invoke<RepoInfo>('open_repo', { path })
      // 每次打开都重新读，且**必然写回**：上一个仓库是 SVN 时留下的能力集合
      // 会把下一个普通 git 仓库的入口一起关掉。
      const caps = await readCapabilities(path)
      setState((s) => ({
        ...s,
        path,
        head: info.head,
        headHash: info.headHash,
        capabilities: caps,
        loading: false,
      }))
      // 打开一个 worktree 时想看的是「它在哪」，所以定位到它自己的 HEAD。
      // 这里必须把 headHash 显式传下去——上面那次 setState 是异步的，
      // refreshGraph 里的 updater 读不到刚写进去的值。
      await refreshGraph(path, info.headHash)
      await refreshStatus(path)
      const wts = await invoke<Worktree[]>('read_worktrees_cmd', { path })
      setState((s) => ({ ...s, worktrees: wts }))
      await refreshUpstream(path)
      await refreshStashes(path)
      // 不 await：这一步要走网络（gh 实测一两秒），让它拖住打开仓库的收尾没有意义。
      // 不支持 PR 的后端连这次 gh 调用都省掉——界面上没有任何地方会用到结果。
      if (caps.pullRequests) void refreshPullRequests(path)
    } catch (err) {
      setState((s) => ({ ...s, loading: false, error: toErrorMessage(err) }))
    }
  }, [])

  /**
   * 重新读取 HEAD，返回新的 headHash。
   *
   * checkout 之后必须调用，否则 header 与分支高亮会停在旧分支。返回值让调用方
   * 能判断 HEAD 有没有动——上面那次 setState 是异步的，读 state 拿到的是旧值。
   */
  const refreshHead = async (path: string): Promise<string> => {
    const info = await invoke<RepoInfo>('open_repo', { path })
    setState((s) => ({ ...s, head: info.head, headHash: info.headHash }))
    return info.headHash
  }

  /**
   * 重新加载提交图。
   *
   * `preferredId` 默认只在还没有任何选中时生效，用来把首次打开的仓库定位到它自己的
   * HEAD。不传的话会退回「图上第一条提交」——那是**全局最新**的提交，可能属于
   * 另一条分支，跟你刚打开的这个 worktree 毫无关系。
   *
   * `force` 让 preferredId 压过已有选中，用于 pull / checkout 这类由用户主动点出来、
   * 且移动了 HEAD 的操作。选中判定见 `nextSelection`。
   */
  const refreshGraph = async (path: string, preferredId?: string, force = false) => {
    const data = await invoke<GraphData>('load_graph', { path })
    setState((s) => ({
      ...s,
      commits: data.commits,
      refs: data.refs,
      selectedId: nextSelection(s.selectedId, data.commits.map((c) => c.id), preferredId, force),
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

  const refreshUpstream = async (path: string) => {
    const upstream = await invoke<UpstreamInfo>('read_upstream', { path })
    setState((s) => ({ ...s, upstream }))
  }

  const refreshStashes = async (path: string) => {
    const stashes = await invoke<StashEntry[]>('read_stashes', { path })
    setState((s) => ({ ...s, stashes }))
  }

  /**
   * 重新读取 open PR 列表。
   *
   * **只在打开仓库和用户主动 fetch/pull/push 之后调用**，绝不能挂进
   * `repo-changed` 那条文件监听通道：那条通道一次编辑就触发一次，而这里要走网络。
   *
   * 失败只写 console 不上报：顶部错误横幅是留给 git 操作的，被这条锦上添花的信息
   * 占住的话，真正的失败反而看不见。后端在 gh 缺失/未登录/非 GitHub 仓库时已经
   * 返回空列表，所以走到 catch 的只剩「gh 输出解析不了」这种本项目自己的 bug。
   */
  const refreshPullRequests = async (path: string) => {
    try {
      const prs = await invoke<PullRequest[]>('list_pull_requests', { path })
      setState((s) => ({ ...s, pullRequests: prs }))
    } catch (err) {
      console.warn('读取 PR 列表失败，分支旁不显示编号徽章：', err)
    }
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
    refreshUpstream,
    refreshStashes,
    refreshPullRequests,
    selectCommit,
    clearError,
    reportError,
  }
}

/** `useRepoEffects` 需要的刷新函数集合。 */
export interface RepoRefreshers {
  refreshHead: (path: string) => Promise<string>
  refreshGraph: (path: string) => Promise<void>
  refreshStatus: (path: string) => Promise<void>
  refreshWorktrees: (path: string) => Promise<void>
  refreshDiff: (path: string, scope: string) => Promise<void>
  refreshUpstream: (path: string) => Promise<void>
  refreshStashes: (path: string) => Promise<void>
}

export function useRepoEffects(
  state: RepoState,
  refreshers: RepoRefreshers,
  viewMode: string,
) {
  const { refreshHead, refreshGraph, refreshStatus, refreshWorktrees, refreshDiff, refreshUpstream, refreshStashes } = refreshers
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
      // 外部 push/fetch 会改变领先落后数，命令行 stash 也要反映到面板上
      void refreshUpstream(path).catch(ignore)
      void refreshStashes(path).catch(ignore)
      if (diffScope) void refreshDiff(path, diffScope).catch(ignore)
    }).then((fn) => { unlisten = fn })
    return () => { unlisten?.() }
  }, [state.path, diffScope])
}

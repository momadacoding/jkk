import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { useCallback, useEffect, useRef, useState } from 'react'
import type { CommitNode, DiffData, GraphData, PullRequest, RefPointer, RepoInfo, StashEntry, StatusData, UpstreamInfo, Worktree } from '../engine/types'
import { ALL_CAPABILITIES, type Capabilities } from '../engine/capabilities'
import {
  auditIsDue,
  recordAfterSuccess,
  syncIfRootRevChanged,
  SVN_AUDIT_POLL_MS,
  type RootRevMemo,
  type ServerAudit,
  type SyncStatus,
} from '../engine/svnSync'
import { baselineMap, remapRefs, spliceHistory, type SvnRevNode } from '../engine/svnGraph'

import { toErrorCode, toErrorMessage } from '../engine/errors'
import type {
  Health,
  InitReport,
  PinnedExternal,
  Preflight,
  RebuildReport,
} from '../engine/svnInit'
import { samePath } from '../engine/path'
import { nextSelection } from '../engine/selection'

/** 后端 `svn_sync::RootRev`。 */
interface RootRev {
  wcRoot: string
  rev: number
}

/** 首屏与每次「加载更多」拉多少条（spec §10.2：不做无限滚动）。 */
const SVN_LOG_PAGE = 200

/**
 * 把一次 rootRev 观测压成一个可比字符串。
 *
 * 排序是必须的：`enumerate_wcs` 的返回顺序没有契约，顺序一飘就会被当成
 * 「有人动过」，于是每次刷新都触发全量校验。
 */
function serializeRootRevs(revs: RootRev[]): string {
  return revs
    .map((r) => `${r.wcRoot}@${r.rev}`)
    .sort()
    .join('|')
}

/**
 * 把 SVN 序列接进 git 历史——**只在 SVN 模式下**。
 *
 * # 为什么普通 git 仓库必须原样返回
 *
 * `spliceHistory` 会把带 `git-svn-id` trailer 的 commit 当成 baseline
 * **从图上删掉**（spec §10.3 ③：它在图上的位置由 SVN 序列决定）。
 *
 * 而一个从 `git svn` 迁移过来的**普通 git 仓库**，历史里遍地都是这种 trailer
 * ——那是 `git svn` 自己写的（spec §6.4.1 就是照它的格式设计的）。无条件接合的话，
 * 这类仓库一打开就会发现大半历史凭空消失，而 svnRevs 是空的、没有任何东西接住它们。
 *
 * 所以判据必须是「这个仓库是不是 SVN 混合仓库」，而不是「svnRevs 空不空」。
 */
function merge(s: RepoState, gitCommits: CommitNode[], svnRevs: SvnRevNode[]): CommitNode[] {
  if (s.capabilities.vcs !== 'svn') return gitCommits
  return spliceHistory(svnRevs, gitCommits)
}

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
  /**
   * 上一次失败的错误码；`null` 表示没失败、或那个异常不带 code。
   *
   * 与 `error` 分开存：`error` 是给人看的消息，这个是给**代码**分流用的。
   * 目前唯一的消费方是 `svn_not_initialized` —— 「有 `.svn` 无 `.git`」不是错误，
   * 是一个还没走的流程（spec §6.1），该显示初始化向导而不是一条红色横幅。
   */
  errorCode: string | null
  /**
   * 被 `-r` 钉死、且与白名单有交集的 external。非空即处于 pinned 模式。
   *
   * spec §6.5.3 第 3 条要求这条残余风险**UI 常驻说明**——只在初始化向导里显示
   * 一次不算常驻，用户下次打开这个仓库就看不到了。所以它进 store，每次打开都读。
   */
  pinned: PinnedExternal[]
  /**
   * 失同步的完整校验结果（spec §6.6.3 的**第二档**）。`null` = 还没读到。
   *
   * `null` 与「同步」是两回事：打开仓库的那一瞬间它是 `null`，那时挂任何横幅
   * 都等于每次打开都闪一下。`describeSync(null)` 因此返回 `null`。
   */
  svnSync: SyncStatus | null
  /** 服务器侧清单核对（§6.3.1 / NB-2）。走网络，失败静默。 */
  svnAudit: ServerAudit | null
  /** SVN 历史（缓存 + 已拉取的），按 revision 降序。 */
  svnRevs: SvnRevNode[]
  /**
   * 后端 `load_graph` 原样给的 git 提交，**未接合**。
   *
   * `commits` 是接合后的结果；两者分开存是因为 svnRevs 与 gitCommits 各自会被
   * 异步刷新，任一变化都要重新接一次，而接合是纯函数，重算比增量维护更不容易错。
   */
  gitCommits: CommitNode[]
  /**
   * 已经翻到最早的 revision 了，「加载更多」该禁用。**由后端算，这里只转发。**
   *
   * 判据是「每个 target 的**连续覆盖区间**都到了 r1」，而覆盖区间只存在于后端
   * 缓存里。前端手上的 `svnRevs` 是所有 target ∪ 孤儿锚点去重后的列表，
   * 既看不出连续性、也分不清哪条属于哪个 target，用它推断完整性怎么写都是错的
   * （2026-09-16 独立 review：原先「这一页没带回新东西 = 到底了」的写法，
   * 配上同样取自并集的翻页游标，会在中间还差几百个 revision 时就把按钮禁掉）。
   */
  svnLogComplete: boolean
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
  errorCode: null,
  pinned: [],
  svnSync: null,
  svnAudit: null,
  svnRevs: [],
  gitCommits: [],
  svnLogComplete: false,
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

/**
 * 当前仓库里被钉死、且与白名单有交集的 external。非混合仓库返回空列表。
 *
 * ⚠️ 后端要跑 `svn propget -R`，在几万文件的工作副本上不便宜。
 * 调用方**不要 await**。
 */
export function svnPinnedExternals(path: string): Promise<PinnedExternal[]> {
  return invoke<PinnedExternal[]>('svn_pinned_externals', { path })
}

/** 完整失同步校验（spec §6.6.3 第二档）。纯本地读，不走网络。 */
export function svnSyncStatus(path: string): Promise<SyncStatus> {
  return invoke<SyncStatus>('svn_sync_status', { path })
}

/** 每个工作副本根的 revision（第一档，O(1)）。挂在高频刷新上。 */
export function svnRootRevs(path: string): Promise<RootRev[]> {
  return invoke<RootRev[]>('svn_root_revs', { path })
}

/** 一页 SVN 历史 + 「还能不能再往下翻」，对应后端 `LogPage`。 */
export interface SvnLogPage {
  revs: SvnRevNode[]
  complete: boolean
}

/** 读缓存里的 SVN 历史，**不走网络**。打开仓库时先用它把图画出来。 */
export function svnReadLog(path: string): Promise<SvnLogPage> {
  return invoke<SvnLogPage>('svn_read_log', { path })
}

/**
 * 拉取 SVN 历史，**走网络**。`older = true` 是往下翻一页。
 *
 * ⚠️ 翻页游标**不在这里传**：它由后端按各 target 自己的连续覆盖区间算
 * （见 Rust 侧 `fetch_log` 的文档）。这里只说「再往下翻一页」。
 */
export function svnFetchLog(path: string, older: boolean, limit: number): Promise<SvnLogPage> {
  return invoke<SvnLogPage>('svn_fetch_log', { path, older, limit })
}

/** 服务器侧清单核对（§6.3.1 / NB-2）。**走网络**，可能不便宜。 */
export function svnServerAudit(path: string): Promise<ServerAudit> {
  return invoke<ServerAudit>('svn_server_audit', { path })
}

export function useRepoStore() {
  const [state, setState] = useState<RepoState>(initialState)
  /**
   * 上一次观测到的每个工作副本根的 revision，序列化成字符串好比对。
   *
   * **放 ref 不放 state**：读它的是 `repo-changed` 监听器，而那个 `useEffect`
   * 的依赖是 `[state.path, diffScope]`。放进 state 的话监听器会**永远闭包住
   * 注册那一刻的值**（`null`），比较恒为不等，于是第二档的全量校验
   * 每次文件变动都跑一遍——分层就白做了。这是独立 review 抓到的。
   *
   * §6.6.3 第一档的判据是「与**上次观测值**比」，**不是**与 `baselineRev` 比：
   * 交付之后根节点会长期停在旧 revision（spec §6.6.2 有实测），拿它跟
   * `baselineRev` 比会每次刷新都判不等，同样让分层归零。
   *
   * 进程重启后是 `null`，第一次刷新当作「变了」跑一次全量——那正是打开仓库
   * 本来就要做的事。
   *
   * 盒子里那个 `value` **什么时候可以写**是 `svnSync.ts` 里
   * `recordAfterSuccess` 的事：只有第二档真的跑成功了才记。
   */
  const lastRootRevs = useRef<RootRevMemo>({ value: null })

  /**
   * 上一次**开始**服务器侧核对的时刻，与 `auditInFlight` 一起构成
   * `auditIsDue` 的输入。用 ref 而不是 state：写它不该引起重渲染，
   * 而且定时器闭包读 state 会永远读到注册那一刻的值（`lastRootRevs`
   * 的注释里记的是同一个坑）。
   */
  const lastAuditAt = useRef(0)
  const auditInFlight = useRef(false)

  const openRepo = useCallback(async (path: string) => {
    setState((s) => ({ ...s, loading: true, error: null, errorCode: null }))
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
      // 同样不 await：后端要跑 `svn propget -R`，在几万文件的工作副本上是秒级。
      // 常驻说明晚一两秒出现无所谓，但不能让打开仓库卡在这儿（spec §6.5.3）。
      void refreshPinned(path)
      if (caps.vcs === 'svn') {
        // 第二档的完整校验：打开仓库是正确性的闸门之一，必须问到底（spec §6.6.3）。
        // 纯本地读，await 得起。
        await refreshSvnSync(path).catch(() => undefined)
        // 先用**缓存**把图画出来，再后台拉新的（spec §10.2）。
        await refreshSvnLog(path).catch(() => undefined)
        // 这两条走网络，照 `refreshPullRequests` 的既有模式：不 await、失败静默。
        void fetchSvnLog(path)
        void refreshSvnAudit(path)
      } else {
        // **必须显式清空**：上一个仓库是 SVN 时留下的横幅与历史，会挂在下一个
        // 完全无关的 git 仓库上（`refreshPinned` 的注释里记过同一个坑）。
        setState((s) => ({
          ...s,
          svnSync: null,
          svnAudit: null,
          svnRevs: [],
          svnLogComplete: false,
          // gitCommits 不用清：refreshGraph 每次都整份覆盖它
        }))
      }
    } catch (err) {
      setState((s) => ({
        ...s,
        loading: false,
        error: toErrorMessage(err),
        errorCode: toErrorCode(err),
      }))
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
    setState((s) => {
      const commits = merge(s, data.commits, s.svnRevs)
      // refs 必须跟着一起重映射：baseline commit 被从节点列表里删掉了，而
      // GraphView 按 `refByHash[node.commit.id]` 渲染标签——不映射的话
      // `svn/base` 匹配不到任何一行，**整个看不见**。
      const refs =
        s.capabilities.vcs === 'svn'
          ? remapRefs(data.refs, baselineMap(data.commits))
          : data.refs
      return {
        ...s,
        gitCommits: data.commits,
        commits,
        refs,
        selectedId: nextSelection(s.selectedId, commits.map((c) => c.id), preferredId, force),
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

  const refreshUpstream = async (path: string) => {
    const upstream = await invoke<UpstreamInfo>('read_upstream', { path })
    setState((s) => ({ ...s, upstream }))
  }

  const refreshStashes = async (path: string) => {
    const stashes = await invoke<StashEntry[]>('read_stashes', { path })
    setState((s) => ({ ...s, stashes }))
  }

  /** 第二档本身：完整的 `nodeRevs` 校验（spec §6.6.3）。纯本地读，不碰观测值。 */
  const applySvnSync = async (path: string) => {
    const st = await svnSyncStatus(path)
    setState((s) => ({ ...s, svnSync: st }))
  }

  const readRootRevs = async (path: string) => serializeRootRevs(await svnRootRevs(path))

  /**
   * 第二档的**无条件**入口：打开仓库、重建基线之后各用一次。
   *
   * 顺带把观测到的 rootRevs 记下来当基准——不记的话下一次刷新会因为
   * `lastRootRevs` 还是 `null` 而再跑一次全量。**先读 revision 再跑校验**，
   * 而且只在校验成功后才记，理由见 `recordAfterSuccess`。
   */
  const refreshSvnSync = async (path: string) => {
    const observed = await readRootRevs(path)
    await recordAfterSuccess(lastRootRevs.current, observed, () => applySvnSync(path))
  }

  /**
   * 第一档（O(1)）：rootRev 与**上次观测值**比，变了才跑第二档。
   *
   * 返回是否真的跑了第二档，只为让调用方能读懂日志；调用方不依赖它。
   */
  const checkSvnSyncIfChanged = (path: string) =>
    syncIfRootRevChanged(
      lastRootRevs.current,
      () => readRootRevs(path),
      () => applySvnSync(path),
    )

  /** 读缓存里的 SVN 历史并接进图。不走网络。 */
  const refreshSvnLog = async (path: string) => {
    const page = await svnReadLog(path)
    // svnRevs 一变就要重接一次：后台拉取是异步的，先画出来的图里
    // svn 节点还没到位，工作分支那时还挂在 baseline commit 上。
    setState((s) => ({
      ...s,
      svnRevs: page.revs,
      commits: merge(s, s.gitCommits, page.revs),
      svnLogComplete: page.complete,
    }))
  }

  /**
   * 走网络拉 SVN 历史。**失败只写 console 不上报**——顶部错误横幅是留给 git
   * 操作的，被这条锦上添花的信息占住的话，真正的失败反而看不见
   * （与 `refreshPullRequests` 同一条理由）。
   */
  const fetchSvnLog = async (path: string, older = false) => {
    try {
      const page = await svnFetchLog(path, older, SVN_LOG_PAGE)
      // svnRevs 一变就要重接一次：后台拉取是异步的，先画出来的图里
      // svn 节点还没到位，工作分支那时还挂在 baseline commit 上。
      setState((s) => ({
        ...s,
        svnRevs: page.revs,
        commits: merge(s, s.gitCommits, page.revs),
        svnLogComplete: page.complete,
      }))
    } catch (err) {
      console.warn('拉取 SVN 历史失败，图上只显示缓存里已有的部分：', err)
    }
  }

  /**
   * 服务器侧清单核对。走网络。
   *
   * `keepOnError` 分开两种调用场景，它们对失败的正确反应**相反**：
   *
   * - 打开仓库 / 重建之后（`false`）：必须**写回 null**。这次的结果是权威的，
   *   失败时留着上一个仓库的结论，会把 A 仓库的漏收挂到 B 仓库头上。
   * - 后台自动重跑（`true`）：必须**保留**上一次的结论。一次网络抖动就把一条
   *   已经成立的漏收警告抹掉，是把真警报降级成沉默——而这条检查存在的全部理由
   *   就是那个沉默很危险。
   */
  const refreshSvnAudit = async (path: string, keepOnError = false) => {
    lastAuditAt.current = Date.now()
    try {
      const audit = await svnServerAudit(path)
      // 核对期间用户可能已经切到别的仓库了。不比一下路径的话，这次的结果会写进
      // 另一个完全无关的仓库的横幅里——后台定时重跑之后这个窗口大了很多。
      setState((s) => (samePath(s.path ?? '', path) ? { ...s, svnAudit: audit } : s))
    } catch (err) {
      console.warn('服务器侧清单核对失败：', err)
      if (keepOnError) return
      setState((s) => (samePath(s.path ?? '', path) ? { ...s, svnAudit: null } : s))
    }
  }

  /**
   * 后台自动重跑的入口：过了闸才真的发请求（`auditIsDue`）。
   *
   * 这条检查要抓的是用户**不知情**的漏收，所以不能只靠手动入口——不知道出事的人
   * 不会去点那颗按钮（2026-09-19 用户决定）。
   */
  const auditSvnIfDue = async (path: string) => {
    if (!auditIsDue(Date.now(), lastAuditAt.current, auditInFlight.current)) return
    auditInFlight.current = true
    try {
      await refreshSvnAudit(path, true)
    } finally {
      auditInFlight.current = false
    }
  }

  /**
   * 重新读取被钉死的 external（spec §6.5.3 的常驻说明）。
   *
   * 失败只写 console 不上报，理由与 `refreshPullRequests` 同：顶部错误横幅是留给
   * git 操作的。**必须写回空数组**——上一个仓库处于 pinned 模式时留下的清单，
   * 会挂在下一个完全无关的仓库上。
   */
  const refreshPinned = async (path: string) => {
    try {
      const pinned = await svnPinnedExternals(path)
      setState((s) => ({ ...s, pinned }))
    } catch (err) {
      console.warn('读取 pinned external 失败，不显示 pinned 模式说明：', err)
      setState((s) => ({ ...s, pinned: [] }))
    }
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

  const clearError = useCallback(
    () => setState((s) => ({ ...s, error: null, errorCode: null })),
    [],
  )

  /** 把一次失败呈现到顶部错误横幅。 */
  const reportError = useCallback(
    (err: unknown) =>
      setState((s) => ({ ...s, error: toErrorMessage(err), errorCode: toErrorCode(err) })),
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
    refreshSvnSync,
    checkSvnSyncIfChanged,
    refreshSvnLog,
    fetchSvnLog,
    refreshSvnAudit,
    auditSvnIfDue,
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
  /** §6.6.3 第二档。只在第一档报告「有人动过」之后才调。 */
  /** §6.6.3 的第一档 + 条件触发第二档。挂在高频的 repo-changed 上。 */
  checkSvnSyncIfChanged: (path: string) => Promise<boolean>
  /** 服务器侧核对的**后台**入口，自带频率闸。挂定时器与窗口焦点。 */
  auditSvnIfDue: (path: string) => Promise<void>
}

export function useRepoEffects(
  state: RepoState,
  refreshers: RepoRefreshers,
  viewMode: string,
) {
  const { refreshHead, refreshGraph, refreshStatus, refreshWorktrees, refreshDiff, refreshUpstream, refreshStashes, checkSvnSyncIfChanged, auditSvnIfDue } = refreshers
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
      // §6.6.3 的**分层**：高频通道上只跑 O(1) 的第一档，变了才跑第二档。
      //
      // 不分层的话，每一次文件保存都会触发一遍白名单子树的 `svn status -v`
      // ——那是本地读，但在几万文件的工作副本上仍然是可感知的开销。
      //
      // 比对用的「上次观测值」存在 **ref** 里（见 `useRepoStore` 的
      // `lastRootRevs`）：这个监听器的 `useEffect` 依赖是 `[state.path, diffScope]`，
      // 把它放进 state 的话闭包会永远读到注册那一刻的值，比较恒为不等，
      // 第二档还是每次都跑——分层白做。
      if (state.capabilities.vcs === 'svn') {
        void checkSvnSyncIfChanged(path).catch(ignore)
      }
    }).then((fn) => { unlisten = fn })
    return () => { unlisten?.() }
  }, [state.path, diffScope])

  /**
   * 服务器侧漏收核对的**自动**触发（2026-09-19）。
   *
   * # 为什么不挂在 repo-changed 上
   *
   * 那条通道一次文件保存就触发一次，而这里要走网络（`svn ls -R`）。
   * `refreshPullRequests` 的注释里记的是同一条理由。
   *
   * # 为什么不能只留手动入口
   *
   * `--set-depth=exclude` 造成的漏收，SVN 侧四道检查全是绿的、磁盘上也看不见。
   * 用户不知情，就不会去点「检查一下」。所以它必须自己跑起来。
   *
   * # 两个触发点
   *
   * - **定时器**：兜底，覆盖「应用一直开着、后台脚本动了工作副本」。
   * - **窗口获得焦点**：用户刚从 TortoiseSVN / 终端切回来——最可能刚出事的时刻。
   *   它会被 `SVN_AUDIT_MIN_GAP_MS` 那道闸限频，来回切窗口打不出高频请求。
   *
   * 页面不可见时跳过：最小化着的窗口每五分钟发一次网络请求没有意义。这不是
   * 安全关键——闸本身已经把最坏情况钉住了，这里只是省掉明显的浪费。
   */
  useEffect(() => {
    if (!state.path || state.capabilities.vcs !== 'svn') return
    const path = state.path
    const tick = () => {
      if (document.hidden) return
      void auditSvnIfDue(path).catch(() => undefined)
    }
    const timer = window.setInterval(tick, SVN_AUDIT_POLL_MS)
    window.addEventListener('focus', tick)
    return () => {
      window.clearInterval(timer)
      window.removeEventListener('focus', tick)
    }
    // ⚠️ 依赖里**不放** `auditSvnIfDue`：它每次渲染都是新函数，放进去会让这个
    // effect 每渲染一次就重装一次定时器，五分钟的计时永远归零、一次都不会触发。
    // 本文件里其它几个 effect 也是同一个写法。
  }, [state.path, state.capabilities.vcs])
}

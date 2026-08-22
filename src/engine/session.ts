import { normalizePath, samePath } from './path'

/** 一个仓库标签。id 只在本次运行内有效，不落盘。 */
export interface RepoTabInfo {
  id: string
  path: string
}

/** 最近打开过的一个仓库。`exists` 由后端探测，用来把已失效的条目置灰。 */
export interface RecentRepo {
  path: string
  exists: boolean
}

/** `load_session` 的返回形状。tabs 已由后端滤掉打不开的仓库。 */
export interface SessionView {
  tabs: string[]
  activeTab: string | null
  recent: RecentRepo[]
  /** 存档里有、但这次打不开的标签。见 Rust 侧 `SessionView::dropped_tabs`。 */
  droppedTabs: string[]
}

export interface RestoredTabs {
  tabs: RepoTabInfo[]
  activeId: string | null
  /** 下一个可用的标签 id 序号，交给 App 继续自增。 */
  nextId: number
  /**
   * 这次还原是不完整的：有标签打不开被滤掉了，或者用户抢在还原之前先开了仓库。
   *
   * 为真时**不要**把还原结果立刻写回存档。仓库在移动硬盘/网络盘上、启动时盘没挂上，
   * 标签会被全部滤掉，紧接着的那次自动保存会把标签集永久截断成空——
   * 盘接回来重启也找不回来了。等用户真的动了标签再存。
   */
  partial: boolean
}

/**
 * 把后端还原出来的路径列表变成标签。
 *
 * 后端已经滤掉打不开的仓库，这里只负责发 id 和选活动标签。
 * `activeTab` 指向的路径可能正好是被滤掉的那个，此时回落到第一个标签；
 * 一个标签都没有就是空态，activeId 为 null。
 *
 * `alreadyOpen` 是还原结果送达时用户已经开着的标签数。`load_session` 只是读一个
 * 本地小文件，但它是异步的，而「Open Repo」按钮从第一帧就能点——真撞上时
 * 用还原结果覆盖过去会让用户刚选的仓库凭空消失，所以由调用方据此决定要不要应用。
 */
export function restoreTabs(view: SessionView, alreadyOpen = 0): RestoredTabs {
  const tabs = view.tabs.map((path, i) => ({ id: `tab-${i}`, path }))
  const active = view.activeTab
    ? tabs.find((t) => samePath(t.path, view.activeTab as string))
    : undefined
  return {
    tabs,
    activeId: active?.id ?? tabs[0]?.id ?? null,
    nextId: tabs.length,
    partial: view.droppedTabs.length > 0 || alreadyOpen > 0,
  }
}

/**
 * 关掉一个标签后的标签列表与活动标签。
 *
 * 关掉的不是活动标签时活动标签不动；关掉的是活动标签就接管右边那个，
 * 最右一个被关掉时回落到左边。全部关完返回 null——**允许 0 个标签**，
 * 由调用方渲染空态页。
 */
export function closeTabAt(
  tabs: RepoTabInfo[],
  activeId: string | null,
  id: string,
): { tabs: RepoTabInfo[]; activeId: string | null } {
  const idx = tabs.findIndex((t) => t.id === id)
  if (idx < 0) return { tabs, activeId }
  const next = tabs.filter((t) => t.id !== id)
  if (activeId !== id) return { tabs: next, activeId }
  return { tabs: next, activeId: next[Math.min(idx, next.length - 1)]?.id ?? null }
}

/**
 * 关掉所有指向某个路径的标签，用于那个目录已经不存在了的时候。
 *
 * 删掉一个 worktree 之后，开着它的标签必须跟着关：留着的话文件监听挂在不存在的
 * 目录上、每次刷新都报错，而且那个死路径会被 `save_session` 原样写回存档——
 * 下次启动它进 `droppedTabs`，`partial` 就一直误判成「本次还原不完整」。
 *
 * 只在**明确的删除动作**之后调用，不要接到「路径探测不到」上：网络盘和移动硬盘
 * 临时离线是常事，那种情况下关掉标签等于把用户的会话悄悄删了。
 *
 * 判空要**归一化之后**再判：`normalizePath('')` 和 `normalizePath('/')` 都是空串，
 * 只挡住 `''` 的话，`'/'`（或 `'\'`）照样能进来，把所有归一化后为空的标签一起关掉。
 * 本项目已经在空 pathspec 上出过误伤事故。
 *
 * 同一路径正常只会有一个标签，这里仍然全部关掉——去重和这里用的是同一个
 * `samePath`，真漂移了也不该留下关不掉的死标签。活动标签的接管规则复用
 * [`closeTabAt`]，两条关闭路径的手感保持一致。
 */
export function closeTabsByPath(
  tabs: RepoTabInfo[],
  activeId: string | null,
  path: string,
): { tabs: RepoTabInfo[]; activeId: string | null } {
  if (!normalizePath(path)) return { tabs, activeId }
  const doomed = tabs.filter((t) => samePath(t.path, path))
  if (doomed.length === 0) return { tabs, activeId }
  return doomed.reduce(
    (acc, t) => closeTabAt(acc.tabs, acc.activeId, t.id),
    { tabs, activeId },
  )
}

/**
 * 标签右键菜单的「Close All」：全部关掉，落到空态页。
 */
export function closeAllTabs(
  tabs: RepoTabInfo[],
  activeId: string | null,
): { tabs: RepoTabInfo[]; activeId: string | null } {
  if (tabs.length === 0) return { tabs, activeId }
  return { tabs: [], activeId: null }
}

/**
 * 标签右键菜单的「Close All but This」：只留被右键的那个。
 *
 * 留下的标签接管为活动标签——哪怕之前它不是活动的：此刻只剩它一个，
 * 活动标签还指着别处的话界面就是一块空白。无标签可关时返回原引用，
 * 调用方可以靠引用比较跳过 setState（与 `closeTabAt` 的约定一致）。
 */
export function closeOtherTabs(
  tabs: RepoTabInfo[],
  activeId: string | null,
  id: string,
): { tabs: RepoTabInfo[]; activeId: string | null } {
  const kept = tabs.find((t) => t.id === id)
  if (!kept || tabs.length === 1) return { tabs, activeId }
  return { tabs: [kept], activeId: kept.id }
}

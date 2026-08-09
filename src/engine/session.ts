import { samePath } from './path'

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

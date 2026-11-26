import { useMemo, useRef, useState, useCallback, useEffect } from 'react'
import type { CommitNode, PullRequest, RefPointer } from '../engine/types'
import { groupPullRequestsByHead, indexPullRequestsByBranch } from '../engine/prs'
import { computeLayout } from '../engine/layout'
import { computePaths, filterPathsToBand, laneColor } from '../engine/paths'
import { createSvgRenderer } from '../engine/svgRenderer'
import { WORKDIR_ID, withWorkdirRow } from '../engine/workdir'
import { findDelivered } from '../engine/svnGraph'
import { CommitRow } from './CommitRow'
import { ContextMenu, useContextMenu } from './ContextMenu'
import { anyDialogOpen, isShown } from './useFocusTrap'

/** 按键落在可编辑的控件上：方向键归它挪光标。 */
function isEditable(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false
  return target.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName)
}
import { commitMenu, type CommitAction } from '../engine/commitActions'
import type { Capabilities } from '../engine/capabilities'

const ROW_HEIGHT = 28
const LANE_WIDTH = 20
const OVERSCAN = 5
/** 浏览器拿来滚动容器的键（↑ / ↓ 归键盘导航换选中，不在这里）。 */
const SCROLL_KEYS = new Set(['PageUp', 'PageDown', 'Home', 'End', ' '])

interface GraphViewProps {
  commits: CommitNode[]
  refs: RefPointer[]
  /** 当前仓库的 open PR，用来在分支标签上带出编号。读不到时是空数组。 */
  pullRequests: PullRequest[]
  selectedId: string | null
  /**
   * 定位请求计数。侧边栏单击分支 / tag / worktree 时加一，表示「把选中的提交滚进视口」。
   *
   * 只靠 `selectedId` 变化触发定位不够：目标若本来就是选中的提交（打开仓库时默认选中 HEAD），
   * 用户滚走之后再点同一个分支，`selectedId` 不变，图就停在原地。
   */
  revealSeq: number
  onSelect: (id: string) => void
  /** 当前 HEAD 的完整 oid（不是分支名），用来挂工作区伪行。 */
  headHash: string
  /** 未提交改动数，显示在工作区伪行上。 */
  changeCount: number
  /** 点击工作区伪行时切到 Local Changes 视图。 */
  onOpenChanges: () => void
  /** 右键菜单选中某一项。确认框、输入框和实际调用都由上层处理。 */
  onCommitAction: (action: CommitAction, commit: CommitNode) => void
  /** 后端能力。不支持的菜单项整条不出现，由 `commitMenu` 裁剪。 */
  capabilities: Capabilities
  /**
   * 往下多拉一页 SVN 历史（spec §10.2：首屏 200 条 + 「加载更多」按钮，
   * **不做无限滚动**）。不是 SVN 仓库时传 `undefined`，按钮整个不渲染。
   */
  onLoadMoreSvn?: () => void
  /** 已经拉到 r1 了，按钮禁用。 */
  svnLogComplete?: boolean
}

const renderer = createSvgRenderer()

export function GraphView({
  commits, refs, pullRequests, selectedId, revealSeq, onSelect, headHash, changeCount, onOpenChanges, onCommitAction,
  capabilities, onLoadMoreSvn, svnLogComplete,
}: GraphViewProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const { menu, open, close } = useContextMenu()
  const [scrollTop, setScrollTop] = useState(0)
  const [viewportHeight, setViewportHeight] = useState(600)

  // 索引建一次，供视口内每一行的每个标签查。虚拟滚动下几十行同时在场，
  // 每行都遍历一遍 PR 列表是白费力气
  const prByBranch = useMemo(() => indexPullRequestsByBranch(pullRequests), [pullRequests])
  const prsByHead = useMemo(() => groupPullRequestsByHead(pullRequests), [pullRequests])

  // 布局与路径对全量历史只算一次：父 commit 可能在视口外，线段必须完整生成，
  // 渲染阶段才按条带过滤（见下方 svg memo）。颜色按 lane 分配，与视口无关。
  const layout = useMemo(() => {
    const withWorkdir = withWorkdirRow(commits, headHash || null)
    const rows = computeLayout(withWorkdir, refs)
    const paths = computePaths(rows, ROW_HEIGHT, LANE_WIDTH)
    return {
      rows,
      paths,
      laneCount: Math.max(1, ...rows.map((r) => r.lane)) + 1,
      rowHeight: ROW_HEIGHT,
      laneWidth: LANE_WIDTH,
      offsetY: 0,
    }
  }, [commits, refs, headHash])

  const startIndex = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN)
  const endIndex = Math.min(
    layout.rows.length - 1,
    Math.floor((scrollTop + viewportHeight) / ROW_HEIGHT) + OVERSCAN,
  )
  const visibleRows = layout.rows.slice(startIndex, endIndex + 1)

  // SVG 只画条带内的线段与节点：元素高度 = 条带高度，容器放在条带顶端，
  // 内容整体上移 offsetY。跨边界的线段保留下来，超出的部分由 SVG 边界裁掉，
  // 视觉上是「延伸到边缘」而不是断头。
  const bandTop = startIndex * ROW_HEIGHT
  const svg = useMemo(() => {
    const band = {
      ...layout,
      rows: visibleRows,
      paths: filterPathsToBand(layout.paths, bandTop, (endIndex + 1) * ROW_HEIGHT),
      offsetY: bandTop,
    }
    return renderer.render(band, refs, selectedId)
  }, [layout, visibleRows, bandTop, endIndex, refs, selectedId])

  // 标签用 display:none 隐藏时浏览器会把 scrollTop 清零，
  // 重新显示后要按记住的值还原，所以这里留一份 ref 供观察者读取。
  const scrollTopRef = useRef(0)
  /** 隐藏的标签重新可见时加一，让定位 effect 补跑一次隐藏期间没跑成的定位。 */
  const [shownSeq, setShownSeq] = useState(0)
  const hiddenRef = useRef(false)

  // 挂载后立即量一次真实高度，并跟随面板拖动 / 标签显隐更新——
  // 否则首次滚动前一直按初始值估算可见行数。
  useEffect(() => {
    const el = containerRef.current
    if (!el) return
    const update = () => {
      const h = el.clientHeight
      if (h === 0) {
        hiddenRef.current = true // 标签被隐藏，别把高度记成 0
        return
      }
      setViewportHeight(h)
      if (el.scrollTop !== scrollTopRef.current) el.scrollTop = scrollTopRef.current
      if (hiddenRef.current) {
        hiddenRef.current = false
        setShownSeq((n) => n + 1)
      }
    }
    update()
    const observer = new ResizeObserver(update)
    observer.observe(el)
    return () => observer.disconnect()
  }, [])

  // 选中的提交不在可见范围内时把它滚到中间；已经可见就不动，避免点击行时画面乱跳。
  //
  // `revealSeq` 在依赖里：选中没变、但侧边栏又点了一次同一个目标时也要重新定位。
  // 选中的是收进 svn 节点、不在图上的本地提交时，定位到收着它的那个节点。
  //
  // 首次打开仓库时 store 会把 selectedId 预置成该 worktree 的 HEAD，于是这里顺带
  // 完成「打开 worktree 就定位到它所在的提交」。
  //
  // clientHeight 为 0（标签被 display:none 隐藏）时没法算位置，先放着；标签重新可见时
  // ResizeObserver 推进 `shownSeq`，在这里补跑。
  //
  // **跟随**：「选中 + revealSeq」变了算一次新的定位请求，从这时起跟随选中的那一行，
  // 直到用户自己动了这张图为止。跟随期间 commits / headHash / 视口高度变了要重新检查——
  // SVN 历史后加载，目标可能那时才出现，或者被收进别处的 svn 节点、被新插进来的行挤出视口。
  // 用户一动就不再跟随：文件变动触发的 refreshGraph 每次都换一个内容相同的新 commits，
  // 跟随不停的话会把只滚动、没单击的用户拽回选中的那一行（2026-11-26 使用报告：
  // 被拽回 HEAD，也就是图的顶部）。
  //
  // 「用户动了」按输入事件认（`stopFollowing`），不按 scroll 事件里的 scrollTop 认：
  // 程序赋值也发 scroll 事件，而且事件晚一帧才到、浏览器还会因内容变短而夹紧 scrollTop、
  // 高 DPI 下读回的值带小数，拿数值比哪一种都会认错（独立 review 第二轮第 1–3 条）。
  const requestRef = useRef<{ selectedId: string; revealSeq: number } | null>(null)
  const followRef = useRef(false)
  useEffect(() => {
    const el = containerRef.current
    if (!el) return
    if (!selectedId) {
      // 清空过选中，之后再选回同一条也算新请求
      requestRef.current = null
      return
    }
    const req = requestRef.current
    if (!req || req.selectedId !== selectedId || req.revealSeq !== revealSeq) {
      requestRef.current = { selectedId, revealSeq }
      followRef.current = true
    }
    if (!followRef.current || el.clientHeight === 0) return
    const targetId = findDelivered(commits, selectedId)?.owner.id ?? selectedId
    const idx = commits.findIndex((c) => c.id === targetId)
    // 目标还不在图上：继续跟随，等 commits 变了再试
    if (idx < 0) return
    // 插了工作区伪行后真实提交整体下移一行
    const rowOffset = headHash ? 1 : 0
    const rowTop = (idx + rowOffset) * ROW_HEIGHT
    if (rowTop >= el.scrollTop && rowTop + ROW_HEIGHT <= el.scrollTop + el.clientHeight) return
    el.scrollTop = Math.max(0, rowTop - el.clientHeight / 2 + ROW_HEIGHT / 2)
    // scroll 事件要晚一帧才到；这之前 ResizeObserver 先跑的话会按旧值把这次定位滚回去
    scrollTopRef.current = el.scrollTop
  }, [selectedId, revealSeq, commits, headHash, shownSeq, viewportHeight])

  // 滚轮、在图上按下鼠标（含拖滚动条）算用户接管。单击别的行或 ↑ / ↓ 换了选中，
  // 会紧接着产生新的定位请求、重新开始跟随，不受影响。
  // 「加载更多 SVN 历史」除外：点它正是为了等目标提交加载出来，之后还得跟过去。
  const stopFollowing = useCallback((e: React.SyntheticEvent) => {
    if ((e.target as HTMLElement).closest?.('.graph-load-more')) return
    followRef.current = false
  }, [])

  // 键盘滚动也算用户接管。graph 不可聚焦，单击一行之后按 PageDown / End / 空格，
  // 浏览器滚的是刚点过的这个容器，但 keydown 的目标是 body，挂在容器上收不到。
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (!SCROLL_KEYS.has(e.key) || !isShown(containerRef.current) || isEditable(e.target)) return
      followRef.current = false
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [])

  const handleScroll = useCallback((e: React.UIEvent<HTMLDivElement>) => {
    // 标签被隐藏时浏览器清零 scrollTop 发来的事件：记下来的话重新显示就回不到原位
    if (e.currentTarget.clientHeight === 0) return
    scrollTopRef.current = e.currentTarget.scrollTop
    setScrollTop(e.currentTarget.scrollTop)
    setViewportHeight(e.currentTarget.clientHeight)
  }, [])

  const handleClick = useCallback((e: React.MouseEvent) => {
    const target = e.target as HTMLElement
    const id = target.closest('[data-id]')?.getAttribute('data-id')
    if (!id) return
    // 工作区伪行不是提交，选不了；点它等于进 Local Changes
    if (id === WORKDIR_ID) onOpenChanges()
    else onSelect(id)
  }, [onSelect, onOpenChanges])

  const handleContextMenu = useCallback((e: React.MouseEvent) => {
    const target = e.target as HTMLElement
    const id = target.closest('[data-id]')?.getAttribute('data-id')
    // 工作区伪行不是提交，这些操作对它一个都不成立
    if (!id || id === WORKDIR_ID) return
    const commit = commits.find((c) => c.id === id)
    if (!commit) return
    // 右键先把这一行选上，菜单里的「哪一条」和 diff 面板显示的才是同一条
    onSelect(id)
    open(e, commitMenu(commit, { isHead: id === headHash, caps: capabilities }).map((entry) => ({
      label: entry.label,
      disabled: entry.disabled,
      onSelect: () => onCommitAction(entry.action, commit),
    })))
  }, [commits, headHash, onSelect, onCommitAction, open, capabilities])

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (!selectedId) return
      // 让路的几种情况，选中都不能跟着按键变：
      // - 弹框或右键菜单开着：它们说的是右键 / 打开时的那一条，选中跑了，菜单的 Reset 就对着别的提交
      // - 这个 graph 所在的标签被藏起来了（所有标签都挂载着，按键会同时到每个 graph）
      // - 焦点在输入框里：↑ / ↓ 是在挪光标
      if (menu || anyDialogOpen() || !isShown(containerRef.current) || isEditable(e.target)) return
      // commits 不含伪行，键盘导航天然跳过它
      const idx = commits.findIndex((c) => c.id === selectedId)
      if (idx < 0) return
      if (e.key === 'ArrowUp' && idx > 0) onSelect(commits[idx - 1].id)
      if (e.key === 'ArrowDown' && idx < commits.length - 1) onSelect(commits[idx + 1].id)
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [commits, selectedId, onSelect, menu])

  const totalHeight = layout.rows.length * ROW_HEIGHT
  const layoutWidth = layout.laneCount * LANE_WIDTH
  const refByHash = useMemo(() => {
    const map: Record<string, RefPointer[]> = {}
    for (const ref of refs) {
      ;(map[ref.hash] ??= []).push(ref)
    }
    return map
  }, [refs])

  return (
    <div
      ref={containerRef}
      className="graph-view"
      onScroll={handleScroll}
      onWheel={stopFollowing}
      onPointerDown={stopFollowing}
      onClick={handleClick}
      onContextMenu={handleContextMenu}
      style={{ overflow: 'auto', height: '100%' }}
    >
      <div style={{ position: 'relative', height: totalHeight, minWidth: layoutWidth + 300 }}>
        <div style={{ position: 'absolute', top: bandTop, left: 0, zIndex: 1 }}>
          {svg}
        </div>
        <div style={{ position: 'absolute', top: bandTop, left: 0, right: 0, zIndex: 2 }}>
          {visibleRows.map((node) => (
            <CommitRow
              key={node.commit.id}
              node={node}
              refs={refByHash[node.commit.id] ?? []}
              prByBranch={prByBranch}
              prsByHead={prsByHead}
              selected={selectedId === node.commit.id}
              layoutWidth={layoutWidth}
              laneColor={laneColor(node.lane)}
              changeCount={changeCount}
              isHead={node.commit.id === headHash}
            />
          ))}
        </div>
      </div>
      {onLoadMoreSvn && (
        <div className="graph-load-more">
          <button
            disabled={svnLogComplete}
            onClick={onLoadMoreSvn}
            title={
              svnLogComplete
                ? '已经拉到 r1，没有更早的历史了'
                : 'SVN 历史每次都走网络，所以按需加载（spec §10.1）'
            }
          >
            {svnLogComplete ? '已到最早的 revision' : '加载更多 SVN 历史'}
          </button>
        </div>
      )}
      <ContextMenu menu={menu} onClose={close} />
    </div>
  )
}

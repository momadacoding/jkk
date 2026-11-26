import { useMemo, useRef, useState, useCallback, useEffect } from 'react'
import type { CommitNode, PullRequest, RefPointer } from '../engine/types'
import { groupPullRequestsByHead, indexPullRequestsByBranch } from '../engine/prs'
import { computeLayout } from '../engine/layout'
import { computePaths, filterPathsToBand, laneColor } from '../engine/paths'
import { createSvgRenderer } from '../engine/svgRenderer'
import { WORKDIR_ID, withWorkdirRow } from '../engine/workdir'
import { findDelivered } from '../engine/svnGraph'
import {
  buildSearchIndex, findMatches, inputSearchKey, matchPosition, parseSearch, stepMatch, windowSearchKey,
} from '../engine/commitSearch'
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
/** 输入停下这么久才跳到命中：每跳一次都会去读那条提交的 diff，逐键跳是白读。 */
const SEARCH_JUMP_DELAY = 150

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
  /**
   * 搜索框里的原文。由上层保管：切到 Local Changes / 文件历史时 GraphView 会被卸载，
   * 放在组件里的话回来就没了（独立 review 第 3 条）。
   */
  searchQuery: string
  onSearchQueryChange: (query: string) => void
}

const renderer = createSvgRenderer()

export function GraphView({
  commits, refs, pullRequests, selectedId, revealSeq, onSelect, headHash, changeCount, onOpenChanges, onCommitAction,
  capabilities, onLoadMoreSvn, svnLogComplete, searchQuery, onSearchQueryChange,
}: GraphViewProps) {
  const containerRef = useRef<HTMLDivElement>(null)
  const { menu, open, close } = useContextMenu()
  const [scrollTop, setScrollTop] = useState(0)
  const [viewportHeight, setViewportHeight] = useState(600)
  const searchInputRef = useRef<HTMLInputElement>(null)
  // 跳到的命中就是当前选中的那条时 selectedId 不变，定位 effect 不重跑；
  // 用户滚走之后再按回车也得把它滚回来，和侧边栏的 `revealSeq` 同理
  const [searchRevealSeq, setSearchRevealSeq] = useState(0)

  // 按词而不是按原文记：只多打了空格、只改了大小写时词没变，下面的自动跳转不该再跑一遍、
  // 把用户滚走的画面拽回去（独立 review 第 4 条）。词里不含空白，用换行拼接不会混淆
  const termsKey = parseSearch(searchQuery).join('\n')
  const terms = useMemo(() => (termsKey ? termsKey.split('\n') : []), [termsKey])
  const searching = terms.length > 0
  // 索引只在搜着的时候建：没人搜的话每次刷新图都白建一遍 5000 条
  const searchIndex = useMemo(
    () => (searching ? buildSearchIndex(commits, refs) : []),
    [searching, commits, refs],
  )
  const matches = useMemo(() => findMatches(searchIndex, terms), [searchIndex, terms])
  const matchIds = useMemo(() => new Set(matches.map((i) => commits[i].id)), [matches, commits])
  // 选中的可能是收进 svn 节点、不在图上的本地提交：按收着它的节点算位置
  const selectedRow = useMemo(() => {
    if (!selectedId) return -1
    const targetId = findDelivered(commits, selectedId)?.owner.id ?? selectedId
    return commits.findIndex((c) => c.id === targetId)
  }, [commits, selectedId])

  const jumpToMatch = useCallback((dir: 1 | -1, inclusive: boolean) => {
    const idx = stepMatch(matches, selectedRow, dir, inclusive)
    if (idx === null) return
    // 换了选中，定位 effect 会跟着 selectedId 跑。已经选着它就不再 onSelect（重新选同一条
    // 会白读一遍 diff），只发一次定位请求。两样都做的话，store 晚一拍更新 selectedId 时
    // 定位会先对着旧选中跑一遍，画面先跳到旧的那条再跳过来
    if (idx !== selectedRow) onSelect(commits[idx].id)
    else setSearchRevealSeq((n) => n + 1)
  }, [matches, selectedRow, onSelect, commits])

  // 输入停下后跳到当前选中处及之后的第一个命中。只跟着输入走：图刷新、选中变了都不该
  // 再把选中拽回命中上，所以函数从 ref 里取最新的。
  //
  // 挂载时不跳：搜索词由 RepoTab 保管，切到 Local Changes 再回来 GraphView 会重新挂载，
  // 这时词没变，用户在别处选的提交不该被拽走。用「上次跳转时的词」判断而不是「是不是首次运行」：
  // StrictMode 下挂载时 effect 会跑两遍，后者第二遍就会跳
  const jumpRef = useRef(jumpToMatch)
  jumpRef.current = jumpToMatch
  const lastJumpKey = useRef(termsKey)
  useEffect(() => {
    if (termsKey === lastJumpKey.current) return
    lastJumpKey.current = termsKey
    if (!termsKey) return
    const timer = setTimeout(() => jumpRef.current(1, true), SEARCH_JUMP_DELAY)
    return () => clearTimeout(timer)
  }, [termsKey])

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

  // 挂载后立即量一次真实高度，并跟随面板拖动 / 标签显隐更新——
  // 否则首次滚动前一直按初始值估算可见行数。
  useEffect(() => {
    const el = containerRef.current
    if (!el) return
    const update = () => {
      const h = el.clientHeight
      if (h === 0) return // 标签被隐藏，别把高度记成 0
      setViewportHeight(h)
      if (el.scrollTop !== scrollTopRef.current) el.scrollTop = scrollTopRef.current
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
  // 注意 clientHeight 为 0（标签被 display:none 隐藏）时这里直接放弃，且依赖数组里
  // 没有高度——也就是说**只有在标签可见时才会定位**。目前成立是因为 `openInNewTab`
  // 总会把新标签设为激活；之后若加「在后台打开标签」，新标签的定位会静默失效，
  // 届时需要让 ResizeObserver 在拿到真实高度后补一次定位。
  useEffect(() => {
    const el = containerRef.current
    if (!el || !selectedId || el.clientHeight === 0) return
    const targetId = findDelivered(commits, selectedId)?.owner.id ?? selectedId
    const idx = commits.findIndex((c) => c.id === targetId)
    if (idx < 0) return
    // 插了工作区伪行后真实提交整体下移一行
    const rowOffset = headHash ? 1 : 0
    const rowTop = (idx + rowOffset) * ROW_HEIGHT
    if (rowTop >= el.scrollTop && rowTop + ROW_HEIGHT <= el.scrollTop + el.clientHeight) return
    el.scrollTop = Math.max(0, rowTop - el.clientHeight / 2 + ROW_HEIGHT / 2)
  }, [selectedId, revealSeq, searchRevealSeq, commits, headHash])

  const handleScroll = useCallback((e: React.UIEvent<HTMLDivElement>) => {
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

  // Ctrl+F 聚焦搜索框，F3 / Shift+F3 在命中间跳。所有标签都挂载着，只认看得见的这个 graph；
  // 弹框、右键菜单开着时让路，理由同上面的方向键
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (menu || anyDialogOpen() || !isShown(containerRef.current)) return
      const action = windowSearchKey(e, searching)
      if (action === 'focus') {
        // 焦点在别的输入框（侧边栏新建分支之类）里时不抢：人家正在打字（独立 review 第 5 条）。
        // 已经在搜索框里时照常处理，再按一次是全选、方便重输
        if (isEditable(e.target) && e.target !== searchInputRef.current) return
        e.preventDefault()
        searchInputRef.current?.focus()
        searchInputRef.current?.select()
      } else if (action) {
        e.preventDefault()
        jumpToMatch(action === 'prev' ? -1 : 1, false)
      }
    }
    window.addEventListener('keydown', handler)
    return () => window.removeEventListener('keydown', handler)
  }, [menu, searching, jumpToMatch])

  const onSearchKeyDown = (e: React.KeyboardEvent<HTMLInputElement>) => {
    // isComposing 只在 nativeEvent 上，React 的合成事件没有透出来（同 LocalChangesView）
    const action = inputSearchKey({
      key: e.key,
      ctrlKey: e.ctrlKey,
      metaKey: e.metaKey,
      shiftKey: e.shiftKey,
      altKey: e.altKey,
      repeat: e.repeat,
      isComposing: e.nativeEvent.isComposing,
    })
    if (action === 'next' || action === 'prev') {
      e.preventDefault()
      jumpToMatch(action === 'prev' ? -1 : 1, false)
    } else if (action === 'escape') {
      // 第一下清空，第二下把焦点还给图，方向键才回到换选中
      if (searchQuery) onSearchQueryChange('')
      else e.currentTarget.blur()
    }
  }

  const position = matchPosition(matches, selectedRow)

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
    <div className="graph-pane">
      {/* 常驻而不是按 Ctrl+F 才弹出：没人知道有快捷键的话，这个功能等于不存在 */}
      <div className="graph-search" role="search">
        <input
          ref={searchInputRef}
          className="graph-search-input"
          value={searchQuery}
          onChange={(e) => onSearchQueryChange(e.target.value)}
          onKeyDown={onSearchKeyDown}
          placeholder="搜索提交：标题、作者、hash、分支名（Ctrl+F）"
          title={
            `只搜图上已加载的 ${commits.length} 条提交，且只看标题，正文搜不到。\n`
            + '多个词用空格隔开，要全部命中同一条。hash 至少输 4 位。\n'
            + 'Enter / F3 下一个，Shift+Enter / Shift+F3 上一个，Esc 清空。'
          }
          aria-label="搜索提交"
          spellCheck={false}
        />
        {searching && (
          <>
            <span
              className={matches.length === 0 ? 'graph-search-count none' : 'graph-search-count'}
              aria-live="polite"
            >
              {matches.length === 0 ? '无匹配' : `${position || '-'} / ${matches.length}`}
            </span>
            <button
              className="graph-search-btn"
              disabled={matches.length === 0}
              onClick={() => jumpToMatch(-1, false)}
              title="上一个（Shift+Enter）"
              aria-label="上一个"
            >
              ↑
            </button>
            <button
              className="graph-search-btn"
              disabled={matches.length === 0}
              onClick={() => jumpToMatch(1, false)}
              title="下一个（Enter）"
              aria-label="下一个"
            >
              ↓
            </button>
            <button
              className="graph-search-btn"
              onClick={() => { onSearchQueryChange(''); searchInputRef.current?.focus() }}
              title="清空（Esc）"
              aria-label="清空搜索"
            >
              ✕
            </button>
          </>
        )}
      </div>
      <div
        ref={containerRef}
        className="graph-view"
        onScroll={handleScroll}
        onClick={handleClick}
        onContextMenu={handleContextMenu}
        style={{ overflow: 'auto', flex: 1, minHeight: 0 }}
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
                // 工作区伪行不是提交，搜不到也不变暗
                searchHit={searching && node.commit.id !== WORKDIR_ID ? matchIds.has(node.commit.id) : undefined}
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
    </div>
  )
}

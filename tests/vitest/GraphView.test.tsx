// @vitest-environment jsdom
//
// 侧边栏单击分支 / tag / worktree 要把 graph 定位到目标提交。
// 回归：目标就是当前选中的提交（打开仓库时默认选中 HEAD）时，滚走之后再点同一个分支，
// selectedId 没变，定位 effect 不重跑，图停在原地（2026-11-01 用户报告，Fork 会重新定位）。
import { act, fireEvent, render } from '@testing-library/react'
import { useRef, useState } from 'react'
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { GraphView } from '@/components/GraphView'
import { useFocusTrap } from '@/components/useFocusTrap'
import { ALL_CAPABILITIES } from '@/engine/capabilities'
import type { CommitNode } from '@/engine/types'

// jsdom 没有 ResizeObserver。记下回调，测试里手动触发，模拟标签从隐藏变为可见。
const resizeCallbacks = new Set<() => void>()
const originalResizeObserver = globalThis.ResizeObserver
globalThis.ResizeObserver = class {
  private readonly cb: () => void
  constructor(cb: () => void) {
    this.cb = cb
  }
  observe() {
    resizeCallbacks.add(this.cb)
  }
  unobserve() {}
  disconnect() {
    resizeCallbacks.delete(this.cb)
  }
} as unknown as typeof ResizeObserver

// jsdom 不做布局，clientHeight 恒为 0，而 GraphView 在高度为 0 时（标签被隐藏）会放弃定位。
// 这里给一个 10 行高的视口；`viewport` 置 0 模拟标签被隐藏。
const ROW_HEIGHT = 28
const VIEWPORT = ROW_HEIGHT * 10
let viewport = VIEWPORT
let original: PropertyDescriptor | undefined
beforeAll(() => {
  original = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'clientHeight')
  Object.defineProperty(HTMLElement.prototype, 'clientHeight', {
    configurable: true,
    get: () => viewport,
  })
})
afterEach(() => {
  viewport = VIEWPORT
})
afterAll(() => {
  if (original) Object.defineProperty(HTMLElement.prototype, 'clientHeight', original)
  globalThis.ResizeObserver = originalResizeObserver
})

/** 一条 100 个提交的线性历史，新的在前。 */
const commits: CommitNode[] = Array.from({ length: 100 }, (_, i) => ({
  id: `c${i}`,
  parents: i < 99 ? [`c${i + 1}`] : [],
  author: 'me',
  time: 1_700_000_000 - i,
  message: `commit ${i}`,
}))

/** 第 idx 行滚到视口中间时的 scrollTop（无工作区伪行）。 */
const centered = (idx: number) => idx * ROW_HEIGHT - VIEWPORT / 2 + ROW_HEIGHT / 2

/**
 * 用户滚动：先有输入事件（滚轮），再有 scroll 事件。GraphView 按输入事件认「用户接管」，
 * 只发 scroll 事件模拟的是程序赋值或浏览器夹紧 scrollTop。
 */
function userScroll(el: HTMLElement, top: number) {
  fireEvent.wheel(el)
  el.scrollTop = top
  fireEvent.scroll(el)
}

function renderGraph(props: {
  commits?: CommitNode[]
  selectedId: string
  revealSeq: number
  /** 非空时图顶多一行工作区伪行（`withWorkdirRow` 只看它），真实提交整体下移一行。 */
  headHash?: string
  changeCount?: number
  onLoadMoreSvn?: () => void
}) {
  const element = (p: typeof props) => (
    <GraphView
      commits={p.commits ?? commits}
      refs={[]}
      pullRequests={[]}
      selectedId={p.selectedId}
      revealSeq={p.revealSeq}
      onSelect={() => {}}
      headHash={p.headHash ?? ''}
      changeCount={p.changeCount ?? 0}
      onOpenChanges={() => {}}
      onCommitAction={() => {}}
      capabilities={ALL_CAPABILITIES}
      onLoadMoreSvn={p.onLoadMoreSvn}
      searchQuery=""
      onSearchQueryChange={() => {}}
    />
  )
  const view = render(element(props))
  const el = view.container.querySelector('.graph-view') as HTMLDivElement
  return { el, rerender: (p: typeof props) => view.rerender(element(p)) }
}

describe('GraphView 定位请求', () => {
  it('选中的提交没变，但又来了一次定位请求：滚走之后也要重新定位', () => {
    const { el, rerender } = renderGraph({ selectedId: 'c50', revealSeq: 0 })
    expect(el.scrollTop).toBe(centered(50))

    // 用户把图滚回顶部，目标行已经不在视口里
    userScroll(el, 0)

    // 再点同一个分支：selectedId 不变，只有定位请求变了
    rerender({ selectedId: 'c50', revealSeq: 1 })
    expect(el.scrollTop).toBe(centered(50))
  })

  it('目标行已经在视口里：定位请求不挪动画面', () => {
    const { el, rerender } = renderGraph({ selectedId: 'c50', revealSeq: 0 })
    const before = el.scrollTop
    rerender({ selectedId: 'c50', revealSeq: 1 })
    expect(el.scrollTop).toBe(before)
  })

  it('选中的是收进 svn 节点、不在图上的本地提交：定位到收着它的那个节点', () => {
    const delivered: CommitNode = {
      id: 'local1',
      parents: [],
      author: 'me',
      time: 1_700_000_000,
      message: 'local work',
    }
    const withSvn = commits.map((c, i) =>
      i === 60 ? { ...c, id: 'svn:60', vcs: 'svn' as const, svnRev: 60, deliveredFrom: [delivered] } : c,
    )
    // 修正前后指针，保持线性
    withSvn[59] = { ...withSvn[59], parents: ['svn:60'] }

    const { el } = renderGraph({ commits: withSvn, selectedId: 'local1', revealSeq: 1 })
    expect(el.scrollTop).toBe(centered(60))
  })
})

// 回归（2026-11-26 使用报告）：工作区有文件写入时 repo-changed 触发 refreshGraph，
// 每次都换一个内容相同的新 commits 数组；定位 effect 依赖 commits，于是把只滚动、
// 没单击的用户拽回选中的 HEAD，也就是图的顶部。
describe('GraphView 后台刷新不抢滚动位置', () => {
  /** 内容相同、引用全新的 commits，模拟 refreshGraph / applySvnLog 的 merge 结果。 */
  const refreshed = () => commits.map((c) => ({ ...c }))

  it('选中 HEAD 后往下滚，后台刷新换了 commits：画面不回到顶部', () => {
    const { el, rerender } = renderGraph({ selectedId: 'c0', revealSeq: 0 })
    expect(el.scrollTop).toBe(0)

    userScroll(el, ROW_HEIGHT * 80)

    rerender({ commits: refreshed(), selectedId: 'c0', revealSeq: 0 })
    expect(el.scrollTop).toBe(ROW_HEIGHT * 80)
  })

  it('已经定位过一次的选中，滚走之后后台刷新：不再拽回', () => {
    const { el, rerender } = renderGraph({ selectedId: 'c50', revealSeq: 0 })
    expect(el.scrollTop).toBe(centered(50))

    userScroll(el, 0)

    rerender({ commits: refreshed(), selectedId: 'c50', revealSeq: 0 })
    expect(el.scrollTop).toBe(0)
  })

  it('目标提交当时还不在图上，后加载出来时仍然定位过去，之后的刷新不再拽回', () => {
    // SVN 历史后加载：选中的提交要等这批数据到了才出现在图上
    const { el, rerender } = renderGraph({
      commits: commits.slice(0, 50),
      selectedId: 'c80',
      revealSeq: 0,
    })
    expect(el.scrollTop).toBe(0)

    rerender({ commits: refreshed(), selectedId: 'c80', revealSeq: 0 })
    expect(el.scrollTop).toBe(centered(80))

    userScroll(el, 0)
    rerender({ commits: refreshed(), selectedId: 'c80', revealSeq: 0 })
    expect(el.scrollTop).toBe(0)
  })

  it('使用报告的真实形态：图顶有工作区伪行（headHash 非空）、选中 HEAD、往下滚，后台刷新不回顶部', () => {
    const shape = { selectedId: 'c0', revealSeq: 0, headHash: 'c0', changeCount: 3 }
    const { el, rerender } = renderGraph(shape)
    expect(el.querySelector('[data-id="c0"]')).not.toBeNull()
    expect(el.scrollTop).toBe(0)

    userScroll(el, ROW_HEIGHT * 80)

    rerender({ ...shape, commits: refreshed(), changeCount: 4 })
    expect(el.scrollTop).toBe(ROW_HEIGHT * 80)
  })
})

// 独立 review 第 1 / 2 / 3 条：用户还没滚过时，选中的那一行因为数据变化挪了位置，
// 要继续把它留在视口里——「后台刷新不抢滚动位置」只针对用户自己滚过之后。
describe('GraphView 用户没滚过时跟随选中的那一行', () => {
  const ids = (n: number, prefix: string): CommitNode[] =>
    Array.from({ length: n }, (_, i) => ({
      id: `${prefix}${i}`,
      parents: [],
      author: 'me',
      time: 1_800_000_000 - i,
      message: `${prefix} ${i}`,
    }))

  it('新的提交插到 HEAD 上面、把它挤出视口：跟过去', () => {
    const { el, rerender } = renderGraph({ selectedId: 'c0', revealSeq: 0 })
    expect(el.scrollTop).toBe(0)

    // 例如 SVN 历史后加载，比 HEAD 新的 revision 排在它上面
    rerender({ commits: [...ids(30, 'n'), ...commits], selectedId: 'c0', revealSeq: 0 })
    expect(el.scrollTop).toBe(centered(30))
  })

  it('选中的本地提交后来被收进 svn 节点：跟到收着它的节点', () => {
    const local: CommitNode = { id: 'local1', parents: [], author: 'me', time: 1_800_000_000, message: 'local' }
    const { el, rerender } = renderGraph({ commits: [local, ...commits], selectedId: 'local1', revealSeq: 0 })
    expect(el.scrollTop).toBe(0)

    const folded = commits.map((c, i) =>
      i === 60 ? { ...c, id: 'svn:60', vcs: 'svn' as const, svnRev: 60, deliveredFrom: [local] } : c,
    )
    rerender({ commits: folded, selectedId: 'local1', revealSeq: 0 })
    expect(el.scrollTop).toBe(centered(60))
  })

  it('标签隐藏时来的定位：重新可见时补上；之后用户滚走，后台刷新不再拽回（独立 review 第 4 条）', () => {
    viewport = 0
    const { el, rerender } = renderGraph({ selectedId: 'c50', revealSeq: 0 })
    expect(el.scrollTop).toBe(0)

    viewport = VIEWPORT
    act(() => resizeCallbacks.forEach((cb) => cb()))
    expect(el.scrollTop).toBe(centered(50))

    userScroll(el, 0)
    rerender({ commits: commits.map((c) => ({ ...c })), selectedId: 'c50', revealSeq: 0 })
    expect(el.scrollTop).toBe(0)
  })

  it('浏览器因内容变短夹紧 scrollTop（只有 scroll 事件、没有输入）不算用户接管（第二轮 review 第 2 条）', () => {
    const { el, rerender } = renderGraph({ selectedId: 'c80', revealSeq: 0 })
    expect(el.scrollTop).toBe(centered(80))

    // 一次刷新暂时只回来 50 条，c80 不在了，浏览器把 scrollTop 夹到新的最大值
    rerender({ commits: commits.slice(0, 50), selectedId: 'c80', revealSeq: 0 })
    el.scrollTop = 50 * ROW_HEIGHT - VIEWPORT
    fireEvent.scroll(el)

    rerender({ commits: commits.map((c) => ({ ...c })), selectedId: 'c80', revealSeq: 0 })
    expect(el.scrollTop).toBe(centered(80))
  })

  it('跟随期间面板被拖矮、选中行掉出视口：重新定位（第二轮 review 第 6 条）', () => {
    const { el } = renderGraph({ selectedId: 'c50', revealSeq: 0 })
    expect(el.scrollTop).toBe(centered(50))

    viewport = ROW_HEIGHT * 2
    act(() => resizeCallbacks.forEach((cb) => cb()))
    expect(el.scrollTop).toBe(50 * ROW_HEIGHT - viewport / 2 + ROW_HEIGHT / 2)
  })

  // jsdom 没有滚动条，这里只验 pointerdown 这条路径；拖滚动条会不会发 pointerdown 在真实窗口里验
  it('在图上按下鼠标（pointerdown）算用户接管', () => {
    const { el, rerender } = renderGraph({ selectedId: 'c50', revealSeq: 0 })
    fireEvent.pointerDown(el)
    el.scrollTop = 0
    fireEvent.scroll(el)
    rerender({ commits: commits.map((c) => ({ ...c })), selectedId: 'c50', revealSeq: 0 })
    expect(el.scrollTop).toBe(0)
  })

  it('单击一行后用 End / PageDown 等键盘滚动：算用户接管（第三轮 review 第 1 条）', () => {
    const { el, rerender } = renderGraph({ selectedId: 'c5', revealSeq: 0 })
    // keydown 的目标是 body：graph 不可聚焦
    fireEvent.keyDown(document.body, { key: 'End' })
    el.scrollTop = 90 * ROW_HEIGHT
    fireEvent.scroll(el)
    rerender({ commits: commits.map((c) => ({ ...c })), selectedId: 'c5', revealSeq: 0 })
    expect(el.scrollTop).toBe(90 * ROW_HEIGHT)
  })

  it('点「加载更多 SVN 历史」不算接管：等的提交加载出来后照样跟过去（第三轮 review 第 2 条）', () => {
    const { el, rerender } = renderGraph({
      commits: commits.slice(0, 50),
      selectedId: 'c80',
      revealSeq: 0,
      onLoadMoreSvn: () => {},
    })
    const button = el.querySelector('.graph-load-more button') as HTMLButtonElement
    fireEvent.pointerDown(button)
    fireEvent.click(button)

    rerender({ commits: commits.map((c) => ({ ...c })), selectedId: 'c80', revealSeq: 0, onLoadMoreSvn: () => {} })
    expect(el.scrollTop).toBe(centered(80))
  })
})

describe('GraphView 键盘导航', () => {
  function Dialog() {
    const ref = useRef<HTMLDivElement>(null)
    useFocusTrap(ref)
    return <div ref={ref} className="dialog"><button>OK</button></div>
  }

  function renderWithSpy(dialog: boolean, opts: { hiddenTab?: boolean; dialogInHiddenTab?: boolean } = {}) {
    const picked: string[] = []
    render(
      <>
        {/* App 把非活动标签整个 display:none 掉，标签里的弹框也跟着藏起来 */}
        {opts.dialogInHiddenTab && <div style={{ display: 'none' }}><Dialog /></div>}
        <div style={{ display: opts.hiddenTab ? 'none' : 'flex' }}>
        <GraphView
          commits={commits}
          refs={[]}
          pullRequests={[]}
          selectedId="c5"
          revealSeq={0}
          onSelect={(id) => picked.push(id)}
          headHash=""
          changeCount={0}
          onOpenChanges={() => {}}
          onCommitAction={() => {}}
          capabilities={ALL_CAPABILITIES}
          searchQuery=""
          onSearchQueryChange={() => {}}
        />
        </div>
        {dialog && <Dialog />}
        <input aria-label="新分支名" />
      </>,
    )
    return picked
  }

  it('↑ / ↓ 换选中的提交', () => {
    const picked = renderWithSpy(false)
    fireEvent.keyDown(window, { key: 'ArrowDown' })
    fireEvent.keyDown(window, { key: 'ArrowUp' })
    expect(picked).toEqual(['c6', 'c4'])
  })

  it('弹框开着时不动背后的选中：弹框说的是哪一条，关掉之后看到的还得是那一条（PR 139 review 第 6 条）', () => {
    const picked = renderWithSpy(true)
    fireEvent.keyDown(window, { key: 'ArrowDown' })
    fireEvent.keyDown(window, { key: 'ArrowUp' })
    expect(picked).toEqual([])
  })

  it('右键菜单开着时不动选中：菜单项捕获的是右键那一条，选中跑了就会对着别的提交 Reset', () => {
    const picked = renderWithSpy(false)
    const row = document.querySelector('[data-id="c5"]')!
    fireEvent.contextMenu(row)
    picked.length = 0 // 右键本身会选中 c5
    fireEvent.keyDown(window, { key: 'ArrowDown' })
    expect(picked).toEqual([])
  })

  it('隐藏标签里的 graph 不跟着当前标签的方向键动', () => {
    const picked = renderWithSpy(false, { hiddenTab: true })
    fireEvent.keyDown(window, { key: 'ArrowDown' })
    expect(picked).toEqual([])
  })

  it('隐藏标签里开着的弹框不挡当前标签的方向键', () => {
    const picked = renderWithSpy(false, { dialogInHiddenTab: true })
    fireEvent.keyDown(window, { key: 'ArrowDown' })
    expect(picked).toEqual(['c6'])
  })

  it('焦点在输入框里：↑ / ↓ 归输入框挪光标，graph 不动', () => {
    const picked = renderWithSpy(false)
    const input = document.querySelector('input[aria-label="新分支名"]') as HTMLInputElement
    input.focus()
    fireEvent.keyDown(input, { key: 'ArrowDown' })
    expect(picked).toEqual([])
  })
})

describe('GraphView 搜索', () => {
  // 标题带 fix 的在第 3、10、50 行；其余是 chore
  const HITS = [3, 10, 50]
  const searchable: CommitNode[] = commits.map((c, i) => ({
    ...c,
    message: HITS.includes(i) ? `fix bug ${i}` : `chore ${i}`,
  }))

  beforeEach(() => { vi.useFakeTimers() })
  afterEach(() => { vi.useRealTimers() })

  function Dialog() {
    const ref = useRef<HTMLDivElement>(null)
    useFocusTrap(ref)
    return <div ref={ref} className="dialog"><button>OK</button></div>
  }

  /**
   * 和 RepoTab 一样：选中和搜索词都由上层保管，onSelect 真的改选中；`picked` 记下每一次 onSelect。
   * `shown = false` 模拟切到 Local Changes 时 GraphView 被卸载。
   */
  function renderSearch(opts: { selectedId?: string; hiddenTab?: boolean; dialog?: boolean } = {}) {
    const picked: string[] = []
    function Harness({ list, shown = true }: { list: CommitNode[]; shown?: boolean }) {
      const [selected, setSelected] = useState(opts.selectedId ?? 'c5')
      const [query, setQuery] = useState('')
      return (
        <>
          <div style={{ display: opts.hiddenTab ? 'none' : 'flex' }}>
            {shown && (
              <GraphView
                commits={list}
                refs={[]}
                pullRequests={[]}
                selectedId={selected}
                revealSeq={0}
                onSelect={(id) => { picked.push(id); setSelected(id) }}
                headHash=""
                changeCount={0}
                onOpenChanges={() => {}}
                onCommitAction={() => {}}
                capabilities={ALL_CAPABILITIES}
                searchQuery={query}
                onSearchQueryChange={setQuery}
              />
            )}
          </div>
          {opts.dialog && <Dialog />}
          <input aria-label="别的输入框" />
        </>
      )
    }
    const view = render(<Harness list={searchable} />)
    const input = view.container.querySelector('.graph-search-input') as HTMLInputElement
    const el = view.container.querySelector('.graph-view') as HTMLDivElement
    const unmountGraph = () => view.rerender(<Harness list={searchable} shown={false} />)
    const remountGraph = () => view.rerender(<Harness list={searchable} />)
    /** 输入并等过防抖。 */
    const type = (value: string) => {
      fireEvent.change(input, { target: { value } })
      act(() => { vi.advanceTimersByTime(200) })
    }
    const count = () => view.container.querySelector('.graph-search-count')?.textContent
    const rerender = (list: CommitNode[]) => view.rerender(<Harness list={list} />)
    return { picked, input, el, type, count, rerender, unmountGraph, remountGraph }
  }

  it('输入停下后跳到选中处之后的第一个命中；停下之前不跳', () => {
    const { picked, input, type } = renderSearch()
    fireEvent.change(input, { target: { value: 'fix' } })
    act(() => { vi.advanceTimersByTime(50) })
    expect(picked).toEqual([])
    type('fix')
    expect(picked).toEqual(['c10'])
  })

  it('多打一个字、当前那条仍然命中：选中不动', () => {
    const { picked, type } = renderSearch()
    type('fix')
    type('fix bug')
    expect(picked).toEqual(['c10'])
  })

  it('Enter 下一个、走到底绕回；Shift+Enter 上一个；计数跟着走', () => {
    const { picked, input, type, count } = renderSearch()
    type('fix')
    expect(count()).toBe('2 / 3')
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(count()).toBe('3 / 3')
    fireEvent.keyDown(input, { key: 'Enter' })
    fireEvent.keyDown(input, { key: 'Enter', shiftKey: true })
    expect(picked).toEqual(['c10', 'c50', 'c3', 'c50'])
  })

  it('F3 / Shift+F3 在焦点不在搜索框时也能跳', () => {
    const { picked, input, type } = renderSearch()
    type('fix')
    input.blur()
    fireEvent.keyDown(window, { key: 'F3' })
    fireEvent.keyDown(window, { key: 'F3', shiftKey: true })
    expect(picked).toEqual(['c10', 'c50', 'c10'])
  })

  it('命中的行高亮、其余变暗；没在搜时两样都没有', () => {
    const { el, type } = renderSearch({ selectedId: 'c0' })
    // SVG 里的节点也带 data-id，要的是文字那一行
    const row = (i: number) => el.querySelector(`.commit-row[data-id="c${i}"]`)!
    expect(row(3).className).not.toMatch(/search-/)
    type('fix')
    expect(row(3).className).toContain('search-hit')
    expect(row(4).className).toContain('search-miss')
  })

  it('没有命中：显示「无匹配」，选中不动', () => {
    const { picked, type, count } = renderSearch()
    type('nothing-like-this')
    expect(count()).toBe('无匹配')
    expect(picked).toEqual([])
  })

  it('唯一的命中就是当前选中、用户滚走了：Enter 把它滚回来，不重新选（不白读 diff）', () => {
    const { picked, input, el, type } = renderSearch({ selectedId: 'c50' })
    type('bug 50')
    // 用滚轮滚走：PR 206 之后滚轮算用户接管、不再跟随选中，Enter 得算一次新的定位请求才滚得回来
    fireEvent.wheel(el)
    el.scrollTop = 0
    fireEvent.scroll(el)
    fireEvent.keyDown(input, { key: 'Enter' })
    expect(picked).toEqual([])
    expect(el.scrollTop).toBe(centered(50))
  })

  it('图刷新不把选中拽回命中上：用户点走之后，搜索词还在也不跳', () => {
    const { picked, el, type, rerender } = renderSearch()
    type('fix')
    fireEvent.click(el.querySelector('[data-id="c12"]')!)
    rerender(searchable.map((c) => ({ ...c })))
    act(() => { vi.advanceTimersByTime(200) })
    expect(picked).toEqual(['c10', 'c12'])
  })

  it('Esc 第一下清空，第二下离开搜索框', () => {
    const { input, type, count } = renderSearch()
    type('fix')
    input.focus()
    fireEvent.keyDown(input, { key: 'Escape' })
    expect(input.value).toBe('')
    expect(count()).toBeUndefined()
    expect(document.activeElement).toBe(input)
    fireEvent.keyDown(input, { key: 'Escape' })
    expect(document.activeElement).not.toBe(input)
  })

  it('Ctrl+F 聚焦搜索框', () => {
    const { input } = renderSearch()
    fireEvent.keyDown(window, { key: 'f', ctrlKey: true })
    expect(document.activeElement).toBe(input)
  })

  it('Ctrl+F：隐藏标签里的 graph 不抢焦点', () => {
    const { input } = renderSearch({ hiddenTab: true })
    fireEvent.keyDown(window, { key: 'f', ctrlKey: true })
    expect(document.activeElement).not.toBe(input)
  })

  it('Ctrl+F：弹框开着时不抢焦点', () => {
    const { input } = renderSearch({ dialog: true })
    fireEvent.keyDown(window, { key: 'f', ctrlKey: true })
    expect(document.activeElement).not.toBe(input)
  })

  it('Ctrl+F：焦点在别的输入框里时不抢（独立 review 第 5 条）', () => {
    const { input } = renderSearch()
    const other = document.querySelector('input[aria-label="别的输入框"]') as HTMLInputElement
    other.focus()
    fireEvent.keyDown(other, { key: 'f', ctrlKey: true })
    expect(document.activeElement).toBe(other)
    expect(input).not.toBe(other)
  })

  it('输入法组词时按 Enter / Esc：不跳、不清空（独立 review 第 1 条）', () => {
    const { picked, input, type } = renderSearch()
    type('fix')
    input.focus()
    fireEvent.keyDown(input, { key: 'Enter', isComposing: true })
    fireEvent.keyDown(input, { key: 'Escape', isComposing: true })
    expect(picked).toEqual(['c10'])
    expect(input.value).toBe('fix')
  })

  it('只多打了空格、或只改了大小写：词没变就不再跳回命中（独立 review 第 4 条）', () => {
    const { el, type } = renderSearch()
    type('fix')
    el.scrollTop = 0
    fireEvent.scroll(el)
    type('fix ')
    type('FIX ')
    expect(el.scrollTop).toBe(0)
  })

  it('搜索词由上层保管：切到 Local Changes 再回来（GraphView 重新挂载）词还在、也不再自动跳（独立 review 第 3 条）', () => {
    const { picked, type, unmountGraph, remountGraph } = renderSearch()
    type('fix')
    unmountGraph()
    remountGraph()
    act(() => { vi.advanceTimersByTime(200) })
    const input = document.querySelector('.graph-search-input') as HTMLInputElement
    expect(input.value).toBe('fix')
    expect(document.querySelector('.graph-search-count')?.textContent).toBe('2 / 3')
    expect(picked).toEqual(['c10'])
  })
})

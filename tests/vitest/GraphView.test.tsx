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

globalThis.ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
}

// jsdom 不做布局，clientHeight 恒为 0，而 GraphView 在高度为 0 时（标签被隐藏）会放弃定位。
// 这里给一个 10 行高的视口。
const ROW_HEIGHT = 28
const VIEWPORT = ROW_HEIGHT * 10
let original: PropertyDescriptor | undefined
beforeAll(() => {
  original = Object.getOwnPropertyDescriptor(HTMLElement.prototype, 'clientHeight')
  Object.defineProperty(HTMLElement.prototype, 'clientHeight', {
    configurable: true,
    get: () => VIEWPORT,
  })
})
afterAll(() => {
  if (original) Object.defineProperty(HTMLElement.prototype, 'clientHeight', original)
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

function renderGraph(props: { commits?: CommitNode[]; selectedId: string; revealSeq: number }) {
  const element = (p: typeof props) => (
    <GraphView
      commits={p.commits ?? commits}
      refs={[]}
      pullRequests={[]}
      selectedId={p.selectedId}
      revealSeq={p.revealSeq}
      onSelect={() => {}}
      headHash=""
      changeCount={0}
      onOpenChanges={() => {}}
      onCommitAction={() => {}}
      capabilities={ALL_CAPABILITIES}
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
    el.scrollTop = 0
    fireEvent.scroll(el)

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

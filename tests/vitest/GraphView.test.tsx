// @vitest-environment jsdom
//
// 侧边栏单击分支 / tag / worktree 要把 graph 定位到目标提交。
// 回归：目标就是当前选中的提交（打开仓库时默认选中 HEAD）时，滚走之后再点同一个分支，
// selectedId 没变，定位 effect 不重跑，图停在原地（2026-11-01 用户报告，Fork 会重新定位）。
import { fireEvent, render } from '@testing-library/react'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'
import { GraphView } from '@/components/GraphView'
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

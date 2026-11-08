// @vitest-environment jsdom
//
// 组件测试只在这个文件里切到 jsdom，其余纯逻辑测试仍跑在 node 环境（vitest.config.ts）。
import { render } from '@testing-library/react'
import { renderToStaticMarkup } from 'react-dom/server'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { ContextMenu, type MenuState } from '@/components/ContextMenu'

const items = [{ label: 'Checkout', onSelect: () => {} }]

function menuEl(container: HTMLElement): HTMLElement {
  const el = container.querySelector<HTMLElement>('.context-menu')
  if (!el) throw new Error('菜单没渲染出来')
  return el
}

afterEach(() => {
  vi.restoreAllMocks()
})

describe('ContextMenu 定位', () => {
  it('第一次渲染就落在鼠标处，不先画在左上角（侧栏分支右键会闪一下）', () => {
    // 服务端渲染不跑任何 effect，拿到的正是浏览器画第一帧时的样子：
    // 位置若要等 effect 量完才改，这里就是 0,0。
    // React 会提醒 useLayoutEffect 在服务端不执行——这正是这里要借的性质，不是问题
    vi.spyOn(console, 'error').mockImplementation(() => {})
    const html = renderToStaticMarkup(
      <ContextMenu menu={{ x: 300, y: 200, items }} onClose={() => {}} />,
    )
    expect(html).toContain('left:300px')
    expect(html).toContain('top:200px')
  })

  it('贴着窗口右下角弹出时收回可视区域', () => {
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue(
      { width: 150, height: 100, x: 0, y: 0, top: 0, left: 0, right: 150, bottom: 100, toJSON: () => ({}) },
    )
    const { container } = render(
      <ContextMenu menu={{ x: window.innerWidth - 10, y: window.innerHeight - 10, items }} onClose={() => {}} />,
    )
    const el = menuEl(container)
    expect(el.style.left).toBe(`${window.innerWidth - 150 - 4}px`)
    expect(el.style.top).toBe(`${window.innerHeight - 100 - 4}px`)
  })

  it('换个地方再右键：用新的位置，不沿用上一次量出来的', () => {
    const { container, rerender } = render(
      <ContextMenu menu={{ x: 100, y: 100, items }} onClose={() => {}} />,
    )
    const next: MenuState = { x: 40, y: 60, items }
    rerender(<ContextMenu menu={next} onClose={() => {}} />)
    const el = menuEl(container)
    expect(el.style.left).toBe('40px')
    expect(el.style.top).toBe('60px')
  })
})

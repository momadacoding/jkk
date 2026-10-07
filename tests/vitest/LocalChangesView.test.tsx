// @vitest-environment jsdom
//
// 组件测试只在这个文件里切到 jsdom，其余纯逻辑测试仍跑在 node 环境（vitest.config.ts）。
import { fireEvent, render, screen } from '@testing-library/react'
import { beforeAll, describe, expect, it, vi } from 'vitest'
import { LocalChangesView } from '@/components/LocalChangesView'
import type { FileChange, StatusData } from '@/engine/types'

// react-resizable-panels 挂载时要 new 一个 ResizeObserver，jsdom 没有。
// 这里不测布局，给个什么都不做的桩就够了。
beforeAll(() => {
  vi.stubGlobal('ResizeObserver', class {
    observe() {}
    unobserve() {}
    disconnect() {}
  })
})

function change(path: string): FileChange {
  return { path, status: 'modified', additions: 1, deletions: 0 }
}

function mount() {
  const onDiscard = vi.fn()
  const status: StatusData = { unstaged: [change('a.txt'), change('b.txt')], staged: [change('s.txt')] }
  render(
    <LocalChangesView
      status={status}
      diff={{ files: [] }}
      onStage={vi.fn()}
      onUnstage={vi.fn()}
      onDiscard={onDiscard}
      onCommit={vi.fn()}
      svnMode={false}
      onBaselineBranch={false}
    />,
  )
  return { onDiscard }
}

/** 文件行所在的可聚焦列表容器，快捷键挂在它上面。 */
function listOf(path: string): HTMLElement {
  const row = screen.getByText(path)
  const list = row.closest('.changes-section-files')
  if (!(list instanceof HTMLElement)) throw new Error(`找不到 ${path} 所在的列表`)
  return list
}

describe('LocalChangesView 的 Delete 快捷键', () => {
  it('选中 Unstaged 里的文件后按 Delete，走和右键菜单一样的 onDiscard', () => {
    const { onDiscard } = mount()
    fireEvent.click(screen.getByText('b.txt'))
    fireEvent.keyDown(listOf('b.txt'), { key: 'Delete' })
    expect(onDiscard).toHaveBeenCalledTimes(1)
    expect(onDiscard).toHaveBeenCalledWith(['b.txt'], false)
  })

  it('Staged 区按 Delete 带 staged=true', () => {
    const { onDiscard } = mount()
    fireEvent.click(screen.getByText('s.txt'))
    fireEvent.keyDown(listOf('s.txt'), { key: 'Delete' })
    expect(onDiscard).toHaveBeenCalledTimes(1)
    expect(onDiscard).toHaveBeenCalledWith(['s.txt'], true)
  })

  it('选中了文件，但在 commit 输入框里按 Delete 删字，不能触发丢弃', () => {
    const { onDiscard } = mount()
    fireEvent.click(screen.getByText('a.txt'))
    const subject = screen.getByPlaceholderText('Commit subject')
    fireEvent.keyDown(subject, { key: 'Delete' })
    fireEvent.keyDown(screen.getByPlaceholderText('Description'), { key: 'Delete' })
    expect(onDiscard).not.toHaveBeenCalled()
  })

  it('焦点在 Unstaged 列表、选中却在 Staged 时不丢弃', () => {
    const { onDiscard } = mount()
    fireEvent.click(screen.getByText('s.txt'))
    fireEvent.keyDown(listOf('a.txt'), { key: 'Delete' })
    expect(onDiscard).not.toHaveBeenCalled()
  })

  it('没选中任何文件时按 Delete 什么都不做', () => {
    const { onDiscard } = mount()
    fireEvent.keyDown(listOf('a.txt'), { key: 'Delete' })
    expect(onDiscard).not.toHaveBeenCalled()
  })
})

// @vitest-environment jsdom
//
// 按钮贴底之后，用户可以在滚动区的任何位置点主按钮；失败时错误要滚进视野，
// 否则看起来像什么都没发生（2026-11-03 review 第 3 条）。
import { act, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { DeleteWorktreeDialog } from '@/components/DeleteWorktreeDialog'
import { useConfirm, type ConfirmRequest } from '@/components/ConfirmDialog'

afterEach(() => {
  vi.restoreAllMocks()
})

describe('弹框里新出现的错误滚进视野', () => {
  it('删 worktree 失败：错误条调用了 scrollIntoView', async () => {
    // jsdom 没有实现 scrollIntoView，挂一个桩记下谁被滚到了
    const seen: Element[] = []
    Element.prototype.scrollIntoView = function (this: Element) {
      seen.push(this)
    }
    render(
      <DeleteWorktreeDialog
        repoPath="C:/r"
        worktree={{ name: 'wt', path: 'C:/r/.worktrees/wt', head: 'h', branch: 'feat/x', isMain: false }}
        onCancel={() => {}}
        onConfirm={async () => {
          throw new Error('目录被占用')
        }}
      />,
    )
    fireEvent.click(screen.getByRole('button', { name: 'Delete' }))
    const error = await screen.findByText(/目录被占用/)
    expect(seen).toContain(error)
  })
})

describe('弹框关掉后焦点回到打开前的位置', () => {
  it('Del 键丢弃的流程：确认框关掉后键盘还在文件列表上', async () => {
    // 不还回去的话焦点落到 body，下一次 Del / Ctrl+A 收不到
    let confirm!: (r: ConfirmRequest) => Promise<boolean>
    function Harness() {
      const c = useConfirm()
      confirm = c.confirm
      return (
        <>
          <button>文件列表</button>
          {c.dialog}
        </>
      )
    }
    render(<Harness />)
    const list = screen.getByRole('button', { name: '文件列表' })
    list.focus()
    let result!: Promise<boolean>
    act(() => {
      result = confirm({ title: '丢弃', message: 'm', confirmLabel: 'Discard', focusCancel: true })
    })
    expect(document.activeElement).not.toBe(list)
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }))
    await act(async () => {
      await result
    })
    expect(document.activeElement).toBe(list)
  })
})

// @vitest-environment jsdom
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import {
  useFollowConflictDialog,
  type FollowConflictRequest,
} from '@/components/FollowConflictDialog'
import { describeFollowConflictModel, type FollowOutcome } from '@/engine/svnFollow'

const PATH = 'LuaScripts/Client/SettingMgr.lua'

function outcome(ready: boolean): Extract<FollowOutcome, { kind: 'needsManual' }> {
  return {
    kind: 'needsManual',
    at: 'c'.repeat(40),
    files: [PATH],
    baselinePublished: true,
    leaks: [],
    staleDisk: false,
    base: 'b'.repeat(40),
    diskReady: ready,
    details: [
      {
        path: PATH,
        svnRev: 70760,
        svnAuthor: 'li',
        svnRemoved: false,
        preview: '<<<<<<< 新基线（SVN）\nnew\n=======\nmine\n>>>>>>> 你的提交',
        disk: ready ? { kind: 'usable' } : { kind: 'refused', reason: '磁盘上还有冲突标记' },
      },
    ],
  }
}

function open(ready: boolean, over: Partial<FollowConflictRequest> = {}) {
  const run = vi.fn(async () => {})
  const opened = vi.fn(async () => {})
  let show!: (x: FollowConflictRequest) => Promise<void>
  function Harness() {
    const { showFollowConflict, dialog } = useFollowConflictDialog()
    show = showFollowConflict
    return <>{dialog}</>
  }
  render(<Harness />)
  let closed!: Promise<void>
  act(() => {
    closed = show({
      model: describeFollowConflictModel(outcome(ready)),
      compare: () => ({ label: 'Beyond Compare', open: opened }),
      run,
      ...over,
    })
  })
  return { run, opened, closed: () => closed }
}

describe('FollowConflictDialog', () => {
  it('能用磁盘内容：三个按钮，默认焦点在磁盘按钮上，没有红色警告', async () => {
    open(true)
    const disk = await screen.findByRole('button', { name: '用磁盘上的内容跟进' })
    expect(screen.getByRole('button', { name: '手动跟进' })).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Cancel' })).toBeTruthy()
    expect(document.activeElement).toBe(disk)
    expect(screen.getByText('✓ 你已经在 SVN 里合并好了，磁盘上就是合并结果。')).toBeTruthy()
    expect(screen.queryByText(/会写工作区/)).toBeNull()
  })

  it('不能用：只剩手动跟进，写明原因，带着红色警告', async () => {
    open(false)
    expect(await screen.findByText('✗ 不能直接用磁盘上的内容：磁盘上还有冲突标记')).toBeTruthy()
    expect(screen.queryByRole('button', { name: '用磁盘上的内容跟进' })).toBeNull()
    expect(screen.getByText(/会写工作区/)).toBeTruthy()
  })

  it('对比按钮用配置里的工具名，点了就启动；不内嵌冲突标记', async () => {
    const { opened } = open(true)
    fireEvent.click(await screen.findByRole('button', { name: `用 Beyond Compare 对比 ${PATH}` }))
    expect(opened).toHaveBeenCalledTimes(1)
    // 全角空格会被 getByText 归一成半角，直接比元素的文字
    expect(document.querySelector('.conflict-sides')?.textContent).toBe(
      '左：SVN 新版本 r70760　右：磁盘上的合并结果',
    )
    expect(document.body.textContent).not.toContain('<<<<<<<')
  })

  it('没有对比工具：退回文本预览，并说明上下两半各是什么', async () => {
    open(true, { compare: () => null })
    expect(await screen.findByText(/上半是 SVN 的新版本/)).toBeTruthy()
    expect(document.querySelector('.conflict-preview')?.textContent).toContain('你的提交')
  })

  it('点磁盘按钮：跑完关框', async () => {
    const { run, closed } = open(true)
    fireEvent.click(await screen.findByRole('button', { name: '用磁盘上的内容跟进' }))
    expect(run).toHaveBeenCalledWith('disk')
    await closed()
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())
  })

  it('跑的时候按钮都禁用；失败留在框里显示，可以再点', async () => {
    let fail!: (e: unknown) => void
    const run = vi.fn(
      () =>
        new Promise<void>((_, reject) => {
          fail = reject
        }),
    )
    open(true, { run })
    fireEvent.click(await screen.findByRole('button', { name: '用磁盘上的内容跟进' }))
    expect(screen.getByRole('button', { name: '正在跟进…' }).hasAttribute('disabled')).toBe(true)
    expect(screen.getByRole('button', { name: 'Cancel' }).hasAttribute('disabled')).toBe(true)
    await act(async () => fail(new Error('索引被占着')))
    expect(await screen.findByText('索引被占着')).toBeTruthy()
    expect(
      screen.getByRole('button', { name: '用磁盘上的内容跟进' }).hasAttribute('disabled'),
    ).toBe(false)
  })

  it('Cancel：不跑任何东西', async () => {
    const { run, closed } = open(true)
    fireEvent.click(await screen.findByRole('button', { name: 'Cancel' }))
    await closed()
    expect(run).not.toHaveBeenCalled()
  })
})

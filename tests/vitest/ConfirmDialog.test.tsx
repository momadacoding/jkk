// @vitest-environment jsdom
//
// 组件测试只在这个文件里切到 jsdom，其余纯逻辑测试仍跑在 node 环境（vitest.config.ts）。
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { useConfirm, type ConfirmRequest } from '@/components/ConfirmDialog'

type Confirm = (r: ConfirmRequest) => Promise<boolean>

/** 把 `useConfirm` 挂进一棵最小的组件树，拿到它的 `confirm`。 */
function mount(): { confirm: () => Confirm } {
  let current: Confirm | null = null
  function Harness() {
    const { confirm, dialog } = useConfirm()
    current = confirm
    return <>{dialog}</>
  }
  render(<Harness />)
  return {
    confirm: () => {
      if (!current) throw new Error('Harness 还没渲染')
      return current
    },
  }
}

function button(name: string): HTMLButtonElement {
  return screen.getByRole('button', { name }) as HTMLButtonElement
}

describe('useConfirm', () => {
  it('带 action 的框跑完、紧接着弹下一个：新框不能继承「处理中」（2026-09-28 E2E 场景 2）', async () => {
    // RepoTab 的真实形状：handleSvnUpdate 先 `await confirm({ action })`，
    // 返回后立刻 `showOutcome` → `await confirm({ confirmLabel: '知道了' })`
    const { confirm } = mount()
    let flow!: Promise<void>
    act(() => {
      flow = (async () => {
        await confirm()({
          title: '更新',
          message: '把 r1 更新到 r2',
          confirmLabel: '开始更新',
          busyLabel: '正在更新…',
          action: async () => {
            await Promise.resolve()
          },
        })
        await confirm()({
          title: '更新遇到冲突',
          message: '· Code/A.cs',
          confirmLabel: '知道了',
        })
      })()
    })

    fireEvent.click(button('开始更新'))

    const ok = await screen.findByRole('button', { name: '知道了' })
    expect((ok as HTMLButtonElement).disabled).toBe(false)
    expect(screen.queryByText('正在更新…')).toBeNull()
    expect(screen.queryByText('处理中…')).toBeNull()
    expect(button('Cancel').disabled).toBe(false)

    fireEvent.click(ok)
    await act(async () => {
      await flow
    })
    expect(screen.queryByText('更新遇到冲突')).toBeNull()
  })

  it('紧接着弹出的新框能用 Esc 关掉（上一个框的 busy 会让 Esc 失效）', async () => {
    const { confirm } = mount()
    let second!: Promise<boolean>
    act(() => {
      void (async () => {
        await confirm()({ title: '第一个', message: 'm', confirmLabel: '执行', action: async () => {} })
        second = confirm()({ title: '第二个', message: 'm', confirmLabel: '知道了' })
      })()
    })
    fireEvent.click(button('执行'))
    await screen.findByText('第二个')

    fireEvent.keyDown(window, { key: 'Escape' })
    await waitFor(() => expect(screen.queryByText('第二个')).toBeNull())
    await expect(second).resolves.toBe(false)
  })

  it('action 进行中显示进度、按钮禁用；完成后关闭并返回 true', async () => {
    const { confirm } = mount()
    let release!: () => void
    const gate = new Promise<void>((r) => {
      release = r
    })
    let result!: Promise<boolean>
    act(() => {
      result = confirm()({
        title: '推送',
        message: 'm',
        confirmLabel: '推送',
        busyLabel: '正在推送…',
        action: () => gate,
      })
    })

    fireEvent.click(button('推送'))
    expect(await screen.findByText('正在推送…')).toBeTruthy()
    expect(button('…').disabled).toBe(true)
    expect(button('Cancel').disabled).toBe(true)

    await act(async () => {
      release()
      await gate
    })
    await expect(result).resolves.toBe(true)
    expect(screen.queryByText('推送')).toBeNull()
  })

  it('旧框的 action 跑完不能把后来顶替它的新框一起关掉（PR #49 Codex review #5）', async () => {
    // 两个请求重叠：第一个框执行中，第二个 confirm 顶替了它。第一个的 action 跑完调
    // onDone 时原先无条件清空 pending，把第二个框也关了，第二个 Promise 永远不结束
    const { confirm } = mount()
    let release!: () => void
    const gate = new Promise<void>((r) => {
      release = r
    })
    act(() => {
      void confirm()({ title: '第一个', message: 'm', confirmLabel: '执行', action: () => gate })
    })
    fireEvent.click(button('执行'))
    let second!: Promise<boolean>
    act(() => {
      second = confirm()({ title: '第二个', message: 'm', confirmLabel: '知道了' })
    })
    await act(async () => {
      release()
      await gate
    })

    expect(screen.queryByText('第二个')).not.toBeNull()
    fireEvent.click(button('知道了'))
    await expect(second).resolves.toBe(true)
  })

  it('结局提示（notice）：只有一个按钮，没有 Cancel；ok 不挂警告图标', async () => {
    // 3a E2E 之后的观察：「已交付 r2」借确认框外观，带警告三角与多余的 Cancel
    const { confirm } = mount()
    let shown!: Promise<boolean>
    act(() => {
      shown = confirm()({ title: '已交付 r2', message: 'm', confirmLabel: '知道了', notice: 'ok' })
    })
    expect(screen.getAllByRole('button').map((b) => b.textContent)).toEqual(['知道了'])
    expect(document.querySelector('.dlg-icon.ok')).not.toBeNull()
    expect(document.querySelector('.dlg-icon.attention')).toBeNull()
    fireEvent.click(button('知道了'))
    await expect(shown).resolves.toBe(true)
  })

  it('结局提示（attention）：同样只有一个按钮，图标是提醒色', () => {
    const { confirm } = mount()
    act(() => {
      void confirm()({ title: '更新遇到冲突', message: 'm', confirmLabel: '知道了', notice: 'attention' })
    })
    expect(screen.queryByRole('button', { name: 'Cancel' })).toBeNull()
    expect(document.querySelector('.dlg-icon.attention')).not.toBeNull()
  })

  it('普通确认框照旧有 Cancel', () => {
    const { confirm } = mount()
    act(() => {
      void confirm()({ title: '删除', message: 'm', confirmLabel: '删除' })
    })
    expect(screen.queryByRole('button', { name: 'Cancel' })).not.toBeNull()
  })

  it('action 抛错：留在原地显示错误，按钮恢复可点', async () => {
    const { confirm } = mount()
    act(() => {
      void confirm()({
        title: '删除',
        message: 'm',
        confirmLabel: '删除',
        action: async () => {
          throw new Error('remote rejected')
        },
      })
    })

    fireEvent.click(button('删除'))
    expect(await screen.findByText(/remote rejected/)).toBeTruthy()
    expect(button('删除').disabled).toBe(false)
    expect(screen.queryByText('处理中…')).toBeNull()
  })
})

describe('正文过长时按钮不能被挤走', () => {
  // jsdom 不做布局，这里只钉结构：滚动的是 .dialog-body，按钮和错误条必须在它外面。
  // 高度上限与滚动是 CSS 的事，要在真实窗口里验
  const body = () => document.querySelector('.dialog-body') as HTMLElement

  it('标题、正文、警告条都在滚动区里', () => {
    const { confirm } = mount()
    act(() => {
      void confirm()({ title: '标题', message: '正文', warning: '警告', confirmLabel: 'OK' })
    })
    for (const text of ['标题', '正文', '警告']) {
      expect(body().contains(screen.getByText(text))).toBe(true)
    }
  })

  it('按钮在滚动区外，滚到哪都看得见', () => {
    const { confirm } = mount()
    act(() => {
      void confirm()({ title: 't', message: 'm', confirmLabel: 'OK' })
    })
    expect(body().contains(button('OK'))).toBe(false)
    expect(body().contains(button('Cancel'))).toBe(false)
  })

  it('action 的错误也在滚动区外：滚动条停在顶部时点确认，失败不能发生在视口之外', async () => {
    const { confirm } = mount()
    act(() => {
      void confirm()({
        title: 't',
        message: 'm',
        confirmLabel: '更新',
        action: async () => {
          throw new Error('remote rejected')
        },
      })
    })
    fireEvent.click(button('更新'))
    const error = await screen.findByText(/remote rejected/)
    expect(body().contains(error)).toBe(false)
  })

  it('滚动区能用 Tab 走到：焦点困在按钮上时键盘滚不动正文', () => {
    // useFocusTrap 只认可聚焦元素，不带 tabindex 的话 Tab 只在按钮之间循环
    const { confirm } = mount()
    act(() => {
      void confirm()({ title: 't', message: 'm', confirmLabel: 'OK' })
    })
    expect(body().tabIndex).toBe(0)
  })

  it('错误区同样能用 Tab 走到', async () => {
    const { confirm } = mount()
    act(() => {
      void confirm()({
        title: 't',
        message: 'm',
        confirmLabel: '更新',
        action: async () => {
          throw new Error('remote rejected')
        },
      })
    })
    fireEvent.click(button('更新'))
    expect((await screen.findByText(/remote rejected/)).tabIndex).toBe(0)
  })

  it('进度也在滚动区外', async () => {
    const { confirm } = mount()
    let release!: () => void
    const gate = new Promise<void>((r) => {
      release = r
    })
    act(() => {
      void confirm()({
        title: 't',
        message: 'm',
        confirmLabel: '更新',
        busyLabel: '正在更新…',
        action: () => gate,
      })
    })
    fireEvent.click(button('更新'))
    const progress = await screen.findByText('正在更新…')
    expect(body().contains(progress)).toBe(false)
    await act(async () => {
      release()
      await gate
    })
  })
})

describe('focusCancel', () => {
  it('危险操作要求默认按钮是取消：焦点落在 Cancel 上，回车不会误确认', async () => {
    const { confirm } = mount()
    let result!: Promise<boolean>
    act(() => {
      result = confirm()({ title: '合并前确认', message: 'm', danger: true, confirmLabel: '仍然继续', focusCancel: true })
    })
    await waitFor(() => expect(document.activeElement).toBe(button('Cancel')))
    fireEvent.click(button('Cancel'))
    await expect(result).resolves.toBe(false)
  })

  it('不设时焦点照旧在确认按钮上', async () => {
    const { confirm } = mount()
    act(() => { void confirm()({ title: 't', message: 'm', confirmLabel: 'Go' }) })
    await waitFor(() => expect(document.activeElement).toBe(button('Go')))
  })
})

describe('第二个选择 alternative（§6.11.3 切到落后基线的分支）', () => {
  function open(onSelect: () => void) {
    const { confirm } = mount()
    let result!: Promise<boolean>
    act(() => {
      result = confirm()({
        title: '切换',
        message: 'm',
        confirmLabel: '切换并手动跟进',
        alternative: { label: '仅切换', onSelect },
      })
    })
    return () => result
  }

  it('默认按钮仍是确认：回车不会落到第二个选择上', async () => {
    open(() => {})
    await waitFor(() => expect(document.activeElement).toBe(button('切换并手动跟进')))
  })

  it('点它：跑回调、框关掉，结果是 false，调用方靠回调分辨', async () => {
    let picked = 0
    const result = open(() => {
      picked += 1
    })
    fireEvent.click(button('仅切换'))
    expect(await result()).toBe(false)
    expect(picked).toBe(1)
    expect(screen.queryByRole('button', { name: '仅切换' })).toBeNull()
  })

  it('取消不触发它', async () => {
    let picked = 0
    const result = open(() => {
      picked += 1
    })
    fireEvent.click(button('Cancel'))
    expect(await result()).toBe(false)
    expect(picked).toBe(0)
  })

  it('没传时不渲染', async () => {
    const { confirm } = mount()
    act(() => {
      void confirm()({ title: 't', message: 'm', confirmLabel: 'OK' })
    })
    expect(screen.getAllByRole('button').map((b) => b.textContent)).toEqual(['Cancel', 'OK'])
  })
})

// @vitest-environment jsdom
//
// 弹框的焦点圈：Tab / Shift+Tab 只在框内循环，焦点不能出框（2026-10-22 E2E 第 7 条复验时发现）。
// jsdom 不会自己按 Tab 移焦点，所以这里断言的正是 hook 自己挪的焦点——真实窗口里也是它在挪。
import { fireEvent, render, screen } from '@testing-library/react'
import { useRef, type ReactNode } from 'react'
import { describe, expect, it } from 'vitest'
import { useFocusTrap } from '@/components/useFocusTrap'

function Trap({ children }: { children: ReactNode }) {
  const ref = useRef<HTMLDivElement>(null)
  useFocusTrap(ref)
  return <div ref={ref} className="dialog">{children}</div>
}

function Page({ children }: { children: ReactNode }) {
  return (
    <>
      <button>背后的 Push</button>
      <Trap>{children}</Trap>
    </>
  )
}

const tab = (shift = false) => fireEvent.keyDown(document.activeElement ?? document.body, { key: 'Tab', shiftKey: shift })
const btn = (name: string) => screen.getByRole('button', { name })

describe('useFocusTrap', () => {
  it('最后一个按 Tab 回到第一个，不去背后的按钮', () => {
    render(<Page><input aria-label="名字" /><button>Cancel</button><button>OK</button></Page>)
    btn('OK').focus()
    tab()
    expect(document.activeElement).toBe(screen.getByLabelText('名字'))
  })

  it('第一个按 Shift+Tab 到最后一个', () => {
    render(<Page><button>Cancel</button><button>OK</button></Page>)
    btn('Cancel').focus()
    tab(true)
    expect(document.activeElement).toBe(btn('OK'))
  })

  it('中间照常往后走', () => {
    render(<Page><button>A</button><button>B</button><button>C</button></Page>)
    btn('A').focus()
    tab()
    expect(document.activeElement).toBe(btn('B'))
  })

  it('焦点在框外（背后的按钮上）时按 Tab，拉回框里第一个', () => {
    render(<Page><button>Cancel</button><button>OK</button></Page>)
    btn('背后的 Push').focus()
    tab()
    expect(document.activeElement).toBe(btn('Cancel'))
  })

  it('禁用的按钮跳过', () => {
    render(<Page><button>Cancel</button><button disabled>OK</button></Page>)
    btn('Cancel').focus()
    tab()
    expect(document.activeElement).toBe(btn('Cancel'))
  })

  it('框里没有可聚焦的（执行中按钮全禁用）：焦点不动，也不出框', () => {
    render(<Page><button disabled>Cancel</button><button disabled>…</button></Page>)
    document.body.focus()
    const before = document.activeElement
    const prevented = !fireEvent.keyDown(document.body, { key: 'Tab' })
    expect(prevented).toBe(true)
    expect(document.activeElement).toBe(before)
  })

  it('其他按键不管', () => {
    render(<Page><button>Cancel</button><button>OK</button></Page>)
    btn('OK').focus()
    const prevented = !fireEvent.keyDown(btn('OK'), { key: 'Enter' })
    expect(prevented).toBe(false)
    expect(document.activeElement).toBe(btn('OK'))
  })

  it('叠了两层框：只有最上面那层管 Tab', () => {
    function Two() {
      return (
        <>
          <Trap><button>下层</button></Trap>
          <Trap><button>上层 A</button><button>上层 B</button></Trap>
        </>
      )
    }
    render(<Two />)
    btn('上层 B').focus()
    tab()
    expect(document.activeElement).toBe(btn('上层 A'))
  })
})

describe('useFocusTrap：左右方向键在按钮行里切换', () => {
  const arrow = (key: 'ArrowLeft' | 'ArrowRight') =>
    fireEvent.keyDown(document.activeElement ?? document.body, { key })

  /** 弹框的真实形状：正文（可能有输入框）+ `.dialog-actions` 按钮行。 */
  function Actions({ before, children }: { before?: ReactNode; children: ReactNode }) {
    return (
      <Page>
        {before}
        <div className="dialog-actions">{children}</div>
      </Page>
    )
  }

  it('→ 从 Cancel 到 Discard，← 回来（2026-11-08 丢弃确认框实测方向键不动）', () => {
    render(<Actions><button>Cancel</button><button>Discard</button></Actions>)
    btn('Cancel').focus()
    arrow('ArrowRight')
    expect(document.activeElement).toBe(btn('Discard'))
    arrow('ArrowLeft')
    expect(document.activeElement).toBe(btn('Cancel'))
  })

  it('到头了绕回另一头', () => {
    render(<Actions><button>Cancel</button><button>仅切换</button><button>OK</button></Actions>)
    btn('OK').focus()
    arrow('ArrowRight')
    expect(document.activeElement).toBe(btn('Cancel'))
    arrow('ArrowLeft')
    expect(document.activeElement).toBe(btn('OK'))
  })

  it('禁用的按钮跳过', () => {
    render(<Actions><button>Cancel</button><button disabled>仅切换</button><button>OK</button></Actions>)
    btn('Cancel').focus()
    arrow('ArrowRight')
    expect(document.activeElement).toBe(btn('OK'))
  })

  it('焦点在输入框里：方向键归输入框挪光标，不抢', () => {
    render(<Actions before={<input aria-label="名字" />}><button>Cancel</button><button>OK</button></Actions>)
    const input = screen.getByLabelText('名字')
    input.focus()
    const prevented = !fireEvent.keyDown(input, { key: 'ArrowRight' })
    expect(prevented).toBe(false)
    expect(document.activeElement).toBe(input)
  })

  it('tabindex=-1 的按钮跳过：和 Tab 认的可聚焦元素一致', () => {
    render(<Actions><button>Cancel</button><button tabIndex={-1}>辅助</button><button>OK</button></Actions>)
    btn('Cancel').focus()
    arrow('ArrowRight')
    expect(document.activeElement).toBe(btn('OK'))
  })

  it('焦点在按钮行之外的按钮上：不管', () => {
    render(<Actions before={<button>正文里的按钮</button>}><button>Cancel</button><button>OK</button></Actions>)
    btn('正文里的按钮').focus()
    arrow('ArrowRight')
    expect(document.activeElement).toBe(btn('正文里的按钮'))
  })

  it('叠了两层框：只有最上面那层管方向键', () => {
    function Two() {
      return (
        <>
          <Trap><div className="dialog-actions"><button>下层 A</button><button>下层 B</button></div></Trap>
          <Trap><div className="dialog-actions"><button>上层 A</button><button>上层 B</button></div></Trap>
        </>
      )
    }
    render(<Two />)
    btn('下层 A').focus()
    arrow('ArrowRight')
    expect(document.activeElement).toBe(btn('下层 A'))
  })
})

describe('useFocusTrap：点框外不抢走焦点', () => {
  // jsdom 不会在 mousedown 的默认动作里挪焦点，所以断言的是 preventDefault 本身——
  // 真实窗口里正是它让点遮罩之后焦点留在按钮上，回车 / 方向键 / Esc 还有着落
  it('在框外按下鼠标：取消默认动作，焦点留在框里的按钮上', () => {
    render(<Page><button>Cancel</button><button>OK</button></Page>)
    btn('OK').focus()
    const prevented = !fireEvent.mouseDown(btn('背后的 Push'))
    expect(prevented).toBe(true)
    expect(document.activeElement).toBe(btn('OK'))
  })

  it('在框里按下鼠标照常：输入框要能点进去、文字要能选', () => {
    render(<Page><input aria-label="名字" /><button>OK</button></Page>)
    const prevented = !fireEvent.mouseDown(screen.getByLabelText('名字'))
    expect(prevented).toBe(false)
  })

  it('框关掉之后不再拦', () => {
    const { unmount } = render(<Trap><button>OK</button></Trap>)
    unmount()
    render(<button>别处</button>)
    const prevented = !fireEvent.mouseDown(btn('别处'))
    expect(prevented).toBe(false)
  })
})

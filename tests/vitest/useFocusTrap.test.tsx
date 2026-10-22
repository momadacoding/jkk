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

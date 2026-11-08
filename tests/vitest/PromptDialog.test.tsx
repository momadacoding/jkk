// @vitest-environment jsdom
import { act, fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { usePrompt, type PromptRequest } from '@/components/PromptDialog'

type Prompt = (r: PromptRequest) => Promise<string | null>

function mount(): Prompt {
  let current: Prompt | null = null
  function Harness() {
    const { prompt, dialog } = usePrompt()
    current = prompt
    return <>{dialog}</>
  }
  render(<Harness />)
  return (r) => {
    if (!current) throw new Error('Harness 还没渲染')
    return current(r)
  }
}

function open(): Promise<string | null> {
  const prompt = mount()
  let result!: Promise<string | null>
  act(() => {
    result = prompt({ title: '新建分支', message: '分支名', defaultValue: 'feat/x' })
  })
  return result
}

describe('usePrompt 关框', () => {
  it('焦点在输入框里按 Esc：取消', async () => {
    const result = open()
    fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Escape' })
    await expect(result).resolves.toBeNull()
  })

  it('Tab 到按钮上再按 Esc 也取消：以前只有输入框认 Esc，点遮罩又不关框之后就只剩 Cancel 能关', async () => {
    const result = open()
    const cancel = screen.getByRole('button', { name: 'Cancel' })
    cancel.focus()
    fireEvent.keyDown(cancel, { key: 'Escape' })
    await expect(result).resolves.toBeNull()
  })

  it('点遮罩不关框，打的字还在', async () => {
    const prompt = mount()
    let settled = false
    act(() => {
      void prompt({ title: '新建分支', message: '分支名' }).then(() => (settled = true))
    })
    fireEvent.change(screen.getByRole('textbox'), { target: { value: 'feat/half-typed' } })
    fireEvent.mouseDown(document.querySelector('.dialog-overlay')!)
    await act(async () => {})
    expect(settled).toBe(false)
    expect((screen.getByRole('textbox') as HTMLInputElement).value).toBe('feat/half-typed')
  })
})

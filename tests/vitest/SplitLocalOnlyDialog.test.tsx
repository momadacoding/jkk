// @vitest-environment jsdom
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { SplitLocalOnlyDialog } from '@/components/SplitLocalOnlyDialog'

const FILES = ['Proxy/ImpChat.lua', 'Proxy/ImpDatabase.lua', 'Proxy/Other.lua']

function open(split = vi.fn(async (_: string[]) => {})) {
  const onClose = vi.fn()
  render(
    <SplitLocalOnlyDialog request={{ label: 'fae50a8 add @avatarclass', files: FILES, split }} onClose={onClose} />,
  )
  const primary = () => screen.getByRole('button', { name: /拆出/ }) as HTMLButtonElement
  const box = (name: string) => screen.getByLabelText(name) as HTMLInputElement
  return { split, onClose, primary, box }
}

describe('SplitLocalOnlyDialog（使用报告 9.2）', () => {
  it('一个都没勾时拆不了；勾一个就能拆', () => {
    const { primary, box } = open()
    expect(primary().disabled).toBe(true)
    fireEvent.click(box('Proxy/ImpDatabase.lua'))
    expect(primary().disabled).toBe(false)
    expect(primary().textContent).toBe('拆出 1 个文件')
  })

  it('全勾了也拆不了，并说明该用「取消仅本地」', () => {
    const { primary, box } = open()
    for (const f of FILES) fireEvent.click(box(f))
    expect(primary().disabled).toBe(true)
    expect(screen.getByText(/取消「仅本地」/)).toBeTruthy()
  })

  it('交给后端的是勾出来的文件，按列表顺序；成功后关框', async () => {
    const { split, onClose, primary, box } = open()
    fireEvent.click(box('Proxy/Other.lua'))
    fireEvent.click(box('Proxy/ImpChat.lua'))
    fireEvent.click(primary())
    await waitFor(() => expect(onClose).toHaveBeenCalled())
    expect(split).toHaveBeenCalledWith(['Proxy/ImpChat.lua', 'Proxy/Other.lua'])
  })

  it('后端拒绝：框留在原地显示原因，可以重试', async () => {
    const split = vi.fn(async () => {
      throw { code: 'local_only_edit_unfinished_sync', message: '有一次 SVN 同步还没做完。' }
    })
    const { onClose, primary, box } = open(split)
    fireEvent.click(box('Proxy/ImpDatabase.lua'))
    fireEvent.click(primary())
    // 错误文案按全应用的统一写法带上错误码（`toErrorMessage`）
    expect(await screen.findByText(/有一次 SVN 同步还没做完。/)).toBeTruthy()
    expect(onClose).not.toHaveBeenCalled()
    expect(primary().disabled).toBe(false)
  })

  it('只改了一个文件的 commit：一打开就说明拆不了，勾上之后也只有一行（独立 review 第 6 条）', () => {
    render(
      <SplitLocalOnlyDialog
        request={{ label: 'x', files: ['Proxy/Only.lua'], split: vi.fn() }}
        onClose={vi.fn()}
      />,
    )
    expect(screen.getAllByText(/只改了一个文件/)).toHaveLength(1)
    fireEvent.click(screen.getByLabelText('Proxy/Only.lua'))
    expect(screen.getAllByText(/只改了一个文件/)).toHaveLength(1)
  })

  it('Esc 关框', () => {
    const { onClose } = open()
    fireEvent.keyDown(window, { key: 'Escape' })
    expect(onClose).toHaveBeenCalled()
  })
})

// @vitest-environment jsdom
//
// diff 面板的工具栏：开关切换后的画法、默认忽略换行符、开关记进 localStorage。
// jsdom 不排版，这里只验证结构与状态；对齐、滚动跟随要在真实窗口里看（AGENTS.md §6.1）。
import { act, cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { DiffView } from '@/components/DiffView'
import { DEFAULT_DIFF_PREFS, DIFF_PREFS_KEY } from '@/engine/diffPrefs'
import { getDiffPrefs, setDiffPrefs } from '@/store/diffPrefsStore'
import type { DiffData, DiffLine } from '@/engine/types'

const del = (content: string, o: number): DiffLine => ({ origin: '-', content, oldLineno: o, newLineno: null })
const add = (content: string, n: number): DiffLine => ({ origin: '+', content, oldLineno: null, newLineno: n })

/** LF → CRLF，第 2 行真改了（libgit2 的真实形态：整块先删后增，新版带 `\r`）。 */
const mixed: DiffData = {
  files: [{
    path: 'a.txt', status: 'modified', oldPath: null, binary: false,
    hunks: [{
      oldStart: 1, newStart: 1, header: '@@ -1,3 +1,3 @@',
      lines: [del('one', 1), del('two', 2), del('three', 3), add('one\r', 1), add('TWO\r', 2), add('three\r', 3)],
    }],
  }],
}

/** 整个文件只换了换行符。 */
const eolOnly: DiffData = {
  files: [{
    path: 'b.txt', status: 'modified', oldPath: null, binary: false,
    hunks: [{ oldStart: 1, newStart: 1, header: '@@ -1 +1 @@', lines: [del('x', 1), add('x\r', 1)] }],
  }],
}

const signs = (root: HTMLElement) =>
  Array.from(root.querySelectorAll('.diff-line .diff-sign')).map((e) => e.textContent).join('')

beforeEach(() => {
  act(() => { setDiffPrefs({ ...DEFAULT_DIFF_PREFS }) })
})
afterEach(cleanup)

describe('DiffView 工具栏', () => {
  it('默认忽略换行符：只剩真改的那一行', () => {
    const { container } = render(<DiffView diff={mixed} />)
    // 上下文行的符号栏是空的，拼起来只剩改动的那两个
    expect(signs(container)).toBe('-+')
    expect(container.querySelectorAll('.diff-line')).toHaveLength(4)
    expect(screen.getByRole('button', { name: /忽略换行符/ }).getAttribute('aria-pressed')).toBe('true')
  })

  it('关掉忽略换行符：每一行都是改动', () => {
    const { container } = render(<DiffView diff={mixed} />)
    fireEvent.click(screen.getByRole('button', { name: /忽略换行符/ }))
    expect(signs(container)).toBe('---+++')
  })

  it('整个文件只换了换行符时说明原因，不说「没有文本改动」', () => {
    render(<DiffView diff={eolOnly} />)
    expect(screen.getByText(/只有换行符/)).toBeTruthy()
  })

  it('左右分栏：一删一增并排在同一行，两侧各一个窗格', () => {
    const { container } = render(<DiffView diff={mixed} />)
    fireEvent.click(screen.getByRole('button', { name: '左右分栏' }))
    const panes = container.querySelectorAll('.diff-split-pane')
    expect(panes).toHaveLength(2)
    // 左边旧版、右边新版，各 3 行（1 行上下文 + 1 行改动 + 1 行上下文）
    const left = panes[0].querySelectorAll('.diff-line')
    const right = panes[1].querySelectorAll('.diff-line')
    expect(left).toHaveLength(3)
    expect(right).toHaveLength(3)
    expect(left[1].className).toContain('del')
    expect(right[1].className).toContain('add')
    expect(left[1].textContent).toContain('two')
    expect(right[1].textContent).toContain('TWO')
  })

  it('分栏 + 自动换行：改成一行一个两列的格子', () => {
    const { container } = render(<DiffView diff={mixed} />)
    fireEvent.click(screen.getByRole('button', { name: '左右分栏' }))
    fireEvent.click(screen.getByRole('button', { name: '自动换行' }))
    expect(container.querySelector('.diff-view')!.className).toContain('wrap')
    expect(container.querySelectorAll('.diff-split-pane')).toHaveLength(0)
    expect(container.querySelectorAll('.diff-split-row')).toHaveLength(3)
  })

  it('不可见字符：空格和行尾 CR 有标记，关着时行尾 CR 不进文本', () => {
    const data: DiffData = {
      files: [{
        path: 'c.txt', status: 'modified', oldPath: null, binary: false,
        hunks: [{ oldStart: 1, newStart: 1, header: '@@ -1 +1 @@', lines: [del('a', 1), add('a b\r', 1)] }],
      }],
    }
    act(() => { setDiffPrefs({ ignoreEol: false }) })
    const { container } = render(<DiffView diff={data} />)
    expect(container.querySelectorAll('.diff-ws')).toHaveLength(0)
    expect(container.textContent).not.toContain('\r')
    fireEvent.click(screen.getByRole('button', { name: /显示空格/ }))
    expect(container.querySelectorAll('.diff-ws-space')).toHaveLength(1)
    expect(container.querySelectorAll('.diff-ws-cr')).toHaveLength(1)
  })

  it('上下文到头时按钮禁用；整个文件时两个都禁用', () => {
    act(() => { setDiffPrefs({ contextLines: 0 }) })
    render(<DiffView diff={mixed} />)
    expect((screen.getByRole('button', { name: /减少上下文/ }) as HTMLButtonElement).disabled).toBe(true)
    fireEvent.click(screen.getByRole('button', { name: /增加上下文/ }))
    expect(getDiffPrefs().contextLines).toBe(1)
    fireEvent.click(screen.getByRole('button', { name: '显示整个文件' }))
    expect((screen.getByRole('button', { name: /减少上下文/ }) as HTMLButtonElement).disabled).toBe(true)
    expect((screen.getByRole('button', { name: /增加上下文/ }) as HTMLButtonElement).disabled).toBe(true)
  })

  it('开关记进 localStorage', () => {
    render(<DiffView diff={mixed} />)
    fireEvent.click(screen.getByRole('button', { name: '左右分栏' }))
    expect(JSON.parse(localStorage.getItem(DIFF_PREFS_KEY)!)).toMatchObject({ split: true, ignoreEol: true })
  })

  it('读取失败的说明也带着工具栏，能把开关调回去', () => {
    act(() => { setDiffPrefs({ fullFile: true }) })
    render(<DiffView diff={{ files: [] }} note="读取改动失败：boom" />)
    expect(screen.getByText('读取改动失败：boom')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: '显示整个文件' }))
    expect(getDiffPrefs().fullFile).toBe(false)
  })

  it('后端按默认上下文读了（文件太多）时说明一句', () => {
    const { rerender } = render(<DiffView diff={mixed} />)
    expect(screen.queryByText(/文件太多/)).toBeNull()
    rerender(<DiffView diff={{ ...mixed, contextNarrowed: true }} />)
    expect(screen.getByText(/文件太多/)).toBeTruthy()
  })

  it('没有文件时也有工具栏', () => {
    render(<DiffView diff={{ files: [] }} />)
    expect(screen.getByRole('toolbar')).toBeTruthy()
    expect(screen.getByText('No changes')).toBeTruthy()
  })
})

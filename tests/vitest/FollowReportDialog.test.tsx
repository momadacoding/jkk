// @vitest-environment jsdom
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import {
  useFollowReportDialog,
  type FollowReportRequest,
} from '@/components/FollowReportDialog'
import type { FollowReport, SvnUserInfo } from '@/engine/svnFollow'

const report: FollowReport = {
  newFloor: 7,
  verified: true,
  changes: [{ path: 'Code/A.cs', rev: 7, author: 'zhang' }],
  removed: [],
  dropped: [],
  rewritten: [],
  stage: { decisions: {}, uncertain: [], lost: [], discarded: [] },
  movedBranches: [],
  behindBranches: [],
  leaks: [],
  pinnedRef: null,
}

const known: SvnUserInfo = { configured: 'zhang', detected: null }
const unknown: SvnUserInfo = { configured: null, detected: null }

function open(r: FollowReport, over: Partial<FollowReportRequest> = {}) {
  const saveUser = vi.fn(async () => {})
  let show!: (x: FollowReportRequest) => Promise<void>
  function Harness() {
    const { showFollowReport, dialog } = useFollowReportDialog()
    show = showFollowReport
    return <>{dialog}</>
  }
  render(<Harness />)
  let closed!: Promise<void>
  act(() => {
    closed = show({ report: r, readUser: async () => known, saveUser, ...over })
  })
  return { saveUser, closed: () => closed }
}

describe('FollowReportDialog', () => {
  it('显示摘要与按作者分组的文件', async () => {
    open(report)
    expect(await screen.findByText('跟进完成')).toBeTruthy()
    expect(screen.getByText('zhang')).toBeTruthy()
    expect(screen.getByText(/Code\/A\.cs\s+r7/)).toBeTruthy()
    await waitFor(() => expect(screen.queryByLabelText('我的 SVN 用户名')).toBeNull())
  })

  it('点「知道了」关闭，Promise 结束', async () => {
    const { closed } = open(report)
    fireEvent.click(screen.getByRole('button', { name: '知道了' }))
    await closed()
    expect(screen.queryByRole('dialog')).toBeNull()
  })

  it('Esc 关闭', async () => {
    const { closed } = open(report)
    fireEvent.keyDown(window, { key: 'Escape' })
    await closed()
    expect(screen.queryByRole('dialog')).toBeNull()
  })

  it('离线没验证、泄漏告警都用警告条', () => {
    open({
      ...report,
      verified: false,
      leaks: [
        {
          branch: 'work',
          commit: 'c'.repeat(40),
          subject: '加日志',
          path: 'Code/L.cs',
          hunks: [{ oldStart: 3, lines: 1, verdict: 'present' }],
        },
      ],
    })
    const warnings = Array.from(document.querySelectorAll('.dlg-warning')).map((n) => n.textContent)
    expect(warnings.some((t) => t?.includes('服务器'))).toBe(true)
    expect(warnings.some((t) => t?.includes('Local-Only'))).toBe(true)
  })

  it('取不到用户时显示一行输入框，说明它的用途', async () => {
    open(report, { readUser: async () => unknown })
    const input = (await screen.findByLabelText('我的 SVN 用户名')) as HTMLInputElement
    expect(input.value).toBe('')
    expect(screen.getByRole('dialog').textContent).toContain('这次 SVN 提交是不是我的')
  })

  it('有拿不准的暂存时也问，并预填已知的名字', async () => {
    open(
      { ...report, stage: { ...report.stage, uncertain: ['Code/U.cs'] } },
      { readUser: async () => ({ configured: null, detected: 'li' }) },
    )
    const input = (await screen.findByLabelText('我的 SVN 用户名')) as HTMLInputElement
    expect(input.value).toBe('li')
  })

  it('配置优先于探测结果预填', async () => {
    open(
      { ...report, stage: { ...report.stage, uncertain: ['Code/U.cs'] } },
      { readUser: async () => ({ configured: 'zhang', detected: 'li' }) },
    )
    const input = (await screen.findByLabelText('我的 SVN 用户名')) as HTMLInputElement
    expect(input.value).toBe('zhang')
  })

  it('读用户失败按取不到处理：报告照常，输入框给出来', async () => {
    open(report, {
      readUser: async () => {
        throw { code: 'svn_config', message: 'x' }
      },
    })
    expect(await screen.findByLabelText('我的 SVN 用户名')).toBeTruthy()
    expect(screen.getByText('跟进完成')).toBeTruthy()
  })

  it('保存：把输入框的内容交给后端，并说下一次跟进生效', async () => {
    const { saveUser } = open(report, { readUser: async () => unknown })
    const input = await screen.findByLabelText('我的 SVN 用户名')
    fireEvent.change(input, { target: { value: 'wang' } })
    fireEvent.click(screen.getByRole('button', { name: '保存' }))

    await waitFor(() => expect(saveUser).toHaveBeenCalledWith('wang'))
    expect(await screen.findByText(/下次跟进/)).toBeTruthy()
  })

  it('回车保存，不关掉报告', async () => {
    const { saveUser } = open(report, { readUser: async () => unknown })
    const input = await screen.findByLabelText('我的 SVN 用户名')
    fireEvent.change(input, { target: { value: 'wang' } })
    fireEvent.keyDown(input, { key: 'Enter' })

    await waitFor(() => expect(saveUser).toHaveBeenCalledWith('wang'))
    expect(screen.getByRole('dialog')).toBeTruthy()
  })

  it('保存失败：留在框里显示错误', async () => {
    open(report, {
      readUser: async () => unknown,
      saveUser: async () => {
        throw { code: 'svn_config', message: '配置坏了' }
      },
    })
    const input = await screen.findByLabelText('我的 SVN 用户名')
    fireEvent.change(input, { target: { value: 'wang' } })
    fireEvent.click(screen.getByRole('button', { name: '保存' }))

    expect(await screen.findByText(/配置坏了/)).toBeTruthy()
    expect(screen.queryByText(/下次跟进/)).toBeNull()
  })

  it('连着两份报告：第二份是全新的框', async () => {
    let show!: (x: FollowReportRequest) => Promise<void>
    function Harness() {
      const { showFollowReport, dialog } = useFollowReportDialog()
      show = showFollowReport
      return <>{dialog}</>
    }
    render(<Harness />)
    const req = { readUser: async () => unknown, saveUser: async () => {} }
    act(() => {
      void show({ report, ...req })
    })
    const input = await screen.findByLabelText('我的 SVN 用户名')
    fireEvent.change(input, { target: { value: 'wang' } })
    fireEvent.click(screen.getByRole('button', { name: '知道了' }))
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())

    act(() => {
      void show({ report, ...req })
    })
    const again = (await screen.findByLabelText('我的 SVN 用户名')) as HTMLInputElement
    expect(again.value).toBe('')
  })
})

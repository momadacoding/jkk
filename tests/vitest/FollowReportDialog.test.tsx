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
    // 路径与 revision 在两个 span 里：getByText 只认元素自己的文本，比对整行
    const row = screen.getByText('Code/A.cs').closest('li')!
    expect(row.querySelector('.follow-meta')?.textContent).toBe('r7')
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

  it('离线没验证、泄漏告警都是红色的「需处理」卡片，排在「需要你留意」里', () => {
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
    const danger = Array.from(document.querySelectorAll('.follow-section.danger')).map(
      (n) => n.textContent,
    )
    // 没体检排在最前：整份报告的前提
    expect(danger[0]).toContain('服务器')
    expect(danger.some((t) => t?.includes('Local-Only'))).toBe(true)
    expect(screen.getByText('跟进完成，有 2 项需要处理')).toBeTruthy()
  })

  it('要留意的节展开；只是告知、条数多的节折起来', () => {
    const changes = Array.from({ length: 9 }, (_, i) => ({
      path: `Code/${i}.cs`,
      rev: 7,
      author: 'zhang',
    }))
    open({ ...report, changes, behindBranches: [['old', '有冲突']] })
    const sections = Array.from(document.querySelectorAll<HTMLDetailsElement>('details.follow-section'))
    const byTitle = (t: string) => sections.find((d) => d.textContent?.includes(t))!
    expect(byTitle('落后的分支').open).toBe(true)
    expect(byTitle('跟进的文件').open).toBe(false)
  })

  it('一节超过 10 条时先列 10 条，点「再显示」铺开剩下的', () => {
    const uncertain = Array.from({ length: 12 }, (_, i) => `Code/U${i}.cs`)
    open({ ...report, stage: { ...report.stage, uncertain } })
    expect(screen.queryByText('Code/U10.cs')).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: '再显示 2 个' }))
    expect(screen.getByText('Code/U10.cs')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: '收起' }))
    expect(screen.queryByText('Code/U10.cs')).toBeNull()
  })

  it('「再显示」的数目是真能铺开的条数，不把 engine 截掉的算进去', () => {
    // engine 只留 200 条，后面是「…还有 50 个」
    const uncertain = Array.from({ length: 250 }, (_, i) => `Code/U${i}.cs`)
    open({ ...report, stage: { ...report.stage, uncertain } })
    fireEvent.click(screen.getByRole('button', { name: '再显示 190 个' }))
    const section = document.querySelector('.follow-section.attention')!
    expect(section.querySelectorAll('.follow-items li:not(.more)')).toHaveLength(200)
    expect(screen.getByText('…还有 50 个')).toBeTruthy()
  })

  it('预览按组分：第一组很长时，后面的组与它们的小标题照样看得到', () => {
    const sure = Array.from({ length: 12 }, (_, i) => ({
      branch: 'work',
      commit: 'c'.repeat(40),
      subject: '加日志',
      path: `Code/S${i}.cs`,
      hunks: [{ oldStart: 1, lines: 1, verdict: 'present' as const }],
    }))
    const unsure = { ...sure[0], path: 'Code/Unsure.cs', hunks: [{ oldStart: 1, lines: 1, verdict: 'unknown' as const }] }
    open({ ...report, leaks: [...sure, unsure] })
    expect(screen.getByText('拿不准')).toBeTruthy()
    expect(screen.getByText('Code/Unsure.cs')).toBeTruthy()
  })

  it('折起来的节：标题能 Tab 到；正文不在 DOM 里，Tab 不会落到藏着的按钮上', () => {
    const changes = Array.from({ length: 12 }, (_, i) => ({
      path: `Code/${i}.cs`,
      rev: 7,
      author: 'zhang',
    }))
    open({ ...report, changes })
    const summary = document.querySelector('details.follow-section > summary') as HTMLElement
    expect(screen.queryByText('Code/0.cs')).toBeNull()
    expect(screen.queryByRole('button', { name: /再显示/ })).toBeNull()

    // 焦点在「知道了」上，Tab 绕回框里第一个可聚焦元素
    fireEvent.keyDown(window, { key: 'Tab' })
    expect(document.activeElement).toBe(summary)

    fireEvent.click(summary)
    expect(screen.getByText('Code/0.cs')).toBeTruthy()
    expect(screen.getByRole('button', { name: '再显示 2 个' })).toBeTruthy()
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

describe('放回 Local-Only 补丁（使用报告 9.1）', () => {
  const drift: FollowReport = {
    ...report,
    drift: ['Code/A.cs', 'Code/B.cs'],
    restorable: ['Code/A.cs'],
  }
  type Outcome = { restored: string[]; skipped: { path: string; reason: string }[] }

  it('没给 restoreLocalOnly 时不显示按钮', () => {
    open(drift)
    expect(screen.queryByRole('button', { name: /放回 Local-Only 补丁/ })).toBeNull()
  })

  it('没有可放回的文件时不显示按钮', () => {
    open({ ...drift, restorable: [] }, { restoreLocalOnly: vi.fn() })
    expect(screen.queryByRole('button', { name: /放回 Local-Only 补丁/ })).toBeNull()
  })

  it('点按钮只传 restorable，成功后按钮消失、显示结果', async () => {
    const restoreLocalOnly = vi.fn(async (): Promise<Outcome> => ({
      restored: ['Code/A.cs'],
      skipped: [],
    }))
    open(drift, { restoreLocalOnly })
    fireEvent.click(screen.getByRole('button', { name: '放回 Local-Only 补丁（1 个文件）' }))

    expect(await screen.findByText(/已放回 1 个文件/)).toBeTruthy()
    expect(restoreLocalOnly).toHaveBeenCalledWith(['Code/A.cs'])
    expect(screen.queryByRole('button', { name: /放回 Local-Only 补丁/ })).toBeNull()
  })

  it('跳过的文件逐个列出原因', async () => {
    const restoreLocalOnly = vi.fn(async (): Promise<Outcome> => ({
      restored: [],
      skipped: [{ path: 'Code/A.cs', reason: '磁盘上的内容不是 SVN 的版本' }],
    }))
    open(drift, { restoreLocalOnly })
    fireEvent.click(screen.getByRole('button', { name: /放回 Local-Only 补丁/ }))

    expect(await screen.findByText('一个文件都没放回，1 个跳过了：')).toBeTruthy()
    expect(screen.getByText('磁盘上的内容不是 SVN 的版本')).toBeTruthy()
  })

  it('出错时显示错误，按钮还在、可以再点', async () => {
    const restoreLocalOnly = vi
      .fn<(paths: string[]) => Promise<Outcome>>()
      .mockRejectedValueOnce({ code: 'svn_index_busy', message: '另一个 git 进程正在占用' })
      .mockResolvedValueOnce({ restored: ['Code/A.cs'], skipped: [] })
    open(drift, { restoreLocalOnly })
    fireEvent.click(screen.getByRole('button', { name: /放回 Local-Only 补丁/ }))
    expect(await screen.findByText(/另一个 git 进程正在占用/)).toBeTruthy()

    fireEvent.click(screen.getByRole('button', { name: /放回 Local-Only 补丁/ }))
    expect(await screen.findByText(/已放回 1 个文件/)).toBeTruthy()
    expect(restoreLocalOnly).toHaveBeenCalledTimes(2)
  })

  it('按钮上方列出文件和补丁，跟进时已经放回的不再给按钮（缺陷 6）', () => {
    open(
      {
        ...drift,
        restorable: ['Code/A.cs', 'Code/B.cs'],
        restore: {
          restored: ['Code/B.cs'],
          skipped: [{ path: 'Code/A.cs', reason: '不是你' }],
          commits: { 'Code/A.cs': ['fae50a8 add @avatarclass'] },
        },
      },
      { restoreLocalOnly: vi.fn() },
    )
    expect(screen.getByRole('button', { name: '放回 Local-Only 补丁（1 个文件）' })).toBeTruthy()
    expect(screen.getAllByText('← fae50a8 add @avatarclass').length).toBe(1)
  })

  it('全部自动放回了：不给按钮', () => {
    open(
      { ...drift, restore: { restored: ['Code/A.cs'], skipped: [] } },
      { restoreLocalOnly: vi.fn() },
    )
    expect(screen.queryByRole('button', { name: /放回 Local-Only 补丁/ })).toBeNull()
  })

  it('放回进行中按钮禁用，不会重复调用', async () => {
    let finish!: (o: Outcome) => void
    const restoreLocalOnly = vi.fn(() => new Promise<Outcome>((r) => (finish = r)))
    open(drift, { restoreLocalOnly })
    fireEvent.click(screen.getByRole('button', { name: /放回 Local-Only 补丁/ }))
    const busy = await screen.findByRole('button', { name: '正在放回…' })
    expect((busy as HTMLButtonElement).disabled).toBe(true)
    fireEvent.click(busy)
    expect(restoreLocalOnly).toHaveBeenCalledTimes(1)
    await act(async () => finish({ restored: ['Code/A.cs'], skipped: [] }))
  })
})

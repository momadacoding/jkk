// @vitest-environment jsdom
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import SvnInitWizard from '@/components/SvnInitWizard'
import type { GitignoreConflict, InitPreflight, InitReport } from '@/engine/svnInit'

const CHECKBOX = '尊重已有的 .gitignore 规则'

function conflict(over: Partial<GitignoreConflict> = {}): GitignoreConflict {
  return {
    path: 'Assets/.gitignore',
    fullPath: 'C:\\w\\Assets\\.gitignore',
    versioned: false,
    activeAfterRename: false,
    rules: [{ line: 3, pattern: '*.exe', effect: 'excludes', count: 2, samples: ['Assets/a.exe'] }],
    ...over,
  }
}

function preflight(conflicts: GitignoreConflict[]): InitPreflight {
  return { nodeRev: 7, reposRoot: 'file:///r', uuid: 'u', gitignoreConflicts: conflicts }
}

const report: InitReport = {
  nodeRev: 7,
  baseline: 'b'.repeat(40),
  health: { status: 'verified', rev: 7, unverified: [] },
  pinned: [],
  timings: [],
  gitignore: { respected: false, conflicts: [], disabled: [] },
}

type OnPreflight = (w: string[], e: string[]) => Promise<InitPreflight>
type OnInit = (w: string[], e: string[], respect: boolean, ack: string[]) => Promise<InitReport>

function mount(onPreflight: OnPreflight, onInit: OnInit) {
  render(
    <SvnInitWizard
      repoPath="C:\\w"
      onCancel={() => {}}
      onFinish={() => {}}
      onPreflight={onPreflight}
      onInit={onInit}
    />,
  )
}

/** 填白名单 → 检查工作副本 → 下一步，停在「确认并建库」。 */
async function toConfirm() {
  fireEvent.change(screen.getByLabelText('交给 git 管的目录（一行一条）'), {
    target: { value: 'Assets/' },
  })
  fireEvent.click(screen.getByRole('button', { name: '检查工作副本' }))
  fireEvent.click(await screen.findByRole('button', { name: '下一步' }))
  await screen.findByRole('button', { name: '建库' })
}

describe('SvnInitWizard：起作用的 .gitignore（2026-10-28 设计 §3）', () => {
  it('没有冲突时不显示勾选框，建库按「不尊重、没确认过任何文件」请求', async () => {
    const onPreflight = vi.fn<OnPreflight>().mockResolvedValue(preflight([]))
    const onInit = vi.fn<OnInit>().mockResolvedValue(report)
    mount(onPreflight, onInit)
    await toConfirm()
    expect(onPreflight).toHaveBeenCalledWith(['Assets/'], ['.cs'])
    expect(screen.queryByLabelText(CHECKBOX)).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: '建库' }))
    await waitFor(() => expect(onInit).toHaveBeenCalledWith(['Assets/'], ['.cs'], false, []))
  })

  it('有冲突时列出完整路径，勾选框默认不勾，建库带上展示过的路径', async () => {
    const onPreflight = vi.fn<OnPreflight>().mockResolvedValue(preflight([conflict()]))
    const onInit = vi.fn<OnInit>().mockResolvedValue(report)
    mount(onPreflight, onInit)
    await toConfirm()
    expect(screen.getByText(/C:\\w\\Assets\\\.gitignore/)).toBeTruthy()
    const box = screen.getByLabelText(CHECKBOX) as HTMLInputElement
    expect(box.checked).toBe(false)
    fireEvent.click(screen.getByRole('button', { name: '建库' }))
    await waitFor(() =>
      expect(onInit).toHaveBeenCalledWith(['Assets/'], ['.cs'], false, ['Assets/.gitignore']),
    )
  })

  it('受版本控制的 .gitignore：不勾时「建库」不能点并说明原因，勾上后才能建', async () => {
    const onPreflight = vi.fn<OnPreflight>().mockResolvedValue(preflight([conflict({ versioned: true })]))
    const onInit = vi.fn<OnInit>().mockResolvedValue(report)
    mount(onPreflight, onInit)
    await toConfirm()
    const build = screen.getByRole('button', { name: '建库' }) as HTMLButtonElement
    expect(build.disabled).toBe(true)
    expect(screen.getByText(/是团队提交的规则/)).toBeTruthy()
    fireEvent.click(screen.getByLabelText(CHECKBOX))
    expect(build.disabled).toBe(false)
    fireEvent.click(build)
    await waitFor(() =>
      expect(onInit).toHaveBeenCalledWith(['Assets/'], ['.cs'], true, ['Assets/.gitignore']),
    )
  })

  it('重新检查后清单空了：勾选框跟着消失，下一次建库不再按「尊重」请求', async () => {
    // /code-review 第 5 条：勾选框藏起来了，state 里的 true 却留着，用户看不见的选项照样生效
    const onPreflight = vi
      .fn<OnPreflight>()
      .mockResolvedValueOnce(preflight([conflict({ versioned: true })]))
      .mockResolvedValueOnce(preflight([]))
    const onInit = vi
      .fn<OnInit>()
      .mockRejectedValueOnce({ code: 'svn_gitignore_unconfirmed', message: '发现了你在上一步没看到的 .gitignore' })
      .mockResolvedValue(report)
    mount(onPreflight, onInit)
    await toConfirm()
    fireEvent.click(screen.getByLabelText(CHECKBOX))
    fireEvent.click(screen.getByRole('button', { name: '建库' }))
    await waitFor(() => expect(onPreflight).toHaveBeenCalledTimes(2))
    await waitFor(() => expect(screen.queryByLabelText(CHECKBOX)).toBeNull())
    fireEvent.click(screen.getByRole('button', { name: '建库' }))
    await waitFor(() => expect(onInit).toHaveBeenLastCalledWith(['Assets/'], ['.cs'], false, []))
  })

  it('重新检查本身失败时，显示的仍是建库那次的错误', async () => {
    const onPreflight = vi
      .fn<OnPreflight>()
      .mockResolvedValueOnce(preflight([conflict()]))
      .mockRejectedValueOnce({ code: 'svn_not_wc', message: '重新检查失败' })
    const onInit = vi
      .fn<OnInit>()
      .mockRejectedValueOnce({ code: 'svn_gitignore_unconfirmed', message: '发现了你在上一步没看到的 .gitignore' })
    mount(onPreflight, onInit)
    await toConfirm()
    fireEvent.click(screen.getByRole('button', { name: '建库' }))
    expect(await screen.findByText(/发现了你在上一步没看到的 \.gitignore/)).toBeTruthy()
    expect(screen.queryByText(/重新检查失败/)).toBeNull()
  })

  it('建库报「有没看过的 .gitignore」时重新检查，把新清单摆出来', async () => {
    const inner = conflict({ path: 'Assets/Sub/.gitignore', fullPath: 'C:\\w\\Assets\\Sub\\.gitignore' })
    const onPreflight = vi
      .fn<OnPreflight>()
      .mockResolvedValueOnce(preflight([conflict()]))
      .mockResolvedValueOnce(preflight([conflict(), inner]))
    const onInit = vi
      .fn<OnInit>()
      .mockRejectedValueOnce({ code: 'svn_gitignore_unconfirmed', message: '发现了你在上一步没看到的 .gitignore' })
      .mockResolvedValue(report)
    mount(onPreflight, onInit)
    await toConfirm()
    fireEvent.click(screen.getByRole('button', { name: '建库' }))
    await waitFor(() => expect(onPreflight).toHaveBeenCalledTimes(2))
    expect(await screen.findByText(/C:\\w\\Assets\\Sub\\\.gitignore/)).toBeTruthy()
    // 仍停在第 3 步，再点一次带上新清单
    fireEvent.click(screen.getByRole('button', { name: '建库' }))
    await waitFor(() =>
      expect(onInit).toHaveBeenLastCalledWith(['Assets/'], ['.cs'], false, [
        'Assets/.gitignore',
        'Assets/Sub/.gitignore',
      ]),
    )
  })
})

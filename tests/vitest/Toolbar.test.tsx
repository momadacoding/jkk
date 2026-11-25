// @vitest-environment jsdom
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

// 自定义命令栏挂载时异步读配置（invoke），与这里测的按钮无关，留着只会在测试结束后
// 才 setState、刷出 act() 警告
vi.mock('@/components/CustomCommandBar', () => ({ CustomCommandBar: () => null }))
import { Toolbar } from '@/components/Toolbar'
import { ALL_CAPABILITIES, NO_GIT_ONLY_CAPABILITIES } from '@/engine/capabilities'

function mount(svnBlockedReason: string | null) {
  render(
    <Toolbar
      branch="work"
      upstream={null}
      onRun={async () => {}}
      commandContext={{ worktree: 'C:/wc1', branch: 'work', headHash: null }}
      capabilities={NO_GIT_ONLY_CAPABILITIES}
      onSvnUpdate={async () => {}}
      onSvnDeliver={async () => {}}
      svnBlockedReason={svnBlockedReason}
    />,
  )
}

function svnButton(action: string): HTMLButtonElement {
  const el = document.querySelector(`[data-action="${action}"]`)
  if (!el) throw new Error(`没有 ${action} 按钮`)
  return el as HTMLButtonElement
}

describe('Toolbar 的「SVN ▾」菜单（设计 2026-11-25 §7）', () => {
  function mountMenu(checkBlockedReason: string | null) {
    const onCheckBaseline = vi.fn()
    const onOpenHooks = vi.fn()
    render(
      <Toolbar
        branch="work"
        upstream={null}
        onRun={async () => {}}
        commandContext={{ worktree: 'C:/wc1', branch: 'work', headHash: null }}
        capabilities={NO_GIT_ONLY_CAPABILITIES}
        onSvnUpdate={async () => {}}
        onSvnDeliver={async () => {}}
        onCheckBaseline={onCheckBaseline}
        checkBlockedReason={checkBlockedReason}
        onOpenHooks={onOpenHooks}
      />,
    )
    fireEvent.click(svnButton('svn-menu'))
    return { onCheckBaseline, onOpenHooks }
  }

  it('里面是「体检基线…」和「TortoiseSVN 钩子…」，点了各走各的', async () => {
    const { onCheckBaseline, onOpenHooks } = mountMenu(null)
    fireEvent.click(screen.getByRole('button', { name: '体检基线…' }))
    // 菜单项先关菜单、下一拍再跑动作（ContextMenu 里的 setTimeout）
    await waitFor(() => expect(onCheckBaseline).toHaveBeenCalledTimes(1))
    fireEvent.click(svnButton('svn-menu'))
    fireEvent.click(screen.getByRole('button', { name: 'TortoiseSVN 钩子…' }))
    await waitFor(() => expect(onOpenHooks).toHaveBeenCalledTimes(1))
  })

  it('不能体检时置灰，悬停说明原因', () => {
    mountMenu('SVN 侧有新内容，先跟进')
    const item = screen.getByRole('button', { name: '体检基线…' }) as HTMLButtonElement
    expect(item.disabled).toBe(true)
    expect(item.title).toBe('SVN 侧有新内容，先跟进')
  })

  it('普通 git 仓库没有这个菜单', () => {
    render(
      <Toolbar
        branch="main"
        upstream={null}
        onRun={async () => {}}
        commandContext={{ worktree: 'C:/repo', branch: 'main', headHash: null }}
        capabilities={ALL_CAPABILITIES}
        onCheckBaseline={() => {}}
        onOpenHooks={() => {}}
      />,
    )
    expect(document.querySelector('[data-action="svn-menu"]')).toBeNull()
  })
})

describe('Toolbar 的「更新」「交付」', () => {
  it('同步被拦住时两个按钮都禁用，悬停说明原因', () => {
    mount('上次交付没有收尾')
    for (const a of ['svn-update', 'svn-deliver']) {
      expect(svnButton(a).disabled, a).toBe(true)
      expect(svnButton(a).title, a).toBe('上次交付没有收尾')
    }
  })

  it('没被拦住时可点，也不挂说明', () => {
    mount(null)
    for (const a of ['svn-update', 'svn-deliver']) {
      expect(svnButton(a).disabled, a).toBe(false)
      expect(svnButton(a).title, a).toBe('')
    }
    expect(screen.getByRole('button', { name: '更新' })).toBeTruthy()
  })
})

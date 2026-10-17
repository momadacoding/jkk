// @vitest-environment jsdom
import { render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

// 自定义命令栏挂载时异步读配置（invoke），与这里测的按钮无关，留着只会在测试结束后
// 才 setState、刷出 act() 警告
vi.mock('@/components/CustomCommandBar', () => ({ CustomCommandBar: () => null }))
import { Toolbar } from '@/components/Toolbar'
import { NO_GIT_ONLY_CAPABILITIES } from '@/engine/capabilities'

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

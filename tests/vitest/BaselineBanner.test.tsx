// @vitest-environment jsdom
import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { BaselineBanner } from '@/components/BaselineBanner'
import type { SyncState } from '@/engine/svnJournal'

function renderWith(syncState: SyncState) {
  const onSyncAction = vi.fn()
  const onSyncOutAction = vi.fn()
  render(
    <BaselineBanner
      isSvn
      sync={null}
      audit={null}
      eol={null}
      syncState={syncState}
      onRebuild={() => {}}
      onCheck={() => {}}
      onMigrateEol={() => {}}
      onSyncAction={onSyncAction}
      onSyncOutAction={onSyncOutAction}
      onSvnUpdate={() => {}}
    />,
  )
  return { onSyncAction, onSyncOutAction }
}

describe('BaselineBanner 未收尾横幅的按钮分发', () => {
  it('交付记录的按钮走交付流程，绝不交给「更新」的继续 / 放弃', () => {
    // 那两个命令会以 sync_out_not_supported 拒绝交付记录——按钮点了只会报错
    const { onSyncAction, onSyncOutAction } = renderWith({
      locked: false,
      holder: null,
      unfinished: { flow: 'sync_out', phase: 'post-commit', opid: 'x', startedAt: 1 },
    })
    fireEvent.click(screen.getByRole('button', { name: '继续交付' }))
    fireEvent.click(screen.getByRole('button', { name: '停止收尾' }))
    expect(onSyncOutAction.mock.calls).toEqual([['syncOutContinue'], ['syncOutAbort']])
    expect(onSyncAction).not.toHaveBeenCalled()
  })

  it('「清理」也走交付流程', () => {
    const { onSyncOutAction } = renderWith({
      locked: false,
      holder: null,
      unfinished: { flow: 'sync_out', phase: 'svn-staged', opid: 'x', startedAt: 1 },
    })
    fireEvent.click(screen.getByRole('button', { name: '清理' }))
    expect(onSyncOutAction).toHaveBeenCalledWith('syncOutCleanup')
  })

  it('更新记录的按钮照旧走更新流程', () => {
    const { onSyncAction, onSyncOutAction } = renderWith({
      locked: false,
      holder: null,
      unfinished: { flow: 'sync_in', phase: 'stashed', opid: 'x', startedAt: 1 },
    })
    for (const b of screen.getAllByRole('button')) fireEvent.click(b)
    expect(onSyncAction.mock.calls.map((c) => c[0])).toEqual(['syncContinue', 'syncAbort'])
    expect(onSyncOutAction).not.toHaveBeenCalled()
  })
})

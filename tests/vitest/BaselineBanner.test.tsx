// @vitest-environment jsdom
import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { BaselineBanner } from '@/components/BaselineBanner'
import type { SyncState } from '@/engine/svnJournal'
import type { SyncStatus } from '@/engine/svnSync'
import type { HookHealth } from '@/engine/tsvnHooks'

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

  it('跟进记录的「继续」交给跟进，不交给交付那一组', () => {
    const { onSyncAction, onSyncOutAction } = renderWith({
      locked: false,
      holder: null,
      unfinished: { flow: 'follow', phase: 'follow-publishing', opid: 'x', startedAt: 1 },
    })
    fireEvent.click(screen.getByRole('button', { name: '继续' }))
    expect(onSyncAction.mock.calls).toEqual([['followContinue']])
    expect(onSyncOutAction).not.toHaveBeenCalled()
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

describe('BaselineBanner 失同步横幅的「跟进」按钮', () => {
  const advanced: SyncStatus = {
    baselineRev: 3,
    nodeRevs: [3, 4],
    base: { floor: 3, newer: 0, gone: 0 },
    verdict: {
      kind: 'externallyAdvanced',
      changed: [{ path: 'Code/B.cs', rev: 4, author: 'zhang' }],
      added: [],
      removed: [],
    },
  }

  function renderSync(sync: SyncStatus, onFollow?: () => void, followBusy?: boolean) {
    render(
      <BaselineBanner
        isSvn
        sync={sync}
        audit={null}
        eol={null}
        syncState={{ locked: false, holder: null, unfinished: null }}
        onRebuild={() => {}}
        onCheck={() => {}}
        onMigrateEol={() => {}}
        onSyncAction={() => {}}
        onSyncOutAction={() => {}}
        onSvnUpdate={() => {}}
        onFollow={onFollow}
        followBusy={followBusy}
      />,
    )
  }

  it('传了 onFollow 时点「跟进」调用它，并列出文件与作者', () => {
    const onFollow = vi.fn()
    renderSync(advanced, onFollow)
    fireEvent.click(screen.getByRole('button', { name: '跟进' }))
    expect(onFollow).toHaveBeenCalledTimes(1)
    expect(screen.getByText(/Code\/B\.cs/).textContent).toContain('zhang')
  })

  it('跟进执行中按钮禁用，换成进度文案，防止重复点击', () => {
    const onFollow = vi.fn()
    renderSync(advanced, onFollow, true)
    expect(screen.queryByRole('button', { name: '跟进' })).toBeNull()
    const busy = screen.getByRole('button', { name: '正在跟进…' }) as HTMLButtonElement
    expect(busy.disabled).toBe(true)
    fireEvent.click(busy)
    expect(onFollow).not.toHaveBeenCalled()
  })

  it('分支落后基线时也有「跟进」（分支跟进入口）', () => {
    const onFollow = vi.fn()
    renderSync(
      { ...advanced, verdict: { kind: 'inSync' }, headBehindBase: true },
      onFollow,
    )
    fireEvent.click(screen.getByRole('button', { name: '跟进' }))
    expect(onFollow).toHaveBeenCalledTimes(1)
  })

  it('没传 onFollow 时不渲染「跟进」：不留一个点了没反应的按钮', () => {
    renderSync(advanced)
    expect(screen.queryByRole('button', { name: '跟进' })).toBeNull()
    expect(screen.getByText('SVN 侧有 1 个文件有新内容')).toBeTruthy()
  })

  it('wcLocked 只挂一条灰色提示，没有按钮', () => {
    renderSync({ ...advanced, verdict: { kind: 'wcLocked', dirs: ['Code'] } })
    expect(screen.getByText('SVN 正在更新…')).toBeTruthy()
    expect(screen.queryAllByRole('button')).toHaveLength(0)
  })
})

describe('BaselineBanner 的 TortoiseSVN 钩子入口（C1h）', () => {
  const inSync: SyncStatus = {
    baselineRev: 3,
    nodeRevs: [3],
    base: { floor: 3, newer: 0, gone: 0 },
    verdict: { kind: 'inSync' },
  }

  function renderHooks(hooks: HookHealth | null) {
    const onOpenHooks = vi.fn()
    const onUnregisterHooks = vi.fn()
    render(
      <BaselineBanner
        isSvn
        sync={inSync}
        audit={null}
        eol={null}
        syncState={{ locked: false, holder: null, unfinished: null }}
        onRebuild={() => {}}
        onCheck={() => {}}
        onMigrateEol={() => {}}
        onSyncAction={() => {}}
        onSyncOutAction={() => {}}
        onSvnUpdate={() => {}}
        hooks={hooks}
        onOpenHooks={onOpenHooks}
        onUnregisterHooks={onUnregisterHooks}
      />,
    )
    return { onOpenHooks, onUnregisterHooks }
  }

  it('体检基线旁有「TortoiseSVN 钩子…」', () => {
    const { onOpenHooks } = renderHooks({ state: 'notRegistered' })
    fireEvent.click(screen.getByRole('button', { name: 'TortoiseSVN 钩子…' }))
    expect(onOpenHooks).toHaveBeenCalledTimes(1)
  })

  it('失效时挂横幅，可以重新注册，也可以撤销注册', () => {
    const { onOpenHooks, onUnregisterHooks } = renderHooks({
      state: 'stale',
      reasons: ['post_commit 钩子指向 D:\\old\\app.exe'],
    })
    expect(screen.getByText('TortoiseSVN 钩子已失效')).toBeTruthy()
    expect(screen.getByText(/D:\\old\\app\.exe/)).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: '重新注册' }))
    fireEvent.click(screen.getByRole('button', { name: '撤销注册' }))
    expect(onOpenHooks).toHaveBeenCalledTimes(1)
    expect(onUnregisterHooks).toHaveBeenCalledTimes(1)
  })

  it('健康时不挂失效横幅', () => {
    renderHooks({ state: 'healthy', blockLeaks: true })
    expect(screen.queryByText('TortoiseSVN 钩子已失效')).toBeNull()
  })
})

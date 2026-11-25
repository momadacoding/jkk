// @vitest-environment jsdom
import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { BaselineBanner } from '@/components/BaselineBanner'
import type { SyncState } from '@/engine/svnJournal'
import type { SyncStatus } from '@/engine/svnSync'
import type { HookHealth } from '@/engine/tsvnHooks'
import { mismatchKey, type BaselineMismatch } from '@/engine/svnMismatch'

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

describe('BaselineBanner 未收尾横幅：「继续」执行中（2026-11-19 使用报告问题 2）', () => {
  // rebase 冲突时的「我已解决，继续」不再弹框，进度挪到按钮上；不然点完没有任何反馈
  const rebase: SyncState = {
    locked: false,
    holder: null,
    unfinished: { flow: 'follow_manual', phase: 'rebase', opid: 'x', startedAt: 1 },
  }

  it('执行中的那颗按钮换成进度文案，同一条横幅上的按钮都禁用', () => {
    const onSyncAction = vi.fn()
    render(
      <BaselineBanner
        isSvn
        sync={null}
        audit={null}
        eol={null}
        syncState={rebase}
        onRebuild={() => {}}
        onCheck={() => {}}
        onMigrateEol={() => {}}
        onSyncAction={onSyncAction}
        onSyncOutAction={() => {}}
        onSvnUpdate={() => {}}
        syncActionBusy={{ id: 'syncContinue', label: '正在继续…' }}
      />,
    )
    const busy = screen.getByRole('button', { name: '正在继续…' }) as HTMLButtonElement
    expect(busy.disabled).toBe(true)
    const abort = screen.getByRole('button', { name: '放弃，回到手动跟进前' }) as HTMLButtonElement
    expect(abort.disabled).toBe(true)
    fireEvent.click(abort)
    expect(onSyncAction).not.toHaveBeenCalled()
  })

  it('没在执行：按原文案、可以点', () => {
    const { onSyncAction } = renderWith(rebase)
    fireEvent.click(screen.getByRole('button', { name: '我已解决，继续' }))
    expect(onSyncAction.mock.calls).toEqual([['syncContinue']])
  })
})

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

  it('同步正常时顶部什么都不挂：体检和钩子的入口挪到了工具栏「SVN ▾」（设计 2026-11-25 §7）', () => {
    // 原来这里常驻一行「revision 对得上不代表……」+「体检基线」「TortoiseSVN 钩子…」，
    // 用户读成了告警：「这是干啥？我需要做什么吗？」
    renderHooks({ state: 'notRegistered' })
    expect(screen.queryAllByRole('button')).toHaveLength(0)
    expect(screen.queryByText(/revision 对得上/)).toBeNull()
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

describe('BaselineBanner：jkk 与 TortoiseSVN 看法不一致（设计 2026-11-25 §6）', () => {
  const inSync: SyncStatus = {
    baselineRev: 3,
    nodeRevs: [3],
    base: { floor: 3, newer: 0, gone: 0 },
    verdict: { kind: 'inSync' },
    headBehindBase: false,
  }
  const eolDone = { ruleInstalled: true, needsMigration: false }
  const files = ['Code/A.cs']

  function renderMismatch(
    sync: SyncStatus,
    mismatch: BaselineMismatch | null,
    mismatchAck: string | null = null,
  ) {
    const onCheck = vi.fn()
    render(
      <BaselineBanner
        isSvn
        sync={sync}
        audit={null}
        eol={eolDone}
        syncState={{ locked: false, holder: null, unfinished: null }}
        onRebuild={() => {}}
        onCheck={onCheck}
        onMigrateEol={() => {}}
        onSyncAction={() => {}}
        onSyncOutAction={() => {}}
        onSvnUpdate={() => {}}
        mismatch={mismatch}
        mismatchAck={mismatchAck}
      />,
    )
    return { onCheck }
  }

  it('有命中时挂横幅、列出文件，「体检」交给 onCheck', () => {
    const { onCheck } = renderMismatch(inSync, { files, total: 1, truncated: false })
    expect(screen.getByText(/^点「体检」核对 1 个文件/)).toBeTruthy()
    expect(screen.getByText('Code/A.cs')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: '体检' }))
    expect(onCheck).toHaveBeenCalledTimes(1)
  })

  it('没有命中时什么都不挂', () => {
    renderMismatch(inSync, { files: [], total: 0, truncated: false })
    expect(screen.queryAllByRole('button')).toHaveLength(0)
  })

  it('上一次的结果还在、但同步结论已经变成「SVN 侧有新内容」：不挂，那时判据不成立', () => {
    renderMismatch(
      {
        ...inSync,
        verdict: {
          kind: 'externallyAdvanced',
          changed: [{ path: 'Code/A.cs', rev: 4, author: 'x' }],
          added: [],
          removed: [],
        },
      },
      { files, total: 1, truncated: false },
    )
    expect(screen.queryByText(/核对 1 个文件/)).toBeNull()
    expect(screen.getByText('SVN 侧有 1 个文件有新内容')).toBeTruthy()
  })

  it('这组文件体检过没问题：不再挂', () => {
    renderMismatch(inSync, { files, total: 1, truncated: false }, mismatchKey(files))
    expect(screen.queryAllByRole('button')).toHaveLength(0)
  })
})

describe('BaselineBanner：基线分支丢了（第 70 条）', () => {
  it('挂出恢复横幅，「重建基线」交给 onRebuild，不挂常驻体检入口', () => {
    const onRebuild = vi.fn()
    render(
      <BaselineBanner
        isSvn
        sync={null}
        audit={null}
        eol={null}
        syncState={null}
        baselineMissing
        onRebuild={onRebuild}
        onCheck={() => {}}
        onMigrateEol={() => {}}
        onSyncAction={() => {}}
        onSyncOutAction={() => {}}
        onSvnUpdate={() => {}}
      />,
    )
    expect(screen.getByText(/基线分支 svn\/base 不见了/)).toBeTruthy()
    expect(screen.queryByRole('button', { name: '体检基线' })).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: '重建基线' }))
    expect(onRebuild).toHaveBeenCalledTimes(1)
  })

  it('行尾规则也没装：恢复横幅优先，不挂那颗必然报错的迁移按钮', () => {
    render(
      <BaselineBanner
        isSvn
        sync={null}
        audit={null}
        eol={{ ruleInstalled: false, needsMigration: true }}
        syncState={null}
        baselineMissing
        onRebuild={() => {}}
        onCheck={() => {}}
        onMigrateEol={() => {}}
        onSyncAction={() => {}}
        onSyncOutAction={() => {}}
        onSvnUpdate={() => {}}
      />,
    )
    expect(screen.getByText(/基线分支 svn\/base 不见了/)).toBeTruthy()
    expect(document.querySelectorAll('.banner-baseline')).toHaveLength(1)
  })
})

describe('BaselineBanner：自动跟进（事故分析 6.12）', () => {
  const advanced: SyncStatus = {
    baselineRev: 3,
    nodeRevs: [3, 4],
    base: { floor: 3, newer: 0, gone: 0 },
    headBehindBase: false,
    verdict: {
      kind: 'externallyAdvanced',
      changed: [{ path: 'Code/B.cs', rev: 4, author: 'zhang' }],
      added: [],
      removed: [],
    },
  }
  const base = {
    isSvn: true,
    audit: null,
    eol: null,
    syncState: { locked: false, holder: null, unfinished: null },
    onRebuild: () => {},
    onCheck: () => {},
    onMigrateEol: () => {},
    onSyncAction: () => {},
    onSyncOutAction: () => {},
    onSvnUpdate: () => {},
    onFollow: () => {},
  }

  it('没自动跟进时，「SVN 侧有新内容」横幅上写明原因，「跟进」照旧在', () => {
    render(<BaselineBanner {...base} sync={advanced} autoFollowSkip="当前分支重放有冲突。" />)
    expect(screen.getByText('没有自动跟进：当前分支重放有冲突。')).toBeTruthy()
    expect(screen.getByRole('button', { name: '跟进' })).toBeTruthy()
  })

  it('被重放冲突拦下：冲突文件单独成组、排在新内容列表上面，不再是一行灰字（使用报告 11-19 问题 1）', () => {
    const { container } = render(
      <BaselineBanner
        {...base}
        sync={advanced}
        autoFollowSkip="当前分支的提交重放到新基线上有冲突：Code/A.cs（r2 li）。"
        autoFollowConflict={{
          files: [{ path: 'Code/A.cs', rev: 2, author: 'li', svnRemoved: false }],
          lagging: true,
        }}
      />,
    )
    const group = screen.getByRole('alert')
    expect(group.textContent).toContain('没有自动跟进：当前分支的提交重放到新基线上，有 1 个文件冲突')
    expect(group.textContent).toContain('Code/A.cs  r2  li')
    expect(group.textContent).toContain('不一定在下面的新内容里')
    // 新内容里的 B 不在冲突组里，列表另有标签
    expect(group.textContent).not.toContain('Code/B.cs')
    expect(screen.getByText('SVN 侧的新内容：')).toBeTruthy()
    // 整句原因不再重复挂一行
    expect(screen.queryByText(/Code\/A\.cs（r2 li）/)).toBeNull()
    // 冲突组在新内容列表前面
    const html = container.innerHTML
    expect(html.indexOf('Code/A.cs')).toBeLessThan(html.indexOf('Code/B.cs'))
    expect(screen.getByRole('button', { name: '跟进' })).toBeTruthy()
  })

  describe('部分跟进留下的文件（使用报告 2026-11-23）', () => {
    const reason = '分支上的提交重放到 SVN 的新内容上有冲突，要你来判断。'
    const onlyHeld: SyncStatus = {
      ...advanced,
      verdict: {
        kind: 'externallyAdvanced',
        changed: [{ path: 'LuaScripts/UI/Common/UIBaseWindow.lua', rev: 70514, author: 'gao' }],
        added: [],
        removed: [],
      },
    }
    const files = [{ path: 'LuaScripts/UI/Common/UIBaseWindow.lua', reason, rev: 70514 }]
    const heldBack = { files, stale: false }

    it('按原因单独成组、样式同整次冲突；路径只出现一次，「跟进」只在按钮和组里那句说明里', () => {
      const { container } = render(<BaselineBanner {...base} sync={onlyHeld} heldBack={heldBack} />)
      const group = screen.getByRole('alert')
      expect(group.className).toContain('banner-baseline-conflict')
      expect(group.textContent).toContain('上一次自动跟进留下了 1 个文件，还停在旧基线上')
      expect(group.textContent).toContain(reason)
      expect(group.textContent).toContain('LuaScripts/UI/Common/UIBaseWindow.lua  r70514  gao')
      const text = container.textContent ?? ''
      expect(text.split('UIBaseWindow.lua').length - 1).toBe(1)
      // 「跟进」出现在：组里的说明、按钮；正文里那句通用说明是「点「跟进」把基线推进……」，不是这次的事
      const detail = container.querySelector('.banner-baseline-detail')?.textContent ?? ''
      const outside = text.replace(detail, '')
      expect(outside.match(/跟进/g)?.length).toBe(3) // 组标题「自动跟进」+ 组说明 + 按钮
      expect(screen.getByRole('button', { name: '跟进' })).toBeTruthy()
      expect(screen.queryByText('SVN 侧的新内容：')).toBeNull()
    })

    it('又来了新内容：留下的那组照旧在，新内容另列在标签下面', () => {
      const more: SyncStatus = {
        ...onlyHeld,
        verdict: {
          kind: 'externallyAdvanced',
          changed: [
            ...(onlyHeld.verdict.kind === 'externallyAdvanced' ? onlyHeld.verdict.changed : []),
            { path: 'Code/B.cs', rev: 70600, author: 'zhang' },
          ],
          added: [],
          removed: [],
        },
      }
      render(<BaselineBanner {...base} sync={more} heldBack={heldBack} />)
      const group = screen.getByRole('alert')
      expect(group.textContent).toContain(reason)
      expect(group.textContent).not.toContain('Code/B.cs')
      expect(screen.getByText('SVN 侧的新内容：')).toBeTruthy()
      expect(screen.getByText('Code/B.cs  r70600  zhang', { normalizer: (s) => s })).toBeTruthy()
    })

    it('SVN 侧那个文件又有了更新的 revision：不再当成留下的', () => {
      render(
        <BaselineBanner
          {...base}
          sync={onlyHeld}
          heldBack={{ files: [{ ...files[0], rev: 70424 }], stale: false }}
        />,
      )
      expect(screen.queryByRole('alert')).toBeNull()
      expect(
        screen.getByText('LuaScripts/UI/Common/UIBaseWindow.lua  r70514  gao', { normalizer: (s) => s }),
      ).toBeTruthy()
    })

    it('HEAD 变了（提交、切分支）：红框照旧在，说明换成「之前的提交上判定的」（独立 review 中 1）', () => {
      render(<BaselineBanner {...base} sync={onlyHeld} heldBack={{ files, stale: true }} />)
      const group = screen.getByRole('alert')
      expect(group.textContent).toContain(reason)
      expect(group.textContent).toContain('之前的提交上判定的')
    })
  })

  it('冲突清单为空或没有时照旧写那一行原因、不加列表标签', () => {
    render(
      <BaselineBanner
        {...base}
        sync={advanced}
        autoFollowSkip="当前分支重放有冲突。"
        autoFollowConflict={{ files: [], lagging: false }}
      />,
    )
    expect(screen.getByText('没有自动跟进：当前分支重放有冲突。')).toBeTruthy()
    expect(screen.queryByRole('alert')).toBeNull()
    expect(screen.queryByText('SVN 侧的新内容：')).toBeNull()
  })

  it('原因只挂在「SVN 侧有新内容」上：分支落后基线的横幅不写', () => {
    render(
      <BaselineBanner
        {...base}
        sync={{ ...advanced, verdict: { kind: 'inSync' }, headBehindBase: true }}
        autoFollowSkip="过时的原因"
      />,
    )
    expect(screen.getByText('当前分支落后基线')).toBeTruthy()
    expect(screen.queryByText(/过时的原因/)).toBeNull()
  })

  it('完成后的提示不拦操作：「查看详情」弹报告，「知道了」关掉；判定已同步时也挂', () => {
    const onDetails = vi.fn()
    const onDismiss = vi.fn()
    render(
      <BaselineBanner
        {...base}
        sync={{ ...advanced, verdict: { kind: 'inSync' } }}
        autoFollowed={{ tone: 'quiet', title: '已自动跟进 SVN 侧的新内容', detail: '跟进了 1 个文件' }}
        onAutoFollowedDetails={onDetails}
        onAutoFollowedDismiss={onDismiss}
      />,
    )
    expect(screen.getByRole('status').textContent).toContain('已自动跟进')
    fireEvent.click(screen.getByRole('button', { name: '查看详情' }))
    fireEvent.click(screen.getByRole('button', { name: '知道了' }))
    expect(onDetails).toHaveBeenCalledTimes(1)
    expect(onDismiss).toHaveBeenCalledTimes(1)
  })

  it('普通 git 仓库不挂自动跟进的提示', () => {
    render(
      <BaselineBanner
        {...base}
        isSvn={false}
        sync={null}
        autoFollowed={{ tone: 'quiet', title: '已自动跟进 SVN 侧的新内容', detail: 'x' }}
      />,
    )
    expect(document.querySelectorAll('.banner-baseline')).toHaveLength(0)
  })
})

describe('BaselineBanner「缺 Local-Only 补丁」横幅（计划 2026-11-12-auto-restore-local-only 的 C）', () => {
  const pending = [{ path: 'Code/A.cs', commits: ['fae50a8 add @avatarclass'] }]
  const idle: SyncState = { locked: false, holder: null, unfinished: null }

  function renderPending(over: Partial<Parameters<typeof BaselineBanner>[0]> = {}) {
    const onRestorePending = vi.fn()
    const onDismissPending = vi.fn()
    render(
      <BaselineBanner
        isSvn
        sync={null}
        audit={null}
        eol={null}
        syncState={idle}
        onRebuild={() => {}}
        onCheck={() => {}}
        onMigrateEol={() => {}}
        onSyncAction={() => {}}
        onSyncOutAction={() => {}}
        onSvnUpdate={() => {}}
        pendingRestores={pending}
        onRestorePending={onRestorePending}
        onDismissPending={onDismissPending}
        {...over}
      />,
    )
    return { onRestorePending, onDismissPending }
  }

  it('列出文件和补丁，「放回」「不放回」各走各的', () => {
    const { onRestorePending, onDismissPending } = renderPending()
    expect(screen.getByText('1 个文件缺 Local-Only 补丁')).toBeTruthy()
    expect(screen.getByText('Code/A.cs ← fae50a8 add @avatarclass')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: '放回' }))
    fireEvent.click(screen.getByRole('button', { name: '不放回' }))
    expect(onRestorePending).toHaveBeenCalledTimes(1)
    expect(onDismissPending).toHaveBeenCalledTimes(1)
  })

  it('放回进行中两颗按钮都禁用', () => {
    renderPending({ restoreBusy: true })
    expect((screen.getByRole('button', { name: '正在放回…' }) as HTMLButtonElement).disabled).toBe(
      true,
    )
    expect((screen.getByRole('button', { name: '不放回' }) as HTMLButtonElement).disabled).toBe(
      true,
    )
  })

  it('空清单不挂', () => {
    renderPending({ pendingRestores: [] })
    expect(screen.queryByText(/缺 Local-Only 补丁/)).toBeNull()
  })

  it('没接「放回」时不挂：留一颗点了没反应的按钮比没有更糟', () => {
    renderPending({ onRestorePending: undefined })
    expect(screen.queryByText(/缺 Local-Only 补丁/)).toBeNull()
  })

  it('同步停在中途时让位', () => {
    renderPending({
      syncState: {
        locked: false,
        holder: null,
        unfinished: { flow: 'follow', phase: 'follow-publishing', opid: 'x', startedAt: 1 },
      },
    })
    expect(screen.queryByText(/缺 Local-Only 补丁/)).toBeNull()
  })

  it('不是 SVN 仓库时不挂', () => {
    renderPending({ isSvn: false })
    expect(screen.queryByText(/缺 Local-Only 补丁/)).toBeNull()
  })
})

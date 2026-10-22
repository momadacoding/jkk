// @vitest-environment jsdom
//
// 五个弹框都接上了 useFocusTrap：框里最后一个可聚焦元素按 Tab 回到第一个，不去背后的主界面
// （2026-10-22 E2E 第 7 条复验时发现：Tab 会把焦点移到菜单栏）。
import { act, fireEvent, render, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { useConfirm, type ConfirmRequest } from '@/components/ConfirmDialog'
import { usePrompt, type PromptRequest } from '@/components/PromptDialog'
import { DeleteWorktreeDialog } from '@/components/DeleteWorktreeDialog'
import { DeleteBranchDialog, type DeleteInfo } from '@/components/DeleteBranchDialog'
import { useDeliverDialog } from '@/components/DeliverDialog'
import { ALL_CAPABILITIES } from '@/engine/capabilities'
import { describeDeliverPreview, type SyncOutPreview } from '@/engine/svnSyncOut'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
const invokeMock = vi.mocked(invoke)
afterEach(() => invokeMock.mockReset())

const FOCUSABLE = 'button:not([disabled]), input:not([disabled]), textarea:not([disabled]), select:not([disabled]), a[href]'

function focusables(): HTMLElement[] {
  const box = document.querySelector('.dialog')
  if (!box) throw new Error('弹框没渲染出来')
  return Array.from(box.querySelectorAll<HTMLElement>(FOCUSABLE))
}

/** 最后一个按 Tab 回到第一个；第一个按 Shift+Tab 到最后一个。 */
function expectWraps() {
  const all = focusables()
  expect(all.length).toBeGreaterThan(1)
  const first = all[0]
  const last = all[all.length - 1]
  last.focus()
  fireEvent.keyDown(last, { key: 'Tab' })
  expect(document.activeElement).toBe(first)
  fireEvent.keyDown(first, { key: 'Tab', shiftKey: true })
  expect(document.activeElement).toBe(last)
}

describe('五个弹框都把 Tab 圈在框里', () => {
  it('ConfirmDialog', () => {
    let confirm!: (r: ConfirmRequest) => Promise<boolean>
    function Harness() {
      const c = useConfirm()
      confirm = c.confirm
      return <>{c.dialog}</>
    }
    render(<Harness />)
    act(() => { void confirm({ title: '合并前确认', message: 'm', danger: true, confirmLabel: '仍然继续', focusCancel: true }) })
    expectWraps()
  })

  it('PromptDialog', () => {
    let prompt!: (r: PromptRequest) => Promise<string | null>
    function Harness() {
      const p = usePrompt()
      prompt = p.prompt
      return <>{p.dialog}</>
    }
    render(<Harness />)
    act(() => { void prompt({ title: '新建分支', message: '名字', defaultValue: 'feat/x' }) })
    expectWraps()
  })

  it('DeleteWorktreeDialog', () => {
    render(
      <DeleteWorktreeDialog
        worktree={{ name: 'wt', path: 'C:/r/.worktrees/wt', head: 'h', branch: 'feat/x', isMain: false }}
        onCancel={() => {}}
        onConfirm={async () => {}}
      />,
    )
    expectWraps()
  })

  it('DeleteBranchDialog（分支信息读回来之后）', async () => {
    const info: DeleteInfo = {
      upstream: null,
      remote: null,
      remoteBranch: null,
      remoteProtected: false,
      unmerged: 0,
      comparedTo: 'main',
      contentMerged: false,
      mergedVia: null,
      mergedInto: null,
      worktree: null,
    }
    invokeMock.mockResolvedValue(info)
    render(
      <DeleteBranchDialog
        repoPath="C:\\repo"
        name="feat/x"
        capabilities={ALL_CAPABILITIES}
        mergedPullRequests={[]}
        onCancel={() => {}}
        onConfirm={async () => {}}
      />,
    )
    await waitFor(() => expect(focusables().length).toBeGreaterThan(1))
    expectWraps()
  })

  it('DeliverDialog', () => {
    const preview: SyncOutPreview = {
      origBranch: 'work',
      origHead: 'h'.repeat(40),
      baselineOid: 'b'.repeat(40),
      baselineRev: 1,
      remoteRev: 1,
      deliverOid: 'd'.repeat(40),
      deliverTree: 't'.repeat(40),
      changes: [{ status: 'M', path: 'Code/A.cs' }],
      localOnly: [],
      outsideChanges: [],
    }
    let deliver!: ReturnType<typeof useDeliverDialog>['deliver']
    function Harness() {
      const d = useDeliverDialog()
      deliver = d.deliver
      return <>{d.dialog}</>
    }
    render(<Harness />)
    act(() => {
      void deliver({
        model: describeDeliverPreview(preview),
        initialMessage: '修复跳跃',
        onMessageChange: () => {},
        action: async () => {},
      })
    })
    expectWraps()
  })
})

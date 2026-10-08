// @vitest-environment jsdom
//
// 组件测试只在这个文件里切到 jsdom，其余纯逻辑测试仍跑在 node 环境（vitest.config.ts）。
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { DeleteBranchDialog, type DeleteInfo } from '@/components/DeleteBranchDialog'
import { ALL_CAPABILITIES } from '@/engine/capabilities'
import type { PullRequest } from '@/engine/types'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
const invokeMock = vi.mocked(invoke)

afterEach(() => {
  invokeMock.mockReset()
})

const INFO: DeleteInfo = {
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

const MERGED_PR: PullRequest = {
  number: 7,
  headRefName: 'feat/x',
  title: 't',
  isDraft: false,
  state: 'MERGED',
  headRefOid: 'a'.repeat(40),
  url: 'https://example.invalid/pr/7',
}

/** 一个手动放行的 Promise：用来让「带合并点的重查」停在半路。 */
function deferred<T>() {
  let resolve!: (v: T) => void
  const promise = new Promise<T>((r) => {
    resolve = r
  })
  return { promise, resolve }
}

function dialog(props: { prs: PullRequest[]; onConfirm?: () => Promise<void> }) {
  return (
    <DeleteBranchDialog
      repoPath="C:\\repo"
      name="feat/x"
      capabilities={ALL_CAPABILITIES}
      mergedPullRequests={props.prs}
      onCancel={() => {}}
      onConfirm={props.onConfirm ?? (async () => {})}
    />
  )
}

function deleteButton(): HTMLButtonElement {
  return screen.getByRole('button', { name: 'Delete' }) as HTMLButtonElement
}

describe('DeleteBranchDialog', () => {
  it('PR 列表晚到触发的重查返回时，不能清掉删除失败的错误（PR 56 Codex review）', async () => {
    // 真实时序：弹框先不带合并点查一次；PR 列表随后到达，带着合并点再查一次，
    // 这一次还在路上时用户点了删除、删除失败——删除按 worktree → 远程 → 本地执行，
    // 失败时可能已经做了一半，这条错误是用户唯一的线索
    const requery = deferred<DeleteInfo>()
    invokeMock.mockResolvedValueOnce(INFO).mockReturnValueOnce(requery.promise)
    const onConfirm = vi.fn(async () => {
      throw new Error('remote rejected')
    })

    const { rerender } = render(dialog({ prs: [], onConfirm }))
    await waitFor(() => expect(deleteButton().disabled).toBe(false))

    rerender(dialog({ prs: [MERGED_PR], onConfirm }))
    await waitFor(() => expect(invokeMock).toHaveBeenCalledTimes(2))

    fireEvent.click(deleteButton())
    expect(await screen.findByText(/remote rejected/)).toBeTruthy()

    await act(async () => {
      requery.resolve(INFO)
      await requery.promise
    })
    expect(screen.queryByText(/remote rejected/)).not.toBeNull()
  })

  it('第一次查询失败、PR 列表晚到后重查成功：旧的查询错误要清掉', async () => {
    invokeMock.mockRejectedValueOnce(new Error('branch info failed')).mockResolvedValueOnce(INFO)

    const { rerender } = render(dialog({ prs: [] }))
    expect(await screen.findByText(/branch info failed/)).toBeTruthy()

    rerender(dialog({ prs: [MERGED_PR] }))
    await waitFor(() => expect(deleteButton().disabled).toBe(false))
    expect(screen.queryByText(/branch info failed/)).toBeNull()
  })
})

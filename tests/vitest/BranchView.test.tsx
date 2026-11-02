// @vitest-environment jsdom
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { BranchView } from '@/components/BranchView'
import { ALL_CAPABILITIES } from '@/engine/capabilities'
import type { RefPointer } from '@/engine/types'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))
const invokeMock = vi.mocked(invoke)

function branch(name: string): RefPointer {
  return { name: `refs/heads/${name}`, hash: 'a'.repeat(40), kind: 'branch' } as RefPointer
}

function renderView(behind: string[], onCheckout = vi.fn()) {
  render(
    <BranchView
      refs={[branch('work'), branch('old')]}
      pullRequests={[]}
      head="work"
      selectedBranch={null}
      onSelect={() => {}}
      onCheckout={onCheckout}
      onCreate={() => {}}
      onRename={() => {}}
      onDelete={async () => {}}
      onCheckoutRemote={() => {}}
      onDeleteRemote={async () => {}}
      repoPath="C:/wc"
      capabilities={ALL_CAPABILITIES}
      behindBaseline={behind}
    />,
  )
  return onCheckout
}

describe('BranchView 落后基线标记', () => {
  it('只给落后的分支打标记', () => {
    renderView(['old'])
    const tags = screen.getAllByText('落后基线')
    expect(tags).toHaveLength(1)
    expect(tags[0].closest('.sb-row')?.textContent).toContain('old')
  })

  it('没有落后的分支时不显示', () => {
    renderView([])
    expect(screen.queryByText('落后基线')).toBeNull()
  })

  it('标记上说清切过去会发生什么', () => {
    renderView(['old'])
    expect(screen.getByText('落后基线').getAttribute('title')).toContain('更早的基线')
  })

  it('切换的入口不变：确认框由 onCheckout 的调用方决定', () => {
    const onCheckout = renderView(['old'])
    fireEvent.doubleClick(screen.getByText('old'))
    expect(onCheckout).toHaveBeenCalledWith('old')
  })
})

describe('BranchView 删远程分支', () => {
  it('选中 Delete remote branch… 就预取远程默认分支，确认之前不删', async () => {
    const onDeleteRemote = vi.fn(async () => {})
    render(
      <BranchView
        refs={[
          branch('work'),
          { name: 'refs/remotes/origin/feat/x', hash: 'b'.repeat(40), kind: 'remote' } as RefPointer,
        ]}
        pullRequests={[]}
        head="work"
        selectedBranch={null}
        onSelect={() => {}}
        onCheckout={() => {}}
        onCreate={() => {}}
        onRename={() => {}}
        onDelete={async () => {}}
        onCheckoutRemote={() => {}}
        onDeleteRemote={onDeleteRemote}
        repoPath="C:/wc"
        capabilities={ALL_CAPABILITIES}
        behindBaseline={[]}
      />,
    )
    // Remote 分组默认收起
    fireEvent.click(screen.getByText('Remote'))
    fireEvent.contextMenu(screen.getByText('origin/feat/x'))
    fireEvent.click(screen.getByText('Delete remote branch…'))

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('prefetch_remote_default_for_ref', {
        path: 'C:/wc',
        remoteBranch: 'origin/feat/x',
      }))
    // 预取只是提速，删除仍然要等用户在确认框里点 Delete
    expect(screen.getByText('Delete Remote Branch')).toBeTruthy()
    expect(onDeleteRemote).not.toHaveBeenCalled()
  })
})

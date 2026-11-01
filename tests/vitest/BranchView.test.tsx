// @vitest-environment jsdom
import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { BranchView } from '@/components/BranchView'
import { ALL_CAPABILITIES } from '@/engine/capabilities'
import type { RefPointer } from '@/engine/types'

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

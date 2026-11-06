// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { SidebarGroup, SidebarSection } from '@/components/SidebarSection'
import { TagView } from '@/components/TagView'
import { SECTION_MEMORY_KEY } from '@/engine/layoutMemory'
import type { RefPointer } from '@/engine/types'

function section(memoryKey?: 'worktrees', defaultOpen?: boolean) {
  return (
    <SidebarSection title="WORKTREES" count={1} memoryKey={memoryKey} defaultOpen={defaultOpen}>
      <div>body</div>
    </SidebarSection>
  )
}

const header = () => screen.getByText('WORKTREES')

beforeEach(() => localStorage.clear())
afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
})

describe('SidebarSection 布局记忆', () => {
  it('没记过时按 defaultOpen', () => {
    render(section('worktrees', false))
    expect(screen.queryByText('body')).toBeNull()
  })

  it('点开后记住，重新挂载（重启 / 新标签）仍是展开的', () => {
    const { unmount } = render(section('worktrees', false))
    fireEvent.click(header())
    expect(screen.getByText('body')).toBeTruthy()
    unmount()

    render(section('worktrees', false))
    expect(screen.getByText('body')).toBeTruthy()
    expect(JSON.parse(localStorage.getItem(SECTION_MEMORY_KEY)!)).toEqual({ worktrees: true })
  })

  it('默认展开的分区被收起后也记住', () => {
    const { unmount } = render(section('worktrees', true))
    fireEvent.click(header())
    unmount()
    render(section('worktrees', true))
    expect(screen.queryByText('body')).toBeNull()
  })

  it('不传 memoryKey 不读也不写存档', () => {
    localStorage.setItem(SECTION_MEMORY_KEY, '{"worktrees":true}')
    const { unmount } = render(section(undefined, false))
    expect(screen.queryByText('body')).toBeNull()
    fireEvent.click(header())
    unmount()
    expect(localStorage.getItem(SECTION_MEMORY_KEY)).toBe('{"worktrees":true}')
  })

  it('存档损坏时回到默认值，不影响渲染', () => {
    localStorage.setItem(SECTION_MEMORY_KEY, '{oops')
    render(section('worktrees', false))
    expect(screen.queryByText('body')).toBeNull()
    fireEvent.click(header())
    expect(screen.getByText('body')).toBeTruthy()
    expect(JSON.parse(localStorage.getItem(SECTION_MEMORY_KEY)!)).toEqual({ worktrees: true })
  })

  it('localStorage 读写抛错时照常折叠展开，只是记不住', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => { throw new Error('denied') })
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new Error('denied') })
    render(section('worktrees', false))
    fireEvent.click(header())
    expect(screen.getByText('body')).toBeTruthy()
  })

  it('不同分区各记各的', () => {
    const tree = () => (
      <>
        {section('worktrees', false)}
        <SidebarGroup label="Remote" count={1} defaultOpen={false} memoryKey="branches.remote">
          <div>remote-body</div>
        </SidebarGroup>
      </>
    )
    const { unmount } = render(tree())
    fireEvent.click(screen.getByText('Remote'))
    unmount()
    render(tree())
    expect(screen.getByText('remote-body')).toBeTruthy()
    expect(screen.queryByText('body')).toBeNull()
  })
})

describe('TagView', () => {
  const tag: RefPointer = {
    name: 'refs/tags/v1.0.0', hash: 'a'.repeat(40), kind: 'tag', ahead: null, behind: null, goneUpstream: null,
  }

  function renderTags() {
    return render(
      <TagView
        tags={[tag]}
        commits={[]}
        selectedTag={null}
        onSelect={() => {}}
        onCheckout={() => {}}
        onPush={() => {}}
      />,
    )
  }

  it('默认收起，标题和计数仍可见', () => {
    renderTags()
    expect(screen.getByText('TAGS')).toBeTruthy()
    expect(screen.queryByText('v1.0.0')).toBeNull()
  })

  it('点开后可见，但不记住：重新挂载（重启 / 新标签）又是收起的', () => {
    const { unmount } = renderTags()
    fireEvent.click(screen.getByText('TAGS'))
    expect(screen.getByText('v1.0.0')).toBeTruthy()
    unmount()
    renderTags()
    expect(screen.queryByText('v1.0.0')).toBeNull()
    expect(localStorage.getItem(SECTION_MEMORY_KEY)).toBeNull()
  })

  it('旧版本存档里残留的 tags:true 不再让它展开', () => {
    localStorage.setItem(SECTION_MEMORY_KEY, '{"tags":true}')
    renderTags()
    expect(screen.queryByText('v1.0.0')).toBeNull()
  })
})

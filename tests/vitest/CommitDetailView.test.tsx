// @vitest-environment jsdom
//
// svn 节点详情里「这次交付包含的本地提交」：能点开单个提交，点开后能回到 svn 节点（2026-10-16 用户要求）。
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { CommitDetailView } from '@/components/CommitDetailView'
import type { ExternalToolsConfig } from '@/engine/externalTools'
import type { CommitNode, DiffData } from '@/engine/types'

// 文件树 + diff 的分栏（react-resizable-panels）要 ResizeObserver，jsdom 没有。这里只看文字，不量尺寸
globalThis.ResizeObserver ??= class {
  observe() {}
  unobserve() {}
  disconnect() {}
}

const git = (id: string, message: string): CommitNode => ({
  id,
  parents: ['B15'],
  author: 'me',
  time: 1_700_000_000,
  message,
})

/** r16 的真实形状：一次交付三个提交，SVN 信息与其中一个相同；前一版没有本地记录，不进 diff 布局。 */
const r16 = (deliveredFrom: CommitNode[], message = 'demo3 修跳跃'): CommitNode => ({
  id: 'svn:16',
  parents: ['svn:15'],
  author: 'liuhao',
  time: 1_700_000_100,
  message,
  vcs: 'svn',
  svnRev: 16,
  deliveredFrom,
})

const three = [
  git('91c3aab000', 'demo3 调参数'),
  git('c47c8cb000', 'demo3 修音效'),
  git('385dd64000', 'demo3 修跳跃'),
]

const empty = { files: [] }

describe('CommitDetailView：svn 节点收着的本地提交', () => {
  it('多个提交：写明个数，每个一行，点哪个就选中哪个', () => {
    const onSelectCommit = vi.fn()
    render(<CommitDetailView commit={r16(three)} diff={empty} repoPath="C:/r" onSelectCommit={onSelectCommit} />)
    expect(screen.getByText('这次交付包含 3 个本地提交：')).toBeTruthy()
    const rows = screen.getAllByRole('button', { name: /demo3/ })
    expect(rows.map((r) => r.textContent)).toEqual([
      '91c3aab demo3 调参数',
      'c47c8cb demo3 修音效',
      '385dd64 demo3 修跳跃',
    ])
    fireEvent.click(rows[1])
    expect(onSelectCommit).toHaveBeenCalledWith('c47c8cb000')
  })

  it('一个提交、交付时改写过信息：说「来自」', () => {
    render(<CommitDetailView commit={r16([three[0]], '另写的信息')} diff={empty} repoPath="C:/r" onSelectCommit={vi.fn()} />)
    expect(screen.getByText('这次交付来自本地提交：')).toBeTruthy()
    expect(screen.getByRole('button', { name: /demo3 调参数/ })).toBeTruthy()
  })

  it('一个提交、信息相同、下面就有它的改动：整块不出现', () => {
    const commit = { ...r16([three[2]]), baselineOid: 'B16', diffBase: 'B15' }
    render(<CommitDetailView commit={commit} diff={empty} repoPath="C:/r" onSelectCommit={vi.fn()} />)
    expect(screen.queryByText(/本地提交/)).toBeNull()
  })
})

describe('CommitDetailView：本机算不出文件变化时的说明', () => {
  // 2026-10-16 review 第 8 条：没在本机记过的 revision 也可能是自己交付的（交付后同事紧跟着提交），
  // 不能猜「多半是同事的提交」
  it('自己没记过：说「这个 revision」，不猜是谁的', () => {
    render(<CommitDetailView commit={r16([])} diff={empty} repoPath="C:/r" onSelectCommit={vi.fn()} />)
    const note = screen.getByText(/本机没有记下/)
    expect(note.textContent).toContain('r16')
    expect(note.textContent).not.toContain('同事')
  })

  it('前一个没记过：点名前一个 revision', () => {
    const commit = { ...r16([]), baselineOid: 'B16' }
    render(<CommitDetailView commit={commit} diff={empty} repoPath="C:/r" onSelectCommit={vi.fn()} />)
    const note = screen.getByText(/本机没有记下/)
    expect(note.textContent).toContain('r15')
    expect(note.textContent).not.toContain('同事')
  })
})

describe('CommitDetailView：点开收着的提交之后', () => {
  it('详情是这个 git 提交本身，顶上有「返回 r16」，点了回到 svn 节点', () => {
    const onSelectCommit = vi.fn()
    render(
      <CommitDetailView
        commit={three[1]}
        diff={empty}
        repoPath="C:/r"
        backTo={r16(three)}
        onSelectCommit={onSelectCommit}
      />,
    )
    expect(screen.getByText('c47c8cb')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: '← 返回 r16' }))
    expect(onSelectCommit).toHaveBeenCalledWith('svn:16')
  })

  it('普通 git 提交没有返回按钮', () => {
    render(<CommitDetailView commit={three[1]} diff={empty} repoPath="C:/r" onSelectCommit={vi.fn()} />)
    expect(screen.queryByRole('button', { name: /返回/ })).toBeNull()
  })
})

describe('CommitDetailView：所在分支', () => {
  // 2026-10-17 用户要求：车道一多，点了提交认不出它在哪条分支上
  it('信息条上显示分支，切到 Changes / File Tree 也还在', () => {
    render(<CommitDetailView commit={three[1]} diff={empty} repoPath={null} onSelectCommit={vi.fn()} branch="已合并进 master" />)
    expect(screen.getByText('⑂ 已合并进 master')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: /Changes/ }))
    expect(screen.getByText('⑂ 已合并进 master')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'File Tree' }))
    expect(screen.getByText('⑂ 已合并进 master')).toBeTruthy()
  })

  it('svn 节点也显示', () => {
    render(<CommitDetailView commit={r16([])} diff={empty} repoPath="C:/r" onSelectCommit={vi.fn()} branch="SVN 主线" />)
    expect(screen.getByText('⑂ SVN 主线')).toBeTruthy()
  })

  it('图外提交不传分支时整栏不出现', () => {
    render(<CommitDetailView commit={three[1]} diff={empty} repoPath="C:/r" onSelectCommit={vi.fn()} branch={null} />)
    expect(screen.queryByText(/⑂/)).toBeNull()
  })
})

describe('CommitDetailView：外部 diff 菜单', () => {
  const renamed: DiffData = {
    files: [{ path: 'n.txt', status: 'renamed', oldPath: 'o.txt', binary: false, hunks: [] }],
  }
  const config: ExternalToolsConfig = {
    version: 1,
    defaultDiff: 'bc',
    defaultMerge: 'bc',
    rules: [],
    tools: [{ id: 'bc', label: 'Beyond Compare', program: 'BComp.exe', diffArgs: ['$LOCAL', '$REMOTE'] }],
  }

  function mount(commit: CommitNode, diffScope: string | null) {
    const onDiff = vi.fn()
    render(
      <CommitDetailView
        commit={commit}
        diff={renamed}
        repoPath="C:/r"
        onSelectCommit={vi.fn()}
        diffScope={diffScope}
        externalTools={{ config, onDiff, onEditConfig: vi.fn() }}
      />,
    )
    return onDiff
  }

  it('commit:abc：右键文件拿到 commit 目标，改名带上旧路径', async () => {
    const onDiff = mount(three[1], 'commit:abc')
    fireEvent.click(screen.getByRole('button', { name: /Changes/ }))
    fireEvent.contextMenu(screen.getByText('n.txt'))
    fireEvent.click(screen.getByText('在 Beyond Compare 中对比'))
    await waitFor(() =>
      expect(onDiff).toHaveBeenCalledWith('bc', { kind: 'commit', oid: 'abc', file: 'n.txt', oldFile: 'o.txt' }),
    )
  })

  it('range:a..b（svn 节点两版基线之差）拿到 range 目标', async () => {
    const commit = { ...r16([]), baselineOid: 'B16', diffBase: 'B15' }
    const onDiff = mount(commit, 'range:a..b')
    fireEvent.contextMenu(screen.getByText('n.txt'))
    fireEvent.click(screen.getByText('在 Beyond Compare 中对比'))
    await waitFor(() =>
      expect(onDiff).toHaveBeenCalledWith('bc', { kind: 'range', from: 'a', to: 'b', file: 'n.txt', oldFile: 'o.txt' }),
    )
  })

  it('diffScope 为 null 时没有对比项', () => {
    mount(three[1], null)
    fireEvent.click(screen.getByRole('button', { name: /Changes/ }))
    fireEvent.contextMenu(screen.getByText('n.txt'))
    expect(screen.queryByText(/中对比/)).toBeNull()
  })
})

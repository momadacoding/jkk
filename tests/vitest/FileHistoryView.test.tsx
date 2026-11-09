// @vitest-environment jsdom
//
// 文件历史 / Blame（计划 2026-11-08-file-history-blame）：视图本身与三个入口的右键菜单。
// store 的 invoke 包装换成桩，这里只看「拿什么参数去读」和「读回来怎么显示」。
import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import type { BlameData, FileHistory, FileHistoryTarget } from '@/engine/fileHistory'
import { UNCOMMITTED, historyTargetForCommit, historyTargetForWorkdir } from '@/engine/fileHistory'
import type { CommitNode, DiffData, FileChange, StatusData } from '@/engine/types'

const store = vi.hoisted(() => ({
  readFileHistory: vi.fn(),
  readBlame: vi.fn(),
  readCommitFileDiff: vi.fn(),
}))
vi.mock('@/store/repoStore', () => store)

import { FileHistoryView } from '@/components/FileHistoryView'
import { CommitDetailView } from '@/components/CommitDetailView'
import { LocalChangesView } from '@/components/LocalChangesView'

beforeAll(() => {
  vi.stubGlobal('ResizeObserver', class {
    observe() {}
    unobserve() {}
    disconnect() {}
  })
})

const A = 'a'.repeat(40)
const B = 'b'.repeat(40)
const C = 'c'.repeat(40)

/** 三条：C 改内容、B 把 old.txt 改名成 new.txt、A 新增 old.txt。 */
const history: FileHistory = {
  truncated: false,
  entries: [
    { id: C, parents: [B], author: 'carol', time: 300, subject: '改内容', path: 'new.txt', oldPath: null, status: 'modified' },
    { id: B, parents: [A], author: 'bob', time: 200, subject: '改名', path: 'new.txt', oldPath: 'old.txt', status: 'renamed' },
    { id: A, parents: [], author: 'alice', time: 100, subject: '新增', path: 'old.txt', oldPath: null, status: 'added' },
  ],
}

const blame: BlameData = {
  binary: false,
  truncated: false,
  totalLines: 3,
  lines: [
    { commit: A, origLine: 1, finalLine: 1, origPath: 'old.txt', content: 'one' },
    { commit: A, origLine: 2, finalLine: 2, origPath: 'old.txt', content: 'two' },
    { commit: UNCOMMITTED, origLine: 3, finalLine: 3, origPath: 'new.txt', content: 'work' },
  ],
  commits: {
    [A]: { author: 'alice', time: 100, summary: '新增', previous: null },
    [UNCOMMITTED]: { author: 'Not Committed Yet', time: 400, summary: '', previous: { commit: C, path: 'new.txt' } },
  },
}

const emptyDiff: DiffData = { files: [] }

beforeEach(() => {
  store.readFileHistory.mockReset().mockResolvedValue(history)
  store.readBlame.mockReset().mockResolvedValue(blame)
  store.readCommitFileDiff.mockReset().mockResolvedValue(emptyDiff)
})

function mountView(target: FileHistoryTarget) {
  const onReveal = vi.fn()
  const onClose = vi.fn()
  render(
    <FileHistoryView repoPath="C:/r" target={target} headHash={C} onClose={onClose} onRevealCommit={onReveal} />,
  )
  return { onReveal, onClose }
}

describe('FileHistoryView：History', () => {
  it('按打开时的起点读历史，选中打开它的那个提交，按那一条的路径读 diff', async () => {
    mountView(historyTargetForCommit('new.txt', { id: B }, 'renamed', 'history'))
    expect(store.readFileHistory).toHaveBeenCalledWith('C:/r', 'new.txt', ['HEAD', B])
    await screen.findByRole('option', { name: /改名/ })
    // 改名那一条要带上旧路径，diff 才配得成一对
    await waitFor(() => expect(store.readCommitFileDiff).toHaveBeenCalledWith('C:/r', B, 'new.txt', 'old.txt'))
    expect(screen.getByRole('option', { name: /改名/ }).getAttribute('aria-selected')).toBe('true')
    expect(screen.getByText('History (3)')).toBeTruthy()
  })

  it('点另一条：用那一条当时的路径读 diff；双击在提交图中定位', async () => {
    const { onReveal } = mountView(historyTargetForWorkdir('new.txt', null, 'history'))
    fireEvent.click(await screen.findByRole('option', { name: /新增/ }))
    await waitFor(() => expect(store.readCommitFileDiff).toHaveBeenCalledWith('C:/r', A, 'old.txt', null))
    fireEvent.doubleClick(screen.getByRole('option', { name: /新增/ }))
    expect(onReveal).toHaveBeenCalledWith(A)
  })

  it('「Blame 这一版」：Blame 那个提交里那时的路径', async () => {
    mountView(historyTargetForWorkdir('new.txt', null, 'history'))
    fireEvent.click(await screen.findByRole('option', { name: /新增/ }))
    fireEvent.click(screen.getByRole('button', { name: 'Blame 这一版' }))
    await waitFor(() => expect(store.readBlame).toHaveBeenCalledWith('C:/r', 'old.txt', A))
  })

  it('打开时的提交不在 --follow 的列表里（合并提交）：说明原因，选中最新一条', async () => {
    const merge = 'd'.repeat(40)
    mountView(historyTargetForCommit('new.txt', { id: merge }, 'modified', 'history'))
    expect(await screen.findByText(/打开时的提交 ddddddd 不在下面的列表里/)).toBeTruthy()
    expect(screen.getByRole('option', { name: /改内容/ }).getAttribute('aria-selected')).toBe('true')
  })

  it('打开时的提交在列表里：没有那条说明', async () => {
    mountView(historyTargetForCommit('new.txt', { id: B }, 'renamed', 'history'))
    await screen.findByRole('option', { name: /改名/ })
    expect(screen.queryByText(/不在下面的列表里/)).toBeNull()
  })

  it('没有历史时说明原因，读失败时说读取失败', async () => {
    store.readFileHistory.mockResolvedValueOnce({ entries: [], truncated: false })
    mountView(historyTargetForWorkdir('x.txt', null, 'history'))
    expect(await screen.findByText(/没有找到改动过这个文件的提交/)).toBeTruthy()
  })

  it('读失败', async () => {
    store.readFileHistory.mockRejectedValueOnce({ code: 'git_cli', message: 'fatal: boom' })
    mountView(historyTargetForWorkdir('x.txt', null, 'history'))
    expect(await screen.findByText(/读取文件历史失败/)).toBeTruthy()
  })

  it('截断时标签带 +、列表底下说明', async () => {
    store.readFileHistory.mockResolvedValueOnce({ ...history, truncated: true })
    mountView(historyTargetForWorkdir('new.txt', null, 'history'))
    expect(await screen.findByText('History (3+)')).toBeTruthy()
    expect(screen.getByText('只列出最近 3 条。')).toBeTruthy()
  })
})

describe('FileHistoryView：Blame', () => {
  it('从工作区打开：blame 工作区，同一提交连着的行只在块首显示一次，未提交的行单独标', async () => {
    mountView(historyTargetForWorkdir('new.txt', null, 'blame'))
    await waitFor(() => expect(store.readBlame).toHaveBeenCalledWith('C:/r', 'new.txt', null))
    expect(await screen.findByText('two')).toBeTruthy()
    expect(screen.getAllByText('aaaaaaa')).toHaveLength(1)
    expect(screen.getByText('未提交')).toBeTruthy()
    expect(screen.getByText(/工作区（含未提交的改动）/)).toBeTruthy()
  })

  it('点块首：历史里有这个提交就跳到 History 选中它', async () => {
    mountView(historyTargetForWorkdir('new.txt', null, 'blame'))
    fireEvent.click(await screen.findByText('aaaaaaa'))
    await waitFor(() =>
      expect(screen.getByRole('option', { name: /新增/ }).getAttribute('aria-selected')).toBe('true'),
    )
  })

  it('右键「Blame 这次改动之前的版本」：换到 previous 那一版，并能回到打开时的版本', async () => {
    mountView(historyTargetForWorkdir('new.txt', null, 'blame'))
    fireEvent.contextMenu(await screen.findByText('work'))
    fireEvent.click(screen.getByText('Blame 这次改动之前的版本'))
    await waitFor(() => expect(store.readBlame).toHaveBeenCalledWith('C:/r', 'new.txt', C))
    fireEvent.click(await screen.findByRole('button', { name: /回到 工作区/ }))
    await waitFor(() => expect(store.readBlame).toHaveBeenLastCalledWith('C:/r', 'new.txt', null))
  })

  it('根提交没有 previous：那一项置灰；未提交的行不能定位、不能复制 hash', async () => {
    mountView(historyTargetForWorkdir('new.txt', null, 'blame'))
    fireEvent.contextMenu(await screen.findByText('one'))
    expect((screen.getByText('Blame 这次改动之前的版本') as HTMLButtonElement).disabled).toBe(true)
    fireEvent.keyDown(window, { key: 'Escape' })
    fireEvent.contextMenu(screen.getByText('work'))
    expect((screen.getByText('在提交图中定位') as HTMLButtonElement).disabled).toBe(true)
    expect((screen.getByText('复制提交 hash') as HTMLButtonElement).disabled).toBe(true)
  })

  it('Blame 工作区：工作区一变就重读，期间留着旧内容不闪（/code-review 第 3 条）', async () => {
    const target = historyTargetForWorkdir('new.txt', null, 'blame')
    const view = (version: object) => (
      <FileHistoryView repoPath="C:/r" target={target} headHash={C} workdirVersion={version} onClose={vi.fn()} onRevealCommit={vi.fn()} />
    )
    const { rerender } = render(view({}))
    await screen.findByText('work')
    expect(store.readBlame).toHaveBeenCalledTimes(1)
    let release: (d: BlameData) => void = () => {}
    store.readBlame.mockReturnValueOnce(new Promise<BlameData>((r) => { release = r }))
    rerender(view({}))
    await waitFor(() => expect(store.readBlame).toHaveBeenCalledTimes(2))
    expect(screen.getByText('work')).toBeTruthy()
    expect(screen.queryByText('正在 Blame…')).toBeNull()
    release({ ...blame, lines: [{ ...blame.lines[2], content: 'edited' }] })
    expect(await screen.findByText('edited')).toBeTruthy()
  })

  it('Blame 固定版本：工作区变化不重读', async () => {
    const target = historyTargetForCommit('new.txt', { id: C }, 'modified', 'blame')
    const view = (version: object) => (
      <FileHistoryView repoPath="C:/r" target={target} headHash={C} workdirVersion={version} onClose={vi.fn()} onRevealCommit={vi.fn()} />
    )
    const { rerender } = render(view({}))
    await screen.findByText('work')
    rerender(view({}))
    await new Promise((r) => setTimeout(r, 20))
    expect(store.readBlame).toHaveBeenCalledTimes(1)
  })

  it('二进制文件不按行显示', async () => {
    store.readBlame.mockResolvedValueOnce({ ...blame, binary: true, lines: [], commits: {} })
    mountView(historyTargetForWorkdir('a.png', null, 'blame'))
    expect(await screen.findByText('二进制文件，无法按行 Blame')).toBeTruthy()
  })

  it('Blame 失败时说明', async () => {
    store.readBlame.mockRejectedValueOnce({ code: 'git_cli', message: "fatal: no such path 'x' in HEAD" })
    mountView(historyTargetForWorkdir('x', null, 'blame'))
    expect(await screen.findByText(/Blame 失败/)).toBeTruthy()
  })
})

describe('入口：提交详情', () => {
  const commit: CommitNode = { id: B, parents: [A], author: 'bob', time: 200, message: '改名' }
  const diff: DiffData = {
    files: [
      { path: 'new.txt', status: 'renamed', oldPath: 'old.txt', binary: false, hunks: [] },
      { path: 'gone.txt', status: 'deleted', oldPath: null, binary: false, hunks: [] },
    ],
  }

  function mountDetail() {
    const onOpen = vi.fn()
    render(
      <CommitDetailView commit={commit} diff={diff} repoPath="C:/r" onSelectCommit={vi.fn()} onOpenFileHistory={onOpen} />,
    )
    fireEvent.click(screen.getByRole('button', { name: /Changes/ }))
    return onOpen
  }

  it('Changes 里右键文件 → File History：从 HEAD 和这个提交出发，选中这个提交', () => {
    const onOpen = mountDetail()
    fireEvent.contextMenu(screen.getByText('new.txt'))
    fireEvent.click(screen.getByText('File History'))
    return waitFor(() =>
      expect(onOpen).toHaveBeenCalledWith(historyTargetForCommit('new.txt', { id: B }, 'renamed', 'history')),
    )
  })

  it('这个提交删掉的文件 → Blame：看父提交里那一版', () => {
    const onOpen = mountDetail()
    fireEvent.contextMenu(screen.getByText('gone.txt'))
    fireEvent.click(screen.getByText('Blame'))
    return waitFor(() =>
      expect(onOpen).toHaveBeenCalledWith(expect.objectContaining({ blame: { rev: `${B}^`, path: 'gone.txt' }, tab: 'blame' })),
    )
  })
})

describe('入口：Local Changes', () => {
  const file = (path: string, status: FileChange['status']): FileChange => ({ path, status, additions: 1, deletions: 0 })

  function mountChanges() {
    const onOpen = vi.fn()
    // s.txt：暂存了新增、又在工作区改过——Unstaged 里是 modified，但 HEAD 里没有它
    const status: StatusData = {
      unstaged: [file('m.txt', 'modified'), file('n.txt', 'added'), file('s.txt', 'modified')],
      staged: [file('s.txt', 'added')],
    }
    render(
      <LocalChangesView
        status={status}
        diff={{ files: [] }}
        onStage={vi.fn()}
        onUnstage={vi.fn()}
        onDiscard={vi.fn()}
        onCommit={vi.fn()}
        svnMode={false}
        onBaselineBranch={false}
        onOpenFileHistory={onOpen}
      />,
    )
    return onOpen
  }

  it('改过的文件 → Blame：看工作区', () => {
    const onOpen = mountChanges()
    fireEvent.contextMenu(screen.getByText('m.txt'))
    fireEvent.click(screen.getByText('Blame'))
    return waitFor(() => expect(onOpen).toHaveBeenCalledWith(historyTargetForWorkdir('m.txt', null, 'blame')))
  })

  it('在 Staged 里右键「暂存了修改、工作区又删掉」的文件 → Blame HEAD（/code-review 第 1 条）', () => {
    const onOpen = vi.fn()
    render(
      <LocalChangesView
        status={{ unstaged: [file('d.txt', 'deleted')], staged: [file('d.txt', 'modified')] }}
        diff={{ files: [] }}
        onStage={vi.fn()}
        onUnstage={vi.fn()}
        onDiscard={vi.fn()}
        onCommit={vi.fn()}
        svnMode={false}
        onBaselineBranch={false}
        onOpenFileHistory={onOpen}
      />,
    )
    // 两个区各有一行 d.txt：第二个是 Staged 那一行
    fireEvent.contextMenu(screen.getAllByText('d.txt')[1])
    fireEvent.click(screen.getByText('Blame'))
    return waitFor(() => expect(onOpen).toHaveBeenCalledWith(historyTargetForWorkdir('d.txt', 'HEAD', 'blame')))
  })

  it('新文件：两项置灰并说明原因', () => {
    mountChanges()
    fireEvent.contextMenu(screen.getByText('n.txt'))
    const item = screen.getByText('File History（新文件还没有历史）') as HTMLButtonElement
    expect(item.disabled).toBe(true)
  })

  it('暂存了新增又改过的文件：在 Unstaged 里右键也置灰（独立 review 第 4 条）', () => {
    mountChanges()
    const rows = screen.getAllByText('s.txt')
    fireEvent.contextMenu(rows[0])
    expect((screen.getByText('Blame（新文件还没有历史）') as HTMLButtonElement).disabled).toBe(true)
  })
})

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
  readSvnFileHistory: vi.fn(),
  readSvnRevFileDiff: vi.fn(),
  readSvnBlameOverlay: vi.fn(),
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
/** diff 面板开关的默认值对应的后端选项（`backendDiffOptions(DEFAULT_DIFF_PREFS)`）。 */
const DEFAULT_OPTIONS = { contextLines: 3, ignoreWhitespace: false }

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
    await waitFor(() => expect(store.readCommitFileDiff).toHaveBeenCalledWith('C:/r', B, 'new.txt', 'old.txt', DEFAULT_OPTIONS))
    expect(screen.getByRole('option', { name: /改名/ }).getAttribute('aria-selected')).toBe('true')
    expect(screen.getByText('History (3)')).toBeTruthy()
  })

  it('点另一条：用那一条当时的路径读 diff；双击在提交图中定位', async () => {
    const { onReveal } = mountView(historyTargetForWorkdir('new.txt', null, 'history'))
    fireEvent.click(await screen.findByRole('option', { name: /新增/ }))
    await waitFor(() => expect(store.readCommitFileDiff).toHaveBeenCalledWith('C:/r', A, 'old.txt', null, DEFAULT_OPTIONS))
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

// ---------------------------------------------------------------------------
// 混合仓库：SVN 补全（计划 2026-11-27-svn-file-history Task 9）
// ---------------------------------------------------------------------------

const M = 'd'.repeat(40) // 本地提交（未交付）
const D = 'e'.repeat(40) // 本地提交（已交付 r7）
const S = 'f'.repeat(40) // 更新录的基线，A.cs 在 r9
const I = '1'.repeat(40) // 初始化录的基线，A.cs 在 r3

const svnHistory: FileHistory = {
  truncated: false,
  entries: [
    { id: M, parents: [S], author: 'me', time: 500, subject: '我的改动', path: 'Code/A.cs', oldPath: null, status: 'modified', svn: { baseline: null, delivered: { kind: 'undelivered' } } },
    { id: S, parents: [D], author: 'me', time: 400, subject: 'svn: r9', path: 'Code/A.cs', oldPath: null, status: 'modified', svn: { baseline: { fileRev: 9, floor: 9, deliveredRev: null }, delivered: null } },
    { id: D, parents: [I], author: 'me', time: 300, subject: '交付过的改动', path: 'Code/A.cs', oldPath: null, status: 'modified', svn: { baseline: null, delivered: { kind: 'delivered', rev: 7 } } },
    { id: I, parents: [], author: 'me', time: 100, subject: 'svn: r3', path: 'Code/A.cs', oldPath: null, status: 'added', svn: { baseline: { fileRev: 3, floor: 3, deliveredRev: null }, delivered: null } },
  ],
}

const svnRev = (rev: number, author: string, message: string) => ({
  rev,
  author,
  date: '2026-11-20T00:00:00.000000Z',
  message,
})

const svnTarget = (tab: 'history' | 'blame'): FileHistoryTarget => ({
  path: 'Code/A.cs',
  revs: ['HEAD'],
  focus: null,
  blame: { rev: null, path: 'Code/A.cs' },
  tab,
})

/** 手动放行的 Promise：用来让「晚到的旧响应」真的晚到。 */
function deferred<T>() {
  let resolve!: (v: T) => void
  const promise = new Promise<T>((r) => { resolve = r })
  return { promise, resolve }
}

function mountSvn(tab: 'history' | 'blame', props: { headHash?: string; workdirVersion?: unknown } = {}) {
  const view = (p: { headHash?: string; workdirVersion?: unknown }) => (
    <FileHistoryView
      repoPath="C:/r"
      target={svnTarget(tab)}
      headHash={p.headHash ?? M}
      workdirVersion={p.workdirVersion ?? 1}
      onClose={vi.fn()}
      onRevealCommit={vi.fn()}
    />
  )
  const r = render(view(props))
  return { rerender: (p: { headHash?: string; workdirVersion?: unknown }) => r.rerender(view(p)) }
}

/** 每次读回一个新对象，与真实的 invoke 一样（同一个引用 React 不会重渲染）。 */
const fresh = <T,>(v: T) => () => Promise.resolve(JSON.parse(JSON.stringify(v)) as T)

describe('FileHistoryView：混合仓库的 SVN 补全', () => {
  beforeEach(() => {
    store.readFileHistory.mockReset().mockImplementation(fresh(svnHistory))
    store.readSvnFileHistory.mockReset().mockResolvedValue({
      groups: {
        [S]: [svnRev(8, 'gao', '调整打包参数')],
        [I]: [svnRev(3, 'pan', '初始版本'), svnRev(2, 'pan', '更早')],
      },
      cursor: null,
      error: null,
    })
    store.readSvnRevFileDiff.mockReset().mockResolvedValue(emptyDiff)
    store.readSvnBlameOverlay.mockReset().mockResolvedValue([])
  })

  it('expands a baseline row into svn entries', async () => {
    mountSvn('history')
    expect(await screen.findByText('调整打包参数')).toBeTruthy()
    expect(screen.getByText('1 次 SVN 提交')).toBeTruthy()
    expect(screen.getByText('2 次 SVN 提交')).toBeTruthy()
    expect(store.readSvnFileHistory).toHaveBeenCalledWith(
      'C:/r',
      [
        { id: S, path: 'Code/A.cs', fileRev: 9 },
        { id: I, path: 'Code/A.cs', fileRev: 3 },
      ],
      [7],
      null,
    )
  })

  it('labels undelivered and delivered commits', async () => {
    mountSvn('history')
    expect(await screen.findByText('未交付')).toBeTruthy()
    expect(screen.getByText('已交付 r7')).toBeTruthy()
  })

  it('shows load more for the oldest group and appends the next page', async () => {
    const cursor = { pegPath: 'Code/A.cs', pegRev: 3, nextTop: 1 }
    store.readSvnFileHistory
      .mockResolvedValueOnce({ groups: { [I]: [svnRev(3, 'pan', '初始版本')] }, cursor, error: null })
      .mockResolvedValueOnce({ groups: { [I]: [svnRev(1, 'pan', '最早')] }, cursor: null, error: null })
    mountSvn('history')
    expect(await screen.findByText('已加载 1 次 SVN 提交…')).toBeTruthy()
    fireEvent.click(screen.getByText('加载更多'))
    expect(await screen.findByText('最早')).toBeTruthy()
    expect(store.readSvnFileHistory).toHaveBeenLastCalledWith('C:/r', expect.any(Array), [7], cursor)
    expect(screen.queryByText('加载更多')).toBeNull()
    expect(screen.getByText('2 次 SVN 提交')).toBeTruthy()
  })

  it('shows the offline banner when the page has svn_unreachable', async () => {
    store.readSvnFileHistory.mockResolvedValue({
      groups: {},
      cursor: { pegPath: 'Code/A.cs', pegRev: 9, nextTop: 9 },
      error: { code: 'svn_unreachable', message: '等了 15 秒，SVN 服务器没有响应' },
    })
    mountSvn('history')
    expect(await screen.findByText('连不上 SVN，基线条目显示的是快照')).toBeTruthy()
    expect(screen.getAllByText('SVN 明细没取到：等了 15 秒，SVN 服务器没有响应')).toHaveLength(2)
  })

  it('clicking an svn entry reads its diff with the group peg', async () => {
    mountSvn('history')
    fireEvent.click(await screen.findByText('调整打包参数'))
    await waitFor(() => expect(store.readSvnRevFileDiff).toHaveBeenCalledWith('C:/r', 'Code/A.cs', 9, 8, DEFAULT_OPTIONS))
    expect((screen.getByText('Blame 这一版') as HTMLButtonElement).disabled).toBe(true)
  })

  it('stale svn response is ignored', async () => {
    const late = deferred<unknown>()
    store.readSvnFileHistory
      .mockReturnValueOnce(late.promise)
      .mockResolvedValueOnce({ groups: { [S]: [svnRev(8, 'gao', '新的那次')] }, cursor: null, error: null })
    const { rerender } = mountSvn('history')
    await waitFor(() => expect(store.readSvnFileHistory).toHaveBeenCalledTimes(1))
    rerender({ headHash: 'x'.repeat(40) })
    expect(await screen.findByText('新的那次')).toBeTruthy()
    // 换一个 revision：同号会被 mergePage 去重，测不出丢没丢弃
    late.resolve({ groups: { [S]: [svnRev(5, 'gao', '旧的那次')] }, cursor: null, error: null })
    await new Promise((r) => setTimeout(r, 0))
    expect(screen.queryByText('旧的那次')).toBeNull()
  })

  const svnBlame: BlameData = {
    binary: false,
    truncated: false,
    totalLines: 3,
    lines: [
      { commit: I, origLine: 1, finalLine: 1, origPath: 'Code/A.cs', content: 'one' },
      { commit: I, origLine: 2, finalLine: 2, origPath: 'Code/A.cs', content: 'two' },
      { commit: M, origLine: 3, finalLine: 3, origPath: 'Code/A.cs', content: 'mine' },
    ],
    commits: {
      [I]: { author: 'me', time: 100, summary: 'svn: r3', previous: null, baseline: { fileRev: 3, floor: 3, deliveredRev: null }, delivered: null },
      [M]: { author: 'me', time: 500, summary: '我的改动', previous: null, baseline: null, delivered: { kind: 'undelivered' } },
    },
  }

  it('blame maps baseline lines to svn authors and keeps git lines', async () => {
    store.readBlame.mockReset().mockResolvedValue(svnBlame)
    store.readSvnBlameOverlay.mockResolvedValue([
      { kind: 'mapped', lines: [{ rev: 2, author: 'ann', date: '2026-11-01T00:00:00Z' }, { rev: 3, author: 'bob', date: '2026-11-02T00:00:00Z' }] },
    ])
    mountSvn('blame')
    expect(await screen.findByText('ann')).toBeTruthy()
    expect(screen.getByText('bob')).toBeTruthy()
    expect(screen.getByText('r2')).toBeTruthy()
    expect(store.readSvnBlameOverlay).toHaveBeenCalledWith('C:/r', [{ baselineId: I, origPath: 'Code/A.cs', fileRev: 3 }])
  })

  it('blame shows the mismatch note', async () => {
    store.readBlame.mockReset().mockResolvedValue(svnBlame)
    store.readSvnBlameOverlay.mockResolvedValue([{ kind: 'mismatch', svnLines: 1, gitLines: 2 }])
    mountSvn('blame')
    expect(await screen.findByText('有 2 行没对上 SVN 版本，保持基线显示')).toBeTruthy()
  })

  it('overlay is recomputed after workdir change', async () => {
    store.readBlame.mockReset().mockImplementation(fresh(svnBlame))
    const late = deferred<unknown>()
    store.readSvnBlameOverlay
      .mockReturnValueOnce(late.promise)
      .mockResolvedValueOnce([{ kind: 'mapped', lines: [{ rev: 2, author: 'newer', date: 'd' }, { rev: 2, author: 'newer', date: 'd' }] }])
    const { rerender } = mountSvn('blame', { workdirVersion: 1 })
    await waitFor(() => expect(store.readSvnBlameOverlay).toHaveBeenCalledTimes(1))
    rerender({ workdirVersion: 2 })
    expect(await screen.findByText('newer')).toBeTruthy()
    expect(store.readBlame).toHaveBeenCalledTimes(2)
    late.resolve([{ kind: 'mapped', lines: [{ rev: 1, author: 'older', date: 'd' }, { rev: 1, author: 'older', date: 'd' }] }])
    await new Promise((r) => setTimeout(r, 0))
    expect(screen.queryByText('older')).toBeNull()
    // 晚到的旧结果也不能把新结果冲掉
    expect(screen.getByText('newer')).toBeTruthy()
  })

  it('plain git repo never calls the svn functions', async () => {
    store.readFileHistory.mockReset().mockResolvedValue(history)
    store.readBlame.mockReset().mockResolvedValue(blame)
    const { rerender } = mountSvn('history')
    // 列表和右侧 diff 的标题里各有一处
    expect((await screen.findAllByText('改内容')).length).toBeGreaterThan(0)
    rerender({})
    fireEvent.click(screen.getByText('Blame'))
    expect(await screen.findByText('alice')).toBeTruthy()
    expect(store.readSvnFileHistory).not.toHaveBeenCalled()
    expect(store.readSvnBlameOverlay).not.toHaveBeenCalled()
  })
})

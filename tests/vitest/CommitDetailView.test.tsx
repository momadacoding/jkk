// @vitest-environment jsdom
//
// svn 节点详情里「这次交付包含的本地提交」：能点开单个提交，点开后能回到 svn 节点（2026-10-16 用户要求）。
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { COMMIT_LIST_LIMIT, CommitDetailView } from '@/components/CommitDetailView'
import { setDiffPrefs } from '@/store/diffPrefsStore'
import type { ExternalToolsConfig } from '@/engine/externalTools'
import type { CommitNode, DiffData } from '@/engine/types'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))
const invokeMock = vi.mocked(invoke)

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

describe('CommitDetailView：SVN 没返回作者和时间的 revision', () => {
  // 2026-11-26 报告：r67823 的 svn log 只有 revision 号，详情里显示 1970、作者空白、说明空白
  const hidden: CommitNode = {
    id: 'svn:67823',
    parents: ['svn:67822'],
    author: '',
    time: 1_700_000_000, // 借来排序用的，不能显示出来
    timeUnknown: true,
    message: '',
    vcs: 'svn',
    svnRev: 67823,
  }

  it('写「时间未知」「SVN 未返回」，并说明为什么是空的', () => {
    render(<CommitDetailView commit={hidden} diff={empty} repoPath="C:/r" onSelectCommit={vi.fn()} />)
    expect(screen.getByText('时间未知')).toBeTruthy()
    expect(screen.getByText('SVN 未返回')).toBeTruthy()
    expect(screen.getByText(/只返回了 revision 号/)).toBeTruthy()
    // 头像不取「SVN 未返回」的「S」：那会像一个真实作者的首字母
    expect(document.querySelector('.commit-avatar')?.textContent).toBe('?')
    expect(screen.queryByText(new Date(1_700_000_000 * 1000).toLocaleString())).toBeNull()
  })

  it('正常的 svn 节点没有这段说明', () => {
    render(<CommitDetailView commit={r16([])} diff={empty} repoPath="C:/r" onSelectCommit={vi.fn()} />)
    expect(screen.queryByText(/只返回了 revision 号/)).toBeNull()
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

// diff 为 null = 这个提交的 diff 还没读回来（`currentDiff`）。2026-10-22 E2E：大提交要几十秒，
// 这期间不能显示上一份 diff 的文件，也不能在 Changes 上标上一份的个数
describe('CommitDetailView：diff 还没读回来', () => {
  it('Commit 标签页说正在读取，不列文件，Changes 不带个数', () => {
    render(<CommitDetailView commit={three[1]} diff={null} repoPath="C:/r" onSelectCommit={vi.fn()} />)
    expect(screen.getByText('正在读取这个提交的改动…')).toBeTruthy()
    expect(screen.getByRole('button', { name: 'Changes' })).toBeTruthy()
    expect(document.querySelector('.commit-file-row')).toBeNull()
  })

  it('Changes 标签页同样说正在读取', () => {
    render(<CommitDetailView commit={three[1]} diff={null} repoPath="C:/r" onSelectCommit={vi.fn()} />)
    fireEvent.click(screen.getByRole('button', { name: 'Changes' }))
    expect(screen.getByText('正在读取这个提交的改动…')).toBeTruthy()
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

// 第 72 条：读失败不能显示成「这个提交没有改动」
describe('CommitDetailView：diff 读失败', () => {
  const failed: DiffData = { files: [], error: 'fatal: bad object B' }

  it('Commit 标签的文件列表处写明读取失败', () => {
    render(<CommitDetailView commit={three[1]} diff={failed} repoPath="C:/r" onSelectCommit={vi.fn()} />)
    expect(screen.getByText(/读取改动失败/)).toBeTruthy()
    expect(screen.getByText(/bad object B/)).toBeTruthy()
  })

  it('Changes 标签整块换成失败说明，不显示 No changes', () => {
    render(<CommitDetailView commit={three[1]} diff={failed} repoPath="C:/r" onSelectCommit={vi.fn()} />)
    fireEvent.click(screen.getByRole('button', { name: /Changes/ }))
    expect(screen.getByText(/读取改动失败/)).toBeTruthy()
    expect(screen.queryByText('No changes')).toBeNull()
  })

  it('确实没有改动时照旧显示 No changes', () => {
    render(<CommitDetailView commit={three[1]} diff={empty} repoPath="C:/r" onSelectCommit={vi.fn()} />)
    fireEvent.click(screen.getByRole('button', { name: /Changes/ }))
    expect(screen.queryByText(/读取改动失败/)).toBeNull()
  })

  it('失败说明里带着 diff 工具栏：开关调成读不出来的样子时能在这里调回去', () => {
    render(<CommitDetailView commit={three[1]} diff={failed} repoPath="C:/r" onSelectCommit={vi.fn()} />)
    fireEvent.click(screen.getByRole('button', { name: /Changes/ }))
    expect(screen.getByRole('toolbar', { name: 'diff 显示选项' })).toBeTruthy()
  })
})

// diff 面板开关：后端因为文件太多按默认上下文读了，只显示选中文件时这个标记也要带过去（真实窗口里丢过）
describe('CommitDetailView：上下文被后端退回默认', () => {
  it('Changes 标签里 diff 上方有说明', () => {
    const narrowed: DiffData = {
      files: [{ path: 'a.txt', status: 'modified', oldPath: null, binary: false, hunks: [] }],
      contextNarrowed: true,
    }
    render(<CommitDetailView commit={three[1]} diff={narrowed} repoPath="C:/r" onSelectCommit={vi.fn()} />)
    fireEvent.click(screen.getByRole('button', { name: /Changes/ }))
    expect(screen.getByText(/文件太多/)).toBeTruthy()
  })
})

// 文件列表的增删行数与右侧 diff 同一口径：只换了换行符的文件在 diff 里被藏掉，列表也不能说 +3/-3
describe('CommitDetailView：增删行数跟着忽略换行符', () => {
  const del = (content: string, o: number) => ({ origin: '-' as const, content, oldLineno: o, newLineno: null })
  const add = (content: string, n: number) => ({ origin: '+' as const, content, oldLineno: null, newLineno: n })
  const crlf: DiffData = {
    files: [{
      path: 'a.txt', status: 'modified', oldPath: null, binary: false,
      hunks: [{
        oldStart: 1, newStart: 1, header: '@@ -1,3 +1,3 @@',
        lines: [del('one', 1), del('two', 2), del('three', 3), add('one\r', 1), add('TWO\r', 2), add('three\r', 3)],
      }],
    }],
  }

  it('默认（忽略换行符开着）只算真改的那一行；关掉后每行都算', () => {
    render(<CommitDetailView commit={three[1]} diff={crlf} repoPath="C:/r" onSelectCommit={vi.fn()} />)
    expect(screen.getByText('+1/-1')).toBeTruthy()
    act(() => { setDiffPrefs({ ignoreEol: false }) })
    expect(screen.getByText('+3/-3')).toBeTruthy()
    act(() => { setDiffPrefs({ ignoreEol: true }) })
  })
})

// 第 73 条：文件太多时后端只列文件（summaryOnly），右侧按选中的文件单独读
describe('CommitDetailView：大提交只列了文件', () => {
  const listed = (n: number): DiffData => ({
    files: Array.from({ length: n }, (_, i) => ({
      path: `big/F${String(i).padStart(5, '0')}.txt`,
      status: i === 1 ? 'renamed' as const : 'added' as const,
      oldPath: i === 1 ? 'old/F00001.txt' : null,
      binary: false,
      hunks: [],
    })),
    summaryOnly: true,
  })
  const oneFile = (path: string): DiffData => ({
    files: [{
      path, status: 'added', oldPath: null, binary: false,
      hunks: [{
        oldStart: 0, newStart: 1, header: '@@ -0,0 +1 @@',
        lines: [{ origin: '+', content: `content of ${path}`, oldLineno: null, newLineno: 1 }],
      }],
    }],
  })

  beforeEach(() => {
    invokeMock.mockReset()
    invokeMock.mockImplementation(async (_cmd, args) => oneFile((args as { file: string }).file))
  })

  function mount(diff: DiffData) {
    render(<CommitDetailView commit={three[1]} diff={diff} repoPath="C:/r" onSelectCommit={vi.fn()} diffScope="commit:abc" />)
  }

  it('Commit 标签：说明没统计行数，不显示 +0/-0，平铺列表截到上限', () => {
    mount(listed(COMMIT_LIST_LIMIT + 5))
    expect(screen.getByText(/改了 1005 个文件/)).toBeTruthy()
    expect(screen.getByText(/只列前 1000 个/)).toBeTruthy()
    expect(document.querySelectorAll('.commit-file-row').length).toBe(COMMIT_LIST_LIMIT)
    expect(document.querySelector('.file-stats')).toBeNull()
    expect(invokeMock).not.toHaveBeenCalled()
  })

  it('不超过上限时不提截断', () => {
    mount(listed(3))
    expect(screen.queryByText(/只列前/)).toBeNull()
    expect(document.querySelectorAll('.commit-file-row').length).toBe(3)
  })

  it('Changes 标签：按选中的文件读 read_diff_file，改名带旧路径；树默认收起', async () => {
    mount(listed(3))
    fireEvent.click(screen.getByRole('button', { name: /Changes/ }))
    expect(screen.getByText('正在读取这个文件的改动…')).toBeTruthy()
    await waitFor(() => expect(screen.getByText('content of big/F00000.txt')).toBeTruthy())
    expect(invokeMock).toHaveBeenCalledWith('read_diff_file', expect.objectContaining({
      path: 'C:/r', scope: 'commit:abc', file: 'big/F00000.txt', oldFile: null,
    }))
    // 树收起：文件行不在，只有目录
    expect(screen.queryByText('F00001.txt')).toBeNull()
    fireEvent.click(screen.getByText('big'))
    fireEvent.click(screen.getByText('F00001.txt'))
    await waitFor(() => expect(screen.getByText('content of big/F00001.txt')).toBeTruthy())
    expect(invokeMock).toHaveBeenLastCalledWith('read_diff_file', expect.objectContaining({
      file: 'big/F00001.txt', oldFile: 'old/F00001.txt',
    }))
  })

  it('改了 diff 开关就按新选项重读这个文件', async () => {
    mount(listed(2))
    fireEvent.click(screen.getByRole('button', { name: /Changes/ }))
    await waitFor(() => expect(invokeMock).toHaveBeenCalledTimes(1))
    act(() => { setDiffPrefs({ ignoreWhitespace: true }) })
    await waitFor(() => expect(invokeMock).toHaveBeenCalledTimes(2))
    expect(invokeMock).toHaveBeenLastCalledWith('read_diff_file', expect.objectContaining({
      options: expect.objectContaining({ ignoreWhitespace: true }),
    }))
    act(() => { setDiffPrefs({ ignoreWhitespace: false }) })
    // 改回去也会重读一次，等它回来再结束，免得结果落在测试之外
    await waitFor(() => expect(invokeMock).toHaveBeenCalledTimes(3))
    expect(await screen.findByText('content of big/F00000.txt')).toBeTruthy()
  })

  it('单个文件读失败：写明原因', async () => {
    invokeMock.mockImplementation(async () => { throw { code: 'git', message: 'bad object' } })
    mount(listed(2))
    fireEvent.click(screen.getByRole('button', { name: /Changes/ }))
    await waitFor(() => expect(screen.getByText(/读取改动失败/)).toBeTruthy())
  })

  it('普通提交不走单文件读', () => {
    const normal: DiffData = { files: [{ path: 'a.txt', status: 'added', oldPath: null, binary: false, hunks: [] }] }
    mount(normal)
    fireEvent.click(screen.getByRole('button', { name: /Changes/ }))
    expect(invokeMock).not.toHaveBeenCalled()
  })
})

// @vitest-environment jsdom
//
// 组件测试只在这个文件里切到 jsdom，其余纯逻辑测试仍跑在 node 环境（vitest.config.ts）。
import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeAll, describe, expect, it, vi } from 'vitest'
import { LocalChangesView } from '@/components/LocalChangesView'
import type { ExternalTool, ExternalToolsConfig } from '@/engine/externalTools'
import type { ConflictInfo, DiffData, FileChange, StatusData } from '@/engine/types'
import type { FileSelection } from '@/engine/fileSelection'

// react-resizable-panels 挂载时要 new 一个 ResizeObserver，jsdom 没有。
// 这里不测布局，给个什么都不做的桩就够了。
beforeAll(() => {
  vi.stubGlobal('ResizeObserver', class {
    observe() {}
    unobserve() {}
    disconnect() {}
  })
})

function change(path: string): FileChange {
  return { path, status: 'modified', additions: 1, deletions: 0 }
}

function mount() {
  const onDiscard = vi.fn()
  const status: StatusData = { unstaged: [change('a.txt'), change('b.txt')], staged: [change('s.txt')] }
  render(
    <LocalChangesView
      status={status}
      diff={{ files: [] }}
      onStage={vi.fn()}
      onUnstage={vi.fn()}
      onDiscard={onDiscard}
      onCommit={vi.fn()}
      svnMode={false}
      onBaselineBranch={false}
    />,
  )
  return { onDiscard }
}

/** 文件行所在的可聚焦列表容器，快捷键挂在它上面。 */
function listOf(path: string): HTMLElement {
  const row = screen.getByText(path)
  const list = row.closest('.changes-section-files')
  if (!(list instanceof HTMLElement)) throw new Error(`找不到 ${path} 所在的列表`)
  return list
}

describe('LocalChangesView 的 Delete 快捷键', () => {
  it('选中 Unstaged 里的文件后按 Delete，走和右键菜单一样的 onDiscard', () => {
    const { onDiscard } = mount()
    fireEvent.click(screen.getByText('b.txt'))
    fireEvent.keyDown(listOf('b.txt'), { key: 'Delete' })
    expect(onDiscard).toHaveBeenCalledTimes(1)
    expect(onDiscard).toHaveBeenCalledWith(['b.txt'], false)
  })

  it('Staged 区按 Delete 带 staged=true', () => {
    const { onDiscard } = mount()
    fireEvent.click(screen.getByText('s.txt'))
    fireEvent.keyDown(listOf('s.txt'), { key: 'Delete' })
    expect(onDiscard).toHaveBeenCalledTimes(1)
    expect(onDiscard).toHaveBeenCalledWith(['s.txt'], true)
  })

  it('选中了文件，但在 commit 输入框里按 Delete 删字，不能触发丢弃', () => {
    const { onDiscard } = mount()
    fireEvent.click(screen.getByText('a.txt'))
    const subject = screen.getByPlaceholderText('Commit subject')
    fireEvent.keyDown(subject, { key: 'Delete' })
    fireEvent.keyDown(screen.getByPlaceholderText('Description'), { key: 'Delete' })
    expect(onDiscard).not.toHaveBeenCalled()
  })

  it('焦点在 Unstaged 列表、选中却在 Staged 时不丢弃', () => {
    const { onDiscard } = mount()
    fireEvent.click(screen.getByText('s.txt'))
    fireEvent.keyDown(listOf('a.txt'), { key: 'Delete' })
    expect(onDiscard).not.toHaveBeenCalled()
  })

  it('没选中任何文件时按 Delete 什么都不做', () => {
    const { onDiscard } = mount()
    fireEvent.keyDown(listOf('a.txt'), { key: 'Delete' })
    expect(onDiscard).not.toHaveBeenCalled()
  })
})

const bc: ExternalTool = {
  id: 'bc',
  label: 'Beyond Compare',
  program: 'BComp.exe',
  diffArgs: ['$LOCAL', '$REMOTE'],
  mergeArgs: ['$LOCAL', '$REMOTE', '$BASE', '$MERGED'],
}
const vscode: ExternalTool = {
  id: 'vscode',
  label: 'VS Code',
  program: 'code',
  diffArgs: ['--wait', '--diff', '$LOCAL', '$REMOTE'],
}

function toolsConfig(tools: ExternalTool[]): ExternalToolsConfig {
  return { version: 1, defaultDiff: 'bc', defaultMerge: 'bc', rules: [], tools }
}

function mountWithTools(config: ExternalToolsConfig | null) {
  const onDiff = vi.fn()
  const onMerge = vi.fn()
  const onEditConfig = vi.fn()
  const status: StatusData = {
    unstaged: [change('a.txt'), { ...change('c.txt'), status: 'conflicted' }],
    staged: [change('s.txt')],
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
      externalTools={{ config, onDiff, onMerge, onEditConfig }}
    />,
  )
  return { onDiff, onEditConfig }
}

/** 菜单最外层的最后一项（子菜单里的不算）。 */
function lastTopLevelItem(): string | null {
  const menu = document.querySelector('.context-menu')
  return menu?.lastElementChild?.textContent ?? null
}

describe('LocalChangesView 的外部 diff 菜单', () => {
  it('右键 Unstaged 文件：「在 Beyond Compare 中对比」，点下去拿到 unstaged 目标', async () => {
    const { onDiff } = mountWithTools(toolsConfig([bc]))
    fireEvent.contextMenu(screen.getByText('a.txt'))
    fireEvent.click(screen.getByText('在 Beyond Compare 中对比'))
    await waitFor(() => expect(onDiff).toHaveBeenCalledWith('bc', { kind: 'unstaged', file: 'a.txt' }))
  })

  it('右键 Staged 文件拿到 staged 目标', async () => {
    const { onDiff } = mountWithTools(toolsConfig([bc]))
    fireEvent.contextMenu(screen.getByText('s.txt'))
    fireEvent.click(screen.getByText('在 Beyond Compare 中对比'))
    await waitFor(() => expect(onDiff).toHaveBeenCalledWith('bc', { kind: 'staged', file: 's.txt' }))
  })

  it('配了两个工具时有「用其他工具打开 ▸」，里面列出 VS Code', async () => {
    const { onDiff } = mountWithTools(toolsConfig([bc, vscode]))
    fireEvent.contextMenu(screen.getByText('a.txt'))
    expect(screen.queryByText('VS Code')).toBeNull()
    fireEvent.click(screen.getByText('用其他工具打开 ▸'))
    fireEvent.click(screen.getByText('VS Code'))
    await waitFor(() => expect(onDiff).toHaveBeenCalledWith('vscode', { kind: 'unstaged', file: 'a.txt' }))
  })

  it('只有一个工具时没有子菜单', () => {
    mountWithTools(toolsConfig([bc]))
    fireEvent.contextMenu(screen.getByText('a.txt'))
    expect(screen.queryByText('用其他工具打开 ▸')).toBeNull()
  })

  it('冲突文件的右键菜单里没有「对比」', () => {
    mountWithTools(toolsConfig([bc, vscode]))
    fireEvent.contextMenu(screen.getByText('c.txt'))
    expect(screen.queryByText(/中对比/)).toBeNull()
    expect(screen.queryByText('用其他工具打开 ▸')).toBeNull()
    expect(lastTopLevelItem()).toBe('编辑外部工具配置…')
  })

  it('最后一项是「编辑外部工具配置…」，点它调用 onEditConfig', async () => {
    const { onEditConfig } = mountWithTools(toolsConfig([bc, vscode]))
    fireEvent.contextMenu(screen.getByText('a.txt'))
    expect(lastTopLevelItem()).toBe('编辑外部工具配置…')
    fireEvent.click(screen.getByText('编辑外部工具配置…'))
    await waitFor(() => expect(onEditConfig).toHaveBeenCalledTimes(1))
  })

  it('配置读取失败时，外部工具相关的只剩「编辑外部工具配置…」', () => {
    mountWithTools(null)
    fireEvent.contextMenu(screen.getByText('a.txt'))
    expect(screen.queryByText(/中对比/)).toBeNull()
    expect(screen.queryByText('用其他工具打开 ▸')).toBeNull()
    expect(lastTopLevelItem()).toBe('编辑外部工具配置…')
  })
})

const uym: ExternalTool = {
  id: 'uym',
  label: 'UnityYAMLMerge',
  program: 'UnityYAMLMerge.exe',
  mergeArgs: ['merge', '-p', '$BASE', '$REMOTE', '$LOCAL', '$MERGED'],
}

const STAGE = { oid: 'a'.repeat(40), mode: 0o100644 }

function conflictInfo(path: string, over: Partial<ConflictInfo> = {}): ConflictInfo {
  return {
    path,
    code: 'UU',
    base: STAGE,
    ours: STAGE,
    theirs: STAGE,
    case: 'markers',
    reason: null,
    driver: null,
    sameAsHead: false,
    ...over,
  }
}

function conflicted(path: string): FileChange {
  return { path, status: 'conflicted', additions: 0, deletions: 0 }
}

function mountConflicts(conflicts: ConflictInfo[] | null) {
  const onMerge = vi.fn()
  const onStage = vi.fn()
  const config: ExternalToolsConfig = {
    version: 1,
    defaultDiff: 'bc',
    defaultMerge: 'bc',
    rules: [{ patterns: ['*.unity'], merge: 'uym' }],
    tools: [bc, vscode, uym],
  }
  const status: StatusData = {
    unstaged: [conflicted('Main.unity'), conflicted('c.txt'), conflicted('img.png'), change('a.txt')],
    staged: [],
  }
  render(
    <LocalChangesView
      status={status}
      diff={{
        files: [{
          path: 'c.txt',
          status: 'conflicted',
          oldPath: null,
          binary: false,
          hunks: [{ oldStart: 1, newStart: 1, header: '@@ -1 +1 @@', lines: [
            { origin: '-', content: 'ours1', oldLineno: 1, newLineno: null },
            { origin: '+', content: 'hand merged', oldLineno: null, newLineno: 1 },
          ] }],
        }],
      }}
      onStage={onStage}
      onUnstage={vi.fn()}
      onDiscard={vi.fn()}
      onCommit={vi.fn()}
      svnMode={false}
      onBaselineBranch={false}
      conflicts={conflicts}
      externalTools={{ config, onDiff: vi.fn(), onMerge, onEditConfig: vi.fn() }}
    />,
  )
  return { onMerge, onStage }
}

describe('LocalChangesView 的冲突文件', () => {
  const infos = [
    conflictInfo('Main.unity', { case: 'notTextMerged', reason: 'customDriver', driver: 'unityyamlmerge' }),
    conflictInfo('c.txt'),
    conflictInfo('img.png', { case: 'notTextMerged', reason: 'binaryContent' }),
  ]

  it('选中冲突文件时显示冲突提示，按钮按规则选工具：在 UnityYAMLMerge 中解决', async () => {
    const { onMerge } = mountConflicts(infos)
    fireEvent.click(screen.getByText('Main.unity'))
    expect(screen.getByText(/但仍处于冲突状态/)).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: '在 UnityYAMLMerge 中解决' }))
    await waitFor(() => expect(onMerge).toHaveBeenCalledWith('uym', 'Main.unity'))
  })

  it('① 有标记时只显示提示，不显示 diff', () => {
    mountConflicts(infos)
    fireEvent.click(screen.getByText('c.txt'))
    expect(screen.getByText('这个文件有冲突')).toBeTruthy()
    expect(screen.queryByText('hand merged')).toBeNull()
  })

  it('② 没有标记时显示 HEAD 对比工作区的 diff 和 Stage 按钮', async () => {
    const { onStage } = mountConflicts([conflictInfo('c.txt', { case: 'noMarkers' })])
    fireEvent.click(screen.getByText('c.txt'))
    expect(screen.getByText('未检测到冲突标记，检查后可以暂存')).toBeTruthy()
    expect(screen.getByText('hand merged')).toBeTruthy()
    fireEvent.click(document.querySelector('.conflict-notice button') as HTMLElement)
    await waitFor(() => expect(onStage).toHaveBeenCalledWith(['c.txt']))
  })

  it('冲突行不显示 +0/-0', () => {
    mountConflicts(infos)
    const row = screen.getByText('c.txt').closest('.changes-file')
    expect(row?.textContent).not.toContain('+0/-0')
    const normal = screen.getByText('a.txt').closest('.changes-file')
    expect(normal?.textContent).toContain('+1/-0')
  })

  it('冲突文件的右键菜单里有「在 Beyond Compare 中解决」', async () => {
    const { onMerge } = mountConflicts(infos)
    fireEvent.contextMenu(screen.getByText('c.txt'))
    // 选中的冲突文件顶部也有同名按钮，这里只点菜单里的
    const menu = document.querySelector('.context-menu') as HTMLElement
    fireEvent.click(within(menu).getByText('在 Beyond Compare 中解决'))
    await waitFor(() => expect(onMerge).toHaveBeenCalledWith('bc', 'c.txt'))
  })

  it('右键菜单的其他 merge 工具只列有 mergeArgs 的', () => {
    mountConflicts(infos)
    fireEvent.contextMenu(screen.getByText('c.txt'))
    fireEvent.click(screen.getByText('用其他工具打开 ▸'))
    expect(screen.getByText('UnityYAMLMerge')).toBeTruthy()
    expect(screen.queryByText('VS Code')).toBeNull()
  })

  it('二进制冲突没有外部 merge 入口', () => {
    mountConflicts(infos)
    fireEvent.contextMenu(screen.getByText('img.png'))
    const menu = document.querySelector('.context-menu') as HTMLElement
    expect(within(menu).queryByText(/中解决/)).toBeNull()
    expect(lastTopLevelItem()).toBe('编辑外部工具配置…')
  })

  it('冲突信息还没读到时既没有合并入口也不显示 diff', () => {
    mountConflicts(null)
    fireEvent.click(screen.getByText('c.txt'))
    expect(screen.queryByText('hand merged')).toBeNull()
    expect(screen.queryByRole('button', { name: /中解决/ })).toBeNull()
    fireEvent.contextMenu(screen.getByText('c.txt'))
    expect(screen.queryByText(/中解决/)).toBeNull()
  })
})

describe('LocalChangesView 的一边删一边改', () => {
  function mountDeleteModify() {
    const onResolveDeleteModify = vi.fn()
    const file = '目录/-d [1].txt'
    render(
      <LocalChangesView
        status={{ unstaged: [conflicted(file)], staged: [] }}
        diff={{ files: [] }}
        onStage={vi.fn()}
        onUnstage={vi.fn()}
        onDiscard={vi.fn()}
        onCommit={vi.fn()}
        svnMode={false}
        onBaselineBranch={false}
        conflicts={[conflictInfo(file, { case: 'deleteModify', code: 'DU', ours: null })]}
        onResolveDeleteModify={onResolveDeleteModify}
      />,
    )
    return { onResolveDeleteModify, file }
  }

  it('点「保留修改后的文件」直接调用 keep = true，不弹框', async () => {
    const { onResolveDeleteModify, file } = mountDeleteModify()
    fireEvent.click(screen.getByRole('button', { name: '保留修改后的文件' }))
    await waitFor(() => expect(onResolveDeleteModify).toHaveBeenCalledWith(file, true))
    expect(document.querySelector('.dialog-actions')).toBeNull()
  })

  it('点「删除文件」先弹危险确认框，标题和内容都带文件名；取消就不执行', async () => {
    const { onResolveDeleteModify, file } = mountDeleteModify()
    fireEvent.click(screen.getByRole('button', { name: '删除文件' }))
    const dialog = await waitFor(() => {
      const el = document.querySelector('.dialog-actions')?.closest('.dialog') ?? document.querySelector('.dialog-actions')?.parentElement
      if (!el) throw new Error('没有弹框')
      return el as HTMLElement
    })
    expect(dialog.textContent).toContain(`删除 ${file}？`)
    // 标题和正文里各出现一次
    expect((dialog.textContent ?? '').split(file).length - 1).toBeGreaterThanOrEqual(2)
    expect(within(dialog).getByRole('button', { name: '删除文件' }).className).toContain('danger')
    fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }))
    await waitFor(() => expect(document.querySelector('.dialog-actions')).toBeNull())
    expect(onResolveDeleteModify).not.toHaveBeenCalled()
  })

  it('确认之后才调用 keep = false', async () => {
    const { onResolveDeleteModify, file } = mountDeleteModify()
    fireEvent.click(screen.getByRole('button', { name: '删除文件' }))
    const actions = await waitFor(() => {
      const el = document.querySelector('.dialog-actions')
      if (!el) throw new Error('没有弹框')
      return el as HTMLElement
    })
    fireEvent.click(within(actions).getByRole('button', { name: '删除文件' }))
    await waitFor(() => expect(onResolveDeleteModify).toHaveBeenCalledWith(file, false))
  })

  it('没接回调时不显示这两个按钮', () => {
    const file = 'd.txt'
    render(
      <LocalChangesView
        status={{ unstaged: [conflicted(file)], staged: [] }}
        diff={{ files: [] }}
        onStage={vi.fn()}
        onUnstage={vi.fn()}
        onDiscard={vi.fn()}
        onCommit={vi.fn()}
        svnMode={false}
        onBaselineBranch={false}
        conflicts={[conflictInfo(file, { case: 'deleteModify', code: 'UD', theirs: null })]}
      />,
    )
    expect(screen.queryByRole('button', { name: '删除文件' })).toBeNull()
    expect(screen.queryByRole('button', { name: '保留修改后的文件' })).toBeNull()
  })
})

// 第 85 条：git add 不检查冲突标记，Stage 一下标记就能被提交进历史。
// 四个入口（行按钮、Stage All、右键、多选后右键）都必须先过同一道确认。
// 冲突说明里的 Stage 按钮也走同一个出口，但它只在 noMarkers / 二进制时出现，
// 这两种本来就放行，所以只是兜底，这里不单独测。
// 后端那一半（暂存 `f[1].txt` 不能按 glob 连带 `f1.txt`）在 src-tauri/tests/vcs_backend.rs。
describe('LocalChangesView：有冲突标记的文件暂存前要确认', () => {
  function mountGuard(conflicts: ConflictInfo[] | null) {
    const onStage = vi.fn()
    render(
      <LocalChangesView
        status={{
          unstaged: [conflicted('c.txt'), conflicted('clean.txt'), change('a.txt'), change('b.txt')],
          staged: [],
        }}
        diff={{ files: [] }}
        onStage={onStage}
        onUnstage={vi.fn()}
        onDiscard={vi.fn()}
        onCommit={vi.fn()}
        svnMode={false}
        onBaselineBranch={false}
        conflicts={conflicts}
      />,
    )
    return { onStage }
  }
  const infos = [conflictInfo('c.txt'), conflictInfo('clean.txt', { case: 'noMarkers' })]

  function rowToggle(path: string): HTMLElement {
    const btn = screen.getByText(path).closest('.changes-file')?.querySelector('.changes-toggle')
    if (!(btn instanceof HTMLElement)) throw new Error(`找不到 ${path} 的 Stage 按钮`)
    return btn
  }

  async function guardDialog(): Promise<HTMLElement> {
    return await waitFor(() => {
      const el = document.querySelector('.dialog-actions')?.closest('.dialog')
      if (!(el instanceof HTMLElement)) throw new Error('没有弹框')
      return el
    })
  }

  function contextMenu(): HTMLElement {
    const menu = document.querySelector('.context-menu')
    if (!(menu instanceof HTMLElement)) throw new Error('右键菜单没打开')
    return menu
  }

  async function cancel(dialog: HTMLElement) {
    fireEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }))
    await waitFor(() => expect(document.querySelector('.dialog-actions')).toBeNull())
  }

  it('行上的 Stage 按钮：弹确认，取消后不暂存', async () => {
    const { onStage } = mountGuard(infos)
    fireEvent.click(rowToggle('c.txt'))
    const dialog = await guardDialog()
    expect(dialog.textContent).toContain('c.txt')
    expect(dialog.textContent).toContain('冲突标记')
    await cancel(dialog)
    expect(onStage).not.toHaveBeenCalled()
  })

  // 只断言焦点：Dialog 没有全局回车处理，回车触发的是有焦点的那个按钮
  it('打开时焦点落在取消上', async () => {
    mountGuard(infos)
    fireEvent.click(rowToggle('c.txt'))
    const dialog = await guardDialog()
    await waitFor(() => expect(document.activeElement).toBe(within(dialog).getByRole('button', { name: 'Cancel' })))
  })

  it('确认之后才暂存', async () => {
    const { onStage } = mountGuard(infos)
    fireEvent.click(rowToggle('c.txt'))
    const dialog = await guardDialog()
    fireEvent.click(within(dialog).getByRole('button', { name: '仍然暂存' }))
    await waitFor(() => expect(onStage).toHaveBeenCalledWith(['c.txt']))
  })

  it('Stage All：弹确认，取消后整批都不暂存', async () => {
    const { onStage } = mountGuard(infos)
    fireEvent.click(screen.getByRole('button', { name: 'Stage All' }))
    await cancel(await guardDialog())
    expect(onStage).not.toHaveBeenCalled()
  })

  it('Stage All 确认后整批一起暂存，不漏普通文件', async () => {
    const { onStage } = mountGuard(infos)
    fireEvent.click(screen.getByRole('button', { name: 'Stage All' }))
    const dialog = await guardDialog()
    fireEvent.click(within(dialog).getByRole('button', { name: '仍然暂存' }))
    await waitFor(() => expect(onStage).toHaveBeenCalledTimes(1))
    expect([...onStage.mock.calls[0][0]].sort()).toEqual(['a.txt', 'b.txt', 'c.txt', 'clean.txt'])
  })

  it('右键菜单的 Stage：弹确认，取消后不暂存', async () => {
    const { onStage } = mountGuard(infos)
    fireEvent.contextMenu(screen.getByText('c.txt'))
    fireEvent.click(within(contextMenu()).getByText('Stage'))
    await cancel(await guardDialog())
    expect(onStage).not.toHaveBeenCalled()
  })

  it('多选（混着普通文件）后右键 Stage：取消后整批都不暂存，不会只暂存普通文件', async () => {
    const { onStage } = mountGuard(infos)
    fireEvent.click(screen.getByText('a.txt'))
    fireEvent.click(screen.getByText('c.txt'), { ctrlKey: true })
    fireEvent.contextMenu(screen.getByText('c.txt'))
    fireEvent.click(within(contextMenu()).getByText('Stage (2)'))
    await cancel(await guardDialog())
    expect(onStage).not.toHaveBeenCalled()
  })

  it('冲突信息还没读到（null）：没有标记的那个冲突文件也要拦', async () => {
    const { onStage } = mountGuard(null)
    fireEvent.click(rowToggle('clean.txt'))
    const dialog = await guardDialog()
    expect(dialog.textContent).toContain('clean.txt')
    await cancel(dialog)
    expect(onStage).not.toHaveBeenCalled()
  })

  it('确认没有冲突标记的冲突文件直接暂存，不弹框', async () => {
    const { onStage } = mountGuard(infos)
    fireEvent.click(rowToggle('clean.txt'))
    await waitFor(() => expect(onStage).toHaveBeenCalledWith(['clean.txt']))
    expect(document.querySelector('.dialog-actions')).toBeNull()
  })

  it('普通文件照旧直接暂存，不弹框', async () => {
    const { onStage } = mountGuard(infos)
    fireEvent.click(rowToggle('a.txt'))
    await waitFor(() => expect(onStage).toHaveBeenCalledWith(['a.txt']))
    expect(document.querySelector('.dialog-actions')).toBeNull()
  })
})

// diff 为 null = 工作区 diff 还没读回来（`currentDiff`），比如刚从提交详情切回来。
// 右侧不能留着上一个视图的 diff
describe('LocalChangesView：diff 还没读回来', () => {
  it('右侧说正在读取', () => {
    render(
      <LocalChangesView
        status={{ unstaged: [change('a.txt')], staged: [] }}
        diff={null}
        onStage={vi.fn()}
        onUnstage={vi.fn()}
        onDiscard={vi.fn()}
        onCommit={vi.fn()}
        svnMode={false}
        onBaselineBranch={false}
      />,
    )
    fireEvent.click(screen.getByText('a.txt'))
    expect(screen.getByText('正在读取工作区的改动…')).toBeTruthy()
  })

  // 6.13 方案 C：右侧只读显示的那一个文件，没有文件可显示时什么都不读，diff 一直是 null。
  // 这时说「正在读取」就永远读不完了
  it('两边都没有文件：不说正在读取', () => {
    render(
      <LocalChangesView
        status={{ unstaged: [], staged: [] }}
        diff={null}
        onStage={vi.fn()}
        onUnstage={vi.fn()}
        onDiscard={vi.fn()}
        onCommit={vi.fn()}
        svnMode={false}
        onBaselineBranch={false}
      />,
    )
    expect(screen.queryByText(/正在读取/)).toBeNull()
  })
})

// 第 65 条：右侧 diff 按选中文件所在的区读。选中由 RepoTab 持有（它要据此决定读哪个 scope），
// 组件拿到的是受控的 selection。
describe('LocalChangesView 的选中按区', () => {
  function mountWith(status: StatusData, selection: FileSelection, diff: DiffData | null = { files: [] }) {
    const onSelectionChange = vi.fn()
    const onDiscard = vi.fn()
    render(
      <LocalChangesView
        status={status}
        diff={diff}
        onStage={vi.fn()}
        onUnstage={vi.fn()}
        onDiscard={onDiscard}
        onCommit={vi.fn()}
        svnMode={false}
        onBaselineBranch={false}
        selection={selection}
        onSelectionChange={onSelectionChange}
      />,
    )
    return { onSelectionChange, onDiscard }
  }

  /** 某个区里的某一行。同一个文件两个区都有时，按区取。 */
  function rowIn(title: 'Unstaged' | 'Staged', path: string): HTMLElement {
    const header = screen.getByText((_, el) => el?.tagName === 'SPAN' && el.textContent?.startsWith(`${title} (`) === true)
    const section = header.closest('.changes-section')
    if (!(section instanceof HTMLElement)) throw new Error(`找不到 ${title} 区`)
    const row = within(section).getByText(path).closest('.changes-file')
    if (!(row instanceof HTMLElement)) throw new Error(`找不到 ${title} 区的 ${path}`)
    return row
  }

  const none: FileSelection = { section: null, paths: [], anchor: null }
  const both: StatusData = { unstaged: [change('a.txt')], staged: [change('a.txt')] }

  it('点 Staged 里的行：交给父组件的选中是 Staged 的它', () => {
    const { onSelectionChange } = mountWith(both, none)
    fireEvent.click(within(rowIn('Staged', 'a.txt')).getByText('a.txt'))
    expect(onSelectionChange).toHaveBeenLastCalledWith({ section: 'staged', paths: ['a.txt'], anchor: 'a.txt' })
  })

  it('同一个文件两边都有：只高亮选中的那一边', () => {
    mountWith(both, { section: 'staged', paths: ['a.txt'], anchor: 'a.txt' })
    expect(rowIn('Staged', 'a.txt').classList.contains('active')).toBe(true)
    expect(rowIn('Unstaged', 'a.txt').classList.contains('active')).toBe(false)
  })

  it('什么都没选：右侧显示默认的第一个，但不把它高亮成选中', () => {
    mountWith({ unstaged: [change('a.txt')], staged: [change('s.txt')] }, none)
    expect(document.querySelectorAll('.changes-file.active')).toHaveLength(0)
  })

  it('选中的文件刚被整份暂存：选中跟到 Staged，在那里按 Delete 丢的就是它', () => {
    // 父组件里记的还是「Unstaged 的 a.txt」，但 a.txt 已经只在 Staged 里了
    const { onDiscard } = mountWith(
      { unstaged: [change('b.txt')], staged: [change('a.txt')] },
      { section: 'unstaged', paths: ['a.txt'], anchor: 'a.txt' },
    )
    const row = rowIn('Staged', 'a.txt')
    expect(row.classList.contains('active')).toBe(true)
    expect(row.classList.contains('selected')).toBe(true)
    const list = row.closest('.changes-section-files')
    if (!(list instanceof HTMLElement)) throw new Error('找不到 Staged 列表')
    fireEvent.keyDown(list, { key: 'Delete' })
    expect(onDiscard).toHaveBeenCalledWith(['a.txt'], true)
  })

  it('受控时不自己写回对齐结果：那是持有者（RepoTab）的事，它在本组件没挂载时也要对齐', () => {
    const { onSelectionChange } = mountWith(
      { unstaged: [change('b.txt')], staged: [change('a.txt')] },
      { section: 'unstaged', paths: ['a.txt'], anchor: 'a.txt' },
    )
    expect(onSelectionChange).not.toHaveBeenCalled()
  })

  it('不受控时自己存回对齐结果：a.txt 暂存后又被改动、两边都有，选中仍在 Staged', () => {
    const props = {
      diff: { files: [] },
      onStage: vi.fn(),
      onUnstage: vi.fn(),
      onDiscard: vi.fn(),
      onCommit: vi.fn(),
      svnMode: false,
      onBaselineBranch: false,
    }
    const { rerender } = render(
      <LocalChangesView {...props} status={{ unstaged: [change('a.txt'), change('b.txt')], staged: [] }} />,
    )
    fireEvent.click(within(rowIn('Unstaged', 'a.txt')).getByText('a.txt'))
    // 整份暂存
    rerender(<LocalChangesView {...props} status={{ unstaged: [change('b.txt')], staged: [change('a.txt')] }} />)
    // 又在工作区改了一下：两边都有
    rerender(<LocalChangesView {...props} status={{ unstaged: [change('a.txt'), change('b.txt')], staged: [change('a.txt')] }} />)
    expect(rowIn('Staged', 'a.txt').classList.contains('active')).toBe(true)
    expect(rowIn('Unstaged', 'a.txt').classList.contains('active')).toBe(false)
  })

  it('Staged 的 diff 还没读回来：说正在读取暂存区，不说工作区', () => {
    mountWith({ unstaged: [], staged: [change('s.txt')] }, { section: 'staged', paths: ['s.txt'], anchor: 's.txt' }, null)
    expect(screen.getByText('正在读取暂存区的改动…')).toBeTruthy()
  })
})

describe('LocalChangesView 的拖动多选与 Ctrl+A', () => {
  function mountThree() {
    const onDiscard = vi.fn()
    const status: StatusData = {
      unstaged: [change('a.txt'), change('b.txt'), change('c.txt')],
      staged: [change('s.txt')],
    }
    render(
      <LocalChangesView
        status={status}
        diff={{ files: [] }}
        onStage={vi.fn()}
        onUnstage={vi.fn()}
        onDiscard={onDiscard}
        onCommit={vi.fn()}
        svnMode={false}
        onBaselineBranch={false}
      />,
    )
    return { onDiscard }
  }

  function row(path: string): HTMLElement {
    const el = screen.getByText(path).closest('.changes-file')
    if (!(el instanceof HTMLElement)) throw new Error(`找不到 ${path} 的行`)
    return el
  }

  const selected = () => Array.from(document.querySelectorAll('.changes-file.selected'))
    .map((el) => el.querySelector('.changes-file-path')?.textContent)

  it('在一行按下左键、拖到另一行：选中从按下的行到当前行的一段', () => {
    mountThree()
    fireEvent.mouseDown(row('a.txt'), { button: 0, buttons: 1 })
    fireEvent.mouseEnter(row('c.txt'), { buttons: 1 })
    expect(selected()).toEqual(['a.txt', 'b.txt', 'c.txt'])
    // 拖回来范围跟着缩，锚点仍是按下的那行
    fireEvent.mouseEnter(row('b.txt'), { buttons: 1 })
    expect(selected()).toEqual(['a.txt', 'b.txt'])
  })

  it('松开鼠标后再划过别的行不再改选中', () => {
    mountThree()
    fireEvent.mouseDown(row('a.txt'), { button: 0, buttons: 1 })
    fireEvent.mouseEnter(row('b.txt'), { buttons: 1 })
    fireEvent.mouseUp(window)
    fireEvent.mouseEnter(row('c.txt'), { buttons: 1 })
    expect(selected()).toEqual(['a.txt', 'b.txt'])
  })

  it('在窗口外松开（没收到 mouseup）：回来时左键已不在按着，不当成还在拖', () => {
    mountThree()
    fireEvent.mouseDown(row('a.txt'), { button: 0, buttons: 1 })
    fireEvent.mouseEnter(row('c.txt'), { buttons: 0 })
    fireEvent.mouseEnter(row('b.txt'), { buttons: 1 })
    expect(selected()).toEqual([])
  })

  it('拖到一半状态刷新、锚点那行没了：从下一行起照样能拖出一段', () => {
    const view = (unstaged: string[]) => (
      <LocalChangesView
        status={{ unstaged: unstaged.map(change), staged: [] }}
        diff={{ files: [] }}
        onStage={vi.fn()}
        onUnstage={vi.fn()}
        onDiscard={vi.fn()}
        onCommit={vi.fn()}
        svnMode={false}
        onBaselineBranch={false}
      />
    )
    const { rerender } = render(view(['a.txt', 'b.txt', 'c.txt', 'd.txt']))
    fireEvent.mouseDown(row('a.txt'), { button: 0, buttons: 1 })
    // a.txt 在外部被暂存走了
    rerender(view(['b.txt', 'c.txt', 'd.txt']))
    fireEvent.mouseEnter(row('b.txt'), { buttons: 1 })
    fireEvent.mouseEnter(row('d.txt'), { buttons: 1 })
    expect(selected()).toEqual(['b.txt', 'c.txt', 'd.txt'])
  })

  it('从行内的 Stage 按钮上按下不开始拖动', () => {
    mountThree()
    fireEvent.mouseDown(within(row('a.txt')).getByText('Stage'), { button: 0, buttons: 1 })
    fireEvent.mouseEnter(row('c.txt'), { buttons: 1 })
    expect(selected()).toEqual([])
  })

  it('右键按下不开始拖动', () => {
    mountThree()
    fireEvent.mouseDown(row('a.txt'), { button: 2, buttons: 2 })
    fireEvent.mouseEnter(row('c.txt'), { buttons: 2 })
    expect(selected()).toEqual([])
  })

  it('拖到另一个区的行上不跨区', () => {
    mountThree()
    fireEvent.mouseDown(row('a.txt'), { button: 0, buttons: 1 })
    fireEvent.mouseEnter(row('b.txt'), { buttons: 1 })
    fireEvent.mouseEnter(row('s.txt'), { buttons: 1 })
    expect(selected()).toEqual(['a.txt', 'b.txt'])
  })

  it('文件列表有焦点时 Ctrl+A 选中本区全部，并挡掉 WebView 的整页全选', () => {
    mountThree()
    const notPrevented = fireEvent.keyDown(listOf('a.txt'), { key: 'a', ctrlKey: true })
    expect(notPrevented).toBe(false)
    expect(selected()).toEqual(['a.txt', 'b.txt', 'c.txt'])
    expect(screen.getByText('已选 3')).toBeTruthy()
  })

  it('Ctrl+A 只选有焦点的那个区：在 Staged 列表里按，只选 Staged', () => {
    mountThree()
    fireEvent.keyDown(listOf('s.txt'), { key: 'a', ctrlKey: true })
    expect(selected()).toEqual(['s.txt'])
  })

  it('Ctrl+A 之后按 Delete，丢弃请求带上全部文件（确认框仍由 onDiscard 弹）', () => {
    const { onDiscard } = mountThree()
    fireEvent.keyDown(listOf('a.txt'), { key: 'a', ctrlKey: true })
    fireEvent.keyDown(listOf('a.txt'), { key: 'Delete' })
    expect(onDiscard).toHaveBeenCalledWith(['a.txt', 'b.txt', 'c.txt'], false)
  })

  it('commit 输入框里的 Ctrl+A 照旧全选文字，不碰文件选中', () => {
    mountThree()
    const notPrevented = fireEvent.keyDown(screen.getByPlaceholderText('Commit subject'), { key: 'a', ctrlKey: true })
    expect(notPrevented).toBe(true)
    expect(selected()).toEqual([])
  })
})

// 第 72 条：读失败不能显示成「没有改动」
describe('LocalChangesView：diff 读失败', () => {
  it('右侧写明读取失败和原因，不说 No changes', () => {
    render(
      <LocalChangesView
        status={{ unstaged: [change('a.txt')], staged: [] }}
        diff={{ files: [], error: 'fatal: index file corrupt' }}
        onStage={vi.fn()}
        onUnstage={vi.fn()}
        onDiscard={vi.fn()}
        onCommit={vi.fn()}
        svnMode={false}
        onBaselineBranch={false}
      />,
    )
    expect(screen.getByText(/读取改动失败/)).toBeTruthy()
    expect(screen.getByText(/index file corrupt/)).toBeTruthy()
    expect(screen.queryByText('No changes')).toBeNull()
  })
})

describe('LocalChangesView 的提交说明（事故分析 6.6）', () => {
  function mountWith(onCommit: (msg: string, amend: boolean) => void | Promise<boolean>) {
    const status: StatusData = { unstaged: [], staged: [change('s.txt')] }
    render(
      <LocalChangesView
        status={status}
        diff={{ files: [] }}
        onStage={vi.fn()}
        onUnstage={vi.fn()}
        onDiscard={vi.fn()}
        onCommit={onCommit}
        svnMode={true}
        onBaselineBranch={false}
      />,
    )
    const subject = screen.getByPlaceholderText('Commit subject') as HTMLInputElement
    fireEvent.change(subject, { target: { value: '修登录' } })
    fireEvent.click(screen.getByRole('button', { name: 'Commit' }))
    return subject
  }

  it('在「先跟进」的提醒里取消了：写好的说明留着', async () => {
    const onCommit = vi.fn(async () => false)
    const subject = mountWith(onCommit)
    await waitFor(() => expect(onCommit).toHaveBeenCalledTimes(1))
    await Promise.resolve()
    expect(subject.value).toBe('修登录')
  })

  it('提交了：说明照旧清空', async () => {
    const subject = mountWith(vi.fn(async () => true))
    await waitFor(() => expect(subject.value).toBe(''))
  })
})

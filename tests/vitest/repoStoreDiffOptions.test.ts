// @vitest-environment jsdom
//
// diff 面板的开关：上下文行数 / 忽略空白要带进 `read_diff`，变了就重读；开关变之前发出的那一轮读回来不能写回。
import { act, renderHook } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { DiffData } from '@/engine/types'
import { DEFAULT_DIFF_PREFS, FULL_FILE_CONTEXT } from '@/engine/diffPrefs'

interface Pending {
  cmd: string
  args: Record<string, unknown>
  resolve: (value: unknown) => void
}

const pending: Pending[] = []
const sent: Pending[] = []

vi.mock('@tauri-apps/api/core', () => ({
  invoke: (cmd: string, args: Record<string, unknown>) => new Promise((resolve) => {
    const p = { cmd, args, resolve }
    // 除了 diff，别的读取一直挂着，这里不关心
    if (cmd === 'read_diff') {
      pending.push(p)
      sent.push(p)
    }
  }),
}))
vi.mock('@tauri-apps/api/event', () => ({ listen: () => Promise.resolve(() => {}) }))

const { useRepoEffects, useRepoStore } = await import('@/store/repoStore')
const { setDiffPrefs } = await import('@/store/diffPrefsStore')

function diffOf(path: string): DiffData {
  return { files: [{ path, status: 'modified', oldPath: null, binary: false, hunks: [] }] }
}

async function answerFirst(data: DiffData) {
  const call = pending.shift()
  if (!call) throw new Error('没有挂着的 read_diff')
  await act(async () => { call.resolve(data) })
}

const reads = () => sent.map((p) => ({ scope: p.args.scope, options: p.args.options }))

beforeEach(() => {
  pending.length = 0
  sent.length = 0
  act(() => { setDiffPrefs({ ...DEFAULT_DIFF_PREFS }) })
})

describe('refreshDiff 带上 diff 面板的后端选项', () => {
  it('默认开关：上下文 3 行、不忽略空白', async () => {
    const { result } = renderHook(() => useRepoStore())
    act(() => { void result.current.refreshDiff('C:/r', 'commit:A') })
    expect(reads()).toEqual([{ scope: 'commit:A', options: { contextLines: 3, ignoreWhitespace: false } }])
  })

  it('整个文件、忽略空白时发对应的值；纯画法的开关不进请求', async () => {
    act(() => { setDiffPrefs({ fullFile: true, ignoreWhitespace: true, split: true, wrap: true }) })
    const { result } = renderHook(() => useRepoStore())
    act(() => { void result.current.refreshDiff('C:/r', 'commit:A') })
    expect(reads()[0].options).toEqual({ contextLines: FULL_FILE_CONTEXT, ignoreWhitespace: true })
  })

  it('开关变之前发出的那一轮读回来不写回，变了之后的那一轮写回', async () => {
    const { result } = renderHook(() => useRepoStore())
    result.current.wantedDiffScope.current = 'commit:A'
    let first!: Promise<void>
    let second!: Promise<void>
    act(() => { first = result.current.refreshDiff('C:/r', 'commit:A') })
    act(() => { setDiffPrefs({ contextLines: 10 }) })
    // 同一个 scope 的读取串行：这一次排在补跑里，开读时取的是新开关
    act(() => { second = result.current.refreshDiff('C:/r', 'commit:A', 0) })
    await answerFirst(diffOf('stale.txt'))
    expect(result.current.state.diffScope).toBeNull()
    expect(reads().map((r) => r.options)).toEqual([
      { contextLines: 3, ignoreWhitespace: false },
      { contextLines: 10, ignoreWhitespace: false },
    ])
    await answerFirst(diffOf('fresh.txt'))
    await act(async () => { await Promise.all([first, second]) })
    expect(result.current.state.diff.files.map((f) => f.path)).toEqual(['fresh.txt'])
  })
})

describe('useRepoEffects：开关变了就重读当前视图的 diff', () => {
  function mount() {
    return renderHook(() => {
      const store = useRepoStore()
      useRepoEffects({ ...store.state, path: 'C:/r' }, store, 'changes', 'unstaged:a.txt', true)
      return store
    })
  }

  it('上下文行数变了重读，画法开关变了不读', async () => {
    mount()
    expect(reads()).toHaveLength(1)
    await answerFirst(diffOf('a.txt'))
    act(() => { setDiffPrefs({ split: true, wrap: true, showInvisibles: true, ignoreEol: false }) })
    expect(reads()).toHaveLength(1)
    act(() => { setDiffPrefs({ contextLines: 5 }) })
    expect(reads()).toEqual([
      { scope: 'unstaged:a.txt', options: { contextLines: 3, ignoreWhitespace: false } },
      { scope: 'unstaged:a.txt', options: { contextLines: 5, ignoreWhitespace: false } },
    ])
  })

  it('没显示的标签开关变了先不读，切回来时补读一次；切换本身不重读', async () => {
    const { rerender } = renderHook(({ active }) => {
      const store = useRepoStore()
      useRepoEffects({ ...store.state, path: 'C:/r' }, store, 'changes', 'unstaged:a.txt', active)
      return store
    }, { initialProps: { active: false } })
    // 刚挂载时照常读：后台恢复的标签也要有内容
    expect(reads()).toHaveLength(1)
    await answerFirst(diffOf('a.txt'))
    act(() => { setDiffPrefs({ fullFile: true }) })
    expect(reads()).toHaveLength(1)
    rerender({ active: true })
    expect(reads().map((r) => r.options)).toEqual([
      { contextLines: 3, ignoreWhitespace: false },
      { contextLines: FULL_FILE_CONTEXT, ignoreWhitespace: false },
    ])
    await answerFirst(diffOf('a.txt'))
    // 选项没再变：来回切标签不发请求
    rerender({ active: false })
    rerender({ active: true })
    expect(reads()).toHaveLength(2)
  })

  it('忽略空白变了也重读', async () => {
    mount()
    await answerFirst(diffOf('a.txt'))
    act(() => { setDiffPrefs({ ignoreWhitespace: true }) })
    expect(reads().map((r) => r.options)).toEqual([
      { contextLines: 3, ignoreWhitespace: false },
      { contextLines: 3, ignoreWhitespace: true },
    ])
  })
})

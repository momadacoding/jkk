import { describe, expect, it } from 'vitest'
import {
  closeAllTabs,
  closeOtherTabs,
  closeTabAt,
  closeTabsByPath,
  restoreTabs,
  type RepoTabInfo,
  type SessionView,
} from '@/engine/session'

function view(overrides: Partial<SessionView> = {}): SessionView {
  return { tabs: [], activeTab: null, recent: [], droppedTabs: [], ...overrides }
}

describe('restoreTabs', () => {
  it('一个标签都没有时是空态', () => {
    const r = restoreTabs(view())
    expect(r.tabs).toEqual([])
    expect(r.activeId).toBeNull()
    expect(r.nextId).toBe(0)
  })

  it('按顺序发 id，并把 nextId 交回去继续自增', () => {
    const r = restoreTabs(view({ tabs: ['C:/a', 'C:/b', 'C:/c'] }))
    expect(r.tabs.map((t) => t.id)).toEqual(['tab-0', 'tab-1', 'tab-2'])
    expect(r.nextId).toBe(3)
  })

  it('激活上次的活动标签', () => {
    const r = restoreTabs(view({ tabs: ['C:/a', 'C:/b'], activeTab: 'C:/b' }))
    expect(r.activeId).toBe('tab-1')
  })

  it('活动标签的写法不同也要认得（反斜杠、大小写、尾部斜杠）', () => {
    const r = restoreTabs(
      view({ tabs: ['C:/Users/Me/Repo', 'C:/other'], activeTab: 'c:\\users\\me\\repo\\' }),
    )
    expect(r.activeId).toBe('tab-0')
  })

  it('活动标签已被后端滤掉时回落到第一个', () => {
    const r = restoreTabs(view({ tabs: ['C:/a', 'C:/b'], activeTab: 'C:/gone' }))
    expect(r.activeId).toBe('tab-0')
  })

  it('没有活动标签记录时回落到第一个', () => {
    const r = restoreTabs(view({ tabs: ['C:/a'], activeTab: null }))
    expect(r.activeId).toBe('tab-0')
  })

  it('activeTab 是空串时不匹配任何标签，回落到第一个', () => {
    // 空串在 samePath 下会和任何只由斜杠组成的路径相等，不能让它误命中
    const r = restoreTabs(view({ tabs: ['C:/a'], activeTab: '' }))
    expect(r.activeId).toBe('tab-0')
  })
})

describe('restoreTabs 的 partial 标记', () => {
  // partial 决定要不要跳过还原后的那次自动保存。报错了会让每次正常启动都不落盘，
  // 漏报了会在驱动器离线时把存档里的标签永久截断——两个方向都不能错。
  it('全部还原成功时不是 partial', () => {
    expect(restoreTabs(view({ tabs: ['C:/a', 'C:/b'] })).partial).toBe(false)
  })

  it('有标签被后端滤掉时是 partial', () => {
    const r = restoreTabs(view({ tabs: ['C:/a'], droppedTabs: ['D:/offline'] }))
    expect(r.partial).toBe(true)
  })

  it('一个都打不开（整块盘离线）时是 partial', () => {
    const r = restoreTabs(view({ tabs: [], droppedTabs: ['D:/a', 'D:/b'] }))
    expect(r.tabs).toEqual([])
    expect(r.partial).toBe(true)
  })

  it('用户抢在还原之前先开了仓库时是 partial', () => {
    expect(restoreTabs(view({ tabs: ['C:/a'] }), 1).partial).toBe(true)
  })

  it('存档本来就是空的、用户也没抢先开，不是 partial', () => {
    // 首次启动走的就是这条：该正常落盘，否则第一次开的仓库记不住
    expect(restoreTabs(view(), 0).partial).toBe(false)
  })
})

describe('closeTabAt', () => {
  const tabs: RepoTabInfo[] = [
    { id: 'tab-0', path: 'C:/a' },
    { id: 'tab-1', path: 'C:/b' },
    { id: 'tab-2', path: 'C:/c' },
  ]

  it('关掉非活动标签时活动标签不动', () => {
    const r = closeTabAt(tabs, 'tab-2', 'tab-0')
    expect(r.tabs.map((t) => t.id)).toEqual(['tab-1', 'tab-2'])
    expect(r.activeId).toBe('tab-2')
  })

  it('关掉活动标签时接管右边那个', () => {
    const r = closeTabAt(tabs, 'tab-1', 'tab-1')
    expect(r.activeId).toBe('tab-2')
  })

  it('关掉最右边的活动标签时回落到左边', () => {
    const r = closeTabAt(tabs, 'tab-2', 'tab-2')
    expect(r.tabs.map((t) => t.id)).toEqual(['tab-0', 'tab-1'])
    expect(r.activeId).toBe('tab-1')
  })

  it('关掉最后一个标签后允许空列表', () => {
    const one: RepoTabInfo[] = [{ id: 'tab-0', path: 'C:/a' }]
    const r = closeTabAt(one, 'tab-0', 'tab-0')
    expect(r.tabs).toEqual([])
    expect(r.activeId).toBeNull()
  })

  it('id 不存在时原样返回', () => {
    const r = closeTabAt(tabs, 'tab-0', 'tab-99')
    expect(r.tabs).toBe(tabs)
    expect(r.activeId).toBe('tab-0')
  })

  it('空列表上关标签不炸', () => {
    const r = closeTabAt([], null, 'tab-0')
    expect(r.tabs).toEqual([])
    expect(r.activeId).toBeNull()
  })
})

describe('closeTabsByPath', () => {
  const tabs: RepoTabInfo[] = [
    { id: 'tab-0', path: 'C:/repo' },
    { id: 'tab-1', path: 'C:/repo/.worktrees/feat' },
    { id: 'tab-2', path: 'C:/other' },
  ]

  it('删掉的 worktree 那个标签会被关掉', () => {
    const r = closeTabsByPath(tabs, 'tab-0', 'C:/repo/.worktrees/feat')
    expect(r.tabs.map((t) => t.id)).toEqual(['tab-0', 'tab-2'])
    expect(r.activeId).toBe('tab-0')
  })

  it('关掉的正好是活动标签时接管右边那个', () => {
    const r = closeTabsByPath(tabs, 'tab-1', 'C:/repo/.worktrees/feat')
    expect(r.activeId).toBe('tab-2')
  })

  it('关掉最右边的活动标签时回落到左边', () => {
    const r = closeTabsByPath(tabs, 'tab-2', 'C:/other')
    expect(r.tabs.map((t) => t.id)).toEqual(['tab-0', 'tab-1'])
    expect(r.activeId).toBe('tab-1')
  })

  // 删除走的是界面传回来的路径，标签的路径则可能来自 dialog 或 git，
  // 两边写法不一致是常态；比不上就等于标签关不掉
  it('反斜杠、大小写、尾部斜杠的写法都要认得', () => {
    const r = closeTabsByPath(tabs, 'tab-0', 'c:\\REPO\\.worktrees\\Feat\\')
    expect(r.tabs.map((t) => t.id)).toEqual(['tab-0', 'tab-2'])
  })

  it('前缀相同但不是同一个目录的标签不能误关', () => {
    const near: RepoTabInfo[] = [
      { id: 'tab-0', path: 'C:/repo/.worktrees/feat' },
      { id: 'tab-1', path: 'C:/repo/.worktrees/feat2' },
    ]
    const r = closeTabsByPath(near, 'tab-0', 'C:/repo/.worktrees/feat')
    expect(r.tabs.map((t) => t.id)).toEqual(['tab-1'])
  })

  it('没有标签开着那个路径时原样返回', () => {
    const r = closeTabsByPath(tabs, 'tab-0', 'C:/never/opened')
    expect(r.tabs).toBe(tabs)
    expect(r.activeId).toBe('tab-0')
  })

  // normalizePath('/') 和 normalizePath('') 都是空串：空路径一旦参与比较，
  // 会把所有只由斜杠组成的标签一起关掉。这类空输入在本项目的 discard 上出过事故。
  // 三种写法都要挡住——只挡 '' 的话 '/' 和 '\' 照样能进来
  it.each([['空串', ''], ['正斜杠根', '/'], ['反斜杠根', '\\']])(
    '%s 不关任何标签',
    (_label, path) => {
      const withRoot: RepoTabInfo[] = [...tabs, { id: 'tab-3', path: '/' }]
      const r = closeTabsByPath(withRoot, 'tab-0', path)
      expect(r.tabs).toBe(withRoot)
      expect(r.activeId).toBe('tab-0')
    },
  )

  it('空标签列表上关不炸', () => {
    const r = closeTabsByPath([], null, 'C:/repo')
    expect(r.tabs).toEqual([])
    expect(r.activeId).toBeNull()
  })

  // 同一路径理论上只会有一个标签（App 开标签时去重），但去重靠的也是 samePath，
  // 真漂移了这里不能只关掉其中一个、留下另一个死标签
  it('同一路径有多个标签时全部关掉', () => {
    const dup: RepoTabInfo[] = [
      { id: 'tab-0', path: 'C:/repo/wt' },
      { id: 'tab-1', path: 'C:/other' },
      { id: 'tab-2', path: 'c:\\repo\\wt' },
    ]
    const r = closeTabsByPath(dup, 'tab-2', 'C:/repo/wt')
    expect(r.tabs.map((t) => t.id)).toEqual(['tab-1'])
    expect(r.activeId).toBe('tab-1')
  })
})

describe('closeAllTabs', () => {
  it('全部关掉后落到空态', () => {
    const tabs: RepoTabInfo[] = [
      { id: 'tab-0', path: 'C:/a' },
      { id: 'tab-1', path: 'C:/b' },
    ]
    const r = closeAllTabs(tabs, 'tab-1')
    expect(r.tabs).toEqual([])
    expect(r.activeId).toBeNull()
  })

  it('本来就是空列表时不炸', () => {
    const r = closeAllTabs([], null)
    expect(r.tabs).toEqual([])
    expect(r.activeId).toBeNull()
  })
})

describe('closeOtherTabs', () => {
  const tabs: RepoTabInfo[] = [
    { id: 'tab-0', path: 'C:/a' },
    { id: 'tab-1', path: 'C:/b' },
    { id: 'tab-2', path: 'C:/c' },
  ]

  it('只留下被右键的那个标签', () => {
    const r = closeOtherTabs(tabs, 'tab-0', 'tab-1')
    expect(r.tabs).toEqual([{ id: 'tab-1', path: 'C:/b' }])
    expect(r.activeId).toBe('tab-1')
  })

  it('活动标签是被关闭的那批时，留下的标签接管为活动', () => {
    const r = closeOtherTabs(tabs, 'tab-2', 'tab-0')
    expect(r.activeId).toBe('tab-0')
  })

  it('被右键的就是活动标签时活动不动', () => {
    const r = closeOtherTabs(tabs, 'tab-1', 'tab-1')
    expect(r.tabs.map((t) => t.id)).toEqual(['tab-1'])
    expect(r.activeId).toBe('tab-1')
  })

  it('只有一个标签时等于什么都没发生', () => {
    const one: RepoTabInfo[] = [{ id: 'tab-0', path: 'C:/a' }]
    const r = closeOtherTabs(one, 'tab-0', 'tab-0')
    expect(r.tabs).toBe(one)
    expect(r.activeId).toBe('tab-0')
  })

  it('id 不存在时原样返回', () => {
    const r = closeOtherTabs(tabs, 'tab-0', 'tab-99')
    expect(r.tabs).toBe(tabs)
    expect(r.activeId).toBe('tab-0')
  })

  it('空列表上不炸', () => {
    const r = closeOtherTabs([], null, 'tab-0')
    expect(r.tabs).toEqual([])
    expect(r.activeId).toBeNull()
  })
})

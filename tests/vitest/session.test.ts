import { describe, expect, it } from 'vitest'
import { closeTabAt, restoreTabs, type RepoTabInfo, type SessionView } from '@/engine/session'

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

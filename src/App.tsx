import { useCallback, useRef, useState } from 'react'
import { open } from '@tauri-apps/plugin-dialog'
import { RepoTab } from './components/RepoTab'
import { TabBar, type RepoTabInfo } from './components/TabBar'
import { samePath } from './engine/path'
import './styles.css'

const DEFAULT_REPO_PATH = 'C:\\Users\\liuhao\\workspace\\dev\\Dev\\mygitclient'

function App() {
  const nextId = useRef(1)
  const [tabs, setTabs] = useState<RepoTabInfo[]>([{ id: 'tab-0', path: DEFAULT_REPO_PATH }])
  const [activeId, setActiveId] = useState('tab-0')

  // 供上面两个回调读取最新标签列表，避免把 tabs 放进依赖导致回调每次渲染都换新
  const tabsRef = useRef(tabs)
  tabsRef.current = tabs

  /**
   * 同一路径不重复开标签，已开就切过去。
   *
   * id 自增和 setActiveId 都放在 updater 外面——StrictMode 会重复调用 updater，
   * 在里面做副作用会多吃一个 id、并且用过期的 id 去激活标签。
   */
  const openInNewTab = useCallback((path: string) => {
    const existing = tabsRef.current.find((t) => samePath(t.path, path))
    if (existing) {
      setActiveId(existing.id)
      return
    }
    const id = `tab-${nextId.current++}`
    setTabs((prev) => [...prev, { id, path }])
    setActiveId(id)
  }, [])

  const closeTab = useCallback((id: string) => {
    const prev = tabsRef.current
    if (prev.length === 1) return // 至少留一个标签，避免空窗口
    const idx = prev.findIndex((t) => t.id === id)
    if (idx < 0) return
    const next = prev.filter((t) => t.id !== id)
    setTabs(next)
    setActiveId((cur) => (cur === id ? next[Math.min(idx, next.length - 1)].id : cur))
  }, [])

  const handleOpenRepo = useCallback(async () => {
    const selected = await open({ directory: true, multiple: false })
    if (typeof selected === 'string') openInNewTab(selected)
  }, [openInNewTab])

  const openPaths = tabs.map((t) => t.path)

  return (
    <div className="app">
      <TabBar
        tabs={tabs}
        activeId={activeId}
        onActivate={setActiveId}
        onClose={closeTab}
        onOpenRepo={handleOpenRepo}
      />
      {/* 所有标签保持挂载，非活动的用 display:none 藏起来——
          卸载会丢掉该标签的 commits、选中态和滚动位置 */}
      {tabs.map((tab) => (
        <div
          key={tab.id}
          className="tab-panel"
          style={{ display: tab.id === activeId ? 'flex' : 'none' }}
        >
          <RepoTab
            repoPath={tab.path}
            openPaths={openPaths}
            onOpenInNewTab={openInNewTab}
          />
        </div>
      ))}
    </div>
  )
}

export default App

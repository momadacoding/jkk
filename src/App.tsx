import { useCallback, useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import { EmptyState } from './components/EmptyState'
import { RepoTab } from './components/RepoTab'
import { TabBar } from './components/TabBar'
import { samePath } from './engine/path'
import {
  closeTabAt,
  closeTabsByPath,
  restoreTabs,
  type RecentRepo,
  type RepoTabInfo,
  type SessionView,
} from './engine/session'
import './styles.css'

function App() {
  const nextId = useRef(0)
  const [tabs, setTabs] = useState<RepoTabInfo[]>([])
  const [activeId, setActiveId] = useState<string | null>(null)
  const [recent, setRecent] = useState<RecentRepo[]>([])
  /**
   * 会话还原完成前禁止落盘。
   *
   * 首帧的 tabs 是空数组，这时候若触发保存，会把上次的会话直接冲掉——
   * 用户看到的就是「每次启动都是空的」。
   */
  const [restored, setRestored] = useState(false)
  /**
   * 跳过紧接还原之后的那一次自动保存。
   *
   * 还原不完整时（标签被滤掉、或用户抢先开了仓库）立刻回存，会把存档里
   * 那些暂时打不开的标签永久抹掉。等用户真的动了标签再存。
   */
  const skipNextSave = useRef(false)

  // 供下面的回调读取最新状态，避免把 tabs / activeId 放进依赖导致回调每次渲染都换新
  const tabsRef = useRef(tabs)
  tabsRef.current = tabs
  const activeIdRef = useRef(activeId)
  activeIdRef.current = activeId

  useEffect(() => {
    void (async () => {
      try {
        const view = await invoke<SessionView>('load_session')
        // 还原是异步的，而「Open Repo」按钮从第一帧就能点。真撞上时以用户
        // 刚开的那个为准——把它替换掉等于让他的选择凭空消失
        const alreadyOpen = tabsRef.current.length
        const next = restoreTabs(view, alreadyOpen)
        skipNextSave.current = next.partial
        if (alreadyOpen === 0) {
          nextId.current = next.nextId
          setTabs(next.tabs)
          setActiveId(next.activeId)
        }
        setRecent(view.recent)
      } catch {
        // 会话读不出来就当第一次启动。这里不该把错误顶到启动路径上：
        // 用户能做的只有「打开一个仓库」，空态页已经在说这件事了。
      } finally {
        setRestored(true)
      }
    })()
  }, [])

  // 标签或活动标签一变就落盘。不做防抖：写的是几百字节的 JSON，
  // 而标签变动本身就是低频操作。
  useEffect(() => {
    if (!restored) return
    if (skipNextSave.current) {
      skipNextSave.current = false
      return
    }
    const activePath = tabs.find((t) => t.id === activeId)?.path ?? null
    void invoke<RecentRepo[]>('save_session', {
      tabs: tabs.map((t) => t.path),
      activeTab: activePath,
    })
      // 最近列表由后端统一维护（去重、排序、截断），前端只接收结果，
      // 免得两侧的规则各写一套之后漂移
      .then(setRecent)
      .catch(() => {
        // 存不下来只影响下次启动的还原，不该打断当前会话
      })
  }, [tabs, activeId, restored])

  /**
   * 同一路径不重复开标签，已开就切过去。
   *
   * id 自增和 setActiveId 都放在 updater 外面——StrictMode 会重复调用 updater，
   * 在里面做副作用会多吃一个 id、并且用过期的 id 去激活标签。
   */
  const openInNewTab = useCallback((path: string, opts?: { activate?: boolean }) => {
    const activate = opts?.activate ?? true
    const existing = tabsRef.current.find((t) => samePath(t.path, path))
    if (existing) {
      if (activate) setActiveId(existing.id)
      return
    }
    const id = `tab-${nextId.current++}`
    setTabs((prev) => [...prev, { id, path }])
    if (activate) setActiveId(id)
  }, [])

  // 同上：算好新的标签列表和活动标签再一起 set，
  // 不要把 setTabs 塞进 setActiveId 的 updater 里——那是个副作用，StrictMode 会跑两遍
  const closeTab = useCallback((id: string) => {
    const next = closeTabAt(tabsRef.current, activeIdRef.current, id)
    setTabs(next.tabs)
    setActiveId(next.activeId)
  }, [])

  /**
   * 某个仓库目录已经被删掉了，把开着它的标签关掉。
   *
   * 只接明确的删除动作（删 worktree、删分支时连带删 worktree），**不要**接
   * 「路径探测不到」：网络盘和移动硬盘临时离线时那会把用户的会话悄悄清空。
   */
  const handleRepoRemoved = useCallback((path: string) => {
    const next = closeTabsByPath(tabsRef.current, activeIdRef.current, path)
    if (next.tabs === tabsRef.current) return
    setTabs(next.tabs)
    setActiveId(next.activeId)
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
      {/* 还原完成前什么都不渲染：先闪一下空态页再跳出标签很难看 */}
      {restored && tabs.length === 0 && (
        <EmptyState
          recent={recent}
          onOpenRepo={handleOpenRepo}
          onOpenPath={openInNewTab}
        />
      )}
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
            onRepoRemoved={handleRepoRemoved}
          />
        </div>
      ))}
    </div>
  )
}

export default App

import { basename } from '../engine/path'
import type { RepoTabInfo } from '../engine/session'
import { ContextMenu, useContextMenu } from './ContextMenu'

interface TabBarProps {
  tabs: RepoTabInfo[]
  activeId: string | null
  onActivate: (id: string) => void
  onClose: (id: string) => void
  onCloseAll: () => void
  onCloseOthers: (id: string) => void
  onOpenRepo: () => void
}

export function TabBar({ tabs, activeId, onActivate, onClose, onCloseAll, onCloseOthers, onOpenRepo }: TabBarProps) {
  const { menu, open, close } = useContextMenu()
  return (
    <div className="tab-bar">
      {tabs.map((tab) => (
        <div
          key={tab.id}
          className={tab.id === activeId ? 'tab active' : 'tab'}
          title={tab.path}
          onClick={() => onActivate(tab.id)}
          // 中键关闭标签：mousedown 先 preventDefault 拦住 Windows 的自动滚动，
          // auxclick（鼠标抬起）再关，和浏览器中键关标签的手感一致；
          // 点在 × 按钮上的中键也会冒泡到这里，同样关掉
          onMouseDown={(e) => { if (e.button === 1) e.preventDefault() }}
          onAuxClick={(e) => { if (e.button === 1) onClose(tab.id) }}
          onContextMenu={(e) =>
            open(e, [
              { label: 'Close All', onSelect: onCloseAll },
              { label: 'Close All but This', onSelect: () => onCloseOthers(tab.id) },
            ])
          }
        >
          <span className="tab-title">{basename(tab.path)}</span>
          {/* 最后一个标签也能关：关完落到空态页，不再需要「至少留一个」这条约束 */}
          <button
            className="tab-close"
            title="Close tab"
            onClick={(e) => { e.stopPropagation(); onClose(tab.id) }}
          >
            ×
          </button>
        </div>
      ))}
      {/* 带文字标签：只有一个灰色小加号时用户找不到「打开仓库」在哪 */}
      <button className="tab-open" onClick={onOpenRepo} title="Open a repository in a new tab">
        <span className="tab-open-icon">+</span>
        <span>Open Repo</span>
      </button>
      <ContextMenu menu={menu} onClose={close} />
    </div>
  )
}

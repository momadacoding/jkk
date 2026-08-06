export interface RepoTabInfo {
  id: string
  path: string
}

interface TabBarProps {
  tabs: RepoTabInfo[]
  activeId: string
  onActivate: (id: string) => void
  onClose: (id: string) => void
  onOpenRepo: () => void
}

/** 取路径最后一段作为标签标题。 */
export function basename(path: string): string {
  const parts = path.replace(/\\/g, '/').replace(/\/+$/, '').split('/')
  return parts[parts.length - 1] || path
}

export function TabBar({ tabs, activeId, onActivate, onClose, onOpenRepo }: TabBarProps) {
  return (
    <div className="tab-bar">
      {tabs.map((tab) => (
        <div
          key={tab.id}
          className={tab.id === activeId ? 'tab active' : 'tab'}
          title={tab.path}
          onClick={() => onActivate(tab.id)}
        >
          <span className="tab-title">{basename(tab.path)}</span>
          {/* 只剩一个标签时不给关，避免出现空窗口 */}
          {tabs.length > 1 && (
            <button
              className="tab-close"
              title="Close tab"
              onClick={(e) => { e.stopPropagation(); onClose(tab.id) }}
            >
              ×
            </button>
          )}
        </div>
      ))}
      {/* 带文字标签：只有一个灰色小加号时用户找不到「打开仓库」在哪 */}
      <button className="tab-open" onClick={onOpenRepo} title="Open a repository in a new tab">
        <span className="tab-open-icon">+</span>
        <span>Open Repo</span>
      </button>
    </div>
  )
}

import { basename } from '../engine/path'
import type { RecentRepo } from '../engine/session'

interface EmptyStateProps {
  recent: RecentRepo[]
  onOpenRepo: () => void
  onOpenPath: (path: string) => void
}

/**
 * 一个标签都没开时的落地页。
 *
 * 已失效的最近项仍然可点：目录可能只是暂时不在（移动硬盘、网络盘没连上），
 * 点开会走 RepoTab 既有的错误提示，比在这里直接拦掉更容易让用户判断状况。
 */
export function EmptyState({ recent, onOpenRepo, onOpenPath }: EmptyStateProps) {
  return (
    <div className="empty-state">
      <div className="empty-state-box">
        <div className="empty-state-title">No repository open</div>
        <button className="empty-state-open" onClick={onOpenRepo}>
          Open Repository…
        </button>

        {recent.length > 0 && (
          <div className="empty-state-recent">
            <div className="empty-state-recent-label">Recent</div>
            {recent.map((repo) => (
              <button
                key={repo.path}
                className={repo.exists ? 'recent-item' : 'recent-item missing'}
                title={repo.exists ? repo.path : `${repo.path} — not found`}
                onClick={() => onOpenPath(repo.path)}
              >
                <span className="recent-name">{basename(repo.path)}</span>
                <span className="recent-path">{repo.path}</span>
              </button>
            ))}
          </div>
        )}
      </div>
    </div>
  )
}

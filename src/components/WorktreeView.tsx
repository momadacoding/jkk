import type { Worktree } from '../engine/types'

interface WorktreeViewProps {
  worktrees: Worktree[]
  currentPath: string
  onSwitch: (path: string) => void
  onDelete: (name: string) => void
  onCreate: (name: string) => void
}

export function WorktreeView({ worktrees, currentPath, onSwitch, onDelete, onCreate }: WorktreeViewProps) {
  return (
    <div className="sidebar-panel">
      <h3>Worktrees</h3>
      {worktrees.map((wt) => (
        <div key={wt.path} className={wt.path === currentPath ? 'worktree-row active' : 'worktree-row'}>
          <span onClick={() => onSwitch(wt.path)}>{wt.name} {wt.branch && `(${wt.branch})`}</span>
          {!wt.isMain && <button onClick={() => onDelete(wt.name)}>×</button>}
        </div>
      ))}
      <button onClick={() => onCreate(`wt-${Date.now()}`)}>+ New worktree</button>
    </div>
  )
}

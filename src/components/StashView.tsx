import type { StashEntry } from '../engine/types'
import { ContextMenu, useContextMenu } from './ContextMenu'

interface StashViewProps {
  stashes: StashEntry[]
  /** 点击条目时用它的 hash 走 `commit:<hash>` diff scope。 */
  onSelect: (hash: string) => void
  onApply: (index: number) => void
  onPop: (index: number) => void
  onDrop: (index: number) => void
}

export function StashView({ stashes, onSelect, onApply, onPop, onDrop }: StashViewProps) {
  const { menu, open, close } = useContextMenu()

  return (
    <div className="sidebar-panel">
      <h3>Stashes</h3>
      {stashes.length === 0 && <div className="changes-empty">No stashes</div>}
      {stashes.map((entry) => {
        const items = [
          { label: 'Apply', onSelect: () => onApply(entry.index) },
          { label: 'Pop', onSelect: () => onPop(entry.index) },
          { label: 'Drop', onSelect: () => onDrop(entry.index), danger: true },
        ]
        return (
          <div
            key={entry.hash}
            className="stash-row"
            title="点击查看内容 · 右键更多操作"
            onClick={() => onSelect(entry.hash)}
            onContextMenu={(e) => open(e, items)}
          >
            <span className="stash-index">{`{${entry.index}}`}</span>
            <span className="stash-message">{entry.message}</span>
          </div>
        )
      })}
      <ContextMenu menu={menu} onClose={close} />
    </div>
  )
}

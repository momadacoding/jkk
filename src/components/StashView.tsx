import type { StashEntry } from '../engine/types'
import { ContextMenu, useContextMenu } from './ContextMenu'
import { SidebarSection } from './SidebarSection'
import { shortenStashMessage } from '../engine/stashLabel'

interface StashViewProps {
  stashes: StashEntry[]
  /** 点击条目时用它的 hash 走 `commit:<hash>` diff scope。 */
  onSelect: (hash: string) => void
  onApply: (index: number) => void
  onPop: (index: number) => void
  onDrop: (index: number) => void
  /** 当前选中的提交 id，用来高亮正在预览的那条 stash。 */
  selectedId: string | null
}

function StashIcon() {
  return (
    <svg className="sb-icon" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.3">
      <path d="M2.5 5.5l5.5-3 5.5 3-5.5 3z" />
      <path d="M2.5 8.5l5.5 3 5.5-3" />
    </svg>
  )
}

export function StashView({ stashes, onSelect, onApply, onPop, onDrop, selectedId }: StashViewProps) {
  const { menu, open, close } = useContextMenu()

  return (
    <SidebarSection title="STASHES" count={stashes.length} defaultOpen={stashes.length > 0}>
      {stashes.length === 0 && <div className="sb-empty">No stashes</div>}
      {stashes.map((entry) => {
        const items = [
          { label: 'Apply', onSelect: () => onApply(entry.index) },
          { label: 'Pop', onSelect: () => onPop(entry.index) },
          { label: 'Drop', onSelect: () => onDrop(entry.index), danger: true },
        ]
        return (
          <div
            key={entry.hash}
            className={entry.hash === selectedId ? 'sb-row selected' : 'sb-row'}
            title={`${entry.message}\n单击查看内容 · 右键更多操作`}
            onClick={() => onSelect(entry.hash)}
            onContextMenu={(e) => open(e, items)}
          >
            <StashIcon />
            <span className="sb-index">{`{${entry.index}}`}</span>
            <span className="sb-label">{shortenStashMessage(entry.message)}</span>
            <span className="sb-actions">
              <button title="把改动取回工作区并保留这条" onClick={(e) => { e.stopPropagation(); onApply(entry.index) }}>
                ↥
              </button>
              <button className="danger" title="丢弃这条 stash" onClick={(e) => { e.stopPropagation(); onDrop(entry.index) }}>
                ✕
              </button>
            </span>
          </div>
        )
      })}
      <ContextMenu menu={menu} onClose={close} />
    </SidebarSection>
  )
}

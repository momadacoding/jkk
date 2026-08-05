import { useState } from 'react'
import type { RefPointer } from '../engine/types'
import { ContextMenu, useContextMenu } from './ContextMenu'

interface BranchViewProps {
  refs: RefPointer[]
  head: string
  /** 当前选中的分支（仅用于高亮与定位，不改工作区）。 */
  selectedBranch: string | null
  onSelect: (name: string, hash: string) => void
  onCheckout: (name: string) => void
  onCreate: (name: string, start?: string) => void
  onRename: (oldName: string, newName: string) => void
  onDelete: (name: string) => void
}

const shortName = (ref: RefPointer) =>
  ref.name.replace('refs/heads/', '').replace('refs/remotes/', '')

export function BranchView({
  refs, head, selectedBranch, onSelect, onCheckout, onCreate, onRename, onDelete,
}: BranchViewProps) {
  const [newName, setNewName] = useState('')
  const { menu, open, close } = useContextMenu()
  const local = refs.filter((r) => r.kind === 'branch')
  const remote = refs.filter((r) => r.kind === 'remote')

  const branchMenu = (name: string) => [
    {
      label: `Checkout ${name}`,
      onSelect: () => onCheckout(name),
      disabled: name === head,
    },
    {
      label: 'Rename…',
      onSelect: () => {
        const next = window.prompt(`Rename branch "${name}" to:`, name)
        if (next && next !== name) onRename(name, next)
      },
    },
    {
      label: 'New branch from here…',
      onSelect: () => {
        const child = window.prompt(`New branch starting at "${name}":`, '')
        if (child) onCreate(child, name)
      },
    },
    {
      label: 'Delete',
      onSelect: () => onDelete(name),
      disabled: name === head,
      danger: true,
    },
  ]

  return (
    <div className="sidebar-panel">
      <h3>Branches</h3>
      <div className="branch-section">
        <strong>Local</strong>
        {local.map((ref) => {
          const name = shortName(ref)
          const classes = ['branch-row']
          if (name === head) classes.push('head')
          if (name === selectedBranch) classes.push('selected')
          return (
            <div
              key={ref.name}
              className={classes.join(' ')}
              title={`单击选中 · 双击 checkout · 右键更多操作`}
              onClick={() => onSelect(name, ref.hash)}
              onDoubleClick={() => { if (name !== head) onCheckout(name) }}
              onContextMenu={(e) => open(e, branchMenu(name))}
            >
              <span className="branch-name">{name}</span>
            </div>
          )
        })}
      </div>
      <div className="branch-section">
        <strong>Remote</strong>
        {remote.map((ref) => (
          <div
            key={ref.name}
            className={shortName(ref) === selectedBranch ? 'branch-row remote selected' : 'branch-row remote'}
            onClick={() => onSelect(shortName(ref), ref.hash)}
          >
            <span className="branch-name">{shortName(ref)}</span>
          </div>
        ))}
      </div>
      <div className="branch-actions">
        <input
          value={newName}
          onChange={(e) => setNewName(e.target.value)}
          onKeyDown={(e) => { if (e.key === 'Enter' && newName.trim()) { onCreate(newName.trim()); setNewName('') } }}
          placeholder="new branch"
        />
        <button
          disabled={!newName.trim()}
          onClick={() => { onCreate(newName.trim()); setNewName('') }}
        >
          Create
        </button>
      </div>
      <ContextMenu menu={menu} onClose={close} />
    </div>
  )
}

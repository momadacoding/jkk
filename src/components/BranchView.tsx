import { useEffect, useRef, useState } from 'react'
import type { RefPointer } from '../engine/types'
import { ContextMenu, useContextMenu } from './ContextMenu'
import { SidebarSection, SidebarGroup } from './SidebarSection'

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

function BranchIcon() {
  return (
    <svg className="sb-icon" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.3">
      <circle cx="4.5" cy="3.5" r="1.7" />
      <circle cx="4.5" cy="12.5" r="1.7" />
      <circle cx="11.5" cy="6.5" r="1.7" />
      <path d="M4.5 5.2v5.6M11.5 8.2c0 2.2-2.6 2.4-4.4 3" />
    </svg>
  )
}

/** 领先/落后微徽章。没有上游（null）时什么都不画。 */
function SyncBadge({ ahead, behind }: { ahead: number | null; behind: number | null }) {
  if (!ahead && !behind) return null
  return (
    <span className="sb-sync">
      {/* 箭头由 CSS ::before 画，好在它和数字之间留出空隙——
          紧挨着的话 ↑10 会被读成 110 */}
      {behind ? <span className="sb-sync-behind">{behind}</span> : null}
      {ahead ? <span className="sb-sync-ahead">{ahead}</span> : null}
    </span>
  )
}

export function BranchView({
  refs, head, selectedBranch, onSelect, onCheckout, onCreate, onRename, onDelete,
}: BranchViewProps) {
  const [creating, setCreating] = useState(false)
  const [newName, setNewName] = useState('')
  const inputRef = useRef<HTMLInputElement>(null)
  const { menu, open, close } = useContextMenu()
  const local = refs.filter((r) => r.kind === 'branch')
  const remote = refs.filter((r) => r.kind === 'remote')

  // 展开输入框却不聚焦的话，用户得多点一次才能打字
  useEffect(() => { if (creating) inputRef.current?.focus() }, [creating])

  const submit = () => {
    const name = newName.trim()
    if (name) onCreate(name)
    setNewName('')
    setCreating(false)
  }

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
    <SidebarSection
      title="BRANCHES"
      action={{ title: 'New branch', onClick: () => setCreating(true) }}
    >
      {creating && (
        <div className="sb-inline-form">
          <input
            ref={inputRef}
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter') submit()
              // Esc 收起来，否则这个输入框会一直杵在那里
              if (e.key === 'Escape') { setNewName(''); setCreating(false) }
            }}
            onBlur={() => { if (!newName.trim()) setCreating(false) }}
            placeholder="branch name"
          />
        </div>
      )}

      <SidebarGroup label="Local" count={local.length}>
        {local.map((ref) => {
          const name = shortName(ref)
          const classes = ['sb-row']
          if (name === head) classes.push('current')
          if (name === selectedBranch) classes.push('selected')
          return (
            <div
              key={ref.name}
              className={classes.join(' ')}
              title={`${name}\n单击定位 · 双击 checkout · 右键更多操作`}
              onClick={() => onSelect(name, ref.hash)}
              onDoubleClick={() => { if (name !== head) onCheckout(name) }}
              onContextMenu={(e) => open(e, branchMenu(name))}
            >
              <BranchIcon />
              <span className="sb-label">{name}</span>
              <SyncBadge ahead={ref.ahead} behind={ref.behind} />
              {name === head && <span className="sb-current-dot" title="当前分支" />}
            </div>
          )
        })}
      </SidebarGroup>

      <SidebarGroup label="Remote" count={remote.length} defaultOpen={false}>
        {remote.map((ref) => (
          <div
            key={ref.name}
            className={shortName(ref) === selectedBranch ? 'sb-row remote selected' : 'sb-row remote'}
            title={`${shortName(ref)}\n单击定位`}
            onClick={() => onSelect(shortName(ref), ref.hash)}
          >
            <BranchIcon />
            <span className="sb-label">{shortName(ref)}</span>
          </div>
        ))}
      </SidebarGroup>

      <ContextMenu menu={menu} onClose={close} />
    </SidebarSection>
  )
}

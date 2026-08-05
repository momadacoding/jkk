import { useState } from 'react'
import type { RefPointer } from '../engine/types'

interface BranchViewProps {
  refs: RefPointer[]
  head: string
  onCheckout: (name: string) => void
  onCreate: (name: string, start?: string) => void
  onDelete: (name: string) => void
}

export function BranchView({ refs, head, onCheckout, onCreate, onDelete }: BranchViewProps) {
  const [newName, setNewName] = useState('')
  const local = refs.filter((r) => r.kind === 'branch')
  const remote = refs.filter((r) => r.kind === 'remote')

  return (
    <div className="sidebar-panel">
      <h3>Branches</h3>
      <div className="branch-section">
        <strong>Local</strong>
        {local.map((ref) => (
          <div key={ref.name} className={ref.name.replace('refs/heads/', '') === head ? 'branch-row head' : 'branch-row'}>
            <span onClick={() => onCheckout(ref.name.replace('refs/heads/', ''))}>
              {ref.name.replace('refs/heads/', '')}
            </span>
            <button onClick={() => onDelete(ref.name.replace('refs/heads/', ''))}>×</button>
          </div>
        ))}
      </div>
      <div className="branch-section">
        <strong>Remote</strong>
        {remote.map((ref) => (
          <div key={ref.name} className="branch-row remote">
            {ref.name.replace('refs/remotes/', '')}
          </div>
        ))}
      </div>
      <div className="branch-actions">
        <input value={newName} onChange={(e) => setNewName(e.target.value)} placeholder="new branch" />
        <button onClick={() => { onCreate(newName); setNewName('') }}>Create</button>
      </div>
    </div>
  )
}

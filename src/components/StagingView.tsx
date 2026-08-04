import { useState } from 'react'
import type { FileChange, StatusData, DiffData } from '../engine/types'
import { DiffHunk } from './DiffHunk'

interface StagingViewProps {
  status: StatusData
  diff: DiffData
  onStage: (paths: string[]) => void
  onUnstage: (paths: string[]) => void
  onCommit: (msg: string, amend: boolean) => void
  onSelectFile: (path: string) => void
  selectedFile: string | null
}

function FileList({ title, files, onToggle, selected, onSelect }: {
  title: string
  files: FileChange[]
  onToggle: (path: string) => void
  selected: string | null
  onSelect: (path: string) => void
}) {
  return (
    <div className="file-list">
      <strong>{title}</strong>
      {files.map((f) => (
        <div key={f.path} className={selected === f.path ? 'file-row selected' : 'file-row'}>
          <input type="checkbox" checked onChange={() => onToggle(f.path)} />
          <span onClick={() => onSelect(f.path)}>{f.path}</span>
          <span className="file-stats">+{f.additions}/-{f.deletions}</span>
        </div>
      ))}
    </div>
  )
}

export function StagingView(props: StagingViewProps) {
  const [message, setMessage] = useState('')
  const [amend, setAmend] = useState(false)

  return (
    <div className="staging-view">
      <div className="staging-sidebar">
        <FileList title="Unstaged" files={props.status.unstaged} onToggle={props.onStage} selected={props.selectedFile} onSelect={props.onSelectFile} />
        <FileList title="Staged" files={props.status.staged} onToggle={props.onUnstage} selected={props.selectedFile} onSelect={props.onSelectFile} />
        <div className="commit-box">
          <textarea value={message} onChange={(e) => setMessage(e.target.value)} placeholder="Commit message" />
          <label><input type="checkbox" checked={amend} onChange={(e) => setAmend(e.target.checked)} /> Amend</label>
          <button onClick={() => props.onCommit(message, amend)}>Commit</button>
        </div>
      </div>
      <div className="staging-diff">
        <DiffHunk diff={props.diff} />
      </div>
    </div>
  )
}

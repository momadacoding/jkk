import { useState, useMemo } from 'react'
import type { DiffData, DiffLine, FileChange, StatusData } from '../engine/types'

function FileListGroup({ title, files, selected, onSelect, onToggle }: {
  title: string
  files: FileChange[]
  selected: string | null
  onSelect: (path: string) => void
  onToggle: (path: string) => void
}) {
  if (files.length === 0) return null
  return (
    <div className="changes-file-group">
      <div className="changes-group-title">{title}</div>
      {files.map((f) => (
        <div
          key={f.path}
          className={selected === f.path ? 'changes-file selected' : 'changes-file'}
          onClick={() => onSelect(f.path)}
        >
          <button className="changes-toggle" onClick={(e) => { e.stopPropagation(); onToggle(f.path) }}>
            {title === 'Unstaged' ? 'Stage' : 'Unstage'}
          </button>
          <span className="changes-file-path">{f.path}</span>
          <span className="changes-file-stats">+{f.additions}/-{f.deletions}</span>
        </div>
      ))}
    </div>
  )
}

function DiffLineView({ line }: { line: DiffLine }) {
  const className = line.origin === '+' ? 'diff-line add' : line.origin === '-' ? 'diff-line del' : 'diff-line'
  return (
    <div className={className}>
      <span className="diff-origin">{line.origin}</span>
      {line.content}
    </div>
  )
}

function DiffFile({ diff }: { diff: DiffData }) {
  if (diff.files.length === 0) return <div className="empty-diff">No changes</div>
  return (
    <div className="diff-view">
      {diff.files.map((file) => (
        <div key={file.path} className="diff-file">
          <div className="diff-file-header">{file.path}</div>
          {file.hunks.map((hunk, i) => (
            <div key={i} className="diff-hunk">
              {hunk.lines.map((line, j) => <DiffLineView key={j} line={line} />)}
            </div>
          ))}
        </div>
      ))}
    </div>
  )
}

interface LocalChangesViewProps {
  status: StatusData
  diff: DiffData
  onStage: (paths: string[]) => void
  onUnstage: (paths: string[]) => void
  onCommit: (msg: string, amend: boolean) => void
}

export function LocalChangesView({ status, diff, onStage, onUnstage, onCommit }: LocalChangesViewProps) {
  const [selectedFile, setSelectedFile] = useState<string | null>(null)
  const [message, setMessage] = useState('')
  const [amend, setAmend] = useState(false)

  const allFiles = useMemo(() => [...status.unstaged, ...status.staged], [status])
  const activeFile = selectedFile ?? allFiles[0]?.path ?? null
  const activeDiff: DiffData = useMemo(() => ({
    files: diff.files.filter((f) => f.path === activeFile),
  }), [diff, activeFile])

  return (
    <div className="local-changes-view">
      <aside className="changes-sidebar">
        <FileListGroup
          title="Unstaged"
          files={status.unstaged}
          selected={activeFile}
          onSelect={setSelectedFile}
          onToggle={(path) => onStage([path])}
        />
        <FileListGroup
          title="Staged"
          files={status.staged}
          selected={activeFile}
          onSelect={setSelectedFile}
          onToggle={(path) => onUnstage([path])}
        />
      </aside>
      <div className="changes-main">
        <div className="changes-diff">
          <DiffFile diff={activeDiff} />
        </div>
        <div className="changes-commit-box">
          <textarea
            value={message}
            onChange={(e) => setMessage(e.target.value)}
            placeholder="Commit subject"
            rows={2}
          />
          <label className="changes-amend">
            <input type="checkbox" checked={amend} onChange={(e) => setAmend(e.target.checked)} /> Amend
          </label>
          <button onClick={() => onCommit(message, amend)}>Commit</button>
        </div>
      </div>
    </div>
  )
}

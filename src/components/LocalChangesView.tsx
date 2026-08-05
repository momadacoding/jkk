import { useMemo, useState } from 'react'
import { Panel, Group, Separator } from 'react-resizable-panels'
import type { DiffData, FileChange, StatusData } from '../engine/types'
import { DiffHunk } from './DiffHunk'

function FileSection({ title, files, actionLabel, selected, onSelect, onToggleOne, onToggleAll }: {
  title: string
  files: FileChange[]
  actionLabel: string
  selected: string | null
  onSelect: (path: string) => void
  onToggleOne: (path: string) => void
  onToggleAll: () => void
}) {
  return (
    <div className="changes-section">
      <div className="changes-section-header">
        <span>{title} ({files.length})</span>
        {files.length > 0 && (
          <button className="changes-action" onClick={onToggleAll}>{actionLabel} All</button>
        )}
      </div>
      <div className="changes-section-files">
        {files.map((f) => (
          <div
            key={f.path}
            className={selected === f.path ? 'changes-file selected' : 'changes-file'}
            onClick={() => onSelect(f.path)}
          >
            <button
              className="changes-toggle"
              onClick={(e) => { e.stopPropagation(); onToggleOne(f.path) }}
            >
              {actionLabel}
            </button>
            <span className="changes-file-path">{f.path}</span>
            <span className="changes-file-stats">+{f.additions}/-{f.deletions}</span>
          </div>
        ))}
        {files.length === 0 && <div className="changes-empty">No files</div>}
      </div>
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
  const [subject, setSubject] = useState('')
  const [description, setDescription] = useState('')
  const [amend, setAmend] = useState(false)

  const allFiles = useMemo(() => [...status.unstaged, ...status.staged], [status])
  const activeFile = selectedFile ?? allFiles[0]?.path ?? null
  const activeDiff: DiffData = useMemo(() => ({
    files: diff.files.filter((f) => f.path === activeFile),
  }), [diff, activeFile])

  const handleCommit = () => {
    const msg = description.trim() ? `${subject.trim()}\n\n${description.trim()}` : subject.trim()
    onCommit(msg, amend)
    setSubject('')
    setDescription('')
  }

  return (
    <div className="local-changes-view">
      <div className="changes-body">
        <Group orientation="horizontal">
          <Panel defaultSize="25%" minSize="15%" maxSize="50%">
            <Group orientation="vertical">
              <Panel minSize="15%">
                <FileSection
                  title="Unstaged"
                  files={status.unstaged}
                  actionLabel="Stage"
                  selected={activeFile}
                  onSelect={setSelectedFile}
                  onToggleOne={(path) => onStage([path])}
                  onToggleAll={() => onStage(status.unstaged.map((f) => f.path))}
                />
              </Panel>
              <Separator className="separator-h" />
              <Panel minSize="15%">
                <FileSection
                  title="Staged"
                  files={status.staged}
                  actionLabel="Unstage"
                  selected={activeFile}
                  onSelect={setSelectedFile}
                  onToggleOne={(path) => onUnstage([path])}
                  onToggleAll={() => onUnstage(status.staged.map((f) => f.path))}
                />
              </Panel>
            </Group>
          </Panel>
          <Separator className="separator-v" />
          <Panel minSize="30%">
            <DiffHunk diff={activeDiff} />
          </Panel>
        </Group>
      </div>
      <div className="changes-commit-box">
        <input
          className="commit-subject"
          value={subject}
          onChange={(e) => setSubject(e.target.value)}
          placeholder="Commit subject"
        />
        <textarea
          className="commit-description"
          value={description}
          onChange={(e) => setDescription(e.target.value)}
          placeholder="Description"
          rows={3}
        />
        <div className="commit-actions">
          <label className="changes-amend">
            <input type="checkbox" checked={amend} onChange={(e) => setAmend(e.target.checked)} /> Amend
          </label>
          <button disabled={!subject.trim()} onClick={handleCommit}>Commit</button>
        </div>
      </div>
    </div>
  )
}

import { useMemo, useState } from 'react'
import type { CommitNode, DiffData } from '../engine/types'
import { DiffHunk } from './DiffHunk'

interface CommitDetailViewProps {
  commit: CommitNode | null
  diff: DiffData
}

interface FileStat {
  path: string
  additions: number
  deletions: number
}

export function CommitDetailView({ commit, diff }: CommitDetailViewProps) {
  const [tab, setTab] = useState<'commit' | 'changes'>('commit')

  const fileStats: FileStat[] = useMemo(
    () =>
      diff.files.map((f) => ({
        path: f.path,
        additions: f.hunks.reduce((n, h) => n + h.lines.filter((l) => l.origin === '+').length, 0),
        deletions: f.hunks.reduce((n, h) => n + h.lines.filter((l) => l.origin === '-').length, 0),
      })),
    [diff],
  )

  if (!commit) {
    return <div className="commit-detail commit-detail-empty">Select a commit</div>
  }

  return (
    <div className="commit-detail">
      <div className="commit-detail-tabs">
        <button className={tab === 'commit' ? 'active' : ''} onClick={() => setTab('commit')}>Commit</button>
        <button className={tab === 'changes' ? 'active' : ''} onClick={() => setTab('changes')}>Changes</button>
      </div>
      {tab === 'commit' ? (
        <div className="commit-info">
          <div className="commit-info-row">
            <span className="commit-info-label">AUTHOR</span>
            <span>{commit.author}</span>
            <span className="commit-info-date">{new Date(commit.time * 1000).toLocaleString()}</span>
          </div>
          <div className="commit-info-row">
            <span className="commit-info-label">SHA</span>
            <span className="commit-info-sha">{commit.id}</span>
          </div>
          <div className="commit-info-row">
            <span className="commit-info-label">PARENTS</span>
            <span className="commit-info-sha">
              {commit.parents.length > 0 ? commit.parents.map((p) => p.slice(0, 7)).join(' ') : '(none)'}
            </span>
          </div>
          <div className="commit-full-message">{commit.message}</div>
          <div className="commit-files">
            {fileStats.map((f) => (
              <div key={f.path} className="commit-file-row">
                <span className="commit-file-path">{f.path}</span>
                <span className="file-stats">+{f.additions}/-{f.deletions}</span>
              </div>
            ))}
          </div>
        </div>
      ) : (
        <div className="commit-changes">
          <DiffHunk diff={diff} />
        </div>
      )}
    </div>
  )
}

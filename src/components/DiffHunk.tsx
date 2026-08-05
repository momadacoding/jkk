import type { DiffData, DiffLine } from '../engine/types'

interface DiffHunkProps {
  diff: DiffData
}

function Line({ line }: { line: DiffLine }) {
  const className = line.origin === '+' ? 'diff-line add' : line.origin === '-' ? 'diff-line del' : 'diff-line'
  return (
    <div className={className}>
      <span className="diff-origin">{line.origin}</span>
      {line.content}
    </div>
  )
}

export function DiffHunk({ diff }: DiffHunkProps) {
  if (diff.files.length === 0) return <div className="empty-diff">No changes</div>
  return (
    <div className="diff-view">
      {diff.files.map((file) => (
        <div key={file.path} className="diff-file">
          <div className="diff-file-header">{file.path}</div>
          {file.hunks.map((hunk, i) => (
            <div key={i} className="diff-hunk">
              {hunk.lines.map((line, j) => <Line key={j} line={line} />)}
            </div>
          ))}
        </div>
      ))}
    </div>
  )
}

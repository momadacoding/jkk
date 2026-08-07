import type { DiffData, DiffLine, FileDiff } from '../engine/types'

function Line({ line }: { line: DiffLine }) {
  const kind = line.origin === '+' ? ' add' : line.origin === '-' ? ' del' : ''
  return (
    <div className={`diff-line${kind}`}>
      {/* 两个行号和 +/- 符号绑在一起横向吸附，长行左右滚动时行号仍然贴在左边 */}
      <span className="diff-gutter">
        <span className="diff-ln">{line.oldLineno ?? ''}</span>
        <span className="diff-ln">{line.newLineno ?? ''}</span>
        <span className="diff-sign">{line.origin === ' ' ? '' : line.origin}</span>
      </span>
      <span className="diff-code">{line.content}</span>
    </div>
  )
}

function FileBody({ file }: { file: FileDiff }) {
  if (file.binary) {
    return <div className="diff-note">二进制文件，无法按行显示</div>
  }
  if (file.hunks.length === 0) {
    return <div className="diff-note">没有文本改动</div>
  }
  return (
    <>
      {file.hunks.map((hunk, i) => (
        <div key={i} className="diff-hunk">
          {/* 文字单独包一层才吸得住：外层和它的包含块一样宽，没有可以位移的余地 */}
          <div className="diff-hunk-header">
            <span>{hunk.header || `@@ -${hunk.oldStart} +${hunk.newStart} @@`}</span>
          </div>
          {hunk.lines.map((line, j) => <Line key={j} line={line} />)}
        </div>
      ))}
    </>
  )
}

interface DiffViewProps {
  diff: DiffData
  /** 是否在每个文件上方显示路径。左边已经有文件树时是多余的。 */
  showPath?: boolean
}

export function DiffView({ diff, showPath = true }: DiffViewProps) {
  if (diff.files.length === 0) return <div className="diff-note">No changes</div>
  return (
    <div className="diff-view">
      {diff.files.map((file) => (
        <div key={file.path} className="diff-file">
          {showPath && (
            <div className="diff-file-header">
              <span>
                {file.oldPath && <span className="diff-file-old">{file.oldPath} → </span>}
                {file.path}
              </span>
            </div>
          )}
          <FileBody file={file} />
        </div>
      ))}
    </div>
  )
}

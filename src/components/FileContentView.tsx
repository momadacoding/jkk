import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { toErrorMessage } from '../engine/errors'

/** 一次最多铺多少行。行数上万时 React 建 DOM 的开销肉眼可见。 */
const MAX_LINES = 5000

interface BlobContent {
  text: string | null
  binary: boolean
  /** blob 的完整字节数，即使内容被截断也是原始大小。 */
  size: number
  truncated: boolean
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`
}

interface FileContentViewProps {
  repoPath: string | null
  /** 读哪个版本的文件，通常是选中提交的 hash。 */
  rev: string | null
  /** 相对仓库根的文件路径；null 表示还没选文件。 */
  path: string | null
}

/** 显示某个提交下某个文件的完整内容（只读）。 */
export function FileContentView({ repoPath, rev, path }: FileContentViewProps) {
  const [blob, setBlob] = useState<BlobContent | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)

  useEffect(() => {
    if (!repoPath || !rev || !path) {
      setBlob(null)
      setError(null)
      return
    }
    let cancelled = false
    setLoading(true)
    setError(null)
    invoke<BlobContent>('read_blob', { path: repoPath, rev, file: path })
      .then((data) => { if (!cancelled) { setBlob(data); setLoading(false) } })
      .catch((err) => {
        if (cancelled) return
        setBlob(null)
        setError(toErrorMessage(err))
        setLoading(false)
      })
    return () => { cancelled = true }
  }, [repoPath, rev, path])

  if (!path) return <div className="diff-note">选择一个文件</div>
  if (error) return <div className="diff-note">{error}</div>
  if (loading || !blob) return <div className="diff-note">读取中…</div>
  if (blob.binary || blob.text === null) {
    return <div className="diff-note">二进制文件（{formatSize(blob.size)}），无法按行显示</div>
  }

  // 末尾换行会分出一个空元素，那不是文件的第 N+1 行
  const all = blob.text.split('\n')
  if (all.length > 0 && all[all.length - 1] === '') all.pop()
  const lines = all.slice(0, MAX_LINES)
  const cut = blob.truncated || all.length > MAX_LINES

  return (
    <div className="file-content">
      <div className="fc-head">
        <span className="fc-head-path">{path}</span>
        <span className="fc-head-meta">{formatSize(blob.size)} · {all.length} 行{cut ? '（已截断）' : ''}</span>
      </div>
      <div className="fc-body">
        {lines.map((line, i) => (
          <div key={i} className="fc-line">
            <span className="fc-ln">{i + 1}</span>
            <span className="fc-code">{line}</span>
          </div>
        ))}
        {cut && <div className="diff-note">文件太大，只显示了前 {lines.length} 行</div>}
        {lines.length === 0 && <div className="diff-note">空文件</div>}
      </div>
    </div>
  )
}

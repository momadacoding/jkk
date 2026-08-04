import { invoke } from '@tauri-apps/api/core'
import { useState } from 'react'
import type { DiffData, GraphData, RepoInfo, StatusData } from './engine/types'

const REPO_PATH = 'C:\\Users\\liuhao\\workspace\\dev\\Dev\\mygitclient'

function App() {
  const [info, setInfo] = useState<RepoInfo | null>(null)
  const [graph, setGraph] = useState<GraphData | null>(null)
  const [status, setStatus] = useState<StatusData | null>(null)
  const [diff, setDiff] = useState<DiffData | null>(null)

  const handleOpen = async () => {
    const result = await invoke<RepoInfo>('open_repo', { path: REPO_PATH })
    setInfo(result)
  }

  const handleLoadGraph = async () => {
    const result = await invoke<GraphData>('load_graph', { path: REPO_PATH })
    setGraph(result)
  }

  const handleReadStatus = async () => {
    const result = await invoke<StatusData>('read_status', { path: REPO_PATH })
    setStatus(result)
  }

  const handleReadDiff = async () => {
    const result = await invoke<DiffData>('read_diff', { path: REPO_PATH, scope: 'workdir' })
    setDiff(result)
  }

  return (
    <div>
      <button onClick={handleOpen}>Open repo</button>
      <button onClick={handleLoadGraph}>Load graph</button>
      <button onClick={handleReadStatus}>Read status</button>
      <button onClick={handleReadDiff}>Read diff</button>
      {info && <pre>{JSON.stringify(info, null, 2)}</pre>}
      {graph && <div>{graph.commits.length} commits, {graph.refs.length} refs</div>}
      {status && <div>staged: {status.staged.length}, unstaged: {status.unstaged.length}</div>}
      {diff && <div>{diff.files.length} diff files</div>}
    </div>
  )
}

export default App

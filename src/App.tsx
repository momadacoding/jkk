import { invoke } from '@tauri-apps/api/core'
import { useState } from 'react'
import type { GraphData, RepoInfo } from './engine/types'

function App() {
  const [info, setInfo] = useState<RepoInfo | null>(null)
  const [graph, setGraph] = useState<GraphData | null>(null)

  const handleOpen = async () => {
    const result = await invoke<RepoInfo>('open_repo', { path: 'C:\\Users\\liuhao\\workspace\\dev\\Dev\\mygitclient' })
    setInfo(result)
  }

  const handleLoadGraph = async () => {
    const result = await invoke<GraphData>('load_graph', { path: 'C:\\Users\\liuhao\\workspace\\dev\\Dev\\mygitclient' })
    setGraph(result)
  }

  return (
    <div>
      <button onClick={handleOpen}>Open repo</button>
      <button onClick={handleLoadGraph}>Load graph</button>
      {info && <pre>{JSON.stringify(info, null, 2)}</pre>}
      {graph && <div>{graph.commits.length} commits, {graph.refs.length} refs</div>}
    </div>
  )
}

export default App

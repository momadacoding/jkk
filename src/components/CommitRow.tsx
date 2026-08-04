import type { LayoutNode, RefPointer } from '../engine/graphTypes'

interface CommitRowProps {
  node: LayoutNode
  refs: RefPointer[]
  selected: boolean
  layoutWidth: number
}

export function CommitRow({ node, refs, selected, layoutWidth }: CommitRowProps) {
  const date = new Date(node.commit.time * 1000).toLocaleString()
  return (
    <div
      className={selected ? 'commit-row selected' : 'commit-row'}
      style={{ height: 28, display: 'flex', alignItems: 'center', paddingLeft: layoutWidth + 8 }}
      data-id={node.commit.id}
    >
      <span className="commit-message" style={{ flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
        {node.commit.message}
      </span>
      <span className="commit-meta" style={{ color: '#888', marginLeft: 8, whiteSpace: 'nowrap' }}>
        {node.commit.author} • {date}
      </span>
      <span style={{ marginLeft: 8, display: 'flex', gap: 4 }}>
        {refs.map((ref) => (
          <span key={ref.name} className={`ref-pill ${ref.kind}`}>
            {ref.name.replace('refs/heads/', '').replace('refs/remotes/', '')}
          </span>
        ))}
      </span>
    </div>
  )
}

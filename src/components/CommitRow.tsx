import type { LayoutNode, RefPointer } from '../engine/graphTypes'
import { laneTint } from '../engine/paths'
import { WORKDIR_ID } from '../engine/workdir'

interface CommitRowProps {
  node: LayoutNode
  refs: RefPointer[]
  selected: boolean
  layoutWidth: number
  /** 该提交所在车道的颜色，用作本地分支标签的左侧色条。 */
  laneColor: string
  /** 工作区伪行显示的未提交改动数。 */
  changeCount: number
  /** 该提交是否是 HEAD，是则整行加粗。 */
  isHead: boolean
}

/** 去掉 refs/heads/ 与 refs/remotes/ 前缀，标签上只留下人看的部分。 */
function shortName(name: string): string {
  return name.replace('refs/heads/', '').replace('refs/remotes/', '').replace('refs/tags/', '')
}

export function CommitRow({
  node, refs, selected, layoutWidth, laneColor, changeCount, isHead,
}: CommitRowProps) {
  if (node.commit.id === WORKDIR_ID) {
    return (
      <div
        className={selected ? 'commit-row workdir selected' : 'commit-row workdir'}
        style={{ height: 28, paddingLeft: layoutWidth + 8 }}
        data-id={node.commit.id}
      >
        <span className="workdir-label">
          {changeCount > 0 ? `${changeCount} changes` : 'No changes'}
        </span>
        <span className="commit-meta">Commit Changes</span>
      </div>
    )
  }

  const date = new Date(node.commit.time * 1000).toLocaleString()
  // HEAD 那条伪 ref 不做成标签——整行加粗已经表达了「当前在这里」，
  // 只有游离 HEAD 时才需要一个显式标记
  const visible = refs.filter((r) => r.kind !== 'head')
  const detached = refs.some((r) => r.kind === 'head') && !refs.some((r) => r.kind === 'branch')

  return (
    <div
      className={`commit-row${selected ? ' selected' : ''}${isHead ? ' head' : ''}`}
      style={{ height: 28, paddingLeft: layoutWidth + 8 }}
      data-id={node.commit.id}
    >
      {/* flex: 0 1 auto 让消息只占它需要的宽度，标签才能紧跟其后而不是被顶到窗口右缘 */}
      <span className="commit-message">{node.commit.message}</span>
      <span className="ref-pills">
        {detached && <span className="ref-pill head">HEAD</span>}
        {visible.map((ref) => (
          <span
            key={ref.name}
            className={`ref-pill ${ref.kind}`}
            // 本地分支的色条与底色都取车道色，标签才能和图上那条线对应起来
            style={ref.kind === 'branch'
              ? { borderLeftColor: laneColor, background: laneTint(laneColor, 0.22) }
              : undefined}
            title={ref.name}
          >
            {shortName(ref.name)}
          </span>
        ))}
      </span>
      <span className="commit-meta">
        {node.commit.author} • {date}
      </span>
    </div>
  )
}

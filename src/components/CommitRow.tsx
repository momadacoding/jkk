import type { LayoutNode, RefPointer } from '../engine/graphTypes'
import type { PullRequest } from '../engine/types'
import { pullRequestForRef } from '../engine/prs'
import { laneTint } from '../engine/paths'
import { WORKDIR_ID } from '../engine/workdir'
import { shortHash } from '../engine/commitActions'

interface CommitRowProps {
  node: LayoutNode
  refs: RefPointer[]
  /**
   * open PR 按 head 分支名建好的索引。
   *
   * 传索引而不是列表：这个组件每行都渲染一次，虚拟滚动下几十行同时在场，
   * 每行再遍历一遍 PR 列表是白费力气。索引由 `GraphView` 建一次。
   */
  prByBranch: Map<string, PullRequest>
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
  node, refs, prByBranch, selected, layoutWidth, laneColor, changeCount, isHead,
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
        {visible.map((ref) => {
          // 只有本地分支拿得到编号，判定在 engine 里，见 pullRequestForRef
          const pr = pullRequestForRef(prByBranch, ref)
          return (
            <span
              key={ref.name}
              className={`ref-pill ${ref.kind}`}
              // 本地分支的色条与底色都取车道色，标签才能和图上那条线对应起来
              style={ref.kind === 'branch'
                ? { borderLeftColor: laneColor, background: laneTint(laneColor, 0.22) }
                : undefined}
              title={pr
                ? `${ref.name}\n${pr.isDraft ? 'Draft PR' : 'PR'} #${pr.number}: ${pr.title}`
                : ref.name}
            >
              {shortName(ref.name)}
              {/* 编号内嵌在同一个 pill 里：另起一个 pill 要多一份边距和间隙，
                  而这一行还挤着提交信息、作者、hash 和日期 */}
              {pr && (
                <span className={pr.isDraft ? 'ref-pill-pr draft' : 'ref-pill-pr'}>
                  #{pr.number}
                </span>
              )}
            </span>
          )
        })}
      </span>
      {/* 三列定宽，各自左对齐——右对齐时 hash 和日期的起始位置每行都在跳，
          竖着扫一列的时候很费眼。定宽后三列各自成一条直线。 */}
      <span className="commit-meta">
        <span className="commit-author" title={node.commit.author}>{node.commit.author}</span>
        <span className="commit-hash" title={node.commit.id}>{shortHash(node.commit.id)}</span>
        <span className="commit-date">{date}</span>
      </span>
    </div>
  )
}

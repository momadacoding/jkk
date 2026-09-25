import type { LayoutNode, RefPointer } from '../engine/graphTypes'
import type { PullRequest } from '../engine/types'
import { prBadge, pullRequestForRef } from '../engine/prs'
import { laneTint } from '../engine/paths'
import { WORKDIR_ID } from '../engine/workdir'
import { shortHash } from '../engine/commitActions'
import { buildRefPills } from '../engine/refPills'

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
  // 本地分支与同一提交上的同名远程分支合成一个标签，见 buildRefPills
  const pills = buildRefPills(refs.filter((r) => r.kind !== 'head'))
  const detached = refs.some((r) => r.kind === 'head') && !refs.some((r) => r.kind === 'branch')

  // svn 节点换一套配色：它们不是本地提交，点开也没有 diff 可看（Phase 4 才做），
  // 与 git 提交长得一模一样会让人以为能对它做 checkout / revert 之类的操作。
  const isSvn = node.commit.vcs === 'svn'

  return (
    <div
      className={`commit-row${selected ? ' selected' : ''}${isHead ? ' head' : ''}${isSvn ? ' svn' : ''}`}
      style={{ height: 28, paddingLeft: layoutWidth + 8 }}
      data-id={node.commit.id}
    >
      {/* flex: 0 1 auto 让消息只占它需要的宽度，标签才能紧跟其后而不是被顶到窗口右缘 */}
      <span className="commit-message">
        {/* 「仅本地」角标（spec §6.7）：这个提交带 Local-Only trailer，
            标记本身不改变任何现有流程，摘除与回落属 Phase 3。 */}
        {node.commit.localOnly && (
          <span
            className="local-only-badge"
            title="仅本地，不交付。注意：交付流程要到 Phase 3 才会读这条标记。"
          >
            仅本地
          </span>
        )}
        {node.commit.message}
      </span>
      <span className="ref-pills">
        {detached && <span className="ref-pill head">HEAD</span>}
        {pills.map((pill) => {
          const { ref } = pill
          // 只有本地分支拿得到编号，判定在 engine 里，见 pullRequestForRef
          const pr = pullRequestForRef(prByBranch, ref)
          // ref.hash 就是这一行的提交，也就是该分支的 tip——正好是判断
          // 「PR 合并之后分支有没有再往前走」需要的那个值
          const badge = pr ? prBadge(pr, ref.hash) : undefined
          const tooltip = [
            ref.name,
            pill.remotes.length > 0 ? `远程同一提交：${pill.remotes.join('、')}` : undefined,
            badge?.tooltip,
          ].filter(Boolean).join('\n')
          return (
            <span
              key={pill.key}
              className={`ref-pill ${pill.kind}`}
              // 本地分支的色条与底色都取车道色，标签才能和图上那条线对应起来
              style={pill.kind === 'branch'
                ? { borderLeftColor: laneColor, background: laneTint(laneColor, 0.22) }
                : undefined}
              title={tooltip}
            >
              {/* 名字单独一个 span 负责省略号：空间不够时先截名字，
                  后面的远程标记和 PR 编号始终完整可见 */}
              <span className="ref-pill-name">{pill.label}</span>
              {/* 远程标记只是个图标，完整远程名在悬停提示里。
                  \uFE0E 要求按文本字形绘制，避免某些平台画成彩色 emoji */}
              {pill.remotes.length > 0 && (
                <span className="ref-pill-remote" aria-label="远程同一提交">{'\u2601\uFE0E'}</span>
              )}
              {/* 编号内嵌在同一个 pill 里：另起一个 pill 要多一份边距和间隙，
                  而这一行还挤着提交信息、作者、hash 和日期 */}
              {badge && (
                <span className={`ref-pill-pr ${badge.variant}`}>{badge.label}</span>
              )}
            </span>
          )
        })}
      </span>
      {/* 三列定宽，各自左对齐——右对齐时 hash 和日期的起始位置每行都在跳，
          竖着扫一列的时候很费眼。定宽后三列各自成一条直线。 */}
      <span className="commit-meta">
        <span className="commit-author" title={node.commit.author}>{node.commit.author}</span>
        {/* svn 节点没有 hash，显示 revision 号（spec §11）。
            拿 `svn:12346` 去截短会得到一串毫无意义的 `svn:123`。 */}
        <span className="commit-hash" title={node.commit.id}>
          {isSvn ? `r${node.commit.svnRev}` : shortHash(node.commit.id)}
        </span>
        <span className="commit-date">{date}</span>
      </span>
    </div>
  )
}

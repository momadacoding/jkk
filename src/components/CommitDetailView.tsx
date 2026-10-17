import { useEffect, useMemo, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { Panel, Group, Separator } from 'react-resizable-panels'
import type { CommitNode, DiffData } from '../engine/types'
import { buildFileTree } from '../engine/fileTree'
import { deliveredNote, svnDiffGap } from '../engine/svnGraph'
import { DiffView } from './DiffView'
import { FileTree } from './FileTree'
import { FileContentView } from './FileContentView'
import { toErrorMessage } from '../engine/errors'

type Tab = 'commit' | 'changes' | 'tree'

interface CommitDetailViewProps {
  commit: CommitNode | null
  diff: DiffData
  /** 仓库路径，File Tree 标签要用它去读这个提交的整棵树。 */
  repoPath: string | null
  /**
   * 选中另一个提交：svn 节点详情里点开收着的本地提交、或「返回 rN」时用。
   * 必传——那几行是看收起的提交的唯一入口，不能渲染成点了没反应的按钮（2026-10-16 review）。
   */
  onSelectCommit: (id: string) => void
  /** `commit` 是收进这个 svn 节点、不在图上的本地提交时传入，顶上给「返回 rN」。 */
  backTo?: CommitNode | null
  /**
   * 这个提交在图上所在车道的分支（`formatLaneBranch` 的结果）。车道一多，光看图认不出
   * 选中的提交在哪条线上（2026-10-17 用户要求）。图外提交（stash、收进 svn 节点的）没有，传 null。
   */
  branch?: string | null
}

/** 信息条上的分支一栏；三个标签页和 svn 节点共用，换标签页时不该消失。 */
function BranchTag({ branch }: { branch: string | null | undefined }) {
  if (!branch) return null
  return (
    <span className="commit-head-branch" title={`所在分支：${branch}`}>
      ⑂ {branch}
    </span>
  )
}

interface TreeListing {
  paths: string[]
  truncated: boolean
}

/** 提交信息条：头像、作者、短 hash、时间、标题。Fork 里三个标签共用这一条。 */
function CommitHeader({ commit, branch }: { commit: CommitNode; branch?: string | null }) {
  const subject = commit.message.split('\n')[0]
  return (
    <div className="commit-head">
      <span className="commit-avatar">{commit.author.slice(0, 1).toUpperCase()}</span>
      <span className="commit-head-author">{commit.author}</span>
      <span className="commit-head-sha">{commit.id.slice(0, 7)}</span>
      <span className="commit-head-date">{new Date(commit.time * 1000).toLocaleString()}</span>
      <BranchTag branch={branch} />
      <span className="commit-head-subject" title={commit.message}>{subject}</span>
    </div>
  )
}

export function CommitDetailView({ commit, diff, repoPath, onSelectCommit, backTo, branch }: CommitDetailViewProps) {
  const [tab, setTab] = useState<Tab>('commit')
  const [picked, setPicked] = useState<string | null>(null)
  const [listing, setListing] = useState<TreeListing | null>(null)
  const [listError, setListError] = useState<string | null>(null)

  const fileStats = useMemo(
    () =>
      diff.files.map((f) => ({
        path: f.path,
        status: f.status,
        additions: f.hunks.reduce((n, h) => n + h.lines.filter((l) => l.origin === '+').length, 0),
        deletions: f.hunks.reduce((n, h) => n + h.lines.filter((l) => l.origin === '-').length, 0),
      })),
    [diff],
  )

  const changesTree = useMemo(
    () => buildFileTree(diff.files.map((f) => ({ path: f.path, status: f.status }))),
    [diff],
  )

  const fullTree = useMemo(
    () => buildFileTree((listing?.paths ?? []).map((path) => ({ path, status: null }))),
    [listing],
  )

  // 选中项从当前 diff 推出来而不是存起来：换一个提交时上一个选中的文件多半
  // 不在新 diff 里，存着的话右边会空白，还得再写一个 effect 去清它。
  const activePath = picked && diff.files.some((f) => f.path === picked)
    ? picked
    : diff.files[0]?.path ?? null

  const activeDiff: DiffData = useMemo(
    () => ({ files: diff.files.filter((f) => f.path === activePath) }),
    [diff, activePath],
  )

  const commitId = commit?.id ?? null
  useEffect(() => {
    if (tab !== 'tree' || !repoPath || !commitId) return
    let cancelled = false
    setListError(null)
    invoke<TreeListing>('list_tree', { path: repoPath, rev: commitId })
      .then((data) => { if (!cancelled) setListing(data) })
      .catch((err) => { if (!cancelled) { setListing(null); setListError(toErrorMessage(err)) } })
    return () => { cancelled = true }
  }, [tab, repoPath, commitId])

  if (!commit) {
    return <div className="commit-detail commit-detail-empty">Select a commit</div>
  }

  // svn 节点。
  //
  // spec §11 的前端改动表写着「svn 节点的 diff 走 `svn diff -c N`」，但 §16 的
  // 分期表把它列在 **Phase 4**。不过前后两版都有本地基线时（交付或更新录下的），
  // 两版基线之差本地就能算（`commitDiffScope` 的 `range:`），这里直接显示；
  // 否则如实说明，而不是给一个空白的文件列表——空白会让人以为这个 revision 什么都没改。
  if (commit.vcs === 'svn') {
    const prevRev = commit.parents[0]?.replace(/^svn:/, '')
    const delivered = deliveredNote(commit)
    return (
      <div className="commit-detail">
        <div className="commit-head">
          <span className="commit-avatar">{commit.author.slice(0, 1).toUpperCase()}</span>
          <span className="commit-head-author">{commit.author}</span>
          <span className="commit-head-sha">r{commit.svnRev}</span>
          <span className="commit-head-date">
            {new Date(commit.time * 1000).toLocaleString()}
          </span>
          <BranchTag branch={branch} />
        </div>
        <pre className="commit-detail-message">{commit.message}</pre>
        {delivered && (
          <div className="commit-delivered">
            <div className="commit-delivered-title">
              {delivered.length > 1
                ? `这次交付包含 ${delivered.length} 个本地提交：`
                : '这次交付来自本地提交：'}
            </div>
            {/* 这些提交不在图上，这里是唯一的入口：点开看它自己的改动（2026-10-16 用户要求） */}
            {delivered.map((d) => (
              <button
                key={d.id}
                type="button"
                className="commit-delivered-item"
                title="查看这个提交自己的改动"
                onClick={() => onSelectCommit(d.id)}
              >
                <span className="commit-delivered-sha">{d.id.slice(0, 7)}</span> {d.message}
              </button>
            ))}
          </div>
        )}
        {commit.diffBase ? (
          <>
            <p className="commit-detail-note">相对 r{prevRev} 的变化（只含 git 管理的文件）。</p>
            <div className="commit-split">
              <Group orientation="horizontal">
                <Panel defaultSize="30%" minSize="15%" maxSize="60%">
                  <FileTree nodes={changesTree} selectedPath={activePath} onSelect={setPicked} />
                </Panel>
                <Separator className="separator-v" />
                <Panel minSize="30%">
                  <DiffView diff={activeDiff} showPath={false} />
                </Panel>
              </Group>
            </div>
          </>
        ) : (
          // 不猜是谁的提交：交付之后同事紧跟着提交时，自己交付的 revision 也没在本机记过（2026-10-16 review）
          <p className="commit-detail-note">
            {svnDiffGap(commit) === 'prev'
              ? `本机没有记下前一个 revision（r${prevRev}）的内容，算不出这一版改了什么。`
              : `本机没有记下 r${commit.svnRev} 的内容，算不出它改了什么。`}
            看文件差异要联网（<code>svn diff -c {commit.svnRev}</code>），属于 Phase 4，本期还没做。
            {delivered && '上面的本地提交可以点开，看各自的改动。'}
          </p>
        )}
      </div>
    )
  }

  return (
    <div className="commit-detail">
      <div className="commit-detail-tabs">
        {backTo && (
          <button
            type="button"
            className="commit-detail-back"
            onClick={() => onSelectCommit(backTo.id)}
          >
            ← 返回 r{backTo.svnRev}
          </button>
        )}
        <button className={tab === 'commit' ? 'active' : ''} onClick={() => setTab('commit')}>Commit</button>
        <button className={tab === 'changes' ? 'active' : ''} onClick={() => setTab('changes')}>
          Changes{diff.files.length > 0 ? ` (${diff.files.length})` : ''}
        </button>
        <button className={tab === 'tree' ? 'active' : ''} onClick={() => setTab('tree')}>File Tree</button>
      </div>

      <CommitHeader commit={commit} branch={branch} />

      {tab === 'commit' && (
        <div className="commit-info">
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
                <span className={`commit-file-status ${f.status}`}>{f.status.slice(0, 1).toUpperCase()}</span>
                <span className="commit-file-path">{f.path}</span>
                <span className="file-stats">+{f.additions}/-{f.deletions}</span>
              </div>
            ))}
          </div>
        </div>
      )}

      {tab === 'changes' && (
        <div className="commit-split">
          <Group orientation="horizontal">
            <Panel defaultSize="30%" minSize="15%" maxSize="60%">
              <FileTree nodes={changesTree} selectedPath={activePath} onSelect={setPicked} />
            </Panel>
            <Separator className="separator-v" />
            <Panel minSize="30%">
              <DiffView diff={activeDiff} showPath={false} />
            </Panel>
          </Group>
        </div>
      )}

      {tab === 'tree' && (
        <div className="commit-split">
          <Group orientation="horizontal">
            <Panel defaultSize="30%" minSize="15%" maxSize="60%">
              {listError ? (
                <div className="diff-note">{listError}</div>
              ) : (
                <>
                  {listing?.truncated && (
                    <div className="ft-truncated">文件太多，只显示前 {listing.paths.length} 条</div>
                  )}
                  {/* 整仓文件树可能上万条，默认收起，展开哪层由用户决定 */}
                  <FileTree
                    nodes={fullTree}
                    selectedPath={picked}
                    onSelect={setPicked}
                    defaultOpen={false}
                  />
                </>
              )}
            </Panel>
            <Separator className="separator-v" />
            <Panel minSize="30%">
              {/* 这里看的是文件在这个提交下的样子，不是它改了什么——那是 Changes 的事 */}
              <FileContentView repoPath={repoPath} rev={commit.id} path={picked} />
            </Panel>
          </Group>
        </div>
      )}
    </div>
  )
}

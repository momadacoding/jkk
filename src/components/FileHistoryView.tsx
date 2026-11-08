import { useEffect, useMemo, useRef, useState } from 'react'
import { Panel, Group, Separator } from 'react-resizable-panels'
import { PANEL_GROUP_IDS } from '@/engine/layoutMemory'
import { usePanelLayout } from '@/store/layoutStore'
import { readBlame, readCommitFileDiff, readFileHistory } from '@/store/repoStore'
import { toErrorMessage } from '@/engine/errors'
import {
  isUncommitted,
  blameBlocks,
  blameHeat,
  blameRevForEntry,
  describeBlameRev,
  initialEntryId,
  previousBlameRev,
  shortId,
  type BlameData,
  type BlameRev,
  type FileHistory,
  type FileHistoryEntry,
  type FileHistoryTab,
  type FileHistoryTarget,
} from '@/engine/fileHistory'
import type { DiffData } from '@/engine/types'
import { DiffView } from './DiffView'
import { STATUS_MARK } from './FileTree'
import { ContextMenu, useContextMenu, type MenuItem } from './ContextMenu'

interface FileHistoryViewProps {
  repoPath: string
  target: FileHistoryTarget
  /** HEAD 变了（提交、切分支之后）就重读历史与 Blame。 */
  headHash: string
  /**
   * 工作区的版本号：每次工作区状态重读之后换一个新值（RepoTab 传 `state.status` 这个对象）。
   * 只有 Blame 工作区时用它——编辑器里改了文件、HEAD 没动，Blame 也要跟上。
   */
  workdirVersion?: unknown
  /** 回到打开之前的视图。 */
  onClose: () => void
  /** 在提交图中定位这个提交；不在图上时由调用方说明原因。 */
  onRevealCommit: (id: string) => void
}

const sameRev = (a: BlameRev, b: BlameRev) => a.rev === b.rev && a.path === b.path

function formatDate(time: number): string {
  return new Date(time * 1000).toLocaleDateString()
}

function formatTime(time: number): string {
  return new Date(time * 1000).toLocaleString()
}

/**
 * 一个文件的历史与 Blame，占满主区域（决策 D3）。
 *
 * 换一个文件时由调用方换 `key` 重新挂载，所以这里的状态只管「同一个文件」之内。
 */
export function FileHistoryView({ repoPath, target, headHash, workdirVersion = null, onClose, onRevealCommit }: FileHistoryViewProps) {
  const [tab, setTab] = useState<FileHistoryTab>(target.tab)
  const [history, setHistory] = useState<FileHistory | null>(null)
  const [historyError, setHistoryError] = useState<string | null>(null)
  const [picked, setPicked] = useState<string | null>(null)
  const [blameRev, setBlameRev] = useState<BlameRev>(target.blame)
  const layout = usePanelLayout(PANEL_GROUP_IDS.fileHistory)

  const revsKey = target.revs.join('\n')
  useEffect(() => {
    let cancelled = false
    setHistoryError(null)
    readFileHistory(repoPath, target.path, revsKey.split('\n'))
      .then((data) => { if (!cancelled) setHistory(data) })
      .catch((err) => {
        if (cancelled) return
        setHistory(null)
        setHistoryError(toErrorMessage(err))
      })
    return () => { cancelled = true }
  }, [repoPath, target.path, revsKey, headHash])

  const entries = useMemo(() => history?.entries ?? [], [history])
  // 选中项从列表推出来：重读之后原来选中的那条可能不在了，存着的话右边会空白
  const selectedId = picked && entries.some((e) => e.id === picked)
    ? picked
    : initialEntryId(entries, target.focus)
  const selected = entries.find((e) => e.id === selectedId) ?? null

  const showEntry = (id: string) => {
    setPicked(id)
    setTab('history')
  }
  const blameEntry = (entry: FileHistoryEntry) => {
    const rev = blameRevForEntry(entry)
    if (!rev) return
    setBlameRev(rev)
    setTab('blame')
  }

  return (
    <div className="file-history">
      <div className="commit-detail-tabs">
        <button type="button" className="commit-detail-back" onClick={onClose}>← 返回</button>
        <button type="button" className={tab === 'history' ? 'active' : ''} onClick={() => setTab('history')}>
          History{history ? ` (${entries.length}${history.truncated ? '+' : ''})` : ''}
        </button>
        <button type="button" className={tab === 'blame' ? 'active' : ''} onClick={() => setTab('blame')}>
          Blame
        </button>
        <span className="fh-title" title={target.path}>{target.path}</span>
      </div>
      {tab === 'history' && history && target.focus && !entries.some((e) => e.id === target.focus) && (
        // 不说明的话会默默选中最新一条，看着像打开错了（独立 review 第 2、7 条）
        <div className="fh-note">
          打开时的提交 {shortId(target.focus)} 不在下面的列表里：<code>git log --follow</code> 不列出只在合并时改动这个文件的合并提交，
          也追不到这个文件之后改了名的那部分历史。
        </div>
      )}
      {tab === 'history' ? (
        <div className="commit-split">
          <Group orientation="horizontal" {...layout}>
            <Panel id="entries" defaultSize="35%" minSize="15%" maxSize="70%">
              <HistoryList
                history={history}
                error={historyError}
                selectedId={selectedId}
                onSelect={setPicked}
                onReveal={onRevealCommit}
              />
            </Panel>
            <Separator className="separator-v" />
            <Panel id="diff" minSize="30%">
              <EntryDiff
                repoPath={repoPath}
                entry={selected}
                onBlame={blameEntry}
                onReveal={onRevealCommit}
              />
            </Panel>
          </Group>
        </div>
      ) : (
        <BlamePane
          repoPath={repoPath}
          rev={blameRev}
          initial={target.blame}
          headHash={headHash}
          workdirVersion={workdirVersion}
          entries={entries}
          onBlameRev={setBlameRev}
          onShowEntry={showEntry}
          onReveal={onRevealCommit}
        />
      )}
    </div>
  )
}

function HistoryList({ history, error, selectedId, onSelect, onReveal }: {
  history: FileHistory | null
  error: string | null
  selectedId: string | null
  onSelect: (id: string) => void
  onReveal: (id: string) => void
}) {
  if (error) return <div className="diff-note">读取文件历史失败：{error}</div>
  if (!history) return <div className="diff-note">正在读取文件历史…</div>
  if (history.entries.length === 0) {
    return <div className="diff-note">没有找到改动过这个文件的提交（可能还没提交过）。</div>
  }
  return (
    <div className="fh-list" role="listbox" aria-label="文件历史">
      {history.entries.map((e) => {
        const mark = STATUS_MARK[e.status]
        return (
          <div
            key={e.id}
            role="option"
            aria-selected={e.id === selectedId}
            className={e.id === selectedId ? 'fh-row selected' : 'fh-row'}
            onClick={() => onSelect(e.id)}
            onDoubleClick={() => onReveal(e.id)}
            title={`${e.subject}\n${e.author} · ${formatTime(e.time)}\n${e.path}\n双击在提交图中定位`}
          >
            <div className="fh-row-top">
              <span className={`ft-badge ${mark.kind}`}>{mark.label}</span>
              <span className="fh-subject">{e.subject}</span>
            </div>
            <div className="fh-row-meta">
              <span className="fh-sha">{shortId(e.id)}</span>
              <span>{e.author}</span>
              <span>{formatDate(e.time)}</span>
              {e.oldPath && <span className="fh-rename" title={`${e.oldPath} → ${e.path}`}>← {e.oldPath}</span>}
            </div>
          </div>
        )
      })}
      {history.truncated && (
        <div className="diff-note">只列出最近 {history.entries.length} 条。</div>
      )}
    </div>
  )
}

function EntryDiff({ repoPath, entry, onBlame, onReveal }: {
  repoPath: string
  entry: FileHistoryEntry | null
  onBlame: (entry: FileHistoryEntry) => void
  onReveal: (id: string) => void
}) {
  const [diff, setDiff] = useState<DiffData | null>(null)
  const [error, setError] = useState<string | null>(null)
  const id = entry?.id ?? null
  const path = entry?.path ?? null
  const oldPath = entry?.oldPath ?? null

  useEffect(() => {
    if (!id || !path) return
    let cancelled = false
    setDiff(null)
    setError(null)
    readCommitFileDiff(repoPath, id, path, oldPath)
      .then((data) => { if (!cancelled) setDiff(data) })
      .catch((err) => { if (!cancelled) setError(toErrorMessage(err)) })
    return () => { cancelled = true }
  }, [repoPath, id, path, oldPath])

  if (!entry) return <div className="diff-note">选择一个提交</div>
  return (
    <div className="file-content">
      <div className="fc-head">
        <span className="fc-head-path" title={entry.subject}>
          <span className="fh-sha">{shortId(entry.id)}</span> {entry.subject}
        </span>
        <button
          type="button"
          className="fh-action"
          disabled={!blameRevForEntry(entry)}
          onClick={() => onBlame(entry)}
          title={entry.status === 'deleted' ? 'Blame 删除之前的那一版' : 'Blame 这个提交里的这一版'}
        >
          Blame 这一版
        </button>
        <button type="button" className="fh-action" onClick={() => onReveal(entry.id)}>
          在提交图中定位
        </button>
      </div>
      <div className="fh-diff">
        {error
          ? <div className="diff-note">读取改动失败：{error}</div>
          : diff
            ? <DiffView diff={diff} />
            : <div className="diff-note diff-note-pending">正在读取改动…</div>}
      </div>
    </div>
  )
}

function BlamePane({ repoPath, rev, initial, headHash, workdirVersion, entries, onBlameRev, onShowEntry, onReveal }: {
  repoPath: string
  rev: BlameRev
  /** 打开时看的那一版：翻到别的版本之后给一个回去的按钮。 */
  initial: BlameRev
  headHash: string
  workdirVersion: unknown
  entries: FileHistoryEntry[]
  onBlameRev: (rev: BlameRev) => void
  onShowEntry: (id: string) => void
  onReveal: (id: string) => void
}) {
  const [blame, setBlame] = useState<BlameData | null>(null)
  const [error, setError] = useState<string | null>(null)
  const { menu, open, close } = useContextMenu()
  // 上一次读的是哪一版：换版本时先清空（不能把别的版本当成这一版显示），同一版因工作区变化重读时
  // 留着旧的直到新的回来，编辑器每存一次都闪一下「正在 Blame…」太吵
  const shownKey = useRef<string | null>(null)
  // 看工作区时，工作区一变就重读（/code-review 第 3 条）；看固定版本时与它无关
  const workdirKey = rev.rev === null ? workdirVersion : null

  useEffect(() => {
    let cancelled = false
    const key = `${rev.rev ?? ''}\n${rev.path}`
    if (shownKey.current !== key) {
      shownKey.current = key
      setBlame(null)
    }
    setError(null)
    readBlame(repoPath, rev.path, rev.rev)
      .then((data) => { if (!cancelled) setBlame(data) })
      .catch((err) => { if (!cancelled) { setBlame(null); setError(toErrorMessage(err)) } })
    return () => { cancelled = true }
    // 看工作区时 HEAD 变了（刚提交）未提交的行就变成了提交过的；看固定版本时重读一次也无妨
  }, [repoPath, rev.rev, rev.path, headHash, workdirKey])

  const blocks = useMemo(() => blameBlocks(blame?.lines ?? []), [blame])
  const heat = useMemo(() => blameHeat(blame?.commits ?? {}), [blame])
  const inHistory = useMemo(() => new Set(entries.map((e) => e.id)), [entries])

  const pickCommit = (commit: string) => {
    if (isUncommitted(commit)) return
    if (inHistory.has(commit)) onShowEntry(commit)
    else onReveal(commit)
  }

  const contextMenu = (commit: string, e: React.MouseEvent) => {
    const uncommitted = isUncommitted(commit)
    const previous = previousBlameRev(blame?.commits[commit])
    const items: MenuItem[] = [
      {
        label: '查看这个提交里的改动',
        onSelect: () => onShowEntry(commit),
        disabled: !inHistory.has(commit),
      },
      { label: '在提交图中定位', onSelect: () => onReveal(commit), disabled: uncommitted },
      {
        label: 'Blame 这次改动之前的版本',
        onSelect: () => { if (previous) onBlameRev(previous) },
        disabled: !previous,
      },
      {
        label: '复制提交 hash',
        onSelect: () => { void navigator.clipboard.writeText(commit) },
        disabled: uncommitted,
      },
    ]
    open(e, items)
  }

  const header = (
    <div className="fc-head">
      <span className="fc-head-path" title={rev.path}>
        Blame：{describeBlameRev(rev, entries)}
        {rev.path !== initial.path && <span className="fh-rename"> （当时的路径 {rev.path}）</span>}
      </span>
      {blame && !blame.binary && (
        <span className="fc-head-meta">
          {blame.totalLines} 行{blame.truncated ? `，只显示前 ${blame.lines.length} 行` : ''}
        </span>
      )}
      {!sameRev(rev, initial) && (
        <button type="button" className="fh-action" onClick={() => onBlameRev(initial)}>
          回到 {describeBlameRev(initial, entries)}
        </button>
      )}
    </div>
  )

  let body: React.ReactNode
  if (error) body = <div className="diff-note">Blame 失败：{error}</div>
  else if (!blame) body = <div className="diff-note diff-note-pending">正在 Blame…</div>
  else if (blame.binary) body = <div className="diff-note">二进制文件，无法按行 Blame</div>
  else if (blame.lines.length === 0) body = <div className="diff-note">空文件</div>
  else {
    body = (
      <div className="fc-body fh-blame">
        <div className="fh-rows">
        {blocks.map((block) => {
          const info = blame.commits[block.commit]
          const uncommitted = isUncommitted(block.commit)
          const h = heat.get(block.commit) ?? 0
          const tip = uncommitted
            ? '还没提交的改动'
            : info
              ? `${shortId(block.commit)} ${info.summary}\n${info.author} · ${formatTime(info.time)}`
              : block.commit
          return (
            <div
              key={block.start}
              className="fh-block"
              onContextMenu={(e) => contextMenu(block.commit, e)}
            >
              {blame.lines.slice(block.start, block.start + block.length).map((line, i) => (
                <div key={line.finalLine} className="fc-line">
                  <span
                    className={i === 0 ? 'fh-gutter fh-gutter-start' : 'fh-gutter'}
                    style={{ borderLeftColor: `rgba(216, 166, 87, ${0.12 + 0.88 * h})` }}
                    title={tip}
                    onClick={i === 0 ? () => pickCommit(block.commit) : undefined}
                  >
                    {i === 0 && (uncommitted ? (
                      <span className="fh-uncommitted">未提交</span>
                    ) : (
                      <>
                        <span className="fh-sha">{shortId(block.commit)}</span>
                        <span className="fh-gutter-author">{info?.author}</span>
                        <span className="fh-gutter-date">{info ? formatDate(info.time) : ''}</span>
                      </>
                    ))}
                  </span>
                  <span className="fc-ln">{line.finalLine}</span>
                  <span className="fc-code">{line.content}</span>
                </div>
              ))}
            </div>
          )
        })}
        </div>
      </div>
    )
  }

  return (
    <div className="file-content">
      {header}
      {body}
      <ContextMenu menu={menu} onClose={close} />
    </div>
  )
}

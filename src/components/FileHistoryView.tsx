import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Panel, Group, Separator } from 'react-resizable-panels'
import { PANEL_GROUP_IDS } from '@/engine/layoutMemory'
import { usePanelLayout } from '@/store/layoutStore'
import {
  readBlame,
  readCommitFileDiff,
  readFileHistory,
  readSvnBlameOverlay,
  readSvnFileHistory,
  readSvnRevFileDiff,
} from '@/store/repoStore'
import { createLatestGate } from '@/engine/latestGate'
import {
  applyOverlay,
  groupComplete,
  groupSpecs,
  hiddenRevs,
  mergePage,
  overlayTargets,
  startCursor,
  type Cursor,
  type GroupSpec,
  type OverlayView,
  type SvnFetchError,
  type SvnGroups,
  type SvnRevEntry,
} from '@/engine/svnHistory'
import { getDiffPrefs, useDiffOptionsKey } from '@/store/diffPrefsStore'
import { backendDiffOptions } from '@/engine/diffPrefs'
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

/** SVN 的时间是 ISO8601 原样（后端不解析），解析不了就原样显示。 */
function svnDate(iso: string, long = false): string {
  const t = Date.parse(iso)
  if (Number.isNaN(t)) return iso
  return long ? new Date(t).toLocaleString() : new Date(t).toLocaleDateString()
}

const firstLine = (s: string) => s.split(/\r?\n/, 1)[0] ?? ''

/** 列表里一条 SVN 提交的选中键：基线提交 id + revision。 */
const svnKey = (group: string, rev: number) => `svn:${group}:${rev}`

/** 已加载的 SVN 补全。 */
interface SvnState {
  groups: SvnGroups
  cursor: Cursor | null
  error: SvnFetchError | null
  loading: boolean
}

const NO_SVN: SvnState = { groups: {}, cursor: null, error: null, loading: false }

/** 连不上服务器时用设计里的原句，其它错误给出原因。 */
function svnBanner(error: SvnFetchError): string {
  return error.code === 'svn_unreachable' ? '连不上 SVN，基线条目显示的是快照' : `SVN 明细没取到：${error.message}`
}

/** 一条 SVN 提交，连同它所在那组基线（读 diff 要用那组的 peg）。 */
interface SvnPicked {
  group: GroupSpec
  entry: SvnRevEntry
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
  const specs = useMemo(() => groupSpecs(entries), [entries])
  const hidden = useMemo(() => hiddenRevs(entries), [entries])
  const [svn, setSvn] = useState<SvnState>(NO_SVN)
  // 换文件、HEAD 变了之后，上一轮还在飞的 SVN 响应不能填进新列表（计划 Review Focus 第 3 条）
  const svnGate = useRef(createLatestGate()).current

  const loadSvn = useCallback((cursor: Cursor | null) => {
    const token = svnGate.begin()
    // 第一页也要带上起始游标：游标为空表示「全部查完」，在途时每一组都会显示「0 次 SVN 提交」
    // （ultrareview 第 1 条）
    setSvn((s) => (cursor
      ? { ...s, loading: true, error: null }
      : { ...NO_SVN, cursor: startCursor(specs), loading: true }))
    readSvnFileHistory(repoPath, specs, hidden, cursor)
      .then((page) => {
        if (!svnGate.isLatest(token)) return
        setSvn((s) => ({ groups: mergePage(s.groups, page), cursor: page.cursor, error: page.error, loading: false }))
      })
      .catch((err) => {
        if (!svnGate.isLatest(token)) return
        // 保留游标：「加载更多」就是重试。第一页就失败时没有游标，从第一组起——不留游标的话
        // 每一组都会被当成「查完了」，显示一句假的「0 次 SVN 提交」（review 第 I1 条）
        setSvn((s) => ({
          ...s,
          cursor: s.cursor ?? cursor ?? startCursor(specs),
          error: { code: 'svn_cli', message: toErrorMessage(err) },
          loading: false,
        }))
      })
  }, [repoPath, specs, hidden, svnGate])

  useEffect(() => {
    // 一条基线都没有（纯 git 仓库、从没进过基线的文件）：不去 SVN
    if (specs.length === 0) {
      svnGate.begin()
      setSvn(NO_SVN)
      return
    }
    loadSvn(null)
  }, [specs, loadSvn, svnGate])

  const svnPicked = useMemo((): SvnPicked | null => {
    if (!picked?.startsWith('svn:')) return null
    for (const group of specs) {
      const entry = svn.groups[group.id]?.find((e) => svnKey(group.id, e.rev) === picked)
      if (entry) return { group, entry }
    }
    return null
  }, [picked, specs, svn.groups])

  // 选中项从列表推出来：重读之后原来选中的那条可能不在了，存着的话右边会空白
  const selectedId = svnPicked
    ? picked
    : picked && entries.some((e) => e.id === picked)
      ? picked
      : initialEntryId(entries, target.focus)
  const selected = svnPicked ? null : entries.find((e) => e.id === selectedId) ?? null

  const showEntry = (id: string) => {
    setPicked(id)
    setTab('history')
  }
  /** Blame 里点了一行 SVN 出处：列表里有这条就选它，没有（还没加载到）就选它所在的基线。 */
  const showSvn = (rev: number, baseline: string) => {
    for (const group of specs) {
      if (svn.groups[group.id]?.some((e) => e.rev === rev)) {
        showEntry(svnKey(group.id, rev))
        return
      }
    }
    showEntry(baseline)
  }
  const svnMessages = useMemo(() => {
    const m = new Map<number, string>()
    for (const list of Object.values(svn.groups)) for (const e of list) m.set(e.rev, e.message)
    return m
  }, [svn.groups])
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
      {tab === 'history' && svn.error && <div className="fh-note">{svnBanner(svn.error)}</div>}
      {tab === 'history' ? (
        <div className="commit-split">
          <Group orientation="horizontal" {...layout}>
            <Panel id="entries" defaultSize="35%" minSize="15%" maxSize="70%">
              <HistoryList
                history={history}
                error={historyError}
                selectedId={selectedId}
                specs={specs}
                svn={svn}
                onSelect={setPicked}
                onReveal={onRevealCommit}
                onLoadMore={() => loadSvn(svn.cursor)}
              />
            </Panel>
            <Separator className="separator-v" />
            <Panel id="diff" minSize="30%">
              {svnPicked ? (
                <SvnEntryDiff repoPath={repoPath} picked={svnPicked} onReveal={onRevealCommit} />
              ) : (
                <EntryDiff
                  repoPath={repoPath}
                  entry={selected}
                  onBlame={blameEntry}
                  onReveal={onRevealCommit}
                />
              )}
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
          svnMessages={svnMessages}
          onBlameRev={setBlameRev}
          onShowEntry={showEntry}
          onShowSvn={showSvn}
          onReveal={onRevealCommit}
        />
      )}
    </div>
  )
}

/** 非基线提交行尾的交付状态。 */
function DeliveredTag({ entry }: { entry: FileHistoryEntry }) {
  const d = entry.svn?.delivered
  if (!d) return null
  return d.kind === 'undelivered'
    ? <span className="fh-svn-tag undelivered">未交付</span>
    : <span className="fh-svn-tag">已交付 r{d.rev}</span>
}

/** 基线行下面那组 SVN 提交的状态：几条、加载完没有、出错了没有。 */
function svnGroupStatus(list: SvnRevEntry[], complete: boolean, svn: SvnState): { text: string; failed: boolean } {
  const n = list.length
  if (complete) return { text: `${n} 次 SVN 提交`, failed: false }
  if (svn.error) return { text: `SVN 明细没取到：${svn.error.message}`, failed: true }
  if (n === 0 && svn.loading) return { text: '正在读取 SVN 日志…', failed: false }
  return { text: `已加载 ${n} 次 SVN 提交…`, failed: false }
}

function HistoryList({ history, error, selectedId, specs, svn, onSelect, onReveal, onLoadMore }: {
  history: FileHistory | null
  error: string | null
  selectedId: string | null
  specs: GroupSpec[]
  svn: SvnState
  onSelect: (id: string) => void
  onReveal: (id: string) => void
  onLoadMore: () => void
}) {
  if (error) return <div className="diff-note">读取文件历史失败：{error}</div>
  if (!history) return <div className="diff-note">正在读取文件历史…</div>
  if (history.entries.length === 0) {
    return <div className="diff-note">没有找到改动过这个文件的提交（可能还没提交过）。</div>
  }
  const specIndex = new Map(specs.map((s, i) => [s.id, i]))
  return (
    <div className="fh-list" role="listbox" aria-label="文件历史">
      {history.entries.map((e) => {
        const mark = STATUS_MARK[e.status]
        const gi = specIndex.get(e.id)
        const list = gi === undefined ? [] : svn.groups[e.id] ?? []
        const status = gi === undefined ? null : svnGroupStatus(list, groupComplete(gi, svn.cursor), svn)
        return (
          <div key={e.id}>
            <div
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
                <DeliveredTag entry={e} />
              </div>
              <div className="fh-row-meta">
                <span className="fh-sha">{shortId(e.id)}</span>
                <span>{e.author}</span>
                <span>{formatDate(e.time)}</span>
                {e.oldPath && <span className="fh-rename" title={`${e.oldPath} → ${e.path}`}>← {e.oldPath}</span>}
              </div>
              {/* 单独一行：放在上一行末尾时列表一窄就被截掉（真实窗口 E2E 看到的） */}
              {status && (
                <div className="fh-row-meta" title={status.text}>
                  <span className={status.failed ? 'fh-svn-count failed' : 'fh-svn-count'}>{status.text}</span>
                </div>
              )}
            </div>
            {list.map((s) => {
              const key = svnKey(e.id, s.rev)
              return (
                <div
                  key={key}
                  role="option"
                  aria-selected={key === selectedId}
                  className={key === selectedId ? 'fh-row fh-svn-row selected' : 'fh-row fh-svn-row'}
                  onClick={() => onSelect(key)}
                  title={`r${s.rev} ${s.message}\n${s.author} · ${svnDate(s.date, true)}`}
                >
                  <div className="fh-row-top">
                    <span className="fh-subject">{firstLine(s.message) || '（没有提交说明）'}</span>
                  </div>
                  <div className="fh-row-meta">
                    <span className="fh-svn-rev">r{s.rev}</span>
                    <span>{s.author}</span>
                    <span>{svnDate(s.date)}</span>
                  </div>
                </div>
              )
            })}
          </div>
        )
      })}
      {svn.cursor && !svn.loading && (
        <button type="button" className="fh-action fh-load-more" onClick={onLoadMore}>加载更多</button>
      )}
      {history.truncated && (
        <div className="diff-note">只列出最近 {history.entries.length} 条。</div>
      )}
    </div>
  )
}

/** 一条 SVN 提交里这个文件的改动。 */
function SvnEntryDiff({ repoPath, picked, onReveal }: {
  repoPath: string
  picked: SvnPicked
  onReveal: (id: string) => void
}) {
  const [diff, setDiff] = useState<DiffData | null>(null)
  const [error, setError] = useState<string | null>(null)
  const optionsKey = useDiffOptionsKey()
  const { group, entry } = picked
  const shownFor = useRef<string | null>(null)

  useEffect(() => {
    let cancelled = false
    const target = JSON.stringify([repoPath, group.path, group.fileRev, entry.rev])
    if (shownFor.current !== target) {
      shownFor.current = target
      setDiff(null)
    }
    setError(null)
    readSvnRevFileDiff(repoPath, group.path, group.fileRev, entry.rev, backendDiffOptions(getDiffPrefs()))
      .then((data) => { if (!cancelled) setDiff(data) })
      .catch((err) => { if (!cancelled) setError(toErrorMessage(err)) })
    return () => { cancelled = true }
    // `optionsKey` 只用来触发重读，选项本身在上面现取
  }, [repoPath, group.path, group.fileRev, entry.rev, optionsKey])

  return (
    <div className="file-content">
      <div className="fc-head">
        <span className="fc-head-path" title={entry.message}>
          <span className="fh-svn-rev">r{entry.rev}</span> {firstLine(entry.message)}
        </span>
        <button type="button" className="fh-action" disabled title="SVN 条目暂不支持">
          Blame 这一版
        </button>
        <button type="button" className="fh-action" onClick={() => onReveal(group.id)} title="定位到包含它的那条基线提交">
          在提交图中定位
        </button>
      </div>
      <div className="fh-diff">
        {error
          ? <DiffView diff={{ files: [] }} note={`读取改动失败：${error}`} />
          : diff
            ? <DiffView diff={diff} />
            : <div className="diff-note diff-note-pending">正在读取改动…</div>}
      </div>
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
  // 只在要后端重读的开关变了时重读；选项在发请求那一刻取
  const optionsKey = useDiffOptionsKey()
  /** 上一次读的是哪一条。只是开关变了时不清空：旧的 diff 留着，新的回来再换，工具栏不会闪掉。 */
  const shownFor = useRef<string | null>(null)

  useEffect(() => {
    if (!id || !path) return
    let cancelled = false
    const target = JSON.stringify([repoPath, id, path, oldPath])
    if (shownFor.current !== target) {
      shownFor.current = target
      setDiff(null)
    }
    setError(null)
    readCommitFileDiff(repoPath, id, path, oldPath, backendDiffOptions(getDiffPrefs()))
      .then((data) => { if (!cancelled) setDiff(data) })
      .catch((err) => { if (!cancelled) setError(toErrorMessage(err)) })
    return () => { cancelled = true }
    // `optionsKey` 只用来触发重读，选项本身在上面现取
  }, [repoPath, id, path, oldPath, optionsKey])

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
          // 说明放进 DiffView：工具栏还在，开关调坏了能在这里调回去
          ? <DiffView diff={{ files: [] }} note={`读取改动失败：${error}`} />
          : diff
            ? <DiffView diff={diff} />
            : <div className="diff-note diff-note-pending">正在读取改动…</div>}
      </div>
    </div>
  )
}

function BlamePane({ repoPath, rev, initial, headHash, workdirVersion, entries, svnMessages, onBlameRev, onShowEntry, onShowSvn, onReveal }: {
  repoPath: string
  rev: BlameRev
  /** 打开时看的那一版：翻到别的版本之后给一个回去的按钮。 */
  initial: BlameRev
  headHash: string
  workdirVersion: unknown
  entries: FileHistoryEntry[]
  /** History 里已经加载的 SVN 提交说明（revision → 说明）。 */
  svnMessages: Map<number, string>
  onBlameRev: (rev: BlameRev) => void
  onShowEntry: (id: string) => void
  /** 点了一行 SVN 出处：`baseline` 是那行在 git 里归的基线提交。 */
  onShowSvn: (rev: number, baseline: string) => void
  onReveal: (id: string) => void
}) {
  const [blame, setBlame] = useState<BlameData | null>(null)
  const [error, setError] = useState<string | null>(null)
  // 记着是哪一份 Blame 算出来的：Blame 刚换、新的 overlay 还没回来的那一次渲染，不能拿旧的去套
  const [overlayFor, setOverlayFor] = useState<{ blame: BlameData; view: OverlayView } | null>(null)
  const overlayGate = useRef(createLatestGate()).current
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

  // 每次 Blame 重读都重算 SVN 出处：旧结果的行号对不上新的 Blame，留着就会把行归给错的人
  useEffect(() => {
    const token = overlayGate.begin()
    if (!blame || blame.binary) return
    const targets = overlayTargets(blame)
    if (targets.length === 0) return
    readSvnBlameOverlay(repoPath, targets)
      .then((results) => {
        if (overlayGate.isLatest(token)) setOverlayFor({ blame, view: applyOverlay(blame, targets, results) })
      })
      .catch((err) => {
        if (!overlayGate.isLatest(token)) return
        const failed = targets.map(() => ({
          kind: 'failed' as const,
          error: { code: 'svn_cli', message: toErrorMessage(err) },
        }))
        setOverlayFor({ blame, view: applyOverlay(blame, targets, failed) })
      })
  }, [repoPath, blame, overlayGate])
  const overlay = overlayFor && overlayFor.blame === blame ? overlayFor.view : null

  // SVN 出处不同的行要分开成块：同一条基线里的行可能来自好几个 SVN 版本
  const blocks = useMemo(() => {
    const lines = blame?.lines ?? []
    return blameBlocks(lines.map((l, i) => {
      const src = overlay?.lines[i]
      return src?.kind === 'svn' ? { ...l, commit: `svn:${src.rev}` } : l
    }))
  }, [blame, overlay])
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

  /** SVN 出处那一块的右键菜单。`baseline` 是这些行在 git 里归的基线提交。 */
  const svnContextMenu = (svnRev: number, baseline: string, e: React.MouseEvent) => {
    const items: MenuItem[] = [
      { label: '查看这个提交里的改动', onSelect: () => onShowSvn(svnRev, baseline) },
      { label: '在提交图中定位', onSelect: () => onReveal(baseline), title: '定位到包含它的那条基线提交' },
      {
        label: 'Blame 这次改动之前的版本',
        onSelect: () => {},
        disabled: true,
        title: 'SVN 条目暂不支持',
      },
      { label: '复制版本号', onSelect: () => { void navigator.clipboard.writeText(`r${svnRev}`) } },
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
          // 块的键是 `svn:<rev>` 时，这一块来自 SVN；git 里它归的提交仍是那一行原来的 commit
          const gitCommit = blame.lines[block.start].commit
          const src = overlay?.lines[block.start]
          const svnSrc = src?.kind === 'svn' ? src : null
          const info = blame.commits[gitCommit]
          const uncommitted = isUncommitted(gitCommit)
          const h = heat.get(gitCommit) ?? 0
          const delivered = info?.delivered
          const deliveredNote = delivered?.kind === 'undelivered'
            ? '\n未交付'
            : delivered?.kind === 'delivered' ? `\n已交付 r${delivered.rev}` : ''
          const tip = svnSrc
            ? `r${svnSrc.rev} ${svnMessages.get(svnSrc.rev) ?? '（说明未加载）'}\n${svnSrc.author} · ${svnDate(svnSrc.date, true)}\n基线 ${shortId(gitCommit)}`
            : uncommitted
              ? '还没提交的改动'
              : info
                ? `${shortId(gitCommit)} ${info.summary}\n${info.author} · ${formatTime(info.time)}${deliveredNote}`
                : gitCommit
          return (
            <div
              key={block.start}
              className="fh-block"
              onContextMenu={(e) => (svnSrc ? svnContextMenu(svnSrc.rev, gitCommit, e) : contextMenu(gitCommit, e))}
            >
              {blame.lines.slice(block.start, block.start + block.length).map((line, i) => (
                <div key={line.finalLine} className="fc-line">
                  <span
                    className={i === 0 ? 'fh-gutter fh-gutter-start' : 'fh-gutter'}
                    style={{ borderLeftColor: `rgba(216, 166, 87, ${0.12 + 0.88 * h})` }}
                    title={tip}
                    onClick={i === 0
                      ? () => (svnSrc ? onShowSvn(svnSrc.rev, gitCommit) : pickCommit(gitCommit))
                      : undefined}
                  >
                    {i === 0 && (svnSrc ? (
                      <>
                        <span className="fh-svn-rev">r{svnSrc.rev}</span>
                        <span className="fh-gutter-author">{svnSrc.author}</span>
                        <span className="fh-gutter-date">{svnDate(svnSrc.date)}</span>
                      </>
                    ) : uncommitted ? (
                      <span className="fh-uncommitted">未提交</span>
                    ) : (
                      <>
                        <span className="fh-sha">{shortId(gitCommit)}</span>
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
      {overlay?.failure && <div className="fh-note">{svnBanner(overlay.failure)}</div>}
      {overlay && overlay.unmapped > 0 && (
        <div className="fh-note">有 {overlay.unmapped} 行没对上 SVN 版本，保持基线显示</div>
      )}
      {body}
      <ContextMenu menu={menu} onClose={close} />
    </div>
  )
}

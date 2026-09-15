import { useMemo, useRef, useState } from 'react'
import { Panel, Group, Separator } from 'react-resizable-panels'
import type { DiffData, FileChange, StatusData } from '../engine/types'
import { DiffView } from './DiffView'
import { ContextMenu, useContextMenu, type MenuItem } from './ContextMenu'
import {
  emptyFileSelection,
  nextFileSelection,
  type FileSection,
  type FileSelection,
} from '../engine/fileSelection'
import { commitKeyAction, type CommitField } from '../engine/commitKeys'
import { buildCommitMessage } from '../engine/commitActions'

interface FileSectionProps {
  title: string
  section: FileSection
  files: FileChange[]
  actionLabel: string
  /** 当前选中（可能属于另一个区，那样本区就不该高亮任何行）。 */
  selection: FileSelection
  /** diff 面板正在显示的那个文件。 */
  activePath: string | null
  onRowClick: (section: FileSection, files: string[], path: string, e: React.MouseEvent) => void
  onRowContextMenu: (section: FileSection, files: string[], path: string, e: React.MouseEvent) => void
  onToggleOne: (path: string) => void
  onToggleAll: () => void
}

function FileSectionList({
  title, section, files, actionLabel, selection, activePath,
  onRowClick, onRowContextMenu, onToggleOne, onToggleAll,
}: FileSectionProps) {
  const paths = files.map((f) => f.path)
  const selectedHere = selection.section === section ? selection.paths : []

  return (
    <div className="changes-section">
      <div className="changes-section-header">
        <span>
          {title} ({files.length})
          {selectedHere.length > 1 && <span className="changes-selcount">已选 {selectedHere.length}</span>}
        </span>
        {files.length > 0 && (
          <button className="changes-action" onClick={onToggleAll}>{actionLabel} All</button>
        )}
      </div>
      <div className="changes-section-files">
        {files.map((f) => (
          <div
            key={f.path}
            className={[
              'changes-file',
              selectedHere.includes(f.path) ? 'selected' : '',
              f.path === activePath && selection.section === section ? 'active' : '',
              f.status === 'conflicted' ? 'conflicted' : '',
            ].filter(Boolean).join(' ')}
            title={`${f.path}\nCtrl+单击多选 · Shift+单击选一段 · 右键更多操作`}
            onClick={(e) => onRowClick(section, paths, f.path, e)}
            onContextMenu={(e) => onRowContextMenu(section, paths, f.path, e)}
          >
            <button
              className="changes-toggle"
              onClick={(e) => { e.stopPropagation(); onToggleOne(f.path) }}
            >
              {actionLabel}
            </button>
            <span className="changes-file-path">{f.path}</span>
            <span className="changes-file-stats">+{f.additions}/-{f.deletions}</span>
          </div>
        ))}
        {files.length === 0 && <div className="changes-empty">No files</div>}
      </div>
    </div>
  )
}

interface LocalChangesViewProps {
  status: StatusData
  diff: DiffData
  onStage: (paths: string[]) => void
  onUnstage: (paths: string[]) => void
  onDiscard: (paths: string[], staged: boolean) => void
  onCommit: (msg: string, amend: boolean) => void
  /**
   * SVN 混合仓库才显示「仅本地，不交付」勾选框（spec §6.7）。
   *
   * 普通 git 仓库里这个标记没有任何含义——没有交付流程会去读它。
   */
  svnMode: boolean
}

export function LocalChangesView({ status, diff, onStage, onUnstage, onDiscard, onCommit, svnMode }: LocalChangesViewProps) {
  const [selection, setSelection] = useState<FileSelection>(emptyFileSelection)
  const [subject, setSubject] = useState('')
  const [description, setDescription] = useState('')
  const [amend, setAmend] = useState(false)
  const [localOnly, setLocalOnly] = useState(false)
  const descriptionRef = useRef<HTMLTextAreaElement>(null)
  const { menu, open, close } = useContextMenu()

  const allFiles = useMemo(() => [...status.unstaged, ...status.staged], [status])
  // diff 面板显示最后点到的那一个；多选时看的是其中一个，不是拼在一起
  const activeFile = selection.paths[selection.paths.length - 1] ?? allFiles[0]?.path ?? null
  const activeDiff: DiffData = useMemo(() => ({
    files: diff.files.filter((f) => f.path === activeFile),
  }), [diff, activeFile])

  const click = (section: FileSection, files: string[], path: string, e: React.MouseEvent) => {
    setSelection((cur) => nextFileSelection(cur, section, files, path, {
      ctrl: e.ctrlKey || e.metaKey,
      shift: e.shiftKey,
    }))
  }

  const contextMenu = (section: FileSection, files: string[], path: string, e: React.MouseEvent) => {
    // 右键点在已选中的项上不该把整片选中缩成一个——那是最容易误删的交互
    const next = nextFileSelection(selection, section, files, path, {
      ctrl: false, shift: false, keepIfSelected: true,
    })
    setSelection(next)

    const targets = next.paths
    const n = targets.length
    const suffix = n > 1 ? ` (${n})` : ''
    const staged = section === 'staged'
    const items: MenuItem[] = [
      staged
        ? { label: `Unstage${suffix}`, onSelect: () => onUnstage(targets) }
        : { label: `Stage${suffix}`, onSelect: () => onStage(targets) },
      {
        label: `Discard changes${suffix}…`,
        onSelect: () => onDiscard(targets, staged),
        danger: true,
      },
      {
        label: n > 1 ? `Copy paths${suffix}` : 'Copy path',
        onSelect: () => { void navigator.clipboard.writeText(targets.join('\n')) },
      },
    ]
    open(e, items)
  }

  const canCommit = subject.trim().length > 0

  const handleCommit = () => {
    onCommit(buildCommitMessage(subject, description, svnMode && localOnly), amend)
    setSubject('')
    setDescription('')
    // 勾选框**不**重置：连着提交几个本地补丁是常见做法，每次都要重勾很烦。
    // amend 同理（既有行为就是不重置）。
  }

  const commitKeyDown = (field: CommitField) => (e: React.KeyboardEvent) => {
    // isComposing 只在 nativeEvent 上，React 的合成事件没有透出来。
    // 直接把 e 传下去能通过类型检查（该字段是可选的），但运行时永远是 undefined，
    // 输入法那道防线就白写了——所以这里显式拼。
    const action = commitKeyAction(field, {
      key: e.key,
      ctrlKey: e.ctrlKey,
      metaKey: e.metaKey,
      shiftKey: e.shiftKey,
      altKey: e.altKey,
      isComposing: e.nativeEvent.isComposing,
    }, canCommit)
    if (action === 'none') return
    // 走到这里都是我们接管的组合：Ctrl+Enter 在 textarea 里默认会插一个换行，
    // subject 的 Enter 也可能被外层当成表单提交，一律挡掉默认行为。
    e.preventDefault()
    if (action === 'commit') handleCommit()
    else descriptionRef.current?.focus()
  }

  return (
    <div className="local-changes-view">
      <div className="changes-body">
        <Group orientation="horizontal">
          <Panel defaultSize="25%" minSize="15%" maxSize="50%">
            <Group orientation="vertical">
              <Panel minSize="15%">
                <FileSectionList
                  title="Unstaged"
                  section="unstaged"
                  files={status.unstaged}
                  actionLabel="Stage"
                  selection={selection}
                  activePath={activeFile}
                  onRowClick={click}
                  onRowContextMenu={contextMenu}
                  onToggleOne={(path) => onStage([path])}
                  onToggleAll={() => onStage(status.unstaged.map((f) => f.path))}
                />
              </Panel>
              <Separator className="separator-h" />
              <Panel minSize="15%">
                <FileSectionList
                  title="Staged"
                  section="staged"
                  files={status.staged}
                  actionLabel="Unstage"
                  selection={selection}
                  activePath={activeFile}
                  onRowClick={click}
                  onRowContextMenu={contextMenu}
                  onToggleOne={(path) => onUnstage([path])}
                  onToggleAll={() => onUnstage(status.staged.map((f) => f.path))}
                />
              </Panel>
            </Group>
          </Panel>
          <Separator className="separator-v" />
          <Panel minSize="30%">
            <DiffView diff={activeDiff} />
          </Panel>
        </Group>
      </div>
      <div className="changes-commit-box">
        <input
          className="commit-subject"
          value={subject}
          onChange={(e) => setSubject(e.target.value)}
          onKeyDown={commitKeyDown('subject')}
          placeholder="Commit subject"
          title="Enter 跳到 Description · Ctrl+Enter 提交"
        />
        <textarea
          ref={descriptionRef}
          className="commit-description"
          value={description}
          onChange={(e) => setDescription(e.target.value)}
          onKeyDown={commitKeyDown('description')}
          placeholder="Description"
          rows={3}
          title="Ctrl+Enter 提交"
        />
        <div className="commit-actions">
          <label className="changes-amend">
            <input type="checkbox" checked={amend} onChange={(e) => setAmend(e.target.checked)} /> Amend
          </label>
          {svnMode && (
            <label className="changes-local-only">
              <input
                type="checkbox"
                checked={localOnly}
                onChange={(e) => setLocalOnly(e.target.checked)}
              />{' '}
              仅本地，不交付
            </label>
          )}
          <button disabled={!canCommit} onClick={handleCommit} title="Ctrl+Enter">Commit</button>
        </div>
        {/* spec §16 明确要求：Phase 1 到 Phase 3 之间这段窗口期必须在 UI 上写明，
            「不能让用户以为勾了『仅本地』就安全了」。
            **故意不做成 tooltip**——勾选框的 title 已经被占用，而这句话是本期
            唯一能防止误解的地方，藏在 hover 里等于没写。 */}
        {svnMode && localOnly && (
          <p className="changes-local-only-warning">
            标记只是一条 commit message trailer。<strong>交付流程要到 Phase 3 才会读它</strong>
            ——在那之前你手工用 TortoiseSVN 交付时，这个提交里的改动会和别的改动
            一起被提交上去。
          </p>
        )}
      </div>
      <ContextMenu menu={menu} onClose={close} />
    </div>
  )
}

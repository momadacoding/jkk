import { Fragment, useEffect, useMemo, useRef, useState } from 'react'
import { Panel, Group, Separator } from 'react-resizable-panels'
import { PANEL_GROUP_IDS } from '@/engine/layoutMemory'
import { usePanelLayout } from '@/store/layoutStore'
import type { ConflictInfo, DiffData, FileChange, StatusData } from '../engine/types'
import { DiffView } from './DiffView'
import { ContextMenu, useContextMenu, type MenuItem } from './ContextMenu'
import {
  activeChange,
  changesRows,
  dirRowKey,
  emptyFileSelection,
  expandRowKeys,
  isDirRowKey,
  nextFileSelection,
  rangeSelection,
  reconcileSelection,
  rowPath,
  selectAllFiles,
  selectedFiles,
  changesTree,
  treeRowKeys,
  type FileSection,
  type FileSelection,
} from '../engine/fileSelection'
import { collectDirPaths, treeFilePaths, type TreeNode } from '../engine/fileTree'
import { Chevron, FileIcon, FolderIcon, STATUS_MARK } from './FileTree'
import { commitKeyAction, type CommitField } from '../engine/commitKeys'
import { discardShortcut, isSelectAllShortcut } from '../engine/fileListKeys'
import { buildCommitMessage } from '../engine/commitActions'
import { describeStageConflicts } from '../engine/conflicts'
import { externalDiffItems, externalMergeItems, type ExternalToolsMenu } from './externalToolItems'
import { canExternalMerge, conflictView, PENDING_CONFLICT_VIEW } from '@/engine/conflictView'
import { toolFor } from '@/engine/externalTools'
import { ConflictNotice } from './ConflictNotice'
import { useConfirm } from './ConfirmDialog'

interface FileSectionProps {
  title: string
  section: FileSection
  files: FileChange[]
  actionLabel: string
  /** 当前选中（可能属于另一个区，那样本区就不该高亮任何行）。 */
  selection: FileSelection
  /** diff 面板正在显示的那个文件。 */
  activePath: string | null
  /**
   * 行的鼠标事件都带上本区**看得见的**行（树的显示顺序，文件夹行 `目录/` 也在里面，跳过收起
   * 文件夹里的行）：Shift / 拖动选一段按它算，不会把收起文件夹里看不见的行夹进选中。
   * `path` 是被点的那一行的 key：文件是路径，文件夹是 `dirRowKey`。
   */
  onRowClick: (section: FileSection, visible: string[], path: string, e: React.MouseEvent) => void
  onRowContextMenu: (section: FileSection, visible: string[], path: string, e: React.MouseEvent) => void
  /** 按住拖动多选：在行上按下开始，划进别的行时扩到那一行。 */
  onRowMouseDown: (section: FileSection, path: string, e: React.MouseEvent) => void
  onRowMouseEnter: (section: FileSection, visible: string[], path: string, e: React.MouseEvent) => void
  /** 双击行：对这一个文件、或这个文件夹下的全部文件做本区的动作（Stage / Unstage）。 */
  onActivate: (paths: string[]) => void
  /** 区头按钮：本区有选中时传选中展开后的文件，没有时传整区。 */
  onAction: (paths: string[]) => void
  /**
   * 收起目录之后，本区还看得见的行。选中只含看得见的行——收起的目录里还留着选中的话，
   * 用户以为没选的文件会被区头「Stage Selected」、Delete 丢弃、右键动作打到。
   * 收起的文件夹自己那一行看得见，选着它就是要动它下面的全部文件。
   */
  onCollapse: (section: FileSection, visible: string[]) => void
  /** Ctrl+A 按 `visible` 选：看得见的行，收起的文件夹那一行在里面，它里面的行不在。 */
  onKeyDown: (section: FileSection, visible: string[], e: React.KeyboardEvent) => void
}

function FileSectionList({
  title, section, files, actionLabel, selection, activePath,
  onRowClick, onRowContextMenu, onRowMouseDown, onRowMouseEnter, onActivate, onAction, onCollapse, onKeyDown,
}: FileSectionProps) {
  // 与 changesRows / changesPaths 用的是同一棵树，显示顺序和选中、默认项用的顺序一致
  const tree = changesTree(files)
  /**
   * 只记收起的目录，没记的一律展开：改动列表要全展开才看得到，状态刷新后
   * 新冒出来的目录也该是展开的，记覆盖项天然做到这一点（同 FileTree）。
   */
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(() => new Set())
  // 目录整个离开列表（里面的文件都暂存走了）就忘掉它收起过：之后再出现的是新改动，该展开给人看
  useEffect(() => {
    setCollapsed((cur) => {
      if (cur.size === 0) return cur
      const dirs = new Set(collectDirPaths(tree))
      const kept = [...cur].filter((dir) => dirs.has(dir))
      return kept.length === cur.size ? cur : new Set(kept)
    })
  }, [tree])
  const toggleDir = (dir: string) => {
    const next = new Set(collapsed)
    if (next.delete(dir)) {
      setCollapsed(next)
      return
    }
    next.add(dir)
    setCollapsed(next)
    onCollapse(section, treeFilePaths(tree, (d) => !next.has(d)))
  }
  const allPaths = useMemo(() => treeFilePaths(tree), [tree])
  // 看得见的行（文件夹 + 文件）：选中、Shift / 拖动选一段、Ctrl+A 都按它算
  const visible = useMemo(() => treeRowKeys(tree, (dir) => !collapsed.has(dir)), [tree, collapsed])
  const changeByPath = useMemo(() => new Map(files.map((f) => [f.path, f])), [files])
  const selectedHere = selection.section === section ? selection.paths : []
  // Ctrl+A 之后上万行每行都要查一次，用 Set
  const selectedSet = useMemo(() => new Set(selectedHere), [selectedHere])

  const rows = (nodes: TreeNode[], depth: number): React.ReactNode[] => nodes.map((node) => {
    // 每层缩进 14px，与提交详情的文件树一致；文件行留出折叠箭头的位置，名字才对得齐
    const indent = { paddingLeft: 6 + depth * 14 }
    if (node.kind === 'dir') {
      const open = !collapsed.has(node.path)
      const key = dirRowKey(node.path)
      return (
        <Fragment key={`dir:${node.path}`}>
          <div
            className={[
              'changes-dir',
              selectedSet.has(key) ? 'selected' : '',
              key === activePath && selection.section === section ? 'active' : '',
            ].filter(Boolean).join(' ')}
            style={indent}
            title={`${node.path}\n双击 ${actionLabel} 整个文件夹 · 点箭头展开 / 收起 · 右键更多操作`}
            onClick={(e) => onRowClick(section, visible, key, e)}
            onDoubleClick={(e) => {
              if (e.ctrlKey || e.metaKey || e.shiftKey || e.altKey) return
              // 含收起的子文件夹里的：双击的是这个文件夹，不是它眼下展开着的那部分
              onActivate(treeFilePaths(node.children))
            }}
            onContextMenu={(e) => onRowContextMenu(section, visible, key, e)}
            onMouseDown={(e) => onRowMouseDown(section, key, e)}
            onMouseEnter={(e) => onRowMouseEnter(section, visible, key, e)}
          >
            {/* 单击行是选中（和 Fork 一样），展开 / 收起只认箭头。箭头上的按下、双击都不往行上冒：
                快速点两下箭头是收起又展开，不能变成「双击暂存整个文件夹」 */}
            <span
              className="changes-dir-toggle"
              onClick={(e) => { e.stopPropagation(); toggleDir(node.path) }}
              onDoubleClick={(e) => e.stopPropagation()}
              onMouseDown={(e) => e.stopPropagation()}
            >
              <Chevron open={open} />
            </span>
            <FolderIcon />
            <span className="changes-dir-name">{node.name}</span>
          </div>
          {open && rows(node.children, depth + 1)}
        </Fragment>
      )
    }
    const change = changeByPath.get(node.path)
    const mark = node.status ? STATUS_MARK[node.status] : null
    return (
      <div
        key={`file:${node.path}`}
        className={[
          'changes-file',
          selectedSet.has(node.path) ? 'selected' : '',
          node.path === activePath && selection.section === section ? 'active' : '',
          node.status === 'conflicted' ? 'conflicted' : '',
        ].filter(Boolean).join(' ')}
        style={indent}
        title={`${node.path}\n双击 ${actionLabel} · Ctrl+单击多选 · Shift+单击或按住拖动选一段 · Ctrl+A 全选 · Delete 丢弃 · 右键更多操作`}
        onClick={(e) => onRowClick(section, visible, node.path, e)}
        onDoubleClick={(e) => {
          // 带修饰键的双击是在连点多选，不是要暂存
          if (e.ctrlKey || e.metaKey || e.shiftKey || e.altKey) return
          onActivate([node.path])
        }}
        onContextMenu={(e) => onRowContextMenu(section, visible, node.path, e)}
        onMouseDown={(e) => onRowMouseDown(section, node.path, e)}
        onMouseEnter={(e) => onRowMouseEnter(section, visible, node.path, e)}
      >
        <span className="ft-chevron-spacer" />
        {mark && (
          <span
            className={`ft-badge ${mark.kind}`}
            title={node.status === 'conflicted' ? '有冲突，处理完再 Stage' : undefined}
          >
            {mark.label}
          </span>
        )}
        <FileIcon />
        <span className="changes-file-path">{node.name}</span>
        {/* 冲突行的行数没有意义（libgit2 给冲突条目的永远是 0/0），只留角标 */}
        {change && node.status !== 'conflicted' && (
          <span className="changes-file-stats">+{change.additions}/-{change.deletions}</span>
        )}
      </div>
    )
  })

  return (
    <div className="changes-section">
      <div className="changes-section-header">
        <span>
          {title} ({files.length})
          {selectedHere.length > 1 && <span className="changes-selcount">已选 {selectedHere.length}</span>}
        </span>
        {/* 行内不再各带一个按钮（像 Fork 那样）：区头这一个按钮有选中时管选中的，没有时管整区 */}
        {files.length > 0 && (
          <button
            className="changes-action"
            // 选中里可能有文件夹行，交出去之前展开成文件
            onClick={() => onAction(selectedHere.length > 0 ? expandRowKeys(selectedHere, tree) : allPaths)}
          >
            {actionLabel} {selectedHere.length > 0 ? 'Selected' : 'All'}
          </button>
        )}
      </div>
      {/* 可聚焦，点一下行焦点就落在这里：快捷键只在文件列表有焦点时生效，
          挂在 window 上的话 commit 输入框里按 Delete 删字也会被当成丢弃 */}
      <div className="changes-section-files" tabIndex={0} onKeyDown={(e) => onKeyDown(section, visible, e)}>
        {rows(tree, 0)}
        {files.length === 0 && <div className="changes-empty">No files</div>}
      </div>
    </div>
  )
}

interface LocalChangesViewProps {
  status: StatusData
  /** `null` = 工作区 diff 还没读回来（`currentDiff`），比如刚从提交详情切回来：说正在读取，不显示上一份。 */
  diff: DiffData | null
  onStage: (paths: string[]) => void
  onUnstage: (paths: string[]) => void
  onDiscard: (paths: string[], staged: boolean) => void
  /** 返回 `false` = 这次没提交（用户在「先跟进」的提醒里取消了），提交说明要留着。 */
  onCommit: (msg: string, amend: boolean) => void | Promise<boolean>

  /**
   * SVN 混合仓库才显示「仅本地，不交付」勾选框（spec §6.7）。
   *
   * 普通 git 仓库里这个标记没有任何含义——没有交付流程会去读它。
   */
  svnMode: boolean
  /**
   * HEAD 当前在不在基线分支上。
   *
   * spec §6.4：「客户端 UI 禁止在基线分支上开发提交。」流程内部会临时切上去，
   * 但用户在那儿提交会让基线 commit 后面多出与服务器无关的内容——此后
   * 「基线 == 服务器在 rN 的状态」这条前提就不成立了，而体检只比磁盘、发现不了。
   */
  onBaselineBranch: boolean
  /** 外部 diff 工具的菜单项；不传时右键菜单里没有这些项。 */
  externalTools?: ExternalToolsMenu
  /**
   * 冲突文件的分类（`read_conflicts`）。null / 不传表示还没读到：冲突文件只显示
   * 「这个文件有冲突」，不给任何操作，也不显示 diff。
   */
  conflicts?: ConflictInfo[] | null
  /** ③ 一边删一边改的二选一，立即暂存；不传时不显示这两个按钮。 */
  onResolveDeleteModify?: (file: string, keep: boolean) => void
  /**
   * 受控的选中（第 65 条）。右侧 diff 按选中文件所在的区读——Unstaged 是暂存区对比工作区，
   * Staged 是 HEAD 对比暂存区——读哪个 scope 由父组件在渲染时从这份选中算出来，
   * 所以选中放在父组件里。不传时组件自己持有（单独挂载、测试）。
   */
  selection?: FileSelection
  onSelectionChange?: (selection: FileSelection) => void
}

export function LocalChangesView({ status, diff, onStage, onUnstage, onDiscard, onCommit, svnMode, onBaselineBranch, externalTools, conflicts, onResolveDeleteModify, selection: selectionProp, onSelectionChange }: LocalChangesViewProps) {
  const [ownSelection, setOwnSelection] = useState<FileSelection>(emptyFileSelection)
  const setSelection = onSelectionChange ?? setOwnSelection
  // 文件列表 | diff、Unstaged / Staged 两处分割都记住，所有仓库标签共用
  const mainLayout = usePanelLayout(PANEL_GROUP_IDS.changesMain)
  const listsLayout = usePanelLayout(PANEL_GROUP_IDS.changesLists)
  const [subject, setSubject] = useState('')
  const [description, setDescription] = useState('')
  const [amend, setAmend] = useState(false)
  const [localOnly, setLocalOnly] = useState(false)
  const descriptionRef = useRef<HTMLTextAreaElement>(null)
  const { menu, open, close } = useContextMenu()
  const { confirm, dialog: confirmDialog } = useConfirm()

  const conflicted = useMemo(
    () => new Set(status.unstaged.filter((f) => f.status === 'conflicted').map((f) => f.path)),
    [status.unstaged],
  )
  // 两个区全部的行（文件夹 + 文件）：选中里可能有文件夹行，对齐时它也得算「还在列表里」
  const paths = useMemo(() => changesRows(status), [status])
  // 状态刷新后先让选中与列表对齐（文件被整份暂存时跟到 Staged），高亮、快捷键、右侧显示
  // 都用对齐后的这一份；父组件算 scope 用的是同一个函数，两边不会分叉
  const rawSelection = selectionProp ?? ownSelection
  const selection = useMemo(
    () => reconcileSelection(rawSelection, paths.unstaged, paths.staged),
    [rawSelection, paths],
  )
  // 渲染已经按对齐后的那份走了，这里只是把它存回去：不存的话，存着的仍是
  // 「Unstaged 的 a.txt」，a.txt 之后再被改动、两边都有时选中会跳回 Unstaged。
  // 只管自己持有的那份；受控时由持有者在状态变化时对齐（本组件不挂载时状态也会变）
  useEffect(() => {
    if (!selectionProp && selection !== ownSelection) setOwnSelection(selection)
  }, [selectionProp, selection, ownSelection])
  // diff 面板显示最后点到的那一个；多选时看的是其中一个，不是拼在一起。
  // 父组件按它所在的区读 diff，传下来的 `diff` 里同一路径最多一条
  const active = activeChange(selection, paths.unstaged, paths.staged)
  const activeFile = active?.path ?? null
  const activeDiff: DiffData = useMemo(() => ({
    files: (diff?.files ?? []).filter((f) => f.path === activeFile),
  }), [diff, activeFile])
  // 没有文件可显示时父组件什么都不读（`changesDiffScope` 为 null），diff 一直是 null：
  // 这时不能说正在读取，按空 diff 显示
  // 只读选中的那一个文件之后，换选中也要等一次读取（通常几十毫秒）：提示延迟出现，读得快就不闪
  // 最后点的是文件夹：父组件不读 diff（`changesDiffScope` 为 null），这里说明选中了什么
  const activeDirFiles = active && isDirRowKey(active.path)
    ? expandRowKeys([active.path], changesTree(status[active.section])).length
    : null
  const diffPane = active && activeDirFiles !== null
    ? <div className="diff-note">
        已选中文件夹 {rowPath(active.path)}，{activeDirFiles} 个文件。
        点其中的文件查看改动；双击或右键可整个{active.section === 'staged' ? '取消暂存' : '暂存'}。
      </div>
    : diff === null && active
    ? <div className="diff-note diff-note-pending">
        {active.section === 'staged' ? '正在读取暂存区的改动…' : '正在读取工作区的改动…'}
      </div>
    // 读失败时 files 是空的：不说明的话就成了「没有改动」（第 72 条）
    : diff?.error
      ? <div className="diff-note">读取改动失败：{diff.error}</div>
      : <DiffView diff={activeDiff} />
  const conflictByPath = useMemo(
    () => new Map((conflicts ?? []).map((c) => [c.path, c])),
    [conflicts],
  )
  /** 这个冲突文件能交给外部 merge 时返回路径，否则 null（③、④c、⑤、还没读到）。 */
  const mergeablePath = (path: string): string | null => {
    const info = conflictByPath.get(path)
    return info && canExternalMerge(info) ? path : null
  }
  const activeConflict = activeFile && conflicted.has(activeFile) ? activeFile : null
  const mergeTool = activeConflict && externalTools?.config
    ? toolFor(externalTools.config, activeConflict, 'merge')
    : null
  const activeConflictInfo = activeConflict ? conflictByPath.get(activeConflict) : undefined
  const activeConflictView = activeConflictInfo
    ? conflictView(activeConflictInfo, mergeTool)
    : PENDING_CONFLICT_VIEW

  /** 「删除文件」会丢掉修改过的那一版，先确认；「保留」不弹框（spec §4.1 ③）。 */
  const deleteConflictFile = async (file: string) => {
    if (!onResolveDeleteModify) return
    const ok = await confirm({
      title: `删除 ${file}？`,
      message: `${file} 会从工作区删除，并立即暂存这次删除；修改过的那一版不会保留。`,
      confirmLabel: '删除文件',
      danger: true,
    })
    if (ok) onResolveDeleteModify(file, false)
  }

  /**
   * 所有 Stage 入口（双击行、区头 Stage Selected / All、右键、冲突说明里的按钮）的唯一出口。
   * 还留着冲突标记、或冲突信息没读到的文件，先确认；取消就整批不暂存（第 85 条）。
   * 没传 `conflicts` 与 null 同义：还没读到。
   */
  const stage = async (paths: string[]) => {
    const request = describeStageConflicts(paths, status.unstaged, conflicts ?? null)
    if (request && !(await confirm(request))) return
    onStage(paths)
  }

  const click = (section: FileSection, files: string[], path: string, e: React.MouseEvent) => {
    // 从对齐后的选中算下一步：父组件里记着的那份可能还停在文件已经离开的那个区
    setSelection(nextFileSelection(selection, section, files, path, {
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

    // 动作作用到文件：选中的文件夹展开成它下面的全部文件，数字是实际会动到的文件数
    const targets = selectedFiles(next, status)
    const n = targets.length
    const suffix = n > 1 ? ` (${n})` : ''
    // 复制的是选中的行本身：选中一个文件夹，复制的是文件夹路径，不是它下面的每个文件
    const copied = next.paths.map(rowPath)
    const staged = section === 'staged'
    const items: MenuItem[] = [
      staged
        ? { label: `Unstage${suffix}`, onSelect: () => onUnstage(targets) }
        : { label: `Stage${suffix}`, onSelect: () => { void stage(targets) } },
      {
        label: `Discard changes${suffix}…`,
        onSelect: () => onDiscard(targets, staged),
        danger: true,
        shortcut: 'Del',
      },
      {
        label: copied.length > 1 ? `Copy paths (${copied.length})` : 'Copy path',
        onSelect: () => { void navigator.clipboard.writeText(copied.join('\n')) },
      },
      // 外部工具一次只开右键点中的那一个文件，不跟随多选；冲突文件只做 merge，不给对比。
      // 文件夹没有可对比的单个文件，不给
      ...(isDirRowKey(path)
        ? []
        : conflicted.has(path)
          ? externalMergeItems(externalTools, mergeablePath(path))
          : externalDiffItems(externalTools, { kind: staged ? 'staged' : 'unstaged', file: path })),
    ]
    open(e, items)
  }

  /**
   * 按住拖动的起点；null = 没在拖。只在 mousedown 到 mouseup 之间有值，
   * 不进 state：它不影响渲染，放 state 里每划过一行都多渲染一次。
   */
  const dragRef = useRef<{ section: FileSection, anchor: string } | null>(null)
  useEffect(() => {
    // 挂在 window 上：在列表外、甚至两个区的分隔条上松开也得结束拖动
    const end = () => { dragRef.current = null }
    window.addEventListener('mouseup', end)
    return () => window.removeEventListener('mouseup', end)
  }, [])

  const rowMouseDown = (section: FileSection, path: string, e: React.MouseEvent) => {
    // 只认不带修饰键的左键：Ctrl / Shift 按下去是单击多选，交给 onClick
    if (e.button !== 0 || e.ctrlKey || e.metaKey || e.shiftKey || e.altKey) return
    // 不改选中、不 preventDefault：没拖动就松开时由 onClick 照常处理；
    // 挡掉默认行为的话列表拿不到焦点，Ctrl+A 和 Delete 就失效了
    dragRef.current = { section, anchor: path }
  }

  const rowMouseEnter = (section: FileSection, files: string[], path: string, e: React.MouseEvent) => {
    const drag = dragRef.current
    if (!drag) return
    // 在窗口外松开时收不到 mouseup，回来时左键已经不在按着了
    if ((e.buttons & 1) === 0) {
      dragRef.current = null
      return
    }
    // 不跨区，理由同 nextFileSelection
    if (drag.section !== section) return
    // 拖到别的行再松开时，click 落在两行的公共祖先（列表容器）上，不会再触发行的 onClick
    const next = rangeSelection(section, files, drag.anchor, path)
    // 锚点在拖动中途被刷新掉时 rangeSelection 换了新锚点，记回来，
    // 否则之后划过的每一行都只会选中它自己
    drag.anchor = next.anchor ?? path
    setSelection(next)
  }

  /** 收起目录后，本区选中里看不见的文件退出选中（见 `FileSectionProps.onCollapse`）。 */
  const collapseDir = (section: FileSection, visible: string[]) => {
    if (selection.section !== section) return
    const shown = new Set(visible)
    const kept = selection.paths.filter((p) => shown.has(p))
    if (kept.length === selection.paths.length) return
    if (kept.length === 0) {
      setSelection(emptyFileSelection)
      return
    }
    const anchor = selection.anchor !== null && shown.has(selection.anchor) ? selection.anchor : kept[0]
    setSelection({ section, paths: kept, anchor })
  }

  const fileListKeyDown = (section: FileSection, visible: string[], e: React.KeyboardEvent) => {
    // isComposing 只在 nativeEvent 上，原因同 commitKeyDown
    const key = {
      key: e.key,
      code: e.code,
      ctrlKey: e.ctrlKey,
      metaKey: e.metaKey,
      shiftKey: e.shiftKey,
      altKey: e.altKey,
      repeat: e.repeat,
      isComposing: e.nativeEvent.isComposing,
    }
    if (isSelectAllShortcut(key)) {
      // 不挡的话 WebView 会把整页文字全选上
      e.preventDefault()
      setSelection(selectAllFiles(selection, section, visible))
      return
    }
    // 选中的文件夹先展开成文件：快捷键和右键 Discard 丢的是同一批
    const req = discardShortcut(key, section, { ...selection, paths: selectedFiles(selection, status) }, paths)
    if (!req) return
    e.preventDefault()
    onDiscard(req.paths, req.staged)
  }

  // 基线分支上一律不能提交（spec §6.4），连 Ctrl+Enter 也走这个判据
  // ——只把按钮置灰而快捷键还通，等于没拦。
  // 提交在途（6.6 review 第 1 条）：要等 onCommit 整个跑完才知道清不清说明，这段时间里按钮和
  // Ctrl+Enter 必须失效，否则双击、按住不放都会把同一条说明提交两次（amend 就是 amend 两次）。
  // state 管按钮置灰，ref 挡同一帧里的第二次调用（state 要等下一次渲染才生效）
  const [committing, setCommitting] = useState(false)
  const committingRef = useRef(false)
  const canCommit = subject.trim().length > 0 && !onBaselineBranch && !committing

  const handleCommit = async () => {
    if (committingRef.current || !canCommit) return
    committingRef.current = true
    setCommitting(true)
    try {
      const committed = await onCommit(buildCommitMessage(subject, description, svnMode && localOnly), amend)
      // 6.6：在「先跟进」的提醒里取消、或者没提交上时，写好的说明不能丢
      if (committed === false) return
      setSubject('')
      setDescription('')
      // 勾选框**不**重置：连着提交几个本地补丁是常见做法，每次都要重勾很烦。
      // amend 同理（既有行为就是不重置）。
    } finally {
      committingRef.current = false
      setCommitting(false)
    }
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
    if (action === 'commit') void handleCommit()
    else descriptionRef.current?.focus()
  }

  return (
    <div className="local-changes-view">
      <div className="changes-body">
        <Group orientation="horizontal" {...mainLayout}>
          <Panel id="files" defaultSize="25%" minSize="15%" maxSize="50%">
            <Group orientation="vertical" {...listsLayout}>
              <Panel id="unstaged" minSize="15%">
                <FileSectionList
                  title="Unstaged"
                  section="unstaged"
                  files={status.unstaged}
                  actionLabel="Stage"
                  selection={selection}
                  activePath={activeFile}
                  onRowClick={click}
                  onRowContextMenu={contextMenu}
                  onRowMouseDown={rowMouseDown}
                  onRowMouseEnter={rowMouseEnter}
                  onActivate={(paths) => { void stage(paths) }}
                  onAction={(paths) => { void stage(paths) }}
                  onCollapse={collapseDir}
                  onKeyDown={fileListKeyDown}
                />
              </Panel>
              <Separator className="separator-h" />
              <Panel id="staged" minSize="15%">
                <FileSectionList
                  title="Staged"
                  section="staged"
                  files={status.staged}
                  actionLabel="Unstage"
                  selection={selection}
                  activePath={activeFile}
                  onRowClick={click}
                  onRowContextMenu={contextMenu}
                  onRowMouseDown={rowMouseDown}
                  onRowMouseEnter={rowMouseEnter}
                  onActivate={onUnstage}
                  onAction={onUnstage}
                  onCollapse={collapseDir}
                  onKeyDown={fileListKeyDown}
                />
              </Panel>
            </Group>
          </Panel>
          <Separator className="separator-v" />
          <Panel id="diff" minSize="30%">
            {activeConflict ? (
              <div className="conflict-pane">
                <ConflictNotice
                  view={activeConflictView}
                  mergeToolLabel={mergeTool?.label ?? null}
                  onOpenMerge={() => { if (mergeTool) externalTools?.onMerge(mergeTool.id, activeConflict) }}
                  onStage={() => { void stage([activeConflict]) }}
                  onKeep={onResolveDeleteModify && (() => onResolveDeleteModify(activeConflict, true))}
                  onDelete={onResolveDeleteModify && (() => { void deleteConflictFile(activeConflict) })}
                />
                {activeConflictView.sameAsHeadNote
                  ? <div className="conflict-same-as-head">{activeConflictView.sameAsHeadNote}</div>
                  : activeConflictView.showDiff && diffPane}
              </div>
            ) : diffPane}
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
          <button
            disabled={!canCommit}
            onClick={handleCommit}
            title={onBaselineBranch ? '基线分支不能用于开发提交' : 'Ctrl+Enter'}
          >
            Commit
          </button>
        </div>
        {onBaselineBranch && (
          <p className="changes-baseline-warning">
            当前在 <code>svn/base</code> 基线分支上。它记的是「上次与服务器对齐时的
            内容」，<strong>不能用于开发提交</strong>（spec §6.4）。请先切回你的工作分支。
          </p>
        )}
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
      {confirmDialog}
    </div>
  )
}

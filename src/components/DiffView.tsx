import { useMemo, useRef, type ReactNode } from 'react'
import type { DiffData, DiffHunk, DiffLine, FileDiff } from '@/engine/types'
import { effectiveContext, stepContext, type DiffPrefs } from '@/engine/diffPrefs'
import { ignoreEolChanges } from '@/engine/diffEol'
import { contentSegments, toSplitRows, type SplitRow } from '@/engine/diffSplit'
import { setDiffPrefs, useDiffPrefs } from '@/store/diffPrefsStore'

/** 按开关处理过的一个 hunk：分栏时的行配对在这里算一次，左右两个窗格共用。 */
interface ShownHunk {
  hunk: DiffHunk
  rows: SplitRow[] | null
}

/** 按开关处理过的一个文件：`eolHidden` = 有改动因为只差换行符被藏掉了。 */
interface ShownFile {
  file: FileDiff
  hunks: ShownHunk[]
  eolHidden: boolean
}

function Code({ content, invisibles }: { content: string, invisibles: boolean }) {
  return (
    <span className="diff-code">
      {contentSegments(content, invisibles).map((seg, i) =>
        seg.mark === null
          ? seg.text
          : <span key={i} className={`diff-ws diff-ws-${seg.mark}`}>{seg.text}</span>,
      )}
    </span>
  )
}

function lineKind(line: DiffLine | null): string {
  if (!line) return ' empty'
  return line.origin === '+' ? ' add' : line.origin === '-' ? ' del' : ''
}

function Line({ line, invisibles }: { line: DiffLine, invisibles: boolean }) {
  return (
    <div className={`diff-line${lineKind(line)}`}>
      {/* 两个行号和 +/- 符号绑在一起横向吸附，长行左右滚动时行号仍然贴在左边 */}
      <span className="diff-gutter">
        <span className="diff-ln">{line.oldLineno ?? ''}</span>
        <span className="diff-ln">{line.newLineno ?? ''}</span>
        <span className="diff-sign">{line.origin === ' ' ? '' : line.origin}</span>
      </span>
      <Code content={line.content} invisibles={invisibles} />
    </div>
  )
}

/** 分栏里一侧的一行：只有这一侧的行号。`line` 为 null 时是对面多出来的那几行的空白占位。 */
function HalfLine({ line, side, invisibles }: { line: DiffLine | null, side: 'old' | 'new', invisibles: boolean }) {
  const no = line ? (side === 'old' ? line.oldLineno : line.newLineno) : null
  return (
    <div className={`diff-line${lineKind(line)}`}>
      <span className="diff-gutter">
        <span className="diff-ln">{no ?? ''}</span>
        <span className="diff-sign">{line && line.origin !== ' ' ? line.origin : ''}</span>
      </span>
      {line ? <Code content={line.content} invisibles={invisibles} /> : <span className="diff-code" />}
    </div>
  )
}

function HunkHeader({ hunk }: { hunk: DiffHunk }) {
  return (
    // 文字单独包一层才吸得住：外层和它的包含块一样宽，没有可以位移的余地
    <div className="diff-hunk-header">
      <span>{hunk.header || `@@ -${hunk.oldStart} +${hunk.newStart} @@`}</span>
    </div>
  )
}

/** 没有可显示的行时说明原因；有行时返回 null。 */
function fileNote(shown: ShownFile, prefs: DiffPrefs): string | null {
  if (shown.file.binary) return '二进制文件，无法按行显示'
  if (shown.hunks.length > 0) return null
  if (shown.eolHidden) return '只有换行符（CRLF / LF）改动，已按设置忽略'
  // 不说「已忽略空白」：改名、只改权限、空的新文件同样没有 hunk，那里并没有藏掉什么（第二轮独立 review 第 8 条）
  if (prefs.ignoreWhitespace) return '没有文本改动（忽略空白开着，只改了空白的行不显示）'
  return '没有文本改动'
}

function FileHeader({ file }: { file: FileDiff }) {
  return (
    <div className="diff-file-header">
      <span>
        {file.oldPath && <span className="diff-file-old">{file.oldPath} → </span>}
        {file.path}
      </span>
    </div>
  )
}

/**
 * 文件 → 说明或 hunk → hunk 头的那一层，三种画法共用；`renderRows` 只管一个 hunk 的行怎么画。
 * 分栏不换行的两个窗格各调一次，结构完全相同，两侧的文件头、hunk 头、说明就逐个对得上。
 */
function FileList({ files, prefs, showPath, renderRows }: {
  files: ShownFile[]
  prefs: DiffPrefs
  showPath: boolean
  renderRows: (hunk: ShownHunk) => ReactNode
}) {
  return (
    <>
      {files.map((shown) => {
        const note = fileNote(shown, prefs)
        return (
          <div key={shown.file.path} className="diff-file">
            {showPath && <FileHeader file={shown.file} />}
            {note
              ? <div className="diff-note">{note}</div>
              : shown.hunks.map((h, i) => (
                <div key={i} className="diff-hunk">
                  <HunkHeader hunk={h.hunk} />
                  {renderRows(h)}
                </div>
              ))}
          </div>
        )
      })}
    </>
  )
}

/**
 * 左右分栏、不换行：两个各自滚动的窗格，纵向和横向滚动互相跟随。
 *
 * 一行一个格子的画法下，长行只能截断或者把右栏整体推出视口；分成两个窗格，两侧各有自己的
 * 横向滚动条、都在视口底部。两侧逐行对齐靠的是每一行高度固定（不换行），占位行也一样高。
 *
 * 跟随是单向的：一侧被用户滚动，另一侧跟过去；跟过去引起的那次 scroll 不再反推回来。
 * 否则两侧最长的行不一样长时，宽的那一侧滚过窄的那一侧的尽头，窄的那一侧被夹在尽头，
 * 它的 scroll 又把宽的那一侧拉回去（独立 review 第 1 条）。
 */
function SplitPanes({ files, prefs, showPath }: { files: ShownFile[], prefs: DiffPrefs, showPath: boolean }) {
  const left = useRef<HTMLDivElement>(null)
  const right = useRef<HTMLDivElement>(null)
  /** 刚被代码改了滚动位置的那一侧，和改完之后它实际停在的位置（被夹到尽头时不等于赋的值）。 */
  const echo = useRef<{ pane: HTMLDivElement, top: number, left: number } | null>(null)
  const follow = (from: HTMLDivElement | null, to: HTMLDivElement | null) => {
    if (!from || !to) return
    const pending = echo.current
    if (pending?.pane === from) {
      echo.current = null
      // 停在代码设的位置上：这次 scroll 是跟随引起的，不反推。位置不一样说明用户在同一帧里也滚了
      // 这一侧，浏览器把两次合成了一个事件，要照常跟随（第二轮独立 review 第 7 条）
      if (from.scrollTop === pending.top && from.scrollLeft === pending.left) return
    }
    const top = to.scrollTop
    const leftPos = to.scrollLeft
    if (top !== from.scrollTop) to.scrollTop = from.scrollTop
    if (leftPos !== from.scrollLeft) to.scrollLeft = from.scrollLeft
    // 只有真的动了才会有那次 scroll：赋的值被夹回原处时不会触发，标记留着就会吞掉用户下一次滚动
    if (to.scrollTop !== top || to.scrollLeft !== leftPos) {
      echo.current = { pane: to, top: to.scrollTop, left: to.scrollLeft }
    }
  }
  const side = (s: 'old' | 'new') => (h: ShownHunk) =>
    h.rows!.map((row, j) => (
      <HalfLine key={j} line={s === 'old' ? row.left : row.right} side={s} invisibles={prefs.showInvisibles} />
    ))
  return (
    <div className="diff-split-panes">
      <div ref={left} className="diff-scroll diff-split-pane" onScroll={() => follow(left.current, right.current)}>
        <FileList files={files} prefs={prefs} showPath={showPath} renderRows={side('old')} />
      </div>
      <div ref={right} className="diff-scroll diff-split-pane" onScroll={() => follow(right.current, left.current)}>
        <FileList files={files} prefs={prefs} showPath={showPath} renderRows={side('new')} />
      </div>
    </div>
  )
}

function ToolButton({ title, pressed, disabled, onClick, children }: {
  title: string
  pressed?: boolean
  disabled?: boolean
  onClick: () => void
  children: ReactNode
}) {
  return (
    <button
      type="button"
      className={`diff-tool${pressed ? ' on' : ''}`}
      title={title}
      aria-label={title}
      aria-pressed={pressed}
      disabled={disabled}
      onClick={onClick}
    >
      {children}
    </button>
  )
}

/** 16×16 的线条图标，颜色跟着按钮的 `color` 走。 */
function Icon({ children }: { children: ReactNode }) {
  return (
    <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.3"
      strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      {children}
    </svg>
  )
}

/** Fork 风格的一排开关，靠右。 */
function DiffToolbar({ prefs }: { prefs: DiffPrefs }) {
  const less = stepContext(prefs.contextLines, -1)
  const more = stepContext(prefs.contextLines, 1)
  return (
    <div className="diff-toolbar" role="toolbar" aria-label="diff 显示选项">
      <ToolButton
        title="忽略换行符改动（CRLF / LF）"
        pressed={prefs.ignoreEol}
        onClick={() => setDiffPrefs({ ignoreEol: !prefs.ignoreEol })}
      >
        <span className="diff-tool-text">EOL</span>
      </ToolButton>
      <ToolButton
        title="忽略空白改动"
        pressed={prefs.ignoreWhitespace}
        onClick={() => setDiffPrefs({ ignoreWhitespace: !prefs.ignoreWhitespace })}
      >
        <Icon><path d="M3 9v3h10V9" /></Icon>
      </ToolButton>
      <ToolButton
        title="显示空格、制表符和行尾 CR"
        pressed={prefs.showInvisibles}
        onClick={() => setDiffPrefs({ showInvisibles: !prefs.showInvisibles })}
      >
        <Icon><path d="M9 13V3M12 13V3M12 3H7.5a2.5 2.5 0 0 0 0 5H9" /></Icon>
      </ToolButton>
      <ToolButton
        title="自动换行"
        pressed={prefs.wrap}
        onClick={() => setDiffPrefs({ wrap: !prefs.wrap })}
      >
        <Icon><path d="M2 4h12M2 8h10a2 2 0 0 1 0 4H8M2 12h3M9.5 10.5 8 12l1.5 1.5" /></Icon>
      </ToolButton>
      <span className="diff-tool-sep" />
      <ToolButton
        title={`减少上下文（当前 ${prefs.contextLines} 行）`}
        disabled={prefs.fullFile || less === null}
        onClick={() => { if (less !== null) setDiffPrefs({ contextLines: less }) }}
      >
        <Icon><path d="M6 4h8M6 8h8M6 12h8M1.5 8h3" /></Icon>
      </ToolButton>
      <span className="diff-tool-count" title="上下文行数">{prefs.fullFile ? '全部' : prefs.contextLines}</span>
      <ToolButton
        title={`增加上下文（当前 ${prefs.contextLines} 行）`}
        disabled={prefs.fullFile || more === null}
        onClick={() => { if (more !== null) setDiffPrefs({ contextLines: more }) }}
      >
        <Icon><path d="M6 4h8M6 8h8M6 12h8M1.5 8h3M3 6.5v3" /></Icon>
      </ToolButton>
      <ToolButton
        title="显示整个文件"
        pressed={prefs.fullFile}
        onClick={() => setDiffPrefs({ fullFile: !prefs.fullFile })}
      >
        <Icon><path d="M7 4h7M7 8h7M7 12h7M3 2v12M1.5 3.5 3 2l1.5 1.5M1.5 12.5 3 14l1.5-1.5" /></Icon>
      </ToolButton>
      <span className="diff-tool-sep" />
      <ToolButton
        title="左右分栏"
        pressed={prefs.split}
        onClick={() => setDiffPrefs({ split: !prefs.split })}
      >
        <Icon><rect x="2" y="2.5" width="12" height="11" rx="1" /><path d="M8 2.5v11" /></Icon>
      </ToolButton>
    </div>
  )
}

interface DiffViewProps {
  diff: DiffData
  /** 是否在每个文件上方显示路径。左边已经有文件树时是多余的。 */
  showPath?: boolean
  /**
   * 有它时不画 diff，只显示这句话（读取失败等）。工具栏照样在：开关调成了读不出来的样子
   * （比如大文件的「整个文件」），用户得能在这里调回去（独立 review 第 6 条）。
   */
  note?: string
}

export function DiffView({ diff, showPath = true, note }: DiffViewProps) {
  const prefs = useDiffPrefs()
  const context = effectiveContext(prefs)
  // 两步分开记：重比换行符（Myers）贵，分栏只是换个画法，切分栏不该把它重跑一遍（第二轮独立 review 第 5 条）
  const eolFiles = useMemo(() => diff.files.map((file) => {
    const eol = prefs.ignoreEol && !file.binary ? ignoreEolChanges(file.hunks, context) : null
    return { file, hunks: eol ? eol.hunks : file.hunks, eolHidden: eol?.hidden ?? false }
  }), [diff, prefs.ignoreEol, context])
  const files = useMemo<ShownFile[]>(() => eolFiles.map((f) => ({
    ...f,
    hunks: f.hunks.map((hunk) => ({ hunk, rows: prefs.split ? toSplitRows(hunk.lines) : null })),
  })), [eolFiles, prefs.split])

  const invisibles = prefs.showInvisibles
  let body: ReactNode
  if (note !== undefined) body = <div className="diff-scroll"><div className="diff-note">{note}</div></div>
  else if (files.length === 0) body = <div className="diff-scroll"><div className="diff-note">No changes</div></div>
  else if (!prefs.split) {
    // 上下合在一栏（原来的画法）。自动换行时同样用它，只是行会折下去
    body = (
      <div className="diff-scroll">
        <FileList files={files} prefs={prefs} showPath={showPath} renderRows={(h) =>
          h.hunk.lines.map((line, j) => <Line key={j} line={line} invisibles={invisibles} />)} />
      </div>
    )
  } else if (prefs.wrap) {
    // 分栏 + 自动换行：一行一个两列的格子，两侧折行后取较高的那一侧，天然对齐。
    // 不换行时不能这样画——两侧要各自横向滚动，见 `SplitPanes`
    body = (
      <div className="diff-scroll">
        <FileList files={files} prefs={prefs} showPath={showPath} renderRows={(h) =>
          h.rows!.map((row, j) => (
            <div key={j} className="diff-split-row">
              <HalfLine line={row.left} side="old" invisibles={invisibles} />
              <HalfLine line={row.right} side="new" invisibles={invisibles} />
            </div>
          ))} />
      </div>
    )
  } else body = <SplitPanes files={files} prefs={prefs} showPath={showPath} />

  return (
    <div className={`diff-view${prefs.wrap ? ' wrap' : ''}${prefs.split ? ' split' : ''}`}>
      <DiffToolbar prefs={prefs} />
      {diff.contextNarrowed && note === undefined && (
        <div className="diff-banner">
          这次的改动文件太多，上下文按默认 3 行显示；「整个文件」和调大的上下文只在单个文件或文件不多时生效
        </div>
      )}
      {body}
    </div>
  )
}

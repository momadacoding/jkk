import type { ConflictViewModel } from '@/engine/conflictView'

/** 文案里用反引号包住的名字（driver 名）渲染成等宽，其余照原样。 */
function withInlineCode(text: string) {
  return text.split('`').map((part, i) => (i % 2 === 1 ? <code key={i}>{part}</code> : part))
}

interface ConflictNoticeProps {
  view: ConflictViewModel
  /** 按规则选中的 merge 工具名；没有可用工具时 `view.actions` 里本来就没有 openMerge。 */
  mergeToolLabel: string | null
  onOpenMerge: () => void
  onStage: () => void
  /** ③ 的二选一；不传时不显示这两个按钮。 */
  onKeep?: () => void
  onDelete?: () => void
}

/** 冲突文件选中时，diff 面板顶部的说明和操作（spec §4.1）。 */
export function ConflictNotice({ view, mergeToolLabel, onOpenMerge, onStage, onKeep, onDelete }: ConflictNoticeProps) {
  return (
    <div className="conflict-notice">
      <div className="conflict-notice-heading">{withInlineCode(view.heading)}</div>
      {view.body && <p className="conflict-notice-body">{view.body}</p>}
      {view.actions.length > 0 && (
        <div className="conflict-notice-actions">
          {view.actions.map((action) => {
            switch (action) {
              case 'openMerge':
                return mergeToolLabel
                  ? <button key={action} onClick={onOpenMerge}>{`在 ${mergeToolLabel} 中解决`}</button>
                  : null
              case 'stage':
                return <button key={action} onClick={onStage}>Stage</button>
              case 'keep':
                return onKeep ? <button key={action} onClick={onKeep}>保留修改后的文件</button> : null
              case 'delete':
                return onDelete
                  ? <button key={action} className="danger" onClick={onDelete}>删除文件</button>
                  : null
            }
          })}
        </div>
      )}
    </div>
  )
}

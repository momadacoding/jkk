import type { ExternalTool } from './externalTools'
import type { ConflictInfo, StageInfo } from './types'

/** 冲突提示里的按钮。`keep` / `delete` 是 ③ 的二选一。 */
export type ConflictAction = 'openMerge' | 'stage' | 'keep' | 'delete'

export interface ConflictViewModel {
  heading: string
  body: string
  /** 下面要不要接着显示 HEAD 对比工作区的 diff。 */
  showDiff: boolean
  /** 非 null 时用这句话代替 diff：两边逐字节相同，还没合并。 */
  sameAsHeadNote: string | null
  actions: ConflictAction[]
}

const NOT_TEXT_MERGED = 'git 没有逐行合并这个文件（按 .gitattributes 或 git 配置），需要手动合并'
const SAME_AS_HEAD = '工作区与 HEAD（你这边）完全相同，还没合并'

/**
 * 冲突信息还没读到（或读取失败）时的显示：只说有冲突，不给任何操作、不显示 diff。
 * 确认不了的一律不说「可以暂存」（spec §4.1 总原则）。
 */
export const PENDING_CONFLICT_VIEW: ConflictViewModel = {
  heading: '这个文件有冲突',
  body: '正在读取冲突信息…',
  showDiff: false,
  sameAsHeadNote: null,
  actions: [],
}

function isRegular(s: StageInfo | null): boolean {
  return s === null || s.mode === 0o100644 || s.mode === 0o100755
}

function structuralHeading(info: ConflictInfo): string {
  switch (info.code) {
    case 'DD': return '结构冲突（DD）：两边都删除或改名了这个文件'
    case 'AU': return '结构冲突（AU）：只有 HEAD 这边有这个路径（通常是改名冲突）'
    case 'UA': return '结构冲突（UA）：只有另一边有这个路径（通常是改名冲突）'
  }
  if (![info.base, info.ours, info.theirs].every(isRegular)) {
    return `结构冲突（${info.code}）：这个路径涉及符号链接、子模块或文件与目录互换`
  }
  return `结构冲突（${info.code}）：工作区里这个路径已经不是普通文件`
}

/** 能不能交给外部 merge 工具：只有 ① / ② / ④a / ④b（spec §4.2「入口」）。 */
export function canExternalMerge(info: ConflictInfo): boolean {
  if (info.case === 'markers' || info.case === 'noMarkers') return true
  return info.case === 'notTextMerged' && info.reason !== 'binaryContent'
}

/**
 * 冲突信息映射成右侧的提示（spec §4.1 的表）。
 *
 * 任何情况都不说「已解决」：git 的 index 仍是冲突，只有用户手动 Stage 才算解决。
 * `mergeTool` 为 null（这个文件按规则没有可用的 merge 工具）时不给合并按钮，不悄悄换工具。
 */
export function conflictView(info: ConflictInfo, mergeTool: ExternalTool | null): ConflictViewModel {
  const view = baseView(info)
  if (!mergeTool) view.actions = view.actions.filter((a) => a !== 'openMerge')
  return view
}

function baseView(info: ConflictInfo): ConflictViewModel {
  const none = { body: '', showDiff: false, sameAsHeadNote: null }
  switch (info.case) {
    case 'structural':
      return { ...none, heading: structuralHeading(info), body: '应用内暂不处理这种冲突，请在命令行里解决。', actions: [] }
    case 'deleteModify':
      return {
        ...none,
        heading: info.code === 'DU'
          ? 'HEAD 这边删除了这个文件，另一边修改了它'
          : 'HEAD 这边修改了这个文件，另一边删除了它',
        body: '选择保留修改后的文件，或删除它。选择后立即暂存。',
        actions: ['keep', 'delete'],
      }
    case 'markers':
      return { ...none, heading: '这个文件有冲突', actions: ['openMerge'] }
    case 'noMarkers':
      return { ...none, heading: '未检测到冲突标记，检查后可以暂存', showDiff: true, actions: ['stage'] }
    case 'notTextMerged': {
      if (info.reason === 'binaryContent') {
        return { ...none, heading: '二进制文件冲突', actions: ['stage'] }
      }
      // ④b 只描述当前配置，不说「刚才的合并调用过它」：冲突之后配置可能被改过
      const custom = info.reason === 'customDriver'
      return {
        heading: custom
          ? `这个文件配置了自动合并工具 \`${info.driver ?? ''}\`，但仍处于冲突状态，说明它没能完全合并`
          : NOT_TEXT_MERGED,
        body: custom
          ? '文件内容不能直接当作合并结果（例如 UnityYAMLMerge 会在冲突处取对方的值），需要手动合并'
          : '',
        showDiff: true,
        sameAsHeadNote: info.sameAsHead ? SAME_AS_HEAD : null,
        actions: ['openMerge', 'stage'],
      }
    }
  }
}

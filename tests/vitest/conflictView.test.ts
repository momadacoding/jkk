import { describe, expect, it } from 'vitest'
import {
  canExternalMerge,
  conflictView,
  PENDING_CONFLICT_VIEW,
} from '@/engine/conflictView'
import type { ExternalTool } from '@/engine/externalTools'
import type { ConflictInfo } from '@/engine/types'

const stage = { oid: 'a'.repeat(40), mode: 0o100644 }

function info(over: Partial<ConflictInfo>): ConflictInfo {
  return {
    path: 'a.txt',
    code: 'UU',
    base: stage,
    ours: stage,
    theirs: stage,
    case: 'markers',
    reason: null,
    driver: null,
    sameAsHead: false,
    ...over,
  }
}

const bc: ExternalTool = {
  id: 'bc',
  label: 'Beyond Compare',
  program: 'BComp.exe',
  mergeArgs: ['$LOCAL', '$REMOTE', '$BASE', '$MERGED'],
}

const ALL_CASES: ConflictInfo[] = [
  info({ case: 'markers' }),
  info({ case: 'noMarkers' }),
  info({ case: 'notTextMerged', reason: 'binaryAttr' }),
  info({ case: 'notTextMerged', reason: 'customDriver', driver: 'unityyamlmerge' }),
  info({ case: 'notTextMerged', reason: 'mergeDefault' }),
  info({ case: 'notTextMerged', reason: 'lfs' }),
  info({ case: 'notTextMerged', reason: 'binaryContent' }),
  info({ case: 'deleteModify', code: 'DU', ours: null }),
  info({ case: 'deleteModify', code: 'UD', theirs: null }),
  info({ case: 'structural', code: 'AU', base: null, theirs: null }),
  info({ case: 'structural', code: 'UU', ours: { oid: stage.oid, mode: 0o120000 } }),
]

describe('conflictView', () => {
  it('① 有冲突标记：只给合并按钮，不显示 diff', () => {
    const v = conflictView(info({ case: 'markers' }), bc)
    expect(v.heading).toBe('这个文件有冲突')
    expect(v.showDiff).toBe(false)
    expect(v.actions).toEqual(['openMerge'])
  })

  it('② 没有冲突标记：可以暂存，显示 HEAD 对比工作区的 diff', () => {
    const v = conflictView(info({ case: 'noMarkers' }), bc)
    expect(v.heading).toBe('未检测到冲突标记，检查后可以暂存')
    expect(v.showDiff).toBe(true)
    expect(v.actions).toEqual(['stage'])
  })

  it('④a binary 属性：需要手动合并，合并和 Stage 都给', () => {
    const v = conflictView(info({ case: 'notTextMerged', reason: 'binaryAttr' }), bc)
    expect(v.heading).toBe('git 没有逐行合并这个文件（按 .gitattributes 或 git 配置），需要手动合并')
    expect(v.actions).toEqual(['openMerge', 'stage'])
    expect(v.showDiff).toBe(true)
    expect(v.sameAsHeadNote).toBeNull()
  })

  it('④a 工作区与 HEAD 相同时提示还没合并', () => {
    const v = conflictView(info({ case: 'notTextMerged', reason: 'binaryAttr', sameAsHead: true }), bc)
    expect(v.sameAsHeadNote).toBe('工作区与 HEAD（你这边）完全相同，还没合并')
  })

  it('④a 的其他来源（merge.default、LFS）用同一句文案', () => {
    for (const reason of ['mergeDefault', 'lfs'] as const) {
      const v = conflictView(info({ case: 'notTextMerged', reason }), bc)
      expect(v.heading).toBe('git 没有逐行合并这个文件（按 .gitattributes 或 git 配置），需要手动合并')
      expect(v.actions).toEqual(['openMerge', 'stage'])
    }
  })

  it('④b 自定义 driver：写明 driver 名字，说明内容不能直接当结果', () => {
    const v = conflictView(
      info({ case: 'notTextMerged', reason: 'customDriver', driver: 'unityyamlmerge' }),
      bc,
    )
    expect(v.heading).toContain('这个文件配置了自动合并工具 `unityyamlmerge`，但仍处于冲突状态')
    expect(v.body).toContain('UnityYAMLMerge 会在冲突处取对方的值')
    expect(v.actions).toEqual(['openMerge', 'stage'])
    expect(v.showDiff).toBe(true)
  })

  it('④c 二进制：没有外部 merge 入口，不显示 diff', () => {
    const v = conflictView(info({ case: 'notTextMerged', reason: 'binaryContent' }), bc)
    expect(v.heading).toBe('二进制文件冲突')
    expect(v.actions).toEqual(['stage'])
    expect(v.showDiff).toBe(false)
  })

  it('③ 一边删一边改：保留 / 删除', () => {
    const du = conflictView(info({ case: 'deleteModify', code: 'DU', ours: null }), bc)
    expect(du.actions).toEqual(['keep', 'delete'])
    expect(du.heading).toBe('HEAD 这边删除了这个文件，另一边修改了它')
    const ud = conflictView(info({ case: 'deleteModify', code: 'UD', theirs: null }), bc)
    expect(ud.heading).toBe('HEAD 这边修改了这个文件，另一边删除了它')
  })

  it('⑤ 结构冲突：没有任何操作', () => {
    for (const i of ALL_CASES.filter((c) => c.case === 'structural')) {
      const v = conflictView(i, bc)
      expect(v.actions).toEqual([])
      expect(v.showDiff).toBe(false)
    }
    expect(conflictView(info({ case: 'structural', code: 'AU', base: null, theirs: null }), bc).heading)
      .toContain('AU')
    expect(conflictView(info({ case: 'structural', ours: { oid: stage.oid, mode: 0o120000 } }), bc).heading)
      .toContain('符号链接')
  })

  it('没有可用的 merge 工具时去掉合并按钮', () => {
    for (const i of ALL_CASES) {
      expect(conflictView(i, null).actions).not.toContain('openMerge')
    }
  })

  it('任何情况都不说「已解决」', () => {
    for (const i of [...ALL_CASES, info({ case: 'notTextMerged', reason: 'binaryAttr', sameAsHead: true })]) {
      const v = conflictView(i, bc)
      expect(v.heading).not.toContain('已解决')
      expect(v.body).not.toContain('已解决')
    }
    expect(PENDING_CONFLICT_VIEW.heading).not.toContain('已解决')
  })

  it('冲突信息还没读到时：不给任何操作，也不显示 diff', () => {
    expect(PENDING_CONFLICT_VIEW.actions).toEqual([])
    expect(PENDING_CONFLICT_VIEW.showDiff).toBe(false)
  })
})

describe('canExternalMerge', () => {
  it('只有 ① / ② / ④a / ④b 能交给外部合并工具', () => {
    const allowed = ALL_CASES.filter(canExternalMerge).map((c) => `${c.case}:${c.reason ?? ''}`)
    expect(allowed).toEqual([
      'markers:',
      'noMarkers:',
      'notTextMerged:binaryAttr',
      'notTextMerged:customDriver',
      'notTextMerged:mergeDefault',
      'notTextMerged:lfs',
    ])
  })
})

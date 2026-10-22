import { describe, expect, it } from 'vitest'
import {
  describeToolExit,
  diffMenuEntries,
  diffTargetFromScope,
  matchesPattern,
  toolFor,
  toolMenu,
  type ExternalTool,
  type ExternalToolsConfig,
} from '@/engine/externalTools'

const bc: ExternalTool = {
  id: 'bc',
  label: 'Beyond Compare',
  program: 'BComp.exe',
  diffArgs: ['$LOCAL', '$REMOTE'],
  mergeArgs: ['$LOCAL', '$REMOTE', '$BASE', '$MERGED'],
}
const vscode: ExternalTool = {
  id: 'vscode',
  label: 'VS Code',
  program: 'code',
  diffArgs: ['--wait', '--diff', '$LOCAL', '$REMOTE'],
  mergeArgs: ['--wait', '--merge', '$REMOTE', '$LOCAL', '$BASE', '$MERGED'],
}
const uym: ExternalTool = {
  id: 'uym',
  label: 'UnityYAMLMerge',
  program: 'UnityYAMLMerge.exe',
  mergeArgs: ['merge', '-p', '$BASE', '$REMOTE', '$LOCAL', '$MERGED'],
}

function config(overrides: Partial<ExternalToolsConfig> = {}): ExternalToolsConfig {
  return {
    version: 1,
    defaultDiff: 'bc',
    defaultMerge: 'bc',
    rules: [],
    tools: [bc, vscode, uym],
    ...overrides,
  }
}

describe('matchesPattern', () => {
  it('不区分大小写', () => {
    expect(matchesPattern('*.unity', 'Main.UNITY')).toBe(true)
  })

  it('* 必须匹配到结尾，.meta 不算 .unity', () => {
    expect(matchesPattern('*.unity', 'Main.unity.meta')).toBe(false)
  })

  it('? 恰好匹配一个字符', () => {
    expect(matchesPattern('a?c.txt', 'abc.txt')).toBe(true)
    expect(matchesPattern('a?c.txt', 'ac.txt')).toBe(false)
    expect(matchesPattern('a?c.txt', 'abbc.txt')).toBe(false)
  })

  it('其他正则元字符按字面量处理', () => {
    expect(matchesPattern('[a].txt', '[a].txt')).toBe(true)
    expect(matchesPattern('[a].txt', 'a.txt')).toBe(false)
    expect(matchesPattern('a+b.txt', 'a+b.txt')).toBe(true)
    expect(matchesPattern('a.txt', 'abtxt')).toBe(false)
  })

  it('多个 * 和回溯', () => {
    expect(matchesPattern('*a*b', 'xxaybb')).toBe(true)
    expect(matchesPattern('*a*b', 'xxaybc')).toBe(false)
  })
})

describe('toolFor', () => {
  const unityRule = config({ rules: [{ patterns: ['*.cs'], merge: 'uym' }] })

  it('规则只匹配文件名部分：正斜杠路径', () => {
    expect(toolFor(unityRule, 'Assets/x.cs', 'merge')?.id).toBe('uym')
  })

  it('规则只匹配文件名部分：Windows 反斜杠路径', () => {
    expect(toolFor(unityRule, 'Assets\\x.cs', 'merge')?.id).toBe('uym')
  })

  it('目录名命中模式不算命中', () => {
    expect(toolFor(unityRule, 'lib.cs/readme.md', 'merge')?.id).toBe('bc')
  })

  it('第一条命中的规则生效', () => {
    const cfg = config({
      rules: [
        { patterns: ['*.unity'], merge: 'vscode' },
        { patterns: ['Main.*'], merge: 'uym' },
      ],
    })
    expect(toolFor(cfg, 'Assets/Main.unity', 'merge')?.id).toBe('vscode')
    expect(toolFor(cfg, 'Assets/Main.prefab', 'merge')?.id).toBe('uym')
  })

  it('规则只配了 merge 时，看 diff 回落到 defaultDiff', () => {
    const cfg = config({ rules: [{ patterns: ['*.unity'], merge: 'uym' }] })
    expect(toolFor(cfg, 'Main.unity', 'diff')?.id).toBe('bc')
  })

  it('没有规则命中时用默认工具', () => {
    expect(toolFor(config({ defaultDiff: 'vscode' }), 'a.txt', 'diff')?.id).toBe('vscode')
  })

  it('选中的工具没有 mergeArgs 时返回 null，不换别的工具', () => {
    const cfg = config({ rules: [{ patterns: ['*.unity'], diff: 'uym' }] })
    expect(toolFor(cfg, 'Main.unity', 'diff')).toBeNull()
  })

  it('没配默认工具时返回 null', () => {
    expect(toolFor(config({ defaultMerge: null }), 'a.txt', 'merge')).toBeNull()
  })

  it('引用了不存在的工具时返回 null', () => {
    expect(toolFor(config({ defaultDiff: 'ghost' }), 'a.txt', 'diff')).toBeNull()
  })
})

describe('toolMenu', () => {
  it('others 不包含 primary，也不包含缺对应参数的工具', () => {
    const menu = toolMenu(config(), 'a.txt', 'diff')
    expect(menu.primary?.id).toBe('bc')
    expect(menu.others.map((t) => t.id)).toEqual(['vscode'])
  })

  it('merge 菜单里能做 merge 的工具都在', () => {
    const cfg = config({ rules: [{ patterns: ['*.unity'], merge: 'uym' }] })
    const menu = toolMenu(cfg, 'Main.unity', 'merge')
    expect(menu.primary?.id).toBe('uym')
    expect(menu.others.map((t) => t.id)).toEqual(['bc', 'vscode'])
  })

  it('没有 primary 时 others 仍列出可用工具', () => {
    const menu = toolMenu(config({ defaultDiff: null }), 'a.txt', 'diff')
    expect(menu.primary).toBeNull()
    expect(menu.others.map((t) => t.id)).toEqual(['bc', 'vscode'])
  })
})

describe('diffMenuEntries', () => {
  it('第一项写成「在 X 中对比」，其余工具只写名字', () => {
    expect(diffMenuEntries(config(), 'a.txt')).toEqual({
      primary: { label: '在 Beyond Compare 中对比', toolId: 'bc' },
      others: [{ label: 'VS Code', toolId: 'vscode' }],
    })
  })

  it('配置读取失败时没有任何工具项', () => {
    expect(diffMenuEntries(null, 'a.txt')).toEqual({ primary: null, others: [] })
  })

  it('改名的文件按新文件名匹配规则', () => {
    const cfg = config({ rules: [{ patterns: ['*.unity'], diff: 'vscode' }] })
    expect(diffMenuEntries(cfg, 'Assets/New.unity').primary?.toolId).toBe('vscode')
  })
})

describe('describeToolExit', () => {
  const exit = (over: Partial<Parameters<typeof describeToolExit>[0]>) => ({
    toolLabel: 'Beyond Compare',
    code: 0,
    error: null,
    success: true,
    codeMessage: null,
    ...over,
  })

  it('后端判为正常结束时不打扰用户，即使退出码非 0（BC 的 13 是「有差异」）', () => {
    expect(describeToolExit(exit({ code: 13 }))).toBeNull()
  })

  it('故障码有说明时带上说明', () => {
    const msg = describeToolExit(exit({ code: 103, success: false, codeMessage: 'BComp.exe 找不到 BCompare.exe' }))
    expect(msg).toBe('Beyond Compare 退出码 103：BComp.exe 找不到 BCompare.exe')
  })

  it('没有说明的非正常退出码带上工具名和退出码', () => {
    const msg = describeToolExit(exit({ toolLabel: 'VS Code', code: 1, success: false }))
    expect(msg).toContain('VS Code')
    expect(msg).toContain('1')
  })

  it('等待失败时报原因，不吞掉', () => {
    expect(describeToolExit(exit({ toolLabel: 'VS Code', code: null, error: '等待外部工具退出失败：x', success: false })))
      .toBe('VS Code：等待外部工具退出失败：x')
  })

  it('没有退出码也没有原因时说明被异常终止', () => {
    expect(describeToolExit(exit({ toolLabel: 'VS Code', code: null, success: false }))).toContain('VS Code')
  })
})

describe('diffTargetFromScope', () => {
  it('commit:<oid>', () => {
    expect(diffTargetFromScope('commit:abc', 'n.txt', 'o.txt')).toEqual({
      kind: 'commit',
      oid: 'abc',
      file: 'n.txt',
      oldFile: 'o.txt',
    })
  })

  it('range:<a>..<b>', () => {
    expect(diffTargetFromScope('range:a1..b2', 'n.txt', null)).toEqual({
      kind: 'range',
      from: 'a1',
      to: 'b2',
      file: 'n.txt',
      oldFile: null,
    })
  })

  it('解析不了或不是提交的 scope 不给入口', () => {
    expect(diffTargetFromScope(null, 'a', null)).toBeNull()
    expect(diffTargetFromScope('workdir', 'a', null)).toBeNull()
    expect(diffTargetFromScope('commit:', 'a', null)).toBeNull()
    expect(diffTargetFromScope('range:a..', 'a', null)).toBeNull()
    expect(diffTargetFromScope('range:..b', 'a', null)).toBeNull()
    expect(diffTargetFromScope('range:ab', 'a', null)).toBeNull()
  })
})

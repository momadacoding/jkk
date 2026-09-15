import { describe, expect, it } from 'vitest'
import {
  buildCommitMessage,
  commitMenu,
  formatCommitInfo,
  planCommitCommand,
  shortHash,
  type CommitAction,
} from '../../src/engine/commitActions'
import { isLocalOnly } from '../../src/engine/svnGraph'

describe('buildCommitMessage', () => {
  it('Local-Only trailer 单独成段', () => {
    // 不空一行的话 git interpret-trailers 不认它是 trailer —— 标记看起来打上了，
    // 实际谁也读不到。
    expect(buildCommitMessage('s', '', true)).toBe('s\n\nLocal-Only: true')
    expect(buildCommitMessage('s', 'body', true)).toBe('s\n\nbody\n\nLocal-Only: true')
  })

  it('不勾时不加任何东西', () => {
    expect(buildCommitMessage('s', 'body', false)).toBe('s\n\nbody')
    expect(buildCommitMessage('s', '', false)).toBe('s')
  })

  it('描述以空白结尾时不产生三个连续换行', () => {
    expect(buildCommitMessage('s', 'body\n\n', true)).toBe('s\n\nbody\n\nLocal-Only: true')
  })

  it('拼出来的 message 能被 isLocalOnly 读回来', () => {
    // 写和读是两个模块（commitActions 写、svnGraph 读）。这条把它们钉在一起——
    // 少了它，两边各自改格式都不会有测试变红，而表现是「勾了但图上没角标」。
    expect(isLocalOnly(buildCommitMessage('s', 'body', true))).toBe(true)
    expect(isLocalOnly(buildCommitMessage('s', 'body', false))).toBe(false)
  })
})
import { ALL_CAPABILITIES } from '../../src/engine/capabilities'
import type { CommitNode } from '../../src/engine/types'

const commit: CommitNode = {
  id: 'd086703a1b2c3d4e5f60718293a4b5c6d7e8f901',
  parents: ['a8b30a7ffffffffffffffffffffffffffffffff0'],
  author: 'momadacoding',
  time: 1754668200, // 2025-08-08T15:50:00Z
  message: 'docs(agent)',
}

describe('shortHash', () => {
  it('取前 7 位，和 git 的默认缩写对齐', () => {
    expect(shortHash(commit.id)).toBe('d086703')
  })

  it('本来就短的原样返回，不要补位', () => {
    expect(shortHash('abc')).toBe('abc')
  })
})

describe('formatCommitInfo', () => {
  const info = formatCommitInfo(commit)

  it('带完整 hash——缩写贴到别处可能不唯一', () => {
    expect(info).toContain(commit.id)
    expect(info).not.toMatch(/^d086703$/m)
  })

  it('带作者和提交说明', () => {
    expect(info).toContain('momadacoding')
    expect(info).toContain('docs(agent)')
  })

  it('每个字段独占一行，贴进 issue 里可读', () => {
    const lines = info.split('\n')
    expect(lines[0]).toBe(`commit ${commit.id}`)
    expect(lines.some((l) => l.startsWith('Author:'))).toBe(true)
    expect(lines.some((l) => l.startsWith('Date:'))).toBe(true)
  })
})

describe('commitMenu', () => {
  const actions = (isHead = false) => commitMenu(commit, { isHead, caps: ALL_CAPABILITIES }).map((e) => e.action)

  it('给齐 8 个菜单项，顺序稳定', () => {
    expect(actions()).toEqual([
      'new-branch',
      'new-tag',
      'checkout',
      'cherry-pick',
      'revert',
      'save-patch',
      'copy-hash',
      'copy-info',
    ] satisfies CommitAction[])
  })

  it('标签里带上短 hash，弹出来能确认点的是哪一条', () => {
    const entry = commitMenu(commit, { isHead: false, caps: ALL_CAPABILITIES }).find((e) => e.action === 'checkout')!
    expect(entry.label).toContain('d086703')
  })

  it('已经在这条提交上时 checkout 置灰，而不是消失', () => {
    // 消失会让菜单项位置每行都不一样，肌肉记忆全废
    const entry = commitMenu(commit, { isHead: true, caps: ALL_CAPABILITIES }).find((e) => e.action === 'checkout')!
    expect(entry.disabled).toBe(true)
    expect(actions(true)).toHaveLength(8)
  })

  it('不是 HEAD 时 checkout 可用', () => {
    const entry = commitMenu(commit, { isHead: false, caps: ALL_CAPABILITIES }).find((e) => e.action === 'checkout')!
    expect(entry.disabled).toBeFalsy()
  })

  it('会动工作区的三项都带确认文案', () => {
    const menu = commitMenu(commit, { isHead: false, caps: ALL_CAPABILITIES })
    for (const action of ['checkout', 'cherry-pick', 'revert'] as CommitAction[]) {
      expect(menu.find((e) => e.action === action)!.confirm).toBeTruthy()
    }
  })

  it('checkout 的确认文案要点明游离 HEAD——这是最容易让人懵的后果', () => {
    const entry = commitMenu(commit, { isHead: false, caps: ALL_CAPABILITIES }).find((e) => e.action === 'checkout')!
    expect(entry.confirm).toMatch(/detached/i)
  })

  it('复制类和只读类不弹确认框', () => {
    const menu = commitMenu(commit, { isHead: false, caps: ALL_CAPABILITIES })
    for (const action of ['copy-hash', 'copy-info', 'new-branch', 'new-tag', 'save-patch'] as CommitAction[]) {
      expect(menu.find((e) => e.action === action)!.confirm).toBeUndefined()
    }
  })
})

describe('planCommitCommand', () => {
  it('checkout 直接检出完整 hash，不要拿短 hash 去打', () => {
    expect(planCommitCommand('checkout', commit)).toEqual({
      command: 'checkout',
      args: { rev: commit.id },
    })
  })

  it('cherry-pick 走 cherry_pick 命令', () => {
    expect(planCommitCommand('cherry-pick', commit)).toEqual({
      command: 'cherry_pick',
      args: { rev: commit.id },
    })
  })

  it('revert 走 revert 命令', () => {
    expect(planCommitCommand('revert', commit)).toEqual({
      command: 'revert',
      args: { rev: commit.id },
    })
  })

  it('需要先问用户的项没有直接可跑的命令', () => {
    // 这几项要先弹输入框或存盘框拿到参数，不能在这一层就拼出调用
    for (const action of ['new-branch', 'new-tag', 'save-patch', 'copy-hash', 'copy-info'] as CommitAction[]) {
      expect(planCommitCommand(action, commit)).toBeNull()
    }
  })
})

import { describe, expect, it } from 'vitest'
import {
  holderReason,
  notKillableReason,
  relativeToWorktree,
  type WorktreeHolder,
} from '@/engine/worktreeHolders'

const ROOT = 'C:/Users/me/repo/.claude/worktrees/wt'

function holder(over: Partial<WorktreeHolder>): WorktreeHolder {
  return { pid: 1, name: 'node.exe', kind: 'cwd', path: ROOT, killable: true, isSelf: false, commandLine: null, ...over }
}

describe('relativeToWorktree', () => {
  it('根本身是空串，底下的路径去掉前缀', () => {
    expect(relativeToWorktree(ROOT, ROOT)).toBe('')
    expect(relativeToWorktree(`${ROOT}/src/a.ts`, ROOT)).toBe('src/a.ts')
  })

  it('Windows 形态：反斜杠、尾斜杠、盘符与目录大小写不同都认', () => {
    // PEB 里读出的工作目录是反斜杠、带尾斜杠，大小写也不一定和 git 给的一致
    expect(relativeToWorktree('c:\\users\\ME\\repo\\.claude\\worktrees\\WT\\src\\', ROOT)).toBe('src')
    expect(relativeToWorktree('C:\\Users\\me\\repo\\.claude\\worktrees\\wt\\', `${ROOT}/`)).toBe('')
  })

  it('兄弟目录不能当成它底下的', () => {
    expect(relativeToWorktree(`${ROOT}2/src`, ROOT)).toBe(`${ROOT}2/src`)
  })

  it('空根原样返回', () => {
    expect(relativeToWorktree('C:/x', '')).toBe('C:/x')
  })
})

describe('holderReason', () => {
  it('工作目录停在根上和子目录里，说法不同', () => {
    expect(holderReason(holder({ kind: 'cwd' }), ROOT)).toBe('工作目录停在 worktree 根目录')
    expect(holderReason(holder({ kind: 'cwd', path: `${ROOT}\\src` }), ROOT)).toBe('工作目录停在 src')
  })

  it('打开文件的说出是哪个文件', () => {
    expect(holderReason(holder({ kind: 'openFile', path: `${ROOT}/src/App.tsx` }), ROOT)).toBe('打开着 src/App.tsx')
  })

  it('命令行匹配要说明只是旁证', () => {
    expect(holderReason(holder({ kind: 'commandLine' }), ROOT)).toContain('可能')
  })
})

describe('notKillableReason', () => {
  it('本应用自己开着文件：指向关标签页', () => {
    expect(notKillableReason(holder({ kind: 'openFile', killable: false, isSelf: true }))).toContain('标签页')
  })

  it('本应用或子进程的工作目录停在里面：关标签页没用，要从别处重启', () => {
    expect(notKillableReason(holder({ kind: 'cwd', killable: false, isSelf: true }))).toContain('重新启动')
  })

  it('其余是系统关键进程', () => {
    expect(notKillableReason(holder({ killable: false }))).toContain('系统关键')
  })
})

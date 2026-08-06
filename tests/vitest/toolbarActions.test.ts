import { describe, expect, it } from 'vitest'
import { planToolbarAction } from '../../src/engine/toolbarActions'

describe('planToolbarAction', () => {
  it('默认 fetch 只 prune，不碰其他远程', () => {
    const plan = planToolbarAction('fetch', false, 'master')
    expect(plan.command).toBe('fetch')
    expect(plan.args).toEqual({ all: false })
    expect(plan.confirm).toBeNull()
  })

  it('Ctrl+fetch 抓所有远程', () => {
    expect(planToolbarAction('fetch', true, 'master').args).toEqual({ all: true })
  })

  it('默认 pull 不带 rebase，尊重用户的 pull.rebase 配置', () => {
    expect(planToolbarAction('pull', false, 'master').args).toEqual({ rebase: false })
  })

  it('Ctrl+pull 走 rebase', () => {
    expect(planToolbarAction('pull', true, 'master').args).toEqual({ rebase: true })
  })

  it('默认 push 不强推，也不需要确认', () => {
    const plan = planToolbarAction('push', false, 'master')
    expect(plan.args).toEqual({ force: false })
    expect(plan.confirm).toBeNull()
  })

  it('Ctrl+push 强推，并且必须要求确认', () => {
    // 强推可能覆盖远程历史，这是本工具栏唯一不可逆的动作
    const plan = planToolbarAction('push', true, 'master')
    expect(plan.args).toEqual({ force: true })
    expect(plan.confirm).toBeTruthy()
  })

  it('强推的确认文案里带上分支名，避免推错分支', () => {
    expect(planToolbarAction('push', true, 'release/2.0').confirm).toContain('release/2.0')
  })

  it('Ctrl+stash 连未跟踪文件一起存', () => {
    expect(planToolbarAction('stash', false, 'master').args).toEqual({ message: null, includeUntracked: false })
    expect(planToolbarAction('stash', true, 'master').args).toEqual({ message: null, includeUntracked: true })
  })

  it('按住 Ctrl 时按钮文字要跟着变，否则这功能没人发现得了', () => {
    for (const action of ['fetch', 'pull', 'push', 'stash'] as const) {
      const plain = planToolbarAction(action, false, 'master').label
      const ctrl = planToolbarAction(action, true, 'master').label
      expect(ctrl).not.toBe(plain)
    }
  })

  it('命令名与后端注册的 Tauri 命令一致', () => {
    expect(planToolbarAction('stash', false, 'master').command).toBe('stash_push')
  })
})

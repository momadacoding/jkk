import { describe, expect, it } from 'vitest'
import { describeStaleHooks, healthLabel, isDevBuildExe, type HookHealth } from '@/engine/tsvnHooks'

describe('healthLabel', () => {
  it('三种状态各有文案', () => {
    expect(healthLabel({ state: 'notRegistered' })).toBe('未注册')
    expect(healthLabel({ state: 'healthy', blockLeaks: true })).toBe('已注册（提交前拦截本地补丁）')
    expect(healthLabel({ state: 'healthy', blockLeaks: false })).toBe('已注册')
  })

  it('失效时列出每条原因', () => {
    const h: HookHealth = { state: 'stale', reasons: ['缺少 post_update 钩子', 'exe 换了位置'] }
    expect(healthLabel(h)).toBe('已失效：缺少 post_update 钩子；exe 换了位置')
  })
})

describe('describeStaleHooks', () => {
  it('只有失效时才有横幅，并写明 TortoiseSVN 提交可能报错', () => {
    expect(describeStaleHooks(null)).toBeNull()
    expect(describeStaleHooks({ state: 'notRegistered' })).toBeNull()
    expect(describeStaleHooks({ state: 'healthy', blockLeaks: true })).toBeNull()
    const m = describeStaleHooks({ state: 'stale', reasons: ['post_commit 钩子指向 D:\\old\\app.exe'] })
    expect(m?.title).toBe('TortoiseSVN 钩子已失效')
    expect(m?.detail).toContain('post_commit 钩子指向 D:\\old\\app.exe')
    expect(m?.detail).toContain('TortoiseSVN 提交时可能报钩子错误')
    expect(m?.detail).toContain('作者 + 快照')
  })
})

describe('isDevBuildExe', () => {
  it('认出 cargo 构建目录与 worktree 里的 exe', () => {
    expect(isDevBuildExe('C:\\src\\jkk\\src-tauri\\target\\debug\\app.exe')).toBe(true)
    expect(isDevBuildExe('C:\\src\\jkk\\src-tauri\\target\\release\\app.exe')).toBe(true)
    expect(isDevBuildExe('C:\\src\\jkk\\.claude\\worktrees\\x\\app.exe')).toBe(true)
    expect(isDevBuildExe('C:/src/jkk/.worktrees/x/app.exe')).toBe(true)
    expect(isDevBuildExe('C:\\Program Files\\jkk\\jkk.exe')).toBe(false)
    expect(isDevBuildExe('D:\\tools\\jkk-portable\\jkk.exe')).toBe(false)
  })
})

// @vitest-environment jsdom
import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { TsvnHooksDialog } from '@/components/TsvnHooksDialog'
import type { HookEntry, TsvnHooksStatus } from '@/engine/tsvnHooks'

const entry = (kind: string, wcPath = 'D:\\work\\game'): HookEntry => ({
  kind,
  wcPath,
  command: `"C:\\jkk\\jkk.exe" --tsvn-hook ${kind} --repo "D:\\work\\game"`,
  wait: 'true',
  show: 'hide',
  enforce: 'ask',
})

const base: TsvnHooksStatus = {
  supported: true,
  health: { state: 'notRegistered' },
  planned: ['check_commit', 'pre_commit', 'post_commit', 'post_update'].map((k) => entry(k)),
  conflicts: [],
  exe: 'C:\\Program Files\\jkk\\jkk.exe',
}

function renderWith(status: TsvnHooksStatus) {
  const onRegister = vi.fn()
  const onUnregister = vi.fn()
  const onClose = vi.fn()
  render(
    <TsvnHooksDialog
      status={status}
      onRegister={onRegister}
      onUnregister={onUnregister}
      onClose={onClose}
    />,
  )
  return { onRegister, onUnregister, onClose }
}

describe('TsvnHooksDialog', () => {
  it('列出将写入的每一条，默认勾选拦截，确认时带上 blockLeaks', () => {
    const { onRegister } = renderWith(base)
    for (const k of ['check_commit', 'pre_commit', 'post_commit', 'post_update']) {
      expect(screen.getAllByText(new RegExp(`--tsvn-hook ${k} `)).length).toBe(1)
    }
    expect(screen.getAllByText('D:\\work\\game').length).toBe(4)
    const box = screen.getByRole('checkbox', { name: '提交前拦截本地补丁' }) as HTMLInputElement
    expect(box.checked).toBe(true)
    fireEvent.click(screen.getByRole('button', { name: '注册' }))
    expect(onRegister).toHaveBeenCalledWith(true)
  })

  it('提醒先关掉 TortoiseSVN 的设置窗口：它开着时点确定可能把钩子盖掉，事后没有横幅', () => {
    renderWith(base)
    expect(screen.getByText(/注册前请先关掉 TortoiseSVN 的设置窗口/)).toBeTruthy()
  })

  it('取消勾选拦截后不列 check_commit，注册时 blockLeaks 为 false', () => {
    const { onRegister } = renderWith(base)
    fireEvent.click(screen.getByRole('checkbox', { name: '提交前拦截本地补丁' }))
    expect(screen.queryByText(/--tsvn-hook check_commit /)).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: '注册' }))
    expect(onRegister).toHaveBeenCalledWith(false)
  })

  it('有冲突时注册按钮禁用，并列出冲突的条目', () => {
    const foreign = { ...entry('post_commit', 'D:\\work'), command: 'C:\\corp\\hook.exe' }
    renderWith({ ...base, conflicts: [foreign] })
    expect((screen.getByRole('button', { name: '注册' }) as HTMLButtonElement).disabled).toBe(true)
    expect(screen.getByText('C:\\corp\\hook.exe')).toBeTruthy()
  })

  it('冲突只在 check_commit 上时，取消勾选拦截就能注册（Review 2 第 3 条）', () => {
    const foreign = { ...entry('check_commit'), command: 'C:\\corp\\check.exe' }
    const { onRegister } = renderWith({ ...base, conflicts: [foreign] })
    expect((screen.getByRole('button', { name: '注册' }) as HTMLButtonElement).disabled).toBe(true)
    expect(screen.getByText('C:\\corp\\check.exe')).toBeTruthy()

    fireEvent.click(screen.getByRole('checkbox', { name: '提交前拦截本地补丁' }))

    expect(screen.queryByText('C:\\corp\\check.exe')).toBeNull()
    const ok = screen.getByRole('button', { name: '注册' }) as HTMLButtonElement
    expect(ok.disabled).toBe(false)
    fireEvent.click(ok)
    expect(onRegister).toHaveBeenCalledWith(false)
  })

  it('必选钩子上的冲突不因取消勾选而消失', () => {
    const foreign = { ...entry('post_commit'), command: 'C:\\corp\\hook.exe' }
    renderWith({ ...base, conflicts: [foreign] })
    fireEvent.click(screen.getByRole('checkbox', { name: '提交前拦截本地补丁' }))
    expect((screen.getByRole('button', { name: '注册' }) as HTMLButtonElement).disabled).toBe(true)
  })

  it('注册失败的原因显示在框内：顶部红条被遮罩盖着（Review 2 Minor 第 8 条）', () => {
    render(
      <TsvnHooksDialog
        status={base}
        error="TortoiseSVN 的钩子设置刚被别的程序改过。请先关掉 TortoiseSVN 的设置窗口，再试一次。"
        onRegister={() => {}}
        onUnregister={() => {}}
        onClose={() => {}}
      />,
    )
    expect(screen.getByRole('alert').textContent).toContain('请先关掉 TortoiseSVN 的设置窗口')
  })

  it('写入进行中按钮禁用，连点不会写第二次', () => {
    const onRegister = vi.fn()
    render(
      <TsvnHooksDialog
        status={{ ...base, health: { state: 'healthy', blockLeaks: true } }}
        busy
        onRegister={onRegister}
        onUnregister={() => {}}
        onClose={() => {}}
      />,
    )
    expect((screen.getByRole('button', { name: '重新注册' }) as HTMLButtonElement).disabled).toBe(true)
    expect((screen.getByRole('button', { name: '撤销注册' }) as HTMLButtonElement).disabled).toBe(true)
  })

  it('已注册或已失效时都能撤销注册', () => {
    const { onUnregister } = renderWith({ ...base, health: { state: 'healthy', blockLeaks: true } })
    fireEvent.click(screen.getByRole('button', { name: '撤销注册' }))
    expect(onUnregister).toHaveBeenCalledTimes(1)
  })

  it('失效时可以撤销，也可以重新注册', () => {
    const { onUnregister, onRegister } = renderWith({
      ...base,
      health: { state: 'stale', reasons: ['exe 换了位置'] },
    })
    expect(screen.getByText(/exe 换了位置/)).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: '撤销注册' }))
    fireEvent.click(screen.getByRole('button', { name: '重新注册' }))
    expect(onUnregister).toHaveBeenCalledTimes(1)
    expect(onRegister).toHaveBeenCalledWith(true)
  })

  it('未注册时没有撤销按钮', () => {
    renderWith(base)
    expect(screen.queryByRole('button', { name: '撤销注册' })).toBeNull()
  })

  it('开发构建的 exe 给出警告', () => {
    renderWith({ ...base, exe: 'C:\\src\\jkk\\src-tauri\\target\\debug\\app.exe' })
    expect(screen.getByText(/这是开发构建/)).toBeTruthy()
  })

  it('不支持时只显示说明，没有注册按钮', () => {
    renderWith({ ...base, supported: false, planned: [] })
    expect(screen.getByText(/只在 Windows 上可用/)).toBeTruthy()
    expect(screen.queryByRole('button', { name: '注册' })).toBeNull()
  })

  it('找不到工作副本时不能注册', () => {
    renderWith({ ...base, planned: [] })
    expect(screen.getByText(/没有找到要挂钩子的工作副本/)).toBeTruthy()
    expect((screen.getByRole('button', { name: '注册' }) as HTMLButtonElement).disabled).toBe(true)
  })
})

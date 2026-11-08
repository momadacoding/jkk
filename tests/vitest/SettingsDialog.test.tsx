// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { SettingsDialog } from '@/components/SettingsDialog'
import type { AppSettingsView } from '@/engine/appSettings'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(async () => undefined) }))
const invokeMock = vi.mocked(invoke)
const recordingCalls = () =>
  invokeMock.mock.calls
    .filter(([cmd]) => cmd === 'set_shortcut_recording')
    .map(([, args]) => (args as { recording: boolean }).recording)

afterEach(() => {
  cleanup()
  invokeMock.mockClear()
})

const VIEW: AppSettingsView = {
  closeToTray: true,
  trayAvailable: true,
  showShortcut: 'Ctrl+Alt+J',
  shortcutError: null,
  defaultShortcut: 'Ctrl+Alt+J',
  svnAutoFollow: true,
}

function setup(view: AppSettingsView | null = VIEW, onSave = vi.fn(async () => {})) {
  const onClose = vi.fn()
  render(<SettingsDialog initial={view} loadError="boom" onSave={onSave} onClose={onClose} />)
  return { onSave, onClose }
}

const recorder = () => screen.getByRole('button', { name: '快捷键' })
const button = (name: string) => screen.getByRole('button', { name }) as HTMLButtonElement

describe('SettingsDialog', () => {
  it('关掉自动跟进后保存传 false，说明随之换成关掉之后的样子', async () => {
    const { onSave } = setup()
    const box = screen.getByLabelText(/自动跟进/) as HTMLInputElement
    expect(box.checked).toBe(true)
    expect(screen.getByText(/不改工作区里的文件/)).toBeTruthy()
    fireEvent.click(box)
    expect(screen.getByText(/点「跟进」才会跟上/)).toBeTruthy()
    fireEvent.click(button('保存'))
    await waitFor(() => expect(onSave).toHaveBeenCalledWith(true, 'Ctrl+Alt+J', false))
  })

  it('已关掉的自动跟进打开框时就是不勾的', () => {
    setup({ ...VIEW, svnAutoFollow: false })
    expect((screen.getByLabelText(/自动跟进/) as HTMLInputElement).checked).toBe(false)
  })

  it('录下新组合键后保存，参数是三项设置', async () => {
    const { onSave, onClose } = setup()
    fireEvent.click(recorder())
    expect(recorder().textContent).toBe('请按下组合键…')
    fireEvent.keyDown(recorder(), { code: 'ControlLeft', key: 'Control', ctrlKey: true })
    expect(recorder().textContent).toBe('请按下组合键…')
    fireEvent.keyDown(recorder(), { code: 'KeyK', key: 'k', ctrlKey: true, altKey: true })
    expect(recorder().textContent).toBe('Ctrl+Alt+K')
    fireEvent.click(button('保存'))
    await waitFor(() => expect(onClose).toHaveBeenCalled())
    expect(onSave).toHaveBeenCalledWith(true, 'Ctrl+Alt+K', true)
  })

  it('录制开始 / 录完各通知后端一次：录制期间全局快捷键只呼出、不隐藏', async () => {
    setup()
    expect(recordingCalls()).toEqual([])
    fireEvent.click(recorder())
    await waitFor(() => expect(recordingCalls()).toEqual([true]))
    fireEvent.keyDown(recorder(), { code: 'KeyK', key: 'k', ctrlKey: true, altKey: true })
    await waitFor(() => expect(recordingCalls()).toEqual([true, false]))
  })

  it('录到一半直接关框（卸载）也复位，不能把「正在录制」留在后端', async () => {
    setup()
    fireEvent.click(recorder())
    cleanup()
    await waitFor(() => expect(recordingCalls()).toEqual([true, false]))
  })

  it('录制中按 Esc 只取消录制，不关框', () => {
    const { onClose } = setup()
    fireEvent.click(recorder())
    fireEvent.keyDown(recorder(), { code: 'Escape', key: 'Escape' })
    expect(recorder().textContent).toBe('Ctrl+Alt+J')
    expect(onClose).not.toHaveBeenCalled()
    // 不在录制时的 Esc 照常关框
    fireEvent.keyDown(window, { key: 'Escape' })
    expect(onClose).toHaveBeenCalledTimes(1)
  })

  it('不带 Ctrl / Alt / Win 的按键给提示，原快捷键不变', () => {
    setup()
    fireEvent.click(recorder())
    fireEvent.keyDown(recorder(), { code: 'KeyK', key: 'K', shiftKey: true })
    expect(screen.getByText('至少要带 Ctrl、Alt 或 Win 中的一个')).toBeTruthy()
    fireEvent.blur(recorder())
    expect(recorder().textContent).toBe('Ctrl+Alt+J')
  })

  it('关掉快捷键、关掉托盘后保存传 false 和 null', async () => {
    const { onSave } = setup()
    fireEvent.click(screen.getByLabelText(/隐藏到托盘/))
    fireEvent.click(screen.getByLabelText(/呼出主界面/))
    expect(recorder().disabled).toBe(true)
    fireEvent.click(button('保存'))
    await waitFor(() => expect(onSave).toHaveBeenCalledWith(false, null, true))
  })

  it('保存失败时错误留在框里，不关框，可以再点', async () => {
    const onSave = vi.fn(async () => {
      throw { code: 'settings', message: '快捷键「Ctrl+Alt+J」注册失败' }
    })
    const { onClose } = setup(VIEW, onSave)
    fireEvent.click(button('保存'))
    await screen.findByRole('alert')
    expect(screen.getByRole('alert').textContent).toContain('注册失败')
    expect(onClose).not.toHaveBeenCalled()
    expect(button('保存').disabled).toBe(false)
  })

  it('已保存的快捷键没注册上时提示，改成别的就不再提示，改回原来的又出现', () => {
    setup({ ...VIEW, showShortcut: 'Ctrl+Alt+K', shortcutError: '被别的程序占用了' })
    expect(screen.getByText('被别的程序占用了')).toBeTruthy()
    fireEvent.click(recorder())
    fireEvent.keyDown(recorder(), { code: 'KeyL', key: 'l', ctrlKey: true, altKey: true })
    expect(screen.queryByText('被别的程序占用了')).toBeNull()
    fireEvent.click(recorder())
    fireEvent.keyDown(recorder(), { code: 'KeyK', key: 'k', ctrlKey: true, altKey: true })
    expect(screen.getByText('被别的程序占用了')).toBeTruthy()
  })

  it('按着 AltGr（物理右 Alt）录不进去，松开后左 Ctrl+Alt 照常能录', () => {
    setup()
    fireEvent.click(recorder())
    // Windows 上 AltGr 报成 ctrlKey + altKey
    fireEvent.keyDown(recorder(), { code: 'AltRight', key: 'AltGraph', ctrlKey: true, altKey: true })
    fireEvent.keyDown(recorder(), { code: 'KeyQ', key: '@', ctrlKey: true, altKey: true })
    expect(screen.getByText(/不能用 AltGr/)).toBeTruthy()
    expect(recorder().textContent).toBe('请按下组合键…')
    fireEvent.keyUp(recorder(), { code: 'AltRight', key: 'AltGraph' })
    fireEvent.keyDown(recorder(), { code: 'KeyQ', key: 'q', ctrlKey: true, altKey: true })
    expect(recorder().textContent).toBe('Ctrl+Alt+Q')
  })

  it('托盘没建出来时「隐藏到托盘」置灰且显示为未勾选，保存不受影响', async () => {
    const { onSave } = setup({ ...VIEW, trayAvailable: false })
    const box = screen.getByLabelText(/隐藏到托盘/) as HTMLInputElement
    expect(box.disabled).toBe(true)
    expect(box.checked).toBe(false)
    expect(screen.getByText(/托盘图标没能创建/)).toBeTruthy()
    fireEvent.click(button('保存'))
    await waitFor(() => expect(onSave).toHaveBeenCalled())
  })

  it('读设置失败时只显示错误和关闭按钮', () => {
    setup(null)
    expect(screen.getByRole('alert').textContent).toContain('boom')
    expect(screen.queryByRole('button', { name: '保存' })).toBeNull()
    expect(button('关闭')).toBeTruthy()
  })
})

import { afterEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { setShortcutRecording } from '@/store/appSettingsStore'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
const invokeMock = vi.mocked(invoke)

afterEach(() => invokeMock.mockReset())

/** 手动控制每次 invoke 什么时候完成，模拟后端线程池上的乱序。 */
function deferredInvokes() {
  const pending: Array<() => void> = []
  invokeMock.mockImplementation(
    () => new Promise((resolve) => { pending.push(() => resolve(undefined)) }),
  )
  return pending
}

describe('setShortcutRecording', () => {
  it('串行发送：前一次没完成，后一次不发出去', async () => {
    const pending = deferredInvokes()
    const first = setShortcutRecording(true)
    const second = setShortcutRecording(false)
    await Promise.resolve()
    await Promise.resolve()
    // 只有 true 发出去了；false 要等它完成
    expect(invokeMock.mock.calls).toEqual([['set_shortcut_recording', { recording: true }]])

    pending[0]()
    await first
    await vi.waitFor(() => expect(invokeMock).toHaveBeenCalledTimes(2))
    expect(invokeMock.mock.calls[1]).toEqual(['set_shortcut_recording', { recording: false }])
    pending[1]()
    await second
  })

  it('前一次失败不卡住后面的调用', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
    invokeMock.mockRejectedValueOnce(new Error('boom')).mockResolvedValueOnce(undefined)
    await setShortcutRecording(true)
    await setShortcutRecording(false)
    expect(invokeMock.mock.calls.map(([, args]) => args)).toEqual([{ recording: true }, { recording: false }])
    expect(warn).toHaveBeenCalledTimes(1)
    warn.mockRestore()
  })
})

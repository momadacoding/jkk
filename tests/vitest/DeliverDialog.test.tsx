// @vitest-environment jsdom
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { useDeliverDialog } from '@/components/DeliverDialog'
import type { DeliverRequest } from '@/components/svnSyncOutFlow'
import { describeDeliverPreview, type SyncOutPreview } from '@/engine/svnSyncOut'

type Deliver = (r: DeliverRequest) => Promise<boolean>

/** 把 `useDeliverDialog` 挂进一棵最小的组件树，拿到它的 `deliver`。 */
function mount(): () => Deliver {
  let current: Deliver | null = null
  function Harness() {
    const { deliver, dialog } = useDeliverDialog()
    current = deliver
    return <>{dialog}</>
  }
  render(<Harness />)
  return () => {
    if (!current) throw new Error('Harness 还没渲染')
    return current
  }
}

const preview: SyncOutPreview = {
  origBranch: 'work',
  origHead: 'h'.repeat(40),
  baselineOid: 'b'.repeat(40),
  baselineRev: 1,
  remoteRev: 1,
  deliverOid: 'd'.repeat(40),
  deliverTree: 't'.repeat(40),
  changes: [{ status: 'M', path: 'Code/A.cs' }],
  localOnly: [{ oid: 'abcdef1234567890', subject: 'mute log' }],
  outsideChanges: [],
}

function request(over: Partial<DeliverRequest> = {}): DeliverRequest {
  return {
    model: describeDeliverPreview(preview),
    initialMessage: '',
    onMessageChange: () => {},
    action: async () => {},
    ...over,
  }
}

const deliverButton = () => screen.getByRole('button', { name: '交付' }) as HTMLButtonElement
const messageBox = () => screen.getByLabelText('提交信息') as HTMLTextAreaElement

describe('DeliverDialog', () => {
  it('没写提交信息时「交付」点不了；写了就能点，草稿同步给流程', () => {
    const deliver = mount()
    const onMessageChange = vi.fn()
    act(() => {
      void deliver()(request({ onMessageChange }))
    })
    expect(deliverButton().disabled).toBe(true)
    fireEvent.change(messageBox(), { target: { value: '修复跳跃' } })
    expect(deliverButton().disabled).toBe(false)
    expect(onMessageChange).toHaveBeenLastCalledWith('修复跳跃')
  })

  it('列出被摘掉的本地补丁与文件清单', () => {
    const deliver = mount()
    act(() => {
      void deliver()(request())
    })
    expect(screen.getByText('abcdef1234 mute log')).toBeTruthy()
    expect(screen.getByText('Code/A.cs')).toBeTruthy()
    expect(screen.getByText(/1 个本地补丁/)).toBeTruthy()
  })

  it('有 git 管不到的修改时，不勾选就不能交付', () => {
    const deliver = mount()
    act(() => {
      void deliver()(
        request({
          model: describeDeliverPreview({ ...preview, outsideChanges: ['Code/x.psd'] }),
          initialMessage: '修复',
        }),
      )
    })
    expect(deliverButton().disabled).toBe(true)
    fireEvent.click(screen.getByRole('checkbox'))
    expect(deliverButton().disabled).toBe(false)
  })

  it('上次没交付成功写下的信息原样带回来', () => {
    const deliver = mount()
    act(() => {
      void deliver()(request({ initialMessage: '上次写的' }))
    })
    expect(messageBox().value).toBe('上次写的')
  })

  it('执行失败：框留在原地显示错误，信息不丢，按钮可以再点', async () => {
    const deliver = mount()
    act(() => {
      void deliver()(
        request({
          initialMessage: '修复',
          action: async () => {
            throw { code: 'sync_out_server_ahead', message: '服务器上已经有 r2' }
          },
        }),
      )
    })
    fireEvent.click(deliverButton())
    await waitFor(() => expect(screen.getByText(/服务器上已经有 r2/)).toBeTruthy())
    expect(messageBox().value).toBe('修复')
    expect(deliverButton().disabled).toBe(false)
  })

  it('交付跑起来之后 Esc 与点遮罩都关不掉（svn commit 可能已经发出）', async () => {
    const deliver = mount()
    let release!: () => void
    let result!: Promise<boolean>
    act(() => {
      result = deliver()(
        request({
          initialMessage: '修复',
          action: () => new Promise<void>((r) => (release = r)),
        }),
      )
    })
    fireEvent.click(deliverButton())
    await waitFor(() => expect(screen.getByText('正在交付…')).toBeTruthy())
    fireEvent.keyDown(window, { key: 'Escape' })
    fireEvent.mouseDown(document.querySelector('.dialog-overlay')!)
    expect(screen.getByLabelText('提交信息')).toBeTruthy()
    await act(async () => release())
    await expect(result).resolves.toBe(true)
  })

  it('Ctrl+Enter 交付；成功后框关掉，Promise 给 true', async () => {
    const deliver = mount()
    const action = vi.fn(async () => {})
    let result!: Promise<boolean>
    act(() => {
      result = deliver()(request({ initialMessage: '修复', action }))
    })
    fireEvent.keyDown(messageBox(), { key: 'Enter', ctrlKey: true })
    await expect(result).resolves.toBe(true)
    expect(action).toHaveBeenCalledWith('修复')
    await waitFor(() => expect(screen.queryByLabelText('提交信息')).toBeNull())
  })

  it('Esc 关闭，Promise 给 false', async () => {
    const deliver = mount()
    let result!: Promise<boolean>
    act(() => {
      result = deliver()(request())
    })
    fireEvent.keyDown(window, { key: 'Escape' })
    await expect(result).resolves.toBe(false)
  })
})

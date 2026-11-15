import { describe, expect, it } from 'vitest'
import { createAttentionReporter } from '@/store/attentionStore'

/** 一个可控的 invoke：记下每次发了什么，由测试决定哪一次什么时候回来。 */
function controlled() {
  const calls: { args: unknown; finish: () => void; fail: () => void }[] = []
  const invoke = (_cmd: string, args: unknown) =>
    new Promise<void>((resolve, reject) => {
      calls.push({ args, finish: resolve, fail: () => reject(new Error('x')) })
    })
  return { calls, invoke }
}

const flush = () => new Promise((r) => setTimeout(r, 0))

describe('createAttentionReporter（独立 review 第 1 条）', () => {
  it('一次只发一个：上一次回来之前，下一次不发出去。后端是 async 命令，同时在飞的两次可能倒着生效', async () => {
    const { calls, invoke } = controlled()
    const report = createAttentionReporter(invoke)
    report('C:/r/Client', '冲突')
    report('C:/r/Client', null)
    await flush()
    expect(calls.map((c) => c.args)).toEqual([{ reason: 'Client 没有自动跟进：冲突', flash: true }])

    calls[0].finish()
    await flush()
    expect(calls.map((c) => c.args)).toEqual([
      { reason: 'Client 没有自动跟进：冲突', flash: true },
      { reason: null, flash: false },
    ])
  })

  it('上一次失败了：后面的照样发', async () => {
    const { calls, invoke } = controlled()
    const report = createAttentionReporter(invoke)
    report('C:/r/Client', '冲突')
    report('C:/r/Client', null)
    await flush()
    calls[0].fail()
    await flush()
    expect(calls).toHaveLength(2)
    expect(calls[1].args).toEqual({ reason: null, flash: false })
  })
})

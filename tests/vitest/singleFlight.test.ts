import { describe, expect, it } from 'vitest'
import { createSingleFlight } from '@/engine/singleFlight'

/** 手动控制完成时机的任务：`calls` 记录每次真正开跑，`finish(i, v)` 让第 i 次完成。 */
function controlled<T>() {
  const calls: { resolve: (v: T) => void; reject: (e: unknown) => void }[] = []
  const task = () => new Promise<T>((resolve, reject) => { calls.push({ resolve, reject }) })
  return { calls, task }
}

/** 让已排队的微任务全部跑完。 */
const flush = () => new Promise((r) => setTimeout(r, 0))

describe('createSingleFlight', () => {
  it('没有在跑的：立刻开跑，交回这一轮的结果', async () => {
    const flights = createSingleFlight()
    const { calls, task } = controlled<string>()
    const p = flights.run('k', task)
    expect(calls).toHaveLength(1)
    calls[0].resolve('a')
    await expect(p).resolves.toBe('a')
  })

  it('在跑时再来：不并发，等这一轮跑完才补跑一次', async () => {
    const flights = createSingleFlight()
    const { calls, task } = controlled<string>()
    void flights.run('k', task)
    void flights.run('k', task)
    expect(calls).toHaveLength(1)
    calls[0].resolve('first')
    await flush()
    expect(calls).toHaveLength(2)
  })

  it('在跑时连来多次：只补跑一次，它们都拿补跑那一轮的结果', async () => {
    const flights = createSingleFlight()
    const { calls, task } = controlled<string>()
    const first = flights.run('k', task)
    const a = flights.run('k', task)
    const b = flights.run('k', task)
    const c = flights.run('k', task)
    calls[0].resolve('first')
    await flush()
    expect(calls).toHaveLength(2)
    calls[1].resolve('trailing')
    await expect(first).resolves.toBe('first')
    await expect(Promise.all([a, b, c])).resolves.toEqual(['trailing', 'trailing', 'trailing'])
    await flush()
    expect(calls).toHaveLength(2)
  })

  // 关键的正确性：在跑的那一轮可能是在调用方的写操作**之前**发起的，读到的是旧状态。
  // 调用方拿到的结果必须来自它发起之后才开跑的那一轮
  it('在跑时再来的调用，拿到的结果一定来自它之后才开跑的那一轮', async () => {
    const flights = createSingleFlight()
    let version = 0
    const started: number[] = []
    const { calls, task } = controlled<number>()
    const read = () => { started.push(version); return task() }
    void flights.run('k', read)
    version = 1 // 模拟一次写操作
    const mine = flights.run('k', read)
    calls[0].resolve(started[0])
    await flush()
    calls[1].resolve(started[1])
    await expect(mine).resolves.toBe(1)
  })

  it('补跑用的是最后一次调用传进来的任务', async () => {
    const flights = createSingleFlight()
    const { calls, task } = controlled<string>()
    void flights.run('k', task)
    let usedLast = false
    void flights.run('k', () => Promise.resolve('older'))
    const last = flights.run('k', () => { usedLast = true; return Promise.resolve('last') })
    calls[0].resolve('first')
    await expect(last).resolves.toBe('last')
    expect(usedLast).toBe(true)
  })

  it('在跑的那一轮失败：只有它自己的调用方拿到错误，补跑照常进行', async () => {
    const flights = createSingleFlight()
    const { calls, task } = controlled<string>()
    const first = flights.run('k', task)
    const second = flights.run('k', task)
    calls[0].reject(new Error('boom'))
    await expect(first).rejects.toThrow('boom')
    await flush()
    calls[1].resolve('ok')
    await expect(second).resolves.toBe('ok')
  })

  it('补跑那一轮失败：等它的调用方都拿到错误，之后的调用重新开跑', async () => {
    const flights = createSingleFlight()
    const { calls, task } = controlled<string>()
    void flights.run('k', task)
    const a = flights.run('k', task)
    const b = flights.run('k', task)
    calls[0].resolve('first')
    await flush()
    calls[1].reject(new Error('trailing failed'))
    await expect(a).rejects.toThrow('trailing failed')
    await expect(b).rejects.toThrow('trailing failed')
    const next = flights.run('k', task)
    expect(calls).toHaveLength(3)
    calls[2].resolve('again')
    await expect(next).resolves.toBe('again')
  })

  it('任务同步抛错也按失败处理，不会卡住后面的调用', async () => {
    const flights = createSingleFlight()
    const p = flights.run('k', () => { throw new Error('sync') })
    await expect(p).rejects.toThrow('sync')
    await expect(flights.run('k', () => Promise.resolve('fine'))).resolves.toBe('fine')
  })

  it('不同 key 互不影响，可以同时在跑', () => {
    const flights = createSingleFlight()
    const { calls, task } = controlled<string>()
    void flights.run('status', task)
    void flights.run('diff:unstaged', task)
    expect(calls).toHaveLength(2)
  })

  it('跑完之后再来：重新开跑，不复用上一轮的结果', async () => {
    const flights = createSingleFlight()
    const { calls, task } = controlled<string>()
    const first = flights.run('k', task)
    calls[0].resolve('first')
    await first
    const second = flights.run('k', task)
    expect(calls).toHaveLength(2)
    calls[1].resolve('second')
    await expect(second).resolves.toBe('second')
  })
})

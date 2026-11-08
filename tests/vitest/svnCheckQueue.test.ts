import { describe, expect, it } from 'vitest'
import { createSvnCheckQueue } from '@/engine/svnCheckQueue'
import type { SvnCheckOutcome } from '@/engine/svnSync'

/** 一个可控的检测：每次开跑记下 force，由测试决定什么时候、以什么结局收尾。 */
function controlled() {
  const runs: { force: boolean, finish: (o: SvnCheckOutcome) => void, fail: (e: Error) => void }[] = []
  let running = 0
  let maxRunning = 0
  const run = (force: boolean) =>
    new Promise<SvnCheckOutcome>((resolve, reject) => {
      running += 1
      maxRunning = Math.max(maxRunning, running)
      const done = () => {
        running -= 1
      }
      runs.push({
        force,
        finish: (o) => {
          done()
          resolve(o)
        },
        fail: (e) => {
          done()
          reject(e)
        },
      })
    })
  return {
    runs,
    run,
    get maxRunning() {
      return maxRunning
    },
  }
}

/** 让已排进微任务队列的收尾与补跑都走完。 */
const flush = () => new Promise((r) => setTimeout(r, 0))

describe('createSvnCheckQueue（计划 2026-11-08 Task 2）', () => {
  it('空闲时请求立即开跑', async () => {
    const q = createSvnCheckQueue()
    const c = controlled()
    const p = q.request('C:/r', false, c.run)
    expect(c.runs.map((r) => r.force)).toEqual([false])
    c.runs[0].finish('skipped')
    expect(await p).toBe('skipped')
  })

  it('在跑时连来三次：只补跑一轮，三个调用方拿到同一个结果', async () => {
    const q = createSvnCheckQueue()
    const c = controlled()
    const first = q.request('C:/r', false, c.run)
    const queued = [q.request('C:/r', false, c.run), q.request('C:/r', false, c.run), q.request('C:/r', false, c.run)]
    expect(c.runs).toHaveLength(1)

    c.runs[0].finish('skipped')
    expect(await first).toBe('skipped')
    await flush()
    expect(c.runs).toHaveLength(2)
    c.runs[1].finish('applied')
    expect(await Promise.all(queued)).toEqual(['applied', 'applied', 'applied'])
    expect(c.maxRunning).toBe(1)
  })

  it('在跑一轮非强制时来了强制请求：补跑那轮是强制（HEAD 动了不能被比戳吞掉）', async () => {
    const q = createSvnCheckQueue()
    const c = controlled()
    void q.request('C:/r', false, c.run)
    const forced = q.request('C:/r', true, c.run)
    const after = q.request('C:/r', false, c.run)

    c.runs[0].finish('skipped')
    await flush()
    expect(c.runs.map((r) => r.force)).toEqual([false, true])
    c.runs[1].finish('applied')
    expect(await forced).toBe('applied')
    expect(await after).toBe('applied')
  })

  it('强制标记开跑时取走：开跑之后再来的非强制请求补跑时不强制', async () => {
    const q = createSvnCheckQueue()
    const c = controlled()
    void q.request('C:/r', true, c.run)
    void q.request('C:/r', false, c.run)

    c.runs[0].finish('applied')
    await flush()
    expect(c.runs.map((r) => r.force)).toEqual([true, false])
  })

  it('这一轮抛错：补跑照样开跑，错误只交给这一轮的调用方', async () => {
    const q = createSvnCheckQueue()
    const c = controlled()
    const first = q.request('C:/r', false, c.run)
    const queued = q.request('C:/r', false, c.run)

    c.runs[0].fail(new Error('wc locked'))
    await expect(first).rejects.toThrow('wc locked')
    await flush()
    expect(c.runs).toHaveLength(2)
    c.runs[1].finish('applied')
    expect(await queued).toBe('applied')
  })

  it('不同路径各跑各的', () => {
    const q = createSvnCheckQueue()
    const c = controlled()
    void q.request('C:/a', false, c.run)
    void q.request('C:/b', true, c.run)
    expect(c.runs.map((r) => r.force)).toEqual([false, true])
  })

  it('一条路径的强制标记不漏到别的路径', async () => {
    const q = createSvnCheckQueue()
    const c = controlled()
    void q.request('C:/a', false, c.run)
    void q.request('C:/a', true, c.run)
    void q.request('C:/b', false, c.run)
    expect(c.runs.map((r) => r.force)).toEqual([false, false])
  })
})

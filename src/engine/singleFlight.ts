/**
 * 同一种读取「单飞 + 尾随补跑」。
 *
 * 一次 Stage 之后，用户操作的回调和文件监听（`git add` 改了 `.git/index`）会先后各发一次
 * `read_status`。两轮并发会抢同一块磁盘，大工作副本上每一轮都被拖慢；而按 `createLatestGate`
 * 「只认最后一次」，先发的那一轮做完也会被丢掉，界面要等第二轮（2026-10-30 排查，约 5 秒）。
 *
 * 这里改成：同一个 key 同时只跑一轮。在跑时再来的调用不并发，只记下「跑完再补一次」，
 * 连来多次也只补一次，它们都拿补跑那一轮的结果。
 *
 * **为什么不能直接复用在跑的那一轮**：它可能是在调用方的写操作之前发起的，读到的是旧状态。
 * 补跑一定在调用之后才开跑，所以调用方拿到的结果总是不早于它发起的时刻。
 * 同一个 key 的各轮串行，结果按开跑的先后到达，写回时不需要再核对代次。
 */
export function createSingleFlight(): {
  run<T>(key: string, task: () => Promise<T>): Promise<T>
} {
  interface Trailing {
    task: () => Promise<unknown>
    promise: Promise<unknown>
    resolve: (value: unknown) => void
    reject: (reason: unknown) => void
  }
  const running = new Set<string>()
  const trailing = new Map<string, Trailing>()

  const start = <T>(key: string, task: () => Promise<T>): Promise<T> => {
    running.add(key)
    let result: Promise<T>
    try {
      result = Promise.resolve(task())
    } catch (err) {
      // 同步抛错也要走到下面的收尾，否则 key 永远停在「在跑」
      result = Promise.reject(err)
    }
    const settle = () => {
      const next = trailing.get(key)
      if (!next) {
        running.delete(key)
        return
      }
      trailing.delete(key)
      start(key, next.task).then(next.resolve, next.reject)
    }
    // 这一轮成败都要接着补跑：失败只交给它自己的调用方，不连累排在后面的
    result.then(settle, settle)
    return result
  }

  return {
    run<T>(key: string, task: () => Promise<T>): Promise<T> {
      if (!running.has(key)) return start(key, task)
      let next = trailing.get(key)
      if (!next) {
        let resolve!: (value: unknown) => void
        let reject!: (reason: unknown) => void
        const promise = new Promise<unknown>((res, rej) => { resolve = res; reject = rej })
        next = { task, promise, resolve, reject }
        trailing.set(key, next)
      }
      // 补跑用最后一次传进来的任务：同一个 key 的任务做的是同一件事，最后那个闭包最新
      next.task = task
      return next.promise as Promise<T>
    },
  }
}

import { createAttentionTracker } from '@/engine/attention'
import { invoke as ipcInvoke } from '@/store/ipc'

type SendInvoke = (cmd: string, args: { reason: string | null; flash: boolean }) => Promise<unknown>

/**
 * 造一个上报函数：各标签的原因在这里汇总（`createAttentionTracker`），变了才发给后端 `set_attention`。
 *
 * **一次只发一个**（独立 review 第 1 条）：后端是 async 命令，每次 invoke 各自落在线程池上，同时在飞的
 * 「挂上」和「撤掉」可能倒着生效，角标就永远留着——前端这边已经记成撤了，不会再发。所以排成一条链，
 * 上一次回来（成功或失败）才发下一次。
 *
 * 失败只写 console：提醒挂不上不该变成错误条，横幅上的原因行还在。
 */
export function createAttentionReporter(send: SendInvoke) {
  let chain: Promise<unknown> = Promise.resolve()
  const tracker = createAttentionTracker((reason, flash) => {
    chain = chain.then(() =>
      send('set_attention', { reason, flash }).catch((err) => console.warn('挂提醒失败：', err)),
    )
  })
  return (path: string, reason: string | null) => tracker.report(path, reason)
}

/** 全应用一份：任务栏和托盘只有一个。 */
const report = createAttentionReporter((cmd, args) => ipcInvoke(cmd, args))

/** 一个标签上报「自动跟进没有执行」的原因；`null` = 没有被挡下。标签关掉时也要报一次 `null`。 */
export function reportAutoFollowBlocked(path: string, reason: string | null): void {
  report(path, reason)
}

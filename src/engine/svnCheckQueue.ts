import { createSingleFlight } from './singleFlight'
import type { SvnCheckOutcome } from './svnSync'

/**
 * SVN 失同步检测的单飞队列（计划 2026-11-08 Task 2）：同一个仓库任一时刻只有一轮检测在跑。
 *
 * # 为什么所有入口都要走它
 *
 * 打开仓库、文件变动、窗口焦点、HEAD 变化四个入口原来只有文件变动单飞。外部更新期间这几样事件
 * 一起来，2026-11-07 公司机器上录到最多 8 路并发的 `svn status`，还和 TortoiseSVN 抢 wc.db 的锁。
 * `createLatestGate` 只保证最后一次的结论生效，挡不住早发起的请求在后台跑完。
 *
 * # 为什么要合并「强制」标记
 *
 * 单飞的补跑只跑一轮，排队的请求要的却不一样：打开仓库、HEAD 变了必须跑第二档（切到落后的分支
 * 不改 wc.db，比戳看不见，§6.11.3），焦点与文件变动是戳变了才跑。补跑那一轮要是按「比戳」跑，
 * 合并进来的 HEAD 变化就被吞掉，横幅上没有「跟进」。所以排队时记下「有没有人要强制」，取或；
 * 标记在一轮**开跑时**取走，之后再来的强制请求进下一轮——那一刻工作副本可能又变了。
 * 做法同 `repoStore` 里 `statusWanted` 合并局部 / 全量重读。
 */
export function createSvnCheckQueue(): {
  /**
   * 请求一轮检测。在跑时来的请求合并成一轮补跑，所有合并进来的调用方拿到补跑那轮的结局；
   * 补跑用最后一次传进来的 `run`（同 `createSingleFlight`）。
   */
  request(
    path: string,
    force: boolean,
    run: (force: boolean) => Promise<SvnCheckOutcome>,
  ): Promise<SvnCheckOutcome>
} {
  const flights = createSingleFlight()
  const forced = new Set<string>()
  return {
    request(path, force, run) {
      if (force) forced.add(path)
      return flights.run(path, () => run(forced.delete(path)))
    },
  }
}

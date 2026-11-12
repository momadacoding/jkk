/**
 * 前端所有 `invoke` 的出口，签名与 `@tauri-apps/api/core` 的 `invoke` 相同。
 *
 * 记录模式关闭时原样转发：参数个数不变，返回的就是底层那个 Promise。打开时记一条 `invoke`
 * 记录，结果和异常原样交回。测试 mock 的是 `@tauri-apps/api/core`，这里底下调的就是它，
 * mock 照样生效。
 */
import { invoke as tauriInvoke } from '@tauri-apps/api/core'
import { perf, type Perf } from './perf'

type Invoke = typeof tauriInvoke

export function createInvoke(target: Perf, raw: Invoke): Invoke {
  return <T>(...call: Parameters<Invoke>): Promise<T> => {
    if (!target.enabled) return raw<T>(...call)
    return target.timeInvoke(call[0], call[1], () => raw<T>(...call))
  }
}

// 底层函数按调用时取：测试里 `vi.mocked(invoke)` 换实现也照样生效
export const invoke: Invoke = createInvoke(perf, <T>(...call: Parameters<Invoke>) => tauriInvoke<T>(...call))

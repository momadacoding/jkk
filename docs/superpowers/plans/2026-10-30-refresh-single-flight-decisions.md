# Stage 刷新慢（方案 B）：实现日志

输入是 2026-10-30 的只读排查报告。方案 A（`read_status` 只扫一遍）在另一个分支，日志见
`2026-10-30-stage-refresh-perf-decisions.md`。本文件只记方案 B：同一种刷新在跑时不并发，只补跑一次。

## 进度总账

| Task | 内容 | 状态 | 产出 | 验证 |
|---|---|---|---|---|
| B1 | 单飞 + 尾随补跑的纯逻辑 | 完成 | `src/engine/singleFlight.ts`、`tests/vitest/singleFlight.test.ts` | 10/10 |
| B2 | `refreshStatus` / `refreshDiff` 接入 | 完成 | `src/store/repoStore.ts`、`tests/vitest/repoStoreDiffRefresh.test.ts` | 新增 / 改写的 3 条在旧实现上失败、新实现上通过；review 后补的 2 条换仓库测试同样先失败后通过；`pnpm build` 通过；`pnpm test` 831/831 |
| V | 真实窗口实测 Stage 耗时 | 未做 | | 见「验证范围」 |

## 决策条目

### B1

**语义**：同一个 key 同时只跑一轮。在跑时再来的调用不并发，只登记一次补跑，连来多次也只补一次，它们都拿补跑那一轮的结果。

**为什么不直接复用在跑的那一轮**：它可能是在调用方的写操作之前发起的（比如上一次保存触发的监听），
读到的是旧状态。补跑一定在调用之后才开跑，调用方拿到的结果总是不早于它发起的时刻。
`repoStoreDiffRefresh.test.ts` 的「写操作之前就已经在读的状态不算数」锁住这一点。

- 补跑用最后一次传进来的任务：同一个 key 做的是同一件事，最后那个闭包最新。
- 某一轮失败只交给它自己的调用方，补跑照常进行；任务同步抛错也按失败收尾，不会让 key 卡在「在跑」。

### B2

**偏离报告**

- 报告写的是「只记一笔完成后补跑一次」。实现上还多了一点：在跑的那一轮做完**照常写回**，不再被作废。
  这正是报告 2.3 的根因（先发的那轮被 `latestGate` 丢掉），两件事是同一个改动。

**做法**

- `refreshStatus` 的单飞只包住「`read_status` + 写回 `state.status`」。`refreshConflicts` 放在外面：
  它有自己的代次闸；放进去的话，下一轮状态读取要排在 `read_conflicts` 后面。
  `repoStoreConflicts.test.ts` 第 3 条（上一份 `read_conflicts` 未回时下一轮 status 已能发出）依赖这一点。
- `refreshDiff` 按 `diff:<scope>` 分开单飞，别的 scope 不排队；`wantedDiffScope` 的核对保留不变。
- key 带仓库路径（`status:<path>`、`diff:<path>:<scope>`），写回前再核对「是最后一次请求的路径」。
  第一版沿用 `latestGate` 的取法不带路径，独立 review 指出：补跑只保留最后一次的任务，换仓库时旧仓库的调用
  会顶掉新仓库排队的读取，`openRepo(B)` 拿回 A 的状态。现在 `openRepo` 只被 `RepoTab` 用自己固定的
  `repoPath` 调用，这个场景触发不了，但 store 不该依赖这一点，所以照修，并补了两条测试。
- `refreshStatus` 不再返回 `undefined`，`RepoRefreshers.refreshStatus` 的类型同步收窄为 `Promise<StatusData>`。
  `refreshChanges` 里「被更晚的刷新盖掉时按 `latestStatus` 算区」的兜底没有用了，连同 `latestStatus` ref 一起删掉。
- `latestGate` 仍用于 `refreshConflicts`，不删。

**改写的旧测试**

| 旧测试 | 为什么改 | 新测试 |
|---|---|---|
| `refreshStatus`「被后发的请求盖掉：交回 undefined」 | 这正是要修的行为 | 「在读时再来：不并发；先发的那轮照常写回并交回，后来的拿补读那一轮的结果」 |
| `refreshDiff`「同一个 scope 发了两次：先发的晚到时丢掉」 | 同一个 scope 不再并发，没有「晚到」 | 「同一个 scope 在读时再来：不并发；先读完的照常写回，再补读一次」 |
| `refreshChanges`「状态被更晚的刷新盖掉」 | 不再有被盖掉这回事 | 「状态读取进行中又来一次刷新：用自己那轮的结果算区」与「写操作之前就已经在读的状态不算数」 |

**为什么这么改是安全的**

- 只改前端调度，不碰任何 git 写操作，也不改 Tauri 命令签名。
- 同一个 key 的各轮串行，结果按开读先后到达，最后写进 store 的一定是最后开读的那一轮。原来靠代次闸保证的
  「旧结果不盖新结果」，现在由串行保证。
- 调用方拿到的结果都来自它调用之后才开读的那一轮，所以写操作之后的刷新不会读到写之前的状态。

**验证范围**

- 没在 `pnpm tauri dev` 里实测。AGENTS.md §5.1 对前端运行时代码要求 `pnpm build` 和 `pnpm test`，这两项已跑；
  本改动不涉及 Tauri 命令签名和 UI 样式。Stage 的端到端耗时要在公司机器的真实工作副本上确认，这台开发机上没有。

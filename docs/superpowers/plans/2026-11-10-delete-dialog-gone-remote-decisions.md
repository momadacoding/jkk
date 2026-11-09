# 删除弹框：远程分支已不在时提前告知 —— 实现日志

没有单独的计划文件。范围来自对话：PR 149 让「顺便删远程」在远程分支已被删时算成功；
本次让弹框在勾选「顺便删远程」后就查出远程已没有这条分支，提前告知并清理本地过期的跟踪 ref。

## 进度总账

| Task | 内容 | 状态 | 产出 | 验证 |
|---|---|---|---|---|
| T1 | 后端 `engine::branch::probe_remote_branch`（只读）+ `forget_gone_remote_branch` | 完成 | `src-tauri/src/engine/branch.rs` | `cargo test --test remote_branch` 50/50 |
| T2 | 命令 `probe_remote_branch`、`forget_gone_remote_branch` 并注册 | 完成 | `src-tauri/src/commands/ops.rs`、`lib.rs`、`tests/svn_write_guard.rs` | 同上；`svn_write_guard` 9/9 |
| T3 | store 封装 + 弹框接入 + 删除序列最后一步清理 + 测试 | 完成 | `src/store/repoStore.ts`、`DeleteBranchDialog.tsx`、`RepoTab.tsx`、`src/engine/deleteBranchTargets.ts`、两个 vitest 文件 | vitest 1475/1475 |
| T4 | 布局场景 | 完成 | `tests/layout/scenarios.ts`、`harness.tsx`、`dialogs.layout.ts` | `pnpm test:layout` 152/152 |
| T5 | 全量验证 | 完成 | — | `pnpm build` 通过；`pnpm test` 1475/1475；`cargo fmt --check`、`clippy --all-targets` 无告警；`cargo test` lib + 89 个测试文件全过（内存不足，分 8 批前台跑）；`pnpm tauri dev` 真实窗口经 CDP 调两个新命令通过 |

### T5 真实窗口验证记录

- 在 scratchpad 建 fixture：bare 远程推 `master`、`feat/x`、`feat/live`，克隆后在远程上删掉 `feat/x`。
- `pnpm tauri dev --config {"identifier":"com.momadacoding.jkk.verifyprobe"}`，核对 `app.exe` 在本 worktree 的
  `src-tauri\target\debug\` 下。
- 经 CDP 调 `__TAURI_INTERNALS__.invoke`：
  - `probe_remote_branch(feat/x)` → `false`
  - `probe_remote_branch(feat/live)` → `true`
  - 故意把参数名写成 `remoteName` → 报 `missing required key remote`
  - 探测之后 `origin/feat/x` 仍在（只读）
  - `forget_gone_remote_branch(feat/x)` 之后只剩 `HEAD`、`feat/live`、`master`
- 验完删掉 `%APPDATA%`、`%LOCALAPPDATA%` 下的 `com.momadacoding.jkk.verifyprobe`。
- **没做**：没在窗口里点弹框的 UI（勾选、看说明）。弹框行为由 jsdom 组件测试和 Edge 布局测试覆盖。
- **计划没写的坑**：本机内存紧，后台的整套 `cargo test` 两次被系统回收。改成前台按测试文件分批跑
  （`-j 2`、`--test-threads=2`）。

## 决策条目

### T1 后端探测

- **偏离对话里的说法**：对话里说的是「打开弹框时先清理」。实际在**勾上「顺便删远程」时**才探测，
  沿用现有预取的门控：只删本地分支的人不该因此走网络（SSH agent 审批、GCM 凭据框都可能弹出来，
  见 `DeleteBranchDialog.tsx` 预取那段注释）。
- **不改护栏缓存的语义**：探测走 `remote_head_cache` 现有的 `prefetch()`。查询闭包顺带记下分支是否
  存在，交给缓存的仍只是默认分支名。护栏的一次性、只存成功、TTL、限时等待这几条安全边界一条不动。
- **计划没写的坑**：`prefetch()` 在结果还新鲜或已有查询在路上时**不跑闭包**，此时拿不到存在性。
  这种情况再单独发一次只问这条分支的 `ls-remote`。只在 15 秒内取消又重新勾选时出现，代价是后台
  多一次握手。闭包跑了但查询失败时**不再补问**（用 `ran` 标记区分），避免断网时白等两次。
- **计划没写的坑**：`ls-remote` 的模式按**后缀**匹配。`refs/heads/feat/x` 会匹配出
  `refs/heads/team/refs/heads/feat/x`，只看有没有输出会把它当成「还在」。解析时按 ref 名整段相等认。
  测试里先断言诱饵确实被模式匹配出来，再断言探测答「不在」（最初写的诱饵 `team/feat/x` 根本不会被
  匹配，测试是假的，已改）。
- **多 push 目标的口径**：任何一个 push 目标上还有 → 还在；每个都明确答了没有 → 不在；
  有任何一个问不到（且没有一个答「还在」）→ 未知，界面不做任何改变。
- **偏离最初实现（review 后改）**：最初探测答「没有」时**当场**删跟踪 ref。独立 review 指出两个问题：
  1. 用户还没确认任何删除，勾一下再 Cancel，`origin/x` 也没了。
  2. 删掉之后，PR 列表晚到触发的重查走 `find_remote_branch`，`branch.upstream()` 解析不到，会退到
     **别的远程上的同名分支**，勾选框重新可用、指向一条用户没选过的分支。弹框里 `completed` 的注释
     警告的正是这一类。

  改为：探测**只读**；清理变成删除序列的最后一步（`forgetGoneRemote`，在删本地分支之后），
  只在探测答了「没有」且用户确认的删除全部完成后才做。
- **为什么清理跟踪 ref 是安全的**：
  - 只在删除做完之后清。这时本地分支已经没了，弹框随即关闭，不会再有重查。
  - 只在 push 目标就是 fetch 侧（按远程名问的那种）时清。跟踪 ref 镜像的是 fetch 侧，pushurl 指向
    别处时 push 目标没有这条分支不能说明 fetch 侧也没有。这个判断只读本地配置，不走网络。
  - 复用 PR 149 的 `drop_stale_tracking_refs`：按 fetch refspec 映射，尽力而为，失败不让删除显示成失败。
  - 探测和确认之间远程又建回了这条分支的话，会多删一条跟踪 ref，下次 fetch 就回来了，不丢数据。
  - 都有 fixture 用例。

### T2 命令

- **不占 tokio 工作线程**：命令写成 `async fn`，探测放进 `spawn_blocking`。`run_git` 没有超时，
  卡住时只占阻塞线程池里的一个线程，和现在预取的 `std::thread::spawn` 一样。这是仓库里第一个
  `async fn` 命令；`commands_are_async.rs` 只检查 `(async)` 标记，照样满足。
- **能力检查放进 `spawn_blocking`**：`require_capability` 要识别仓库模式（可能读盘），不放在 async
  上下文里同步跑。
- **前端 `prefetchRemoteDefault` 删掉了**：唯一调用方是删除弹框，改用探测后它成了死代码。后端命令
  `prefetch_remote_default` 保留：review 建议一并删掉，没采纳。不在本次范围内删 Tauri 命令（AGENTS
  §1.2），它的引擎函数仍被测试用来验证护栏与缓存，留作后续单独清理。
- **线程 panic 记日志**：`spawn_blocking` 的 JoinError 当成「问不到」，同时 `log::warn!`，免得和断网分不清。
- 新增命令 `forget_gone_remote_branch`（同步、`(async)` 标记），只读配置加一次 `update-ref -d`。
- **计划没写的坑**：`tests/svn_write_guard.rs` 的 `fn_body` 按下一个 `pub fn` / `fn` 切函数体，不认
  `pub async fn`。第一个 `async fn` 命令一出现，它前面的 `prefetch_remote_default` 的函数体就延伸进
  `probe_remote_branch`，把后者的 `require_capability` 算到前者头上，报「加了守卫却还在放行表里」。
  给 `fn_body` 补上 `pub async fn` / `async fn` 两个边界。两个新命令都自己调 `require_capability`，
  不进放行表。

### T3 弹框

- **记坐标不记布尔**：「已经没有」按 `远程\0分支` 记。PR 列表晚到的重查可能换了远程（现有测试覆盖过
  这个时序），旧答案不能沿用到新的那条上。
- **只认明确的 false**：`null`（问不到）和命令报错都不改界面。删除那一步对「远程已经没有」本来就按
  成功处理（PR 149），这里只是提前告知，出错一律当问不到。
- **用户在探测回来之前取消了勾选**：答案丢弃，不改界面。探测只读，什么都没动。
- **点删除时探测还没回来**：照常带上远程删除，由 PR 149 的容错兜底。探测在删除途中回来、取消了勾选，
  进度文案也不变：文案在点确认时按真正发出去的目标定下（`contactingRemote`），review 指出过它会
  在 push 还在跑时变成「正在删除…」。
- **「远程已经没有」只有一个真相来源**：探测答「没有」时直接 `setAlsoRemote(false)`，之后删不删远程只看
  `alsoRemote`；`remoteGone` 只管勾选框灰掉、说明和删除后的清理（review 指出最初三处各判一次）。

### 最终 review（第二轮）的处理

- **已改**：
  - 探测还在路上时取消再勾选，后端缓存跳过查询、又补问一次 `ls-remote`，等于每勾一次握一次手。改为弹框里
    同一条分支只探测一次（`probes` 复用 Promise，还在路上的也复用），只有上次问不到（null）才重探。
    「取消再勾选会再发一次」的旧测试相应改写为三条：在路上复用、答过「还在」不重探、问不到才重探。
  - 删除进行中回来的「没有」不再采用：push 已经发出去，这时取消勾选并灰掉，push 若因别的原因失败，重试会
    悄悄跳过远程、勾选框也改不回来。于是去掉了上一轮加的 `contactingRemote`（文案本来就不会再变）。
  - 清理失败只 `console.warn`，不再整个吞掉。
  - 恢复 store 的 `prefetchRemoteDefault` 导出：AGENTS §1.2 不许改 store 公共接口，上一轮删掉它属于越界。
  - `staleRemote` 并进 `deleteBranchTargets`（`choice.remoteGone`），弹框的 `DeleteTargets` 改成它的类型别名；
    远程坐标的字符串键统一走 `remoteKey()`。这样「删远程 / 清跟踪 ref」都在那一个有测试的函数里算。
  - `ensure_deletable_on_remote` 的注释仍说弹框走 `prefetch_remote_default`，已改。
- **没改**：
  - 后端 `prefetch_remote_default` 命令无前端调用方：同上，删命令不在本次范围。
  - 探测答「没有」之后、确认之前远程又建回这条分支，清理会多删一条跟踪 ref：下次 fetch 就回来，不丢数据，
    再问一次远程要多一次握手，不值。
  - 删除途中回来的「没有」因为不采用，这次不会清跟踪 ref：下次 fetch `--prune` 会清。
  - `(async)` 加在 `async fn` 上多余：`commands_are_async.rs` 要求每个命令都带这个标记。

### T4 布局

- **计划没写的坑**：布局测试要求每个场景都真的超长。分支名在框里单行省略，靠长名字撑不出滚动。
  新场景改成「远程已经没有」说明 + worktree 说明 + 未合并警告，再点删除失败、长错误撑满框，
  这是说明文字把错误往下挤的最坏形态。
- 给场景加了 `clickFirst`（先点某个元素、等某段文字出现再量），因为这条说明只在勾选之后出现。

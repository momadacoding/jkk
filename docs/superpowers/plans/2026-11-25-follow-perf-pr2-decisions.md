# 跟进提速 PR2 实现日志

计划：`2026-11-25-follow-perf-pr2.md`。

## 进度总账

| Task | 内容 | 状态 | 产出 | 验证 |
|---|---|---|---|---|
| 1 | 拆出按给定节点判定 | 完成 | `svn_sync::judge_nodes`、`judge_at` | 由 Task 2 的测试与现有 `svn_sync*` 覆盖 |
| 2 | 跟进带回判定 | 完成 | `svn_follow::{Recheck, SyncAfterFollow, Followed, sync_after, follow_with_sync, follow_auto_with_sync}`；命令 `svn_follow`、`svn_follow_auto` 返回 `Followed<_>` | `tests/svn_follow_carry_sync.rs` 5 条；把判定改成只用一半节点时 4 条失败 |
| 3 | 前端用带回的判定 | 完成 | `repoStore.{svnSyncMark, applyFollowedSync}`、`svnSync.canApplyFollowedSync`、`createAfterSync(checked)`、`svnSyncFlow.afterFollow(sync, since)` | `svnSyncFlow.test.ts` 新增 5 条、`svnHeadMove.test.ts` 新增 5 条；`pnpm tauri dev` 实测见 Task 4 |
| 4 | 门槛、review、PR | 完成 | 独立 review 7 条全部修复 | review 前：`pnpm build`、`pnpm test`、`cargo fmt --check`、`cargo clippy --all-targets`、`cargo test`（lib 188 条 + 111 个集成测试二进制分四批）全过。修复后：`pnpm build`、`pnpm test`（104 文件 1924 条）、fmt、clippy 重跑；Rust 改动只在 `svn_follow.rs`，重跑了跟进相关的 16 个测试二进制（含 `svn_follow` 144 条、`svn_follow_auto` 43 条、`svn_follow_carry_sync` 6 条），全过 |

## 决策条目

### 计划之前：与分析报告不同的地方

- **偏离（返回结构）**：报告说「改 Tauri 命令返回结构」。没改结局枚举本身：`FollowOutcome::Done` 加字段会让
  七十多处 `Done { report }` 模式全部要改。改在命令层包一层 `Followed { #[serde(flatten)] outcome, sync }`，
  JSON 只多一个 `sync` 字段，前端按可选字段读，旧形状照常认。
- **计划没写的坑（wc.db 戳）**：报告说「这期间 TortoiseSVN 的改动由 wc.db 轮询兜底」。这只在前端记下的戳早于
  R3c 读节点时成立。后端在 R3c 之前读戳要么只能 stat 缓存里的位置，要么要重新枚举（贵，且枚举本身就是那一遍
  扫描，读戳会落在扫描之后）。所以干脆不记：留着跟进前那次检测的戳，它一定早于 R3c，兜底照样成立；代价最多是
  R3c 之前 wc.db 变过时多跑一轮检测。
- **计划没写的坑（HEAD）**：不记 HEAD 的话，后端跟进后补发的 `repo-changed` 读到新 HEAD，会强制跑一轮第二档，
  省下的那遍 `svn status` 又回来了，只是挪到了后台。所以要记，且要记后端判定之前读的那个：前端返回后再读，
  中间被外部工具切了分支就会把没校验过的 HEAD 记成校验过。
- **范围（冲突结局）**：点「跟进」冲突（`NeedsManual`，基线已发布）也带回：之后紧跟着手动跟进的确认框，
  这时的判定（分支落后）同样能省一遍扫描。分支跟进入口上的冲突（没有录制、没有 R3c）不带。
- **范围（续跟）**：`svn_follow_continue`（崩溃后续跟）不带：恢复路径没有这次的 R3c 节点，照原来复查。

### Task 2：后端

- **计划没写的坑（测试的真实性）**：等价测试比的是「带回的」和「紧接着 `check_sync`」，两边都走 `judge_at`，
  光比相等可能是同一个错误两边都有。所以临时把判定改成只用一半节点，确认 5 条里 4 条失败（剩下那条测的是
  不带的情形），再改回来。
- **为什么判定放在放回补丁之后**：放回（`with_restore`）写磁盘上的文件内容，判定里「外部新增」要查磁盘上
  是不是文件；放回不改 svn 节点的 revision / item，所以 R3c 读的节点照样适用。

### Task 4：`pnpm tauri dev` 实测（2026-11-25）

- 做法：`scripts/e2e-svn-fixture.ps1` 建临时 fixture，`pnpm tauri dev --config` 换 identifier 隔离，核对
  `app.exe` 在本 worktree 的 `target\debug` 下；经 CDP 调 `svn_init_hybrid`、`save_session` 打开仓库，
  包 `window.fetch` 记每个 invoke。在 wc1 里改 `Code/A.cs` 后 `svn commit`（同 TortoiseSVN 提交）。
- 结果：`svn_sync_status`（检测到新内容，210 ms）→ `svn_follow_auto`（1488 ms）→ 之后 25 秒里**没有**
  `svn_sync_status`：补发的 `repo-changed` 只读了 HEAD 和 wc.db 戳，2 秒后的尾随复查也只读了戳。横幅撤掉，
  「已自动跟进」提示挂出。直接 invoke `svn_follow`（没东西可跟）返回 `kind: 'refused'`、没有 `sync`。
- 没测到的：点「跟进」完整发布、冲突两种结局在真实窗口里的表现（由 `tests/svn_follow_carry_sync.rs` 与
  `svnSyncFlow.test.ts` 覆盖）；公司机器上的实际省时要看现场 `--perf`。
- 收尾：停掉本 worktree 的 `app.exe`，删掉 fixture 与两处 identifier 目录。
- review 之后改了前端的写回条件（下一节），没有再起一次 `pnpm tauri dev`：Tauri 命令签名与返回形状没变，
  改动是 `canApplyFollowedSync` 多了两种「不写、照原来复查」的情形，由 vitest 覆盖。

### Task 4：独立 review（`/code-review high`，7 条，全部修复）

- **计划的错误（重要 1）**：计划只写了「领号顶掉更早开跑的检测」，漏了「命令跑的这几秒里已经**写回**的检测」。
  它可能读得比 R3c 晚、还记下了那一刻的 wc.db 戳；带回的旧结论盖上去，之后比戳判没变，横幅一直不纠正。
  改成：命令发起前取号（`svnSyncMark`，第二档写回次数），期间有写回就不用带回的判定，照原来复查。
- **计划的错误（重要 2）**：原来按「别的 HEAD 在排队就不记 HEAD」处理，但写回本身已经领号把那一轮顶掉了，
  它的事件用掉、没人再替那个 HEAD 校验。改成：别的 HEAD 在排队时整份不写。
- **计划没写的坑（重要 3）**：「戳不记、留着跟进前的」依赖跟进前确实记过戳。戳是 `null`（比如触发跟进的那轮
  读戳失败）时，之后没人补记，失焦快照在戳为 `null` 时不比戳（`markOnBlur`），会把快照写在没核对过的 InSync
  上。改成：戳没记过就不写。三条判断合成纯函数 `canApplyFollowedSync`，逐条有用例。
- **小问题**：`RepoTab` 里 `sceneAfterFollow` 的注释改成新行为；续跟（`resume_follow` → `follow`）不再在锁内
  白算一次判定（`follow` / `follow_auto` 走 `carry = false`）；冲突时第二次点「跟进」那条测试的注释写错了
  原因（它在准备完就返回，不是 `nodes: None`），改正，并补一条分支跟进入口**完成**时不带的用例；本日志补了
  「计划的错误」「为什么这么改是安全的」两类。

### 为什么这么改是安全的

- 不碰任何 git / SVN 写操作：后端只在发布完成之后、持锁时多读一次（HEAD、基线树、配置、磁盘上是不是文件、
  `diff-tree`），读出错就不带。
- 判定与 `check_sync` 是同一个函数（`judge_at`），只把「读节点」换成 R3c 读的那份；跟进不写 SVN 工作副本，
  放回补丁只改文件内容、不改节点的 revision / item。等价测试覆盖整次、部分、冲突三种，且确认过测试能失败。
- 前端最坏的情形是「不写、照原来复查」，与改动前相同；会写的情形都满足「记下的观测值不晚于判定的输入」：
  戳不记（留着更早的），HEAD 记后端判定之前读的那个。

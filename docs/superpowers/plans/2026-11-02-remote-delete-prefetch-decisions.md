# 删远程分支提速（护栏查询预取）+ SVN 耗时排查 — 实现日志

> 没有对应的计划文件：方案 A 在对话中定下（用户表明要删远程时后台预取远程默认分支，点确认时复用结果），
> 用户附加要求「护栏不能失效」。按 AGENTS.md §2.1 落库进度与决策。

## 进度总账

| Task | 内容 | 状态 | 产出文件 | 验证 |
|---|---|---|---|---|
| T1 | 后端：一次性预取 + 两个 Tauri 命令，护栏改走预取 | 完成 | `src-tauri/src/engine/remote_head_cache.rs`、`engine/branch.rs`、`commands/ops.rs`、`lib.rs`、`tests/remote_branch.rs` | 见下方「最终验证」 |
| T2 | 前端：侧边栏入口在确认框打开时、删本地分支弹框在勾选删远程时触发预取 | 完成 | `src/store/repoStore.ts`、`components/DeleteBranchDialog.tsx`、`components/BranchView.tsx`、对应 vitest | 见下方「最终验证」 |
| T3 | SVN 耗时实测 | 完成（只排查，未改），跟进事项已记 issue | 本文件「T3」节 | 一次性计时探针，已删除 |
| T4 | 修两个改动前就有的护栏缺陷：问 push 目标；删 worktree 前预检 | 完成 | `engine/branch.rs`、`engine/remote_head_cache.rs`、`commands/ops.rs`、`lib.rs`、`engine/deleteBranchTargets.ts`、`components/RepoTab.tsx`、对应测试 | 见下方「最终验证」 |

## 决策条目

### 背景实测（动手前）

- 每次 SSH 连接 ~3.3s：`ssh.github.com:443`，TCP 15ms，但 SSH 协议每往返 ~310ms × 约 9 次往返。
  系统 git、Fork 自带 git 2.50.1、Windows OpenSSH 三者无差别——不是本项目代码问题。
- 本项目删远程 = `ls-remote --symref`（护栏）+ `push --delete` 两次连接 ~7.5s；只 push ~3.7s。

### T1 后端

**偏离方案**
- 方案 A 原文只说「缓存」。最终实现是**一次性预取**：护栏取走即消失，护栏自己查到的不存。见下方「设计演变」。
- 预取由前端发起；后端不在 `branch_delete_info` 里偷偷起线程预取，否则只删本地分支也会走网络。

**计划的错误**
- 没有计划文件。方案阶段的两个判断是错的，修正如下，避免下一个人再踩：
  - 「缓存 30 秒」：缓存语义让一份答案服务多次删除，窗口超出「读一个弹框」的时间。正确语义是一次性。
  - 「在途查询失败时沿用失败，与改动前等价」：不等价。预取可能发生在用户重连之前，而此刻 push 已能到达远程，
    没有本地 ref 可兜底时等于放行默认分支。

**设计演变（review 驱动，四轮）**

| 版本 | 做法 | 被否的原因 |
|---|---|---|
| 1 | 带 TTL 的缓存，查询全程持锁 | 远程连不上时等待者串行重复超时 |
| 2 | 等待者沿用失败结果 | 重连后放行默认分支（高危） |
| 3 | 不持锁 + 限时等待 + 卡死后替换 | 卡死查询累积占满 tokio 线程；旧查询绕过 TTL（均高危） |
| 4 | 查询编号、只认亲自等到的那次 | 一份答案仍服务多次删除；编号豁免实为死代码 |
| 5（最终） | 一次性预取，护栏不写缓存，预取不替换在途查询 | —— |

版本 5 去掉了查询编号、`newer` 检查和替换逻辑：护栏不写缓存、预取不替换在途查询，同一 key 永远最多一个在途查询，
过期结果由 TTL 硬判，不存在「旧查询盖新结果」的状态。

**为什么这么改是安全的（护栏不失效的论证）**
- 结果一次性：取走即消失，护栏自己的答案不存（`a_prefetched_result_serves_only_one_delete`、
  `the_guards_own_answer_is_not_kept`）。一份答案只服务一次删除。
- 失败不留痕：护栏等到失败或没有结果都自己问（`a_waiter_does_not_trust_the_failure_of_the_prefetch_it_waited_for`、
  `a_failed_prefetch_does_not_disarm_the_guard`）。代价只是慢：断网时护栏在预取失败后还要自己超时一次。
- TTL 没有例外（`an_expired_result_is_not_used`）。
- 先等在途查询再看结果，刷新优先于旧结果（`a_waiter_prefers_the_refresh_in_flight_over_an_aged_result`）。
- 等待有上限（`MAX_WAIT` 15s，小于 TTL），查询期间不持锁（`a_stuck_prefetch_cannot_block_the_guard`）。
  等不到就自己问，护栏结论与没有预取时相同。
- 护栏**确实**用了预取：`the_guard_uses_the_prefetch_instead_of_asking_the_remote_again`、
  `prefetching_by_remote_ref_fills_the_cache_the_guard_reads` 预取后把远程目录挪走，只有用上预取才会报 `protected_branch`；
  变异验证（预取改成 no-op）两条都变红（`git_cli`）。
- key = 仓库路径 + 远程名 + 地址（T4 起是 push 目标的地址）。集成测试 `changing_the_remote_url_does_not_reuse_a_stale_prefetch`；
  变异验证：把 URL 从 key 里去掉，该用例变红（护栏放行，只靠裸仓库自身的 `denyDeleteCurrent` 挡住，错误码变成 `git_cli`）。
- 默认分支仍然先问远程、本地 ref 只兜底，顺序没变；普通分支照常能删（`an_ordinary_branch_is_deletable_after_a_prefetch`）。

**已知、接受的放宽**
- 「远程默认分支被改」的感知窗口从「ls-remote 到 push 之间的几秒」放宽到「最多 30s，且只限这一次删除」。
- 预取卡死时这次删除最多多等 15s，再加护栏自己一次查询。
- 没有 symbolic HEAD 的远程（空仓库、游离 HEAD）查询结果等同失败，后台多一次连接，用户等待时间与改动前相同。
- HTTPS 远程没有缓存凭据时，预取弹的 GCM 窗口 15s 没人理，护栏再发一次查询可能弹第二个窗口。SSH agent 审批同理。
- `run_git` 没有超时：半开连接下一次预取可能永远不返回，留下一个线程和 git/ssh 进程，直到应用退出。
  每个 key 最多一个，不累积，也不占 tokio 工作线程；这个 key 之后退化成没有预取。过去只有确认删除才会撞上，现在打开弹框也可能。
- query panic 时 `in_flight` 不会清除：最坏退化成没有预取。`run_git` 不会 panic，不加 Drop 守卫。

**不采纳**
- 给 ls-remote 加 kill 超时：对护栏自己的查询，到时杀进程会把慢但能成功的查询变成失败，退回本地 ref 反而可能放行。
  只给预取加是安全的，但要写跨平台杀进程树（git → ssh）的新封装，放到遗留项。

**改动前就有的缺陷（用户决定修，见 T4）**
- 护栏问 fetch URL，`push --delete` 走 pushurl：两者指向不同服务器时，护栏看不到 push 目标的默认分支。
- `git remote add` 的远程在弹框里 `remoteProtected` 恒为 false：勾上删远程后，worktree 先被删、护栏才报错，形成部分删除。

**计划没写的坑**
- `cargo fmt` 会改写新测试的换行，`fmt --check` 首轮不过属正常。
- 用 `sed -i` 或 tauri CLI 改过的文件工作区会从 CRLF 变成 LF，`git status` 出现无内容差异的 `M`；用
  `sed 's/\r\?$/\r/'` 恢复，没有用 `git checkout --`（AGENTS.md 把它列为破坏性命令）。

### T2 前端

**偏离方案**
- `DeleteBranchDialog`（删本地分支、可选连远程）**在勾上「Also delete remote branch」时**才预取，
  而不是弹框一打开就预取：只删本地的人不该因此走网络（SSH agent 审批、GCM 凭据框可能弹出）。
  代价是重叠时间变短（勾选 → 点 Delete 之间那一两秒）。侧边栏「Delete remote branch…」本身就表明要删远程，
  确认框打开即预取。
- 条件直接调用 `deleteBranchTargets(..., { alsoRemote, alsoWorktree: false }).remote`，不手抄。
  effect 依赖算出来的远程名：PR 列表晚到的重查不再发一趟；取消再勾选会再发，上次过期或失败时靠它刷新，重复由后端去重。
  门控有判别力的用例是「勾着的时候重查发现受保护」；变异验证（改成只看勾没勾）该用例变红。

**计划的错误**
- 无计划文件；方案阶段写的「两个入口都在确认框打开时预取」对删本地分支弹框是错的，见上。

**计划没写的坑**
- JSX 属性字符串不处理转义：测试里 `repoPath="C:\\repo"` 实际是两个反斜杠，断言要用 `String.raw`。
- `pnpm tauri dev` 从 `.claude/worktrees/` 这种上层目录启动时，tauri CLI 会往上找到**主仓库**的 `src-tauri` 并跑 master 代码，
  不报错。第一轮验证就中了：`Command ... not found` 不是代码缺陷，两条护栏 PASS 也是 master 的结果，全部作废。
  起 dev 必须用绝对路径切到 worktree，并核对 `app.exe` 的路径。
- 开发实例与安装版共用 `%APPDATA%\com.momadacoding.jkk\session.json`（启动时会改 `activeTab`）和默认 WebView2 配置目录。
  前者验证前备份、结束后恢复；后者用 `WEBVIEW2_USER_DATA_FOLDER` 隔开。

**为什么这么改是安全的**
- 前端只发只读预取，真正的删除路径和确认流程没有变；预取失败只 `console.warn`，护栏不依赖它。

### 独立 review 处理汇总（`/code-review high`，四轮）

**第一轮**

| # | 发现 | 处理 |
|---|---|---|
| 1 | 失败不缓存 + 查询全程持锁，等待者串行重复超时 | 已修（修法在第二轮被否） |
| 2 | 不勾远程也预取，可能弹凭据/agent 审批 | 已修：勾选后才预取 |
| 3 | 排队的预取占住工作线程 | 已修：预取绝不等待 |
| 4 | 测试注释夸大；没有测试证明缓存被命中 | 已修：新增「挪走远程」用例并做变异验证 |
| 5 | 等待超过 TTL 后又重查 | 已修：`MAX_WAIT` 小于 TTL |
| 6 | effect 依赖缺 `capabilities.remotes` | 已修 |
| 7 | 条件手抄 `deleteBranchTargets` | 已修：直接调用 |
| 8 | 删远程路径打开仓库 3 次 | 不改：本地 open 毫秒级，相对 3s 网络可忽略 |
| 9 | 全局表只增不减 | 不改：每个仓库×远程一条小记录 |
| 10 | 两个命令、两个包装 | 不改：`prefetch_remote_default` 用确切的远程名，比从 `upstream` 反拆更准 |

**第二轮**

| # | 发现 | 处理 |
|---|---|---|
| 1 | 护栏沿用预取的失败，重连后放行默认分支 | **已修（高危）** |
| 2 | 卡死的预取拖死之后的删除 | **已修（高危）**：限时等待，查询不持锁 |
| 3 | 护栏问 fetch URL、push 走 pushurl | 不改：改动前就有，注释已改正，已告知用户 |
| 4 | 无 symbolic HEAD 的远程不命中 | 不改：见「已知、接受的放宽」 |
| 5 | 预取结果可回填勾选框 | 不改：新功能，已告知用户 |
| 6 | 按 ref 预取的测试换成空操作也过（AGENTS §8 阻断） | 已修，变异验证变红 |
| 7 | 注释停留在旧设计 | 已修 |
| 8 | 前端一次性标记挡住重新勾选时的刷新 | 已修：去掉标记，后端去重 |
| 9 | 并发测试靠 sleep 排序 | 已修：改用 channel 与等待者计数 |
| 10 | 两个前端用例走不到声称的条件 | 已修：改名实相符，第四轮再补有判别力的用例 |

**第三轮**

| # | 发现 | 处理 |
|---|---|---|
| 1 | 卡死查询累积占满 tokio 工作线程 | **已修（高危）**：预取绝不替换在途查询 |
| 2 | 旧查询绕过 TTL | **已修（高危）**：TTL 无例外 |
| 3 | `MAX_WAIT` 注释低估最坏情况 | 已修 |
| 4 | 凭据窗口没人理时可能弹第二个 | 不改：见「已知、接受的放宽」 |
| 5 | 建议给 ls-remote 加 kill 超时 | 不采纳：见上 |
| 6 | 结果快过期时预取被跳过 | 已修：超过 TTL 一半就刷新 |
| 7 | 新 import 未用 `@/` 别名 | 已修 |
| 8 | 测试 fixture 代码重复 | 已修 |

**第四轮**

| # | 发现 | 处理 |
|---|---|---|
| 1 | 预取可无限挂起 | 部分：每 key 最多一个，见「已知、接受的放宽」 |
| 2 | 一份答案服务多次删除 | **已修**：一次性预取 |
| 3 | 护栏不等在途刷新 | 已修：先等再取 |
| 4 | 多值 URL 下 libgit2 与 CLI 不一致 | 不改：未复现，见「已知、接受的放宽」 |
| 5 | query panic 不清 `in_flight` | 不改：见「已知、接受的放宽」 |
| 6 | 编号豁免是死代码 | 已修：删除 |
| 7 | 负向用例走不到门控 | 已修：新增有判别力的用例并变异验证 |
| 8 | 日志缺「计划的错误」类别 | 已修 |
| 9 | 测试 helper panic 时目录残留 | 已修：Drop 恢复 |
| 10 | helper 重复 | 已修 |

### T3 SVN 耗时排查（只测量，未修改）

**偏离方案 / 计划的错误**：无计划。用户怀疑 SVN 也是网络问题，实测否定：本机 SVN 测试全部对着 `file://` 本地仓库，不经过网络。

**测量**（`%TEMP%\perf15k_*` fixture 的**副本**，31k 文件，debug 构建，第二轮）：
- 单次 svn 进程 ~30ms；`svn status` 热 ~180ms、冷 1.8s。svn 本身不慢。

  | 命令 | 总耗时 | 大头 |
  |---|---|---|
  | `svn_check_baseline` | 20.7s（首轮 45s） | `git add -A -- Code/` 15.2s + `write-tree` 3.5s |
  | `svn_sync_in_preview` | 12.9s | `git add -A -- Code/` 10.3s |
  | `svn_sync_status` | 1.4s | 两次 `svn status --xml` |
  | 其余 | < 1.2s | |

**根因**：`check_current_baseline`（`svn_init.rs:1663`）与 `precheck_include_set`（`svn_sync_check.rs:393`）用 `TempIndex::empty`
建候选索引，没有 stat 缓存，`git add -A` 每次把白名单里所有文件读出来重新哈希。这是 git.exe 自己的耗时，release 构建一样慢。

实测（fixture 副本，git CLI）：空索引 `add` 15,077ms；拷贝真实索引（带 stat）再 `add` 434ms，**两者 `write-tree` 得到同一个 tree OID**。

跟进路径已在 2026-11-01 用 `TempIndex::from_tree_keeping_stat` 修过同一个问题（9.2s → 0.38s），这两处没跟上。

**为什么没改**：用户要求测试，未要求修改。这两处是核对路径，spec §6.3.1 对「从空索引起步」有要求：从真实索引起步时白名单外的条目会留在索引里、
影响 tree，需要先确认语义再动。

**计划没写的坑**：探针写在 `src-tauri/tests/zz_debug_svnperf.rs` 与 `[DEBUG-svnperf]` 日志里，已全部删除。

SVN 的跟进事项记在 GitHub issue「perf(svn): 基线体检与同步预检仍从空索引 git add -A」，逐处列了 `TempIndex::empty` 的调用点和不能直接替换的原因。

### T4 修两个改动前就有的护栏缺陷

用户在收到第一版汇报后决定修。放进同一个 PR、单独一个提交：两处都改这个 PR 新写的缓存 key 和护栏入口，分开走要处理叠 PR 和 squash 合并。

**实测的 git 行为**（都固化成了 `tests/remote_branch.rs` 的用例）
- `git remote get-url --push --all` 已经把 pushurl、多值、`pushInsteadOf` 算进去了，和 push 同一个口径，所以读它而不是读 libgit2。
- `git push` 会推到**所有** pushurl；没配 pushurl 而有多个 url 时同样推到所有 url，`ls-remote <name>` 却只问第一个。
- `get-url --push` 和 `ls-remote` 都接受 `--` 分隔符。
- fixture 的 bare 仓库自己会拒绝删当前分支（`denyDeleteCurrent`），测试要看错误码，`git_cli` 说明护栏放行了。

**偏离最初的做法**
- 第一版永远拿 URL 去 `ls-remote`。自查发现这是回归：`remote.<name>.proxy`、`uploadpack` 这类配置只认名字，push 按名字走得通，问 URL 却可能失败，
  护栏就无依据放行。改成 `PushTarget { url, ask }`：只有一个目标且和 fetch 地址相同时按名字问，和改动前的调用一致；否则才问 URL。url 始终进缓存 key。
- 预检答案不能走 `take_or_query`：取走的话紧接着的删除要再握一次手，提速白做。新增 `peek_or_query`，看过不取走。
- 删除顺序从 `RepoTab` 抽到 `runBranchDeletion`：`RepoTab` 没有测试文件，顺序是这次修复的核心，留在那里没有回归保护。
- 用户没要求修「部分失败后重试」，review 指出它和部分删除是同一件事，确认框原地重试时会卡死，所以一并修了。

**为什么这么改是安全的**
- 预检不放宽任何一条：答案仍只被一次删除取走（`the_preflight_does_not_turn_the_answer_into_a_lasting_cache`），
  TTL 仍按查询发出时刻算，预检不续命（`a_peek_does_not_reset_the_age_of_a_result`），失败不留痕（`a_failed_peek_leaves_nothing_behind`）。
- 只在 worktree 和远程都要删时预检：没有 worktree 时删远程本身就是第一步，护栏先拦，预检只会白跑。
- 预检问不到远程时放行，与护栏口径一致。这种情况 worktree 仍会先删、push 再失败，部分删除的窗口还在，但那是网络失败，不是护栏拒绝，改动前就如此。
- fetch 侧的默认分支在 push 目标上是普通分支时不拦：被删的不是 fetch 侧（`the_fetch_sides_default_is_deletable_on_a_push_target_that_does_not_use_it`）。
- 错误信息不带 URL：URL 里可能有凭据。

**变异验证**
- 改成永远问 URL：两条「按名字问」的用例变红。
- 多值时按名字问：`every_url_is_asked_when_a_remote_has_several` 变红。
- 修复前这组新用例 8 条红、前端 6 条红，红的原因都对得上。

**已知、接受的边界**
- 多个 push 目标同时预取；同时发出的预取共用一个 15s 截止时刻，护栏未命中时逐个查询，所以最坏是 15s 加 N 次串行握手。
- 同时配了 `pushInsteadOf` 和 `insteadOf` 且链式命中时，`ls-remote <url>` 会多改写一次，问到的地址和 push 不同。这种配置很少见，未复现。
- 问不到 push 目标时退回本地 `refs/remotes/<remote>/HEAD`，它记的是 fetch 侧，宁可多拦。
- UI 的 `remoteProtected` 仍只看本地 ref，默认分支在弹框里不会提前灰掉，要点 Delete 才报错。把预取结果回填给勾选框属于新功能。

**T4 的独立 review（`/code-review high`）处理**

| # | 发现 | 处理 |
|---|---|---|
| 1 | 部分删除后重试从 worktree 重新开始，永远走不到远程和本地 | 已修。第一版做法是失败后重查，被下两轮 review 推翻，最终做法见第三轮 |
| 2 | pushurl 目标按 URL 问，丢掉 `remote.<name>.*`，配了 proxy 时查询可能失败而 push 成功 | 接受：比改动前好（当时根本不问 push 目标）。pushurl 加按远程配的 proxy 才会撞上；问不到时退回本地 ref |
| 3 | 网络预检排在本地脏检查之前，脏 worktree 的秒拒变成要等网络 | 接受：预取通常已经回来，预检不走网络；卡死时最多多等 15s 加一次查询。要改须加只读的脏检查命令，放到后续 |
| 4 | 部分删除后侧边栏不刷新 | 已修：worktree 这一步之后立刻刷新。真实窗口场景 C |
| 5 | 预取晚回来会盖掉预检的新答案 | 已修：`State::remember` 按发出时刻取新，`a_slow_prefetch_does_not_overwrite_a_newer_result` |
| 6 | 多目标预取串行，后面的目标总要多握手 | 已修：同时问 |
| 7 | 序列中止时预检答案留到 TTL | 接受：受 30s 约束，和用户取消确认框时残留的预取同一性质，仍只被一次删除取走 |
| 8 | 改成单一后端命令就不需要跨 IPC 带答案 | 不采纳：要把 worktree 回收站和标签关闭的流程搬进后端，改动面远大于这个缺陷；预检解决的是护栏拒绝，不是所有部分删除 |
| 9 | 组件直接 `invoke` 新命令 | 已修：`checkRemoteDeletable` 进 `repoStore.ts` |
| 10 | 最终验证没覆盖 T4 | 已修：见下 |
| 附 | 缺空输入测试 | 已补 `the_preflight_survives_empty_unknown_and_option_like_remote_names` |
| 附 | 一次删除最多多 6 次 `git remote get-url` | 接受：本地命令约 30ms，比一次握手小两个量级 |

**对上面这批修复的第二轮 review（`/code-review high`）处理**

| # | 发现 | 处理 |
|---|---|---|
| 1 | **上一轮的修复引入的回归**：失败后重查会改写远程目标，`find_remote_branch` 扫到别的远程的同名分支，重试就删了用户没选过的分支 | 先写失败测试复现。第一版改成固定目标，被第三轮推翻 |
| 2 | 失败后重查回来之前 Delete 可点，用的是旧信息 | 第三轮去掉重查后不再存在 |
| 3 | 预检只防护栏拒绝，防不了 push 自身失败 | 接受：注释已写明。远程失效引用（分支已被自动删除）和断网时 worktree 仍会先删，这和改动前一样，确认框留在原地，取消勾选远程即可继续 |
| 4 | 测试注释声称证明了 `--` 的作用，去掉 `--` 也过（AGENTS §8 虚假信心） | 已修：删掉这句，测试只声明它实际验证的「不崩、不删」 |
| 5 | 护栏和预检自己的查询不登记 `in_flight`，手快时多一次握手，并留下一份孤儿答案 | 接受：偏慢不偏松，答案受 TTL 约束 |
| 6 | 护栏未命中时逐个查询，重复 URL 不去重；日志的最坏情况写错 | 查询不改（慢路径，配多个 pushurl 的少见场景）；日志已改正 |
| 7 | T4 的命令结果没落库 | 已修：写进下方「最终验证」 |
| 8 | TTL 判断有两份 | 已修：收到 `Prefetched::is_within` |
| 9 | 单目标也多起一个线程 | 接受：预取线程本来就短命，不值得分支 |

**第三轮 review（`/code-review high`，只看确认框）处理**

这一轮指出了根因：重试不知道哪些步骤已经成功，靠重查去推断，每种部分失败都要单独打补丁。固定目标又造成新的死路：远程那条已经删掉，还反复 `push --delete`，永远走不到本地分支。

决定撤掉重查、固定目标和相关状态，换成单一机制：`runBranchDeletion` 每做完一步调 `onStepDone`，确认框用 `withoutCompletedSteps` 把做完的目标从信息里去掉。
重试自然只走剩下的步骤；PR 列表晚到的重查也改不回来，因为过滤发生在每次渲染时。

| # | 发现 | 处理 |
|---|---|---|
| 1、2 | 固定目标导致重复删已删掉的远程分支；测试把这个行为钉死了 | 已修：见上。测试按真实步骤重写，模拟的 `onConfirm` 会上报进度 |
| 3、4 | 重查失败后放开、未选远程时也固定 | 随重查一起删除 |
| 5 | 全部删完后刷新失败，被确认框当成删除失败 | 已修：`.catch(reportError)` |
| 6 | 失败后焦点没回到 Delete | 不改：改动前就有，Tab 一下即可 |
| 7 | 进度文案读 `alsoRemote`，可能在纯本地删除时说「正在联系远程」 | 已修：改读算出来的目标 |
| 8 | 根因：进度靠事后推断 | 已修：见上 |
| 9 | 每次失败都全量重查 | 随重查一起删除 |
| 10 | 组件直接 `invoke('branch_delete_info')` | 不改：改动前就有，这轮已不再新增调用 |

**第四轮 review（`/code-review high`，确认框、删除序列、RepoTab）处理**

收尾标准：只处理会错删、会卡死，或由这次改动引入的问题；其余记录在此。

| # | 发现 | 处理 |
|---|---|---|
| 1 | 不勾 worktree 只勾远程时，远程先删，本地必定被 `used by worktree` 拒绝 | 不改：改动前就有。确认框已提示；重试时进度机制保证不重复删。**留给用户决定** |
| 2 | 预检问不到远程、或分支已不在时仍放行 | 接受：同第二轮第 3 条 |
| 3 | `remote ref does not exist` 算硬失败：GitHub 合并时自动删了分支、本地 `origin/x` 没清理时，序列走不到本地分支 | 不改：改动前就有，取消勾选远程即可继续。**留给用户决定**：可把它当成功并顺手清理跟踪引用 |
| 4 | 做完的步骤悄悄消失，用户不知道已经删了什么 | 已修：确认框显示「已经删除：…」 |
| 5 | `worktree_dirty` 提示可以继续，但这个弹框没有继续的入口 | 不改：改动前就有 |
| 6 | 网络预检排在本地脏检查之前 | 接受：同第一轮第 3 条 |
| 7 | 删 worktree 时进度文案也说「正在联系远程」 | 不改：文案问题，改动前就有 |
| 8 | 删除序列其余步骤仍在组件里直接 `invoke` | 不改：改动前就有 |
| 9 | `targets` 的形状在三处声明 | 不改：改动前就有两处 |
| 10 | 侧边栏刷新挡住了进度上报 | 已修：不再等待 |

## 第五轮 review 的处理

| # | 发现 | 处理 |
|---|---|---|
| 1 | 护栏问 fetch URL、push 走 pushurl | 当时不改，用户决定后在 T4 修复 |
| 2 | 预取卡死时占住 tokio 工作线程 | 已修：命令立刻返回，查询放到独立线程 |
| 3 | worktree 先删、护栏后拦 | 当时不改，用户决定后在 T4 修复 |
| 4 | 等待上限每轮重算 | 已修：截止时刻只算一次 |
| 5 | 卡死测试没进超时等待（AGENTS §8 阻断） | 已修 |
| 6 | 注释声称失败重试会重发 | 已改注释 |
| 7 | `push_url` 进 key 没有实际作用 | 已删 |
| 8 | 最终验证是占位 | 已填写 |
| 9 | `MAX_WAIT < TTL` 未强制 | 已加编译期断言 |

第五轮之后的改动由 T4 的独立 review 覆盖。

## 最终验证

T1–T3 收尾时（提交 `088e4ea`）：

| 命令 | 结果 |
|---|---|
| `pnpm tauri dev` 真实窗口（核对过 `app.exe` 在 worktree） | 两个预取命令走通且 2ms 返回；`remote_branch` 被拒（`missing required key remoteBranch`）；预取后删默认分支 6ms 返回 `protected_branch`；普通分支可删 |

T4 的真实窗口验证（独立端口 5199/9333、独立 `identifier`，不碰其他实例；核对过 `app.exe` 在本 worktree）。用 CDP 发真实鼠标事件走「右键 → Delete… → 勾选 → Delete」，fixture 的远程用 `git remote add` 接上：

| 场景 | 结果 |
|---|---|
| 反例 | `check_remote_deletable` 缺参数被 Tauri 拒绝（`missing required key branch`） |
| A：默认分支 + worktree + 删远程 | 勾选框可勾；确认框 267ms 报 `protected_branch`；worktree、远程、本地分支都没动 |
| B：普通分支 + worktree + 删远程 | 三样都删掉 |
| C：worktree 删掉后 push 失败（`pre-receive` 钩子拒绝一次） | 确认框报错；worktree 勾选框和侧边栏行都消失；重试后远程和本地都删掉 |

T4 最终代码上的命令门槛：

| 命令 | 结果 |
|---|---|
| `cargo fmt --check` | 通过 |
| `cargo clippy --all-targets` | 无告警 |
| `cargo test` | 75 组全部 ok，1538 通过，0 失败，1 ignored（既有用例） |
| `pnpm build` | 通过 |
| `pnpm test` | 1024 通过 |

过程中有两次异常，都不是代码问题：一次 `E0786 invalid metadata`，重新编译后消失；一次 `cargo test` 被我的清理命令杀掉（按路径匹配连带杀了测试进程），只跑到 62 组，结果作废后完整重跑。

**没有验证的**
- 真实网络下删除的端到端耗时。曾打算对无写权限的公开仓库发 `delete_remote_ref` 来测，被权限分类器以 `Git Destructive` 拦下，
  判断正确（不该用刚改过的护栏对真实远程发删除），已撤掉。预期收益来自少一次连接（约 3.5s），这是推算，不是实测。
- 删本地分支弹框是勾选后才预取，重叠时间短，这条路径上省不满一次握手。

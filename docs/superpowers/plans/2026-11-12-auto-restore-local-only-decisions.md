# 跟进完成后自动放回 Local-Only 补丁：实现日志

计划：`2026-11-12-auto-restore-local-only.md`

## 进度总账

| Task | 状态 | 产出 | 验证 |
|---|---|---|---|
| T1 | 完成 | `svn_restore_local_only.rs`：`restore_locked`（拆出拿锁）、`check_paths` 带出 Local-Only 提交、`RestoreOutcome.commits`、`RestoreSkip.retry` | `cargo test --test svn_restore_local_only` 20 过 |
| T2 | 完成 | `svn_follow.rs`：`auto_restorable`、`with_restore`（follow / follow_auto / resume_follow 串联）、`auto_blocker` 排除我的可放回文件并改原因行、`FollowReport.restore` | `svn_follow_auto.rs` 末尾 3 条新测试过；`svn_backend.rs` 钉住 `restore` 的 JSON 形状与旧 journal |
| T3 | 完成 | 待放回清单 `record_pending` / `pending_restores` / `dismiss_pending_restores` / `local_only_commits`；命令 `svn_pending_restores`、`svn_dismiss_pending_restores` | `svn_restore_local_only.rs` 里 4 条清单测试 |
| T4 | 完成 | `svnFollow.ts`：`restore` 类型、`restorePending` / `restored` 两节、`drift` 排除可放回、`safe` 文案、`describePendingRestores`；`FollowReportDialog` 按钮上方列「文件 ← 提交」；`BaselineBanner` 新横幅；`RepoTab` 接线；`repoStore` 两个 invoke | `pnpm build` 过；`pnpm test` 1594 过（+19）；`pnpm test:layout` 152 过 |
| T5 | 完成 | E2E `2026-11-01-svn-mixed-baseline-follow-e2e.md` 改写 9B、新增 9C、覆盖表；独立 review 一轮（1 高 3 中 5 低），修完后复核 | `pnpm tauri dev` 真实窗口经 CDP 跑完 9B（含反例）、9C（含同事改过、横幅放回 / 不放回）全过；两个新命令参数名正反例都验过 |

## 决策条目

### 总体

- **偏离前一版计划：推翻 PR 159 的「不自动写，做成按钮」。** 那一版的理由是跟进模块「全程不写磁盘」。实际依赖这条的是崩溃恢复与 journal：journal 存续期间磁盘不能被跟进改动。放回放在 journal 删除之后、同步锁释放之前，崩溃恢复看到的仍然是「没有 journal」或「journal 里的步骤都不写盘」。按钮方案在固定工作流里每次提交都要手点，关掉报告就没入口，而且报告还给了会丢补丁的建议（使用报告 2026-11-12）。模块头改成「journal 存续期间不写磁盘」，并列出这个例外。
- **偏离使用报告：只自动放回「SVN 上最后一次改它的是我」的文件。** 使用报告的 A 是全部可放回文件都自动写。review 指出：用户有意去掉补丁、还没提交时，`svn update` 带来别人对这个文件的改动，跟进后它也满足放回条件，自动写回等于撤销用户的准备。B 的放行用同一个集合（`auto_restorable`）。
- **偏离使用报告：C 不按当前状态推断，只认跟进记下的清单。** 使用报告的 C 是「『本地改动』里、被 Local-Only 提交改过、满足放回条件的文件」。用户提交 SVN 前去掉补丁时正好满足这些条件，横幅会在用户准备提交时冒出来让他放回。

### T1

- **为什么安全：写盘条件没变。** `restore_locked` 就是原来 `restore_local_only` 拿锁之后的那段，五条条件逐字保留，测试 15 条照旧过（只改了造现场的方式，见下）。
- **计划没写的坑：原有 11 条按钮测试依赖「跟进不写盘」。** 它们的现场是「我」提交的文件，现在跟进就会放回。改法：`report_case_for` 跟进完成后把磁盘改回 SVN 版本，模拟「自动放回没写成 / 用户又去掉了补丁」，按钮测的仍是磁盘缺补丁的现场；原来第一条拆成「跟进自动放回」与「按钮放回」两条。
- **`RestoreSkip.retry` 不序列化。** 只决定一条跳过要不要留在待放回清单里：被占用、暂存了别的版本、分支落后、不是我的 → 留；普通 commit 改过、磁盘有用户的编辑、已经是分支的版本 → 删。前端不需要它。

### T2

- **只在 `Done` 时放回。** `NeedsManual`（冲突）与 `Refused` 不碰；`Err`（动过 ref 之后失败）也不碰。`publish` / `publish_auto` 返回后同步锁还在 `follow` / `follow_auto` 的循环体里，`with_restore` 在锁内跑。
- **崩溃恢复（`resume_follow`）不自动写，只记清单。** 崩溃前那次的作者信息（`Who`）没进 journal，分不清哪些是我最后提交的。
- **放回出错不让跟进失败。** 前置条件不满足（git 正忙、索引被占）时这几个文件全部记成没放回、写上原因、进清单；清单写不进去只记日志。
- **计划没写的坑：fixture 里同事的改动挨着补丁会重放冲突。** 补丁加在第一行前面，同事改第一行时 merge-tree 冲突、跟进走 `NeedsManual`。测试里同事改最后一行。

### T3

- **计划的错误：横幅查询不能拿同步锁。** 计划写「拿得到锁时才删条目」。`commands/watch.rs` 把 `.git` 下同步锁文件的变化当成刷新信号，横幅查询跟着状态刷新跑，拿锁 → watcher 刷新 → 再查询，形成回路。改为：`pending_restores` 只读、不拿锁（`svn_lock::is_held` 时返回空）；修剪只在持锁的 `record_pending`（跟进结束）与 `restore_local_only`（按钮）里做。
- **清单文件放在 `.git/mygitclient/restore-pending.json`。** watcher 已经放过 `.git/mygitclient/` 下的变化（同失焦快照）。写临时文件再换名。读不了、内容坏了按空清单：它只是提醒。
- **按钮不往清单里加，只删。** 报告里的按钮点了没写成的文件，原本就在清单里（跟进时记的）。

### T4

- **`restorablePaths` 改了语义：只给还没放回的。** 报告按钮与清单都只该提供还缺补丁的文件。旧后端没有 `restore`，全部算没放回。
- **横幅只在 `onRestorePending` 接上、不在同步中途、是 SVN 仓库时挂。** 中途时后端本来就返回空，前端再挡一次。
- **自动跟进完成的提示不单独改。** 放回结果写进 `safe`（「工作区里只放回了 N 个文件的 Local-Only 补丁」），没放回的那一节是 attention，自动让提示变成要人看的颜色。

### 独立 review（opus subagent，按 AGENTS.md §13）

- **H1（高）计划没写的坑：后端 `overlap_files` 把整个 drift 并进 overlap。** 能放回的文件磁盘 ≠ tip，`matchesDisk` 恒为 false。前端把 `drift` 改成「去掉可放回的」之后，`overlapDiffers` 按新 `drift` 排除，可放回的文件全掉进了「和磁盘不一致」一节，那里的文案也建议以磁盘为准提交：缺陷 5 换了一节又出现，全部放回时自动跟进的提示也变成要人看。修：按原始 `r.drift` 排除。测试的 fixture 原先不带 `overlap`，不是后端的真实形状（虚假信心）；补上之后两条新测试先红后绿。
- **M2 偏离计划（已改回计划）：`restorable` 为空时不修剪清单。** 计划是「每次跟进结束」更新清单。现在没有可放回文件时也调用 `record_pending(&[], &[])` 修剪。
- **M1：修剪路径原来没有测试，一条测试的注释说测了。** 那条测试里的 `follow` 走的是 `Refused`，碰不到 `record_pending`。改注释，补两条：完成的跟进删掉永久失效的条目、保留暂时性的条目。用新加的只读 `pending_list` 读清单文件核对。
- **M3：崩溃恢复路径的原因写成了「不是你」。** 加 `held` 参数，恢复路径写「崩溃后续上的，分不清」。未补测试：要靠故障注入造崩溃现场，代价与收益不成比例；改动只是换了一个字符串常量。
- **L1：** 修剪核对失败时降级为只做加删，不让这次的条目丢掉。
- **L2：** 放回后没写成的原因改走错误条（`describeRestoreSkips`）。原来挂在横幅里，而「再等也不会满足」的文件会被移出清单、横幅随之消失，原因就看不到了。
- **L3：** `svn_follow` 命令的文档注释去掉「不写磁盘」。
- **L5：** 横幅查询加序号，只认最后一次的结果。
- **L4 没改代码，记为遗留风险**：见下。

### 真实窗口 E2E（2026-11-13）

- **怎么跑的：** `scripts/e2e-svn-fixture.ps1` 造 `%TEMP%` 下的一次性 fixture；`pnpm tauri dev --config` 换 identifier 隔离安装版配置，`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222`；Playwright `connectOverCDP` 派发 `blur` / `focus` 模拟切窗口、点横幅和报告里的按钮、读 DOM。核对过运行的是本 worktree 的 `target\debug\app.exe`。
- **结果：** 9B 主路径（报告「已放回」、磁盘第一行带补丁、`git status` 干净、`svn status` 是 `M`）、9B 反例（`mine` 不被覆盖，不进 Local-Only 两节，不进清单）、9C 主路径（使用报告的现场：自动跟进不被拦、直接放回）、9C 去掉补丁未提交时清单为空（横幅不误报）、同事改过的（原因行、报告「没放回」+ 按钮、关掉报告后横幅、「不放回」清掉横幅不动磁盘、「放回」写回）全部符合预期。`svn_pending_restores` 带 `repoPath`、`svn_dismiss_pending_restores` 带 `files` 都被 Tauri 拒绝（参数名缺了），正确的形状通过。横幅挂着时 10 秒内 0 次 IPC，没有刷新回路。
- **清单的错误（已改）：** ① 9B / 9C 在操作窗口里连着做 git commit 和 `SvnCommit`，失焦快照停在 commit 之前，跟进把普通 commit 当成没上 SVN 的照常重放，分支与磁盘一致、看不到放回。真实工作流里 commit 在 jkk 里做，切去 TortoiseSVN 时快照已包含它们。补上「先点回应用再回操作窗口」。② 9B 反例的文件实际出现在「和磁盘不一致」（overlap）一节，不是「分支与磁盘对不上」。③ 9C 同事那一步「再加一次补丁再 commit」在补丁已放回时是空 commit，`git commit` 会失败；改成只在磁盘上去掉，并直接用 `Colleague`（自带作者）。
- **计划没写的坑：** `file://` 仓库探测不到 SVN 用户名，没走场景 1 填用户名时所有提交都算别人的，放回一律不发生。清单里补了前提。

### 全量测试里的一条偶发失败

- `commands::watch::tests::stage_and_unstage_only_touch_the_index` 在 worktree 里第一次全量跑连续失败 3 次（unstage 时 fixture 的 `.git/hooks` 冒出一个目录事件）。基点 1ac8025 的 scratch clone 里跑过；之后在 worktree 里再跑 3 次全过。本 diff 没碰 `watch.rs` 与 stage / unstage。判断为与本改动无关的偶发失败，如实记录，未修。

### 遗留风险

- **L4：「我的」只看 `svn status -v` 的最后提交作者。** 钩子健康时别处用本机事件判定，这里没用；分支跟进入口（InSync）上「最后作者是我」可能是很久以前的提交，不再满足「自己刚提交过」的前提。按计划实现，暂不改。

- **E2E 用的是 CDP 派发的 `focus` / `blur` 和 `svn` 命令行，不是真的切窗口和 TortoiseSVN。** 钩子触发的那条路径（TortoiseSVN 提交完自动检测）没有走。
- 编辑器保存撞上写盘的毫秒级窗口，同 PR 159 decisions 第 30 行的论证，接受。
- 「我的」靠 `svn status -v` 的最后提交作者：在别的机器上以自己名义提交、本机又有意去掉了补丁，仍会被自动放回。极少见；泄漏检测兜底。
- 横幅查询跟着每次状态刷新跑：清单为空时只读一个不存在的文件；非空时跑一次 revwalk 与 `hash-object`。未实测大仓库耗时。

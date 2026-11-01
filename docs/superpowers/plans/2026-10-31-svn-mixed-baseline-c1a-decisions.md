# SVN 混合版本基线 C1a：实现日志

- 计划：`2026-10-31-svn-mixed-baseline-c1a.md`（同目录）
- 设计：`docs/superpowers/specs/2026-10-30-svn-mixed-revision-baseline-analysis.md`（下称「分析」）
- 分支：`worktree-svn-mixed-baseline-c1a`（基点 `deadb94`）
- 约定：需要用户拍板的地方先按保守做法绕过去，条目标【待决】，收尾时按用户决定回头修

## 进度总账

| Task | 内容 | 状态 | 产出文件 | 验证 |
|---|---|---|---|---|
| 0 | 实测分析 §11 中 C1a 相关的未知项 | 完成 | 本文件 Task 0 | E13–E21、W17–W28 |
| 1 | BaseMap 记账格式 | 完成（未 commit） | 新增 `engine/svn_basemap.rs`、`tests/svn_basemap.rs`；改 `engine/mod.rs`、`svn_baseline.rs`、`svn_sync_out.rs` | `cargo fmt --check` 通过、`cargo clippy --all-targets` 无告警；`svn_basemap` 5/5；`svn_baseline` 23、`svn_init` 78、`svn_sync_in` 73、`svn_sync_out` 50、`svn_graph_contract` 6 全过；全量 `cargo test` 0 失败（merge_drivers 有 1 个原本就 ignored） |
| 2 | 节点状态解析与映射计算 | 完成（未 commit） | 新增 `engine/svn_nodes.rs`、`tests/svn_nodes.rs`；改 `svn_xml.rs`（`NodeStatus`、`parse_node_status`）、`svn_sync.rs`（抽出 `status_targets`）、`engine/mod.rs` | `svn_nodes` 10/10；fmt、clippy 干净 |
| 3 | 文件级失同步判定 | 完成（未 commit） | 改 `svn_sync.rs`、`svn_sync_check.rs`、`src/engine/svnSync.ts`、`src/components/BaselineBanner.tsx`；测试 `tests/svn_sync.rs`、`svn_backend.rs`、`svn_init.rs`、`svn_server_set.rs`、`svn_sync_in.rs`、`svn_sync_out.rs`、`tests/vitest/svnSync.test.ts`、`BaselineBanner.test.tsx` | `cargo fmt --check` 通过、`cargo clippy --all-targets` 无告警；`cargo test --no-fail-fast` 69 个测试二进制 1165 过 / 0 失败 / 1 ignored（merge_drivers 原有）；`pnpm build` 通过；`pnpm test` 55 文件 833 过 |
| 4 | 分组体检、墓碑与服务器侧核对 | 完成（未 commit） | 新增 `engine/svn_tombstone.rs`（`server_head_rev`、`confirm_absent`、URL 编解码）；改 `svn_health.rs`（`check_baseline_map`、`HealthMap`、`plan_diffs`，`check_baseline` 成单值包装）、`svn_server_set.rs`（`audit_server_set_map`）、`svn_xml.rs`（`parse_excluded_nodes`）、`svn_sync.rs`（`read_baseline_map`）、`svn_sync_out_check.rs`（`remote_head_rev` 共用）、`engine/mod.rs`；测试 `tests/svn_health.rs` +14、`tests/svn_server_set.rs` +1 | `cargo fmt --check` 通过、`cargo clippy --all-targets` 无告警；`svn_health` 34/34、`svn_server_set` 12/12；`cargo test --no-fail-fast` 69 个测试二进制 1182 过 / 0 失败 / 1 ignored（merge_drivers 原有）；两次变异核对见 Task 4 小节；前端未改，未跑 `pnpm` |
| 5 | 录制泛化（重建、初始化、sync_in 接管、交付拒绝） | 完成（未 commit） | 改 `svn_preflight.rs`（`run_preflight_with`、`Preflight.nodes`、R1 冲突 / 写锁）、`svn_init.rs`（`Recorded`、`record_mixed`、init / rebuild / `check_current_baseline` / `init_preflight`、`RebuildReport.base`）、`svn_health.rs`（`check_baseline_map_with_nodes`）、`svn_baseline.rs`（`create_baseline_commit` / `publish_baseline` 收 `&BaseMap`）、`svn_sync.rs`（`read_baseline_rev` 返回 floor）、`svn_sync_check.rs`（任何 ExternallyAdvanced 接管）、`svn_sync_in.rs`（S7 混合走 `record_mixed`）、`svn_sync_out.rs`、`svn_sync_out_check.rs`（`sync_out_mixed_baseline`）、`src/engine/svnInit.ts`、`svnSyncIn.ts`；测试 `tests/svn_init.rs` +5（1 条改写）、`svn_sync_in.rs` +2、`svn_sync_check.rs` 改 1、`svn_sync_out_check.rs` +1、`svn_backend.rs`、`svn_baseline.rs`、`tests/vitest/svnInit.test.ts` +1、`svnSyncIn.test.ts` +1 | `cargo fmt --check` 通过、`cargo clippy --all-targets` 无告警；新测试先红（9 条全红）后绿；`cargo test --no-fail-fast` 69 个测试二进制 1189 过 / 0 失败 / 1 ignored（merge_drivers 原有）；`pnpm build` 通过；`pnpm test` 55 文件 835 过 |
| 6 | 对象层重放引擎 | 完成（未 commit） | 新增 `engine/svn_replay.rs`（`ReplayCodes`、`ReplayCommit`、`ReplayOpts`、`ReplayOutcome`、`commits_between`、`replay_onto`、`all_committed`、`no_superseded`）、`tests/svn_replay.rs`（14 条）；改 `svn_deliver.rs`（改为调用 `svn_replay`，`DELIVER_CODES`）、`svn_baseline.rs`（`TempIndex::abs_path`、`TempFile` 改 `pub(crate)` 加 `in_git_dir` / `abs_path`）、`engine/mod.rs` | `cargo fmt --check` 通过、`cargo clippy --all-targets` 无告警；新测试先红（stub 下 14/14 红）后绿；4 次变异核对见 Task 6 小节；`svn_deliver` 11、`svn_sync_out` 50、`svn_sync_out_check` 16、`svn_baseline` 23 全过；`cargo test --no-fail-fast` 70 个测试二进制 1203 过 / 0 失败 / 1 ignored（merge_drivers 原有）；前端未改，未跑 `pnpm` |
| 7 | Local-Only 泄漏按 hunk 判定 | 完成（未 commit） | 新增 `engine/svn_leak.rs`（`Leak`、`HunkLeak`、`LeakVerdict`、`local_only_leaks`）、`tests/svn_leak.rs`（10 条）；改 `engine/mod.rs` | `cargo fmt --check` 通过、`cargo clippy --all-targets` 无告警；`svn_leak` 先红（stub 下 7/9 红）后绿 10/10；6 次变异全部抓到；全量 `cargo test --no-fail-fast` 71 个测试二进制 1223 过 / 4 失败 / 1 ignored：4 个失败是 `svn_health` 的 `non_ascii_excluded_directory_absent_at_the_floor_does_not_refuse` 与 `svn_init` 的 `rebuild_refuses_property_only_conflict`、`rebuild_refuses_tree_conflict`、`rebuild_refuses_a_locked_subdirectory`，当时 Task 1–5 的 review 修复正在进行，且除 `mod.rs` 的注册外没有代码引用 `svn_leak`，不是本 Task 引入；这几条是否全绿待该修复收尾后复核 |
| 8 | 跟进时拼索引 | 完成（未 commit） | 新增 `engine/svn_follow_index.rs`（`PathFacts`、`StageDecision`、`ComposeReport`、`PreparedIndex`、`prepare_index`、`rename_lock_into_index`）、`tests/svn_follow_index.rs`（31 条，含 2 条 Windows 专属）；改 `svn_baseline.rs`（`IndexLock` 加 `path` / `abs_path` / `is_released` / `release`，抽出 `parse_entry_flags`，`copy_with_mtime` 改 `pub(crate)`）、`engine/mod.rs` | `cargo fmt --check` 通过；`cargo clippy --all-targets` 本 Task 的文件无告警（5 条告警全在 Task 9 进行中的 `tests/svn_follow_mark.rs`）；`svn_follow_index` 先红（stub 下 29/30 红，那 1 条是「prepare 失败不留锁」，stub 恒失败时天然成立）后绿 31/31；20 个变异全部抓到；`cargo test --no-fail-fast` 73 个测试二进制 1295 过 / 0 失败 / 1 ignored（merge_drivers 原有），第一次因 Task 9 并行占用输出文件 link.exe 报 1104，重跑通过；前端未改，未跑 `pnpm` |
| 9 | 失焦快照与「我是谁」 | 完成（未 commit） | 新增 `engine/svn_follow_mark.rs`（`FollowMark`、`write_mark`、`read_mark`、`clear_mark`、`my_svn_user`、`resolve_svn_user`、`parse_svn_auth`、`svn_auth_listing`、`WcDbStamp`、`wc_db_stamps`、`wc_db_stamps_with`）、`tests/svn_follow_mark.rs`（34 条）；改 `svn_config.rs`（`svn_user`）、`svn_init.rs`（字面量）、`commands/svn.rs`（`svn_follow_mark`、`svn_wc_db_stamps`）、`commands/watch.rs`（过滤 `.git/mygitclient/`）、`lib.rs`、`engine/mod.rs`、`src/store/repoStore.ts`（invoke 封装）；测试 `tests/svn_config.rs` +1、`svn_backend.rs` +1 | `cargo fmt --check` 通过、`cargo clippy --all-targets` 无告警；新测试先红（stub 下 33 条里 32 红，剩下 1 条由命令层守卫挡住）后绿 34/34；18 次变异全部抓到；`cargo test --no-fail-fast` 73 个测试二进制 1295 过 / 0 失败 / 1 ignored（merge_drivers 原有，含 Task 8）；`pnpm build` 通过；`pnpm test` 55 文件 836 过。**`pnpm tauri dev` 未验证，留给 Task 11**（见小节遗留风险） |
| 10 | 跟进编排与崩溃恢复 | 完成（未 commit） | 新增 `engine/svn_follow.rs`（`follow`、`resume_follow`、`FollowOutcome`、`FollowReport`、`git_version_ok`、`fault`）、`tests/svn_follow.rs`（72 条）、`src/engine/svnFollow.ts`、`tests/vitest/svnFollow.test.ts`；改 `svn_journal.rs`（`Flow::Follow`、`Phase::FollowPublishing`、`FollowStep`、`FollowSteps`、`BranchMove`）、`commands/svn_sync.rs`（`svn_follow`、`svn_follow_continue`）、`lib.rs`、`engine/mod.rs`、`svn_follow_index.rs`（`keep_lock_on_drop`、`Deserialize`）、`svn_leak.rs`、`svn_sync.rs`（`Deserialize`）、`svn_sync_in.rs`、`svn_sync_out.rs`（跟进记录的报错）、`src/engine/svnJournal.ts`、`svnSync.ts`、`src/store/repoStore.ts`、`src/components/BaselineBanner.tsx`、`svnSyncFlow.ts`；测试 `tests/svn_journal.rs` +2、`svn_backend.rs` +1、vitest 3 个文件 +8；计划 Global Constraints 改 git ≥ 2.43 | `cargo fmt --check` 通过、`cargo clippy --all-targets` 无告警；新测试先红（stub 下 64/64 红，vitest 6 条红加 1 个文件加载失败）后绿；34 次变异全部抓到（第一轮 2 个存活，补测试后抓到，见 Task 10 小节）；`cargo test --no-fail-fast` 74 个测试二进制 1370 过 / 0 失败 / 1 ignored（merge_drivers 原有）；`pnpm build` 通过；`pnpm test` 56 文件 849 过。**`pnpm tauri dev` 未验证**（见小节遗留风险） |
| 11 | 前端：检测、跟进入口与结果展示 | 完成（未 commit），**真实窗口未验证** | 新增 `src/components/FollowReportDialog.tsx`、`tests/vitest/FollowReportDialog.test.tsx`、`BranchView.test.tsx`、`repoStoreFollowFocus.test.ts`；改 `src/engine/svnFollow.ts`（`describeFollowReport`、`needsSvnUserPrompt`、`describeFollowConflict`、`describeCheckoutBehind`）、`svnSync.ts`（`WcDbStamp`、`wcDbStampsChanged`、`syncIfWcDbChanged`、`shouldMarkOnBlur`、`markOnBlur`）、`src/store/repoStore.ts`、`src/components/svnSyncFlow.ts`（`handleFollow`、`handleCheckoutBehind`、`settleFollow`）、`BaselineBanner.tsx`（`followBusy`）、`BranchView.tsx`、`ConfirmDialog.tsx`（`alternative`）、`RepoTab.tsx`（`active`）、`src/App.tsx`、`src/styles.css`；后端 `engine/svn_follow.rs`（`branches_behind`）、`svn_follow_mark.rs`（`SvnUserInfo`、`svn_user_info`）、`svn_config.rs`（`set_svn_user`）、`commands/svn.rs`（`svn_branches_behind`、`svn_get_user`、`svn_set_user`）、`lib.rs`；测试 `tests/svn_follow.rs` +7、`svn_follow_mark.rs` +10、`svn_backend.rs` +1，vitest 见小节 | `cargo fmt --check` 通过、`cargo clippy --all-targets` 无告警、`cargo build` 通过；新测试先红（Rust 编译不过，vitest 7 个文件 55 条红）后绿；54 次变异，第一轮 2 个存活，1 个补断言后抓到，1 个是等价变异、删了多余代码（见 Task 11 小节）；cargo test --no-fail-fast 74 个测试二进制 1467 过 / 0 失败 / 1 ignored（merge_drivers 原有）；`pnpm build` 通过；`pnpm test` 60 文件 994 过。**`pnpm tauri dev` 未验证**，手测清单见小节 |
| 12 | 提交图 | 完成（未 commit） | 改 `engine/graph.rs`（`baseline_floor`、`baseline_newer`、`baseline_gone`，改用 `parse_message`）、`refs.rs`（过滤 `refs/mygitclient/`）、`svn_basemap.rs`（`parse_message` 快路径）、`svn_log.rs`（锚点补 floor）、`src/engine/types.ts`、`svnGraph.ts`（`liveParents`、`mixedBaselineLabel`、`deliveredCommits`）、`src/components/CommitRow.tsx`、`src/styles.css`；测试 `tests/svn_graph_contract.rs` +9、`svn_backend.rs` +1、`tests/vitest/svnGraph.test.ts` +19，新增 `tests/vitest/CommitRow.test.tsx`（2 条） | `cargo fmt --check` 通过、`cargo clippy --all-targets` 无告警；新测试先红（vitest 11 条红，Rust 编译不过，补字段后 3 条红）后绿；12 次变异全部抓到；`cargo test --no-fail-fast` 74 个测试二进制 1380 过 / 0 失败 / 1 ignored（merge_drivers 原有）；`pnpm build` 通过；`pnpm test` 57 文件 870 过。**`pnpm tauri dev` 未验证**（见小节遗留风险） |
| 13 | 手动跟进 | 完成（未 commit） | 改 `engine/svn_follow.rs`（`manual_follow`、`require_linear`、`NeedsManual.leaks`）、`svn_sync_in.rs`（`SyncInOutcome::StageLost`、`take_over`、`restore_step`、`finish_forward`、`resume_stashed`）、`svn_sync_stash.rs`（`restore_wip_keeping_index`）、`svn_journal.rs`（`Flow::FollowManual`、`stage_lost`）、`svn_sync_out.rs`、`commands/svn_sync.rs`（`svn_follow_manual`）、`lib.rs`、`src/engine/svnFollow.ts`、`svnSyncIn.ts`、`svnJournal.ts`、`src/store/repoStore.ts`、`src/components/svnSyncFlow.ts`；测试 `tests/svn_follow.rs` +26（含改 1）、`svn_sync_stash.rs` +5、`svn_journal.rs` +1（含改 1）、`svn_backend.rs`、`tests/vitest/svnFollow.test.ts`、`svnJournal.test.ts`、`svnSyncFlow.test.ts`、`svnSyncIn.test.ts` | `cargo fmt --check` 通过、`cargo clippy --all-targets` 无告警；新测试先红（stub 下 Rust 25 条里 24 红，剩下 1 条断言「不会发生」；vitest 23 条红）后绿；22 个变异第一轮 2 个存活，1 个改测试后抓到，1 个是等价变异、删了多余代码（见 Task 13 小节）；`cargo test --no-fail-fast` 74 个测试二进制 1449 过 / 0 失败 / 1 ignored（merge_drivers 原有）；`pnpm build` 通过；`pnpm test` 57 文件 897 过。**`pnpm tauri dev` 未验证**：新命令 `svn_follow_manual` 没有按钮（Task 11） |
| 14 | 收尾 | 待做 | | |
| R1 | Review 1 修复（Task 1–5） | 完成（未 commit） | 改 `svn_health.rs`、`svn_tombstone.rs`（`fault`）、`svn_nodes.rs`（`dir_revs`）、`svn_sync.rs`、`svn_sync_check.rs`、`svn_xml.rs`（`copied`）、`svn_init.rs`、`svn_baseline.rs`（`TempFile::for_svn_arg`）、`src/engine/svnInit.ts`；测试 `tests/svn_health.rs`、`svn_init.rs`、`svn_nodes.rs`、`svn_sync.rs`、`svn_backend.rs`、`tests/vitest/svnInit.test.ts`。详见「Review 1 修复」小节 | `cargo fmt --check` 通过、`cargo clippy --all-targets` 无告警；`cargo test --no-fail-fast` 71 个测试二进制 1227 过 / 0 失败 / 1 ignored（merge_drivers 原有，含 Task 7 的 `svn_leak`）；`pnpm build` 通过；`pnpm test` 55 文件 836 过 |
| R2 | Review 2 修复（Task 6–10） | 完成（未 commit） | 改 `engine/svn_follow.rs`、`svn_follow_index.rs`（`prepare_index_with`、`pin_raw_index` 挂 `blobs/<oid>`）、`svn_follow_mark.rs`（`base_oid`）、`svn_leak.rs`、`svn_health.rs`（`repo_rel_of`、`incomplete`）、`svn_tombstone.rs`（`FirstDiff` / `LaterDiff` / `DiffPath`）、`svn_journal.rs`（`indexStages`、`lockCreated`）、`svn_sync.rs`（`head_behind_base`）、`src/engine/svnSync.ts`、`svnFollow.ts`；测试 `tests/svn_follow.rs`、`svn_follow_index.rs`、`svn_follow_mark.rs`、`svn_leak.rs`、`svn_health.rs`、`svn_journal.rs`、`svn_backend.rs`、`tests/vitest/svnSync.test.ts`、`svnFollow.test.ts`。详见「Review 2 修复」小节 | `cargo fmt --check` 通过、`cargo clippy --all-targets` 无告警；回归测试先红后绿，14 次变异全部抓到；`cargo test --no-fail-fast` 74 个测试二进制 1417 过 / 0 失败 / 1 ignored（merge_drivers 原有）；`pnpm build` 通过；`pnpm test` 57 文件 874 过。**`pnpm tauri dev` 未验证**：`SyncStatus`、`FollowReport` 加字段，没有改命令签名，靠契约测试 |
| R3 | Review 3 修复（最终 review） | 完成（未 commit） | 改 `engine/svn_sync_stash.rs`（`staged_only_paths`）、`svn_sync_in.rs`（`RestoreStep::StageKept`、`SyncInOutcome::StageLost` 加 `stash_oid` / `files`、`rolled_back` 尾句）、`svn_follow.rs`（`stale_disk_paths`、`NeedsManual.stale_disk`）、`svn_config.rs`（`set_svn_user` 临时文件名与 rename 重试）、`src/engine/svnSyncIn.ts`（`stageLostDetail`、`STAGING_NOTE`）、`svnFollow.ts`、`svnSync.ts`、`src/components/svnSyncFlow.ts`（删 `handleManualFollow`）；测试 `tests/svn_follow.rs` +4（改名 1、改 3）、`svn_sync_in.rs` +2、`svn_config.rs` +1、`svn_backend.rs`，vitest `svnSyncIn.test.ts` +3、`svnFollow.test.ts` +3、`svnSyncFlow.test.ts`（`handleManualFollow` 的 5 条改成 3 条经 `handleCheckoutBehind`）。详见「Review 3 修复」小节 | `cargo fmt --check` 通过、`cargo clippy --all-targets` 无告警；第 2、3 条的回归测试修前跑红，第 1 条修前编译不过（证据见小节）；5 个变异第一轮 1 个存活，补 `a_file_the_lagging_branch_…` 后抓到；`cargo test --no-fail-fast` 74 个测试二进制 1473 过 / 0 失败 / 1 ignored（merge_drivers 原有），之后补的那 1 条单独跑过；`pnpm build` 通过；`pnpm test` 60 文件 998 过。**`pnpm tauri dev` 未验证**：没有改命令签名，两个结局加字段，靠契约测试 |
| P1 | 性能：录制的候选索引沿用 stat | 完成 | 改 `svn_baseline.rs`（`TempIndex::from_tree_keeping_stat`）、`svn_follow.rs`（`record_from_old_base`）；测试 `tests/svn_index_stat.rs` +1、`tests/svn_follow.rs` +2 | 退路测试先红（`include_set_mismatch`）后绿；stat 测试先编译不过后绿；同树测试改动前后都绿（守行为不变）。全量验证见 P3 |
| P2 | 性能：准备阶段的 svn 扫描去重 | 完成 | 改 `svn_cli.rs`（`with_scan_cache`、`forget_node_scans`）、`svn_follow.rs`（包住 `prepare`）、`svn_init.rs`（R3c 前 `forget_node_scans`）；测试 `tests/svn_cli.rs` +3、`tests/svn_follow.rs` +1 | 2 个变异各被一条测试抓到；全量验证见 P3 |
| P3 | 性能：git 只读查询改走 libgit2 | 完成 | 新增 `engine/git_read.rs`、`tests/git_read.rs`（9 条）；改 `svn_baseline.rs`、`svn_sync.rs`、`svn_sync_check.rs`、`svn_basemap.rs`、`svn_follow.rs`、`svn_replay.rs`、`svn_scope.rs`、`svn_follow_mark.rs`、`svn_include_check.rs`、`engine/mod.rs` | `cargo fmt --check` 通过、`cargo clippy --all-targets` 无告警；`cargo test --no-fail-fast` 75 个测试二进制 1490 过 / 0 失败 / 1 ignored（merge_drivers 原有）；真实窗口回归见「性能」小节；前端未改，未跑 `pnpm` |

## 【待决】汇总

| # | 事项 | 当前采用的保守做法 | 位置 |
|---|---|---|---|
| D1 | 实现期间是否按 Task 提交 commit（AGENTS.md §1.1 要求用户放行；用户全局规则允许 worktree 分支例行提交） | **已定（2026-11-01）：** 实现期间不 commit；全部完成后用户放行提交并建 Draft PR | 全程 |
| D2 | 映射条目上限 | **已定（2026-11-01）：** 保持 `MAX_MAP_ENTRIES = 5000`，不做目录压缩。用户确认白名单一周内变化的代码文件不会超过这个数 | Task 1、2 |
| D3 | 跟进钉住旧索引的 ref 保留多少 | **已定（2026-11-01）：** 保持最近 20 个（`KEEP_PINS`） | Task 10 |
| D4 | 「我的 SVN 用户名」取不到时 | 一律当「不是我」，暂存走三方合并，commit 正常重放（不会误丢数据，只是 U2 / U3 的中间版本不会自动让位）。Task 9 已实现 `my_svn_user`：`svnUser` 配置优先，否则看 `svn auth`；`None` 的情形是 `file://`、没有缓存、同一服务器多个不同用户名、非 ASCII 用户名（管道输出是 GBK，见 Task 9）。入口已加（Task 11）：跟进报告里 `uncertain` 非空或取不到用户时显示一行输入框，`svn_set_user` 写配置，空串清除。**已定（2026-11-01）：** 不加常驻入口，维持现状；先手测自动探测在用户机器上能否取到用户名，取不到再议 | Task 9、10、11 |
| D5 | Q8：C3 与 C2 的先后 | **已定（2026-11-01）：** 先做 C3（在 jkk 里不更新整个项目的交付），再做 C2。不影响 C1a | 分析 §9、§10 Q8 |
| D11 | 离线跟进：第一条 diff 就失败（通常是断网）时怎么办（Review 2 第 6 条） | **已定（2026-11-01）：** 维持现状，照常发布并在报告里标「未验证」（`verified: false`）；D 已知后的任何失败仍一律 `Unconfirmed` 拒绝。离线录制可能漏记已提交删除，见分析 R35，联网后体检会报出 | Review 2 修复第 6 条 |
| D6 | Task 3～5 之间 sync_in 遇到 `ExternallyAdvanced` 怎么办（指令要求暂时一律拒绝） | ~~只拒绝「非整体前进」的那部分~~ Task 5 已收口：任何 `ExternallyAdvanced` 都走 A16 接管（`rebuild_baseline` 已能录混合版本），不再报 `sync_in_out_of_sync` | Task 3（`svn_sync_check::uniform_forward_rev`），Task 5 收口 |
| D7 | ~~Task 11 接线之前，整体前进的横幅只剩一颗不渲染的「跟进」~~ | **已处理**（Task 11）：`RepoTab` 接上 `onFollow`，横幅的「跟进」可用，执行中禁用 | Task 3、11 |
| D8 | 被 depth-exclude 的节点没法用尾随 PATH 点名（单元根本身、路径含非 ASCII 字符）时怎么判 | **已处理**（Review 1 第 3 条）：对它 `svn ls -R --xml <编码后的 url>@<F>`，列表按 W-file 口径筛，筛出来的与点得了名的走同一条 HEAD 确认；没有 W-file 的不阻断 | Task 4（`svn_health::check_grouped`），Review 1 收口 |
| D9 | 跟进的最低 git 版本：`merge-file --object-id` 要 2.43，计划写的是 2.40 | Task 10 已把预检改成 2.43，在 F1 之前报 `svn_follow_git_too_old`，计划 Global Constraints 同步。**需用户知晓：** 最低版本上调 | Task 8、10 |
| D10 | ~~Task 11 之前 `needsManual` 的出路~~ | **已处理**（Task 13、11）：`describeFollowOutcome` 改成走「手动跟进」，不再提「更新」；Task 11 把这段话放到手动跟进的确认框上（`describeFollowConflict`），「手动跟进」就是框上的按钮 | Task 10、13、11 |

## 决策条目

### Task 0：实测

编号续接分析 §3：SVN 侧 E13 起，git 侧 W17 起。环境：开发机 SlikSvn 1.14.2、git 2.51.2.windows.1（全局 `core.autocrlf=true`、`core.quotepath=false`），全部在 scratchpad 的临时 fixture 里跑，跑完即删。脚本与原始输出当时留在 scratchpad 的 `exp-svn/`、`exp-replay/` 下，不入库。现在这两个目录已经找不到了：Review 1 修复时一次实验脚本的 `rm -rf` 参数写错，清空过本会话的 scratchpad，清空前没有列目录，分不清它们是被那次清掉的还是更早就不在了。按找不回处理，下面表格是这些实验的唯一记录。

#### 计划没写的坑（实测结论）

| # | 场景 | 结论 | 对实现的约束 |
|---|---|---|---|
| E13 | `svn status -v --xml` 各状态的 `<commit>` | 有 BASE 才有 `<commit>`：`added`（revision=-1）、`replaced` 没有；冲突 `item="conflicted"` 有；树冲突是 `item="added" copied="true" tree-conflicted="true"` 且无 revision；局部更新推高的文件 `<commit revision>` 不变（再证 E12） | 映射的键取基线树文件；`added`/`replaced` 不能读 c(f)，`replaced` 按「内容变了」的保守方向处理不了，只能视为本地改动（不改映射）；树冲突按 SVN 冲突拦截 |
| E14 | changelist 成员在 XML 中的位置 | 在 `</target>` 之后、与之平级的 `<changelist name>` 块里，每个文件只出现一次 | 新解析必须遍历整份文档的 `<entry>`（`descendants()`），不能只遍历 `<target>` |
| E15 | 工作副本被锁（活锁：commit 等编辑器；陈旧锁：进程被杀） | `svn status -v --xml` 仍 rc=0、stderr 空，只有**目录** entry 带 `wc-locked="true"`；`svn update` 报 E155004；分不出活锁与陈旧锁 | 「更新进行中」的检测看目录上的 `wc-locked`，不看 status 失败；不能自动 cleanup |
| E16 | 墓碑核对 `svn info <url>/<p>@<rev>` | 不存在：rc=1，stderr 每个缺失 target 一条 W170000，末尾 E200009；版本号不存在是 E160006（另一回事）；多 target 时存在的照常输出到 stdout 的 XML，`path` 属性只有文件名，**要按 `<url>` 对回**；支持 `--targets FILE` | 墓碑核对批量走 `svn info --xml --targets <file>`，按 W170000 判不存在；rc=1 不等于全部失败 |
| E17 | `--set-depth=exclude` 的文件 | 从 `status -v` 消失，与「提交删除」看起来一样；但 `svn info <wc 路径>` 仍成功并带 `Depth: exclude` | 「消失」先查服务器（分析 §6.2 原设计），本地 `svn info` 可作为额外的漏收证据 |
| E18 | 分组体检的命令形状 | 整单元 diff 的根目录总有一条假的 `kind=dir item=none props=modified`，`--ignore-properties` 可去掉；`--old=unit@r --new=wc PATH...` 可带多个尾随 PATH（相对单元根），但共用一个 OLDREV；任一 PATH 未版本化或不存在 → 整条 E155010；**磁盘丢失（!）的文件不报**；没有 `--targets` | 每个 revision 组一条（按命令行长度分批）；发之前用 status 剔除未版本化 / 丢失的 PATH，丢失的单独按 status 判；结果路径去掉 `unit_url/` 前缀对回 |
| E19 | jkk 发的只读 svn 命令是否写 wc.db | `info`（含 -R、--include-externals）、各种 `diff`、`propget -R`、`ls`、`status -v`、`status -u -v`、`cat -r BASE` 全部不写；`changelist`、`cleanup` 会写；`wc.db-journal` 常驻 0 字节 | 焦点检测可直接用 wc.db 的 mtime + size，不会自激；changelist / cleanup 也会触发一次校验（无害） |
| E20 | `svn:keywords` / `svn:eol-style` | 常规情形「干净取磁盘、脏取 `cat -r BASE`」与 BASE 一致，局部更新只推高节点 revision 时字节与 mtime 不变；**例外**：只改关键字展开值或换行符时 svn 视为干净，但磁盘 ≠ BASE | 现行录制已有的盲区，不是 C-α 引入的。C1a 不改，记为遗留风险（见 Task 0 末） |
| E21 | changelist 与外部提交 | 命令行 `svn ci` 显式提交 changelist 里的文件会把它移出 changelist（`--keep-changelists` 除外）；命令行不认 `ignore-on-commit`，提交整个目录照样提交；update / 合并 / 冲突 / resolve / revert 都不改成员 | C1b 的分组只读成员；「文件离开 changelist」可作为外部提交的旁证（C1a 不用） |
| W17 | 对象层重放边界 | 重命名：一侧 `git mv`、另一侧改内容 → 干净；双方改名 → `CONFLICT (rename/rename)`；改名又重写内容 → 退化成 modify/delete；二进制两侧都改 → rc=1、结果树里放 ours；mode 与内容各改一侧 → 干净；`-z` 下中文路径原样 UTF-8 | 冲突解析只认 `-z` 输出的 stage 行与信息段的类型标签，不解析文本；现有 `replay` 已带 `-z` |
| W18 | 「变空」判据 | 「结果树 == 当前父（上一个重放结果）的树」正确；「== onto 的树」两个方向都会错（把改回去的 commit 误丢、把真正变空的保留） | 沿用分析 §6.5 F2 ② |
| W19 | 「自己提交的路径取 SVN 版本」构造 C' | W10-R2、W14 两种形态下 C' 重放后全部变空丢弃、tip == NEW；C 同时改 a 和 e 时只留 e。构造：临时索引 `read-tree C` → 对 p 写 C^ 的条目（C^ 没有就写 mode 0 删掉）`update-index -z --index-info` → `write-tree` → `commit-tree -p C^` | P 只能放文件路径（喂 tree 条目会写坏索引）；`ls-tree` 要 `--full-tree`；`GIT_INDEX_FILE` 用绝对路径；pathspec 调用加 `--literal-pathspecs` |
| W20 | 「已全部提交」捷径判据 | `diff-tree -r -z --name-only --no-renames O HEAD` 与 `diff-tree ... HEAD NEW` 的交集为空 ⇔ HEAD 改过的每个路径（含删除、mode）都与 NEW 相同；4 个用例（含 `--no-ff` merge、删除、新增、mode、重命名）与逐路径 `ls-tree` 比较结论一致 | 两次进程调用、无命令行长度问题；外加 `rev-list O..HEAD` 里没有 Local-Only |
| W21 | Local-Only 泄漏按 hunk 判定 | 整个 L 三方重放：相邻行（line5 log、line6 功能）合成一个冲突块，判不出；`git apply --unidiff-zero --check` 会按偏移全文搜索，假阳性；**用 `diff -U0 B NEW` 的 hunk 做 B→NEW 行号映射，在映射位置比较 L 的 + 行：12 个判定全对** | 按 M4 实现（`git2::Patch::from_blobs`，context_lines=0）：不重叠 → 没泄漏；重叠且 NEW 有更早的改变行数的 hunk → 拿不准；其余内容相等 → 泄漏 |
| W22 | `git merge-file` | 参数顺序 `<ours> <base> <theirs>`；退出码 = 冲突块数；二进制一律 rc=255；`--object-id` 可直接对 blob id 合并并把结果写入对象库、stdout 打印 oid（冲突时 blob 带标记）；不做 eol 规范化 | 先处理 trivial（ours==base 等），二进制两侧都改按冲突；用 `--object-id` + 三个 `-L`；以退出码为准 |

| W23 | 持有 `.git/index.lock` 期间 | 副本（或直接 `GIT_INDEX_FILE=.git/index.lock`）上的 `read-tree -i -m`、`update-index --index-info` / `--cacheinfo --add` / `--force-remove`、refresh、write-tree 都正常；`update-ref` 不需要 index.lock，但 HEAD 指向该分支时要拿 HEAD.lock；外部 `add` / `commit` / `checkout <已有分支>` / `reset` / `stash push` 立即失败（不等待），`status` / `diff` 静默不写；**`checkout -b`、`switch -c`、`symbolic-ref` 不受阻挡，能把 HEAD 改走**（Task 10 更正：`checkout -q -b` 会撞锁，不带 `-q` 才绕得过，`switch -q -c` 也绕得过）。**Review 2 补充：**① `checkout -b` 建的分支落在**当时 HEAD 解析到的 commit** 上，落在 CAS 之后就是 NEW_TIP，这时「HEAD 不在原分支上」不等于「要回滚」；② 活着的 `git commit -a` 等编辑器时 index.lock 存在、句柄已关，`remove_file` 成功（探针 P7），「删不掉才是别人的」不成立 | 发布前核对 HEAD 的符号引用，CAS 之后核对 HEAD 的 OID（第 3 条）；用 `create_new` 建锁 + RAII guard；只删 journal 记过的锁（第 7 条） |
| W24 | 换入索引的 rename | 外部进程打开 `.git/index` 时没给 FILE_SHARE_DELETE，`std::fs::rename` 报 os error 5；给了就能成功（MoveFileEx 仍失败） | 用 `std::fs::rename`，遇 os error 5 / 32 退避重试 1–2 秒；仍失败则回滚 |
| W25 | `read-tree -i -m OLD NEW` 遇到非内容修改的暂存 | 暂存了新增 / 删除 / mode / 修改 / i-t-a 的路径，若 NEW 也改了它且结果不同 → 整体拒绝（rc=128，副本不被半写，报错可逐条解析出路径）；没暂存且 NEW 删除 → 条目删掉；skip-worktree 保留；**assume-unchanged 在 NEW 改了该路径时丢失** | 先求「已暂存 ∩ OLD→NEW 改过」的路径，预先写成等于 NEW 的条目再 read-tree，之后再写最终条目；read-tree 前记下 assume-unchanged 的路径，之后补回 |
| W26 | 暂存条目的 mtime（推翻 W16 的推论） | `add -p`、`--cacheinfo`、`--index-info`、read-tree 取来的条目、`stash apply` 后与磁盘不一致的条目，mtime / ctime / size **全是 0**；暂存后 touch 再跑 status，mtime 被刷成更晚的时刻；整文件 add 记的是文件自己的 mtime，常远早于暂存时刻 | **「暂存条目 mtime ≤ 暂存时刻」不成立，分析 §6.11.1 的 T0 判据不能用**，改法见下 |
| W27 | `commit-tree -F` 与 trailer | UTF-8 逐字节往返，1.1MB 无长度限制；`log %B` 多一个结尾 `\n`；未设 `i18n.commitEncoding` 时写 GBK 字节会被不可逆改写；`interpret-trailers` 会改写前导空格、续行、`#` 行 | message 始终写 UTF-8，用 `cat-file commit` 读回；trailer 只放最后一段；路径做百分号编码；自己解析 trailer，不依赖 `interpret-trailers` |
| W28 | 钉住旧索引 | write-tree 包成 commit 再 read-tree 还原会丢 i-t-a、skip-worktree / assume-unchanged 标志与全部 stat；冲突时 write-tree 失败。`hash-object -w .git/index` 存原始 blob 并用 ref 钉住，能逐字节还原（含冲突 stage、标志、stat）；`refs/mygitclient/*` 能挡住 gc。**Review 2 更正：挡住的只是 index 这个 blob 本身。索引文件对 git 是一段不透明的字节，它引用的暂存 blob 不可达，`gc --prune=now` 之后钉住的索引还在、内容没了（探针 P6）** | 钉原始 index blob（`refs/mygitclient/follow/<opid>` 指向一个 commit，其树里放 `index` blob，便于 gc 保护与 reflog 之外的查找），**同一棵树里挂 `blobs/<oid>`：相对 OLD 的全部已暂存 blob**（第 4 条）；没暂存的条目等于 OLD 的树，靠 OLD 可达；索引有冲突时拒绝跟进 |

#### 计划的错误

- **分析 §6.11.1 的 T0 判据（暂存条目 mtime 早于 T0 ⇒ 暂存早于外部提交）不成立**（W26）。改为**失焦快照**：前端在「最近一次校验为 InSync」之后窗口失焦时，调用后端记下一份快照 `{ at, head_oid, branch, staged: { path → 暂存 blob oid } }`（`git diff-index --cached -z HEAD` 只比索引与树，不扫工作区，开销小），存在 `.git/mygitclient/follow-mark.json`。跟进时：
  - 暂存：该路径当前的暂存 blob 等于快照里的 blob ⇒ 这份暂存在外部提交之前就有了 ⇒ 作者是我时丢弃；不相等或快照里没有 ⇒ 拿不准，保留
  - commit：commit 是快照 `head_oid` 的祖先（或就是它）⇒ 早于外部提交 ⇒ 作者是我时按 F2 ① 换成 SVN 版本；否则正常重放
  - 没有快照（重启前从未记过、跟进后已清掉）⇒ 一律按拿不准处理
  - **Review 2 更正：上面两条默认了「快照早于这批 SVN 提交」。** 判定 InSync 不等于没有 SVN 提交：基线已发布、分支落后时写的快照比那批提交新，快照里的 commit 与暂存可能是提交之后的新工作。现在快照多记 `base_oid`，这两条只对 `diff-tree <base_oid> NEW` 里的路径成立；`base_oid` 缺失、读不到、不是 NEW 的祖先时一律拿不准
  - 外部工具在 jkk 失焦前做的暂存同样被快照收进来，比「jkk 自己记暂存时刻」覆盖面更大；判断不依赖任何时钟
- **分析 §6.11.1「实现」第 1、2 步的顺序有误**：先把特殊路径写成最终条目再跑 `read-tree -i -m`，最终条目与 OLD、NEW 都不同时 read-tree 会整体拒绝（W25）。改为：特殊路径先写成等于 NEW 的条目 → `read-tree -i -m OLD NEW` → 再写最终条目 → 补回 assume-unchanged → refresh
- **分析 §6.5 F5 ② 钉住旧索引用 write-tree 包成 commit，还原会丢标志与 stat**（W28）。改为钉原始 index blob

#### 偏离计划

- **发布顺序**（分析 §6.5 F5）：原为「基线 → 换索引 → 移当前分支」。改为「基线 → 持 IndexLock 在锁内拼好新索引（写进 `.git/index.lock`）→ 核对 HEAD 符号引用 → CAS 移当前分支 → 再核对 HEAD → rename 成 `.git/index`」，rename 失败（W24）时先把分支 CAS 回去再删锁。理由：分析担心的「分支已移、索引未换」的反向暂存窗口，在整个过程都持有 index.lock 时外部 git 无法提交（W23），窗口不存在；而先 CAS 分支，CAS 失败时什么都还没动，回滚最简单。崩溃在 CAS 之后、rename 之前时，残留的 index.lock 会挡住所有 git 写操作，由 journal 在重开时补做 rename

- **映射取值**（细化分析 §6.1 / §6.2）：分析只说「r(f) = 节点 revision，floor 取最小」。按节点 revision 记，周一根目录更新后未改过的文件仍各记各的旧值，映射永远回不到单值；全记节点 revision，局部更新又会把整个目录写进映射。改为：F = 所有文件节点 revision 的最小值（**Review 1 第 2 条更正：还要算上白名单内的目录节点**，原写法在目录 revision 更低时会发布假的单值）；c(f) ≤ F 的文件隐含 F，其余记 n(f)。正确性：对任意 r ∈ [c(f), n(f)]，f@r 都等于当前的 pristine（f 在 c(f) 之后没改过），F 与 n(f) 都落在这个区间里。效果：周一更新后自动回到单值、旧格式；局部更新只让真正有改动的文件进映射
- **墓碑的产生方式**（细化分析 §6.2 / §6.3）：不从「旧映射有、现在消失」推导，而是在 R3b（`JustRecorded`）里把 floor 组没被解释的 D 逐个向服务器 HEAD 确认，确认不存在的成为墓碑。初始化（没有旧映射，Q4）与跟进共用同一条路径；服务器上仍存在的拒绝录制（漏收，E17）
- **「SVN 模式下合并默认快进 / rebase」（分析 §6.11.3、第五轮）在 C1a 不做**：仓库里没有合并入口，merge commit 只能来自外部工具，由「已全部提交」捷径（Task 6）兜住。以后加合并入口时再按分析实现

#### 为什么这么改是安全的

- 全部实验只在 scratchpad 的临时仓库里做，没有碰本仓库、用户工程或任何真实仓库
- 子 agent 在 PowerShell 下调 Git Bash 跑脚本，因为 Bash 工具拒绝在 worktree 外执行 git；fixture 跑完已删

#### 遗留风险

- E20：带 `svn:keywords` / `svn:eol-style` 的文件只改关键字或换行时，现行录制会把非 BASE 字节录进基线。C-α 之前就存在，C1a 不处理

### Task 1：BaseMap 记账格式

#### 偏离计划

- **只改了 `create_baseline_commit_with` 的签名**，`create_baseline_commit(.., &Trailer)` 与 `publish_baseline(.., &Trailer, ..)` 保持原签名，内部转成 `BaseMap::single` 再调 `_with`。init、rebuild、sync_in S7 经这两个包装、sync_out D8a 直接调 `_with`，四个调用方实际都走 `BaseMap::single`，行为不变。理由：diff 最小，现有测试（`tests/svn_baseline.rs`、`tests/svn_sync_in.rs` 里十余处 `&Trailer` 调用）一处不用改。Task 5 要录混合基线时，再把 `publish_baseline` / `create_baseline_commit` 改成接 `&BaseMap`，测试调用点一起改
- **写入前自检**（Interfaces 没写）：`entry_count() > MAX_MAP_ENTRIES` 报 `svn_map_too_large`；`format_message` 的结果用 `parse_message` / `parse_delivered_rev` 读不回原值报 `svn_map_unreadable`。这只是兜底；带「revision 最低的 10 个文件」的友好报错仍归 Task 2 的 `compute_map`
- **单值基线也写 `summary_extra`**：计划只规定混合时写。单值且 `summary_extra` 非空时，摘要作为首行之后单独一段，`git-svn-id` 仍在最后一行；`summary_extra` 为空时与旧格式逐字节相同（跟进后正好回到单值时，摘要不会丢）
- 测试比计划多断言了几处，都没有新增测试名：`single_map_formats_exactly_like_legacy_trailer` 还比较了 `-F` 与旧 `-m` 写进 commit 对象的原始 message 字节；`message_with_1200_entries_round_trips_through_commit_tree` 末尾还断言 `.git/` 下没有残留的临时文件，以及对非基线 commit 报 `svn_baseline_no_trailer`

#### 计划的错误

- 无

#### 计划没写的坑

- **trailer 内容行不能 `trim_end`**：`trim_end` 会削掉 Unicode 空白，路径末尾的全角空格（U+3000）读回来就少一个字符。编码只处理 ASCII 空格和控制字符，所以解析只靠 `lines()` 去掉 `\r\n` 里的 `\r`。用例里加了一个以 U+3000 结尾的路径
- **新 trailer 写坏了不能退回旧格式解析**：最后一段只要有 `Svn-Base-` 行，就按新格式严格解析，失败返回 `None`。否则一个被拼接或改坏的 message 里残留的 `git-svn-id` 会被当成单值基线，那是读错而不是读不懂。新格式里还要拒绝这些：重复路径、同一路径既是 Rev 又是 Gone、Rev 小于 floor、最后一段里有不认识的行、revision 不是纯数字
- **摘要行可能伪造记账**：`parse_trailer` 和 `parse_delivered_rev` 都是逐行扫全文、按行首匹配的。所以摘要行若以 `git-svn-id:`、`Svn-Base-`、`Delivered-Rev:` 开头，前面垫两个空格；摘要里的控制字符一律换成空格，免得多出一段，让 trailer 不再是最后一段
- **`-F` 要给绝对路径**：`git_dir()` 在 `repo_root` 是相对路径时也返回相对路径，而 git 以 `repo_root` 为 cwd，会再解析一次。crate 的 `rust-version` 是 1.77.2，用不了 `std::path::absolute`（1.79），所以手工拼 `current_dir()`
- **`-m` 与 `-F` 的字节差异**：`commit-tree -m` 会补一个行尾 `\n`，`-F` 原样读。现行 message 本来就以 `\n` 结尾，所以两者一致；测试实建两个 commit 比较了原始字节
- `svn_deliver.rs:267` 的注释引用「`create_baseline_commit` 记过 stdin 的坑」。原注释被替换，新注释里保留了「不走 `-F -`（stdin 是 null）」这一条，那处引用仍然成立
- `read_base_map` 用 `cat-file commit` 取原始字节。message 不是 UTF-8 时直接判为读不出（`svn_baseline_no_trailer`），不走 lossy 转换，免得替换字符混进路径

#### 为什么这么改是安全的

- 单值路径：`format_message` 单值分支复用 `format_trailer`，测试同时比较了「手拼的旧格式字符串」和「旧 `-m` 实建的 commit 字节」。`svn_baseline` / `svn_init` / `svn_sync_in` / `svn_sync_out` / `svn_graph_contract` 全部不回归
- 自检只是把「录出一个读不懂的基线」提前成报错。svn 给出的 Repository Root 是 URI 编码的，不含空白；UUID 是十六进制。正常路径不会触发自检
- 临时 message 文件放在 `.git/` 下，用 `create_new` 创建，名字含 pid、纳秒和序号，不覆盖别人的文件。文件先建 RAII guard 再写内容，写入失败、`commit-tree` 失败、`?` 早退时都会删掉；测试断言了没有残留
- 没有动任何 ref、索引或工作区。测试全部用 `tempfile` 建的临时仓库

### Task 2：节点状态解析与映射计算

#### 偏离计划

- **`NodeStatus` 与 `parse_node_status` 放在 `svn_xml.rs`**，`svn_nodes.rs` 用 `pub use` 再导出。理由：`svn_xml` 是全项目唯一接触 XML 文本的地方（模块头的规矩）。两个路径（`svn_xml::` 与 `svn_nodes::`）都能引用
- **抽出 `svn_sync::status_targets`**：`read_node_status` 与失同步判定共用同一份 target 构造（白名单条目 + 每个 wc(P) 根），两边分家时判定与录制看到的就不是同一批节点。原 `node_revs` 的说明（`-v`、`--ignore-externals` 为什么一个都不能省）随之挪到这个函数上
- **`parse_node_status` 保留空集守卫**：一个带服务器 revision 的节点都没有时报错，错误码沿用 `svn_xml_empty_node_revs` / `svn_xml_all_nodes_uncommitted`。理由：漏了 `-v` 时干净工作副本得到空输出，基线里每个文件都会被判成「消失」
- **`compute_map` 在这里落实墓碑的保留规则**（计划只写了「传入 gone」）：已重新进入基线树的路径、以及删除时 HEAD ≤ F 的墓碑去掉（分析 §6.1「墓碑一直带着，直到 floor ≥ 墓碑的 revision」）。否则 `parse_message` 会拒绝「既在树里又是墓碑」的记账，周一根目录更新后也回不到单值
- **基线树为空时**（白名单目录里一个文件都没有），F 取节点里最小的服务器 revision；连这个都没有才报 `svn_map_unknown_node`
- 测试比计划多 5 条：`parse_node_status_refuses_output_without_externals_ignored`、`compute_map_takes_node_rev_when_commit_rev_is_missing`、`compute_map_refuses_a_file_without_a_node_revision`、`compute_map_drops_tombstones_the_floor_has_passed_or_that_reappeared`、`compute_map_too_large_lists_the_lowest_files`

#### 计划的错误

- 无

#### 计划没写的坑

- **去重**：外层 wc 的 status 里 external 根是一条 `item="external"`、没有 revision 的标记，它自己那个 wc 的 status 里才是带 revision 的目录节点；白名单条目互相嵌套时也会重复。合并时保留带节点 revision 的那条
- 实测再证 E14：changelist 成员在 `</target>` 之后与之平级的 `<changelist name>` 块里；E15：中途失败的 `svn update` 留下的锁出现在白名单目录 entry 上（`wc-locked="true"`），status 仍 rc=0
- `<commit>` 里没有 `<author>`（匿名提交）时作者记 `None`，不记空串

#### 为什么这么改是安全的

- 只新增只读的 `svn status` 调用与纯计算，没有任何写操作
- `parse_node_revs` / `parse_node_rev_entries` 原样保留，初始化、重建的 P2 不受影响
- 测试全部在 `hybrid_with_colleague` 等临时 fixture 里跑；跑实验用的 scratchpad 目录跑完即删

### Task 3：文件级失同步判定

#### 偏离计划

- **sync_in 的 A16 没有一律拒绝 `ExternallyAdvanced`**【待决 D6】。指令要求「暂时拒绝，Task 5 再接管」，但整体前进（周一在 TortoiseSVN 里对项目根 Update）是日常，有 5 条现成测试覆盖（`svn_sync_check`、`svn_sync_in` 的 catch_up 系列、`svn_sync_out` 的放弃后接管），而 `rebuild_baseline` 在「所有节点同一 revision」时本来就能录。一律拒绝就是功能回退。实现：`uniform_forward_rev` 要求基线是单值、白名单**所有**节点（含目录，与 P2 同一口径）只有一个 revision 且比 floor 新，满足就照旧 `catch_up_rev = Some(n)`——判定是 `InSync`（只动了白名单外的文件）时也接管，与改造前一致；其余 `ExternallyAdvanced` 报 `sync_in_out_of_sync`，文案提示先「跟进」
- **「消失」的口径**：R 的键在合并后的 node status 里找不到，**或者**找到的条目是 `unversioned` / `ignored` / `external`。后者的现实成因是外部 `svn rm --keep-local` 后提交：服务器上删了，磁盘上还留着
- **c(f) 缺失（replaced 等）不判前进**，只看 n(f) 是否回退。与 `compute_map`「缺失取 n(f)」不同：判定问的是「SVN 侧有没有新内容」，replaced 是本地改动（E13）；录制时取 n(f) 是保守标签
- **`Diverged` 取路径序第一个回退文件的 n(f) 与 R(f)**；`PinnedReset` 用这个 n(f) 去比 external 的 peg，与原「单值 n 等于 peg」同一判据。优先级：`WcLocked` > 回退 > `ExternallyAdvanced` > `InSync`
- **前端 `diverged` 的「前进」分支删掉**（后端不再产出），只剩红色「回退」。横幅新增 `BannerTone` `'quiet'`（复用常驻体检入口的灰色样式）和 `BannerModel.files`。`svnUpdate` 按钮暂时没有横幅产出它，工具栏的「更新」不受影响【待决 D7】
- 消失的文件在横幅里写成「工作副本里已没有」而不是「已在 SVN 上删除」：本地分不出提交删除与 depth-exclude 漏收（E17）
- **`check_sync` 的映射与文件清单取自同一个 OID**：先 `rev-parse` 出 OID 再分别读 message 与树，免得中间基线被移动时拼出一个不存在的 R
- 新增 `check_sync_with_nodes`（crate 内）：A16 要看所有节点（含目录），与判定共用一次 status
- 改了的既有测试：`svn_sync.rs` 的 `mixed_revision_is_reported_with_every_value` → `rolled_back_file_is_diverged`、`a_single_value_that_differs_from_the_baseline_is_diverged` → `whole_root_update_with_a_changed_file_is_externally_advanced`；`svn_init.rs` 的 `whenever_the_banner_says_mixed_revision_rebuild_is_refused` 改名为 `commit_without_update_is_externally_advanced_and_rebuild_is_still_refused`（Task 5 要改成断言重建成功）；`svn_server_set.rs` 的 `the_existing_checks_really_do_miss_a_depth_excluded_subtree` 改名为 `local_checks_cannot_tell_a_depth_excluded_subtree_from_a_deletion`，断言改为 `removed`；`svn_sync_in` / `svn_sync_out` 各一处断言从 `Diverged` 改为 `ExternallyAdvanced`；`svn_backend` 契约删掉 `MixedRevision`、补 `WcLocked` 与 `base`
- 测试比计划多：`colleague_added_w_file_is_reported_as_added_and_non_w_file_is_not`、`tombstoned_path_that_reappears_is_reported_as_added`（手工发布一个带墓碑的混合基线）、`rollback_wins_over_external_advance`

#### 计划的错误

- Step 1 `commit_without_update_is_externally_advanced_not_mixed` 写「`svn_repo_committed_without_update` 形态」直接用那个 fixture 造不出来：它在初始化**之前**就是混合版本，初始化的 P2 会拒绝，没有基线可比。改为三 revision 仓库初始化后再在 wc1 里只提交 B（同一形态）
- Step 3 说要改 `BaselineBanner` 里的 `mixedRevision` 消费点：组件只经 `describeSync`，没有直接引用。真正要改的还有计划没列的 `svn_server_set.rs`、`svn_init.rs` 两条把旧判据行为钉成回归的测试（见上）

#### 计划没写的坑

- **depth-exclude 现在会被判成 `removed`**（E17）：横幅会出现，这是新判据比旧的多看见的东西，不是误报；但它与提交删除同形，跟进前必须向服务器确认（Task 4 / 5），服务器上还在的要拒绝
- **`svnadmin setrevprop` 按本机代码页读文件**：写进中文作者会被当成 GBK 转码成乱码。只是 fixture 的限制（真实作者由服务器记），测试里用 ASCII 作者
- file:// 访问时 `svn commit --username` 不改作者；造「同事」作者只能改 revprop
- **全量 `cargo test` 要加 `--no-fail-fast`**：默认遇到第一个失败的测试二进制就停，后面的根本没跑，第一次全量跑时差点据此误判「只有一处回归」
- worktree 里没有 `node_modules`，跑前端验证前先 `pnpm install --frozen-lockfile`（只按锁文件装，不改依赖）
- 整体前进但没有 W-file 内容变化（同事只改了美术资源）现在判 `InSync`，横幅不再出现；基线标签仍是旧 floor，但 f@floor 等于 pristine，标签是真的。交付侧 `remoteRev == baselineRev` 那道门照样拦着

#### 为什么这么改是安全的

- 判定全是只读：`svn status`、`git rev-parse` / `cat-file` / `ls-tree`，外加 `.gitignore` 查询；不动 ref、索引、工作区
- 交付（sync_out）对非 `InSync` 照旧拒绝；外部前进、锁、回退、消失都拦在交付之前
- 更新（sync_in）只在与改造前完全相同的形态（单值基线、所有节点同一 revision）下接管，`rebuild_baseline` 的 P2 照样把关；其余混合形态拒绝，不会把外部改动录成服务器内容
- `MixedRevision` 删除后 Rust 与 TS 两侧的消费点都改了：`svn_sync_check`、`svn_sync_out_check`（`matches!` 不变）、`commands/svn.rs`（只透传）、`svnSync.ts`、`BaselineBanner.tsx`；契约测试钉住 `externallyAdvanced` / `wcLocked` / `base` 的 camelCase 形状与 `author: null`
- 测试全部在临时 fixture 里；锁的场景用独占打开文件让 `svn update` 半途失败（与 `svn_repo_interrupted_update` 同一造法），只在 Windows 上编译

#### 遗留风险

- 锁只在 status 覆盖到的目录上可见：TortoiseSVN 只锁白名单外的目录时看不到，但那也不影响白名单
- ~~跟进按钮要到 Task 11 才接线；这期间混合版本（非整体前进）只有横幅提示、没有一键出口~~ Task 11 已接

### Task 4：分组体检、墓碑与服务器侧核对

#### 偏离计划

- **TDD 顺序**：先写了实现、再写测试，Step 2「确认失败」没有按字面做。补救：新测试全部依赖 Task 4 才有的接口（`check_baseline_map`、`plan_diffs`、`confirm_absent`、`audit_server_set_map`），并做了两次变异核对——去掉服务器侧核对的墓碑剔除，`server_audit_ignores_tombstoned_files` 变红；关掉被排除节点的扫描，`excluded_file_is_refused_not_tombstoned`、`excluded_directory_is_refused_file_by_file` 变红。另外 `grouped_check_verifies_commit_without_update_baseline`、`committed_deletion_becomes_tombstone_when_just_recorded` 里都带了「旧的单值体检在同一状态下报 Lying」的对照断言
- **`single_rev_map_uses_exactly_the_legacy_command` 的观测手段**：仓库里没有记录 svn 命令的机制。按指令取最小侵入的做法：把「发哪些 diff」抽成纯函数 `svn_health::plan_diffs(units, map, skip, excluded) -> DiffPlan`，体检本体和单值包装都只经它取命令。测试断言单值映射每单元恰好一条、参数逐字等于改造前（不带 `--ignore-properties`、不带尾随 PATH），嵌套 external 两单元各一条
- **`check_baseline` 单值包装不产生墓碑**：内部 `check_grouped(.., tombstoning)`，`check_baseline` 传 `false`，`check_baseline_map` 在 `JustRecorded` 时传 `true`。所以 `check_baseline(.., JustRecorded)`（init / rebuild 现在的调用方）行为与改造前完全相同：floor 组没被解释的 D 照旧是 Lying，不问 HEAD、不扫被排除节点。Task 5 改用 `check_baseline_map` 时才会产生墓碑
- **E17 的「漏收」不是从整单元 diff 的 D 来的**（见「计划的错误」）。`JustRecorded` 时每个单元额外跑一次 `svn info -R --xml -- <wc_root>@`（纯本地读，E19），找出 `<depth>exclude</depth>` 的节点，用 floor revision 把它们**点名**再 diff 一次（`-- <PATH>`），报出来的 D 与整单元的 D 走同一条「向 HEAD 确认」：仍存在 → `Excluded`，已不存在 → 墓碑。排除掉的是非 W-file（美术）时不拒绝（`excluded_non_code_file_does_not_refuse_recording`）
- **点不了名的被排除节点**（就是单元根本身，或路径含非 ASCII 字符）：~~文件按 W-file 口径筛后直接算 `Excluded`，目录不知道里面有什么，一律算 `Excluded`【待决 D8】~~ Review 1 第 3 条改为 `svn ls` 列出 floor 上的内容再筛
- **含非 ASCII 字符的 PATH 不走尾随参数**，改为该组一条整单元 diff、只看这些文件（见「计划没写的坑」）。E18 的「每组一条带尾随 PATH」只对 ASCII 路径成立
- **丢失 / 未版本化的 PATH 剔除后报进 `unverified`**：计划只说剔除。它们没被比过，`Verified` 不能把它们算成覆盖到了（与 `Unknown` 时报 S 同一个理由）。`JustRecorded` 的新映射里不会出现这类路径（录制时丢失的在 S 里，未版本化的进不了 `compute_map`），只有旧映射才会碰到
- **结论的优先级**：Lying（各组 `D \ S`）> 已有墓碑被证伪（Lying）> `Excluded` > `Verified { new_gone }`。有 Lying 时不再向 HEAD 确认候选 D：录制反正要失败，候选里多半是正常的提交删除，混进 Lying 清单会误导
- **`HealthMap::Verified.health` 不是 `Verified` 时，`new_gone` 恒为空**；`HealthMap` 没有派生 `Serialize`（还不过 IPC，Task 5 / 11 用到时再加）
- **服务器侧核对**：新增 `audit_server_set_map(repo, &BaseMap)`（floor 的清单、剔除 `gone` 的键）；`audit_server_set(repo, rev)` 保留为不剔除的单值形态；`audit_server_set_at_baseline` 改读完整记账（新增 `svn_sync::read_baseline_map`），混合基线现在也核对得了（之前 `read_baseline_rev` 只认 `git-svn-id`，混合基线一律 `NotChecked`）
- **`server_head_rev` 放在新模块 `svn_tombstone`**，交付侧的 `remote_head_rev` 改为调同一个 `server_head_rev_with`，错误码 `sync_out_server_unreachable` 与文案逐字不变
- **`svn_sync::read_baseline_rev` 暂时没有调用方**，加了 `#[allow(dead_code)]` 并注明理由：计划 Task 5 要把其余读 trailer 的地方统一换成它（返回 floor），不在本 Task 删
- 测试比计划多：`excluded_directory_is_refused_file_by_file`、`excluded_non_code_file_does_not_refuse_recording`、`newer_group_paths_are_batched_under_the_command_line_limit`、`non_ascii_path_in_newer_group_is_checked_by_whole_unit_diff`、`unversioned_path_in_newer_group_is_skipped_and_reported_unverified`、`newer_file_inside_a_nested_external_is_checked_against_that_unit`、`confirm_absent_matches_by_url_and_tolerates_missing_targets`、`confirm_absent_refuses_a_revision_the_server_does_not_have`；`svn_tombstone` 内两条单元测试（错误码提取、URL 编码往返）

#### 计划的错误

- **「`Excluded`：这些 D 在服务器 HEAD 上仍然存在（漏收，E17）」的前提不成立**。实测（SlikSvn 1.14.2）：`--set-depth=exclude` 掉的文件或目录，整单元 `svn diff --old=<unit>@F --new=<wc_root> --summarize` **根本不报**，floor 组里也就没有对应的 D；只有把它作为尾随 PATH 点名时才报 `deleted`（目录会展开成里面的每个文件）。照计划原样实现，被排除的代码文件会静默地既不进基线、也不成墓碑。改法见「偏离计划」第四条
- **`tombstone_that_reappeared_on_server_is_lying` 的字面场景造不出来**：墓碑 `g → d` 核对的是 `g@d`，服务器上某个 revision 的内容永不改变，`d` 之后重新加回来不会让 `g@d` 存在。测试改为「r4 删、r5 加回，记成 `B → 5` 的墓碑是假话、被证伪；`B → 4` 仍成立」。重新出现在**工作副本**里的情形由 Task 2 的 `compute_map`（去掉墓碑）与 Task 3 的「墓碑路径重新出现算外部新增」负责

#### 计划没写的坑

- **svn.exe 按系统代码页取命令行参数**：开发机代码页 1252，`svn diff ... -- "sp ace/中文 %x@y.cs"` 报 `E155010: The node '…\sp ace\?? %x@y.cs' was not found`（Git Bash 与 PowerShell 两种方式都一样）。中文 Windows（936）上中文能过，但日文、emoji 之类照样不行。所以：尾随 PATH 只放纯 ASCII 的；`svn info` 走 `--targets` 文件，URL 全部百分号编码成 ASCII
- **`svn diff --summarize --xml` 输出的路径是 URL 编码的**（`sp%20ace/%E4%B8%AD…`，`%` 是 `%25`，`@` 不编码）。改造前剥掉单元 URL 前缀后**直接**当仓库相对路径用，含空格 / 中文 / `%` 的文件对不上 S，报给用户的也是编码后的样子。现在剥前缀后先解码，单值体检也受益（这是对旧行为的修正，不是回归：只影响含这些字符的路径，以前在这些路径上要么误报 Lying、要么报出编码后的名字）
- **`confirm_absent` 必须按 (url, revision) 对回**：同一路径在两个 revision 上各问一次时（一个在、一个不在），只按 url 对回会把不在的那条也当成在。`svn info` 对 `URL@N` 的 entry 的 `revision` 属性就是 N（与最后修改 revision 无关）
- **E160006（revision 不存在）时 stdout 的 XML 被截断在 `<info>` 之后**，解析会失败；先看 stderr 的错误码再解析。stderr 里除 W170000 / E200009 以外的错误码一律报错；W170000 的条数必须等于没出现在 stdout 里的 target 数，对不上就报错，不猜
- 被排除的节点：`svn info -R --xml` 照样列出 entry，`<wc-info><depth>exclude</depth>`；`status -v` 里没有（再证 E17）。`svn info -R` 不下沉进 external，每个单元各跑一次
- 尾随 PATH 指向磁盘丢失（`!`）的文件不报错、也不报差异；指向已 `svn rm` 计划删除的文件照常报 `deleted`（实测）。所以 skip 只需要剔未版本化 / ignored / 找不到的，丢失的剔不剔都一样，为了「如实报没比过」一并剔掉
- **提交删除一个目录**时，整单元 diff 逐个报目录下的文件 `deleted`（外加一条 `kind=dir`）。墓碑因此是逐文件的，不需要目录条目
- ~~`--targets` 临时文件放系统临时目录（`std::env::temp_dir()`），不放 `.git/`：体检的测试与部分调用方（纯 SVN fixture）不是 git 仓库。`create_new` + RAII 删除，内容是纯 ASCII，不受代码页影响~~ 这里只考虑了文件内容，漏了文件**路径**同样要过代码页。Review 1 第 7 条改为优先放 `.git/`、传相对路径，不是 git 仓库才退回系统临时目录
- crate 的 `rust-version` 是 1.77.2，`Option::is_none_or`（1.82）不能用

#### 为什么这么改是安全的

- 全部是只读的 svn 命令：`diff`、`info`（含 `-R`、`--targets`）、`status -v`，E19 已证不写 wc.db；不动 ref、索引、工作区。唯一的写是系统临时目录里的一个 targets 文件，用完即删
- 单值路径的命令与判据不变：`check_baseline` 不产生墓碑、不问 HEAD、不扫被排除节点，`single_rev_map_uses_exactly_the_legacy_command` 钉住命令，`svn_health` 原有 20 条、`svn_health_dirty`、`svn_init`（init / rebuild 经 `check_baseline`）全部不回归。唯一的行为差异是结果路径解码（见上）
- 墓碑只在向服务器确认「`<url>/<rel>@<HEAD>` 不存在」之后才产生，HEAD 用确认时的同一个数字作 peg，不用 `@HEAD` 字样，避免确认与取值之间服务器前进；revision 不存在、输出对不上、网络失败都不会当成「不存在」去立墓碑。**更正（Review 1 第 1 条）**：这里原写「都落到 `NotChecked` 或 `Err`」，只说了不立错墓碑，没看到另一面：`NotChecked` 会被录制放行，已知的 D 静默丢失。现在这些失败在录制时是 `HealthMap::Unconfirmed`，拒绝录制
- 服务器侧核对只多剔除墓碑路径；墓碑由体检核对过（或手工发布），真正的漏收（depth-exclude）照样报出来，`server_audit_ignores_tombstoned_files` 最后一段钉住了这一点
- 测试全部在 `tempfile` 的临时 fixture 里；实验在 scratchpad 的临时仓库里做，没有碰任何真实仓库

#### 遗留风险

- 被同事在 r_d 删除、又在更晚的 revision 加回来的文件，若本机只局部更新到 r_d（工作副本里没有它），HEAD 上它仍存在 → 被判 `Excluded`、拒绝录制，文案会说成漏收。方向保守（不立错误的墓碑），用户 Update 后即可。要更准需要按父目录节点 revision 去问，但提交删除不推高父目录（E7），那个判据对最常见的形态反而不成立
- ~~被排除的节点若在 floor 上还不存在（之后才加进来又被排除），点名 diff 可能整条失败 → `NotChecked`，录制会因此停下（未实测，形态罕见）~~ **两处都写错了**（Review 1 第 5 条）：实测点名 diff rc=0、输出为空，结果正确；而且 `NotChecked` 并不会让录制停下，`record_mixed_with` 接受它，这正是第 1 条缺陷的成因
- `JustRecorded` 多一次每单元的 `svn info -R`（本地读，规模与 `status -v` 相当）；有单列文件时多一次 `status -v`。Task 5 若在录制时已有 node status，可以考虑把 skip 由调用方传入，省掉这一次

### Task 5：录制泛化（重建、初始化、sync_in 接管、交付拒绝）

#### 偏离计划

- **`run_preflight` 没有加参数**，新增 `run_preflight_with(repo, whitelist, allow_mixed)`，`run_preflight` 成为 `allow_mixed = false` 的包装。理由：`tests/svn_preflight.rs` 等十几处调用点不用改，单值 P2 的测试原样钉住旧行为（同 Task 1 的取舍）。`allow_mixed = true` 时不做 P2，读一次 `read_node_status` 存进 `Preflight.nodes`（`#[serde(skip)]`，契约测试断言它不过 IPC），并做 R1 的两条补充拒绝：SVN 冲突（`item=conflicted`、`tree-conflicted`、`props=conflicted`）报新码 `svn_preflight_conflicted`；任一目录 `wc-locked` 报 `svn_preflight_operation_in_progress`（沿用 P3 的码）。`node_rev` 此时取所有节点（含目录）的最小值，只给向导第 2 步展示
- **`record_mixed(repo, cfg, idx)` 之外另有私有的 `record_mixed_with(.., pre, idx, svn_url, clock, tail)`**：初始化与重建在 `git add` 之前已经跑过 R1（还要拿 `pre.dirty` 做 `.gitignore` 检测与纳入集合预检），不再跑第二遍 `svn status`；`tail` 是拒绝文案末尾「此刻仓库处在什么状态」那句（初始化「整个回退了」、重建「基线指针没有改动」、S7「基线没有推进」）。公开的 `record_mixed` 自己跑 R1，给 S7（以及之后的跟进）用
- **R3b 复用 R2 的节点状态**：`svn_health` 新增 `check_baseline_map_with_nodes(.., nodes: Option<&[NodeStatus]>)`，`check_baseline_map` 是传 `None` 的包装；`check_grouped` 的两个开关收进私有结构 `Grouped`（否则 8 个参数触发 clippy `too_many_arguments`）。省掉了 Task 4 遗留风险里说的那一次 `status -v`
- **墓碑合并**：R3b 产生 `new_gone` 时用同一份节点状态再调一次 `compute_map(.., new_gone)`，墓碑的保留规则（不在树里、`d > floor`）与条目上限只在 `compute_map` 一处
- **R3c 比的是映射，不是逐节点 revision**：按同一规则重算（`gone` 传空）后与 R2 时的映射比较，`compute_map` 算不出来（文件在 status 里消失）也算被改动。局部 update 只推高没改过的文件时映射不变、录下的内容仍对，不该中止
- **初始化的体检挪到发布之前**：原来是「发布 → read-tree → 刷新 stat → 体检」，体检 Lying 时回退整个 `.git`。现在体检在 `record_mixed_with` 里、发布之前；失败照样走 `build_repo` 的唯一回退点。耗时表阶段名「基线体检（svn diff）」不变（`init` 的阶段名测试照过），只是顺序提前。初始化因此也多了一道原来没有的 R3c
- **S6–S7 只在不是单值时才走 `record_mixed`**：先照旧 `run_preflight`（单值），报 `svn_preflight_mixed_revision` 时才改走 `record_mixed`。单值时命令、判据与改造前逐字相同（S7 原来就不做体检，单值路径继续不做）；混合时带体检与墓碑。`record_mixed` 的 `svn_rebuild_revision_moved` 在 S7 里换成原有的 `sync_in_revision_moved`。抽出了 `capture_single` 与 `moved_error`
- **`create_baseline_commit` / `publish_baseline` 改收 `&BaseMap`**（Task 1 留下的待办），新增 `impl From<&Trailer> for BaseMap`；测试调用点：`tests/svn_baseline.rs` 的两个 helper 改为返回 `BaseMap`，`tests/svn_sync_in.rs` 两处 `&(&t).into()`
- **`catch_up_rev` 的取值**：整体前进时仍是那个 revision（与改造前一致）；其余 `ExternallyAdvanced` 取「基线树 − removed + added」这些文件节点 revision 的最小值（`floor_after_catch_up`，与 `compute_map` 同一规则，只给确认框看）。只提交不更新时它等于 `baseline_rev`，前端 `describeSyncInPreview` 为此加了一句不说「更新到了 r1」的文案
- **sync_out 的混合拒绝放在最前面**（读完 `svn/base` 的 OID 就查，在「分支是否包含最新基线」之前）：纯本地、最根本；放后面时混合基线上的分支多半先撞上 `sync_out_behind_baseline` 或 `sync_out_server_ahead`，给的理由都不对
- **6 处读 trailer**：`check_current_baseline` → `read_base_map` + `check_baseline_map(Unknown)`（记账与树取自同一个 OID）；`svn_sync::read_baseline_rev` 改为返回 floor（去掉 Task 4 的 `#[allow(dead_code)]`）；`svn_sync_in::baseline_rev` 调它；`svn_sync_out::trailer_rev`、`svn_sync_out_check` 的 Repository Root 改读 `read_base_map`（删掉 `repos_root`，原错误码 `svn_baseline_trailer` 随之变成 `svn_baseline_no_trailer`，前后端都没有消费它）；`svn_server_set` 在 Task 4 已改。`graph.rs`、`svn_log.rs` 保留 `parse_trailer`
- **前端**：`RebuildReport` 加 `base: BaseMapSummary`、文档改为 floor；`wizardErrorHint` 新增 `svn_preflight_conflicted`、`svn_baseline_excluded_paths`，`svn_rebuild_revision_moved` 的引导改成不点名「重建基线」（初始化也会报它）；`svn_preflight_mixed_revision` 条目保留并注明兼容旧报错。`svnSyncOut.ts` 没有错误码映射表，不用改
- **改了的既有测试**：`svn_init.rs` 的 `commit_without_update_is_externally_advanced_and_rebuild_is_still_refused` 按 Task 3 的预告改成 `rebuild_accepts_commit_without_update_and_writes_mixed_trailer`；`svn_sync_check.rs` 的 `a_mixed_revision_working_copy_is_still_refused` 改成 `a_commit_without_update_previews_a_catch_up_at_the_new_floor`（D6 收口）；`svn_backend.rs` 的 `Preflight` / `RebuildReport` 契约补 `nodes`（断言不过 IPC）与 `base`
- **TDD**：Rust 侧 9 条新 / 改测试先写、先跑，全部红（重建冲突那条红在 `expect_err`：改造前冲突文件被当成普通改动录进去、重建成功）。前端两条 vitest 是和实现一起写的，没有单独跑红；两条在改造前都必然失败（旧文案含「更新到了 r1」；新错误码在旧表里返回空串）

#### 计划的错误

- Interfaces 写「`check_sync_in`：`ExternallyAdvanced` 时 `catch_up_rev = Some(新 floor)`」——「新 floor」在预览时还没录，只能预估，见上「`catch_up_rev` 的取值」

#### 计划没写的坑

- **交付的混合拒绝不能放在 `require_on_latest_baseline` 之后**：重建之后工作分支还在旧基线上，先撞上的是 `sync_out_behind_baseline`（「请先更新」），用户照做会去点更新，而真正的原因是基线混合
- **初始化多出来的 R3c 与「整体回退」的关系**：初始化原来没有 R3c；现在录制期间 TortoiseSVN 提交一次会让初始化回退，报 `svn_rebuild_revision_moved`。码名带 rebuild 是历史原因，前端引导改成通用措辞
- **只提交不更新时 `catch_up_rev == baseline_rev`**：前端原文案「工作副本已经在外部更新到了 r{catchUpRev}，基线还停在 r{baselineRev}」会说成「更新到了 r1，基线还停在 r1」
- **sync_in 预览不再在「非整体前进」时拒绝**，于是 depth-exclude 造成的 `removed` 现在要到执行的接管（`rebuild_baseline` 的 R3b）才报 `svn_baseline_excluded_paths`。此时还在 S0 之前、没写 journal，什么都没动；但用户是在点了确认之后才看到

#### 为什么这么改是安全的

- 单值路径的行为：重建与初始化的 R1 多了冲突 / 写锁两条拒绝（冲突原本被静默录进基线，是修正）；体检从 `check_baseline` 换成 `check_baseline_map(JustRecorded)`，单值映射发的 diff 命令逐字相同（`single_rev_map_uses_exactly_the_legacy_command`），多出的是每单元一次本地 `svn info -R` 与「D 向 HEAD 确认」；trailer 单值时与旧格式逐字节相同（Task 1）。S7 单值路径一行没改。D7（交付后录制）没改
- 混合基线只会在 R3b 通过之后发布：Lying、漏收拒绝；墓碑只在服务器确认不存在后产生（Task 4）；R3c 复查映射；发布照旧以旧 OID 为条件
- 「任何 ExternallyAdvanced 都接管」：接管是 `rebuild_baseline`，不碰 HEAD、真实索引与磁盘，只以旧 OID 为条件移一个 ref；之后锁内复核照旧。原来 5 条整体前进测试与 `a_catch_up_is_refused_when_the_branch_moved_since_the_preview` 全过
- 交付在混合基线上一律拒绝，且在任何网络调用与 D1 计算之前；不改 ref、不写 journal（测试断言 `for-each-ref` 不变）
- 测试全部在临时 fixture 里；没有碰本仓库或任何真实仓库

#### 遗留风险

- S7 遇到混合（S5 之后仍不是单值）走 `record_mixed` 这条路径**没有专门的测试**：要造「`svn update` 之后白名单仍是混合版本」需要钉住的 external（已被预检拒绝）或 Windows 上占住文件让 update 跳过，造法不稳定。路径本身与重建共用 `record_mixed_with`，由重建的测试覆盖
- R3c（映射在录制期间被改）没有能稳定触发的测试，与改造前的 R3c 一样只靠代码审查
- 向导第 2 步的「记账 revision」在混合版本时显示的是所有节点（含目录）的最小值，比真正的 floor 可能更小；没加「混合版本」字样（要改 IPC，留给 Task 11 一并做）
- depth-exclude 的拒绝在 sync_in 里从预览挪到了执行（见「计划没写的坑」末条）

### Task 6：对象层重放引擎

#### 偏离计划

- **`commits_between` 多一个 `codes: &ReplayCodes` 参数**：读不懂 `git log` 输出、OID 不完整时的错误码要落在调用方自己的码上（交付原来是 `sync_out_replay_failed`），引擎不替交付和跟进选码
- **`ReplayCodes` 多一个 `too_old_message`**：交付的「git 太旧」文案里有「把本地补丁移到分支顶端（那样不需要重放）」这种只有交付说得清的出路，必须原样保留；跟进会在 F1 之前自己预检 git 版本，文案由 Task 10 定。`conflict` 码引擎自己不用（冲突走 `ReplayOutcome::Conflict`），交付拿它组装 `sync_out_local_only_conflict`
- **冲突解析只抽了「stage 行 → 文件列表」**：「点名是哪个本地补丁」与那段文案是交付专有的，留在 `svn_deliver::replay_conflict`，改为收 `files`
- **`rewritten` 只列「P 里 C 真正改过的路径」**（C 与 C^ 的条目不同）：C 没改的路径换不换结果一样，列出来报告就说谎了。没有这样的路径时 C' = C，不建临时索引
- **memo 命中**：值等于当前父提交 ⇔ 上次是丢弃（新 commit 不可能等于它自己的父提交），照样记进本次的 `dropped`；否则核对「值的父提交 == 当前父提交」，对不上报 `codes.failed`（不同 onto 共用一份 memo 是调用方的编程错误，不猜）。命中时 `rewritten` 照样重算（两次 `ls-tree`），每条分支的报告各自完整
- **`replay_onto` 拒绝 merge commit**（`codes.failed`）：线性由调用方先查（交付 §8.1 第 6 条、跟进 `assert_linear`），这里是兜底，不静默压平
- **`commit_like` 改走 `commit-tree -F <临时文件>`**（Global Constraints / W27）：内容按 `-m` 的规矩补一个结尾换行（commit-tree 对 `-m` 做 complete_line、对 `-F` 不做），与改造前逐字节相同；顺带去掉 message 超过 Windows 命令行上限时交付会失败的问题。为此 `svn_baseline::TempFile` 改为 `pub(crate)`，加 `in_git_dir` / `abs_path`；`TempIndex` 加 `abs_path`。既有调用点行为不变
- **新增 `no_superseded`**（交付用的空 P）
- **计划外多 4 条测试**：`originally_empty_commit_is_kept_and_drop_empty_off_keeps_everything`、`superseded_path_absent_in_parent_is_deleted_from_c_prime`（W19 的 mode 0 删除）、`superseded_directory_path_is_an_internal_error`、`replay_refuses_merge_commits`
- **测试在对象层造 commit 与 NEW**（临时索引 + `commit-tree`，不经过工作区），fixture 仍是 `hybrid_with_colleague`（真实的 `svn/base`），没有新增 fixture。被测对象只看对象；W10-R2、W14 这种「同一行先后两个版本」的形态在对象层摆最精确。真实的 TortoiseSVN 提交 + 录制 NEW 留给 Task 10 的端到端测试

#### 计划的错误

- Interfaces 写「`ReplayCodes` 给出冲突 / git 太旧 / 失败三类错误码」：冲突按要求不走 `Err`；git 太旧只给码不够，交付的文案也得由调用方给（见上）

#### 计划没写的坑

- **判空要排除「原本就空」**：要比 C 与 C^ 的树。只在结果树 == 当前父树时才查（多一次 `rev-parse`），不拖慢常态
- **P 里的目录要在 C 和 C^ 两侧都查**：`ls-tree` 不带 `-r` 时，目录列出的是它自己的 tree 条目，正好用来拒绝；只查一侧会漏掉 D/F 形态（C^ 里是文件、C 里同名变成目录，或反过来），那正是写坏临时索引的形状
- **`--index-info` 删除行**写 `0 <与 OID 等长的全零>\t<path>`（`superseded_path_absent_in_parent_is_deleted_from_c_prime` 覆盖；替换与删除混在同一次输入里的形态没有单独测）
- **`--literal-pathspecs` 在 `ls-tree` 上不是唯一防线**：变异核对时去掉它，`[1]` 那条测试仍过——`ls-tree` 的结果本来就按 P 精确过滤（只认完全相同的路径）。flag 按全局约束保留，测试守住的是端到端行为
- **「S' 相同」不足以证明 memo 生效**：`commit_like` 取当前时间作提交者时间，同一秒内重放两次碰巧得到同一个 OID。测试另外预置一个哨兵映射，证明 memo 真被查了
- **`all_committed` 的 Local-Only 检查用 `log --format=%B%x1e O..HEAD`**（区间含 merge），与 W20 写的 `rev-list` 等价；`O..HEAD` 改过的路径为空时不跑第二次 `diff-tree`，Local-Only 照查（改了又改回去的补丁）
- **变异核对**（各改一处、跑 `svn_replay`，均已还原并逐字节比对）：判空改成与 onto 比 → `drop_empty_compares_with_current_parent_not_onto` 红；不排除原本就空 → `originally_empty_…` 红；不查 memo → `shared_commit_replayed_once_across_branches` 红；去掉 ls-tree 的 `--literal-pathspecs` → 全绿（见上）

#### 为什么这么改是安全的

- **交付不变**：`drop_empty = false`、P 为空时，`replay_onto` 发出的 git 命令与改造前逐条相同（同样的 `merge-tree --write-tree -z --merge-base`、同样的 `log -1 --date=raw` 读元数据），唯一变化是 `commit-tree` 的 message 从 `-m` 改成内容相同的 `-F` 文件；冲突的 stage 行解析原样搬过去；错误码与文案（`sync_out_local_only_conflict`、`sync_out_git_too_old`、`sync_out_replay_failed`、「没有父提交」「git merge-tree 退出」）逐字不变。`svn_deliver`、`svn_sync_out`、`svn_sync_out_check` 全过
- **重放只写对象**：merge-tree、commit-tree、临时索引上的 `read-tree` / `update-index` / `write-tree`。不写任何 ref，不碰真实索引与磁盘；临时索引与 message 文件在 `.git/` 下、绝对路径、RAII 删除
- **C' 只动 P 里 C 真正改过的文件条目**；目录、空路径、带反斜杠或尾斜杠的路径一律报错，不会写坏临时索引
- 测试全部在临时 fixture 里；没有碰本仓库或任何真实仓库

#### 遗留风险

- `require_oid` 只认 40 位十六进制（与交付原有的检查一致），SHA-256 仓库会被拒绝
- 没有 `encoding` 头的非 UTF-8 commit message 经 `log %B` 与 `from_utf8_lossy` 会被替换成 U+FFFD——交付原有行为，未改
- P（`superseded`）由 Task 10 按作者与失焦快照计算；本 Task 只保证「给的是文件路径就正确换、给了目录就报错」
- 跟进用的 `ReplayCodes`（码名与文案）留给 Task 10 定

### Task 7：Local-Only 泄漏按 hunk 判定

#### 偏离计划

- **用 `Patch::from_buffers` 而不是 `from_blobs`**：同一个 libgit2 diff 引擎，但 `from_blobs` 两侧都要求有 blob，L 新增的文件（B 侧不存在）没法传；读 blob 内容后统一走 `from_buffers`，B 侧缺失时当空文件。`DiffOptions` 设 `context_lines(0)`、`interhunk_lines(0)`、`force_text(true)`（二进制在调用前自己判掉，不让 libgit2 再按内容猜一次）
- **纯删除的 hunk 比原型收紧**：`hunkcheck.pl` 拿空的 `+` 行去比，永远相等——L 删第 15 行、NEW 把第 15 行改成别的，原型也判 Present。改为：重叠的 NEW hunk 全落在 L 的区间内、且净删掉的正好是这几行 → Present，否则 → Unknown（宁可多一条告警，不漏报）。`+` 行非空的 hunk 仍按原型只比 `|+|` 行：若改成「按区间长度比」，NEW 在同一处多插一行就会把泄漏判成 Absent，是危险方向
- **整文件判定的坐标约定**：二进制 / 非 blob、L 删了文件且 NEW 也删了，没有行号可给，记一条 `HunkLeak { old_start: 0, lines: 0 }`（与「从空文件新增」的 hunk 头 `@@ -0,0` 同形）。`HunkLeak.lines` 取 L 的 hunk 头 `-old_start,old_lines` 里的 `old_lines`（纯插入为 0），`old_start` 是 git 的原值
- **路径级规则**（计划只写了 hunk 级）：NEW 里该路径与 B 相同 → 不可能泄漏；L 删、NEW 也删 → Present；L 删、NEW 只是改了 → Absent；L 改 / 增、NEW 删 → Absent；L 新增、NEW 也新增 → B 当空文件按 hunk 比；任何一侧是二进制或不是 blob（gitlink、文件变目录）且 NEW 改过 → 整文件 Unknown
- **`Leak` / `HunkLeak` / `LeakVerdict` 先派生了 `serde::Serialize`（camelCase）**：Task 10 的 `FollowReport.leaks` 要过 IPC。本 Task 没有命令暴露它们，`svn_backend.rs` 的序列化契约测试与前端类型留给 Task 10 / 11 一起加
- **场景多一个**：计划列的六个是 y4.sh 的 s1–s6（第六个「前面插入 3 行后不泄漏」在 y4 里是 s6：插入 3 行后只有 line30 上去、line5 不泄漏）。为免歧义另加「前面插入 3 行后只提交了功能改动」→ 无泄漏，共 7 个用例
- **计划外多 8 条测试**（其中二进制与「两边都删」两条是派活时建议的）：`overlapping_earlier_hunk_that_changes_line_count_is_unknown`（Unknown 规则本身，六个场景里没有一个走到它）、`pure_insertion_and_deletion_hunks_are_mapped`（`oc = 0` 的坐标与上面的收紧）、`unsubmitted_feature_below_local_only_maps_through_its_reverse_diff`（L 的父提交不是旧基线，见「计划没写的坑」）、`paths_outside_changed_paths_are_ignored`、`binary_change_on_both_sides_is_unknown`、`deleted_in_both_is_present`、`path_deleted_in_new_is_absent`、`file_added_by_local_only_and_by_new_with_same_content_is_present`
- **测试 fixture 是纯 git 临时仓库**（`TempDir` + `git init`，写在 `tests/svn_leak.rs` 里），没有用 `svn_fixtures.rs` 的 SVN fixture：被测函数只读三个 commit 的对象，与 SVN 无关；用 `hybrid_with_colleague` 只会多起 svnadmin / checkout / `init_hybrid`，还会让本测试被并行修改中的 `svn_init.rs` 牵连。仍在临时目录、对象层造 commit（同 `svn_replay.rs` 的 `commit_on`，`hash-object` 加 `--no-filters`），跑完即删

#### 计划的错误

- 无。（Interfaces 的 `HunkLeak { old_start, lines }` 没说坐标是哪一侧、整文件判定怎么表示，按上面的约定补齐）

#### 计划没写的坑

- **零上下文 hunk 头里纯插入的 `old_start` 是插入点之前那一行**（可以是 0），B 侧区间要换成 `[old_start + 1, old_start + 1)`。libgit2 与 `git diff -U0` 一致，`pure_insertion_and_deletion_hunks_are_mapped` 与 `file_added_…` 钉住（变异核对：不加 1 → 这两条红）
- **同一插入点的两个纯插入不能算「在前面」**：否则 L 与 NEW 在同一处插同样内容时，NEW 的插入被计进偏移，映射错位一行（原型里也有这条例外；变异核对：去掉 → 同上两条红）
- **L 的 B 是它自己的父提交，不是旧基线**：分支上 L 之前还有没提交的功能 commit 时，B→NEW 的 hunk 会包含这些改动的反向差异，偏移可以是负的。W21 的六个场景里 B 都等于旧基线，走不到这个形态。M4 只用它做行号映射，位置照样对：`unsubmitted_feature_below_local_only_maps_through_its_reverse_diff` 钉住「B 的第 8 行对应 NEW 的第 5 行」。与 L 的 hunk 重叠时仍走 Unknown 或内容比较，只有内容真的相同才判 Present
- **变异核对**（各改一处、跑 `svn_leak`，脚本 `leak_mutate.sh` 在会话 scratchpad，跑完已还原并 `cmp` 逐字节比对）：去掉 Unknown 规则 → `overlapping_earlier_…` 红；去掉偏移 → W21 场景、插删用例与 `unsubmitted_feature_…` 红；去掉同插入点例外 / 纯插入不加 1 → 插删用例与新增文件用例红；纯删除退回原型 → 插删用例红；不跳过「NEW 与 B 相同」→ 二进制用例的对照红

#### 为什么这么改是安全的

- 只读对象：`Repository::open` + `find_commit` / `find_blob` / `diff_tree_to_tree` / `Patch::from_buffers`，不写 ref、索引、磁盘与对象库
- ~~误判方向偏保守：拿不准（映射位置不确定、纯删除对不齐、二进制、非 blob）一律 Unknown；只有「NEW 没碰这几行」与「NEW 对应位置的内容与 L 不同」才判没泄漏~~ **Review 2 更正：这句是错的。**「映射位置的内容不同」是漏报的方向：功能行与补丁落在同一个 hunk 时，补丁不在映射位置上却已经上了服务器（探针 a、b），半个 hunk 上服务器时整段比较直接判没泄漏（探针 c）。现在只有「重叠的 NEW hunk 里一条 `+` 行都查不到」才算没泄漏，见「Review 2 修复」第 5 条
- 不新增依赖（git2、serde 已有）；测试全在临时仓库里

#### 遗留风险

- **只改 mode 的 Local-Only commit**（例如本地加了可执行位）不检查：没有 hunk，路径级规则也不比 mode
- **同事独立写出了同样的行**：NEW 对应位置的内容与 L 的 `+` 行相同就判 Present，分不出是 Local-Only 补丁被带上去，还是别人碰巧提交了一模一样的改动。属可接受的误报（方向安全）
- **非 UTF-8 路径**按 `from_utf8_lossy` 转换，对不上 `changed_paths` 时会被跳过（与仓库其他按 String 存路径的地方一致）
- **大文件**：每个命中的路径要把三个 blob 读进内存、做两次 diff；只对「L 改过 ∩ 外部变更」的路径做，通常很少

### Task 8：跟进时拼索引

#### 偏离计划

- **`PreparedIndex` 没有生命周期参数**：`commit()` 只用锁文件的路径，不需要借 `repo`。调用点写法不变
- **`commit` 收 `&mut self`，不是 `self`**：Task 10 的 rename 失败回滚要「先把分支 CAS 回去、再删锁」（「偏离计划」的发布顺序），`self` 版本在返回 `Err` 时锁已随 Drop 删掉，窗口就开了。成功后 `IndexLock::release` 交出所有权，之后 Drop 不删（那个路径可能已是别的 git 进程的锁），再 commit 一次报错。`abandon(self)` 不变
- **`ComposeReport` 多一个 `discarded`**：分析 §6.12 的通知要列「哪些暂存被丢弃」，而 `TakeNew` 同时覆盖「索引本来就等于 NEW」（没丢东西）与「作者是我、早于快照」（丢了）两种，靠 `decisions` 分不开
- **多公开 `pinned_ref()` 与 `rename_lock_into_index(lock)`**：后者给 Task 10 的 journal 补做 rename 用，那时手里没有 `PreparedIndex`
- **i-t-a 且作者是我：永远 `TakeNew`，不走 `KeepStaged`，也不记 `uncertain` / `discarded`**。指令写的是「作者是我其余情况 → KeepStaged」，但 i-t-a 没有暂存内容可保留，硬留有害：NEW 里有这个文件时，i-t-a 条目不进 write-tree，下次提交等于把 NEW 的文件删掉；`--index-info` 也写不出 i-t-a 标志，只能靠 `git add -N`，而那要求磁盘上有文件。作者不是我时照指令走 `LostToNew`，让用户看到这条路径被同事也加了
- **删除走 `--index-info` 的 mode 0 行，不走 `--force-remove`**：一次写完，少一次进程；形状与 Task 6 相同，`own_commit_keeps_staged_delete_when_uncertain` 覆盖
- **没有 `.git/index` 时报 `svn_follow_index_missing`**：计划没提。这时锁文件是 0 字节，git 读 0 字节索引是 fatal；这个形状不值得往下走
- **`git2::Index::open` 读 i-t-a 标志**：见下面「计划没写的坑」。读取走 libgit2，符合 §7.3 的分工
- **`svn_baseline.rs` 抽出 `parse_entry_flags`**（`clear_entry_flags` 与 `restore_flags` 共用），`copy_with_mtime` 改 `pub(crate)`，`IndexLock` 加 `path` / `abs_path` / `is_released` / `release`
- **测试 fixture 是纯 git 临时仓库**（同 Task 7 的理由），NEW 在对象层造，工作区由测试直接写文件；`publish` 辅助函数模拟编排的「CAS → commit」
- **计划外多 21 条测试**：决策表其余的行（trivial、非内容、二进制、作者是我时保留暂存的删除、读不到对象）、i-t-a 三条（留存、让位、空文件不被误判）、skip-worktree、D/F 三条、锁与收尾七条（放弃 / Drop、拼好时真实索引不变、二次 commit、撞别人的锁、缺索引、中途失败、OLD == NEW）、Windows 下 rename 重试两条

#### 计划的错误

- **Global Constraints 的 git ≥ 2.40 不够**：`merge-file --object-id` 从 **git 2.43** 才有（`RelNotes/2.43.0`：「learns a mode to read three variants of the contents to be merged from blob objects」）。2.40–2.42 上退出 129，本 Task 报 `svn_follow_git_too_old`；但这时基线已经 CAS 过了（发布顺序：svn/base → `prepare_index`），所以 **Task 10 的预检线要改成 2.43**，不能靠这条运行时报错兜底
- **W22 与计划写的「rc>0 或 255 → LostToNew」太宽**：实测 128 = 读不到对象、129 = 不认识选项，这两个不是「合不了」。把它们判成 lost 等于把故障说成正常结果。现在只有 1..=127（冲突块数）与 255（二进制）算 `LostToNew`，其余报错
- **W25「skip-worktree 保留」只对 read-tree 成立**：`--index-info` 写过的条目 skip-worktree 与 assume-unchanged 都丢。计划只写了补 assume-unchanged；skip-worktree 同样要补，否则「已暂存 ∩ NEW 改过」的路径丢标志

#### 计划没写的坑

实测环境：git 2.51.2.windows.1。探针是临时的 cargo 集成测试（`tests/t8_probe.rs`，`TempDir` 里的 fixture，跑完已删），因为 Bash 工具的 worktree 隔离检查拒绝了在 scratchpad 里直接跑 git 的脚本。

| 场景 | 实测 | 处理 |
|---|---|---|
| 暂存文件 `dfb`，NEW 新增 `dfb/y` | `read-tree -i -m` **退出 0**，暂存的 `dfb` 被无声删掉 | 与 NEW 带进来的路径互为文件 / 目录的暂存记 `LostToNew` 并报告（`collides_with`），第 4 步摘掉 |
| 暂存 `dfa/x`，NEW 新增文件 `dfa` | **退出 0**，留下 `dfa/x`、吞掉 NEW 的 `dfa`，等于暂存一个反向删除 | 同上 |
| 暂存 `dfc/x`，`--index-info` 写文件 `dfc` | 退出 0，`dfc/x` 被顶掉（`--index-info` 打开了 allow_replace） | `KeepStaged` 写回会把 NEW 带进来的目录顶掉，碰撞时改判 `LostToNew` |
| i-t-a 在 `diff-index --cached` 里 | `:000000 100644 0… e69de29… A`，与暂存空文件完全同形；`ls-files -s` / `-v` 也分不开；`--ita-invisible-in-index` 对 OLD 里没有的路径能分开（OLD 里已有的路径按语义会显示成 D，分不开，这一形态未实测） | 用 libgit2 读条目的 `flags_extended`，`empty_staged_file_is_not_mistaken_for_intent_to_add` 钉住 |
| i-t-a，NEW 新增同路径 | read-tree 拒绝（rc=128，再证 W25）；`--index-info` 写过之后标志清掉 | 走第 4 步，无需特殊处理 |
| i-t-a，NEW 没碰 | `read-tree -i -m` 保留条目与标志 | `intent_to_add_survives_when_new_untouched` |
| 0 字节索引 | `fatal: index file smaller than expected` | 见「偏离计划」 |
| 给索引里没有的路径打标志 | `fatal: Unable to mark file` | 只补「之前有、现在没有、条目还在」的路径（`gone.txt` 用例） |

- **路径严格按 UTF-8 解**：`read_changes` 走 `run_git_bytes` + `str::from_utf8`，不走 lossy。这些路径要原样写回索引，换成 U+FFFD 就成了另一条条目，宁可报 `svn_follow_index_failed`。`ls-files -v` 仍是 lossy，只用来比对标志，碰不到写回
- **`merge-file` 不做 eol 规范化**（W22）：两侧换行不同时按推理会整文件冲突，落到 `LostToNew`，方向安全。这个形态没有单独测试
- **变异核对**（脚本 `t8_mutate.py` 在会话 scratchpad，每次改一处跑 `svn_follow_index`，跑完逐字节还原）：20 个变异全部变红。去掉第 4 步预写 → 15 条红；去掉第 6 步 → 3 条；去掉标志补回 → 2 条；补标志不看条目是否还在 → `assume_unchanged_…`；只不补 skip-worktree → `skip_worktree_…`；去掉 D/F 判定 → 3 条；去掉 KeepStaged 的碰撞改判 → `kept_stage_never_…`；关 i-t-a 识别 → `intent_to_add_gives_way_…`；去掉 refresh → `stat_kept_…`；不 release、去掉二次 commit 守卫 → `second_commit_…`；去掉 rename 重试 → 两条 Windows 用例；128 当冲突 → `unreadable_blob_…`；255 不算 lost → `binary_…`；去掉 trivial → `index_already_equal_…`；`staged_before_mark=None` 当早于快照 → `stage_after_mark_…`；`author_is_me=None` 当是我 → 两条同事用例；不比 ours / theirs 的 mode → `colleague_vs_non_content_…`；改钉住的 ref 名 → 3 条

#### 为什么这么改是安全的

- **真实索引在 `commit()` 之前一个字节都不变**：全部写操作都在 `GIT_INDEX_FILE=<abs>/.git/index.lock` 上，`prepared_index_is_written_into_the_lock_not_the_real_index`、`abandon_and_drop_remove_lock_and_keep_pin` 断言
- **不碰磁盘**：没有 `checkout`、`read-tree -u`；`--refresh` 只写 stat。关键用例前后对比工作区全部文件的内容与 mtime
- **任何丢暂存的路径都能找回**：动手之前钉了原始字节，`pinned_raw_index_restores_byte_for_byte` 写回后断言标志与暂存内容；放弃与换入之后 ref 都保留。（Review 2 更正：当时只在 gc 之前成立，见 W28 与「Review 2 修复」第 4 条）
- **失败不留残锁，不抢别人的锁**：RAII 删自己的锁；`create_new` 撞到别人的锁时报 `svn_index_busy`，文件原样（`busy_index_lock_is_left_alone`）；换入之后 Drop 不删（`second_commit_is_an_error_and_does_not_touch_someone_elses_lock`）
- **空集合不发命令**：`staged` 或 `changed` 为空时不读 libgit2、不写条目；`write_entries`、`restore_flags` 空列表直接返回。全程没有 pathspec（`diff-index` / `diff-tree` / `ls-files -v` 都不带路径，写入走 stdin），`--literal-pathspecs` 无从加起
- 探针与测试都只在临时目录里跑 git，没有碰本仓库或任何真实仓库；变异核对只改 `svn_follow_index.rs` 自身，跑完逐字节还原

#### 遗留风险

- **给 Task 10**：
  - 预检线改 2.43（见「计划的错误」）
  - 崩溃恢复要分清残留的 `index.lock` 是否完整：`prepare_index` 中途崩溃时它是半成品，只有 journal 显示已经走过 CAS 才能 `rename_lock_into_index`，否则只能删
  - D/F 与 `LostToNew` 都进 `report.lost`，§6.12 条件 12 据此不自动跟进
  - `ComposeReport` 派生了 `Serialize`（`Merged` 序列化成 `{"merged": "<oid>"}`），序列化契约测试与前端类型留给 Task 10 / 11
- **split index**（`core.splitIndex`）：钉住的只是 `.git/index`，共享索引文件 `sharedindex.*` 过期被清理后写回还原不了。默认关闭，未处理
- **sparse index**：libgit2 读不了 `sdir` 扩展时 `git2::Index::open` 失败，跟进报错退出（不丢东西）。未实测
- **OLD 里已有路径又被 `rm --cached` + `add -N` 的 i-t-a**：决策走 i-t-a 分支（取 NEW），没有单独测试
- **`git merge-file` 对读不到的对象退出 128，但 `error()` 路径上的其他故障也可能是 255**，会被当成二进制记 `lost`。原内容在钉住的索引里，方向安全

### Task 9：失焦快照与「我是谁」

实测都在 scratchpad 的 `t9-exp/`（git 2.51.2、SlikSvn 1.14.2）。`svn auth` 全程带 `--config-dir` 指向脚本自建的假缓存（虚构的用户名与口令），没有碰开发机真实的凭据缓存。

#### 偏离计划

- **`svn_wc_db_stamps` 的枚举：选缓存，不选「白名单往上找 `.svn`」。** `enumerate_wcs` 每个 wc 发 `svn info` + `svn status`，每次焦点都跑太贵；只从白名单往上找又看不到白名单里的 external，而往 external 提交只写它自己的 wc.db，焦点检测会漏（`a_nested_external_gets_its_own_stamp`，就是 Review Focus 1 的形态）。做法：进程内记住 `repo → {whitelist, wc.db 位置, 枚举完那一刻的戳}`，之后只 `stat`；戳变了或白名单变了才重新枚举。external 的增删一定写父 wc 的 wc.db（`an_external_added_later_shows_up_after_the_parent_wc_db_changed`），不用单独盯。重新枚举与前端随后的第二档校验同一时机，多花的是一次 status。枚举失败不进缓存：有旧清单用旧清单，没有才退回白名单往上找，下次焦点重试
- **`WcDbStamp.mtime_ns` 在 IPC 上是十进制字符串**（Rust 字段仍是 `u128`，`mtimeNs: string`）。见「计划的错误」
- **`svn auth` 不走 `run_svn`**，用 `svn_command` + `with_global` 自己组（仍是 svn 的唯一入口，`--non-interactive` 也还在），因为要多设 `LC_ALL=C`；`stdin` 显式 null。见「计划没写的坑」
- **比较 realm 用解析后的 scheme + host + 端口**，不是计划写的「realm 包含 repos_root 的 scheme+host」：子串比较会让 `https://svn.corp.example` 命中 `https://svn.corp.example.evil.test`。两边都写了端口就要相等；一边省略时另一边必须是默认端口（http 80、https 443、svn 3690、`svn+*` 的 22 与 3690）
- **`clear_mark` 没有返回值**（计划的签名就没写），删不掉只记 warn。理由见「为什么这么改是安全的」
- **`read_mark` 的 IO 错误也按 `None`**，所以它现在不会返回 `Err`；`Result` 按计划的接口保留。oid 形状不对（不是 40 / 64 位十六进制）算损坏：这个值以后要进 git 命令行
- **`write_mark` 不写的情形多一种**：`.git` 不是目录时什么也不做。命令层再挡一道，不是 `SvnHybrid` 时 `svn_follow_mark` 不写、`svn_wc_db_stamps` 返回空列表（与 `svn_pinned_externals` 同一约定）：焦点事件不分仓库类型
- **多公开了三个函数**：`resolve_svn_user`、`wc_db_stamps_with`、`svn_auth_listing(repo, config_dir)`。前两个让测试注入 `svn auth` 的输出与枚举函数，后者的 `config_dir` 让测试指向假缓存，生产传 `None`。这样测试不必碰开发机的真实缓存，也数得出枚举的次数
- **改了计划没列的 `commands/watch.rs`**（见「计划没写的坑」）
- **计划外多的测试**：列出的 5 条之外，`tests/svn_follow_mark.rs` 另有 29 条，覆盖 i-t-a、冲突、detached、无提交、不造 `.git`、损坏快照、realm 边界、缓存失效与枚举失败；`tests/svn_config.rs` +1、`tests/svn_backend.rs` +1、`commands/watch.rs` 的单元测试 +1

#### 计划的错误

- **`git diff-index --cached -z HEAD` 不够用，要加 `--ita-invisible-in-index`。** 实测它把 i-t-a 报成 `A` + 空 blob（`e69de29…`），与真的暂存了一个空文件同形，靠 oid 排除不了。i-t-a 没有暂存内容，快照里不该说它「暂存了空 blob」。加上之后 `staged` 只含真的暂存；Task 8 的 `decide` 对 i-t-a 另有判断，不依赖快照，这条改动不改变它的结果。Task 10 算 `staged_before_mark` 时，当前条目是 i-t-a 的路径在快照里查不到，得到 `false`
- **冲突条目**在同一条命令里是 `U` + 全 0，与「暂存的删除」同形。不记。索引有冲突时跟进本来就拒绝，这里是防快照在冲突期间写下、解决之后才跟进时留下一条假的删除
- **`mtime_ns: u128` 不能按 JSON 数字传。** 当代纳秒时间戳约 1.8e18，超过 2^53，`JSON.parse` 后舍入到 256ns 的倍数。前端只拿它比相等，改成字符串就没有这个问题。`wc_db_stamp_serializes_with_the_nanoseconds_as_a_string` 钉住
- **计划写的 `Authentication realm:` / `Username:` 只在英文输出下成立**，见下

#### 计划没写的坑

- **`svn auth` 的标签走 gettext，不是固定文本。** 实测 `LC_ALL=de` 时输出 `Anmeldebereich: <https://…`，`LC_ALL=zh_CN` 时同样被翻译。中文系统上不强制 locale 的话解析永远取不到用户名，而且不报错。`LC_ALL=C` 压得住 `LANGUAGE=de`（实测）。`real_svn_auth_output_parses_under_a_foreign_locale_and_never_shows_passwords` 用真 `svn auth` + 假缓存钉住
- **非 ASCII 用户名经管道是系统代码页**：实测 `张三` 输出 `d5 c5 c8 fd`（GBK），不是 UTF-8。没有解码依赖（Global Constraints 不新增 crate），`from_utf8_lossy` 之后含 U+FFFD。这种情形 `parse_svn_auth` 返回 `None`，走 D4；这样的用户要在配置里写 `svnUser`
- **`svn auth` 的实际形状**：每条凭据以 72 个 `-` 分隔，`Credential kind:` → `Authentication realm:` → 其余字段；`svn.simple` 的 `Password:` 恒为 `[not shown]`；`svn.ssl.server` 也有 realm 行，但形如 `https://host:443`（无尖括号）且没有 `Username:`；`svn.username` 的 realm 是 `svn+ssh://host`。空缓存 rc=0
- **失焦写快照会触发刷新。** `is_relevant` 对 `.git/` 下的文件默认放行（只过滤 `*.lock` 与 `objects/`），`follow-mark.json` 每次切窗口都会触发一轮 `repo-changed`。没有回路（刷新不产生失焦），但每次切窗口多一轮全量刷新。过滤 `/.git/mygitclient/` 这个子目录和目录本身（第一次写时目录是新建的）；`mygitclient-sync.json` 等同前缀的文件不受影响，`the_follow_mark_written_on_blur_is_noise` 钉住两个方向
- **建目录可能凭空造出 `.git`。** git 会往上找外层仓库，所以 HEAD 解析得出来不代表当前目录有 `.git`；`create_dir_all` 随后会造一个，§6.1 就把目录认成 `SvnSuspicious`（与 `hold_for_baseline_move` 的注释同一类）
- **纯 mode 的暂存**：`diff-index` 报 `M`，目标 oid 与 HEAD 相同。快照只记 blob，`staged` 里这条的值就是原 blob
- **变异核对**（`svn_follow_mark.rs` 每次改一处，脚本在 scratchpad 的 `t9-exp/`，跑完 `cmp` 确认还原）：18 处变异全部有测试变红，覆盖 i-t-a 选项、`U` 跳过、`.git` 守卫、`LC_ALL`、U+FFFD、多用户名、realm 复位、oid 校验、缓存命中 / 永不失效 / 白名单键、`file` 排除、端口、host 大小写、URL 里的用户名、去重。第一轮有三处存活：按 `---` 分隔线复位 realm 是死代码（每条凭据都有自己的 realm 行），删掉；它同时掩盖了「realm 不复位」的变异，删掉后这条被抓到；`scheme == "file"` 的判断原来没有测试走到（`file://server/…` 靠 realm 对不上才返回 `None`，但已经多跑了一次 `svn auth`），补了断言

#### 为什么这么改是安全的

- `write_mark` 只跑 `rev-parse`、`symbolic-ref`、`diff-index --cached`，都不写索引、不拿 index.lock，所以跟进发布期间失焦也不互相挡。唯一的写是 `.git/mygitclient/` 下的一个文件，先写同目录临时文件再 rename，读到的要么是旧快照要么是新快照
- 快照只会让判定走向保守或说真话（Review 2 更正：当时这句不成立，快照比 SVN 提交新时说的是假话，见「Review 2 修复」第 1 条。下面三种情形仍然成立）：
  - 没有 / 坏了 ⇒ 拿不准 ⇒ 保留暂存、正常重放
  - `clear_mark` 没删掉，或者跟进收尾与失焦交错留下一份旧快照 ⇒ 里面的话仍然成立（这份暂存、这些 commit 确实早于下一次外部提交）
  - 同步中途失焦 ⇒ HEAD 可能停在 `svn/base`，分支上的 commit 不是它的祖先 ⇒ 按拿不准处理
- 取不到用户名、用户名不唯一、解码坏了都返回 `None`，全部走 D4，不会把别人的提交当成自己的
- 凭据：不带 `--show-passwords`；只解析两类行；`svn auth` 的 stdout / stderr 都不进日志，失败只记退出码；配置有值或 `file://` 时根本不跑。测试断言输出里没有假口令；`no_config_and_no_credentials_means_no_identity` 用 `file://`，不会走到真缓存
- 缓存只影响「什么时候重新枚举」，不影响戳本身：每次调用都现读 `stat`

#### 遗留风险

- **命令未在真实窗口验证，留给 Task 11。** 本 Task 没有前端调用点，`svn_follow_mark` / `svn_wc_db_stamps` 的 `path` 参数只有一个、camelCase 无变化，`the_commands_work_on_a_hybrid_repo` 从命令函数层走通，但没有经过 Tauri IPC（AGENTS.md §7.3 的 `pnpm tauri dev` 这一步没做）
- ~~**`svnUser` 目前只能手写进 `.git/mygitclient.json`。** 分析 §6.10 说「设置里新增」，计划 Task 11 没列，需要决定归哪个 Task~~ Task 11 在跟进报告里加了入口（D4）
- **`svn+ssh` 的 realm 端口没有实测**（开发机没有这种服务器），`default_ports` 把 22 与 3690 都认了。host 对得上时错认的后果是把同一台机器上另一个端口的用户名当成我，风险很小
- **崩溃在写临时文件与 rename 之间**会在 `.git/mygitclient/` 留一个 `*.tmp`，不影响读取，也不进 git。没有清理（journal 的 `sweep_stale_tmp` 在锁里跑，这里不在锁里，扫了可能删到并发写的文件）
- **枚举失败且没有缓存时看不到 external**：比如 svn 不在 PATH，但那时第二档校验本身也跑不了
- **快照只比 blob**：暂存后只改 mode、blob 不变，跟进时 Task 10 会判成「暂存没变」
- **linked worktree**（`.git` 是文件）下 `write_mark` 什么也不做。混合仓库的识别依赖 `<root>/.git/mygitclient.json`，这种形态本来就进不来

### Task 10：跟进编排（F1–F6）与崩溃恢复

实测环境同前（git 2.51.2.windows.1、SlikSvn 1.14.2）。探针与测试都在临时 fixture 里跑，探针目录用完已删。

#### 偏离计划

- **`FollowOutcome::Refused` 的含义定为「一个 ref 都没动」**：预检失败、F1–F3 里任何失败（录制、重放）、拿不到同步锁、锁内复核出错，一律 `Ok(Refused)`，错误码原样带出（`svn_preflight_conflicted`、`svn_baseline_lying`、`sync_in_progress` 等）。动过之后（基线已前进）的失败走 `Err`，文案说明此刻的状态
- **`FollowReport` 比计划多两个字段**：`verified`（断网时 `record_mixed` 给 `NotChecked`，基线照样发布，分析 §6.10 要求用户知情）；`removed`（`ExternalChange` 要 rev 与作者，删除的文件 status 里已经没有了，塞不进 `changes`）
- **`Done { report: Box<FollowReport> }`**：不装箱时 clippy 报 `large_enum_variant`。序列化形状不变
- **`FollowSteps` 带完整的 `report`**，`index_blob` 记拼好的索引的 blob OID。报告类型（`ExternalChange`、`Leak`、`HunkLeak`、`LeakVerdict`、`StageDecision`、`ComposeReport`）因此加了 `Deserialize`。理由：泄漏告警、丢弃的暂存只在这次跟进里算得出来，不能因为崩溃就丢
- **旧 / 新基线与当前分支沿用 journal 已有字段**（`baselineOidBefore`、`newBaselineOid`、`origBranch`、`origHead`），`FollowSteps` 只放跟进才有的东西
- **`PreparedIndex` 新增 `keep_lock_on_drop`（`pub(crate)`）**：分支移不回去时留着锁，崩溃注入也用它（真实崩溃时 Drop 不会跑）
- **分支的基线（下称 scope）取 `merge-base HEAD svn/base`，普通入口也一样**：计划只在「分支跟进」入口写了 merge-base。上次只发布了基线、分支还落后时又来一个外部提交，这时走普通入口。commit 清单两种写法相同（`A..B` 本来就排除 A 的全部祖先），差别在「外部变了什么」：只看 `svn/base..NEW` 会漏掉上一个基线带进来的路径，F2 ① 与 F3 都看不到它们（`lagging_branch_still_supersedes_paths_from_the_earlier_baseline`）。merge-base 不是基线 commit 时拒绝（`svn_follow_branch_unrelated`）
- **P（F2 ①）、F3 的路径集合取 `diff-tree <分支的基线> NEW`，作者取节点状态**，不直接取判定的 `changed` / `added`：这样「只算真实内容变了的路径」由构造保证，分支落后时也算得对。`report.changes` 普通入口仍取判定（有 rev 与作者），分支跟进入口没有判定可用，取这份 diff
- **`staged_before_mark` 要的当前暂存 blob 用 `ls-files --stage -z` 整份读一次**（`OnceCell`，第一次调 `facts` 时读）：`facts` 只在 `prepare_index` 持索引锁时被调用，这时读到的就是拼索引看到的那份；逐路径发进程在 Windows 上更慢
- **分支跟进入口单独跑一次 R1（`run_preflight_with(.., true)`）**：这个入口不录制，分析 §6.5 的前置条件（SVN 冲突、写锁）没有别处把关
- **没有做分析 §6.12 条件 11（重放后仍改动外部变更路径）对其他分支的挡板**：指令与计划的 `behind_branches` 条件都只列了冲突、不线性、F3。条件 11 防的是磁盘与重放结果不一致（W14），其他分支不在磁盘上。自动跟进（§6.12）本 Task 不做
- **前端多做了一点**：指令要求未完成的跟进能提示并继续。`describeUnfinishedSync` 的 `follow` 分支只给「继续」（C1a 没有放弃入口），`BannerAction` 加 `followContinue`，`svnSyncFlow.ts` 接 `svnFollowContinue`。按钮点了要有结局可看，所以提前建了计划归 Task 11 的 `src/engine/svnFollow.ts`，放类型与 `describeFollowOutcome`（完成时也要把泄漏、没保留的暂存、落后的分支列出来）。`describeFollowReport` 与弹框仍归 Task 11
- **「继续」也过 `guardMerge`**：恢复走不下去时会重新跟进，重放要走 merge-tree
- **`svn_sync_in::take_over`、`svn_sync_out::take_over`** 遇到 `follow` 记录各报自己的码（`sync_follow_not_supported`、`sync_out_wrong_flow` 改文案），不再说成「这是交付留下的」
- **`with_watch_suspended` 包住整个 `follow`，含联网准备阶段**，与分析 F4（发布阶段才挂起 watcher）不同（Review 2 第 9 条补记）。这样写是因为挂起的范围在命令层，而准备与发布在 `follow` 内部，还可能重来一次。代价是准备阶段的文件事件被丢弃，不是攒着，界面在这几秒里不刷新；结束时无论成败补发一次 `repo-changed`，刷新不会丢。代码没改
- **其他分支的范围**（Review 2 第 9 条）：原来用 `scope`（当前分支的 merge-base）判，当前分支落后时会移动只含更早基线的分支，这是一处未记录的偏离，已改回 §6.11.3。分支跟进入口没有「这次的旧基线」，取 `svn/base` 的父提交
- **测试比计划多**：计划的 14 条之外，`tests/svn_follow.rs` 另有 58 条（见文件），`tests/svn_journal.rs` +2、`tests/svn_backend.rs` +1、vitest 新增 `svnFollow.test.ts`（5 条）与 `svnJournal` / `svnSyncFlow` / `BaselineBanner` 共 8 条

#### 计划的错误

- **Global Constraints 的 git ≥ 2.40**：已改 2.43（D9），`svn_follow_git_too_old` 在 F1 之前报，`git_older_than_2_43_is_refused_before_anything_is_recorded` 断言对象库都没变
- **计划写「测试钩子用 `#[cfg(test)]` 或 feature」**：`#[cfg(test)]` 对集成测试不可见，与 Review 1 的 `svn_tombstone::fault` 同样的理由，用 `#[doc(hidden)] pub` 的线程局部开关
- **`resume_after_crash_between_each_step` 写的「在每一步之后 panic」**：普通 panic 的 unwind 会跑 `IndexLock` 的 Drop，把 `.git/index.lock` 删掉，测的就不是崩溃了。改为注入点先让 `PreparedIndex` 放弃删锁，再 `resume_unwind`（不触发 panic hook，输出也干净）

#### 计划没写的坑

- **W23 要加限定（更正）：`git checkout -q -b <新分支>` 会被 index.lock 挡住，`git checkout -b`、`switch -c`、`switch -q -c`、`symbolic-ref` 不会。** 探针对调过顺序、重复三轮，只跟 `-q` 有关，与工作区干净、脏、有暂存无关（git 2.51.2.windows.1）。原因没有查证。对实现没有影响：两次 HEAD 核对防的是绕得过的形态，而 Rider 之类的工具用不用 `-q` 不由我们定（Review 2 更正：这里漏了落点。落在 CAS 之后时新分支建在 NEW_TIP 上，回滚反而造出反向暂存，见第 3 条）。**测试钩子不能加 `-q`**，否则挡在钩子里，测不到竞态。W23 那一行已同步
- **`<commit author>` 读的是 wc.db**：提交之后再 `svnadmin setrevprop` 不会刷新，`svn update` 同一 revision 也不刷新。fixture 里「我」直接取 wc1 提交的作者（系统用户）配进 `svnUser`；同事的 revision 要在 wc1 update **之前**改 revprop
- **index.lock 存在时不能靠「文件在」判断它是谁的**：崩在 rename 之后，那个路径上可能是别的 git 进程的锁。journal 记 blob OID 比对，对不上的一律不碰
- ~~**rename 发生过没有，靠「`.git/index` 的字节还是不是钉住的旧索引」**：不能拿「等于拼好的那一份」判：重开之后 `git status` 刷新 stat 就会改写索引。代价见遗留风险~~ **Review 2 第 10 条更正：** 字节两个方向都不可靠。现在比 `ls-files --stage` 的指纹（`indexStages`），stat 不进去，等于拼好的那份才算 rename 发生过
- **往回走总是安全的**：就算 rename 其实发生过，索引 == NEW_TIP、分支 == OLD，重新跟进时这些路径都走「索引正好等于新 HEAD」，结果相同。所以拿不准（索引读不了、指纹对不上）时往回走。**Review 2 补充例外：** HEAD 已被 `checkout -b` 带到 NEW_TIP 上时，移回原分支动不了 HEAD，往回走就是反向暂存，只能往前（第 3 条）
- **变异核对纠正过一条错误的理解**：最初以为 `svn/base..HEAD` 在分支落后时会列错 commit，并照此写了注释与测试。变异「scope 取 `svn/base`」存活，查下来两者列出的 commit 相同，真正受影响的是外部变更集合（见「偏离计划」）。注释已改，补了能打到它的测试
- **`git diff` 会顺手刷新索引**：`check_git_idle` 的 `unmerged_paths` 走 `git diff --name-only`，stat 对不上时可能写索引。发生在拿索引锁之前，不影响发布
- **C1a 没有放弃入口，恢复的每条路都要走得到删 journal**：卡住的 journal 会挡死更新、交付与重建基线。分支被外部移到别处时不再报错停下，放手、删记录、重新跟进（`branch_moved_elsewhere_while_crashed_stops_without_clobbering_it`）。只有一种情形留着 journal：分支还在 NEW_TIP 上又移不回去，这时删了就是反向暂存窗口
- **`rename` 之后 index.lock 已放开，分析 §6.5 F5 ④ 的「持锁移其他分支」做不到了**：这是发布顺序改动的代价。`finish` 里移动之前重查一遍 `worktree list`，见遗留风险

#### 测试钩子的处置

`svn_follow::fault`，形状同 `svn_tombstone::fault`：`#[doc(hidden)] pub`、线程局部、守卫式撤销，生产代码没有地方 arm 它，每个注入点只多几次线程局部读。三种注入：

| 入口 | 用途 |
|---|---|
| `crash_at(Point)` | 模拟进程死掉：`resume_unwind(Crashed)`。journal 与 index.lock 留在磁盘上，同步锁随 unwind 释放（对应操作系统回收句柄） |
| `call_at(Point, f)` | 在固定位置插外部操作：`checkout -b`、`update-ref`、外部 `commit`、别的进程的锁、占住 `.git/index` |
| `pretend_git_version(v)` | 开发机 git 够新，走不到拒绝那一支 |

11 个 `Point` 覆盖准备结束到删 journal 之前的每一步。留在非测试构建里，与 `svn_tombstone::fault` 同一取舍。

#### 为什么这么改是安全的

- **不写磁盘**：写操作只有 `update-ref`、对象写入、`.git/index.lock` 与 rename，没有 `checkout` / `read-tree -u` / `svn` 写命令。`follow_never_writes_working_tree` 比较全部文件的内容与 mtime
- **准备阶段不动 ref**：`Refused` 的几条测试比较全部 ref 与索引字节。基线 commit 与重放结果只是对象，没有 ref 指着，放弃时等 gc
- **发布期间外部 git 进不来**：从拼索引到 rename 一直持 index.lock（W23）。`checkout -b` 绕得过，由两次 HEAD 核对兜住；`update-ref` 绕得过，由 CAS 兜住
- ~~**「分支已移、索引是旧的、没有 index.lock」走不到**：正常路径先 CAS 回去再放锁（`back_out`）；CAS 不回去时留着锁（`keep_lock_on_drop`）；恢复时锁没了就移回分支。`assert_no_reverse_stage` 在每个崩溃点断言~~ **Review 2 更正：走得到（探针 P3、P3b）。** `checkout -b` 落在 CAS 之后时，上面每一条都把 `work` 移回去了，而 HEAD 已经在 `race@NEW_TIP` 上。`assert_no_reverse_stage` 只看 `work`，所以没抓到；原来的两条测试一条测的是 CAS 之前，一条手工把 `race` 改回 OLD。现在的保证是：HEAD 在 NEW_TIP 上就往前（`head_left`），索引换不进去时留着锁和记录，拼好的索引没了就重拼，见第 3 条
- **任何被丢的暂存都找得回来**：`prepare_index` 动手前钉了原始索引，`refs/mygitclient/follow/<opid>` 保留最近 20 个（Review 2 更正：当时 gc 之后找不回来，第 4 条起挂了 `blobs/<oid>`）
- **每个 ref 都按旧值 CAS**：`svn/base`、当前分支、其他分支。其他分支移不动时进 `behind_branches`，不覆盖
- **崩溃恢复幂等**：`finish` 的每一步都先看 ref 现值，`clear_mark` 与修剪重复跑无害；`second_crash_during_resume_is_still_recoverable` 覆盖恢复途中再崩
- **测试全部在临时 fixture 里**，没有碰本仓库或任何真实仓库

#### 变异核对

`svn_follow.rs` 每次改一处，跑点名的测试，必须变红。脚本 `mutate.py` 在会话 scratchpad，写回用 `write`（更新 mtime，否则 cargo 不重编译），每轮结束 sha256 确认逐字节还原。

64 条测试在 stub 下全红，但 stub 下什么都红，这个证据太粗，所以逐条核对。34 个变异全部抓到；第一轮有 2 个存活：

| 存活的变异 | 原因 | 处置 |
|---|---|---|
| scope 取 `svn/base` 而不是 merge-base | 选的测试只看 commit 清单，而两种写法列出的 commit 相同。这暴露了对 `A..B` 的错误理解 | 改注释，补 `lagging_branch_still_supersedes_paths_from_the_earlier_baseline`，原测试改名 |
| 其他分支失败时阻断跟进 | 选的测试走 merge 那一支，碰不到通用的 `Err` 分支 | 补 `replay_failure_on_another_branch_marks_it_behind_without_blocking`（P 里的路径在那条分支上是目录） |

其余 32 个按类：

| 类别 | 变异 |
|---|---|
| 发布 | 去掉第二次 HEAD 核对；`back_out` 不移回分支；swap 步骤不写 journal；`index_blob` 不记；暂存报告不进 journal |
| 恢复 | 任何锁都当成自己的；半成品锁不删 / rename 进索引；索引恒判已 rename / 没 rename；恢复时不移回分支；不补 CAS 当前分支；崩溃时锁被 Drop 删掉；读不到索引时卡死 |
| 判定 | 所有 / 没有 commit 算早于快照；取不到用户名也算我；所有路径都算我的；暂存删除不按全 0 比；外部变更算成空区间；不丢弃变空 |
| 复核与范围 | 锁内不复核映射 / HEAD OID；不跳过 upstream / worktree HEAD；已在 NEW 上的分支也重放；rename 之后不复查检出；分支跟进不做 R1；Local-Only 泄漏不挡其他分支 |
| 收尾与预检 | 不清快照；不修剪钉住的 ref；预检线退回 2.40 |

没做变异的两道兜底：去掉第一次 HEAD 核对（第二次核对同样抓得到，错误码也相同）；`finish` 里不比较分支现值（CAS 同样失败，结果相同）。

#### 遗留风险

- **其他分支在 index.lock 放开之后才移动**：`finish` 重查 worktree 之后、`update-ref` 之前，外部工具检出那条分支时，它会在已检出的状态下被移走，`git status` 显示一批反向改动。窗口是一次 git 进程的时间
- ~~**rename 判定的盲区**：崩在记完 `swap-index` 与 rename 之间（通常是几毫秒，`.git/index` 被占着时最长 1.5 秒），用户又手工删了 index.lock，**而且**索引被别的 git 写过（哪怕只是刷新 stat），会被判成 rename 已发生，留下分支已移、索引是旧的。要三件事同时发生。`move-branch` 那一步的同类情形不受影响，总是往回走~~ 已由 Review 2 第 10 条关掉
- ~~**删残留的 index.lock 分不清是不是别人的**：停在 `prepare-index` 时只能删。假定 Windows 上活着的 git 开着它，删除会失败，这时报 `svn_index_busy`、留着记录。**这个假定没有实测**~~ 假定被探针 P7 证伪，已由 Review 2 第 7 条改掉
- ~~**`NeedsManual` 不带报告**：当前分支冲突时 F3 与其他分支都没算，泄漏告警要等 Task 13~~ Task 13 已带上 `leaks`；其他分支仍然没算（见 Task 13 小节）
- **`FollowOutcome` 的命令没有经过 Tauri IPC 验证**（AGENTS.md §7.3 的 `pnpm tauri dev`）：`svn_follow` 没有前端入口（Task 11），`svn_follow_continue` 只有真的崩溃才走得到。契约测试钉了序列化形状，`path` 参数名没有转换
- ~~**`needsManual` 的文案让用户去用「更新」**：Task 13 之前的权宜之计，Task 13 要改~~ 已改（D10）
- **linked worktree 里 rebase 中的分支**：porcelain 里是 detached，判不出它「在用」，可能被移动
- **重新跟进（`Again`）丢失原报告**：往回走之后重新算，泄漏与外部变更算得回来，但「丢弃的暂存」若 rename 已发生，第二次看到的索引已经等于 NEW，不再列出（只有拿不准而往回走时才会遇到）
- `dropped` / `rewritten` 里可能含最后一刻落入 `behind_branches` 的分支的 commit（`finish` 那一步才发现的情形）

### Task 11：前端——检测、跟进入口与结果展示

**真实窗口未验证**（`pnpm tauri dev` 要人操作）。下面的「手测清单」是这个 Task 欠的验证，要在 fixture 仓库上做，不要拿真实仓库试跟进。

#### 偏离计划

- **`svn_branches_behind` 比 `head_behind_base` 宽**：判据是「与 `svn/base` 有共同祖先，且分叉点不是它自己」，分叉点不要求是基线 commit。基线是一条链，共同祖先之下一定有更早的基线，这才是指令写的「包含更早基线、不含当前 `svn/base`」。差别在分叉点是交付 commit 的分支：确实落后，要打标记；手动跟进对它报 `svn_follow_branch_unrelated`，错误留在确认框里，「仅切换」不受影响。`head_behind_base` 不改，它是给「跟进」按钮的入口，按跟进接不接判
- **`BranchView` 的标记只来自新命令**（计划写「跟进报告和新命令」）：`behindBranches` 里的分支都含旧基线、不含新基线，全在命令结果里，再合并一份没有多的信息。原因文字只在报告里，分支列表的 tooltip 是通用说明
- **`Refused` 用结局提示框，不是顶部错误条**：`describeFollowOutcome` 已有的文案带「分支、基线和暂存区都没有改动」，这句话是用户此刻最想知道的，错误条放不下。`svn_follow` 命令本身抛错才进错误条
- **`NeedsManual` 只有一个框，不是先「知道了」再确认**：`describeFollowConflict` 把冲突的事实（含 leaks）放到手动跟进的确认框上，按钮就是出路。流程里调的是内部的 `confirmManualFollow`，不是 `handleManualFollow`，见「计划没写的坑」
- **`describeFollowOutcome` 收窄成不收 `done`**：完成时说什么挪到 `describeFollowReport`，留着 `done` 分支就是死代码。原有的几条 done 测试改写到 `describeFollowReport` 下，断言没有放松
- **`followContinue` 也走同一套结局分派**（`settleFollow`）：恢复之后重新跟进可能遇到冲突，原来只弹一句「点手动跟进」，界面上没有这颗按钮
- **报告的节按「要用户留意的在前」排**：泄漏、丢失的暂存、拿不准的暂存排在文件清单前面。计划的顺序是发生顺序，根目录更新一次带进几千个文件时，排在后面的节等于看不见。清单每组截到 200 条（`MAX_REPORT_ITEMS`），写明还有多少
- **切标签等同于失焦 / 拿到焦点**：指令只写「多仓库 tab 时只处理当前活动仓库」。标签藏着时外部提交没人检测，切回来要补一次；切走之后窗口失焦轮不到它，所以切走时写一次快照。`RepoTab` 因此多一个 `active` prop，改了 `App.tsx`
- **失焦多看一眼 wc.db（`markOnBlur`）**：指令只要求「判定是 inSync 且没有校验在跑」。还差一种形状：TortoiseSVN 进度框还在走时切回 jkk，提交在 jkk 前台时完成，焦点事件已经过去了，判定还是旧的 InSync，这时失焦写的快照晚于提交。多一次 `stat`，戳变了就不写。还没有过焦点检测时没有戳可比，按判定写
- **`applySvnSync` 加了代次闸，失败时清判定**：指令没写。焦点与 `repo-changed` 都会发起校验，先发起的那次晚到时会把 ExternallyAdvanced 盖回 InSync，横幅消失、下一次失焦写出假快照。校验失败时 `latestVerdict` 置空：失败不是 InSync 的判定
- **`repo-changed` 读到 HEAD 变了也跑第二档（`checkSvnSyncIfHeadMoved`）**：指令没写。`headBehindBase` 是第二档的结论，切分支不改 revision、不写 wc.db，原有两档都看不见：「仅切换」之后横幅上没有「跟进」，切回来又留着一条过时的横幅。分析 §6.11.3 写的正是「在 repo-changed 之后发现 HEAD 落后基线」。HEAD 变了才跑，提交也算一次，存盘不会走到第二档；打开之后第一次文件事件多跑一次
- **`latestVerdict` 放 ref、在拿到结果时写**，不读 `state.svnSync`：失焦回调闭包里那份是注册时的，走 ref 同步也要等一次渲染
- **`ConfirmRequest.alternative`**：确认框本来只有确认 / 取消，「仅切换」是第二个选择。`confirm()` 仍返回 boolean，点它时调 `onSelect`、结果是 false，调用方靠回调与取消区分。默认焦点仍在确认上
- **`set_svn_user` 不走 `write_config`**，见「计划没写的坑」。函数放在 `svn_config.rs`；`svn_user_info` 放在 `svn_follow_mark.rs`，挨着 `my_svn_user`
- **焦点用 DOM 的 `focus` / `blur`，不用 Tauri 的 `onFocusChanged`**：与现有的 `auditSvnIfDue` 同一来源，不新增权限和 API。两者在 WebView2 上是否完全一致没有实测，列在手测清单里
- **样式加了三条规则**（`.follow-report`、`.follow-user`）：其余复用 `deliver-*`、`dlg-*`；落后标记复用 `.sb-gone`
- **计划外的测试**：`tests/vitest/repoStoreFollowFocus.test.ts`（store 与 effect 的接线，23 条）、`BranchView.test.tsx`（4 条）、`ConfirmDialog.test.tsx` +4、`svnSyncFlow.test.ts` +17、`svnSync.test.ts` +20、`src-tauri/tests/svn_follow.rs` +7、`svn_follow_mark.rs` +10、`svn_backend.rs` +1。`banner follow button invokes svnFollow` 在 `svnSyncFlow.test.ts`：横幅只知道 `onFollow`，发 `svn_follow` 的是流程；横幅一侧另有按钮分发与禁用的测试

#### 计划的错误

- `FollowReport.dropped` 与 `rewritten` 的键是 commit OID，不是标题。报告里只能显示前 10 位。要显示标题得改 IPC，这次没改（这些 commit 已经不在分支上，图上也找不到）
- 指令里的行号是旧的（`refreshSvnSync` 实际在 :738、`checkSvnSyncIfChanged` 在 :748），以代码为准

#### 计划没写的坑

- **`handleManualFollow` 不能在跟进流程里调用**：它和 `handleFollow` 共用 `exclusive`，`running` 这时是真的，调用会直接返回，什么都不发生，也不报错（Review 3 第 4 条：这个导出没有调用方，已删）
- **`write_config` 存不了用户名**：它要过 `validate_new_extensions`，旧版本写下的 `.lua、.java` 读得出来、写不回去；经 `SvnConfig` 走一遍还会规范化白名单、丢掉不认识的字段。改成按 JSON 只动 `svnUser` 这一个键，临时文件 + rename（这个文件每次刷新都有人读）。键序会变，JSON 不在乎
- **`BASELINE_BRANCH` 不用跳过**：基线分支自己的分叉点就是它自己，判据天然排除，写了就是死代码
- **`branches_behind` 走 libgit2**：`repo-changed` 每次都要跑，逐条起 `git merge-base` 在 Windows 上一条几十毫秒，分支一多就是可感知的卡顿
- **Testing Library 的 `getByText` 会折叠空白**：`Code/A.cs  r7` 匹配不到自己，测试用正则
- 组件测试的 act 警告（`Dialog`、`Harness`）是 `readUser` 在断言之后才回来，与 `DeliverDialog.test.tsx` 现有的同一种，不影响结论

#### 为什么这么改是安全的

- **新命令**：`svn_branches_behind`、`svn_get_user` 只读。`svn_set_user` 只写 `.git/mygitclient.json` 一个键，写之前先 `read_config`，所以不会在没建库的目录里造出这个文件（§6.1 据它判定混合仓库），坏文件原样不动。`svnUser` 只与作者比相等，不进命令行
- **快照的四道闸都只会让它少写**：判定不是 inSync、校验在跑、wc.db 变了、读戳失败。少写的后果是用更旧的快照或者走「拿不准」：暂存保留、commit 正常重放，这是 Task 9 论证过的保守方向。写失败只记 warn
- **`trackSvnCheck` 的区间从读第一档的输入开始**：焦点刚回来、戳还没读回时切走同样不写。这条变异核对抓到过测试的盲点，见下
- **检测是只读的**：`svn_wc_db_stamps` 只有 `stat`，wc.db 没变时不发 svn 命令（E19）。校验失败不记戳，下一次焦点重试，同 `recordAfterSuccess`
- **跟进按钮**：`exclusive` 挡重复点击与其他流程，`setFollowing` 只管界面反馈。跟进不写工作区，所以不要确认框，但要过 merge driver（重放走 merge-tree）。手动跟进永远走确认框
- **「切换并手动跟进」**：切分支失败（工作区改动挡住）报错就停，不走手动跟进，也不替用户 stash

#### 变异核对

脚本在会话 scratchpad 的 `t11/`（`mutate_fe.py`、`mutate_rs.py`），每次改一处，只跑点名的测试文件，写回后 sha256 确认逐字节还原。

- 前端 45 个，第一轮 1 个存活：「读戳那段不算校验中」。原因是失焦自己也要读戳，测试把读戳挂住了，断言落在放行之前，有没有写都看不出来。改成放行之后再断言，抓到
- Rust 9 个，第一轮 1 个存活：「不排序」。libgit2 按引用名迭代，本来就有序，前端也只用 `includes`，是等价变异，删掉了 `sort()`，测试改成自己排序后比较

#### 遗留风险

- **真实窗口没有验证**，下面的清单都没跑过。`svn_follow`、`svn_follow_continue`、`svn_follow_manual`、`svn_follow_mark`、`svn_wc_db_stamps`、`svn_branches_behind`、`svn_get_user`、`svn_set_user` 都只到命令函数层，没有经过 Tauri IPC。参数名只有 `path`、`user`，camelCase 无变化
- **打开仓库之后第一次失焦**没有戳可比，提交在 jkk 前台完成的那种形状放得过去。下一次拿到焦点之后就有了
- **切分支被工作区改动挡住**时不做额外处理，用户要自己先收起改动（指令如此）
- **分叉点是交付 commit 的分支**：标「落后基线」，但手动跟进拒绝，只能「仅切换」或自己整理历史
- **`svn_branches_behind` 不区分 worktree 里检出的分支和有 upstream 的分支**：它们确实落后，只是跟进本来就不动
- **`dropped` 只显示 OID 前 10 位**，用户要对 reflog 才知道是哪个 commit
- **指令范围之外、这次没做的**：自动跟进（§6.12，只有按钮）；体检弹框按分组展示（§6.10）；向导第 2 步的「混合版本」字样（Task 5 遗留）；「本地改动」按 changelist 分组（§6.11.2）

#### 手测清单

全部在 fixture 仓库上做（临时 `svnadmin create` + 向导初始化），不打开真实仓库。

| # | 操作 | 要看什么 |
|---|---|---|
| 1 | Alt-Tab、点任务栏、点别的窗口切走再切回 | DOM `focus` / `blur` 都触发；每次切走 `.git/mygitclient/follow-mark.json` 的 mtime 变，切回时只有 `svn_wc_db_stamps`，没有 `svn status` |
| 2 | 打开 TortoiseSVN 提交框 | 切到它的瞬间算失焦，快照早于提交；同一进程弹的系统对话框（Discard）也算失焦，写一次无害 |
| 3 | TortoiseSVN 提交完切回 jkk | 一两秒内出现黄色横幅，文件与作者正确 |
| 4 | 提交进度框还在走时切回 jkk，等它走完再切走 | 这次失焦不写快照（mtime 不变），下一次切回出横幅 |
| 5 | 最小化再还原 | 最小化算不算 `blur`、还原算不算 `focus`；不能漏，不能叠两轮校验 |
| 6 | 点 jkk 自己的标题栏、拖动窗口、点进输入框 | 有没有多余的 `blur`；多一次快照无害，但不能出现频繁刷新 |
| 7 | 焦点刚回来立刻又切走 | 校验在跑时不写快照 |
| 8 | 开两个标签，切到另一个仓库再切回 | 只有当前标签响应窗口焦点；切走写快照，切回跑检测 |
| 9 | 点「跟进」 | 按钮变「正在跟进…」且禁用；完成后横幅消失、图与本地改动已刷新、报告弹出；各节内容对得上 |
| 10 | 造一个重放冲突，点「跟进」 | 确认框列出冲突文件与 leaks；确认后走手动跟进；取消后横幅变「当前分支落后基线」，再点「跟进」回到同一个框 |
| 11 | 造一个留在原地的分支 | 分支列表有「落后基线」；双击弹三个按钮，默认焦点在「切换并手动跟进」；有工作区改动挡住时错误条提示，不继续 |
| 12 | 「仅切换」，再切回来 | 磁盘写回旧内容（TortoiseSVN 显示已修改），确认框的风险说明属实；横幅出现「当前分支落后基线」，切回后消失 |
| 13 | 报告里的用户名一行 | 取不到用户时出现（`file://` 必然取不到）；保存后 `.git/mygitclient.json` 有 `svnUser`，清空后键消失 |
| 14 | 崩溃恢复 | 杀进程留下跟进记录，横幅「继续」走通，结局进同一个报告框 |

### Task 12：提交图

#### 偏离计划

- **新增 `baseline_gone`**（计划只有 `baseline_floor`、`baseline_newer`）。只有墓碑的混合基线 `revs` 为空，只靠前两个字段标注会写成「r10 + 0 个文件」。只在有墓碑时序列化。`baseline_newer` 对混合基线总有值，可以是 0
- **被单值基线取代的混合基线不再画**。计划写「作为普通 git 节点保留」，§6.9 讲的是工作分支挂在它上面的情形，没说取代之后。根目录更新录下 `B20 → M2 → M1 → B10` 之后，混合基线没有 ref、没有分支，留着就悬成一条死分支，和 2026-10-11 真实窗口里「已交付的提交」是同一个样子。所以沿用已交付的判据收起，但**不进 `deliveredFrom`**，它不是谁交付的提交。还有分支骑着、或取代它的 revision 没拉到时照常显示。代价是 graph 上看不到历史上的混合基线，要看走 `git log svn/base`
- **根上的混合基线退回挂 `svn:<floor>`**（计划只写了父边重写）。`init_hybrid` 录的混合基线没有父（`svn_init.rs:945`），不接的话工作分支整条悬在主线外。同样的退路覆盖「父基线的 revision 没拉到」。父边连得上时不改接 floor：分析 §6.9 明说「`svn:<floor>` 或更早」。所以 `M(floor 12) → B10` 画在 `svn:10` 上，不在 `svn:12` 上
- **改了 `svn_log::fetch_missing_anchors`**（计划没列这个文件）：改用 `parse_message` 取 floor。floor 和单值基线的 revision 一样常常没碰过白名单，补不到就没有节点可挂。测试 `svn_fetch_log_includes_the_floor_of_a_mixed_baseline_as_an_anchor`
- **改了 `engine/refs.rs`、`svn_basemap.rs`、`styles.css`**（计划没列），另加 `tests/vitest/CommitRow.test.tsx` 和一批计划没列名的测试：契约测试 +9、`svn_backend.rs` +1、`svnGraph.test.ts` +19（含计划点名的 2 条）、`CommitRow` 2
- **`follow pin refs are not drawn` 的 Rust 名字是 `follow_pin_refs_are_not_drawn`**，其余两条在 vitest 里按计划原文
- **混合基线那一行用标注代替标题**：标题是 `format_message` 拼的同一句话（`svn: r10（另有 3 个文件在更新的 revision）`），两个都显示就是一行说两遍。原文放在标注的 `title`，详情里也看得到
- **`mixedBaselineLabel` 只在接合过的节点上生效**（`vcs === 'git'`）。普通 git 仓库不接合，那里带这种 trailer 的提交不是这个仓库的基线

#### 计划的错误

- **`graph.rs` 不能只是「解析不出 `git-svn-id` 就是 None」**（分析 §6.9）。不改的话混合基线的 `baseline_rev` 确实是 None，但改坏的新 trailer 旁边若还有 `git-svn-id`，`parse_trailer` 会读成单值，与 `read_base_map` 的结论相反。现在统一走 `parse_message`，测试 `a_damaged_new_trailer_is_not_read_as_the_git_svn_id_next_to_it`
- 计划的 Interfaces 说「删除单值基线时，混合基线的父边重写到对应的 svn 节点」，这一步在现有第 ③ 步里已经成立，不是缺口。真正的缺口是下面两条：收起时混进 `deliveredFrom`，以及没有父时接不上

#### 计划没写的坑

- **`refs::collect` 返回全部 ref，`refs/mygitclient/*` 就混在里面**，kind 是 `other`。`graph.rs` 的 revwalk 只 push HEAD、分支和 tag，所以这些 ref 的提交本身不会进图（W28 这一半原本就成立）。但 `buildRefPills` 不看 kind：deliver ref 走快路径时指着分支上的提交，那一行会多一个写着全名的标签。`repoStore.merge` 还把所有 ref 的 hash 当 tip，基线已发布、deliver ref 还没删的那段时间，这个提交收不起来。在 `collect` 里过滤。`collect` 只有 `graph::load` 一个调用方
- **混合基线的形状和「已交付的提交」完全一样**：夹在两条基线之间、子节点全是基线、没有 ref。`deliveredCommits` 改之前会把它列进 r20 的「这次交付包含 N 个本地提交」
- **隐藏的混合基线要继续传递 owner**：只从 `bySvn` 里滤掉，不能不记 `owner`。否则夹在它下面的提交找不到归属的 svn 节点，隐藏了却哪里都不列，测试 `混合基线夹在交付的提交中间`
- **性能实测**（临时测试，跑完已删，开发机 test profile）：5000 条的混合 trailer 一次 `parse_message` 约 4.9ms，293KB；5000 条普通提交（每条 852 字节）合计约 11ms。没有另写只数行数的解析器：两份解析规则会漂移，而 5000 条的基线本身就少见，根目录更新一次就清零
- `parse_message` 的快路径是整段 `contains("Svn-Base-")`，再由原逻辑判行首。子串不存在时原逻辑必然走到 `parse_trailer`，所以两者等价

#### 为什么这么改是安全的

- 只读路径：`load_graph`、`svn_fetch_log` 和前端接合。没有动 ref、索引、工作区，所有测试都在临时 fixture 上
- `refs::collect` 过滤的前缀是本应用独占的 `refs/mygitclient/`，用户的分支、tag、`refs/stash` 不受影响。跟进、交付读这些 ref 走 `run_git`，不经过 `collect`
- `fetch_missing_anchors` 对单值基线取到的值和原来相同（`floor == rev`），多出来的只有混合基线的 floor；失败照旧只跳过
- 单值基线的 `baseline_rev` 仍是同一个数，判定口径从 `parse_trailer` 换成 `parse_message` 只差「最后一段有 `Svn-Base-` 行」的情形
- 收起混合基线不丢数据：它仍在 `svn/base` 的历史里。判据和已交付的提交相同：有 ref 或 HEAD 指着、有分支骑着都不收
- **变异核对**：前端 8 个（混合基线进 `deliveredFrom`、不退回 floor、总是改接 floor、不带墓碑、普通仓库也标、floor 用真值判断、标注与标题都显示、混合基线进映射表），Rust 4 个（混合基线也给 `baseline_rev`、没有墓碑也给 `Some(0)`、单值仍走 `parse_trailer`、快路径判断反了），全部抓到。`refs.rs` 与 `svn_log.rs` 的改动由实现前的红灯覆盖

#### 遗留风险

- **真实窗口未验证**：没有跑 `pnpm tauri dev`。标注的外观（10px、长文案在窄窗口里被截断）、混合基线行的车道、分支线走向都只有 jsdom 与纯函数测试，AGENTS.md §6.1 要求的视觉验证留给收尾
- **标注写的是 `svn/base：...`，但只有当前的混合基线才真有这个分支标签**。同一天多次跟进时，图上 `M2 → M1` 两行都这么写，`svn/base` 的标签只在 M2 上；分支骑着的旧混合基线也一样。计划要的文案，没有区分
- **连续多条混合基线都画出来**：一天跟进五次，工作分支下面就有五行，直到根目录更新才收起。之后要不要折叠，等真实使用
- **混合基线是普通 git 节点，右键菜单照常有 revert、checkout**。这与 svn 节点不同；计划没有要求屏蔽
- 断网且 floor 没缓存时，根上的混合基线仍是没有父的孤立节点，联网补到锚点后恢复

### Task 13：手动跟进（冲突兜底）

实测环境同前（git 2.51.2.windows.1，这台机器全局开着 `rebase.autostash=true`）。探针写成临时集成测试跑在 `TempDir` 里，跑完删掉，下表是它的唯一记录：Bash 工具的 worktree 隔离拒绝在 worktree 外跑 git，所以没有走脚本。

#### 偏离计划

- **journal 用独立的 `Flow::FollowManual`（线序 `follow_manual`），不是计划写的 `flow = follow`**：`Flow::Follow` 的恢复入口是 `resume_follow`，只认 `follow-publishing`；Task 10 又让 `sync_in` 拒绝 `follow` 记录。两边都要改分派才接得上，还会让「这条记录该谁接手」取决于 phase。新 flow 只加一个枚举值，旧 journal 读取不受影响。分派表：

  | 入口 | `sync_in` | `follow` | `follow_manual` | `sync_out` |
  |---|---|---|---|---|
  | `continue_sync` / `abort_sync` | 接手 | `sync_follow_not_supported` | **接手** | `sync_out_not_supported` |
  | `resume_follow` | `svn_follow_wrong_flow` | 接手 | `svn_follow_wrong_flow` | `svn_follow_wrong_flow` |
  | `continue_sync_out` / `abort_sync_out` | `sync_out_wrong_flow` | 同左 | 同左，文案点名手动跟进 | 接手 |

  `follow_manual` 只会写 Start、Stashing、Stashed、Rebase、ApplyingStash。与 `sync_in` 的差别只有三处，都按 `j.flow` 判：`resume_stashed` 接到 `from_baseline_ready`（不走 S3–S7）；`restore_step` 带 `--index`；`finish_forward` 清快照
- **「暂存没保住」是新变体 `SyncInOutcome::StageLost { new_rev: Option<u64> }`，不是 `Done` 的字段**：`SyncInOutcome` 的这些变体在 Rust 测试里有 44 处引用，更新流程也永远不会有这个结局。`new_rev` 为 `None` 表示收尾的是放弃。`StashConflict` 不带标记：冲突时暂存必然没保住，文案直接说；`Settled` 也不带：磁盘上已经是用户处理过的内容
- **`Journal.stage_lost`**：计划没写。崩在放回之后、删记录之前时不会再 apply 一次，只有它记得这件事
- **放弃有一处没有回到原位：放回那一步（`applying-stash`）沿用 `sync_in` 的 `Released`**，分支留在新基线上，备份留在 stash。指令写「放弃回到原分支原位」，但这时工作区可能已有冲突标记，回滚要覆盖它们；UI 上这一步的按钮本来就是「不再自动放回」。其余阶段（Stashing、Stashed、Rebase）都回到原位
- **指令里的 CAS 没有单独实现**：分支唯一的移动者是 `git rebase`，`rebase --abort` 把它放回 `orig-head`，随后 `back_to_orig` 核对 `HEAD == origHead`、`require_our_rebase` 核对 rebase 身份。rebase 在外面走完之后再点放弃时报 `sync_in_state_changed`、记录留着，「继续」能往前走完。没有做成「把分支 reset 回去」：这时分不出分支上有没有用户后来加的提交，往回移动会把它们从分支上摘掉
- **前置条件比计划多四条**，都在写 journal 之前：判定必须是 `InSync`（`svn_follow_manual_stale`，基线又落后了就先跟进）；R1（同分支跟进入口，没录制就没人把关）；分支线性（`svn_follow_merge_in_range`）；白名单外没有已跟踪改动（`svn_follow_manual_outside_changes`，见下面的坑）。**没有**查 git ≥ 2.43：`merge-file --object-id` 只有跟进用，手动跟进走的是 `sync_in` 同款命令
- **`NeedsManual` 带上 `leaks`**：代价是冲突分支上多调一次 `local_only_leaks`，它本来就只看原始 commit，不依赖重放结果。IPC 加字段，`svn_backend` 契约、TS 类型、`describeFollowOutcome` 同步。**其他分支仍然没算**：冲突时 `plan_others` 不跑，算它们要多做一轮重放，而基线发布后下一次跟进会算
- **`baselineOidBefore` 取分支的旧基线（merge-base）**，不是 `svn/base`：`settle_checkout_residue` 拿它认「基线里的字节」，这里两个基线都要认
- **快照的处理**：分支落到基线上之后清（`finish_forward` 开头，与 F6 同义）；放弃时留着，分支没跟上
- **前端比计划多一点**：`handleManualFollow`（确认框 + guardMerge，按钮接线归 Task 11）、`describeManualFollow`、`describeManualFollowAction`、`describeManualFollowOutcome`。按钮沿用更新，话不能照搬：这里没有「回到更新前」，基线也不是这次推进的。`syncActions` 加可选的 `abortLabel`
- **抽出 `require_linear`、`not_hybrid`、`unfinished`**：`manual_follow` 与 `prepare` 共用，错误码不变

#### 计划的错误

- **Interfaces 写 `flow = follow`**：见上。会与 Task 10 的分派打架
- **Files 漏了 `svn_sync_stash.rs`、`svn_journal.rs`、`svn_sync_out.rs`、`commands/svn_sync.rs`、`lib.rs`、`repoStore.ts`、`svnJournal.ts`、`svnSyncIn.ts`**：`--index` 就在 stash 层，新 flow 要过每一个分派点
- **计划只列 `src/components/svnSyncFlow.ts` 的「冲突出口」**：那一层的结局与确认框按 flow 换文案，纯函数落在 `src/engine/svnFollow.ts`

#### 计划没写的坑

`stash apply --index` 的真实形状（分析 §11 的未知项）：

| # | 场景 | 结果 | 处理 |
|---|---|---|---|
| P1 | 暂存补丁的上下文被新 HEAD 改了 | rc=1，`conflicts in index`，status 不变；不带 `--index` 成功 | 退回，`stage_lost` |
| P2 | 暂存打得上，未暂存的部分冲突 | rc=1，`Index was not unstashed`，`UU`；现场与不带 `--index` 冲突时相同 | 走冲突出口，**不能再 apply** |
| P3 | 白名单外有已跟踪改动（暂存或未暂存） | stash 收不走；带 `rebase.autoStash=false` 时 rebase 启动就拒绝，已经 stash 完了 | 动手之前拒绝 |
| P4 | 暂存的内容新 HEAD 已经有了 | rc=0，工作区干净 | 按保住算，内容就在 HEAD 里 |
| P5 | 回到原 HEAD：暂存 + 未暂存 + 新文件 + 未跟踪 | rc=0，`MM`、`A `、`??` 逐项复原 | 放弃路径比更新更好：暂存保得住 |
| P6 | 只有未暂存 | rc=0，等同不带 `--index` | 无 |
| P7 | 内容只在索引里（磁盘等于 HEAD），且暂存补丁打得上 | `--index` 先复位索引，再报「本地改动会被覆盖」：没有未合并条目，**status 却变了**，而这时不带 `--index` 能成功 | 比 status，变了就报 `sync_stash_apply_failed`，不接着写 |

- **能不能退回看现场，不看报错文字**：git 的文案随语言变。比 `status --porcelain -z --untracked-files=all` 的前后，这才是「什么都没写」的证据
- **开发机全局 `rebase.autostash=true` 会掩盖 P3**：探针直接跑 `git rebase` 成功了，加上我们的 `-c rebase.autoStash=false` 才是真实形态。用户机器上同样可能开着，这也是 `from_baseline_ready` 固定这个配置的原因
- **W10-R2 里 stash 收走的 A 就是 SVN 版本**：冲突解成 SVN 版本时放回干净；解成别的内容时，放回又在同一处冲突（base = git 版本，ours = 解的，theirs = SVN 版本）。方向安全，用户会连着见到两次冲突，`manual_follow_stash_conflict_exit_settles_and_keeps_the_backup` 就是这个形态
- **放弃路径上暂存走不到退回**：stash 的 base 就是 `origHead`，补丁必定打得上。`StageLost { None }` 只在崩溃记录里走得到，测试手写 journal 打这条路
- **测试助手 `git()` 会 trim**：第一行的 ` M` 被吃掉，要看前两列用不 trim 的 `porcelain`

#### 为什么这么改是安全的

- **手动跟进永远不自动执行**：入口只有 `svn_follow_manual` 一个命令，前端走确认框。`NeedsManual` 不留 journal，「继续」无从接手（`manual_follow_is_never_started_by_continue_or_resume`）
- **所有拒绝都在写 journal 之前**：拒绝的测试比较全部 ref 与索引字节，并断言没有 stash。例外是白名单外那条：测试自己 `git add` 过，只比 ref
- **不切基线、不跑 `svn update`**：`manual_follow_neither_updates_svn_nor_visits_the_baseline` 断言节点 revision 不变、服务器上更新的内容没被拉下来、reflog 里没有切到基线
- **基线不动**：全程只读 `svn/base`。冲突期间被外部挪走时 `published_baseline` 停下
- **WIP 不丢**：沿用 `sync_in` 的 S1 / S10：按 opid 找、OID 钉住、冲突时保留备份、撞名时一个字节不写（A3、A17）。`restore_wip` 对更新流程逐字不变，`svn_sync_stash` 原有的测试全过
- **`--index` 失败时不留半成品**：干净才退回，不干净报错并保留 stash；冲突走冲突出口。`manual_follow_apply_index_fallback_reports_stage_lost` 断言没有半成品暂存、没有未合并条目
- **测试全部在临时 fixture 里**，没有碰本仓库或任何真实仓库；本任务没有在 fixture 之外执行 rebase、stash

#### 变异核对

脚本 `t13_mutate.py` 在会话 scratchpad，每轮只跑点名的一条测试，写回后 sha256 确认逐字节还原。22 个变异，第一轮抓到 20 个，存活的 2 个：

| 存活的变异 | 原因 | 处置 |
|---|---|---|
| 不核对现场就退回 | 测试形状太弱：磁盘上也有本地改动时不带 `--index` 同样失败，错误码相同 | 测试改成 P7（内容只在索引里），这时不带 `--index` 会成功。重跑抓到 |
| 恢复接受基线上的 HEAD（去掉我加的 `&& !manual`） | 等价变异：手动跟进的 `baselineOidBefore` 是旧基线，HEAD 在基线分支上只可能是新基线，下一句必报同一个错 | **多余代码，删掉**，那段恢复 `sync_in` 原样，判定改用注释说明。测试保留 |

抓到的 20 个按类：

| 类别 | 变异 |
|---|---|
| 放回 | 未合并时也退回；冲突不算丢暂存；退回不报丢暂存；不带 `--index` |
| 结局 | `Done` 不看 `stage_lost`；放弃不看 `stage_lost`；不清快照 |
| 分派 | 恢复走 S3；手动跟进记录拒绝继续；`flow` 写成 `SyncIn`；交付把记录说成更新 |
| 前置条件 | 不查分支已含基线；不查判定；不跑 R1；不查白名单外；白名单外把圈内也算上；不查线性；不查 journal |
| F3 | `NeedsManual` 不算；分支入口丢 `leaks` |

#### 遗留风险

- **`svn_follow_manual` 没有经过 Tauri IPC 验证**（AGENTS.md §7.3 的 `pnpm tauri dev`）：按钮归 Task 11，真实窗口要人来操作。契约测试钉了序列化形状，`path` 参数名没有转换
- **`NeedsManual` 不含其他分支的泄漏**：手动跟进完成后再跟进才算得到（见「偏离计划」）
- **P7 的现场 git 已经改过了**：停在撞名时用户暂存了 stash 也碰过的文件，而且内容只在索引里，「继续」时 `--index` 会把这份暂存从索引里复位。报错、stash 保留，但那份暂存只剩悬空 blob，gc 之前能用 `git fsck` 找回。要几件事同时发生，没有再加预检
- ~~**`stageLost` 可能略微多报**：暂存里只有一部分放不回时，整份都显示未暂存，内容不受影响~~ **更正（Review 3 第 1 条）**：内容会受影响。暂存版本与磁盘不同时，普通 apply 只放回磁盘那一份，暂存那一份原来随 drop 一起没了。现在退回普通 apply 时 stash 与钉子都留着、点名这些路径
- **`Released` 之后分支在新基线上**：与更新一致，见「偏离计划」
- **`sync_in` 同样有 P3 的弱点**：白名单外有已跟踪改动时 S9 才失败。本任务不改更新流程，记录、放弃走得通
- **`status` 多跑两次**：大工作区上放回时多几秒，只在手动跟进里

### Review 1 修复（Task 1–5）

一次独立 review 对 Task 1–5 的 7 条发现，逐条处理如下。实测环境同 Task 0（SlikSvn 1.14.2，系统代码页 1252）。

#### 第 1 条（高）：确认失败时静默丢文件

- **修法**：`HealthMap` 新增 `Unconfirmed { files, reason }`。`JustRecorded` 时这几种失败都返回它：`server_head_rev`、`confirm_absent`、`excluded_nodes`（被排除节点的扫描）、新增的 `svn ls`（第 3 条）、待确认路径不在任何单元底下。`record_mixed_with` 把它变成 `svn_baseline_unconfirmed`，前端 `wizardErrorHint` 补引导文案，引导用户连上服务器后重试
- **测试**：`deletion_that_cannot_be_confirmed_refuses_recording_instead_of_not_checked`、`excluded_node_scan_failure_refuses_recording`（`svn_health`）；`init_refuses_when_a_committed_deletion_cannot_be_confirmed`（`svn_init`，断言 `.git` 整个回退）；vitest 一条
- **注入点**：`svn_tombstone::fault`（`arm(Point)` 返回 RAII 守卫）。仓库里原本没有这类机制；`file://` 仓库造不出「diff 成功、紧接着的查询失败」，集成测试又只能看到非 `cfg(test)` 的 lib，所以是 `#[doc(hidden)] pub` 的线程局部开关。检查全程在调用线程上同步执行，不影响并行的其他测试；生产代码里没有地方 arm 它
- **为什么安全**：只把「放行」改成「拒绝」，方向与改造前的 Lying 相同。`NotChecked` 的既有语义不变：还没走到确认这一步就失败（比如 diff 本身跑不成）时照旧放行，见遗留风险。单值包装与手动体检不产生 `Unconfirmed`，万一走到按 Lying 报

#### 第 2 条（中高）：floor 只看文件

- **修法**：`compute_map` 的 F 取文件与目录节点 revision 的最小值。新增 `svn_nodes::dir_revs`，`floor_after_catch_up` 用同一规则。`svn status` 不给 kind，目录靠「是别的节点的祖先」认；不退成「所有不在基线树里的节点」，否则白名单内的美术文件会无谓地拖低 floor。`svn_map_too_large` 的清单带上目录
- **测试**：`commit_all_without_update_after_colleague_added_file_keeps_dir_floor`（`svn_init`，修前红在 `svn_baseline_excluded_paths`）；`compute_map_floor_includes_directory_nodes`（`svn_nodes`，含「美术文件不拖低 floor」的反例）
- **既有测试**：一条断言都不用改。它们的目录 revision 都不低于文件，F 不变
- **为什么安全**：降低 F 对标签总是安全的，c(f) > F 的文件自动单列，隐含 F 的文件满足 c(f) ≤ F ≤ n(f)。代价是映射多几条
- **行为变化**：提交了白名单内全部文件、没 update 时，映射现在是混合的（这是对的），而混合基线上交付一律拒绝（Task 5），要先 Update

#### 第 3 条（中）：D8

- **修法**：`excluded_files_at_floor` 对点不了名的被排除节点跑 `svn ls -R --xml -- <编码 URL>@<F>`，按 `WScope` 筛，筛出来的进 `candidates`，与点得了名的走同一条 HEAD 确认（还在 → `Excluded`，已删 → 墓碑）。列不出来 → `Unconfirmed`，点名那个节点
- **实测**：`<name>` 是未编码的相对路径；对文件 URL 也返回一条 entry；floor 上不存在时 rc=1、W160013 + E200009，这时返回空。**只认这两个码**：`parse_list` 的文档写明失败时 stdout 也是合法的空清单，其余失败一律报错
- **测试**：`non_ascii_excluded_directory_with_only_non_code_files_does_not_refuse`、`…_with_a_code_file_is_refused_file_by_file`、`…_absent_at_the_floor_does_not_refuse`（`svn_health`）
- **fixture 的坑**：svn.exe 收不到中文参数，`--targets` 也按代码页读。改为以该目录为 cwd 跑 `svn update --set-depth=exclude .`：wc.db 记下 exclude，删目录因 cwd 占用失败、留在 work queue，再从 wc 根 `svn cleanup` 补做
- **为什么安全**：只读（`svn ls`）；W-file 口径与体检同一份，不是 W-file 的本来就不进基线

#### 第 4 条（中低）：copied 的 replaced 节点

- **实测**：`svn rm H; svn cp G@1 H` → `item=replaced copied=true`、无 `revision`，`<commit>` 是复制源的
- **修法**：`NodeStatus` 加 `copied`。`judge` 对 `item` 为 added / replaced、或 `copied`、或 `node_rev` 为 None 的节点跳过 c(f) 比较，按本地改动处理
- **测试**：`copied_replacement_is_a_local_change_not_an_external_advance`（`svn_sync`，用 `svn cp <url>/G@4`，让 `<commit>` 真的比基线新，修前红）；`parse_node_status_reads_commit_rev_author_and_changelist` 补 `copied` 断言
- **为什么安全**：只影响 c(f) 判定，回退判定在它之前；这类节点内容没进服务器，不算前进。提交之后它们变成普通节点，照常被判定

#### 第 5 条（低）：日志更正

- Task 4 遗留风险那条「被排除节点在 floor 上还不存在」已更正：实测点名 diff rc=0、输出为空（`--old=<url>/Code@1 ... -- New`，`New` 是 r2 才有的目录），结果正确；同一条里「`NotChecked` 让录制停下」也是错的
- 同时更正了 Task 4「为什么安全」里 `NotChecked` 那句、`--targets` 放系统临时目录那条，以及「映射取值」的 F 定义

#### 第 6 条（低）：混合 R1 的拒绝

- **测试**：`rebuild_refuses_tree_conflict`、`rebuild_refuses_property_only_conflict`、`rebuild_refuses_a_locked_subdirectory`（`svn_init`）。每条先断言前提形状（`tree-conflicted`、`props="conflicted"` 且没有 `item="conflicted"`、工作副本根上没有 `wc-locked`），再断言错误码、路径与基线指针不动
- 被测代码早就有，这三条一写出来就是绿的，所以做了变异核对，见下

#### 第 7 条（低）：`--targets` 临时文件

- **修法**：删掉 `TargetsFile`，复用 `TempFile`：新增 `for_svn_arg`（优先 `.git/`，不是 git 仓库退回系统临时目录）与 `svn_arg`（在 `repo_root` 底下时给相对路径）。svn 以 `repo_root` 为 cwd，所以相对路径能用，而且纯 ASCII，比「放 `.git/`、传绝对路径」更稳：仓库路径本身是中文时也不受影响
- **测试**：`targets_file_lives_under_git_dir_with_an_ascii_relative_arg`（仓库路径是中文）、`targets_file_falls_back_to_temp_dir_outside_a_git_repo`（`svn_tombstone` 单元测试）；`confirm_absent` 的既有用例覆盖退路
- **没覆盖**：「`%TEMP%` 含代码页表示不了的字符」没有端到端测试：改环境变量是进程级的，会影响并行测试，这里只钉住路径的性质

#### 偏离指令

- 第 1 条的错误构造函数叫 `not_confirmed_error`：`unconfirmed_error` 这个名字已被 `.gitignore` 冲突确认占用
- 第 1 条多了 `Unconfirmed.files`（指令只要求一个变体或错误码），为的是报错时能点名
- `svn_baseline::unique_git_dir_path` 拆出 `unique_file_name`，`TempFile::for_svn_arg` 的退路要用

#### 变异核对

第 6 条与 `…_absent_at_the_floor_…` 是绿着写出来的，各改一处后跑对应测试，用 Edit 还原（`Copy-Item` 恢复会保留 mtime，cargo 不重编译），`Get-FileHash` 确认与变异前逐字节相同：

| 改动 | 结果 |
|---|---|
| R1 不看 `tree_conflicted` | `rebuild_refuses_tree_conflict` 红 |
| R1 不看 `props == conflicted` | `rebuild_refuses_property_only_conflict` 红 |
| R1 不看 `wc_locked` | `rebuild_refuses_a_locked_subdirectory` 红 |
| `svn ls` 的 W160013 不当作「不存在」 | `non_ascii_excluded_directory_absent_at_the_floor_does_not_refuse` 红 |

其余新测试（第 1–4 条）都先在只有接口、没有实现的骨架上跑红，再实现。例外：第 7 条的两条单元测试和 vitest 那条与实现一起写，没有单独跑红。

#### 遗留风险

- **离线时已提交的删除仍会静默丢失**：floor 组的 diff 本身跑不成时还不知道有候选，走 `NotChecked`，录制放行，发布的基线缺这个文件、也没有墓碑。与改造前的离线行为相同，这次没动；联网后手动体检会报 Lying。第 1 条只修了「D 已知」的情形
- **copied 的 replaced 文件上重建**：`compute_map` 报 `svn_map_unknown_node`，拒绝录制（方向安全）。单值录制路径（S7 `capture_single`、D7）会把复制源的字节当 BASE，属范围外，未修
- **空目录认不出来**（`dir_revs` 的文档）：要低于父目录得有人单独 `update -r`。在线时报成漏收，方向保守；离线时可能发布偏高的 floor
- **linked worktree 里 `.git` 目录在 `repo_root` 外**：`svn_arg` 退回完整路径，路径含代码页表示不了的字符时仍会失败（未实测这个形态）
- 每个点不了名的被排除节点多一次 `svn ls -R`，走网络；只在录制时发生
- `svn_tombstone::fault` 留在非测试构建里，每个注入点多一次线程局部读

### Review 2 修复（Task 6–10）

第二次独立 review 对 Task 6–10 的发现，逐条处理如下。探针源码在会话 scratchpad 的 `review2/`，已改写成下面点名的回归测试。回归测试先在只有接口、没有实现的骨架上跑红（`svn_follow` 21 条、`svn_follow_index` 1 条、`svn_health` 4 条、`svn_leak` 7 条），再实现。例外：`rename_failure_during_resume_…`、`trailing_newline_…`、`mark_in_a_repo_without_a_baseline_…` 与实现一起写，用下面的变异或断言核对。实测环境同 Task 0。

#### 第 1 条（高）：快照比 SVN 提交新

- **修法**：`FollowMark` 加 `base_oid`（快照时的 `svn/base`，`#[serde(default)]`），`write_mark` 在读完 HEAD 之后读。`paths_after_mark` 算 `diff-tree <base_oid> NEW`，存进 `Who::after_mark`。可替换的路径 = `mine ∩ after_mark`；`staged_before_mark` 只对 `after_mark` 里的路径给值，其余 `None`，作者是我时走 `KeepStaged`。`base_oid` 缺失、读不到、不是 NEW 的祖先时 `after_mark = None`：空 superseded、全部 `None`
- **测试**：`commit_made_after_the_baseline_was_published_is_not_superseded`（P2a）、`stage_made_after_the_baseline_was_published_is_kept`（P2b）、`switching_to_a_lagging_branch_does_not_wipe_its_commit`（P2c）、`only_paths_that_reached_the_baseline_after_the_mark_are_superseded`（同一个 commit 里一条路径在快照前进基线、一条在快照后）、`mark_without_a_trustworthy_baseline_is_uncertain`（旧格式、读不到、不是祖先三种，commit 与暂存各一遍）、`mark_records_the_baseline_it_was_taken_on`、`mark_in_a_repo_without_a_baseline_has_no_base_oid`；`a_damaged_mark_…` 补 `baseOid` 非法的形状。`lagging_branch_still_supersedes_paths_from_the_earlier_baseline` 原样保留，它是快照更早的形态
- **为什么安全**：只把「可丢弃」收窄，其余路径落回「正常重放 / 保留暂存」。`after_mark` 算不出来时不报错，走拿不准。HEAD 先读、基线后读：两次读之间基线前进时 `base_oid` 偏新，「快照之后」只会算小
- **代价**：跟进只发布了基线（冲突出口）之后再失焦，新快照的 `base_oid` 已是 NEW，上一份快照的判据被覆盖，再点跟进时 F2 ① 不再生效，走冲突出口。方向安全，多一次人工判断

#### 第 2 条（高）：F2 ① 把 Local-Only commit 也换掉

- **修法**：`superseded` 对 `c.local_only` 恒返回空集。真的被带上服务器时重放变空（F3 报告）或者冲突（`NeedsManual`）
- **测试**：`local_only_commit_survives_restore_after_commit`（P1）、`local_only_commit_that_conflicts_goes_to_the_manual_exit_instead_of_being_replaced`
- **为什么安全**：Local-Only 本来就不是「中间版本」，少一条 P 只是回到正常重放。`local_only_leak_on_current_branch_…`、`all_committed_shortcut_does_not_swallow_a_local_only_commit` 不变

#### 第 3 条（高）：`checkout -b` 落在 CAS 与第二次核对之间

- **修法**：第二次核对改成「HEAD 解析到 NEW_TIP」（`head_at`）。在 NEW_TIP 上就往前，`work` 留在新位置；不在才回滚。这样 `update-ref` 把分支挪走的形态也一并落到回滚（原来 `head_on` 看不出来）。`head_left` 判「HEAD 在 NEW_TIP 上、却不在原分支上」，这个状态回滚不了：
  - `back_out`：rename 失败时留着锁和 journal（`keep_for_continue`），不移回分支
  - `settle_branch_and_index`：锁是我们的就往前；rename 失败同样留着
  - 锁被手工删了（`recompose_index`）：照 OLD → NEW_TIP 重拼一份，`PathFacts::default()`（手里没有快照与作者了，全按拿不准），记进 journal，再走往前的路径
- **测试**：`checkout_b_after_the_cas_goes_forward_and_keeps_the_stage`（P3）、`checkout_b_while_crashed_goes_forward`（P3b）、`checkout_b_and_lock_deleted_by_hand_recompose_the_index`、`rename_failure_with_head_on_a_new_branch_keeps_the_lock_for_continue`、`rename_failure_during_resume_with_head_on_a_new_branch_keeps_the_lock`（后两条 Windows 专属）、`head_pointed_at_another_commit_after_the_cas_rolls_back`。**名实不符的两条：** `follow_detects_checkout_b_during_publish_and_rolls_back` 改名 `checkout_b_before_the_cas_is_caught_by_the_second_check_and_rolls_back`，断言不变；`head_moved_while_crashed_rolls_back_and_follows_the_new_branch` 原来手工把 `race` 改回 OLD，改名 `head_on_another_commit_while_crashed_rolls_back_and_follows_that_branch`，用 `symbolic-ref` 造真实形态
- **为什么安全**：拼好的索引就是 NEW_TIP 的索引，HEAD 在 NEW_TIP 上时换入是对的，不管 HEAD 经哪条分支到的。恢复里重拼时 `diff-index --cached OLD` 看的是语义，索引其实已经换过也只是全部走「索引等于 NEW」那一行

#### 第 4 条（高）：钉住的索引保不住暂存 blob

- **修法**：`pin_raw_index` 的树里挂 `blobs/<oid>`：相对 OLD 的全部已暂存 blob。`existing_blobs` 先过一遍 `cat-file --batch-check`，只收对象库里有的 blob：gitlink 是 commit，缺的对象会让 mktree 整条失败（`unreadable_blob_fails_instead_of_losing_the_stage` 要的是 merge-file 那个错误）。`staged` 的读取挪到钉索引之前。`FollowReport.pinned_ref`（`record_prepared` 记，随 journal 过崩溃），前端在丢了或丢弃暂存时点名
- **测试**：`lost_stage_stays_recoverable_after_gc`（P6，真的 `git gc --prune=now` 后 `cat-file`）、`pin_holds_the_staged_blobs_through_gc`（含奇怪路径、暂存删除、i-t-a）、`pin_without_staged_changes_has_no_blobs_directory`、`stage_report_survives_the_crash` 补断言、`svn_backend` 契约、vitest 两条
- **为什么安全**：只多建对象和一层树，不动 ref 以外的东西；保留策略仍是最近 20 个
- **没做**：OLD 不是 pin commit 的父提交，没暂存的条目靠 OLD 可达（分支、reflog）。reflog 过期之后整份索引还原不全，但被丢的暂存内容仍在 `blobs/` 里

#### 第 5 条（高）：泄漏判定三种漏报

- **修法**：`judge_hunk` 收集重叠的 NEW hunk 的新内容（`carried`）。顺序：有起点更早且改变行数的 hunk → Unknown（保持原有规则，`overlapping_earlier_hunk_that_changes_line_count_is_unknown` 不动）；L 纯删除 → 原规则；映射位置逐字相等 → Present；否则按条消费 `carried`：全部查到 Present、查到一部分 Unknown、一条没有才 Absent。行尾不算内容
- **指令之外多修的一处**：「在 h 之前」的 hunk 原来 `continue`，把「碰到边界算重叠」挡住了。L 在 l4 后插 log、同一次提交也改了 l4 时，B→NEW 是 `@@ -4 +4,2 @@`，在 B 里止于插入点，判成没重叠，漏报。现在 `before` 只决定算不算偏移，不再跳过重叠判断；紧贴上边界的 hunk 不触发 `unknown`（位置算得出来）。随之加 `beside_deletion`：L 纯删除时 NEW 只在边界插行，这几行还在，不算碰到（原来右边界会多报一条 Unknown）
- **测试**：`leak_inserted_at_the_same_point_as_a_feature_line_is_present`（a）、`leak_below_a_line_added_in_the_same_hunk_is_present`（b）、`half_a_hunk_on_the_server_is_unknown`（c）、`same_hunk_with_none_of_the_local_lines_is_absent`、`repeated_local_lines_are_counted_not_just_seen`、`insertion_right_below_a_changed_line_is_compared`、`line_inserted_right_above_a_local_hunk_still_shifts_the_mapping`、`pure_deletion_next_to_an_insertion_is_not_a_leak`、`trailing_newline_is_not_part_of_the_comparison`。W21 全部场景原样通过
- **为什么安全**：方向是多报。一行常见内容（`}`、空行）碰巧出现在重叠 hunk 里时会多一条 Unknown 或 Present，告警不阻断手动跟进

#### 第 6 条（中）：`Unconfirmed` 没覆盖 D 已知后的失败

- **修法**：`check_grouped` 加 `incomplete`：录制（tombstoning）时返回 `Unconfirmed`，其余照旧 `NotChecked`。用在第二条起的 diff 失败、路径还原失败（抽成 `repo_rel_of`）、旧墓碑核对失败与「不在任何单元底下」。`files` 取到此刻已知的 floor 删除
- **与指令的差别**：第一条 diff 就失败时仍是 `NotChecked`。这时一条都没比成，通常是断网，改成拒绝等于离线不能录制也不能跟进，推翻 `verified: false` 这条既定语义。指令写的是「已开始比对后」，我按「第一条 diff 有了结果之后」理解
- **测试**：`diff_failure_after_the_comparison_started_refuses_recording`、`first_diff_failing_is_still_not_checked_while_recording`、`unreadable_diff_path_refuses_recording`、`existing_tombstone_check_failure_refuses_recording_with_the_known_deletions`。注入点 `Point::FirstDiff` / `LaterDiff` / `DiffPath`
- **为什么安全**：只把放行改成拒绝，单值包装与手动体检（`tombstoning = false`）行为不变

#### 第 7 条（中）：删 index.lock 的假定被证伪

- **修法**：`FollowSteps.lock_created`：`prepare_index_with` 建锁后立刻回调写进 journal（新增 `Point::IndexLocked`）。恢复停在 `PrepareIndex` 时只有记过才删，没记过走 `require_index_free`，报 `svn_index_busy`，记录留着
- **测试**：`half_built_index_lock_is_deleted_not_renamed` 改到 `IndexLocked`（unwind 会跑 `IndexLock` 的 Drop，真实崩溃不会，所以测试把半成品写回去）、`lock_we_never_recorded_is_left_alone_at_the_prepare_step`、`locked_callback_runs_with_the_lock_held_and_its_failure_releases_it`；`resume_after_crash_between_each_step` 加 `IndexLocked`。没有起活的 `git commit`：探针已证明它在这件事上与一个普通文件同形
- **为什么安全**：崩在建锁与写记录之间（一次文件写的时间）时残锁没人认领，用户按报错提示手工删掉后点「继续」，记录仍走得到删除
- **剩余窗口**：记过 `lock_created`、崩溃、用户手工删锁、别的 git 又建了锁，这时恢复会删别人的锁。要四件事同时发生

#### 第 8 条（中）：分支跟进入口

- **修法**：`SyncStatus.head_behind_base`，`head_behind_base()` 与跟进的 `branch_baseline` 同一口径：附着的非基线分支，merge-base 不是 `svn/base`，且是一条基线 commit。只是提示，读不出来都按 `false`。`describeSync` 在 inSync 且 `headBehindBase` 时给黄色横幅「当前分支落后基线」与 `follow` 按钮；其余判定不被它盖过
- **测试**：`sync_status_says_when_the_branch_lags_the_baseline`（分支在基线上 / 之前、ExternallyAdvanced、落后、detached、分叉点不是基线、无关历史）、`svn_backend` 契约、vitest 两条
- **为什么安全**：校验多一次 `symbolic-ref` 与一次 `merge-base`，只读
- ~~**没做完**：按钮要 `BaselineBanner` 的 `onFollow`，归 Task 11（D7）~~ Task 11 已接

#### 第 9 条（中）：未记录的偏离

- **修法**：`plan_others` 改用 `prev_base`（`previous_baseline`）判范围：普通入口是这次的旧基线；分支跟进入口是 `svn/base` 的父提交。`with_watch_suspended` 记进 Task 10「偏离计划」，不改代码
- **测试**：`lagging_current_branch_does_not_drag_branches_older_than_the_old_baseline`、`branch_entry_scopes_other_branches_by_the_previous_baseline`
- **为什么安全**：只缩小范围，多留下的分支由切换时的「落后基线」处理
- **已知偏差**：交付录的基线父提交是交付 commit，紧接着走分支跟进入口时只含更早基线的分支不移动。方向同上

#### 第 10 条（低）：rename 判定

- **修法**：`record_prepared` 把拼好的索引 `ls-files --stage -z` 的指纹写进 `indexStages`；`index_was_swapped` 比真实索引的同一指纹，相等才算 rename 发生过，读不出来、对不上都按没发生。删掉 `index_is_pinned`
- **与指令的差别**：比的是「等于拼好的那份」，不是「不等于钉住的旧索引」。后者在 rename 之前外部 `git add` 过时仍会判成发生过，留下反向暂存；前者这时往回走，安全
- **测试**：`stat_refresh_after_the_lock_was_deleted_does_not_look_like_a_rename`（先断言索引字节确实变了）；`someone_elses_lock_after_the_rename_is_left_alone`、`unreadable_index_during_resume_rolls_back_instead_of_wedging_the_record` 不变
- **为什么安全**：误判只会走向往回走，文档已说明它安全

#### 变异核对

修复之后再改回去，确认测试抓得到（脚本 `mutate.py` 在会话 scratchpad，写回更新 mtime，每轮 sha256 确认逐字节还原）。14 个全部抓到，每个只跑点名的一条测试：

| 变异 | 抓到它的测试 |
|---|---|
| `existing_blobs` 不过滤类型 | `unreadable_blob_fails_instead_of_losing_the_stage` |
| 不校验 `baseOid` 形状 | `a_damaged_mark_…` |
| 贴上边界的 hunk 也触发 unknown | `insertion_right_below_a_changed_line_is_compared` |
| `take` 不消费 | `repeated_local_lines_are_counted_not_just_seen` |
| 行尾算内容 | `trailing_newline_is_not_part_of_the_comparison` |
| 恢复时不重拼 | `checkout_b_and_lock_deleted_by_hand_recompose_the_index` |
| 恢复时 rename 失败照样回滚 | `rename_failure_during_resume_with_head_on_a_new_branch_keeps_the_lock` |
| `back_out` 不看 `head_left` | `rename_failure_with_head_on_a_new_branch_keeps_the_lock_for_continue` |
| 第一条 diff 失败也拒绝 | `first_diff_failing_is_still_not_checked_while_recording` |
| 旧墓碑核对失败放行 | `existing_tombstone_check_failure_refuses_recording_with_the_known_deletions` |
| 分支跟进入口取不到旧基线 | `branch_entry_scopes_other_branches_by_the_previous_baseline` |
| 分叉点不验基线 | `sync_status_says_when_the_branch_lags_the_baseline` |
| 基线不是 NEW 的祖先也算 | `mark_without_a_trustworthy_baseline_is_uncertain` |
| `staged_before_mark` 不看 `after_mark` | `stage_made_after_the_baseline_was_published_is_kept` |

**探针 P2b 的形状要改：** 探针用 `commit_all` 提交 W，`add -A` 把磁盘上的 a2 也带进 commit，A 不在 OLD→NEW 里，暂存碰不到决策表。回归测试改成只提交 W，这才是「已暂存 ∩ OLD→NEW 改过」的形态。上面最后一行变异确认它真的打到了修复

#### 遗留风险

- ~~**快照写在外部提交之后、判定还没刷新时**：TortoiseSVN 提交后在 Rider 里 `git commit`，切回 jkk，第二档校验出结果之前又失焦。这时前端记的最近判定还是 InSync，快照的 `head_oid` 含那个新 commit、`base_oid` 还是旧基线，新 commit 会被当成中间版本。窗口是一次校验的时间。Task 11 接失焦时可以再收紧（校验进行中不写）~~ Task 11 已收紧：校验进行中不写，读戳那一段也算（`trackSvnCheck`）
- **hunk 滑动**：B→L 与 B→NEW 是两次独立的 diff，内容重复时同一处插入可能被摆在不同位置，判成不重叠。第 5 条只管重叠的 hunk
- **`recompose_index` 报告里的暂存决定是第二次算的**：第一次的 `discarded` 等信息被覆盖。只有 HEAD 被带走且手工删锁时才走到
- 第 3 条之后 `svn_follow_head_moved` 也覆盖「分支被 update-ref 挪走」，文案改成了「HEAD 被别的工具移走」

### Review 3 修复（最终 review）

最终独立 review（第三轮）的 4 条发现，逐条处理如下。实测环境同 Task 0（git 2.51.2.windows.1）。全部在临时 fixture 上跑。

#### 第 1 条（高）：手动跟进 StageLost 之后删掉 stash，只在暂存区的内容永久丢失

- **现场（U2）**：暂存区是方案甲，磁盘继续改成方案乙；同事的改动落在暂存补丁的上下文里。手动跟进的 `--index` 打不上（P1）→ `apply_keeping_index` 退回 `apply_plain` → `StageLost`，而 `restore_step` 照常 `drop_wip` 并拆掉 `refs/mygitclient/sync/<opid>`。方案甲只剩悬空对象，gc 之后找不回来。更新（sync_in）的 S10 本来就不带 `--index`（计划 A8），暂存与磁盘不同时同样丢
- **修法**：
  - `svn_sync_stash::staged_only_paths(oid)`：`diff-tree stash^1 stash^2`（暂存改过）∩ `diff-tree stash^2 stash`（暂存的那份不是工作区那份），即「暂存版本此后只在 stash 里」的路径
  - `restore_step` 干净 apply 之后：手动跟进 `--index` 成功时不查；手动跟进退回普通 apply（`stage_lost`）时一律保留；更新时 `staged_only` 非空才保留，并置 `stage_lost`。保留 = **不 drop、不拆钉子**，删 journal，返回新的 `RestoreStep::StageKept { oid, files }`
  - `SyncInOutcome::StageLost` 加 `stash_oid: Option<String>` 与 `files: Vec<String>`。`None` 只出现在崩溃恢复、stash 已经不在的时候（`Finished if stage_lost`）。`rolled_back` 的尾句对 `StageKept` 单独说明备份在哪：那时记录已删，没有横幅可看
  - 前端 `stageLostDetail`（`svnSyncIn.ts`，更新与手动跟进共用）：点名文件，说暂存的那一份没有丢、在 stash 列表里（给完整 OID），取回办法 `git show <oid>^2:<文件>` 与 `git restore --source=<oid>^2 --staged -- <文件>`（只写暂存区、不动磁盘），也可以在侧栏的 STASHES 里找到；另说明 `refs/mygitclient/sync/` 下有个引用钉着它。`describeSyncOutcome('stageLost')` 原来是「更新走不到这里」的占位，现在更新会走到，按完成 / 放弃分开说。`STAGING_NOTE` 与手动跟进确认框的警告各补一句
- **测试**：
  - `manual_follow_stage_lost_keeps_a_stage_that_differs_from_disk`（U2）：`files == [B]`、stash 列表里有 `mygitclient-sync:mf1`、pin ref 等于 `stash_oid`、`git show <oid>^2:B` 是方案甲，`git restore --source=<oid>^2 --staged` 取回之后索引是方案甲、磁盘不变
  - **名实不符的那条**：`manual_follow_apply_index_fallback_reports_stage_lost` 的暂存是磁盘的子集（暂存「2-mine」，磁盘是「2-mine」加 svn update 合进来的同事那一行），却断言「内容都放回来了，备份可以删」。改名 `manual_follow_apply_index_fallback_with_stage_a_subset_of_disk_keeps_the_backup`，断言改成备份与钉子都在、`files == [B]`（暂存版本缺同事那一行，字面上确实与磁盘不同）。两条共用 `stage_lost_scene`
  - `a_stage_that_differs_from_disk_stays_in_the_stash_after_update`、`a_stage_equal_to_disk_is_dropped_as_before`（`svn_sync_in`）
  - `stage_lost_survives_a_crash_after_the_apply_in_both_directions`：注释改成「stash 被外部删掉」，断言新字段为 `None` / 空
  - `svn_backend` 契约（`stashOid`、`files`）；vitest：`svnSyncIn.test.ts` +3、`svnFollow.test.ts` +1（改 1）、`svnSyncFlow.test.ts` 的结局对象补字段
- **先红**：新测试要新字段，修前编译不过，没有单独跑出断言失败。修前行为的证据是被改掉的那条旧测试：它在 U2 的同一现场明确断言 `stash list` 为空（即 drop 了）
- **为什么安全**：只把「drop」改成「留着」，方向是多留备份；留的只有我们按 opid 压的那一条。崩在 apply 之后、删 journal 之前时，「继续」走既有的 `stashApplied` 分支：查到 stash 还在就是 `Kept`（拆钉子、留 stash，报 `Settled`），备份照样在 stash 列表里，只是不点名文件
- **偏离指令**：
  - 更新里暂存与磁盘相同（`files` 为空）时照旧 drop、报 `Done`：内容都在磁盘上，丢的只是「已暂存」标记，这是计划 A8 的既有行为；有暂存就留一条 stash 会让日常更新都多一条备份。手动跟进按指令一律保留
  - 「暂存与磁盘不同」按「`^1 → ^2` 改过，且 `^2` 与 stash 树不同」判，比「`stash^2` 与工作区树的全部差异」窄：只在磁盘上改、没暂存的路径不算，那里的暂存版本就是旧 HEAD，历史里有
  - 取回命令没写 `git stash apply --index <ref>`：在新 HEAD 上它正是刚才打不上的那条命令
- **代价**：钉子不拆，用户在 侧栏 STASHES删掉那一条之后对象仍被 `refs/mygitclient/sync/<opid>` 引用，不会被 gc，也没有自动清理（提示里说了要一并删）

#### 第 2 条（中）：落后分支走对象层跟进后，「本地改动」出现反向修改

- **现场**：「仅切换」或外部工具切到含旧基线的分支，git 把旧内容写回磁盘（SVN 看着 M）；判定 InSync、`headBehindBase`。点跟进 → HEAD 前进到新基线、磁盘仍是旧内容 → git diff 显示把 SVN 内容撤回，`commit -a` 会造出回退 commit。修前的回归测试结局是 `Done`（已跑红）
- **修法**：`svn_follow::stale_disk_paths(scope, old_base, old_head, tip)`：只看 `scope..old_base` 改过（分支落后的那一段）且 `old_head → tip` 也变的路径；HEAD 里有、磁盘上是普通文件的用 `git hash-object --stdin-paths` 与 HEAD 的 blob 比，HEAD 里没有的要磁盘上也没有。`prepare` 在重放成功（`Replayed::Moved`）之后算，非空就改成 `Target::Conflict { at: old_head, files, stale_disk: true }`，不移分支、不碰索引。`FollowOutcome::NeedsManual` 加 `stale_disk`（`staleDisk`）。前端 `describeFollowOutcome` 对它换一套话（标题「这次要手动跟进」，不说冲突，说清只移分支会造出回退、手动跟进会把新内容写回磁盘）；横幅「当前分支落后基线」补一句「用『仅切换』或别的工具切过来时会改为提示手动跟进」
- **实测**：`--stdin-paths` 逐个按路径走 attributes（autocrlf=true、磁盘 CRLF 时与 `ls-files -s` 的 blob 相同，与 `--path=` 结果相同，`--no-filters` 不同）
- **测试**：`branch_entry_sends_a_stale_disk_to_manual_follow`（探针形状：切到旧基线上的 `old`、磁盘写回 a1；跟进返回 `NeedsManual { staleDisk: true, files: [A], at: HEAD }`，HEAD、分支、索引字节都不动；随后手动跟进 `Done`，磁盘 a2、`status` 干净）；`a_stale_disk_on_a_lagging_branch_also_stops_a_recording_follow`（同一现场再来一次外部提交：基线照常发布，分支交给手动跟进；这次 update 进来的 B 磁盘上是新内容，不在名单里）；`a_file_the_lagging_branch_does_not_have_counts_as_stale_when_it_is_gone_from_disk`（变异核对后补，见下）；`switching_to_a_lagging_branch_does_not_wipe_its_commit` 与 `follow_conflict_publishes_only_baseline` 补断言 `stale_disk == false`；`svn_backend` 契约；vitest +1
- **探针形状要改**：指令写「切到 old 分支」。`old` 若在第一次跟进之前就建好，F5 ④ 会把它一起移到新基线，切过去磁盘就是新内容了；测试改成跟进之后再从旧基线 `checkout -b old`
- **偏离指令**：
  - 不只 InSync 入口：ExternallyAdvanced 且分支落后（scope ≠ 旧基线）时同样检查。同一机制，同样会造出回退；这时走既有的冲突出口，基线照常发布
  - 重放冲突优先于磁盘核对：两者都要手动跟进，先报冲突让 `switching_to_a_lagging_branch_…` 继续测它原本要测的 F2 ①
  - 「会变」按 HEAD 与 NEW_TIP 比，不按 HEAD 与 `svn/base` 比：分支自己也改过这条路径时，重放结果才是 HEAD 移过去之后的样子
  - 符号链接、子模块不判，按「不是旧内容」处理，与修前相同
- **为什么安全**：只把一次对象层跟进改成 `NeedsManual`，后者不写磁盘、不动分支与索引；InSync 入口连基线都不动。手动跟进的 rebase 覆盖的是这些文件，它们的内容等于旧 HEAD 的 blob，在历史里，不是只在磁盘上的用户数据（git 看着干净，stash 也不会收它们）
- **代价**：分支落后时每次跟进多两次 `diff-tree` 与一次 `hash-object`，只算落后那一段的路径

#### 第 3 条（低）：`set_svn_user` 的临时文件名固定

- **修法**：临时文件名改成 `mygitclient.json.<pid>-<进程内序号>.tmp`；rename 遇 Windows os error 5 / 32 按 50…550 ms 退避六次（合计约 1.5 秒），与 `svn_follow_index` 换入索引同一口径（W24）
- **测试**：`concurrent_set_svn_user_calls_do_not_collide_on_the_temp_file`（8 个线程各写 25 次，全部成功、读回合法、`.git` 下没有残留 `.tmp`）。修前跑红：`替换配置文件失败: The system cannot find the file specified. (os error 2)`
- **为什么安全**：只换名字与加重试，写入仍是「临时文件 + rename」；失败时照旧删掉自己的临时文件。重试只在 Windows、只对那两个错误码

#### 第 4 条（低）：前端死代码与无效字段

- **`handleManualFollow`**：没有调用方。Task 13 留它是为了 Task 11 接按钮，Task 11 改成手动跟进只经由跟进冲突的确认框（`confirmManualFollow`）与「切换并手动跟进」发起，两处都已在 `exclusive` 里。删掉导出，内部的 `manualFollow` 留给 `checkoutBehind`，行为不变。它的 5 条测试：顺序与「同一时间只走一个流程」已由 `handleCheckoutBehind` 的同名测试覆盖，删掉；「不确认就不发」「merge driver 没确认不弹框」「冲突与暂存没保住要看见」改成经 `handleCheckoutBehind` 走到同一个 `manualFollow`
- **`describeCheckoutBehind` 的 `busyLabel`**：这个框不走 `action`（切完还要接着弹手动跟进的框），忙碌态不会出现。删掉，返回类型改成 `Omit<SyncConfirmModel, 'busyLabel'>`；vitest +1 断言没有这个字段
- **`RepoTab` 直接 `invoke('checkout')`**：既有写法（组件绕过 store，AGENTS.md §7.5），不在这次范围，不改

#### 变异核对

脚本 `mutate.py` 在会话 scratchpad 的 `r3/`，每次改一处，只跑点名的测试，写回后 sha256 确认逐字节还原：

| 变异 | 结果 |
|---|---|
| `staged_only_paths` 不取交集（暂存过就算） | `a_stage_equal_to_disk_is_dropped_as_before` 红 |
| 更新不查 `staged_only` | `a_stage_that_differs_from_disk_stays_in_the_stash_after_update` 红 |
| `StageKept` 之后照常 drop | `manual_follow_stage_lost_keeps_a_stage_that_differs_from_disk` 红 |
| 磁盘核对比反 | `branch_entry_sends_a_stale_disk_to_manual_follow` 红 |
| HEAD 里没有的路径不算 | 第一轮**存活**：没有测试覆盖「基线新增、旧分支没有」的文件。补 `a_file_the_lagging_branch_does_not_have_counts_as_stale_when_it_is_gone_from_disk`（切过去时 git 删掉它，跟进点名它，手动跟进后写回）后红 |

没做的变异：去掉 `scope..old_base` 的过滤。现有场景里过滤掉的路径磁盘上都是新内容，结果不变；它要防的是「这次 svn update 进来、用户又在磁盘上改回旧内容」被误判成旧内容

#### 遗留风险

- **`pnpm tauri dev` 未验证**：`SyncInOutcome::StageLost`、`FollowOutcome::NeedsManual` 加字段，没有改命令签名与参数名，靠 `svn_backend` 契约测试钉住线上形状
- **钉子不会自动清理**（第 1 条「代价」）；侧栏 STASHES里删掉那条之后，对象要用户自己 `git update-ref -d` 才能回收
- **崩在 StageKept 落盘之后、删 journal 之前**：「继续」报 `Settled`，不点名暂存与磁盘不同的路径，钉子被拆，备份只靠 stash 列表。这期间用户又在外部删掉了那条 stash 的话，「继续」照旧拆钉子、报 `StageLost { stashOid: null }`，暂存那一份就没有引用了（用户自己删的 stash，没有再加保护）
- **磁盘内容恰好等于旧 HEAD 的路径是用户有意改回去的**：第 2 条会让它走手动跟进，rebase 把它写成新内容；旧内容在历史里（旧 HEAD），但不会出现在 stash 里
- **路径以 `"` 开头时 `hash-object --stdin-paths` 会按 C 风格引号解析**，算出的值对不上，按「不是旧内容」处理，回到修前行为

### 性能（E2E 之后，用户选方案 A）

#### 背景与测量

- 起因：用户在 e2e fixture 里感到「跟进要 4 秒左右」，担心 500GB 工程上要几分钟。没有公司机器的数据；以前的参考只有
  `2026-10-30-stage-refresh-perf-decisions.md`（git，15,000 文件：开发机约 140 ms / 公司机器约 1 秒一遍，冷缓存 53 秒）。
- 测法：在 `git_cli` / `svn_cli` 的子进程入口临时加计时（环境变量 `JKK_PROC_TRACE`，**不提交**），`pnpm tauri dev` 里点跟进，
  按跟进前后的标记截取。开发机，debug 构建。
- e2e fixture（7 个文件、6 条分支）：7.7 秒，170 个子进程（git 143、svn 27），子进程合计 7.5 秒；单个约 45 ms，几乎全是起进程的固定开销。
- 15,000 文件 fixture（`50 × 20 × 15` 个 `.cs`，与 git 基准同形；1 条分支）：**18.3 秒**，85 个子进程。最大的一块是
  `git add -A -- Code/` 7.1 秒 + `write-tree` 0.9 秒；svn 合计 4.6 秒（`status` 每遍约 0.2 秒、扫了约 10 遍，`info -R` 1.1 秒，
  体检 `diff --summarize` 1.4 秒）。
- 单次扫描（同一 fixture，取 3 次最小值）：`status --xml` 159 ms、`status -v --xml` 226 ms、`info -R --xml` 994 ms、
  `diff --summarize`（`file://`）1181 ms、空 `git --version` 40 ms。
- 计划外的发现：同一 fixture 初始化 2 分 32 秒，其中 `git add` 2 分 13 秒。初始化一次性，不在本轮范围，记一笔。

#### P1：录制的候选索引沿用 stat

- **计划没写的坑：** 跟进录新基线用 `TempIndex::empty`，条目没有 stat，`git add -A` 只能把白名单全量读出来哈希——和
  10-28 现场「索引没 stat 冷缓存 53 秒」是同一个病，只是这次发生在每次跟进的临时索引上。
- **做法：** `TempIndex::from_tree_keeping_stat(旧基线)`：复制真实索引（保留 mtime）再 `read-tree -i -m <旧基线>`，复用
  `swap_index_to_tree` 已有的 `build_index_keeping_stat`（含清 skip-worktree / assume-unchanged、失败退回空索引）。
- **实测（15k fixture，磁盘上另有 1 处本地修改、1 个新文件）：** 空索引 add 7.9 s + write-tree 1.3 s；沿用 stat 时
  read-tree 0.18 s + add 0.14 s + write-tree 0.06 s。两边 `write-tree` 得到同一个树 OID。
- **为什么这么改是安全的：**
  - 原来从空索引起步是为了甩掉服务器上已删除的文件。从旧基线起步时这些文件磁盘上已不在，`add -A` 一样摘掉。
  - 两种起点只在一处不等价：旧基线里有**现在已不算 W** 的文件（白名单或扩展名收窄过）。它是已跟踪文件，`add -A` 会留下，
    R2c 纳入集合核对报 `include_set_mismatch`。这时 `record_from_old_base` 退回空索引按原口径重录一次，结果与改动前相同。
  - stat 只是缓存，内容仍以磁盘为准：复制时保留了索引文件的 mtime，racily clean 的条目照样重新哈希（同 `swap_index_to_tree`
    的论证）；assume-unchanged / skip-worktree 被清掉，`add -A` 不会因为它们跳过磁盘。真实索引只被读、不被写。
- **偏离：** 原方案 A 只列了「git 只读查询改 libgit2」与「svn 去重」；P1 是测量后新发现的最大项，一并做。`sync_in` S7 与
  交付用的 `TempIndex::from_head` 有同样的问题，**没改**（不在本轮范围，待用户决定）。
- **测试：** `svn_index_stat::a_temp_index_from_a_tree_keeps_stat_for_entries_the_tree_did_not_change`（先编译不过后绿）；
  `svn_follow::follow_records_the_same_tree_an_empty_index_would`（改动前后都绿，守行为不变）；
  `follow_falls_back_to_an_empty_index_when_the_old_base_has_files_outside_the_whitelist`（只换起点、不带退路时红：
  `include_set_mismatch`，补退路后绿）。

#### P2：准备阶段的 svn 扫描去重

- **计划没写的坑：** 准备阶段里失同步判定、R1 前置校验、R3b 体检、R3c 各调一次 `enumerate_wcs`（`info` + 全量 `status`）
  与 `read_node_status`（全量 `status -v`），同一片白名单扫了约 10 遍。
- **做法：** `svn_cli::with_scan_cache`：范围内同一条 `svn info` / `svn status`（同目录、同参数）只跑一次，线程局部；
  跟进只把 `prepare` 包进去，每次重试是新的范围。R3c 重读之前 `forget_node_scans` 丢掉 `status -v` 的结果；
  `info` 与找 external 的 `status` 留着（工作副本布局，不是 revision）。发布前的复核不在范围内。
- **为什么这么改是安全的：** 范围内跟进本身不发 SVN 写命令、不写工作区；外部在范围内的改动由 R3c（读最新节点状态）与
  体检（直接问服务器）两道兜住。`a_stale_scan_cache_cannot_record_a_lying_baseline` 先缓存、再在 TortoiseSVN 侧提交、再录，
  断言被拦下、基线不动。
- **偏离：** 没有改成「把节点状态一路传参」：那要动 `check_sync` / `run_preflight_with` / `record_mixed` / 体检四处的签名，
  它们还被初始化、重建、更新、交付共用；缓存只在跟进范围内生效，别的入口行为不变。
- **测试：** `svn_cli` 三条（范围内复用、忘掉后重读、出范围不复用），两个变异（`forget` 改成空操作、范围结束不清）各抓到一条。

#### P3：git 只读查询改走 libgit2

- 新模块 `engine/git_read.rs`：`git_dir`、`resolve`（`rev-parse -q --verify`）、`symbolic_head`、`commit_message`
  （`cat-file commit` 空行之后的原始字节）、`merge_base`、`is_ancestor`、`tree_paths`（`ls-tree -r --name-only`）、
  `unmerged_paths`（`diff --diff-filter=U`）、`ignore_case`。替换了 `svn_baseline::git_dir` / `read_baseline`、
  `svn_sync::baseline_oid` / `tree_files` / `head_behind_base`、`svn_sync_check::current_branch` / `rev_parse` /
  `unmerged_paths`、`svn_basemap::read_base_map`、`svn_follow::branch_baseline` / `previous_baseline`、`svn_replay::rev_parse`、
  `svn_scope::baseline_files`、`svn_follow_mark::write_mark`、`svn_include_check::ignore_case`。`check_git_version` 进程内
  只在「够新」时记一次。
- **「不存在」与「坏了」：** `read_baseline` / `baseline_files` 原先按退出码 1（不存在）/ 128（损坏）区分。`resolve` 只把
  `NotFound` / `InvalidSpec` / `Ambiguous` 当成没有，ref 文件损坏返回 `Err`，`git_read::a_corrupt_ref_is_an_error_not_absent` 钉住。
- **计划没写的坑：** `core.ignoreCase` 仓库里不设时两条路都取全局配置（本机 `~/.config/git/config` 里是 true），契约测试
  改成与 CLI 比，而不是断言 false。
- **没改的（偏离方案 A 的「全部只读查询」）：** `diff-tree`（重命名检测与 `--literal-pathspecs`）、`log` / `rev-list` 的排序
  （`--topo-order --reverse`）、`worktree list`（prunable）、`for-each-ref` 的 `%(upstream)`。两边语义有差，换了要逐条复核；
  它们在 e2e 6 条分支时约占 1 秒，留作后续。
- **测试：** `tests/git_read.rs` 9 条，每条在同一 fixture 上与原 CLI 输出逐一对照（含游离 HEAD、冲突态、损坏 ref、
  linked worktree、中文路径）。

#### 结果

| fixture | 改动前 | P1 + P2 | P1 + P2 + P3 |
|---|---|---|---|
| 15,000 文件，1 条分支（跟进耗时 / 子进程数） | 18.3 s / 85 | 10.1 s / 85 | 8.0～8.2 s / 54 |
| e2e fixture，分支逐渐增多（场景 1～14A 各次跟进） | 4.6～8.8 s | — | 2.6～4.8 s |

- 都是开发机上的 debug 构建。15k 时还有约 2.4 秒花在子进程之外（debug 构建下解析 4MB 的 svn XML、libgit2 未优化），
  release 构建会更少，没有单独测。
- 剩下的大头：体检 `svn diff --summarize` 约 1 秒（要问服务器，真实网络下更慢）、`svn info -R` 约 0.9 秒（找被 depth-exclude 的
  节点），都是正确性检查，没动。
- **真实窗口回归（2026-11-01）：** 新 fixture 上用脚本驱动 `pnpm tauri dev`，按测试指引跑场景 1、2、3、4、6、7、10、12、14A，
  27 项核对全过。焦点改用页面内派发的 `window` blur / focus（与真实切窗口走同一组监听）：用户正在用电脑时系统不让抢前台。
  第一轮脚本自己的错误（报告弹框被提前关掉，用户名没存上）导致场景 2 起连锁失败，修脚本后在新 fixture 上重跑全过。
- 临时计时代码（`JKK_PROC_TRACE`）已删，不进提交。
- **遗留：** 「更新」（`sync_in` S7）与交付用的 `TempIndex::from_head` 同样从没有 stat 的索引起步，15k 规模下也要多花约 8 秒；
  初始化在 15k fixture 上 2 分 32 秒（`git add` 2 分 13 秒）。都不在本轮范围，待用户决定。

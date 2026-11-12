# 跟进后把 Local-Only 补丁放回磁盘：实现日志

计划：`2026-11-12-restore-local-only-patches.md`

## 进度总账

| Task | 状态 | 产出 | 验证 |
|---|---|---|---|
| T1 | 完成 | `engine/svn_restore_local_only.rs` 的 `restorable_paths`；`svn_follow.rs` 报告加 `restorable` | `svn_restore_local_only.rs` 10 条全过；`svn_follow_snapshot.rs` 的普通 commit 合错用例断言 `restorable` 为空；`svn_backend.rs` 钉住字段名与旧 journal |
| T2 | 完成 | `restore_local_only` | 同上 |
| T3 | 完成 | `commands/svn_sync.rs::svn_restore_local_only`、`lib.rs` 注册、`repoStore.ts::svnRestoreLocalOnly` | clippy 0 告警；`pnpm build` |
| T4 | 完成 | `svnFollow.ts` 类型与 `restorablePaths` / `describeRestoreOutcome`；`FollowReportDialog.tsx` 的 `RestoreRow`；`RepoTab.tsx` 接线；`styles.css` | `pnpm test` 1575 过；`pnpm test:layout` 152 过（长报告场景加了 100 个可放回文件） |
| T5 | 完成 | E2E `2026-11-01-svn-mixed-baseline-follow-e2e.md` 场景 9B 与覆盖表 | 文档；真实窗口未跑 |

## 决策条目

### 总体

- **偏离使用报告：不自动写回，做成按钮。** 使用报告给了两个选项：「自动写回磁盘」或「报告里给按钮」。这里选按钮。理由：`svn_follow.rs` 模块头的不变量是「全程不写磁盘」，崩溃恢复、journal、自动跟进都按这个前提设计。把写盘塞进跟进，要么进 journal（多一种崩溃状态），要么在 journal 外做（中途失败没有记录）。按钮是一个独立、幂等的命令，失败了就是现在的状态：只报告。

### T1：跟进时挑出能放回的

- **为什么安全：跟进仍然不写磁盘。** `restorable_paths` 只读：git2 读树与 revwalk，`git hash-object --stdin-paths` 不写对象库。出错只记日志、`restorable` 为空，不影响跟进结果。
- **计划没写的坑：跟进时不能查索引。** 算 `restorable` 时新 tip 和新基线都还没发布，索引也还是旧的，拼新索引是在发布阶段。所以「索引 = HEAD」这一条只在按钮里查，`restorable_paths` 不查。
- **偏离计划：「普通文件」只看 HEAD 一侧。** `svn/base:p` 只要求存在：mode 不同时 hash 也比得出来，不用单独判。

### T2：按钮

- **为什么安全：每条条件现场重验。** 不信报告里的 `restorable`：从报告弹出到点按钮，用户可能改了磁盘、暂存了别的版本、又提交了普通 commit。这三种情况各有一条集成测试（`user_edit_on_disk_after_the_report_is_not_overwritten`、`staged_version_is_not_overwritten`、`a_normal_commit_touching_the_file_makes_it_not_restorable`）。
- **为什么安全：写盘的窗口。** 磁盘 hash 是点按钮时、紧挨着写之前算的（不是跟进时那一次）。它与 `checkout-index` 之间仍然隔着一次进程启动，持同步锁也挡不住用户在编辑器里保存。真撞上了，用户的那次保存会被分支版本盖掉。代价有上限：被盖掉的是一次刚保存的编辑，而能放回的前提是磁盘 = SVN，编辑之前的版本在 SVN 里。没做「写之前再 hash 一次」，因为它缩小不了这个窗口。
- **为什么安全：空参数。** 列表为空时直接返回、不起 git；`checkout-index` 走 `--stdin -z`，路径不上命令行，也不会遇到「空 pathspec = 全部」（AGENTS.md §6.2）。
- **计划没写的坑：写回之后要核对一次。** `checkout-index` 按 smudge / eol 写出的字节，再按 `git add` 的规则 hash，按理应当回到 HEAD 的 blob；过滤规则不对称时可能对不上。对不上的写进 `skipped`，原因写明「已经写回磁盘，但内容与分支不同」。核对本身出错时，文件已经写了，按写成算、记日志，不报「什么都没写」的错。
- **计划没写的坑：fixture 的白名单只收 `.cs`。** 用 `.lua` 新建文件时，第一次跟进返回 `svn_follow_nothing_to_do`：文件不在 include 集合里。测试改用 `.cs`。
- **计划没写的坑：fixture 不能用中文路径。** 本机 ACP 1252，`svn add` 收不了中文参数（与 `svn_follow.rs` 的 `follow_handles_paths_with_spaces_and_percent` 同因）。路径改用空格加 `%`。中文路径在 git 这一侧只经过 `--stdin -z` 与 git2，不经命令行。
- **偏离计划：错误码。** 同步中途为 `svn_restore_unfinished_sync`；索引被占沿用跟进的 `svn_index_busy`，前端文案一致；HEAD 不在工作分支上为 `svn_restore_not_on_branch`；git 正忙为 `svn_restore_git_busy`。

### T3：命令

- **为什么不挂起 watcher。** 跟进命令都用 `with_watch_suspended` 包住，因为它们中途的索引状态不能被刷新读到。这里只写工作区文件，写完正需要「本地改动」刷新；前端也在回调里主动 `refreshStatus`。
- 参数 `path` / `paths` 都是单个词，snake_case 与 camelCase 相同，不存在转换错位。
- **真实窗口验证（AGENTS.md §7.3）：** 2026-11-12 `pnpm tauri dev --config`（隔离 identifier `com.momadacoding.jkk.verify91`），核对运行的是本 worktree 的 `target\debug\app.exe`。经 CDP 调 `invoke('svn_restore_local_only', …)`：
  - 空列表返回 `{"restored":[],"skipped":[]}`；
  - 普通 git 仓库返回结构化 `GitError`（`git_cli`，找不到 `svn/base`）；
  - 参数名故意写成 `pathz` 时报 `missing required key paths`。
  验完删掉了两个隔离目录。按钮本身没在真实窗口里点：要造 SVN 混合仓库，见 E2E 场景 9B。

### 验证中遇到的无关失败

- `commands::watch::tests::stage_and_unstage_only_touch_the_index` 时过时不过：单独连跑，一次过、一次挂，挂时报 `.git\hooks` 出现了事件。本分支没有改 `watch.rs`、`vcs/`，master 从 3a9fb96 到 c850abd 也没改它们；同一天 9.4 分支的全量测试里它是过的。判断是环境问题（临时仓库的 `.git\hooks` 被外部进程碰了），不是本改动引起的，没有修。
- 全量 `cargo test` 两次被 Claude Code 因内存不足中止，第二次停在 27 个测试二进制（519 过）。改为按改动范围分批跑：lib（跳过上面那条）、`svn_follow_snapshot`、`svn_backend`、`svn_restore_local_only`、`svn_follow`、`svn_follow_hooks`，全过。

### T4：前端

- **按钮放在「需要你留意」的最后。** 紧跟「分支与磁盘对不上」那一节（它总在 danger 级、排在前面）。不放进 `Section` 组件：`Section` 是 engine 里描述出来的纯展示模型，加按钮就要往模型里塞回调。
- **成功之后按钮消失**，避免重复点；出错时按钮还在，可以再点；进行中禁用。三种情况都有组件测试。
- 长报告的 layout 场景加了 100 个可放回文件，量的是按钮条在内容更长时是否还够得着。

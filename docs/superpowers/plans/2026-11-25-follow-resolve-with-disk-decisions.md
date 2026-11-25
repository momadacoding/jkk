# 用磁盘上的内容跟进：实现日志

计划：`2026-11-25-follow-resolve-with-disk.md`。

## 进度总账

| Task | 状态 | 产出文件 | 验证 |
|---|---|---|---|
| 1 重放引擎替换口子 | 完成 | `svn_replay.rs`、`svn_deliver.rs`（传 `no_resolved`）、`tests/svn_replay.rs`、`tests/replay_inproc_identity.rs` | `cargo test --test svn_replay --test replay_inproc_identity` 18 + 1 通过（新增 4 个） |
| 2 磁盘判定 | 完成 | 新建 `svn_follow_disk.rs`、`git_cli::run_git_code_bytes`、`tests/svn_follow_disk.rs` | `cargo test --test svn_follow_disk` 19 通过 |
| 3 准备阶段探测与替换 | 完成 | `svn_follow.rs`、`tests/svn_follow_resolve_with_disk.rs`、`tests/svn_follow.rs`、`tests/svn_backend.rs` | 新测试 6 通过；`svn_follow` 35、`svn_backend` 144、`svn_follow_auto` 43、`svn_follow_snapshot` 24、`svn_follow_hooks` 1、`svn_follow_reuse_base` 2、`svn_follow_reuse_safety` 12 通过 |
| 4 外部对比 Worktree | 完成 | `external_diff.rs`、`tests/external_diff.rs` | `cargo test --test external_diff` 28 通过（新增 2 个） |
| 5 Tauri 命令与 store | 完成 | `commands/svn_sync.rs`、`lib.rs`、`src/store/repoStore.ts`、`src/engine/externalTools.ts` | 真实窗口里 `svn_follow_with_disk { path, files }` 调通（Task 8） |
| 6 前端文案 | 完成 | `src/engine/svnFollow.ts`、`tests/vitest/svnFollow.test.ts` | 新增 5 个用例 |
| 7 冲突框组件与流程 | 完成 | 新建 `FollowConflictDialog.tsx`、`svnSyncFlow.ts`、`RepoTab.tsx`、`styles.css`、`tests/layout/{scenarios.ts,harness.tsx}`、`tests/vitest/{FollowConflictDialog.test.tsx,svnSyncFlow.test.ts}` | `pnpm test` 1931 通过；`pnpm test:layout` 新场景全过 |
| 8 验证与收尾 | 完成 | — | 两轮真实窗口 E2E 通过；独立 review 的高危 1 条、中 2 条、低 2 条已修（见下）；`cargo test` 全部测试二进制分批跑完，全过 |

## 决策条目

### 开工前

- **计划没写的坑**：worktree 隔离的会话不允许在临时目录里直接跑 git，`git merge-file --object-id` 的行为放进 Task 2 的 fixture 测试里实测，不凭记忆。
- **偏离设计**：设计第 6 节第 1 条「SVN 状态不是冲突」不单独实现。跟进的预检 `svn_preflight::check_recordable` 已经拒绝白名单内的任何 SVN 冲突（含树冲突、属性冲突），带着冲突走不到冲突框。
- **偏离设计**：设计第 7 节初稿写「不通过就停在冲突框」。冲突框是前端模态框，命令返回后才能更新，所以实现为「命令返回新的 `NeedsManual`，前端按新结果重开冲突框」。

### Task 1

- **偏离计划**：`ReplayOutcome::Conflict.files` 改成「没解决的冲突文件」。没有替换时与原来相同，交付与跟进现有调用方不受影响。
- **偏离计划**：`c_prime_tree` 抽出通用的 `edit_tree(treeish, edits)`，C' 与「冲突树换条目」共用同一套临时索引写法。零 OID 的长度跟着传入 OID 走（原来跟着 `c.oid`），SHA-256 仓库下仍然对。
- **为什么安全**：替换只在 merge 冲突、且冲突路径**全部**有指定结果时生效；有一个没给就照旧冲突。用过替换之后这条链既不查也不写 memo（`private`），测试 `resolved_commits_and_their_descendants_stay_out_of_the_memo` 钉住「同一份 memo 再给另一条分支重放仍然冲突」。冲突之前的 commit 照常命中 memo，与其他分支共用同一个 OID（不会造出重复 commit，R20）。

### Task 2

- **计划没写的坑**：`run_git_code` 把 stdout 过 `from_utf8_lossy`，GBK 文件会被换掉字节，和磁盘比不上。新增 `git_cli::run_git_code_bytes`。
- **计划没写的坑**：`merge-file` 的退出码是冲突个数（封顶 127），0 是没有冲突，其余是出错。merge-tree 说冲突、merge-file 说不冲突时按不能用处理（假阴性，安全）。
- **偏离设计**：设计第 6 节表里的「磁盘 ≠ 分支旧内容 / ≠ 新基线」不读 `old_head` 与 `new_base`，直接比 stage 3 与 stage 2：前置条件保证只有一个提交改过这个文件，stage 3 就是分支旧内容，stage 2 就是新基线里的版本。
- **新增检查**：超过 8 MB 不做逐行核对；两边 mode 不同（可执行位）不算文本内容冲突。
- **实测**：`merge.conflictStyle=diff3 / zdiff3` 配在仓库里时判定照样「能用」（`still_usable_when_the_user_configured_diff3_or_zdiff3`）。diff3 的 `|||||||` 段落在冲突块里面，本来就不参与比对；`-c merge.conflictStyle=merge` 是防御性的。
- **为什么安全**：冲突块以外按行逐字节比（含行尾），磁盘内容按 `git add` 的过滤规则写成 blob 后再读出来比（CRLF 用例钉住）。少了 SVN 的非冲突改动、多了冲突块外的手改都判不能用（各有一个用例）。

### Task 3

- **偏离计划**：`ConflictDetail.disk` 用 `Option<DiskVerdict>`（`{ kind: 'usable' } | { kind: 'refused', reason }`），`None` 表示没探测（自动跟进、试重放没走到）。
- **偏离计划**：clippy 报 `replay_branch` 参数过多（8/7）。没用 `#[allow]`：把 `superseded` 与 `resolved` 合成 `Rules`，`probe_disk` 的输入合成 `BranchReplay`。
- **计划没写的坑**：试重放必须复用同一份 `memo`。用新的空 memo 会让冲突之前的 commit 重新 `commit-tree`，提交者时间不同、OID 不同，其他分支共享这些 commit 时就出现两份（R20）。
- **决定**：冲突框列出的 `files` 是试重放一路遇到的全部冲突文件（不只第一个 commit 的）；`at` 仍是第一个冲突的 commit。
- **决定**：点按钮时磁盘在冲突块里被改过（比如在对比工具里改），检查仍然通过就照新内容继续（设计 §4 第 3 步「改了也算数」）；`files` 之外的文件需要替换时不替换，回到冲突框。
- **为什么安全**：
  - 按钮路径与「跟进」同一套 `prepare → still_valid → publish`，只多了「冲突文件用指定 blob」；不写工作区（测试断言内容与 mtime 都没变）。
  - `still_valid` 在锁内重算替换文件的 blob，准备之后磁盘变了就重来一次，按新内容重判。
  - 其他分支 `plan_other` 传 `no_resolved`，测试 `another_branch_with_the_same_commit_is_not_moved_with_disk_content` 钉住它留在原地、进 `behind_branches`。
  - 自动跟进传 `DiskUse::Off`，行为不变。
  - `FollowReport.disk_resolved` 带 `serde(default)`，旧 journal 照常读。

### Task 4

- 无偏离。右边用 `workdir_side`，与「未暂存」对比同一套链接 / 越界检查。

### Task 6、7

- **偏离计划**：`describeFollowOutcome` / `describeFollowConflict` 不改，只给 `staleDisk`（磁盘上是旧内容）继续用：那不是重放冲突，没有逐文件的对比可看。重放冲突一律走新的 `FollowConflictDialog`。
- **决定**：冲突框是独立组件，不扩展 `ConfirmDialog`：要逐文件的对比按钮、退回预览与两个出路，塞进通用确认框会让它背上跟进专用的结构。
- **决定**：按钮执行期间冲突框显示进度、禁用全部按钮、Esc 不关；失败留在框里（与 `ConfirmDialog.action` 一致），错误用 `Reveal` 滚进视野（布局测试第一次跑时就是这条没过）。
- **决定**：能用磁盘内容时点「手动跟进」，冲突框先关、再弹现有的手动跟进确认框（带「会写工作区」警告），设计 §5.3。只剩手动跟进时冲突框本身带警告，点了直接跑，不再多一个框（保持事故分析 6.7 的「按钮就是出路」）。
- **测试取舍**：`svnSyncFlow.test.ts` 的公共 `deps` 把冲突框当成确认框（确认 = 手动跟进，取消 = Cancel），原有 6 个冲突用例的断言不变；磁盘按钮的 5 个用例自己接冲突框。
- **布局测试**：第一次全量跑 `confirm-long-warning @ 800x600 › 打开时有焦点的控件不被按钮条盖住` 失败一次，单独重跑通过。`ConfirmDialog` 本分支没改，判为偶发，记在这里不处理。

### Task 8：真实窗口 E2E（2026-11-26）

- 现场：scratchpad 里 `svnadmin create` + wc1 / wc2；wc1 经应用的 `svn_init_hybrid` 初始化；git 提交改第 2 行；同事在 wc2 改第 2、8 行（作者改成 colleague）；wc1 `svn update` 真的报了文本冲突；写入合并结果、`svn resolve --accept working`。
- `pnpm tauri dev --config`（独立 identifier），核对进程是本 worktree 的 `target\debug\app.exe`。
- 横幅：「没有自动跟进：当前分支的提交重放到新基线上，有 1 个文件冲突」——与事故现场相同。
- 点「跟进」：冲突框与设计 §5.1 一致（✓ 行、`用 … 对比`、三个按钮，默认焦点在磁盘按钮）。
- 「对比」：换成探针工具（.cmd 记录参数并复制左侧文件），左边是新基线里的 SVN 版本，右边是工作区真实文件路径。
- 点「用磁盘上的内容跟进」：报告「跟进完成」，含「按磁盘内容解决的冲突」一节；`work~1 == svn/base`；`git status -- Code` 为空；`HEAD:Code/A.cs` 等于磁盘；A.cs 修改时间跟进前后都是 `2026-11-25T16:05:57.2582201Z`。
- 验完删除了 `%APPDATA%` / `%LOCALAPPDATA%` 下的隔离配置目录。
- **计划没写的坑**：worktree 隔离的会话里，Bash 里任何「复杂」的 git 命令、带 `eval` 字样的命令都会被拦；PowerShell 里单条 `git -C <scratch路径>` 可以。CDP 驱动脚本改成从文件读 JS。

### 独立 review（2026-11-26，独立 review agent，按 AGENTS.md §13）

| 条目 | 结论 | 处理 |
|---|---|---|
| 高危 1：merge-tree 退出 1 却没有 stage 行（目录被拆到两处）时，`files` 为空，`open` 也为空，冲突树被当成「都已解决」的干净结果。跟进、自动跟进、交付都会静默发布 | 属实，测试复现（Done 而不是 Conflict） | `replay_onto` 改成 `files.is_empty() \|\| !open.is_empty()` 时照旧冲突；回归测试 `a_conflict_without_any_stage_entry_still_conflicts`；`probe_disk` 遇到没有文件条目的冲突直接判不能用 |
| 中 2：试重放走到第一次重放没走到的提交，那里的 `Err` 会让普通「跟进」变成 Refused、基线也不发布 | 属实 | `probe_disk` 出错时降级为 `probe_failed`：照旧列第一次重放的冲突文件，全部标「核对时出错」，`disk_ready = false` |
| 中 3：多轮冲突（`probe_disk` 循环）没有测试 | 属实 | 新增 `conflicts_in_two_commits_both_resolved_in_svn_follow_in_one_click`、`second_commit_not_usable_leaves_only_manual_follow`（真 SVN 现场，X 改 A、Y 改 B） |
| 低 4：后续轮次的文件「对比你的提交」右边取了第一个冲突的 commit | 属实 | `ConflictDetail.commit`（`serde(default)`）记每个文件自己的冲突 commit，前端 `range.to` 用它；说明里写「从 X 起 N 个提交」 |
| 低 5：Apply 不核对新基线是否就是冲突框里对比的那个 | 属实，与设计 §11 第 6 条不一致 | `svn_follow_with_disk` 加 `base` 参数，`new_base != base` 时回到冲突框；前后端同步改签名，真实窗口再验一次（见下） |
| 低 6：`disk_unchanged` 一直算不出 hash 时，文案说「仓库一直在变」 | 属实，方向安全 | 不改，记在这里：只在文件被独占锁住两次都算不出时出现 |
| nit：merge-ort 用 histogram、`merge-file` 默认 myers，冲突块可能划得不同 | 只多出假阴性，方向安全 | 不改：`merge-file --diff-algorithm` 要求的 git 版本未核实，跟进的最低版本是 2.43 |

- **计划没写的坑**：加了 `base` 核对之后，「不先弹冲突框、直接点按钮」的测试会被拦下（那时基线还没发布，`svn/base` 是旧的）。真实流程里冲突框一定在基线发布之后才出现，测试 `another_branch_with_the_same_commit_is_not_moved_with_disk_content` 改成先「跟进」再点按钮。
- **为什么安全**：前端在没有 `base` 时不给磁盘按钮；流程里「点磁盘」缺 `base` 时直接返回，不会落进会写磁盘的手动跟进分支。

### 第二轮真实窗口 E2E（签名加了 `base` 之后）

- 新 fixture、新的隔离 identifier，进程核对为本 worktree 的 `app.exe`。
- 反例：`invoke('svn_follow_with_disk', { path, files })` 不带 `base` → `missing required key base`，参数名确实生效。
- 正例：点「跟进」→ 冲突框 → 点「用磁盘上的内容跟进」→「跟进完成」；`work~1 == svn/base`；`status -- Code` 为空；`HEAD` 与磁盘无差异；A.cs 修改时间不变（`2026-11-25T20:54:24.0606507Z`）。

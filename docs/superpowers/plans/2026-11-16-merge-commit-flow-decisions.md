# 合并进行中的提交流程（第 74 条）——实现日志

没有对应的计划文件：issue 本身就是需求，按 `AGENTS.md` §2.1「预计 ≥3 个 Task」建本日志。

## 进度总账

| Task | 内容 | 状态 | 产出文件 | 验证 |
|---|---|---|---|---|
| 0 | fixture 里实测 git 真实行为 | 完成 | 本文件「计划没写的坑」 | scratchpad 脚本，见下 |
| 1 | Rust：提交前拦未解决冲突（中文报错）；cherry-pick / revert 收尾允许空提交 | 完成 | `src-tauri/src/engine/repo.rs`（`commit_extra_args`）、`src-tauri/src/vcs/git.rs`、`src-tauri/tests/commit_in_progress.rs` | 新测试先 6 红 2 绿（2 条是行为不变的护栏），实现后 8/8 绿 |
| 2 | 前端纯逻辑：提交闸门与横幅文案 | 完成 | `src/engine/conflicts.ts`（`commitGate`、`repoStateBanner`）、`tests/vitest/conflicts.test.ts` | 先 9 红，实现后全绿 |
| 3 | UI 接线：按钮禁用 / 改名、提示 | 完成 | `src/components/LocalChangesView.tsx`、`src/components/RepoTab.tsx`、`src/styles.css`、`tests/vitest/LocalChangesView.test.tsx` | 先 2 红，实现后全绿；`pnpm build`、`pnpm test`（1781）通过 |

## 决策条目

### Task 0：实测

#### 计划没写的坑

在临时 fixture（scratchpad，git 2.x，Windows）里实测，2026-11-16：

| 场景 | merge | cherry-pick | revert |
|---|---|---|---|
| 有未解决冲突时 `git commit -m x` | exit 128，英文 `error: Committing is not possible because you have unmerged files.` | 同左 | 同左 |
| 冲突解决成与 HEAD 相同、Staged 为空时 `git commit -m x` | **成功**，生成合并提交，`MERGE_HEAD` 消失 | **失败** exit 1：`nothing to commit` / `The previous cherry-pick is now empty` | **失败** exit 1：`nothing to commit` |
| 同上加 `--allow-empty` | —— | 成功，`CHERRY_PICK_HEAD` 消失 | 成功，`REVERT_HEAD` 消失（测试 `revert_resolved_to_head_commits_and_finishes_the_revert` 固化） |

结论：issue 说的「允许 Staged 为空时提交」对 merge 来说 git 本来就允许，缺的只是界面告诉用户；
对 cherry-pick / revert，git 必须带 `--allow-empty` 才能收尾。

### Task 1：后端

#### 偏离 issue 的提议

- issue 提议「有未解决冲突时禁用 Commit，**或**给中文提示」。两样都做了：前端禁用并说明（Task 3），
  后端在 `git commit` 前再拦一道、返回 `commit_unresolved_conflicts`。前端的冲突数来自上一次 status，
  外部工具刚改过 index 时可能是旧的，后端这道用的是提交那一刻的 index
- 「允许 Staged 为空时提交」没有加 Tauri 命令参数，而是由后端按 `repo.state()` 自己决定是否带
  `--allow-empty`。这样 `commit` 的签名不变（`AGENTS.md` §7.3 的 camelCase 风险为零），
  也不会出现「前端以为在 cherry-pick、后端其实已经不在」的错配

#### 为什么这么改是安全的

- `--allow-empty` **只**在 libgit2 报告 CherryPick / Revert 状态时追加。普通状态下什么都没暂存仍然报错，
  有护栏测试 `normal_commit_with_nothing_staged_still_fails`
- 冲突检查只读 index（libgit2），不写任何东西；拒绝时 HEAD 与 `MERGE_HEAD` 都不变，测试里断言了
- linked worktree：`Repository::open` 认 `.git` 文件，状态与 index 都取自该 worktree 自己的目录，
  测试 `merge_in_linked_worktree_is_guarded_and_finished_the_same_way` 覆盖，并断言主工作区不受影响
- SVN 混合仓库的 `commit` 委托给同一个 `GitBackend::commit`，自动得到同样的行为；
  它前面的 `assert_not_syncing` / `refuse_while_on_baseline` 顺序不变

### Task 2：前端纯逻辑

#### 偏离 issue 的提议

- 横幅写「没有未解决的冲突，提交以完成合并」，不是 issue 里的「冲突已全部解决」：
  `merge --no-commit` 停下时本来就没有冲突，已有测试明确要求不编造「已处理」
- rebase 不改按钮也不改横幅：rebase 的收尾是 `rebase --continue`，在 rebase 中间做一次普通提交
  并不会结束 rebase。这部分归第 51 条（Continue / Abort 入口）
- 冲突拦截不看 state：`stash pop` 撞出的冲突不留进行中的状态，git 照样拒绝提交

#### 计划没写的坑

- cherry-pick / revert 在 Staged 为空时提交会生成一个没有改动的提交。git 自己给的另一条出路是
  `cherry-pick --skip`，那属于第 51 条的入口；本分支只负责让「提交」这条路走得通，并在按钮下方说清会记一个空提交

### Task 3：真实窗口验证（2026-11-16）

`pnpm tauri dev --config`（隔离 identifier，核对过运行的是本 worktree 的 `target\debug\app.exe`），
scratchpad fixture：merge 停在 `a.txt`、`b.txt` 两个冲突上。经 CDP 驱动窗口，13 项全部通过：

- 有冲突时：横幅「merge in progress · 2 个文件有冲突」；按钮叫「提交合并」且置灰；下方红底中文原因；
  填好标题后 Ctrl+Enter 不生成提交（HEAD 不变）
- 在外部把两个文件解决成与 HEAD 相同并暂存（Staged 为空）：横幅变为「没有未解决的冲突，提交以完成合并」；
  蓝底说明出现；按钮可点。点击后生成双父合并提交、`MERGE_HEAD` 消失、横幅消失、按钮回到 Commit

review 修复后用新 fixture 重跑一遍，并加了 Amend 一项：合并中勾上 Amend 按钮置灰、显示
「merge 进行中不能 Amend，取消勾选后再提交」，取消勾选恢复。16 项全部通过。

cherry-pick / revert 的收尾、一次处理多个提交、split index 只在 Rust fixture 测试里覆盖，没有在真实窗口里走。

### 独立 review（`/code-review high`，2026-11-16）

| 发现 | 结论 | 处理 |
|---|---|---|
| 1 一次挑多个提交（`CherryPickSequence` / `RevertSequence`）时提交只了结当前一个，横幅却说「提交以完成」，`--allow-empty` 还会替序列里该 skip 的那个记空提交 | 成立 | `repo_state` 对序列报 `cherry-pick-sequence` / `revert-sequence`；横幅写「一次处理多个提交」、按钮不改名、不加 `--allow-empty`。测试 `sequence_cherry_pick_*` |
| 2 没有冲突、只是「挑过来是空的」而停下的 cherry-pick 也会被提示提交收尾，记下空提交 | 不改 | 按钮下方已写明「会记一个没有改动的 cherry-pick 提交」，是知情选择；`--skip` 归第 51 条 |
| 3 提交前用 libgit2 读 index，split index 等形状读不了时整个提交失败（原来能提交） | 成立，实测复现 | 读不了就跳过预检、交给 git。测试 `split_index_repo_still_commits`（先红后绿） |
| 4 本应用自己提交后 `repoState` 只靠监听事件刷新，按钮可能还叫「提交合并」 | 部分成立（真实窗口里是刷新了的） | 提交成功后不在 normal 时立即重读 `repo_state` |
| 5 操作进行中勾了 Amend，git 报英文 | 成立，且比 review 说的更严重 | 见下方「计划没写的坑」；前后端都拦 |
| 6 revert 测试只断言状态回到 normal，没断言生成了提交 | 成立 | 补 `HEAD~1 == 原 HEAD` 与提交标题 |
| 7 冲突计数重复实现了 `git_read::unmerged_paths` | 成立 | 改用它（按路径去重） |
| 8 `.changes-commit-blocked` 与 `.changes-baseline-warning` 逐条重复 | 成立 | 合并为一条规则 |
| 9 `finishLabel` 靠返回值带前导空格区分中英文 | 成立 | 改成名字表 + `spaced()` |
| 10 中文拦截文案前后端各写一份 | 不改 | 两层各管一种情形（前端事先说明；后端兜住 status 过期），本仓库没有前后端共享文案的机制 |

#### 计划没写的坑（review 后实测）

操作进行中 `git commit --amend` 的真实行为（scratchpad fixture，2026-11-16）：

| 进行中 | 结果 |
|---|---|
| merge | exit 128 `You are in the middle of a merge -- cannot amend.` |
| cherry-pick | exit 128 `You are in the middle of a cherry-pick -- cannot amend.` |
| **revert** | **exit 0**：上一个提交被改写成 revert 的结果，`REVERT_HEAD` 消失 |

所以 revert 中的 Amend 不只是英文报错，而是改错了提交。后端 `commit_extra_args(path, amend)` 在
merge / cherry-pick / revert（含序列）进行中拒绝 amend（`commit_amend_in_progress`），前端 `commitGate` 同样拦并说明。
rebase 不拦：停在 edit 上 amend 是它的正常用法，保持原行为。测试 `amend_during_revert_is_refused_*`（先红后绿）、
`amend_during_merge_is_refused_in_chinese`、`amend_outside_any_operation_still_works`。

### 遗留

- 提交说明不会自动填入 git 准备好的 `MERGE_MSG`，用户仍要自己写标题
- split index 等 libgit2 读不了 index 的仓库里，未解决冲突时仍是 git 的英文报错（预检跳过，交给 git）
- 一次处理多个提交的收尾（`--continue` / `--skip`）、结果为空的单个 cherry-pick 的 `--skip`，归第 51 条

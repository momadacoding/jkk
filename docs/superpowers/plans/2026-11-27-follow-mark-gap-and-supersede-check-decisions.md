# 实现日志：补上 F2 ① 的快照空窗，换成 SVN 版本前先核对

计划：`2026-11-27-follow-mark-gap-and-supersede-check.md`。

## 进度总账

| Task | 状态 | 产出文件 | 验证 |
|---|---|---|---|
| 1 收尾写只含 commit 的快照 | 完成 | `svn_follow_mark.rs`、`svn_follow.rs`（`finish`）、`svn_sync_in.rs`（`finish_forward`）、`tests/svn_follow_hooks.rs`、`tests/svn_follow.rs` | 新用例先红（NeedsManual，拒绝理由与现场截图相同）后绿；改过断言的 3 个用例与 `resume_after_crash_between_each_step` 等 13 个崩溃用例通过 |
| 2 换成 SVN 版本前核对 | 完成 | `svn_follow_disk.rs`（`supersede_safe` 带 `blocked`、`supersede_ok` 只走 `git cat-file`、`touched_paths`、与冲突框共用的 `remerge`）、`svn_follow.rs`（`supersede_plan` 沿快照之前的 commit 链从旧到新预先算好，`superseded` 只查表；`FollowReport.not_rewritten`）、`svn_replay.rs`（开放 `entries_at`）、`src/engine/svnFollow.ts`（报告新节）、`tests/svn_follow_hooks.rs`、`tests/vitest/svnFollow.test.ts`、`tests/svn_backend.rs` | 两个「部分提交」用例先红后绿；现场形状放行；W14 用例记下新行为；两轮 review 后补的回归用例逐个确认去掉修复时失败 |
| 3 现有测试与文档、验证、PR | 完成（待合并） | 本日志、`src/engine/svnFollow.ts`、`tests/vitest/svnFollow.test.ts`、`tests/svn_backend.rs` | review 修复后重跑：`cargo fmt --check`、`cargo clippy --all-targets` 无告警；`cargo test` 分批跑完 116 个集成测试二进制 + 库内 188 个单测，全部通过；`pnpm build` 通过；`pnpm test` 106 个文件 1968 个用例通过 |

## Task 1

### 偏离计划

- **收尾快照只在钩子健康时用**（`FollowMark::after_follow` + `Who::gather` 过滤）。计划没有这个限制，原因见
  「计划没写的坑」最后一条。
- 新增 `renew_mark_after_follow`：写失败时退回原来的 `clear_mark`，收尾不因快照失败。计划没写。
- `finish_forward` 里仍放在原来 `clear_mark` 的位置（放回收起的改动之前）：放回只动磁盘与索引，不动
  HEAD；新快照不记暂存，放在前后结果相同。放回停在冲突时（StashConflict），HEAD 早已落到基线上，
  快照说的也是真话。

### 计划的错误

无。

### 计划没写的坑

- 现有 3 处断言「收尾清掉快照」（`follow_drops_intermediate_commits_of_own_svn_commit`、崩溃用例共用的
  `assert_finished`、手动跟进的 `manual_follow_..._keeps_it_on_abort`），这次是有意改掉的行为，改成断言
  「快照的 HEAD = 落下之后的 HEAD、基线 = 新基线、暂存为空」。手动跟进那个用例按 AGENTS §8 改名为
  `manual_follow_renews_the_mark_...`，名字与断言一致。
- 测试里钩子事件的时刻要用真实时钟（`now_ms() + 1000`）：收尾快照的时刻是真实的，旧用例用 `mark_at`
  把两者放在同一条假时间轴上，这里不能这么做。
- **没有钩子时收尾快照会判错先后（全量测试暴露，`a_replay_conflict_on_one_file_leaves_only_that_file_behind`）。**
  没有钩子时，「快照之后的 SVN 提交」= 快照的 `base_oid` 之后才进基线的路径。失焦快照只在判定一致时写，
  那时基线含工作副本里的全部 SVN 提交，这个等式成立；收尾快照不满足：部分自动跟进把冲突文件留在旧
  基线上（held back），那次早于快照的 TortoiseSVN 提交不在 `base_oid` 里，下一次跟进会把它当成快照之后的
  提交。如果用户在那次提交之后又在 git 里提交了新工作，F2 ① 会把新工作换成 SVN 版本。整行冲突时
  Task 2 的核对也拦不住（冲突块内部按「SVN 是最终版」放行）。
  修法：`FollowMark` 加 `after_follow`（旧格式默认 false），`Who::gather` 在没有钩子事件时丢掉收尾快照，
  等于没有快照，与改动前相同。有钩子时按提交开始时刻比，held back 的那次提交早于快照，不算快照之后。
  两种模式各加一个用例（`svn_follow_auto.rs`、`svn_follow_hooks.rs` 的 `..._held_back_...`），没有钩子的那个
  在去掉修复时确认失败过（报 Done，a4 被换掉）。
  **代价：** 没有钩子的用户得不到这次的修复，空窗照旧。现场的仓库钩子健康，不受影响。

### 为什么这么改是安全的

- **commit 的先后。** 快照的 HEAD 是跟进刚落下的 tip，其中的 commit 都是跟进开始前就有的 commit 重写的
  （跟进对 work 做 CAS，期间 work 被移动就不发布）。所以它们早于收尾之后才开始的任何 TortoiseSVN 提交。
- **跟进期间就开始的提交。** 有钩子时按 `mark_ms < t_commit` 判断，这类提交开始得早于快照，判为「不是快照
  之后」，F2 ① 不换，照常重放，方向保守。没有钩子时收尾快照不用（见上面「计划没写的坑」最后一条）。
- **暂存为什么不记。** 没有钩子时，跟进期间别处暂存的内容可能晚于同一段时间的 TortoiseSVN 提交，按基线
  判断分不出来。记空后 `staged_in_mark` 恒为 `Some(false)`，暂存一律走三方合并，与没有快照时相同，
  不会比改动前更激进。
- **与「判定一致才写」的规则。** `svnSync.ts` 的 `shouldMarkOnBlur` 防的是「焦点刚回来、结论还没出时写下
  的快照可能晚于外部提交」。收尾快照的 HEAD 是 jkk 自己刚写的，不依赖判定结论，那条理由不适用。失焦
  快照的规则不变，判定一致时照旧用完整快照替换它。

## Task 2

### 偏离计划

- **按「提交 × 文件」逐个核对，不按报告第 6.2 节写的「多个提交合成一个算」。** 合成一个算时，合并基础取
  第一个改过它的提交的父提交，中间夹着 Local-Only 提交就把本地补丁算进「你的改动」，只能整个不放行
  （报告第 6.4 节的守卫）。逐个核对时每个提交以自己的父提交为基础，Local-Only 提交本来就不参与 F2 ①，
  照常重放。
- **（独立 review 第 1 条后改）逐个核对，但按从旧到新的顺序传递「没通过」。** 第一版每个「提交 × 文件」各判
  各的，我在这里写了「C1 加了 B、C2 又删掉 B 时 C1 不通过，是保守的假阴性」，**这是错的**：C1 照常重放
  把 B 加回来，C2 单独看等于 SVN 版本、被换掉，它删 B 的改动也跟着没了，分支上又出现用户删掉的 B。
  改成 `svn_follow::supersede_plan` 预先沿快照之前的 commit 链从旧到新算：一条路径在某个 commit 上没通过，
  之后的 commit 在这条路径上一律不换（`supersede_safe` 的 `blocked`）。Local-Only commit 不换，也不挡后面的。
  `superseded` 闭包只查这张表，原来的 `RefCell` 缓存去掉。
- **「停下来问」落到现有出口，不新加界面。** 不通过就不换、照常重放：合并干净就保留改动（部分提交那篇
  希望的结果），重放结果与磁盘不同时由条件 11 拦住自动跟进、跟进报告列出对不上的文件；冲突就进冲突框，
  冲突框里有 Beyond Compare 对比。
- **（独立 review 第 4 条后加）跟进报告新增一节「没换成 SVN 版本的路径」。** 后端 `FollowReport.not_rewritten`
  （JSON `notRewritten`，旧 journal 没有时为空），前端 `describeFollowReport` 的 `notRewritten` 节，级别
  attention。用户看到 drift 或「本地改动」里多出东西时知道原因。只列当前分支的 commit。

### 计划的错误

无。

### 计划没写的坑

- **W14（挪位置）的行为变了。** commit 在第 2 行后加 `foo`，TortoiseSVN 提交前挪到第 6 行后：三方合并干净，
  结果里 `foo` 有两份，不等于 SVN 版本。仅凭内容，它与「只提交了一部分」分不开（没提交的那部分同样表现为
  「合并结果多出几行」）。按报告第 6.2 节「不一致就停下来问」，现在不换。结果退回 F2 ① 出现之前的处理：
  按钮跟进完成，报告里 `drift` 列出 A.cs，`overlap.matches_disk = false`，自动跟进被条件 11 拦下，「本地改动」
  里出现一条反向的 `-foo`。用例 `a_line_moved_before_the_tortoise_commit_is_no_longer_superseded` 记下这个行为。
  考虑过「多出来的行都在 SVN 版本别处出现过就当成挪动」的启发式，没做：没提交的那部分如果恰好是
  `}`、`return;` 这类常见行，会被当成挪动放行，又是静默丢改动。
- 合并基础缺失（本地新建的文件）时用空 blob（`hash-object -w --stdin`）当基础：`merge-file --object-id` 要求
  对象存在。
- （独立 review 第 3 条后改）`git_read::tree_entries` 遇到 replace ref、ls-tree 另有解释的路径写法等会报
  unsupported。第一版不兜底，结果是一个这样的仓库里 F2 ① 整个失效。改成复用 `svn_replay` 的 ls-tree 兜底
  （新开放的 `svn_replay::entries_at`，`ls_entries` 也改走它）。单个文件核对出错只拒绝这一个文件，不连累别的。
- （独立 review 第 2、5 条后改）判二进制的顺序与范围：第一版先看大小（> 8 MB 不换）再看 NUL，大的贴图、
  模型因此不再换成 SVN 版本；而且任何含 NUL 的都当二进制放行，UTF-16 文本只提交了一部分时照样丢改动。
  改成：先看 UTF-16 / UTF-32 BOM（核对不了，不换），再按 git 的规则看前 8000 字节有没有 NUL（二进制，
  放行），最后才是 8 MB 上限（只管文本）。没有 BOM 的 UTF-16 仍会被当成二进制放行，这是已知的剩余风险。
- （第二轮独立 review 第 1、2 条后改）第一轮修复时加过 `git_read::blob_head`（libgit2 读开头字节），已删掉：
  它没有 `sha1_repo` / `no_replace` 防护、没有 git 命令兜底，SHA-256 仓库里读不了，有 blob 的 replace ref
  时读到的和 `merge-file` 读到的不是同一份；而且会把整个大 blob 读进内存。现在 `supersede_ok` 全走
  `git cat-file`：先 `--batch-check` 取三份的大小；有一份超过 8 MB 时只读 SVN 那一份判二进制（是二进制就
  放行，否则不换），提交前后两份不读；都不超过时三份都读，按 BOM → 二进制 → 逐行核对的顺序判断。
- 测试夹具的白名单只收 `.cs`：用例里的二进制、UTF-16 文件也得叫 `.cs`，否则跟进看不到它们。

### 为什么这么改是安全的

- **不通过时只会少换，不会多换。** 核对的输出是 `paths` 的子集，替换照旧走 `svn_replay` 的 `swaps_for` /
  `c_prime`。核对出错、父提交不止一个，一律返回空集，等于没有 F2 ①，也就是快照缺失时本来的行为。
- **直接放行的几种都是整文件操作。** 删除、新建后删除、二进制、符号链接 / 子模块，TortoiseSVN 都不能只提交
  一部分，换成 SVN 版本不会丢掉「没提交的那部分」。超过 8 MB 的文本反过来按不放行处理，不读进内存。
- **放行的冲突块内部仍有风险。** 冲突块里混着没提交的改动时分不出来，按「SVN 版本是最终版」放行，这是
  F2 ① 原本的前提（W10-R2 中间版本），与冲突框磁盘核对的已知限制相同。这次只是把冲突块以外的部分管住了。
- **与冲突框核对共用一份合并。** `judge`（冲突框）与 `supersede_ok`（F2 ①）都走 `remerge`：同一套两段式标记、
  同一种带 OID 的标签、同一个 `outside_hunks_match`。冲突框的 19 个用例与 9 个 resolve-with-disk 用例照旧通过。

## 独立 review（`/code-review high`，2026-11-27）

6 条，全部核对属实并修了。修法写在上面各节，这里只列对应关系：

| 条 | 级别 | 问题 | 修法 | 回归用例（去掉修复时确认失败） |
|---|---|---|---|---|
| 1 | important | 前面的 commit 没通过、后面的被换掉，删掉的内容回来 | `supersede_plan` 沿链传递 `blocked` | `a_later_commit_is_not_superseded_on_a_path_an_earlier_commit_kept` |
| 2 | important | > 8 MB 的二进制不再换 | 先判二进制再看大小 | `a_large_binary_committed_whole_in_tortoise_is_still_superseded` |
| 3 | important | 进程内读不了时 F2 ① 整个失效 | 复用 ls-tree 兜底；单文件出错不连累别的 | `the_check_falls_back_to_git_when_in_process_reads_are_refused` |
| 4 | important | W14 只在 drift 里体现，看不出原因 | 报告新增「没换成 SVN 版本的路径」 | W14 与部分提交用例加 `not_rewritten` 断言；vitest 两条 |
| 5 | nit | UTF-16 文本当二进制放行 | 带 BOM 的不换 | `a_utf16_text_file_is_not_superseded_without_a_line_check` |
| 6 | nit | 收尾快照没写成时旧快照留下 | `write_commit_mark` 返回是否写了，没写就删 | 无。不写的两种条件：`<repo>/.git` 不是目录（链接 worktree，这时快照路径本身就在一个文件下面，旧快照也不可能存在），或 HEAD 解析不出来（跟进收尾时 HEAD 刚被跟进写过）。都造不出「旧快照在、新快照没写」，改动只是让代码不依赖这个推理 |

第 4 条里 W14 本身的取舍（不换）没有改，只是让用户看得见。

## 第二轮独立 review（`/code-review high 54f2c28..HEAD`，2026-11-27）

只看第一轮修复的 diff。7 条，6 条修了，1 条（pre_existing）核对后不修：

| 条 | 级别 | 问题 | 处理 |
|---|---|---|---|
| 1 | important | `blob_head` 只走 libgit2：SHA-256 仓库里 F2 ① 全部失效，blob 有 replace ref 时判断与合并读到的内容不同 | 删掉 `blob_head`，`supersede_ok` 全走 `git cat-file`。没有专门的回归用例：fixture 造不出 SHA-256 的混合仓库；blob 的 replace ref 只能证明「两边读到的一致」，现在两边都是 git 命令，没有第二条读取路径可比 |
| 2 | nit | 大 blob 整个读进内存 | 先 `--batch-check` 取大小，超过 8 MB 只读 SVN 那一份。`a_large_binary_committed_whole_in_tortoise_is_still_superseded` 照旧通过 |
| 3 | nit | 核对出错的 commit 不进报告 | 出错时用 `touched_paths`（libgit2 读不了退回 `git diff-tree`）把它改过的路径记进 `refused`。没有回归用例：要让 libgit2 与 ls-tree 都读不了树，fixture 里造不出来 |
| 4 | nit | 被前面挡住的 commit，报告说「有改动不在 SVN 里」，不对 | 报告文案改成三种原因都覆盖（改动不在 SVN 里 / 跟着更早的 commit 照常重放 / 核对出错）；vitest 断言三种都在 |
| 5 | nit | 6.10 快照规则换掉的路径同时出现在 `snapshot` 和 `notRewritten` | 建 `not_rewritten` 时减掉 `snapshot.paths(&scope, c)`。没有回归用例：要造「C^ 是纯 SVN、C 等于某个中间基线、新基线又部分回退」的三段基线，代价大，改动是一行集合差 |
| 6 | pre_existing | 后面的 commit 修改 Local-Only 补丁行时，修改被换掉 | **不修**。先照建议做了「Local-Only commit 改过的路径，之后不换」，全量测试里 `svn_restore_local_only.rs` 9 个用例失败：2026-11-11 现场是用户在普通 commit 里**删掉**补丁行、再用 TortoiseSVN 提交，原设计要把那个删除换掉，让补丁重放回来、再由 9.1 放回磁盘。按内容分不开「删补丁」与「改补丁」，撤回，保留原设计，在 `supersede_plan` 注释里写明这个代价 |
| 7 | nit | 进度总账第 2 行过时 | 已更新 |

## 遗留

- 后面的 commit 修改 Local-Only 补丁行、TortoiseSVN 只提交了别处时，那处修改会被换掉（第二轮 review 第 6 条，
  改动之前就有）。要分开「删补丁」与「改补丁」需要另外的信号（比如 TortoiseSVN 提交框里看过的警告），这次不做。
- 报告第 7 节的未决问题：F2 ① 拿不准时在冲突框里给「换成 SVN 版本」按钮；横幅提示「磁盘上已经解决」。
  都没做。
- ③ 冲突框支持多个提交改过同一文件：按报告延后。

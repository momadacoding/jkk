# TortoiseSVN 提交时已确认的泄漏不再拦自动跟进：实现日志

设计：[`../specs/2026-11-16-tsvn-acknowledged-leaks.md`](../specs/2026-11-16-tsvn-acknowledged-leaks.md)（方案 A）。
没有单独的计划文件，设计 §9 的五步即 Task 划分。

## 进度总账

| Task | 状态 | 产出 | 验证 |
|---|---|---|---|
| T1 `TsvnEvent.warned`、`WarnedHunk`；Check-commit 写事件 | 完成 | `tsvn_inbox::{TsvnEvent::warned, WarnedHunk}`；`tsvn_hook_cli::{check_commit, repo_paths}` | `tests/tsvn_hook_cli.rs` 改 4 条、新增 3 条，30/30 |
| T2 收件箱 C↔P 配对、已确认集合、清理 | 完成 | `tsvn_inbox::{HookEvidence::acknowledged, latest_before, check_of, is_lead_up}`；`read_evidence`、`prune_inbox` | `tests/tsvn_inbox.rs` 新增 11 条，25/25 |
| T3 跟进拆分 `leaks` / `acknowledgedLeaks`；journal 兼容 | 完成 | `FollowReport::acknowledged_leaks`、`FollowOutcome::NeedsManual::acknowledged_leaks`、`ManualNeeded`、`Who::acknowledged`、`split_acknowledged`、`OthersCtx::acknowledged` | `tests/svn_follow_hooks.rs` 新增 11 条（含 review 后补的 1 条），24/24；`svn_backend.rs` 序列化与旧 journal；`svn_follow.rs` 的 `NeedsManual` 解构补字段 |
| T4 前端报告分节 | 完成 | `svnFollow.ts` 的 `acknowledgedLeaks` 类型、`acknowledgedLeaks` 节、冲突框的 `ackedLeakSection` | `svnFollow.test.ts` 新增 5 条；`FollowReportDialog.test.tsx` 新增 1 条 |
| T5 E2E 5d、独立 review、PR | 进行中：review 完成；E2E 5d 未跑 | E2E 5d 写进 `e2e/2026-11-02-svn-c1h-tsvn-hooks-e2e.md`；review 修复见 T5 节 | review 后 `svn_follow_hooks` 24/24、`tsvn_hook_cli` 30/30，fmt / clippy 干净 |

T1–T4 的整体验证（2026-11-20，本 worktree）：

- `cargo fmt --check` 通过；`cargo clippy --all-targets` 无告警。
- `cargo test` 分三批（全量一次跑会被系统以内存不足停下）：`--lib` 177、`svn_backend` 35、`svn_follow_hooks` 23、`svn_leak` 51、
  `tsvn_hook_cli` 30、`tsvn_hooks` 28、`tsvn_inbox` 25；`svn_follow` 137、`svn_follow_auto` 38；`svn_follow_snapshot` 12、
  `svn_sync_in` 77、`svn_follow_replay_budget` 1、`svn_follow_scan_budget` 1。全部通过。其余测试二进制只确认了能编译（`cargo build --tests`），没有跑。
- `pnpm build` 通过；`pnpm test` 104 个文件 1856 条全过。
- 没跑 `pnpm test:layout`：没改弹框组件和样式，只改了 `src/engine/svnFollow.ts` 给出的节。

## 决策条目

### 开工前定下的（设计 §8 的未决问题）

- **§8 第 2 条 分节 level**：用 `info`。用户在提交框里已经确认过，再标成要处理的一项等于第三次问；分节保留作留档。
- **§8 第 3 条 窗口**：沿用 `PAIR_WINDOW_MS`（10 分钟），没有实测数据前不另起常量。
- **§8 第 4 条**：留到 T5 的 E2E 5d 实测。实测之前，配对规则按设计要求路径集合完全相同；万一 Proceed anyway 之后 PATH
  不同，结果是配不上、照旧拦（安全方向），不会错误放行。

### T1 Check-commit 写事件

- 事件的 `paths` 用与 Pre / Post 同一个转换（抽成 `repo_paths`）。全在仓库外时不写事件，判定照旧返回，与 Pre / Post「空事件不写」一致。
- `warned` 取 `local_only_leaks_on_disk` 的每个 hunk，Present 和 Unknown 都算：弹框里两种都列了，用户看过的就是这些。
- 写事件放在判定之后、返回之前，失败只记日志。用测试钉住：收件箱的位置被文件占住时，有泄漏照样退出 1，没有泄漏照样退出 0。
- 原来的 `check_commit_writes_no_event_and_exits_zero` 实际测的是「目录不是 git 仓库 → 算不出 → 放行」，改名为
  `check_commit_in_a_directory_that_is_not_a_repo_writes_no_event_and_exits_zero`，断言不变（仍然不写事件）。

### T2 配对与清理

- **计划没写的坑**：原来的 `prune_inbox` 对「不是 Pre」的事件一律按 `processed` 删，跟进传的 `processed` 是「早于读收件箱的时刻」。
  Check-commit 事件如果走这条，用户盯着警告框时恰好跑了一次跟进（自动跟进由别人的提交触发），C 会被清掉，之后点 Proceed
  anyway 写下的 P / Q 就配不上了（结果是照拦，方向安全，但功能失效）。所以 Check-commit 和 Pre 一样归为「只靠配对才有意义的事件」
  （`is_lead_up`）：不按 `processed` 直接删，跟着配上的已处理 Post 一起删，配不上的按 `ORPHAN_PRE_TTL_MS` 删。设计 §5.3 的
  「孤儿 C 按 24 小时删」就是这个意思，这里把它落实成不被 `processed` 提前删。
- 孤儿的定义：没有被「某条成功 Post 配上的 Pre」配上。失败的 Post（有 error）配不上任何 Pre，它的 P 和 C 都按孤儿 TTL 删，
  与原来 Pre 的处理一致。
- `pair_of` 抽成 `latest_before(events, i, kind)`，P↔Q 与 C↔P 共用同一套条件（路径集合相同、不晚于、窗口内、取最晚），
  原来 P↔Q 的行为没变（原有用例全过）。
- 没有 `warned` 字段的 `check_commit`（手写、或以后格式变了）照样读得懂，但不算确认。

### T3 跟进拆分

- **偏离设计**：设计 §5.3 写「其他分支：`plan_others` 本期不拆分」。核对代码发现 `plan_other` 把其他分支的泄漏也并进
  `report.leaks`，而 `auto_blocker` 条件 8 看的是整个 `report.leaks`。不拆的话，只要另有一条分支带着同一个 Local-Only commit
  （从工作分支切出来的分支都带着它），自动跟进照样被挡，目标落空。所以 `plan_other` 也拆，但**拆完照旧不移动那条分支**（仍进
  `behindBranches`，原因文案不变），设计 §2.2「本期不处理其他分支的落后基线」照样成立。拆的判据与当前分支相同，含 commit OID：
  OID 相同才算同一个已确认的 commit，各自重写过的照旧拦（与 §8 第 1 条的倾向一致）。测试
  `a_sibling_branch_with_the_same_commit_stays_behind_without_stopping_auto_follow`。
- 已确认集合只在钩子健康时才有（`Who::events` 只在 `HookHealth::Healthy` 时读）。钩子失效时即使收件箱里有完整的 C→P→Q 也不用：
  测试 `acknowledgement_is_not_used_when_hooks_are_not_healthy`。
- **计划没写的坑**：部分自动跟进（`held_candidates` 留下文件之后的第二遍）的报告是排除留下的文件之后算的，留下的那个文件的泄漏
  不在 `report.leaks` 里，只出现在 `heldBack` 的原因里。这是原有行为，与本改动无关；测试 `only_the_unacknowledged_file_stays_behind`
  按这个形状断言（初稿断言 `leaks == [B]`，跑出来是空，改了断言，不是改了代码）。
- 冲突出口（`ManualNeeded`，含 `stale_disk`）同样拆，`NeedsManual` 新增 `acknowledgedLeaks`。`held_candidates` 对冲突出口只看
  `m.leaks`，已确认的不再以「泄漏」为由留下（冲突文件照样以冲突为由留下）。
- 同一文件里只有部分 hunk 被警告过时，按 hunk 拆成两条 `Leak`（同 branch / commit / path），一条进 `leaks`、一条进 `acknowledgedLeaks`。
- **为什么这么改是安全的**：放行的唯一路径是 `report.leaks` 里的 hunk 被挪到 `acknowledgedLeaks`，条件是 `(commit, path, oldStart,
  lines)` 与一条 Check-commit 的 `warned` 完全相同，且那条 C 经 P 配上了一条成功的 Post，且钩子健康。设计 §5.2 表里每个「拦」的场景
  都有测试：只有 C（命令行提交）、只有 P→Q（关掉拦截 / Check-commit 算不出）、commit 重写过、同文件另有没警告过的 hunk、钩子失效。
  跟进的写入路径没有改动，只改了「拦不拦」这个判定。

### T4 前端

- 新节 `acknowledgedLeaks` 放在 info 节的最前（文件清单之前），文件清单可能上千行。已丢弃的提交照「泄漏告警」一节的规则单列
  「已从分支移除」一组（`isRemoved`：提交在 `dropped` 里、且分支不在 `behindBranches` 里）。
- 冲突框（`describeFollowOutcome` 的 `needsManual`）里加一段「提交时已确认的 Local-Only 改动：…」，只有未确认泄漏时才出原来那段。
- 旧后端没有字段时按空处理，不出这一节。

### T5 独立 review（opus 子代理，2026-11-20）

0 阻断、2 建议、3 nit。处理：

1. **建议，已修**：`proceed_anyway_leak_does_not_stop_auto_follow` 断言收件箱清空，但事件时刻是 1970 年，按 14 天 TTL 本来就全删，
   测不到 C / P / Q 连带删除（虚假信心）。改用真实时刻，另放一条将来时刻的事件，断言只剩它一条。
2. **建议，不改代码、记入设计 §6**：Abort 之后 10 分钟内再提交，这次 Check-commit 算不出或写不进事件时，P 会配上前一次的 C。
   reviewer 提的收紧办法（C 与 P 之间不能有别的 P）挡不住这个场景（Abort 不写 P），所以没采用。用户看过的就是同一批 hunk，接受。
   设计 §5.2 表第 6 行补上前提「窗口内没有更早的 C」。
3. **nit，已补**：跟进层补了「Abort → 删补丁 → 干净的第二次检查」的用例 `a_clean_recheck_after_an_abort_is_the_one_that_counts`
   （手写干净的 C2，补丁仍上了服务器，照旧拦）。
4. **nit，不改**：整文件判定的哨兵 `(0, 0)` 与「文件开头纯插入」的 hunk 同形。只有磁盘与 NEW 对二进制的判断不一致、且补丁另从别处上了
   服务器时才会误认，本期不改；以后要收紧，就在 `WarnedHunk` 里区分整文件。
5. **nit，已补**：两条「写事件失败」的用例加断言：收件箱位置仍是那个文件，确实没写进去。

# 提交图显示作者时间：实现日志

来源：2026-11-26 的使用报告（对话中提供，未单独落库）。报告的方案 A：提交图列表、提交详情、
复制提交信息显示作者时间（author date），排序仍用提交时间（committer date）；两者不同时，
详情里多一行提交时间。报告第 6 节的 SVN 空时间问题另开 PR，不在这里。

## 进度总账

| Task | 状态 | 产出文件 | 验证 |
|---|---|---|---|
| 1. Rust：`CommitNode` 加 `author_time` | 完成 | `src-tauri/src/engine/graph.rs`、`src-tauri/tests/fixtures.rs` | 新用例先失败（`left: None`）后通过；fmt、clippy 无告警 |
| 2. 前端：取显示时间的纯函数 | 完成 | `src/engine/commitTime.ts`、`src/engine/types.ts`、`tests/vitest/commitTime.test.ts` | vitest 通过 |
| 3. 显示：列表、详情、复制提交信息 | 完成 | `CommitRow.tsx`、`CommitDetailView.tsx`、`commitActions.ts`、`styles.css` 及三个组件/模块的测试 | 三条新用例先失败后通过；`pnpm build`、`pnpm test`（1976 条）通过 |
| 4. 真实窗口验证 | 完成 | — | `pnpm tauri dev`（隔离 identifier，核对 `app.exe` 在本 worktree 的 `target\debug` 下），fixture 见下 |
| 5. 独立 review 的小问题 | 完成 | `commitTime.ts`（`rewrittenTitle`）、`CommitRow.tsx`、`CommitDetailView.tsx` 及测试 | `pnpm build`、`pnpm test`（1979 条）通过 |

## 决策条目

### Task 1

- **偏离报告**：报告写「新增作者时间字段，取 `commit.author().when().seconds()`」。实际做成
  `Option<i64>`，**只在和提交时间不同时才带**（`skip_serializing_if = "Option::is_none"`）。
  原因：绝大多数提交两个时间相同，5000 条提交每条多一个恒等字段是白给的传输量（同 `is_false`
  的取舍）。前端本来就要处理「没有作者时间」（stash、SVN 节点），缺省退回 `time`，语义一致。
- **为什么安全**：`time` 的取值和含义不变，排序（`svnGraph.ts` 的 `dateOrder`、`tagOrder.ts`）
  和图布局都只读 `time`，不受影响。
- **测试用真实的 cherry-pick 造重放提交**，不直接设两个不同的环境变量：要钉住的是「git 重放后
  作者时间保留、提交时间变成当时」这件事本身。实测 cherry-pick 后 `time` = 设的
  `GIT_COMMITTER_DATE`（01-09），`author().when()` = 原提交的 01-02。

### Task 2–3

- **偏离报告**：报告写详情里多一行「最后重放：…（跟进、rebase 会改这个时间）」。实际文字是
  「提交时间：…」，解释放进悬停提示。原因：amend、cherry-pick、`git am`、别人代提交也会让两个时间
  不同，「最后重放」不准；悬停提示里列全了会改它的操作。
- **计划没写的坑**：信息条 `.commit-head` 是单行 flex，自带下边框。多出的那一行放在它后面
  （`CommitHeader` 改成返回 fragment，父容器 `.commit-detail` 是纵向 flex），并用
  `.commit-head:has(+ .commit-head-rewritten)` 把分隔线挪到第二行下面。
- **不改的地方**：SVN 节点详情（`CommitDetailView` 的 svn 分支）和 stash 详情继续显示 `time`：
  它们没有 `authorTime`，`shownTime` 也会退回 `time`，结果相同；svn 那处保持原样以免把
  报告第 6 节的改动混进来。文件历史和 blame 本来就用作者时间（`file_history.rs` 的 `%at`、
  `author-time`），不用动。

### Task 4：真实窗口

fixture：scratch 目录里的一次性仓库。`base`（11-01）→ `主线 11-25`；`side` 分支上 11-09 写的
「重新登录机制」，用 `GIT_COMMITTER_DATE=2026-11-26T11:25:03+08:00 git cherry-pick side` 重放到 master。

| 看什么 | 结果 |
|---|---|
| 列表顺序 | 重放的提交 → 主线 11-25 → side 上的原提交 → base，和改之前按提交时间排的一样 |
| 列表日期 | 重放的提交显示 11/9 10:54:04；悬停提示写着提交时间 11/26 11:25:03 |
| 详情 | 信息条显示 11/9，下面一行「提交时间：11/26/2026, 11:25:03 AM」；分隔线只在第二行下面（信息条 `border-bottom` 0px） |
| 对齐 | 第二行左边 179 + 34 = 213，和作者名的 x（213.125）一致 |
| 没重放过的提交 | 没有第二行，信息条分隔线照常 |

### Task 5：独立 review（`/code-review medium`）

没有 bug，有 4 条 nit，全部处理：

1. svn 节点详情直接读 `time` → 改成 `shownTime`，结果不变，以后统一一个入口。
2. 信息条日期的悬停提示恒写「作者时间」，stash 详情的时间其实是提交时间 → 提示只在重放过时出现
   （`rewrittenTitle`）。
3. 列表按提交时间排、显示作者时间，日期列可能「不按顺序」却没有解释 → 日期加同一条悬停提示。
4. 真实窗口还没验 → 见 Task 4。

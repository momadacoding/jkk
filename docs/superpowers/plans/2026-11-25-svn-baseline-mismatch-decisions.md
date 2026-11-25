# 体检基线按症状提示（B + C）实现日志

计划：`2026-11-25-svn-baseline-mismatch.md`

## 进度总账

| Task | 状态 | 产出 | 验证 |
|---|---|---|---|
| 1 后端判定 | 完成 | `src-tauri/src/engine/svn_mismatch.rs`、`commands/svn.rs` 的 `svn_baseline_mismatch`、`tests/svn_baseline_mismatch.rs` | `cargo test --test svn_baseline_mismatch` 9 passed |
| 2 前端纯逻辑 | 完成 | `src/engine/svnMismatch.ts`、`tests/vitest/svnMismatch.test.ts`；删 `needsStandaloneCheck` 及其用例 | vitest 19 passed |
| 3 接线 | 完成 | `repoStore.ts` 的 `svnBaselineMismatch`、`RepoTab.tsx`、`BaselineBanner.tsx`、`BaselineBanner.test.tsx` | `pnpm build`、`pnpm test`（1931 passed） |
| 4 C 的菜单 | 完成 | `Toolbar.tsx`「SVN ▾」、`ContextMenu.tsx` 加 `title` 与 `openAt`、`Toolbar.test.tsx` | 同上 |
| 5 注释、文档、验证 | 完成 | `svn_init.rs` 过时注释、e2e 文档提示、`svnSync.ts` / `repoStore.ts` 注释 | fmt / clippy 0 告警；cargo test 相关 6 个二进制（`--lib`、`svn_baseline_mismatch`、`commands_are_async`、`svn_backend`、`svn_init`、`svn_sync`）全过，其余二进制没跑；真实窗口验收见下 |

### 真实窗口验收（2026-11-25，`pnpm tauri dev --config` 隔离 identifier，scratchpad 临时 fixture）

`app.exe` 路径核对为本 worktree 的 `src-tauri\target\debug\app.exe`。

| 场景 | 结果 |
|---|---|
| 初始化后正常状态 | 顶部没有横幅；工具栏有「SVN ▾」，菜单为「体检基线…」「TortoiseSVN 钩子…」，都可点 |
| invoke `svn_baseline_mismatch` | `{ path }` 返回 `{"files":[],"total":0,"truncated":false}`；故意写错成 `{ repoPath }` 被拒（missing required key `path`） |
| 改坏基线（在 `work` 上提交错内容、`svn/base` 指过去、`svn revert` 磁盘） | 主动出横幅「点『体检』核对 1 个文件……」，列出 `Code/A.cs` |
| 点「体检」 | 「基线与服务器不符，共 1 处：Code/A.cs」+「重建基线」 |
| 重建 | 横幅换成「当前分支落后基线 → 跟进」（设计 §6.2 预期的让位） |
| 跟进 | 跟进完成，「No changes」，没有横幅，fixture 的 `git status` 为空 |
| 用户自己改 `Code/B.cs` | 「Local Changes (1)」，没有横幅 |

小瑕疵（未修）：菜单开着时顶部冒出横幅，工具栏被推下去，菜单停在原坐标。`ContextMenu` 只在窗口 resize 时关，点别处就关，影响很小。

## 决策条目

### 总体

- **用户决定**：2026-11-25 用户回复「要」，即按 review 意见修订报告并实现 B + C。报告原文不在仓库里，修订后落为 `docs/superpowers/specs/2026-11-25-svn-baseline-mismatch-design.md`。
- **偏离原报告**：判定条件从三条加到四条（前提门槛 + 现跑 svn status + 严格「完全干净」），理由写在 spec §6.1。
- **偏离原报告**：反方向症状（SVN 改了、git 干净）本期不做，进 spec §6.5 与非目标。理由：要对整个白名单跑 `svn status`，不能挂在每次刷新上。
- **新增（原报告没有）**：体检 `Verified` 之后同一组文件不再提示（spec §6.2），兜住行尾 / 关键字展开这类体检查不出差异、横幅却一直挂着的情形。ack 只存在 RepoTab 的 state 里，关掉标签就忘；同一组文件下次打开仓库会再提示一次。取舍：持久化要多一份落盘状态，而这类情形本身少见，先不做。

### Task 1

- **计划没写的坑**：`svn status` 带 `peg_safe` 的 target（末尾补 `@`）时，输出的 path 不带那个 `@`。代码里仍 `trim_end_matches('@')` 一次，防将来 svn 版本差异；用例覆盖的是不带 `@` 的常规路径。
- **计划没写的坑**：判定本身分不出「基线记错」与「TortoiseSVN 刚 update、还没跟进」，两者在 git / SVN 两侧的形状完全相同。用 `files_updated_by_svn_before_follow_look_the_same_so_the_frontend_gate_is_required` 把这一点钉成测试，前提门槛放在前端（`mismatchCheckApplies`），后端文档写明「只在前提成立时调」。
- **为什么是安全的**：整条路径只读：`git diff` / `git diff-tree` 不写索引以外的东西（`git diff HEAD` 会刷新 stat cache，与「本地改动」的 `git status` 相同）；`svn status` 是本地读，不进扫描缓存。测试里构造坏基线用的 `git update-ref` / `svn revert` 都在 `TempDir` 的 fixture 里。
- **偏离计划**：「候选为空时不起 svn 进程」没有写进程计数测试。进程计数要单独一个测试二进制（全局性能记录器一个进程只能装一次，见 `svn_follow_scan_budget.rs`），这里按代码结构保证：三处 `return Ok(BaselineMismatch::none())` 都在 `run_svn_targets` 之前，且 `run_svn_targets` 对空列表本身也不起进程。

### Task 2–4

- **偏离计划**：前提判定没有放进 `svnSync.ts`，而是新文件 `svnMismatch.ts`。它要用 `svnJournal.ts` 的 `otherBannersAreSuppressed`，而 `svnJournal.ts` 已经从 `svnSync.ts` 导入类型；放进 `svnSync.ts` 会成环。
- **与原常驻入口的差别**：`mismatchCheckApplies` 在 `sync === null`（还没读到失同步结论）和 `eol === null`（行尾状态还没读到）时都返回 `false`，原来的 `needsStandaloneCheck` 两者都放行。原因：常驻入口只是一颗按钮，早出现无害；这条横幅是一个结论，结论的前提没读到就不能下。
- **计划没写的坑**：`useContextMenu().open` 要一个真实的 React 事件（会调 `preventDefault`），拿 `{ ...e }` 展开出来的对象没有原型上的方法。给它加了 `openAt(x, y, items)`，下拉按钮用它贴着按钮左下角弹出。
- **计划没写的坑**：菜单项原来没有悬停说明，置灰时看不出原因。`MenuItem` 加了可选的 `title`，与工具栏「更新」「交付」置灰时用 `title` 说明原因同一做法。

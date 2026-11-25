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

### 独立 review（2026-11-25，opus 子 agent，只读）

| 条目 | 级别 | 处理 |
|---|---|---|
| 1. TortoiseSVN update 之后，失同步检测要几秒才跟上，这段时间前端门槛看的是旧的 InSync，刚 update 下来的文件被报成「基线记错」，出口是重建基线。每天早上必现 | 阻断 | **已修**。判定自己比对 revision：SVN 的 `c(f) > R(f)`（前进）或 `n(f) < R(f)`（回退）就丢掉（`svn_mismatch.rs::same_revision`，判据与 `svn_sync::judge` 相同）。原来钉「前端必须挡」的用例改成断言为空，另补「无关的 update 之后，记错的文件照样报」 |
| 2. 每次「本地改动」刷新都起一个 svn 进程，连续保存会叠起来 | 应修 | **已修**。RepoTab 里 1.5 秒防抖（`MISMATCH_DEBOUNCE_MS`）+ `createSingleFlight` 单飞补跑。没有按路径集合跳过：同一组脏文件里，TortoiseSVN revert 一下就可能从「SVN 说改了」变成「SVN 说干净」，路径集合不变，跳过会漏 |
| 3. 体检 `verified` 时，横幅上的文件可能在 `unverified` 里（体检没比过），照样记 ack 会把横幅永久压掉 | 应修 | **已修**。`mismatchAckAfterCheck`：有交集就不记，补 vitest |
| 4. 一个候选让 `svn status` 非零退出，整批作废，横幅静默不出 | 建议 | 未改。候选都是白名单内、git 跟踪的 W-file，必在工作副本里；即使发生，后果是漏报（只打 `console.warn`），手动体检仍在 |
| 5. 注释说必须现跑，代码走的是会读扫描缓存的 `run_svn` | 建议 | **已修**。改成 `chunk_targets` + `run_svn_fresh` |
| 6. 缺 props 改过、copied、external、截断、`@` 路径的用例 | 建议 | 补了 props 改过的 fixture 用例；其余未补，copied 与 external 的过滤与 `svn_dirty::read_dirty` 同一写法 |
| 7. 菜单开着时置灰状态不刷新 | 建议 | 未改，影响很小 |

复核（同一个 review agent，只读看 b01d8b9）：无阻断。两条建议：

- 一批候选在 SVN 侧都没有 revision（unversioned / added）时，`parse_node_status` 为全量扫描设的守卫报 `svn_xml_empty_node_revs`，整次查询失败、横幅静默不出。**已修**：这两个错误码按「这批没有命中」处理；先写用例 `a_candidate_that_svn_does_not_know_does_not_fail_the_query` 复现到这个错误码，再改。
- ack 的键只取横幅上的前 50 个文件，只对它们和 `unverified` 求交集。未改：超过 50 个命中时体检本来就会报 `lying`，不会走到记 ack。

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

修完 review 之后在同一个 fixture 上再验一次：重新改坏 `Code/A.cs` 的基线、同时开着用户对 `Code/B.cs` 的未提交改动，横幅在防抖之后出现，只列 `Code/A.cs`。「update 之后、失同步检测跟上之前」那几秒的时序在真实窗口里不好稳定复现，由 Rust 用例 `files_updated_by_svn_before_follow_are_not_reported` 覆盖判据本身。

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

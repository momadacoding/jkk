# Phase 3a：`sync_out` 交付闭环 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 SVN 混合仓库「能交付」：一次点击走完 spec §8.2 的 D0–D10，把当前分支相对 `svn/base` 的净变更（摘掉 Local-Only commit）提交到 SVN，再把基线推进到服务器的新 revision、把工作分支接到新基线上；D5 之前的任何失败零代价收尾，D5 被服务器明确拒绝时精确回滚登记，D5 之后的中断能继续或放弃。

**Architecture:** 后端新增四个 engine 模块：`svn_deliver`（D0.5 构造交付树，对象层重放，不碰磁盘）、`svn_sync_out_check`（§8.1 前置检查 + D1/D2 清单，纯读）、`svn_stage`（D4 登记与 §8.5 精确撤销）、`svn_sync_out`（D0–D10 编排、早退收尾、继续 / 放弃状态机）。命令层在 `commands/svn_sync.rs` 加四个命令。前端照 `sync_in` 的结构：纯逻辑 `engine/svnSyncOut.ts`、流程 `components/svnSyncOutFlow.ts`（依赖注入，可在 node 里测），外加一个能输入提交信息的 `DeliverDialog`。

**Tech Stack:** Rust（git CLI 写 / libgit2 读 / svn CLI）、Tauri 2 命令、React 18 + TypeScript、vitest + React Testing Library（jsdom）、cargo test（fixture 仓库）。

**Spec:** `docs/superpowers/specs/2026-09-12-svn-git-hybrid-design.md`——主要是 §6.7、§7.3.2、§7.6、§8（全节）、§12、§13、§14.2。执行者必须**两份一起读**：本计划只在 spec 沉默、与实测冲突或与 Tauri 的调用模型冲突的地方做决定，并在下面「对 spec 的补足与偏离」里逐条写明。

**上一期：** Phase 2c（PR #49）交付了 `sync_in`；PR #53（`58becab`）补了基线分支护栏，并让重建 / 迁移全程持锁。本期在它们上面施工，**不改 `sync_in` 的对外语义**。

**分期（用户 2026-10-07 决定）：** Phase 3 拆成 3a / 3b。**3a = 本计划**。**3b** = 断网查证（§8.6.2 的 opid revprop 与内容比对兜底）、revprop 开仓自检、服务器拒绝幂等键时由用户确认重试（§8.6.3 形态 B）、`committing` 阶段的继续入口。D9 的「移到 `local/<name>` 分支」入口也推后（同日决定），本期 D9 只给文字提示。

---

## Global Constraints

- 本仓库主干是 `master`；本期在 worktree `.claude/worktrees/svn-phase3a-sync-out`、分支 `worktree-svn-phase3a-sync-out` 上施工（基点：开工时最新的 `master`，即提交本计划文档的那个 commit，它在 `91ac411` 之上）。用 `EnterWorktree` 切入，切入后先跑 `git log --oneline -3` 确认基点（`AGENTS.md` §4.1 / §4.3）。
- `AGENTS.md` 最高优先级：**所有 git / svn 写操作的验证只在临时 fixture 里做**，禁止碰本仓库或任何真实工作副本。**真实窗口冒烟（Task 12）也只对 fixture 生成的临时混合仓库做**。
- 所有 git CLI 调用走 `engine::git_cli::{run_git, run_git_code, run_git_env}`；所有 svn 调用走 `engine::svn_cli::{run_svn, run_svn_code}`（`AGENTS.md` §7.3）。错误一律 `GitError::new(code, message)`，不向前端抛裸字符串。
- **空 target / 空 pathspec / 空参数列表一律拒绝**（spec §8.3、§12.2，`AGENTS.md` §6.2）。**空 target 的 `svn commit` 会提交整个工作副本，不可撤销**：D5 的 target 列表在调用前必须有独立断言（Task 5），并有专门测试（必测 #1）。
- **白名单外的文件绝不出现在任何 git / svn 命令的参数里**（spec §12.1，必测 #11）。
- **失败只回滚本流程自己做的修改**（spec §12.4）。删 ref 前断言前缀是 `refs/mygitclient/deliver/`；撤销 svn 登记只按「当前状态 − D4 之前的快照」里的 `added` / `deleted` 两类条目做。
- **D5 之后绝不重复提交、绝不自动回滚 SVN 侧**；任何一类失败都不自动重试（spec §8.7）。
- 新 Tauri 命令全部 `#[tauri::command(async)]`（`tests/commands_are_async.rs` 守着）；返回 / 参数结构体一律 `#[serde(rename_all = "camelCase")]`，枚举再加 `rename_all_fields = "camelCase"`，并在 `tests/svn_backend.rs` 里断言序列化后的字段名。
- 不新增任何 npm 包或 crate。`Cargo.toml` 的 `rust-version = "1.77.2"`：不用 `File::try_lock`、不用 `#[allow(..., reason = ...)]`。
- 注释用中文、解释「为什么」；前端单引号、尾随逗号、`@/` 别名。
- 验证：Rust 改动跑 `cargo fmt --check` → `cargo clippy --all-targets` → `cargo test`（在 `src-tauri/`）；前端改动跑 `pnpm build` → `pnpm test`。**本仓库没有 lint、没有 PR 门禁 CI**，禁止声称「lint 通过」「CI 通过」。`tsc` 不覆盖 `tests/vitest/`。
- 新 Tauri 命令必须实跑一次 `pnpm tauri dev` 确认 invoke 通（`AGENTS.md` §7.3）。
- 决策日志：`docs/superpowers/plans/2026-10-07-svn-phase3a-sync-out-decisions.md`，Task 0 建，每个 Task 完成立即更新（`AGENTS.md` §2.1）。
- PR 描述与 commit message 里**不写裸的 `#数字`** 给条目编号（GitHub 会链成 PR）；本计划内部的「必测 #N」「B3」引用到 PR 文本时改写成「必测第 N 条」「偏离 B3」。

---

## 对 spec 的补足与偏离（review 先读这一节）

执行前在临时仓库上实测过的事实一律标「实测」，记录见下文「开工前实测」。每一条都要抄进决策日志对应 Task 的「偏离计划 / 计划没写的坑」。

| # | 内容 | 类型 | 依据 |
|---|---|---|---|
| B1 | **D3 的确认移到流程之外：预览（纯读）→ 用户确认 → 执行时锁内复核**，与 `sync_in` 同一模型。预览就把 D0.5（对象层，见 B2）、D1、D2 算出来给用户看；执行时重算一遍，交付树的 **tree OID** 或文件清单与预览不同就收尾并报 `sync_out_needs_reconfirm`。「D3 用户取消」于是不再是一个需要清理的出口：用户取消时执行命令根本没被调用 | 偏离（结构性） | Tauri 命令不能在中途停下等用户点确认。照 spec 字面做，要么在持锁、journal 已写的状态下跨两次 invoke，要么在预览里建临时 worktree 却不记 journal。两者都会在崩溃时留下孤儿 |
| B2 | **D0.5 慢路径不用临时 worktree + cherry-pick，改用 `git merge-tree --write-tree --merge-base=<C^> <cur> <C>` 在对象层逐个重放**，再 `commit-tree` 保留原作者与原 message。磁盘、索引、HEAD、worktree 注册全不碰。journal 的 `tempWorktree` 本期不再写入；收尾代码仍处理它（字段存在就按 §8.2.4 ① 清，兼容将来） | 偏离（实现方式） | **实测 M1**：T1（不重叠）时两种方式得到的 tree 完全相同；T2（紧挨着）时 `merge-tree` 退出 1 并列出冲突文件的三个 stage，与 cherry-pick 一样报冲突。两条硬约束（D0.5 不动主工作副本、早退必须清干净）在这个做法下天然成立：没有东西要清。代价：需要 git ≥ 2.40（`--merge-base`），旧 git 在慢路径上报 `sync_out_git_too_old`，快路径不受影响 |
| B3 | **D5 失败的分类：只在 stderr 里每一个 `E\d{6}` 错误码都属于「服务器确定没变」的集合时才回滚**。集合 = `E155011`、`E160028`、`E160013`、`E160024`、`E165001`、`E170001`、`E200009`、`E200007`。其余情况（含 `E170013` / `E180001`、没有错误码、退出非零但 stdout 里有 `Committed revision`）一律判「状态未知」，phase 停在 `committing`，不回滚、不推进 | 补足 | **实测 M2**：out-of-date 是 `E155011 + E160028`；同事删了我要改的文件是 `E155011 + E160013`；钩子拒绝是 `E165001`；仓库不可达是 `E170013 + E180001`。只有前三类能确定服务器没变。断网时的错误码随协议不同，保守地全归「未知」 |
| B4 | **3a 里 `committing` 阶段的「继续」与「放弃」都拒绝**（`sync_out_commit_unknown`），横幅只显示 opid、`expectedStartRev` 与提交信息，请用户在 TortoiseSVN 的 log 里核对。查证在 3b | 分期决定 | 盲目回滚会把可能已成功的提交再交付一遍（§8.6）；盲目推进会让基线撒谎。D3.5 刚确认过网络，所以只有提交途中断线才会走到这里 |
| B5 | `newParentDirs` = D4 之后新出现的 `added` 条目里**磁盘上是目录**的那些。`svn status --xml` 不给 `kind`，只能在 D4 当下查磁盘 | 补足 | **实测 M2-E1**：`add --parents` 之后 XML 里三条 entry 都只有 `item="added" props="none" revision="-1"`，路径用反斜杠（`parse_status_entries` 会归一成正斜杠） |
| B6 | **撤销 D4 不只看 journal 里的 `svnDelta`，而是以「当前状态 − `svnStatusBefore`」重算，只取 `added` / `deleted` 两类**。`svnDelta` 照常落盘（3b 查证与人工排查要用） | 补足 | 崩在 `svn add` 之后、写 `svnDelta` 之前时，journal 里只有 `svnStatusBefore`。D4 只会产生这两类登记，`svnStatusBefore` 在 D4 第一条 svn 命令之前就已落盘 |
| B7 | **D5 之后的「放弃」= 停止自动收尾**：若我们的 rebase 在进行就 `rebase --abort`；HEAD 不在原分支上时，先确认白名单内 W-file 在 svn 看来全部干净（= 磁盘就是服务器内容，能取回），再 `checkout -f` 回原分支；删交付 ref 与 journal。基线若已发布就留在新位置；若没发布，失同步横幅会给出「更新」（2c 的 A16 接管）或「重建基线」 | 补足 | spec §7.6 表格只写了「不回滚」，没写放弃怎么收尾。提交已在服务器上，放弃只能是「不再替用户做后面的事」，分支与磁盘回到可理解的状态 |
| B8 | **D5 之前的所有 phase（`prepare` … `svn-staged`），「继续」与「放弃」都走同一条清理路径**：`svn-staged` 先按 B6 撤销登记，再走 §8.2.4。横幅只给一个「清理」按钮 | 补足 | 继续往 D5 走需要用户重新看过清单（B1）；崩溃之后什么都可能变了，直接提交不安全 |
| B9 | **D2 发现白名单外的变更时拒绝交付**（`sync_out_outside_whitelist`），不静默过滤 | 偏离（更严） | spec 说「按白名单过滤」。但 D8d 之后分支会快进到新基线（= 服务器内容），被过滤掉的改动会从分支上消失。exclude 规则下 git 本不该跟踪它们，出现了就是 `git add -f` 之类的人为操作，停下来让人看 |
| B10 | **D8d 无 Local-Only 时不切两次分支**：先 `git update-ref refs/heads/<orig> <NEWBASE> <origHead>`（CAS），再从 `svn/base` 切到 `<orig>`。两者指向同一个 commit，git 不写任何文件 | 偏离（等价实现） | spec 写的是 `checkout <orig>` + `merge --ff-only svn/base`：先把磁盘切回不含 D6 服务器改动的旧内容，再快进回来，多两次 Unity 全量重编译（§7.5 的口径）。快进条件改成显式的 `merge-base --is-ancestor <origHead> <NEWBASE>` 断言，不成立就报 `sync_out_not_fast_forward`（§13「D8d 不是快进」） |
| B11 | D8d 有 Local-Only 时，rebase 前的 merge 门槛对 `merge-base(NEWBASE, origHead)..origHead` 做，不对 `deliverRef..origHead` 做 | 补足 | 慢路径的交付 commit 是重放出来的新对象，不是 `origHead` 的祖先；rebase 真正要重放的区间是从两者的 merge-base 开始的。快路径下 merge-base 就是 `deliverOid`，与 spec 字面一致 |
| B12 | **pinned 模式下 `sync_out` 在动手前拒绝**（`sync_out_pinned_unsupported`），必测 #53 / #55 推迟 | 与 2c 的 A1 一致 | `sync_in` 已经拒绝 pinned；只能交付不能更新的仓库没有意义 |
| B13 | **D3.5 合并进执行时的锁内复核**：执行一开始的前置检查里那次 `remoteRev` 查询就是 D3.5（它发生在用户确认之后），值写进 `expectedStartRev` | 补足（B1 的推论） | — |
| B14 | `svn commit` 加 `--force-log` | 补足 | 提交信息恰好是工作副本里一个已有文件的路径时，svn 以 `E205004` 拒绝 `-m`，要求改用 `-F`。用户写什么都不该让交付失败 |
| B15 | 交付 commit 的作者沿用原 commit（`GIT_AUTHOR_NAME/EMAIL/DATE`），提交者是当前用户 | 补足 | 与 cherry-pick 的行为一致；`git log` 里看得出这是重放出来的 |

---

## Review Focus

spec 没有逐条写到、但最可能咬到真实用户的五类输入。每条都在对应 Task 里有一条钉住它的测试。

1. **提交信息恰好是一个文件路径，或以 `-` 开头、或含换行** → 照常提交，服务器上的 log 原样。钉在 Task 5 `a_message_that_looks_like_a_path_or_an_option_is_committed_verbatim`（B14）。
2. **用户开着确认框的几分钟里，在 IDE 里又提交了一个 commit** → 执行被拒（`sync_out_state_changed`），SVN 侧、磁盘、分支都没动，提交信息不丢（前端保留输入框内容）。钉在 Task 7 `a_commit_made_while_the_dialog_was_open_is_caught_before_anything_moves` 与 Task 11 `svnSyncOutFlow.test.ts` 的「失败后保留提交信息」。
3. **Unity 开着、占着一个要交付的 `.cs`，D3.8 的 checkout「成功」却没写进去** → 指名文件、回到原分支、SVN 侧一个字节没动。钉在 Task 7 `a_file_held_open_at_checkout_is_named_and_nothing_is_committed`（Windows；share=Read 打开，与 2c 的 A15 同一手法）。
4. **D5 之后同事又提交了同一目录下的文件** → D6 把它拉下来，新基线 = 服务器最新内容（不是只有我的提交），分支快进到它。钉在 Task 5 `a_colleague_commit_between_d5_and_d6_ends_up_in_the_new_baseline`（用 probe 在 `Committed` 之后让 wc2 提交）。
5. **交付后马上再点一次「交付」，或马上点「更新」** → 不误报失同步（必测 #26）、没有可交付的净变更时明确说「本次没有可交付的改动」并列出本地补丁数，不弹空确认框。钉在 Task 5 `delivering_again_right_after_a_delivery_has_nothing_to_deliver` 与 `no_false_out_of_sync_after_a_delivery`。

---

## 文件地图

| 文件 | 新建 / 修改 | 职责 |
|---|---|---|
| `src-tauri/src/engine/svn_journal.rs` | 改 | `SvnNode`；字段 `svn_modified`、`svn_status_before`、`svn_delta`、`deliver_oid`、`committed_rev` |
| `src-tauri/src/engine/graph.rs` | 改 | `parse_local_only` 改 `pub(crate)` |
| `src-tauri/src/engine/svn_deliver.rs` | **新** | D0.5：Local-Only 识别、快 / 慢路径、`merge-tree` 重放（B2）、交付 ref 的建与删 |
| `src-tauri/src/engine/svn_sync_out_check.rs` | **新** | §8.1 第 0–8 条（纯读）、D1/D2 清单、白名单外本地修改、`remote_head_rev` |
| `src-tauri/src/engine/svn_stage.rs` | **新** | D4 登记、状态快照、§8.5 精确撤销（B5 / B6） |
| `src-tauri/src/engine/svn_sync_out.rs` | **新** | D0–D10 编排、D5 失败分类（B3）、早退收尾 §8.2.4、继续 / 放弃（B4 / B7 / B8）、测试用 `Probe` |
| `src-tauri/src/engine/svn_sync_check.rs` | 改 | `check_git_idle`、`precheck_include_set` 改 `pub(crate)` |
| `src-tauri/src/engine/svn_sync_in.rs` | 改 | `checkout_verified` 抽出「核对」那半为 `verify_checkout`（供 detached 用）；`svn_conflicts`、`tracked_changes`、`untracked_leftovers` 改 `pub(crate)` |
| `src-tauri/src/engine/mod.rs` | 改 | 挂四个新模块 |
| `src-tauri/src/commands/svn_sync.rs` | 改 | 四个命令：`svn_sync_out_preview`、`svn_sync_out`、`svn_sync_out_continue`、`svn_sync_out_abort` |
| `src-tauri/src/lib.rs` | 改 | 注册 |
| `src-tauri/tests/svn_deliver.rs` | **新** | 交付树（T1 / T2 / T4 / 快慢路径 / 交付 ref） |
| `src-tauri/tests/svn_sync_out_check.rs` | **新** | 前置检查 |
| `src-tauri/tests/svn_stage.rs` | **新** | D4 与撤销 |
| `src-tauri/tests/svn_sync_out.rs` | **新** | 主路径、Local-Only、早退、D5 失败、继续 / 放弃 |
| `src-tauri/tests/common/svn_fixtures.rs` | 改 | `deliver_expect()` 等辅助、`hold_open()`（Windows share=Read） |
| `src-tauri/tests/svn_journal.rs`、`svn_backend.rs`、`svn_write_guard.rs` | 改 | 各自补用例 |
| `src/engine/svnSyncOut.ts` | **新** | 类型、预览 → 对话框模型、结局文案、横幅动作 |
| `src/engine/svnJournal.ts` | 改 | `sync_out` 各 phase 的横幅正文与动作 |
| `src/engine/svnSync.ts` | 改 | `BannerAction.id` 加 `syncOutCleanup` / `syncOutContinue` / `syncOutAbort` |
| `src/engine/toolbarActions.ts` | 改 | `showsSvnDeliver(caps)` |
| `src/store/repoStore.ts` | 改 | 四个 invoke |
| `src/components/DeliverDialog.tsx` | **新** | 交付确认框：提交信息、A/M/D 清单、被摘掉的本地补丁、白名单外修改的勾选确认 |
| `src/components/svnSyncOutFlow.ts` | **新** | 预览 → 对话框 → 执行 → 刷新 → 结局；横幅动作 |
| `src/components/Toolbar.tsx`、`BaselineBanner.tsx`、`RepoTab.tsx` | 改 | 「交付」按钮、横幅动作、对话框接线 |
| `tests/vitest/svnSyncOut.test.ts`、`svnSyncOutFlow.test.ts`、`DeliverDialog.test.tsx` | **新** | |
| `tests/vitest/svnJournal.test.ts`、`toolbarActions.test.ts` | 改 | |
| `docs/superpowers/specs/2026-09-12-svn-git-hybrid-design.md` | 改 | §8.2.3 慢路径注记（B2）、§8.6 分类注记（B3）、§18 修订记录 |
| `docs/superpowers/e2e/2026-10-07-svn-phase3a-e2e.md` | **新** | 真实窗口清单 |
| `docs/superpowers/plans/2026-10-07-svn-phase3a-sync-out-decisions.md` | **新** | 决策日志 |

**行尾：** `src/vcs/svn.rs`、`src/commands/watch.rs`、`repoStore.ts`、`BaselineBanner.tsx` 是 CRLF，其余多为 LF。用 Edit 工具改即可；用脚本批量替换前先数 `CR` / `LF` 字节，改完复核。

---

## 错误码一览（前后端共用，Task 10 的文案按它写）

| code | 何时 | 修改了什么 |
|---|---|---|
| `sync_in_progress` | 锁被别人拿着（2b 的 `busy_error`，沿用） | 无 |
| `sync_in_unfinished` | 已有未收尾的 journal（沿用） | 无 |
| `sync_out_empty_message` | 提交信息 trim 后为空 | 无 |
| `sync_out_git_busy` | git 在 rebase / merge / cherry-pick / revert 中途，或索引有未合并条目 | 无 |
| `sync_out_detached_head` | HEAD 没附着在分支上（§8.1.1） | 无 |
| `sync_out_on_baseline` | 当前在 `svn/base` 上 | 无 |
| `sync_out_behind_baseline` | §8.1 第 0 条：`svn/base` 不是 HEAD 的祖先 | 无 |
| `sync_out_pinned_unsupported` | B12 | 无 |
| `sync_out_out_of_sync` | `nodeRevs` 不是 `{baselineRev}` | 无 |
| `sync_out_dirty` | §8.1 第 2 条前半：白名单内 git 有未提交改动 | 无 |
| `sync_out_svn_conflicted` | 白名单内有 svn 冲突态 | 无 |
| `sync_out_server_unreachable` | 读服务器 HEAD revision 失败 | 无 |
| `sync_out_server_ahead` | §8.1 第 5 条 / D3.5：服务器 HEAD ≠ `baselineRev` | 无（执行时：已收尾） |
| `sync_out_merge_in_slow_path` | §8.1 第 6 条 | 无 |
| `sync_out_local_only_merge` | §8.1 第 7 条 | 无 |
| `sync_out_local_only_conflict` | D0.5 重放冲突（T2） | 无 |
| `sync_out_replay_failed` / `sync_out_git_too_old` | D0.5 非冲突失败 / git 不支持 `--merge-base`（B2） | 无 |
| `sync_out_nothing_to_deliver` | D1/D2 空清单（§8.3） | 无（执行时：已收尾） |
| `sync_out_outside_whitelist` | B9 | 无 |
| `sync_out_unsupported_change` | D1 出现 `T`（类型变化）等非 A/M/D 状态 | 无 |
| `include_set_mismatch` | 纳入集合预检 / D7a 核对不过（沿用） | 预检：无；D7a：停在 `post-commit` |
| `sync_out_needs_reconfirm` | B1：执行时的交付树、清单、白名单外修改与预览不同 | 已收尾 |
| `sync_out_state_changed` | 锁内复核 / 继续前复核发现 HEAD、分支或基线变了 | 无（或保留 journal 原样） |
| `sync_in_checkout_failed` | D3.8 / D8c / D8d / 收尾时 checkout 没切干净（沿用 `checkout_verified` 的码） | D3.8：已收尾；之后：journal 保留 |
| `sync_out_disk_changed` | D4.9 断言失败（§8.2.5） | 已撤销登记并收尾 |
| `sync_out_stage_failed` | D4 的 `svn add` / `svn delete` 失败 | 已撤销登记并收尾 |
| `sync_out_rollback_incomplete` | 撤销后 `svn status` 与 `svnStatusBefore` 不一致 | journal 保留，停在 `svn-staged` |
| `sync_out_commit_rejected` | D5 被服务器明确拒绝（B3 集合） | 已撤销登记并收尾 |
| `sync_out_commit_unknown` | D5 状态未知（B3 / B4） | journal 停在 `committing` |
| `sync_out_update_failed` / `sync_out_update_conflict` | D6 | journal 停在 `post-commit` |
| `sync_out_revision_moved` | D7 前后 `nodeRevs` 变了 | journal 停在 `post-commit` |
| `svn_baseline_moved` | D8b CAS 失败（沿用） | journal 停在 `publishing-baseline` |
| `sync_out_not_fast_forward` | B10 的祖先断言不成立 | journal 停在 `restoring-branch` |
| `sync_out_rebase_failed` | D8d rebase 非零且不是冲突 | journal 停在 `restoring-branch` |
| `sync_out_undelivered_left` | D10：`svn/base..<orig>` 里还有没标 Local-Only 的 commit | journal 停在 `restoring-branch` |
| `sync_out_unsafe_to_return` | B7 放弃时 W-file 在 svn 看来不干净 | journal 保留 |
| `sync_out_bad_deliver_ref` | 要删的 ref 不在 `refs/mygitclient/deliver/` 下 | 无 |
| `sync_no_unfinished` | 点继续 / 放弃时没有 journal（沿用） | 无 |
| `sync_out_wrong_flow` | `sync_out` 的继续 / 放弃遇上 `sync_in` 的 journal | 无 |

---

## 开工前实测（写计划时在临时仓库上跑过，执行者不必重跑）

脚本在写计划时的 scratchpad 里，跑完已删。结论如下。

**M1：`merge-tree` 重放与 cherry-pick 等价**（git 2.51.2）

```text
base：A.cs = 1..30 各一行；B.cs
C1 改第 1 行；L（Local-Only）改第 10 行；C2 改第 20 行；C3 新增 Deep/Nested/N.cs 并删 B.cs

T1：对 C1 C2 C3 逐个 `git merge-tree --write-tree --merge-base=<C^> <cur> <C>`，再 commit-tree
    结果：A.cs 第 1 / 10 / 20 行 = one / 10 / twenty；树里只有 A.cs、Deep/Nested/N.cs
    对照：临时 worktree 里 cherry-pick 同一批 commit → tree OID 相同（trees equal? yes）
T2：再加 C4 改第 11 行（紧挨 L 的第 10 行），重放到 C4 时
    merge-tree 退出 1，输出首行是 tree OID，随后是冲突信息：
      100644 <oid> 1	A.cs
      100644 <oid> 2	A.cs
      100644 <oid> 3	A.cs
主工作区：全程没有任何改动
```

**M2：D4 / D5 的 svn 行为**（SlikSvn 1.14.2，`file://` 仓库）

```text
E1  svn add --parents Code/X/Y/Q.cs 之后 svn status --xml Code/X：
      三条 entry（Code\X、Code\X\Y、Code\X\Y\Q.cs），wc-status 只有 props / item="added" / revision="-1"，
      没有 kind 属性；路径是反斜杠
E2  rm Code/Old.cs → svn status 显示 `!`；svn delete Code/Old.cs → `D`
E3  svn commit --depth=empty -- Code/Deep Code/Deep/Nested Code/Deep/Nested/New.cs Code/Old.cs Code/A.cs
      → 一个 revision：M A.cs / A Deep / A Nested / A New.cs / D Old.cs
E4  同事先改 Shared.cs 并提交，我也改 Shared.cs 再提交：
      svn: E155011: Commit failed (details follow):
      svn: E155011: File '...\Shared.cs' is out of date
      svn: E160028: File '/Code/Shared.cs' is out of date            退出 1
E5  同事删了 A.cs，我改 A.cs 再提交：E155011 + E160013（File not found: transaction ...）
E6  同事改的是别的文件：我的提交照样成功（Committed revision 6），之后工作副本是
      mixed-revision（revision 2 / 3 / 4 / 6 都有）——D6 必须把它拉齐
E7  pre-commit.bat 退出 1：E165001 Commit blocked by pre-commit hook (exit code 1) with output: …
E8  仓库目录改名（不可达）：E170013 Unable to connect + E180001 Unable to open repository
```

**M3：`git branch -D @{-1}` 这类简写**（PR #53 已修，`SvnGitBackend` 的删 / 改名守卫拒绝含 `@{` 的名字）。本期的 `update-ref` 一律用完整 ref 名 `refs/heads/<orig>`，不受简写影响。

---

## Task 0：决策日志 + 开工自检

**Files:**
- Create: `docs/superpowers/plans/2026-10-07-svn-phase3a-sync-out-decisions.md`

- [ ] **Step 1: 切入 worktree 并确认基点**

用 `EnterWorktree`（`name: svn-phase3a-sync-out`）建并切入，然后：

```powershell
Get-Location
git log --oneline -3      # 期望首行是 docs(plans): Phase 3a … 那个提交，且与 origin/master 一致
git status --short        # 期望为空
```

`Get-Location` 不在 `.claude\worktrees\svn-phase3a-sync-out` 里就停下（`AGENTS.md` §4.1 的自检信号）。

- [ ] **Step 2: 建决策日志**

```markdown
# Phase 3a `sync_out` 决策日志

计划：`2026-10-07-svn-phase3a-sync-out.md`。

## 进度总账

| Task | 状态 | 产出文件 | 验证 |
|---|---|---|---|
| 0 决策日志 + 自检 | ✅ | 本文件 | 基点 <开工时 git log -1 的短 OID> |
| 1 journal 字段 + Local-Only 解析 | ⏳ | | |
| 2 交付树 D0.5 | ⏳ | | |
| 3 前置检查与清单 | ⏳ | | |
| 4 D4 登记与撤销 | ⏳ | | |
| 5 编排主路径（无 Local-Only） | ⏳ | | |
| 6 Local-Only 全流程 | ⏳ | | |
| 7 早退与 D5 失败 | ⏳ | | |
| 8 继续 / 放弃 | ⏳ | | |
| 9 D6 / D7a 失败 | ⏳ | | |
| 10 Tauri 命令 | ⏳ | | |
| 11 前端纯逻辑与流程 | ⏳ | | |
| 12 前端接线 + 真实窗口冒烟 | ⏳ | | |
| 13 E2E 清单、spec 修订、收尾 | ⏳ | | |

## 决策条目

（按 Task 分节。每节至少覆盖：偏离计划 / 计划的错误 / 计划没写的坑 / 为什么这么改是安全的。）
```

- [ ] **Step 3: 把计划的 B1–B15 抄进对应 Task 的「偏离计划」**（B1 / B13 → Task 3、5；B2 → Task 2；B3 / B4 / B14 → Task 5、7；B5 / B6 → Task 4；B7 / B8 → Task 8；B9 → Task 3；B10 / B11 → Task 5、6；B12 → Task 3；B15 → Task 2）。

- [ ] **Step 4: Commit**

```bash
git add docs/superpowers/plans/2026-10-07-svn-phase3a-sync-out-decisions.md
git commit -m "docs(svn): Phase 3a 决策日志"
```

---

## Task 1：journal 字段 + Local-Only 解析共享

**Files:**
- Modify: `src-tauri/src/engine/svn_journal.rs`（`Journal` 结构体末尾，`expected_start_rev` 之后）
- Modify: `src-tauri/src/engine/graph.rs:44`（`fn parse_local_only` → `pub(crate) fn`）
- Test: `src-tauri/tests/svn_journal.rs`

**Interfaces:**
- Produces: `svn_journal::SvnNode { path: String, status: String, dir: bool }`（`Clone + PartialEq + Eq + Serialize + Deserialize`）；`Journal` 新字段 `svn_modified: Vec<String>`、`svn_status_before: Vec<SvnNode>`、`svn_delta: Vec<SvnNode>`、`deliver_oid: Option<String>`、`committed_rev: Option<u64>`；`crate::engine::graph::parse_local_only(&str) -> bool` 变成 `pub(crate)`。

- [ ] **Step 1: 写失败测试**

追加到 `src-tauri/tests/svn_journal.rs`（`use` 行加上 `SvnNode`）：

```rust
#[test]
fn a_sync_out_journal_round_trips_with_its_svn_snapshot() {
    // D4 的撤销靠 svnStatusBefore（计划 B6），3b 的查证靠 svnAdded / svnDeleted / svnModified
    // 与 expectedStartRev（spec §8.6.2）。少落一个字段，崩溃重开后就拼不回来
    let fx = git_dir_fixture();
    let root = fx.path().to_string_lossy().to_string();
    let j = Journal {
        opid: "op-out".into(),
        flow: Flow::SyncOut,
        phase: Phase::SvnStaged,
        started_at: 1_757_000_000,
        orig_branch: Some("work".into()),
        orig_head: Some("e5d3f77".into()),
        svn_added: vec!["Code/Deep/Nested/New.cs".into()],
        svn_deleted: vec!["Code/Old.cs".into()],
        svn_modified: vec!["Code/A.cs".into()],
        svn_status_before: vec![
            SvnNode { path: "Code/Deep".into(), status: "unversioned".into(), dir: true },
            SvnNode { path: "Code/Old.cs".into(), status: "missing".into(), dir: false },
        ],
        svn_delta: vec![SvnNode { path: "Code/Deep".into(), status: "added".into(), dir: true }],
        new_parent_dirs: vec!["Code/Deep".into(), "Code/Deep/Nested".into()],
        deliver_ref: Some("refs/mygitclient/deliver/op-out".into()),
        deliver_oid: Some("d".repeat(40)),
        commit_message: Some("修复跳跃".into()),
        expected_start_rev: Some(12),
        committed_rev: Some(13),
        ..Default::default()
    };
    write(&root, &j).unwrap();
    assert_eq!(read(&root).unwrap().as_ref(), Some(&j));

    // 线序格式：字段名逐字是 camelCase，`dir` 为假时不写
    let raw = std::fs::read_to_string(journal_path(&root)).unwrap();
    for key in ["svnStatusBefore", "svnDelta", "svnModified", "deliverOid", "committedRev"] {
        assert!(raw.contains(&format!("\"{key}\"")), "缺字段 {key}：{raw}");
    }
    assert!(!raw.contains("\"dir\":false"), "{raw}");
}

#[test]
fn a_journal_written_before_phase_3a_still_reads() {
    // 2c 写出的 sync_in journal 没有任何新字段，读回来必须照常（新字段取空值）
    let fx = git_dir_fixture();
    let root = fx.path().to_string_lossy().to_string();
    std::fs::write(
        journal_path(&root),
        r#"{"opid":"x","flow":"sync_in","phase":"stashed","startedAt":7,"origBranch":"work"}"#,
    )
    .unwrap();
    let j = read(&root).unwrap().unwrap();
    assert!(j.svn_status_before.is_empty() && j.svn_delta.is_empty());
    assert_eq!(j.deliver_oid, None);
    assert_eq!(j.committed_rev, None);
}
```

- [ ] **Step 2: 跑测试确认失败**

Run（在 `src-tauri/`）：`cargo test --test svn_journal`
Expected: 编译失败——`SvnNode` 不存在、`Journal` 没有 `svn_modified` 等字段。

- [ ] **Step 3: 实现**

`svn_journal.rs`，在 `pub struct Journal` 之前加：

```rust
/// D4 前后一个 svn 节点的登记状态（spec §8.5；计划 B5 / B6）。
///
/// `status` 是 `<wc-status item>` 原样（`unversioned` / `added` / `deleted` /
/// `missing` / `modified` …）。`dir` 是**记录当下**磁盘上它是不是目录：
/// `svn status --xml` 不给 `kind`（计划开工前实测 M2-E1），而撤销时文件可能已经
/// 不在了，到那时再查磁盘就晚了。
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SvnNode {
    pub path: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dir: bool,
}
```

在 `Journal` 的 `expected_start_rev` 之后加：

```rust
    /// D1 清单里的 M（spec §8.6.2：查证要「本次提交的完整路径集合」）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub svn_modified: Vec<String>,
    /// D4 第一条 svn 命令**之前**的白名单 svn 状态（spec §8.5）。撤销的不变量是
    /// 「回到这份快照」，计划 B6：撤销时以「当前 − 它」重算差量，不只信 `svn_delta`。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub svn_status_before: Vec<SvnNode>,
    /// D4 之后相对 `svn_status_before` 新增的登记（spec §8.5 的 `svnDelta`）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub svn_delta: Vec<SvnNode>,
    /// `deliver_ref` 指向的 OID。D8a 的父提交（spec §8.2.2），D4.9 的断言对象。
    /// 单独记一份：ref 被删之后（崩在 D10 收尾中途）仍要知道它是谁。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deliver_oid: Option<String>,
    /// D5 的 `Committed revision N`。读不出来时为空，D7 会从 `nodeRevs` 重新得到。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub committed_rev: Option<u64>,
```

`graph.rs`：把 `fn parse_local_only(message: &str) -> bool` 改成 `pub(crate) fn parse_local_only(message: &str) -> bool`，doc 注释末尾加一句：「`svn_deliver` 用同一份判定摘掉本地补丁——两处规则不能漂移。」

- [ ] **Step 4: 跑测试确认通过**

Run：`cargo test --test svn_journal`
Expected: 全部 PASS（含 2b 的 `every_phase_uses_the_exact_string_from_the_spec`）。

- [ ] **Step 5: 全量 Rust 验证并 Commit**

```bash
cargo fmt --check && cargo clippy --all-targets && cargo test
git add src-tauri/src/engine/svn_journal.rs src-tauri/src/engine/graph.rs src-tauri/tests/svn_journal.rs
git commit -m "feat(svn): journal 补上 sync_out 的 svn 快照、交付 OID 与提交 revision"
```

更新决策日志进度总账。

---

## Task 2：交付树 D0.5（`svn_deliver`）

**Files:**
- Create: `src-tauri/src/engine/svn_deliver.rs`
- Modify: `src-tauri/src/engine/mod.rs`（`pub mod svn_deliver;`）
- Test: `src-tauri/tests/svn_deliver.rs`

**Interfaces:**
- Consumes: `graph::parse_local_only`（Task 1）；`git_cli::{run_git, run_git_code, run_git_env}`。
- Produces:
  - `pub const DELIVER_REF_PREFIX: &str = "refs/mygitclient/deliver/"`
  - `pub struct LocalOnlyCommit { pub oid: String, pub subject: String }`（serde camelCase，前端要显示）
  - `pub struct DeliverPlan { pub deliver_oid: String, pub deliver_tree: String, pub local_only: Vec<LocalOnlyCommit>, pub fast: bool }`
  - `pub fn plan_deliver(repo: &str, base: &str, head: &str) -> Result<DeliverPlan>`——纯对象层，不写任何 ref
  - `pub fn deliver_ref(opid: &str) -> String`
  - `pub fn pin(repo: &str, opid: &str, oid: &str) -> Result<String>`——建 ref，已存在则报错
  - `pub fn unpin(repo: &str, r: &str) -> Result<()>`——幂等，前缀不对报 `sync_out_bad_deliver_ref`

- [ ] **Step 1: 写失败测试**

`src-tauri/tests/svn_deliver.rs`：

```rust
//! D0.5：从 HEAD 摘掉 Local-Only commit 构造交付树（spec §8.2.3；计划 B2）。
//! 全部在临时 fixture 上跑。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::git_cli::run_git;
use app_lib::engine::svn_deliver::{deliver_ref, pin, plan_deliver, unpin};
use svn_fixtures::{hybrid_with_colleague, SvnFixture};

fn git(fx: &SvnFixture, args: &[&str]) -> String {
    run_git(&fx.wc_str(), args).unwrap().trim().to_string()
}

/// `Code/A.cs` 写成 30 行（第 n 行是 n），并作为普通 commit 提交。之后各测试改不同的行。
fn thirty_lines(fx: &SvnFixture) {
    let body: String = (1..=30).map(|n| format!("{n}\n")).collect();
    std::fs::write(fx.wc().join("Code/A.cs"), body).unwrap();
    git(fx, &["commit", "-qam", "thirty lines"]);
}

fn edit_line(fx: &SvnFixture, line: usize, to: &str, message: &str) {
    let p = fx.wc().join("Code/A.cs");
    let text = std::fs::read_to_string(&p).unwrap();
    let out: String = text
        .lines()
        .enumerate()
        .map(|(i, l)| if i + 1 == line { format!("{to}\n") } else { format!("{l}\n") })
        .collect();
    std::fs::write(&p, out).unwrap();
    git(fx, &["commit", "-qam", message]);
}

fn base(fx: &SvnFixture) -> String {
    git(fx, &["rev-parse", "svn/base"])
}

fn head(fx: &SvnFixture) -> String {
    git(fx, &["rev-parse", "HEAD"])
}

fn show(fx: &SvnFixture, rev: &str, path: &str) -> String {
    git(fx, &["show", &format!("{rev}:{path}")])
}

#[test]
fn without_local_only_the_head_is_delivered_as_is() {
    let fx = hybrid_with_colleague();
    thirty_lines(&fx);
    let p = plan_deliver(&fx.wc_str(), &base(&fx), &head(&fx)).unwrap();
    assert!(p.fast);
    assert_eq!(p.deliver_oid, head(&fx));
    assert!(p.local_only.is_empty());
}

#[test]
fn a_local_only_suffix_is_cut_off_without_replaying_anything() {
    // 快路径：只是给一个已有 commit 起名字（spec §8.2.3）
    let fx = hybrid_with_colleague();
    thirty_lines(&fx);
    edit_line(&fx, 1, "one", "C1");
    let c1 = head(&fx);
    edit_line(&fx, 10, "ten-LOCAL", "L\n\nLocal-Only: true");
    let p = plan_deliver(&fx.wc_str(), &base(&fx), &head(&fx)).unwrap();
    assert!(p.fast);
    assert_eq!(p.deliver_oid, c1, "交付的就是补丁之前那个已有 commit");
    assert_eq!(p.local_only.len(), 1);
    assert_eq!(p.local_only[0].subject, "L");
}

#[test]
fn a_local_only_in_the_middle_is_replayed_out_without_touching_the_disk() {
    // T1 + T6：不重叠时重放成功；磁盘、HEAD、索引、ref 全没动
    let fx = hybrid_with_colleague();
    thirty_lines(&fx);
    edit_line(&fx, 1, "one", "C1");
    edit_line(&fx, 10, "ten-LOCAL", "L\n\nLocal-Only: true");
    edit_line(&fx, 20, "twenty", "C2");
    let before_head = head(&fx);
    let before_disk = std::fs::read(fx.wc().join("Code/A.cs")).unwrap();
    let before_refs = git(&fx, &["for-each-ref"]);

    let p = plan_deliver(&fx.wc_str(), &base(&fx), &head(&fx)).unwrap();
    assert!(!p.fast);
    let a = show(&fx, &p.deliver_oid, "Code/A.cs");
    assert!(a.contains("one") && a.contains("twenty"), "{a}");
    assert!(!a.contains("ten-LOCAL"), "本地补丁进了交付树：{a}");
    assert_eq!(
        git(&fx, &["rev-parse", &format!("{}^{{tree}}", p.deliver_oid)]),
        p.deliver_tree
    );

    assert_eq!(head(&fx), before_head);
    assert_eq!(std::fs::read(fx.wc().join("Code/A.cs")).unwrap(), before_disk);
    assert_eq!(git(&fx, &["for-each-ref"]), before_refs, "预览不许留下任何 ref");
    assert_eq!(git(&fx, &["status", "--porcelain"]), "");
}

#[test]
fn replayed_commits_keep_their_author_and_message() {
    // 计划 B15：与 cherry-pick 一致，git log 里看得出是谁写的
    let fx = hybrid_with_colleague();
    thirty_lines(&fx);
    edit_line(&fx, 10, "ten-LOCAL", "L\n\nLocal-Only: true");
    let p = fx.wc().join("Code/A.cs");
    let text = std::fs::read_to_string(&p).unwrap().replace("20\n", "twenty\n");
    std::fs::write(&p, text).unwrap();
    git(
        &fx,
        &["-c", "user.name=Someone Else", "-c", "user.email=else@x", "commit", "-qam", "C2\n\nbody line"],
    );
    let plan = plan_deliver(&fx.wc_str(), &base(&fx), &head(&fx)).unwrap();
    assert_eq!(git(&fx, &["log", "-1", "--format=%an <%ae>", &plan.deliver_oid]), "Someone Else <else@x>");
    assert_eq!(git(&fx, &["log", "-1", "--format=%B", &plan.deliver_oid]), "C2\n\nbody line");
}

#[test]
fn an_overlapping_local_only_aborts_and_names_both_commits() {
    // T2：紧挨着的 hunk 冲突，必须中止（spec §8.2.3「绝不自动解冲突」）
    let fx = hybrid_with_colleague();
    thirty_lines(&fx);
    edit_line(&fx, 10, "ten-LOCAL", "mute log\n\nLocal-Only: true");
    edit_line(&fx, 11, "eleven", "fix jump");
    let before = head(&fx);
    let err = plan_deliver(&fx.wc_str(), &base(&fx), &head(&fx)).unwrap_err();
    assert_eq!(err.code, "sync_out_local_only_conflict", "{}", err.message);
    assert!(err.message.contains("mute log") && err.message.contains("fix jump"), "{}", err.message);
    assert!(err.message.contains("Code/A.cs"), "{}", err.message);
    assert_eq!(head(&fx), before);
    assert_eq!(git(&fx, &["status", "--porcelain"]), "");
}

#[test]
fn the_slow_path_refuses_any_merge_but_the_fast_path_does_not_care() {
    // 必测 #24 / #72①：慢路径 + merge → 前置拒绝；把补丁移到顶端（快路径）就能交付
    let fx = hybrid_with_colleague();
    thirty_lines(&fx);
    git(&fx, &["checkout", "-q", "-b", "side"]);
    std::fs::write(fx.wc().join("Code/Side.cs"), "side\n").unwrap();
    git(&fx, &["add", "Code/Side.cs"]);
    git(&fx, &["commit", "-qm", "side"]);
    git(&fx, &["checkout", "-q", "work"]);
    edit_line(&fx, 10, "ten-LOCAL", "L\n\nLocal-Only: true");
    git(&fx, &["merge", "-q", "--no-ff", "-m", "merge side", "side"]);
    edit_line(&fx, 20, "twenty", "C2");

    let err = plan_deliver(&fx.wc_str(), &base(&fx), &head(&fx)).unwrap_err();
    assert_eq!(err.code, "sync_out_merge_in_slow_path", "{}", err.message);
    assert!(err.message.contains("merge side"), "{}", err.message);

    // 同样的历史，但没有夹在中间的补丁：快路径，merge 原样在交付树里
    let fx2 = hybrid_with_colleague();
    thirty_lines(&fx2);
    git(&fx2, &["checkout", "-q", "-b", "side"]);
    std::fs::write(fx2.wc().join("Code/Side.cs"), "side\n").unwrap();
    git(&fx2, &["add", "Code/Side.cs"]);
    git(&fx2, &["commit", "-qm", "side"]);
    git(&fx2, &["checkout", "-q", "work"]);
    git(&fx2, &["merge", "-q", "--no-ff", "-m", "merge side", "side"]);
    let merged = head(&fx2);
    edit_line(&fx2, 10, "ten-LOCAL", "L\n\nLocal-Only: true");
    let p = plan_deliver(&fx2.wc_str(), &base(&fx2), &head(&fx2)).unwrap();
    assert!(p.fast);
    assert_eq!(p.deliver_oid, merged);
}

#[test]
fn a_local_only_suffix_that_contains_a_merge_is_refused() {
    // 必测 #72③ / §8.1 第 7 条：后缀会在 D8d 被 rebase 重放，rebase 会压平 merge
    let fx = hybrid_with_colleague();
    thirty_lines(&fx);
    git(&fx, &["checkout", "-q", "-b", "side"]);
    edit_line(&fx, 5, "five-LOCAL", "side patch\n\nLocal-Only: true");
    git(&fx, &["checkout", "-q", "work"]);
    edit_line(&fx, 25, "twenty-five", "C");
    git(&fx, &["merge", "-q", "--no-ff", "-m", "merge local\n\nLocal-Only: true", "side"]);
    let err = plan_deliver(&fx.wc_str(), &base(&fx), &head(&fx)).unwrap_err();
    assert_eq!(err.code, "sync_out_local_only_merge", "{}", err.message);
}

#[test]
fn when_everything_is_local_only_the_baseline_itself_is_the_delivery() {
    let fx = hybrid_with_colleague();
    edit_line_on_fresh_file(&fx);
    let p = plan_deliver(&fx.wc_str(), &base(&fx), &head(&fx)).unwrap();
    assert_eq!(p.deliver_oid, base(&fx), "D1 会是空清单，由前置检查报「没有可交付的改动」");
    assert_eq!(p.local_only.len(), 1);
}

fn edit_line_on_fresh_file(fx: &SvnFixture) {
    std::fs::write(fx.wc().join("Code/A.cs"), "a1 LOCAL\n").unwrap();
    git(fx, &["commit", "-qam", "L\n\nLocal-Only: true"]);
}

#[test]
fn the_deliver_ref_never_collides_with_a_user_branch_named_deliver() {
    // 必测 #25：放进 refs/mygitclient/，用户叫 deliver 的分支照常存在
    let fx = hybrid_with_colleague();
    git(&fx, &["branch", "deliver"]);
    let r = pin(&fx.wc_str(), "op-1", &head(&fx)).unwrap();
    assert_eq!(r, deliver_ref("op-1"));
    assert_eq!(r, "refs/mygitclient/deliver/op-1");
    assert_eq!(git(&fx, &["rev-parse", "deliver"]), head(&fx));
}

#[test]
fn pin_refuses_to_overwrite_and_unpin_is_idempotent_and_fenced() {
    let fx = hybrid_with_colleague();
    let r = pin(&fx.wc_str(), "op-2", &head(&fx)).unwrap();
    assert!(pin(&fx.wc_str(), "op-2", &base(&fx)).is_err(), "同名残留不能被覆盖");
    unpin(&fx.wc_str(), &r).unwrap();
    unpin(&fx.wc_str(), &r).unwrap(); // §8.2.4：崩溃重开会重跑收尾，第二次不算失败
    for bad in ["refs/heads/work", "refs/mygitclient/deliver/", "refs/mygitclient/sync/x"] {
        let err = unpin(&fx.wc_str(), bad).unwrap_err();
        assert_eq!(err.code, "sync_out_bad_deliver_ref", "{bad}：{}", err.message);
    }
    assert_eq!(git(&fx, &["rev-parse", "work"]), head(&fx), "别的 ref 一律不碰");
}
```

- [ ] **Step 2: 跑测试确认失败**

Run：`cargo test --test svn_deliver`
Expected: 编译失败——`app_lib::engine::svn_deliver` 不存在。

- [ ] **Step 3: 实现 `svn_deliver.rs`**

```rust
//! D0.5：从 HEAD 摘掉 Local-Only commit，构造交付树（spec §8.2.3；计划 B2）。
//!
//! # 为什么在对象层重放，而不是临时 worktree + cherry-pick
//!
//! spec 的两条硬约束是「D0.5 不动主工作副本」与「D0.5 之后的早退必须清干净」。
//! `git merge-tree --write-tree --merge-base=<C^> <cur> <C>` 与 cherry-pick 用同一套
//! 合并引擎（ort），但只产出对象：磁盘、索引、HEAD、worktree 注册一样都不碰，
//! 于是两条约束天然成立——没有东西需要清理。计划开工前实测 M1：不重叠时两种做法
//! 得到的 tree 相同；重叠时 merge-tree 退出 1，并列出冲突文件的三个 stage。
//!
//! 这也是预览能把交付树算给用户看的前提（计划 B1）：预览不拿锁、不写 journal，
//! 所以它不能留下任何需要事后清理的东西。
//!
//! 代价：`--merge-base` 要 git ≥ 2.40。只在慢路径上用到，旧 git 报 `sync_out_git_too_old`。

use crate::engine::git_cli::{run_git, run_git_code, run_git_env};
use crate::engine::graph::parse_local_only;
use crate::error::{GitError, Result};
use std::collections::HashMap;

/// 交付树 ref 的前缀。不在 `refs/heads/` 下，与用户分支不可能撞名（spec §8.2.3、必测 #25）。
pub const DELIVER_REF_PREFIX: &str = "refs/mygitclient/deliver/";

/// 被摘掉的本地补丁。确认框必须列出它们（spec §8.2.3「漏标是主要人为风险」）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalOnlyCommit {
    pub oid: String,
    pub subject: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliverPlan {
    /// 交付树所在的 commit。快路径是一个已有 commit，慢路径是重放出来的新 commit
    /// （只是对象，没有任何 ref 指着它，直到执行时 [`pin`]）。
    pub deliver_oid: String,
    pub deliver_tree: String,
    pub local_only: Vec<LocalOnlyCommit>,
    /// 全部 Local-Only commit 构成分支顶端连续的单父后缀。
    pub fast: bool,
}

struct Commit {
    oid: String,
    parents: Vec<String>,
    subject: String,
    local_only: bool,
}

pub fn deliver_ref(opid: &str) -> String {
    format!("{DELIVER_REF_PREFIX}{opid}")
}

/// 算出交付树。**不写任何 ref、不动磁盘**——预览与执行都调它。
///
/// 包含 §8.1 第 6、7 条：慢路径上历史里不许有 merge；快路径的 Local-Only 后缀必须单父。
pub fn plan_deliver(repo: &str, base: &str, head: &str) -> Result<DeliverPlan> {
    for oid in [base, head] {
        // 空 OID 会让区间退化成 `..head`（= `HEAD..head`），查的就不是那一段了
        if oid.len() != 40 || !oid.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(GitError::new(
                "sync_out_replay_failed",
                format!("构造交付树拿到的不是一个完整 OID：`{oid}`"),
            ));
        }
    }
    let list = commits_between(repo, base, head)?;
    let by_oid: HashMap<&str, &Commit> = list.iter().map(|c| (c.oid.as_str(), c)).collect();
    let local_only: Vec<LocalOnlyCommit> = list
        .iter()
        .filter(|c| c.local_only)
        .map(|c| LocalOnlyCommit {
            oid: c.oid.clone(),
            subject: c.subject.clone(),
        })
        .collect();

    // 沿第一父从 HEAD 往回走，收集顶端连续的 Local-Only 后缀
    let mut cut = head.to_string();
    let mut suffix: Vec<&str> = Vec::new();
    while let Some(c) = by_oid.get(cut.as_str()) {
        if !c.local_only {
            break;
        }
        if c.parents.len() != 1 {
            return Err(local_only_merge(c)); // §8.1 第 7 条
        }
        suffix.push(c.oid.as_str());
        cut = c.parents[0].clone();
    }
    let fast = list
        .iter()
        .all(|c| !c.local_only || suffix.contains(&c.oid.as_str()));

    let deliver_oid = if fast {
        cut // 后缀之前的那个已有 commit（全是补丁时就是 base）
    } else {
        // §8.1 第 6 条：cherry-pick 语义下 merge 没有安全的自动解（spec §8.2.3 T4 / T5）
        if let Some(m) = list.iter().find(|c| c.parents.len() > 1) {
            return Err(merge_in_slow_path(m));
        }
        let picks: Vec<&Commit> = list.iter().filter(|c| !c.local_only).collect();
        let locals: Vec<&Commit> = list.iter().filter(|c| c.local_only).collect();
        replay(repo, base, &picks, &locals)?
    };
    let deliver_tree = run_git(repo, &["rev-parse", &format!("{deliver_oid}^{{tree}}")])?
        .trim()
        .to_string();
    Ok(DeliverPlan {
        deliver_oid,
        deliver_tree,
        local_only,
        fast,
    })
}

/// `base..head`，从旧到新。`%B` 是完整 message，Local-Only 判定与图上的角标同一份实现。
fn commits_between(repo: &str, base: &str, head: &str) -> Result<Vec<Commit>> {
    let out = run_git(
        repo,
        &[
            "log",
            "--reverse",
            "--topo-order",
            "--format=%H%x00%P%x00%s%x00%B%x1e",
            &format!("{base}..{head}"),
        ],
    )?;
    let mut list = Vec::new();
    for rec in out.split('\u{1e}') {
        let rec = rec.trim_start_matches('\n');
        if rec.is_empty() {
            continue;
        }
        let mut f = rec.splitn(4, '\0');
        let (Some(oid), Some(parents), Some(subject), Some(body)) =
            (f.next(), f.next(), f.next(), f.next())
        else {
            return Err(GitError::new(
                "sync_out_replay_failed",
                format!("读不懂 git log 的输出：{rec:?}"),
            ));
        };
        list.push(Commit {
            oid: oid.to_string(),
            parents: parents.split_whitespace().map(str::to_string).collect(),
            subject: subject.to_string(),
            local_only: parse_local_only(body),
        });
    }
    Ok(list)
}

/// 慢路径：在对象层逐个重放待交付 commit（计划 B2）。
fn replay(repo: &str, onto: &str, picks: &[&Commit], locals: &[&Commit]) -> Result<String> {
    let mut cur = onto.to_string();
    for c in picks {
        let parent = c.parents.first().ok_or_else(|| {
            GitError::new("sync_out_replay_failed", format!("{} 没有父提交", c.oid))
        })?;
        let mb = format!("--merge-base={parent}");
        let (code, out) = run_git_code(repo, &["merge-tree", "--write-tree", &mb, &cur, &c.oid])?;
        match code {
            0 => {}
            1 => return Err(replay_conflict(repo, c, locals, &out)?),
            // 129 = 用法错误：多半是 git 太旧，不认识 --merge-base
            129 => {
                return Err(GitError::new(
                    "sync_out_git_too_old",
                    "本地补丁夹在待交付的提交中间，摘掉它们需要 git 2.40 或更新的版本\
                     （`git merge-tree --merge-base`）。请升级 git，或把本地补丁移到分支顶端\
                     （那样不需要重放）。",
                ))
            }
            _ => {
                return Err(GitError::new(
                    "sync_out_replay_failed",
                    format!("重放 {} 时 git merge-tree 退出 {code}：{}", short(&c.oid), out.trim()),
                ))
            }
        }
        let tree = out.lines().next().unwrap_or("").trim().to_string();
        cur = commit_like(repo, &tree, &cur, &c.oid)?;
    }
    Ok(cur)
}

/// 冲突：指出是哪个待交付 commit、哪些文件、可能是哪几个本地补丁（spec §13「D0.5 cherry-pick 冲突」）。
fn replay_conflict(repo: &str, c: &Commit, locals: &[&Commit], out: &str) -> Result<GitError> {
    // 实测 M1：首行是 tree，随后每行 `<mode> <oid> <stage>\t<path>`，空行之后是说明文字
    let mut files: Vec<String> = out
        .lines()
        .skip(1)
        .take_while(|l| !l.trim().is_empty())
        .filter_map(|l| l.split_once('\t').map(|(_, p)| p.to_string()))
        .collect();
    files.sort();
    files.dedup();
    let mut blamed = Vec::new();
    for l in locals {
        let touched = run_git(repo, &["diff-tree", "--no-commit-id", "--name-only", "-r", "-z", &l.oid])?;
        if touched.split('\0').any(|p| files.iter().any(|f| f == p)) {
            blamed.push(format!("  - {} {}", short(&l.oid), l.subject));
        }
    }
    Ok(GitError::new(
        "sync_out_local_only_conflict",
        format!(
            "摘掉本地补丁之后，待交付的提交 {} {} 放不上去——它和下面的本地补丁改到了同一处：\n{}\n\
             冲突的文件：\n{}\n\
             什么都没有改动，也没有碰 SVN。请把这个补丁移到分支顶端，或者把相邻的改动拆开，再交付。",
            short(&c.oid),
            c.subject,
            if blamed.is_empty() { "  （没认出具体是哪个）".to_string() } else { blamed.join("\n") },
            files.iter().map(|f| format!("  - {f}")).collect::<Vec<_>>().join("\n"),
        ),
    ))
}

/// 用 `like` 的作者与 message 在 `parent` 上创建 `tree` 的 commit（计划 B15）。
fn commit_like(repo: &str, tree: &str, parent: &str, like: &str) -> Result<String> {
    let meta = run_git(repo, &["log", "-1", "--date=raw", "--format=%an%x00%ae%x00%ad%x00%B", like])?;
    let mut f = meta.splitn(4, '\0');
    let (name, email, date, msg) = (
        f.next().unwrap_or(""),
        f.next().unwrap_or(""),
        f.next().unwrap_or(""),
        f.next().unwrap_or("").trim_end_matches('\n'),
    );
    // message 走 -m：run_git* 的子进程 stdin 是 null，走 stdin 会产出空 message
    // （svn_baseline::create_baseline_commit 记过这个坑）
    Ok(run_git_env(
        repo,
        &["commit-tree", tree, "-p", parent, "-m", msg],
        &[
            ("GIT_AUTHOR_NAME", name),
            ("GIT_AUTHOR_EMAIL", email),
            ("GIT_AUTHOR_DATE", date),
        ],
    )?
    .trim()
    .to_string())
}

/// 给交付树起名字。**旧值写空串**：ref 必须还不存在，撞上残留就报错而不是覆盖。
pub fn pin(repo: &str, opid: &str, oid: &str) -> Result<String> {
    let r = deliver_ref(opid);
    run_git(repo, &["update-ref", &r, oid, ""])?;
    Ok(r)
}

/// 删交付 ref。幂等（spec §8.2.4：崩溃恢复会重跑收尾），且**只删本前缀下的**（§8.5 末）。
pub fn unpin(repo: &str, r: &str) -> Result<()> {
    let ok = r
        .strip_prefix(DELIVER_REF_PREFIX)
        .is_some_and(|rest| !rest.is_empty() && !rest.contains('/') && !rest.contains(".."));
    if !ok {
        return Err(GitError::new(
            "sync_out_bad_deliver_ref",
            format!("拒绝删除 `{r}`：它不是本应用建的交付 ref（必须在 {DELIVER_REF_PREFIX} 下）。"),
        ));
    }
    let (code, _) = run_git_code(repo, &["rev-parse", "-q", "--verify", r])?;
    if code != 0 {
        return Ok(()); // 已经不在了
    }
    run_git(repo, &["update-ref", "-d", r])?;
    Ok(())
}

fn local_only_merge(c: &Commit) -> GitError {
    GitError::new(
        "sync_out_local_only_merge",
        format!(
            "分支顶端的本地补丁里有一个 merge commit：{} {}\n交付之后这些补丁要重放到新基线上，\
             而 rebase 会压平 merge，只存在于 merge 结果里的改动可能无声消失。\
             请把本地补丁整理成一串普通提交，再交付。什么都没有改动。",
            short(&c.oid),
            c.subject
        ),
    )
}

fn merge_in_slow_path(m: &Commit) -> GitError {
    GitError::new(
        "sync_out_merge_in_slow_path",
        format!(
            "本地补丁夹在待交付的提交中间，摘掉它们要逐个重放待交付的提交，而历史里有一个\
             merge commit：{} {}\n重放没法安全地处理 merge。可以把本地补丁移到分支顶端\
             （那样不需要重放，merge 原样交付），或者保留原分支、人工整理成线性历史。\
             什么都没有改动。",
            short(&m.oid),
            m.subject
        ),
    )
}

fn short(oid: &str) -> &str {
    &oid[..oid.len().min(10)]
}
```

`engine/mod.rs` 加 `pub mod svn_deliver;`（按字母序放在 `svn_dirty` 之前）。

- [ ] **Step 4: 跑测试确认通过**

Run：`cargo test --test svn_deliver`
Expected: 10 个用例全部 PASS。若 `an_overlapping_local_only_aborts_and_names_both_commits` 的冲突文件列表为空，先打印 `out` 核对 M1 的输出格式（`run_git_code` 只返回 stdout；merge-tree 的冲突信息在 stdout 上——实测如此），把实际格式记进决策日志「计划没写的坑」。

- [ ] **Step 5: 全量 Rust 验证并 Commit**

```bash
cargo fmt --check && cargo clippy --all-targets && cargo test
git add src-tauri/src/engine/svn_deliver.rs src-tauri/src/engine/mod.rs src-tauri/tests/svn_deliver.rs
git commit -m "feat(svn): 交付树在对象层摘掉本地补丁，不碰工作副本"
```

更新决策日志：B2 / B15 的实施与 M1 的复核结果。

---

## Task 3：前置检查与 D1 / D2 清单（`svn_sync_out_check`）

**Files:**
- Create: `src-tauri/src/engine/svn_sync_out_check.rs`
- Modify: `src-tauri/src/engine/svn_sync_check.rs`（`check_git_idle` 加两个参数并改 `pub(crate)`；`precheck_include_set` 改 `pub(crate)`）
- Modify: `src-tauri/src/engine/svn_preflight.rs`（新增 `pub(crate) fn svn_targets`）
- Modify: `src-tauri/src/engine/mod.rs`
- Test: `src-tauri/tests/svn_sync_out_check.rs`

**Interfaces:**
- Consumes: `svn_deliver::{plan_deliver, LocalOnlyCommit}`（Task 2）。
- Produces:
  - `pub struct DeliverChange { pub status: String, pub path: String }`（`status` ∈ `"A" | "M" | "D"`；serde camelCase，`Deserialize` 也要：执行时前端把它原样带回来）
  - `pub struct SyncOutPreview { orig_branch, orig_head, baseline_oid, baseline_rev: u64, remote_rev: u64, deliver_oid, deliver_tree, changes: Vec<DeliverChange>, local_only: Vec<LocalOnlyCommit>, outside_changes: Vec<String> }`（serde camelCase）
  - `pub fn preview_sync_out(repo: &str) -> Result<SyncOutPreview>`——查锁、查 journal，再调 `check_sync_out`
  - `pub(crate) fn check_sync_out(repo: &str) -> Result<SyncOutPreview>`——**不查锁**（持锁者在锁内复核时调它）
  - `pub(crate) fn svn_targets(repo: &str, whitelist: &[String]) -> Result<Vec<String>>`（在 `svn_preflight`）：白名单根 + 其中的嵌套工作副本根，与 `read_node_rev` 用同一套
  - `svn_sync_check::check_git_idle(repo, code: &str, verb: &str)` 新签名；`sync_in` 的调用改成 `check_git_idle(repo_root, "sync_in_git_busy", "更新")`

- [ ] **Step 1: 小重构，让 `sync_in` 的辅助函数可复用（行为不变）**

`svn_sync_check.rs`：

```rust
/// git 不在任何未完成的操作里，索引也没有未合并条目。
///
/// `code` / `verb` 由调用方给：同一个判断，「更新」与「交付」各报各的码与文案。
pub(crate) fn check_git_idle(repo_root: &str, code: &str, verb: &str) -> Result<()> {
    let dir = git_dir(repo_root)?;
    let busy = [
        ("rebase-merge", "rebase"),
        ("rebase-apply", "rebase"),
        ("MERGE_HEAD", "merge"),
        ("CHERRY_PICK_HEAD", "cherry-pick"),
        ("REVERT_HEAD", "revert"),
    ];
    for (marker, what) in busy {
        if dir.join(marker).exists() {
            return Err(GitError::new(
                code,
                format!("git 正处在一次没做完的 {what} 里。请先完成或中止它，再{verb}。"),
            ));
        }
    }
    if !unmerged_paths(repo_root)?.is_empty() {
        return Err(GitError::new(
            code,
            format!("暂存区里有未解决的冲突。请先解决（或 `git restore --staged`）再{verb}。"),
        ));
    }
    Ok(())
}
```

`check_sync_in` 里那一行改成 `check_git_idle(repo_root, "sync_in_git_busy", "更新")?; // 第 1 条`。`precheck_include_set` 前面加 `pub(crate)`。

`svn_preflight.rs`，在 `read_node_rev` 之后加：

```rust
/// 本流程对 svn 发命令时用的 target：白名单根，加上其中的嵌套工作副本根。
///
/// 与 [`read_node_rev`] 同一套（external 里的代码要显式指到，spec §6.5.3 / §6.6.5）。
/// 空白名单直接报错——空 target 会变成「整个工作副本」（spec §12.2）。
pub(crate) fn svn_targets(repo_root: &str, whitelist: &[String]) -> Result<Vec<String>> {
    let roots = whitelist_roots(whitelist);
    if roots.is_empty() {
        return Err(GitError::new("svn_empty_whitelist", "白名单是空的，拒绝对整个工作副本发命令。"));
    }
    let wcs = enumerate_wcs(repo_root, whitelist)?;
    Ok(check_targets(&roots, &wcs))
}
```

Run：`cargo test --test svn_sync_check --test svn_sync_in`。Expected：全部 PASS（纯搬家，行为零变化）。

- [ ] **Step 2: 写失败测试**

`src-tauri/tests/svn_sync_out_check.rs`：

```rust
//! `sync_out` 的前置检查（spec §8.1）与 D1 / D2 清单。全部在临时 fixture 上跑。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::git_cli::run_git;
use app_lib::engine::svn_journal::{self, Flow, Journal, Phase};
use app_lib::engine::svn_lock;
use app_lib::engine::svn_sync_check::preview_sync_in;
use app_lib::engine::svn_sync_in::{run_sync_in, SyncInExpect};
use app_lib::engine::svn_sync_out_check::preview_sync_out;
use std::process::Command;
use svn_fixtures::{hybrid_with_colleague, SvnFixture};

fn git(fx: &SvnFixture, args: &[&str]) -> String {
    run_git(&fx.wc_str(), args).unwrap().trim().to_string()
}

fn svn(dir: &std::path::Path, args: &[&str]) -> String {
    let out = Command::new("svn").current_dir(dir).args(args).output().unwrap();
    assert!(out.status.success(), "svn {args:?}：{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// 用 Phase 2c 的「更新」把基线推进到服务器最新。
fn update(fx: &SvnFixture) {
    let p = preview_sync_in(&fx.wc_str()).unwrap();
    run_sync_in(
        &fx.wc_str(),
        &SyncInExpect {
            orig_branch: p.orig_branch,
            orig_head: p.orig_head,
            baseline_oid: p.baseline_oid,
            confirmed_outside: p.outside_changes,
        },
    )
    .unwrap();
}

fn write(fx: &SvnFixture, rel: &str, content: &str) {
    let p = fx.wc().join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

#[test]
fn the_preview_lists_a_m_d_and_the_local_only_patches() {
    let fx = hybrid_with_colleague();
    write(&fx, "Code/A.cs", "a2\n");
    std::fs::remove_file(fx.wc().join("Code/B.cs")).unwrap();
    write(&fx, "Code/Deep/Nested/New.cs", "new\n");
    git(&fx, &["add", "-A", "Code"]);
    git(&fx, &["commit", "-qm", "work"]);
    write(&fx, "Code/A.cs", "a2 DEBUG\n");
    git(&fx, &["commit", "-qam", "debug\n\nLocal-Only: true"]);

    let p = preview_sync_out(&fx.wc_str()).unwrap();
    let got: Vec<(String, String)> =
        p.changes.iter().map(|c| (c.status.clone(), c.path.clone())).collect();
    assert_eq!(
        got,
        vec![
            ("M".to_string(), "Code/A.cs".to_string()),
            ("D".to_string(), "Code/B.cs".to_string()),
            ("A".to_string(), "Code/Deep/Nested/New.cs".to_string()),
        ]
    );
    assert_eq!(p.local_only.len(), 1);
    assert_eq!(p.local_only[0].subject, "debug");
    assert_eq!(p.orig_branch, "work");
    assert_eq!(p.remote_rev, p.baseline_rev);
    assert!(p.outside_changes.is_empty());
}

#[test]
fn the_preview_changes_nothing_and_takes_no_lock() {
    let fx = hybrid_with_colleague();
    write(&fx, "Code/A.cs", "a2\n");
    git(&fx, &["commit", "-qam", "work"]);
    let refs = git(&fx, &["for-each-ref"]);
    // A.cs 在 svn 看来是 M（git 提交了、还没交付），所以比的是「预览前后不变」，不是「为空」
    let svn_before = svn(&fx.wc(), &["status", "Code"]);
    preview_sync_out(&fx.wc_str()).unwrap();
    assert_eq!(git(&fx, &["for-each-ref"]), refs);
    assert_eq!(git(&fx, &["status", "--porcelain"]), "");
    assert!(svn_journal::read(&fx.wc_str()).unwrap().is_none());
    assert!(!svn_lock::is_held(&fx.wc_str()));
    assert_eq!(svn(&fx.wc(), &["status", "Code"]), svn_before);
}

#[test]
fn a_branch_still_on_the_old_baseline_is_refused() {
    // 必测 #57（§8.1 第 0 条、§8.1.2）：分支 B 从旧基线分出来，基线已被别的交付 / 更新推进
    let fx = hybrid_with_colleague();
    git(&fx, &["branch", "B"]);
    fx.colleague_commits("Code/ANew.cs", "anew\n");
    update(&fx); // work 与 svn/base 前进到 r2，含 ANew.cs
    git(&fx, &["checkout", "-q", "B"]);
    write(&fx, "Code/BOnly.cs", "b\n");
    git(&fx, &["add", "Code/BOnly.cs"]);
    git(&fx, &["commit", "-qm", "b only"]);

    let err = preview_sync_out(&fx.wc_str()).unwrap_err();
    assert_eq!(err.code, "sync_out_behind_baseline", "{}", err.message);

    // 反向证据：这道门真拦着东西——两点 diff 会把别人刚交付的文件列成删除
    let two_dot = git(&fx, &["diff", "--name-status", "--no-renames", "svn/base", "B"]);
    assert!(two_dot.contains("D\tCode/ANew.cs"), "{two_dot}");
}

#[test]
fn detached_head_and_the_baseline_branch_are_refused() {
    // 必测 #31 的交付一半（§8.1.1）
    let fx = hybrid_with_colleague();
    write(&fx, "Code/A.cs", "a2\n");
    git(&fx, &["commit", "-qam", "work"]);
    git(&fx, &["checkout", "-q", "--detach"]);
    assert_eq!(preview_sync_out(&fx.wc_str()).unwrap_err().code, "sync_out_detached_head");
    git(&fx, &["checkout", "-q", "svn/base"]);
    assert_eq!(preview_sync_out(&fx.wc_str()).unwrap_err().code, "sync_out_on_baseline");
}

#[test]
fn uncommitted_work_inside_the_whitelist_is_refused() {
    // §8.1 第 2 条前半：不自动 stash——不该悄悄排除，也不该悄悄带上
    let fx = hybrid_with_colleague();
    write(&fx, "Code/A.cs", "a2\n");
    git(&fx, &["commit", "-qam", "work"]);
    write(&fx, "Code/Untracked.cs", "u\n");
    let err = preview_sync_out(&fx.wc_str()).unwrap_err();
    assert_eq!(err.code, "sync_out_dirty", "{}", err.message);
    assert!(err.message.contains("Code/Untracked.cs"), "{}", err.message);
}

#[test]
fn an_unfinished_git_operation_is_refused() {
    let fx = hybrid_with_colleague();
    write(&fx, "Code/A.cs", "a2\n");
    git(&fx, &["commit", "-qam", "work"]);
    git(&fx, &["revert", "--no-commit", "HEAD"]); // 留下 REVERT_HEAD
    let err = preview_sync_out(&fx.wc_str()).unwrap_err();
    assert_eq!(err.code, "sync_out_git_busy", "{}", err.message);
    assert!(err.message.contains("交付"), "{}", err.message);
}

#[test]
fn a_newer_revision_on_the_server_is_refused_even_outside_the_whitelist() {
    // §8.1 第 5 条：比的是 baselineRev。同事只提交了美术也一样要先更新
    let fx = hybrid_with_colleague();
    write(&fx, "Code/A.cs", "a2\n");
    git(&fx, &["commit", "-qam", "work"]);
    fx.colleague_commits("Art/x.psd", "psd2\n");
    let err = preview_sync_out(&fx.wc_str()).unwrap_err();
    assert_eq!(err.code, "sync_out_server_ahead", "{}", err.message);
}

#[test]
fn files_git_does_not_manage_inside_the_whitelist_are_listed_for_confirmation() {
    // §8.1 第 2 条后半 / 必测 #65 前半：Code/x.psd 在白名单目录里、被扩展名排除
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/x.psd", "psd1\n");
    update(&fx);
    write(&fx, "Code/x.psd", "psd LOCAL\n");
    write(&fx, "Code/A.cs", "a2\n");
    git(&fx, &["commit", "-qam", "work"]);
    let p = preview_sync_out(&fx.wc_str()).unwrap();
    assert_eq!(p.outside_changes, vec!["Code/x.psd".to_string()]);
}

#[test]
fn nothing_to_deliver_says_how_many_local_patches_were_set_aside() {
    // §8.3 + spec §13「D1 / D2 空清单」：提示是否全部改动都被标成了 Local-Only
    let fx = hybrid_with_colleague();
    write(&fx, "Code/A.cs", "a1 DEBUG\n");
    git(&fx, &["commit", "-qam", "debug\n\nLocal-Only: true"]);
    let err = preview_sync_out(&fx.wc_str()).unwrap_err();
    assert_eq!(err.code, "sync_out_nothing_to_deliver", "{}", err.message);
    assert!(err.message.contains("1 个本地补丁"), "{}", err.message);
}

#[test]
fn a_change_outside_the_whitelist_is_refused_not_filtered() {
    // 计划 B9：被 `git add -f` 加进来的白名单外文件
    let fx = hybrid_with_colleague();
    write(&fx, "Art/x.psd", "psd LOCAL\n");
    git(&fx, &["add", "-f", "Art/x.psd"]);
    write(&fx, "Code/A.cs", "a2\n");
    git(&fx, &["commit", "-qam", "work with art"]);
    let err = preview_sync_out(&fx.wc_str()).unwrap_err();
    assert_eq!(err.code, "sync_out_outside_whitelist", "{}", err.message);
    assert!(err.message.contains("Art/x.psd"), "{}", err.message);
}

#[test]
fn an_unfinished_sync_or_a_held_lock_blocks_the_preview() {
    let fx = hybrid_with_colleague();
    write(&fx, "Code/A.cs", "a2\n");
    git(&fx, &["commit", "-qam", "work"]);
    {
        let _l = svn_lock::acquire(&fx.wc_str(), "other").unwrap();
        assert_eq!(preview_sync_out(&fx.wc_str()).unwrap_err().code, "sync_in_progress");
    }
    svn_journal::write(
        &fx.wc_str(),
        &Journal {
            opid: "left".into(),
            flow: Flow::SyncIn,
            phase: Phase::Rebase,
            started_at: 1,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(preview_sync_out(&fx.wc_str()).unwrap_err().code, "sync_in_unfinished");
}
```

- [ ] **Step 3: 跑测试确认失败**

Run：`cargo test --test svn_sync_out_check`
Expected: 编译失败——`svn_sync_out_check` 不存在。

- [ ] **Step 4: 实现 `svn_sync_out_check.rs`**

```rust
//! `sync_out` 的前置检查（spec §8.1 第 0–8 条）与 D1 / D2 清单。**纯读**：不拿锁、
//! 不写 journal、不建 ref、不碰磁盘。确认框里的全部事实都来自这里（计划 B1）。

use crate::engine::git_cli::{run_git, run_git_code};
use crate::engine::svn_baseline::parse_trailer;
use crate::engine::svn_cli::run_svn;
use crate::engine::svn_config::{read_config, SvnConfig, BASELINE_BRANCH};
use crate::engine::svn_deliver::{plan_deliver, LocalOnlyCommit};
use crate::engine::svn_externals::detect_pinned;
use crate::engine::svn_include_check::{is_w_file, normalize_roots};
use crate::engine::svn_journal;
use crate::engine::svn_lock;
use crate::engine::svn_preflight::svn_targets;
use crate::engine::svn_sync::{check_sync, SyncVerdict};
use crate::engine::svn_sync_check::{
    check_git_idle, current_branch, list_paths, precheck_include_set, rev_parse, unfinished_error,
};
use crate::engine::svn_wc::peg_safe;
use crate::engine::svn_xml::{parse_info, parse_update_status};
use crate::error::{GitError, Result};
use crate::vcs::{detect_repo_kind, RepoKind};

/// D1 清单里的一条。`status` 只会是 `A` / `M` / `D`（`--no-renames`，其余状态拒绝）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliverChange {
    pub status: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncOutPreview {
    pub orig_branch: String,
    pub orig_head: String,
    pub baseline_oid: String,
    pub baseline_rev: u64,
    /// 服务器 HEAD revision。执行时这一次查询就是 D3.5（计划 B13）。
    pub remote_rev: u64,
    pub deliver_oid: String,
    pub deliver_tree: String,
    pub changes: Vec<DeliverChange>,
    pub local_only: Vec<LocalOnlyCommit>,
    /// 白名单目录里 git 管不到、有本地修改的文件（§8.1 第 2 条后半）。D6 会走到它们
    /// 头上，所以**非空时必须由用户确认**，确认只授权 update，不授权任何 revert。
    pub outside_changes: Vec<String>,
}

/// 给界面用的入口：锁被拿着、或有未收尾的 journal 时直接拒绝。
pub fn preview_sync_out(repo: &str) -> Result<SyncOutPreview> {
    svn_lock::assert_not_syncing(repo)?;
    if svn_journal::read(repo)?.is_some() {
        return Err(unfinished_error());
    }
    check_sync_out(repo)
}

/// §8.1 第 0–8 条 + D0.5（对象层）+ D1 / D2。**不查锁**：持锁者在锁内复核时调它。
///
/// 顺序：纯本地的先查、便宜的先查，走网络的放最后（§8.1 第 5 条）。
pub(crate) fn check_sync_out(repo: &str) -> Result<SyncOutPreview> {
    if detect_repo_kind(repo) != RepoKind::SvnHybrid {
        return Err(GitError::new("svn_not_hybrid", "这个目录不是已初始化的混合仓库。"));
    }
    let cfg = read_config(repo)?;

    check_git_idle(repo, "sync_out_git_busy", "交付")?; // 第 3 条
    let orig_branch = attached_branch(repo)?; // 第 4 条
    let orig_head = rev_parse(repo, "HEAD")?;
    let baseline_oid = rev_parse(repo, &format!("refs/heads/{BASELINE_BRANCH}"))?;
    require_on_latest_baseline(repo, &baseline_oid, &orig_head, &orig_branch)?; // 第 0 条

    if !detect_pinned(repo, &cfg.whitelist)?.is_empty() {
        return Err(GitError::new(
            "sync_out_pinned_unsupported",
            "白名单所在的 external 被 `-r` 钉住了（pinned 模式）。本版本的「更新」还不支持\
             这种形态，交付之后基线也就跟不上，所以不在这里冒险。请先用 TortoiseSVN 交付。",
        ));
    }

    let status = check_sync(repo)?; // 第 1 条
    if !matches!(status.verdict, SyncVerdict::InSync) {
        return Err(GitError::new(
            "sync_out_out_of_sync",
            format!(
                "基线已经失同步（白名单 revision {:?}，基线记的是 r{}）。在失同步的基线上算出来的\
                 「改了什么」不可信，请先处理顶部的失同步横幅（多半点「更新」即可）。",
                status.node_revs, status.baseline_rev
            ),
        ));
    }

    let dirty = git_dirty(repo, &cfg)?; // 第 2 条前半
    if !dirty.is_empty() {
        return Err(GitError::new(
            "sync_out_dirty",
            format!(
                "白名单里还有没提交的改动：\n{}\n交付的是提交过的内容，这些改动不会被悄悄带上，\
                 也不会被悄悄排除。请先提交（不打算交付的就勾「仅本地，不交付」提交），或者撤销。",
                list_paths(&dirty)
            ),
        ));
    }

    // 第 6、7 条与 D0.5：对象层，不碰磁盘（计划 B2）
    let plan = plan_deliver(repo, &baseline_oid, &orig_head)?;
    let changes = changed_files(repo, &cfg, &baseline_oid, &plan.deliver_oid)?; // D1 / D2
    if changes.is_empty() {
        return Err(nothing_to_deliver(plan.local_only.len()));
    }

    precheck_include_set(repo, &cfg.whitelist, &cfg.extensions)?; // 第 8 条

    let outside_changes = outside_changes(repo, &cfg)?; // 第 2 条后半

    // 第 5 条：走网络，放最后
    let root = repos_root(repo, &baseline_oid)?;
    let remote_rev = remote_head_rev(repo, &root)?;
    if remote_rev != status.baseline_rev {
        return Err(GitError::new(
            "sync_out_server_ahead",
            format!(
                "服务器上已经有 r{remote_rev}，你的基线还在 r{}。请先「更新」，再交付——\
                 在旧 revision 上交付，SVN 也会拒绝有冲突的文件。什么都没有改动。",
                status.baseline_rev
            ),
        ));
    }

    Ok(SyncOutPreview {
        orig_branch,
        orig_head,
        baseline_oid,
        baseline_rev: status.baseline_rev,
        remote_rev,
        deliver_oid: plan.deliver_oid,
        deliver_tree: plan.deliver_tree,
        changes,
        local_only: plan.local_only,
        outside_changes,
    })
}

fn attached_branch(repo: &str) -> Result<String> {
    let Some(branch) = current_branch(repo)? else {
        let at = rev_parse(repo, "HEAD").unwrap_or_default();
        return Err(GitError::new(
            "sync_out_detached_head",
            format!(
                "HEAD 没有附着在分支上（停在 {}）。交付之后要把一个分支接到新基线上，\
                 detached HEAD 没有分支可接。请 `git switch -c <新分支名>` 或切回原来的分支。",
                &at[..at.len().min(10)]
            ),
        ));
    };
    if branch == BASELINE_BRANCH {
        return Err(GitError::new(
            "sync_out_on_baseline",
            format!("当前在基线分支 `{BASELINE_BRANCH}` 上。请切回你的工作分支再交付。"),
        ));
    }
    Ok(branch)
}

/// §8.1 第 0 条：唯一挡得住「拿旧基线的分支交付」的一条（spec §8.1.2）。
fn require_on_latest_baseline(repo: &str, base: &str, head: &str, branch: &str) -> Result<()> {
    let (code, _) = run_git_code(repo, &["merge-base", "--is-ancestor", base, head])?;
    match code {
        0 => Ok(()),
        1 => Err(GitError::new(
            "sync_out_behind_baseline",
            format!(
                "分支 `{branch}` 还停在旧基线上，没有包含最新的 `{BASELINE_BRANCH}`。\
                 强行交付会把别人（或你自己另一个分支）上一次交付的文件当成删除提上去。\
                 请先对这个分支点一次「更新」，再交付。什么都没有改动。"
            ),
        )),
        _ => Err(GitError::new(
            "sync_out_behind_baseline",
            format!("查不出分支 `{branch}` 是否包含最新基线（git 退出 {code}）。查不成不等于没问题，这里停下。"),
        )),
    }
}

/// 白名单内 git 的未提交改动，含未跟踪文件。
fn git_dirty(repo: &str, cfg: &SvnConfig) -> Result<Vec<String>> {
    let roots = normalize_roots(&cfg.whitelist);
    if roots.is_empty() {
        return Err(GitError::new("svn_empty_whitelist", "白名单是空的。")); // §12.2
    }
    let mut args = vec!["status", "--porcelain", "-z", "--untracked-files=all", "--"];
    args.extend(roots.iter().map(String::as_str));
    let out = run_git(repo, &args)?;
    let mut paths = Vec::new();
    let mut it = out.split('\0').filter(|s| !s.is_empty());
    while let Some(entry) = it.next() {
        if entry.len() > 3 {
            paths.push(entry[3..].to_string());
        }
        // `-z` 下改名 / 复制的条目后面紧跟一个原路径字段
        if entry.starts_with('R') || entry.starts_with('C') {
            it.next();
        }
    }
    Ok(paths)
}

/// D1 + D2：`git diff --name-status --no-renames base deliver`，再核对白名单（计划 B9）。
pub(crate) fn changed_files(
    repo: &str,
    cfg: &SvnConfig,
    base: &str,
    deliver: &str,
) -> Result<Vec<DeliverChange>> {
    let out = run_git(repo, &["diff", "--name-status", "--no-renames", "-z", base, deliver])?;
    let mut it = out.split('\0').filter(|s| !s.is_empty());
    let mut changes = Vec::new();
    let mut odd = Vec::new();
    while let Some(st) = it.next() {
        let Some(path) = it.next() else { break };
        match st {
            "A" | "M" | "D" => changes.push(DeliverChange {
                status: st.to_string(),
                path: path.to_string(),
            }),
            other => odd.push(format!("{other} {path}")),
        }
    }
    if !odd.is_empty() {
        return Err(GitError::new(
            "sync_out_unsupported_change",
            format!(
                "下面这些变化不是普通的新增 / 修改 / 删除（多半是文件和符号链接互换）：\n{}\n\
                 SVN 侧没有对应的操作，请用 TortoiseSVN 处理它们。",
                list_paths(&odd)
            ),
        ));
    }
    let roots = normalize_roots(&cfg.whitelist);
    let outside: Vec<String> = changes
        .iter()
        .filter(|c| !is_w_file(&c.path, &roots, &cfg.extensions))
        .map(|c| c.path.clone())
        .collect();
    if !outside.is_empty() {
        return Err(GitError::new(
            "sync_out_outside_whitelist",
            format!(
                "下面这些改动不在白名单里，本应用不替你交付它们：\n{}\n它们多半是用 `git add -f` \
                 加进来的。交付之后分支会接到服务器的新内容上，这些改动会从分支上消失——所以这里\
                 停下，不悄悄跳过。请先把它们移出这个分支的提交，或用 TortoiseSVN 提交。",
                list_paths(&outside)
            ),
        ));
    }
    Ok(changes)
}

/// 白名单目录里 git 管不到、有本地修改的文件（W-path 减 W-file，§6.3.2）。有 svn 冲突态就拒绝。
fn outside_changes(repo: &str, cfg: &SvnConfig) -> Result<Vec<String>> {
    let roots = normalize_roots(&cfg.whitelist);
    let mut out = Vec::new();
    let mut conflicted = Vec::new();
    for t in svn_targets(repo, &cfg.whitelist)? {
        let xml = run_svn(
            repo,
            &["status", "--xml", "--ignore-externals", "--non-interactive", "--", &peg_safe(&t)],
        )?;
        for e in parse_update_status(&xml)?.entries {
            if e.is_conflicted() {
                conflicted.push(e.path.clone());
                continue;
            }
            if is_w_file(&e.path, &roots, &cfg.extensions) {
                continue; // git 管着，上面的 git_dirty 已经要求它干净
            }
            let local = matches!(e.item.as_str(), "modified" | "missing" | "replaced")
                || e.props == "modified";
            if local {
                out.push(e.path.clone());
            }
        }
    }
    if !conflicted.is_empty() {
        return Err(GitError::new(
            "sync_out_svn_conflicted",
            format!(
                "白名单里有 SVN 冲突还没处理：\n{}\n请先用 TortoiseSVN 解决（Resolve）后再交付。",
                list_paths(&conflicted)
            ),
        ));
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// 基线 trailer 里的 Repository Root（spec §6.4.1：trailer 的 `<url>` 就是它）。
fn repos_root(repo: &str, baseline: &str) -> Result<String> {
    let msg = run_git(repo, &["log", "-1", "--format=%B", baseline])?;
    parse_trailer(&msg)
        .map(|t| t.url)
        .ok_or_else(|| GitError::new("svn_baseline_trailer", "基线 commit 上读不到 git-svn-id。"))
}

/// 服务器 HEAD revision。`<root>@HEAD` 的 peg 写法顺带避开 URL 里含 `@` 的歧义。
pub(crate) fn remote_head_rev(repo: &str, repos_root: &str) -> Result<u64> {
    let target = format!("{repos_root}@HEAD");
    let xml = run_svn(repo, &["info", "--xml", "--non-interactive", &target]).map_err(|e| {
        GitError::new(
            "sync_out_server_unreachable",
            format!("问服务器当前 revision 失败了，什么都没有改动。svn 原话：{}", e.message.trim()),
        )
    })?;
    let e = parse_info(&xml)?
        .into_iter()
        .next()
        .ok_or_else(|| GitError::new("svn_xml", "svn info 没有返回任何 entry"))?;
    u64::try_from(e.revision)
        .map_err(|_| GitError::new("svn_xml", format!("svn info 给了一个负的 revision：{}", e.revision)))
}

fn nothing_to_deliver(local_only: usize) -> GitError {
    let tail = if local_only > 0 {
        format!("这个分支上有 {local_only} 个本地补丁（标了「仅本地，不交付」），它们被排除了——如果其中有本该交付的，去掉那个标记后再来。")
    } else {
        "分支和基线的内容完全一样。".to_string()
    };
    GitError::new(
        "sync_out_nothing_to_deliver",
        format!("没有可交付的改动。{tail}"),
    )
}
```

`engine/mod.rs` 加 `pub mod svn_sync_out_check;`。`svn_sync_check.rs` 的 `list_paths`、`current_branch`、`rev_parse`、`unfinished_error` 已是 `pub(crate)`；若 `SyncVerdict` 未实现 `matches!` 所需的可见性（它是 `pub enum`），无需改动。

- [ ] **Step 5: 跑测试确认通过**

Run：`cargo test --test svn_sync_out_check`
Expected: 11 个用例全部 PASS。`nothing_to_deliver…` 断言的文字是「1 个本地补丁」，与 `nothing_to_deliver` 的格式串一致。

- [ ] **Step 6: 全量 Rust 验证并 Commit**

```bash
cargo fmt --check && cargo clippy --all-targets && cargo test
git add src-tauri/src/engine/svn_sync_out_check.rs src-tauri/src/engine/svn_sync_check.rs src-tauri/src/engine/svn_preflight.rs src-tauri/src/engine/mod.rs src-tauri/tests/svn_sync_out_check.rs
git commit -m "feat(svn): 交付的前置检查与净变更清单（只读）"
```

更新决策日志：B9 / B12 / B13；pinned 的拒绝没有专门测试（与 `sync_in` 共用 `detect_pinned`，2c 已覆盖判定本身）——记为已知覆盖缺口。

---

## Task 4：D4 登记与 §8.5 精确撤销（`svn_stage`）

**Files:**
- Create: `src-tauri/src/engine/svn_stage.rs`
- Modify: `src-tauri/src/engine/mod.rs`
- Test: `src-tauri/tests/svn_stage.rs`

**Interfaces:**
- Consumes: `svn_journal::SvnNode`（Task 1）、`svn_sync_out_check::DeliverChange`（Task 3）。
- Produces:
  - `pub struct StageRecord { pub delta: Vec<SvnNode>, pub new_parent_dirs: Vec<String> }`
  - `pub fn snapshot(repo: &str, targets: &[String]) -> Result<Vec<SvnNode>>`——白名单 target 下所有非干净节点，按路径排序
  - `pub fn stage(repo: &str, targets: &[String], changes: &[DeliverChange], before: &[SvnNode]) -> Result<StageRecord>`
  - `pub fn unstage(repo: &str, targets: &[String], before: &[SvnNode]) -> Result<()>`——撤到 `before` 为止，撤完逐条比对
  - `pub fn svn_revert(repo: &str, paths: &[String]) -> Result<()>`——空列表拒绝（必测 #2）

**本 Task 对 spec 的一处补足（记进决策日志）：** `newParentDirs` = 「A 文件的各级祖先目录里，D4 之后状态为 `added` 的那些」，**不只是本次差量里的**。用户事先在 TortoiseSVN 里 `svn add` 过同一个新目录时，它在 D4 之前就已是 `A`，不在差量里；只按差量算，D5 会被 `E200009` 拒（「父目录不在提交里」）。撤销仍然只撤差量——用户自己的登记不碰（§12.4）。

- [ ] **Step 1: 写失败测试**

`src-tauri/tests/svn_stage.rs`：

```rust
//! D4 登记与 §8.5 精确撤销。在 wc1 上直接摆出「D3.8 之后」的磁盘：
//! A.cs 改了、B.cs 被 git 删掉、新文件落在两层新目录里。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::svn_journal::SvnNode;
use app_lib::engine::svn_stage::{snapshot, stage, svn_revert, unstage};
use app_lib::engine::svn_sync_out_check::DeliverChange;
use std::process::Command;
use svn_fixtures::{hybrid_with_colleague, SvnFixture};

fn svn(fx: &SvnFixture, args: &[&str]) -> String {
    let out = Command::new("svn").current_dir(fx.wc()).args(args).output().unwrap();
    assert!(out.status.success(), "svn {args:?}：{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn targets() -> Vec<String> {
    vec!["Code".to_string()]
}

fn change(status: &str, path: &str) -> DeliverChange {
    DeliverChange { status: status.into(), path: path.into() }
}

/// 摆出 D3.8 之后的磁盘，返回对应的 D1 清单。
fn after_checkout(fx: &SvnFixture) -> Vec<DeliverChange> {
    std::fs::write(fx.wc().join("Code/A.cs"), "a2\n").unwrap();
    std::fs::remove_file(fx.wc().join("Code/B.cs")).unwrap();
    std::fs::create_dir_all(fx.wc().join("Code/Deep/Nested")).unwrap();
    std::fs::write(fx.wc().join("Code/Deep/Nested/New.cs"), "new\n").unwrap();
    vec![
        change("M", "Code/A.cs"),
        change("D", "Code/B.cs"),
        change("A", "Code/Deep/Nested/New.cs"),
    ]
}

fn status_of(nodes: &[SvnNode], path: &str) -> Option<String> {
    nodes.iter().find(|n| n.path == path).map(|n| n.status.clone())
}

#[test]
fn staging_registers_adds_and_deletes_and_names_the_new_parent_dirs() {
    let fx = hybrid_with_colleague();
    let changes = after_checkout(&fx);
    let before = snapshot(&fx.wc_str(), &targets()).unwrap();
    assert_eq!(status_of(&before, "Code/B.cs").as_deref(), Some("missing"));
    assert_eq!(status_of(&before, "Code/Deep").as_deref(), Some("unversioned"));

    let rec = stage(&fx.wc_str(), &targets(), &changes, &before).unwrap();
    let after = snapshot(&fx.wc_str(), &targets()).unwrap();
    assert_eq!(status_of(&after, "Code/B.cs").as_deref(), Some("deleted"));
    assert_eq!(status_of(&after, "Code/A.cs").as_deref(), Some("modified"));
    assert_eq!(status_of(&after, "Code/Deep/Nested/New.cs").as_deref(), Some("added"));
    assert_eq!(
        rec.new_parent_dirs,
        vec!["Code/Deep".to_string(), "Code/Deep/Nested".to_string()],
        "父目录必须进 D5 的 target，否则 E200009（spec §8.2.6）"
    );
    assert!(rec.delta.iter().any(|n| n.path == "Code/Deep" && n.status == "added" && n.dir));
    assert!(!rec.delta.iter().any(|n| n.path == "Code/A.cs"), "M 不是本流程新增的登记");
}

#[test]
fn unstaging_returns_svn_to_exactly_the_snapshot() {
    // 必测 #29：父目录登记也要撤干净，比 svn status 而不是 git status（§8.5.1）
    let fx = hybrid_with_colleague();
    let changes = after_checkout(&fx);
    let before = snapshot(&fx.wc_str(), &targets()).unwrap();
    stage(&fx.wc_str(), &targets(), &changes, &before).unwrap();

    unstage(&fx.wc_str(), &targets(), &before).unwrap();
    assert_eq!(snapshot(&fx.wc_str(), &targets()).unwrap(), before);
    assert!(!fx.wc().join("Code/B.cs").exists(), "revert 恢复出来的 B.cs 必须删掉（§8.5 表第二行）");
    assert!(fx.wc().join("Code/Deep/Nested/New.cs").exists(), "A 撤销后文件留在磁盘上");
}

#[test]
fn unstaging_works_when_the_crash_came_before_the_delta_was_written() {
    // 计划 B6：只做了 svn add 就崩了，journal 里只有 svnStatusBefore
    let fx = hybrid_with_colleague();
    after_checkout(&fx);
    let before = snapshot(&fx.wc_str(), &targets()).unwrap();
    svn(&fx, &["add", "--parents", "Code/Deep/Nested/New.cs"]);
    unstage(&fx.wc_str(), &targets(), &before).unwrap();
    assert_eq!(snapshot(&fx.wc_str(), &targets()).unwrap(), before);
}

#[test]
fn a_parent_dir_the_user_had_already_added_still_goes_into_the_commit_but_is_not_undone() {
    // 本 Task 的补足：Code/Deep2 被用户自己 svn add 过（连带一个不交付的 Stray.txt）
    let fx = hybrid_with_colleague();
    std::fs::create_dir_all(fx.wc().join("Code/Deep2/Nested")).unwrap();
    std::fs::write(fx.wc().join("Code/Deep2/Stray.txt"), "stray\n").unwrap();
    svn(&fx, &["add", "--parents", "Code/Deep2/Stray.txt"]);
    std::fs::write(fx.wc().join("Code/Deep2/Nested/New.cs"), "new\n").unwrap();
    let changes = vec![change("A", "Code/Deep2/Nested/New.cs")];
    let before = snapshot(&fx.wc_str(), &targets()).unwrap();

    let rec = stage(&fx.wc_str(), &targets(), &changes, &before).unwrap();
    assert_eq!(rec.new_parent_dirs, vec!["Code/Deep2".to_string(), "Code/Deep2/Nested".to_string()]);

    unstage(&fx.wc_str(), &targets(), &before).unwrap();
    let now = snapshot(&fx.wc_str(), &targets()).unwrap();
    assert_eq!(status_of(&now, "Code/Deep2").as_deref(), Some("added"), "用户自己的登记不碰");
    assert_eq!(status_of(&now, "Code/Deep2/Stray.txt").as_deref(), Some("added"));
    assert_eq!(now, before);
}

#[test]
fn an_empty_change_list_is_refused_before_any_svn_command() {
    // 必测 #1 的 D4 一层（§8.3）
    let fx = hybrid_with_colleague();
    let before = snapshot(&fx.wc_str(), &targets()).unwrap();
    let err = stage(&fx.wc_str(), &targets(), &[], &before).unwrap_err();
    assert_eq!(err.code, "sync_out_empty_targets", "{}", err.message);
    assert_eq!(svn(&fx, &["status", "Code"]), "");
}

#[test]
fn an_empty_revert_is_refused() {
    // 必测 #2：空 target 的 revert 会退化成对当前目录操作
    let fx = hybrid_with_colleague();
    std::fs::write(fx.wc().join("Code/A.cs"), "local\n").unwrap();
    let err = svn_revert(&fx.wc_str(), &[]).unwrap_err();
    assert_eq!(err.code, "sync_out_empty_targets", "{}", err.message);
    assert_eq!(std::fs::read_to_string(fx.wc().join("Code/A.cs")).unwrap(), "local\n");
}

#[test]
fn a_path_with_an_at_sign_is_staged_and_unstaged() {
    // svn 把 `@` 后面当 peg revision；不加尾 `@` 时 svn add 会报 E125001
    let fx = hybrid_with_colleague();
    std::fs::write(fx.wc().join("Code/a@b.cs"), "x\n").unwrap();
    let before = snapshot(&fx.wc_str(), &targets()).unwrap();
    stage(&fx.wc_str(), &targets(), &[change("A", "Code/a@b.cs")], &before).unwrap();
    assert!(svn(&fx, &["status", "Code"]).contains("a@b.cs"));
    unstage(&fx.wc_str(), &targets(), &before).unwrap();
    assert_eq!(snapshot(&fx.wc_str(), &targets()).unwrap(), before);
}
```

- [ ] **Step 2: 跑测试确认失败**

Run：`cargo test --test svn_stage`
Expected: 编译失败——`svn_stage` 不存在。

- [ ] **Step 3: 实现 `svn_stage.rs`**

```rust
//! D4：把交付清单登记到 SVN（`svn add --parents` / `svn delete`），以及 spec §8.5 的精确撤销。
//!
//! # 撤销的不变量
//!
//! 工作目录与 `.svn` 的登记都回到 D4 之前的快照（`svnStatusBefore`）。撤完逐条比对
//! `svn status`，**不拿 git 的干净当证据**——git 看不见 `.svn` 里的登记（§8.5.1）。

use crate::engine::svn_cli::run_svn;
use crate::engine::svn_journal::SvnNode;
use crate::engine::svn_sync_check::list_paths;
use crate::engine::svn_sync_out_check::DeliverChange;
use crate::engine::svn_wc::peg_safe;
use crate::engine::svn_xml::parse_status_entries;
use crate::error::{GitError, Result};
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageRecord {
    /// 相对 `before` 新增的节点状态（spec §8.5 的 `svnDelta`）。
    pub delta: Vec<SvnNode>,
    /// D5 的 target 必须带上的父目录（spec §8.2.6；本 Task 的补足见计划）。
    pub new_parent_dirs: Vec<String>,
}

/// 白名单 target 下所有非干净节点，按路径排序。
///
/// `dir` 在此刻查磁盘：`svn status --xml` 不给 `kind`（计划开工前实测 M2-E1）。
pub fn snapshot(repo: &str, targets: &[String]) -> Result<Vec<SvnNode>> {
    if targets.is_empty() {
        return Err(empty_targets("svn status"));
    }
    let mut out = Vec::new();
    for t in targets {
        let xml = run_svn(
            repo,
            &["status", "--xml", "--ignore-externals", "--non-interactive", "--", &peg_safe(t)],
        )?;
        for e in parse_status_entries(&xml)? {
            // external 标记带了 --ignore-externals 也照样出现，它不是登记（svn_dirty.rs 记过）
            if e.item == "external" || (e.item == "normal" && e.props == "none") {
                continue;
            }
            let dir = Path::new(repo).join(&e.path).is_dir();
            out.push(SvnNode {
                path: e.path,
                status: e.item,
                dir,
            });
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out.dedup();
    Ok(out)
}

/// D4。调用方必须**先**把 `before` 写进 journal（spec §7.6 写前）。
pub fn stage(
    repo: &str,
    targets: &[String],
    changes: &[DeliverChange],
    before: &[SvnNode],
) -> Result<StageRecord> {
    if changes.is_empty() {
        return Err(empty_targets("svn add / svn delete")); // §8.3
    }
    let adds: Vec<&str> = changes.iter().filter(|c| c.status == "A").map(|c| c.path.as_str()).collect();
    let dels: Vec<&str> = changes.iter().filter(|c| c.status == "D").map(|c| c.path.as_str()).collect();
    if !adds.is_empty() {
        svn_batch(repo, &["add", "--parents", "--non-interactive", "--"], &adds)?;
    }
    if !dels.is_empty() {
        svn_batch(repo, &["delete", "--non-interactive", "--"], &dels)?;
    }
    let after = snapshot(repo, targets)?;
    let delta: Vec<SvnNode> = after.iter().filter(|n| !before.contains(n)).cloned().collect();

    // A 文件的各级祖先里、此刻是 added 的目录——包括用户自己事先 add 过的
    let added_dirs: BTreeSet<&str> = after
        .iter()
        .filter(|n| n.status == "added" && n.dir)
        .map(|n| n.path.as_str())
        .collect();
    let mut parents = BTreeSet::new();
    for a in &adds {
        let mut p = Path::new(a).parent();
        while let Some(dir) = p {
            let s = dir.to_string_lossy().replace('\\', "/");
            if s.is_empty() {
                break;
            }
            if added_dirs.contains(s.as_str()) {
                parents.insert(s);
            }
            p = dir.parent();
        }
    }
    Ok(StageRecord {
        delta,
        // BTreeSet 有序：父在子之前，与 svn 对 target 顺序的期望一致
        new_parent_dirs: parents.into_iter().collect(),
    })
}

/// spec §8.5：撤到 `before` 为止。只撤**本流程能产生的**两类登记（计划 B6）。
pub fn unstage(repo: &str, targets: &[String], before: &[SvnNode]) -> Result<()> {
    let now = snapshot(repo, targets)?;
    let mut ours: Vec<SvnNode> = now
        .into_iter()
        .filter(|n| matches!(n.status.as_str(), "added" | "deleted") && !before.contains(n))
        .collect();
    // 从深到浅：先 revert 父目录会让子路径的登记状态变得不确定（§8.5）
    ours.sort_by_key(|n| std::cmp::Reverse(n.path.matches('/').count()));
    for n in &ours {
        svn_revert(repo, std::slice::from_ref(&n.path))?;
        if n.status == "deleted" {
            // revert 把它从 base 恢复到了磁盘；D4 之前它是 missing（D3.8 的 checkout 删的）。
            // 用显式文件删除，不用 git clean——那条命令语义太宽（§8.5）
            let p = Path::new(repo).join(&n.path);
            if p.is_file() {
                std::fs::remove_file(&p)?;
            }
        }
    }
    let again = snapshot(repo, targets)?;
    if again != before {
        let diff: Vec<String> = again
            .iter()
            .filter(|n| !before.contains(n))
            .map(|n| format!("{} {}", n.status, n.path))
            .chain(before.iter().filter(|n| !again.contains(n)).map(|n| format!("（应为）{} {}", n.status, n.path)))
            .collect();
        return Err(GitError::new(
            "sync_out_rollback_incomplete",
            format!(
                "撤销 SVN 登记之后，白名单的 svn 状态与交付前不一致：\n{}\n记录已保留。\
                 请用 TortoiseSVN 看一下这些路径，处理好之后在横幅上点「清理」。",
                list_paths(&diff)
            ),
        ));
    }
    Ok(())
}

/// `svn revert`（**不递归**）。空列表拒绝：空 target 会退化成对当前目录操作（必测 #2）。
pub fn svn_revert(repo: &str, paths: &[String]) -> Result<()> {
    if paths.is_empty() {
        return Err(empty_targets("svn revert"));
    }
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    svn_batch(repo, &["revert", "--non-interactive", "--"], &refs)
}

fn svn_batch(repo: &str, head: &[&str], paths: &[&str]) -> Result<()> {
    let pegged: Vec<String> = paths.iter().map(|p| peg_safe(p)).collect();
    let mut args: Vec<&str> = head.to_vec();
    args.extend(pegged.iter().map(String::as_str));
    run_svn(repo, &args).map(|_| ()).map_err(|e| {
        GitError::new(
            "sync_out_stage_failed",
            format!("svn {} 失败了：{}", head[0], e.message.trim()),
        )
    })
}

fn empty_targets(what: &str) -> GitError {
    GitError::new(
        "sync_out_empty_targets",
        format!("拒绝发出没有目标的 {what}：空目标会作用于整个工作副本。"),
    )
}
```

`engine/mod.rs` 加 `pub mod svn_stage;`。

- [ ] **Step 4: 跑测试确认通过**

Run：`cargo test --test svn_stage`
Expected: 7 个用例全部 PASS。若 `staging_registers…` 里 `Code/B.cs` 在 `before` 中不是 `missing`，打印 XML 核对（实测 M2-E2 是 `!` = missing），把实际值记进决策日志。

- [ ] **Step 5: 全量 Rust 验证并 Commit**

```bash
cargo fmt --check && cargo clippy --all-targets && cargo test
git add src-tauri/src/engine/svn_stage.rs src-tauri/src/engine/mod.rs src-tauri/tests/svn_stage.rs
git commit -m "feat(svn): 交付的 SVN 登记与按快照逐条撤销"
```

更新决策日志：B5 / B6、`newParentDirs` 的补足、M2-E1 / E2 的复核结果。

**已知限制（记进决策日志，不在本期处理）：** git 不跟踪目录，所以「删掉一整个目录」在 D1 里是一串文件删除；D4 只 `svn delete` 这些文件，服务器上会留下空目录。

---

## Task 5：编排主路径 D0–D10（`svn_sync_out`）

本 Task 写出整个模块的主体：执行入口、早退收尾、D5 分类、D6–D10。Local-Only 的 D8d rebase 分支也在这里写好（它和无补丁分支同在 `restore_branch` 里），但它的测试在 Task 6；继续 / 放弃在 Task 8。

**Files:**
- Create: `src-tauri/src/engine/svn_sync_out.rs`
- Modify: `src-tauri/src/engine/svn_sync_in.rs`（`tracked_changes`、`untracked_leftovers` 改 `pub(crate)`；`checkout_verified` 的报错部分抽成 `pub(crate) fn checkout_failed`）
- Modify: `src-tauri/src/engine/svn_sync_out_check.rs`（`git_dirty` 改 `pub(crate)`）
- Modify: `src-tauri/src/engine/mod.rs`
- Test: `src-tauri/tests/svn_sync_out.rs`

**Interfaces:**
- Consumes: Task 1–4 的全部产出；`svn_sync_in::{checkout_verified, checkout_failed, tracked_changes, untracked_leftovers, dirty_w_files, rebase_in_progress, new_opid, need}`；`svn_init::record_server_tree_into`；`svn_preflight::{run_preflight, read_node_rev, svn_targets}`；`svn_baseline::{create_baseline_commit, move_baseline_ref, read_baseline, swap_index_to_tree, parse_trailer, TempIndex, Trailer}`。
- Produces:
  - `pub struct SyncOutExpect { orig_branch, orig_head, baseline_oid, deliver_tree, changes: Vec<DeliverChange>, confirmed_outside: Vec<String>, message: String }`（`Deserialize`，camelCase）
  - `pub enum SyncOutOutcome { Delivered { rev: u64, branch: String, local_only_left: usize }, RebaseConflict { rev: u64, files: Vec<String> }, Abandoned { committed: bool } }`（`tag = "kind"`，camelCase + `rename_all_fields`）
  - `pub trait Probe { fn at(&self, step: Step, repo: &str) {} }`，闭包 `Fn(Step, &str)` 自动实现；`pub enum Step { Built, CheckedOut, Staged, BeforeCommit, Committed, Updated, Captured, Published }`
  - `pub fn run_sync_out(repo: &str, expect: &SyncOutExpect) -> Result<SyncOutOutcome>`
  - `pub fn run_sync_out_probed(repo: &str, expect: &SyncOutExpect, probe: &dyn Probe) -> Result<SyncOutOutcome>`——**只给集成测试用**，doc 注释写明
  - `pub fn commit_targets(new_parent_dirs: &[String], changes: &[DeliverChange]) -> Result<Vec<String>>`
  - `pub enum CommitFailure { Rejected, Unknown }`、`pub fn classify_commit_failure(stderr: &str, stdout: &str) -> CommitFailure`
  - 给 Task 8 用（`pub(crate)`）：`cleanup_before_svn`、`after_commit`、`publish_delivered`、`restore_branch`、`finish`

- [ ] **Step 1: 小重构 `svn_sync_in.rs`（行为不变）**

把 `checkout_verified` 末尾构造错误的那段抽出来：

```rust
/// checkout 没切干净时的报错（计划 A15）。`sync_out` 的 detached checkout 也用它。
pub(crate) fn checkout_failed(
    repo: &str,
    target: &str,
    stale: &[String],
    ran: Result<String>,
) -> GitError {
    let files = if stale.is_empty() {
        String::new()
    } else {
        format!("\n下面这些文件没能写进去或删掉：\n{}", list_paths(stale))
    };
    let git_said = match ran {
        Err(e) => format!("\ngit 原话：{}", e.message.trim()),
        Ok(_) => String::new(),
    };
    let root = Path::new(repo);
    GitError::new(
        "sync_in_checkout_failed",
        format!(
            "切到 `{target}` 没有成功，多半是有程序占着文件（Unity / IDE）。{files}\n{}{git_said}",
            describe(root, &find_blockers(root, 5))
        ),
    )
}
```

`checkout_verified` 的结尾改成 `Err(checkout_failed(repo, branch, &stale, ran))`。`tracked_changes` 与 `untracked_leftovers` 前加 `pub(crate)`。`svn_sync_out_check.rs` 的 `git_dirty` 前加 `pub(crate)`。

Run：`cargo test --test svn_sync_in`。Expected：全部 PASS（文案里「切到分支 `x`」变成「切到 `x`」，若有测试断言旧文案就跟着改，记进决策日志）。

- [ ] **Step 2: 写失败测试**

`src-tauri/tests/svn_sync_out.rs`（Task 6–9 继续往这个文件里加用例）：

```rust
//! `sync_out` 编排（spec §8.2 D0–D10）。全部在临时 fixture 上跑：wc1 是「你」，wc2 是同事。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::git_cli::run_git;
use app_lib::engine::svn_journal;
use app_lib::engine::svn_lock;
use app_lib::engine::svn_sync::{check_sync, SyncVerdict};
use app_lib::engine::svn_sync_out::{
    classify_commit_failure, commit_targets, run_sync_out, run_sync_out_probed, CommitFailure,
    Step, SyncOutExpect, SyncOutOutcome,
};
use app_lib::engine::svn_sync_out_check::{preview_sync_out, DeliverChange, SyncOutPreview};
use std::process::Command;
use svn_fixtures::{hybrid_whitelist_in_external, hybrid_with_colleague, SvnFixture};

fn git(fx: &SvnFixture, args: &[&str]) -> String {
    run_git(&fx.wc_str(), args).unwrap().trim().to_string()
}

fn svn_in(dir: &std::path::Path, args: &[&str]) -> String {
    let out = Command::new("svn").current_dir(dir).args(args).output().unwrap();
    assert!(out.status.success(), "svn {args:?}：{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn repo_url(fx: &SvnFixture) -> String {
    svn_fixtures::file_url(&fx.repo())
}

/// 服务器上某个文件在 HEAD 的内容。
fn server_cat(fx: &SvnFixture, rel: &str) -> String {
    svn_in(fx.path(), &["cat", &format!("{}/{rel}", repo_url(fx))])
}

fn write(fx: &SvnFixture, rel: &str, content: &str) {
    let p = fx.wc().join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

fn expect_from(p: &SyncOutPreview, message: &str) -> SyncOutExpect {
    SyncOutExpect {
        orig_branch: p.orig_branch.clone(),
        orig_head: p.orig_head.clone(),
        baseline_oid: p.baseline_oid.clone(),
        deliver_tree: p.deliver_tree.clone(),
        changes: p.changes.clone(),
        confirmed_outside: p.outside_changes.clone(),
        message: message.to_string(),
    }
}

/// 预览 → 「用户确认」→ 执行。
fn deliver(fx: &SvnFixture, message: &str) -> app_lib::error::Result<SyncOutOutcome> {
    let p = preview_sync_out(&fx.wc_str())?;
    run_sync_out(&fx.wc_str(), &expect_from(&p, message))
}

fn deliver_probed(
    fx: &SvnFixture,
    message: &str,
    probe: &dyn Fn(Step, &str),
) -> app_lib::error::Result<SyncOutOutcome> {
    let p = preview_sync_out(&fx.wc_str())?;
    run_sync_out_probed(&fx.wc_str(), &expect_from(&p, message), &probe)
}

/// 一次典型的交付内容：改 A、删 B、在两层新目录里加 New.cs。
fn typical_work(fx: &SvnFixture) {
    write(fx, "Code/A.cs", "a2\n");
    std::fs::remove_file(fx.wc().join("Code/B.cs")).unwrap();
    write(fx, "Code/Deep/Nested/New.cs", "new\n");
    git(fx, &["add", "-A", "Code"]);
    git(fx, &["commit", "-qm", "work"]);
}

/// 交付彻底收尾的通用断言（spec §8.2 D10、必测 #36）。
fn assert_settled(fx: &SvnFixture) {
    assert!(svn_journal::read(&fx.wc_str()).unwrap().is_none(), "journal 没删");
    assert!(!svn_lock::is_held(&fx.wc_str()), "锁没放");
    assert_eq!(git(fx, &["for-each-ref", "refs/mygitclient/deliver/"]), "", "交付 ref 没删");
    assert_eq!(git(fx, &["symbolic-ref", "--short", "HEAD"]), "work");
    assert_eq!(git(fx, &["status", "--porcelain"]), "");
}

#[test]
fn a_plain_delivery_goes_all_the_way_to_d10() {
    // 必测 #36（无 Local-Only，走快进）+ #18 + #62 前半
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    let before = git(&fx, &["rev-parse", "HEAD"]);

    let out = deliver(&fx, "修复跳跃").unwrap();
    assert_eq!(
        out,
        SyncOutOutcome::Delivered { rev: 2, branch: "work".into(), local_only_left: 0 }
    );

    // 服务器：一个 revision，五条路径，提交信息原样
    let log = svn_in(fx.path(), &["log", "-v", "-r", "2", &repo_url(&fx)]);
    for line in ["M /Code/A.cs", "D /Code/B.cs", "A /Code/Deep", "A /Code/Deep/Nested", "A /Code/Deep/Nested/New.cs"] {
        assert!(log.contains(line), "缺 {line}：{log}");
    }
    assert!(log.contains("修复跳跃"), "{log}");
    assert_eq!(server_cat(&fx, "Code/A.cs"), "a2\n");

    // 基线推进到新内容，trailer 记 r2；工作分支快进到它（spec §8.2.2）
    let base_msg = git(&fx, &["log", "-1", "--format=%B", "svn/base"]);
    assert!(base_msg.contains("@2 "), "{base_msg}");
    assert_eq!(git(&fx, &["rev-parse", "work"]), git(&fx, &["rev-parse", "svn/base"]));
    assert_eq!(git(&fx, &["show", "svn/base:Code/A.cs"]), "a2");
    assert!(git(&fx, &["merge-base", "--is-ancestor", &before, "svn/base"]).is_empty());

    assert_eq!(svn_in(&fx.wc(), &["status", "Code"]), "");
    assert_settled(&fx);
}

#[test]
fn no_false_out_of_sync_after_a_delivery() {
    // 必测 #26：根节点停在旧 revision 是常态，按白名单节点判定不该报
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    deliver(&fx, "work").unwrap();
    assert!(matches!(check_sync(&fx.wc_str()).unwrap().verdict, SyncVerdict::InSync));
}

#[test]
fn delivering_again_right_after_a_delivery_has_nothing_to_deliver() {
    // Review Focus 5
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    deliver(&fx, "work").unwrap();
    assert_eq!(preview_sync_out(&fx.wc_str()).unwrap_err().code, "sync_out_nothing_to_deliver");
}

#[test]
fn a_stray_add_in_the_same_new_dir_is_not_committed_along() {
    // 必测 #62 后半：--depth=empty 只提交 target 自身（spec §8.2.6）
    let fx = hybrid_with_colleague();
    std::fs::create_dir_all(fx.wc().join("Code/Deep2/Nested")).unwrap();
    write(&fx, "Code/Deep2/Stray.txt", "stray\n"); // W-path 内、W-file 外，git 不管
    svn_in(&fx.wc(), &["add", "--parents", "Code/Deep2/Stray.txt"]);
    write(&fx, "Code/Deep2/Nested/New.cs", "new\n");
    git(&fx, &["add", "Code/Deep2/Nested/New.cs"]);
    git(&fx, &["commit", "-qm", "nested"]);

    deliver(&fx, "nested").unwrap();
    let log = svn_in(fx.path(), &["log", "-v", "-r", "2", &repo_url(&fx)]);
    assert!(log.contains("A /Code/Deep2/Nested/New.cs"), "{log}");
    assert!(!log.contains("Stray.txt"), "{log}");
    assert!(svn_in(&fx.wc(), &["status", "Code/Deep2/Stray.txt"]).starts_with('A'));
}

#[test]
fn a_message_that_looks_like_a_path_or_an_option_is_committed_verbatim() {
    // Review Focus 1 / 计划 B14
    let fx = hybrid_with_colleague();
    write(&fx, "Code/A.cs", "a2\n");
    git(&fx, &["commit", "-qam", "w1"]);
    deliver(&fx, "Code/A.cs").unwrap();
    write(&fx, "Code/A.cs", "a3\n");
    git(&fx, &["commit", "-qam", "w2"]);
    deliver(&fx, "-m 第一行\n第二行").unwrap();
    let log = svn_in(fx.path(), &["log", "-r", "2:3", &repo_url(&fx)]);
    assert!(log.contains("Code/A.cs"), "{log}");
    assert!(log.contains("-m 第一行\n第二行"), "{log}");
}

#[test]
fn a_colleague_commit_between_d5_and_d6_ends_up_in_the_new_baseline() {
    // Review Focus 4：新基线 = 服务器最新内容，不只是我的提交
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    let out = deliver_probed(&fx, "mine", &|s, _| {
        if s == Step::Committed {
            fx.colleague_commits("Code/C.cs", "c\n");
        }
    })
    .unwrap();
    assert!(matches!(out, SyncOutOutcome::Delivered { rev: 3, .. }), "{out:?}");
    assert_eq!(git(&fx, &["show", "svn/base:Code/C.cs"]), "c");
    assert_eq!(std::fs::read_to_string(fx.wc().join("Code/C.cs")).unwrap(), "c\n");
    assert_eq!(git(&fx, &["rev-parse", "work"]), git(&fx, &["rev-parse", "svn/base"]));
    assert_settled(&fx);
}

#[test]
fn the_baseline_is_published_from_the_very_tree_that_was_checked() {
    // 必测 #73 的快进一半：D7a 核对过的 TREE 就是新基线的 tree，索引对齐不改磁盘
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    let captured = std::cell::RefCell::new(None::<String>);
    deliver_probed(&fx, "work", &|s, repo| {
        if s == Step::Captured {
            let j = svn_journal::read(repo).unwrap().unwrap();
            *captured.borrow_mut() = j.capture_tree.clone();
        }
    })
    .unwrap();
    let tree = captured.into_inner().expect("Captured 时 journal 里应有 captureTree");
    assert_eq!(git(&fx, &["rev-parse", "svn/base^{tree}"]), tree);
    assert_eq!(git(&fx, &["diff", "--cached", "--name-only"]), "", "索引与 HEAD 一致");
}

#[test]
fn art_outside_the_whitelist_is_neither_updated_nor_merged() {
    // 必测 #15 / #11：D6 只更新白名单路径，美术文件的本地修改原样、不产生 .mine
    let fx = hybrid_with_colleague();
    std::fs::write(fx.wc().join("Art/x.psd"), "psd LOCAL\n").unwrap();
    typical_work(&fx);
    deliver_probed(&fx, "work", &|s, _| {
        if s == Step::Committed {
            fx.colleague_commits("Art/x.psd", "psd colleague\n");
        }
    })
    .unwrap();
    assert_eq!(std::fs::read_to_string(fx.wc().join("Art/x.psd")).unwrap(), "psd LOCAL\n");
    let leftovers: Vec<_> = std::fs::read_dir(fx.wc().join("Art"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    assert_eq!(leftovers, vec!["x.psd".to_string()], "{leftovers:?}");
}

#[test]
fn a_user_branch_named_deliver_does_not_get_in_the_way() {
    // 必测 #25 的全流程一半
    let fx = hybrid_with_colleague();
    git(&fx, &["branch", "deliver"]);
    typical_work(&fx);
    deliver(&fx, "work").unwrap();
    assert!(git(&fx, &["rev-parse", "--verify", "deliver"]).len() == 40);
}

#[test]
fn delivery_works_when_the_whitelist_is_an_external() {
    // 必测 #49 的交付一半：记账取 code 那个 WC
    let fx = hybrid_whitelist_in_external();
    write(&fx, "code/A.cs", "a2\n");
    git(&fx, &["commit", "-qam", "work"]);
    let out = deliver(&fx, "in external").unwrap();
    assert!(matches!(out, SyncOutOutcome::Delivered { .. }), "{out:?}");
    assert_eq!(server_cat(&fx, "code-trunk/A.cs"), "a2\n");
    assert!(matches!(check_sync(&fx.wc_str()).unwrap().verdict, SyncVerdict::InSync));
}

#[test]
fn a_held_lock_refuses_the_delivery_and_leaves_no_journal() {
    // 必测 #41 的交付一半（跨进程那半由 2b 的 svn_lock 测试覆盖）
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    let p = preview_sync_out(&fx.wc_str()).unwrap();
    let _other = svn_lock::acquire(&fx.wc_str(), "other").unwrap();
    let err = run_sync_out(&fx.wc_str(), &expect_from(&p, "x")).unwrap_err();
    assert_eq!(err.code, "sync_in_progress", "{}", err.message);
    assert!(svn_journal::read(&fx.wc_str()).unwrap().is_none());
}

#[test]
fn an_empty_message_is_refused_before_anything() {
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    let p = preview_sync_out(&fx.wc_str()).unwrap();
    let err = run_sync_out(&fx.wc_str(), &expect_from(&p, "  \n ")).unwrap_err();
    assert_eq!(err.code, "sync_out_empty_message");
    assert!(!svn_lock::is_held(&fx.wc_str()));
    assert!(svn_journal::read(&fx.wc_str()).unwrap().is_none());
}

#[test]
fn an_empty_commit_target_list_is_refused() {
    // 必测 #1：空 target 的 svn commit 会提交整个工作副本，且不可撤销（spec §8.3）
    let err = commit_targets(&[], &[]).unwrap_err();
    assert_eq!(err.code, "sync_out_empty_targets");
    let err = commit_targets(&["Code/Deep".into()], &[]).unwrap_err();
    assert_eq!(err.code, "sync_out_empty_targets", "只有父目录、没有文件也不行");
    let ok = commit_targets(
        &["Code/Deep".into()],
        &[DeliverChange { status: "A".into(), path: "Code/Deep/N.cs".into() }],
    )
    .unwrap();
    assert_eq!(ok, vec!["Code/Deep@".to_string(), "Code/Deep/N.cs@".to_string()]);
}

#[test]
fn commit_failures_are_rolled_back_only_when_the_server_surely_did_nothing() {
    // 计划 B3，输入是开工前实测 M2 的原文
    let ood = "svn: E155011: Commit failed (details follow):\nsvn: E155011: File 'x' is out of date\nsvn: E160028: File '/Code/Shared.cs' is out of date\n";
    let gone = "svn: E155011: Commit failed (details follow):\nsvn: E160013: File not found: transaction '4-5', path '/Code/A.cs'\n";
    let hook = "svn: E165001: Commit failed (details follow):\nsvn: E165001: Commit blocked by pre-commit hook (exit code 1) with output:\nrejected\n";
    let unreachable = "svn: E170013: Commit failed (details follow):\nsvn: E170013: Unable to connect to a repository at URL 'file:///x'\nsvn: E180001: Unable to open repository 'file:///x'\n";
    assert_eq!(classify_commit_failure(ood, ""), CommitFailure::Rejected);
    assert_eq!(classify_commit_failure(gone, ""), CommitFailure::Rejected);
    assert_eq!(classify_commit_failure(hook, ""), CommitFailure::Rejected);
    assert_eq!(classify_commit_failure(unreachable, ""), CommitFailure::Unknown);
    assert_eq!(classify_commit_failure("connection reset", ""), CommitFailure::Unknown);
    assert_eq!(
        classify_commit_failure(ood, "Committed revision 9.\n"),
        CommitFailure::Unknown,
        "stdout 说提交了就绝不回滚"
    );
}
```

- [ ] **Step 3: 跑测试确认失败**

Run：`cargo test --test svn_sync_out`
Expected: 编译失败——`svn_sync_out` 不存在。

- [ ] **Step 4: 实现 `svn_sync_out.rs`**

```rust
//! `sync_out` 编排（spec §8.2 D0–D10）、早退收尾（§8.2.4）、D5 失败分类（§8.6；计划 B3），
//! 以及中断后的继续 / 放弃（§7.6；计划 B4 / B7 / B8，在 Task 8 加）。
//!
//! # 为什么不走 `trait Vcs`、锁、watcher
//!
//! 与 `svn_sync_in` 完全相同：`SvnGitBackend` 的写方法都挂着 `assert_not_syncing`，
//! 本流程全程持锁，走那一层会被自己挡回来；锁在入口拿、返回时由 Drop 释放
//! （D8d 冲突出口也是一次返回）；watcher 由命令层挂起并在结束后补发 `repo-changed`。
//!
//! # 确认在流程之外（计划 B1）
//!
//! 用户对着预览确认（D3）。执行时锁内把前置检查、D0.5、D1 / D2 全部重算，与
//! [`SyncOutExpect`] 比对，不一致就停下让用户重新确认。

use crate::engine::git_cli::{run_git, run_git_code};
use crate::engine::graph::parse_local_only;
use crate::engine::svn_baseline::{
    create_baseline_commit, move_baseline_ref, parse_trailer, read_baseline, swap_index_to_tree,
    TempIndex, Trailer,
};
use crate::engine::svn_cli::{run_svn, run_svn_code};
use crate::engine::svn_config::{read_config, SvnConfig, BASELINE_BRANCH};
use crate::engine::svn_deliver::{deliver_ref, pin, unpin};
use crate::engine::svn_init::record_server_tree_into;
use crate::engine::svn_journal::{self, Flow, Journal, Phase};
use crate::engine::svn_lock;
use crate::engine::svn_preflight::{read_node_rev, run_preflight, svn_targets};
use crate::engine::svn_stage::{snapshot, stage, unstage};
use crate::engine::svn_sync_check::{
    assert_linear, current_branch, list_paths, rev_parse, unfinished_error, unmerged_paths,
};
use crate::engine::svn_sync_in::{
    checkout_failed, checkout_verified, need, new_opid, rebase_in_progress, tracked_changes,
    untracked_leftovers,
};
use crate::engine::svn_sync_out_check::{
    check_sync_out, git_dirty, DeliverChange, SyncOutPreview,
};
use crate::engine::svn_wc::peg_safe;
use crate::engine::svn_xml::parse_update_status;
use crate::error::{GitError, Result};

/// 用户在确认框里看到并同意的预览事实（计划 B1）。
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncOutExpect {
    pub orig_branch: String,
    pub orig_head: String,
    pub baseline_oid: String,
    /// 比 tree 不比 commit：慢路径每次重放出来的 commit OID 不同（提交时间），树相同。
    pub deliver_tree: String,
    pub changes: Vec<DeliverChange>,
    pub confirmed_outside: Vec<String>,
    pub message: String,
}

/// 一次交付 / 继续 / 放弃的结局。D8d 的 rebase 冲突**不是错误**，是正常出口。
///
/// `rename_all_fields` 不能省：只有 `rename_all` 时变体内部字段原样发出（`local_only_left`），
/// 前端读 `localOnlyLeft` 拿到 undefined——`SyncVerdict` 上踩过同一个坑。
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum SyncOutOutcome {
    /// D10 完成。`local_only_left > 0` 时界面不得提示「可删除分支」（spec §8.2.3 D9）。
    Delivered {
        rev: u64,
        branch: String,
        local_only_left: usize,
    },
    /// D8d 的 rebase 冲突：提交已在服务器上、基线已推进，只剩本地补丁没落回去。
    /// journal 停在 `restoring-branch`，锁已释放（spec §13）。
    RebaseConflict { rev: u64, files: Vec<String> },
    /// 继续 / 放弃把一次中断的交付清理掉了。`committed` 为真时提交已在服务器上。
    Abandoned { committed: bool },
}

/// **测试缝**：在流程的几个固定点插一段动作（外部工具改磁盘、同事提交、模拟崩溃）。
/// 生产路径用 [`run_sync_out`]，等价于什么都不插。
pub trait Probe {
    fn at(&self, _step: Step, _repo: &str) {}
}

impl<F: Fn(Step, &str)> Probe for F {
    fn at(&self, step: Step, repo: &str) {
        self(step, repo)
    }
}

/// [`Probe`] 的插入点。每一个都在对应 phase 落盘**之后**。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// D0.5 完成，`deliver-built`。
    Built,
    /// D3.8 完成，`checked-out`。
    CheckedOut,
    /// D4 完成，`svn-staged`，D4.9 之前。
    Staged,
    /// D4.9 通过，D5 之前（phase 仍是 `svn-staged`）。
    BeforeCommit,
    /// D5 成功，`post-commit`。
    Committed,
    /// D6 完成，D7 之前（`post-commit`）。
    Updated,
    /// D8a 完成：新基线 commit 已建，指针未动（`publishing-baseline`）。
    Captured,
    /// D8c 完成，`restoring-branch`，D8d 之前。
    Published,
}

/// 一次完整的交付。
pub fn run_sync_out(repo: &str, expect: &SyncOutExpect) -> Result<SyncOutOutcome> {
    run_sync_out_probed(repo, expect, &|_: Step, _: &str| {})
}

/// [`run_sync_out`] 加测试缝。**只给集成测试用**。
pub fn run_sync_out_probed(
    repo: &str,
    expect: &SyncOutExpect,
    probe: &dyn Probe,
) -> Result<SyncOutOutcome> {
    let message = expect.message.trim().to_string();
    if message.is_empty() {
        return Err(GitError::new("sync_out_empty_message", "提交信息不能为空。"));
    }
    let opid = new_opid();
    let _lock = svn_lock::acquire(repo, &opid)?;
    if svn_journal::read(repo)?.is_some() {
        return Err(unfinished_error());
    }
    // 锁内复核：前置检查、D0.5、D1 / D2 全部重算。其中的服务器 revision 查询发生在
    // 用户确认之后，就是 D3.5（计划 B13）
    let p = check_sync_out(repo)?;
    verify_expect(&p, expect)?;
    let cfg = read_config(repo)?;
    let targets = svn_targets(repo, &cfg.whitelist)?;

    // D0：任何修改之前落盘
    let mut j = Journal {
        opid: opid.clone(),
        flow: Flow::SyncOut,
        phase: Phase::Prepare,
        started_at: now_secs(),
        orig_branch: Some(p.orig_branch.clone()),
        orig_head: Some(p.orig_head.clone()),
        baseline_rev_before: Some(p.baseline_rev),
        baseline_oid_before: Some(p.baseline_oid.clone()),
        commit_message: Some(message.clone()),
        expected_start_rev: Some(p.remote_rev),
        local_only_commits: p.local_only.iter().map(|c| c.oid.clone()).collect(),
        svn_added: paths_with(&p.changes, "A"),
        svn_deleted: paths_with(&p.changes, "D"),
        svn_modified: paths_with(&p.changes, "M"),
        ..Default::default()
    };
    svn_journal::write(repo, &j)?;

    // D0.5：ref 名先落盘再建 ref（§7.6 写前）。交付树已在复核时算好（对象层，计划 B2）
    j.phase = Phase::BuildingDeliver;
    j.deliver_ref = Some(deliver_ref(&opid));
    j.deliver_oid = Some(p.deliver_oid.clone());
    svn_journal::write(repo, &j)?;
    if let Err(e) = pin(repo, &opid, &p.deliver_oid) {
        return early_exit(repo, &cfg, &j, e);
    }
    j.phase = Phase::DeliverBuilt;
    svn_journal::write(repo, &j)?;
    probe.at(Step::Built, repo);

    // D3.8：全流程第一次动主工作副本，在用户确认之后
    j.phase = Phase::CheckedOut;
    svn_journal::write(repo, &j)?;
    if let Err(e) = checkout_detached(repo, &cfg, &p.deliver_oid) {
        return early_exit(repo, &cfg, &j, e);
    }
    probe.at(Step::CheckedOut, repo);

    // D4：快照先落盘，再发第一条 svn 命令（§7.6 写前；计划 B6）
    let before = match snapshot(repo, &targets) {
        Ok(b) => b,
        Err(e) => return early_exit(repo, &cfg, &j, e),
    };
    j.phase = Phase::SvnStaged;
    j.svn_status_before = before;
    svn_journal::write(repo, &j)?;
    match stage(repo, &targets, &p.changes, &j.svn_status_before) {
        Ok(rec) => {
            j.svn_delta = rec.delta;
            j.new_parent_dirs = rec.new_parent_dirs;
            svn_journal::write(repo, &j)?;
        }
        Err(e) => return rollback_exit(repo, &cfg, &j, &targets, e),
    }
    probe.at(Step::Staged, repo);

    // D4.9：确认之后磁盘仍可能被外部工具改动（spec §8.2.5）
    if let Err(e) = assert_still_delivering(repo, &cfg, &p.deliver_oid) {
        return rollback_exit(repo, &cfg, &j, &targets, e);
    }
    probe.at(Step::BeforeCommit, repo);

    // D5
    let commit_targets = match commit_targets(&j.new_parent_dirs, &p.changes) {
        Ok(t) => t,
        Err(e) => return rollback_exit(repo, &cfg, &j, &targets, e),
    };
    j.phase = Phase::Committing;
    svn_journal::write(repo, &j)?;
    let opid_prop = format!("mygitclient:opid={opid}");
    let mut args: Vec<&str> = vec![
        "commit",
        "--non-interactive",
        "--depth=empty",
        "--force-log", // 计划 B14
        "--with-revprop",
        &opid_prop,
        "-m",
        &message,
        "--",
    ];
    args.extend(commit_targets.iter().map(String::as_str));
    let out = match run_svn_code(repo, &args) {
        Ok(o) => o,
        // svn 根本没起来：什么都没发出去，服务器确定没变
        Err(e) => {
            j.phase = Phase::SvnStaged;
            svn_journal::write(repo, &j)?;
            return rollback_exit(repo, &cfg, &j, &targets, e);
        }
    };
    if out.code != 0 {
        return match classify_commit_failure(&out.stderr, &out.stdout) {
            CommitFailure::Rejected => {
                // 先把 phase 退回 svn-staged 再撤：撤到一半崩了，重开时走撤销而不是「状态未知」
                j.phase = Phase::SvnStaged;
                svn_journal::write(repo, &j)?;
                rollback_exit(repo, &cfg, &j, &targets, commit_rejected(&out.stderr))
            }
            CommitFailure::Unknown => Err(commit_unknown(&j, &out.stderr)),
        };
    }
    j.committed_rev = parse_committed_rev(&out.stdout);
    j.phase = Phase::PostCommit;
    svn_journal::write(repo, &j)?;
    probe.at(Step::Committed, repo);

    after_commit(repo, &cfg, &mut j, probe)
}

/// D6–D10。继续 `post-commit` 时也从这里进（Task 8）。
pub(crate) fn after_commit(
    repo: &str,
    cfg: &SvnConfig,
    j: &mut Journal,
    probe: &dyn Probe,
) -> Result<SyncOutOutcome> {
    update_whitelist(repo, cfg, j)?; // D6
    probe.at(Step::Updated, repo);
    capture_delivered(repo, cfg, j)?; // D7、D7a、D8a
    probe.at(Step::Captured, repo);
    publish_delivered(repo, cfg, j)?; // D8b、D8c
    probe.at(Step::Published, repo);
    restore_branch(repo, cfg, j) // D8d、D9、D10
}

fn verify_expect(p: &SyncOutPreview, e: &SyncOutExpect) -> Result<()> {
    if p.orig_branch != e.orig_branch || p.orig_head != e.orig_head || p.baseline_oid != e.baseline_oid {
        return Err(state_changed(
            "预览之后分支或基线变了（比如在 IDE 里又提交了一次）",
        ));
    }
    if p.deliver_tree != e.deliver_tree || p.changes != e.changes || p.outside_changes != e.confirmed_outside {
        return Err(GitError::new(
            "sync_out_needs_reconfirm",
            "要交付的内容和你确认时看到的不一样了（白名单里 git 管不到的文件也算）。\
             什么都没有改动，请重新点「交付」再确认一次。",
        ));
    }
    Ok(())
}

/// D3.8：detached 切到交付树，事后核对真的切干净了（计划 A15 同一手法：退出码证明不了什么）。
fn checkout_detached(repo: &str, cfg: &SvnConfig, oid: &str) -> Result<()> {
    let from = rev_parse(repo, "HEAD")?;
    let ran = run_git(repo, &["checkout", "--detach", oid]);
    let on_target = current_branch(repo)?.is_none() && rev_parse(repo, "HEAD")? == oid;
    let mut stale = tracked_changes(repo, cfg)?;
    stale.extend(untracked_leftovers(repo, cfg, &[from])?);
    if on_target && stale.is_empty() {
        return Ok(());
    }
    Err(checkout_failed(repo, &oid[..oid.len().min(10)], &stale, ran))
}

/// D4.9（spec §8.2.5）：HEAD 仍是交付树，白名单里 git 没有任何改动。
fn assert_still_delivering(repo: &str, cfg: &SvnConfig, deliver: &str) -> Result<()> {
    let head = rev_parse(repo, "HEAD")?;
    let dirty = git_dirty(repo, cfg)?;
    if head == deliver && current_branch(repo)?.is_none() && dirty.is_empty() {
        return Ok(());
    }
    let mut what = Vec::new();
    if head != deliver {
        what.push(format!("HEAD 现在是 {}，应为交付树 {}", &head[..head.len().min(10)], &deliver[..10]));
    }
    if !dirty.is_empty() {
        what.push(format!("白名单里有改动：\n{}", list_paths(&dirty)));
    }
    Err(GitError::new(
        "sync_out_disk_changed",
        format!(
            "工作副本在你确认之后被外部工具改动过（{}）。再提交的话，交上去的会是磁盘上的这些字节，\
             而不是你确认过的内容，所以这里中止。",
            what.join("；")
        ),
    ))
}

/// D5 的 target：新增的父目录 + 显式文件清单（spec §8.2.6）。
///
/// **空了就拒绝**（spec §8.3）：空 target 的 `svn commit` 会提交整个工作副本，且不可撤销。
/// 只有父目录、没有文件也拒绝——那说明清单在别处丢了。
pub fn commit_targets(new_parent_dirs: &[String], changes: &[DeliverChange]) -> Result<Vec<String>> {
    if changes.is_empty() {
        return Err(GitError::new(
            "sync_out_empty_targets",
            "拒绝发出没有文件的 svn commit：空目标会提交整个工作副本，且不可撤销。",
        ));
    }
    Ok(new_parent_dirs
        .iter()
        .chain(changes.iter().map(|c| &c.path))
        .map(|p| peg_safe(p))
        .collect())
}

/// D5 非零退出时，服务器是不是**确定**没变（计划 B3）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitFailure {
    /// 被服务器明确拒绝：撤销 D4 登记、收尾。
    Rejected,
    /// 拿不准：不回滚、不推进，phase 停在 `committing`（计划 B4）。
    Unknown,
}

/// 「服务器确定什么都没做」的错误码。开工前实测 M2：out-of-date 是 E155011 + E160028，
/// 同事删了文件是 E155011 + E160013，钩子拒绝是 E165001；其余来自 spec §8.6 的表。
const REJECTED_CODES: [&str; 8] = [
    "E155011", "E160028", "E160013", "E160024", "E165001", "E170001", "E200009", "E200007",
];

pub fn classify_commit_failure(stderr: &str, stdout: &str) -> CommitFailure {
    if stdout.contains("Committed revision") {
        return CommitFailure::Unknown; // svn 说提交了：绝不回滚
    }
    let codes = error_codes(stderr);
    if !codes.is_empty() && codes.iter().all(|c| REJECTED_CODES.contains(&c.as_str())) {
        CommitFailure::Rejected
    } else {
        CommitFailure::Unknown
    }
}

/// stderr 里所有 `E` + 6 位数字的错误码。
fn error_codes(s: &str) -> Vec<String> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    for i in 0..b.len() {
        let starts = b[i] == b'E' && (i == 0 || !b[i - 1].is_ascii_alphanumeric());
        if starts
            && i + 7 <= b.len()
            && b[i + 1..i + 7].iter().all(u8::is_ascii_digit)
            && (i + 7 == b.len() || !b[i + 7].is_ascii_digit())
        {
            out.push(s[i..i + 7].to_string());
        }
    }
    out
}

fn parse_committed_rev(stdout: &str) -> Option<u64> {
    let rest = &stdout[stdout.find("Committed revision ")? + "Committed revision ".len()..];
    rest.chars().take_while(char::is_ascii_digit).collect::<String>().parse().ok()
}

fn commit_rejected(stderr: &str) -> GitError {
    GitError::new(
        "sync_out_commit_rejected",
        format!(
            "SVN 拒绝了这次提交，服务器上什么都没有发生。svn 原话：\n{}\n\
             常见原因：有人刚提交了你也改过的文件（先「更新」再交付）、凭据或权限不对、\
             服务器的钩子不接受。",
            stderr.trim()
        ),
    )
}

fn commit_unknown(j: &Journal, stderr: &str) -> GitError {
    let start = j.expected_start_rev.unwrap_or(0);
    GitError::new(
        "sync_out_commit_unknown",
        format!(
            "提交已经发出，但没有拿到服务器的明确答复：\n{}\n\
             它可能已经成功，也可能没有。本版本还不能自动查证，所以什么都不再动：\
             不撤销登记，也不推进基线。请在 TortoiseSVN 的 Show log 里看 r{} 之后有没有\
             提交信息为「{}」的 revision（它带着属性 mygitclient:opid={}）。记录已保留。",
            stderr.trim(),
            start + 1,
            j.commit_message.as_deref().unwrap_or(""),
            j.opid
        ),
    )
}

/// D6（spec §8.2.1）：只更新白名单路径。冲突只看白名单 target 之内。
fn update_whitelist(repo: &str, cfg: &SvnConfig, j: &Journal) -> Result<()> {
    let targets = svn_targets(repo, &cfg.whitelist)?;
    let pegged: Vec<String> = targets.iter().map(|t| peg_safe(t)).collect();
    let mut args: Vec<&str> = vec!["update", "--accept", "postpone", "--non-interactive", "--"];
    args.extend(pegged.iter().map(String::as_str));
    let committed = committed_text(j);
    let up = run_svn_code(repo, &args)?;
    if up.code != 0 {
        return Err(GitError::new(
            "sync_out_update_failed",
            format!(
                "{committed}，但把白名单更新到最新时失败了（常见原因：VPN 断了）。基线还没推进，\
                 记录已保留；网络恢复后在顶部横幅上点「继续」，不会重复提交。svn 原话：{}",
                up.stderr.trim()
            ),
        ));
    }
    // 成败不看退出码：--accept postpone 遇到冲突也退出 0（2c 计划 A4，实测）
    let mut conflicts = Vec::new();
    for t in &pegged {
        let xml = run_svn(repo, &["status", "--xml", "--non-interactive", "--", t])?;
        conflicts.extend(parse_update_status(&xml)?.entries.into_iter().filter(|e| e.is_conflicted()).map(|e| e.path));
    }
    if !conflicts.is_empty() {
        return Err(GitError::new(
            "sync_out_update_conflict",
            format!(
                "{committed}，但更新白名单时有冲突：\n{}\n这些是 git 管不到的文件（被扩展名排除的，\
                 或目录级的树冲突），本应用没法替你解决。请用 TortoiseSVN 处理（Resolve），\
                 然后在顶部横幅上点「继续」。基线还没推进，不会重复提交。",
                list_paths(&conflicts)
            ),
        ));
    }
    Ok(())
}

fn committed_text(j: &Journal) -> String {
    match j.committed_rev {
        Some(r) => format!("提交已经成功（r{r}）"),
        None => "提交已经成功".to_string(),
    }
}

/// D7 + D7a + D8a。D7a 的纳入集合核对不过时停在 `post-commit`（spec §13）。
fn capture_delivered(repo: &str, cfg: &SvnConfig, j: &mut Journal) -> Result<()> {
    let deliver = need(&j.deliver_oid, "deliverOid")?;
    // 从 HEAD 树起步录（§6.3.1）——HEAD 必须还是交付树，否则录的就不是它
    if current_branch(repo)?.is_some() || rev_parse(repo, "HEAD")? != deliver {
        return Err(state_changed("HEAD 已经不在交付树上了"));
    }
    let pre = run_preflight(repo, &cfg.whitelist)?; // D7：nodeRevs 单值
    let idx = TempIndex::from_head(repo)?;
    let (tree, _explained) = record_server_tree_into(repo, cfg, &pre, &idx)?; // D7a
    let now = read_node_rev(repo, &cfg.whitelist)?;
    if now != pre.node_rev {
        return Err(GitError::new(
            "sync_out_revision_moved",
            format!(
                "{}，但录新基线的过程中工作副本被外部改动了（r{} → r{now}）。基线没有推进，\
                 记录已保留；外部操作结束后在顶部横幅上点「继续」。",
                committed_text(j),
                pre.node_rev
            ),
        ));
    }
    // 创建 commit 前记录 captureTree（§7.6）
    j.phase = Phase::PublishingBaseline;
    j.capture_tree = Some(tree.clone());
    j.new_baseline_oid = None;
    svn_journal::write(repo, j)?;
    let trailer = Trailer {
        url: pre.repos_root,
        rev: pre.node_rev,
        uuid: pre.uuid,
    };
    // 父提交是交付树，不是旧基线（spec §8.2.2；§7.6 末「不能套用 S7 的旧基线父」）
    let commit = create_baseline_commit(repo, &tree, Some(&deliver), &trailer)?;
    // 移动 ref 前记录 newBaselineOid（§7.6）
    j.new_baseline_oid = Some(commit);
    svn_journal::write(repo, j)
}

/// D8b + D8c。重开时也从这里续（`publishing-baseline` 且已有 `newBaselineOid`）。
pub(crate) fn publish_delivered(repo: &str, cfg: &SvnConfig, j: &mut Journal) -> Result<()> {
    let old = need(&j.baseline_oid_before, "baselineOidBefore")?;
    let new = need(&j.new_baseline_oid, "newBaselineOid")?;
    let deliver = need(&j.deliver_oid, "deliverOid")?;
    match read_baseline(repo)?.as_deref() {
        Some(cur) if cur == new => {} // 已发布（崩溃恢复时会走到）
        Some(cur) if cur == old => move_baseline_ref(repo, &new, Some(&old))?,
        other => {
            return Err(GitError::new(
                "svn_baseline_moved",
                format!(
                    "基线分支现在是 {other:?}，既不是交付前的 {old}，也不是本次录好的 {new}。\
                     有别的工具动过它，这里停下，不覆盖。提交已在服务器上，记录已保留。"
                ),
            ))
        }
    }
    // D8c：索引从交付树换成新基线树（CAS），磁盘不动——磁盘已经是 D6 之后的候选快照
    let (code, _) = run_git_code(repo, &["diff-index", "--cached", "--quiet", &new])?;
    if code != 0 {
        swap_index_to_tree(repo, &deliver, &new)?;
    }
    // 树 == 索引 == 磁盘，git 不写任何文件
    if current_branch(repo)?.as_deref() != Some(BASELINE_BRANCH) {
        checkout_verified(repo, cfg, BASELINE_BRANCH, false)?;
    }
    j.phase = Phase::RestoringBranch;
    svn_journal::write(repo, j)
}

/// D8d（spec §8.2.2）+ D9 / D10。
pub(crate) fn restore_branch(repo: &str, cfg: &SvnConfig, j: &mut Journal) -> Result<SyncOutOutcome> {
    let orig = need(&j.orig_branch, "origBranch")?;
    let orig_head = need(&j.orig_head, "origHead")?;
    let new = published(repo, j)?;
    let rev = trailer_rev(repo, &new)?;
    let orig_ref = format!("refs/heads/{orig}");

    if j.local_only_commits.is_empty() {
        // 计划 B10：快进 = 直接把分支指针移到新基线（CAS），再从基线切过去——两者是同一个
        // commit，git 不写任何文件。spec 字面的 checkout + merge --ff-only 会多两次全量重编译
        let tip = rev_parse(repo, &orig_ref)?;
        if tip == orig_head {
            let (code, _) = run_git_code(repo, &["merge-base", "--is-ancestor", &orig_head, &new])?;
            if code != 0 {
                return Err(GitError::new(
                    "sync_out_not_fast_forward",
                    format!(
                        "提交已在服务器上（r{rev}），基线已推进，但分支 `{orig}` 不能快进到新基线——\
                         前面某一步的假设不成立了。这里停下，不自作主张 merge。记录已保留。"
                    ),
                ));
            }
            run_git(repo, &["update-ref", &orig_ref, &new, &orig_head])?;
        } else if tip != new {
            return Err(state_changed(&format!("分支 `{orig}` 的位置变了")));
        }
        if current_branch(repo)?.as_deref() != Some(orig.as_str()) {
            checkout_verified(repo, cfg, &orig, false)?;
        }
    } else {
        match current_branch(repo)?.as_deref() {
            // 磁盘带上本地补丁，D6 拉下来的别人的改动暂时退回——rebase 之后两者都在
            Some(b) if b == BASELINE_BRANCH => checkout_verified(repo, cfg, &orig, false)?,
            Some(b) if b == orig => {}
            other => {
                return Err(state_changed(&format!(
                    "HEAD 在 {other:?} 上，既不是基线分支也不是原分支 `{orig}`"
                )))
            }
        }
        if rev_parse(repo, "HEAD")? != orig_head {
            return Err(state_changed(&format!("分支 `{orig}` 的位置变了")));
        }
        // 计划 B11：rebase 真正要重放的区间从 merge-base 开始
        let mb = run_git(repo, &["merge-base", &new, &orig_head])?.trim().to_string();
        assert_linear(repo, &mb, &orig_head)?;
        // 与 sync_in 的 S9 同一组 -c：用户配置不能改变 rebase 的语义（2c 独立 review C1）
        if let Err(e) = run_git(
            repo,
            &["-c", "rebase.autoStash=false", "-c", "rebase.updateRefs=false", "rebase", "--empty=drop", &new],
        ) {
            return rebase_conflict_or_error(repo, rev, e);
        }
    }
    finish(repo, j, rev)
}

pub(crate) fn rebase_conflict_or_error(repo: &str, rev: u64, e: GitError) -> Result<SyncOutOutcome> {
    let files = unmerged_paths(repo)?;
    if rebase_in_progress(repo)? && !files.is_empty() {
        return Ok(SyncOutOutcome::RebaseConflict { rev, files });
    }
    Err(GitError::new(
        "sync_out_rebase_failed",
        format!(
            "提交已在服务器上（r{rev}），基线已推进，但把本地补丁接到新基线上失败了，而且不是冲突。\
             记录已保留：按 git 说的处理好之后点「继续」。git 原话：{}",
            e.message.trim()
        ),
    ))
}

/// D10：断言成功路径自己的不变量（spec §8.2.4 末尾的表），再复用 ③ 与 ⑤。
pub(crate) fn finish(repo: &str, j: &Journal, rev: u64) -> Result<SyncOutOutcome> {
    let orig = need(&j.orig_branch, "origBranch")?;
    if current_branch(repo)?.as_deref() != Some(orig.as_str()) {
        return Err(state_changed(&format!("收尾时 HEAD 不在原分支 `{orig}` 上")));
    }
    let range = format!("refs/heads/{BASELINE_BRANCH}..refs/heads/{orig}");
    let out = run_git(repo, &["log", "--format=%H%x00%s%x00%B%x1e", &range])?;
    let mut left = 0usize;
    let mut undelivered = Vec::new();
    for rec in out.split('\u{1e}').map(|r| r.trim_start_matches('\n')).filter(|r| !r.is_empty()) {
        let mut f = rec.splitn(3, '\0');
        let (oid, subject, body) = (f.next().unwrap_or(""), f.next().unwrap_or(""), f.next().unwrap_or(""));
        if parse_local_only(body) {
            left += 1;
        } else {
            undelivered.push(format!("{} {subject}", &oid[..oid.len().min(10)]));
        }
    }
    if !undelivered.is_empty() {
        return Err(GitError::new(
            "sync_out_undelivered_left",
            format!(
                "提交已在服务器上（r{rev}），基线已推进，但分支 `{orig}` 上还剩下没标「仅本地」的提交：\n{}\n\
                 它们该交付却没进交付树。这里停下，记录已保留，请把记录导出给开发者看。",
                list_paths(&undelivered)
            ),
        ));
    }
    // ③：成功路径上 deliverRef 还在；⑤：删 journal（锁由调用方的守卫释放）
    if let Some(r) = &j.deliver_ref {
        unpin(repo, r)?;
    }
    svn_journal::remove(repo)?;
    Ok(SyncOutOutcome::Delivered {
        rev,
        branch: orig,
        local_only_left: left,
    })
}

/// spec §8.2.4：D5 之前所有出口共用的收尾。每一步幂等（崩溃恢复会重跑）。
pub(crate) fn cleanup_before_svn(repo: &str, cfg: &SvnConfig, j: &Journal) -> Result<()> {
    let orig = need(&j.orig_branch, "origBranch")?;
    let orig_head = need(&j.orig_head, "origHead")?;
    // ①：本期不建临时 worktree（计划 B2）；字段在就只清它记着的这一个路径
    if let Some(t) = &j.temp_worktree {
        if std::path::Path::new(t).exists() {
            run_git(repo, &["worktree", "remove", "--force", t])?;
        }
    }
    // ②：只有 D3.8 之后才会真的切
    match current_branch(repo)? {
        Some(b) if b == orig => {}
        None if Some(rev_parse(repo, "HEAD")?) == j.deliver_oid => {
            checkout_verified(repo, cfg, &orig, false)?;
        }
        other => {
            return Err(state_changed(&format!(
                "HEAD 在 {other:?} 上，既不是原分支 `{orig}` 也不是交付树"
            )))
        }
    }
    // ③
    if let Some(r) = &j.deliver_ref {
        unpin(repo, r)?;
    }
    // ④：什么都不该发生过
    if rev_parse(repo, "HEAD")? != orig_head {
        return Err(state_changed(&format!("分支 `{orig}` 的位置变了")));
    }
    // ⑤
    svn_journal::remove(repo)
}

/// D4 之前的出口：收尾，并把结果接在原因后面（原因永远在前）。
fn early_exit(repo: &str, cfg: &SvnConfig, j: &Journal, cause: GitError) -> Result<SyncOutOutcome> {
    let tail = match cleanup_before_svn(repo, cfg, j) {
        Ok(()) => format!(
            "已回到分支 `{}`，SVN 侧没有任何改动。",
            j.orig_branch.as_deref().unwrap_or("")
        ),
        Err(e) => format!(
            "另外：收尾没有完成（{}）。记录已保留，处理好之后在顶部横幅上点「清理」。",
            e.message
        ),
    };
    Err(GitError::new(&cause.code, format!("{}\n\n{tail}", cause.message)))
}

/// D4 之后、D5 落地之前的出口：先按快照撤销登记（spec §8.5），再收尾。
fn rollback_exit(
    repo: &str,
    cfg: &SvnConfig,
    j: &Journal,
    targets: &[String],
    cause: GitError,
) -> Result<SyncOutOutcome> {
    if let Err(e) = unstage(repo, targets, &j.svn_status_before) {
        return Err(GitError::new(
            &cause.code,
            format!(
                "{}\n\n另外：撤销 SVN 登记没有完成（{}）。记录已保留，处理好之后在顶部横幅上点「清理」。",
                cause.message, e.message
            ),
        ));
    }
    early_exit(repo, cfg, j, cause)
}

fn published(repo: &str, j: &Journal) -> Result<String> {
    let new = need(&j.new_baseline_oid, "newBaselineOid")?;
    if read_baseline(repo)?.as_deref() != Some(new.as_str()) {
        return Err(GitError::new(
            "svn_baseline_moved",
            "基线分支已经不是本次交付发布的那个了，有别的工具动过它。这里停下，记录已保留。",
        ));
    }
    Ok(new)
}

fn trailer_rev(repo: &str, commit: &str) -> Result<u64> {
    let msg = run_git(repo, &["log", "-1", "--format=%B", commit])?;
    parse_trailer(&msg)
        .map(|t| t.rev)
        .ok_or_else(|| GitError::new("svn_baseline_trailer", "新基线上读不到 git-svn-id。"))
}

pub(crate) fn state_changed(what: &str) -> GitError {
    GitError::new(
        "sync_out_state_changed",
        format!("{what}。这里停下，不覆盖任何东西；请重新点「交付」（或处理好之后点横幅上的按钮）。"),
    )
}

fn paths_with(changes: &[DeliverChange], status: &str) -> Vec<String> {
    changes.iter().filter(|c| c.status == status).map(|c| c.path.clone()).collect()
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
```

`engine/mod.rs` 加 `pub mod svn_sync_out;`。

- [ ] **Step 5: 跑测试确认通过**

Run：`cargo test --test svn_sync_out`
Expected: 本 Task 的 14 个用例全部 PASS。

几处要留意、出现了就记进决策日志「计划没写的坑」：

- `a_plain_delivery…` 的 `log` 断言用 `-v` 的文本输出，SlikSvn 在中文系统上的日期部分可能是乱码（实测 M2 的 `(??, 07 ?? 2026)`），只断言路径与 message，不断言日期。
- `record_server_tree_into` 若在 D7a 因为白名单外的美术文件（`Art/x.psd` 有本地修改）报错，说明它对「W-path 之外」的脏项判断与预期不同——停下核对 `classify` 的输入范围，不要放宽判据。
- `delivery_works_when_the_whitelist_is_an_external` 若 D6 没把 external 拉齐（`nodeRevs` 多值），核对 `svn_targets` 是否把 external 根显式列出（spec §6.5.3）。

- [ ] **Step 6: 全量 Rust 验证并 Commit**

```bash
cargo fmt --check && cargo clippy --all-targets && cargo test
git add src-tauri/src/engine/svn_sync_out.rs src-tauri/src/engine/svn_sync_in.rs src-tauri/src/engine/svn_sync_out_check.rs src-tauri/src/engine/mod.rs src-tauri/tests/svn_sync_out.rs
git commit -m "feat(svn): sync_out 编排 D0–D10——交付、发布新基线、分支快进"
```

更新决策日志：B1 / B3 / B10 / B13 / B14 的实施；`checkout_failed` 的抽取；三处「要留意」的实际结果。

**里程碑 review：** 本 Task 是跨文件的非平凡里程碑（CLAUDE.md「每完成一个非平凡里程碑先跑一次 review」）。跑一次 `/code-review` 覆盖 Task 1–5 的累计 diff，按 `AGENTS.md` §13 rubric 重点看第 1–5、8、9 条，高危问题修完再进 Task 6。

---

## Task 6：Local-Only 全流程（D0.5 慢 / 快路径 + D8d rebase）

代码已在 Task 2 / Task 5 写好；本 Task 只加端到端测试，把 spec §8.2.3 的稳态与 §13 的三行钉住。任何一条不过，修的是 Task 2 / 5 的代码，并在决策日志里记「计划的错误」。

**Files:**
- Test: `src-tauri/tests/svn_sync_out.rs`（追加）

**Interfaces:**
- Consumes: Task 5 的 `run_sync_out` / `run_sync_out_probed` / `SyncOutOutcome::RebaseConflict`。

- [ ] **Step 1: 写测试**

追加到 `svn_sync_out.rs`（复用 Task 2 测试里的 `thirty_lines` / `edit_line` 两个辅助，原样复制到本文件顶部）：

```rust
fn thirty_lines(fx: &SvnFixture) {
    let body: String = (1..=30).map(|n| format!("{n}\n")).collect();
    std::fs::write(fx.wc().join("Code/A.cs"), body).unwrap();
    git(fx, &["commit", "-qam", "thirty lines"]);
}

fn edit_line(fx: &SvnFixture, line: usize, to: &str, message: &str) {
    let p = fx.wc().join("Code/A.cs");
    let text = std::fs::read_to_string(&p).unwrap();
    let out: String = text
        .lines()
        .enumerate()
        .map(|(i, l)| if i + 1 == line { format!("{to}\n") } else { format!("{l}\n") })
        .collect();
    std::fs::write(&p, out).unwrap();
    git(fx, &["commit", "-qam", message]);
}

fn line(text: &str, n: usize) -> &str {
    text.lines().nth(n - 1).unwrap()
}

#[test]
fn a_local_only_patch_in_the_middle_is_never_delivered_and_survives() {
    // 必测 #19（T1 + T3）+ #36 的 rebase 一半：同一文件里既有补丁又有待交付改动
    let fx = hybrid_with_colleague();
    thirty_lines(&fx);
    edit_line(&fx, 1, "one", "C1");
    edit_line(&fx, 10, "ten-LOCAL", "mute log\n\nLocal-Only: true");
    edit_line(&fx, 20, "twenty", "C2");
    let orig_head = git(&fx, &["rev-parse", "HEAD"]);

    let out = deliver(&fx, "jump fix").unwrap();
    assert_eq!(
        out,
        SyncOutOutcome::Delivered { rev: 2, branch: "work".into(), local_only_left: 1 }
    );

    let server = server_cat(&fx, "Code/A.cs");
    assert_eq!((line(&server, 1), line(&server, 10), line(&server, 20)), ("one", "10", "twenty"));
    let base = git(&fx, &["show", "svn/base:Code/A.cs"]);
    assert_eq!(line(&base, 10), "10", "基线里是干净原文");
    let disk = std::fs::read_to_string(fx.wc().join("Code/A.cs")).unwrap();
    assert_eq!(line(&disk, 10), "ten-LOCAL", "磁盘上补丁还在");
    assert_eq!(line(&disk, 20), "twenty");

    let left = git(&fx, &["log", "--format=%s", "svn/base..work"]);
    assert_eq!(left, "mute log", "svn/base..work 只剩那个补丁");
    assert_ne!(git(&fx, &["rev-parse", "work"]), orig_head, "origHead 变了（spec §8.2.4 末）");
    assert_settled(&fx);
}

#[test]
fn a_local_only_suffix_takes_the_fast_path_and_is_rebased_back_on_top() {
    let fx = hybrid_with_colleague();
    thirty_lines(&fx);
    edit_line(&fx, 1, "one", "C1");
    edit_line(&fx, 10, "ten-LOCAL", "mute log\n\nLocal-Only: true");
    let out = deliver(&fx, "c1").unwrap();
    assert!(matches!(out, SyncOutOutcome::Delivered { local_only_left: 1, .. }), "{out:?}");
    assert_eq!(line(&server_cat(&fx, "Code/A.cs"), 10), "10");
    assert_eq!(git(&fx, &["rev-parse", "work~1"]), git(&fx, &["rev-parse", "svn/base"]));
    assert_settled(&fx);
}

#[test]
fn an_overlapping_local_only_aborts_before_anything_is_touched() {
    // 必测 #20（T2）：SVN 侧一个字节没动、没有交付 ref、主工作副本磁盘全程未变
    let fx = hybrid_with_colleague();
    thirty_lines(&fx);
    edit_line(&fx, 10, "ten-LOCAL", "mute log\n\nLocal-Only: true");
    edit_line(&fx, 11, "eleven", "fix jump");
    let disk = std::fs::read(fx.wc().join("Code/A.cs")).unwrap();
    let head = git(&fx, &["rev-parse", "HEAD"]);

    assert_eq!(
        preview_sync_out(&fx.wc_str()).unwrap_err().code,
        "sync_out_local_only_conflict"
    );
    // 执行入口自己也拦得住（锁内复核重算 D0.5），用一份手写的 expect 直接调
    let forged = SyncOutExpect {
        orig_branch: "work".into(),
        orig_head: head.clone(),
        baseline_oid: git(&fx, &["rev-parse", "svn/base"]),
        deliver_tree: "0".repeat(40),
        changes: vec![DeliverChange { status: "M".into(), path: "Code/A.cs".into() }],
        confirmed_outside: vec![],
        message: "x".into(),
    };
    assert_eq!(
        run_sync_out(&fx.wc_str(), &forged).unwrap_err().code,
        "sync_out_local_only_conflict"
    );
    assert_eq!(std::fs::read(fx.wc().join("Code/A.cs")).unwrap(), disk);
    assert_eq!(git(&fx, &["rev-parse", "HEAD"]), head);
    assert_eq!(git(&fx, &["for-each-ref", "refs/mygitclient/"]), "");
    assert!(svn_journal::read(&fx.wc_str()).unwrap().is_none());
    assert!(svn_in(fx.path(), &["log", "-q", &repo_url(&fx)]).contains("r1 "));
    assert!(!svn_in(fx.path(), &["log", "-q", &repo_url(&fx)]).contains("r2 "));
}

#[test]
fn a_merge_inside_the_delivery_is_kept_and_the_local_patch_is_replayed_on_top() {
    // 必测 #72②：merge 已在交付树内，其后只有连续单父的 Local-Only，独有文件保留
    let fx = hybrid_with_colleague();
    thirty_lines(&fx);
    git(&fx, &["checkout", "-q", "-b", "side"]);
    write(&fx, "Code/Side.cs", "side\n");
    git(&fx, &["add", "Code/Side.cs"]);
    git(&fx, &["commit", "-qm", "side"]);
    git(&fx, &["checkout", "-q", "work"]);
    git(&fx, &["merge", "-q", "--no-ff", "-m", "merge side", "side"]);
    edit_line(&fx, 10, "ten-LOCAL", "mute log\n\nLocal-Only: true");

    let out = deliver(&fx, "with merge").unwrap();
    assert!(matches!(out, SyncOutOutcome::Delivered { local_only_left: 1, .. }), "{out:?}");
    assert_eq!(server_cat(&fx, "Code/Side.cs"), "side\n");
    assert_eq!(git(&fx, &["show", "svn/base:Code/Side.cs"]), "side");
    assert!(fx.wc().join("Code/Side.cs").exists());
    assert_eq!(line(&std::fs::read_to_string(fx.wc().join("Code/A.cs")).unwrap(), 10), "ten-LOCAL");
}

#[test]
fn a_local_patch_that_clashes_with_a_colleague_stops_in_a_resolvable_rebase() {
    // 必测 #69 的 D8d 一半（停下的状态）；继续的一半在 Task 8
    let fx = hybrid_with_colleague();
    thirty_lines(&fx);
    edit_line(&fx, 1, "one", "C1");
    edit_line(&fx, 10, "ten-LOCAL", "mute log\n\nLocal-Only: true");
    let out = deliver_probed(&fx, "c1", &|s, _| {
        if s == Step::Committed {
            // 同事在我们 D5 之后改了第 10 行：D6 拉下来，D8d 重放补丁时冲突
            let theirs: String = (1..=30)
                .map(|n| match n {
                    1 => "one\n".to_string(),
                    10 => "ten-colleague\n".to_string(),
                    _ => format!("{n}\n"),
                })
                .collect();
            fx.colleague_commits("Code/A.cs", &theirs);
        }
    })
    .unwrap();
    assert_eq!(
        out,
        SyncOutOutcome::RebaseConflict { rev: 3, files: vec!["Code/A.cs".into()] }
    );
    let j = svn_journal::read(&fx.wc_str()).unwrap().expect("journal 保留");
    assert_eq!(j.phase, app_lib::engine::svn_journal::Phase::RestoringBranch);
    assert!(!svn_lock::is_held(&fx.wc_str()), "冲突出口放锁，用户要编辑、暂存文件");
    assert!(git(&fx, &["log", "-1", "--format=%B", "svn/base"]).contains("@3 "), "基线已推进");
    assert!(server_cat(&fx, "Code/A.cs").contains("ten-colleague"));
}
```

- [ ] **Step 2: 跑测试**

Run：`cargo test --test svn_sync_out`
Expected: 新增 5 个用例全部 PASS。

不过时的排查顺序：先看 Task 2 的 `plan_deliver` 给出的 `deliver_oid`（打印 `preview_sync_out` 的结果），再看 `restore_branch` 的 rebase 输出。`a_local_only_patch_in_the_middle…` 若 rebase 冲突，多半是重放出来的交付 commit 与原 commit 的 patch-id 不同、又不能被 `--empty=drop` 吸收——把实际 git 输出抄进决策日志「计划没写的坑」，**不要**改成 `-X theirs` 之类的自动解冲突（spec §8.2.3「绝不自动解冲突」）。

- [ ] **Step 3: Commit**

```bash
cargo fmt --check && cargo clippy --all-targets && cargo test
git add src-tauri/tests/svn_sync_out.rs
git commit -m "test(svn): 本地补丁随交付摘除、落回，冲突时停在可解决的 rebase"
```

---

## Task 7：早退与 D5 失败

**Files:**
- Modify: `src-tauri/src/engine/svn_sync_out.rs`（`cleanup_before_svn` 第 ② 步加 `-f` 退路）
- Modify: `src-tauri/src/engine/svn_sync_in.rs`（`is_baseline_bytes` 改 `pub(crate)`）
- Test: `src-tauri/tests/svn_sync_out.rs`（追加）

**Interfaces:**
- Consumes: `svn_sync_in::is_baseline_bytes(repo, bases: &[String], p) -> Result<bool>`（名字沿用，语义是「磁盘上的 `p` 是不是这几个 commit 之一里原样存着的字节，或已不在」）。

- [ ] **Step 1: 写失败测试**

追加到 `svn_sync_out.rs`：

```rust
/// 白名单当前的 svn 状态文本，用来断言「SVN 侧回到了交付前」。
fn svn_status(fx: &SvnFixture) -> String {
    svn_in(&fx.wc(), &["status", "Code"])
}

fn server_head(fx: &SvnFixture) -> String {
    svn_in(fx.path(), &["info", "--show-item", "revision", &repo_url(fx)]).trim().to_string()
}

/// 早退之后的通用断言：回到原分支原位置、没有交付 ref、没有 journal、锁已放、SVN 原样。
fn assert_untouched(fx: &SvnFixture, head: &str, status: &str, server: &str) {
    assert_eq!(git(fx, &["symbolic-ref", "--short", "HEAD"]), "work");
    assert_eq!(git(fx, &["rev-parse", "HEAD"]), head);
    assert_eq!(git(fx, &["for-each-ref", "refs/mygitclient/deliver/"]), "");
    assert!(svn_journal::read(&fx.wc_str()).unwrap().is_none());
    assert!(!svn_lock::is_held(&fx.wc_str()));
    assert_eq!(svn_status(fx), status, "svn 登记没回到交付前");
    assert_eq!(server_head(fx), server, "服务器上多了 revision");
    assert_eq!(git(fx, &["status", "--porcelain"]), "");
}

#[test]
fn a_commit_made_while_the_dialog_was_open_is_caught_before_anything_moves() {
    // Review Focus 2
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    let p = preview_sync_out(&fx.wc_str()).unwrap();
    write(&fx, "Code/A.cs", "a3\n");
    git(&fx, &["commit", "-qam", "one more"]);
    let (head, status, server) = (git(&fx, &["rev-parse", "HEAD"]), svn_status(&fx), server_head(&fx));
    let err = run_sync_out(&fx.wc_str(), &expect_from(&p, "x")).unwrap_err();
    assert_eq!(err.code, "sync_out_state_changed", "{}", err.message);
    assert_untouched(&fx, &head, &status, &server);
}

#[test]
fn a_file_git_does_not_manage_changed_after_the_preview_needs_reconfirmation() {
    // 计划 B1：用户确认时没看到的白名单外修改，不能被执行时悄悄带进 D6
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/x.psd", "psd1\n");
    let p = app_lib::engine::svn_sync_check::preview_sync_in(&fx.wc_str()).unwrap();
    app_lib::engine::svn_sync_in::run_sync_in(
        &fx.wc_str(),
        &app_lib::engine::svn_sync_in::SyncInExpect {
            orig_branch: p.orig_branch,
            orig_head: p.orig_head,
            baseline_oid: p.baseline_oid,
            confirmed_outside: p.outside_changes,
        },
    )
    .unwrap();
    typical_work(&fx);
    let p = preview_sync_out(&fx.wc_str()).unwrap();
    assert!(p.outside_changes.is_empty());
    std::fs::write(fx.wc().join("Code/x.psd"), "psd LOCAL\n").unwrap();
    let (head, status, server) = (git(&fx, &["rev-parse", "HEAD"]), svn_status(&fx), server_head(&fx));
    let err = run_sync_out(&fx.wc_str(), &expect_from(&p, "x")).unwrap_err();
    assert_eq!(err.code, "sync_out_needs_reconfirm", "{}", err.message);
    assert_untouched(&fx, &head, &status, &server);
}

#[test]
fn the_server_moving_after_the_preview_is_caught_at_d35() {
    // spec §8.4 / §13「D3.5 复查失败」：保留提交信息由前端负责（Task 11）
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    let p = preview_sync_out(&fx.wc_str()).unwrap();
    fx.colleague_commits("Art/x.psd", "psd2\n");
    let (head, status, server) = (git(&fx, &["rev-parse", "HEAD"]), svn_status(&fx), server_head(&fx));
    let err = run_sync_out(&fx.wc_str(), &expect_from(&p, "x")).unwrap_err();
    assert_eq!(err.code, "sync_out_server_ahead", "{}", err.message);
    assert_untouched(&fx, &head, &status, &server);
}

#[test]
fn switching_back_in_another_tool_after_confirming_is_caught_at_d49() {
    // 必测 #30（spec §8.2.5）
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    let (head, status, server) = (git(&fx, &["rev-parse", "HEAD"]), svn_status(&fx), server_head(&fx));
    let err = deliver_probed(&fx, "x", &|s, repo| {
        if s == Step::Staged {
            run_git(repo, &["checkout", "-q", "work"]).unwrap(); // 用户在 Rider 里切了回去
        }
    })
    .unwrap_err();
    assert_eq!(err.code, "sync_out_disk_changed", "{}", err.message);
    assert_untouched(&fx, &head, &status, &server);
}

#[test]
fn an_out_of_date_commit_is_rolled_back_precisely() {
    // 必测 #3 + #29 的全流程版：交付含新增（两层新目录）+ 删除 + 修改，同事抢先改了 A.cs
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    let (head, status) = (git(&fx, &["rev-parse", "HEAD"]), svn_status(&fx));
    let err = deliver_probed(&fx, "mine", &|s, _| {
        if s == Step::BeforeCommit {
            fx.colleague_commits("Code/A.cs", "colleague\n");
        }
    })
    .unwrap_err();
    assert_eq!(err.code, "sync_out_commit_rejected", "{}", err.message);
    assert!(err.message.contains("E155011"), "原样展示 svn stderr：{}", err.message);
    assert_untouched(&fx, &head, &status, "2"); // r2 是同事的
    let log = svn_in(fx.path(), &["log", "-r", "2", &repo_url(&fx)]);
    assert!(log.contains("colleague Code/A.cs"), "{log}");
}

#[test]
fn a_hook_rejection_is_rolled_back_and_not_retried() {
    // spec §8.6 表「服务端钩子拒绝」/ §8.7 绝不自动重试
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    install_rejecting_pre_commit_hook(&fx);
    let (head, status, server) = (git(&fx, &["rev-parse", "HEAD"]), svn_status(&fx), server_head(&fx));
    let err = deliver(&fx, "mine").unwrap_err();
    assert_eq!(err.code, "sync_out_commit_rejected", "{}", err.message);
    assert!(err.message.contains("rejected by hook"), "{}", err.message);
    assert_untouched(&fx, &head, &status, &server);
}

fn install_rejecting_pre_commit_hook(fx: &SvnFixture) {
    let hooks = fx.repo().join("hooks");
    if cfg!(windows) {
        std::fs::write(hooks.join("pre-commit.bat"), "@echo off\r\necho rejected by hook 1>&2\r\nexit 1\r\n").unwrap();
    } else {
        let p = hooks.join("pre-commit");
        std::fs::write(&p, "#!/bin/sh\necho rejected by hook >&2\nexit 1\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
}

#[test]
fn an_unknown_commit_result_leaves_everything_where_it_is() {
    // 计划 B3 / B4：仓库在 D5 时不可达（E170013 + E180001），既不回滚也不推进
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    let away = fx.path().join("repo-away");
    let err = deliver_probed(&fx, "mine", &|s, _| {
        if s == Step::BeforeCommit {
            std::fs::rename(fx.repo(), &away).unwrap();
        }
    })
    .unwrap_err();
    std::fs::rename(&away, fx.repo()).unwrap();
    assert_eq!(err.code, "sync_out_commit_unknown", "{}", err.message);
    assert!(err.message.contains("mygitclient:opid="), "{}", err.message);
    let j = svn_journal::read(&fx.wc_str()).unwrap().expect("journal 保留");
    assert_eq!(j.phase, app_lib::engine::svn_journal::Phase::Committing);
    assert!(!svn_lock::is_held(&fx.wc_str()));
    assert!(svn_status(&fx).contains("A       Code\\Deep") || svn_status(&fx).contains("A       Code/Deep"));
    assert_eq!(server_head(&fx), "1");
}
```

- [ ] **Step 2: 跑测试确认失败的那几条**

Run：`cargo test --test svn_sync_out`
Expected: 大部分已经 PASS（Task 5 的代码覆盖了这些出口）。若 `switching_back_in_another_tool…` 或 `an_out_of_date…` 在收尾时报 `sync_in_checkout_failed`，说明第 ② 步的普通 checkout 被挡——进入 Step 3；都通过则 Step 3 仍要做（它服务于 Task 8 的 Unity 用例），但先把「本来就通过」记进决策日志。

- [ ] **Step 3: 给收尾第 ② 步加 `-f` 退路**

`svn_sync_in.rs`：`fn is_baseline_bytes` 前加 `pub(crate)`。

`svn_sync_out.rs` 的 `cleanup_before_svn`，第 ② 步改成：

```rust
    // ②：只有 D3.8 之后才会真的切
    match current_branch(repo)? {
        Some(b) if b == orig => {}
        None if Some(rev_parse(repo, "HEAD")?) == j.deliver_oid => back_from_deliver(repo, cfg, j, &orig, &orig_head)?,
        other => {
            return Err(state_changed(&format!(
                "HEAD 在 {other:?} 上，既不是原分支 `{orig}` 也不是交付树"
            )))
        }
    }
```

并加：

```rust
/// 从交付树切回原分支。普通切换被挡住时（多半是 D3.8 没切干净：Unity 占着文件，
/// 磁盘上新旧两种字节混着，计划 A15），只要每个不一致的文件都是交付树或原分支里
/// 原样存着的字节（或已不在），`-f` 就不会丢任何东西；否则原样报错、不覆盖。
fn back_from_deliver(repo: &str, cfg: &SvnConfig, j: &Journal, orig: &str, orig_head: &str) -> Result<()> {
    let Err(first) = checkout_verified(repo, cfg, orig, false) else {
        return Ok(());
    };
    let deliver = need(&j.deliver_oid, "deliverOid")?;
    let bases = [deliver.clone(), orig_head.to_string()];
    let mut paths = tracked_changes(repo, cfg)?;
    paths.extend(changed_between(repo, cfg, &deliver, orig_head)?);
    for p in &paths {
        if !is_baseline_bytes(repo, &bases, p)? {
            return Err(first);
        }
    }
    checkout_verified(repo, cfg, orig, true)
}

/// `a` 与 `b` 之间白名单内有差异的路径。
fn changed_between(repo: &str, cfg: &SvnConfig, a: &str, b: &str) -> Result<Vec<String>> {
    let roots = crate::engine::svn_include_check::normalize_roots(&cfg.whitelist);
    if roots.is_empty() {
        return Ok(Vec::new()); // 空 pathspec 会变成「全部」（§12.2）
    }
    let mut args: Vec<&str> = vec!["diff", "--name-only", "-z", a, b, "--"];
    args.extend(roots.iter().map(String::as_str));
    Ok(run_git(repo, &args)?.split('\0').filter(|p| !p.is_empty()).map(str::to_string).collect())
}
```

`use` 里加上 `is_baseline_bytes`。

- [ ] **Step 4: 跑测试确认通过**

Run：`cargo test --test svn_sync_out`
Expected: 本 Task 的 7 个用例与之前的全部 PASS。`an_unknown_commit_result…` 里 svn status 的路径分隔符随平台不同，断言已兼容两种写法。

- [ ] **Step 5: 全量 Rust 验证并 Commit**

```bash
cargo fmt --check && cargo clippy --all-targets && cargo test
git add src-tauri/src/engine/svn_sync_out.rs src-tauri/src/engine/svn_sync_in.rs src-tauri/tests/svn_sync_out.rs
git commit -m "feat(svn): 交付在 D5 之前的每个出口零代价收尾，被服务器拒绝时精确撤销登记"
```

更新决策日志：B3 的实测复核（E155011 / E165001 / E170013 在全流程里的实际 stderr）、`-f` 退路的论证。

---

## Task 8：继续 / 放弃（计划 B4 / B7 / B8）

**Files:**
- Modify: `src-tauri/src/engine/svn_sync_out.rs`（加 `continue_sync_out`、`abort_sync_out` 及辅助）
- Modify: `src-tauri/src/engine/svn_sync_in.rs`（`require_our_rebase`、`is_ancestor`、`unresolved`、`no_unfinished` 改 `pub(crate)`；`sync_out_unsupported` 的文案改成指向交付横幅）
- Test: `src-tauri/tests/svn_sync_out.rs`（追加）

**Interfaces:**
- Produces: `pub fn continue_sync_out(repo: &str) -> Result<SyncOutOutcome>`、`pub fn abort_sync_out(repo: &str) -> Result<SyncOutOutcome>`。
- 两者只接 `flow = sync_out` 的 journal，否则 `sync_out_wrong_flow`；`sync_in` 的 `continue_sync` / `abort_sync` 照旧拒绝 `sync_out` 的 journal（2c 的 A13，码不变）。

**各 phase 的行为（横幅文案在 Task 11 按这张表写）：**

| phase | 继续 | 放弃 |
|---|---|---|
| `prepare` / `building-deliver` / `deliver-built` / `checked-out` | 清理（§8.2.4），结局 `Abandoned { committed: false }`（B8） | 同左 |
| `svn-staged` | 先按快照撤销登记（§8.5），再清理（B8） | 同左 |
| `committing` | 拒绝，`sync_out_commit_unknown`（B4） | 同左 |
| `post-commit` | 从 D6 起走完（不重复 D5） | 先跑 D6 把白名单拉齐，再停下回到原分支（B7） |
| `publishing-baseline` | 有 `newBaselineOid` 则只补发布；没有则从 D7 重录（HEAD 必须仍是交付树） | 回到原分支（B7），基线若已发布就留在新位置 |
| `restoring-branch` | 我们的 rebase 在进行：要求冲突已解决，`rebase --continue`；没有 rebase：D8d 没做就做、做完了就 D10 | 我们的 rebase 在进行就 `--abort`，回到原分支（B7） |

- [ ] **Step 1: 写失败测试**

追加到 `svn_sync_out.rs`（`use` 里加 `abort_sync_out, continue_sync_out`）：

```rust
use app_lib::engine::svn_journal::Phase;

/// 在 `step` 处模拟崩溃：panic 展开时锁的 Drop 照常释放（等同于进程死后操作系统放锁），
/// journal 停在那一步写下的 phase。
fn crash_at(fx: &SvnFixture, step: Step) {
    let p = preview_sync_out(&fx.wc_str()).unwrap();
    let e = expect_from(&p, "crash");
    let wc = fx.wc_str();
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_sync_out_probed(&wc, &e, &move |s: Step, _: &str| {
            if s == step {
                panic!("模拟崩溃于 {s:?}");
            }
        })
    }));
    assert!(r.is_err(), "应在 {step:?} 崩溃");
    assert!(!svn_lock::is_held(&fx.wc_str()));
}

fn phase(fx: &SvnFixture) -> Phase {
    svn_journal::read(&fx.wc_str()).unwrap().expect("journal 应保留").phase
}

#[test]
fn every_crash_before_the_commit_is_cleaned_up_by_either_button() {
    // 必测 #16 的 D5 之前部分 + 计划 B8：继续与放弃都走同一条清理路径
    for (step, want, use_continue) in [
        (Step::Built, Phase::DeliverBuilt, false),
        (Step::CheckedOut, Phase::CheckedOut, true),
        (Step::Staged, Phase::SvnStaged, false),
        (Step::BeforeCommit, Phase::SvnStaged, true),
    ] {
        let fx = hybrid_with_colleague();
        typical_work(&fx);
        let (head, status, server) = (git(&fx, &["rev-parse", "HEAD"]), svn_status(&fx), server_head(&fx));
        crash_at(&fx, step);
        assert_eq!(phase(&fx), want, "{step:?}");
        let out = if use_continue { continue_sync_out(&fx.wc_str()) } else { abort_sync_out(&fx.wc_str()) }
            .unwrap_or_else(|e| panic!("{step:?}：{}", e.message));
        assert_eq!(out, SyncOutOutcome::Abandoned { committed: false }, "{step:?}");
        assert_untouched(&fx, &head, &status, &server);
    }
}

#[test]
fn a_commit_in_an_unknown_state_cannot_be_continued_or_abandoned_yet() {
    // 计划 B4：查证在 3b
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    let away = fx.path().join("repo-away");
    let _ = deliver_probed(&fx, "mine", &|s, _| {
        if s == Step::BeforeCommit {
            std::fs::rename(fx.repo(), &away).unwrap();
        }
    });
    std::fs::rename(&away, fx.repo()).unwrap();
    let before = svn_journal::read(&fx.wc_str()).unwrap();
    assert_eq!(continue_sync_out(&fx.wc_str()).unwrap_err().code, "sync_out_commit_unknown");
    assert_eq!(abort_sync_out(&fx.wc_str()).unwrap_err().code, "sync_out_commit_unknown");
    assert_eq!(svn_journal::read(&fx.wc_str()).unwrap(), before, "journal 原样");
}

#[test]
fn every_crash_after_the_commit_is_finished_by_continue_without_committing_again() {
    // 必测 #16 的 D5 之后部分 + #73 的「只续对应阶段、不重做 SVN commit」
    for (step, want) in [
        (Step::Committed, Phase::PostCommit),
        (Step::Updated, Phase::PostCommit),
        (Step::Captured, Phase::PublishingBaseline),
        (Step::Published, Phase::RestoringBranch),
    ] {
        let fx = hybrid_with_colleague();
        typical_work(&fx);
        crash_at(&fx, step);
        assert_eq!(phase(&fx), want, "{step:?}");
        let out = continue_sync_out(&fx.wc_str()).unwrap_or_else(|e| panic!("{step:?}：{}", e.message));
        assert_eq!(
            out,
            SyncOutOutcome::Delivered { rev: 2, branch: "work".into(), local_only_left: 0 },
            "{step:?}"
        );
        assert_eq!(server_head(&fx), "2", "{step:?}：服务器上只能有一次提交");
        assert_settled(&fx);
    }
}

#[test]
fn abandoning_after_the_commit_leaves_a_state_that_update_takes_over() {
    // 计划 B7：放弃 = 不再替用户做后面的事；先 D6 拉齐，避免留下 mixed-revision
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    let head = git(&fx, &["rev-parse", "HEAD"]);
    let old_base = git(&fx, &["rev-parse", "svn/base"]);
    crash_at(&fx, Step::Committed);
    assert_eq!(abort_sync_out(&fx.wc_str()).unwrap(), SyncOutOutcome::Abandoned { committed: true });
    assert_eq!(git(&fx, &["rev-parse", "HEAD"]), head);
    assert_eq!(git(&fx, &["symbolic-ref", "--short", "HEAD"]), "work");
    assert_eq!(git(&fx, &["rev-parse", "svn/base"]), old_base, "基线没发布就不发布");
    assert!(svn_journal::read(&fx.wc_str()).unwrap().is_none());
    assert_eq!(git(&fx, &["for-each-ref", "refs/mygitclient/deliver/"]), "");
    match check_sync(&fx.wc_str()).unwrap().verdict {
        SyncVerdict::Diverged { node_rev, baseline_rev } => assert_eq!((node_rev, baseline_rev), (2, 1)),
        v => panic!("应是整体前进，实际 {v:?}"),
    }
    // 「更新」接管（2c 的 A16）：已交付的 commit 在 rebase 时变空、被丢掉
    let p = app_lib::engine::svn_sync_check::preview_sync_in(&fx.wc_str()).unwrap();
    assert_eq!(p.catch_up_rev, Some(2));
    app_lib::engine::svn_sync_in::run_sync_in(
        &fx.wc_str(),
        &app_lib::engine::svn_sync_in::SyncInExpect {
            orig_branch: p.orig_branch,
            orig_head: p.orig_head,
            baseline_oid: p.baseline_oid,
            confirmed_outside: p.outside_changes,
        },
    )
    .unwrap();
    assert_eq!(git(&fx, &["rev-parse", "work"]), git(&fx, &["rev-parse", "svn/base"]));
}

#[test]
fn a_baseline_moved_by_someone_else_is_not_overwritten_on_continue() {
    // 必测 #73：注入旧基线 OID 改变，CAS 拒绝覆盖
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    crash_at(&fx, Step::Captured);
    git(&fx, &["update-ref", "refs/heads/svn/base", "work"]); // 外部工具挪了基线
    let err = continue_sync_out(&fx.wc_str()).unwrap_err();
    assert_eq!(err.code, "svn_baseline_moved", "{}", err.message);
    assert_eq!(phase(&fx), Phase::PublishingBaseline);
    assert_eq!(server_head(&fx), "2");
}

#[test]
fn a_d8d_conflict_is_continued_after_the_user_resolves_it() {
    // 必测 #69 的继续一半
    let fx = hybrid_with_colleague();
    make_d8d_clash(&fx);
    assert_eq!(continue_sync_out(&fx.wc_str()).unwrap_err().code, "sync_in_unresolved", "没解决不能继续");
    let resolved: String = (1..=30)
        .map(|n| match n {
            1 => "one\n".to_string(),
            10 => "ten-LOCAL\n".to_string(),
            _ => format!("{n}\n"),
        })
        .collect();
    std::fs::write(fx.wc().join("Code/A.cs"), resolved).unwrap();
    git(&fx, &["add", "Code/A.cs"]);
    let out = continue_sync_out(&fx.wc_str()).unwrap();
    assert_eq!(
        out,
        SyncOutOutcome::Delivered { rev: 3, branch: "work".into(), local_only_left: 1 }
    );
    assert_settled(&fx);
}

#[test]
fn a_d8d_conflict_can_be_abandoned() {
    let fx = hybrid_with_colleague();
    let head = make_d8d_clash(&fx);
    assert_eq!(abort_sync_out(&fx.wc_str()).unwrap(), SyncOutOutcome::Abandoned { committed: true });
    assert_eq!(git(&fx, &["rev-parse", "HEAD"]), head, "rebase 已中止，分支回到交付前的位置");
    assert_eq!(git(&fx, &["symbolic-ref", "--short", "HEAD"]), "work");
    assert!(git(&fx, &["log", "-1", "--format=%B", "svn/base"]).contains("@3 "), "基线留在新位置");
    assert!(svn_journal::read(&fx.wc_str()).unwrap().is_none());
}

/// 造出 D8d 的 rebase 冲突（与 Task 6 的同一场景），返回交付前的 HEAD。
fn make_d8d_clash(fx: &SvnFixture) -> String {
    thirty_lines(fx);
    edit_line(fx, 1, "one", "C1");
    edit_line(fx, 10, "ten-LOCAL", "mute log\n\nLocal-Only: true");
    let head = git(fx, &["rev-parse", "HEAD"]);
    let out = deliver_probed(fx, "c1", &|s, _| {
        if s == Step::Committed {
            let theirs: String = (1..=30)
                .map(|n| match n {
                    1 => "one\n".to_string(),
                    10 => "ten-colleague\n".to_string(),
                    _ => format!("{n}\n"),
                })
                .collect();
            fx.colleague_commits("Code/A.cs", &theirs);
        }
    })
    .unwrap();
    assert!(matches!(out, SyncOutOutcome::RebaseConflict { .. }), "{out:?}");
    head
}

#[test]
fn the_delivery_buttons_do_not_touch_an_update_journal_and_vice_versa() {
    let fx = hybrid_with_colleague();
    svn_journal::write(
        &fx.wc_str(),
        &app_lib::engine::svn_journal::Journal {
            opid: "in".into(),
            flow: app_lib::engine::svn_journal::Flow::SyncIn,
            phase: Phase::Rebase,
            started_at: 1,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(continue_sync_out(&fx.wc_str()).unwrap_err().code, "sync_out_wrong_flow");
    assert_eq!(abort_sync_out(&fx.wc_str()).unwrap_err().code, "sync_out_wrong_flow");

    let fx2 = hybrid_with_colleague();
    typical_work(&fx2);
    crash_at(&fx2, Step::Built);
    let err = app_lib::engine::svn_sync_in::continue_sync(&fx2.wc_str()).unwrap_err();
    assert_eq!(err.code, "sync_out_not_supported", "{}", err.message);
}

#[cfg(windows)]
#[test]
fn a_file_held_open_at_checkout_is_named_and_nothing_is_committed() {
    // Review Focus 3：Unity 以 share=Read 打开文件时，D3.8 的 checkout 退出 0 却没写进去（2c 的 A15）
    use std::os::windows::fs::OpenOptionsExt;
    let fx = hybrid_with_colleague();
    write(&fx, "Code/A.cs", "a2\n");
    git(&fx, &["commit", "-qam", "deliverable"]);
    write(&fx, "Code/A.cs", "a2 LOCAL\n");
    git(&fx, &["commit", "-qam", "patch\n\nLocal-Only: true"]); // 交付树里的 A.cs 与磁盘不同
    let (head, status, server) = (git(&fx, &["rev-parse", "HEAD"]), svn_status(&fx), server_head(&fx));

    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0x1) // FILE_SHARE_READ
        .open(fx.wc().join("Code/A.cs"))
        .unwrap();
    let err = deliver(&fx, "x").unwrap_err();
    assert_eq!(err.code, "sync_in_checkout_failed", "{}", err.message);
    assert!(err.message.contains("Code/A.cs"), "要指名被占用的文件：{}", err.message);
    drop(held);

    // 收尾在占用期间若没走完，journal 会保留；放开文件之后点「放弃」必须能收干净
    if svn_journal::read(&fx.wc_str()).unwrap().is_some() {
        abort_sync_out(&fx.wc_str()).unwrap();
    }
    assert_untouched(&fx, &head, &status, &server);
    assert_eq!(std::fs::read_to_string(fx.wc().join("Code/A.cs")).unwrap(), "a2 LOCAL\n");
}
```

- [ ] **Step 2: 跑测试确认失败**

Run：`cargo test --test svn_sync_out`
Expected: 编译失败——`continue_sync_out` / `abort_sync_out` 不存在。

- [ ] **Step 3: 实现**

`svn_sync_in.rs`：`require_our_rebase`、`is_ancestor`、`unresolved`、`no_unfinished` 前加 `pub(crate)`。`sync_out_unsupported` 的文案改成：

```rust
        "这条记录是「交付」流程留下的。请用顶部横幅上交付那一组按钮处理，不要手工去动分支指针。",
```

`svn_sync_out.rs` 追加（`use` 里补上 `run_git_env`、`require_our_rebase`、`is_ancestor`、`unresolved`、`no_unfinished`、`dirty_w_files`）：

```rust
// —— 继续 / 放弃（spec §7.6「提供继续 / 放弃两个入口」；计划 B4 / B7 / B8）——

/// 从中断处接着走。
pub fn continue_sync_out(repo: &str) -> Result<SyncOutOutcome> {
    let (_lock, mut j) = take_over(repo)?;
    let cfg = read_config(repo)?;
    let j = &mut j;
    match j.phase {
        // B8：往 D5 走需要用户重新看过清单；崩溃之后什么都可能变了
        Phase::Prepare
        | Phase::BuildingDeliver
        | Phase::DeliverBuilt
        | Phase::CheckedOut
        | Phase::SvnStaged => abandon_before_commit(repo, &cfg, j),
        Phase::Committing => Err(commit_unknown(j, "（上一次在提交途中中断）")),
        Phase::PostCommit => after_commit(repo, &cfg, j, &|_: Step, _: &str| {}),
        Phase::PublishingBaseline => {
            if j.new_baseline_oid.is_none() {
                // 没有新 OID = commit 还没建：从 D7 重录（§7.6）
                capture_delivered(repo, &cfg, j)?;
            }
            publish_delivered(repo, &cfg, j)?;
            restore_branch(repo, &cfg, j)
        }
        Phase::RestoringBranch => continue_restoring(repo, &cfg, j),
        _ => Err(wrong_flow()),
    }
}

/// 放弃这次交付。
pub fn abort_sync_out(repo: &str) -> Result<SyncOutOutcome> {
    let (_lock, mut j) = take_over(repo)?;
    let cfg = read_config(repo)?;
    let j = &mut j;
    match j.phase {
        Phase::Prepare
        | Phase::BuildingDeliver
        | Phase::DeliverBuilt
        | Phase::CheckedOut
        | Phase::SvnStaged => abandon_before_commit(repo, &cfg, j),
        Phase::Committing => Err(commit_unknown(j, "（上一次在提交途中中断）")),
        Phase::PostCommit | Phase::PublishingBaseline | Phase::RestoringBranch => {
            stop_after_commit(repo, &cfg, j)
        }
        _ => Err(wrong_flow()),
    }
}

/// 拿锁并在锁内重读 journal（两次读之间可能被另一个进程推进过）。
fn take_over(repo: &str) -> Result<(svn_lock::SyncLock, Journal)> {
    let first = svn_journal::read(repo)?.ok_or_else(no_unfinished)?;
    if first.flow != Flow::SyncOut {
        return Err(wrong_flow());
    }
    let lock = svn_lock::acquire(repo, &first.opid)?;
    let j = svn_journal::read(repo)?.ok_or_else(no_unfinished)?;
    if j.opid != first.opid || j.flow != Flow::SyncOut {
        return Err(state_changed("记录在拿锁的那一瞬间被换掉了"));
    }
    Ok((lock, j))
}

/// D5 之前的 phase：撤销登记（若有）并走 §8.2.4 收尾。
fn abandon_before_commit(repo: &str, cfg: &SvnConfig, j: &Journal) -> Result<SyncOutOutcome> {
    if j.phase == Phase::SvnStaged {
        let targets = svn_targets(repo, &cfg.whitelist)?;
        unstage(repo, &targets, &j.svn_status_before)?;
    }
    cleanup_before_svn(repo, cfg, j)?;
    Ok(SyncOutOutcome::Abandoned { committed: false })
}

/// 计划 B7：提交已在服务器上，放弃 = 不再替用户做后面的事。
fn stop_after_commit(repo: &str, cfg: &SvnConfig, j: &Journal) -> Result<SyncOutOutcome> {
    let orig = need(&j.orig_branch, "origBranch")?;
    if j.phase == Phase::PostCommit {
        // 只提交、没更新的工作副本是 mixed-revision（实测 M2-E6），「更新」与「重建基线」都不接。
        // 先把白名单拉齐，留下一个「整体前进」的状态，让「更新」能接管（2c 的 A16）
        update_whitelist(repo, cfg, j)?;
    }
    if rebase_in_progress(repo)? {
        // A14：冲突期间锁是放开的，不是我们发起的 rebase 不碰
        require_our_rebase(repo, j)?;
        run_git(repo, &["rebase", "--abort"])?;
    }
    if current_branch(repo)?.as_deref() != Some(orig.as_str()) {
        // 能 -f 的前提：W-file 在 svn 看来全部干净 = 磁盘就是服务器内容，能取回（2c 的 A5）
        let dirty = dirty_w_files(repo, cfg)?;
        if !dirty.is_empty() {
            return Err(GitError::new(
                "sync_out_unsafe_to_return",
                format!(
                    "要回到分支 `{orig}` 就得覆盖工作区里的文件，而下面这些文件的内容既不是 git 写的、\
                     也不是服务器上的，不知道是谁改的，不能覆盖：\n{}\n记录已保留。请先确认这些改动\
                     （需要的话自己备份），再点「放弃」。",
                    list_paths(&dirty)
                ),
            ));
        }
        checkout_verified(repo, cfg, &orig, true)?;
    }
    if let Some(r) = &j.deliver_ref {
        unpin(repo, r)?;
    }
    svn_journal::remove(repo)?;
    Ok(SyncOutOutcome::Abandoned { committed: true })
}

/// `restoring-branch` 的继续：按 HEAD、原分支 OID 与 git 的 rebase 状态续 D8d 或进 D10。
/// 不回到 D6 / D8c——那会覆盖用户正在解决的冲突（§7.6）。
fn continue_restoring(repo: &str, cfg: &SvnConfig, j: &mut Journal) -> Result<SyncOutOutcome> {
    let orig = need(&j.orig_branch, "origBranch")?;
    let orig_head = need(&j.orig_head, "origHead")?;
    let new = published(repo, j)?;
    let rev = trailer_rev(repo, &new)?;
    if rebase_in_progress(repo)? {
        require_our_rebase(repo, j)?;
        let files = unmerged_paths(repo)?;
        if !files.is_empty() {
            return Err(unresolved(&files));
        }
        // GIT_EDITOR=true：非交互地接受 git 给的提交信息（2c 实测）
        if let Err(e) = run_git_env(
            repo,
            &["-c", "rebase.autoStash=false", "-c", "rebase.updateRefs=false", "rebase", "--continue"],
            &[("GIT_EDITOR", "true")],
        ) {
            return rebase_conflict_or_error(repo, rev, e);
        }
        return finish(repo, j, rev);
    }
    let on_orig = current_branch(repo)?.as_deref() == Some(orig.as_str());
    if on_orig && rev_parse(repo, "HEAD")? != orig_head {
        // D8d 已经做完（快进或 rebase 走完了），只差 D10
        if !is_ancestor(repo, &new, "HEAD")? {
            return Err(state_changed(
                "原分支还没有落到新基线上（rebase 可能在外面被中止了）；要停在这里，请点「放弃」",
            ));
        }
        return finish(repo, j, rev);
    }
    restore_branch(repo, cfg, j)
}

fn wrong_flow() -> GitError {
    GitError::new(
        "sync_out_wrong_flow",
        "这条记录是「更新」流程留下的，请用横幅上更新那一组按钮处理。",
    )
}
```

`capture_delivered` 与 `update_whitelist` 目前是私有函数，同一模块内可直接调用，无需改可见性。

- [ ] **Step 4: 跑测试确认通过**

Run：`cargo test --test svn_sync_out`
Expected: 全部 PASS（Windows 上含 Unity 用例）。

几处要留意：

- `every_crash_after_the_commit…` 的 `Step::Captured` 一项：重开时 `publishing-baseline` 已有 `newBaselineOid`，走「只补发布」分支。若改成在 `capture_delivered` 写 `captureTree` 之后、建 commit 之前崩（没有对应 Step），走的是「从 D7 重录」分支——那条路径没有专门测试，记为覆盖缺口。
- `a_file_held_open…` 若 `deliver` 返回的不是 `sync_in_checkout_failed` 而是 Ok（checkout 真的写进去了），说明这台机器上 share=Read 不挡写入——打印 `git checkout` 的 stderr 核对，记进决策日志，**不要**删掉这条测试，改用 share=None（0）重试并说明两者差异（2c 的 A15 记过 L1 / L1′ 两种表现）。

- [ ] **Step 5: 全量 Rust 验证并 Commit**

```bash
cargo fmt --check && cargo clippy --all-targets && cargo test
git add src-tauri/src/engine/svn_sync_out.rs src-tauri/src/engine/svn_sync_in.rs src-tauri/tests/svn_sync_out.rs
git commit -m "feat(svn): 交付中断后的继续与放弃——D5 前一律清理，D5 后只续不重提"
```

更新决策日志：B4 / B7 / B8 的实施；`post-commit` 放弃先跑 D6 的补足与理由（M2-E6）。

---

## Task 9：D6 / D7a 失败（提交已在服务器上）

代码已在 Task 5 / 8 写好（`update_whitelist`、`capture_delivered` 的失败都停在 `post-commit`，继续从 D6 起）。本 Task 只加测试，钉住 spec §13 的「D6 update 失败 / 冲突」与「纳入集合核对不通过」两行，以及必测 #65 后半、#70 的 D7a 一半。

**Files:**
- Test: `src-tauri/tests/svn_sync_out.rs`（追加）

- [ ] **Step 1: 写测试**

```rust
/// 用「更新」把同事提交的文件拉到 wc1（基线随之推进）。
fn update_from_server(fx: &SvnFixture) {
    let p = app_lib::engine::svn_sync_check::preview_sync_in(&fx.wc_str()).unwrap();
    app_lib::engine::svn_sync_in::run_sync_in(
        &fx.wc_str(),
        &app_lib::engine::svn_sync_in::SyncInExpect {
            orig_branch: p.orig_branch,
            orig_head: p.orig_head,
            baseline_oid: p.baseline_oid,
            confirmed_outside: p.outside_changes,
        },
    )
    .unwrap();
}

#[test]
fn a_failed_d6_is_retried_without_committing_again() {
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    let away = fx.path().join("repo-away");
    let err = deliver_probed(&fx, "mine", &|s, _| {
        if s == Step::Committed {
            std::fs::rename(fx.repo(), &away).unwrap();
        }
    })
    .unwrap_err();
    std::fs::rename(&away, fx.repo()).unwrap();
    assert_eq!(err.code, "sync_out_update_failed", "{}", err.message);
    assert!(err.message.contains("提交已经成功（r2）"), "{}", err.message);
    assert_eq!(phase(&fx), Phase::PostCommit);

    let out = continue_sync_out(&fx.wc_str()).unwrap();
    assert!(matches!(out, SyncOutOutcome::Delivered { rev: 2, .. }), "{out:?}");
    assert_eq!(server_head(&fx), "2", "不重复提交");
    assert_settled(&fx);
}

#[test]
fn a_d6_conflict_on_a_file_git_does_not_manage_is_handed_to_tortoisesvn() {
    // 必测 #65 后半（spec §8.2.1 末、§15 第 25 条）
    let fx = hybrid_with_colleague();
    fx.colleague_commits("Code/x.psd", "psd1\n");
    update_from_server(&fx);
    std::fs::write(fx.wc().join("Code/x.psd"), "psd LOCAL\n").unwrap();
    typical_work(&fx);
    let p = preview_sync_out(&fx.wc_str()).unwrap();
    assert_eq!(p.outside_changes, vec!["Code/x.psd".to_string()], "确认框要列出它");

    let err = run_sync_out_probed(&fx.wc_str(), &expect_from(&p, "mine"), &|s: Step, _: &str| {
        if s == Step::Committed {
            fx.colleague_commits("Code/x.psd", "psd colleague\n");
        }
    })
    .unwrap_err();
    assert_eq!(err.code, "sync_out_update_conflict", "{}", err.message);
    assert!(err.message.contains("Code/x.psd") && err.message.contains("TortoiseSVN"), "{}", err.message);
    assert_eq!(phase(&fx), Phase::PostCommit);

    // 用户在 TortoiseSVN 里解决（这里用命令行模拟），再点继续
    svn_in(&fx.wc(), &["resolve", "--accept", "working", "Code/x.psd"]);
    let out = continue_sync_out(&fx.wc_str()).unwrap();
    assert!(matches!(out, SyncOutOutcome::Delivered { .. }), "{out:?}");
    assert_eq!(std::fs::read_to_string(fx.wc().join("Code/x.psd")).unwrap(), "psd LOCAL\n");
}

#[test]
fn a_new_gitignore_from_the_server_stops_d7a_and_continue_picks_up_after_the_fix() {
    // 必测 #70 的 D7a 一半：保持已提交状态、不重复 D5；修正规则后从原录制阶段重新核对
    let fx = hybrid_with_colleague();
    typical_work(&fx);
    let err = deliver_probed(&fx, "mine", &|s, _| {
        if s == Step::Committed {
            fx.colleague_commits("Code/.gitignore", "*.cs\n");
            fx.colleague_commits("Code/New2.cs", "n2\n");
        }
    })
    .unwrap_err();
    assert_eq!(err.code, "include_set_mismatch", "{}", err.message);
    assert!(err.message.contains("Code/New2.cs"), "{}", err.message);
    assert_eq!(phase(&fx), Phase::PostCommit);
    let base_before = git(&fx, &["rev-parse", "svn/base"]);
    assert!(git(&fx, &["log", "-1", "--format=%B", "svn/base"]).contains("@1 "), "基线没动");

    // 修正规则：本地把 .gitignore 改空（它在 W-path 内、不是 W-file，svn 视为本地修改）
    std::fs::write(fx.wc().join("Code/.gitignore"), "").unwrap();
    let out = continue_sync_out(&fx.wc_str()).unwrap();
    assert!(matches!(out, SyncOutOutcome::Delivered { rev: 4, .. }), "{out:?}");
    assert_ne!(git(&fx, &["rev-parse", "svn/base"]), base_before);
    assert_eq!(git(&fx, &["show", "svn/base:Code/New2.cs"]), "n2");
    assert_eq!(server_head(&fx), "4", "r2 是我们的，r3 / r4 是同事的，没有第二次交付");
}
```

- [ ] **Step 2: 跑测试**

Run：`cargo test --test svn_sync_out`
Expected: 3 个用例 PASS。

- 若 `a_d6_conflict…` 的 D6 没有冲突（`x.psd` 被合并成功），说明 svn 把两行改动合并了：把同事的内容改成与本地完全不同的单行（已是），仍不冲突就打印 `svn update` 输出核对（实测 M2 之前 spec §8.2.1 记过重叠才冲突）。
- 若 `a_new_gitignore…` 报的不是 `include_set_mismatch` 而是别的码，核对 `record_server_tree_into` 的报错路径；**不要**为了让测试过而在 D7a 跳过核对。
- 若修正之后的 `continue` 仍报 `include_set_mismatch`：核对 git 是否按磁盘上的 `.gitignore` 判定（`git check-ignore --no-index -v Code/New2.cs`），把结果记进决策日志。

- [ ] **Step 3: Commit**

```bash
cargo fmt --check && cargo clippy --all-targets && cargo test
git add src-tauri/tests/svn_sync_out.rs
git commit -m "test(svn): 交付在 D6 / D7a 失败时停在已提交状态，修好后继续且不重复提交"
```

---

## Task 10：Tauri 命令

**Files:**
- Modify: `src-tauri/src/commands/svn_sync.rs`
- Modify: `src-tauri/src/commands/svn.rs`（`UnfinishedSync` 加两个可选字段）
- Modify: `src-tauri/src/lib.rs`（`invoke_handler` 里注册）
- Test: `src-tauri/tests/svn_backend.rs`、`src-tauri/tests/svn_write_guard.rs`

**Interfaces:**
- Produces（JS 侧调用形状，Task 11 的 store 按它写）：
  - `invoke('svn_sync_out_preview', { path })` → `SyncOutPreview`
  - `invoke('svn_sync_out', { path, expect })` → `SyncOutOutcome`，`expect` 是 `{ origBranch, origHead, baselineOid, deliverTree, changes, confirmedOutside, message }`
  - `invoke('svn_sync_out_continue', { path })` / `invoke('svn_sync_out_abort', { path })` → `SyncOutOutcome`
  - `svn_sync_state` 的 `unfinished` 多两个可选字段：`expectedStartRev?: number`、`commitMessage?: string`（计划 B4 的横幅要显示）

- [ ] **Step 1: 写失败测试**

`svn_backend.rs` 追加（照该文件已有的「序列化字段名」用例的写法）：

```rust
#[test]
fn sync_out_shapes_serialize_to_the_exact_names_the_frontend_reads() {
    use app_lib::engine::svn_deliver::LocalOnlyCommit;
    use app_lib::engine::svn_sync_out::{SyncOutExpect, SyncOutOutcome};
    use app_lib::engine::svn_sync_out_check::{DeliverChange, SyncOutPreview};

    let p = SyncOutPreview {
        orig_branch: "work".into(),
        orig_head: "h".into(),
        baseline_oid: "b".into(),
        baseline_rev: 1,
        remote_rev: 1,
        deliver_oid: "d".into(),
        deliver_tree: "t".into(),
        changes: vec![DeliverChange { status: "A".into(), path: "Code/N.cs".into() }],
        local_only: vec![LocalOnlyCommit { oid: "l".into(), subject: "mute".into() }],
        outside_changes: vec![],
    };
    let v = serde_json::to_value(&p).unwrap();
    let mut keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "baselineOid", "baselineRev", "changes", "deliverOid", "deliverTree", "localOnly",
            "origBranch", "origHead", "outsideChanges", "remoteRev",
        ]
    );
    assert_eq!(v["localOnly"][0]["subject"], "mute");

    let o = serde_json::to_value(SyncOutOutcome::Delivered { rev: 2, branch: "work".into(), local_only_left: 1 }).unwrap();
    assert_eq!(o, serde_json::json!({ "kind": "delivered", "rev": 2, "branch": "work", "localOnlyLeft": 1 }));
    let o = serde_json::to_value(SyncOutOutcome::RebaseConflict { rev: 3, files: vec!["a".into()] }).unwrap();
    assert_eq!(o, serde_json::json!({ "kind": "rebaseConflict", "rev": 3, "files": ["a"] }));
    let o = serde_json::to_value(SyncOutOutcome::Abandoned { committed: true }).unwrap();
    assert_eq!(o, serde_json::json!({ "kind": "abandoned", "committed": true }));

    let e: SyncOutExpect = serde_json::from_value(serde_json::json!({
        "origBranch": "work", "origHead": "h", "baselineOid": "b", "deliverTree": "t",
        "changes": [{ "status": "M", "path": "Code/A.cs" }],
        "confirmedOutside": [], "message": "m"
    }))
    .unwrap();
    assert_eq!(e.changes[0].path, "Code/A.cs");
}
```

`svn_write_guard.rs` 追加（照文件里 `fn_body` 的用法）：

```rust
/// 三个会写的交付命令都挂起 watcher 并在结束后补发 repo-changed（spec §5.2）。
#[test]
fn delivery_commands_run_with_the_watcher_suspended() {
    let src = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/svn_sync.rs")).unwrap();
    for func in ["svn_sync_out", "svn_sync_out_continue", "svn_sync_out_abort"] {
        let body = fn_body(&src, func);
        assert!(body.contains("with_watch_suspended("), "`{func}` 没有挂起 watcher");
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run：`cargo test --test svn_backend --test svn_write_guard`
Expected: `svn_write_guard` 失败（找不到 `svn_sync_out` 的函数体）；`svn_backend` 若 Task 1–8 已完成则直接通过——它守的是形状，先写也无妨。

- [ ] **Step 3: 实现**

`commands/svn_sync.rs` 追加（模块文档补一段 `sync_out` 的 invoke 形状说明）：

```rust
use crate::engine::svn_sync_out::{
    abort_sync_out, continue_sync_out, run_sync_out, SyncOutExpect, SyncOutOutcome,
};
use crate::engine::svn_sync_out_check::{preview_sync_out, SyncOutPreview};

/// 交付预览：前置检查与净变更清单，**什么都不改**（计划 B1）。
#[tauri::command(async)]
pub fn svn_sync_out_preview(path: String) -> Result<SyncOutPreview, GitError> {
    preview_sync_out(&path)
}

/// 执行一次交付。`expect` 是用户在确认框里看到并同意的预览与提交信息。
#[tauri::command(async)]
pub fn svn_sync_out(
    handle: AppHandle,
    path: String,
    expect: SyncOutExpect,
) -> Result<SyncOutOutcome, GitError> {
    with_watch_suspended(&handle, &path, || run_sync_out(&path, &expect))
}

/// 从中断处接着走（D5 之前一律是清理，计划 B8）。
#[tauri::command(async)]
pub fn svn_sync_out_continue(handle: AppHandle, path: String) -> Result<SyncOutOutcome, GitError> {
    with_watch_suspended(&handle, &path, || continue_sync_out(&path))
}

/// 放弃这次交付（D5 之后是「停止收尾」，计划 B7）。
#[tauri::command(async)]
pub fn svn_sync_out_abort(handle: AppHandle, path: String) -> Result<SyncOutOutcome, GitError> {
    with_watch_suspended(&handle, &path, || abort_sync_out(&path))
}
```

`commands/svn.rs` 的 `UnfinishedSync` 末尾加：

```rust
    /// 交付停在 `committing` 时，横幅要告诉用户去 TortoiseSVN 的 log 里从哪个 revision
    /// 之后找、找哪条提交信息（计划 B4）。其余 phase 也照发，前端按需用。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_start_rev: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_message: Option<String>,
```

`svn_sync_state` 里的构造补上 `expected_start_rev: j.expected_start_rev, commit_message: j.commit_message,`。

`lib.rs`：在注册 `svn_sync_in_preview` 等四个命令的地方紧接着加 `commands::svn_sync::svn_sync_out_preview, commands::svn_sync::svn_sync_out, commands::svn_sync::svn_sync_out_continue, commands::svn_sync::svn_sync_out_abort,`（写法照抄相邻几行）。

- [ ] **Step 4: 跑测试确认通过**

Run：`cargo test --test svn_backend --test svn_write_guard --test commands_are_async`
Expected: 全部 PASS。

- [ ] **Step 5: 全量 Rust 验证并 Commit**

```bash
cargo fmt --check && cargo clippy --all-targets && cargo test
git add src-tauri/src/commands/svn_sync.rs src-tauri/src/commands/svn.rs src-tauri/src/lib.rs src-tauri/tests/svn_backend.rs src-tauri/tests/svn_write_guard.rs
git commit -m "feat(svn): 交付的四个 Tauri 命令"
```

`pnpm tauri dev` 的实跑放在 Task 12（那时前端已接上，能真正点到）。

---

## Task 11：前端纯逻辑与流程

**Files:**
- Create: `src/engine/svnSyncOut.ts`
- Create: `src/components/svnSyncOutFlow.ts`
- Modify: `src/engine/svnJournal.ts`（`sync_out` 的横幅正文、按钮、note）
- Modify: `src/engine/svnSync.ts`（`BannerAction.id` 加三个值）
- Modify: `src/engine/toolbarActions.ts`（`showsSvnDeliver`）
- Modify: `src/store/repoStore.ts`（四个 invoke 包装）
- Test: `tests/vitest/svnSyncOut.test.ts`、`tests/vitest/svnSyncOutFlow.test.ts`（新）；`tests/vitest/svnJournal.test.ts`、`tests/vitest/toolbarActions.test.ts`（改）

**Interfaces:**
- Consumes: Task 10 的 invoke 形状。
- Produces（Task 12 用）：
  - `svnSyncOut.ts`：类型 `DeliverChange`、`LocalOnlyCommit`、`SyncOutPreview`、`SyncOutExpect`、`SyncOutOutcome`、`SyncOutPhase`、`SyncOutActionId = 'syncOutCleanup' | 'syncOutContinue' | 'syncOutAbort'`、`DeliverDialogModel`；函数 `expectFromDeliverPreview(p, message)`、`describeDeliverPreview(p)`、`canDeliver(message, outsideAcknowledged, model)`、`describeSyncOutOutcome(o)`、`describeSyncOutPhase(u)`、`syncOutActions(phase)`、`describeSyncOutAction(id, phase)`
  - `svnSyncOutFlow.ts`：`DeliverRequest { model; initialMessage; onMessageChange(m); action(message): Promise<void> }`、`createSvnSyncOutFlow(deps) → { handleSvnDeliver, handleSyncOutAction(id) }`
  - `repoStore.ts`：`svnSyncOutPreview(path)`、`svnSyncOut(path, expect)`、`svnSyncOutContinue(path)`、`svnSyncOutAbort(path)`
  - `toolbarActions.ts`：`showsSvnDeliver(caps): boolean`

- [ ] **Step 1: 写失败测试**

`tests/vitest/svnSyncOut.test.ts`：

```ts
import { describe, expect, it } from 'vitest'
import {
  canDeliver,
  describeDeliverPreview,
  describeSyncOutAction,
  describeSyncOutOutcome,
  describeSyncOutPhase,
  expectFromDeliverPreview,
  syncOutActions,
  type SyncOutPreview,
} from '@/engine/svnSyncOut'

const preview: SyncOutPreview = {
  origBranch: 'work',
  origHead: 'h'.repeat(40),
  baselineOid: 'b'.repeat(40),
  baselineRev: 7,
  remoteRev: 7,
  deliverOid: 'd'.repeat(40),
  deliverTree: 't'.repeat(40),
  changes: [
    { status: 'M', path: 'Code/A.cs' },
    { status: 'D', path: 'Code/B.cs' },
    { status: 'A', path: 'Code/Deep/New.cs' },
  ],
  localOnly: [],
  outsideChanges: [],
}

describe('describeDeliverPreview', () => {
  it('按新增 / 修改 / 删除分组并标出数量，空组不出现', () => {
    const m = describeDeliverPreview(preview)
    expect(m.groups).toEqual([
      { label: '新增 1', files: ['Code/Deep/New.cs'] },
      { label: '修改 1', files: ['Code/A.cs'] },
      { label: '删除 1', files: ['Code/B.cs'] },
    ])
    expect(m.summary).toContain('work')
    expect(m.summary).toContain('r7')
    expect(m.summary).toContain('不能撤回')
  })

  it('没有本地补丁时也要明说（spec §8.2.3 防漏标）', () => {
    const m = describeDeliverPreview(preview)
    expect(m.localOnly).toEqual([])
    expect(m.localOnlyNote).toContain('本次没有本地补丁被排除')
  })

  it('列出被摘掉的本地补丁', () => {
    const m = describeDeliverPreview({
      ...preview,
      localOnly: [{ oid: 'abcdef1234567890', subject: 'mute log' }],
    })
    expect(m.localOnly).toEqual(['abcdef1234 mute log'])
    expect(m.localOnlyNote).toContain('1 个本地补丁')
  })

  it('白名单里 git 管不到的修改：要警告，且必须勾选才能交付', () => {
    const m = describeDeliverPreview({ ...preview, outsideChanges: ['Code/x.psd'] })
    expect(m.outsideFiles).toEqual(['Code/x.psd'])
    expect(m.outsideWarning).toContain('TortoiseSVN')
    expect(canDeliver('修复', false, m)).toBe(false)
    expect(canDeliver('修复', true, m)).toBe(true)
  })
})

describe('canDeliver', () => {
  it('提交信息不能是空的或只有空白', () => {
    const m = describeDeliverPreview(preview)
    expect(canDeliver('', true, m)).toBe(false)
    expect(canDeliver('  \n ', true, m)).toBe(false)
    expect(canDeliver('修复', false, m)).toBe(true)
  })
})

describe('expectFromDeliverPreview', () => {
  it('预览里的事实原样带回去做锁内复核，提交信息原样（后端 trim）', () => {
    const e = expectFromDeliverPreview({ ...preview, outsideChanges: ['Code/x.psd'] }, ' 修复\n')
    expect(e).toEqual({
      origBranch: 'work',
      origHead: preview.origHead,
      baselineOid: preview.baselineOid,
      deliverTree: preview.deliverTree,
      changes: preview.changes,
      confirmedOutside: ['Code/x.psd'],
      message: ' 修复\n',
    })
  })
})

describe('describeSyncOutOutcome', () => {
  it('D9：还有本地补丁时绝不说「可以删除分支」', () => {
    const n = describeSyncOutOutcome({ kind: 'delivered', rev: 8, branch: 'work', localOnlyLeft: 2 })!
    expect(n.title).toContain('r8')
    expect(n.detail).toContain('2 个本地补丁')
    expect(n.detail).not.toContain('可以删除')
  })

  it('D9：没有本地补丁时提示可删除，但不自动删', () => {
    const n = describeSyncOutOutcome({ kind: 'delivered', rev: 8, branch: 'work', localOnlyLeft: 0 })!
    expect(n.detail).toContain('可以删除')
    expect(n.detail).toContain('不会自动删')
  })

  it('rebase 冲突：说清提交已成功、只剩补丁', () => {
    const n = describeSyncOutOutcome({ kind: 'rebaseConflict', rev: 8, files: ['Code/A.cs'] })!
    expect(n.detail).toContain('r8')
    expect(n.detail).toContain('Code/A.cs')
    expect(n.detail).toContain('继续')
  })

  it('放弃的两种结局说法不同', () => {
    expect(describeSyncOutOutcome({ kind: 'abandoned', committed: false })!.detail).toContain('SVN 侧没有任何改动')
    expect(describeSyncOutOutcome({ kind: 'abandoned', committed: true })!.detail).toContain('没有撤回')
  })
})

describe('横幅', () => {
  it('D5 之前只给「清理」；committing 不给任何按钮；D5 之后给继续与停止', () => {
    for (const p of ['prepare', 'building-deliver', 'deliver-built', 'checked-out', 'svn-staged']) {
      expect(syncOutActions(p).map((a) => a.id)).toEqual(['syncOutCleanup'])
    }
    expect(syncOutActions('committing')).toEqual([])
    for (const p of ['post-commit', 'publishing-baseline', 'restoring-branch']) {
      expect(syncOutActions(p).map((a) => a.id)).toEqual(['syncOutContinue', 'syncOutAbort'])
    }
    expect(syncOutActions('stashed')).toEqual([]) // 认不出的 phase 不给按钮
  })

  it('committing 的正文告诉用户去 log 里从哪个 revision 之后找什么（计划 B4）', () => {
    const d = describeSyncOutPhase({
      phase: 'committing',
      opid: 'op-1',
      expectedStartRev: 7,
      commitMessage: '修复跳跃',
    })!
    expect(d).toContain('r8')
    expect(d).toContain('修复跳跃')
    expect(d).toContain('mygitclient:opid=op-1')
  })

  it('D5 之后的「继续」明说不会重复提交，「停止」明说不撤回', () => {
    expect(describeSyncOutAction('syncOutContinue', 'post-commit').message).toContain('不会重复提交')
    expect(describeSyncOutAction('syncOutAbort', 'post-commit').message).toContain('不会撤回')
    expect(describeSyncOutAction('syncOutCleanup', 'svn-staged').message).toContain('撤销')
  })
})
```

`tests/vitest/svnSyncOutFlow.test.ts`：

```ts
import { beforeEach, describe, expect, it, vi } from 'vitest'

vi.mock('@/store/repoStore', () => ({
  svnSyncOutPreview: vi.fn(),
  svnSyncOut: vi.fn(),
  svnSyncOutContinue: vi.fn(),
  svnSyncOutAbort: vi.fn(),
}))

import * as store from '@/store/repoStore'
import {
  createSvnSyncOutFlow,
  type DeliverRequest,
  type SvnSyncOutFlowDeps,
} from '@/components/svnSyncOutFlow'
import type { ConfirmRequest } from '@/components/ConfirmDialog'
import type { SyncOutPreview } from '@/engine/svnSyncOut'

const preview: SyncOutPreview = {
  origBranch: 'work',
  origHead: 'h'.repeat(40),
  baselineOid: 'b'.repeat(40),
  baselineRev: 1,
  remoteRev: 1,
  deliverOid: 'd'.repeat(40),
  deliverTree: 't'.repeat(40),
  changes: [{ status: 'M', path: 'Code/A.cs' }],
  localOnly: [],
  outsideChanges: [],
}

function deps(log: string[], typed = '修复跳跃'): SvnSyncOutFlowDeps & { draft: { current: string } } {
  const draft = { current: '' }
  return {
    repoPath: 'C:/wc1',
    running: { current: false },
    draft,
    syncState: () => null,
    deliver: async (r: DeliverRequest) => {
      log.push(`deliver:initial=${r.initialMessage}`)
      r.onMessageChange(typed)
      try {
        await r.action(typed)
      } catch (e) {
        log.push(`dialogError:${(e as { code?: string }).code}`)
        return false // 用户看了错误之后关掉对话框
      }
      return true
    },
    confirm: async (r: ConfirmRequest) => {
      log.push(`confirm:${r.title}`)
      await r.action?.()
      return true
    },
    clearError: () => log.push('clearError'),
    reportError: (err) => log.push(`reportError:${(err as { code?: string }).code}`),
    refreshSvnSyncState: async () => {
      log.push('refreshSvnSyncState')
    },
    refreshSvnSync: async () => {
      log.push('refreshSvnSync')
    },
    fetchSvnLog: async () => {
      log.push('fetchSvnLog')
    },
  }
}

beforeEach(() => {
  for (const f of [store.svnSyncOutPreview, store.svnSyncOut, store.svnSyncOutContinue, store.svnSyncOutAbort]) {
    vi.mocked(f).mockReset()
  }
})

describe('handleSvnDeliver', () => {
  it('先清旧错误再预览；预览被拒直接报错，不弹对话框', async () => {
    const log: string[] = []
    vi.mocked(store.svnSyncOutPreview).mockRejectedValue({ code: 'sync_out_behind_baseline' })
    await createSvnSyncOutFlow(deps(log)).handleSvnDeliver()
    expect(log).toEqual(['clearError', 'reportError:sync_out_behind_baseline'])
  })

  it('成功：带着用户写的信息执行、刷新、清空草稿、弹 D9 提示', async () => {
    const log: string[] = []
    const d = deps(log)
    vi.mocked(store.svnSyncOutPreview).mockResolvedValue(preview)
    vi.mocked(store.svnSyncOut).mockResolvedValue({ kind: 'delivered', rev: 2, branch: 'work', localOnlyLeft: 0 })
    await createSvnSyncOutFlow(d).handleSvnDeliver()
    expect(vi.mocked(store.svnSyncOut).mock.calls[0][1].message).toBe('修复跳跃')
    expect(log).toContain('refreshSvnSyncState')
    expect(d.draft.current).toBe('')
    expect(log.at(-1)).toBe('confirm:已交付 r2')
  })

  it('执行失败（比如服务器在确认期间前进了）：刷新状态、保留提交信息（spec §13）', async () => {
    const log: string[] = []
    const d = deps(log)
    vi.mocked(store.svnSyncOutPreview).mockResolvedValue(preview)
    vi.mocked(store.svnSyncOut).mockRejectedValue({ code: 'sync_out_server_ahead' })
    await createSvnSyncOutFlow(d).handleSvnDeliver()
    expect(log).toContain('dialogError:sync_out_server_ahead')
    expect(log).toContain('refreshSvnSyncState')
    expect(d.draft.current).toBe('修复跳跃')

    // 下一次打开对话框，草稿原样带回来
    const log2: string[] = []
    const d2 = { ...deps(log2), draft: d.draft }
    vi.mocked(store.svnSyncOut).mockResolvedValue({ kind: 'delivered', rev: 3, branch: 'work', localOnlyLeft: 0 })
    await createSvnSyncOutFlow(d2).handleSvnDeliver()
    expect(log2).toContain('deliver:initial=修复跳跃')
  })

  it('同一时间只走一个流程', async () => {
    const log: string[] = []
    const d = deps(log)
    d.running.current = true
    await createSvnSyncOutFlow(d).handleSvnDeliver()
    expect(log).toEqual([])
  })
})

describe('handleSyncOutAction', () => {
  it('「清理」走放弃那个命令；「继续」走继续', async () => {
    const log: string[] = []
    vi.mocked(store.svnSyncOutAbort).mockResolvedValue({ kind: 'abandoned', committed: false })
    vi.mocked(store.svnSyncOutContinue).mockResolvedValue({ kind: 'delivered', rev: 2, branch: 'work', localOnlyLeft: 0 })
    const flow = createSvnSyncOutFlow(deps(log))
    await flow.handleSyncOutAction('syncOutCleanup')
    expect(store.svnSyncOutAbort).toHaveBeenCalledTimes(1)
    await flow.handleSyncOutAction('syncOutContinue')
    expect(store.svnSyncOutContinue).toHaveBeenCalledTimes(1)
  })
})
```

`tests/vitest/svnJournal.test.ts` 追加（并把原来断言 `NOTE_SYNC_OUT` 旧文案「恢复入口在后续版本提供」的用例改成新文案，记进决策日志）：

```ts
  it('交付留下的记录：标题、正文、按钮都按 sync_out 的 phase 给', () => {
    const b = describeUnfinishedSync({
      locked: false,
      holder: null,
      unfinished: { flow: 'sync_out', phase: 'svn-staged', opid: 'op', startedAt: 1 },
    })!
    expect(b.title).toBe('上次交付没有正常结束')
    expect(b.detail).toContain('撤销')
    expect(b.actions.map((a) => a.id)).toEqual(['syncOutCleanup'])
  })

  it('提交状态未知：换标题，不给按钮', () => {
    const b = describeUnfinishedSync({
      locked: false,
      holder: null,
      unfinished: {
        flow: 'sync_out',
        phase: 'committing',
        opid: 'op',
        startedAt: 1,
        expectedStartRev: 7,
        commitMessage: 'm',
      },
    })!
    expect(b.title).toBe('交付状态未知')
    expect(b.actions).toEqual([])
    expect(b.detail).toContain('r8')
  })
```

`tests/vitest/toolbarActions.test.ts` 追加：

```ts
  it('「交付」与「更新」一样只在 SVN 混合仓库上出现', () => {
    expect(showsSvnDeliver(NO_GIT_ONLY_CAPABILITIES)).toBe(true)
    expect(showsSvnDeliver(ALL_CAPABILITIES)).toBe(false)
  })
```

（`import` 里补 `showsSvnDeliver`，以及该文件尚未导入的 capabilities 常量。）

- [ ] **Step 2: 跑测试确认失败**

Run：`pnpm test`
Expected: 新文件因模块不存在而失败；`svnJournal` 的新用例失败。

- [ ] **Step 3: 实现**

`src/engine/svnSyncOut.ts`：

```ts
/**
 * 交付（sync_out）的纯展示逻辑：预览 → 交付对话框模型、结局 → 提示、横幅 → 按钮与确认框。
 *
 * 纯函数，无 React / Tauri 依赖（`AGENTS.md` §7.2）。每句文案只在这里写一份。
 *
 * ⚠️ 横幅与对话框都按纯文本渲染：不要写 `**`。
 */
import type { BannerAction } from './svnSync'
import type { OutcomeNotice, SyncConfirmModel } from './svnSyncIn'

/** 后端 `svn_sync_out_check::DeliverChange`。 */
export interface DeliverChange {
  status: 'A' | 'M' | 'D'
  path: string
}

/** 后端 `svn_deliver::LocalOnlyCommit`。 */
export interface LocalOnlyCommit {
  oid: string
  subject: string
}

/** 后端 `svn_sync_out_check::SyncOutPreview`。 */
export interface SyncOutPreview {
  origBranch: string
  origHead: string
  baselineOid: string
  baselineRev: number
  remoteRev: number
  deliverOid: string
  deliverTree: string
  changes: DeliverChange[]
  localOnly: LocalOnlyCommit[]
  outsideChanges: string[]
}

/** 后端 `svn_sync_out::SyncOutExpect`。字段名逐字对齐（`AGENTS.md` §7.3）。 */
export interface SyncOutExpect {
  origBranch: string
  origHead: string
  baselineOid: string
  deliverTree: string
  changes: DeliverChange[]
  confirmedOutside: string[]
  message: string
}

/** 后端 `svn_sync_out::SyncOutOutcome`。 */
export type SyncOutOutcome =
  | { kind: 'delivered'; rev: number; branch: string; localOnlyLeft: number }
  | { kind: 'rebaseConflict'; rev: number; files: string[] }
  | { kind: 'abandoned'; committed: boolean }

/** 后端 `svn_journal::Phase` 里交付流程的那几个（线序名，spec §7.6）。 */
export type SyncOutPhase =
  | 'prepare'
  | 'building-deliver'
  | 'deliver-built'
  | 'checked-out'
  | 'svn-staged'
  | 'committing'
  | 'post-commit'
  | 'publishing-baseline'
  | 'restoring-branch'

export type SyncOutActionId = 'syncOutCleanup' | 'syncOutContinue' | 'syncOutAbort'

export interface DeliverGroup {
  label: string
  files: string[]
}

export interface DeliverDialogModel {
  title: string
  summary: string
  groups: DeliverGroup[]
  /** 被摘掉的本地补丁，`<oid 前 10 位> <subject>`。 */
  localOnly: string[]
  /** 列表为空时也有话说（spec §8.2.3「漏标是主要人为风险」）。 */
  localOnlyNote: string
  /** 白名单里 git 管不到、本地改过、交付后会被更新碰到的文件。非空时必须勾选确认。 */
  outsideFiles: string[]
  outsideWarning?: string
  confirmLabel: string
  busyLabel: string
}

/** 预览里的事实原样带回去做锁内复核（计划 B1）。提交信息原样，trim 在后端做。 */
export function expectFromDeliverPreview(p: SyncOutPreview, message: string): SyncOutExpect {
  return {
    origBranch: p.origBranch,
    origHead: p.origHead,
    baselineOid: p.baselineOid,
    deliverTree: p.deliverTree,
    changes: [...p.changes],
    confirmedOutside: [...p.outsideChanges],
    message,
  }
}

const MAX_LISTED = 20

function listFiles(files: string[]): string {
  const shown = files.slice(0, MAX_LISTED).map((f) => `· ${f}`)
  if (files.length > MAX_LISTED) shown.push(`…还有 ${files.length - MAX_LISTED} 个`)
  return shown.join('\n')
}

export function describeDeliverPreview(p: SyncOutPreview): DeliverDialogModel {
  const pick = (s: DeliverChange['status']) => p.changes.filter((c) => c.status === s).map((c) => c.path)
  const groups: DeliverGroup[] = [
    { label: '新增', files: pick('A') },
    { label: '修改', files: pick('M') },
    { label: '删除', files: pick('D') },
  ]
    .filter((g) => g.files.length > 0)
    .map((g) => ({ label: `${g.label} ${g.files.length}`, files: g.files }))
  const n = p.localOnly.length
  return {
    title: '交付到 SVN',
    summary:
      `把分支 ${p.origBranch} 相对基线（r${p.baselineRev}）的 ${p.changes.length} 个文件变化提交到 SVN。` +
      '提交之后不能撤回，会直接影响全组。',
    groups,
    localOnly: p.localOnly.map((c) => `${c.oid.slice(0, 10)} ${c.subject}`),
    localOnlyNote:
      n === 0
        ? '本次没有本地补丁被排除。如果你有打算「仅本地」的改动却没在这里看到，说明它没有标记，会被一起交付。'
        : `下面 ${n} 个本地补丁（标了「仅本地，不交付」）不会被提交，交付之后它们仍留在你的分支上：`,
    outsideFiles: [...p.outsideChanges],
    outsideWarning:
      p.outsideChanges.length === 0
        ? undefined
        : '下面这些文件在白名单目录里，但 git 管不到（被扩展名排除），你本地改过。交付之后会把白名单目录' +
          '更新到最新，它们可能被合并；冲突时只能回 TortoiseSVN 处理，本应用没有它们的备份。' +
          '勾选只表示同意更新，不会还原它们：\n' +
          listFiles(p.outsideChanges),
    confirmLabel: '交付',
    busyLabel: '正在交付…',
  }
}

/** 交付按钮可不可以点。 */
export function canDeliver(message: string, outsideAcknowledged: boolean, m: DeliverDialogModel): boolean {
  return message.trim().length > 0 && (m.outsideFiles.length === 0 || outsideAcknowledged)
}

export function describeSyncOutOutcome(o: SyncOutOutcome): OutcomeNotice | null {
  switch (o.kind) {
    case 'delivered':
      // D9（spec §8.2.3）：分支上还有本地补丁时，它们是唯一副本，绝不能提示删除
      return {
        title: `已交付 r${o.rev}`,
        detail:
          o.localOnlyLeft > 0
            ? `分支 ${o.branch} 上还有 ${o.localOnlyLeft} 个本地补丁，没有随交付删除。它们只存在于这个分支上，请保留这个分支。`
            : `分支 ${o.branch} 的改动都已交付，可以删除这个分支了（不会自动删）。`,
      }
    case 'rebaseConflict':
      return {
        title: '交付成功，本地补丁落回时有冲突',
        detail:
          `提交已在服务器上（r${o.rev}），基线也已推进。只剩你的本地补丁没接到新基线上：\n${listFiles(o.files)}\n\n` +
          '在「本地改动」里改好这些文件并暂存，然后点顶部横幅上的「继续」。想先不管补丁，点「停止」。',
      }
    case 'abandoned':
      return o.committed
        ? {
            title: '已停止收尾',
            detail:
              '提交已在服务器上，没有撤回。分支回到了交付前的位置；顶部会提示基线需要更新，点「更新」即可把分支接到服务器的最新内容上。',
          }
        : {
            title: '已清理',
            detail: '这次交付在提交之前就停下了。已回到原分支，SVN 侧没有任何改动。',
          }
  }
}

/** 横幅正文需要的那部分未收尾记录。 */
export interface SyncOutUnfinished {
  phase: string
  opid: string
  expectedStartRev?: number
  commitMessage?: string
}

/** 每个 phase 回答同一个问题：我的提交现在在哪、我的文件现在在哪。认不出时返回 `undefined`。 */
export function describeSyncOutPhase(u: SyncOutUnfinished): string | undefined {
  switch (u.phase) {
    case 'prepare':
    case 'building-deliver':
    case 'deliver-built':
      return '交付在提交之前就停下了。磁盘上的文件和 SVN 都没有动过，只剩一个临时的交付标记要清掉。'
    case 'checked-out':
      return '交付在提交之前停下了。工作副本切到了交付用的版本（本地补丁暂时不在磁盘上），SVN 没有动过。清理会切回你的分支。'
    case 'svn-staged':
      return '交付在提交之前停下了。SVN 里登记了一些新增 / 删除，还没提交。清理会逐条撤销这些登记，再切回你的分支。'
    case 'committing': {
      const from = u.expectedStartRev === undefined ? '' : `r${u.expectedStartRev + 1} 之后`
      return (
        '提交已经发出，但没有拿到服务器的答复，可能成功也可能没有。本版本还不能自动查证，所以什么都没再动。' +
        `请在 TortoiseSVN 的 Show log 里看${from}有没有提交信息为「${u.commitMessage ?? ''}」的 revision` +
        `（它带着属性 mygitclient:opid=${u.opid}）。`
      )
    }
    case 'post-commit':
      return '提交已经成功，服务器上有了。还没把白名单更新到最新、也没推进基线。'
    case 'publishing-baseline':
      return '提交已经成功，新基线可能已经发布，你的分支还没接上去。'
    case 'restoring-branch':
      return '提交已经成功，基线已推进，正在把你的分支（和本地补丁）接到新基线上。如果提示冲突，改好并暂存后点继续。'
    default:
      return undefined
  }
}

const CLEANUP: BannerAction = { id: 'syncOutCleanup', label: '清理' }

/** 横幅上的按钮（计划 B4 / B8 的表）。 */
export function syncOutActions(phase: string): BannerAction[] {
  switch (phase) {
    case 'prepare':
    case 'building-deliver':
    case 'deliver-built':
    case 'checked-out':
    case 'svn-staged':
      return [CLEANUP]
    case 'post-commit':
    case 'publishing-baseline':
      return [
        { id: 'syncOutContinue', label: '继续交付' },
        { id: 'syncOutAbort', label: '停止收尾' },
      ]
    case 'restoring-branch':
      return [
        { id: 'syncOutContinue', label: '我已解决，继续' },
        { id: 'syncOutAbort', label: '停止收尾' },
      ]
    default:
      return [] // committing（3b 才能查证）与认不出的 phase
  }
}

export function describeSyncOutAction(id: SyncOutActionId, phase: string): SyncConfirmModel {
  if (id === 'syncOutCleanup') {
    return {
      title: '清理这次交付',
      message:
        phase === 'svn-staged'
          ? '逐条撤销 SVN 里的新增 / 删除登记，切回你的分支，删掉临时的交付标记。服务器上没有任何东西。'
          : '切回你的分支，删掉临时的交付标记。服务器上没有任何东西。',
      confirmLabel: '清理',
      busyLabel: '正在清理…',
    }
  }
  if (id === 'syncOutContinue') {
    return {
      title: '继续交付',
      message:
        phase === 'restoring-branch'
          ? '确认冲突文件都已改好并暂存。会接着把本地补丁接到新基线上，不会重复提交。'
          : '提交已经在服务器上。会接着更新白名单、推进基线、把分支接上去，不会重复提交。',
      confirmLabel: '继续',
      busyLabel: '正在继续…',
    }
  }
  return {
    title: '停止收尾',
    message:
      '提交已经在服务器上，不会撤回。只是不再替你做后面的事：分支回到交付前的位置' +
      (phase === 'restoring-branch' ? '（中止补丁的重放）' : '') +
      '。之后顶部会提示基线需要更新，点「更新」即可。',
    confirmLabel: '停止',
    busyLabel: '正在停止…',
    danger: true,
  }
}
```

`src/engine/svnSync.ts`：`BannerAction.id` 的联合类型末尾加 `| 'syncOutCleanup' | 'syncOutContinue' | 'syncOutAbort'`。

`src/engine/svnJournal.ts`：

- `UnfinishedSync` 改为：`phase: SyncPhase | SyncOutPhase`；加 `expectedStartRev?: number`、`commitMessage?: string`（后端 Task 10 的两个可选字段）。
- `import { describeSyncOutPhase, syncOutActions, type SyncOutPhase } from './svnSyncOut'`。
- `NOTE_SYNC_OUT` 改成：`'提交之前停下的，「清理」会把一切撤回原样；提交之后停下的，「继续」只会接着做完，绝不重复提交。'`
- `describeUnfinishedSync` 里，`if (!s.unfinished) return null` 之后、计算 `back` 之前插入：

```ts
  if (s.unfinished.flow === 'sync_out') {
    const u = s.unfinished
    return {
      tone: 'danger',
      title: u.phase === 'committing' ? '交付状态未知' : '上次交付没有正常结束',
      detail:
        describeSyncOutPhase(u) ??
        `交付停在「${u.phase}」这一步，这个版本的界面还不认识它。请不要手工去动分支指针，记录都还在。`,
      actions: syncOutActions(u.phase),
      note: NOTE_SYNC_OUT,
    }
  }
```

  其后原有的 `sync_in` 分支里 `PHASE_DETAIL[s.unfinished.phase]` 改成 `(PHASE_DETAIL as Record<string, string>)[s.unfinished.phase]`（联合类型变宽之后的索引；注释写明「`sync_out` 已在上面分流，这里只剩 `sync_in` 的 phase」），`isSyncIn` 判断随之去掉（恒为真）。

`src/engine/toolbarActions.ts`：

```ts
/** 「交付」（sync_out）按钮，与「更新」同一条件（由 RepoTab 编排，不进 `ToolbarAction`）。 */
export function showsSvnDeliver(caps: Capabilities): boolean {
  return caps.vcs === 'svn'
}
```

`src/store/repoStore.ts`（放在 `svnSyncAbort` 之后；文件是 CRLF，用 Edit 工具改）：

```ts
export function svnSyncOutPreview(path: string): Promise<SyncOutPreview> {
  return invoke<SyncOutPreview>('svn_sync_out_preview', { path })
}

/** `expect` 的字段名是 camelCase，与后端 `SyncOutExpect` 的 serde 约定对齐（AGENTS.md §7.3）。 */
export function svnSyncOut(path: string, expect: SyncOutExpect): Promise<SyncOutOutcome> {
  return invoke<SyncOutOutcome>('svn_sync_out', { path, expect })
}

export function svnSyncOutContinue(path: string): Promise<SyncOutOutcome> {
  return invoke<SyncOutOutcome>('svn_sync_out_continue', { path })
}

export function svnSyncOutAbort(path: string): Promise<SyncOutOutcome> {
  return invoke<SyncOutOutcome>('svn_sync_out_abort', { path })
}
```

（`import type { SyncOutExpect, SyncOutOutcome, SyncOutPreview } from '../engine/svnSyncOut'`。）

`src/components/svnSyncOutFlow.ts`：

```ts
/**
 * 「交付」与横幅上交付那一组按钮的交互流程（spec §8）。
 *
 * 与 `svnSyncFlow.ts` 同一结构：不依赖 React，依赖全部从参数进来，能在 node 环境里测先后顺序。
 * 区别是确认要用户写提交信息，所以对话框是 `DeliverDialog`，不是 `ConfirmDialog`。
 */
import {
  svnSyncOut,
  svnSyncOutAbort,
  svnSyncOutContinue,
  svnSyncOutPreview,
} from '../store/repoStore'
import {
  describeDeliverPreview,
  describeSyncOutAction,
  describeSyncOutOutcome,
  expectFromDeliverPreview,
  type DeliverDialogModel,
  type SyncOutActionId,
  type SyncOutOutcome,
} from '../engine/svnSyncOut'
import type { SyncState, SyncStateUnreadable } from '../engine/svnJournal'
import type { ConfirmRequest } from './ConfirmDialog'

/** 交付对话框的请求。`action` 抛错时对话框留在原地显示错误。 */
export interface DeliverRequest {
  model: DeliverDialogModel
  /** 上一次没交付成功时写下的提交信息（spec §13：保留用户输入的提交信息）。 */
  initialMessage: string
  onMessageChange: (message: string) => void
  action: (message: string) => Promise<void>
}

export interface SvnSyncOutFlowDeps {
  repoPath: string
  /** 是否已有一次流程在走。调用方用 `useRef` 持有，理由同 `svnSyncFlow`。 */
  running: { current: boolean }
  /** 提交信息草稿。调用方用 `useRef` 持有：对话框关了、下次打开还要在。 */
  draft: { current: string }
  syncState: () => SyncState | SyncStateUnreadable | null
  deliver: (request: DeliverRequest) => Promise<boolean>
  confirm: (request: ConfirmRequest) => Promise<boolean>
  clearError: () => void
  reportError: (err: unknown) => void
  refreshSvnSyncState: (path: string) => Promise<void>
  refreshSvnSync: (path: string) => Promise<void>
  fetchSvnLog: (path: string) => Promise<unknown>
}

export function createSvnSyncOutFlow(d: SvnSyncOutFlowDeps) {
  const afterSync = async () => {
    await d.refreshSvnSyncState(d.repoPath)
    await d.refreshSvnSync(d.repoPath)
    void d.fetchSvnLog(d.repoPath)
  }

  const showOutcome = async (o: SyncOutOutcome | null) => {
    const n = o && describeSyncOutOutcome(o)
    if (!n) return
    await d.confirm({ title: n.title, message: n.detail, confirmLabel: '知道了' })
  }

  const exclusive = async (run: () => Promise<void>) => {
    if (d.running.current) return
    d.running.current = true
    try {
      await run()
    } finally {
      d.running.current = false
    }
  }

  /** 「交付」：预览 → 对话框（写信息、确认）→ 执行。 */
  const svnDeliver = async () => {
    d.clearError()
    let preview
    try {
      preview = await svnSyncOutPreview(d.repoPath)
    } catch (err) {
      d.reportError(err)
      return
    }
    const box: { outcome: SyncOutOutcome | null } = { outcome: null }
    await d.deliver({
      model: describeDeliverPreview(preview),
      initialMessage: d.draft.current,
      onMessageChange: (m) => {
        d.draft.current = m
      },
      action: async (message) => {
        try {
          box.outcome = await svnSyncOut(d.repoPath, expectFromDeliverPreview(preview, message))
        } catch (err) {
          // 失败也可能留下了记录（D5 状态未知、D6 失败），横幅要立刻跟上
          await afterSync()
          throw err
        }
        d.draft.current = ''
        await afterSync()
      },
    })
    await showOutcome(box.outcome)
  }

  /** 横幅上交付那一组按钮。「清理」与「停止」都走放弃命令（计划 B8）。 */
  const syncOutAction = async (id: SyncOutActionId) => {
    d.clearError()
    const s = d.syncState()
    const phase = s && !('readError' in s) && s.unfinished ? s.unfinished.phase : 'prepare'
    const box: { outcome: SyncOutOutcome | null } = { outcome: null }
    await d.confirm({
      ...describeSyncOutAction(id, phase),
      action: async () => {
        box.outcome =
          id === 'syncOutContinue'
            ? await svnSyncOutContinue(d.repoPath)
            : await svnSyncOutAbort(d.repoPath)
        await afterSync()
      },
    })
    await showOutcome(box.outcome)
  }

  return {
    handleSvnDeliver: () => exclusive(svnDeliver),
    handleSyncOutAction: (id: SyncOutActionId) => exclusive(() => syncOutAction(id)),
  }
}
```

- [ ] **Step 4: 跑测试确认通过**

Run：`pnpm build && pnpm test`
Expected: `tsc` 通过；vitest 全部 PASS（新增约 20 条）。

- [ ] **Step 5: Commit**

```bash
git add src/engine/svnSyncOut.ts src/components/svnSyncOutFlow.ts src/engine/svnJournal.ts src/engine/svnSync.ts src/engine/toolbarActions.ts src/store/repoStore.ts tests/vitest/svnSyncOut.test.ts tests/vitest/svnSyncOutFlow.test.ts tests/vitest/svnJournal.test.ts tests/vitest/toolbarActions.test.ts
git commit -m "feat(svn): 交付的前端纯逻辑与流程编排"
```

---

## Task 12：交付对话框、接线、真实窗口冒烟

**Files:**
- Create: `src/components/DeliverDialog.tsx`
- Modify: `src/components/Toolbar.tsx`（「交付」按钮）
- Modify: `src/components/BaselineBanner.tsx`（交付那一组按钮的分发）
- Modify: `src/components/RepoTab.tsx`（`useDeliverDialog` + `createSvnSyncOutFlow`）
- Modify: `src/styles.css`（交付对话框的样式，放在 `.dlg-warning` 那一组附近；写计划时在第 876 行，以实际搜索结果为准）
- Test: `tests/vitest/DeliverDialog.test.tsx`

**Interfaces:**
- Consumes: Task 11 的 `DeliverRequest`、`canDeliver`、`createSvnSyncOutFlow`。
- Produces: `useDeliverDialog() → { deliver: (r: DeliverRequest) => Promise<boolean>, dialog: JSX.Element | null }`（与 `useConfirm` 同一用法）。

- [ ] **Step 1: 写失败测试**

`tests/vitest/DeliverDialog.test.tsx`：

```tsx
// @vitest-environment jsdom
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { useDeliverDialog } from '@/components/DeliverDialog'
import type { DeliverRequest } from '@/components/svnSyncOutFlow'
import { describeDeliverPreview, type SyncOutPreview } from '@/engine/svnSyncOut'

type Deliver = (r: DeliverRequest) => Promise<boolean>

function mount(): () => Deliver {
  let current: Deliver | null = null
  function Harness() {
    const { deliver, dialog } = useDeliverDialog()
    current = deliver
    return <>{dialog}</>
  }
  render(<Harness />)
  return () => {
    if (!current) throw new Error('Harness 还没渲染')
    return current
  }
}

const preview: SyncOutPreview = {
  origBranch: 'work',
  origHead: 'h'.repeat(40),
  baselineOid: 'b'.repeat(40),
  baselineRev: 1,
  remoteRev: 1,
  deliverOid: 'd'.repeat(40),
  deliverTree: 't'.repeat(40),
  changes: [{ status: 'M', path: 'Code/A.cs' }],
  localOnly: [{ oid: 'abcdef1234567890', subject: 'mute log' }],
  outsideChanges: [],
}

function request(over: Partial<DeliverRequest> = {}): DeliverRequest {
  return {
    model: describeDeliverPreview(preview),
    initialMessage: '',
    onMessageChange: () => {},
    action: async () => {},
    ...over,
  }
}

const deliverButton = () => screen.getByRole('button', { name: '交付' }) as HTMLButtonElement
const messageBox = () => screen.getByLabelText('提交信息') as HTMLTextAreaElement

describe('DeliverDialog', () => {
  it('没写提交信息时「交付」点不了；写了就能点，草稿同步给流程', () => {
    const deliver = mount()
    const onMessageChange = vi.fn()
    act(() => {
      void deliver()(request({ onMessageChange }))
    })
    expect(deliverButton().disabled).toBe(true)
    fireEvent.change(messageBox(), { target: { value: '修复跳跃' } })
    expect(deliverButton().disabled).toBe(false)
    expect(onMessageChange).toHaveBeenLastCalledWith('修复跳跃')
  })

  it('列出被摘掉的本地补丁与文件清单', () => {
    const deliver = mount()
    act(() => {
      void deliver()(request())
    })
    expect(screen.getByText('abcdef1234 mute log')).toBeTruthy()
    expect(screen.getByText('Code/A.cs')).toBeTruthy()
    expect(screen.getByText(/1 个本地补丁/)).toBeTruthy()
  })

  it('有 git 管不到的修改时，不勾选就不能交付', () => {
    const deliver = mount()
    act(() => {
      void deliver()(request({
        model: describeDeliverPreview({ ...preview, outsideChanges: ['Code/x.psd'] }),
        initialMessage: '修复',
      }))
    })
    expect(deliverButton().disabled).toBe(true)
    fireEvent.click(screen.getByRole('checkbox'))
    expect(deliverButton().disabled).toBe(false)
  })

  it('上次没交付成功写下的信息原样带回来', () => {
    const deliver = mount()
    act(() => {
      void deliver()(request({ initialMessage: '上次写的' }))
    })
    expect(messageBox().value).toBe('上次写的')
  })

  it('执行失败：框留在原地显示错误，信息不丢，按钮可以再点', async () => {
    const deliver = mount()
    act(() => {
      void deliver()(request({
        initialMessage: '修复',
        action: async () => {
          throw { code: 'sync_out_server_ahead', message: '服务器上已经有 r2' }
        },
      }))
    })
    fireEvent.click(deliverButton())
    await waitFor(() => expect(screen.getByText(/服务器上已经有 r2/)).toBeTruthy())
    expect(messageBox().value).toBe('修复')
    expect(deliverButton().disabled).toBe(false)
  })

  it('Ctrl+Enter 交付；成功后框关掉，Promise 给 true', async () => {
    const deliver = mount()
    const action = vi.fn(async () => {})
    let result!: Promise<boolean>
    act(() => {
      result = deliver()(request({ initialMessage: '修复', action }))
    })
    fireEvent.keyDown(messageBox(), { key: 'Enter', ctrlKey: true })
    await expect(result).resolves.toBe(true)
    expect(action).toHaveBeenCalledWith('修复')
    expect(screen.queryByLabelText('提交信息')).toBeNull()
  })

  it('Esc 关闭，Promise 给 false', async () => {
    const deliver = mount()
    let result!: Promise<boolean>
    act(() => {
      result = deliver()(request())
    })
    fireEvent.keyDown(window, { key: 'Escape' })
    await expect(result).resolves.toBe(false)
  })
})
```

- [ ] **Step 2: 跑测试确认失败**

Run：`pnpm test`
Expected: `DeliverDialog` 模块不存在。

- [ ] **Step 3: 实现 `DeliverDialog.tsx`**

```tsx
import { useCallback, useEffect, useRef, useState } from 'react'
import { toErrorMessage } from '../engine/errors'
import { canDeliver } from '../engine/svnSyncOut'
import type { DeliverRequest } from './svnSyncOutFlow'

interface Pending extends DeliverRequest {
  resolve: (ok: boolean) => void
  /** 每个请求一个，做 `<Dialog>` 的 key，理由见 `useConfirm`。 */
  id: number
}

function Dialog({ request, onDone }: { request: Pending; onDone: () => void }) {
  const m = request.model
  const [message, setMessage] = useState(request.initialMessage)
  const [ack, setAck] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const textRef = useRef<HTMLTextAreaElement>(null)
  const ready = canDeliver(message, ack, m) && !busy

  const finish = (ok: boolean) => {
    request.resolve(ok)
    onDone()
  }

  const accept = async () => {
    if (!ready) return
    setBusy(true)
    setError(null)
    try {
      await request.action(message)
      finish(true)
    } catch (err) {
      // 留在原地：提交信息还在框里，用户看完错误可以关掉去「更新」，回来时草稿还在
      setError(toErrorMessage(err))
      setBusy(false)
    }
  }

  // 焦点落进输入框：打开就能写提交信息
  useEffect(() => {
    textRef.current?.focus()
  }, [])

  useEffect(() => {
    // 交付跑起来之后 Esc 不再关闭：关掉只会让用户以为取消了，而 svn commit 可能已经发出
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !busy) finish(false)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [busy])

  return (
    <div className="dialog-overlay" onMouseDown={() => { if (!busy) finish(false) }}>
      <div className="dialog dialog-wide deliver-dialog" role="dialog" aria-label={m.title} onMouseDown={(e) => e.stopPropagation()}>
        <div className="dialog-title">{m.title}</div>
        <div className="dialog-message">{m.summary}</div>

        <div className="deliver-files">
          {m.groups.map((g) => (
            <div key={g.label}>
              <div className="deliver-group">{g.label}</div>
              <ul>
                {g.files.map((f) => (
                  <li key={f}>{f}</li>
                ))}
              </ul>
            </div>
          ))}
        </div>

        <div className="deliver-note">{m.localOnlyNote}</div>
        {m.localOnly.length > 0 && (
          <ul className="deliver-local-only">
            {m.localOnly.map((c) => (
              <li key={c}>{c}</li>
            ))}
          </ul>
        )}

        {m.outsideWarning && (
          <div className="dlg-warning">
            {m.outsideWarning}
            <label className="deliver-ack">
              <input type="checkbox" checked={ack} disabled={busy} onChange={(e) => setAck(e.target.checked)} />
              我知道这些文件可能被合并，继续交付
            </label>
          </div>
        )}

        <label className="dlg-field-label" htmlFor="deliver-message">提交信息</label>
        <textarea
          id="deliver-message"
          className="deliver-message"
          ref={textRef}
          rows={4}
          value={message}
          disabled={busy}
          onChange={(e) => {
            setMessage(e.target.value)
            request.onMessageChange(e.target.value)
          }}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
              e.preventDefault()
              void accept()
            }
          }}
        />

        {error && <div className="dlg-warning">{error}</div>}
        {busy && (
          <div className="dlg-progress">
            <span className="dlg-spinner" />
            {m.busyLabel}
          </div>
        )}

        <div className="dialog-actions">
          <button disabled={busy} onClick={() => finish(false)}>Cancel</button>
          <button className="primary danger" disabled={!ready} onClick={() => void accept()}>
            {busy ? '…' : m.confirmLabel}
          </button>
        </div>
      </div>
    </div>
  )
}

/** 交付确认框。用法与 `useConfirm` 相同；要输入提交信息，所以单独一个组件。 */
export function useDeliverDialog() {
  const [pending, setPending] = useState<Pending | null>(null)
  const seq = useRef(0)

  const deliver = useCallback(
    (request: DeliverRequest) =>
      new Promise<boolean>((resolve) => setPending({ ...request, resolve, id: ++seq.current })),
    [],
  )

  // key 与「只关自己」的理由同 useConfirm（2026-09-28 E2E 场景 2、PR #49 Codex review #5）
  const dialog = pending ? (
    <Dialog
      key={pending.id}
      request={pending}
      onDone={() => setPending((p) => (p?.id === pending.id ? null : p))}
    />
  ) : null

  return { deliver, dialog }
}
```

样式（加进 `src/styles.css` 的 `.dlg-warning` 那一组附近；颜色、字体变量沿用该文件已有的）：

```css
.deliver-files {
  max-height: 220px;
  overflow-y: auto;
  margin: 8px 0;
  font-family: var(--font-mono, monospace);
  font-size: 12px;
}
.deliver-files ul,
.deliver-local-only {
  margin: 2px 0 6px;
  padding-left: 18px;
}
.deliver-group {
  font-weight: 600;
  margin-top: 4px;
}
.deliver-note {
  margin-top: 6px;
  font-size: 12px;
}
.deliver-ack {
  display: flex;
  gap: 6px;
  align-items: center;
  margin-top: 6px;
}
.deliver-message {
  width: 100%;
  box-sizing: border-box;
  resize: vertical;
  font: inherit;
}
```

- [ ] **Step 4: 接线**

- `Toolbar.tsx`：props 加 `onSvnDeliver?: () => Promise<void>`；照「更新」按钮（`Toolbar.tsx:77` 起，`showsSvnUpdate(capabilities) && onSvnUpdate && (...)`）的写法紧挨着加一个「交付」按钮，条件 `showsSvnDeliver(capabilities) && onSvnDeliver`，自己的 `delivering` 状态与 `.finally(() => setDelivering(false))`。
- `BaselineBanner.tsx`：props 加 `onSyncOutAction: (id: SyncOutActionId) => void`；在分发按钮的地方（`a.id === 'svnUpdate'` 那一串，`BaselineBanner.tsx:159` 附近）加：`else if (a.id === 'syncOutCleanup' || a.id === 'syncOutContinue' || a.id === 'syncOutAbort') onSyncOutAction(a.id)`。文件是 CRLF。
- `RepoTab.tsx`：
  - `const { deliver, dialog: deliverDialog } = useDeliverDialog()`、`const deliverDraft = useRef('')`。
  - 在 `createSvnSyncFlow({...})`（`RepoTab.tsx:557` 起）之后照抄一份 `createSvnSyncOutFlow({...})`。**`running` 传同一个 ref**：更新与交付同一时间只能走一个，两个确认框会互相顶替（PR #49 Codex review #5 的同一个坑）。`draft: deliverDraft`、`deliver`，其余依赖与更新那份相同。
  - `<Toolbar ... onSvnDeliver={handleSvnDeliver} />`；`<BaselineBanner ... onSyncOutAction={(id) => void handleSyncOutAction(id)} />`；在渲染 `confirmDialog` 的地方旁边渲染 `{deliverDialog}`。

Run：`pnpm build && pnpm test`。Expected：全部通过。

- [ ] **Step 5: 真实窗口冒烟（只对 fixture 生成的临时仓库）**

```powershell
# 应用窗口（在本 worktree 里）
pnpm tauri dev
```

```powershell
# 操作窗口：造临时混合仓库（脚本输出 wc1 / wc2 的路径）
pwsh -File scripts\e2e-svn-fixture.ps1
```

在应用里打开脚本给出的 `wc1`，然后：

1. 改 `Code` 下一个 `.cs`、在提交框里提交；再改另一处、勾「仅本地，不交付」提交。
2. 点工具栏「交付」：对话框列出 1 个修改、1 个本地补丁；不写信息时「交付」灰着。
3. 写信息、点交付：结束后弹「已交付 r…」，正文说还有 1 个本地补丁、请保留分支；没有任何横幅。
4. 操作窗口里 `svn log -v -l 1 <wc1>`：只有那一个文件，信息原样；`svn cat` 看内容里没有本地补丁。
5. 采集视觉证据（`AGENTS.md` §6.1）：对话框在默认窗口尺寸与把窗口拉窄到 800px 宽时的截图；文件清单超过 20 条时的滚动（再造一次多文件交付）。截图路径与观察记进决策日志。

四个新 invoke（预览、执行、继续、放弃）里，第 3 步走通证明前两个接对了；继续 / 放弃在 Task 13 的 E2E 场景里验证。**任何一步不符合就停下修，不要带着已知问题进 Task 13。**

- [ ] **Step 6: Commit**

```bash
git add src/components/DeliverDialog.tsx src/components/Toolbar.tsx src/components/BaselineBanner.tsx src/components/RepoTab.tsx src/styles.css tests/vitest/DeliverDialog.test.tsx
git commit -m "feat(svn): 工具栏「交付」与交付确认框"
```

---

## Task 13：E2E 清单、spec 修订、收尾

**Files:**
- Create: `docs/superpowers/e2e/2026-10-07-svn-phase3a-e2e.md`
- Modify: `docs/superpowers/specs/2026-09-12-svn-git-hybrid-design.md`
- Modify: `docs/superpowers/plans/2026-10-07-svn-phase3a-sync-out-decisions.md`

- [ ] **Step 1: 写 E2E 清单**

结构照抄 `docs/superpowers/e2e/2026-09-25-svn-phase2c-e2e.md`（「这一期验的是什么」「状态」表、两个 PowerShell 窗口、`scripts\e2e-svn-fixture.ps1` 造 fixture、每个场景都给能原样粘贴的命令）。场景：

1. 普通交付：改 / 增 / 删各一，新增文件放在两层新目录里 → 服务器一个 revision 五条路径；分支快进到新基线；没有横幅。
2. 本地补丁：同一个文件里既有补丁又有待交付改动（补丁夹在中间，走慢路径）→ 服务器上没有补丁、磁盘上补丁还在、「已交付」提示说保留分支。
3. 补丁与待交付改动紧挨着 → 预览就拒绝，指名两个提交；什么都没动。
4. 同事在你开着交付框时提交了 → 点交付被拒（服务器已前进），关掉框、点「更新」、再点「交付」，**提交信息还在**。
5. 白名单目录里的 `.psd` 本地改过 → 交付框红色警告 + 勾选框，不勾不能交付。
6. D8d 冲突：用操作窗口在你交付的同时让同事改补丁那一行（交付框开着时先别点，按清单里的命令准备好同事的改动，点交付后立刻在操作窗口提交）→ 弹「交付成功，本地补丁落回时有冲突」→ 改好、暂存 → 横幅「我已解决，继续」→ 完成。若时间窗口太难卡，改用「在 D6 之前让同事提交」的等价做法：在操作窗口里先跑一个循环脚本，检测到 wc1 的 svn 日志出现你的提交就立即让 wc2 提交（脚本写进清单）。
7. 崩溃恢复：交付进行中（对话框显示「正在交付…」时）在任务管理器里结束 mygitclient 进程 → 重开 → 横幅显示「上次交付没有正常结束」及对应按钮 → 按按钮收尾。记录实际停在哪个 phase。

每个场景写「期望看到」与「验证命令」。状态表初始全部不勾（「没跑的别勾」）。

- [ ] **Step 2: spec 修订**

- §8.2.3「慢路径」那段之后加一段注记：实现改为对象层 `merge-tree` 重放（计划 B2），理由与实测 M1；`tempWorktree` 字段保留但不再写入。
- §8.2 的 D3 / D3.5 旁加注记：确认在流程之外、执行时锁内重算（计划 B1 / B13）。
- §8.6 表格之后加「失败分类」一段：B3 的错误码集合与「其余一律未知」的规则，实测 M2。
- §8.2.2 D8d 加注记：无 Local-Only 时 CAS 移动分支指针再切换（计划 B10），快进断言改为显式 `--is-ancestor`。
- §7.6 `sync_out` 的 phase 表下补：`svnStatusBefore` / `svnDelta` / `svnModified` / `deliverOid` / `committedRev` 字段；`post-commit` 放弃先跑 D6（计划 B7 的补足）。
- §18 新增一条修订记录：`2026-10-07（十五）· Phase 3a 施工：sync_out 的实测补足`，列 B1–B15 的要点与 M1 / M2。

- [ ] **Step 3: 全量验证**

在 worktree 根目录：

```powershell
cd src-tauri; cargo fmt --check; cargo clippy --all-targets; cargo test --no-fail-fast; cd ..
pnpm build
pnpm test
```

把每条命令的结果（通过 / 数量 / 失败原文）填进决策日志的「全量验证」表。**全部通过才进 Step 4。**

- [ ] **Step 4: 独立 review**

按 `AGENTS.md` §4.1 / §13，由独立于实现者的 agent 做一次全分支 review（`/code-review` 或 Codex），review 输入包括本计划与决策日志（§13 第 12 条：**先读「偏离计划」再看 diff**）。高危问题修完、补测试、重跑 Step 3，再进 Step 5。

- [ ] **Step 5: 收尾**

- 决策日志进度总账全部更新；「本期不做」原样抄进决策日志末尾。
- 本分支是未合并的 feature worktree：Step 3 全绿之后 commit 并 push 属常规操作（用户全局规则）。**开 PR 前先问用户**；用户同意后 `gh pr create --draft`，PR 描述里条目编号写成「第 N 条 / 偏离 B3」，不写裸 `#数字`。
- 向用户报告：改了什么、跑了哪些命令与结果、遗留风险（至少：B4 的 `committing` 死胡同要等 3b；pinned 拒绝没有专门测试；「删目录」在服务器上留下空目录；`publishing-baseline` 无 OID 分支没有专门测试）、review 发现并修复的问题。

---

## 进度总账

执行时以决策日志里的进度总账为准（Task 0 建）。本节不重复维护。

## 全量验证

| 命令 | 结果 |
|---|---|
| `cargo fmt --check` | |
| `cargo clippy --all-targets` | |
| `cargo test --no-fail-fast` | |
| `pnpm build` | |
| `pnpm test` | |
| `pnpm tauri dev` 冒烟（Task 12 Step 5） | |
| E2E 7 个场景（Task 13） | |

执行者在决策日志里填这张表；本计划里保持空白。

---

## 本期不做（review 时不要当成遗漏）

| 项 | 归属 | 说明 |
|---|---|---|
| 断网查证（opid revprop、changed-paths + 内容比对兜底，spec §8.6.2） | Phase 3b | 3a 里 `committing` 的继续 / 放弃都拒绝（B4） |
| revprop 开仓自检、`supportsRevprop` 写配置（§8.6.3 形态 A） | Phase 3b | |
| 服务器拒绝幂等键时由用户确认重试（§8.6.3 形态 B，必测 #39） | Phase 3b | 3a 里钩子拒绝按普通 `E165001` 回滚并原样展示 stderr |
| 必测 #14、#33、#34、#35、#66 | Phase 3b | 都依赖查证 |
| D9 的「移到 `local/<name>` 分支」入口 | 之后 | 用户 2026-10-07 决定；本期只给文字提示 |
| pinned 模式下的交付（必测 #53、#55） | 与 pinned 的 `sync_in` 一起 | B12 |
| 跨多个同仓库 WC 的交付原子性（必测 #54） | 之后 | 现有 fixture 没有「项目根 + 同仓库 external 同在白名单」的形态；`delivery_works_when_the_whitelist_is_an_external` 只覆盖 external 单独作白名单 |
| 交付后删掉服务器上变空的目录 | 之后 | Task 4 的已知限制 |
| 「隔离的临时 SVN 工作副本里提交」以根治 D4.9 之后的窄竞态 | 不做 | spec §8.2.5 / §15 第 15 条的已知限制 |


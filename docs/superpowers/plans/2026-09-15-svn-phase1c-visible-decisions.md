# Phase 1c 实现日志：看得见那半

> 配套 `2026-09-15-svn-phase1c-visible.md`。按 `AGENTS.md` §2.1 建立。
> **review 请先读每个 Task 的「偏离计划」再看 diff**（§13 rubric #12）。

本文件只有两部分：**进度总账**（每完成一个 Task 立即更新）与**决策条目**
（按 Task 分节，四类缺一不可）。

---

## 进度总账

| Task | 状态 | 产出文件 | 验证结果 |
|---|---|---|---|
| 0 开工自检与基线数字 | ✅ 完成 | 本文件 | 改动前基线（本 worktree 内实测）：Rust **412 passed / 0 failed / 33 个二进制**、前端 `pnpm test` **290 passed / 22 files**、`pnpm build` ✓。三项与计划预期一字不差 |
| 1 `svn_xml` 加 log / ls 解析器 | ✅ 完成 | `src-tauri/src/engine/svn_xml.rs`、`src-tauri/tests/svn_xml.rs` | `cargo test --test svn_xml` **37 passed**（基线 27 + 新增 10）；`cargo fmt --check` ✓、`cargo clippy --all-targets` **0 warning**；两轮变异测试（详见下节） |
| 2 `svn_log` 拉取编排 | ✅ 完成 | `src-tauri/src/engine/svn_log.rs`（新）、`engine/mod.rs`、`tests/svn_log.rs`（新）、`tests/common/svn_fixtures.rs` | `cargo test --test svn_log` **11 passed**；`cargo fmt --check` ✓、`cargo clippy --all-targets` **0 warning**；变异测试（删钳制）3 条变红并报 `E160006` |
| 3 `svn_log_cache` 缓存与合并 | ⬜ 未开始 | | |
| 4 `svn_sync` 失同步判定 | ✅ 完成 | `engine/svn_sync.rs`（新）、`engine/mod.rs`、`tests/svn_sync.rs`（新）、`tests/common/svn_fixtures.rs`（+3 个构造） | `cargo test --test svn_sync` **8 passed**；`cargo fmt --check` ✓、`cargo clippy --all-targets` **0 warning**；实现第一版就撞上 `wc_root` 语义那个坑，被测试当场抓住（详见下节） |
| 5 基线体检按 WC 分组（§6.6.5 欠账） | ✅ 完成 | `engine/svn_health.rs`、`tests/svn_health.rs`、`tests/svn_init.rs` | `cargo test --test svn_health` **20 passed**；**全量 460 passed / 0 failed / 36 个二进制**（= 基线 412 + 新增 48，二进制 33 + 3）；`cargo fmt --check` ✓、`cargo clippy --all-targets` **0 warning** |
| 6 服务器侧清单证伪漏收（NB-2 欠账） | ✅ 完成 | `engine/svn_server_set.rs`（新）、`engine/svn_include_check.rs`（提出 `is_w_file` / `normalize_roots`）、`engine/svn_health.rs`、`engine/mod.rs`、`tests/svn_server_set.rs`（新）、`tests/common/svn_fixtures.rs` | `cargo test --test svn_server_set` **8 passed**（+ svn_health 20、svn_exclude 25 无回归）；`cargo fmt --check` ✓、`cargo clippy --all-targets` **0 warning**；变异测试抓出一条**测试覆盖不足**（详见下节） |
| 7 Tauri 命令层与 `Capabilities.vcs` | ✅ 完成 | `commands/svn.rs`、`lib.rs`、`vcs/mod.rs`、`vcs/svn.rs`、`engine/svn_log.rs`（+编排层）、`engine/svn_server_set.rs`、`engine/svn_sync.rs`、`src/engine/capabilities.ts`、`tests/svn_backend.rs`、`tests/vitest/capabilities.test.ts` | Rust **全量 476 passed / 0 failed / 37 个二进制**；前端 **291 passed / 22 files**、`pnpm build` ✓；`cargo fmt --check` ✓、`cargo clippy --all-targets` **0 warning** |
| 8 前端接合逻辑 `svnGraph.ts` | ✅ 完成 | `src/engine/svnGraph.ts`（新）、`src/engine/types.ts`、`tests/vitest/svnGraph.test.ts`（新） | `pnpm test` **311 passed / 23 files**（新增 20）、`pnpm build` ✓ |
| 9 前端展示逻辑 `svnSync.ts` | ✅ 完成 | `src/engine/svnSync.ts`（新）、`tests/vitest/svnSync.test.ts`（新） | **323 passed / 24 files**（新增 12）、`pnpm build` ✓ |
| 10 `BaselineBanner` 与 store 接线（NB-6 欠账） | ✅ 完成 | `src/components/BaselineBanner.tsx`（新）、`src/store/repoStore.ts`、`src/components/RepoTab.tsx`、`src/styles.css` | **323 passed**、`pnpm build` ✓ |
| 11 混合图接线与分页 | ✅ 完成 | `CommitRow.tsx`、`CommitDetailView.tsx`、`GraphView.tsx`、`RepoTab.tsx`、`styles.css` | **323 passed**、`pnpm build` ✓ |
| 12 本地补丁标记 | ✅ 完成 | `src/engine/commitActions.ts`、`LocalChangesView.tsx`、`RepoTab.tsx`、`styles.css`、`tests/vitest/commitActions.test.ts` | **327 passed**（新增 4）、`pnpm build` ✓ |
| 13 基线分支 UI 保护 | ✅ 完成 | `src/engine/capabilities.ts`、`BranchView.tsx`、`LocalChangesView.tsx`、`RepoTab.tsx`、`styles.css`、`tests/vitest/capabilities.test.ts` | **329 passed**（新增 2）、`pnpm build` ✓ |
| 14 收尾验证 | 🟡 自动化部分完成 | 本文件、`docs/superpowers/e2e/2026-09-15-svn-phase1c-e2e.md`（新） | Rust **476 passed / 0 failed / 37 个二进制**、`cargo fmt --check` ✓、`cargo clippy --all-targets` **0 warning**；前端 **329 passed / 24 files**、`pnpm build` ✓；零 SVN 写操作核对通过；命令对账 **56 == 56**。**真实窗口的 10 个场景未执行**，见下 |

---

## 开工自检（`AGENTS.md` §4.2）

```text
- 当前分支 / worktree：feat/svn-phase1c @ .claude/worktrees/svn-phase1c
- 当前分支是否已合并进 master：否，全新分支
- 是否允许在当前位置修改：是，独立 worktree，已用 EnterWorktree 会话级切入
- 目标 worktree 是否在改动前已创建并切入：是
- pwd 实测值：/c/Users/liuhao/workspace/dev/Dev/mygitclient/.claude/worktrees/svn-phase1c ✓
- 基点 commit：4b8ce86（本计划文件本身）；其父是 f67a007（Phase 1b 的 squash 合并）
- 是否存在未提交改动：无（建完即查，干净）
```

---

## 全局

### 本期范围由用户在 2026-09-15 的对话里定下

spec §16 的 Phase 1 被 2026-09-14 切成两半，1b 做完了「录基线」那半。本期 = 剩下的
「看得见」那半（失同步横幅、SVN log、混合图、本地补丁标记、基线分支 UI 保护）
**加 1b 记账的三条正确性欠账**（NB-6 无 UI 入口、§6.6.5 每个 wc 各体检一次、
NB-2 depth-excluded 漏收）。

用户同时选定：本地补丁标记做到「勾选框 + 角标 + **显式警告文案**」——spec §16 要求
Phase 1→3 之间的窗口期必须在 UI 上写明，不能让用户以为勾了「仅本地」就安全了。

### 为什么 NB-6 是本期最要紧的一条

Phase 1 的验收语写着「此期只报警 + 提供**重建基线**」。1b 把
`svn_rebuild_baseline` / `svn_check_baseline` 两个命令做好了，但没有任何 UI 入口，
用户点不出来——也就是说 1b 单独交付时，Phase 1 的自洽前提是断的。

---

## Task 0：开工自检与基线数字

**改动前基线（在本 worktree 内实测，不是抄上一期的）：**

| 项 | 计划预期 | 实测 |
|---|---|---|
| `cargo test` | 412 passed / 0 failed / 33 个二进制 | **412 passed / 0 failed / 33 个二进制** ✓ |
| `pnpm test` | 290 passed / 22 files | **290 passed / 22 files** ✓ |
| `pnpm build`（含 `tsc`） | 通过 | **通过** ✓ |

三项全部与计划预期一致，无需修正计划。

**统计方法**（1b 踩过的坑，这里照着做）：不要用 `Select-String -Pattern 'FAILED'`
——PowerShell 默认大小写不敏感，会把 33 行 `0 failed;` 全部匹配进来。
本期用 `grep -o "test result: ok\. [0-9]* passed" | awk '{s+=$1}'` 求和。

#### 偏离计划

- **基点是 `4b8ce86` 而不是计划 Task 0 写的 `f67a007`。** 计划文件本身先按
  `AGENTS.md` §4.1 的纯文档例外提交到了 `master`，于是 worktree 的基点比计划
  写作时多了一个 commit。内容上无差别（那个 commit 只有本计划这一个文件）。

- **worktree 用 `git worktree add` + `EnterWorktree(path)` 两步建，而不是
  `EnterWorktree(name)`。** 原因：本仓库没有配 `worktree.baseRef`，`EnterWorktree`
  默认走 `fresh`——从 **`origin/master`** 切分支，而计划文件那个 commit 还没 push，
  新 worktree 里就不会有它。两步法从本地 HEAD 建，再用 `EnterWorktree` 完成
  **会话级切入**，满足 §4.1「`git worktree add` 本身不算切入」那条。
  分支名因此是 `feat/svn-phase1c`（符合 §4.4 的约定），不是 `worktree-<name>`。

#### 计划的错误

计划里 Task 2 / 4 / 6 / 7 的测试代码片段有两处写错，实现时按实际改：

- **crate 名是 `app_lib`，不是计划写的 `mygitclient_lib`。** `Cargo.toml` 里
  `[lib] name = "app_lib"`，既有测试全部 `use app_lib::engine::…`。
- **fixture 的引入方式是 `#[path = "common/svn_fixtures.rs"] mod svn_fixtures;`**，
  不是计划写的 `mod common; use common::svn_fixtures;`。`src-tauri/tests/common/`
  底下没有 `mod.rs`，走 `#[path]` 直接挂。

两处都是照着别的项目的惯例顺手写的，没有先看本仓库既有测试的头部。

#### 计划没写的坑

- **`EnterWorktree` 的默认基点是 `origin/<默认分支>`，不是本地 HEAD。** 1a / 1b
  没踩到是因为那两期的计划文件在开工前就已经合进 `origin/master` 了。凡是
  「先在 master 上提交文档、再开 worktree 施工」的流程都会撞上这一条。

- **磁盘空间是这个项目的真实约束，会伪装成编译错误。** 开工时 C 盘只剩
  **12.8 MB**，`cargo test` 报的是：

  ```text
  LINK : fatal error LNK1318: Unexpected PDB error; LIMIT (12) '…\tree-….pdb'
  error: linking with `link.exe` failed: exit code: 1318
  ```

  这条信息里没有任何一个字提到磁盘。第一次撞上时我误判成「并发导致」（当时
  `cargo test` 和 `pnpm install` 同时在跑），串行重跑一次才排除。
  **判据是查 `Get-PSDrive C` 的 Free，不是看错误文本。**

  成因：仓库下当时有 8 个 `src-tauri/target`，合计约 77 GB，其中 6 个属于**已合并
  分支的残留 worktree**。本项目每个 worktree 的 debug target 是 6–15 GB，
  开三四个 worktree 就能填满一块盘。

  **已知后果**：`AGENTS.md` §4.1 要求每个任务开独立 worktree，而 §4.3 又限制清理
  worktree 需要逐条授权——两条合起来会让 target 目录单调累积。收尾时应当提醒用户
  清理，或考虑给 worktree 配共享的 `CARGO_TARGET_DIR`（本期不改，记给后面）。

- **`cargo test -j 1` 比 `-j 2` 更适合磁盘紧张时跑。** 并发链接会同时打开多个
  PDB，峰值占用翻倍。基线最终是用 `-j 1` 跑出来的。

#### 为什么这么改是安全的

- 两步建 worktree 没有降低隔离性：`EnterWorktree(path)` 之后 session 的真实工作目录
  就在该 worktree 内（`pwd` 已实测），后续所有相对路径命令都落在这里。

---

## 计划开工前已完成的实测（2026-09-15，临时 fixture，跑完即删）

以下五组是本期所有判断的事实来源，review 要能对照。**它们是实测结果，不是设计意图。**
原表见计划文件「开工前已完成的实测」一节，此处只记结论与编号，避免双维护：

- **A 组（`svn log --xml` 形状）**：M1 属性逐行排列 / 默认降序、M2 `-v` 的 path 是仓库
  绝对路径、M3/M4 区间方向决定顺序、P4/P5 任一端越界即 exit 1 `E160006`、
  P6 `--limit 0` → `E200004`、P9 空仓库 exit 0 空 `<log>`、Q5 `-q` 时 `<msg>` 整个不出现、
  M7 `log` 拒绝 `--ignore-externals`。
- **B 组（target 形态）**：Q4 多 URL 被拒、R1 `<URL> + 多个相对路径`可行且返回并集、
  R3 工作副本路径只能给一个、**R4 一条坏路径炸掉整次调用**、**R5 相对路径带 peg 尾缀
  报 `E160013`**、R2 `-v` 不按 target 过滤。
- **C 组（孤儿 baseline）**：Q0 update 后 nodeRevs = HEAD、Q1 白名单 URL 上查不到该
  revision、Q2 Repository Root 上查得到、Q3 白名单历史跳号。
  ⇒ **孤儿 baseline 是常态，不是边界情况；锚点必须打给 Repository Root。**
- **D 组（`svn ls -R` 与 depth-exclude）**：M8/P3 输出形状、N4 `<name>` 相对 `<list path>`
  重新编号、**P1 路径不存在时 exit 1 但 stdout 是结构完整的空清单**、P2 revision 不存在
  时 XML 被截断、P10 空仓库是合法的空、N3a–N3e **NB-2 完整复现**（四道既有检查全绿，
  只有 `svn ls -R` 看得见）。
- **E 组**：P7 `svn info --show-item revision` 输出裸数字、N6 git trailer 往返正常。

---

## Task 1：`svn_xml` 加 `parse_log` 与 `parse_list`

#### 偏离计划

- **多加了两条计划里没有的测试**：`log_treats_a_missing_author_as_empty`
  （匿名提交 / revprop 被删时 `<author>` 不出现）与
  `list_does_not_confuse_the_inner_commit_author_with_an_entry`
  （每个 `<entry>` 里嵌着 `<commit><author>`，全文 `descendants()` 搜 `name`
  会捞错东西）。后者是写实现时发现的真实风险——既有的 `parse_info` 正是用
  `descendants()` 写的，照抄过来就会踩。

- **`parse_list` 用 `list.children()` 而不是 `doc.descendants()`**，与同文件其他
  解析器的写法不一致。理由见上一条：`<entry>` 是嵌套结构，这里必须限定层级。

#### 计划的错误

- 计划的测试片段里 crate 名写成 `mygitclient_lib`，实际是 **`app_lib`**；
  fixture 引入写成 `mod common;`，实际是
  **`#[path = "common/svn_fixtures.rs"] mod svn_fixtures;`**。
  （已记在 Task 0 一节，Task 2 及以后的片段同样要改。）

- 计划的实现片段里 `parse_log` 直接用 `child_text` 风格的闭包取文本。
  实际本文件已有的 `child_text` **缺失即报错**，语义正好相反，不能复用——
  新写了 `joined_child_text`（缺失返回空串）。

#### 计划没写的坑

- **变异测试有一次是「没变红」，这本身是结论。** 计划只安排了「把 `parse_log`
  改成同行正则」这一个变异（结果 5 条 log 测试全红，断言有牙）。我另外试了
  把 `joined_child_text` 改回 `child.text()`——**37 条全绿**。

  说明 roxmltree 会把展开实体后的文本仍合并成**单个**文本节点，原先「`.text()`
  会因为 `&lt;` 被静默截断」的担心在当前输入上**不成立**。拼接版保留下来是防御
  （防 CDATA、注释穿插这类真会产生多文本节点的形态），但**现有测试挡不住有人
  把它改回去**。这句话已经写进函数的文档注释——留一条自己都证明不了的断言在
  注释里，比没有注释更坏。

- **CRLF → LF 的归一确实由 XML 规范保证**，roxmltree 实现了，`<msg>` 里的
  `\r\n` 读出来就是 `\n`。这条**有**测试守着
  （`log_decodes_entities_and_normalizes_crlf_inside_the_message`），
  与上一条的处境不同。

- **`cargo fmt` 会把跨行的 `ok_or_else(|| GitError::new(...))` 压成一行**，
  手写时按可读性排版会导致 `cargo fmt --check` 失败。先跑 `cargo fmt` 再
  `--check`，不要手工对齐。

#### 为什么这么改是安全的（Task 1）

- 本 Task **只新增**两个 `pub fn` 和三个结构体，没有改动任何既有函数的签名或
  行为。既有 27 条 `svn_xml` 测试一条没动，全部照常通过。
- 两个新函数都是**纯函数**（`&str` → 结构），不发命令、不碰磁盘、不碰 git，
  失败路径只有「返回 `Err`」一种。
- `parse_list` 的文档里写明了「空清单不代表成功」这条契约由调用方的
  `run_svn` 退出码把守——这是 Task 6 最危险的失败形态，提前钉在解析层的注释里，
  让下一个人在写调用方之前就读到。

---

## Task 2：`svn_log` 拉取编排

#### 偏离计划

- **`SvnFixture` 没有扩展字段。** 计划的测试片段假设新 fixture 会额外暴露
  `whitelist_url` / `repos_root` / `baseline_rev` 三个字段。实际 `SvnFixture`
  只是 `TempDir` 的薄包装（`path()` / `repo()` / `wc()` / `wc_str()`），
  加字段会动到全部 18 个既有 fixture 的构造。改成**测试自己推导**：
  `repos_root = file_url(&fx.repo())`、`whitelist_url = "{repos_root}/Code"`、
  `baseline_rev` 由 fixture 的构造过程固定为 4（写在 fixture 的文档注释里）。

- **多写了 4 条计划里没有的测试**：
  - `fetch_range_returns_entries_newest_first_for_a_descending_range`
  - `fetch_range_clamps_only_the_out_of_range_end`（**半越界**：`3:999`。
    计划只测了「整段越界」，而「拉到最新」最常见的写法恰恰是半越界，
    只测整段越界会让「两端各自钳制」退化成「越界就整段丢掉」也能变绿）
  - `fetch_anchor_returns_none_for_a_revision_above_head`
  - `log_targets_dedupes_working_copies_that_share_a_url`

- **`log_targets` 的测试自己调 `init_hybrid` 造配置**，没有新增
  `svn_repo_initialized()` 共用 fixture。理由：既有 `tests/svn_init.rs` 就是这么
  做的（本地 `req()` 辅助函数），跟着既有写法走；而把 `init_hybrid` 塞进
  `svn_fixtures.rs` 会让纯 svn 的 fixture 模块反向依赖 engine 代码。

#### 计划的错误

- **计划假设了约 10 个并不存在的 fixture**（`svn_repo_initialized`、
  `svn_repo_initialized_then_*`、`plain_git_repo`、
  `svn_repo_initialized_with_*` 等），却没在任何一个 Task 里写「先建这些
  fixture」。本 Task 只需要一个，自己补了；**Task 4/5/6/7 还要补更多，
  那几个 Task 的工作量比计划写的大**。

#### 计划没写的坑

- **「整段越界」必须单独早退，光靠钳制不够。** 钳制会把 `5:9`（HEAD=4）压成
  `4:4`，于是「没有新东西」被拉回一条早就有的记录——**比报错更糟，因为它看起来
  是成功的**（增量合并会反复把同一条记录当成新的）。实现里加了
  `if from > head && to > head { return Ok(vec![]) }` 早退。

- **`svn info --show-item revision` 对 URL target 也成立。** 计划只实测过对
  `.`（工作副本路径）的形态（P7），对 URL 是未经验证的假设。本 Task 的
  `head_revision_reads_the_servers_latest` 实际跑通，假设成立，现在有测试守着。

#### 为什么这么改是安全的（Task 2）

- 本模块**只发 `svn log` 与 `svn info` 两个只读子命令**，不写磁盘、不碰 git、
  不碰工作副本。最坏的失败是返回 `Err`，调用方（Task 7）按「不 await、失败静默」
  处理。
- 钳制**只会让查询范围变窄，不会变宽**：`clamp(1, head)` 的结果恒在仓库真实存在
  的 revision 区间内。不存在「因为钳制而拉到了不该拉的东西」这条路径。
- `fetch_anchor` 对越界 revision 返回 `None` 而不是 `Err`，是为了让一个坏锚点
  只损失图上一个节点，而不是整条历史。

---

## Task 3：`svn_log_cache` 缓存与合并

#### 偏离计划

- **加了 `TargetLog::new()` 与 `is_fresh()`**。计划让调用方直接构造
  `TargetLog { covered_min: 0, covered_max: 0, entries: vec![] }`，并在
  `merge_into` 里判「`covered_max == 0 && entries.is_empty()`」。后半个条件是错的：
  一个 target 可能先并进若干**条目**再确定区间。判据收窄成 `covered_max == 0`
  （revision 从 1 起算，0 不是合法值），语义更干净。

- **多写了 7 条计划里没有的测试**：相邻区间可并（`8` 之后拉 `9..10`）、降序区间
  参数、重复 revision 保留先到的那条、uuid 大小写不敏感、未来版本号作废、
  缓存文件不存在、覆写既有文件、缓存路径必须在 `.git/` 内。

#### 计划的错误

- 计划的 `write_cache` 片段用 `d.subsec_nanos()` 拼临时文件名。`subsec_nanos`
  只有纳秒的小数部分（0–999999999，且每秒回绕一次），同一秒内两次写有真实的
  碰撞概率。改成 `d.as_nanos()`。

#### 计划没写的坑

- **clippy 会拦 `sort_by(|a, b| b.rev.cmp(&a.rev))`**，要求换成
  `sort_by_key(|e| Reverse(e.rev))`。按 `AGENTS.md` §7.3 处理掉了，没有用
  `#[allow]` 掩盖。

#### 为什么这么改是安全的（Task 3）

- 本模块**不发任何命令**，只读写一个 `.git/` 内的 JSON。
- 所有读失败（文件不存在、JSON 坏、版本不认、uuid 不符）都降级成**空缓存**，
  绝不向上抛错——最坏的后果是「历史要重拉一次」，而不是仓库打不开。
- 写是原子的（同目录临时文件 + rename），rename 失败时清理临时文件，
  不会在 `.git/` 里堆出一地长得像合法缓存的垃圾。
- uuid 不符时**整份作废**：这是唯一一个会产生**错误信息**而不是缺失信息的
  失败形态（旧仓库的 revision 号在新仓库里指向别的提交），所以宁可多重拉一次。

---

## Task 4：`svn_sync` 失同步判定

#### 偏离计划

- **`node_revs` 的 targets 直接取 `WcIdentity::wc_root`**，没有按计划写的
  「把白名单根按落在哪个 WC 内分组、取最长前缀匹配」。原因见下面「计划的错误」：
  `wc_root` 本身就已经是仓库内相对路径形式的记账单位，`enumerate_wcs` 返回的
  就是白名单条目 ∪ 其下嵌套 external 根，不需要再分组。

- **`read_root_revs` 返回 `Vec<RootRev>` 而不是单个值**，且比对逻辑不在后端。
  模块注释里写明了理由：后端保持无状态，上次观测值由前端 store 持有。

- **多写了 2 条测试**：`root_revs_changes_when_someone_updates_outside`
  （第一档的判据要求这个值真的会变，否则分层等于没分）与
  `a_repo_without_a_baseline_branch_is_a_loud_error`。

#### 计划的错误

- **计划假设 `WcIdentity::wc_root` 是绝对路径。** 实际它是**仓库内相对路径**
  （实测打印：`"Code"`、`"Code/Vendor"`），而且字段文档明确写着「名字里的
  `wc_root` 指的是这条白名单条目 / external 根，**不是** SVN 意义上的工作副本根」
  ——后者是 `wc_root_abspath`。

  我按计划写了个 `relative_to(repo_root, wc.wc_root)` 去换算，结果**恒为 `None`**，
  于是嵌套 external 从没进过 targets。详见下一节。

- 计划的 fixture 假设同前几个 Task：`svn_repo_initialized_then_*` 一个都不存在，
  全部要自己造。本 Task 新增了 `svn_cmd`（给测试发外部变更用）、
  `svn_repo_three_revisions`、`svn_repo_pinned_external_updated_to_head` 三个。

#### 计划没写的坑

- **那个 `wc_root` 的误解造出来的是一个「看起来完全正常」的盲区。** 嵌套
  external 漏掉之后，白名单根那一次查询照样返回**单值**，于是判定是 `InSync`
  ——没有任何报错、没有任何异常，正是 §6.6.5 警告的那块「git 管着、SVN 侧没人盯」
  的区域。

  **计划安排的变异测试（只查第一个 WC）因此不用另做：实现的第一版就是那个变异。**
  `node_revs_unions_every_working_copy_including_nested_externals` 当场变红，
  报出 `SyncStatus { baseline_rev: 2, node_revs: [2], verdict: InSync }`。
  断言有牙，有实据。

- **排查时先去量了 svn 本身**（`AGENTS.md` §6.2）：临时 fixture 上确认
  `svn update -r1 -- Code/Vendor@` 之后，`svn status -v --xml --ignore-externals
  -- Code/Vendor@` 确实报 `revision="1"`。svn 侧没问题，才回头查自己的代码。
  没有这一步很容易误判成「svn 不支持这么用」然后去改测试。

- **`parse_node_revs` 返回 `BTreeSet<i64>`，而本模块对外用 `u64`。** 转换处显式
  拒绝负数并报 `svn_sync_negative_rev`，不做静默 `as` 截断。

- **`PinnedReset` 的 fixture 比想象中难造**：要先让 external 被**单独** update
  到 HEAD（这样才能初始化出一个高的 baselineRev），再由测试在项目根跑递归
  `svn update` 把它拽回 peg。直接造一个「已经被钉住」的仓库是不行的——
  那种状态下 P2 会拦住初始化，根本走不到失同步判定。

#### 为什么这么改是安全的（Task 4）

- 本模块只发 `svn status` 与 `svn info` 两个**只读**子命令，外加一条
  `git log -1 --format=%B` 读基线 message。不写任何东西。
- 没有基线时返回 `Err(svn_no_baseline)` 而不是编一个 0：返回 0 会让「N != 0」把
  **每一个**仓库都判成失同步，返回 `InSync` 则等于宣称一个不存在的基线是好的。
- `--ignore-externals` 的作用域是**每个 WC 内部**（挡住更深一层），而不是从项目根
  覆盖白名单——后者会把白名单整个跳过。这条差别写进了 `node_revs` 的文档。
- 判定只读不改，任何误判的代价都止于「横幅文案不对」，不会动仓库状态。

---

## Task 5：基线体检按工作副本分组（§6.6.5 欠账）

#### 偏离计划

- **命令形态改成 `--old=<url(P)>@N --new=<P>`，不再是 `--new=. -- <targets…>`。**
  计划写的是「对每组跑一次，尾随 target 给该组的根」。实际上 `enumerate_wcs`
  返回的每一条本身就是一个记账单位（白名单条目，或其下的嵌套 external 根），
  所以 `--new` 直接指向它、**不带任何尾随 target** 更简单，也少一处 peg 陷阱。

- **没有按计划写「最长前缀匹配分组」那段。** 同 Task 4 的原因：`wc_root` 已经是
  相对路径形式的记账单位，`enumerate_wcs` 的返回值就是分好组的结果。

- **`whitelist_not_under_base` 保留未动。** 计划没提它。它守的是「白名单子树被
  `svn switch` 到别的分支」——改成 per-unit URL 之后，那种情况其实能正确比对了
  （用的就是它实际指向的 URL）。但放行它属于**行为扩大**，超出本 Task 的范围，
  而且会让既有测试
  `a_switched_whitelist_subtree_is_not_checked_rather_than_lying` 的期望反转。
  **保持现状，记账给后面**：将来若要放开，需要先想清楚「分支上的 rN」与
  「trunk 上的 rN」在基线语义下是不是同一个东西。

- **多写了一条计划里没有的测试** `one_unit_failing_does_not_silently_verify_the_others`
  （计划有这条的意图，但没给可行的 fixture 造法）。造法：跨仓库嵌套 external +
  把 external 指向的仓库从磁盘删掉。工作副本仍在（`svn info` 是本地读，单位照样
  列得出来），但它的 URL 已经不可达。

#### 计划的错误

- 计划说「`nested_external_not_covered` 这道守卫删掉」，但**没提它会连带推翻两条
  既有断言**：
  - `tests/svn_health.rs` 的 `a_nested_external_under_the_whitelist_is_never_claimed_verified`
  - `tests/svn_init.rs` 的 `init_succeeds_on_a_clean_same_repo_nested_external_and_never_tracks_its_dot_svn`
    里那句 `Health::NotChecked { .. } => {}`

  第二条只有跑**全量** `cargo test` 才会暴露（只跑 `--test svn_health` 是绿的）。
  计划的 Task 5 Step 5 要求跑全量，这一步救了场。

#### 计划没写的坑

- **单文件白名单条目在新命令形态下没有子路径可剥。** `--new=Code/notes.txt` 时
  svn 打出来的路径**恰好等于** `unit_url` 本身，`strip_prefix_ci(path, "url/")`
  会返回 `None`，于是走进「形状不认识」那条 `NotChecked` 分支——一个完全正常的
  配置被判成未验。加了一个相等分支专门处理它，既有测试
  `a_single_file_whitelist_entry_is_not_filtered_out_by_extension` 守着。

- **删守卫时容易留下半截循环体。** 我第一次编辑时把新循环写完、旧循环体没删干净，
  还顺手写了个占位函数去接住它——那是错的，当场删掉重来。改这种「整段替换」时
  应当先读完整个函数再一次性替换，而不是分两次编辑。

#### 为什么这么改是安全的（Task 5）

- **方向只会变严，不会变松**：改动前嵌套 external 那块内容**从没被比对过**
  （diff 不下沉），改动后它被服务器逐条验过。唯一「变松」的是结论从
  `NotChecked` 变成 `Verified`——而那是因为**真的验过了**，不是因为跳过了检查。
- 反向证据齐备：`a_lying_baseline_inside_a_nested_external_is_caught` 证明脏的
  external 会被抓住。少了它，「见到 external 就返回 Verified」也能让干净那条变绿。
- 一个单位失败即整体 `NotChecked`，不做「其余都干净所以 Verified」的部分担保。
- 本 Task **不改任何写路径**：`check_baseline` 只发 `svn diff` / `svn info`，
  它的三个调用方（初始化、重建、手动体检）对三种结论的处置逻辑一字未动。

---

## Task 6：服务器侧清单证伪 depth-excluded 漏收（NB-2 欠账）

#### 偏离计划

- **`ServerAudit::Missing` 多了一个 `total` 字段。** 计划只有 `files`。真实工程
  一旦漏收就可能是几千条，界面上只列前 20 条，但**总数必须说**——只列 20 条而
  不说总数，用户会以为只漏了 20 个。
- **多写了 3 条计划里没有的测试**：`a_file_only_on_disk_is_not_reported`
  （反方向不该报）、`a_revision_the_server_does_not_have_is_not_checked_either`、
  以及下面那条从变异测试里逼出来的
  `a_path_missing_at_that_revision_is_not_checked_rather_than_matched`。
- **`is_w_file` / `normalize_roots` 提到了 `svn_include_check`**（计划只说提
  `is_w_file`）。`normalize_roots` 是它的前置，分开放两个模块没有意义。

#### 计划的错误

- 计划的 `an_unreachable_url_is_not_checked_rather_than_matched` 用的 fixture 是
  `svn_repo_initialized_with_bad_url`（不存在）。我用「把仓库本体从磁盘删掉」
  实现它——**而这个造法测不出计划真正想守的那条**，见下。

#### 计划没写的坑

- **变异测试抓出的不是实现缺陷，是「测试覆盖不足」。**

  计划安排：把 `run_svn` 换成 `run_svn_code` 并忽略退出码，`an_unreachable_url…`
  应当变红。实测**照样全绿**。

  原因：「把整个仓库删掉」时 svn 吐出来的 XML 是**坏的**（截断），`parse_list`
  报错，于是被解析失败那条路兜住了——**退出码那道门根本没被考验到**。

  而实测 P1 真正危险的形态是另一种：**仓库存在、只是路径在该 revision 上不存在**。
  那时 svn exit 1，但 stdout 是一份**结构完整的空** `<lists><list></list></lists>`，
  `parse_list` 会**成功**解析出空清单 ⇒ 服务器集合为空 ⇒ 差集恒为空 ⇒
  报告「没有漏收」。

  补了 fixture `svn_repo_whitelist_added_in_r2`（`Code/` 在 r1 上还不存在）与测试
  `a_path_missing_at_that_revision_is_not_checked_rather_than_matched`。
  带着变异重跑：**变红，实际值就是 `Matched`** —— 正是「用修 NB-2 的代码重造一个
  NB-2」那个失败形态。撤销变异后转绿。

  **教训**：「制造一个失败」和「制造 spec 实测记录的那一种失败」不是一回事。
  P1 的记录里写着「exit 1 **但 stdout 结构完整**」，我造的场景丢掉了后半句。

- **Task 5 的改动没有顺带修掉 NB-2。** 一度以为按单位分开发 diff 之后，
  `svn diff --old=URL@N --new=Code` 会把 depth-excluded 的 `Code/Deep/D.cs`
  报成删除。实测**没有**——`the_existing_checks_really_do_miss_a_depth_excluded_subtree`
  在 Task 5 改完之后仍然绿。这块新机制确实是唯一入口。

#### 为什么这么改是安全的（Task 6）

- 这是本期**唯一一处新增的检查**，所以单独论证：
  1. **它只读。** 只发 `svn ls -R`（纯读）与 `expected_set`（遍历磁盘，不写）。
  2. **它不参与任何既有门槛。** 结论不进初始化 / 重建 / 交付的任何判定，
     只在界面上多挂一条说明（Task 10）。**误报的代价止于噪音**。
  3. **漏报的代价与改动前相同**——改动前这块盲区完全没人看。
  4. 失败一律落 `NotChecked`，不冒泡成 `Err`：它是锦上添花，不该让仓库打不开。
- 只报「服务器有、磁盘没有」一个方向。反方向是「本地新增还没 `svn add`」，
  完全正常；报了就是噪音，而噪音会让用户学会忽略这条提示。
- 过滤复用 `is_w_file` 这**同一处**定义，不再有第二份「什么算 W-file」。

---

## Task 7：Tauri 命令层与 `Capabilities.vcs`

#### 偏离计划

- **编排逻辑放进 engine，不塞在命令体里。** 计划的 Task 7 Step 3 把
  `svn_fetch_log` 的整段编排（读缓存 → 逐 target 拉 → 补锚点 → 写缓存）写在命令
  函数里。那样它只能靠 `pnpm tauri dev` 手工验。改成
  `engine::svn_log::{read_log, fetch_log}` 与
  `engine::svn_server_set::audit_server_set_at_baseline`，命令体只剩一行转发
  （`AGENTS.md` §7.2：纯逻辑下沉，可直接单测）。

- **`Capabilities.vcs` 用 `&'static str` 而不是 enum。** serde 直接给出前端好判的
  字符串，不必再约定一层 tag 名。TS 侧对应 `'git' | 'svn'` 联合类型。

- **前端 `capabilities.ts` 在本 Task 就改了**（计划排在 Task 10）。理由：Rust 侧
  一改，两侧的接口就不一致了，而这正是本仓库那两条「字段名钉死」测试在防的事。
  隔着三个 Task 不同步，中间任何一次 `pnpm build` 都不会报错（`tsc` 看不见 Rust）。

#### 计划的错误

- 计划的 Task 7 测试片段写 `read_capabilities(&git.root)`，而实际签名是
  `read_capabilities(path: String)`（命令函数按值收）。
- 计划提到的 fixture `plain_git_repo` 不存在；`svn_backend.rs` 里本来就有
  `git_init(dir)` 辅助函数，直接用它。

#### 计划没写的坑

- **加一个非布尔字段会把两条既有断言变成空话。**
  - Rust 侧 `capabilities_serializes_to_the_exact_field_names_the_frontend_reads`
    里有 `obj.values().all(|v| v == &Bool(true))`；
  - 前端 `capabilities.test.ts` 里有
    `Object.values(ALL_CAPABILITIES).every(v => v === true)`。

  加了 `vcs: "git"` 之后，这两条会**直接变红**（好事，说明有牙）。但顺手改成
  「排除 vcs 再断言」才是对的——如果图省事改成 `.every(v => v !== undefined)`，
  那条测试就再也拦不住「某个开关被误设成 false」了。两侧都按前者改了。

- **覆盖区间要用「实际拿到的 revision 范围」记，不能用请求区间。**
  增量拉取的请求上界我写的是 `u64::MAX`（交给 `fetch_range` 钳成 HEAD），
  直接把它记进 `covered_max` 会谎称「覆盖到了天边」，下一次增量拉取永远算出空区间
  ——**历史从此不再更新，而且完全无声**。实现里改成取 `got` 里的 min/max。

#### 为什么这么改是安全的（Task 7）

- 五个新命令**全部只读**：`svn status` / `svn info` / `svn log` / `svn ls` 加
  `git log`，外加写一个 `.git/` 内的缓存 JSON（原子写）。
- `Capabilities` 加字段是**纯增量**：既有六个布尔字段的取值一个没动，
  两侧的字段名对齐由两条测试各守一端。
- `svn_fetch_log` 单个 target 失败不中断其余——一条 external 的 URL 不可达不该
  让整张图空掉；**全部**失败才算整次失败。
- 锚点补拉失败只跳过、不上抛：一个拿不到的锚点只损失图上一个节点。
- 锚点数量有硬上限（200）。没有上限的话，一个跑了三年的仓库打开一次要发几百次
  网络往返——spec §10.3 说「几十个」是对刚起步的仓库说的。

---

## Task 8–13：前端（接合、横幅、图、补丁标记、分支保护）

这六个 Task 改动集中、互相咬合，决策条目合并记。

#### 偏离计划

- **没有单独建 `src/engine/svnLog.ts`。** 计划让它装「SVN 历史的前端类型与日期
  归一」，实际只有一个类型和一个 `toEpochSeconds`，并进 `svnGraph.ts` 了。
  为两样东西单开一个文件是噪音。

- **`spliceHistory` 只在 `caps.vcs === 'svn'` 时调用**（计划没说条件）。理由见
  下面「计划没写的坑」第一条——这是本期最大的一处回归风险。

- **store 里多了一个 `gitCommits` 字段**（计划没提）。`commits` 现在是接合后的
  结果，而 svnRevs 与 gitCommits 各自会被异步刷新，任一变化都要重接一次。
  接合是纯函数，重算比增量维护更不容易错。

- **Task 11 的 store 接线提前到 Task 10 做了**：Task 10 引入 `spliceHistory` 之后
  不接线的话 `tsc` 会因为未使用的 import 直接失败（`noUnusedLocals`）。

- **体检结果的文案复用 `svnInit.describeHealth`**，没有按计划在 `svnSync.ts` 里
  另写一份。两份文案迟早说出不同的话。

- **`describeAudit` 是计划里没有的**：计划让组件直接读 `ServerAudit`。
  「漏收几条、要不要显示、显示成什么」是判断，散进组件就测不到了。

- **`BaselineBanner` 的确认框走 `ConfirmDialog` 的 `action` 而不是「确认后自己跑」**：
  重建要发一次走网络的体检，确认完就关框的话那几秒界面上毫无动静，看着像卡死
  （这是 `ConfirmDialog` 里既有的注释写明的用法）。

#### 计划的错误

- 计划的 `ConfirmDialog` 调用写的是 `{ title, body, confirmLabel, hideCancel }`，
  实际 API 是 `{ title, message, detail?, warning?, confirmLabel?, danger?,
  action?, busyLabel? }`——**没有 `body`，也没有 `hideCancel`**。

- 计划让 `useRepoEffects` 直接调 `refreshSvnSync`，但那个函数在 `useRepoStore`
  里、`useRepoEffects` 是**另一个**函数，刷新函数是通过 `RepoRefreshers` 传进去的。
  要在接口上加一条并在调用点补上。

- 计划的 Task 14 Step 2 用
  `(Select-String -Pattern '#\[tauri::command').Count` 与 `'commands::'` 的计数
  对账，实测报 **57 vs 56 不一致**——多出来的那一处是 `session.rs` 的**文档注释**
  里引用的 `#[tauri::command(async)]` 字样。**朴素计数会误报**，正确做法是
  按函数名做集合 diff（实测 56 == 56，无差异）。

#### 计划没写的坑

- **`spliceHistory` 会误伤从 `git svn` 迁移过来的普通 git 仓库。**

  它按 `git-svn-id` trailer 认 baseline，并把 baseline commit **从图上删掉**
  （spec §10.3 ③：它在图上的位置由 SVN 序列决定）。而那个 trailer 格式**正是
  照 `git svn` 抄的**（spec §6.4.1），所以一个用过 `git svn` 的普通 git 仓库
  历史里遍地都是。

  无条件接合的话，这类仓库一打开就少掉大半历史，而 svnRevs 是空的、没有任何
  东西接住它们——图上那些提交**凭空消失且没有任何报错**。

  判据必须是「这个仓库是不是 SVN 混合仓库」（`caps.vcs`），**不能**是
  「svnRevs 空不空」。这也是 `Capabilities.vcs` 那个字段除了省 invoke 之外的
  第二个理由。

- **`rootRevs` 的比对字符串必须排序。** `enumerate_wcs` 的返回顺序没有契约，
  顺序一飘就会被当成「有人动过」，于是每次刷新都触发全量校验——分层白做。

- **基线分支判定必须精确匹配。** `svn/base-backup`、`feature/svn/base`、
  `svn/basement` 都是合法分支名，`startsWith` 会把它们全判成基线，
  用户会在自己的备份分支上发现「不能提交」且没有任何解释。

- **禁用提交要同时挡住按钮和快捷键。** 只把按钮置灰而 `Ctrl+Enter` 还通，
  等于没拦。判据放进 `canCommit`，两条路径共用。

- **勾选框提交后不重置。** 连着提交几个本地补丁是常见做法，每次重勾很烦；
  `amend` 的既有行为也是不重置，保持一致。

#### 为什么这么改是安全的（Task 8–13）

- 前端这六个 Task **不发任何新的写命令**。唯一会改变仓库状态的入口是
  「重建基线」，它调的是 1b 就已经存在、已被 18 条测试覆盖的
  `svn_rebuild_baseline`，而且前面加了一道确认框。
- 接合是纯函数且只读：最坏的失败是图画得不对，不会动仓库。
- 对普通 git 仓库的影响被 `caps.vcs` 这一道判据挡住：横幅不渲染、勾选框不渲染、
  「加载更多」不渲染、接合直接原样返回。`spliceHistory([], commits)` 在纯 git
  仓库上与输入拓扑等价，有测试钉住。
- 基线分支保护是**只减不增**：禁用一个按钮、置灰一个菜单项，没有新增任何操作。
  checkout 仍然放行——spec §6.4 选可见分支的核心收益就是那个 diff 要能看。

---

## Task 14：收尾验证

#### 已完成的自动化验证

| 项 | 结果 |
|---|---|
| `cargo test -j 1 --no-fail-fast` | **476 passed / 0 failed / 37 个二进制**（基线 412 / 33） |
| `cargo fmt --check` | 通过 |
| `cargo clippy --all-targets` | **0 warning** |
| `pnpm test` | **329 passed / 24 files**（基线 290 / 22） |
| `pnpm build`（含 `tsc`） | 通过 |
| 零 SVN 写操作 | **通过**，见下 |
| 命令对账 | **56 == 56** |

#### 零 SVN 写操作的核对过程（不是一句「已确认」）

对 `src-tauri/src/` 下所有 `run_svn` / `run_svn_code` 调用点提取子命令实参：

```text
6 info   3 status   1 propget   1 ls   1 log
```

加上 `svn_health.rs:114` 那条变量拼出来的 `diff`（正则抓不到，单独核对），
以及 `svn_cli.rs:105` 的 `--version --quiet`（可用性探测）。
**合计 7 个子命令，与 Global Constraints 允许的集合逐字相同。**

另外对 `add|commit|update|revert|delete|propset|merge|switch|cleanup|import|
mkdir|copy|move|lock|unlock` 全文搜索，7 处命中逐条核对，**全部是 git**：

| 位置 | 实际是什么 |
|---|---|
| `commands/ops.rs:95` | `run_git(… "revert" …)` |
| `commands/repo.rs:20` | `Ok("merge".to_string())`，仓库状态字符串，不是命令 |
| `commands/worktree.rs:179` | `git worktree add` |
| `engine/svn_baseline.rs:105` | `self.git(…)` 的 `git add -A`（临时索引） |
| `vcs/git.rs:60/85/87` | `git add` / `git commit` |

#### ⬜ 真实窗口验证：**未执行**

清单见 `docs/superpowers/e2e/2026-09-15-svn-phase1c-e2e.md`，10 个场景全部未勾。

**已完成的那部分**：`pnpm tauri dev` 能编译、能启动、不 panic
（说明 5 个新命令的注册没问题）。**这不能替代 IPC 层的验证**——
集成测试是直接调命令函数本体的，碰不到 `snake_case` → `camelCase` 那层转换，
而那层没有编译期检查（`AGENTS.md` §7.3）。

场景 1（普通 git 仓库不被误判成 SVN）**尤其不能省**：本期改了 `Capabilities`
的形状，这是最容易出的回归。

#### 遗留风险

| 风险 | 说明 |
|---|---|
| `svn ls -R` 在大仓库上的代价未实测 | fixture 只有几个文件。真实 Unity 工程的白名单子树可能上万文件，一次全量列举的耗时没有数据。目前走「不 await、失败静默」，最坏是提示晚出现 |
| 缓存文件无上限 | `.git/mygitclient-svnlog.json` 随历史单调增长 |
| `git-svn-id` 正则两份 | Rust 与 TS 各一份，格式由 spec §6.4.1 钉死且两侧都有测试 |
| `whitelist_not_under_base` 未放开 | 改成 per-unit URL 之后，被 `svn switch` 的子树其实能正确比对了，但放开属行为扩大，超出 Task 5 范围。记账给后面 |
| worktree 的 `target/` 单调累积 | `AGENTS.md` §4.1 要求每任务开 worktree，§4.3 限制清理，两条合起来会填满磁盘（本期开工时就因此撞上 `LNK1318`）。可考虑给 worktree 配共享 `CARGO_TARGET_DIR` |

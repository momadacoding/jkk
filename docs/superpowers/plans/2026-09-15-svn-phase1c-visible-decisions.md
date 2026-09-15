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
| 2 `svn_log` 拉取编排 | ⬜ 未开始 | | |
| 3 `svn_log_cache` 缓存与合并 | ⬜ 未开始 | | |
| 4 `svn_sync` 失同步判定 | ⬜ 未开始 | | |
| 5 基线体检按 WC 分组（§6.6.5 欠账） | ⬜ 未开始 | | |
| 6 服务器侧清单证伪漏收（NB-2 欠账） | ⬜ 未开始 | | |
| 7 Tauri 命令层与 `Capabilities.vcs` | ⬜ 未开始 | | |
| 8 前端接合逻辑 `svnGraph.ts` | ⬜ 未开始 | | |
| 9 前端展示逻辑 `svnSync.ts` | ⬜ 未开始 | | |
| 10 `BaselineBanner` 与 store 接线（NB-6 欠账） | ⬜ 未开始 | | |
| 11 混合图接线与分页 | ⬜ 未开始 | | |
| 12 本地补丁标记 | ⬜ 未开始 | | |
| 13 基线分支 UI 保护 | ⬜ 未开始 | | |
| 14 收尾验证 | ⬜ 未开始 | | |

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

#### 为什么这么改是安全的

- 本 Task **只新增**两个 `pub fn` 和三个结构体，没有改动任何既有函数的签名或
  行为。既有 27 条 `svn_xml` 测试一条没动，全部照常通过。
- 两个新函数都是**纯函数**（`&str` → 结构），不发命令、不碰磁盘、不碰 git，
  失败路径只有「返回 `Err`」一种。
- `parse_list` 的文档里写明了「空清单不代表成功」这条契约由调用方的
  `run_svn` 退出码把守——这是 Task 6 最危险的失败形态，提前钉在解析层的注释里，
  让下一个人在写调用方之前就读到。

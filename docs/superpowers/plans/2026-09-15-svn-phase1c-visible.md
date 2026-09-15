# SVN Phase 1c：看得见那半 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Phase 1b 录下的基线**变得有用**——失同步能被看见并有出口、SVN 历史能被读到并与本地 git 历史画在同一张图上、本地补丁能被标记，并把 1b 记账的三条正确性欠账清掉。

**Architecture:** 本期**依旧一条 SVN 写命令都不发**。新增的 svn 子命令只有 `log` 与 `ls`，都是纯读。判定逻辑（失同步、漏收、接合）全部下沉为纯函数：Rust 侧进 `engine/svn_*.rs`，前端侧进 `src/engine/svn*.ts`（无 React、无 Tauri 依赖，可直接单测）。`svn log` 走网络，按现有 `refreshPullRequests` 的模式处理——不 await、失败静默、只在角落挂低调标记，**不弹错误横幅**。

**Tech Stack:** Rust（`std::process::Command` 经 `engine::git_cli` / `engine::svn_cli` 封装）、roxmltree（已在树里）、serde_json（已在树里）、React 18 + TypeScript + Vite、SVG 渲染。

**Spec:** `docs/superpowers/specs/2026-09-12-svn-git-hybrid-design.md`（本计划基于 master 的 `310c17d` 版本，即 `f67a007` 的祖先）

**上两期:** `2026-09-13-svn-phase1a-foundation.md`、`2026-09-14-svn-phase1b-baseline.md` 及其决策日志。两期均已合并（`ca304d6`、`f67a007`）。

---

## Global Constraints

以下逐字抄自 spec 与 `AGENTS.md`，**每个 Task 的要求都隐含包含本节**。

- **本期不发出任何 SVN 写操作。** `svn add` / `commit` / `update` / `revert` / `delete` / `propset` / `merge` / `switch` / `cleanup` 一个都不得进入 `src-tauri/src/`。本期允许的 svn 子命令集合是 1b 的五个（`info`、`status`、`propget`、`diff`、`--version`）**加 `log` 与 `ls`**，共七个。Task 14 用 grep 做机械断言。
- **`--non-interactive` 是全局选项**，由 `svn_cli::with_global` 统一注入，调用点不要重复写。
- **`--ignore-externals` 按子命令区分**：`status` / `update` 接受；`info`、`diff` **拒绝**（exit 1）；`log` **拒绝**（本期 M7 实测，`Subcommand 'log' doesn't accept option '--ignore-externals'`）；`ls` 未用到，不要试探性加。
- **peg 后缀 `@` 的三处规则互相矛盾，必须逐处对照，不许统一**：
  | 调用点 | 是否过 `svn_wc::peg_safe` | 依据 |
  |---|---|---|
  | `svn info` / `svn status` 的 target | **必须加** | 1b 实测；`--` 挡不住 peg 解析 |
  | `svn diff --old=… --new=…` 的尾随 target | **不能加** | 1b 实测 `E155010` |
  | `svn log <URL> <相对路径…>` 的相对路径 | **不能加** | 本期 R5 实测 `E160013: File not found: path '/Code@'` |
- **`--xml` 输出必须走真正的 XML parser，禁止行内正则。** svn 把属性逐行排列，`<logentry` 与 `revision="…"` 不在同一行（本期 M1 再次确认），行内正则会**静默返回空集**。
- **`git-svn-id` trailer 格式固定一行**：`git-svn-id: <url>@<rev> <uuid>`，正则 `^git-svn-id: (\S+)@(\d+) ([0-9a-fA-F-]+)$`。三字段缺一不可，解析失败即视为该 commit 不是 baseline——不猜、不部分接受（spec §6.4.1）。`<url>` 是 **Repository Root**。
- **`Local-Only` trailer 判定只认 `Local-Only: true` 这一条**，不新增任何配置字段（spec §6.7）。
- **MSRV**：`Cargo.toml` 声明 `rust-version = "1.77.2"`。`#[allow(lint, reason = "...")]` 需要 1.81，**禁止使用**；用裸 `#[allow(...)]` + 中文注释说明理由。
- **所有涉及 git / svn 写操作的测试必须自建临时 fixture 仓库**（`AGENTS.md` 开篇最高优先级约束 + §8），复用 `src-tauri/tests/common/svn_fixtures.rs`。禁止依赖本仓库或任何固定路径，测完即删。
- **禁止新增依赖**（npm 包或 crate）而不先征得用户确认（`AGENTS.md` §7.1）。**本计划不需要任何新依赖**：XML 走已在树里的 roxmltree，JSON 走已在树里的 serde_json。
- **注释用中文，解释「为什么」而不是「做了什么」**（`AGENTS.md` §7.3）。
- **本仓库没有 PR 门禁 CI，也没有 lint 工具。** 禁止宣称「CI 通过」或「lint 通过」（`AGENTS.md` §4.1、§5.1）。唯一的静态检查是 `pnpm build` 里的 `tsc`，且 `tsc` **不覆盖 `tests/vitest/`**。
- **Tauri 的 `snake_case` → `camelCase` 转换没有编译期检查**（`AGENTS.md` §7.3）。本期新增 5 个命令，签名改动必须实际跑 `pnpm tauri dev` 验证（Task 14）。

---

## 开工前已完成的实测（2026-09-15，临时 fixture，跑完即删）

写计划之前先把承重机制实测掉。**下面这些是事实，不是设计意图，实现时不要再猜。**
1a / 1b 的决策日志里「计划的错误」一栏全是没实测就写进计划的命令和断言。

### A 组：`svn log --xml` 的形状

| # | 实测项 | 结果 |
|---|---|---|
| M1 | `svn log --xml` 的结构 | `<log><logentry⏎   revision="3"><author>…</author><date>…</date><msg>…</msg></logentry>…</log>`。属性**逐行排列**，`<msg>` 走 XML 实体转义（`&lt;` `&amp;`），`<date>` 是 ISO8601 带微秒 + `Z` |
| M1b | 默认顺序 | **降序**（新的在前） |
| M2 | `-v` | 多出 `<paths><path action prop-mods text-mods kind>`，文本是**仓库绝对路径**（`/Code/Deep/D.cs`），**不是 URL** |
| M3/M4 | `-r 2:3` / `-r 3:2` | 区间方向决定输出顺序：升序 / 降序 |
| P4/P5 | `-r 9:1` 或 `-r 1:9`，HEAD=1 | **exit 1**，`svn: E160006: No such revision 9`。**两端任一越界即整条失败** |
| P6 | `--limit 0` | **exit 1**，`svn: E200004: Argument to --limit must be positive` |
| P9 | 空仓库（HEAD=0） | **exit 0**，`<log></log>` |
| Q5 | `-q` | `<msg>` **整个不出现**（不是空元素）。解析器必须把缺失当成 `""` 而不是报错 |
| M7 | `--ignore-externals` | **exit 1，拒绝** |

### B 组：`svn log` 的 target 形态（决定拉取是 1 次调用还是 N 次）

| # | 实测项 | 结果 |
|---|---|---|
| Q4 | 多个 **URL** target | **exit 1**，`E205000: Only relative paths can be specified after a URL for 'svn log'` |
| R1 | `svn log --xml <URL> Code Other` | ✅ **成功，返回并集**，降序，已去重 |
| R3 | `svn log --xml . Code`（工作副本路径 + 相对路径） | **exit 1**，`E200007: When specifying working copy paths, only one target may be given` |
| R4 | `svn log --xml <URL> Code NoSuch`（其中一条不存在） | **exit 1**，`E160013: File not found: revision 4, path '/NoSuch'`。**一条坏路径炸掉整次拉取** |
| R5 | `svn log --xml <URL> "Code@"` | **exit 1**，`E160013: File not found: path '/Code@'`。**peg 尾缀在这里是毒药** |
| R2 | subpath target 下的 `-v` | `<paths>` 给的是**该 revision 的完整路径列表**，不按 target 过滤 |

### C 组：孤儿 baseline —— spec §10.3 对成因的描述是错的

构造：r1 建 `Code/` 与 `Other/`；r2 只动 `Code`；r3、r4 只动 `Other`（模拟同事在别的项目上提交）；然后 `svn update`。

| # | 实测项 | 结果 |
|---|---|---|
| Q0 | `svn status -v --xml --ignore-externals -- Code@` | `<wc-status revision="4">`，`<commit revision="2">` ⇒ **`nodeRevs = {4}`，即 `baselineRev = 4`** |
| Q1 | `svn log --xml -r 4:4 <url>/Code` | **空 log，exit 0** —— baseline 那个 revision **不在白名单路径的历史里** |
| Q2 | `svn log --xml -r 4:4 <Repository Root>` | ✅ 返回 r4 |
| Q3 | `svn log --xml --limit 10 <url>/Code` | 只有 r2、r1 |

**结论（对 spec 的有依据修正）：** spec §10.3 把孤儿 baseline 描述成「若某个 baseline 的
revision **太老**、不在已拉取范围内」。**成因不是「太老」，是「那个 revision 根本没碰过
白名单路径」**——而 `svn update` 会把白名单内所有节点的 `wc-status revision` 一律推到
当时的 HEAD，所以 `baselineRev` **通常**就是一个没碰过白名单的 revision。
**孤儿 baseline 是常态，不是边界情况。**

spec 给的补救（「拉取时显式包含」）方向对，但没说打给谁。Q1/Q2 定死了答案：
**锚点必须打给 Repository Root，打给白名单 URL 会拿到空结果**。这条写进 Task 2。

### D 组：`svn ls -R --xml` 与 depth-exclude（NB-2 的检测机制）

| # | 实测项 | 结果 |
|---|---|---|
| M8/P3 | `svn ls -R --xml <url>@N` | `<lists><list path="<URL>"><entry kind="dir\|file"><name>Code/A.cs</name><size>…</size><commit revision=…>`。`<name>` 是**相对 `<list path>` 的递归路径、正斜杠**，**目录也是 entry** |
| N4 | target 换成子路径 URL | `<name>` 相对**那个子路径**重新编号（`A.cs`、`Deep`、`Deep/D.cs`），**不是仓库绝对路径** |
| P1 | 路径不存在 | **exit 1**，但 **stdout 仍是结构完整的 `<lists><list></list></lists>`**，错误只在 stderr |
| P2 | revision 不存在 | **exit 1**，XML **在中途被截断**（`<list …>` 没闭合） |
| P10 | 空仓库 `@0` | **exit 0**，空 `<lists>` —— 合法的空 |
| N3a | `svn update --set-depth=exclude Code\Deep` | 磁盘上 `Code\Deep` **消失** |
| N3b | 之后 `svn status -v --xml --ignore-externals -- Code@` | `Code/Deep` **一条 entry 都没有**，`nodeRevs` 仍是单值 ⇒ **P1/P2/失同步判据全部显示正常** |
| N3c | 之后 `svn info --xml -- Code@` | **`<depth>infinity</depth>`** —— **父目录照样报 infinity，靠读 depth 检测不出来** |
| N3d | 之后 `svn diff --old=<url>@N --new=. --summarize --xml` | 只剩两条 `kind="dir" item="none"`，被 §6.8.4 的既有过滤全部剔除 ⇒ **体检报 `Verified`** |
| N3e | 之后 `svn ls -R --xml <url>@N` | **仍然列出 `Code/Deep` 与 `Code/Deep/D.cs`** ⇒ 服务器侧清单是唯一能发现它的入口 |

**N3b–N3e 就是 NB-2 的完整复现：基线静默少了服务器上确实存在的代码文件，而四道
既有检查全部显示健康。** 检测只能靠 D 组的 `svn ls -R`，这正是 Task 6。

⚠️ **P1 是实现这条检测时最容易踩的坑**：退出码非零但 stdout 是一份**合法的空清单**。
若实现写成「解析 stdout，忽略退出码」，一个打错的 URL 会得到**空的服务器清单**，
而「服务器清单 − 磁盘清单 = 漏收」这个差集在空集上恒为空 ⇒ **报告「没有漏收」**。
那就是用修复 NB-2 的代码重新造一个 NB-2。**必须走 `run_svn`（非零退出即 `Err`），
禁止用 `run_svn_code` 再自行判断。**

### E 组：其余

| # | 实测项 | 结果 |
|---|---|---|
| P7 | `svn info --show-item revision .` | **exit 0，stdout 就是裸的 `1`**。O(1) 档不需要解析 XML |
| N6 | git `Local-Only: true` trailer 往返 | `git log --format='%H%x1f%B%x00'` 原样读回；`git interpret-trailers --parse` 输出 `Local-Only: true` |

### 已读过的既有代码形状（不要重新发明）

| 既有件 | 形状 |
|---|---|
| `svn_cli::run_svn(path, args)` | 非零退出 → `Err(GitError)`（stderr 进 message）；`with_global` 注入 `--non-interactive` |
| `svn_xml::parse_info` / `parse_node_revs` / `parse_external_roots` / `parse_diff_summary` | 已在树里，roxmltree DOM |
| `svn_wc::enumerate_wcs(repo_root, whitelist) -> Vec<WcIdentity{wc_root,url,repos_root,uuid,…}>` | 已递归覆盖嵌套 external |
| `svn_baseline::parse_trailer(message) -> Option<Trailer{url,rev,uuid}>` | 已在树里 |
| `svn_config::{read_config, config_path}` | 配置在 `<repo>/.git/mygitclient.json` |
| `svn_health::check_baseline(repo_root, svn_url, rev, whitelist, extensions) -> Health` | **目前只对项目根跑一次 diff**，Task 5 改成按 WC 分组 |
| `svn_include_check::expected_set(repo_root, whitelist, extensions) -> BTreeSet<String>` | 磁盘侧 W-file 集合，仓库内相对路径 + 正斜杠 |
| `session.rs` 的原子写 | 先写 `.<pid>.<seq>.tmp` 再 rename，Task 3 照抄 |
| 前端 `computePaths` | `if (!parentNode) continue` —— **悬空父不画线**；但 `computeOwners` 仍会用它算归属，所以接合函数必须自己剔除悬空父 |

---

## File Structure

新增：

| 文件 | 职责 |
|---|---|
| `src-tauri/src/engine/svn_log.rs` | `svn log` 的拉取编排：per-WC target、区间钳制、孤儿 baseline 锚点补拉 |
| `src-tauri/src/engine/svn_log_cache.rs` | `.git/mygitclient-svnlog.json` 的读写与合并（原子写） |
| `src-tauri/src/engine/svn_sync.rs` | §6.6 失同步判定：`nodeRevs` 并集、两条判据、pinned 成因分辨、O(1) 档 |
| `src-tauri/src/engine/svn_server_set.rs` | §6.3.1 的服务器侧证伪：`svn ls -R` 清单 vs 磁盘 W-file（NB-2） |
| `src/engine/svnLog.ts` | SVN 历史的前端类型与日期归一（纯逻辑） |
| `src/engine/svnGraph.ts` | §10.3 两条历史的接合 + `Local-Only` trailer 判定（纯逻辑） |
| `src/engine/svnSync.ts` | 失同步状态 → 横幅文案与可用出口（纯逻辑） |
| `src/components/BaselineBanner.tsx` | 失同步横幅 + 重建基线 / 手动体检入口（NB-6） |
| `src-tauri/tests/svn_log.rs` / `svn_log_cache.rs` / `svn_sync.rs` / `svn_server_set.rs` | 各自的集成测试 |
| `tests/vitest/svnGraph.test.ts` / `svnSync.test.ts` | 前端纯逻辑单测 |

修改：

| 文件 | 改动 |
|---|---|
| `src-tauri/src/engine/svn_xml.rs` | 加 `parse_log`、`parse_list` |
| `src-tauri/src/engine/svn_health.rs` | `check_baseline` 改成**按 WC 分组各体检一次**（§6.6.5 欠账） |
| `src-tauri/src/vcs/mod.rs`、`src-tauri/src/commands/repo.rs` | `Capabilities` 加 `vcs: &'static str` 字段 |
| `src-tauri/src/commands/svn.rs` | 本期 5 个新命令 |
| `src-tauri/src/engine/mod.rs`、`src-tauri/src/lib.rs` | 模块声明与命令注册 |
| `src/engine/types.ts` | `CommitNode` 加 `vcs?` / `svnRev?` / `localOnly?` |
| `src/engine/capabilities.ts` | `Capabilities` 加 `vcs` 字段 |
| `src/store/repoStore.ts` | SVN 状态与历史的拉取、两档检查频次 |
| `src/components/RepoTab.tsx` | 挂 `BaselineBanner` |
| `src/components/CommitRow.tsx` | svn 节点显示 `r12346` + 换配色；`localOnly` 角标 |
| `src/components/LocalChangesView.tsx` | 提交框「仅本地，不交付」勾选框 + 窗口期警告 |
| `src/components/BranchView.tsx` | `svn/base` 特殊标记 + 禁止在其上提交 |
| `src/components/GraphView.tsx` | 底部「加载更多」按钮 |
| `src/styles.css` | 横幅、角标、svn 行配色 |

**为什么 `svn_log.rs` 与 `svn_log_cache.rs` 分开：** 前者走网络、要处理 svn 的七种失败
形态；后者纯本地 IO + 集合合并，可以在没有 svn 的机器上单测。混在一起的话，缓存合并
逻辑的每个测试都要先起一个 SVN fixture。

**为什么接合逻辑在前端而不是 Rust：** `AGENTS.md` §7.2 —— 布局、选择、路径这类纯逻辑
放 `src/engine/`，vitest 比 cargo test 快一个数量级，而接合是纯粹的列表变换。
代价是 `git-svn-id` 正则在两侧各有一份；格式由 spec §6.4.1 钉死且已有测试，接受。

---

## Task 0：开工自检、worktree 与基线数字

**Files:**
- Create: `docs/superpowers/plans/2026-09-15-svn-phase1c-visible-decisions.md`

- [ ] **Step 1: 在动第一行代码之前建好并切入 worktree**

`AGENTS.md` §4.1/§4.2 MUST。Claude Code 用 `EnterWorktree`；基点必须是**最新的
`master`**（`f67a007`，Phase 1b 的 squash 合并）。建完立刻：

```powershell
git log --oneline -3
Get-Location
```

`Get-Location` 的实测值必须就在该 worktree 内。不在就停下来先切进去。

- [ ] **Step 2: 新 worktree 里必须先 `pnpm install`**

1a、1b 连续两期踩过：不装的话 `pnpm test` 会**静默跑主工作区的测试**（`@` 别名解析到
主 checkout 的 `src/`），拿到一个看起来正常、实际测的是另一份代码的数字。

```powershell
Test-Path node_modules    # 预期 False
pnpm install
```

- [ ] **Step 3: 记录改动前的测试基线数字**

```powershell
cd src-tauri; cargo test -j 2
cd ..; pnpm test; pnpm build
```

预期（`f67a007` 上）：Rust **412 passed / 0 failed / 33 个二进制**，前端 **290 passed /
22 files**，`pnpm build` 通过。**实测值与这里不符时以实测为准**，在决策日志里记下差异，
不要改这份计划去迁就。

统计失败数时**不要**用 `Select-String -Pattern 'FAILED'`——PowerShell 默认大小写不敏感，
会把 33 行 `0 failed;` 全部匹配进来。判「非 `test result: ok.` 开头的行数」。

- [ ] **Step 4: 建决策日志骨架**

`AGENTS.md` §2.1 MUST：**在动第一个 Task 之前建好**。含进度总账表（Task 0–14）与
四类决策条目（偏离计划 / 计划的错误 / 计划没写的坑 / 为什么这么改是安全的）的空节。

把上面「开工前已完成的实测」A–E 五组表原样抄进决策日志的「计划没写的坑」前置节——
它们是本期所有判断的事实来源，review 要能对照。

---

## Task 1：`svn_xml` 加 `parse_log` 与 `parse_list`

**Files:**
- Modify: `src-tauri/src/engine/svn_xml.rs`
- Test: `src-tauri/tests/svn_xml.rs`（既有文件，追加）

**Interfaces:**
- Consumes: 无（纯字符串 → 结构）
- Produces:
  ```rust
  pub struct SvnLogEntry { pub revision: u64, pub author: String, pub date: String, pub message: String }
  pub fn parse_log(xml: &str) -> Result<Vec<SvnLogEntry>>
  pub struct SvnListEntry { pub name: String, pub kind: String }
  pub struct SvnListResult { pub base: String, pub entries: Vec<SvnListEntry> }
  pub fn parse_list(xml: &str) -> Result<SvnListResult>
  ```

- [ ] **Step 1: 写失败测试**

追加到 `src-tauri/tests/svn_xml.rs`。**XML 字面量直接抄本计划 A/D 组实测的输出形状，
包括属性逐行排列与 CRLF 无关的换行**——照着想象写的 XML 测不出行内正则那个坑。

```rust
#[test]
fn parse_log_reads_revision_author_date_and_unescaped_message() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<log>
<logentry
   revision="3">
<author>liuhao</author>
<date>2026-09-15T15:21:22.337313Z</date>
<msg>r3 deep change plus new dir</msg>
</logentry>
<logentry
   revision="2">
<author>liuhao</author>
<date>2026-09-15T15:21:22.106490Z</date>
<msg>r2 subject line

body line with &lt;angle&gt; &amp; ampersand</msg>
</logentry>
</log>"#;
    let got = svn_xml::parse_log(xml).unwrap();
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].revision, 3);
    assert_eq!(got[0].author, "liuhao");
    assert_eq!(got[0].date, "2026-09-15T15:21:22.337313Z");
    // 实体必须已经解码，且多行 message 原样保留
    assert_eq!(
        got[1].message,
        "r2 subject line\n\nbody line with <angle> & ampersand"
    );
}

#[test]
fn parse_log_treats_missing_msg_as_empty_not_error() {
    // `svn log -q` 实测：<msg> 整个元素不出现（Q5）
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<log>
<logentry
   revision="4">
<author>liuhao</author>
<date>2026-09-15T15:23:50.323496Z</date>
</logentry>
</log>"#;
    let got = svn_xml::parse_log(xml).unwrap();
    assert_eq!(got[0].message, "");
}

#[test]
fn parse_log_accepts_empty_log_from_empty_repository() {
    // 空仓库实测 exit 0 + <log></log>（P9）。空历史不是错误。
    let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<log>\n</log>\n";
    assert!(svn_xml::parse_log(xml).unwrap().is_empty());
}

#[test]
fn parse_log_rejects_a_logentry_without_revision() {
    // revision 是本模块唯一的身份字段，缺了就不能猜。
    let xml = "<log><logentry><author>x</author></logentry></log>";
    assert!(svn_xml::parse_log(xml).is_err());
}

#[test]
fn parse_list_reads_base_and_recursive_names_with_kind() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<lists>
<list
   path="file:///C:/tmp/repo">
<entry
   kind="dir">
<name>Code</name>
<commit
   revision="1">
<author>liuhao</author>
<date>2026-09-15T15:22:51.983752Z</date>
</commit>
</entry>
<entry
   kind="file">
<name>Code/A.cs</name>
<size>10</size>
<commit
   revision="1">
<author>liuhao</author>
<date>2026-09-15T15:22:51.983752Z</date>
</commit>
</entry>
</list>
</lists>"#;
    let got = svn_xml::parse_list(xml).unwrap();
    assert_eq!(got.base, "file:///C:/tmp/repo");
    assert_eq!(got.entries.len(), 2);
    assert_eq!(got.entries[0].kind, "dir");
    assert_eq!(got.entries[0].name, "Code");
    assert_eq!(got.entries[1].kind, "file");
    assert_eq!(got.entries[1].name, "Code/A.cs");
}

#[test]
fn parse_list_accepts_a_legitimately_empty_listing() {
    // 空仓库 @0 实测 exit 0 + 空 <lists>（P10）。
    // ⚠️ 路径不存在时（P1）stdout 也长这样，但那次 exit 是 1 —— 那道门由
    // run_svn 的非零退出判定把守，不在本函数里。
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<lists>
<list
   path="file:///C:/tmp/empty">
</list>
</lists>"#;
    let got = svn_xml::parse_list(xml).unwrap();
    assert_eq!(got.base, "file:///C:/tmp/empty");
    assert!(got.entries.is_empty());
}

#[test]
fn parse_list_rejects_truncated_output() {
    // revision 不存在时 svn 把 XML 截断在半路（P2）。roxmltree 必须报错，
    // 不能当成「空清单」。
    let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<lists>\n<list\n   path=\"file:///C:/tmp/repo\">\n";
    assert!(svn_xml::parse_list(xml).is_err());
}
```

- [ ] **Step 2: 跑测试确认失败**

```powershell
cd src-tauri; cargo test --test svn_xml
```

预期：编译失败，`parse_log` / `parse_list` 未定义。

- [ ] **Step 3: 实现**

加到 `src-tauri/src/engine/svn_xml.rs` 末尾。**沿用本文件既有的 roxmltree DOM 写法
（`descendants()` 按 tag 名筛），不要引入流式解析。**

```rust
/// `svn log --xml` 的一条 entry。
///
/// # 为什么 `message` 缺失算空而不算错
///
/// 2026-09-15 实测：`svn log -q` 的输出里 `<msg>` **整个元素不出现**（不是空元素）。
/// 本期不用 `-q`，但把「缺失 = 空」写死在这里，是为了让将来有人为了省流量加上
/// `-q` 时不会得到一个「历史拉取整体失败」。`revision` 反过来——它是这条记录的
/// 身份，缺了只能报错。
#[derive(Debug, Clone, PartialEq)]
pub struct SvnLogEntry {
    pub revision: u64,
    /// 匿名提交或 revprop 被删时为空串。
    pub author: String,
    /// ISO8601 原样，不在这里解析成时间戳——归一化是前端 `svnLog.ts` 的事，
    /// 免得 Rust 侧和 TS 侧各有一套时区处理。
    pub date: String,
    pub message: String,
}

/// 解析 `svn log --xml`。
///
/// ⚠️ 属性逐行排列（`<logentry⏎   revision="3">`），行内正则会静默扫出空集，
/// 见 spec §6.6.4 陷阱三。必须走 DOM。
pub fn parse_log(xml: &str) -> Result<Vec<SvnLogEntry>> {
    let doc = parse_doc(xml)?;
    let mut out = Vec::new();
    for node in doc.descendants().filter(|n| n.has_tag_name("logentry")) {
        let revision = node
            .attribute("revision")
            .and_then(|s| s.parse::<u64>().ok())
            .ok_or_else(|| {
                GitError::new(
                    "svn_xml_log_no_revision",
                    "svn log 的 logentry 缺少可解析的 revision 属性",
                )
            })?;
        let child_text = |tag: &str| {
            node.children()
                .find(|c| c.has_tag_name(tag))
                .and_then(|c| c.text())
                .unwrap_or("")
                .to_string()
        };
        out.push(SvnLogEntry {
            revision,
            author: child_text("author"),
            date: child_text("date"),
            message: child_text("msg"),
        });
    }
    Ok(out)
}

/// `svn ls -R --xml` 的一条 entry。
#[derive(Debug, Clone, PartialEq)]
pub struct SvnListEntry {
    /// **相对 [`SvnListResult::base`] 的递归路径，正斜杠。**
    ///
    /// 实测（N4）：target 换成子路径 URL 时，这里会相对**那个子路径**重新编号，
    /// 不是仓库绝对路径。调用方必须自己接前缀，不能假设它是仓库绝对路径——
    /// 这与同文件 `parse_diff_summary` 给的**完整 URL** 又不一样。
    pub name: String,
    /// `"file"` 或 `"dir"`。
    pub kind: String,
}

/// `svn ls -R --xml` 的解析结果。
pub struct SvnListResult {
    /// `<list path="…">` 的值，即本次列举的基准 URL。
    pub base: String,
    pub entries: Vec<SvnListEntry>,
}

/// 解析 `svn ls -R --xml`。
///
/// # ⚠️ 空清单在这里是合法的，判「是不是失败」不归本函数
///
/// 实测 P1：路径不存在时 svn **exit 1**，但 stdout 仍是一份结构完整的空
/// `<lists><list></list></lists>`。若调用方忽略退出码，就会拿到一个空的服务器
/// 清单——而「服务器有、磁盘没有」的差集在空集上恒为空，于是**报告没有漏收**。
/// 那正是本期要修的 NB-2 被重新造一遍。
///
/// 所以：调用方必须走 [`crate::engine::svn_cli::run_svn`]（非零退出即 `Err`），
/// 禁止用 `run_svn_code` 自行判断。
pub fn parse_list(xml: &str) -> Result<SvnListResult> {
    let doc = parse_doc(xml)?;
    let list = doc
        .descendants()
        .find(|n| n.has_tag_name("list"))
        .ok_or_else(|| {
            GitError::new("svn_xml_list_no_list", "svn ls 的输出里没有 <list> 元素")
        })?;
    let base = list.attribute("path").unwrap_or("").trim_end_matches('/').to_string();
    let mut entries = Vec::new();
    for node in list.children().filter(|n| n.has_tag_name("entry")) {
        let Some(name) = node
            .children()
            .find(|c| c.has_tag_name("name"))
            .and_then(|c| c.text())
        else {
            continue;
        };
        entries.push(SvnListEntry {
            name: name.replace('\\', "/"),
            kind: node.attribute("kind").unwrap_or("").to_string(),
        });
    }
    Ok(SvnListResult { base, entries })
}
```

如果本文件里还没有共用的 `parse_doc` 辅助函数，就照既有各 `parse_*` 的写法内联
`roxmltree::Document::parse(xml).map_err(...)`，**不要**为此重构既有函数（`AGENTS.md`
§1.2：不要把无关重构和业务修改混在一起）。

- [ ] **Step 4: 跑测试确认通过**

```powershell
cd src-tauri; cargo test --test svn_xml
cargo fmt --check; cargo clippy --all-targets
```

- [ ] **Step 5: 变异测试——确认「必须走 DOM」那条断言真的有牙**

把 `parse_log` 临时改成按行正则 `<logentry.*revision="(\d+)"` 取值，重跑
`cargo test --test svn_xml`。**必须变红**（属性在下一行，正则扫空 → 返回空列表）。
确认后改回来，把结果记进决策日志。这一步不提交。

- [ ] **Step 6: 提交**

```bash
git add src-tauri/src/engine/svn_xml.rs src-tauri/tests/svn_xml.rs
git commit -m "feat(svn): svn_xml 加 log 与 ls 两个解析器"
```

---

## Task 2：`svn_log` 拉取编排

**Files:**
- Create: `src-tauri/src/engine/svn_log.rs`
- Modify: `src-tauri/src/engine/mod.rs`
- Test: `src-tauri/tests/svn_log.rs`

**Interfaces:**
- Consumes: `svn_xml::{parse_log, SvnLogEntry}`、`svn_cli::run_svn`、`svn_wc::enumerate_wcs`、`svn_config::read_config`
- Produces:
  ```rust
  pub fn head_revision(repo_root: &str, url: &str) -> Result<u64>
  pub fn fetch_range(repo_root: &str, url: &str, from: u64, to: u64, limit: u32) -> Result<Vec<SvnLogEntry>>
  pub fn fetch_anchor(repo_root: &str, repos_root: &str, rev: u64) -> Result<Option<SvnLogEntry>>
  pub fn log_targets(repo_root: &str) -> Result<Vec<String>>
  ```

- [ ] **Step 1: 写失败测试**

```rust
mod common;
use common::svn_fixtures;
use mygitclient_lib::engine::svn_log;

#[test]
fn head_revision_reads_the_servers_latest() {
    let fx = svn_fixtures::svn_repo();   // 既有 fixture，HEAD 已知
    let head = svn_log::head_revision(&fx.wc, &fx.url).unwrap();
    assert!(head >= 1, "HEAD 应当至少是 r1，实际 {head}");
}

#[test]
fn fetch_range_clamps_to_head_instead_of_failing() {
    // 实测 P4/P5：-r 的任一端越界，整条命令 exit 1 (E160006)。
    // 增量拉取天然会算出「上次覆盖到 N，现在拉 N+1..HEAD」，而 N+1 可能 > HEAD。
    let fx = svn_fixtures::svn_repo();
    let head = svn_log::head_revision(&fx.wc, &fx.url).unwrap();
    let got = svn_log::fetch_range(&fx.wc, &fx.url, head + 5, head + 9, 100).unwrap();
    assert!(got.is_empty(), "整段都在 HEAD 之上时应当返回空，而不是报错");
}

#[test]
fn fetch_range_never_passes_limit_zero() {
    // 实测 P6：--limit 0 → exit 1 (E200004)。limit 是调用方算出来的，会是 0。
    let fx = svn_fixtures::svn_repo();
    let head = svn_log::head_revision(&fx.wc, &fx.url).unwrap();
    let got = svn_log::fetch_range(&fx.wc, &fx.url, head, 1, 0).unwrap();
    assert!(got.is_empty(), "limit=0 应当被判成「不用拉」，而不是送给 svn");
}

#[test]
fn fetch_anchor_finds_a_revision_that_never_touched_the_whitelist() {
    // C 组实测：baselineRev 通常就是一个没碰过白名单的 revision。
    // 这条测试是本期对 spec §10.3 那处修正的回归锚点。
    let fx = svn_fixtures::svn_repo_baseline_rev_outside_whitelist();
    // 白名单 URL 上查不到
    let none = svn_log::fetch_range(&fx.wc, &fx.whitelist_url, fx.baseline_rev, fx.baseline_rev, 10).unwrap();
    assert!(none.is_empty(), "白名单 URL 上不该有这个 revision —— 这正是孤儿 baseline 的成因");
    // Repository Root 上查得到
    let got = svn_log::fetch_anchor(&fx.wc, &fx.repos_root, fx.baseline_rev).unwrap();
    assert!(got.is_some(), "锚点必须打给 Repository Root 才拿得到");
    assert_eq!(got.unwrap().revision, fx.baseline_rev);
}

#[test]
fn log_targets_returns_one_url_per_working_copy() {
    // 实测 Q4：svn log 拒绝多个 URL target。所以是每个 WC 一次调用，客户端合并。
    let fx = svn_fixtures::svn_repo_with_same_repo_nested_external(
        svn_fixtures::NestedExternal::Clean,
    );
    let targets = svn_log::log_targets(&fx.wc).unwrap();
    assert!(targets.len() >= 2, "嵌套 external 是独立工作副本，必须各占一个 target");
    for t in &targets {
        assert!(!t.ends_with('@'), "log 的 target 不能带 peg 尾缀（实测 R5 会 E160013）");
    }
}
```

`svn_repo_baseline_rev_outside_whitelist` 是本 Task 要**新增**的 fixture，形状照
C 组实测：r1 建 `Code/` 与 `Other/`；r2 只动 `Code`；r3、r4 只动 `Other`；然后
`svn update`。它要额外暴露 `whitelist_url`（`<repos_root>/Code`）、`repos_root`、
`baseline_rev`（= 4）三个字段。

- [ ] **Step 2: 跑测试确认失败**

```powershell
cd src-tauri; cargo test --test svn_log
```

预期：编译失败，模块未定义。

- [ ] **Step 3: 实现**

```rust
//! `svn log` 的拉取编排。
//!
//! 本模块只负责**打给谁、打什么区间、失败算谁的**；解析归 `svn_xml`，缓存归
//! `svn_log_cache`。
//!
//! # 为什么是每个工作副本一次调用
//!
//! 2026-09-15 实测：
//! - `svn log <url1> <url2>` → `E205000: Only relative paths can be specified
//!   after a URL`（Q4）—— 多 URL 不行。
//! - `svn log <url> Code Other` → ✅ 成功，返回并集（R1）—— URL + 多个相对路径可以。
//! - `svn log . Code` → `E200007: When specifying working copy paths, only one
//!   target may be given`（R3）—— 工作副本路径只能给一个。
//!
//! R1 那个形态看起来能把 N 次调用压成 1 次，**但没有采用**，理由是 R4：
//! `svn log <url> Code NoSuch` 中只要有**一条**相对路径在该 revision 上不存在，
//! **整次调用 exit 1**，一条历史都拿不到。白名单里的目录被上游删掉、或在早期
//! revision 上还不存在，都是正常情况，而本设计的白名单**天然可能落在不同的
//! external / 工作副本里**（spec §6.6.5），URL 前缀本来就不止一个。
//! 所以：每个 WC 一次调用，一个失败只损失那一个 WC 的历史。

use crate::engine::svn_cli::run_svn;
use crate::engine::svn_xml::{parse_log, SvnLogEntry};
use crate::error::Result;

/// 读服务器当前 HEAD 的 revision 号。
///
/// 走 `svn info --show-item revision <url>`：实测（P7）stdout 就是裸的十进制数字，
/// 不需要过 XML 解析。**`svn info` 绝不能带 `--ignore-externals`**（exit 1）。
pub fn head_revision(repo_root: &str, url: &str) -> Result<u64> {
    let out = run_svn(repo_root, &["info", "--show-item", "revision", "--", url])?;
    let text = out.trim();
    text.parse::<u64>().map_err(|_| {
        crate::error::GitError::new(
            "svn_log_bad_head",
            format!("svn info --show-item revision 返回了不是数字的内容：`{text}`"),
        )
    })
}

/// 拉一段区间的 log。**区间两端都会先被钳到 `[1, HEAD]`**。
///
/// # 为什么必须钳制而不是把错误抛上去
///
/// 实测 P4/P5：`-r` 的**任一端**越界，整条命令 `exit 1` 并报
/// `E160006: No such revision N`。而增量拉取的自然写法就是「上次覆盖到 N，
/// 这次拉 `N+1 : HEAD`」——当没有新提交时 `N+1 > HEAD`，于是**每一次「没有新东西」
/// 都会变成一次错误**。把这种情况判成「拉到 0 条」才是它真实的含义。
///
/// `limit == 0` 同理：实测 P6 `--limit 0` 直接 `E200004`，而 limit 是调用方算出来的
/// 剩余配额，算到 0 是正常的。
pub fn fetch_range(
    repo_root: &str,
    url: &str,
    from: u64,
    to: u64,
    limit: u32,
) -> Result<Vec<SvnLogEntry>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let head = head_revision(repo_root, url)?;
    if head == 0 {
        // 空仓库。实测 P9：svn log 对 HEAD=0 返回 exit 0 + 空 <log>，
        // 但我们连命令都不用发。
        return Ok(Vec::new());
    }
    let clamp = |v: u64| v.clamp(1, head);
    let (lo, hi) = (clamp(from), clamp(to));
    // 钳制之后区间可能塌成同一个点，那仍然是一次合法查询（Q2 就是 -r N:N）。
    // 但若原区间**整段**在 HEAD 之上，钳完两端都等于 head，会错误地拉回 head 那条。
    if from > head && to > head {
        return Ok(Vec::new());
    }
    let range = format!("{lo}:{hi}");
    let limit_s = limit.to_string();
    let xml = run_svn(
        repo_root,
        &["log", "--xml", "-r", &range, "--limit", &limit_s, "--", url],
    )?;
    parse_log(&xml)
}

/// 补拉一个**孤儿 baseline** 的锚点 commit。
///
/// # 这不是边界情况，是常态（2026-09-15 实测，C 组）
///
/// spec §10.3 把孤儿 baseline 描述成「若某个 baseline 的 revision **太老**」。
/// 成因不是太老：`svn update` 会把白名单内**所有**节点的 `wc-status revision`
/// 一律推到当时的 HEAD，所以 `baselineRev` 通常就是一个**根本没碰过白名单**的
/// revision（实测 Q0：只动 `Other` 的 r3/r4 之后，`Code` 的 nodeRevs = {4}）。
///
/// 于是：
/// - `svn log -r 4:4 <url>/Code` → **空**（Q1）
/// - `svn log -r 4:4 <Repository Root>` → ✅ 拿到 r4（Q2）
///
/// **锚点必须打给 Repository Root。** 打给白名单 URL 会拿到空结果，图上那个
/// `svn:N` 节点就永远建不出来，工作分支挂不上去。
pub fn fetch_anchor(repo_root: &str, repos_root: &str, rev: u64) -> Result<Option<SvnLogEntry>> {
    let entries = fetch_range(repo_root, repos_root, rev, rev, 1)?;
    Ok(entries.into_iter().next())
}

/// 本仓库要拉哪些 target：每个白名单所属工作副本的 URL，各一条，已去重。
///
/// **不加 `peg_safe` 的空 peg 尾缀。** 这是与 `svn info` / `svn status` 相反的一条：
/// 实测 R5，`svn log <URL> "Code@"` 报 `E160013: File not found: path '/Code@'`——
/// 这里的相对路径是**纯路径**，`@` 被当字面字符。同一族的第三个例外（前两个是
/// `svn diff` 的尾随 target 与本条），见 Global Constraints 的对照表。
pub fn log_targets(repo_root: &str) -> Result<Vec<String>> {
    let cfg = crate::engine::svn_config::read_config(repo_root)?;
    let wcs = crate::engine::svn_wc::enumerate_wcs(repo_root, &cfg.whitelist)?;
    let mut out: Vec<String> = Vec::new();
    for wc in wcs {
        let url = wc.url.trim_end_matches('/').to_string();
        if !out.iter().any(|u| u.eq_ignore_ascii_case(&url)) {
            out.push(url);
        }
    }
    Ok(out)
}
```

- [ ] **Step 4: 跑测试确认通过**

```powershell
cd src-tauri; cargo test --test svn_log
cargo fmt --check; cargo clippy --all-targets
```

- [ ] **Step 5: 变异测试——确认钳制那条断言有牙**

把 `fetch_range` 里的钳制整段删掉，重跑。`fetch_range_clamps_to_head_instead_of_failing`
**必须变红**（`E160006`）。确认后改回来，记进决策日志。不提交。

- [ ] **Step 6: 提交**

```bash
git add src-tauri/src/engine/svn_log.rs src-tauri/src/engine/mod.rs \
        src-tauri/tests/svn_log.rs src-tauri/tests/common/svn_fixtures.rs
git commit -m "feat(svn): svn log 拉取编排——区间钳制与孤儿 baseline 锚点"
```

---

## Task 3：`svn_log_cache` 缓存与合并

**Files:**
- Create: `src-tauri/src/engine/svn_log_cache.rs`
- Modify: `src-tauri/src/engine/mod.rs`
- Test: `src-tauri/tests/svn_log_cache.rs`

**Interfaces:**
- Consumes: `svn_xml::SvnLogEntry`
- Produces:
  ```rust
  pub struct CachedRev { pub rev: u64, pub author: String, pub date: String, pub message: String }
  pub struct TargetLog { pub covered_min: u64, pub covered_max: u64, pub entries: Vec<CachedRev> }
  pub struct SvnLogCache { pub version: u32, pub uuid: String,
                           pub targets: BTreeMap<String, TargetLog>,
                           pub anchors: BTreeMap<u64, CachedRev> }
  pub fn cache_path(repo_root: &str) -> PathBuf
  pub fn read_cache(repo_root: &str, uuid: &str) -> SvnLogCache
  pub fn write_cache(repo_root: &str, cache: &SvnLogCache) -> Result<()>
  pub fn merge_into(target: &mut TargetLog, fetched: &[SvnLogEntry], from: u64, to: u64)
  pub fn merged_sequence(cache: &SvnLogCache) -> Vec<CachedRev>
  ```

- [ ] **Step 1: 写失败测试**

```rust
use mygitclient_lib::engine::svn_log_cache::{self, CachedRev, SvnLogCache, TargetLog};
use mygitclient_lib::engine::svn_xml::SvnLogEntry;
use std::collections::BTreeMap;

fn entry(rev: u64) -> SvnLogEntry {
    SvnLogEntry { revision: rev, author: "a".into(), date: "d".into(), message: format!("r{rev}") }
}

#[test]
fn merge_extends_the_covered_range_and_dedupes() {
    let mut t = TargetLog { covered_min: 5, covered_max: 8, entries: vec![] };
    svn_log_cache::merge_into(&mut t, &[entry(8), entry(7), entry(5)], 5, 8);
    svn_log_cache::merge_into(&mut t, &[entry(10), entry(8)], 8, 10);
    assert_eq!(t.covered_min, 5);
    assert_eq!(t.covered_max, 10);
    let revs: Vec<u64> = t.entries.iter().map(|e| e.rev).collect();
    // 降序、无重复
    assert_eq!(revs, vec![10, 8, 7, 5]);
}

#[test]
fn merge_does_not_widen_coverage_across_a_gap() {
    // 只拉了 20..25，而已覆盖区间是 5..8。中间 9..19 没拉过，
    // 把 covered 记成 5..25 会让下一次增量拉取**跳过**那一段，永久丢历史。
    let mut t = TargetLog { covered_min: 5, covered_max: 8, entries: vec![] };
    svn_log_cache::merge_into(&mut t, &[entry(25), entry(20)], 20, 25);
    assert_eq!(t.covered_max, 25, "上界可以往上走");
    assert_eq!(t.covered_min, 20, "下界必须跟着跳到新区间的起点，不能假装 5..25 全覆盖");
}

#[test]
fn a_cache_with_a_different_uuid_is_discarded_whole() {
    // 同一个磁盘路径换了仓库（重新 checkout 到别的 URL）时，旧缓存的 revision
    // 号在新仓库里指向完全不同的提交。混用会画出一张**看起来正常**的假历史。
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().to_str().unwrap();
    std::fs::create_dir_all(dir.path().join(".git")).unwrap();
    let mut cache = SvnLogCache::new("uuid-A");
    cache.anchors.insert(4, CachedRev { rev: 4, author: "a".into(), date: "d".into(), message: "m".into() });
    svn_log_cache::write_cache(root, &cache).unwrap();

    let same = svn_log_cache::read_cache(root, "uuid-A");
    assert_eq!(same.anchors.len(), 1);

    let other = svn_log_cache::read_cache(root, "uuid-B");
    assert!(other.anchors.is_empty(), "uuid 不匹配必须整份作废");
    assert_eq!(other.uuid, "uuid-B");
}

#[test]
fn a_corrupt_cache_file_degrades_to_empty_instead_of_failing() {
    // 缓存是**加速**手段，不是事实来源。坏了就重拉，不能让仓库打不开。
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().to_str().unwrap();
    std::fs::create_dir_all(dir.path().join(".git")).unwrap();
    std::fs::write(svn_log_cache::cache_path(root), b"{ this is not json").unwrap();
    let got = svn_log_cache::read_cache(root, "uuid-A");
    assert!(got.targets.is_empty() && got.anchors.is_empty());
}

#[test]
fn merged_sequence_unions_all_targets_and_anchors_descending() {
    let mut cache = SvnLogCache::new("u");
    let mut a = TargetLog { covered_min: 1, covered_max: 3, entries: vec![] };
    svn_log_cache::merge_into(&mut a, &[entry(3), entry(1)], 1, 3);
    let mut b = TargetLog { covered_min: 1, covered_max: 2, entries: vec![] };
    svn_log_cache::merge_into(&mut b, &[entry(2), entry(1)], 1, 2);
    cache.targets.insert("url-a".into(), a);
    cache.targets.insert("url-b".into(), b);
    cache.anchors.insert(4, CachedRev { rev: 4, author: "a".into(), date: "d".into(), message: "anchor".into() });

    let seq = svn_log_cache::merged_sequence(&cache);
    let revs: Vec<u64> = seq.iter().map(|e| e.rev).collect();
    assert_eq!(revs, vec![4, 3, 2, 1], "并集、去重、降序");
}

#[test]
fn write_cache_is_atomic_and_leaves_no_temp_file_behind() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().to_str().unwrap();
    std::fs::create_dir_all(dir.path().join(".git")).unwrap();
    svn_log_cache::write_cache(root, &SvnLogCache::new("u")).unwrap();
    let leftovers: Vec<_> = std::fs::read_dir(dir.path().join(".git"))
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "临时文件必须被 rename 掉：{leftovers:?}");
}
```

- [ ] **Step 2: 跑测试确认失败**

```powershell
cd src-tauri; cargo test --test svn_log_cache
```

- [ ] **Step 3: 实现**

```rust
//! SVN 历史缓存：`<仓库根>/.git/mygitclient-svnlog.json`。
//!
//! 放在 `.git/` 内部而不是工作目录，理由同 `svn_config`：工作目录**只多出 `.git`
//! 一个条目**，多一个文件就多一个被 TortoiseSVN 误提交上去的东西（spec §3）。
//!
//! # 为什么可以永久缓存
//!
//! SVN 历史不可变（除非有人改 revprop，罕见到可忽略，spec §10.2）。所以已拉取的
//! revision 只增不改，增量只拉新的。
//!
//! # 缓存是加速手段，不是事实来源
//!
//! 任何一种读失败（文件不存在、JSON 坏了、版本号不认识、uuid 对不上）都**降级成
//! 空缓存**，绝不向上抛错。让一个加速用的 JSON 把仓库打不开，是本模块最坏的失败
//! 形态。

use crate::engine::svn_xml::SvnLogEntry;
use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const CACHE_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedRev {
    pub rev: u64,
    pub author: String,
    pub date: String,
    pub message: String,
}

/// 一个 `svn log` target 已缓存的历史。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetLog {
    /// 已**连续**覆盖区间的下界（含）。往下翻页时朝这个方向扩。
    pub covered_min: u64,
    /// 已**连续**覆盖区间的上界（含）。增量拉取只拉 `(covered_max, HEAD]`。
    pub covered_max: u64,
    /// 按 revision **降序**。
    pub entries: Vec<CachedRev>,
}

/// 整份缓存。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SvnLogCache {
    pub version: u32,
    /// 仓库身份。与当前 `svn info` 的 UUID 不符时整份作废——同一个磁盘路径重新
    /// checkout 到别的仓库之后，旧的 revision 号在新仓库里指向完全不同的提交，
    /// 混用会画出一张**看起来完全正常**的假历史。
    pub uuid: String,
    /// key = target URL（`svn_log::log_targets` 的产物）。
    pub targets: BTreeMap<String, TargetLog>,
    /// 孤儿 baseline 的锚点，key = revision。
    ///
    /// **必须与 `targets` 分开存。** 它们是逐个 revision 单点补拉的（`-r N:N`
    /// 打给 Repository Root），**不构成连续区间**；记进某个 target 的
    /// `covered_min/max` 会谎称中间那一大段也拉过了，下一次增量拉取就会跳过它。
    pub anchors: BTreeMap<u64, CachedRev>,
}

impl SvnLogCache {
    pub fn new(uuid: &str) -> Self {
        Self {
            version: CACHE_VERSION,
            uuid: uuid.to_string(),
            targets: BTreeMap::new(),
            anchors: BTreeMap::new(),
        }
    }
}

pub fn cache_path(repo_root: &str) -> PathBuf {
    Path::new(repo_root).join(".git").join("mygitclient-svnlog.json")
}

/// 读缓存。**任何失败都返回一份干净的空缓存**，永不报错。
pub fn read_cache(repo_root: &str, uuid: &str) -> SvnLogCache {
    let fallback = SvnLogCache::new(uuid);
    let Ok(raw) = std::fs::read_to_string(cache_path(repo_root)) else {
        return fallback;
    };
    let Ok(parsed) = serde_json::from_str::<SvnLogCache>(&raw) else {
        return fallback;
    };
    if parsed.version != CACHE_VERSION || !parsed.uuid.eq_ignore_ascii_case(uuid) {
        return fallback;
    }
    parsed
}

/// 原子写：先写同目录临时文件再 rename，形状照抄 `session.rs`。
///
/// 直接覆写的话，写到一半掉电会留下一份半截 JSON；虽然 `read_cache` 会把它降级
/// 成空缓存（不致命），但用户会看到历史莫名其妙整个消失一次。
pub fn write_cache(repo_root: &str, cache: &SvnLogCache) -> Result<()> {
    let path = cache_path(repo_root);
    let dir = path.parent().expect("cache_path 恒有父目录");
    let mut tmp = dir.to_path_buf();
    tmp.push(format!(
        "mygitclient-svnlog.{}.{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    ));
    std::fs::write(&tmp, serde_json::to_vec_pretty(cache).map_err(|e| {
        crate::error::GitError::new("svn_log_cache_encode", e.to_string())
    })?)?;
    // rename 失败时别把临时文件留在 .git 里
    if let Err(e) = std::fs::rename(&tmp, &path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

/// 把一次拉取的结果并进某个 target。
///
/// # 覆盖区间只记**连续**的那一段
///
/// `covered_min/max` 的用途是「下次从哪儿接着拉」，所以它必须是一段**真的没有洞**
/// 的区间。若新区间与旧区间不相接（中间隔着没拉过的 revision），就**整段换成新区间**
/// 而不是取并集的最值——后者会谎称中间那段也拉过了，下一次增量拉取跳过它，
/// 那段历史就永久缺失，而且界面上看不出来。
pub fn merge_into(target: &mut TargetLog, fetched: &[SvnLogEntry], from: u64, to: u64) {
    let (lo, hi) = if from <= to { (from, to) } else { (to, from) };
    for e in fetched {
        if !target.entries.iter().any(|c| c.rev == e.revision) {
            target.entries.push(CachedRev {
                rev: e.revision,
                author: e.author.clone(),
                date: e.date.clone(),
                message: e.message.clone(),
            });
        }
    }
    target.entries.sort_by(|a, b| b.rev.cmp(&a.rev));

    let overlaps = lo <= target.covered_max.saturating_add(1)
        && hi.saturating_add(1) >= target.covered_min;
    if overlaps {
        target.covered_min = target.covered_min.min(lo);
        target.covered_max = target.covered_max.max(hi);
    } else {
        target.covered_min = lo;
        target.covered_max = hi;
    }
}

/// 所有 target 与锚点的并集，按 revision 降序去重。
///
/// 这是画图用的那条「SVN 序列」（spec §10.3 ①）。
pub fn merged_sequence(cache: &SvnLogCache) -> Vec<CachedRev> {
    let mut by_rev: BTreeMap<u64, CachedRev> = BTreeMap::new();
    for t in cache.targets.values() {
        for e in &t.entries {
            by_rev.entry(e.rev).or_insert_with(|| e.clone());
        }
    }
    for (rev, e) in &cache.anchors {
        by_rev.entry(*rev).or_insert_with(|| e.clone());
    }
    by_rev.into_values().rev().collect()
}
```

初始化一个新 `TargetLog` 时，用 `TargetLog { covered_min: u64::MAX, covered_max: 0,
entries: vec![] }` 会让上面的相接判断写起来别扭。**统一约定：新建时
`covered_min = 0, covered_max = 0`，并在 `merge_into` 里把 `covered_max == 0 &&
entries.is_empty()` 当成「还没覆盖任何东西」，直接整段赋值。** 把这条写进
`merge_into` 的开头，并补一条测试
`merge_into_on_a_fresh_target_sets_the_range_directly`。

- [ ] **Step 4: 跑测试确认通过**

```powershell
cd src-tauri; cargo test --test svn_log_cache
cargo fmt --check; cargo clippy --all-targets
```

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/engine/svn_log_cache.rs src-tauri/src/engine/mod.rs src-tauri/tests/svn_log_cache.rs
git commit -m "feat(svn): SVN 历史缓存与增量合并"
```

---

## Task 4：`svn_sync` 失同步判定

**Files:**
- Create: `src-tauri/src/engine/svn_sync.rs`
- Modify: `src-tauri/src/engine/mod.rs`
- Test: `src-tauri/tests/svn_sync.rs`

**Interfaces:**
- Consumes: `svn_wc::{enumerate_wcs, peg_safe}`、`svn_xml::parse_node_revs`、`svn_baseline::parse_trailer`、`svn_externals::detect_pinned`、`svn_config::read_config`
- Produces:
  ```rust
  pub enum SyncVerdict { InSync, MixedRevision { revs: Vec<u64> },
                         Diverged { node_rev: u64, baseline_rev: u64 },
                         PinnedReset { node_rev: u64, baseline_rev: u64, external: String } }
  pub struct SyncStatus { pub baseline_rev: u64, pub node_revs: Vec<u64>, pub verdict: SyncVerdict }
  pub struct RootRev { pub wc_root: String, pub rev: u64 }
  pub fn check_sync(repo_root: &str) -> Result<SyncStatus>
  pub fn read_root_revs(repo_root: &str) -> Result<Vec<RootRev>>
  ```

- [ ] **Step 1: 写失败测试**

```rust
mod common;
use common::svn_fixtures;
use mygitclient_lib::engine::{svn_sync, svn_sync::SyncVerdict};

#[test]
fn a_freshly_initialized_repo_is_in_sync() {
    let fx = svn_fixtures::svn_repo_initialized();   // 已跑过 init_hybrid
    let st = svn_sync::check_sync(&fx.wc).unwrap();
    assert_eq!(st.verdict, SyncVerdict::InSync);
    assert_eq!(st.node_revs.len(), 1);
    assert_eq!(st.node_revs[0], st.baseline_rev);
}

#[test]
fn mixed_revision_is_reported_with_every_value() {
    // 外部只提交了一个深层文件 —— 根的 revision 纹丝不动（spec §6.6.1），
    // 只有 nodeRevs 多值这一条能抓到。
    let fx = svn_fixtures::svn_repo_initialized_then_external_deep_commit();
    let st = svn_sync::check_sync(&fx.wc).unwrap();
    match st.verdict {
        SyncVerdict::MixedRevision { revs } => assert!(revs.len() >= 2, "要把每个值都列出来：{revs:?}"),
        other => panic!("应当判成 mixed revision，实际 {other:?}"),
    }
}

#[test]
fn a_single_value_that_differs_from_the_baseline_is_diverged() {
    let fx = svn_fixtures::svn_repo_initialized_then_external_update();
    let st = svn_sync::check_sync(&fx.wc).unwrap();
    match st.verdict {
        SyncVerdict::Diverged { node_rev, baseline_rev } => assert_ne!(node_rev, baseline_rev),
        other => panic!("应当判成 diverged，实际 {other:?}"),
    }
}

#[test]
fn a_reset_to_the_externals_peg_is_distinguished_from_a_plain_divergence() {
    // spec §6.6.2：N 恰好等于该 external 的 peg revision ⇒ 有人在项目根跑了
    // 递归 svn update，把白名单拽回了 peg。这时的出路是一条
    // `svn update <白名单目录>`，**不是** §9.4 的完整恢复流程。
    // 两者给的引导相反，判错就是把用户送进一个不必要的重型流程。
    let fx = svn_fixtures::svn_repo_pinned_then_root_update();
    let st = svn_sync::check_sync(&fx.wc).unwrap();
    match st.verdict {
        SyncVerdict::PinnedReset { external, .. } => assert!(!external.is_empty(), "要点名是哪条 external"),
        other => panic!("应当判成 pinned reset，实际 {other:?}"),
    }
}

#[test]
fn node_revs_unions_every_working_copy_including_nested_externals() {
    // spec §6.6.5：不变量是所有白名单所属工作副本的 nodeRevs **并集**是单元素集合。
    // 只查白名单根自己那个 WC，会漏掉底下嵌套 external 那块「git 管着、SVN 侧
    // 没人盯」的盲区。
    let fx = svn_fixtures::svn_repo_initialized_with_nested_external_at_other_rev();
    let st = svn_sync::check_sync(&fx.wc).unwrap();
    assert!(st.node_revs.len() >= 2, "嵌套 external 的 revision 必须进并集：{:?}", st.node_revs);
}

#[test]
fn root_revs_is_cheap_and_covers_every_working_copy() {
    let fx = svn_fixtures::svn_repo_initialized_with_nested_external_at_other_rev();
    let got = svn_sync::read_root_revs(&fx.wc).unwrap();
    assert!(got.len() >= 2, "每个 wc(P) 各一条（spec §6.6.5 的 rootRev 行）");
    assert!(got.iter().all(|r| r.rev > 0));
}
```

- [ ] **Step 2: 跑测试确认失败**

```powershell
cd src-tauri; cargo test --test svn_sync
```

- [ ] **Step 3: 实现**

要点（逐条对应 spec，实现时不要简化）：

1. `baseline_rev` 从 `refs/heads/svn/base` 顶端 commit 的 message 走
   `svn_baseline::parse_trailer` 取。分支不存在 → 返回 `GitError`
   `svn_no_baseline`，**不要**当成 0。
2. `node_revs` = 对 `enumerate_wcs` 的**每个** WC 各跑一次
   `svn status -v --xml --ignore-externals -- <peg_safe(该 WC 内的白名单 target)>`，
   用 `parse_node_revs` 取值后求**并集**。
   - `-v` 和 `--ignore-externals` **都不能省**（spec §6.6.4 陷阱一/二）。
   - target 这里**要**过 `peg_safe`（`status` 属于「必须加」那一类）。
3. 判据只有两条（spec §6.6.2）：并集多值 → `MixedRevision`；单值 `{N}` 且
   `N != baseline_rev` → 先问是不是 `PinnedReset`，不是才 `Diverged`。
4. `PinnedReset` 的判据：`N` 恰好等于 `svn_externals::detect_pinned` 报出的某条
   external 的 `pinned_rev`。命中就带上那条 external 的 `local_path`。
5. **`rootRev` 不是判据**（spec §6.6.2 有实测：一次完全成功的交付之后
   `rootRev != baselineRev` 必然成立，拿它报警等于每次交付后都挂红横幅）。
   `read_root_revs` 只提供 O(1) 的「要不要花钱查」提示，走
   `svn info --show-item revision -- <peg_safe(wc_root)>`（实测 P7，裸数字）。
6. **上次观测值不存在后端。** 后端保持无状态，比对由前端 store 做——它本来就按
   仓库路径持有每个标签的状态，而后端要为此引一张按路径索引的进程内表，还要处理
   多标签同仓库的重入。这条写进模块注释。

- [ ] **Step 4: 跑测试确认通过**

```powershell
cd src-tauri; cargo test --test svn_sync
cargo fmt --check; cargo clippy --all-targets
```

- [ ] **Step 5: 变异测试——确认并集那条有牙**

把 `check_sync` 改成只查第一个 WC，重跑。
`node_revs_unions_every_working_copy_including_nested_externals` **必须变红**。
确认后改回来，记进决策日志。不提交。

- [ ] **Step 6: 提交**

```bash
git add src-tauri/src/engine/svn_sync.rs src-tauri/src/engine/mod.rs \
        src-tauri/tests/svn_sync.rs src-tauri/tests/common/svn_fixtures.rs
git commit -m "feat(svn): 失同步判定——nodeRevs 并集、两条判据与 pinned 成因分辨"
```

---

## Task 5：§6.6.5 欠账——基线体检按工作副本分组

**Files:**
- Modify: `src-tauri/src/engine/svn_health.rs`
- Test: `src-tauri/tests/svn_health.rs`（既有文件，追加 + 改一条）

**Interfaces:**
- Consumes: `svn_wc::enumerate_wcs`
- Produces: `check_baseline` 签名不变，行为从「对项目根跑一次」变成「按 WC 分组各跑一次」

**背景（1b 决策日志记账）：** 现在 `check_baseline` 只用配置里的 `svn_url` 对项目根跑
**一次** `svn diff`；白名单底下挂着 external 时，`nested_external_not_covered` 让整次
体检退化成 `NotChecked` 并点名那条 external。spec §6.6.5 第 4 行要求的是**每个 `wc(P)`
各体检一次，用它自己的 `url(P)`**。本 Task 把那条守卫从「放弃」升级成「分组」。

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn a_clean_nested_external_is_verified_not_merely_unchecked() {
    // 改动前：nested_external_not_covered 让它退化成 NotChecked。
    // 改动后：每个 WC 用自己的 url(P) 各比一次，干净就该是 Verified。
    let fx = svn_fixtures::svn_repo_initialized_with_clean_nested_external();
    let got = svn_health::check_baseline(&fx.wc, &fx.url, fx.rev, &fx.whitelist, &fx.extensions).unwrap();
    assert!(matches!(got, Health::Verified { .. }), "实际 {got:?}");
}

#[test]
fn a_lying_baseline_inside_a_nested_external_is_caught() {
    // 这条是上一条的反向证据：少了它，「见到 external 就返回 Verified」
    // 也能让上一条变绿，而那会把嵌套 external 变成一块无人盯防的盲区。
    let fx = svn_fixtures::svn_repo_initialized_with_tampered_nested_external();
    let got = svn_health::check_baseline(&fx.wc, &fx.url, fx.rev, &fx.whitelist, &fx.extensions).unwrap();
    match got {
        Health::Lying { files } => assert!(
            files.iter().any(|f| f.contains("Vendor")),
            "差异文件要落在那条 external 里：{files:?}"
        ),
        other => panic!("应当判成 Lying，实际 {other:?}"),
    }
}

#[test]
fn one_working_copy_failing_does_not_silently_verify_the_others() {
    // 某个 WC 的 diff 跑不成（URL 不可达）时，整体只能是 NotChecked。
    // 「其余的都干净所以 Verified」是这条检查最坏的失败形态 —— Verified 是
    // 一句主动的保证，§6.6 的失同步判定与 §9 的恢复流程都建立在它之上。
    let fx = svn_fixtures::svn_repo_initialized_with_unreachable_nested_external();
    let got = svn_health::check_baseline(&fx.wc, &fx.url, fx.rev, &fx.whitelist, &fx.extensions).unwrap();
    assert!(matches!(got, Health::NotChecked { .. }), "实际 {got:?}");
}
```

- [ ] **Step 2: 跑测试确认失败**

```powershell
cd src-tauri; cargo test --test svn_health
```

预期：前两条红（现在返回 `NotChecked`）。

- [ ] **Step 3: 实现**

结构改成：

```text
1. 既有的 url_mismatch 守卫保留（它查的是配置里的 svnUrl 对不对，与分组无关）
2. enumerate_wcs 拿到全部 WC
3. 把白名单根按「落在哪个 WC 内」分组：
     对每个白名单根 R，找到 wc_root 是 R 的前缀且**最长**的那个 WC
     （最长匹配是必须的——嵌套 external 的 wc_root 是外层 wc_root 的前缀的延长，
       取第一个匹配会把内层的路径算到外层头上，于是又用错了 url(P)）
4. 对每组跑一次：
     svn diff --old=<url(W)>@<rev> --new=<wc_root(W)> --summarize --xml -- <该组的根…>
     - 尾随 target **不过 peg_safe**（1b 实测 E155010）
     - 不带 --ignore-externals（svn diff 拒绝）
5. 任何一组 run_svn 失败 → 立刻整体返回 NotChecked，带上是哪个 WC
6. 各组的 Lying 文件列表合并、排序、去重
7. 全部组都无差异 → Verified { rev }
```

`nested_external_not_covered` 这道守卫**删掉**，并在删除处留注释说明它被什么取代了
（review 会按 1b 的记账来核对这一条）。

⚠️ 剥前缀那段现在要按**每组自己的** `url(W)` 剥，不能再用全局 `base`。`is_w_file`
的相对路径基准也要跟着变成「相对 repo_root」而不是「相对 base」——分组后 svn 打出来
的路径是 `<url(W)>/<相对 wc_root 的路径>`，而白名单与扩展名过滤用的是**仓库内相对
路径**。转换公式：`repo_rel = <wc_root(W) 相对 repo_root 的路径> + "/" + <剥掉 url(W) 前缀后的部分>`。
把这个公式写成一个带单测的私有函数，不要散在循环里。

- [ ] **Step 4: 跑测试确认通过**

```powershell
cd src-tauri; cargo test --test svn_health
cargo fmt --check; cargo clippy --all-targets
```

- [ ] **Step 5: 跑一次全量，确认没有回归**

```powershell
cd src-tauri; cargo test -j 2
```

`check_baseline` 被 `svn_init`（初始化 + 重建 + 手动体检）三处调用，改它一定要看全量。

- [ ] **Step 6: 提交**

```bash
git add src-tauri/src/engine/svn_health.rs src-tauri/tests/svn_health.rs src-tauri/tests/common/svn_fixtures.rs
git commit -m "fix(svn): 基线体检按工作副本分组，嵌套 external 不再退化成 NotChecked"
```

---

## Task 6：NB-2 欠账——服务器侧清单证伪 depth-excluded 漏收

**Files:**
- Create: `src-tauri/src/engine/svn_server_set.rs`
- Modify: `src-tauri/src/engine/mod.rs`
- Test: `src-tauri/tests/svn_server_set.rs`

**Interfaces:**
- Consumes: `svn_xml::parse_list`、`svn_cli::run_svn`、`svn_wc::enumerate_wcs`、`svn_include_check::expected_set`
- Produces:
  ```rust
  pub enum ServerAudit { Matched, Missing { files: Vec<String> }, NotChecked { reason: String } }
  pub fn audit_server_set(repo_root: &str, rev: u64) -> Result<ServerAudit>
  ```

**背景（1b 决策日志 NB-2，本期 D 组已完整复现）：**
`svn update --set-depth=exclude Code/Deep` 之后，磁盘上那棵子树消失，而
**P1 零条目、P2 单值、P3 干净、体检 `Verified`**——四道既有检查全部显示健康，
基线却静默少了服务器上确实存在的 `Code/Deep/D.cs`。唯一能发现它的入口是
`svn ls -R <url>@N`（实测 N3e 仍然列出它）。

- [ ] **Step 1: 写失败测试**

```rust
mod common;
use common::svn_fixtures;
use mygitclient_lib::engine::svn_server_set::{self, ServerAudit};

#[test]
fn a_depth_excluded_subtree_is_reported_as_missing() {
    // 本期 D 组实测的那个形态，逐字复现。
    let fx = svn_fixtures::svn_repo_initialized_then_depth_excluded();
    let got = svn_server_set::audit_server_set(&fx.wc, fx.rev).unwrap();
    match got {
        ServerAudit::Missing { files } => assert!(
            files.iter().any(|f| f.ends_with("Code/Deep/D.cs")),
            "要点名漏掉的是哪个文件：{files:?}"
        ),
        other => panic!("应当报漏收，实际 {other:?}"),
    }
}

#[test]
fn the_four_existing_checks_really_do_miss_it() {
    // 这条不是测新代码，是把「为什么需要这块新机制」钉成回归测试。
    // 少了它，将来有人「优化」掉这次网络调用时，不会有任何测试变红。
    let fx = svn_fixtures::svn_repo_initialized_then_depth_excluded();
    let st = mygitclient_lib::engine::svn_sync::check_sync(&fx.wc).unwrap();
    assert_eq!(st.verdict, mygitclient_lib::engine::svn_sync::SyncVerdict::InSync,
               "失同步判据看不见 depth-exclude —— 这正是 NB-2 的成因");
    let health = mygitclient_lib::engine::svn_health::check_baseline(
        &fx.wc, &fx.url, fx.rev, &fx.whitelist, &fx.extensions).unwrap();
    assert!(matches!(health, mygitclient_lib::engine::svn_health::Health::Verified { .. }),
            "体检也看不见 —— 实际 {health:?}");
}

#[test]
fn a_normal_repo_matches() {
    let fx = svn_fixtures::svn_repo_initialized();
    let got = svn_server_set::audit_server_set(&fx.wc, fx.rev).unwrap();
    assert!(matches!(got, ServerAudit::Matched), "实际 {got:?}");
}

#[test]
fn art_files_on_the_server_are_not_reported_as_missing() {
    // 服务器上有一堆美术资产，它们**本来就不该**进 git。
    // 不做扩展名过滤的话，每个仓库一打开就报几千条「漏收」。
    let fx = svn_fixtures::svn_repo_initialized();   // fixture 里有 Art/b.psd
    let got = svn_server_set::audit_server_set(&fx.wc, fx.rev).unwrap();
    match got {
        ServerAudit::Matched => {}
        ServerAudit::Missing { files } => panic!("美术资产被误报成漏收：{files:?}"),
        other => panic!("实际 {other:?}"),
    }
}

#[test]
fn an_unreachable_url_is_not_checked_rather_than_matched() {
    // 实测 P1：路径不存在时 exit 1，但 stdout 是一份**结构完整的空清单**。
    // 忽略退出码的写法会拿到空的服务器集合，差集恒为空 ⇒ 报告「没有漏收」，
    // 用修 NB-2 的代码重造一个 NB-2。
    let fx = svn_fixtures::svn_repo_initialized_with_bad_url();
    let got = svn_server_set::audit_server_set(&fx.wc, fx.rev).unwrap();
    assert!(matches!(got, ServerAudit::NotChecked { .. }), "实际 {got:?}");
}
```

- [ ] **Step 2: 跑测试确认失败**

```powershell
cd src-tauri; cargo test --test svn_server_set
```

- [ ] **Step 3: 实现**

```text
1. cfg = read_config；wcs = enumerate_wcs
2. 对每个 WC W：
     xml = run_svn(repo_root, ["ls", "-R", "--xml", "--", &format!("{}@{rev}", url(W))])
       ⚠️ 必须走 run_svn（非零退出即 Err），禁止 run_svn_code —— 见 Task 1 注释
       失败 → 立刻整体 NotChecked，带上是哪个 WC 和原文错误
     list = parse_list(&xml)
     对 kind == "file" 的每条 entry：
       server_rel = <wc_root(W) 相对 repo_root 的路径> + "/" + entry.name
       （实测 N4：<name> 相对 <list path> 重新编号，不是仓库绝对路径）
3. 服务器集合先过**与 git 侧同一套**的白名单 + 扩展名过滤
     —— 直接复用判定函数，不要另写一份。两份过滤迟早分叉，而分叉的表现是
        「每次打开都报几千条漏收」或者「永远报 Matched」，两头都很难查。
4. disk = svn_include_check::expected_set(repo_root, &cfg.whitelist, &cfg.extensions)
5. missing = server − disk
     只报这个方向。反方向（磁盘有、服务器没有）是「本地新增还没 svn add」，
     完全正常，报了就是噪音。
6. missing 为空 → Matched，否则 Missing { files }（排序，最多列 20 条 + 计数）
```

⚠️ 第 3 步那句「复用同一套过滤」是本 Task 最容易走偏的地方。`svn_include_check` 里
决定「一个仓库内相对路径算不算 W-file」的逻辑目前**内联在 `expected_set` 的磁盘遍历
里**。实现时把它抽成一个纯函数
`pub(crate) fn is_w_file(rel: &str, whitelist: &[String], extensions: &[String]) -> bool`，
两处共用。`svn_health.rs` 里已经有一个同名私有函数 —— **把那个也换成共用的这一个**，
让本仓库只剩一处「什么算 W-file」的定义。这算 Task 5 遗留的收尾，一并做掉。

- [ ] **Step 4: 跑测试确认通过**

```powershell
cd src-tauri; cargo test --test svn_server_set --test svn_health --test svn_exclude
cargo fmt --check; cargo clippy --all-targets
```

- [ ] **Step 5: 变异测试——确认「必须看退出码」那条有牙**

把 `run_svn` 临时换成 `run_svn_code` 并忽略 `code`，重跑。
`an_unreachable_url_is_not_checked_rather_than_matched` **必须变红**（会返回
`Matched`）。确认后改回来，记进决策日志——这是本期最危险的一个失败形态。

- [ ] **Step 6: 提交**

```bash
git add src-tauri/src/engine/svn_server_set.rs src-tauri/src/engine/svn_include_check.rs \
        src-tauri/src/engine/svn_health.rs src-tauri/src/engine/mod.rs \
        src-tauri/tests/svn_server_set.rs src-tauri/tests/common/svn_fixtures.rs
git commit -m "feat(svn): 服务器侧清单证伪 depth-excluded 漏收"
```

---

## Task 7：Tauri 命令层与 `Capabilities.vcs`

**Files:**
- Modify: `src-tauri/src/commands/svn.rs`、`src-tauri/src/lib.rs`、`src-tauri/src/vcs/mod.rs`、`src-tauri/src/vcs/git.rs`、`src-tauri/src/vcs/svn.rs`、`src-tauri/src/commands/repo.rs`
- Test: `src-tauri/tests/svn_backend.rs`（既有文件，追加）

**Interfaces:**
- Produces（JS 侧参数名是 camelCase，括号里标出）：
  ```rust
  #[tauri::command(async)] pub fn svn_sync_status(path: String) -> Result<SyncStatus, GitError>
  #[tauri::command(async)] pub fn svn_root_revs(path: String) -> Result<Vec<RootRev>, GitError>
  #[tauri::command(async)] pub fn svn_read_log(path: String) -> Result<Vec<CachedRev>, GitError>
  #[tauri::command(async)] pub fn svn_fetch_log(path: String, before_rev: Option<u64>, limit: u32)
      -> Result<Vec<CachedRev>, GitError>          // JS: { path, beforeRev, limit }
  #[tauri::command(async)] pub fn svn_server_audit(path: String) -> Result<ServerAudit, GitError>
  ```

- [ ] **Step 1: 写失败测试**

```rust
#[test]
fn capabilities_declare_which_vcs_backs_the_repo() {
    // 前端要据此决定「要不要发 SVN 那几个 invoke」。靠
    // 「所有 git 专属能力都是 false」反推是脆的 —— 将来某个 git 仓库
    // 恰好关掉全部可选能力时就会被当成 SVN。
    let git = svn_fixtures::plain_git_repo();
    assert_eq!(read_capabilities(&git.root).unwrap().vcs, "git");
    let svn = svn_fixtures::svn_repo_initialized();
    assert_eq!(read_capabilities(&svn.wc).unwrap().vcs, "svn");
}

#[test]
fn svn_read_log_never_touches_the_network() {
    // 打开仓库要先用缓存把图画出来（spec §10.2）。这条命令必须是纯本地读，
    // 断网时照样成功返回（可能是空列表）。
    let fx = svn_fixtures::svn_repo_initialized();
    // 缓存文件还不存在
    assert!(svn_read_log(fx.wc.clone()).unwrap().is_empty());
}

#[test]
fn svn_fetch_log_populates_the_cache_and_is_idempotent() {
    let fx = svn_fixtures::svn_repo_initialized();
    let first = svn_fetch_log(fx.wc.clone(), None, 200).unwrap();
    assert!(!first.is_empty());
    let second = svn_fetch_log(fx.wc.clone(), None, 200).unwrap();
    assert_eq!(first, second, "第二次拉取不应当产生任何差异");
    // 缓存已落盘，read 现在拿得到
    assert_eq!(svn_read_log(fx.wc.clone()).unwrap(), first);
}

#[test]
fn svn_fetch_log_includes_the_baseline_anchor_even_when_it_missed_the_whitelist() {
    // C 组：这是常态而非边界。锚点不在，工作分支在图上就挂不到 SVN 主干。
    let fx = svn_fixtures::svn_repo_initialized_at_orphan_baseline();
    let got = svn_fetch_log(fx.wc.clone(), None, 200).unwrap();
    assert!(got.iter().any(|e| e.rev == fx.baseline_rev),
            "baseline r{} 必须在序列里：{:?}", fx.baseline_rev,
            got.iter().map(|e| e.rev).collect::<Vec<_>>());
}
```

- [ ] **Step 2: 跑测试确认失败**

```powershell
cd src-tauri; cargo test --test svn_backend
```

- [ ] **Step 3: 实现**

`vcs/mod.rs` 的 `Capabilities` 加：

```rust
    /// 这个仓库背后是哪种版本控制。取值只有 `"git"` 与 `"svn"`。
    ///
    /// 用显式字段而不是让前端从「所有 git 专属能力都是 false」反推：那种反推
    /// 在今天成立纯属巧合，将来某个 git 仓库恰好关掉全部可选能力时就会被当成
    /// SVN，于是对一个普通 git 仓库发 SVN 的 invoke。
    pub vcs: &'static str,
```

`all()` 填 `"git"`；SVN 后端那份填 `"svn"`。

`svn_fetch_log` 的编排：

```text
1. cfg = read_config；uuid 从 enumerate_wcs 的第一个 WC 取
2. cache = read_cache(repo_root, &uuid)
3. targets = svn_log::log_targets(repo_root)
4. 对每个 target：
     head = head_revision
     若 before_rev 为 None（增量拉新）：
        from = covered_max + 1, to = head
     否则（往下翻页）：
        to = before_rev - 1, from = to.saturating_sub(limit)，且不低于 1
     entries = fetch_range(...)；merge_into(...)
     ⚠️ 单个 target 失败**不中断其余 target**，记下来最后一起说；
        全部 target 都失败才算整次失败
5. 锚点：baseline_rev（以及 svn/base 上历史 baseline commit 的 revision）
     凡不在 merged_sequence 里的，逐个 fetch_anchor 打给 repos_root，写进 cache.anchors
6. write_cache；返回 merged_sequence
```

第 5 步的「历史 baseline commit」范围：`git log refs/heads/svn/base --format=%H%x1f%B`
里能解析出 trailer 的那些，spec §10.3 说「几十个」，全部补齐。**加一条硬上限
（比如 200）**并在超出时只补最近的那些——没有上限的话，一个跑了三年的仓库
打开一次要发几百次网络调用。

- [ ] **Step 4: 跑测试确认通过**

```powershell
cd src-tauri; cargo test --test svn_backend
cargo fmt --check; cargo clippy --all-targets
```

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/commands/svn.rs src-tauri/src/lib.rs src-tauri/src/vcs/ \
        src-tauri/src/commands/repo.rs src-tauri/tests/svn_backend.rs
git commit -m "feat(svn): 本期五个 Tauri 命令与 Capabilities.vcs"
```

---

## Task 8：前端接合逻辑 `svnGraph.ts`

**Files:**
- Create: `src/engine/svnGraph.ts`、`src/engine/svnLog.ts`
- Modify: `src/engine/types.ts`
- Test: `tests/vitest/svnGraph.test.ts`

**Interfaces:**
- Consumes: `CommitNode`（已扩展）
- Produces:
  ```ts
  export interface SvnRevNode { rev: number; author: string; date: string; message: string }
  export const LOCAL_ONLY_TRAILER = 'Local-Only: true'
  export function isLocalOnly(message: string): boolean
  export function parseGitSvnId(message: string): { url: string; rev: number; uuid: string } | null
  export function spliceHistory(svnRevs: SvnRevNode[], gitCommits: CommitNode[]): CommitNode[]
  ```

`src/engine/types.ts` 的 `CommitNode` 加三个**可选**字段（spec §10.4，缺省视为 git，
现有 Git 模式零影响）：

```ts
export interface CommitNode {
  id: string
  parents: string[]
  author: string
  time: number
  message: string
  /** 新增；缺省视为 git。 */
  vcs?: 'git' | 'svn'
  /** 新增；仅 svn 节点有。 */
  svnRev?: number
  /** 新增；message 带 Local-Only trailer（spec §6.7），不会被交付。 */
  localOnly?: boolean
}
```

- [ ] **Step 1: 写失败测试**

```ts
import { describe, expect, it } from 'vitest'
import { isLocalOnly, parseGitSvnId, spliceHistory, type SvnRevNode } from '@/engine/svnGraph'
import type { CommitNode } from '@/engine/types'

const svn = (rev: number): SvnRevNode => ({
  rev, author: 'a', date: '2026-09-15T15:21:22.337313Z', message: `r${rev}`,
})

const git = (id: string, parents: string[], message = 'work'): CommitNode => ({
  id, parents, author: 'a', time: 1, message,
})

const baseline = (id: string, parents: string[], rev: number): CommitNode =>
  git(id, parents, `svn: r${rev}\n\ngit-svn-id: https://x/game@${rev} 8f3a1c7e-4b21-0410-9d2f-1a7b3c5d9e01`)

describe('parseGitSvnId', () => {
  it('三个字段缺一不可，缺了就不是 baseline', () => {
    expect(parseGitSvnId('svn: r1\n\ngit-svn-id: https://x/game@12 uuid-1')?.rev).toBe(12)
    expect(parseGitSvnId('svn: r1\n\ngit-svn-id: https://x/game uuid-1')).toBeNull()
    expect(parseGitSvnId('svn: r1\n\ngit-svn-id: @12 uuid-1')).toBeNull()
    expect(parseGitSvnId('svn: r1\n\ngit-svn-id: https://x/game@abc uuid-1')).toBeNull()
  })

  it('不匹配正文里顺手提到 git-svn-id 的行', () => {
    expect(parseGitSvnId('聊到了 git-svn-id: https://x@1 u 这个格式')).toBeNull()
  })
})

describe('isLocalOnly', () => {
  it('只认 Local-Only: true 这一条 trailer', () => {
    expect(isLocalOnly('local: mute log\n\nLocal-Only: true')).toBe(true)
    expect(isLocalOnly('local: mute log\n\nLocal-Only: false')).toBe(false)
    expect(isLocalOnly('提到了 Local-Only: true 但在正文里\n\n真正的结尾')).toBe(false)
    expect(isLocalOnly('plain commit')).toBe(false)
  })
})

describe('spliceHistory', () => {
  it('SVN 序列连成一条链，parents 指向下一个更小的 revision', () => {
    const out = spliceHistory([svn(3), svn(2), svn(1)], [])
    expect(out.map((c) => c.id)).toEqual(['svn:3', 'svn:2', 'svn:1'])
    expect(out[0].parents).toEqual(['svn:2'])
    expect(out[2].parents).toEqual([])
    expect(out[0].vcs).toBe('svn')
    expect(out[0].svnRev).toBe(3)
  })

  it('SVN 序列不连号时仍然接成链（revision 本来就稀疏）', () => {
    // 白名单路径的历史天然跳号：只动了别的目录的 revision 不会出现在这里。
    const out = spliceHistory([svn(9), svn(4), svn(1)], [])
    expect(out[0].parents).toEqual(['svn:4'])
    expect(out[1].parents).toEqual(['svn:1'])
  })

  it('baseline commit 被替换掉，指向它的 git commit 改指 svn 节点', () => {
    const out = spliceHistory(
      [svn(3), svn(2)],
      [git('aaa', ['bbb']), baseline('bbb', ['ccc'], 3)],
    )
    expect(out.find((c) => c.id === 'bbb')).toBeUndefined()
    expect(out.find((c) => c.id === 'aaa')!.parents).toEqual(['svn:3'])
  })

  it('孤儿 baseline —— 锚点没拉到时不留悬空父', () => {
    // 悬空父 id 不会画出线（computePaths 里 `if (!parentNode) continue`），
    // 但 computeOwners 仍然拿它算归属，会把车道分配算歪。所以必须剔除。
    const out = spliceHistory([svn(2)], [git('aaa', ['bbb']), baseline('bbb', [], 99)])
    const aaa = out.find((c) => c.id === 'aaa')!
    expect(aaa.parents).toEqual([])
    expect(out.every((c) => c.parents.every((p) => out.some((n) => n.id === p)))).toBe(true)
  })

  it('普通 git commit 之间的 parent 关系原样保留', () => {
    const out = spliceHistory([], [git('aaa', ['bbb']), git('bbb', [])])
    expect(out.find((c) => c.id === 'aaa')!.parents).toEqual(['bbb'])
  })

  it('localOnly 按 trailer 标出来', () => {
    const out = spliceHistory([], [git('aaa', [], 'local: x\n\nLocal-Only: true')])
    expect(out[0].localOnly).toBe(true)
  })

  it('纯 git 仓库（svnRevs 为空）时输出与输入等价', () => {
    // 现有 Git 模式零影响，spec §10.4。
    const input = [git('aaa', ['bbb']), git('bbb', [])]
    const out = spliceHistory([], input)
    expect(out.map((c) => ({ id: c.id, parents: c.parents })))
      .toEqual(input.map((c) => ({ id: c.id, parents: c.parents })))
  })
})
```

- [ ] **Step 2: 跑测试确认失败**

```powershell
pnpm test svnGraph
```

- [ ] **Step 3: 实现**

`spliceHistory` 按 spec §10.3 的三步，外加实测逼出来的第四步：

```text
① SVN 序列 → 节点：id = `svn:${rev}`，parents = [下一个更小 rev 的 id]（没有则 []）
② 扫 gitCommits，凡 parseGitSvnId 成功的即 baseline，建 <git id> → `svn:${rev}` 映射
③ 重写所有 git commit 的 parents：命中映射的替换成 svn 节点 id；
   baseline commit 自身从输出里**删掉**（它在图上的位置由 SVN 序列决定）
④ 收尾：把**任何**不在最终节点集合里的 parent id 剔除
```

第 ④ 步 spec 没写，是本期读代码读出来的：`computePaths` 遇到找不到的父会
`continue`（不画线），但 `computeOwners` 仍然用 parents 算归属，一个悬空 id 会静默
改变车道分配。孤儿 baseline 的锚点补拉失败（断网）时就会出现这种父，所以这不是
理论情况。

`parseGitSvnId` / `isLocalOnly` 都必须**逐行匹配且只认 trailer 区**：用
`message.split('\n')` 后对每行做 `^git-svn-id: (\S+)@(\d+) ([0-9a-fA-F-]+)$` /
`^Local-Only: true$` 的完整匹配，**不要** `message.includes(...)`——正文里提到这个
格式的 commit 会被误判，而误判 baseline 会让一个普通 commit 从图上消失。

- [ ] **Step 4: 跑测试确认通过**

```powershell
pnpm test svnGraph
pnpm build
```

- [ ] **Step 5: 提交**

```bash
git add src/engine/svnGraph.ts src/engine/svnLog.ts src/engine/types.ts tests/vitest/svnGraph.test.ts
git commit -m "feat(graph): 两条历史的接合与 Local-Only 判定"
```

---

## Task 9：前端展示逻辑 `svnSync.ts`

**Files:**
- Create: `src/engine/svnSync.ts`
- Test: `tests/vitest/svnSync.test.ts`

**Interfaces:**
- Consumes: 后端 `SyncStatus`（serde `camelCase`：`baselineRev` / `nodeRevs` / `verdict`；
  `verdict` 用 `#[serde(tag = "kind", rename_all = "camelCase")]`，与既有 `Health` 的
  `tag = "status"` 写法保持同一风格）
- Produces:
  ```ts
  export type BannerTone = 'ok' | 'warn' | 'danger'
  export interface BannerAction { id: 'rebuild' | 'check' | 'updateWhitelistDir'; label: string }
  export interface BannerModel { tone: BannerTone; title: string; detail: string; actions: BannerAction[] }
  export function describeSync(status: SyncStatus | null): BannerModel | null
  ```

- [ ] **Step 1: 写失败测试**

```ts
import { describe, expect, it } from 'vitest'
import { describeSync } from '@/engine/svnSync'

describe('describeSync', () => {
  it('同步时不挂横幅', () => {
    expect(describeSync({ baselineRev: 4, nodeRevs: [4], verdict: { kind: 'inSync' } })).toBeNull()
  })

  it('还没读到状态时也不挂横幅', () => {
    // 打开仓库的那一瞬间 status 是 null。此时挂一个「状态未知」的横幅
    // 等于每次打开都闪一下红。
    expect(describeSync(null)).toBeNull()
  })

  it('mixed revision 列出每一个值，出口是重建基线', () => {
    const m = describeSync({ baselineRev: 4, nodeRevs: [4, 7], verdict: { kind: 'mixedRevision', revs: [4, 7] } })!
    expect(m.tone).toBe('danger')
    expect(m.detail).toContain('4')
    expect(m.detail).toContain('7')
    expect(m.actions.map((a) => a.id)).toContain('rebuild')
  })

  it('pinned reset 给的是 svn update 那条出路，不是重建基线', () => {
    // spec §6.6.2：这两种情况的修复完全不同，给错等于把用户送进
    // 一个不必要的重型流程。
    const m = describeSync({
      baselineRev: 7, nodeRevs: [5],
      verdict: { kind: 'pinnedReset', nodeRev: 5, baselineRev: 7, external: 'Code' },
    })!
    expect(m.actions.map((a) => a.id)).toContain('updateWhitelistDir')
    expect(m.actions.map((a) => a.id)).not.toContain('rebuild')
    expect(m.detail).toContain('Code')
  })

  it('diverged 时说清是前进还是回退', () => {
    const fwd = describeSync({ baselineRev: 4, nodeRevs: [9], verdict: { kind: 'diverged', nodeRev: 9, baselineRev: 4 } })!
    const back = describeSync({ baselineRev: 9, nodeRevs: [4], verdict: { kind: 'diverged', nodeRev: 4, baselineRev: 9 } })!
    expect(fwd.detail).not.toEqual(back.detail)
  })
})
```

- [ ] **Step 2: 跑测试确认失败**

```powershell
pnpm test svnSync
```

- [ ] **Step 3: 实现**

文案要求（spec §6.6.2、§13）：

- **`mixedRevision`**：「工作副本处于多个 revision」，列出每个值，出口 = 重建基线。
- **`pinnedReset`**：点名那条 external，说明「有人在项目根跑了递归 `svn update`，
  把它拽回了 peg revision」，出口 = **`svn update <白名单所属目录>`**（给命令文本让
  用户自己去 TortoiseSVN 跑，本期不代劳——一条 SVN 写命令都不发）。
  **不要**给重建基线。
- **`diverged`**：区分 `nodeRev > baselineRev`（被外部推进）与 `<`（被回退），
  出口 = 重建基线 + 手动体检。
- 所有 danger 文案都要写明「重建基线**不会碰工作目录，也不会推进工作分支**」
  （spec §16 那段「代价是工作分支不会自动跟进新基线」）。

- [ ] **Step 4: 跑测试确认通过**

```powershell
pnpm test svnSync
pnpm build
```

- [ ] **Step 5: 提交**

```bash
git add src/engine/svnSync.ts tests/vitest/svnSync.test.ts
git commit -m "feat(svn): 失同步状态到横幅文案的展示逻辑"
```

---

## Task 10：`BaselineBanner` 与 store 接线（含 NB-6 的 UI 入口）

**Files:**
- Create: `src/components/BaselineBanner.tsx`
- Modify: `src/store/repoStore.ts`、`src/components/RepoTab.tsx`、`src/engine/capabilities.ts`、`src/styles.css`
- Test: `tests/vitest/capabilities.test.ts`（既有文件，改 + 追加）

**背景（1b 决策日志 NB-6）：** `svn_rebuild_baseline` 与 `svn_check_baseline` 两个命令
1b 就做好了，但**没有任何 UI 入口**——`check_current_baseline` 是 `Lying` 唯一真正可达的
入口，而用户点不出来。Phase 1 的验收语写着「此期只报警 + 提供重建基线」，没有入口
这一期就是不自洽的。本 Task 补上。

- [ ] **Step 1: 写失败测试（capabilities 的 vcs 字段）**

```ts
it('Capabilities 带 vcs 字段，SVN 侧是 svn', () => {
  expect(ALL_CAPABILITIES.vcs).toBe('git')
  expect(NO_GIT_ONLY_CAPABILITIES.vcs).toBe('svn')
})
```

同时把既有那条「加了新字段却忘了在 SVN 侧关掉」的测试改成**只遍历 boolean 字段**
——否则新加的 `vcs: 'svn'` 会被它当成「没关掉的能力」。改的时候在注释里写明为什么。

- [ ] **Step 2: 跑测试确认失败**

```powershell
pnpm test capabilities
```

- [ ] **Step 3: 实现 store 侧的两档检查**

`repoStore.ts`：

```text
新增 state：
  svnSync: SyncStatus | null
  svnAudit: ServerAudit | null
  lastRootRevs: string | null     ← 序列化成字符串好比对

打开仓库（openRepo）里，caps.vcs === 'svn' 时：
  await refreshSvnSync(path)                  ← 完整校验，spec §6.6.3 第二档
  void refreshSvnLog(path)                    ← 先读缓存画图，再后台拉新的
  void refreshSvnAudit(path)                  ← 走网络，失败静默

repo-changed 监听里，caps.vcs === 'svn' 时：
  const revs = await svnRootRevs(path)         ← O(1)，第一档
  if (serialize(revs) !== lastRootRevs) {
    setLastRootRevs(...)
    void refreshSvnSync(path)                  ← 变了才跑第二档
  }
```

⚠️ **第一档比的是「上次观测值」，不是 `baselineRev`**（spec §6.6.3 有实测理由：
交付之后根节点会长期停在旧 revision，拿它跟 `baselineRev` 比会每次刷新都判不等，
分层的意义直接归零）。进程重启后 `lastRootRevs` 是 `null`，第一次刷新当作「变了」
跑一次全量——这正是打开仓库本来就要做的事。

⚠️ `refreshSvnLog` / `refreshSvnAudit` 走网络，照 `refreshPullRequests` 的既有模式：
**不 await、失败只 `console.warn` 不上报**、并且**必须写回空值**（上一个仓库的状态
会挂在下一个完全无关的仓库上——`refreshPinned` 的注释里已经记过这个坑）。

- [ ] **Step 4: 实现 `BaselineBanner`**

- 挂在 `RepoTab` 顶部，`describeSync` 返回 `null` 时整个不渲染。
- 按 `BannerModel.actions` 渲染按钮：
  - `rebuild` → `svnRebuildBaseline`，**必须先过 `ConfirmDialog`**（复用既有组件），
    确认文案写明「不会碰工作目录、不会推进工作分支」。
  - `check` → `svnCheckBaseline`，结果用既有的对话框展示 `Verified` / `Lying` / `NotChecked`
    三态，`Lying` 时列出差异文件。
  - `updateWhitelistDir` → **不执行任何命令**，只展示要用户自己去跑的
    `svn update <目录>` 命令文本 + 复制按钮。
- `svnAudit` 是 `Missing` 时，在横幅下方**另起一条 warn 行**：
  「服务器上有 N 个白名单内的代码文件不在本地，git 没有在保护它们」+ 文件列表。
  这条与失同步是**两件独立的事**（一个是 revision 对不上，一个是文件根本没下来），
  不能合并成一条，也不能让其中一条的「正常」盖住另一条。

- [ ] **Step 5: 跑测试与构建**

```powershell
pnpm test
pnpm build
```

- [ ] **Step 6: 提交**

```bash
git add src/components/BaselineBanner.tsx src/components/RepoTab.tsx \
        src/store/repoStore.ts src/engine/capabilities.ts src/styles.css \
        tests/vitest/capabilities.test.ts
git commit -m "feat(svn): 失同步横幅，并补上重建基线与手动体检的 UI 入口"
```

---

## Task 11：混合图接线与分页

**Files:**
- Modify: `src/store/repoStore.ts`、`src/components/GraphView.tsx`、`src/components/CommitRow.tsx`、`src/components/CommitDetailView.tsx`、`src/styles.css`
- Test: 纯逻辑已在 Task 8 覆盖；本 Task 只做接线

- [ ] **Step 1: store 侧合并**

`refreshGraph` 之后，若 `caps.vcs === 'svn'`，把 `spliceHistory(svnRevs, data.commits)`
的结果写进 `commits`，而不是原始的 `data.commits`。**`svnRevs` 变化时也要重跑一次
接合**——后台拉取是异步的，先画出来的图里 svn 节点还没到位。

- [ ] **Step 2: `CommitRow` 显示 svn 节点**

- `node.commit.vcs === 'svn'` 时：hash 位置显示 `r${svnRev}` 而不是短 hash，整行换配色
  （在 `styles.css` 加 `.commit-row.svn`）。
- `node.commit.localOnly` 时加一个「仅本地」角标。
- **refs 标签逻辑不动**：svn 节点上本来就没有 ref。

- [ ] **Step 3: `CommitDetailView` 对 svn 节点的降级**

svn 节点点开时只显示 revision / 作者 / 时间 / message，**不拉 diff**，并明确写一句
「SVN 节点的文件差异属 Phase 4」。

**这是对 spec §11 的一处有依据收窄**：§11 那张表写了「svn 节点的 diff 走
`svn diff -c N`」，但 §16 的分期表把「svn 节点 diff」明确列在 **Phase 4**。两处冲突时
按分期表走（它是更晚、更具体的那处），并把这条冲突记进决策日志。

- [ ] **Step 4: 「加载更多」**

`GraphView` 底部加按钮，点击调 `svnFetchLog(path, oldestSvnRev, 200)`。
首屏 200 条，**不做无限滚动**（spec §10.2 明确第一期做简单版）。
已经拉到 r1 时按钮禁用。

- [ ] **Step 5: 跑测试与构建**

```powershell
pnpm test
pnpm build
```

- [ ] **Step 6: 提交**

```bash
git add src/store/repoStore.ts src/components/GraphView.tsx src/components/CommitRow.tsx \
        src/components/CommitDetailView.tsx src/styles.css
git commit -m "feat(graph): SVN 历史与本地 git 历史画在同一张图上"
```

---

## Task 12：本地补丁标记

**Files:**
- Modify: `src/components/LocalChangesView.tsx`、`src/styles.css`
- Test: `tests/vitest/svnGraph.test.ts`（`isLocalOnly` 已覆盖）；本 Task 补一条组装测试

**Interfaces:**
- Consumes: `svnGraph.LOCAL_ONLY_TRAILER`

- [ ] **Step 1: 把 message 组装抽成纯函数并写测试**

现在 `handleCommit` 里直接拼字符串。抽成 `src/engine/commitActions.ts` 的
（该文件已存在）：

```ts
/**
 * 把提交框的三个输入拼成最终的 commit message。
 *
 * `localOnly` 追加 spec §6.7 的 trailer。trailer 必须**单独成段**（前面空一行），
 * 否则 `git interpret-trailers` 不认，而本设计的判定与将来 Phase 3 的摘除都按
 * trailer 语义走。
 */
export function buildCommitMessage(subject: string, description: string, localOnly: boolean): string
```

```ts
it('Local-Only trailer 单独成段', () => {
  expect(buildCommitMessage('s', '', true)).toBe('s\n\nLocal-Only: true')
  expect(buildCommitMessage('s', 'body', true)).toBe('s\n\nbody\n\nLocal-Only: true')
  expect(buildCommitMessage('s', 'body', false)).toBe('s\n\nbody')
})

it('勾了但描述以空白结尾时不产生三个连续换行', () => {
  expect(buildCommitMessage('s', 'body\n\n', true)).toBe('s\n\nbody\n\nLocal-Only: true')
})
```

- [ ] **Step 2: 跑测试确认失败**

```powershell
pnpm test commitActions
```

- [ ] **Step 3: 实现勾选框**

`LocalChangesView` 的 `commit-actions` 里，`Amend` 旁加：

```tsx
<label className="changes-local-only" title="给这个提交打上 Local-Only 标记，标记本身不会改变任何现有流程">
  <input type="checkbox" checked={localOnly} onChange={(e) => setLocalOnly(e.target.checked)} />
  仅本地，不交付
</label>
```

**只在 `capabilities.vcs === 'svn'` 时渲染**——普通 git 仓库里这个勾选框没有任何含义。
`capabilities` 已经传进 `LocalChangesView` 的上层，按既有方式往下传。

- [ ] **Step 4: 写明窗口期没有保护（用户已确认要这条）**

勾选框旁边（或勾上之后展开）必须有一句话：

> 标记只是一条 commit message trailer。**交付流程要到 Phase 3 才会读它**——在那之前
> 你手工用 TortoiseSVN 交付时，这个提交里的改动会和别的改动一起被提交上去。

spec §16 明确要求这段窗口期在 UI 上写明，「不能让用户以为勾了『仅本地』就安全了」。
**不要**把它做成只在 hover 时出现的 tooltip——勾选框的 `title` 已经被占用，而这句话
是本期唯一能防止用户误解的地方。

- [ ] **Step 5: 跑测试与构建**

```powershell
pnpm test
pnpm build
```

- [ ] **Step 6: 提交**

```bash
git add src/components/LocalChangesView.tsx src/engine/commitActions.ts \
        src/styles.css tests/vitest/commitActions.test.ts
git commit -m "feat(svn): 提交框的「仅本地，不交付」标记与窗口期提示"
```

---

## Task 13：基线分支的 UI 保护

**Files:**
- Modify: `src/components/BranchView.tsx`、`src/components/LocalChangesView.tsx`、`src/engine/capabilities.ts`
- Test: `tests/vitest/capabilities.test.ts`

**Interfaces:**
- Produces: `export const BASELINE_BRANCH = 'svn/base'`、
  `export function isBaselineBranch(name: string): boolean`

- [ ] **Step 1: 写失败测试**

```ts
it('识别基线分支，容忍 refs/heads/ 前缀', () => {
  expect(isBaselineBranch('svn/base')).toBe(true)
  expect(isBaselineBranch('refs/heads/svn/base')).toBe(true)
  expect(isBaselineBranch('svn/base-backup')).toBe(false)
  expect(isBaselineBranch('feature/svn/base')).toBe(false)
})
```

- [ ] **Step 2: 跑测试确认失败**

```powershell
pnpm test capabilities
```

- [ ] **Step 3: 实现**

- `BranchView`：`svn/base` 加一个明显的标记（图标或标签），并在右键菜单里**去掉
  删除项**——spec §6.4 说真删了能从 reflog 恢复，但让它一键可删没有任何好处。
- `LocalChangesView`：HEAD 就在 `svn/base` 上时，Commit 按钮**禁用**，并显示
  「基线分支不能用于开发提交（spec §6.4）。请先切回工作分支。」
  判定用 `state.head`，它已经在 store 里。
- checkout 到 `svn/base` **不禁止**——spec §6.4 选可见分支的核心收益就是
  「我相对上次更新改了什么」这个 diff 要能看。禁止的只是**在其上提交**。

- [ ] **Step 4: 跑测试与构建**

```powershell
pnpm test
pnpm build
```

- [ ] **Step 5: 提交**

```bash
git add src/components/BranchView.tsx src/components/LocalChangesView.tsx \
        src/engine/capabilities.ts tests/vitest/capabilities.test.ts
git commit -m "feat(branch): 基线分支加标记并禁止在其上提交"
```

---

## Task 14：收尾验证

**Files:**
- Modify: `docs/superpowers/plans/2026-09-15-svn-phase1c-visible-decisions.md`
- Modify: `docs/superpowers/e2e/2026-09-15-svn-phase1c-e2e.md`（新建）

- [ ] **Step 1: 机械断言——本期零 SVN 写操作**

```powershell
Select-String -Path src-tauri/src/**/*.rs -Pattern '"(add|commit|update|revert|delete|propset|merge|switch|cleanup)"' `
  | Where-Object { $_.Line -match 'svn' }
```

逐条人工确认命中的都不是 svn 子命令实参（git 侧的 `"commit-tree"`、`"add"` 会命中，
那是 git 的）。把核对过程与结论写进决策日志，**不要只写一句「已确认」**。

- [ ] **Step 2: 机械断言——命令数对账**

```powershell
(Select-String -Path src-tauri/src/commands/*.rs -Pattern '#\[tauri::command').Count
(Select-String -Path src-tauri/src/lib.rs -Pattern 'commands::').Count
```

两个数必须相等（1b 的收尾做过同样的对账，当时是 50 == 50；本期 +5）。

- [ ] **Step 3: 全量测试**

```powershell
cd src-tauri; cargo fmt --check; cargo clippy --all-targets; cargo test -j 2
cd ..; pnpm test; pnpm build
```

把实际数字与 Task 0 的基线对账写进决策日志（新增了多少条、来自哪个 Task）。

- [ ] **Step 4: 真实窗口验证（`AGENTS.md` §5.1 硬要求）**

本期新增 5 个 Tauri 命令 + 改了 `Capabilities` 的形状，`snake_case` → `camelCase`
那层转换**没有编译期检查**，必须实际跑：

```powershell
pnpm tauri dev
```

在真实窗口里逐条验，结果写进 `docs/superpowers/e2e/2026-09-15-svn-phase1c-e2e.md`：

| # | 场景 | 预期 |
|---|---|---|
| 1 | 打开一个**普通 git 仓库** | 无横幅、无 svn 节点、提交框没有「仅本地」勾选框、控制台无 SVN 相关报错 |
| 2 | 打开一个已初始化的 SVN 混合仓库 | 先用缓存画出图，随后 svn 节点出现；无横幅 |
| 3 | 外部 TortoiseSVN 提交一个深层文件后回到窗口 | 横幅出现，判成 mixed revision 并列出每个 revision |
| 4 | 点「重建基线」 | 先弹确认；成功后横幅消失，**工作目录与当前分支未变** |
| 5 | 点「手动体检」 | 三态都能正确显示；`Lying` 时列出差异文件 |
| 6 | `svn update --set-depth=exclude` 一个白名单子目录后刷新 | 失同步横幅**不出现**（判据看不见它），但漏收提示出现并点名该文件 |
| 7 | 断网后打开仓库 | 图仍能从缓存画出；角落有低调的「SVN 历史可能不是最新」标记；**不弹错误横幅** |
| 8 | 点「加载更多」 | 往下多出 200 条；到 r1 后按钮禁用 |
| 9 | 勾上「仅本地，不交付」提交 | 图上出现「仅本地」角标；`git log -1 --format=%B` 能看到 trailer 单独成段 |
| 10 | checkout 到 `svn/base` | 能切过去看 diff，但 Commit 按钮禁用并给出说明 |

⚠️ **第 1 条不能省。** 本期改了 `Capabilities` 的形状，最容易出的回归是普通 git 仓库
被误判成 SVN，然后每次刷新都发几个必然失败的 invoke。

⚠️ 场景 3 / 6 / 9 涉及 SVN 写操作，**必须在临时 fixture 仓库里做**
（`AGENTS.md` 开篇最高优先级约束），不得拿本仓库或用户的任何真实仓库试。

- [ ] **Step 5: 补决策日志的四类条目**

`AGENTS.md` §2.1：偏离计划 / 计划的错误 / 计划没写的坑 / 为什么这么改是安全的，
缺一类视为记录不完整。本期至少要写清楚：

- **偏离计划**：Task 11 Step 3 对 spec §11 的收窄（svn 节点 diff 归 Phase 4）。
- **为什么这么改是安全的**：Task 6 的服务器侧核对是**只读且不阻断**的——它只加一条
  提示，不参与任何既有门槛的判定，所以即使误报也不会挡住用户；而漏报的代价（NB-2
  原样保留）与改动前相同。这一条必须论证，因为它是本期唯一一处「新增的检查」。

- [ ] **Step 6: 请一次独立 review**

`AGENTS.md` §4.1 独立 review 门槛：**实现者的自查不满足**。用 `/code-review`、
独立 review subagent 或 Codex 二次意见，按 §13 rubric 执行。
**先读决策日志的「偏离计划」条目再看 diff**（rubric #12）。
发现的高危问题合并前必须修完。

- [ ] **Step 7: 开 Draft PR**

```powershell
gh pr create --draft --base master --title "feat(svn): Phase 1c 看得见那半——失同步横幅、SVN log、混合图与本地补丁标记"
```

PR body 按 `AGENTS.md` §2 的 PR 模板写。**Test plan 里没实际跑的项必须留空并写原因**
——1a 的两个复选框当时空着是对的（没谎报），不要为了好看勾上。

---

## Self-Review

**1. Spec 覆盖核对**

| spec 章节 | 覆盖它的 Task |
|---|---|
| §4.4 `SvnClient`（`log` / `ls` 的参数约定） | Global Constraints + Task 2 |
| §6.4 基线分支 UI 保护 | Task 13 |
| §6.6.2 两条判据 + pinned 成因 | Task 4、Task 9 |
| §6.6.3 检查频次分层 | Task 10 Step 3 |
| §6.6.4 三个读法陷阱 | Task 1（DOM）、Task 4（`-v` + `--ignore-externals`） |
| §6.6.5 每个 `wc(P)` 各体检一次 | Task 5 |
| §6.3.1 服务器侧证伪（NB-2） | Task 6 |
| §6.7 本地补丁标记 | Task 8（判定）、Task 12（勾选框 + 窗口期提示） |
| §9.3 `svn revert` 抓不到，靠 git 工作区状态 | **无新代码**——现有 watcher + `git status` 已覆盖；Task 9 的文案里写明它不归失同步检测管 |
| §10.1/§10.2 网络、缓存、分页 | Task 2、Task 3、Task 7、Task 11 |
| §10.3 两条历史的接合 + 孤儿 baseline | Task 8 + Task 2 `fetch_anchor` |
| §10.4 `CommitNode` 数据模型 | Task 8 |
| §11 前端改动表 | Task 10–13（`SvnExternalView` / `SyncDialog` 属 Phase 2/4，本期不做） |
| §16 Phase 1 验收语 | Task 10（重建基线入口）+ Task 14 场景 4 |

**未覆盖且有意留下的**：`svn 节点 diff`（spec §16 归 Phase 4）、`SvnExternalView`
（Phase 4）、`SyncDialog`（Phase 2）、§9.4 完整恢复向导（Phase 2）。

**2. 类型一致性核对**

- Rust `SvnLogEntry.revision: u64` → 缓存 `CachedRev.rev: u64` → 前端 `SvnRevNode.rev: number`。
  字段名在 Rust 侧从 `revision` 变成 `rev` 是**有意的**（缓存 JSON 要短），转换点唯一，
  在 `merge_into` 里。
- `SyncVerdict` 的 serde tag 是 `kind`，与既有 `Health` 的 `status` 不同——两者都写进了
  各自的定义处，前端 `svnSync.ts` 按 `kind` 取。
- `svn_fetch_log` 的 Rust 参数 `before_rev` → JS `beforeRev`（Task 7 的 Interfaces 已标注）。
- `Capabilities.vcs` 在 Rust 是 `&'static str`，TS 是 `'git' | 'svn'`。

**3. 占位符扫描**：无 TBD / TODO / 「类似 Task N」/ 「补上适当的错误处理」。
Task 4、5、10、11 的实现步骤用的是逐条要求而不是完整代码块——它们是编排与接线，
每条要求都指向一个具体文件与具体判据，且失败测试已经把行为钉死。

---

## 已知风险与留给下一期的

| 风险 | 说明 |
|---|---|
| `svn ls -R` 在大仓库上的代价未实测 | fixture 只有几个文件。真实 Unity 工程的白名单子树可能上万文件，一次全量列举的耗时没有数据。Task 10 让它走「不 await、失败静默」，最坏是提示晚出现；但若实测发现是十秒级，下一期要考虑降频或只在打开仓库时跑一次 |
| 缓存没有上限 | `.git/mygitclient-svnlog.json` 随历史单调增长。一个十年的仓库拉满会是多大没有测过 |
| `git-svn-id` 正则在 Rust 与 TS 各有一份 | 格式由 spec §6.4.1 钉死且两侧都有测试，但仍是两个事实源 |
| revprop 被改时缓存不会失效 | spec §10.2 明确接受（罕见到可忽略） |
| `svn revert` 仍然只能靠 git 工作区状态发现 | spec §9.3 的既定结论，本期不新增机制，只在文案里写明 |

# SVN Phase 1b：建库与基线录制 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 能把一个干净的 SVN 工作副本变成混合仓库、录下**可被服务器证伪**的首个 `svn/base` 基线，并在基线因外部 TortoiseSVN 操作而失同步后重建它。

**Architecture:** 在 Phase 1a 的只读地基上加**写**能力，但写的全部是 **git 侧**——`git init`、临时索引、`commit-tree`、`update-ref`。SVN 侧仍然只读（`info` / `status` / `propget` / `diff`），本期**依旧一条 SVN 写命令都不发**。基线录制走「临时索引 → `write-tree` → `commit-tree` → `update-ref`」，全程不碰真实 index、不碰 HEAD、不碰工作目录文件。

**Tech Stack:** Rust（`std::process::Command` 经 `engine::git_cli` / `engine::svn_cli` 封装）、roxmltree（已在树里）、React 18 + TypeScript。

**Spec:** `docs/superpowers/specs/2026-09-12-svn-git-hybrid-design.md`（本计划基于 master 的 `310c17d` 版本）

**上一期:** `docs/superpowers/plans/2026-09-13-svn-phase1a-foundation.md` 及其决策日志。Phase 1a 已合并（`ca304d6`）。

---

## Global Constraints

以下逐字抄自 spec 与 `AGENTS.md`，**每个 Task 的要求都隐含包含本节**。

- **本期不发出任何 SVN 写操作。** `svn add` / `commit` / `update` / `revert` / `delete` / `propset` / `merge` 一个都不得进入 `src-tauri/src/`。允许的 svn 子命令只有 `info`、`status`、`propget`、`diff`、`--version`。收尾任务用 grep 做机械断言。
- **`--non-interactive` 是全局选项**，由 `svn_cli::with_global` 统一注入，不要在调用点重复写。
- **`svn info` 与 `svn diff` 拒绝 `--ignore-externals`**（spec §4.4，1a 与本期两次实测确认，exit 1）。`status` / `update` 接受。
- **所有 svn 路径 target 必须过 `svn_wc::peg_safe`**（追加空 peg 后缀 `@`）。SVN 把 target 里最后一个 `@` 之后的部分当 peg revision，`--` 挡不住。
- **`baselineBranch` 恒为 `svn/base`**，配置里那个字段只接受这一个值（spec §6.2.1）。
- **`git-svn-id` trailer 格式固定一行**：`git-svn-id: <url>@<rev> <uuid>`，解析正则 `^git-svn-id: (\S+)@(\d+) ([0-9a-fA-F-]+)$`。三字段缺一不可，解析失败即视为该 commit 不是 baseline——**不猜、不部分接受**（spec §6.4.1）。
- **`<url>` 取 `Repository Root`**，不是工作副本 URL（spec §6.4.1）。
- **`git-svn-id` 的 `<rev>` 取自 `nodeRevs` 的单值 N**，不是 `svn info .` 的 rootRev。取错会让初始化**完成的那一瞬间**就判失同步（spec §6.5.1 实测：rootRev=1 而 nodeRevs={3}）。
- **MSRV**：`Cargo.toml` 声明 `rust-version = "1.77.2"`。`#[allow(lint, reason = "...")]` 需要 1.81，**禁止使用**；用裸 `#[allow(...)]` + 中文注释。
- **所有涉及 git / svn 写操作的测试必须自建临时 fixture 仓库**（`AGENTS.md` 开篇最高优先级约束 + §8），参照 `src-tauri/tests/fixtures.rs` 与 `src-tauri/tests/common/svn_fixtures.rs`。禁止依赖本仓库或任何固定路径。
- **禁止新增依赖**（npm 包或 crate）而不先征得用户确认（`AGENTS.md` §7.1）。本计划不需要新依赖。
- **注释用中文，解释「为什么」而不是「做了什么」**（`AGENTS.md` §7.3）。
- **本仓库没有 PR 门禁 CI，也没有 lint 工具。** 禁止宣称「CI 通过」或「lint 通过」（`AGENTS.md` §4.1、§5.1）。

---

## 开工前已完成的实测（2026-09-14，临时 fixture，跑完即删）

写计划之前先把承重机制实测掉，**下面这些是事实，不是设计意图，实现时不要再猜**。
Phase 1a 的决策日志里「计划的错误」一栏全是没实测就写进计划的命令和断言。

| # | 实测项 | 结果 |
|---|---|---|
| E1 | `GIT_INDEX_FILE=<tmp>` + `git add -A -- Code` + `git write-tree` | 成功。**真实索引逐字节未变、HEAD 仍未出生、工作目录文件都在**（`Code/B.psd` 未被删） |
| E2 | 空临时索引起步 + `add -A -- <W-path>` 的产物 | 恰好是 W-file（`Code/A.cs`、`Code/Sentinel.cs` 进，`Code/B.psd` 被 exclude 挡住） |
| E3 | `git commit-tree TREE`（无父）+ `git update-ref refs/heads/svn/base` | 成功；`git log -1 --format=%B` 能原样读回 trailer；`git log --grep='^git-svn-id'` 能命中 |
| E4 | **`svn diff --summarize` 接受 `--xml`** | 接受。输出 `<path item="modified" props="none" kind="file">URL</path>`，`item` / `props` / `kind` 是**三个独立属性** |
| E5 | `svn diff --ignore-externals` | **exit 1，拒绝**。印证 spec §4.4 与 §6.8.2 的 R3 |
| E6 | `svn status --xml` 的 `path` 属性 | **反斜杠**（`Code\A.cs`） |
| E7 | `svn propget svn:externals -R --xml` 的 `<target path>` 属性 | **绝对路径 + 正斜杠**（`C:/Users/.../wc/Code`）。**与 E6 形状不同，同一个 CLI 两种写法** |
| E8 | 五种钉死写法实际效果 | `^/lib follow` → r2（HEAD）；`-r1` / `-r 1` / `^/lib@1` / 旧格式 `pinD -r1 ^/lib` → **全部 r1（钉住）** |
| E9 | `svn status --xml` 的 `item` 取值 | `modified` / `added` / `unversioned`（以及干净节点的 `normal`） |
| E10 | `svn propset svn:externals -F <file>` 遇到混合行尾 | `E135000: Inconsistent line ending style`，**且不影响 PowerShell 的退出码链**，会静默失败。fixture 构造必须写一致行尾 |

**E4 带来一处对 spec 的有依据偏离。** spec §6.8.4 用一整节讲文本版 `--summarize` 的两个
解析坑（路径是 URL；输出里混进目录条目和纯属性变更的条目，靠**列位置**区分）。`--xml`
把第二个坑从结构上消掉了——`kind` 与 `props` 是独立属性，不必再靠数空格。第一个坑
（路径是 URL）两种形式都有，仍要剥前缀。**本计划一律用 `--xml`。**

---

## File Structure

新增：

| 文件 | 职责 |
|---|---|
| `src-tauri/src/engine/svn_preflight.rs` | P1 / P2 / P3 三道硬门槛（P4 已在 1a 的 `svn_wc`） |
| `src-tauri/src/engine/svn_externals.rs` | P5：解析 `svn:externals` 判定有没有被 `-r` 钉死 |
| `src-tauri/src/engine/svn_baseline.rs` | 临时索引录 tree、`git-svn-id` trailer 的生成与解析、`commit-tree` + `update-ref` |
| `src-tauri/src/engine/svn_health.rs` | §6.8 基线体检 |
| `src-tauri/src/engine/svn_init.rs` | 初始化向导编排 + 失败回退；重建基线 R1–R5 |
| `src-tauri/src/commands/svn.rs` | 本期的 `#[tauri::command]` 入口 |
| `src/engine/svnInit.ts` | 向导的纯逻辑（步骤状态机、错误分类），无 React 依赖 |
| `src/components/SvnInitWizard.tsx` | 向导 UI |
| `src-tauri/tests/svn_preflight.rs` / `svn_externals.rs` / `svn_baseline.rs` / `svn_health.rs` / `svn_init.rs` | 各自的集成测试 |
| `tests/vitest/svnInit.test.ts` | 向导纯逻辑单测 |

修改：

| 文件 | 改动 |
|---|---|
| `src-tauri/src/engine/git_cli.rs` | 加**带环境变量**的执行入口（`GIT_INDEX_FILE` 只能靠 env 传，而现有三个 `run_git*` 都不接受 env） |
| `src-tauri/src/engine/svn_xml.rs` | 加 `parse_status_entries`、`parse_externals_props`、`parse_diff_summary` |
| `src-tauri/src/engine/svn_include_check.rs` | `actual_set` 改走临时索引（**删掉纯读模拟**，见 Task 5） |
| `src-tauri/src/engine/mod.rs`、`src-tauri/src/lib.rs` | 模块声明与命令注册 |

**为什么 `svn_init.rs` 同时装初始化和重建基线：** 两者共用同一段录制序列（临时索引 →
`write-tree` → `commit-tree` → 体检 → `update-ref`），差别只在前置校验与父提交。
拆成两个文件会让那段序列被抄两遍，而它正是本期最不能写错的地方。

---

## Task 0：开工自检与基线数字

**Files:**
- Create: `docs/superpowers/plans/2026-09-14-svn-phase1b-baseline-decisions.md`

- [ ] **Step 1: 记录 `AGENTS.md` §4.2 的开工自检**

worktree 已在动第一行代码之前建好并切入，基点 `ca304d6`。把实测值抄进决策日志。

- [ ] **Step 2: 记录改动前的测试基线数字**

```powershell
cd src-tauri; cargo test -j 2
cd ..; pnpm test
```

预期（Phase 1a 合并后的 master）：Rust **316 passed / 28 个二进制**，前端 **272 passed / 21 files**。
**实测值与这里不符时以实测为准**，并在决策日志里记下差异——不要改这份计划去迁就。

- [ ] **Step 3: 建决策日志骨架**

`AGENTS.md` §2.1 MUST：**在动第一个 Task 之前建好**。含进度总账表（Task 0–10）与四类决策条目的空节。

---

## Task 1：`git_cli` 支持带环境变量执行

**Files:**
- Modify: `src-tauri/src/engine/git_cli.rs`
- Test: `src-tauri/tests/git_cli.rs`

**Interfaces:**
- Consumes: 现有 `git_command(path) -> Command`
- Produces: `pub fn run_git_env(path: &str, args: &[&str], envs: &[(&str, &str)]) -> Result<String>`

**为什么这是独立的第一个任务：** R2 的临时索引只能靠 `GIT_INDEX_FILE` 环境变量指定
（`git add` 没有 `--index-output`，只有 `git read-tree` 有）。而现有的 `run_git` /
`run_git_bytes` / `run_git_code` **三个都不接受 env**。`AGENTS.md` §7.3 MUST：
「所有 git CLI 调用统一经过现有的 `run_git` 封装，不要新写一套进程调用」——所以必须
扩展这里，不能在 `svn_baseline.rs` 里自己 `Command::new("git")`。

- [ ] **Step 1: 写失败测试**

```rust
// src-tauri/tests/git_cli.rs 末尾追加
#[test]
fn run_git_env_passes_the_variable_through_to_the_child() {
    // GIT_INDEX_FILE 是本期唯一的用途，直接拿它做断言：指向一个不存在的
    // 临时路径时，git 会在那里**新建**一个空索引，于是 ls-files 为空，
    // 而真实索引里那个文件原样还在。
    let fx = repo_with_commit();
    let root = fx.path_str();
    std::fs::write(fx.path().join("extra.txt"), "x").unwrap();
    run_git(&root, &["add", "--", "extra.txt"]).unwrap();
    let real_before = run_git(&root, &["ls-files"]).unwrap();
    assert!(real_before.contains("extra.txt"), "前提不成立：{real_before}");

    let tmp = fx.path().join("tmp.index");
    let out = run_git_env(
        &root,
        &["ls-files"],
        &[("GIT_INDEX_FILE", tmp.to_str().unwrap())],
    )
    .unwrap();
    assert_eq!(out.trim(), "", "临时索引应当是空的，实际：{out}");

    let real_after = run_git(&root, &["ls-files"]).unwrap();
    assert_eq!(real_before, real_after, "真实索引被动过了");
}

#[test]
fn run_git_env_still_maps_failure_to_giterror() {
    // 带 env 的入口不能因为多了一层就把失败吞掉
    let fx = repo_with_commit();
    let err = run_git_env(&fx.path_str(), &["cat-file", "-p", "deadbeef"], &[])
        .expect_err("不存在的对象必须报错");
    assert_eq!(err.code, "git_cli");
}
```

> ⚠️ `repo_with_commit()` / `path_str()` 是 `src-tauri/tests/fixtures.rs` 里既有的
> helper，实现前先读那个文件确认真实签名，**不要照抄这里的名字**。

- [ ] **Step 2: 跑测试确认失败**

```powershell
cd src-tauri; cargo test --test git_cli run_git_env
```

预期：编译失败，`cannot find function run_git_env`。

- [ ] **Step 3: 实现**

```rust
/// 同 [`run_git`]，但额外注入一组环境变量。
///
/// # 为什么需要它
///
/// `GIT_INDEX_FILE` 是把 `git add` 引到**临时索引**上的唯一手段——`git add` 没有
/// `--index-output`（只有 `git read-tree` 有），而基线录制的全部安全性都建立在
/// 「不碰用户的真实暂存区」上（spec §16 R2）。
///
/// 走这里而不是在调用点自己 `Command::new("git")`，是因为那样会绕开
/// [`git_command`] 里的 `GIT_TERMINAL_PROMPT=0` 与 `hidden_command`——前者防子进程
/// 等 stdin 挂死，后者防装机版闪 console 黑框（`AGENTS.md` §7.3）。
pub fn run_git_env(path: &str, args: &[&str], envs: &[(&str, &str)]) -> Result<String> {
    let mut cmd = git_command(path);
    cmd.args(args);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let output = cmd.output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(GitError::new("git_cli", stderr));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
```

- [ ] **Step 4: 跑测试确认通过**

```powershell
cd src-tauri; cargo test --test git_cli
cargo fmt --check; cargo clippy --all-targets
```

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/engine/git_cli.rs src-tauri/tests/git_cli.rs
git commit -m "feat(engine): run_git 支持注入环境变量"
```

---

## Task 2：`svn_xml` 三个新解析器

**Files:**
- Modify: `src-tauri/src/engine/svn_xml.rs`
- Test: `src-tauri/tests/svn_xml.rs`

**Interfaces:**
- Produces:
  ```rust
  /// 一个非干净节点。`path` 已归一成**仓库相对 + 正斜杠**。
  pub struct StatusEntry { pub path: String, pub item: String }
  pub fn parse_status_entries(xml: &str) -> Result<Vec<StatusEntry>>

  /// 一处 svn:externals 属性。`target` 是**属性所在目录**的绝对路径（正斜杠）。
  pub struct ExternalsProp { pub target: String, pub raw: String }
  pub fn parse_externals_props(xml: &str) -> Result<Vec<ExternalsProp>>

  /// `svn diff --summarize --xml` 的一条。`path` 仍是 URL，剥前缀是调用方的事。
  pub struct DiffEntry { pub path: String, pub item: String, pub props: String, pub kind: String }
  pub fn parse_diff_summary(xml: &str) -> Result<Vec<DiffEntry>>
  ```

**为什么三个放一个 Task：** 它们都是「喂一段 XML 字符串、吐一个结构体列表」的纯函数，
在同一个文件里，测试形态相同，reviewer 不会approve 其中两个而 reject 第三个。
真正会被单独否决的是**用它们的那三个模块**，各自是后面独立的 Task。

**⚠️ 三个解析器的路径形状互不相同**（开工前实测 E6 / E7 / E4）：

| 解析器 | 源 XML 里的路径形状 | 本函数是否归一 |
|---|---|---|
| `parse_status_entries` | 反斜杠，相对 target（`Code\A.cs`） | **归一**成正斜杠 |
| `parse_externals_props` | 绝对路径 + 正斜杠（`C:/…/wc/Code`） | **不归一**，原样给出（调用方要拿它和仓库根求相对） |
| `parse_diff_summary` | URL（`file:///…/repo/Code/A.cs`） | **不归一**，剥 URL 前缀是 §6.8.4 调用方的事 |

**故意不在解析层统一**：1a 的 `parse_external_roots` 已经立过这个规矩——解析层只负责
「XML 说了什么」，归一属于知道上下文的调用方。`parse_status_entries` 是唯一例外，
因为「相对 target 的反斜杠」没有任何调用方会想要原样保留。这条差异要写进各自的
doc 注释，否则下一个人会以为是漏了。

- [ ] **Step 1: 写失败测试（status）**

```rust
#[test]
fn status_entries_come_back_with_forward_slashes() {
    // 2026-09-14 实测：svn status --xml 的 path 用**反斜杠**，而白名单、
    // git ls-files、期望集合全是正斜杠。不归一的话 P1 的比对永远对不上。
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<status>
<target path="Code">
<entry path="Code\A.cs"><wc-status item="modified" props="none"></wc-status></entry>
<entry path="Code\New.cs"><wc-status item="unversioned" props="none"></wc-status></entry>
<entry path="Code\Added.cs"><wc-status item="added" props="none"></wc-status></entry>
</target>
</status>"#;
    let got = parse_status_entries(xml).unwrap();
    let paths: Vec<&str> = got.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(paths, vec!["Code/A.cs", "Code/Added.cs", "Code/New.cs"]);
    assert_eq!(got[0].item, "modified");
    assert_eq!(got[2].item, "unversioned");
}

#[test]
fn a_clean_status_yields_no_entries() {
    // 干净工作副本上不带 -v 的 status 只有 <target>，没有 <entry>。
    // 这里**必须返回空列表而不是报错**——和 parse_node_revs 的空集守卫相反，
    // 那边空集是「输入本身就不对」，这边空集是「工作副本是干净的」，是好消息。
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<status>
<target path="Code"></target>
</status>"#;
    assert!(parse_status_entries(xml).unwrap().is_empty());
}
```

- [ ] **Step 2: 写失败测试（externals，五种钉死写法）**

```rust
#[test]
fn externals_props_keep_the_raw_text_verbatim() {
    // 判定钉没钉住是 svn_externals 的事，这里只负责原样取出来——
    // 五行写法的字段顺序不同（新格式 URL 在前、旧格式本地路径在前），
    // 解析层一旦试图分列就会分错。
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<properties>
<target path="C:/tmp/wc/Code">
<property name="svn:externals">^/lib follow
-r1 ^/lib pinA
-r 1 ^/lib pinB
^/lib@1 pinC
pinD -r1 ^/lib
</property>
</target>
</properties>"#;
    let got = parse_externals_props(xml).unwrap();
    assert_eq!(got.len(), 1);
    // 绝对路径 + 正斜杠，原样保留（与 status 的反斜杠形状不同，实测 E7）
    assert_eq!(got[0].target, "C:/tmp/wc/Code");
    assert_eq!(got[0].raw.lines().count(), 5);
    assert!(got[0].raw.contains("pinD -r1 ^/lib"));
}

#[test]
fn no_externals_anywhere_is_an_empty_list_not_an_error() {
    let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<properties>\n</properties>";
    assert!(parse_externals_props(xml).unwrap().is_empty());
}
```

- [ ] **Step 3: 写失败测试（diff --summarize --xml）**

```rust
#[test]
fn diff_summary_separates_content_from_property_changes() {
    // spec §6.8.4 讲的是文本版 --summarize 靠**列位置**区分内容变更与属性变更，
    // 且会混进目录条目。2026-09-14 实测 --xml 被接受，且 item / props / kind
    // 是三个独立属性——本计划一律用 --xml，那个解析坑从结构上消失。
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<diff>
<paths>
<path item="modified" props="none" kind="file">file:///srv/repo/Code/A.cs</path>
<path item="none" props="modified" kind="dir">file:///srv/repo/Code</path>
<path item="deleted" props="none" kind="file">file:///srv/repo/Code/Gone.cs</path>
</paths>
</diff>"#;
    let got = parse_diff_summary(xml).unwrap();
    assert_eq!(got.len(), 3);
    assert_eq!(got[0].item, "modified");
    assert_eq!(got[0].kind, "file");
    assert_eq!(got[1].item, "none");
    assert_eq!(got[1].props, "modified");
    assert_eq!(got[1].kind, "dir");
    // path 仍是 URL，剥前缀是调用方的事
    assert!(got[0].path.starts_with("file:///"));
}

#[test]
fn an_empty_diff_is_an_empty_list() {
    let xml = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<diff>\n<paths>\n</paths>\n</diff>";
    assert!(parse_diff_summary(xml).unwrap().is_empty());
}
```

- [ ] **Step 4: 跑测试确认失败**

```powershell
cd src-tauri; cargo test --test svn_xml
```

预期：三个函数都 `cannot find function`。

- [ ] **Step 5: 实现三个解析器**

沿用 `svn_xml.rs` 里既有的 roxmltree 用法（看 `parse_info` / `parse_node_revs`）。
要点：

```rust
// status：<status><target><entry path="…"><wc-status item="…"/></entry></target></status>
// ⚠️ entry 可能出现在多个 <target> 下（一次传多个白名单根时）
// ⚠️ 归一：path.replace('\\', "/")
// ⚠️ 排序：按 path 升序，让报错信息可复现

// externals：<properties><target path="…"><property name="svn:externals">RAW</property></target></properties>
// ⚠️ 只取 name="svn:externals" 的 property，别的属性一律忽略
// ⚠️ raw 原样保留，**不 trim 行内容**——旧格式靠空格分列

// diff：<diff><paths><path item props kind>URL</path></paths></diff>
// ⚠️ 四个字段都用 attribute(…).unwrap_or("") 兜底，缺字段不 panic
```

- [ ] **Step 6: 跑测试确认通过 + 全套检查**

```powershell
cd src-tauri; cargo test --test svn_xml
cargo fmt --check; cargo clippy --all-targets
```

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/engine/svn_xml.rs src-tauri/tests/svn_xml.rs
git commit -m "feat(svn): svn_xml 加 status / externals / diff-summary 三个解析器"
```

---

## Task 3：P1 / P2 / P3 三道硬门槛

**Files:**
- Create: `src-tauri/src/engine/svn_preflight.rs`
- Modify: `src-tauri/src/engine/mod.rs`
- Test: `src-tauri/tests/svn_preflight.rs`

**Interfaces:**
- Consumes: `svn_cli::run_svn`、`svn_xml::{parse_status_entries, parse_node_revs}`、`svn_wc::{enumerate_wcs, check_single_repository, peg_safe}`
- Produces:
  ```rust
  pub struct Preflight { pub node_rev: u64, pub repos_root: String, pub uuid: String }
  /// P1–P4 全过时返回录基线需要的三个事实；任一不过返回 Err。
  pub fn run_preflight(repo_root: &str, whitelist: &[String]) -> Result<Preflight>
  ```

**P1 的命令形态（spec §6.5，三个参数缺一不可）：**

```text
svn status --no-ignore --ignore-externals --xml -- <每个白名单根，过 peg_safe>
            ↑ 不带它会漏掉 SVN-ignored 文件，而那些文件 git 照收
                          ↑ 不带它 externals 的节点会混进来（§6.6.4）
```

不允许存在 `M` / `A` / `D` / `R` / `C` / `!` / `~` / `?` 任何一种。**不自动规范化**
——revert 用户的本地修改是破坏性的，只拦住并引导去 TortoiseSVN（对齐 §8.1 #2）。

**P2 的读法（spec §6.6.2）：** `nodeRevs` 必须是单元素集合。1a 的 `parse_node_revs`
已经实现并带空集守卫；这里只负责把多值转成一条有用的错误。

> ⚠️ spec §6.5 写着「多值说明工作副本是 mixed-revision，引导用户先跑一次完整
> `svn update`」，但 §6.6.4 明确指出**当白名单落在 external 里时这条引导永远无效**
> （external 的 revision 来自别的仓库）。错误信息必须按「白名单是否落在 external 内」
> 分两种说法，不要无脑给那句引导。

**P3：** 不在 `svn update` / `merge` 中途。判据是工作副本根的 `.svn` 里有没有未完成
操作的痕迹。**实现前必须先在 fixture 上实测怎么可靠地检测**（造一个中断的 update
很难，若确认造不出稳定 fixture，就降级为「读 `svn status` 是否报 `L`（locked）」并
在决策日志里如实写明覆盖范围）。

- [ ] **Step 1: 写失败测试——P1 抓住每一种脏状态**

```rust
#[test]
fn p1_rejects_every_dirty_state() {
    // 逐个造出来，而不是只造一个 M 就宣称「覆盖了 P1」——
    // AGENTS.md §8：名为「覆盖 X」的测试若测的不是 X 的真实形态属虚假信心。
    for (setup, want) in [
        (DirtyKind::Modified, "modified"),
        (DirtyKind::Unversioned, "unversioned"),
        (DirtyKind::Added, "added"),
        (DirtyKind::Deleted, "deleted"),
    ] {
        let fx = svn_repo_dirty(setup);
        let err = run_preflight(&fx.wc_str(), &["Code/".to_string()])
            .unwrap_or_else(|e| panic!("{want} 必须被 P1 拒绝"));
        assert_eq!(err.code, "svn_preflight_dirty");
        assert!(err.message.contains(want), "实际：{}", err.message);
    }
}

#[test]
fn p1_must_pass_no_ignore_or_svn_ignored_files_slip_through() {
    // **反向证据**：svn:ignore 一个 .cs，再造出那个文件。
    // 不带 --no-ignore 时 svn status 不报它，而 git 照收 → 假基线。
    // 这条如果哪天开始失败，说明有人把 --no-ignore 删了。
    let fx = svn_repo_with_svn_ignored_cs();
    let err = run_preflight(&fx.wc_str(), &["Code/".to_string()])
        .expect_err("SVN-ignored 但 git 会收的文件必须被 P1 拦住");
    assert_eq!(err.code, "svn_preflight_dirty");
}

#[test]
fn a_clean_working_copy_passes_and_reports_the_three_facts() {
    let fx = svn_repo();
    let pre = run_preflight(&fx.wc_str(), &["Code/".to_string()]).unwrap();
    assert_eq!(pre.node_rev, 1);
    assert!(pre.repos_root.starts_with("file:///"));
    assert!(!pre.uuid.is_empty());
}
```

- [ ] **Step 2: 写失败测试——P2 的 rootRev 陷阱**

```rust
#[test]
fn p2_reads_node_revs_not_the_root_revision() {
    // spec §6.5.1 实测：mixed-revision 工作副本里 rootRev=1 而 nodeRevs={3}。
    // 照 rootRev 取 git-svn-id 的 rev，初始化**完成的那一瞬间**就判失同步。
    // 这条测试的价值全在于 fixture 真的造出了两者不等的形态——先断言前提。
    let fx = svn_repo_mixed_revision();
    let root_rev = read_root_rev(&fx.wc_str());
    let err = run_preflight(&fx.wc_str(), &["Code/".to_string()])
        .expect_err("mixed-revision 必须被 P2 拒绝");
    assert_eq!(err.code, "svn_preflight_mixed_revision");
    assert_ne!(root_rev, 3, "fixture 没造出 rootRev != nodeRevs 的形态");
}
```

- [ ] **Step 3: 跑测试确认失败，再实现**

- [ ] **Step 4: 全套检查 + Commit**

---

## Task 4：P5 —— `svn:externals` 钉死检测

**Files:**
- Create: `src-tauri/src/engine/svn_externals.rs`
- Test: `src-tauri/tests/svn_externals.rs`

**Interfaces:**
- Consumes: `svn_xml::parse_externals_props`
- Produces:
  ```rust
  pub struct PinnedExternal { pub defined_in: String, pub local_path: String, pub pinned_rev: u64 }
  /// 判定一行 externals 定义有没有被钉死。**纯函数，不发命令**，便于穷举写法。
  pub fn parse_external_line(line: &str) -> Option<ExternalDef>
  /// 白名单所在的 external 有没有被钉死。命中**不阻断**，返回 pinned 模式的清单。
  pub fn detect_pinned(repo_root: &str, whitelist: &[String]) -> Result<Vec<PinnedExternal>>
  ```

**判据（spec §6.5.2，开工前实测 E8 确认五种写法全部生效）：**

> **不能按列取字段。** 新格式是 `[-r N] URL LOCAL`，旧格式是 `LOCAL [-r N] URL`，
> **字段顺序相反**。唯一可靠的判据是「这一行里出现了 `-r <N>` 记号，**或** URL 带
> `@<N>` 后缀」。漏掉 `@<N>` 是最容易犯的错，因为它不含 `-r`。

- [ ] **Step 1: 写失败测试——五种写法逐一穷举**

```rust
#[test]
fn all_five_pinning_forms_are_detected() {
    // 2026-09-14 实测：这五种写法签出后 follow 停在 HEAD(r2)，其余四种全在 r1。
    // 按列取字段的实现会漏掉最后两种。
    for (line, want) in [
        ("^/lib follow",        None),
        ("-r1 ^/lib pinA",      Some(1)),
        ("-r 1 ^/lib pinB",     Some(1)),   // -r 与数字之间有空格
        ("^/lib@1 pinC",        Some(1)),   // peg 语法，**不含 -r**
        ("pinD -r1 ^/lib",      Some(1)),   // 旧格式，本地路径在前
    ] {
        let def = parse_external_line(line)
            .unwrap_or_else(|| panic!("{line:?} 没解析出来"));
        assert_eq!(def.pinned_rev, want, "line={line:?}");
    }
}

#[test]
fn malformed_external_lines_do_not_panic() {
    // 实测发现：旧格式不带 -r（`lib ^/lib`）时签出直接失败——那种定义本来就
    // 没法用，但解析器不能因此 panic（spec §6.5.2）。
    for line in ["", "   ", "#comment", "lib ^/lib", "-r ^/lib x", "-rABC ^/lib x"] {
        let _ = parse_external_line(line); // 不 panic 即可
    }
}
```

- [ ] **Step 2: 跑测试确认失败，再实现**

- [ ] **Step 3: 写 fixture 级测试**——用真实 `svn propset` 造出五行定义，
      断言 `detect_pinned` 只报那四条、且 `follow` 不在列表里。

> ⚠️ fixture 写 externals 文件时**必须用一致行尾**。实测 E10：混合行尾会让
> `svn propset -F` 报 `E135000: Inconsistent line ending style`，**而且 PowerShell
> 的退出码链抓不到**，属性静默没设上，测试会拿到一个「没有 externals」的假绿。
> Rust 侧用 `std::fs::write` 写 `\n` 即可。

- [ ] **Step 4: 全套检查 + Commit**

---

## Task 5：基线录制 —— 临时索引、trailer、`commit-tree`

**Files:**
- Create: `src-tauri/src/engine/svn_baseline.rs`
- Modify: `src-tauri/src/engine/svn_include_check.rs`（`actual_set` 改走临时索引）
- Test: `src-tauri/tests/svn_baseline.rs`

**Interfaces:**
- Consumes: `git_cli::{run_git, run_git_env}`（Task 1）
- Produces:
  ```rust
  /// 一次录制用的临时索引。Drop 时删文件——录制中途 `?` 早退也不留垃圾。
  pub struct TempIndex { path: std::path::PathBuf }
  impl TempIndex {
      /// 从空索引开始（初始化 / R2）。
      pub fn empty(repo_root: &str) -> Result<Self>
      /// 从当前 HEAD 树开始（其余录制入口，spec §6.3.1）。
      pub fn from_head(repo_root: &str) -> Result<Self>
      pub fn add_all(&self, repo_root: &str, w_paths: &[String]) -> Result<()>
      pub fn ls_files(&self, repo_root: &str) -> Result<BTreeSet<String>>
      pub fn write_tree(&self, repo_root: &str) -> Result<String>
  }

  pub struct Trailer { pub url: String, pub rev: u64, pub uuid: String }
  pub fn format_trailer(t: &Trailer) -> String
  pub fn parse_trailer(message: &str) -> Option<Trailer>

  /// `commit-tree` + 更新 `refs/heads/svn/base`，**用旧 OID 做比较条件**。
  pub fn publish_baseline(
      repo_root: &str, tree: &str, parent: Option<&str>,
      t: &Trailer, expected_old: Option<&str>,
  ) -> Result<String>
  ```

**这是本期最不能写错的一段。** 四条硬要求，每条都有对应的回归测试：

1. **全程不碰真实 index、不碰 HEAD、不碰工作目录文件**（实测 E1 确认可行）
2. **临时索引文件必须清理干净**，包括早退路径 —— 用 `Drop`，不要靠调用点记得删
3. **trailer 三字段缺一不可**，解析失败即「这不是 baseline」，不猜、不部分接受
4. **`update-ref` 必须带旧 OID 作比较条件**（`git update-ref <ref> <new> <old>`）。
   并发的第二个 mygitclient 进程或用户手工操作可能在这中间动过基线；无条件覆盖
   等于悄悄丢掉那次录制

- [ ] **Step 1: 写失败测试 —— 录制不碰用户的任何东西**

```rust
#[test]
fn recording_touches_neither_the_real_index_nor_head_nor_the_worktree() {
    // 这条是整个 Phase 1b 的安全前提。2026-09-14 实测 GIT_INDEX_FILE 能做到，
    // 但「能做到」和「我们的实现做到了」是两回事。
    let dir = workdir_with_exclude();     // Code/A.cs + Code/B.psd，exclude 只放行 .cs
    let root = dir.path().to_str().unwrap();
    run_git(root, &["add", "--", "Code/A.cs"]).unwrap();
    let index_before = run_git(root, &["ls-files", "-s"]).unwrap();
    let head_before = run_git_code(root, &["rev-parse", "-q", "--verify", "HEAD"]).unwrap();

    let idx = TempIndex::empty(root).unwrap();
    idx.add_all(root, &["Code/".to_string()]).unwrap();
    let tree = idx.write_tree(root).unwrap();

    assert_eq!(index_before, run_git(root, &["ls-files", "-s"]).unwrap(), "真实索引被动了");
    assert_eq!(
        head_before,
        run_git_code(root, &["rev-parse", "-q", "--verify", "HEAD"]).unwrap(),
        "HEAD 被动了"
    );
    assert!(dir.path().join("Code/B.psd").exists(), "工作目录文件被删了");

    // 录出来的确实是 W-file：.cs 进、.psd 不进
    let listed = run_git(root, &["ls-tree", "-r", "--name-only", &tree]).unwrap();
    assert!(listed.contains("Code/A.cs"), "{listed}");
    assert!(!listed.contains("B.psd"), "{listed}");
}

#[test]
fn the_temp_index_file_is_gone_even_on_an_early_return() {
    // Drop 而不是显式删：录制中途任何一步 `?` 早退都不能留垃圾。
    let dir = workdir_with_exclude();
    let root = dir.path().to_str().unwrap();
    let path = {
        let idx = TempIndex::empty(root).unwrap();
        idx.path().to_path_buf()
    };
    assert!(!path.exists(), "临时索引没被清掉：{path:?}");
}
```

- [ ] **Step 2: 写失败测试 —— trailer 的往返与拒绝**

```rust
#[test]
fn the_trailer_round_trips_through_a_real_commit() {
    // 不是「格式化出我以为的字符串」，是**真的建一个 commit 再读回来**。
    let dir = workdir_with_exclude();
    let root = dir.path().to_str().unwrap();
    let idx = TempIndex::empty(root).unwrap();
    idx.add_all(root, &["Code/".to_string()]).unwrap();
    let tree = idx.write_tree(root).unwrap();
    let t = Trailer {
        url: "https://svn.example.com/game".into(),
        rev: 12346,
        uuid: "8f3a1c7e-4b21-0410-9d2f-1a7b3c5d9e01".into(),
    };
    let oid = publish_baseline(root, &tree, None, &t, None).unwrap();

    let msg = run_git(root, &["log", "-1", "--format=%B", &oid]).unwrap();
    let back = parse_trailer(&msg).expect("读不回来");
    assert_eq!(back.rev, 12346);
    assert_eq!(back.url, "https://svn.example.com/game");
    assert_eq!(back.uuid, "8f3a1c7e-4b21-0410-9d2f-1a7b3c5d9e01");

    // git svn 的标准用法也要能命中（spec §6.4.1 的互操作收益）
    let found = run_git(
        root,
        &["log", "--grep=^git-svn-id", "--format=%H", "refs/heads/svn/base"],
    )
    .unwrap();
    assert!(found.contains(&oid), "--grep 找不到：{found}");
}

#[test]
fn a_trailer_missing_any_field_is_not_a_baseline() {
    // 「不猜、不部分接受」（spec §6.4.1）。三个字段各缺一次。
    for bad in [
        "svn: r1\n\ngit-svn-id: https://x/game@12346\n",          // 缺 uuid
        "svn: r1\n\ngit-svn-id: https://x/game 8f3a-1c7e\n",      // 缺 @rev
        "svn: r1\n\ngit-svn-id: @12346 8f3a-1c7e\n",              // 缺 url
        "svn: r1\n\ngit-svn-id: https://x/game@abc 8f3a-1c7e\n",  // rev 不是数字
        "svn: r1\n\n",                                            // 根本没有
    ] {
        assert!(parse_trailer(bad).is_none(), "不该被接受：{bad:?}");
    }
}
```

- [ ] **Step 3: 写失败测试 —— `update-ref` 的并发保护**

```rust
#[test]
fn publishing_refuses_when_the_baseline_moved_underneath_us() {
    // 另一个 mygitclient 进程、或用户手工操作，都可能在录制与发布之间动过基线。
    // 无条件覆盖等于悄悄丢掉那次录制。
    let dir = workdir_with_exclude();
    let root = dir.path().to_str().unwrap();
    let idx = TempIndex::empty(root).unwrap();
    idx.add_all(root, &["Code/".to_string()]).unwrap();
    let tree = idx.write_tree(root).unwrap();
    let first = publish_baseline(root, &tree, None, &trailer(1), None).unwrap();

    // 有人在这中间把基线挪走了
    let other = publish_baseline(root, &tree, Some(&first), &trailer(2), Some(&first)).unwrap();

    // 我们还拿着 first 当旧值 → 必须失败，且不能把 other 覆盖掉
    let err = publish_baseline(root, &tree, Some(&first), &trailer(3), Some(&first))
        .expect_err("旧 OID 对不上必须拒绝");
    assert_eq!(err.code, "svn_baseline_moved");
    let now = run_git(root, &["rev-parse", "refs/heads/svn/base"]).unwrap();
    assert_eq!(now.trim(), other, "别人的基线被覆盖了");
}
```

- [ ] **Step 4: 跑测试确认失败，再实现**

- [ ] **Step 5: 把 `actual_set` 改走临时索引**

这是 Phase 1a 决策日志 §9.8 记账过来的一条。1a 的 `actual_set` 用
`--cached ∪ --others --exclude-standard − --deleted` **模拟** `git add -A`，是当时
写明的偏离；新版 spec §6.3.1 已改成「核对与录制**必须使用同一份候选索引**」。

现在 `TempIndex` 有了，模拟版就是**第二个会分叉的事实源**——而 1a 的 B1 已经证明
「模拟 `git add -A`」很容易写错（那条等价性测试跑在空索引上，会分叉的一半根本没
执行到）。删掉它，`actual_set` 改成「在调用方给的候选索引上读 `ls-files -z`」。

```rust
// 新签名：核对与录制共用同一个 TempIndex，不再各自构造
pub fn verify_include_set(
    repo_root: &str, idx: &TempIndex,
    whitelist: &[String], extensions: &[String],
) -> Result<()>
```

⚠️ **1a 那两条等价性测试要一并处理**，不要留着测一个已经删掉的东西：

- `the_actual_set_equals_what_git_add_would_have_produced`
- `the_actual_set_equals_git_add_even_when_the_index_is_already_populated`

它们的**断言目标仍然成立且仍然有价值**（「候选索引的内容 == `git add -A` 的产物」），
只是被测对象从模拟版换成了真索引。改写而不是删除，并在决策日志里写清为什么。

- [ ] **Step 6: 全套检查 + Commit**

---

## Task 6：§6.8 基线体检

**Files:**
- Create: `src-tauri/src/engine/svn_health.rs`
- Test: `src-tauri/tests/svn_health.rs`

**Interfaces:**
- Consumes: `svn_cli::run_svn`、`svn_xml::parse_diff_summary`、`svn_wc::peg_safe`
- Produces:
  ```rust
  pub enum Health {
      /// 命令成功，过滤后无 W-file 内容差异。
      Verified { rev: u64 },
      /// 命令成功，仍有 W-file 内容差异 —— **基线撒谎**。
      Lying { files: Vec<String> },
      /// 命令没跑成（断网 / 认证 / URL 不可达）。**不是基线有问题。**
      NotChecked { reason: String },
  }
  pub fn check_baseline(repo_root: &str, svn_url: &str, rev: u64,
                        whitelist: &[String], extensions: &[String]) -> Result<Health>
  ```

**命令形态（spec §6.8.2，实测 E4 / E5）：**

```text
svn diff --old=<svnUrl>@<rev> --new=<工作副本根> --summarize --xml -- <白名单根…>
                                                              ↑ 用 --xml，不是文本版
⚠️ diff **拒绝** --ignore-externals（实测 exit 1），而且**不需要**它
   ——URL 与工作副本目录比对时本来就不走进 externals（spec §6.8.2 R3）
```

**过滤规则（spec §6.8.4）：只看内容变更、只看 W-file 文件。**
`--xml` 让这一步变成三个独立判断，不必靠列位置：

```text
保留：kind == "file"  且  item != "none"  且  剥掉 URL 前缀后命中 W-file
剔除：kind == "dir"（目录条目）
剔除：item == "none" && props == "modified"（纯属性变更）
剔除：扩展名不在 extensions 里的文件
      —— 否则恢复流程有意保留的 Code/Art.psd 本地修改会让体检误报
```

- [ ] **Step 1: 写失败测试 —— spec §6.8.2 的 T1–T7 逐条**

spec 已经在真实 fixture 上跑过这七个场景并记下了结果，**照着它建测试**：

```rust
#[test] fn t1_a_clean_working_copy_at_the_right_revision_is_verified() { /* … */ }
#[test] fn t2_a_locally_modified_source_file_is_reported() { /* … */ }

#[test]
fn t3_a_modified_art_file_does_not_trigger_a_false_alarm() {
    // 只传白名单根 + 只看 W-file。这条是「不误报」那一侧，比 T2 更重要
    // ——误报会让一个正常仓库永远建不起来。
}

#[test]
fn t4_an_outdated_baseline_rev_is_caught() {
    // 磁盘已是 r2，仍拿 baselineRev=1 比 → 必须报 Lying。这正是要抓的那类。
}

#[test]
fn t7_a_file_deleted_on_the_server_is_caught() {
    // 覆盖 §9.4.1 那类漏检
}
```

- [ ] **Step 2: 写失败测试 —— 三种结果必须分开**

```rust
#[test]
fn a_command_failure_is_not_reported_as_a_lying_baseline() {
    // spec §6.8.6：把「没能验证」当成「验证不通过」会让离线场景寸步难行，
    // 当成「验证通过」则等于这条检查不存在。指一个不可达的 URL。
    let fx = svn_repo();
    let got = check_baseline(
        &fx.wc_str(),
        "file:///definitely/not/here",
        1,
        &["Code/".to_string()],
        &[".cs".to_string()],
    )
    .unwrap();
    assert!(matches!(got, Health::NotChecked { .. }), "实际：{got:?}");
}
```

- [ ] **Step 3: 跑测试确认失败，再实现**

- [ ] **Step 4: 全套检查 + Commit**

---

## Task 7：初始化向导编排与失败回退

**Files:**
- Create: `src-tauri/src/engine/svn_init.rs`
- Test: `src-tauri/tests/svn_init.rs`

**Interfaces:**
- Consumes: Task 3–6 的全部产物 + 1a 的 `svn_config`、`svn_exclude`、`svn_include_check`
- Produces:
  ```rust
  pub struct InitRequest { pub repo_root: String, pub whitelist: Vec<String>, pub extensions: Vec<String> }
  pub fn init_hybrid(req: &InitRequest) -> Result<InitReport>
  ```

**序列（spec §6.5，逐字）：**

```text
P1–P5                                ← P1–P4 硬门槛，P5 命中只切 pinned 模式
git init -b svn/base                 ← 显式指定，不吃 init.defaultBranch 的默认值
写入 .git/mygitclient.json
生成 .git/info/exclude               ← 含末尾的 .svn/ 兜底行
纳入集合核对（§6.3.1）                ← 临时索引，期望集合 vs ls-files，得到 TREE
用该 TREE 录首个基线                  ← rev 取自 P2 的 N；**无父提交**
基线体检（§6.8）                      ← 有差异则整个回退，删掉刚建的 .git
git checkout -b work                 ← 立刻离开基线分支
```

**回退是这个任务的核心，不是收尾。** 失败点有七处，每一处的回退范围不同：

| 失败点 | 已经做了什么 | 回退到哪 |
|---|---|---|
| P1–P4 任一不过 | 什么都没做 | 无需回退 |
| `git init` 失败 | 可能留了半个 `.git` | 删掉**本次创建的** `.git` |
| 写配置 / exclude 失败 | `.git` 已建 | 同上 |
| 纳入集合核对不过 | `.git` 已建，未录基线 | 同上；错误里带 `check-ignore --no-index` 的规则来源 |
| 录制失败 | 临时索引可能还在 | `TempIndex` 的 Drop 负责；`.git` 照删 |
| **体检报 Lying** | 基线已录、ref 已指 | **整个回退，删掉刚建的 `.git`**（spec §6.8.5：此刻还没建工作分支，代价最小） |
| 体检 `NotChecked` | 同上 | **不回退**，标记「基线未经体检」并在 UI 如实显示 |

> ⚠️ **只能删本次创建的 `.git`。** 目录里原本就有 `.git` 时（`RepoKind::SvnSuspicious`）
> 根本不该走到这里——`backend_for` 在 1a 就已经拦住了。但 `init_hybrid` 自己**也要
> 再判一次**：这是唯一一处会 `remove_dir_all` 的代码，不能依赖上游调用点判对了。

- [ ] **Step 1: 写失败测试 —— 每一处失败都不留痕迹**

```rust
#[test]
fn a_failing_health_check_rolls_the_whole_thing_back() {
    // 造一个「磁盘不等于服务器」的工作副本（用 T4 的形态：基线 rev 与服务器
    // 内容不一致）。断言：.git 不存在、工作目录文件一个没少。
    let fx = svn_repo_whose_disk_lies();
    let err = init_hybrid(&req(&fx)).expect_err("体检必须拦住");
    assert_eq!(err.code, "svn_baseline_lying");
    assert!(!fx.wc().join(".git").exists(), "刚建的 .git 没删干净");
    assert!(fx.wc().join("Code/A.cs").exists(), "工作目录文件被删了");
}

#[test]
fn init_refuses_to_touch_a_directory_that_already_has_a_git() {
    // 唯一一处 remove_dir_all，不能依赖上游判对了。
    let fx = svn_repo();
    std::fs::create_dir_all(fx.wc().join(".git")).unwrap();
    std::fs::write(fx.wc().join(".git/PRECIOUS"), "x").unwrap();
    let err = init_hybrid(&req(&fx)).expect_err("已有 .git 必须拒绝");
    assert_eq!(err.code, "svn_unconfigured_git");
    assert!(fx.wc().join(".git/PRECIOUS").exists(), "把别人的 .git 删了");
}

#[test]
fn a_successful_init_leaves_head_on_work_not_on_the_baseline() {
    // spec §6.5 最后一步：立刻离开基线分支。UI 禁止在基线分支上开发提交。
    let fx = svn_repo();
    init_hybrid(&req(&fx)).unwrap();
    let head = run_git(&fx.wc_str(), &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap();
    assert_eq!(head.trim(), "work");
}

#[test]
fn a_successful_init_emits_no_svn_write_command() {
    // G8 在本期的延续。服务器侧 revision 必须没变。
    let fx = svn_repo();
    let before = server_head_rev(&fx);
    init_hybrid(&req(&fx)).unwrap();
    assert_eq!(before, server_head_rev(&fx), "初始化动了服务器");
}
```

- [ ] **Step 2: 跑测试确认失败，再实现**

- [ ] **Step 3: 全套检查 + Commit**

---

## Task 8：重建基线 R1–R5

**Files:**
- Modify: `src-tauri/src/engine/svn_init.rs`
- Test: `src-tauri/tests/svn_init.rs`（追加）

**Interfaces:**
- Produces: `pub fn rebuild_baseline(repo_root: &str) -> Result<RebuildReport>`

**序列（spec §16）：**

```text
R1  校验：白名单内 svn status --no-ignore 干净、nodeRevs 单值 {N}
      不干净就引导去 TortoiseSVN，**不自动 revert**（同 P1）
R2  临时索引录 tree，全程不碰真实 index、不碰 HEAD
R3  git commit-tree TREE -p svn/base …                   → NEWBASE
R3b 基线体检（§6.8）        ← 此刻 TREE 就是磁盘内容，前提成立
                              **有差异就中止，连指针都还没动，代价为零**
R4  git update-ref refs/heads/svn/base NEWBASE           ← 只动指针
R5  删掉临时索引文件
```

**spec 专门写了一段「为什么不能照抄 §9.4 的 `git checkout svn/base`」，必须照做：**

> §9.4 先 checkout 旧基线是因为它后面紧跟着 `svn revert`，由 revert 把服务器内容重新
> 铺回磁盘。极简版**没有 revert**（前提就是工作副本干净），此时先 checkout 会把磁盘
> **换回旧基线内容**，随后录下来的就是旧内容却标着新 revision ——**正好造出这一节要
> 防的那种撒谎基线**。

- [ ] **Step 1: 写失败测试 —— 反向证据**

```rust
#[test]
fn rebuild_records_the_current_disk_not_the_old_baseline_content() {
    // **反向证据**：这条证明「不 checkout」不是偷懒。
    // 造出「基线停在 r1、磁盘已是 r2」的形态（模拟用户在 TortoiseSVN 里 update 过），
    // 重建后断言基线树里是 r2 的内容、rev 也是 2。
    // 若实现里混进了 git checkout svn/base，这条会拿到 r1 的内容配 r2 的 rev。
    let fx = hybrid_repo_updated_externally();
    rebuild_baseline(&fx.wc_str()).unwrap();
    let content = run_git(&fx.wc_str(), &["show", "refs/heads/svn/base:Code/A.cs"]).unwrap();
    assert!(content.contains("v2"), "录进去的是旧基线内容：{content}");
    let msg = run_git(&fx.wc_str(), &["log", "-1", "--format=%B", "refs/heads/svn/base"]).unwrap();
    assert_eq!(parse_trailer(&msg).unwrap().rev, 2);
}

#[test]
fn rebuild_never_moves_head_and_survives_uncommitted_work() {
    // spec §16：R1–R5 全程不动 HEAD，于是工作分支上有未提交改动也不受影响，
    // 也不存在「流程结束忘了切回原分支」。
    let fx = hybrid_repo_updated_externally();
    std::fs::write(fx.wc().join("Code/WIP.cs"), "wip").unwrap();
    let head_before = run_git(&fx.wc_str(), &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap();
    rebuild_baseline(&fx.wc_str()).unwrap();
    assert_eq!(
        head_before,
        run_git(&fx.wc_str(), &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap()
    );
    assert!(fx.wc().join("Code/WIP.cs").exists(), "未提交的工作被吃掉了");
}

#[test]
fn a_failing_r3b_leaves_the_baseline_pointer_untouched() {
    // R3b 在 R4 之前，代价为零 —— 断言指针真的没动。
    let fx = hybrid_repo_whose_disk_lies();
    let before = run_git(&fx.wc_str(), &["rev-parse", "refs/heads/svn/base"]).unwrap();
    let err = rebuild_baseline(&fx.wc_str()).expect_err("体检必须拦住");
    assert_eq!(err.code, "svn_baseline_lying");
    assert_eq!(
        before,
        run_git(&fx.wc_str(), &["rev-parse", "refs/heads/svn/base"]).unwrap()
    );
}
```

- [ ] **Step 2: 跑测试确认失败，再实现**

- [ ] **Step 3: 全套检查 + Commit**

---

## Task 9：Tauri 命令与向导 UI

**Files:**
- Create: `src-tauri/src/commands/svn.rs`、`src/engine/svnInit.ts`、`src/components/SvnInitWizard.tsx`
- Modify: `src-tauri/src/lib.rs`（注册）、`src/store/repoStore.ts`
- Test: `src-tauri/tests/svn_backend.rs`（追加）、`tests/vitest/svnInit.test.ts`

**命令：**

```rust
#[tauri::command(async)] pub fn svn_preflight_check(path: String, /* … */) -> Result<PreflightReport, GitError>
#[tauri::command(async)] pub fn svn_init_hybrid(path: String, /* … */) -> Result<InitReport, GitError>
#[tauri::command(async)] pub fn svn_rebuild_baseline(path: String) -> Result<RebuildReport, GitError>
#[tauri::command(async)] pub fn svn_check_baseline(path: String) -> Result<HealthReport, GitError>
```

**⚠️ `AGENTS.md` §7.3 的两条，本期比 1a 更要紧：**

1. **Tauri 把 `snake_case` 转成 JS 的 `camelCase`，这层转换没有编译期检查。** 每个
   跨边界的结构体都要 `#[serde(rename_all = "camelCase")]`，并**用测试钉住序列化后的
   字段名**——1a 已经立了这个做法（`svn_backend.rs` 里那条 serde 测试），照做。
2. **签名改动必须实际跑一次 `pnpm tauri dev` 验证该命令真的通。** 本期是四个**新**
   命令，且其中三个会真的改磁盘 —— 这一条不是形式。

**前端纯逻辑下沉（`AGENTS.md` §7.2）：** 向导的步骤状态机、错误分类、「哪一步该显示
什么」放 `src/engine/svnInit.ts`，组件只做渲染与事件绑定。1a 的 P2-2 已经证明过
「判断散在组件里」会漏 —— 同一个 `confirm()` 的两半，两轮 review 才抓齐。

- [ ] **Step 1: 写前端纯逻辑的失败测试**（步骤状态机、三种体检结果的显示文案分类）
- [ ] **Step 2: 实现 `src/engine/svnInit.ts`**
- [ ] **Step 3: 写 Rust 命令层测试**（含 serde 字段名断言）
- [ ] **Step 4: 实现命令与注册**
- [ ] **Step 5: 实现向导 UI**
- [ ] **Step 6: 两侧全套检查 + Commit**

---

## Task 10：收尾验证

- [ ] **Step 1: 两侧全套检查，记录实测数字**

```powershell
cd src-tauri
cargo fmt --check
cargo clippy --all-targets
cargo test -j 2
cd ..
pnpm build
pnpm test
```

- [ ] **Step 2: 命令注册对账**

`#[tauri::command]` 的数量 == `lib.rs` 注册数量 == Phase 1a 的 46 + 本期新增数。
1a 踩过一次：raw grep 命中数比真实命令数多 1，因为 `session.rs:9` 是条 doc 注释。

- [ ] **Step 3: G8 断言 —— 本期仍然零 SVN 写操作**

```powershell
# 必须为空
Select-String -Path src-tauri\src\**\*.rs -Pattern '"(add|commit|update|revert|delete|propset|merge|mkdir|copy|move)"' |
  Where-Object { $_.Line -match 'run_svn' }
# 正向枚举：svn_* 模块实际发出的子命令应当只有 info / status / propget / diff / --version
```

- [ ] **Step 4: 真实窗口验证（`AGENTS.md` §5.1）**

**本期新增四个 Tauri 命令，其中三个会改磁盘 —— 这一步不能再跳。**

同时补跑 **Phase 1a 遗留的六个场景**（用户已确认并进本期）：1a 的两个命令
（`read_capabilities` / `probe_svn_environment`）是在窗口验证没跑的情况下进 master 的，
清单见 `2026-09-13-svn-phase1a-foundation-decisions.md` 的 9.4 一节。

本期自己的场景：

| # | 形态 | 期望 |
|---|---|---|
| 11 | 干净 SVN 工作副本走完向导 | 建出 `.git`、`svn/base` 有 trailer、HEAD 在 `work`、服务器 revision 未变 |
| 12 | 白名单内有本地修改 | P1 拦住，**不自动 revert**，引导去 TortoiseSVN |
| 13 | mixed-revision | P2 拦住；白名单在 external 内时**不给「先 update」那句无效引导** |
| 14 | 白名单跨仓库 | P4 拦住并指出是哪两条分属哪两个仓库 |
| 15 | externals 被 `-r` 钉死 | **不阻断**，进 pinned 模式并常驻说明 |
| 16 | 体检报 Lying | 整个回退，`.git` 不留，工作目录文件一个没少 |
| 17 | 断网时体检 | `NotChecked`，**不回退**，UI 如实显示「基线未经体检」 |
| 18 | 外部 TortoiseSVN update 后重建基线 | 录的是新磁盘内容配新 rev；HEAD 与未提交改动都没动 |

全部在临时 fixture 上做，**禁止拿任何真实 SVN 仓库试**（`AGENTS.md` 开篇的最高优先级约束）。

- [ ] **Step 5: 独立 review（`AGENTS.md` §4.1 合并前门槛）**

由独立于实现者的 agent 按 §13 rubric 跑一遍全量 diff。**先读决策日志的「偏离计划」
再看 diff**（rubric #12）。发现的高危问题合并前必须修完。

- [ ] **Step 6: 决策日志补完四类条目，开 Draft PR**

---

## Self-Review

**1. Spec 覆盖**

| spec 节 | 对应 Task |
|---|---|
| §6.4 基线分支 / §6.4.1 trailer 格式 | Task 5 |
| §6.5 初始化向导 P1–P5 | Task 3（P1–P3）、1a 的 `svn_wc`（P4）、Task 4（P5）、Task 7（编排） |
| §6.5.1 为什么 P1–P3 不能省 | Task 3 Step 2 的 rootRev 陷阱测试 |
| §6.5.2 钉死检测的两条手段 | Task 4 覆盖**手段一**（离线解析、负责阻断）。**手段二（行为校验）依赖 `svn update` 之后的比对，而本期不发 update —— 明确留给 Phase 2**，已在决策日志记为有意缺口 |
| §6.5.3 pinned 模式 | Task 4 只做检测与模式标记；两步 update 属 §7.3 S5，Phase 2 |
| §6.3.1 纳入集合核对（新版：临时索引） | Task 5 Step 5 |
| §6.8 基线体检 T1–T7、三种结果 | Task 6 |
| §16 重建基线 R1–R5 | Task 8 |
| §12.2 空参数一律拒绝 | 每个 Task 的空白名单用例（1a 已立此规矩） |

**明确不在本期的**：§6.6 状态判定与失同步检测、§6.7 本地补丁标记、§10 混合图、
SVN log 解析。它们属于 Phase 1c（「看得见」那半），与本期没有依赖倒置——
本期产出基线，1c 才有东西可比、可画。

**2. 占位符扫描**

Task 3 的 P3 是本计划唯一没有给出确定实现的地方，**这是有意的**：怎么可靠检测
「update 中途」需要先在 fixture 上实测，而我没能在写计划时造出稳定的中断态。
计划里已写明「若确认造不出稳定 fixture，就降级为读 `svn status` 的 `L` 并在决策
日志里如实写明覆盖范围」——给了判据和退路，不是 TODO。

**3. 类型一致性**

`TempIndex` 在 Task 5 定义，Task 7 / Task 8 使用；
`Trailer` / `parse_trailer` 在 Task 5 定义，Task 8 的测试使用；
`Preflight` 在 Task 3 定义，Task 7 使用其 `node_rev` 作为 trailer 的 rev；
`Health` 在 Task 6 定义，Task 7 / Task 8 按三个变体分别处理。

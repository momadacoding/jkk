# Phase 2a：行尾归一化落地 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让混合仓库按字节读写工作目录——在 `.git/info/attributes` 装上 `* -text`，新建库天然生效，旧库侦测到规则缺失时引导用户走一次「写规则 + 重建基线」的原子迁移。

**Architecture:** 新增 `svn_attributes.rs`，形状照抄同目录的 `svn_exclude.rs`（生成头标记 + 备份用户原文 + 临时文件 `fsync` 后 `rename`）。`init_hybrid` 在**录第一个基线之前**写规则。旧库的迁移做成一条命令：写规则 → 重建基线，重建失败则把 `attributes` 原样还原，绝不让仓库停在「规则装了但基线还是旧的」这个状态上。前端把它接进已有的 `BaselineBanner`。

**Tech Stack:** Rust（`std::fs`、`run_git`）、Tauri 2 命令、React 18 + TypeScript、cargo test（fixture 仓库）、Vitest。

**Spec:** `docs/superpowers/specs/2026-09-12-svn-git-hybrid-design.md` —— 本计划实现 §7.3.0 全节。执行者必须先读 §7.3.0，尤其是「迁移」那一小节。

## Global Constraints

以下每一条都来自 spec 或 `AGENTS.md`，是**每个 Task 的隐含验收项**：

- 规则文本固定为 `* -text`，写在 `<仓库根>/.git/info/attributes`。**不写工作目录里的 `.gitattributes`**（spec §3：工作目录里只多出 `.git` 一个条目；写进工作树会被提交进用户的 SVN 仓库）。
- **不改 `core.autocrlf`**。spec §7.3.0 实测：工作树里一个 `* text=auto` 就能把 `core.autocrlf=false` 覆盖掉，而 `$GIT_DIR/info/attributes` 压得住。改 config 是一道能被用户仓库内容掀翻的闸。
- **判断「要不要迁移」禁止用 `git status` 干净与否。** git 的 stat cache 会在翻完设置后假报 0 个脏文件（spec §7.3.0 实测）。判据只能是「规则在不在」。
- 写文件一律走「临时文件 → `sync_all` → `rename`」，临时文件与目标**同目录**（跨卷 `rename` 在 Windows 上报 `ERROR_NOT_SAME_DEVICE`）。参见 `src-tauri/src/engine/svn_exclude.rs` 的 `write_exclude`。
- 所有 git CLI 调用走 `crate::engine::git_cli::run_git`，错误映射为 `crate::error::GitError`，不向前端抛裸字符串（`AGENTS.md` §7.3）。
- 所有涉及 git / svn 写操作的测试必须自建临时 fixture 仓库，用 `src-tauri/tests/common/svn_fixtures.rs` 里的构造器，测完自动清理（`AGENTS.md` 开篇最高优先级约束 + §8）。
- Tauri 命令参数在 JS 侧是 `camelCase`，Rust 侧是 `snake_case`，这层转换**没有编译期检查**（`AGENTS.md` §7.3）。本计划新增两条命令，Task 5 结束时必须在 `pnpm tauri dev` 的真实窗口里各调用一次。
- Rust 注释用中文，解释「为什么」而不是「做了什么」；TS 单引号、尾随逗号；`tsconfig` 的 `strict` / `noUnusedLocals` / `noUnusedParameters` 不许为了编译通过而放宽，不许加 `any` / `@ts-ignore`。
- 本仓库**没有 lint 工具、没有 PR 门禁 CI**。禁止声称「lint 通过」或「CI 通过」。`tests/vitest/**` 不在 `tsc` 覆盖范围内，测试文件的类型错误只有 `pnpm test` 跑起来才会暴露。

## File Structure

| 文件 | 职责 | 动作 |
|---|---|---|
| `src-tauri/src/engine/svn_attributes.rs` | 生成规则文本、原子写入、侦测规则是否已装、回滚还原 | **新建** |
| `src-tauri/src/engine/mod.rs` | 注册新模块 | 改 1 行 |
| `src-tauri/src/engine/svn_init.rs` | `build_repo` 写规则（录基线之前）；新增 `eol_status` / `migrate_eol` | 改 |
| `src-tauri/src/commands/svn.rs` | 暴露 `svn_eol_status` / `svn_migrate_eol` | 改 |
| `src-tauri/src/lib.rs` | 注册两条命令到 `invoke_handler` | 改 |
| `src-tauri/tests/svn_attributes.rs` | 模块级测试：断言 **git 的实际行为**，不是生成的文本 | **新建** |
| `src-tauri/tests/svn_init.rs` | 建库/侦测/迁移的集成测试 | 追加 |
| `src/engine/svnSync.ts` | `EolStatus` 类型 + `describeEolMigration` + 横幅优先级 | 改 |
| `src/store/repoStore.ts` | 两条 `invoke` 包装 + `svnEol` 状态 | 改 |
| `src/components/BaselineBanner.tsx` | 渲染迁移横幅 | 改 |
| `src/components/RepoTab.tsx` | `handleMigrateEol` + 确认框 + 接线 | 改 |
| `tests/vitest/svnSync.test.ts` | 横幅与优先级的纯逻辑测试 | 追加 |

**为什么单独开 `svn_attributes.rs` 而不是并进 `svn_exclude.rs`：** 两者都写 `.git/info/`，但回答的是不同问题（哪些文件受 git 管 vs. 字节怎么读写），且 `attributes` 多一条 `exclude` 没有的职责——**回滚**（Task 4 要在重建失败时还原原文）。并在一起会让 `svn_exclude.rs` 那段「为什么分三段发」的长论证和行尾的论证互相干扰。

---

### Task 1: `svn_attributes.rs` —— 生成、原子写入、侦测、回滚

**Files:**
- Create: `src-tauri/src/engine/svn_attributes.rs`
- Modify: `src-tauri/src/engine/mod.rs`（加 `pub mod svn_attributes;`，按字母序插在 `svn_baseline` 之前）
- Test: `src-tauri/tests/svn_attributes.rs`

**Interfaces:**
- Consumes: `crate::error::{GitError, Result}`
- Produces:
  - `pub const GENERATED_HEADER: &str`
  - `pub const EOL_RULE: &str = "* -text";`
  - `pub fn generate_attributes() -> String`
  - `pub fn write_attributes(repo_root: &str) -> Result<Option<Vec<u8>>>` —— 返回**写之前**那个文件的原始字节（`None` = 原本不存在），给调用方回滚用
  - `pub fn restore_attributes(repo_root: &str, previous: Option<&[u8]>) -> Result<()>`
  - `pub fn has_eol_rule(repo_root: &str) -> bool`

- [ ] **Step 1: 写失败的测试**

新建 `src-tauri/tests/svn_attributes.rs`：

```rust
//! `.git/info/attributes` 的生成与写入测试。
//!
//! **断言对象是 git 的实际裁决（`git check-attr`），不是生成出来的文本**
//! ——测文本只能证明「拼出了我以为的字符串」，证明不了 git 会不会照做，
//! 而这个模块存在的全部理由就是后者（spec §7.3.0）。同 `svn_exclude.rs` 的立场。

use app_lib::engine::svn_attributes::{
    has_eol_rule, restore_attributes, write_attributes, GENERATED_HEADER,
};
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn run_git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("git not found");
    assert!(
        out.status.success(),
        "git {args:?} 失败：{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// `git check-attr text -- <path>` 的裁决，形如 `Code/A.cs: text: unset`。
fn text_attr(dir: &Path, path: &str) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["check-attr", "text", "--", path])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .rsplit(':')
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

fn repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    run_git(dir.path(), &["init", "-b", "main"]);
    // 本机 core.autocrlf 可能是任何值；显式打开，测的才是「规则挡不挡得住它」
    run_git(dir.path(), &["config", "core.autocrlf", "true"]);
    fs::create_dir_all(dir.path().join("Code")).unwrap();
    fs::write(dir.path().join("Code/A.cs"), "a\n").unwrap();
    dir
}

#[test]
fn rule_turns_off_eol_conversion_for_every_path() {
    let dir = repo();
    assert_eq!(
        text_attr(dir.path(), "Code/A.cs"),
        "unspecified",
        "前提：没写规则时 text 属性未指定"
    );

    write_attributes(dir.path().to_str().unwrap()).unwrap();

    assert_eq!(
        text_attr(dir.path(), "Code/A.cs"),
        "unset",
        "写了 `* -text` 之后 git 必须把 text 判成 unset"
    );
}

#[test]
fn rule_beats_a_gitattributes_committed_into_the_working_tree() {
    // spec §7.3.0 取 C 不取 B 的**决定性理由**：$GIT_DIR/info/attributes 的优先级
    // 压过工作树里的 .gitattributes。别人真把一个 `* text=auto` 提交进 SVN 时，
    // `core.autocrlf=false` 那条路会被掀翻，这条不会。
    let dir = repo();
    write_attributes(dir.path().to_str().unwrap()).unwrap();
    fs::write(dir.path().join(".gitattributes"), "* text=auto\n").unwrap();

    assert_eq!(
        text_attr(dir.path(), "Code/A.cs"),
        "unset",
        "工作树里的 `* text=auto` 不该盖过 .git/info/attributes"
    );
}

#[test]
fn has_eol_rule_reports_whether_we_installed_it() {
    let dir = repo();
    let root = dir.path().to_str().unwrap();
    assert!(!has_eol_rule(root), "没写过就该报 false");

    write_attributes(root).unwrap();
    assert!(has_eol_rule(root), "写过就该报 true");
}

#[test]
fn a_users_own_attributes_file_is_backed_up_not_swallowed() {
    // `.git/info/attributes` 是 git 给用户自己用的地方。无条件整文件覆盖
    // 等于删掉用户手写的规则且不留痕迹。同 write_exclude 的立场。
    let dir = repo();
    let root = dir.path().to_str().unwrap();
    let info = dir.path().join(".git/info");
    fs::create_dir_all(&info).unwrap();
    fs::write(info.join("attributes"), "*.psd binary\n").unwrap();

    write_attributes(root).unwrap();

    assert_eq!(
        fs::read_to_string(info.join("attributes.bak")).unwrap(),
        "*.psd binary\n",
        "用户原有的规则必须备份成 attributes.bak"
    );
    assert!(fs::read_to_string(info.join("attributes"))
        .unwrap()
        .starts_with(GENERATED_HEADER));
}

#[test]
fn rewriting_our_own_file_does_not_create_a_second_backup() {
    // 备份已存在则不再覆盖——那说明我们早就接管过一次，attributes.bak 里
    // 才是用户真正的原件。第二次写不许把它糊成我们自己生成的内容。
    let dir = repo();
    let root = dir.path().to_str().unwrap();
    let info = dir.path().join(".git/info");
    fs::create_dir_all(&info).unwrap();
    fs::write(info.join("attributes"), "*.psd binary\n").unwrap();

    write_attributes(root).unwrap();
    write_attributes(root).unwrap();

    assert_eq!(
        fs::read_to_string(info.join("attributes.bak")).unwrap(),
        "*.psd binary\n",
        "第二次写把用户的原件覆盖掉了"
    );
}

#[test]
fn restore_puts_back_exactly_what_was_there_before() {
    // Task 4 的回滚要靠它：迁移时重建基线失败，仓库必须回到「规则没装」的
    // 原状，不能停在「规则装了但基线还是旧的」——那正是 spec §7.3.0 说的
    // 潜伏故障状态。
    let dir = repo();
    let root = dir.path().to_str().unwrap();
    let target = dir.path().join(".git/info/attributes");

    let prev = write_attributes(root).unwrap();
    assert!(prev.is_none(), "原本不存在时应返回 None");
    assert!(target.exists());

    restore_attributes(root, prev.as_deref()).unwrap();
    assert!(!target.exists(), "原本不存在，还原后就该不存在");
    assert!(!has_eol_rule(root));
}

#[test]
fn restore_brings_back_a_users_original_bytes() {
    let dir = repo();
    let root = dir.path().to_str().unwrap();
    let info = dir.path().join(".git/info");
    fs::create_dir_all(&info).unwrap();
    fs::write(info.join("attributes"), "*.psd binary\n").unwrap();

    let prev = write_attributes(root).unwrap();
    restore_attributes(root, prev.as_deref()).unwrap();

    assert_eq!(
        fs::read_to_string(info.join("attributes")).unwrap(),
        "*.psd binary\n",
        "还原必须逐字节回到用户原来的内容"
    );
}
```

- [ ] **Step 2: 跑测试确认它失败**

Run: `cd src-tauri && cargo test --test svn_attributes`

Expected: 编译失败，`unresolved import app_lib::engine::svn_attributes`。

- [ ] **Step 3: 写实现**

新建 `src-tauri/src/engine/svn_attributes.rs`：

```rust
//! 由代码生成 `.git/info/attributes`，关掉 git 的行尾转换（spec §7.3.0）。
//!
//! # 为什么混合仓库必须按字节读写
//!
//! 工作目录同时是 SVN 工作副本。`core.autocrlf=true`（多数 Windows 开发机的默认
//! 状态）下，git 每次往磁盘写文件都会把 LF 转成 CRLF，而没设 `svn:eol-style` 的
//! 文件在 SVN 里存的可能是 LF。于是 `git stash` 往返一次，那个文件对 svn 就变脏了
//! ——内容一个字符没改，整文件的行尾全翻了一遍。最坏的下游后果是交付时把一整批
//! 纯行尾变化提交到 SVN。
//!
//! # 为什么是这里，不是 `core.autocrlf=false`，也不是工作树的 `.gitattributes`
//!
//! - 工作树的 `.gitattributes` 会进工作目录，也就会被提交进**用户的 SVN 仓库**。
//!   spec §3 明写「工作目录里只多出 `.git` 一个条目」。
//! - `core.autocrlf=false` 挡不住工作树里的 `.gitattributes`。2026-09-21 实测：
//!   工作树放一个 `* text=auto`，`git check-attr text` 就从 `unset` 变回 `auto`，
//!   归一化又回来了。而 `$GIT_DIR/info/attributes` 的优先级压过工作树，掀不翻。
//!
//! 剩下 `.git/info/attributes` 这一条路，正好是 `svn_exclude.rs` 的邻居，
//! 写法也照抄它。

use crate::error::{GitError, Result};
use std::io::Write;

/// 生成文件的第一行。也是「这个 attributes 是不是我们写的」的唯一判据。
pub const GENERATED_HEADER: &str =
    "# 由 mygitclient 自动生成，请勿手工编辑——改动会在下次打开仓库时被覆盖。";

/// 关掉行尾转换的规则本体。
///
/// `-text` 的含义是「这个路径不参与行尾归一化」，**不是**「当成二进制」：
/// `git diff` 照常按文本显示。
pub const EOL_RULE: &str = "* -text";

/// 规则文本。只有一条规则，但仍然走生成函数——判据、注释和文件内容必须长在
/// 一处，否则改了一边忘了另一边时，`has_eol_rule` 会对着一个新格式的文件报 false。
pub fn generate_attributes() -> String {
    format!("{GENERATED_HEADER}\n{EOL_RULE}\n")
}

fn info_dir(repo_root: &str) -> std::path::PathBuf {
    std::path::Path::new(repo_root).join(".git").join("info")
}

/// 规则是否已经装好。
///
/// # 为什么读文件，而不是问 `git check-attr`
///
/// 这个函数回答的是「**我们的**规则装了没有」，用来判定旧仓库要不要迁移。
/// `git check-attr` 回答的是「这条路径最终的属性是什么」——用户自己在别处
/// 配出同样的效果也会返回 `unset`，那时不该判成「已迁移」，因为基线仍然是旧
/// 设置下录的。两个问题不一样，别拿后者答前者。
pub fn has_eol_rule(repo_root: &str) -> bool {
    match std::fs::read_to_string(info_dir(repo_root).join("attributes")) {
        Ok(s) => {
            s.lines().next() == Some(GENERATED_HEADER)
                && s.lines().any(|l| l.trim() == EOL_RULE)
        }
        Err(_) => false,
    }
}

/// 把规则写进 `<仓库根>/.git/info/attributes`，返回**写之前**的原始字节。
///
/// 返回值是给调用方回滚用的（Task 4 的迁移要在重建基线失败时还原原状）。
/// `None` 表示这个文件原本不存在。
///
/// 写入是原子的，理由同 `svn_exclude::write_exclude`：`File::create` 先截断再写，
/// 写到一半失败会留下半个规则文件，而半个规则比没有规则更难发现。
///
/// 用户原有的 attributes 不会被静默吃掉：首次接管时备份成 `attributes.bak`。
/// 「含真实规则」= 至少有一行既不是空行也不是 `#` 注释；备份已存在则不再覆盖。
pub fn write_attributes(repo_root: &str) -> Result<Option<Vec<u8>>> {
    let info = info_dir(repo_root);
    std::fs::create_dir_all(&info)?;
    let target = info.join("attributes");

    let previous = std::fs::read(&target).ok();

    if let Some(bytes) = previous.as_deref() {
        let existing = String::from_utf8_lossy(bytes);
        let ours = existing.lines().next() == Some(GENERATED_HEADER);
        let has_rules = existing
            .lines()
            .any(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'));
        let backup = info.join("attributes.bak");
        if !ours && has_rules && !backup.exists() {
            std::fs::write(&backup, bytes)?;
        }
    }

    // 临时文件必须与目标**同目录**：跨卷 rename 在 Windows 上报
    // ERROR_NOT_SAME_DEVICE，而 %TEMP% 完全可能在另一个盘上。
    let tmp = info.join("attributes.mygitclient.tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(generate_attributes().as_bytes())?;
        // rename 是原子的，但「新内容已经落盘」不是它保证的。
        f.sync_all()?;
    }
    std::fs::rename(&tmp, &target).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        GitError::new("io", format!("替换 .git/info/attributes 失败: {e}"))
    })?;
    Ok(previous)
}

/// 把 `.git/info/attributes` 还原成 `previous` 描述的样子。
///
/// `None` 表示原本不存在 —— 此时**删掉**文件，而不是留一个空文件：留空文件
/// 会让 `has_eol_rule` 继续报 false 但目录里多出一个来路不明的条目。
pub fn restore_attributes(repo_root: &str, previous: Option<&[u8]>) -> Result<()> {
    let target = info_dir(repo_root).join("attributes");
    match previous {
        Some(bytes) => {
            std::fs::write(&target, bytes)?;
        }
        None => {
            if target.exists() {
                std::fs::remove_file(&target)?;
            }
        }
    }
    Ok(())
}
```

在 `src-tauri/src/engine/mod.rs` 里按字母序加一行：

```rust
pub mod svn_attributes;
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cd src-tauri && cargo test --test svn_attributes`

Expected: 7 passed。

- [ ] **Step 5: 变异验证（这一步不许跳）**

把 `EOL_RULE` 临时改成 `"* text=auto"`，重跑 `cargo test --test svn_attributes`。

Expected: `rule_turns_off_eol_conversion_for_every_path` 与 `rule_beats_a_gitattributes_committed_into_the_working_tree` 必须变红。改回来。

一条不变红就说明断言没有牙齿——`AGENTS.md` §13 rubric #9 把这类「虚假信心」列为阻断项。

- [ ] **Step 6: 门槛 + 提交**

Run: `cd src-tauri && cargo fmt --check && cargo clippy --all-targets && cargo test`

```bash
git add src-tauri/src/engine/svn_attributes.rs src-tauri/src/engine/mod.rs src-tauri/tests/svn_attributes.rs
git commit -m "feat(svn): .git/info/attributes 写 * -text，关掉混合仓库的 EOL 转换

- What: 新增 svn_attributes 模块，生成/原子写入/侦测/回滚 .git/info/attributes。
- Why: spec 7.3.0。autocrlf 下 git 每次写磁盘都会翻行尾，让文件对 svn 变脏。
- Impact/Test: cargo test --test svn_attributes 7 passed；含「工作树 .gitattributes
  盖不过它」这条——那是取本方案而非 core.autocrlf=false 的决定性理由。"
```

---

### Task 2: `init_hybrid` 建库时写规则（必须在录基线之前）

**Files:**
- Modify: `src-tauri/src/engine/svn_init.rs`（`build_repo` 里 `write_exclude` 那一行之后）
- Test: `src-tauri/tests/svn_init.rs`（追加）

**Interfaces:**
- Consumes: Task 1 的 `write_attributes`
- Produces: 无新公开签名。行为变化：此后新建库录出的基线 blob 与磁盘**逐字节相同**

- [ ] **Step 1: 写失败的测试**

先在 `src-tauri/tests/svn_init.rs` 的辅助函数区加三个 helper（文件里已有的 `baseline_blob` 返回 `String`，本组测试要比**字节**，不能复用）：

```rust
/// 基线树里某个路径的 blob **原始字节**。
///
/// 行尾是本组测试的断言对象，过一遍 UTF-8 转换就什么都测不出来了。
fn baseline_blob_bytes(repo_root: &str, path: &str) -> Vec<u8> {
    let spec = format!("svn/base:{path}");
    app_lib::engine::git_cli::run_git_bytes(repo_root, &["cat-file", "-p", &spec]).unwrap()
}

fn git_status_porcelain(repo_root: &str) -> String {
    app_lib::engine::git_cli::run_git(repo_root, &["status", "--porcelain"])
        .unwrap()
        .trim()
        .to_string()
}

fn svn_status_short(repo_root: &str) -> String {
    let out = std::process::Command::new("svn")
        .current_dir(repo_root)
        .args(["status", "--non-interactive"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}
```

文件顶部 import 补上：

```rust
use app_lib::engine::svn_attributes::has_eol_rule;
```

再追加三条测试：

```rust
#[test]
fn a_new_repo_records_the_baseline_byte_for_byte() {
    // spec §7.3.0：建库时就得装上规则，此后录的基线天然是对的。
    //
    // `svn:eol-style=native` 的文件在 Windows 上磁盘是 CRLF。装规则之前
    // `git add` 会把它归一成 LF 录进基线；装了之后录的必须是磁盘上那些字节。
    //
    // 这不是洁癖：基线与磁盘不同形，意味着任何一次 `git checkout svn/base`
    // 都会把磁盘的行尾翻一遍，那个文件对 svn 立刻变脏。
    let fx = svn_repo_with_native_eol();
    init_hybrid(&req(&fx)).unwrap();

    let disk = std::fs::read(fx.wc().join("Code/Nat.cs")).unwrap();
    assert!(
        disk.windows(2).any(|w| w == b"\r\n"),
        "前提不成立：native 的文件在 Windows 上该是 CRLF"
    );

    let blob = baseline_blob_bytes(&fx.wc_str(), "Code/Nat.cs");
    assert_eq!(blob, disk, "基线里的字节必须与磁盘完全一致");
}

#[test]
fn a_new_repo_has_the_eol_rule_installed() {
    let fx = svn_repo();
    init_hybrid(&req(&fx)).unwrap();
    assert!(
        has_eol_rule(&fx.wc_str()),
        "建库必须装上 EOL 规则，否则录出来的是旧形状的基线"
    );
}

#[test]
fn a_new_repo_is_clean_on_both_sides_right_after_init() {
    // 规则生效之后最容易砸的就是这条：blob 形状变了，索引对齐要是没跟上，
    // 用户一打开就看到满屏 M。git 与 svn 两侧都必须干净。
    let fx = svn_repo_with_native_eol();
    init_hybrid(&req(&fx)).unwrap();

    let git_dirty = git_status_porcelain(&fx.wc_str());
    assert_eq!(git_dirty, "", "建完库 git 工作区必须干净，实际：\n{git_dirty}");

    let svn_dirty = svn_status_short(&fx.wc_str());
    assert_eq!(svn_dirty, "", "建完库 svn 也必须干净，实际：\n{svn_dirty}");
}
```

- [ ] **Step 2: 跑测试确认它失败**

Run: `cd src-tauri && cargo test --test svn_init a_new_repo`

Expected: `a_new_repo_records_the_baseline_byte_for_byte` 失败（blob 是 LF、磁盘是 CRLF），`a_new_repo_has_the_eol_rule_installed` 失败。

- [ ] **Step 3: 写实现**

在 `src-tauri/src/engine/svn_init.rs` 的 `build_repo` 里，`write_exclude` 之后、`record_server_tree` **之前**插入：

```rust
    run_git(root, &["init", "-b", BASELINE_BRANCH])?;
    write_config(root, cfg)?;
    write_exclude(root, &generate_exclude(&cfg.whitelist, &cfg.extensions))?;
    // 必须在 record_server_tree 之前：规则决定 `git add` / `hash-object --path`
    // 读磁盘时跑不跑行尾过滤器，也就决定了第一个基线里 blob 长什么样。
    // 放到录制之后等于建库当场就造出一个需要迁移的旧形状基线（spec §7.3.0）。
    write_attributes(root)?;
```

顶部 import 补上：

```rust
use crate::engine::svn_attributes::write_attributes;
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cd src-tauri && cargo test --test svn_init`

Expected: 全绿。**特别确认 `recording_a_file_produces_the_same_blob_whether_it_was_dirty_or_not` 仍然通过**——它断言「干净路径 `git add`」与「脏路径 `hash-object --path`」产出同一个 blob。2026-09-21 实测这条在装了规则之后仍然成立（两条路都不再转换），但它是本次改动影响面最大的既有断言，必须亲眼看到它绿。

- [ ] **Step 5: 全量回归**

Run: `cd src-tauri && cargo test`

Expected: 537 passed 之上再加本次新增。**任何既有测试变红都必须查清原因再继续**，不要直接改断言迁就新行为。

- [ ] **Step 6: 门槛 + 提交**

Run: `cd src-tauri && cargo fmt --check && cargo clippy --all-targets`

```bash
git add src-tauri/src/engine/svn_init.rs src-tauri/tests/svn_init.rs
git commit -m "feat(svn): 建库时装上 EOL 规则，基线逐字节录磁盘内容

- What: build_repo 在 record_server_tree 之前 write_attributes。
- Why: 规则决定第一个基线里 blob 的形状；放到录制之后等于建库当场就造出
  一个需要迁移的旧基线（spec 7.3.0）。
- Impact/Test: cargo test 全绿；新增三条断言覆盖 native eol 的字节一致、
  规则已装、建完库 git/svn 两侧都干净。"
```

---

### Task 3: 侦测旧仓库（纯读，一个字节都不写）

**Files:**
- Modify: `src-tauri/src/engine/svn_init.rs`（新增 `EolStatus` 与 `eol_status`）
- Modify: `src-tauri/src/commands/svn.rs`（新增 `svn_eol_status`）
- Modify: `src-tauri/src/lib.rs`（注册命令）
- Test: `src-tauri/tests/svn_init.rs`（追加）

**Interfaces:**
- Consumes: Task 1 的 `has_eol_rule`；既有的 `detect_repo_kind` / `RepoKind::SvnHybrid`
- Produces:
  - `pub struct EolStatus { pub rule_installed: bool, pub needs_migration: bool }`（`serde(rename_all = "camelCase")`）
  - `pub fn eol_status(repo_root: &str) -> Result<EolStatus>`
  - Tauri 命令 `svn_eol_status(path: String) -> Result<EolStatus, GitError>`，JS 侧 `invoke('svn_eol_status', { path })`

- [ ] **Step 1: 写失败的测试**

追加到 `src-tauri/tests/svn_init.rs`：

```rust
#[test]
fn a_freshly_built_repo_needs_no_migration() {
    let fx = svn_repo();
    init_hybrid(&req(&fx)).unwrap();

    let st = eol_status(&fx.wc_str()).unwrap();
    assert!(st.rule_installed);
    assert!(!st.needs_migration, "刚建的库不该要求迁移");
}

#[test]
fn a_repo_built_before_the_rule_needs_migration() {
    // 模拟 Phase 1 建的库：建完把规则文件删掉，等价于那时根本没写过。
    let fx = svn_repo();
    init_hybrid(&req(&fx)).unwrap();
    std::fs::remove_file(fx.wc().join(".git/info/attributes")).unwrap();

    let st = eol_status(&fx.wc_str()).unwrap();
    assert!(!st.rule_installed);
    assert!(st.needs_migration, "旧形状的基线必须被指认出来");
}

#[test]
fn detection_writes_nothing() {
    // 判定必须是纯读。写了规则却没重建基线，仓库就停在 spec §7.3.0 说的
    // 那个潜伏状态上：git status 因为 stat cache 还报干净，等某个文件
    // 的 mtime 被碰一下才一个一个地无故变脏。
    let fx = svn_repo();
    init_hybrid(&req(&fx)).unwrap();
    let attrs = fx.wc().join(".git/info/attributes");
    std::fs::remove_file(&attrs).unwrap();
    let before = git_status_porcelain(&fx.wc_str());

    eol_status(&fx.wc_str()).unwrap();
    eol_status(&fx.wc_str()).unwrap();

    assert!(!attrs.exists(), "侦测不许写规则文件");
    assert_eq!(git_status_porcelain(&fx.wc_str()), before, "侦测不许改工作区");
}

#[test]
fn detection_refuses_on_a_directory_that_is_not_a_hybrid_repo() {
    let fx = svn_repo();
    let err = eol_status(&fx.wc_str()).expect_err("没建过库就没有基线可谈");
    assert_eq!(err.code, "svn_not_hybrid", "实际：{}", err.message);
}
```

- [ ] **Step 2: 跑测试确认它失败**

Run: `cd src-tauri && cargo test --test svn_init eol_status`

Expected: 编译失败，`cannot find function eol_status`。

- [ ] **Step 3: 写实现**

在 `src-tauri/src/engine/svn_init.rs` 里（放在 `RebuildReport` 附近，让基线相关的类型聚在一处）：

```rust
/// 行尾规则的安装状态（spec §7.3.0）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EolStatus {
    /// `.git/info/attributes` 里有没有我们写的那条规则。
    pub rule_installed: bool,
    /// 要不要走一次迁移。
    ///
    /// 等价于 `!rule_installed`——规则没装，说明这个仓库的基线是在旧的行尾
    /// 设置下录的。**判据只能是这个**：spec §7.3.0 实测，翻完设置之后
    /// `git status` 会因为 stat cache 假报干净，拿它当判据必然漏判。
    pub needs_migration: bool,
}

/// 读出行尾规则的安装状态。**纯读，一个字节都不写。**
pub fn eol_status(repo_root: &str) -> Result<EolStatus> {
    if detect_repo_kind(repo_root) != RepoKind::SvnHybrid {
        return Err(GitError::new(
            "svn_not_hybrid",
            "这个目录不是已初始化的混合仓库。",
        ));
    }
    let installed = has_eol_rule(repo_root);
    Ok(EolStatus {
        rule_installed: installed,
        needs_migration: !installed,
    })
}
```

import 改成：

```rust
use crate::engine::svn_attributes::{has_eol_rule, write_attributes};
```

在 `src-tauri/src/commands/svn.rs` 里照着邻居 `svn_check_baseline` 的形状加：

```rust
/// 行尾规则的安装状态（spec §7.3.0）。打开仓库时读一次。
#[tauri::command]
pub fn svn_eol_status(path: String) -> Result<EolStatus, GitError> {
    crate::engine::svn_init::eol_status(&path)
}
```

顶部 `use` 补上 `EolStatus`（与既有的 `RebuildReport` 同一行）。

在 `src-tauri/src/lib.rs` 的 `invoke_handler![...]` 列表里，紧挨着 `svn_check_baseline` 加：

```rust
            commands::svn::svn_eol_status,
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cd src-tauri && cargo test --test svn_init`

Expected: 全绿。

- [ ] **Step 5: 确认命令真的注册上了**

Run: `cd src-tauri && cargo test --test commands_are_async`

先读一遍 `src-tauri/tests/commands_are_async.rs`：若它按名单逐条核对命令表，新命令必须一并加进去；若它是反射式的，则无需改动。**读了再决定，不要猜。**

- [ ] **Step 6: 门槛 + 提交**

Run: `cd src-tauri && cargo fmt --check && cargo clippy --all-targets && cargo test`

```bash
git add src-tauri/src/engine/svn_init.rs src-tauri/src/commands/svn.rs src-tauri/src/lib.rs src-tauri/tests/svn_init.rs
git commit -m "feat(svn): 侦测旧仓库的行尾规则是否缺失

- What: 新增 EolStatus / eol_status 与 svn_eol_status 命令，纯读。
- Why: 旧仓库的基线是在 autocrlf 下录的，必须指认出来并引导迁移（spec 7.3.0）。
- Impact/Test: 四条断言，含「侦测不许写任何东西」——写了规则却没重建基线
  会让仓库停在潜伏故障状态。"
```

---

### Task 4: 迁移命令（写规则 + 重建基线，失败必须回滚规则）

**Files:**
- Modify: `src-tauri/src/engine/svn_init.rs`（新增 `migrate_eol`）
- Modify: `src-tauri/src/commands/svn.rs`（新增 `svn_migrate_eol`）
- Modify: `src-tauri/src/lib.rs`（注册命令）
- Test: `src-tauri/tests/svn_init.rs`（追加）

**Interfaces:**
- Consumes: Task 1 的 `write_attributes` / `restore_attributes` / `has_eol_rule`；既有的 `rebuild_baseline(repo_root: &str) -> Result<RebuildReport>`
- Produces:
  - `pub fn migrate_eol(repo_root: &str) -> Result<RebuildReport>`
  - Tauri 命令 `svn_migrate_eol(path: String) -> Result<RebuildReport, GitError>`，JS 侧 `invoke('svn_migrate_eol', { path })`

**为什么写规则和重建必须是一条命令：** 拆成两步的话，用户装完规则还没点重建的那段时间里，仓库正停在 spec §7.3.0 说的潜伏故障状态——基线是旧形状，工作树按新规则读，而 `git status` 因为 stat cache 暂时还报干净。那不是一个可以让用户停留的状态。

- [ ] **Step 1: 写失败的测试**

追加到 `src-tauri/tests/svn_init.rs`：

```rust
#[test]
fn migration_installs_the_rule_and_rerecords_the_baseline() {
    let fx = svn_repo_with_native_eol();
    init_hybrid(&req(&fx)).unwrap();
    // 退回成「Phase 1 建的库」：删规则 + 把基线按旧形状重录一遍
    std::fs::remove_file(fx.wc().join(".git/info/attributes")).unwrap();
    rebuild_baseline(&fx.wc_str()).unwrap();
    let stale = baseline_blob_bytes(&fx.wc_str(), "Code/Nat.cs");
    let disk = std::fs::read(fx.wc().join("Code/Nat.cs")).unwrap();
    assert_ne!(stale, disk, "前提不成立：旧形状的基线本该与磁盘不同");

    migrate_eol(&fx.wc_str()).unwrap();

    assert!(has_eol_rule(&fx.wc_str()), "迁移后规则必须装上");
    assert_eq!(
        baseline_blob_bytes(&fx.wc_str(), "Code/Nat.cs"),
        disk,
        "迁移后基线必须与磁盘逐字节一致"
    );
    assert_eq!(
        git_status_porcelain(&fx.wc_str()),
        "",
        "迁移后 git 工作区必须干净——这正是迁移要解决的问题"
    );
}

#[test]
fn migration_refuses_when_the_rule_is_already_installed() {
    // 重建会移动 svn/base 指针，旧基线只能从 reflog 找回来。已经迁移过还
    // 再来一次，等于凭空动一次用户的基线。
    let fx = svn_repo();
    init_hybrid(&req(&fx)).unwrap();

    let err = migrate_eol(&fx.wc_str()).expect_err("已经装过就不该再迁移");
    assert_eq!(err.code, "svn_eol_already_migrated", "实际：{}", err.message);
}

#[test]
fn a_failed_rebuild_rolls_the_rule_back() {
    // **本 Task 最重要的一条。** 重建失败时仓库必须回到「规则没装」的原状，
    // 不能停在「规则装了但基线还是旧的」——那是 spec §7.3.0 说的潜伏故障：
    // git status 因为 stat cache 还报干净，等文件 mtime 被碰一下才逐个变脏。
    //
    // 用 obstructed（`~`）制造重建失败：§16.1 之后那是 R1 唯一仍然阻断的脏状态。
    let fx = svn_repo();
    init_hybrid(&req(&fx)).unwrap();
    std::fs::remove_file(fx.wc().join(".git/info/attributes")).unwrap();

    let obstructed = fx.wc().join("Code/A.cs");
    std::fs::remove_file(&obstructed).unwrap();
    std::fs::create_dir(&obstructed).unwrap();

    let err = migrate_eol(&fx.wc_str()).expect_err("obstructed 必须让迁移失败");
    assert_eq!(err.code, "svn_preflight_obstructed", "实际：{}", err.message);
    assert!(
        !has_eol_rule(&fx.wc_str()),
        "重建失败了，规则必须还原——否则仓库停在潜伏故障状态"
    );
    assert!(obstructed.is_dir(), "拦住之后磁盘必须原样，不许替用户动手");
}

#[test]
fn a_failed_rebuild_restores_a_users_own_attributes_file() {
    // 用户原本就有自己的 attributes 时，回滚要还回**他的**内容，不是删文件。
    let fx = svn_repo();
    init_hybrid(&req(&fx)).unwrap();
    let attrs = fx.wc().join(".git/info/attributes");
    std::fs::write(&attrs, "*.psd binary\n").unwrap();

    let obstructed = fx.wc().join("Code/A.cs");
    std::fs::remove_file(&obstructed).unwrap();
    std::fs::create_dir(&obstructed).unwrap();

    migrate_eol(&fx.wc_str()).expect_err("obstructed 必须让迁移失败");

    assert_eq!(
        std::fs::read_to_string(&attrs).unwrap(),
        "*.psd binary\n",
        "回滚必须还回用户自己的内容"
    );
}

#[test]
fn migration_refuses_on_a_directory_that_is_not_a_hybrid_repo() {
    let fx = svn_repo();
    let err = migrate_eol(&fx.wc_str()).expect_err("没建过库就没有基线可迁移");
    assert_eq!(err.code, "svn_not_hybrid", "实际：{}", err.message);
}
```

- [ ] **Step 2: 跑测试确认它失败**

Run: `cd src-tauri && cargo test --test svn_init migration`

Expected: 编译失败，`cannot find function migrate_eol`。

- [ ] **Step 3: 写实现**

在 `src-tauri/src/engine/svn_init.rs` 里，紧挨 `rebuild_baseline` 放：

```rust
/// 行尾迁移：装上规则 + 按新规则重录基线（spec §7.3.0「迁移」）。
///
/// # 为什么是一条命令，不是两步
///
/// 规则决定 git 读磁盘时跑不跑行尾过滤器，基线 blob 的形状则是**录制时**定下的。
/// 只装规则不重建，仓库就停在「基线是旧形状、工作树按新规则读」的状态上，而
/// `git status` 因为 stat cache 暂时还报干净 —— 等某个文件的 mtime 被碰一下
/// （编辑一次、svn update 一次）它才单独冒出来。用户看到的是「文件一个一个地
/// 无故变脏」，比一次性报错难查得多。那不是一个可以让用户停留的状态。
///
/// # 失败时必须把规则还原
///
/// 同理：重建没成，规则就不能留着。回滚用 [`write_attributes`] 返回的原始字节，
/// 用户自己原有的 attributes 也能逐字节还回去。
pub fn migrate_eol(repo_root: &str) -> Result<RebuildReport> {
    if detect_repo_kind(repo_root) != RepoKind::SvnHybrid {
        return Err(GitError::new(
            "svn_not_hybrid",
            "这个目录不是已初始化的混合仓库，没有基线可迁移。",
        ));
    }
    if has_eol_rule(repo_root) {
        return Err(GitError::new(
            "svn_eol_already_migrated",
            "行尾规则已经装好了，不需要再迁移一次。\
             重建会移动基线指针，没有理由时不该动它。",
        ));
    }

    let previous = write_attributes(repo_root)?;
    match rebuild_baseline(repo_root) {
        Ok(report) => Ok(report),
        Err(e) => {
            // 回滚失败也不能把它盖掉原来那个错：用户要先知道重建为什么没成。
            // 两个都失败时把回滚的情况附在消息里，否则仓库可能真的停在
            // 「规则装了但基线是旧的」状态上而无人知晓。
            match restore_attributes(repo_root, previous.as_deref()) {
                Ok(()) => Err(e),
                Err(re) => Err(GitError::new(
                    e.code,
                    format!(
                        "{}\n\n另外：还原 .git/info/attributes 也失败了（{}）。\
                         这个仓库的基线现在是旧形状、而行尾规则已生效，\
                         请手工删掉 .git/info/attributes 后重试。",
                        e.message, re.message
                    ),
                )),
            }
        }
    }
}
```

import 改成：

```rust
use crate::engine::svn_attributes::{has_eol_rule, restore_attributes, write_attributes};
```

在 `src-tauri/src/commands/svn.rs` 加：

```rust
/// 走一次行尾迁移：装规则 + 重录基线（spec §7.3.0）。
///
/// **前端必须先过确认框**：这会移动 `svn/base` 指针，与 `svn_rebuild_baseline`
/// 同等分量。
#[tauri::command]
pub fn svn_migrate_eol(path: String) -> Result<RebuildReport, GitError> {
    crate::engine::svn_init::migrate_eol(&path)
}
```

在 `src-tauri/src/lib.rs` 的 `invoke_handler![...]` 里加：

```rust
            commands::svn::svn_migrate_eol,
```

- [ ] **Step 4: 跑测试确认通过**

Run: `cd src-tauri && cargo test --test svn_init`

Expected: 全绿。

- [ ] **Step 5: 变异验证（不许跳）**

把 `migrate_eol` 里那个 `match restore_attributes(...)` 整段临时换成直接 `Err(e)`（即不回滚），重跑 `cargo test --test svn_init`。

Expected: `a_failed_rebuild_rolls_the_rule_back` 与 `a_failed_rebuild_restores_a_users_own_attributes_file` 必须变红。改回来。

- [ ] **Step 6: 门槛 + 提交**

Run: `cd src-tauri && cargo fmt --check && cargo clippy --all-targets && cargo test`

```bash
git add src-tauri/src/engine/svn_init.rs src-tauri/src/commands/svn.rs src-tauri/src/lib.rs src-tauri/tests/svn_init.rs
git commit -m "feat(svn): 行尾迁移做成原子命令，重建失败则回滚规则

- What: migrate_eol = 写规则 + rebuild_baseline，失败逐字节还原 attributes。
- Why: 只装规则不重建，仓库会停在潜伏故障状态——stat cache 让 git status
  假报干净，等文件 mtime 被碰才逐个变脏（spec 7.3.0）。
- Impact/Test: 五条断言，含两条回滚路径；变异验证已跑（去掉回滚必变红）。"
```

---

### Task 5: 前端接线 —— 侦测、横幅、确认框

**Files:**
- Modify: `src/engine/svnSync.ts`
- Modify: `src/store/repoStore.ts`
- Modify: `src/components/BaselineBanner.tsx`
- Modify: `src/components/RepoTab.tsx`
- Test: `tests/vitest/svnSync.test.ts`（追加）

**Interfaces:**
- Consumes: Task 3 / Task 4 的两条命令
- Produces:
  - `export interface EolStatus { ruleInstalled: boolean; needsMigration: boolean }`
  - `export function describeEolMigration(eol: EolStatus | null, sync: SyncStatus | null): BannerModel | null`
  - `export function syncBannerIsSuppressed(eol: EolStatus | null, sync: SyncStatus | null): boolean`
  - `BannerAction['id']` 联合类型追加 `'migrateEol'`
  - store：`svnEolStatus(path: string): Promise<EolStatus>`、`svnMigrateEol(path: string): Promise<RebuildReport>`、state 上的 `svnEol: EolStatus | null`

**横幅优先级规则（本 Task 的核心判断）：** 需要迁移时，迁移横幅**取代**失同步横幅——两者的出口都是重建基线，挂两条红条只是噪音。**唯一的例外是 `pinnedReset`**：那种状态下重建基线是明确错误的（会把被钉死的旧内容录成标着新 revision 的基线，自己造出 §9.2 的基线撒谎），必须先让用户把那个目录更新回来。所以 `pinnedReset` 时迁移横幅让位。

- [ ] **Step 1: 写失败的测试**

追加到 `tests/vitest/svnSync.test.ts`（顶部 import 补上 `describeEolMigration`、`syncBannerIsSuppressed`、`EolStatus`）：

```ts
describe('describeEolMigration', () => {
  const needs: EolStatus = { ruleInstalled: false, needsMigration: true }
  const done: EolStatus = { ruleInstalled: true, needsMigration: false }
  const inSync: SyncStatus = { baselineRev: 3, nodeRevs: [3], verdict: { kind: 'inSync' } }

  it('还没读到状态时不挂任何东西', () => {
    expect(describeEolMigration(null, inSync)).toBeNull()
  })

  it('规则已装就不挂', () => {
    expect(describeEolMigration(done, inSync)).toBeNull()
  })

  it('需要迁移时给出重建入口', () => {
    const b = describeEolMigration(needs, inSync)
    expect(b).not.toBeNull()
    expect(b!.tone).toBe('danger')
    expect(b!.actions.map((a) => a.id)).toEqual(['migrateEol'])
  })

  it('revision 一致也照样挂——判据是规则在不在，不是同步与否', () => {
    // spec §7.3.0：翻完设置后 git status 会因为 stat cache 假报干净。
    // 拿任何「看起来没事」当判据都会漏判。
    expect(describeEolMigration(needs, inSync)).not.toBeNull()
  })

  it('pinnedReset 时让位——那种状态下重建基线是错的', () => {
    // 被钉死的旧内容录成标着新 revision 的基线 = 自己造出基线撒谎（§6.6.2）。
    const pinned: SyncStatus = {
      baselineRev: 9,
      nodeRevs: [4],
      verdict: { kind: 'pinnedReset', nodeRev: 4, baselineRev: 9, external: 'Code' },
    }
    expect(describeEolMigration(needs, pinned)).toBeNull()
  })

  it('其余失同步状态下迁移横幅仍然出现', () => {
    const diverged: SyncStatus = {
      baselineRev: 3,
      nodeRevs: [5],
      verdict: { kind: 'diverged', nodeRev: 5, baselineRev: 3 },
    }
    expect(describeEolMigration(needs, diverged)).not.toBeNull()
  })
})

describe('迁移横幅与失同步横幅互斥', () => {
  const needs: EolStatus = { ruleInstalled: false, needsMigration: true }
  const done: EolStatus = { ruleInstalled: true, needsMigration: false }
  const diverged: SyncStatus = {
    baselineRev: 3,
    nodeRevs: [5],
    verdict: { kind: 'diverged', nodeRev: 5, baselineRev: 3 },
  }

  it('需要迁移时失同步横幅让位，避免两条红条', () => {
    expect(syncBannerIsSuppressed(needs, diverged)).toBe(true)
  })

  it('不需要迁移时失同步横幅照常', () => {
    expect(syncBannerIsSuppressed(done, diverged)).toBe(false)
  })

  it('迁移未完成时不挂常驻「体检基线」——基线本来就不可信', () => {
    const inSync: SyncStatus = { baselineRev: 3, nodeRevs: [3], verdict: { kind: 'inSync' } }
    expect(needsStandaloneCheck(true, inSync, needs)).toBe(false)
    expect(needsStandaloneCheck(true, inSync, done)).toBe(true)
  })
})
```

- [ ] **Step 2: 跑测试确认它失败**

Run: `pnpm test`

Expected: `svnSync.test.ts` 报 `describeEolMigration is not a function`。

- [ ] **Step 3: 写实现**

在 `src/engine/svnSync.ts` 里加类型：

```ts
/** 行尾规则的安装状态（spec §7.3.0），对应 Rust 的 `EolStatus`。 */
export interface EolStatus {
  ruleInstalled: boolean
  needsMigration: boolean
}
```

`BannerAction` 的 `id` 联合追加 `'migrateEol'`：

```ts
export interface BannerAction {
  id: 'rebuild' | 'check' | 'updateWhitelistDir' | 'migrateEol'
  label: string
}
```

新增两个函数：

```ts
const MIGRATE_EOL_NOTE =
  '这次重建同样不会修改工作目录里的任何文件，也不会推进你的工作分支。' +
  '不迁移的话，文件会在被编辑或 update 之后一个一个地无故显示成「已修改」。'

/**
 * 行尾迁移横幅。规则没装 ⇒ 这个仓库的基线是在旧的行尾设置下录的。
 *
 * # 为什么判据是「规则在不在」，不是「看起来有没有问题」
 *
 * spec §7.3.0 实测：规则生效之后 `git status` 会因为 git 的 stat cache 假报
 * 0 个脏文件，要等某个文件的 mtime 被碰一下才单独冒出来。任何基于「现在看着
 * 没事」的判据都会漏判，而漏判的表现正是用户最难自查的那种——文件逐个变脏。
 *
 * # 为什么 `pinnedReset` 时让位
 *
 * 迁移的动作是重建基线，而 `pinnedReset` 下重建是**明确错误**的：会把被
 * `svn:externals` 钉死的旧内容录成一个标着新 revision 的基线，自己造出 §9.2
 * 的基线撒谎。那种状态必须先把目录更新回来，迁移排在它后面。
 */
export function describeEolMigration(
  eol: EolStatus | null,
  sync: SyncStatus | null,
): BannerModel | null {
  if (!eol || !eol.needsMigration) return null
  if (sync?.verdict.kind === 'pinnedReset') return null
  return {
    tone: 'danger',
    title: '基线是在旧的行尾设置下录的',
    detail:
      '这个仓库建库时还没有关掉 git 的行尾转换，基线里记的内容与磁盘上的字节对不上。' +
      '现在重建一次基线就能对齐，之后不会再出现这个问题。',
    actions: [{ id: 'migrateEol', label: '重建基线…' }],
    note: MIGRATE_EOL_NOTE,
  }
}

/**
 * 失同步横幅要不要让位给迁移横幅。
 *
 * 两者的出口都是重建基线，同时挂两条红条只是噪音。`pinnedReset` 不在此列
 * ——那时 `describeEolMigration` 自己返回 `null`，失同步横幅照常渲染。
 */
export function syncBannerIsSuppressed(
  eol: EolStatus | null,
  sync: SyncStatus | null,
): boolean {
  return describeEolMigration(eol, sync) !== null
}
```

`needsStandaloneCheck` 也要让位——迁移未完成时基线本来就不可信，挂一个「体检基线」只会给出误导性的结论：

```ts
export function needsStandaloneCheck(
  isSvn: boolean,
  sync: SyncStatus | null,
  eol: EolStatus | null,
): boolean {
  if (describeEolMigration(eol, sync) !== null) return false
  return isSvn && describeSync(sync) === null
}
```

> ⚠️ **第三个参数不要给默认值。** 给了默认值，漏传的调用点会被编译器放过，
> 结果是迁移未完成时仍然挂出「体检基线」——那正是这条改动要防的。让 `tsc`
> 把每个调用点都逼出来。

在 `src/store/repoStore.ts` 里，照着 `svnRebuildBaseline` 的形状加：

```ts
export function svnEolStatus(path: string): Promise<EolStatus> {
  return invoke<EolStatus>('svn_eol_status', { path })
}

export function svnMigrateEol(path: string): Promise<RebuildReport> {
  return invoke<RebuildReport>('svn_migrate_eol', { path })
}
```

并在仓库状态里加 `svnEol: EolStatus | null`（初值 `null`），在已有的「打开仓库后读 SVN 状态」那条路径里顺带读一次，配一个 `refreshSvnEol(path)`。读失败时置 `null` 并走既有的错误上报，**不要**把读失败当成「不需要迁移」。跨仓库竞态按 `refreshSvnAudit` 既有的 `samePath` 守卫同样处理。

在 `src/components/BaselineBanner.tsx` 里加 `eol: EolStatus | null` 与 `onMigrateEol: () => void` 两个 prop，渲染顺序为：迁移横幅 → （未被让位时的）失同步横幅 → audit。失同步横幅的渲染条件包上 `!syncBannerIsSuppressed(eol, sync)`。

在 `src/components/RepoTab.tsx` 里，照着 `handleRebuildBaseline` 的形状加：

```ts
  /**
   * 走一次行尾迁移（spec §7.3.0）。
   *
   * **必须先过确认框**：和重建基线同等分量——它就是一次重建，会移动
   * `svn/base` 指针，旧基线只能从 reflog 找回来。
   */
  const handleMigrateEol = async () => {
    await confirm({
      title: '重建基线以对齐行尾',
      message:
        '这个仓库建库时还没有关掉 git 的行尾转换，基线里记的内容与磁盘上的字节对不上。',
      warning:
        '不会修改工作目录里的任何文件，也不会推进你的工作分支。' +
        '不做的话，文件会在被编辑或 update 之后一个一个地无故显示成「已修改」。',
      confirmLabel: '重建',
      busyLabel: '正在重建…',
      action: async () => {
        await svnMigrateEol(repoPath)
        await refreshSvnEol(repoPath)
        await refreshSvnSync(repoPath)
        void fetchSvnLog(repoPath)
        void refreshSvnAudit(repoPath)
      },
    })
  }
```

并把 `eol={state.svnEol}` 和 `onMigrateEol={() => void handleMigrateEol()}` 接到 `<BaselineBanner>` 上。

- [ ] **Step 4: 跑测试确认通过**

Run: `pnpm test`

Expected: 既有 354 条之上加本次新增，全绿。

- [ ] **Step 5: 类型检查与构建**

Run: `pnpm build`

Expected: `tsc` + `vite build` 均通过。`needsStandaloneCheck` 多了一个必填参数，所有调用点都得显式传 `eol`——编译器逼出来的每一处都要**想一遍该传什么**，不要图快传 `null`。

- [ ] **Step 6: 变异验证（不许跳）**

把 `describeEolMigration` 里 `if (sync?.verdict.kind === 'pinnedReset') return null` 这一行临时删掉，重跑 `pnpm test`。

Expected: `pinnedReset 时让位——那种状态下重建基线是错的` 必须变红。改回来。

- [ ] **Step 7: 真实窗口验证（Tauri 命令签名改动的硬要求）**

`AGENTS.md` §7.3：`snake_case` → `camelCase` 这层转换没有编译期检查，写错只会在运行时静默失败。本计划新增两条命令，必须实测。

Run: `pnpm tauri dev`

在真实窗口里：

1. 用 `pwsh -File scripts/e2e-svn-fixture.ps1` 造一个临时 SVN fixture，在应用里走完初始化向导 → 确认**没有**迁移横幅（新建库天然带规则）。
2. 关掉应用，手工删掉该 fixture 的 `.git/info/attributes`，重新打开仓库 → 确认**出现**迁移横幅，且失同步横幅与常驻「体检基线」都让了位。
3. 点「重建基线…」→ 过确认框 → 确认横幅消失，且在 fixture 目录里跑 `git status --porcelain` 与 `svn status` 两侧都为空。
4. 全程看 devtools console 有没有 `invoke` 参数报错。

看到的与预期不符**必须停下查清**，不要改测试迁就。

- [ ] **Step 8: 门槛 + 提交**

Run: `pnpm build && pnpm test`

```bash
git add src/engine/svnSync.ts src/store/repoStore.ts src/components/BaselineBanner.tsx src/components/RepoTab.tsx tests/vitest/svnSync.test.ts
git commit -m "feat(svn): 旧仓库打开时提示行尾迁移，复用重建基线的确认流程

- What: describeEolMigration + 横幅优先级 + 两条 invoke 包装 + 确认框接线。
- Why: 旧基线是在 autocrlf 下录的，不迁移的话文件会在 mtime 被碰之后
  一个一个地无故变脏（spec 7.3.0）。
- Impact/Test: pnpm test 全绿；变异验证已跑；pnpm tauri dev 真实窗口
  走通「建库无横幅 → 删规则重开有横幅 → 重建后两侧干净」。"
```

---

## 收尾

- [ ] **决策日志**

`AGENTS.md` §2.1 要求：执行 `docs/superpowers/plans/` 下的计划必须配套
`docs/superpowers/plans/2026-09-22-svn-phase2a-eol-normalization-decisions.md`，
**在动第一个 Task 之前建好**，每完成一个 Task 立即更新进度总账。

决策条目至少覆盖四类，缺一类视为记录不完整：

| 类型 | 这一期特别要记什么 |
|---|---|
| 偏离计划 | 实际做法与本文不一致的地方**和原因** |
| 计划的错误 | 本文写错的代码、命令、断言（比如 `commands_are_async.rs` 到底要不要改） |
| 计划没写的坑 | 实现时才暴露的 git / Tauri 真实行为 |
| 为什么这么改是安全的 | 尤其是 Task 4 的回滚路径，和「迁移必须是原子的」这条论证 |

- [ ] **E2E 清单**

新建 `docs/superpowers/e2e/2026-09-22-svn-phase2a-eol-e2e.md`，把 Task 5 Step 7 那四步写成照着能做的完整清单（造 fixture 的命令、点哪里、应该看到什么、看到别的说明什么），沿用 `2026-09-18-svn-baseline-from-server-e2e.md` 的写法。**没跑的别勾。**

- [ ] **独立 review**

`AGENTS.md` §4.1：合并前必须由**独立于实现者**的人或 agent 完成一次 review，按 §13 rubric 执行。用 `/code-review`、独立 review subagent 或 Codex 二次意见均可。高危问题合并前必须修完。

本期 review 要特别盯的三条：

1. **Task 4 的回滚路径**：重建失败后 `.git/info/attributes` 是否真的回到原状；回滚本身也失败时，错误消息有没有说清仓库停在哪。
2. **`needsStandaloneCheck` 的新参数**：既有调用点是不是都想过该传什么，还是图快传了 `null`——后者会让迁移未完成时仍然挂出「体检基线」，给出误导性结论。
3. **测试的输入是不是实际场景的形态**（§13 rubric #9）：行尾相关的断言必须比**字节**，任何一处过了 UTF-8 转换或 `trim()` 的比较都等于没测。

---

## Self-Review

**Spec 覆盖：** §7.3.0 的四个部分——问题（Task 2 的字节断言覆盖）、定案取 C（Task 1 的优先级测试覆盖，那是取 C 不取 B 的决定性理由）、对 R2b 无副作用（Task 2 Step 4 明确要求确认既有的 `recording_a_file_produces_the_same_blob_whether_it_was_dirty_or_not` 仍绿）、迁移（Task 3 侦测 + Task 4 原子迁移 + 回滚 + Task 5 的入口）。§3 架构图已在 2026-09-21 的 spec 提交里改过，本计划不重复。

**有意不覆盖的：** spec §7.3.0 末尾说「Phase 2 的 plan 必须把这条迁移列成一个独立 Task」——本计划拆成了 Task 3（侦测）+ Task 4（迁移），比「一个 Task」更细，方向一致。`sync_in` 的 S1/S3/S10 本身不在本计划范围内，属 Plan 2b。

**类型一致性：** `EolStatus` 在 Rust 侧是 `rule_installed` / `needs_migration`，序列化成 `ruleInstalled` / `needsMigration`（`serde(rename_all = "camelCase")`），TS 侧接口按后者写；两条命令的参数都只有 `path`，JS 侧一律 `{ path }`。`BannerAction['id']` 新增的 `'migrateEol'` 与 `describeEolMigration` 产出的 action id 一致。`migrate_eol` 与 `rebuild_baseline` 同样返回 `RebuildReport`，前端复用既有类型。`has_eol_rule` / `write_attributes` / `restore_attributes` 三个签名在 Task 1 定义，Task 2/3/4 的用法与之一致。

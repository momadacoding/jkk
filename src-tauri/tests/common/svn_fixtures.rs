//! SVN fixture 的共用构造。
//!
//! 全部在 `TempDir` 里自建 `svnadmin create` 仓库，跑完即删——禁止依赖本仓库、
//! 用户的仓库或任何仓库外的固定路径（`AGENTS.md` §8 与开篇的最高优先级约束）。
//!
//! 这些 fixture 需要 `svn` 与 `svnadmin` 在 PATH 上（spec §17）。**故意不做
//! 「没装就 skip」**：静默跳过等于把「没测」显示成绿色。

#![allow(dead_code)] // 不同测试文件用到的子集不同，未用到的构造不算缺陷

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

pub struct SvnFixture {
    pub dir: TempDir,
}

fn run(dir: &Path, program: &str, args: &[&str]) {
    let out = Command::new(program)
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("{program} 起不来：{e}（spec §17：svn 必须在 PATH 上）"));
    assert!(
        out.status.success(),
        "{program} {args:?} 失败：{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Windows 上 `file://` URL 是 `file:///C:/path`（三斜杠 + 盘符 + 正斜杠）。
/// 这是已知跨平台坑，不能靠路径拼接碰运气（spec §14.1）。
pub fn file_url(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "/");
    if s.starts_with('/') {
        format!("file://{s}")
    } else {
        format!("file:///{s}")
    }
}

impl SvnFixture {
    pub fn path(&self) -> &Path {
        self.dir.path()
    }
    pub fn repo(&self) -> PathBuf {
        self.dir.path().join("repo")
    }
    pub fn wc(&self) -> PathBuf {
        self.dir.path().join("wc1")
    }
    pub fn wc_str(&self) -> String {
        self.dir.path().join("wc1").to_string_lossy().to_string()
    }
}

/// 一个仓库 + 一个 checkout，`Code/` 下有两个 `.cs`。
pub fn svn_repo() -> SvnFixture {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    run(&root, "svnadmin", &["create", "repo"]);
    let url = file_url(&root.join("repo"));
    run(
        &root,
        "svn",
        &["checkout", &url, "wc1", "--non-interactive"],
    );
    let wc = root.join("wc1");
    std::fs::create_dir_all(wc.join("Code")).unwrap();
    std::fs::write(wc.join("Code/A.cs"), "a\n").unwrap();
    std::fs::write(wc.join("Code/B.cs"), "b\n").unwrap();
    run(&wc, "svn", &["add", "Code", "--non-interactive"]);
    run(&wc, "svn", &["commit", "-m", "r1", "--non-interactive"]);
    run(&wc, "svn", &["update", "--non-interactive"]);
    SvnFixture { dir }
}

/// 在 [`svn_repo`] 的基础上加一个**名字里带 `@`** 的子目录。
///
/// SVN 把 target 里**最后一个** `@` 之后的部分当 peg revision 解析，`--` 挡不住
/// （2026-09-14 实测：`svn info -- Code/Team@Home` → `E205000: Syntax error
/// parsing peg revision 'Home'`；同一个 target 送给 `svn status` → `E200009:
/// a peg revision is not allowed here`）。追加一个空的 peg 后缀 `@` 两者都通过。
pub fn svn_repo_with_at_sign_in_path() -> SvnFixture {
    let fx = svn_repo();
    let wc = fx.wc();
    std::fs::create_dir_all(wc.join("Code/Team@Home")).unwrap();
    std::fs::write(wc.join("Code/Team@Home/C.cs"), "c\n").unwrap();
    // fixture 自己的 `svn add` 也要过同一道坎，所以这里同样带尾缀
    run(&wc, "svn", &["add", "Code/Team@Home@", "--non-interactive"]);
    run(&wc, "svn", &["commit", "-m", "r2", "--non-interactive"]);
    run(&wc, "svn", &["update", "--non-interactive"]);
    fx
}

/// 在 [`svn_repo`] 的基础上，往 `Code/` 里挂一条指向**另一个仓库**的 external。
///
/// 这是 spec §6.5.5 的场景：只查白名单根会拿到 repo1 的 UUID，P4 干净通过，
/// 而一个另一个仓库的子树已经进了白名单——它会被 git 跟踪、进基线、进交付清单，
/// 却从不参与 `nodeRevs` 校验，最后在 D5 撞上 E200007。
pub fn svn_repo_with_cross_repo_nested_external() -> SvnFixture {
    let fx = svn_repo();
    let root = fx.path().to_path_buf();
    run(&root, "svnadmin", &["create", "repo2"]);
    let url2 = file_url(&root.join("repo2"));
    run(
        &root,
        "svn",
        &["checkout", &url2, "wcv", "--non-interactive"],
    );
    let wcv = root.join("wcv");
    std::fs::write(wcv.join("V.cs"), "v\n").unwrap();
    run(&wcv, "svn", &["add", "V.cs", "--non-interactive"]);
    run(
        &wcv,
        "svn",
        &["commit", "-m", "vendor r1", "--non-interactive"],
    );

    let wc = fx.wc();
    let externals = format!("{url2} Vendor");
    run(
        &wc,
        "svn",
        &[
            "propset",
            "svn:externals",
            &externals,
            "Code",
            "--non-interactive",
        ],
    );
    run(
        &wc,
        "svn",
        &["commit", "-m", "nested external", "--non-interactive"],
    );
    run(&wc, "svn", &["update", "--non-interactive"]);
    fx
}

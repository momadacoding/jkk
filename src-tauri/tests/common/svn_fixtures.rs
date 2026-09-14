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

/// 允许失败的 svn 调用，返回是否成功。构造「操作半路失败」的 fixture 要用。
fn try_svn(dir: &Path, args: &[&str]) -> bool {
    Command::new("svn")
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("svn 起不来：{e}"))
        .status
        .success()
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

// ---------------------------------------------------------------------------
// Phase 1b：初始化向导的前置校验（P1 / P2 / P3）要拦的各种形态
// ---------------------------------------------------------------------------

/// P1 要拦住的四种脏。逐个造出来，而不是只造一个 `M` 就宣称「覆盖了 P1」
/// （`AGENTS.md` §13 rubric #9）。
#[derive(Debug, Clone, Copy)]
pub enum DirtyKind {
    /// 内容被改过，没提交。
    Modified,
    /// 白名单目录里多出一个没纳入版本控制的 `.cs`。
    Unversioned,
    /// `svn add` 了但没提交。
    Added,
    /// `svn delete` 了但没提交。
    Deleted,
}

/// 在 [`svn_repo`] 的基础上制造一种脏状态。
pub fn svn_repo_dirty(kind: DirtyKind) -> SvnFixture {
    let fx = svn_repo();
    let wc = fx.wc();
    match kind {
        DirtyKind::Modified => {
            std::fs::write(wc.join("Code/A.cs"), "changed\n").unwrap();
        }
        DirtyKind::Unversioned => {
            std::fs::write(wc.join("Code/New.cs"), "n\n").unwrap();
        }
        DirtyKind::Added => {
            std::fs::write(wc.join("Code/Added.cs"), "x\n").unwrap();
            run(&wc, "svn", &["add", "Code/Added.cs", "--non-interactive"]);
        }
        DirtyKind::Deleted => {
            run(&wc, "svn", &["delete", "Code/B.cs", "--non-interactive"]);
        }
    }
    fx
}

/// `svn:ignore` 掉一个 `.cs`，再把那个文件造出来。
///
/// 这是 `--no-ignore` 存在的理由（spec §6.5）：不带它 `svn status` 根本不报这个
/// 文件，而它匹配白名单的扩展名、**git 照收**——录进基线就是一个 SVN 里不存在
/// 的文件，交付时必然对不上。
pub fn svn_repo_with_svn_ignored_cs() -> SvnFixture {
    let fx = svn_repo();
    let wc = fx.wc();
    run(
        &wc,
        "svn",
        &[
            "propset",
            "svn:ignore",
            "Generated.cs",
            "Code",
            "--non-interactive",
        ],
    );
    run(
        &wc,
        "svn",
        &["commit", "-m", "ignore Generated.cs", "--non-interactive"],
    );
    run(&wc, "svn", &["update", "--non-interactive"]);
    std::fs::write(wc.join("Code/Generated.cs"), "g\n").unwrap();
    fx
}

/// `svn:ignore` 掉一个**不匹配白名单扩展名**的文件，再把它造出来。
///
/// 和 [`svn_repo_with_svn_ignored_cs`] 成对，用来钉住「P1 不按扩展名放行」：
/// 这个文件 git 不会收，但它仍然是白名单目录里一个 SVN 不管的文件，
/// spec §6.5 的禁止清单里 `?` 是明写的，一样要拦。
pub fn svn_repo_with_svn_ignored_non_code_file() -> SvnFixture {
    let fx = svn_repo();
    let wc = fx.wc();
    run(
        &wc,
        "svn",
        &[
            "propset",
            "svn:ignore",
            "Notes.txt",
            "Code",
            "--non-interactive",
        ],
    );
    run(
        &wc,
        "svn",
        &["commit", "-m", "ignore Notes.txt", "--non-interactive"],
    );
    run(&wc, "svn", &["update", "--non-interactive"]);
    std::fs::write(wc.join("Code/Notes.txt"), "n\n").unwrap();
    fx
}

/// §6.8 基线体检用的仓库：白名单内既有 `.cs` 也有 `.psd`，白名单外还有美术资产。
///
/// r1 的内容：
///
/// ```text
/// Code/A.cs        ← 白名单内、匹配扩展名
/// Code/Art.psd     ← 白名单内、**不**匹配扩展名（T3b 的误报对照组）
/// Code/Deep/D.cs   ← 深层，服务器删掉它来覆盖 T7
/// Art/boss.fbx     ← 白名单外（T3 的误报对照组）
/// ```
pub fn svn_repo_for_health() -> SvnFixture {
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
    std::fs::create_dir_all(wc.join("Code/Deep")).unwrap();
    std::fs::create_dir_all(wc.join("Art")).unwrap();
    std::fs::write(wc.join("Code/A.cs"), "v1\n").unwrap();
    std::fs::write(wc.join("Code/Art.psd"), "p1\n").unwrap();
    std::fs::write(wc.join("Code/Deep/D.cs"), "d1\n").unwrap();
    std::fs::write(wc.join("Art/boss.fbx"), "f1\n").unwrap();
    run(&wc, "svn", &["add", "Code", "Art", "--non-interactive"]);
    run(&wc, "svn", &["commit", "-m", "r1", "--non-interactive"]);
    run(&wc, "svn", &["update", "--non-interactive"]);
    SvnFixture { dir }
}

/// [`svn_repo_for_health`] 再往前走一个 revision：服务器上改了 `Code/A.cs`、
/// **删掉了** `Code/Deep/D.cs`，工作副本已更新到 r2。
///
/// 拿它配 `baselineRev = 1` 就是 spec §6.8.2 的 T4（假基线）与 T7（服务器删文件）。
pub fn svn_repo_for_health_at_r2() -> SvnFixture {
    let fx = svn_repo_for_health();
    let wc = fx.wc();
    std::fs::write(wc.join("Code/A.cs"), "v2\n").unwrap();
    run(
        &wc,
        "svn",
        &["delete", "Code/Deep/D.cs", "--non-interactive"],
    );
    run(&wc, "svn", &["commit", "-m", "r2", "--non-interactive"]);
    run(&wc, "svn", &["update", "--non-interactive"]);
    fx
}

/// `Code/` 上挂五条 external，把 spec §6.5.2 的五种写法一次摆齐。
///
/// 一条跟随 HEAD（`follow`），四条用四种不同语法钉在 r1。
///
/// ⚠️ **属性值必须是一致的行尾。** 实测 E10：混合行尾会让 `svn propset` 报
/// `E135000: Inconsistent line ending style`，属性静默没设上，测试会拿到一个
/// 「这个仓库没有 externals」的假绿。Rust 侧 `std::fs::write` 写死 `\n` 即可。
pub fn svn_repo_with_five_external_forms() -> SvnFixture {
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
    std::fs::create_dir_all(wc.join("lib")).unwrap();
    std::fs::write(wc.join("Code/A.cs"), "a\n").unwrap();
    std::fs::write(wc.join("lib/L.txt"), "l\n").unwrap();
    run(&wc, "svn", &["add", "Code", "lib", "--non-interactive"]);
    run(&wc, "svn", &["commit", "-m", "r1", "--non-interactive"]);
    run(&wc, "svn", &["update", "--non-interactive"]);

    let ext = root.join("externals.txt");
    std::fs::write(
        &ext,
        "^/lib follow\n-r1 ^/lib pinA\n-r 1 ^/lib pinB\n^/lib@1 pinC\npinD -r1 ^/lib\n",
    )
    .unwrap();
    run(
        &wc,
        "svn",
        &[
            "propset",
            "svn:externals",
            "-F",
            ext.to_str().unwrap(),
            "Code",
            "--non-interactive",
        ],
    );
    run(
        &wc,
        "svn",
        &["commit", "-m", "five external forms", "--non-interactive"],
    );
    // 旧格式 + `^/` 相对 URL（`pinD -r1 ^/lib`）签出会失败，spec §6.5.2 记了这件事。
    // 那条定义本来就没法用，不该让整个 fixture 挂掉——检测走的是属性文本，不是磁盘。
    try_svn(&wc, &["update", "--non-interactive"]);
    SvnFixture { dir }
}

/// 一个 `Code/` 子树被 `svn switch` 到了另一条分支的工作副本。
///
/// # 为什么这个形态是必须覆盖的
///
/// 2026-09-14 独立 review 给的反例，实测**四道前置门槛一个都拦不住**：
/// `svn status` 对「target 就是被 switch 的那个目录」一条 entry 都不出（P1 过）、
/// `nodeRevs` 单值（P2 过）、工作副本干净（P3 过）、同一仓库（P4 过），
/// XML 里连 `switched` 属性都没有。
///
/// 而体检的 `--old=<工作副本根 URL>@N` 指向的是 trunk 的内容，磁盘上是分支的内容
/// ——一个**完全合法**的工作副本会被报成「基线撒谎」，按 spec §6.8.6 初始化时
/// 那会触发整个回退。
///
/// 布局：`trunk/Code/{A.cs,B.cs}`、`branches/feature`（从 trunk 拷），
/// 分支上把 `A.cs` 改成别的内容。工作副本签出 trunk，再把 `Code` switch 到分支。
pub fn svn_repo_with_switched_subtree() -> SvnFixture {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    run(&root, "svnadmin", &["create", "repo"]);
    let url = file_url(&root.join("repo"));

    let seed = root.join("seed");
    run(
        &root,
        "svn",
        &["checkout", &url, "seed", "--non-interactive"],
    );
    std::fs::create_dir_all(seed.join("trunk/Code")).unwrap();
    std::fs::create_dir_all(seed.join("branches")).unwrap();
    std::fs::write(seed.join("trunk/Code/A.cs"), "class A_trunk {}\n").unwrap();
    std::fs::write(seed.join("trunk/Code/B.cs"), "class B {}\n").unwrap();
    run(
        &seed,
        "svn",
        &["add", "trunk", "branches", "--non-interactive"],
    );
    run(&seed, "svn", &["commit", "-m", "r1", "--non-interactive"]);
    run(&seed, "svn", &["update", "--non-interactive"]);
    run(
        &seed,
        "svn",
        &[
            "copy",
            &format!("{url}/trunk"),
            &format!("{url}/branches/feature"),
            "-m",
            "r2 branch",
            "--non-interactive",
        ],
    );
    run(&seed, "svn", &["update", "--non-interactive"]);
    std::fs::write(
        seed.join("branches/feature/Code/A.cs"),
        "class A_FEATURE {}\n",
    )
    .unwrap();
    run(&seed, "svn", &["commit", "-m", "r3", "--non-interactive"]);

    run(
        &root,
        "svn",
        &[
            "checkout",
            &format!("{url}/trunk"),
            "wc1",
            "--non-interactive",
        ],
    );
    let wc = root.join("wc1");
    run(
        &wc,
        "svn",
        &[
            "switch",
            &format!("{url}/branches/feature/Code"),
            "Code",
            "--non-interactive",
        ],
    );
    run(&wc, "svn", &["update", "--non-interactive"]);
    SvnFixture { dir }
}

/// 建一个有三个 revision 的仓库，返回它的 `file://` URL。`Code/A.cs` 每次都改。
fn seed_three_revisions(root: &Path) -> String {
    run(root, "svnadmin", &["create", "repo"]);
    let url = file_url(&root.join("repo"));
    run(
        root,
        "svn",
        &["checkout", &url, "seed", "--non-interactive"],
    );
    let seed = root.join("seed");
    std::fs::create_dir_all(seed.join("Code")).unwrap();
    std::fs::write(seed.join("Code/A.cs"), "v1\n").unwrap();
    std::fs::write(seed.join("Code/B.cs"), "v1\n").unwrap();
    run(&seed, "svn", &["add", "Code", "--non-interactive"]);
    run(&seed, "svn", &["commit", "-m", "r1", "--non-interactive"]);
    std::fs::write(seed.join("Code/A.cs"), "v2\n").unwrap();
    run(&seed, "svn", &["commit", "-m", "r2", "--non-interactive"]);
    std::fs::write(seed.join("Code/A.cs"), "v3\n").unwrap();
    run(&seed, "svn", &["commit", "-m", "r3", "--non-interactive"]);
    url
}

/// **真正的 mixed-revision**：整体在 r3，`Code/A.cs` 被单独退回 r1。
///
/// 2026-09-14 实测：`svn info .` 的 revision = 3，`nodeRevs` = `{1, 3}`。
/// P2 必须拒绝。
pub fn svn_repo_mixed_revision() -> SvnFixture {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    let url = seed_three_revisions(&root);
    run(
        &root,
        "svn",
        &["checkout", &url, "wc1", "--non-interactive"],
    );
    let wc = root.join("wc1");
    run(
        &wc,
        "svn",
        &["update", "-r1", "--non-interactive", "--", "Code/A.cs@"],
    );
    SvnFixture { dir }
}

/// **rootRev 落后，但 nodeRevs 是单值**：根留在 r1，只把 `Code` 子树更到 r3。
///
/// 2026-09-14 实测：`svn info .` 的 revision = 1，`nodeRevs` = `{3}`
/// ——正是 spec §6.5.1 描述的那个形态。
///
/// P2 **必须放行**，而基线 trailer 的 rev 必须取 3。取成 `svn info .` 的 1，
/// 初始化**完成的那一瞬间**就判失同步。
pub fn svn_repo_root_rev_behind() -> SvnFixture {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    let url = seed_three_revisions(&root);
    run(
        &root,
        "svn",
        &["checkout", "-r1", &url, "wc1", "--non-interactive"],
    );
    let wc = root.join("wc1");
    run(&wc, "svn", &["update", "--non-interactive", "--", "Code@"]);
    SvnFixture { dir }
}

/// 一个 `svn update` 被打断在半路的工作副本。
///
/// # 怎么稳定地造出来
///
/// Windows 上用 `FileShare::None` 独占占住一个**待更新**的文件，`svn update`
/// 走到那一项时必然失败（2026-09-14 实测：`E155009: Failed to run the WC DB
/// work queue` + `E720005: Can't move … Access is denied`）。不依赖时序、
/// 不需要杀进程，每次都能复现。
///
/// 留下的痕迹（`svn status --depth=empty --xml` 打在工作副本根上）：
/// `wc-locked="true"` **和** `item="incomplete"` 两个都在。
#[cfg(windows)]
pub fn svn_repo_interrupted_update() -> SvnFixture {
    use std::os::windows::fs::OpenOptionsExt;

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
    for i in 1..=8 {
        std::fs::write(wc.join(format!("Code/F{i}.cs")), "v1\n").unwrap();
    }
    run(&wc, "svn", &["add", "Code", "--non-interactive"]);
    run(&wc, "svn", &["commit", "-m", "r1", "--non-interactive"]);
    run(&wc, "svn", &["update", "--non-interactive"]);

    // 另一个工作副本提交 r2，让 wc1 落后一个 revision，于是 update 真的有活干
    run(
        &root,
        "svn",
        &["checkout", &url, "wc2", "--non-interactive"],
    );
    let wc2 = root.join("wc2");
    for i in 1..=8 {
        std::fs::write(wc2.join(format!("Code/F{i}.cs")), "v2\n").unwrap();
    }
    run(&wc2, "svn", &["commit", "-m", "r2", "--non-interactive"]);

    {
        let _hold = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(wc.join("Code/F5.cs"))
            .expect("独占打开失败");
        assert!(
            !try_svn(&wc, &["update", "--non-interactive"]),
            "fixture 前提不成立：被独占占住的文件居然更新成功了"
        );
    }
    SvnFixture { dir }
}

/// [`svn_repo_interrupted_update`] 之后又跑了一次 `svn cleanup` 的工作副本。
///
/// # 为什么单独造这一个
///
/// 这是**最现实**的那个场景：update 失败，SVN 提示 "run cleanup"，用户照做了，
/// 以为好了。2026-09-14 实测，cleanup 之后：
///
/// | 观察点 | 结果 |
/// |---|---|
/// | `wc-locked` | **没了**（cleanup 就是干这个的） |
/// | 工作副本根的 `item` | **仍然是 `incomplete`** |
/// | 白名单根 `Code@` 的 status | **一条 entry 都没有，看起来完全干净** |
/// | `nodeRevs` | 单值，P2 放行 |
///
/// 也就是说 P1、P2 全绿，只有工作副本根还写着 incomplete。P3 若只查白名单根，
/// 就会在这个最该拦住的场景下放行。
#[cfg(windows)]
pub fn svn_repo_incomplete_after_cleanup() -> SvnFixture {
    let fx = svn_repo_interrupted_update();
    run(&fx.wc(), "svn", &["cleanup", "--non-interactive"]);
    fx
}

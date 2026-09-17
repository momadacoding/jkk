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

/// 在某个目录下跑一条 svn 命令，失败即 panic。
///
/// 给测试**构造外部变更**用——「有人用 TortoiseSVN 在外面提交 / update 了」
/// 是本期一大半场景的前提，而那一步按定义发生在本应用之外，只能由测试自己发。
///
/// ⚠️ 这不违反本期「零 SVN 写操作」的约束：那条约束的作用域是
/// `src-tauri/src/`（产品代码），不是测试。
pub fn svn_cmd(dir: &Path, args: &[&str]) {
    run(dir, "svn", args);
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

/// 同仓库嵌套 external 的三种形态，配 [`svn_repo_with_same_repo_nested_external`]。
///
/// 这三种必须成组出现。只造 [`Self::Dirty`] / [`Self::PinnedOlder`] 会让「P1/P2 把
/// 嵌套 external 也查一遍」看起来成立，却证明不了它**没有误拦**——而误拦砸的正是
/// 本设计的核心场景（代码就在 external 里，spec §6.6.5）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NestedExternal {
    /// 跟随 HEAD、内容干净。**必须放行。**
    Clean,
    /// 跟随 HEAD，但里面有一个未提交的改动。P1 必须拦住。
    Dirty,
    /// 用 `-r` 钉在旧 revision，与根的 revision 不一致。P2 必须拦住。
    PinnedOlder,
}

/// 在白名单根 `Code/` 底下挂一条指向**同一个仓库**的嵌套 external `Code/Vendor`。
///
/// # 为什么必须单独造这一个
///
/// 已有的 [`svn_repo_with_cross_repo_nested_external`] 是**跨**仓库的，P4 会拦下。
/// 同仓库那条 P4 按设计**放行**（spec §6.5.5：「同仓库的嵌套 external 不阻断」），
/// 于是它成了唯一能走完全程的形态——2026-09-15 实测，这条路上：
///
/// | 关卡 | 实测结果 |
/// |---|---|
/// | P4 | 同 UUID，**通过** |
/// | P1 `status --ignore-externals -- Code@` | 只报 `item="external"`，脏文件**看不见** |
/// | P2 `status -v --ignore-externals -- Code@` | 只有根的 revision，external 的**不在里面** |
/// | 体检 `diff --old=URL@N --new=. -- Code` | `<paths>` **空的**，diff 不下沉进 external |
/// | `git add -- Code` | `Code/Vendor/V.cs` **照收** |
///
/// 叠起来就是：一个未提交（或停在旧 revision）的子树被录进基线、标着根的 revision、
/// 体检报 `Verified`。这正是 §6.8 存在的理由被绕开的那种形态。
///
/// 布局：`Code/A.cs` + `vendor/V.cs`，`svn:externals` 挂在 `Code` 上。
pub fn svn_repo_with_same_repo_nested_external(kind: NestedExternal) -> SvnFixture {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    run(&root, "svnadmin", &["create", "repo"]);
    let url = file_url(&root.join("repo"));
    run(
        &root,
        "svn",
        &["checkout", &url, "seed", "--non-interactive"],
    );
    let seed = root.join("seed");
    std::fs::create_dir_all(seed.join("Code")).unwrap();
    std::fs::create_dir_all(seed.join("vendor")).unwrap();
    std::fs::write(seed.join("Code/A.cs"), "a1\n").unwrap();
    std::fs::write(seed.join("vendor/V.cs"), "v1\n").unwrap();
    run(
        &seed,
        "svn",
        &["add", "Code", "vendor", "--non-interactive"],
    );
    run(&seed, "svn", &["commit", "-m", "r1", "--non-interactive"]);

    if kind == NestedExternal::PinnedOlder {
        // 让 vendor 自己往前走一个 revision，这样 `-r1` 钉住的内容与根的 revision
        // 真的错开——不错开的话「钉住」和「跟随」在 nodeRevs 上看不出区别。
        run(&seed, "svn", &["update", "--non-interactive"]);
        std::fs::write(seed.join("vendor/V.cs"), "v2\n").unwrap();
        run(
            &seed,
            "svn",
            &["commit", "-m", "r2 vendor moves on", "--non-interactive"],
        );
    }
    run(&seed, "svn", &["update", "--non-interactive"]);

    // ⚠️ 属性值走 `-F` 文件，不走命令行：行尾必须一致（实测 E10，混合行尾会让
    // propset 报 E135000 而属性静默没设上，测试拿到「这个仓库没有 externals」的假绿）。
    let peg = if kind == NestedExternal::PinnedOlder {
        "-r1 "
    } else {
        ""
    };
    let ext = root.join("externals.txt");
    std::fs::write(&ext, format!("{peg}{url}/vendor Vendor\n")).unwrap();
    run(
        &seed,
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
        &seed,
        "svn",
        &["commit", "-m", "nested external", "--non-interactive"],
    );

    run(
        &root,
        "svn",
        &["checkout", &url, "wc1", "--non-interactive"],
    );
    let wc = root.join("wc1");
    if kind == NestedExternal::Dirty {
        std::fs::write(wc.join("Code/Vendor/V.cs"), "LOCALLY MODIFIED\n").unwrap();
    }
    SvnFixture { dir }
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

/// 白名单目录**在 r1 上还不存在**，r2 才建出来。
///
/// # 干什么用的
///
/// 复现 2026-09-15 实测 P1 那个形态：`svn ls -R <url>/Code@1` 时路径不存在，
/// svn **exit 1**，但 stdout 仍是一份**结构完整的空** `<lists><list></list></lists>`
/// ——`parse_list` 会成功解析出一个空清单。
///
/// 这是服务器侧核对最危险的失败形态：忽略退出码的写法会拿到空的服务器集合，
/// 而「服务器有、磁盘没有」的差集在空集上**恒为空** ⇒ 报告「没有漏收」。
///
/// 「把整个仓库删掉」那种造法**测不出这一条**：那时 XML 是坏的，解析会失败，
/// 于是靠解析错误兜住了，退出码那道门根本没被考验到。
pub fn svn_repo_whitelist_added_in_r2() -> SvnFixture {
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

    // r1：只有 Other/，还没有 Code/
    std::fs::create_dir_all(wc.join("Other")).unwrap();
    std::fs::write(wc.join("Other/O.txt"), "o\n").unwrap();
    run(&wc, "svn", &["add", "Other", "--non-interactive"]);
    run(
        &wc,
        "svn",
        &["commit", "-m", "r1 other", "--non-interactive"],
    );

    // r2：Code/ 这才出现
    std::fs::create_dir_all(wc.join("Code")).unwrap();
    std::fs::write(wc.join("Code/A.cs"), "a\n").unwrap();
    run(&wc, "svn", &["add", "Code", "--non-interactive"]);
    run(
        &wc,
        "svn",
        &["commit", "-m", "r2 code", "--non-interactive"],
    );
    run(&wc, "svn", &["update", "--non-interactive"]);
    SvnFixture { dir }
}

/// 三个 revision、工作副本**干净地停在 r3**。
///
/// 失同步检测的测试都要从这里起步：先在一个干净仓库上初始化（P1/P2/P3 都得过），
/// 再由测试自己制造外部变更。直接拿一个已经失同步的 fixture 去初始化是造不出来的
/// ——向导会在前置校验那一步就把它拦下。
///
/// `seed/` 那个工作副本留着不删，测试可以用它扮演「同事的机器」发外部提交。
pub fn svn_repo_three_revisions() -> SvnFixture {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    let url = seed_three_revisions(&root);
    run(
        &root,
        "svn",
        &["checkout", &url, "wc1", "--non-interactive"],
    );
    SvnFixture { dir }
}

/// 白名单落在一条**被 `-r` 钉死、但已被单独 update 到 HEAD** 的 external 里。
///
/// # 这是 spec §6.6.2 点名的那个「具体且常见」的成因
///
/// 布局：`Code` 是一条 `-r2 ^/vendor Code` 的 external（钉在 r2），而仓库已经
/// 走到 r4。fixture 最后把 `Code` **单独** update 到 HEAD，于是：
///
/// ```text
/// nodeRevs(Code) = {4}      ← 可以初始化，baselineRev = 4
/// externals 定义仍然写着 -r2
/// ```
///
/// 测试接着在**项目根**跑一次递归 `svn update`，SVN 会按 externals 定义把 `Code`
/// **拽回 r2**。此时 `nodeRevs = {2}`、`baselineRev = 4`，而 2 恰好等于该 external
/// 的 peg —— 这正是 `PinnedReset` 要认出来的形态。
///
/// 认错的代价很具体：`Diverged` 给的引导是「重建基线」（重型，而且会把钉死的
/// 旧内容录成新基线），`PinnedReset` 给的是一条 `svn update <目录>`（一步到位）。
pub fn svn_repo_pinned_external_updated_to_head() -> SvnFixture {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    run(&root, "svnadmin", &["create", "repo"]);
    let url = file_url(&root.join("repo"));
    run(
        &root,
        "svn",
        &["checkout", &url, "seed", "--non-interactive"],
    );
    let seed = root.join("seed");

    // r1：建 vendor/ 与项目根 proj/
    std::fs::create_dir_all(seed.join("vendor")).unwrap();
    std::fs::create_dir_all(seed.join("proj")).unwrap();
    std::fs::write(seed.join("vendor/A.cs"), "v1\n").unwrap();
    std::fs::write(seed.join("proj/readme.txt"), "p\n").unwrap();
    run(
        &seed,
        "svn",
        &["add", "vendor", "proj", "--non-interactive"],
    );
    run(&seed, "svn", &["commit", "-m", "r1", "--non-interactive"]);

    // r2：vendor 往前走一格 —— 这个 revision 就是后面要钉住的那个 peg
    std::fs::write(seed.join("vendor/A.cs"), "v2\n").unwrap();
    run(
        &seed,
        "svn",
        &["commit", "-m", "r2 vendor", "--non-interactive"],
    );

    // r3：把 external 定义挂到 proj 上，钉死在 r2
    //
    // ⚠️ 属性值走 `-F` 文件：命令行传值遇到混合行尾会让 propset 报 E135000
    // 而属性**静默没设上**，测试会拿到「这个仓库没有 externals」的假绿（1b 实测 E10）。
    let ext = root.join("externals.txt");
    std::fs::write(&ext, "-r2 ^/vendor Code\n").unwrap();
    run(
        &seed,
        "svn",
        &[
            "propset",
            "svn:externals",
            "-F",
            ext.to_str().unwrap(),
            "proj",
            "--non-interactive",
        ],
    );
    run(
        &seed,
        "svn",
        &["commit", "-m", "r3 pin", "--non-interactive"],
    );

    // r4：vendor 再往前走一格，让「HEAD」与「peg」真的错开
    run(&seed, "svn", &["update", "--non-interactive"]);
    std::fs::write(seed.join("vendor/A.cs"), "v4\n").unwrap();
    run(
        &seed,
        "svn",
        &["commit", "-m", "r4 vendor", "--non-interactive"],
    );

    // 把 proj 检出成 wc1，此时 external Code 会按定义停在 r2
    let proj_url = format!("{url}/proj");
    run(
        &root,
        "svn",
        &["checkout", &proj_url, "wc1", "--non-interactive"],
    );
    let wc = root.join("wc1");

    // 有人**单独**把 Code 更到了 HEAD —— 初始化就发生在这个状态下
    run(&wc, "svn", &["update", "--non-interactive", "--", "Code@"]);
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

/// **白名单所处的 revision 没有碰过白名单**——「孤儿 baseline」的成因。
///
/// # 这是常态，不是边界情况（2026-09-15 实测）
///
/// spec §10.3 把孤儿 baseline 描述成「若某个 baseline 的 revision **太老**」。
/// 成因其实与新旧无关：`svn update` 会把工作副本内**所有**节点的
/// `wc-status revision` 一律推到当时的 HEAD，**不管那个 revision 有没有动过它们**。
/// 于是同事在别的目录上提交几次之后，白名单的 `nodeRevs` 就等于一个从没碰过
/// 白名单的 revision，而 `baselineRev` 正是取自 `nodeRevs`。
///
/// 本 fixture 把这个形态固定下来：
///
/// ```text
/// r1  建 Code/A.cs 与 Other/O.txt
/// r2  只改 Code/A.cs          ← 白名单最后一次真正变化
/// r3  只改 Other/O.txt        ← 同事的提交
/// r4  只改 Other/O.txt        ← 同事的提交
/// svn update
///   → svn status -v -- Code   的 wc-status revision = 4
///   → svn log -r 4:4 <url>/Code    **空**（实测 Q1）
///   → svn log -r 4:4 <Repository Root>  拿得到 r4（实测 Q2）
/// ```
///
/// 所以锚点补拉**必须打给 Repository Root**；打给白名单 URL 会拿到空结果，
/// 图上那个 `svn:4` 节点就永远建不出来，工作分支挂不上主干。
pub fn svn_repo_baseline_rev_outside_whitelist() -> SvnFixture {
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
    std::fs::create_dir_all(wc.join("Other")).unwrap();
    std::fs::write(wc.join("Code/A.cs"), "a\n").unwrap();
    std::fs::write(wc.join("Other/O.txt"), "o\n").unwrap();
    run(&wc, "svn", &["add", "Code", "Other", "--non-interactive"]);
    run(
        &wc,
        "svn",
        &["commit", "-m", "r1 both", "--non-interactive"],
    );

    std::fs::write(wc.join("Code/A.cs"), "a2\n").unwrap();
    run(
        &wc,
        "svn",
        &["commit", "-m", "r2 code only", "--non-interactive"],
    );

    // r3 / r4：只动白名单**之外**的目录，模拟同事在别的项目上提交
    std::fs::write(wc.join("Other/O.txt"), "o3\n").unwrap();
    run(
        &wc,
        "svn",
        &["commit", "-m", "r3 other only", "--non-interactive"],
    );
    std::fs::write(wc.join("Other/O.txt"), "o4\n").unwrap();
    run(
        &wc,
        "svn",
        &["commit", "-m", "r4 other only", "--non-interactive"],
    );

    // update 之后 Code 的 nodeRevs 才会被推到 4 —— 少了这一步整个形态不成立
    run(&wc, "svn", &["update", "--non-interactive"]);
    SvnFixture { dir }
}

/// 工作副本停在 r1，而服务器已经走到 r9 —— **基线的 revision 远低于历史顶端**。
///
/// # 这个形态是翻页游标的试金石
///
/// `svn update` 不是每天都跑。工作副本停在一个旧 revision 上时，基线记的也是那个
/// 旧 revision，而它**不在首屏那一页里**——首屏拿的是最新的一页。于是基线只能
/// 靠单点补拉的锚点进入序列，序列变成「顶上一段 + 底下一个孤零零的锚点」，
/// 中间整段是空的。
///
/// 拿「序列里最旧的 revision」当翻页游标，就会直接从那个锚点往下翻，
/// **中间那一大段永远拿不到**，而界面上只表现为图里少了些节点。
pub fn svn_repo_working_copy_far_behind() -> SvnFixture {
    let dir = TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    run(&root, "svnadmin", &["create", "repo"]);
    let url = file_url(&root.join("repo"));

    // r1：wc1 自己建出白名单内容，之后**再也不 update**
    run(
        &root,
        "svn",
        &["checkout", &url, "wc1", "--non-interactive"],
    );
    let wc = root.join("wc1");
    std::fs::create_dir_all(wc.join("Code")).unwrap();
    std::fs::write(wc.join("Code/A.cs"), "a\n").unwrap();
    run(&wc, "svn", &["add", "Code", "--non-interactive"]);
    run(&wc, "svn", &["commit", "-m", "r1", "--non-interactive"]);
    run(&wc, "svn", &["update", "--non-interactive"]);

    // r2..r9 由**另一个 checkout** 提交：wc1 的 nodeRevs 因此停在 1，
    // 正是「同事在推进，我还没 update」的日常形态。
    run(
        &root,
        "svn",
        &["checkout", &url, "other", "--non-interactive"],
    );
    let other = root.join("other");
    for n in 2..=9 {
        std::fs::write(other.join("Code/A.cs"), format!("a{n}\n")).unwrap();
        run(
            &other,
            "svn",
            &["commit", "-m", &format!("r{n}"), "--non-interactive"],
        );
        run(&other, "svn", &["update", "--non-interactive"]);
    }

    SvnFixture { dir }
}

/// 白名单里放两个 `.cs`：一个设了 `svn:eol-style=native`（Windows 上磁盘是 CRLF），
/// 一个什么都不设（磁盘是 LF）。
///
/// 这个 fixture 钉住的是 **git 的 `core.autocrlf` 与录制路径的交互**。多数 Windows
/// 开发机上 `core.autocrlf=true`，于是：
///
/// ```text
/// 磁盘（native）          = "n1\r\n"   CRLF
/// git add -A 进 blob      = "n1\n"     LF     ← 归一过
/// hash-object --stdin     = "n1\r\n"   CRLF   ← 没归一
/// ```
///
/// 两条录制路径产出不同的 blob，意味着「这个文件在基线里长什么样」取决于它当时
/// 脏不脏——在脏工作副本上重建一次就会把行尾整个翻一遍（2026-09-17 实测）。
pub fn svn_repo_with_native_eol() -> SvnFixture {
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
    std::fs::write(wc.join("Code/Nat.cs"), "n1\n").unwrap();
    std::fs::write(wc.join("Code/Plain.cs"), "p1\n").unwrap();
    run(&wc, "svn", &["add", "Code", "--non-interactive"]);
    run(
        &wc,
        "svn",
        &[
            "propset",
            "svn:eol-style",
            "native",
            "Code/Nat.cs",
            "--non-interactive",
        ],
    );
    run(&wc, "svn", &["commit", "-m", "r1", "--non-interactive"]);
    run(&wc, "svn", &["update", "--non-interactive"]);
    SvnFixture { dir }
}

/// 把一个白名单内的 `.cs` 换成**同名目录**——SVN 意义上的 obstructed（`~`）。
///
/// 这是 spec §16.1 之后 P1 **唯一**仍然阻断的脏状态：svn 自己都说不清那个位置上
/// 是什么，`svn cat -r BASE` 拿不到可信的内容，录不出「服务器在 rN 的样子」。
pub fn svn_repo_obstructed() -> SvnFixture {
    let fx = svn_repo();
    let wc = fx.wc();
    std::fs::remove_file(wc.join("Code/A.cs")).unwrap();
    std::fs::create_dir(wc.join("Code/A.cs")).unwrap();
    fx
}

/// 白名单内放一个 **GBK 编码**的 `.cs`，并在本地改脏它。
///
/// 国内 Unity 工程里 GBK 源码很常见。这个 fixture 钉住 spec §16.2 末那条约束：
/// `svn cat` 的结果必须按字节接收。一旦过了 `String::from_utf8_lossy`，每个非法
/// 字节会变成 U+FFFD，录进基线就是一棵被悄悄改坏的树——而基线体检**查不出来**
/// （它比的是磁盘与服务器，不比基线树，spec §6.8.3）。
pub fn svn_repo_with_gbk_source() -> SvnFixture {
    let fx = svn_repo();
    let wc = fx.wc();
    // `// 中文` 的 GBK 字节：D6 D0 = 中，CE C4 = 文。D6 后面跟着的 D0 不是合法的
    // UTF-8 续字节（续字节必须是 10xxxxxx），所以这串一定会触发 lossy 替换，
    // 正是要钉的那种输入。
    let gbk: Vec<u8> = vec![
        0x2F, 0x2F, 0x20, 0xD6, 0xD0, 0xCE, 0xC4, 0x0A, 0x63, 0x6C, 0x61, 0x73, 0x73, 0x20, 0x47,
        0x20, 0x7B, 0x7D, 0x0A,
    ];
    std::fs::write(wc.join("Code/Gbk.cs"), &gbk).unwrap();
    run(&wc, "svn", &["add", "Code/Gbk.cs", "--non-interactive"]);
    run(&wc, "svn", &["commit", "-m", "r2 gbk", "--non-interactive"]);
    run(&wc, "svn", &["update", "--non-interactive"]);
    // 本地改脏——这才是 R2b 真正会调用 cat 的形态
    std::fs::write(wc.join("Code/Gbk.cs"), b"// local\n").unwrap();
    fx
}

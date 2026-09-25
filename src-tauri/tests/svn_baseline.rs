//! 基线录制：临时索引、`git-svn-id` trailer、`commit-tree` + `update-ref`
//! （spec §6.3.1 / §6.4.1）。
//!
//! **这是本期最不能写错的一段。** 四条硬要求各有回归测试：
//!
//! 1. 全程不碰真实 index、不碰 HEAD、不碰工作目录文件
//! 2. 临时索引文件必须清理干净，包括早退路径
//! 3. trailer 三字段缺一不可，解析失败即「这不是 baseline」
//! 4. `update-ref` 必须带旧 OID 作比较条件

use app_lib::engine::git_cli::{run_git, run_git_code};
use app_lib::engine::svn_baseline::{parse_trailer, publish_baseline, TempIndex, Trailer};
use app_lib::engine::svn_exclude::{generate_exclude, write_exclude};
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn fixture_git(dir: &Path, args: &[&str]) {
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

/// 一个已写好 exclude 规则的工作目录：`Code/A.cs` 会被收，`Code/B.psd` 不会。
/// HEAD 尚未出生——初始化向导跑到录制这一步时就是这个状态。
fn workdir_with_exclude() -> TempDir {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    fixture_git(p, &["init", "-b", "svn/base"]);
    fixture_git(p, &["config", "user.email", "t@t"]);
    fixture_git(p, &["config", "user.name", "T"]);
    fs::create_dir_all(p.join("Code")).unwrap();
    fs::write(p.join("Code/A.cs"), "a").unwrap();
    fs::write(p.join("Code/B.psd"), "b").unwrap();
    let w = vec!["Code/".to_string()];
    let e = vec![".cs".to_string()];
    write_exclude(p.to_str().unwrap(), &generate_exclude(&w, &e)).unwrap();
    dir
}

fn wl() -> Vec<String> {
    vec!["Code/".to_string()]
}

fn trailer(rev: u64) -> Trailer {
    Trailer {
        url: "https://svn.example.com/game".into(),
        rev,
        uuid: "8f3a1c7e-4b21-0410-9d2f-1a7b3c5d9e01".into(),
    }
}

// ---------------------------------------------------------------------------
// 硬要求 1 / 2：不碰用户的任何东西，且不留垃圾
// ---------------------------------------------------------------------------

#[test]
fn recording_touches_neither_the_real_index_nor_head_nor_the_worktree() {
    // 这条是整个 Phase 1b 的安全前提。2026-09-14 实测 GIT_INDEX_FILE 能做到，
    // 但「能做到」和「我们的实现做到了」是两回事。
    let dir = workdir_with_exclude();
    let root = dir.path().to_str().unwrap();
    fixture_git(dir.path(), &["add", "--", "Code/A.cs"]);
    let index_before = run_git(root, &["ls-files", "-s"]).unwrap();
    let head_before = run_git_code(root, &["rev-parse", "-q", "--verify", "HEAD"]).unwrap();

    let idx = TempIndex::empty(root).unwrap();
    idx.add_all(root, &wl()).unwrap();
    let tree = idx.write_tree(root).unwrap();

    assert_eq!(
        index_before,
        run_git(root, &["ls-files", "-s"]).unwrap(),
        "真实索引被动了"
    );
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
        idx.add_all(root, &wl()).unwrap();
        let p = idx.path().to_path_buf();
        assert!(p.exists(), "add 之后临时索引文件应当已经存在：{p:?}");
        p
    };
    assert!(!path.exists(), "临时索引没被清掉：{path:?}");
}

#[test]
fn two_temp_indexes_do_not_collide() {
    // 两个流程同时在录（或一个流程里先核对后录制），文件名撞了就会互相覆盖。
    let dir = workdir_with_exclude();
    let root = dir.path().to_str().unwrap();
    let a = TempIndex::empty(root).unwrap();
    let b = TempIndex::empty(root).unwrap();
    assert_ne!(a.path(), b.path());
}

#[test]
fn add_all_with_an_empty_whitelist_adds_nothing() {
    // G6：空 pathspec 绝不能退化成「全部」。`git add -A` 不带 pathspec
    // 会把整个仓库收进来——包括 .svn/ 那种一次 checkout 就能毁掉工作副本的东西。
    let dir = workdir_with_exclude();
    let root = dir.path().to_str().unwrap();
    let idx = TempIndex::empty(root).unwrap();
    idx.add_all(root, &[]).unwrap();
    assert!(idx.ls_files(root).unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// from_head：其余录制入口的起点
// ---------------------------------------------------------------------------

#[test]
fn from_head_starts_from_the_head_tree_not_the_users_staging_area() {
    // spec §6.3.1：初始化 / R2 从空索引起步，其余入口从**当前 HEAD 树**起步。
    // 不是从用户的暂存区——他随时可能 stage 了一半东西在那儿。
    let dir = workdir_with_exclude();
    let root = dir.path().to_str().unwrap();
    fixture_git(dir.path(), &["add", "-A"]);
    fixture_git(dir.path(), &["commit", "-m", "c1"]);

    // 用户在真实索引里 stage 了一个**磁盘上已经不存在**的文件的新增
    fs::write(dir.path().join("Code/Ghost.cs"), "g").unwrap();
    fixture_git(dir.path(), &["add", "--", "Code/Ghost.cs"]);
    fs::remove_file(dir.path().join("Code/Ghost.cs")).unwrap();

    let idx = TempIndex::from_head(root).unwrap();
    idx.add_all(root, &wl()).unwrap();
    let set = idx.ls_files(root).unwrap();

    assert!(set.contains("Code/A.cs"), "{set:?}");
    assert!(
        !set.contains("Code/Ghost.cs"),
        "候选索引跟着用户的暂存区跑了：{set:?}"
    );
    // 真实索引里它还在——我们确实没动它
    let real = run_git(root, &["ls-files"]).unwrap();
    assert!(real.contains("Code/Ghost.cs"), "真实索引被动了：{real}");
}

#[test]
fn from_head_on_an_unborn_head_is_a_clear_error_not_an_empty_index() {
    // 2026-09-14 实测：HEAD 未出生时 `git read-tree HEAD` 退出 128
    // （`fatal: Not a valid object name HEAD`）。静默当成空索引会让「从 HEAD 起步」
    // 的入口悄悄退化成「从空起步」，两者录出来的 tree 可能不同。
    let dir = workdir_with_exclude();
    let err = TempIndex::from_head(dir.path().to_str().unwrap())
        .expect_err("没有 HEAD 时 from_head 必须报错");
    assert_eq!(err.code, "svn_baseline_no_head");
}

// ---------------------------------------------------------------------------
// 硬要求 3：trailer
// ---------------------------------------------------------------------------

#[test]
fn the_trailer_round_trips_through_a_real_commit() {
    // 不是「格式化出我以为的字符串」，是**真的建一个 commit 再读回来**。
    let dir = workdir_with_exclude();
    let root = dir.path().to_str().unwrap();
    let idx = TempIndex::empty(root).unwrap();
    idx.add_all(root, &wl()).unwrap();
    let tree = idx.write_tree(root).unwrap();
    let oid = publish_baseline(root, &tree, None, &trailer(12346), None).unwrap();

    let msg = run_git(root, &["log", "-1", "--format=%B", &oid]).unwrap();
    let back = parse_trailer(&msg).expect("读不回来");
    assert_eq!(back.rev, 12346);
    assert_eq!(back.url, "https://svn.example.com/game");
    assert_eq!(back.uuid, "8f3a1c7e-4b21-0410-9d2f-1a7b3c5d9e01");

    // git svn 的标准用法也要能命中（spec §6.4.1 的互操作收益）
    let found = run_git(
        root,
        &[
            "log",
            "--grep=^git-svn-id",
            "--format=%H",
            "refs/heads/svn/base",
        ],
    )
    .unwrap();
    assert!(found.contains(&oid), "--grep 找不到：{found}");
}

#[test]
fn a_trailer_missing_any_field_is_not_a_baseline() {
    // 「不猜、不部分接受」（spec §6.4.1）。三个字段各缺一次。
    for bad in [
        "svn: r1\n\ngit-svn-id: https://x/game@12346\n", // 缺 uuid
        "svn: r1\n\ngit-svn-id: https://x/game 8f3a-1c7e\n", // 缺 @rev
        "svn: r1\n\ngit-svn-id: @12346 8f3a-1c7e\n",     // 缺 url
        "svn: r1\n\ngit-svn-id: https://x/game@abc 8f3a-1c7e\n", // rev 不是数字
        "svn: r1\n\ngit-svn-id: https://x/game@12346 not-a-uuid-zzz\n", // uuid 不是十六进制
        "svn: r1\n\ngit-svn-id: https://x/game@12346 8f3a-1c7e extra\n", // 尾巴上多了东西
        "svn: r1\n\n",                                   // 根本没有
    ] {
        assert!(parse_trailer(bad).is_none(), "不该被接受：{bad:?}");
    }
}

#[test]
fn the_url_may_itself_contain_an_at_sign() {
    // `svn+ssh://user@host/repo` 是完全正常的 URL。取**第一个** @ 会把 rev 读成
    // `host/repo@12346` 解析失败，整条基线被判成「不是 baseline」。
    let t = parse_trailer("git-svn-id: svn+ssh://user@host/repo@12346 8f3a-1c7e").unwrap();
    assert_eq!(t.url, "svn+ssh://user@host/repo");
    assert_eq!(t.rev, 12346);
}

#[test]
fn a_crlf_message_still_parses() {
    // commit message 经由不同工具往返时可能带上 CR。行尾一个 \r 会让
    // 正则里的 `$` 对不上，基线凭空变成「不是 baseline」。
    let t = parse_trailer("svn: r7\r\n\r\ngit-svn-id: https://x/game@7 8f3a-1c7e\r\n").unwrap();
    assert_eq!(t.rev, 7);
    assert_eq!(t.uuid, "8f3a-1c7e");
}

// ---------------------------------------------------------------------------
// 硬要求 4：update-ref 的并发保护
// ---------------------------------------------------------------------------

#[test]
fn publishing_refuses_when_the_baseline_moved_underneath_us() {
    // 另一个 mygitclient 进程、或用户手工操作，都可能在录制与发布之间动过基线。
    // 无条件覆盖等于悄悄丢掉那次录制。
    let dir = workdir_with_exclude();
    let root = dir.path().to_str().unwrap();
    let idx = TempIndex::empty(root).unwrap();
    idx.add_all(root, &wl()).unwrap();
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

#[test]
fn publishing_a_first_baseline_refuses_when_one_already_exists() {
    // `expected_old = None` 的含义是「这个 ref 现在不该存在」。
    // 不校验的话，第二次初始化会把已有基线连同它的全部历史顶掉。
    let dir = workdir_with_exclude();
    let root = dir.path().to_str().unwrap();
    let idx = TempIndex::empty(root).unwrap();
    idx.add_all(root, &wl()).unwrap();
    let tree = idx.write_tree(root).unwrap();
    let first = publish_baseline(root, &tree, None, &trailer(1), None).unwrap();

    let err = publish_baseline(root, &tree, None, &trailer(2), None)
        .expect_err("基线已存在时不该再当成首个基线发布");
    assert_eq!(err.code, "svn_baseline_moved");
    assert_eq!(
        run_git(root, &["rev-parse", "refs/heads/svn/base"])
            .unwrap()
            .trim(),
        first
    );
}

#[test]
fn recording_the_same_tree_again_is_legal() {
    // spec §6.3.1：「创建相同 tree 的新 revision 基线仍然合法，commit-tree
    // 不要求内容变化」。SVN 那边可能只动了白名单外的文件。
    let dir = workdir_with_exclude();
    let root = dir.path().to_str().unwrap();
    let idx = TempIndex::empty(root).unwrap();
    idx.add_all(root, &wl()).unwrap();
    let tree = idx.write_tree(root).unwrap();
    let first = publish_baseline(root, &tree, None, &trailer(1), None).unwrap();
    let second = publish_baseline(root, &tree, Some(&first), &trailer(2), Some(&first)).unwrap();
    assert_ne!(first, second, "同一棵 tree 也要产出新的 commit");
    assert_eq!(
        parse_trailer(&run_git(root, &["log", "-1", "--format=%B", &second]).unwrap())
            .unwrap()
            .rev,
        2
    );
}

// ---------------------------------------------------------------------------
// §16 R2b：把「磁盘树」逐条改写成「服务器 rN 的树」
//
// 这几条只测 TempIndex 这一层的机械动作；内容从哪来（`svn cat -r BASE`）由
// `svn_pristine` 负责，串起来由 `svn_init` 负责。
// ---------------------------------------------------------------------------

/// 读出某棵 tree 里某个路径的 blob **字节**。
fn blob_in_tree(root: &str, tree: &str, path: &str) -> Vec<u8> {
    app_lib::engine::git_cli::run_git_bytes(root, &["cat-file", "blob", &format!("{tree}:{path}")])
        .unwrap()
}

#[test]
fn replace_blob_puts_the_given_bytes_into_the_tree() {
    let dir = workdir_with_exclude();
    let root = dir.path().to_str().unwrap();
    let idx = TempIndex::empty(root).unwrap();
    idx.add_all(root, &wl()).unwrap();

    idx.replace_blob(root, "Code/A.cs", b"from-server\n")
        .unwrap();
    let tree = idx.write_tree(root).unwrap();

    assert_eq!(blob_in_tree(root, &tree, "Code/A.cs"), b"from-server\n");
    assert_eq!(
        fs::read(dir.path().join("Code/A.cs")).unwrap(),
        b"a",
        "磁盘一个字节都不许动——整条重建对工作目录只读（spec §16.1）"
    );
}

#[test]
fn replace_blob_round_trips_non_utf8_bytes() {
    // GBK 的 `.cs`。这条和 `svn_pristine` 那头是一对：那边保证取出来的字节没被
    // lossy 转换，这边保证写进 tree 的字节也没有。链条上任何一环过一次
    // `from_utf8_lossy`，录进基线的就是一棵被悄悄改坏的树，而体检查不出来。
    let dir = workdir_with_exclude();
    let root = dir.path().to_str().unwrap();
    let idx = TempIndex::empty(root).unwrap();
    idx.add_all(root, &wl()).unwrap();

    let gbk: &[u8] = &[0x2F, 0x2F, 0x20, 0xD6, 0xD0, 0xCE, 0xC4, 0x0A];
    idx.replace_blob(root, "Code/A.cs", gbk).unwrap();
    let tree = idx.write_tree(root).unwrap();

    assert_eq!(blob_in_tree(root, &tree, "Code/A.cs"), gbk);
}

#[test]
fn replace_blob_can_create_a_path_that_is_not_in_the_index_yet() {
    // `deleted` / `missing` 的形态：磁盘上没有这个文件，所以 `git add -A` 收不到它，
    // 但服务器 rN 上有，基线里必须有。
    let dir = workdir_with_exclude();
    let root = dir.path().to_str().unwrap();
    let idx = TempIndex::empty(root).unwrap();
    idx.add_all(root, &wl()).unwrap();
    assert!(
        !idx.ls_files(root).unwrap().contains("Code/Gone.cs"),
        "前提：这个路径本来不在索引里"
    );

    idx.replace_blob(root, "Code/Gone.cs", b"still-on-server\n")
        .unwrap();
    let tree = idx.write_tree(root).unwrap();

    assert_eq!(
        blob_in_tree(root, &tree, "Code/Gone.cs"),
        b"still-on-server\n"
    );
    assert!(
        !dir.path().join("Code/Gone.cs").exists(),
        "只进 tree，不许写回磁盘"
    );
}

#[test]
fn remove_path_takes_an_entry_out_of_the_candidate_index() {
    // `added` / `unversioned` 的形态：磁盘上有、服务器 rN 上没有。
    let dir = workdir_with_exclude();
    let root = dir.path().to_str().unwrap();
    let idx = TempIndex::empty(root).unwrap();
    idx.add_all(root, &wl()).unwrap();
    assert!(idx.ls_files(root).unwrap().contains("Code/A.cs"));

    idx.remove_path(root, "Code/A.cs").unwrap();

    assert!(!idx.ls_files(root).unwrap().contains("Code/A.cs"));
    assert!(
        dir.path().join("Code/A.cs").exists(),
        "从索引里摘掉，不是从磁盘上删掉"
    );
}

#[test]
fn replace_blob_keeps_the_mode_the_entry_already_had() {
    // 替换动作不该顺手改掉一个与本期无关的属性。索引里没有的路径退回 100644。
    let dir = workdir_with_exclude();
    let root = dir.path().to_str().unwrap();
    let idx = TempIndex::empty(root).unwrap();
    idx.add_all(root, &wl()).unwrap();
    let before = idx.stage_mode(root, "Code/A.cs").unwrap();
    assert!(before.is_some(), "前提：这个路径在索引里");

    idx.replace_blob(root, "Code/A.cs", b"x\n").unwrap();

    assert_eq!(idx.stage_mode(root, "Code/A.cs").unwrap(), before);
    assert_eq!(
        idx.stage_mode(root, "Code/Nope.cs").unwrap(),
        None,
        "不在索引里的路径必须返回 None，而不是编一个 mode 出来"
    );
}

#[test]
fn an_empty_path_is_refused_by_both_writers() {
    // 空 pathspec 在本仓库已经误伤过一次（`AGENTS.md` §6.2）。
    // `git update-index --force-remove -- ""` 的行为不该由我们去猜。
    let dir = workdir_with_exclude();
    let root = dir.path().to_str().unwrap();
    let idx = TempIndex::empty(root).unwrap();

    assert_eq!(
        idx.replace_blob(root, "", b"x").unwrap_err().code,
        "svn_baseline_empty_path"
    );
    assert_eq!(
        idx.remove_path(root, "").unwrap_err().code,
        "svn_baseline_empty_path"
    );
}

mod split_publish {
    use app_lib::engine::git_cli::run_git;
    use app_lib::engine::svn_baseline::{
        create_baseline_commit, move_baseline_ref, publish_baseline, Trailer,
    };

    fn repo() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let r = d.path().to_str().unwrap();
        run_git(r, &["init", "-q", "-b", "svn/base"]).unwrap();
        std::fs::write(d.path().join("A.cs"), "a\n").unwrap();
        run_git(r, &["add", "A.cs"]).unwrap();
        run_git(r, &["commit", "-qm", "base"]).unwrap();
        d
    }

    fn t(rev: u64) -> Trailer {
        Trailer {
            url: "file:///C:/repo".into(),
            rev,
            uuid: "0f3a0000-0000-0000-0000-000000000000".into(),
        }
    }

    #[test]
    fn creating_a_commit_does_not_move_the_ref() {
        // §7.6 publishing-baseline：commit 建好、ref 还没动之间要能落一次 journal
        let d = repo();
        let r = d.path().to_str().unwrap();
        let old = run_git(r, &["rev-parse", "svn/base"])
            .unwrap()
            .trim()
            .to_string();
        let tree = run_git(r, &["rev-parse", "svn/base^{tree}"])
            .unwrap()
            .trim()
            .to_string();
        let c = create_baseline_commit(r, &tree, Some(&old), &t(2)).unwrap();
        assert_eq!(run_git(r, &["rev-parse", "svn/base"]).unwrap().trim(), old);
        move_baseline_ref(r, &c, Some(&old)).unwrap();
        assert_eq!(run_git(r, &["rev-parse", "svn/base"]).unwrap().trim(), c);
    }

    #[test]
    fn moving_refuses_when_the_baseline_moved_in_between() {
        // 必测 #73：注入旧基线 OID 改变时 CAS 拒绝覆盖
        let d = repo();
        let r = d.path().to_str().unwrap();
        let old = run_git(r, &["rev-parse", "svn/base"])
            .unwrap()
            .trim()
            .to_string();
        let tree = run_git(r, &["rev-parse", "svn/base^{tree}"])
            .unwrap()
            .trim()
            .to_string();
        let ours = create_baseline_commit(r, &tree, Some(&old), &t(2)).unwrap();
        let theirs = create_baseline_commit(r, &tree, Some(&old), &t(3)).unwrap();
        run_git(r, &["update-ref", "refs/heads/svn/base", &theirs]).unwrap();
        let err = move_baseline_ref(r, &ours, Some(&old)).unwrap_err();
        assert_eq!(err.code, "svn_baseline_moved");
        assert_eq!(
            run_git(r, &["rev-parse", "svn/base"]).unwrap().trim(),
            theirs,
            "别人的不被覆盖"
        );
    }

    #[test]
    fn publish_is_still_create_then_move() {
        let d = repo();
        let r = d.path().to_str().unwrap();
        let old = run_git(r, &["rev-parse", "svn/base"])
            .unwrap()
            .trim()
            .to_string();
        let tree = run_git(r, &["rev-parse", "svn/base^{tree}"])
            .unwrap()
            .trim()
            .to_string();
        let c = publish_baseline(r, &tree, Some(&old), &t(2), Some(&old)).unwrap();
        assert_eq!(run_git(r, &["rev-parse", "svn/base"]).unwrap().trim(), c);
        let msg = run_git(r, &["log", "-1", "--format=%B", "svn/base"]).unwrap();
        assert!(msg.contains("git-svn-id: file:///C:/repo@2 "), "{msg}");
    }

    #[test]
    fn a_failed_update_ref_reads_as_one_sentence() {
        // 预检通过、`update-ref` 本身失败的那条窄竞态路径。用残留的 ref 锁文件
        // 触发（读 ref 不受影响，写 ref 报 `cannot lock ref`）。报错是给用户看的
        // 中文句子，中间不能夹着源码缩进出来的一串空格
        let d = repo();
        let r = d.path().to_str().unwrap();
        let old = run_git(r, &["rev-parse", "svn/base"])
            .unwrap()
            .trim()
            .to_string();
        let tree = run_git(r, &["rev-parse", "svn/base^{tree}"])
            .unwrap()
            .trim()
            .to_string();
        let c = create_baseline_commit(r, &tree, Some(&old), &t(2)).unwrap();
        std::fs::write(d.path().join(".git/refs/heads/svn/base.lock"), "").unwrap();
        let err = move_baseline_ref(r, &c, Some(&old)).unwrap_err();
        assert_eq!(err.code, "svn_baseline_moved");
        assert!(
            err.message.contains("改了。已经建好的 commit 是"),
            "{}",
            err.message
        );
        let ours = err.message.split("git 原话").next().unwrap();
        assert!(!ours.contains("  "), "{}", err.message);
    }
}

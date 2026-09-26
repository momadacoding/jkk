//! S1 / S10：WIP 按 opid 压栈、按 OID 取回（spec §7.3.1、§7.6.1）。
//! 都在临时 fixture 上跑，git 行为以 2026-09-25 实测为准（计划 A3 / A7）。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::git_cli::run_git;
use app_lib::engine::svn_journal::{read, Flow, Journal, Phase};
use app_lib::engine::svn_sync_stash::{
    drop_wip, find_by_opid, pin_ref, read_stash_ref, restore_wip, settle_stashing, stash_wip,
    RestoreOutcome,
};
use svn_fixtures::hybrid_with_colleague;

fn git(root: &str, args: &[&str]) -> String {
    run_git(root, args).unwrap().trim().to_string()
}

fn journal(opid: &str) -> Journal {
    Journal {
        opid: opid.into(),
        flow: Flow::SyncIn,
        phase: Phase::Start,
        started_at: 1,
        ..Default::default()
    }
}

fn wl() -> Vec<String> {
    vec!["Code/".to_string()]
}

#[test]
fn nothing_to_stash_leaves_stashed_without_an_oid() {
    // A7：无可压内容时 `git stash push` 退出 0、refs/stash 不变
    let fx = hybrid_with_colleague();
    let root = fx.wc_str();
    std::fs::write(fx.wc().join("Art/x.psd"), "psd-local\n").unwrap(); // 白名单外的改动不算
    let mut j = journal("none");
    stash_wip(&root, &mut j, &wl()).unwrap();
    assert_eq!(j.phase, Phase::Stashed);
    assert_eq!(j.stash_oid, None);
    assert_eq!(read_stash_ref(&root).unwrap(), None);
    assert_eq!(
        read(&root).unwrap().unwrap().phase,
        Phase::Stashed,
        "phase 落盘"
    );
}

#[test]
fn wip_is_stashed_by_opid_and_pinned() {
    let fx = hybrid_with_colleague();
    let root = fx.wc_str();
    std::fs::write(fx.wc().join("Code/A.cs"), "a-wip\n").unwrap();
    std::fs::write(fx.wc().join("Code/New.cs"), "untracked\n").unwrap();
    let mut j = journal("op1");
    stash_wip(&root, &mut j, &wl()).unwrap();
    let oid = j.stash_oid.clone().expect("有 WIP 就必须有 OID");
    assert_eq!(
        read_stash_ref(&root).unwrap().as_deref(),
        Some(oid.as_str())
    );
    assert_eq!(
        git(&root, &["rev-parse", &pin_ref("op1")]),
        oid,
        "§7.6.1：push 之后立刻钉住"
    );
    assert_eq!(
        git(&root, &["status", "--porcelain", "--", "Code"]),
        "",
        "白名单内被压干净"
    );
    assert!(
        !fx.wc().join("Code/New.cs").exists(),
        "-u：未跟踪的 W-file 也进 stash"
    );
}

#[test]
fn a_failed_push_that_changed_nothing_leaves_no_journal() {
    // push 失败时要先判断它落没落地：没落地就什么都没发生，journal 必须删掉，
    // 否则横幅会对一次根本没开始的同步喊「上次没收尾」。
    // 用残留的 `index.lock` 让 git stash push 失败（2026-09-26 实测：退出 1、
    // 工作区与 stash 栈都不动）。计划原来用「白名单指向不存在的路径」——实测带
    // `-u` 时 git 对不匹配的 pathspec 只说 `No local changes to save` 并退出 0，
    // 根本不失败。
    // （「journal 写在 push 之前」这一条这里观察不到——它由 Task 8 的
    //  `continue_from_stashing_with_a_landed_push` 钉住。）
    let fx = hybrid_with_colleague();
    let root = fx.wc_str();
    std::fs::write(fx.wc().join("Code/A.cs"), "a-wip\n").unwrap();
    let lock = fx.wc().join(".git/index.lock");
    std::fs::write(&lock, "").unwrap();
    let mut j = journal("op-early");
    let r = stash_wip(&root, &mut j, &wl());
    std::fs::remove_file(&lock).unwrap();
    assert!(r.is_err());
    // push 没落地、refs/stash 没变 ⇒ 什么都没发生，journal 已被删掉
    assert!(
        read(&root).unwrap().is_none(),
        "失败且没有任何改动时不留 journal"
    );
    assert_eq!(
        git(&root, &["status", "--porcelain", "--", "Code"]),
        "M Code/A.cs"
    );
}

#[test]
fn an_empty_whitelist_is_refused_not_widened() {
    // spec §12.2：空 pathspec 会退化成「全部」
    let fx = hybrid_with_colleague();
    let mut j = journal("op-empty");
    let err = stash_wip(&fx.wc_str(), &mut j, &[]).unwrap_err();
    assert_eq!(err.code, "sync_stash_empty_whitelist");
}

#[test]
fn settle_finds_our_stash_even_when_another_was_pushed_on_top() {
    // 必测 #17 / #40：外部工具在窗口内又压了一个 stash
    let fx = hybrid_with_colleague();
    let root = fx.wc_str();
    std::fs::write(fx.wc().join("Code/A.cs"), "a-wip\n").unwrap();
    let mut j = journal("op-drift");
    j.stash_ref_before = read_stash_ref(&root).unwrap();
    j.phase = Phase::Stashing;
    git(
        &root,
        &[
            "stash",
            "push",
            "-u",
            "-m",
            "mygitclient-sync:op-drift",
            "--",
            "Code",
        ],
    );
    let ours = read_stash_ref(&root).unwrap().unwrap();
    std::fs::write(fx.wc().join("Code/B.cs"), "someone-else\n").unwrap();
    git(&root, &["stash", "push", "-m", "someone else WIP"]);
    assert_ne!(
        read_stash_ref(&root).unwrap().unwrap(),
        ours,
        "前提：栈顶已经不是我们的"
    );

    settle_stashing(&root, &mut j).unwrap();
    assert_eq!(j.stash_oid.as_deref(), Some(ours.as_str()));
    assert_eq!(
        find_by_opid(&root, "op-drift").unwrap().as_deref(),
        Some(ours.as_str())
    );
}

#[test]
fn settle_stops_when_the_stash_vanished() {
    // §7.3.1：refs/stash 变了却找不到我们那条 ⇒ 被外部 clear / drop 过，必须停下
    let fx = hybrid_with_colleague();
    let root = fx.wc_str();
    let mut j = journal("op-lost");
    j.stash_ref_before = None;
    j.phase = Phase::Stashing;
    std::fs::write(fx.wc().join("Code/A.cs"), "someone\n").unwrap();
    git(&root, &["stash", "push", "-m", "not ours"]);
    let err = settle_stashing(&root, &mut j).unwrap_err();
    assert_eq!(err.code, "sync_stash_lost");
}

#[test]
fn restore_applies_and_drop_removes_only_ours() {
    let fx = hybrid_with_colleague();
    let root = fx.wc_str();
    std::fs::write(fx.wc().join("Code/B.cs"), "someone-else\n").unwrap();
    git(&root, &["stash", "push", "-m", "someone else WIP"]); // 预先有一条别人的
    std::fs::write(fx.wc().join("Code/A.cs"), "a-wip\n").unwrap();
    let mut j = journal("op-r");
    stash_wip(&root, &mut j, &wl()).unwrap();
    let oid = j.stash_oid.clone().unwrap();
    // 再在我们上面压一条：我们那条落到 stash@{1}，按栈顶 drop 就会吃掉别人的
    std::fs::write(fx.wc().join("Code/B.cs"), "on-top\n").unwrap();
    git(&root, &["stash", "push", "-m", "pushed on top"]);

    assert_eq!(restore_wip(&root, &oid).unwrap(), RestoreOutcome::Applied);
    assert_eq!(
        std::fs::read_to_string(fx.wc().join("Code/A.cs")).unwrap(),
        "a-wip\n"
    );
    drop_wip(&root, "op-r", &oid).unwrap();
    let list = git(&root, &["stash", "list"]);
    assert!(
        list.contains("someone else WIP"),
        "别人的 stash 不能碰：{list}"
    );
    assert!(
        list.contains("pushed on top"),
        "压在上面的那条也不能碰：{list}"
    );
    assert!(
        !list.contains("mygitclient-sync:op-r"),
        "我们的已经 drop：{list}"
    );
    assert_eq!(
        run_git(&root, &["rev-parse", "-q", "--verify", &pin_ref("op-r")]).ok(),
        None,
        "钉住的 ref 已删"
    );
}

#[test]
fn restore_refuses_when_a_parent_of_an_untracked_file_is_now_a_file() {
    // PR #49 review 第三轮 #3（2026-09-26 实测）：stash 里有未跟踪的 `Code/Sub/u.cs`，
    // 磁盘上 `Code/Sub` 却成了普通文件。对 `Code/Sub/u.cs` 取元数据会报错，原先被当成
    // 「不存在」放行；随后 `stash apply` 先写已跟踪部分、再在 `cannot create directory`
    // 上退出 1——正是 A3 要防的半套 apply
    let fx = hybrid_with_colleague();
    let root = fx.wc_str();
    std::fs::write(fx.wc().join("Code/A.cs"), "a-wip\n").unwrap();
    std::fs::create_dir(fx.wc().join("Code/Sub")).unwrap();
    std::fs::write(fx.wc().join("Code/Sub/u.cs"), "mine\n").unwrap();
    let mut j = journal("op-anc");
    stash_wip(&root, &mut j, &wl()).unwrap();
    let oid = j.stash_oid.clone().unwrap();
    assert!(!fx.wc().join("Code/Sub").exists());
    std::fs::write(fx.wc().join("Code/Sub"), "server\n").unwrap(); // 服务器新增了同名文件

    let out = restore_wip(&root, &oid).unwrap();
    assert_eq!(
        out,
        RestoreOutcome::Blocked {
            collisions: vec!["Code/Sub".into()]
        },
        "指名挡路的那个文件：用户要挪走的是它"
    );
    assert_eq!(
        std::fs::read_to_string(fx.wc().join("Code/A.cs")).unwrap(),
        "a1\n",
        "一个字节都不能 apply"
    );
    assert_eq!(
        std::fs::read_to_string(fx.wc().join("Code/Sub")).unwrap(),
        "server\n"
    );
}

#[test]
fn restore_refuses_when_an_untracked_file_would_collide() {
    // Review Focus 1 / A3：撞名时 git 会半套 apply，所以必须在 apply 之前拒绝
    let fx = hybrid_with_colleague();
    let root = fx.wc_str();
    std::fs::write(fx.wc().join("Code/A.cs"), "a-wip\n").unwrap();
    std::fs::write(fx.wc().join("Code/N.cs"), "mine\n").unwrap();
    let mut j = journal("op-c");
    stash_wip(&root, &mut j, &wl()).unwrap();
    let oid = j.stash_oid.clone().unwrap();
    std::fs::write(fx.wc().join("Code/N.cs"), "server\n").unwrap(); // 模拟服务器新增同名文件

    let out = restore_wip(&root, &oid).unwrap();
    assert_eq!(
        out,
        RestoreOutcome::Blocked {
            collisions: vec!["Code/N.cs".into()]
        }
    );
    assert_eq!(
        std::fs::read_to_string(fx.wc().join("Code/A.cs")).unwrap(),
        "a1\n",
        "一个字节都不能 apply——已跟踪部分也不行"
    );
    assert!(
        git(&root, &["stash", "list"]).contains("mygitclient-sync:op-c"),
        "stash 保留"
    );
}

#[test]
fn restore_passes_over_a_collision_that_is_the_same_file_everywhere() {
    // 计划 A17：外部 update 带来的服务器新增文件被 -u 收进 stash，基线跟上之后它又作为
    // 已跟踪文件回到磁盘。磁盘、stash、HEAD 三处是同一个 blob——不是撞名，是同一个文件
    let fx = hybrid_with_colleague();
    let root = fx.wc_str();
    let file = |rel: &str| std::fs::read_to_string(fx.wc().join(rel)).unwrap();
    std::fs::write(fx.wc().join("Code/A.cs"), "a-wip\n").unwrap();
    std::fs::write(fx.wc().join("Code/S.cs"), "server-new\n").unwrap();
    std::fs::write(fx.wc().join("Code/Mine.cs"), "mine\n").unwrap();
    let mut j = journal("op-s");
    stash_wip(&root, &mut j, &wl()).unwrap();
    let oid = j.stash_oid.clone().unwrap();
    // 模拟「新基线里有 S.cs」：同样的字节进了 HEAD
    std::fs::write(fx.wc().join("Code/S.cs"), "server-new\n").unwrap();
    git(&root, &["add", "Code/S.cs"]);
    git(&root, &["commit", "-m", "server adds S"]);

    assert_eq!(restore_wip(&root, &oid).unwrap(), RestoreOutcome::Applied);
    assert_eq!(file("Code/A.cs"), "a-wip\n");
    assert_eq!(file("Code/S.cs"), "server-new\n");
    assert_eq!(file("Code/Mine.cs"), "mine\n");
    assert_eq!(
        git(&root, &["status", "--porcelain", "--", "Code/S.cs"]),
        "",
        "S.cs 与 HEAD 一致，不显示成改动"
    );
}

#[test]
fn a_real_collision_blocks_even_next_to_a_same_file_one_and_removes_nothing() {
    // A17 的反方向：只要有一处字节不同就是真撞名；同一次里字节相同的那个也不能先被删
    let fx = hybrid_with_colleague();
    let root = fx.wc_str();
    std::fs::write(fx.wc().join("Code/S.cs"), "server-new\n").unwrap();
    std::fs::write(fx.wc().join("Code/N.cs"), "mine\n").unwrap();
    let mut j = journal("op-t");
    stash_wip(&root, &mut j, &wl()).unwrap();
    let oid = j.stash_oid.clone().unwrap();
    std::fs::write(fx.wc().join("Code/S.cs"), "server-new\n").unwrap();
    git(&root, &["add", "Code/S.cs"]);
    git(&root, &["commit", "-m", "server adds S"]);
    std::fs::write(fx.wc().join("Code/N.cs"), "server\n").unwrap();

    assert_eq!(
        restore_wip(&root, &oid).unwrap(),
        RestoreOutcome::Blocked {
            collisions: vec!["Code/N.cs".into()]
        }
    );
    assert!(
        fx.wc().join("Code/S.cs").exists(),
        "拦下时不能已经删了 S.cs"
    );
    assert!(git(&root, &["stash", "list"]).contains("mygitclient-sync:op-t"));
}

#[test]
fn a_same_name_file_edited_on_disk_is_a_real_collision() {
    // A17 必须三处都比：HEAD 与 stash 相同、但磁盘上的已经被改过（比如用户在冲突窗口里
    // 动了它），删掉它就是丢掉用户的字节
    let fx = hybrid_with_colleague();
    let root = fx.wc_str();
    std::fs::write(fx.wc().join("Code/S.cs"), "server-new\n").unwrap();
    let mut j = journal("op-u");
    stash_wip(&root, &mut j, &wl()).unwrap();
    let oid = j.stash_oid.clone().unwrap();
    std::fs::write(fx.wc().join("Code/S.cs"), "server-new\n").unwrap();
    git(&root, &["add", "Code/S.cs"]);
    git(&root, &["commit", "-m", "server adds S"]);
    std::fs::write(fx.wc().join("Code/S.cs"), "edited\n").unwrap();

    assert_eq!(
        restore_wip(&root, &oid).unwrap(),
        RestoreOutcome::Blocked {
            collisions: vec!["Code/S.cs".into()]
        }
    );
    assert_eq!(
        std::fs::read_to_string(fx.wc().join("Code/S.cs")).unwrap(),
        "edited\n"
    );
}

#[test]
fn restore_reports_conflicts_and_keeps_the_stash() {
    let fx = hybrid_with_colleague();
    let root = fx.wc_str();
    std::fs::write(fx.wc().join("Code/A.cs"), "a-wip\n").unwrap();
    let mut j = journal("op-x");
    stash_wip(&root, &mut j, &wl()).unwrap();
    let oid = j.stash_oid.clone().unwrap();
    std::fs::write(fx.wc().join("Code/A.cs"), "a-committed\n").unwrap();
    git(&root, &["commit", "-am", "same line"]);

    match restore_wip(&root, &oid).unwrap() {
        RestoreOutcome::Conflicted { files } => assert_eq!(files, vec!["Code/A.cs".to_string()]),
        other => panic!("应当是冲突：{other:?}"),
    }
    assert!(git(&root, &["stash", "list"]).contains("mygitclient-sync:op-x"));
}

#[test]
fn restore_refuses_an_object_that_is_not_a_stash() {
    // §7.6.1 末：「恢复前必须先校验该 OID……是一个 stash 形状的 commit」
    let fx = hybrid_with_colleague();
    let root = fx.wc_str();
    let head = git(&root, &["rev-parse", "HEAD"]);
    assert_eq!(
        restore_wip(&root, &head).unwrap_err().code,
        "sync_stash_missing"
    );
}

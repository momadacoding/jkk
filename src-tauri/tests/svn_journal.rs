//! 写前 operation journal（spec §7.6）。
//!
//! 它不是「冲突时才写的状态文件」，而是**写前日志**：任何一步修改落地之前，
//! journal 必须已经在磁盘上。否则存在无法恢复的窗口——S1 完成 stash 之后、
//! 状态写盘之前崩溃，用户的 WIP 就只剩一个匿名 stash 条目，客户端重开后对它
//! 一无所知（§7.3.1）。

use app_lib::engine::svn_journal::{journal_path, read, remove, write, Flow, Journal, Phase};

fn git_dir_fixture() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join(".git")).unwrap();
    d
}

fn minimal(opid: &str) -> Journal {
    Journal {
        opid: opid.to_string(),
        flow: Flow::SyncIn,
        phase: Phase::Start,
        started_at: 1_757_000_000,
        orig_branch: Some("work".into()),
        orig_head: Some("e5d3f77".into()),
        ..Default::default()
    }
}

#[test]
fn a_journal_round_trips() {
    let fx = git_dir_fixture();
    let root = fx.path().to_string_lossy().to_string();
    let j = minimal("opid-1");
    write(&root, &j).unwrap();
    assert_eq!(read(&root).unwrap().as_ref(), Some(&j));
}

#[test]
fn no_journal_is_none_not_an_error() {
    // 「上次干净收尾了」与「读失败了」必须分得开：前者什么都不做，后者要停下来报错。
    let fx = git_dir_fixture();
    let root = fx.path().to_string_lossy().to_string();
    assert_eq!(read(&root).unwrap(), None);
}

#[test]
fn a_corrupt_journal_is_an_error_not_a_default() {
    // **最要紧的一条。** 坏 JSON 静默退化成默认值，等于把用户 stash 的 OID
    // 悄悄丢掉——恢复代码会以为「上次什么都没做」，而磁盘上工作区已经被清空了。
    let fx = git_dir_fixture();
    let root = fx.path().to_string_lossy().to_string();
    std::fs::write(journal_path(&root), b"{ this is not json").unwrap();

    let err = read(&root).expect_err("坏 journal 必须报错，不能当成没有");
    assert_eq!(err.code, "svn_journal_corrupt", "实际：{}", err.message);
    assert!(
        journal_path(&root).exists(),
        "报错之后必须把文件留在原地——它是唯一的现场"
    );
}

#[test]
fn a_journal_missing_its_phase_is_an_error_not_a_default() {
    // 2026-09-25 独立 review（B3）。`a_corrupt_journal_is_an_error_not_a_default`
    // 只覆盖了**语法坏掉**的 JSON；容器级 `#[serde(default)]` 开出第二类：
    // 语法合法但少了关键字段，serde 会静默填上默认值。
    //
    // 后果比语法坏掉更糟。少了 `phase` 会读成 `Phase::Start`，而界面对 `start`
    // 说的是「还没有动过工作目录里的任何东西——可以直接当作没发生过」。对一次
    // 实际停在 `svn-staged`（`.svn` 里挂着 add / delete 登记）的流程说这句话，
    // 是整个界面能说出的最有害的一句。
    let fx = git_dir_fixture();
    let root = fx.path().to_string_lossy().to_string();

    for bad in [
        r#"{"opid":"x","flow":"sync_out","startedAt":7,"stashOid":"abc"}"#, // 少 phase
        r#"{"flow":"sync_in","phase":"stashed","startedAt":7}"#,            // 少 opid
        "{}",                                                               // 什么都没有
    ] {
        std::fs::write(journal_path(&root), bad).unwrap();
        let err = read(&root).unwrap_err();
        assert_eq!(
            err.code, "svn_journal_corrupt",
            "缺字段的 journal 必须报错而不是填默认值。输入：{bad}"
        );
    }
}

#[test]
fn writing_leaves_no_temp_file_behind() {
    let fx = git_dir_fixture();
    let root = fx.path().to_string_lossy().to_string();
    write(&root, &minimal("opid-2")).unwrap();

    let leftovers: Vec<String> = std::fs::read_dir(fx.path().join(".git"))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.contains(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "留下了临时文件：{leftovers:?}");
}

#[test]
fn overwriting_never_leaves_a_half_written_file() {
    // 原子写的可观察后果：任何时刻读到的要么是旧的完整 JSON，要么是新的完整
    // JSON。用「写一个大 journal 覆盖小 journal，中间反复读」来逼近——读到的
    // 每一份都必须能解析。
    let fx = git_dir_fixture();
    let root = fx.path().to_string_lossy().to_string();
    write(&root, &minimal("small")).unwrap();

    let mut big = minimal("big");
    big.svn_added = (0..2000)
        .map(|i| format!("Assets/Generated/F{i}.cs"))
        .collect();

    let r2 = root.clone();
    let t = std::thread::spawn(move || {
        for _ in 0..200 {
            // 每一次读都必须成功——读到半个文件就会在这里炸
            read(&r2).expect("并发读到了一个解析不了的 journal，说明写不是原子的");
        }
    });
    for _ in 0..50 {
        write(&root, &big).unwrap();
        write(&root, &minimal("small")).unwrap();
    }
    t.join().unwrap();
}

#[test]
fn every_phase_uses_the_exact_string_from_the_spec() {
    // 线序格式写错是**看不见的**：本地全绿，崩溃恢复时才发现读不回来。
    // 这些字符串逐字抄自 spec §7.6 的两张 phase 表。
    let cases = [
        (Phase::Start, "start"),
        (Phase::Stashing, "stashing"),
        (Phase::Stashed, "stashed"),
        (Phase::CapturingBaseline, "capturing-baseline"),
        (Phase::PublishingBaseline, "publishing-baseline"),
        (Phase::BaselineReady, "baseline-ready"),
        (Phase::Rebase, "rebase"),
        (Phase::Prepare, "prepare"),
        (Phase::BuildingDeliver, "building-deliver"),
        (Phase::DeliverBuilt, "deliver-built"),
        (Phase::CheckedOut, "checked-out"),
        (Phase::SvnStaged, "svn-staged"),
        (Phase::Committing, "committing"),
        (Phase::PostCommit, "post-commit"),
        (Phase::RestoringBranch, "restoring-branch"),
    ];
    for (phase, text) in cases {
        let encoded = serde_json::to_string(&phase).unwrap();
        assert_eq!(encoded, format!("\"{text}\""), "{phase:?} 的线序名不对");
    }
}

#[test]
fn flow_uses_the_exact_strings_from_the_spec() {
    // 同上。`flow` 决定恢复入口走哪条分支——§7.6 明写 `publishing-baseline`
    // 是两个流程共用的阶段名，但父提交与返回入口完全不同，不能套用。
    assert_eq!(serde_json::to_string(&Flow::SyncIn).unwrap(), "\"sync_in\"");
    assert_eq!(
        serde_json::to_string(&Flow::SyncOut).unwrap(),
        "\"sync_out\""
    );
}

#[test]
fn removing_a_missing_journal_is_not_an_error() {
    // S11 / D10 收尾会无条件调它；失败路径上可能已经删过一次。
    let fx = git_dir_fixture();
    let root = fx.path().to_string_lossy().to_string();
    remove(&root).expect("删一个不存在的 journal 不该报错");
}

#[test]
fn a_stash_oid_survives_the_round_trip() {
    // §7.6.1：stash 必须按**不可变 OID** 记，绝不能记 `stash@{N}`（那是栈位置，
    // 不是身份）。这条测的是它真的落盘并读得回来——恢复流程全靠它。
    let fx = git_dir_fixture();
    let root = fx.path().to_string_lossy().to_string();
    let mut j = minimal("opid-stash");
    j.phase = Phase::Stashed;
    j.stash_ref_before = Some("9c1e4b2".into());
    j.stash_oid = Some("f2208125aad7d35d63ee9495ad0a6cc3c1ce21be".into());
    write(&root, &j).unwrap();

    let back = read(&root).unwrap().unwrap();
    assert_eq!(
        back.stash_oid.as_deref(),
        Some("f2208125aad7d35d63ee9495ad0a6cc3c1ce21be")
    );
    assert_eq!(back.stash_ref_before.as_deref(), Some("9c1e4b2"));
    assert_eq!(back.phase, Phase::Stashed);
}

#[test]
fn a_stale_tmp_left_by_a_crash_is_swept_on_the_next_write() {
    // 2b 第二轮 review R7：崩溃在 write 中途会在 `.git/` 里留下
    // `mygitclient-sync.json.<pid>.<nanos>.tmp`，没有任何机制清理它。
    // 只有持锁者写 journal（模块头「所有权由锁界定」），所以写入时看到的别的
    // tmp 必然是前人留下的残骸。
    let fx = git_dir_fixture();
    let root = fx.path().to_str().unwrap();
    let stale = fx
        .path()
        .join(".git")
        .join("mygitclient-sync.json.999.123.tmp");
    std::fs::write(&stale, b"half").unwrap();
    // 不相干的文件不能被误删
    let bystander = fx.path().join(".git").join("mygitclient-sync.json.bak");
    std::fs::write(&bystander, b"keep").unwrap();

    write(root, &minimal("sweep")).unwrap();

    assert!(!stale.exists(), "残留 tmp 必须被清掉");
    assert!(bystander.exists(), "只清 `.tmp` 结尾的同前缀文件");
}

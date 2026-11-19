#![allow(clippy::unwrap_used, clippy::panic)] // 测试里 unwrap / panic 即断言，见 clippy.toml
//! 保住 SVN changelist：记录、补挂、延迟核对（`svn_changelist`；计划
//! `2026-11-20-svn-changelist-keep.md` T2）。
//!
//! 「IDE 摘掉 changelist」在测试里用 `svn changelist --remove` 模拟：IDEA 的
//! `SvnChangelistListener.changesRemoved` 发的就是这条命令。三个流程的端到端场景在
//! `svn_sync_in.rs`、`svn_sync_out.rs`、`svn_follow.rs` 里。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::svn_changelist::{
    read_changelists, read_record, recheck, record, record_path, settle, Record,
};
use app_lib::engine::svn_cli::run_svn;
use app_lib::engine::svn_journal::{self, Journal};
use app_lib::engine::svn_lock;
use app_lib::engine::svn_tombstone::fault::{self, Point};
use app_lib::engine::svn_xml::parse_changelist_status;
use std::collections::BTreeMap;
use svn_fixtures::{hybrid_with_colleague, svn_cmd, SvnFixture};

fn wl() -> Vec<String> {
    vec!["Code/".to_string()]
}

fn write(fx: &SvnFixture, rel: &str, s: &str) {
    std::fs::write(fx.wc().join(rel), s).unwrap();
}

fn cl(fx: &SvnFixture, name: &str, rel: &str) {
    svn_cmd(
        &fx.wc(),
        &["changelist", name, "--non-interactive", "--", rel],
    );
}

/// IDE 在文件「已还原」时发的那条命令。
fn ide_removes(fx: &SvnFixture, rel: &str) {
    svn_cmd(
        &fx.wc(),
        &["changelist", "--remove", "--non-interactive", "--", rel],
    );
}

/// 此刻 svn 眼里白名单底下的 changelist（走 svn，不走被测代码）。
fn lists(fx: &SvnFixture) -> BTreeMap<String, String> {
    let xml = run_svn(&fx.wc_str(), &["status", "--xml", "--", "Code@"]).unwrap();
    parse_changelist_status(&xml)
        .unwrap()
        .into_iter()
        .filter_map(|s| s.changelist.map(|c| (s.path, c)))
        .collect()
}

fn pairs(v: &[(&str, &str)]) -> BTreeMap<String, String> {
    v.iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
}

/// A 改过挂在 speed，B 改过挂在 logs。
fn two_lists(fx: &SvnFixture) {
    write(fx, "Code/A.cs", "a-wip\n");
    write(fx, "Code/B.cs", "b-wip\n");
    cl(fx, "speed", "Code/A.cs");
    cl(fx, "logs", "Code/B.cs");
}

#[test]
fn a_modified_file_whose_changelist_was_removed_gets_it_back() {
    let fx = hybrid_with_colleague();
    two_lists(&fx);
    record(&fx.wc_str(), &wl(), "op1");
    ide_removes(&fx, "Code/A.cs");
    ide_removes(&fx, "Code/B.cs");

    let s = settle(&fx.wc_str()).expect("有记录、没有半路的流程，应当补挂");
    assert_eq!(s.opid, "op1");
    assert!(s.report.failed.is_empty(), "{:?}", s.report);
    assert_eq!(
        lists(&fx),
        pairs(&[("Code/A.cs", "speed"), ("Code/B.cs", "logs")])
    );
    assert!(
        record_path(&fx.wc_str()).exists(),
        "settle 之后记录留给延迟核对"
    );

    let r = recheck(&fx.wc_str(), "op1").expect("自己那份记录，应当核对");
    assert!(r.reattached.is_empty(), "已经挂好了，没有可补的：{r:?}");
    assert!(!record_path(&fx.wc_str()).exists(), "核对完删记录");
}

#[test]
fn a_file_that_is_no_longer_modified_is_not_put_back() {
    // 交付提交掉的文件、或服务器内容与本地改动相同：svn 自己也会摘掉 changelist
    let fx = hybrid_with_colleague();
    two_lists(&fx);
    record(&fx.wc_str(), &wl(), "op1");
    ide_removes(&fx, "Code/B.cs");
    write(&fx, "Code/B.cs", "b1\n"); // 回到 BASE

    let s = settle(&fx.wc_str()).unwrap();
    assert!(s.report.reattached.is_empty(), "{:?}", s.report);
    assert_eq!(lists(&fx), pairs(&[("Code/A.cs", "speed")]));
}

#[test]
fn a_file_moved_to_another_list_is_left_there() {
    let fx = hybrid_with_colleague();
    two_lists(&fx);
    record(&fx.wc_str(), &wl(), "op1");
    cl(&fx, "other", "Code/A.cs");

    settle(&fx.wc_str()).unwrap();
    assert_eq!(
        lists(&fx),
        pairs(&[("Code/A.cs", "other"), ("Code/B.cs", "logs")])
    );
}

#[test]
fn a_property_only_change_counts_as_modified() {
    let fx = hybrid_with_colleague();
    svn_cmd(
        &fx.wc(),
        &[
            "propset",
            "x:flag",
            "1",
            "--non-interactive",
            "--",
            "Code/A.cs",
        ],
    );
    cl(&fx, "props", "Code/A.cs");
    record(&fx.wc_str(), &wl(), "op1");
    ide_removes(&fx, "Code/A.cs");

    settle(&fx.wc_str()).unwrap();
    assert_eq!(lists(&fx), pairs(&[("Code/A.cs", "props")]));
}

#[test]
fn a_file_scheduled_for_deletion_gets_its_list_back() {
    // `svn delete` 掉的文件不在磁盘上，IDE 仍把它列成一条「本地删除」的改动
    let fx = hybrid_with_colleague();
    svn_cmd(
        &fx.wc(),
        &["delete", "--non-interactive", "--", "Code/B.cs"],
    );
    cl(&fx, "del", "Code/B.cs");
    record(&fx.wc_str(), &wl(), "op1");
    ide_removes(&fx, "Code/B.cs");

    let s = settle(&fx.wc_str()).unwrap();
    assert!(s.report.failed.is_empty(), "{:?}", s.report);
    assert_eq!(lists(&fx), pairs(&[("Code/B.cs", "del")]));
}

#[test]
fn a_file_name_with_an_at_sign() {
    // svn 把 target 里最后一个 `@` 之后当 peg revision，补挂与问状态都要带空 peg
    let fx = hybrid_with_colleague();
    write(&fx, "Code/a@b.cs", "x\n");
    svn_cmd(
        &fx.wc(),
        &["add", "--non-interactive", "--", "Code/a@b.cs@"],
    );
    svn_cmd(
        &fx.wc(),
        &[
            "changelist",
            "at",
            "--non-interactive",
            "--",
            "Code/a@b.cs@",
        ],
    );
    record(&fx.wc_str(), &wl(), "op1");
    svn_cmd(
        &fx.wc(),
        &[
            "changelist",
            "--remove",
            "--non-interactive",
            "--",
            "Code/a@b.cs@",
        ],
    );

    let s = settle(&fx.wc_str()).unwrap();
    assert!(s.report.failed.is_empty(), "{:?}", s.report);
    assert_eq!(lists(&fx), pairs(&[("Code/a@b.cs", "at")]));
}

#[test]
fn a_list_name_starting_with_a_dash() {
    let fx = hybrid_with_colleague();
    write(&fx, "Code/A.cs", "a-wip\n");
    svn_cmd(
        &fx.wc(),
        &[
            "changelist",
            "--non-interactive",
            "--",
            "-dash",
            "Code/A.cs",
        ],
    );
    record(&fx.wc_str(), &wl(), "op1");
    ide_removes(&fx, "Code/A.cs");

    let s = settle(&fx.wc_str()).unwrap();
    assert!(s.report.failed.is_empty(), "{:?}", s.report);
    assert_eq!(lists(&fx), pairs(&[("Code/A.cs", "-dash")]));
}

#[test]
fn a_recorded_file_that_is_gone_from_disk_does_not_break_the_others() {
    let fx = hybrid_with_colleague();
    two_lists(&fx);
    write(&fx, "Code/New.cs", "n\n");
    svn_cmd(&fx.wc(), &["add", "--non-interactive", "--", "Code/New.cs"]);
    cl(&fx, "new", "Code/New.cs");
    record(&fx.wc_str(), &wl(), "op1");
    ide_removes(&fx, "Code/A.cs");
    // 新增的文件被撤销登记、也从磁盘上删掉，连父目录一起没了的路径也算上：svn status 对它们只给
    // 警告，同一批的其他文件照常补挂
    svn_cmd(
        &fx.wc(),
        &["revert", "--non-interactive", "--", "Code/New.cs"],
    );
    std::fs::remove_file(fx.wc().join("Code/New.cs")).unwrap();
    let mut r = read_record(&fx.wc_str()).unwrap();
    r.entries.insert("Code/NoDir/Gone.cs".into(), "gone".into());
    std::fs::write(
        record_path(&fx.wc_str()),
        serde_json::to_string(&r).unwrap(),
    )
    .unwrap();

    let s = settle(&fx.wc_str()).unwrap();
    assert!(s.report.failed.is_empty(), "{:?}", s.report);
    assert_eq!(
        lists(&fx),
        pairs(&[("Code/A.cs", "speed"), ("Code/B.cs", "logs")])
    );
}

#[test]
fn nothing_is_put_back_while_a_flow_is_stopped_halfway() {
    // 停在冲突上时文件可能还是原版：挂上也会被 IDE 再摘掉，等流程走完再挂
    let fx = hybrid_with_colleague();
    two_lists(&fx);
    record(&fx.wc_str(), &wl(), "op1");
    ide_removes(&fx, "Code/A.cs");
    svn_journal::write(
        &fx.wc_str(),
        &Journal {
            opid: "op1".into(),
            ..Default::default()
        },
    )
    .unwrap();

    assert!(settle(&fx.wc_str()).is_none());
    assert!(recheck(&fx.wc_str(), "op1").is_none());
    assert_eq!(lists(&fx), pairs(&[("Code/B.cs", "logs")]));
    assert!(record_path(&fx.wc_str()).exists(), "记录留着，流程走完再用");
}

#[test]
fn nothing_happens_while_another_flow_holds_the_lock() {
    let fx = hybrid_with_colleague();
    two_lists(&fx);
    record(&fx.wc_str(), &wl(), "op1");
    ide_removes(&fx, "Code/A.cs");
    let lock = svn_lock::acquire(&fx.wc_str(), "someone").unwrap();

    assert!(settle(&fx.wc_str()).is_none());
    assert!(recheck(&fx.wc_str(), "op1").is_none());
    assert!(record_path(&fx.wc_str()).exists());
    drop(lock);
    assert!(settle(&fx.wc_str()).is_some(), "锁放了就能补");
}

#[test]
fn recheck_leaves_a_newer_flows_record_alone() {
    let fx = hybrid_with_colleague();
    two_lists(&fx);
    record(&fx.wc_str(), &wl(), "op1");
    settle(&fx.wc_str()).unwrap();
    record(&fx.wc_str(), &wl(), "op2"); // 下一次流程

    assert!(recheck(&fx.wc_str(), "op1").is_none());
    assert_eq!(read_record(&fx.wc_str()).unwrap().opid, "op2");
}

#[test]
fn a_left_over_record_is_put_back_before_the_next_flow_records() {
    // 上一次流程收尾时锁被占（或进程崩了），记录没人处理：下一次流程记录之前先补挂它
    let fx = hybrid_with_colleague();
    two_lists(&fx);
    record(&fx.wc_str(), &wl(), "op1");
    ide_removes(&fx, "Code/A.cs");

    record(&fx.wc_str(), &wl(), "op2");
    assert_eq!(
        lists(&fx),
        pairs(&[("Code/A.cs", "speed"), ("Code/B.cs", "logs")])
    );
    let r = read_record(&fx.wc_str()).unwrap();
    assert_eq!(r.opid, "op2");
    assert_eq!(r.entries.len(), 2, "A 挂回去之后也记进了这一次");
}

#[test]
fn no_record_is_written_when_nothing_has_a_changelist() {
    let fx = hybrid_with_colleague();
    write(&fx, "Code/A.cs", "a-wip\n");
    record(&fx.wc_str(), &wl(), "op1");
    assert!(!record_path(&fx.wc_str()).exists());
    assert!(settle(&fx.wc_str()).is_none());
}

#[test]
fn a_corrupt_record_is_dropped() {
    let fx = hybrid_with_colleague();
    let p = record_path(&fx.wc_str());
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, "{not json").unwrap();
    assert!(read_record(&fx.wc_str()).is_none());
    assert!(!record_path(&fx.wc_str()).exists());
}

#[test]
fn the_record_survives_a_round_trip() {
    let fx = hybrid_with_colleague();
    two_lists(&fx);
    record(&fx.wc_str(), &wl(), "op1");
    let r = read_record(&fx.wc_str()).unwrap();
    assert_eq!(
        r,
        Record {
            opid: "op1".into(),
            recorded_at: r.recorded_at,
            settled_at: None,
            entries: pairs(&[("Code/A.cs", "speed"), ("Code/B.cs", "logs")]),
        }
    );
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// 把记录的 settledAt 改到 `secs_ago` 秒前，模拟「settle 过、程序在核对前退出了」。
fn settled_long_ago(fx: &SvnFixture, secs_ago: u64) {
    let mut r = read_record(&fx.wc_str()).unwrap();
    r.settled_at = Some(now() - secs_ago);
    std::fs::write(
        record_path(&fx.wc_str()),
        serde_json::to_string(&r).unwrap(),
    )
    .unwrap();
}

#[test]
fn settle_stamps_the_record_once() {
    let fx = hybrid_with_colleague();
    two_lists(&fx);
    record(&fx.wc_str(), &wl(), "op1");
    settle(&fx.wc_str()).unwrap();
    let first = read_record(&fx.wc_str()).unwrap().settled_at;
    assert!(first.is_some(), "settle 记下时刻");
    settled_long_ago(&fx, 30);
    let stamped = read_record(&fx.wc_str()).unwrap().settled_at;
    settle(&fx.wc_str()).expect("还没过期，照样补挂");
    assert_eq!(
        read_record(&fx.wc_str()).unwrap().settled_at,
        stamped,
        "再 settle 不往后推，过期从流程结束算起"
    );
}

#[test]
fn a_record_settled_long_ago_is_dropped_instead_of_put_back() {
    // 独立 review 第 2 条：程序在核对前退出，第二天用户在 IDE 里有意摘掉了 A 的 changelist，
    // 下一次 settle 不能拿旧记录把它挂回去
    let fx = hybrid_with_colleague();
    two_lists(&fx);
    record(&fx.wc_str(), &wl(), "op1");
    settle(&fx.wc_str()).unwrap();
    settled_long_ago(&fx, 24 * 3600);
    ide_removes(&fx, "Code/A.cs");

    assert!(settle(&fx.wc_str()).is_none());
    assert!(!record_path(&fx.wc_str()).exists(), "过期记录删掉");
    assert_eq!(lists(&fx), pairs(&[("Code/B.cs", "logs")]));
}

#[test]
fn a_record_settled_long_ago_is_not_put_back_by_the_next_flow_either() {
    let fx = hybrid_with_colleague();
    two_lists(&fx);
    record(&fx.wc_str(), &wl(), "op1");
    settle(&fx.wc_str()).unwrap();
    settled_long_ago(&fx, 24 * 3600);
    ide_removes(&fx, "Code/A.cs");

    record(&fx.wc_str(), &wl(), "op2");
    assert_eq!(lists(&fx), pairs(&[("Code/B.cs", "logs")]));
    assert_eq!(read_record(&fx.wc_str()).unwrap().opid, "op2");
}

#[test]
fn an_unsettled_record_does_not_expire() {
    // 停在冲突上的流程可能隔几小时才继续：没 settle 过的记录不按时间作废
    let fx = hybrid_with_colleague();
    two_lists(&fx);
    record(&fx.wc_str(), &wl(), "op1");
    let mut r = read_record(&fx.wc_str()).unwrap();
    r.recorded_at = now() - 24 * 3600;
    std::fs::write(
        record_path(&fx.wc_str()),
        serde_json::to_string(&r).unwrap(),
    )
    .unwrap();
    ide_removes(&fx, "Code/A.cs");

    settle(&fx.wc_str()).expect("没 settle 过，照样补挂");
    assert_eq!(
        lists(&fx),
        pairs(&[("Code/A.cs", "speed"), ("Code/B.cs", "logs")])
    );
}

#[test]
fn settle_does_not_take_the_sync_lock() {
    // 独立 review 第 1 条：拿同步锁会把这期间的自动跟进、git 提交挡成「正忙」，还让横幅闪一下。
    // settle 结束时锁文件若被建过又删掉也看不出来，所以另一线程在 settle 的全程（两次起 svn 进程，
    // 上百毫秒）不停地探测锁
    let fx = hybrid_with_colleague();
    two_lists(&fx);
    record(&fx.wc_str(), &wl(), "op1");
    ide_removes(&fx, "Code/A.cs");
    let repo = fx.wc_str();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let seen = {
        let (repo, stop) = (repo.clone(), stop.clone());
        std::thread::spawn(move || {
            let mut seen = false;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                seen |= svn_lock::is_held(&repo);
            }
            seen
        })
    };
    settle(&repo).unwrap();
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(!seen.join().unwrap(), "settle 期间同步锁被占过");
    assert_eq!(
        lists(&fx),
        pairs(&[("Code/A.cs", "speed"), ("Code/B.cs", "logs")])
    );
}

#[test]
fn an_empty_whitelist_reads_nothing_and_records_nothing() {
    // 空 target 的 svn status 作用于整个工作副本（AGENTS.md §6.2）：空白名单必须什么都不问
    let fx = hybrid_with_colleague();
    cl(&fx, "outside", "Code/A.cs");
    assert!(read_changelists(&fx.wc_str(), &[]).unwrap().is_empty());
    assert!(app_lib::engine::svn_wcdb::changelists(&fx.wc_str(), &[])
        .unwrap()
        .is_empty());
    let _armed = fault::arm(Point::WcdbRefused);
    assert!(
        read_changelists(&fx.wc_str(), &[]).unwrap().is_empty(),
        "退回 svn 那条路也不能拿空 target 去问整个工作副本"
    );
    record(&fx.wc_str(), &[], "op1");
    assert!(!record_path(&fx.wc_str()).exists());
}

#[test]
fn reading_falls_back_to_svn_when_wc_db_is_not_recognised() {
    let fx = hybrid_with_colleague();
    two_lists(&fx);
    cl(&fx, "unchanged", "Code/B.cs");
    let from_db = read_changelists(&fx.wc_str(), &wl()).unwrap();
    let _armed = fault::arm(Point::WcdbRefused);
    let from_svn = read_changelists(&fx.wc_str(), &wl()).unwrap();
    assert_eq!(from_svn, from_db);
    assert_eq!(
        from_svn,
        pairs(&[("Code/A.cs", "speed"), ("Code/B.cs", "unchanged")])
    );
}

//! SVN 历史缓存的读写与合并。
//!
//! 全是纯本地逻辑 + 文件 IO，**不需要 svn**——所以这些测试在没装 svn 的机器上
//! 也能跑，也比集成测试快一个数量级。缓存正确性的大部分风险都在这里，
//! 不要把它们混进需要 fixture 仓库的测试文件。

use app_lib::engine::svn_log_cache::{self, CachedRev, SvnLogCache, TargetLog};
use app_lib::engine::svn_xml::SvnLogEntry;

fn entry(rev: u64) -> SvnLogEntry {
    SvnLogEntry {
        revision: rev,
        author: "a".into(),
        date: "2026-09-15T00:00:00.000000Z".into(),
        message: format!("r{rev}"),
    }
}

fn cached(rev: u64) -> CachedRev {
    CachedRev {
        rev,
        author: "a".into(),
        date: "2026-09-15T00:00:00.000000Z".into(),
        message: format!("r{rev}"),
    }
}

/// 建一个带 `.git/` 的空目录当仓库根。缓存文件落在 `.git/` 里面。
fn repo_dir() -> tempfile::TempDir {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".git")).unwrap();
    dir
}

#[test]
fn merge_into_a_fresh_target_sets_the_range_directly() {
    let mut t = TargetLog::new();
    svn_log_cache::merge_into(&mut t, &[entry(3), entry(1)], 1, 3);
    assert_eq!(t.covered_min, 1);
    assert_eq!(t.covered_max, 3);
    assert_eq!(t.entries.len(), 2);
}

#[test]
fn merge_extends_the_covered_range_and_dedupes() {
    let mut t = TargetLog::new();
    svn_log_cache::merge_into(&mut t, &[entry(8), entry(7), entry(5)], 5, 8);
    svn_log_cache::merge_into(&mut t, &[entry(10), entry(8)], 8, 10);
    assert_eq!(t.covered_min, 5);
    assert_eq!(t.covered_max, 10);
    let revs: Vec<u64> = t.entries.iter().map(|e| e.rev).collect();
    assert_eq!(revs, vec![10, 8, 7, 5], "降序、无重复");
}

#[test]
fn merge_treats_an_adjacent_range_as_contiguous() {
    // 覆盖 5..8 之后拉 9..10：中间没有洞（8 与 9 相邻），可以并成 5..10。
    let mut t = TargetLog::new();
    svn_log_cache::merge_into(&mut t, &[entry(8), entry(5)], 5, 8);
    svn_log_cache::merge_into(&mut t, &[entry(10), entry(9)], 9, 10);
    assert_eq!(t.covered_min, 5);
    assert_eq!(t.covered_max, 10);
}

#[test]
fn merge_does_not_widen_coverage_across_a_gap() {
    // 已覆盖 5..8，这次只拉了 20..25，中间 9..19 **没拉过**。
    //
    // 把 covered 记成 5..25 会让下一次增量拉取从 26 开始，
    // 9..19 那段历史**永久缺失，而且界面上看不出来**——图上只是少了些节点，
    // 不会有任何报错。所以不相接时整段换成新区间。
    let mut t = TargetLog::new();
    svn_log_cache::merge_into(&mut t, &[entry(8), entry(5)], 5, 8);
    svn_log_cache::merge_into(&mut t, &[entry(25), entry(20)], 20, 25);
    assert_eq!(t.covered_max, 25);
    assert_eq!(
        t.covered_min, 20,
        "下界必须跟到新区间起点，不能假装 5..25 全覆盖"
    );
    // 已经拉到的记录仍然留着，只是不再声称那段区间被覆盖过
    assert_eq!(t.entries.len(), 4);
}

#[test]
fn merge_accepts_a_descending_range_argument() {
    // fetch_range 会用降序区间（`4:1`）拉「最近 N 条」，传进来的 from > to。
    let mut t = TargetLog::new();
    svn_log_cache::merge_into(&mut t, &[entry(4), entry(1)], 4, 1);
    assert_eq!(t.covered_min, 1);
    assert_eq!(t.covered_max, 4);
}

#[test]
fn merge_keeps_the_first_seen_record_for_a_duplicate_revision() {
    let mut t = TargetLog::new();
    svn_log_cache::merge_into(&mut t, &[entry(3)], 3, 3);
    let mut changed = entry(3);
    changed.message = "被改过的 revprop".into();
    svn_log_cache::merge_into(&mut t, &[changed], 3, 3);
    assert_eq!(t.entries.len(), 1);
    assert_eq!(
        t.entries[0].message, "r3",
        "SVN 历史不可变（spec §10.2），已缓存的记录不被后来的覆盖"
    );
}

#[test]
fn a_cache_with_a_different_uuid_is_discarded_whole() {
    // 同一个磁盘路径重新 checkout 到别的仓库之后，旧缓存的 revision 号在新仓库里
    // 指向完全不同的提交。混用会画出一张**看起来完全正常**的假历史。
    let dir = repo_dir();
    let root = dir.path().to_str().unwrap();
    let mut cache = SvnLogCache::new("uuid-A");
    cache.anchors.insert(4, cached(4));
    svn_log_cache::write_cache(root, &cache).unwrap();

    let same = svn_log_cache::read_cache(root, "uuid-A");
    assert_eq!(same.anchors.len(), 1, "同一个 uuid 必须读得回来");

    let other = svn_log_cache::read_cache(root, "uuid-B");
    assert!(other.anchors.is_empty(), "uuid 不匹配必须整份作废");
    assert_eq!(other.uuid, "uuid-B", "作废后要带上新的 uuid，不是旧的");
}

#[test]
fn uuid_comparison_is_case_insensitive() {
    // svn 各处对 UUID 的大小写写法并不一致，大小写敏感比较会让缓存每次都作废,
    // 表现为「每次打开仓库都要重拉全部历史」。
    let dir = repo_dir();
    let root = dir.path().to_str().unwrap();
    let mut cache = SvnLogCache::new("8F3A1C7E-4B21-0410-9D2F-1A7B3C5D9E01");
    cache.anchors.insert(1, cached(1));
    svn_log_cache::write_cache(root, &cache).unwrap();
    let got = svn_log_cache::read_cache(root, "8f3a1c7e-4b21-0410-9d2f-1a7b3c5d9e01");
    assert_eq!(got.anchors.len(), 1);
}

#[test]
fn a_corrupt_cache_file_degrades_to_empty_instead_of_failing() {
    // 缓存是**加速**手段，不是事实来源。坏了就重拉。
    // 让一个加速用的 JSON 把仓库打不开，是本模块最坏的失败形态。
    let dir = repo_dir();
    let root = dir.path().to_str().unwrap();
    std::fs::write(svn_log_cache::cache_path(root), b"{ this is not json").unwrap();
    let got = svn_log_cache::read_cache(root, "uuid-A");
    assert!(got.targets.is_empty() && got.anchors.is_empty());
}

#[test]
fn a_missing_cache_file_is_an_empty_cache_not_an_error() {
    let dir = repo_dir();
    let got = svn_log_cache::read_cache(dir.path().to_str().unwrap(), "uuid-A");
    assert!(got.targets.is_empty() && got.anchors.is_empty());
}

#[test]
fn a_cache_from_a_future_version_is_discarded() {
    // 格式变了之后 serde 可能仍然解析得出来（字段是 default 的），
    // 但语义已经不同。版本号不认识就整份作废，别猜。
    let dir = repo_dir();
    let root = dir.path().to_str().unwrap();
    std::fs::write(
        svn_log_cache::cache_path(root),
        br#"{"version":99,"uuid":"uuid-A","targets":{},"anchors":{"4":{"rev":4,"author":"a","date":"d","message":"m"}}}"#,
    )
    .unwrap();
    let got = svn_log_cache::read_cache(root, "uuid-A");
    assert!(got.anchors.is_empty());
}

#[test]
fn merged_sequence_unions_all_targets_and_anchors_descending() {
    let mut cache = SvnLogCache::new("u");
    let mut a = TargetLog::new();
    svn_log_cache::merge_into(&mut a, &[entry(3), entry(1)], 1, 3);
    let mut b = TargetLog::new();
    svn_log_cache::merge_into(&mut b, &[entry(2), entry(1)], 1, 2);
    cache.targets.insert("url-a".into(), a);
    cache.targets.insert("url-b".into(), b);
    cache.anchors.insert(4, cached(4));

    let seq = svn_log_cache::merged_sequence(&cache);
    let revs: Vec<u64> = seq.iter().map(|e| e.rev).collect();
    assert_eq!(revs, vec![4, 3, 2, 1], "并集、去重、降序");
}

#[test]
fn merged_sequence_on_an_empty_cache_is_empty() {
    assert!(svn_log_cache::merged_sequence(&SvnLogCache::new("u")).is_empty());
}

#[test]
fn write_cache_round_trips_through_json() {
    let dir = repo_dir();
    let root = dir.path().to_str().unwrap();
    let mut cache = SvnLogCache::new("u");
    let mut t = TargetLog::new();
    svn_log_cache::merge_into(&mut t, &[entry(2), entry(1)], 1, 2);
    cache.targets.insert("file:///C:/tmp/repo".into(), t);
    cache.anchors.insert(9, cached(9));
    svn_log_cache::write_cache(root, &cache).unwrap();
    assert_eq!(svn_log_cache::read_cache(root, "u"), cache);
}

#[test]
fn write_cache_leaves_no_temp_file_behind() {
    let dir = repo_dir();
    let root = dir.path().to_str().unwrap();
    svn_log_cache::write_cache(root, &SvnLogCache::new("u")).unwrap();
    let leftovers: Vec<String> = std::fs::read_dir(dir.path().join(".git"))
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.contains(".tmp"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "临时文件必须被 rename 掉：{leftovers:?}"
    );
}

#[test]
fn write_cache_overwrites_an_existing_file() {
    let dir = repo_dir();
    let root = dir.path().to_str().unwrap();
    let mut first = SvnLogCache::new("u");
    first.anchors.insert(1, cached(1));
    svn_log_cache::write_cache(root, &first).unwrap();

    let mut second = SvnLogCache::new("u");
    second.anchors.insert(2, cached(2));
    svn_log_cache::write_cache(root, &second).unwrap();

    let got = svn_log_cache::read_cache(root, "u");
    assert_eq!(got.anchors.len(), 1);
    assert!(got.anchors.contains_key(&2));
}

#[test]
fn the_cache_lives_inside_dot_git() {
    // 工作目录**只多出 .git 一个条目**（spec §3）——多一个文件就多一个被
    // TortoiseSVN 误提交上去的东西。
    let p = svn_log_cache::cache_path("C:/tmp/repo");
    let s = p.to_string_lossy().replace('\\', "/");
    assert!(s.ends_with(".git/mygitclient-svnlog.json"), "实际 {s}");
}

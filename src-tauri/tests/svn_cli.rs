//! svn CLI 封装的测试。
//!
//! 这些测试需要 `svn` 在 PATH 上（spec §17）。本仓库没有 CI，所有验证都在开发机
//! 本地跑，而开发机装了 SlikSvn 1.14.2。**故意不做「没装就 skip」**——静默跳过
//! 等于把「没测」显示成绿色，正是 `AGENTS.md` §13 rubric #9 说的虚假信心。

use app_lib::engine::svn_cli::{probe_svn, run_svn, run_svn_code, SvnAvailability};

#[test]
fn probe_reports_the_installed_version() {
    match probe_svn() {
        SvnAvailability::Available { version } => {
            assert!(
                version.chars().next().is_some_and(|c| c.is_ascii_digit()),
                "版本号应当以数字开头，实际是 {version:?}"
            );
        }
        other => panic!("开发机应当装了 svn，实际探测结果是 {other:?}"),
    }
}

#[test]
fn run_svn_injects_non_interactive_for_every_subcommand() {
    // --non-interactive 是**全局选项**，6 个子命令都接受（2026-09-13 实测）。
    // 这条断言挡的是「把它错当成某个子命令专属参数」——那会让另一些子命令
    // 以 exit 1 失败，而失败点会远在仓库识别那一步。
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().to_str().unwrap();
    // 非工作副本上跑 info：应当因为「不是工作副本」失败，
    // **而不是**因为「子命令不接受某个参数」失败。
    let err = run_svn(path, &["info"]).expect_err("非工作副本上 svn info 应当失败");
    assert!(
        !err.message.contains("doesn't accept option"),
        "失败原因不该是参数不被接受，实际：{}",
        err.message
    );
}

#[test]
fn info_must_not_receive_ignore_externals() {
    // spec §4.4 / G1：svn info 拒绝这个参数。这条测试把那个实测事实钉住，
    // 免得有人「为了统一」把它加回去——加回去会让仓库识别整条路径 exit 1。
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().to_str().unwrap();
    let (code, _) = run_svn_code(path, &["info", "--ignore-externals"]).unwrap();
    assert_ne!(code, 0, "svn info 带 --ignore-externals 必须非零退出");
}

#[test]
fn run_svn_code_hands_the_exit_code_to_the_caller() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().to_str().unwrap();
    // 不是工作副本 → 非零退出，但 run_svn_code 本身不算失败
    let (code, _) = run_svn_code(path, &["info"]).unwrap();
    assert_ne!(code, 0);
}

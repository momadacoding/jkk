//! svn CLI 封装的测试。
//!
//! 这些测试需要 `svn` 在 PATH 上（spec §17）。本仓库没有 CI，所有验证都在开发机
//! 本地跑，而开发机装了 SlikSvn 1.14.2。**故意不做「没装就 skip」**——静默跳过
//! 等于把「没测」显示成绿色，正是 `AGENTS.md` §13 rubric #9 说的虚假信心。

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::svn_cli::{
    path_dirs_from_reg_output, probe_svn, run_svn, run_svn_code, with_global, SvnAvailability,
};
use svn_fixtures::svn_repo;

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
fn with_global_actually_prepends_non_interactive() {
    // 2026-09-13 独立 review：原来只有下面那条「各子命令都不因它失败」的测试，
    // 名字却叫 "injects"。把 with_global 改成原样返回参数，那条**照样全绿**
    // ——它证明的是「svn 接受这个参数」，不是「我们真的加了」。
    //
    // GUI 应用没有终端，svn 一旦要提示输入凭据就会永久挂在那里、占住 Tauri 的
    // 线程池（与 git_cli 的 GIT_TERMINAL_PROMPT=0 同源）。漏掉注入不会有任何
    // 编译期或运行期信号，只会在某个需要认证的仓库上表现为「点了没反应」。
    assert_eq!(
        with_global(&["status", "--xml"]),
        vec!["--non-interactive", "status", "--xml"],
        "全局选项必须排在子命令**之前**"
    );
    // 空参数表也要注入，否则 `svn` 裸跑时仍可能进交互
    assert_eq!(with_global(&[]), vec!["--non-interactive"]);
}

#[test]
fn every_subcommand_accepts_the_injected_global_option() {
    // --non-interactive 是**全局选项**，所有子命令都接受（2026-09-13 实测）。
    // 这条断言挡的是「把它错当成某个子命令专属参数」——那会让另一些子命令
    // 以 exit 1 失败，而失败点会远在仓库识别那一步。
    //
    // 名字说的是 every subcommand，就真的逐个跑一遍：只测 info 一个的话，
    // 名字比覆盖范围大，属 rubric #9 那类虚假信心。
    //
    // 「真的注入了」由上面那条负责——本条只管「注入之后不会把别的子命令搞挂」。
    let fx = svn_repo();
    let wc = fx.wc_str();
    for args in [
        vec!["info", "--xml"],
        vec!["status", "--xml"],
        vec!["log", "--xml", "-l", "1"],
        vec!["diff"],
    ] {
        let out = run_svn_code(&wc, &args).unwrap();
        assert!(
            !out.stderr.contains("doesn't accept option"),
            "svn {args:?} 因参数不被接受而失败，stderr：{}",
            out.stderr
        );
    }
}

#[test]
fn run_svn_wraps_stderr_into_a_git_error() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().to_str().unwrap();
    let err = run_svn(path, &["info"]).expect_err("非工作副本上 svn info 应当失败");
    assert_eq!(err.code, "svn_cli");
    assert!(
        !err.message.trim().is_empty(),
        "错误信息不能是空的——前端只有这一条线索"
    );
}

#[test]
fn info_must_not_receive_ignore_externals() {
    // spec §4.4 / G1：svn info 拒绝这个参数。这条测试把那个实测事实钉住，
    // 免得有人「为了统一」把它加回去——加回去会让仓库识别整条路径 exit 1。
    //
    // ⚠️ **必须在真实工作副本上跑，且必须断言失败的原因。**
    // 初版在一个空 TempDir（非工作副本）上只断言 `code != 0`，那是假绿：
    // 那种目录下**不带**该参数的 `svn info` 同样退出 1（`E155007: is not a
    // working copy`）。就算哪天 svn 开始接受这个参数，那条断言照样通过，
    // 而 G1 那张子命令表已经失效了（`AGENTS.md` §13 rubric #9）。
    let fx = svn_repo();
    let wc = fx.wc_str();

    // 对照组：同一个目录、同一个子命令，不带该参数必须**成功**。
    // 没有这一半，下面那半证明不了任何事。
    let ok = run_svn_code(&wc, &["info", "--xml"]).unwrap();
    assert_eq!(ok.code, 0, "真实工作副本上 svn info 应当成功");

    let bad = run_svn_code(&wc, &["info", "--xml", "--ignore-externals"]).unwrap();
    assert_ne!(bad.code, 0, "svn info 带 --ignore-externals 必须非零退出");
    assert!(
        bad.stderr.contains("doesn't accept option"),
        "失败原因必须是「子命令不接受该选项」，实际 stderr：{}",
        bad.stderr
    );
}

#[test]
fn status_and_update_do_accept_ignore_externals() {
    // G1 那张表的另一半：status 接受它。只测「info 拒绝」证明不了这是一张
    // **按子命令区分**的表——也可能是「svn 根本不认识这个参数」。
    let fx = svn_repo();
    let out = run_svn_code(&fx.wc_str(), &["status", "--xml", "--ignore-externals"]).unwrap();
    assert_eq!(
        out.code, 0,
        "svn status 应当接受该参数，stderr：{}",
        out.stderr
    );
}

#[test]
fn run_svn_code_hands_the_exit_code_and_stderr_to_the_caller() {
    let dir = tempfile::TempDir::new().unwrap();
    let path = dir.path().to_str().unwrap();
    // 不是工作副本 → 非零退出，但 run_svn_code 本身不算失败
    let out = run_svn_code(path, &["info"]).unwrap();
    assert_ne!(out.code, 0);
    // stderr 必须给到调用方：svn 的退出码基本只有 0/1，光看它分不出任何
    // 两种失败——这正是 git_cli::run_git_code 那个形状不能照抄的原因。
    assert!(
        !out.stderr.is_empty(),
        "run_svn_code 必须把 stderr 交给调用方，否则它比 run_svn 提供的信息还少"
    );
}

#[test]
fn reg_output_parsing_keeps_the_first_path_entry() {
    // 这是 2026-09-13 review 抓到的真缺陷：整段 stdout 按 `;` 切时，**第一个
    // chunk 是「注册表键名 + 换行 + `Path REG_EXPAND_SZ ` + 第一个目录」**，
    // trim() 去不掉中间的换行与键名，于是 PATH 的**第一个条目被静默吞掉**。
    //
    // 安装器把自己 prepend 到 PATH 是常见做法；SVN 恰好排第一时，probe_svn()
    // 会报 NotInstalled 并引导「去安装」，而这个函数存在的唯一理由就是要在这种
    // 情况下说「重启应用」——两种引导正好相反。
    //
    // 下面这段是真实 `reg query` 输出的形态（含前导空行与 CRLF）。
    let raw = "\r\nHKEY_LOCAL_MACHINE\\SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment\r\n    Path    REG_EXPAND_SZ    C:\\Program Files\\SlikSvn\\bin;C:\\Windows;C:\\Windows\\system32\r\n\r\n";
    let dirs = path_dirs_from_reg_output(raw);
    assert_eq!(
        dirs,
        vec![
            "C:\\Program Files\\SlikSvn\\bin".to_string(),
            "C:\\Windows".to_string(),
            "C:\\Windows\\system32".to_string(),
        ],
        "第一个条目不能被吞掉"
    );
}

#[test]
fn reg_output_parsing_tolerates_a_missing_value() {
    // 查不到值时 reg 的输出里没有类型标记，必须给出空列表而不是把键名当目录。
    assert!(path_dirs_from_reg_output("ERROR: The system was unable to find").is_empty());
    assert!(path_dirs_from_reg_output("").is_empty());
}

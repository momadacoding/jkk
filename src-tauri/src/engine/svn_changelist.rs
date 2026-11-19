//! 更新、手动跟进、交付前后保住 SVN changelist（计划 `2026-11-20-svn-changelist-keep.md`）。
//!
//! # 为什么要补挂
//!
//! 三个流程都会暂时把白名单里的文件换回 SVN 原版：「更新」「手动跟进」先 stash（只收白名单），
//! 再 checkout 基线 / rebase，最后放回；「交付」切到去掉 Local-Only 提交的交付树。这段时间里，
//! 有本地改动的文件内容与 SVN BASE 完全一样。IDEA / Rider 的 SVN 插件刚好在这时刷新，就把它们当成
//! 改动消失，执行 `svn changelist --remove`（`SvnChangelistListener.changesRemoved`，没有开关）。
//! 改动回来以后 IDE 把它们放进默认列表，而默认列表不挂 SVN changelist，标记就丢了。
//!
//! jkk 自己从不摘 changelist：`svn revert`、`svn update`、文件内容怎么变都不改 wc.db 里的
//! changelist（开工前实测，见决策日志）。所以补救只能是「先记下、事后挂回去」。
//!
//! # 时序
//!
//! ```text
//! 流程内、动磁盘之前  record     记下「文件 → changelist」，写 .git/mygitclient/changelists.json
//! 流程函数返回之后    settle     journal 已删（流程走完或放弃完）才补挂；记下 settledAt，记录留着
//! settle 约 10 秒后   recheck    再补挂一次（IDE 正在进行的那次刷新可能晚于 settle 才 --remove），删记录
//! ```
//!
//! 记录不放 journal：journal 在收尾的好几个地方被删（`svn_sync_in::restore_step_with`），而 recheck
//! 发生在 journal 删掉之后。停在冲突上时不补挂：文件可能仍是原版，挂上也会被 IDE 再摘掉。放在
//! `.git/mygitclient/` 底下：watcher 不把那里的写入当成仓库变化（`commands/watch.rs::is_relevant`）。
//!
//! # 过期
//!
//! settle 过、却没等到 recheck 的记录（10 秒内退出了程序）在 [`STALE_AFTER`] 之后作废，不再补挂：
//! 那之后用户在 IDE 里有意摘掉的 changelist，不该被一份旧记录挂回去（独立 review 第 2 条）。没 settle
//! 过的记录不按时间作废：停在冲突上的流程可能隔几小时才继续，那时补挂正是要做的事。
//!
//! # 补挂谁
//!
//! 记录里的文件，此刻**没有** changelist、**仍有修改**（[`ChangelistStatus::is_modified`]）的，挂回
//! 原来的列表。已经在别的列表里的不动（用户或 IDE 挪过）；不再修改的不挂（交付提交掉的文件，
//! svn commit 本来就摘掉它的 changelist）。
//!
//! # 失败不改变流程的结局
//!
//! 这是附加的修补：记录、补挂失败只写日志，不让一次成功的更新报错。补挂失败时记录留着，recheck 再
//! 试一次；recheck 之后无论成败都删记录，不无限重试。已知会失败的：本机 ACP 不是 936 时，中文文件名
//! 或中文列表名经不了 svn.exe 的命令行（决策日志「开工前实测」）。
//!
//! # 并发
//!
//! **不拿同步锁**：拿了的话，流程结束后的 settle 与 10 秒后的 recheck 各占它一次，这期间钩子触发的
//! 自动跟进、git 面板的提交都会被挡成「正忙」，锁文件的变化还会让界面闪一下同步横幅（独立 review
//! 第 1 条）。改为：
//!
//! - 本模块的三个入口在进程内互斥（[`GUARD`]），读写记录文件不会交错；
//! - [`settle`] / [`recheck`] 看到同步锁被占（[`svn_lock::is_held`] 只探测、不建文件）或 journal 还在，
//!   就跳过：有流程正在跑或停在半路，记录由它收尾时处理。
//!
//! 探测与补挂之间仍可能有流程开始：它先 [`record`]（等 [`GUARD`]），那时补挂已经做完；补挂只对
//! 「有修改、没挂 changelist」的文件发 `svn changelist`，与流程的 stash / svn update 撞上时 svn 自己
//! 按 wc.db 的锁排队。

use crate::engine::svn_cli::{chunk_targets, run_svn, run_svn_fresh, MAX_TARGETS_CMDLINE};
use crate::engine::svn_include_check::normalize_roots;
use crate::engine::svn_journal;
use crate::engine::svn_lock;
use crate::engine::svn_util::now_secs;
use crate::engine::svn_wc::peg_safe;
use crate::engine::svn_wcdb;
use crate::engine::svn_xml::{parse_changelist_status, ChangelistStatus};
use crate::error::{GitError, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

/// `.git/` 底下的相对路径。`mygitclient/` 是客户端私有目录（失焦快照、TortoiseSVN 收件箱也在这里）。
const RECORD_FILE: &str = "mygitclient/changelists.json";

/// [`settle`] 之后多久 [`recheck`]。IDE 一次 VCS 刷新（跑 `svn status`、比对列表、发 `--remove`）
/// 在大工程上是秒级；10 秒覆盖「settle 时那次刷新还在路上」，又短到用户来不及在这期间开始下一次
/// 流程（开始了也没关系，见模块头「并发」）。
pub const RECHECK_DELAY: Duration = Duration::from_secs(10);

/// settle 过的记录多久之后作废（见模块头「过期」）。比 [`RECHECK_DELAY`] 宽出一截，容得下 recheck
/// 线程被调度得晚一些。
pub const STALE_AFTER: Duration = Duration::from_secs(120);

/// 本模块三个入口的进程内互斥（见模块头「并发」）。不分仓库：每次只占几百毫秒。
static GUARD: Mutex<()> = Mutex::new(());

fn guard() -> std::sync::MutexGuard<'static, ()> {
    // 持锁的线程 panic 过也照用：守护的只是一个文件的读写顺序，没有会被打坏的内存状态
    GUARD.lock().unwrap_or_else(|e| e.into_inner())
}

/// 落盘的记录。
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    /// 写下它的那次流程。[`recheck`] 只处理自己那一份，免得删掉下一次流程刚写的记录。
    pub opid: String,
    pub recorded_at: u64,
    /// [`settle`] 补挂的时刻（Unix 秒）。有它的记录在 [`STALE_AFTER`] 之后作废。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settled_at: Option<u64>,
    /// 仓库相对路径（正斜杠）→ changelist 名。
    pub entries: BTreeMap<String, String>,
}

impl Record {
    /// settle 过、而且已经过了 [`STALE_AFTER`]。
    pub fn is_stale(&self, now: u64) -> bool {
        self.settled_at
            .is_some_and(|t| now.saturating_sub(t) > STALE_AFTER.as_secs())
    }
}

/// 一次补挂的结果。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Reattached {
    /// 挂回去的 `(路径, 列表名)`。
    pub reattached: Vec<(String, String)>,
    /// 该挂却没挂上的 `(路径, 列表名)`。
    pub failed: Vec<(String, String)>,
    /// 出错时 svn 的原话（多个列表各自失败时用换行连起来）。
    pub error: Option<String>,
}

/// [`settle`] 补挂过一次：`opid` 交给随后的 [`recheck`]。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settled {
    pub opid: String,
    pub report: Reattached,
}

/// `<repo_root>/.git/mygitclient/changelists.json`。
pub fn record_path(repo_root: &str) -> PathBuf {
    Path::new(repo_root).join(".git").join(RECORD_FILE)
}

/// 读出还能用的记录：过期的删掉、返回 `None`。
fn live_record(repo_root: &str) -> Option<Record> {
    let r = read_record(repo_root)?;
    if r.is_stale(now_secs()) {
        log::info!(
            "changelist 记录（{}）settle 之后没等到核对就过期了，不再补挂",
            r.opid
        );
        remove_record(repo_root);
        return None;
    }
    Some(r)
}

/// 有流程正在跑（同步锁被占）或停在半路（journal 还在）。读不出 journal 也按「在」处理。
fn flow_in_progress(repo_root: &str) -> bool {
    svn_lock::is_held(repo_root) || !matches!(svn_journal::read(repo_root), Ok(None))
}

/// 读记录。没有返回 `None`；坏了也返回 `None` 并删掉它：里面没有别的程序能用的东西，留着只会让
/// 每次 settle 都报同一条日志。
pub fn read_record(repo_root: &str) -> Option<Record> {
    let p = record_path(repo_root);
    let bytes = match std::fs::read(&p) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            log::warn!("changelist 记录读不了（{}）：{e}", p.display());
            return None;
        }
    };
    match serde_json::from_slice(&bytes) {
        Ok(r) => Some(r),
        Err(e) => {
            log::warn!("changelist 记录坏了，删掉（{}）：{e}", p.display());
            let _ = std::fs::remove_file(&p);
            None
        }
    }
}

fn write_record(repo_root: &str, r: &Record) -> Result<()> {
    let text = serde_json::to_string_pretty(r)
        .map_err(|e| GitError::new("svn_changelist_encode", e.to_string()))?;
    crate::engine::session::write_atomic(&record_path(repo_root), &text)
}

fn remove_record(repo_root: &str) {
    match std::fs::remove_file(record_path(repo_root)) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => log::warn!("删不掉 changelist 记录：{e}"),
    }
}

/// 白名单底下挂了 changelist 的文件。先读 wc.db，认不出来时改用 `svn status --xml`。
pub fn read_changelists(repo_root: &str, whitelist: &[String]) -> Result<BTreeMap<String, String>> {
    match svn_wcdb::changelists(repo_root, whitelist) {
        Ok(m) => Ok(m),
        Err(why) => {
            log::info!("changelist 改用 svn status 读：{why}");
            changelists_by_svn(repo_root, whitelist)
        }
    }
}

fn changelists_by_svn(repo_root: &str, whitelist: &[String]) -> Result<BTreeMap<String, String>> {
    let roots = normalize_roots(whitelist);
    if roots.is_empty() {
        // 空 target 的 svn status 会作用于整个工作副本（§12.2）
        return Ok(BTreeMap::new());
    }
    let targets: Vec<String> = roots
        .iter()
        .map(|r| peg_safe(r.trim_end_matches('/')))
        .collect();
    let mut out = BTreeMap::new();
    for st in status(repo_root, &targets)? {
        if let Some(name) = st.changelist {
            out.insert(st.path, name);
        }
    }
    Ok(out)
}

/// `svn status --xml` 读此刻的磁盘（不走扫描缓存：它可能是流程更早时读的）。
fn status(repo_root: &str, targets: &[String]) -> Result<Vec<ChangelistStatus>> {
    let head = ["status", "--xml", "--"];
    let mut out = Vec::new();
    for batch in chunk_targets(&head, targets, MAX_TARGETS_CMDLINE) {
        let mut args: Vec<&str> = head.to_vec();
        args.extend(batch.iter().map(String::as_str));
        out.extend(parse_changelist_status(&run_svn_fresh(repo_root, &args)?)?);
    }
    Ok(out)
}

/// 流程动磁盘之前调用，调用方持同步锁。**不返回错误**：记不下来只是这次没有补挂可做。
///
/// 上一次的记录还在、又没过期时（流程收尾后程序随即崩了、settle 时有别的流程在跑），先把它补挂掉
/// 再换成这一次的：那时文件都已回到流程之后的样子，此刻正是补挂的时机。
pub fn record(repo_root: &str, whitelist: &[String], opid: &str) {
    let _g = guard();
    if let Some(left) = live_record(repo_root) {
        log_report(&reattach(repo_root, &left), "上一次留下的记录");
        remove_record(repo_root);
    }
    let entries = match read_changelists(repo_root, whitelist) {
        Ok(m) => m,
        Err(e) => {
            log::warn!(
                "读不出 changelist，这次流程之后不补挂：{} {}",
                e.code,
                e.message
            );
            return;
        }
    };
    if entries.is_empty() {
        return;
    }
    let r = Record {
        opid: opid.to_string(),
        recorded_at: now_secs(),
        settled_at: None,
        entries,
    };
    if let Err(e) = write_record(repo_root, &r) {
        log::warn!("changelist 记录写不进去，这次流程之后不补挂：{}", e.message);
    }
}

/// 按记录补挂（规则见模块头「补挂谁」）。不拿锁、不动记录文件。
pub fn reattach(repo_root: &str, r: &Record) -> Reattached {
    let mut report = Reattached::default();
    // 已经不在工作副本里的路径照样问：svn status 对它只给一个空的 <target> 和一条 W155010 警告，
    // 退出码仍是 0（2026-11-20 实测，父目录也不在时同样），不会拖累同一批的其他文件
    let present: Vec<&String> = r.entries.keys().collect();
    if present.is_empty() {
        return report;
    }
    let targets: Vec<String> = present.iter().map(|p| peg_safe(p)).collect();
    let now: BTreeMap<String, ChangelistStatus> = match status(repo_root, &targets) {
        Ok(v) => v.into_iter().map(|s| (s.path.clone(), s)).collect(),
        Err(e) => {
            report.failed = present
                .iter()
                .map(|p| ((*p).clone(), r.entries[p.as_str()].clone()))
                .collect();
            report.error = Some(format!("svn status 失败：{}", e.message.trim()));
            return report;
        }
    };
    let mut groups: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for p in present {
        let Some(st) = now.get(p.as_str()) else {
            continue;
        };
        if st.changelist.is_some() || !st.is_modified() {
            continue;
        }
        groups
            .entry(r.entries[p.as_str()].as_str())
            .or_default()
            .push(p.clone());
    }
    let mut errors = Vec::new();
    for (name, paths) in groups {
        let pegged: Vec<String> = paths.iter().map(|p| peg_safe(p)).collect();
        // 列表名放在 `--` 之后：以 `-` 开头的名字不会被当成选项
        let head = ["changelist", "--", name];
        let mut done = 0;
        for batch in chunk_targets(&head, &pegged, MAX_TARGETS_CMDLINE) {
            let mut args: Vec<&str> = head.to_vec();
            args.extend(batch.iter().map(String::as_str));
            let n = batch.len();
            let slice = &paths[done..done + n];
            done += n;
            match run_svn(repo_root, &args) {
                Ok(_) => report
                    .reattached
                    .extend(slice.iter().map(|p| (p.clone(), name.to_string()))),
                Err(e) => {
                    report
                        .failed
                        .extend(slice.iter().map(|p| (p.clone(), name.to_string())));
                    errors.push(format!("{name}：{}", e.message.trim()));
                }
            }
        }
    }
    if !errors.is_empty() {
        report.error = Some(errors.join("\n"));
    }
    report
}

/// 流程函数返回之后调用（命令层）。返回 `Some` 时调用方约 [`RECHECK_DELAY`] 之后调 [`recheck`]。
///
/// 返回 `None`：没有（还能用的）记录；或有流程正在跑 / 停在半路（它收尾时处理这份记录）。
///
/// 第一次 settle 记下 `settledAt`，之后再 settle（下一条命令紧跟着来）不往后推：过期从流程结束算起。
pub fn settle(repo_root: &str) -> Option<Settled> {
    let _g = guard();
    // 先占互斥再读记录：读完到补挂之间，记录不会被别的入口换掉（独立 review 第 3 条）
    if flow_in_progress(repo_root) {
        return None;
    }
    let mut r = live_record(repo_root)?;
    let report = reattach(repo_root, &r);
    log_report(&report, "流程结束");
    if r.settled_at.is_none() {
        r.settled_at = Some(now_secs());
        if let Err(e) = write_record(repo_root, &r) {
            // 记不下 settledAt 只是这份记录不会按时过期，recheck 照样会删它
            log::warn!("changelist 记录写不进 settledAt：{}", e.message);
        }
    }
    Some(Settled {
        opid: r.opid,
        report,
    })
}

/// [`settle`] 之后再补挂一次，然后删记录。只处理 `opid` 那一份记录。
///
/// 返回 `None`：记录已经不是它（下一次流程换掉了）或过期了；或有流程正在跑 / 停在半路——这几种
/// 情况记录都由后来的流程负责。
pub fn recheck(repo_root: &str, opid: &str) -> Option<Reattached> {
    let _g = guard();
    if flow_in_progress(repo_root) {
        return None;
    }
    let r = live_record(repo_root).filter(|r| r.opid == opid)?;
    let report = reattach(repo_root, &r);
    log_report(&report, "延迟核对");
    remove_record(repo_root);
    Some(report)
}

fn log_report(r: &Reattached, when: &str) {
    if !r.reattached.is_empty() {
        log::info!(
            "changelist 补挂（{when}）：{}",
            r.reattached
                .iter()
                .map(|(p, c)| format!("{p} → {c}"))
                .collect::<Vec<_>>()
                .join("、")
        );
    }
    if !r.failed.is_empty() {
        log::warn!(
            "changelist 没能挂回去（{when}）：{}；{}",
            r.failed
                .iter()
                .map(|(p, c)| format!("{p} → {c}"))
                .collect::<Vec<_>>()
                .join("、"),
            r.error.as_deref().unwrap_or("")
        );
    }
}

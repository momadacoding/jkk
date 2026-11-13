//! 性能记录模式：启动参数带 `--perf` 时，把子进程、前端 invoke 与业务阶段的耗时
//! 写进一个 JSONL 文件。
//!
//! 设计见 `docs/superpowers/specs/2026-11-12-performance-monitor-design.md`，
//! 实现偏离见 `docs/superpowers/plans/2026-11-12-performance-monitor-decisions.md`。
//!
//! 关闭时（绝大多数进程）每个入口只多一次 [`OnceLock::get`]，不构造记录、不碰文件。

use serde::Serialize;
use serde_json::{Map, Value};
use std::ffi::OsStr;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// 打开记录模式的启动参数。
pub const FLAG: &str = "--perf";

/// 前端在页面脚本执行前就能读到的开关（决策日志 D1）。
pub const INIT_SCRIPT: &str = "Object.defineProperty(window, '__JKK_PERF__', { value: true });";

/// `args` 里（不含 argv[0]）有没有 [`FLAG`]。
pub fn flag_in<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    args.into_iter()
        .skip(1)
        .any(|a| a.as_ref() == OsStr::new(FLAG))
}

/// 一条记录。字段名就是文件里的键，见设计 §4.2。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Record {
    pub ts: f64,
    pub dur: f64,
    pub src: &'static str,
    pub kind: &'static str,
    pub name: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Map::is_empty")]
    pub meta: Map<String, Value>,
}

impl Record {
    /// Rust 侧的一条记录，`ts` 取当前时刻。
    pub fn rust(kind: &'static str, name: impl Into<String>) -> Self {
        Self {
            ts: now_ms(),
            dur: 0.0,
            src: "rust",
            kind,
            name: name.into(),
            ok: true,
            meta: Map::new(),
        }
    }
}

/// 墙钟时刻，Unix 毫秒带小数。前端用 `performance.timeOrigin + performance.now()`，
/// 同一台机器上两者对得上（设计 §4.2）。
pub fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

/// 一个记录文件。按实例构造，测试不必碰全局状态。
pub struct PerfLog {
    path: PathBuf,
    out: Mutex<BufWriter<File>>,
}

impl PerfLog {
    /// 在 `path` 新建文件（父目录不存在时一并建出）。
    pub fn create(path: PathBuf) -> std::io::Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let file = File::create(&path)?;
        Ok(Self {
            path,
            out: Mutex::new(BufWriter::new(file)),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn write(&self, rec: &Record) {
        if let Ok(line) = serde_json::to_string(rec) {
            self.write_line(&line);
        }
    }

    /// 写前端送来的一批行，返回实际写入的行数。坏行丢弃，见 [`web_line`]。
    ///
    /// 整批只拿一次锁、只 flush 一次：每行一锁一 flush 的话，100 条一批就是 100 次系统调用，
    /// 还和起 git 的线程抢同一把锁，记录本身就出现在它要量的耗时里（review 第 8 条）。
    pub fn append_web(&self, lines: &[String]) -> usize {
        let valid: Vec<String> = lines.iter().filter_map(|l| web_line(l)).collect();
        if valid.is_empty() {
            return 0;
        }
        let mut out = self.out.lock().unwrap_or_else(|e| e.into_inner());
        for line in &valid {
            let _ = writeln!(out, "{line}");
        }
        let _ = out.flush();
        valid.len()
    }

    /// 每行 flush：进程崩溃时已记的不丢。写失败不报错——记录模式出问题不能拖垮被测对象。
    fn write_line(&self, line: &str) {
        let mut out = self.out.lock().unwrap_or_else(|e| e.into_inner());
        let _ = writeln!(out, "{line}").and_then(|()| out.flush());
    }
}

/// 前端能写的记录种类。`proc` / `meta` 只由 Rust 写。
const WEB_KINDS: [&str; 3] = ["invoke", "span", "mark"];

/// 校验前端送来的一行：必须是 JSON 对象、`src == "web"`、`kind` 是前端的种类、
/// 字段类型对；通过的重新序列化成一行（去掉行内换行），不通过返回 `None`。
pub fn web_line(line: &str) -> Option<String> {
    let value: Value = serde_json::from_str(line).ok()?;
    let obj = value.as_object()?;
    let ok = obj.get("src")?.as_str()? == "web"
        && WEB_KINDS.contains(&obj.get("kind")?.as_str()?)
        && obj.get("name")?.as_str()?.len() <= 200
        && obj.get("ts")?.is_number()
        && obj.get("dur")?.is_number()
        && obj.get("ok")?.is_boolean()
        && obj.get("meta").is_none_or(Value::is_object);
    if !ok {
        return None;
    }
    serde_json::to_string(&value).ok()
}

static LOG: OnceLock<PerfLog> = OnceLock::new();

/// 装上全局记录器。只能装一次；已经装过返回 `false`。
pub fn install(log: PerfLog) -> bool {
    LOG.set(log).is_ok()
}

/// 记录模式开着没有。关闭时这就是一次 `OnceLock::get`。
pub fn enabled() -> bool {
    LOG.get().is_some()
}

/// 全局记录器的文件路径。
pub fn current_file() -> Option<&'static Path> {
    LOG.get().map(PerfLog::path)
}

/// 写一条记录；关闭时什么都不做。
pub fn record(rec: &Record) {
    if let Some(log) = LOG.get() {
        log.write(rec);
    }
}

/// 写前端送来的一批行。返回记录器在不在：`false` 时前端关掉自己（`--perf` 带了、
/// 文件却没建成），不再白白计时、每秒发一次 IPC（review 第 2 条）。
pub fn append_web(lines: &[String]) -> bool {
    match LOG.get() {
        Some(log) => {
            log.append_web(lines);
            true
        }
        None => false,
    }
}

/// 记录文件名：`perf-<UTC 年月日>-<UTC 时分秒>Z-<pid>.jsonl`（决策日志 D2）。
pub fn file_name(unix_secs: u64, pid: u32) -> String {
    let days = (unix_secs / 86_400) as i64;
    let rem = unix_secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "perf-{y:04}{m:02}{d:02}-{:02}{:02}{:02}Z-{pid}.jsonl",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// 1970-01-01 起的天数 → 公历年月日（Howard Hinnant 的 `civil_from_days`）。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// 文件头的 `meta` 记录（设计 §4.1）。`extra_args` 是除 argv[0] 与 `--perf` 外的参数个数。
pub fn meta_record(extra_args: usize, dev: bool) -> Record {
    let mut rec = Record::rust("meta", "start");
    rec.meta
        .insert("version".into(), env!("CARGO_PKG_VERSION").into());
    rec.meta.insert(
        "build".into(),
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
        .into(),
    );
    rec.meta.insert("dev".into(), dev.into());
    rec.meta.insert("argc".into(), extra_args.into());
    rec.meta.insert(
        "cpus".into(),
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(0)
            .into(),
    );
    rec.meta.insert("pid".into(), std::process::id().into());
    rec
}

// ---- 子进程 ----

/// 子进程参数脱敏后的样子（设计 §4.5，决策日志 D5）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redacted {
    /// 第一个位置参数；不像子命令时是 `?`。
    pub sub: Option<String>,
    /// 以 `-` 开头的选项：`--key=value` 只留 `--key`，短选项只留前两个字符。
    pub flags: Vec<String>,
    pub argc: usize,
}

/// 后面跟一个值的选项，不论出现在哪。值一律丢掉，即使它以 `-` 开头。
const VALUE_OPTS: [&str; 4] = ["-m", "--message", "-F", "--file"];
/// 只在子命令之前带值的 git 全局选项。
const GLOBAL_VALUE_OPTS: [&str; 4] = ["-c", "-C", "--git-dir", "--work-tree"];

pub fn redact_args(args: &[String]) -> Redacted {
    let mut sub: Option<String> = None;
    let mut flags = Vec::new();
    let mut skip_value = false;
    let mut only_paths = false;
    for a in args {
        if skip_value {
            skip_value = false;
            continue;
        }
        if only_paths {
            continue;
        }
        if a == "--" {
            flags.push(a.clone());
            only_paths = true;
            continue;
        }
        if a.len() > 1 && a.starts_with('-') {
            let (flag, attached) = if a.starts_with("--") {
                match a.split_once('=') {
                    Some((k, _)) => (k.to_string(), true),
                    None => (a.clone(), false),
                }
            } else {
                let head: String = a.chars().take(2).collect();
                let attached = a.chars().count() > 2;
                (head, attached)
            };
            let takes_value = VALUE_OPTS.contains(&flag.as_str())
                || (sub.is_none() && GLOBAL_VALUE_OPTS.contains(&flag.as_str()));
            if takes_value && !attached {
                skip_value = true;
            }
            flags.push(flag);
            continue;
        }
        if sub.is_none() {
            sub = Some(if looks_like_subcommand(a) {
                a.clone()
            } else {
                "?".to_string()
            });
        }
    }
    Redacted {
        sub,
        flags,
        argc: args.len(),
    }
}

fn looks_like_subcommand(a: &str) -> bool {
    !a.is_empty() && a.len() <= 32 && a.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// 程序名：路径最后一段去掉扩展名，小写（`C:\Program Files\Git\cmd\git.exe` → `git`）。
pub fn program_name(program: &OsStr) -> String {
    Path::new(program)
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| "?".to_string())
}

/// 工作目录最后一段，用来区分同时开着的几个 worktree；不记完整路径。
fn repo_name(cmd: &Command) -> Option<String> {
    cmd.get_current_dir()
        .and_then(Path::file_name)
        .map(|s| s.to_string_lossy().into_owned())
}

/// 从 `Command` 摘出记录名和脱敏后的 meta。只在记录模式开着时调用。
pub fn describe(cmd: &Command) -> (String, Map<String, Value>) {
    let program = program_name(cmd.get_program());
    let args: Vec<String> = cmd
        .get_args()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let r = redact_args(&args);
    let name = match &r.sub {
        Some(sub) => format!("{program} {sub}"),
        None => program,
    };
    let mut meta = Map::new();
    meta.insert("flags".into(), r.flags.into());
    meta.insert("argc".into(), r.argc.into());
    if let Some(repo) = repo_name(cmd) {
        meta.insert("repo".into(), repo.into());
    }
    (name, meta)
}

/// 一个子进程从启动到结束的计时。关闭时 [`ProcTimer::start`] 返回 `None`。
pub struct ProcTimer {
    rec: Record,
    started: Instant,
}

impl ProcTimer {
    pub fn start(cmd: &Command) -> Option<Self> {
        enabled().then(|| Self::begin(cmd))
    }

    /// 不看开关，测试用。
    pub fn begin(cmd: &Command) -> Self {
        let (name, meta) = describe(cmd);
        let mut rec = Record::rust("proc", name);
        rec.meta = meta;
        Self {
            rec,
            started: Instant::now(),
        }
    }

    /// 结束计时、得到记录。`Err` 是进程没起来。
    pub fn into_record(self, result: Result<&ExitStatus, &std::io::Error>) -> Record {
        let mut rec = self.rec;
        rec.dur = self.started.elapsed().as_secs_f64() * 1000.0;
        match result {
            Ok(status) => {
                rec.ok = status.success();
                rec.meta.insert("exit".into(), status.code().into());
            }
            Err(_) => {
                rec.ok = false;
                rec.meta.insert("spawnFailed".into(), true.into());
            }
        }
        rec
    }

    /// 结束计时并写进全局记录器。
    pub fn finish(self, result: Result<&ExitStatus, &std::io::Error>) {
        record(&self.into_record(result));
    }
}

/// `run_svn` 缓存命中：不起进程，记一条 `cached: true`、`dur` 为 0 的 `proc`。
pub fn record_cached(cmd: &Command) {
    if !enabled() {
        return;
    }
    let (name, meta) = describe(cmd);
    let mut rec = Record::rust("proc", name);
    rec.meta = meta;
    rec.meta.insert("cached".into(), true.into());
    record(&rec);
}

/// 启动外部程序（编辑器、diff 工具）只记一条 `mark`，不计时：它们的耗时由用户决定。
pub fn record_launch(program: &OsStr) {
    if !enabled() {
        return;
    }
    record(&Record::rust(
        "mark",
        format!("launch {}", program_name(program)),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn flag_is_recognised_anywhere_after_argv0_but_not_as_argv0() {
        assert!(flag_in(["jkk.exe", "--perf"]));
        assert!(flag_in(["jkk.exe", "x", "--perf"]));
        assert!(!flag_in(["jkk.exe"]));
        assert!(!flag_in(["--perf"]));
        assert!(!flag_in(["jkk.exe", "--perf=1", "--perfx", "-perf"]));
    }

    #[test]
    fn record_serialises_with_the_documented_keys() {
        let mut rec = Record::rust("proc", "git status");
        rec.ts = 1.5;
        rec.dur = 2.25;
        rec.ok = false;
        let json = serde_json::to_value(&rec).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "ts": 1.5, "dur": 2.25, "src": "rust", "kind": "proc",
                "name": "git status", "ok": false,
            })
        );
        rec.meta.insert("argc".into(), 3.into());
        let json = serde_json::to_value(&rec).unwrap();
        assert_eq!(json["meta"]["argc"], 3);
    }

    #[test]
    fn file_name_uses_utc_with_z_suffix() {
        assert_eq!(file_name(0, 7), "perf-19700101-000000Z-7.jsonl");
        // 2000-03-01 是第 11017 天；2024-02-29 是第 19782 天（闰日）
        assert_eq!(
            file_name(11_017 * 86_400 + 3_661, 1),
            "perf-20000301-010101Z-1.jsonl"
        );
        assert_eq!(
            file_name(19_782 * 86_400 + 86_399, 42),
            "perf-20240229-235959Z-42.jsonl"
        );
        assert_eq!(
            file_name(19_783 * 86_400, 42),
            "perf-20240301-000000Z-42.jsonl"
        );
    }

    #[test]
    fn web_lines_are_validated_and_normalised() {
        let good = r#"{"ts":1,"dur":2,"src":"web","kind":"invoke","name":"read_status","ok":true,"meta":{"paths":3}}"#;
        let out = web_line(good).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&out).unwrap(),
            serde_json::from_str::<Value>(good).unwrap()
        );
        // 行内换行被去掉：一条记录只占一行
        let multiline = "{\"ts\":1,\"dur\":2,\"src\":\"web\",\n\"kind\":\"mark\",\"name\":\"focus\",\"ok\":true}";
        assert!(!web_line(multiline).unwrap().contains('\n'));
        // 不带 meta 也行
        assert!(
            web_line(r#"{"ts":1,"dur":0,"src":"web","kind":"mark","name":"focus","ok":true}"#)
                .is_some()
        );
        for bad in [
            "",
            "not json",
            "[1,2]",
            r#"{"ts":1,"dur":2,"src":"rust","kind":"invoke","name":"x","ok":true}"#,
            r#"{"ts":1,"dur":2,"src":"web","kind":"proc","name":"x","ok":true}"#,
            r#"{"ts":1,"dur":2,"src":"web","kind":"meta","name":"x","ok":true}"#,
            r#"{"ts":"1","dur":2,"src":"web","kind":"span","name":"x","ok":true}"#,
            r#"{"ts":1,"dur":2,"src":"web","kind":"span","name":"x","ok":1}"#,
            r#"{"ts":1,"dur":2,"src":"web","kind":"span","name":7,"ok":true}"#,
            r#"{"ts":1,"dur":2,"src":"web","kind":"span","name":"x","ok":true,"meta":[1]}"#,
            r#"{"dur":2,"src":"web","kind":"span","name":"x","ok":true}"#,
        ] {
            assert_eq!(web_line(bad), None, "{bad}");
        }
        let long = format!(
            r#"{{"ts":1,"dur":2,"src":"web","kind":"span","name":"{}","ok":true}}"#,
            "x".repeat(201)
        );
        assert_eq!(web_line(&long), None);
    }

    #[test]
    fn append_web_drops_bad_lines_and_keeps_one_record_per_line() {
        let dir = tempfile::tempdir().unwrap();
        let log = PerfLog::create(dir.path().join("nested").join("p.jsonl")).unwrap();
        let lines = s(&[
            "{\"ts\":1,\"dur\":2,\"src\":\"web\",\n\"kind\":\"mark\",\"name\":\"focus\",\"ok\":true}",
            r#"{"ts":1,"dur":2,"src":"rust","kind":"mark","name":"forged","ok":true}"#,
            "garbage",
            r#"{"ts":3,"dur":0,"src":"web","kind":"mark","name":"blur","ok":true}"#,
        ]);
        assert_eq!(log.append_web(&lines), 2);
        log.write(&Record::rust("meta", "start"));
        let text = std::fs::read_to_string(log.path()).unwrap();
        let rows: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(rows.len(), 3, "{text}");
        assert_eq!(rows[0]["name"], "focus");
        assert_eq!(rows[1]["name"], "blur");
        assert_eq!(rows[2]["kind"], "meta");
    }

    /// 单测进程从不装全局记录器：前端靠这个 `false` 关掉自己。
    #[test]
    fn global_append_reports_false_when_no_log_is_installed() {
        assert!(!enabled());
        assert!(!append_web(&s(&[
            r#"{"ts":1,"dur":0,"src":"web","kind":"mark","name":"focus","ok":true}"#
        ])));
    }

    #[test]
    fn meta_record_has_no_paths_or_args() {
        let rec = meta_record(2, true);
        assert_eq!(rec.kind, "meta");
        assert_eq!(rec.meta["argc"], 2);
        assert_eq!(rec.meta["dev"], true);
        assert!(rec.meta["cpus"].as_u64().is_some());
    }

    // ---- 脱敏（设计 §4.5 / 决策日志 D5）----

    #[test]
    fn keeps_program_subcommand_and_flags_and_counts_the_rest() {
        let r = redact_args(&s(&["status", "--porcelain=v2", "-z", "src/secret.rs"]));
        assert_eq!(r.sub.as_deref(), Some("status"));
        assert_eq!(r.flags, s(&["--porcelain", "-z"]));
        assert_eq!(r.argc, 4);
    }

    #[test]
    fn long_option_value_after_equals_is_dropped() {
        let r = redact_args(&s(&["log", "--format=%H %s", "--author=alice@example.com"]));
        assert_eq!(r.flags, s(&["--format", "--author"]));
    }

    #[test]
    fn message_and_file_values_are_dropped_even_when_they_look_like_options() {
        for opt in ["-m", "--message", "-F", "--file"] {
            let r = redact_args(&s(&["commit", opt, "--amend all the things", "-q"]));
            assert_eq!(r.sub.as_deref(), Some("commit"), "{opt}");
            assert_eq!(r.flags, s(&[opt, "-q"]), "{opt}");
        }
        // 值粘在后面的写法：截成选项本身，下一个参数照常处理
        let r = redact_args(&s(&["commit", "-mhello world", "-q"]));
        assert_eq!(r.flags, s(&["-m", "-q"]));
        let r = redact_args(&s(&["commit", "--message=-x secret", "-q"]));
        assert_eq!(r.flags, s(&["--message", "-q"]));
    }

    #[test]
    fn short_options_keep_only_two_characters() {
        let r = redact_args(&s(&["log", "-Spassword", "-uno", "-1"]));
        assert_eq!(r.flags, s(&["-S", "-u", "-1"]));
    }

    #[test]
    fn git_global_options_before_the_subcommand_take_a_value() {
        let r = redact_args(&s(&[
            "-c",
            "core.quotepath=false",
            "-C",
            r"C:\Users\me\repo",
            "--literal-pathspecs",
            "log",
            "-C",
            "HEAD",
        ]));
        assert_eq!(r.sub.as_deref(), Some("log"));
        // 子命令之后的 -C 是不带值的开关，HEAD 只计数
        assert_eq!(r.flags, s(&["-c", "-C", "--literal-pathspecs", "-C"]));
        assert_eq!(r.argc, 8);
    }

    #[test]
    fn everything_after_double_dash_is_only_counted() {
        let r = redact_args(&s(&["add", "--", "-not-a-flag.txt", r"D:\work\x.cs"]));
        assert_eq!(r.flags, s(&["--"]));
        assert_eq!(r.argc, 4);
    }

    #[test]
    fn windows_paths_and_odd_first_arguments_never_become_the_name() {
        let r = redact_args(&s(&[r"C:\Users\me\secret repo\a.txt"]));
        assert_eq!(r.sub.as_deref(), Some("?"));
        let r = redact_args(&s(&["--non-interactive", "status", "--xml", r"E:\proj"]));
        assert_eq!(r.sub.as_deref(), Some("status"));
        assert_eq!(r.flags, s(&["--non-interactive", "--xml"]));
        let r = redact_args(&s(&["中文"]));
        assert_eq!(r.sub.as_deref(), Some("?"));
        let r = redact_args(&[]);
        assert_eq!(r.sub, None);
        assert!(r.flags.is_empty());
    }

    #[test]
    fn non_ascii_short_option_is_cut_on_a_char_boundary() {
        let r = redact_args(&s(&["log", "-中文"]));
        assert_eq!(r.flags, s(&["-中"]));
    }

    #[test]
    fn program_name_is_the_lowercase_file_stem() {
        assert_eq!(program_name(OsStr::new("git")), "git");
        assert_eq!(
            program_name(OsStr::new(r"C:\Program Files\Git\cmd\GIT.EXE")),
            "git"
        );
        assert_eq!(program_name(OsStr::new("code.cmd")), "code");
    }

    #[test]
    fn describe_records_only_the_last_segment_of_the_working_directory() {
        let mut cmd = Command::new("git");
        cmd.current_dir(r"C:\Users\me\secret\my-repo").args([
            "commit",
            "-m",
            "top secret message",
            "--",
            "a.txt",
        ]);
        let (name, meta) = describe(&cmd);
        assert_eq!(name, "git commit");
        assert_eq!(meta["repo"], "my-repo");
        assert_eq!(meta["argc"], 5);
        let dumped = serde_json::to_string(&meta).unwrap();
        for leak in ["secret", "a.txt", "Users"] {
            assert!(!dumped.contains(leak), "{leak} 出现在 {dumped}");
        }
    }

    #[test]
    fn proc_timer_reports_exit_code_and_spawn_failure() {
        let mut cmd = Command::new("definitely-not-a-real-program-xyz");
        cmd.arg("status");
        let err = cmd.output().unwrap_err();
        let rec = ProcTimer::begin(&cmd).into_record(Err(&err));
        assert!(!rec.ok);
        assert_eq!(rec.meta["spawnFailed"], true);
        assert_eq!(rec.name, "definitely-not-a-real-program-xyz status");
    }
}

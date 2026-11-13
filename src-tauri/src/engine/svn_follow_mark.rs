//! 失焦快照、「我是谁」与 wc.db 戳（计划 Task 9；分析 §6.6、§6.10；日志 E19、W26）。
//!
//! 跟进要回答两个问题，这里提供线索：
//!
//! - **这份暂存 / 这个 commit 早于外部提交吗？** 暂存条目的 mtime 不可靠（W26），所以在窗口
//!   失焦时记一份快照 [`FollowMark`]：快照里已有的暂存 blob、快照 HEAD 的祖先，都早于失焦
//!   之后才可能发生的外部提交。只对快照之后才进基线的路径成立（[`FollowMark::base_oid`]）：
//!   基线已发布、分支落后时写的快照比那批 SVN 提交新
//! - **外部提交是我做的吗？** 见 [`my_svn_user`]
//!
//! 两者都只是判定线索。拿不准（没有快照、快照坏了、取不到用户名）时跟进走保守分支：暂存走
//! 三方合并、commit 正常重放，所以这里的失败一律不阻断跟进。
//!
//! [`wc_db_stamps`] 是另一件事：窗口拿到焦点时用来发现外部提交，开销要小到每次焦点都能跑。

use crate::engine::git_cli::run_git_bytes;
use crate::engine::git_read;
use crate::engine::svn_basemap::read_base_map;
use crate::engine::svn_cli::{svn_command, with_global};
use crate::engine::svn_config::{read_config, BASELINE_BRANCH};
use crate::engine::svn_wc::{enumerate_wcs, WcIdentity};
use crate::error::{GitError, Result};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

const MARK_DIR: &str = "mygitclient";
const MARK_FILE: &str = "follow-mark.json";

/// 窗口失焦那一刻的 HEAD 与暂存区。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FollowMark {
    /// unix 秒，只供排查。
    pub at: i64,
    /// unix 毫秒。C1h 起有钩子事件时要与 t_commit 比先后（分析 §6.13.3），秒分不出同一秒内
    /// 的先后。旧格式没有（`None`），按拿不准处理。
    #[serde(default)]
    pub at_ms: Option<i64>,
    pub head_oid: String,
    /// 快照时的 `svn/base`。跟进只把这之后才进基线的路径算成「快照之后的 SVN 提交」：判定 InSync
    /// 时写的快照可以比 SVN 提交新（基线已发布、分支落后）。旧格式没有这个字段（`None`），按
    /// 拿不准处理。
    #[serde(default)]
    pub base_oid: Option<String>,
    /// detached 时为 `None`。
    pub branch: Option<String>,
    /// `仓库相对路径 → 暂存的 blob oid`。暂存的删除记全 0 oid。
    pub staged: BTreeMap<String, String>,
}

impl FollowMark {
    /// oid 以后要进 git 命令行，坏文件里的值不能往下传。
    fn is_well_formed(&self) -> bool {
        is_oid(&self.head_oid)
            && self.base_oid.as_deref().is_none_or(is_oid)
            && self.staged.values().all(|v| is_oid(v))
    }
}

fn is_oid(s: &str) -> bool {
    matches!(s.len(), 40 | 64) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `<repo>/.git/mygitclient/follow-mark.json`。
pub fn mark_path(repo: &str) -> PathBuf {
    Path::new(repo).join(".git").join(MARK_DIR).join(MARK_FILE)
}

/// 记一份快照。HEAD 还解析不出来（没有提交的新仓库）时不写、不报错。
///
/// `diff-index --cached` 只比索引与树，不扫工作区，也不写索引，失焦时跑得起。
pub fn write_mark(repo: &str) -> Result<()> {
    // git 会往上找外层仓库，HEAD 解析得出来不代表这里有 `.git`。这时往下建目录会凭空造出
    // 一个 `.git`，§6.1 随后把目录认成 SvnSuspicious（与 `hold_for_baseline_move` 同一理由）
    if !Path::new(repo).join(".git").is_dir() {
        return Ok(());
    }
    let Some(head_oid) = git_read::resolve(repo, "HEAD^{commit}")? else {
        return Ok(());
    };
    // 在 HEAD 之后读：两次读之间基线前进时，这里偏新，只会把「快照之后」算小，走向拿不准
    let base_oid = git_read::resolve(repo, &format!("refs/heads/{BASELINE_BRANCH}^{{commit}}"))?;
    let branch = git_read::symbolic_head(repo)?
        .as_deref()
        .and_then(|r| r.strip_prefix("refs/heads/"))
        .map(str::to_string);
    // 比的是上面读到的 oid 而不是 `HEAD`：快照里的 HEAD 与暂存必须来自同一个基准。
    // `--ita-invisible-in-index`：否则 i-t-a 被报成 A + 空 blob，与真的暂存了一个空文件同形
    let raw = run_git_bytes(
        repo,
        &[
            "diff-index",
            "--cached",
            "-z",
            "--no-renames",
            "--ita-invisible-in-index",
            &head_oid,
        ],
    )?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok();
    let mark = FollowMark {
        at: now.map_or(0, |d| d.as_secs() as i64),
        at_ms: now.map(|d| d.as_millis() as i64),
        head_oid,
        base_oid,
        branch,
        staged: parse_staged(&raw)?,
    };
    store(&mark_path(repo), &mark)
}

/// 解析 `diff-index -z --no-renames`：`:<mode> <mode> <oid> <oid> <status>\0<path>\0` 重复。
///
/// 冲突条目（`U`，目标 oid 全 0）不记：与「暂存的删除」同形，记进去就成了一条假的删除。
/// 不记等于「快照里没有」，跟进时按拿不准处理；而且索引有冲突时跟进本来就拒绝。
fn parse_staged(raw: &[u8]) -> Result<BTreeMap<String, String>> {
    let bad = || GitError::new("svn_follow_mark", "diff-index 的输出不是预期的格式");
    let mut staged = BTreeMap::new();
    let mut fields = raw.split(|b| *b == 0).filter(|f| !f.is_empty());
    while let Some(meta) = fields.next() {
        let path = fields.next().ok_or_else(bad)?;
        let meta = std::str::from_utf8(meta).map_err(|_| bad())?;
        let cols: Vec<&str> = meta.split(' ').collect();
        let [_, _, _, dst, status] = cols[..] else {
            return Err(bad());
        };
        if status == "U" {
            continue;
        }
        staged.insert(String::from_utf8_lossy(path).into_owned(), dst.to_string());
    }
    Ok(staged)
}

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// 临时文件 + rename：读到的要么是旧快照要么是新快照。不 `sync_all`：失焦很频繁，断电后
/// 读到空文件只会按没有快照处理。
fn store(path: &Path, mark: &FollowMark) -> Result<()> {
    let dir = path.parent().expect("快照路径一定有父目录");
    std::fs::create_dir_all(dir)?;
    let body = serde_json::to_vec(mark)
        .map_err(|e| GitError::new("svn_follow_mark", format!("快照序列化失败: {e}")))?;
    // 失焦快照不在同步锁里写，名字要在并发下也不重复
    let tmp = dir.join(format!(
        "{MARK_FILE}.{}.{}.tmp",
        std::process::id(),
        TMP_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    if let Err(e) = std::fs::write(&tmp, &body) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        GitError::new("io", format!("替换失焦快照失败: {e}"))
    })
}

/// 读快照。没有文件、读不了、内容坏了都是 `Ok(None)`：快照只是判定线索，拿不准就走保守
/// 分支，不能挡住跟进。这一点与 journal 相反（journal 坏了必须报错，里面有用户的 stash）。
pub fn read_mark(repo: &str) -> Result<Option<FollowMark>> {
    let path = mark_path(repo);
    let raw = match std::fs::read(&path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            log::warn!("失焦快照读不了，按没有快照处理：{e}");
            return Ok(None);
        }
    };
    match serde_json::from_slice::<FollowMark>(&raw) {
        Ok(mark) if mark.is_well_formed() => Ok(Some(mark)),
        Ok(_) => {
            log::warn!("失焦快照里的 oid 不合法，按没有快照处理");
            Ok(None)
        }
        Err(e) => {
            log::warn!("失焦快照坏了，按没有快照处理：{e}");
            Ok(None)
        }
    }
}

/// 删掉快照。删不掉只记 warn：留下的快照说的仍是真话（这份暂存、这些 commit 确实早于下一次
/// 外部提交），不会让跟进判错。
pub fn clear_mark(repo: &str) {
    match std::fs::remove_file(mark_path(repo)) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => log::warn!("失焦快照删不掉：{e}"),
    }
}

/// 我的 SVN 用户名。`cfg.svn_user` 优先，否则看 `svn auth`；取不到返回 `None`，调用方把所有
/// 作者都当成「不是我」（日志【待决】D4）。
pub fn my_svn_user(repo: &str, repos_root: &str) -> Option<String> {
    let configured = match read_config(repo) {
        Ok(cfg) => cfg.svn_user,
        Err(e) => {
            log::warn!(
                "读配置失败，「我的 SVN 用户名」退回 svn auth：{}",
                e.message
            );
            None
        }
    };
    resolve_svn_user(configured.as_deref(), repos_root, || {
        svn_auth_listing(repo, None)
    })
}

/// 「我的 SVN 用户名」的两个来源，分开给界面：配了的不问，探测到的预填，都没有才要用户填。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SvnUserInfo {
    /// 配置里的 `svnUser`。全空白算没配，与 [`resolve_svn_user`] 同一口径。
    pub configured: Option<String>,
    /// `svn auth` 探测到的，与配置无关。基线读不出来时不知道服务器是哪台，也是 `None`。
    pub detected: Option<String>,
}

/// 读 [`SvnUserInfo`]。配置读不了就报错，这里不像 [`my_svn_user`] 那样退回探测：调用方要写回的
/// 正是这份配置。
pub fn svn_user_info(repo: &str) -> Result<SvnUserInfo> {
    let configured = read_config(repo)?
        .svn_user
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty());
    let spec = format!("refs/heads/{BASELINE_BRANCH}");
    let detected = read_base_map(repo, &spec)
        .ok()
        .and_then(|map| resolve_svn_user(None, &map.repos_root, || svn_auth_listing(repo, None)));
    Ok(SvnUserInfo {
        configured,
        detected,
    })
}

/// [`my_svn_user`] 的判定部分。`auth` 给 `svn auth` 的输出，只在真的需要时才调：配置里有值，
/// 或者仓库根没有 realm（`file://`）时不去碰用户的凭据缓存。
pub fn resolve_svn_user(
    configured: Option<&str>,
    repos_root: &str,
    auth: impl FnOnce() -> Option<String>,
) -> Option<String> {
    if let Some(me) = configured.map(str::trim).filter(|s| !s.is_empty()) {
        return Some(me.to_string());
    }
    Origin::of_url(repos_root)?;
    parse_svn_auth(&auth()?, repos_root)
}

/// 跑 `svn auth` 列出凭据缓存的摘要。不带 `--show-passwords`，密码在输出里只是 `[not shown]`；
/// 不把输出写进日志（AGENTS.md §11）。`config_dir` 只给测试指向假的缓存，生产传 `None`。
///
/// **不能走 `run_svn`：** 标签走 gettext，`Authentication realm:` 在德语环境是
/// `Anmeldebereich:`，中文系统同理（Task 9 实测）。那样解析不到任何东西，也不报错，所以必须
/// 强制 C locale（实测压得住 `LANGUAGE`）。
pub fn svn_auth_listing(repo: &str, config_dir: Option<&Path>) -> Option<String> {
    let mut cmd = svn_command(repo);
    cmd.args(with_global(&["auth"]));
    if let Some(dir) = config_dir {
        cmd.arg("--config-dir").arg(dir);
    }
    cmd.env("LC_ALL", "C").stdin(Stdio::null());
    match crate::engine::proc::output(&mut cmd) {
        Ok(out) if out.status.success() => Some(String::from_utf8_lossy(&out.stdout).into_owned()),
        Ok(out) => {
            log::warn!("svn auth 退出码 {:?}，取不到用户名", out.status.code());
            None
        }
        Err(e) => {
            log::warn!("svn auth 跑不起来，取不到用户名：{e}");
            None
        }
    }
}

/// 从 `svn auth` 的输出里找 `repos_root` 所在服务器的用户名。
///
/// 只认 `Authentication realm:` 与 `Username:` 两类行，`Username:` 只算在它所属的那条凭据上。
/// 匹配到多个不同的用户名时返回 `None`，不猜：猜错了，别人的提交会被当成自己的而让位。
///
/// 非 ASCII 用户名同样返回 `None`：`svn auth` 经管道输出的是系统代码页（GBK），到这里已经
/// 被 lossy 成 U+FFFD，与 XML 里 UTF-8 的 author 对不上。这种用户要在配置里写 `svnUser`。
pub fn parse_svn_auth(text: &str, repos_root: &str) -> Option<String> {
    let want = Origin::of_url(repos_root)?;
    let mut found: Option<&str> = None;
    let mut in_matching_realm = false;
    for line in text.lines() {
        if let Some(realm) = line.strip_prefix("Authentication realm:") {
            in_matching_realm = Origin::of_realm(realm).is_some_and(|o| o.same_server(&want));
        } else if let Some(user) = line.strip_prefix("Username:") {
            let user = user.trim();
            if !in_matching_realm || user.is_empty() {
                continue;
            }
            if user.contains(char::REPLACEMENT_CHARACTER) {
                return None;
            }
            match found {
                Some(prev) if prev != user => return None,
                _ => found = Some(user),
            }
        }
    }
    found.map(str::to_string)
}

/// URL 或 realm 里的 scheme + host + 端口。scheme 与 host 已转小写。
#[derive(Debug, PartialEq, Eq)]
struct Origin {
    scheme: String,
    host: String,
    port: Option<u16>,
}

impl Origin {
    /// `file://` 没有服务器，也就没有 realm，返回 `None`。
    fn of_url(url: &str) -> Option<Self> {
        let (scheme, rest) = url.split_once("://")?;
        let scheme = scheme.to_ascii_lowercase();
        if scheme == "file" {
            return None;
        }
        let authority = rest.split(['/', '?', '#']).next()?;
        // `svn+ssh://zhang@host/…`：realm 里没有用户名这一段
        let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
        let (host, port) = match host_port.strip_prefix('[') {
            // IPv6 字面量里有冒号，不能按最后一个冒号切
            Some(v6) => {
                let (host, tail) = v6.split_once(']')?;
                (host, tail.strip_prefix(':'))
            }
            None => match host_port.rsplit_once(':') {
                Some((host, port)) => (host, Some(port)),
                None => (host_port, None),
            },
        };
        if host.is_empty() {
            return None;
        }
        let port = match port {
            Some("") | None => None,
            Some(p) => Some(p.parse().ok()?),
        };
        Some(Self {
            scheme,
            host: host.to_ascii_lowercase(),
            port,
        })
    }

    /// realm 形如 `<svn://host:3690> uuid`、`<https://host:443> Realm`，`svn+ssh` 的
    /// `svn.username` 凭据是不带尖括号的 `svn+ssh://host`（实测）。
    fn of_realm(realm: &str) -> Option<Self> {
        let realm = realm.trim();
        let url = match realm.strip_prefix('<') {
            Some(rest) => rest.split_once('>')?.0,
            None => realm.split_whitespace().next()?,
        };
        Self::of_url(url)
    }

    fn same_server(&self, other: &Self) -> bool {
        self.scheme == other.scheme
            && self.host == other.host
            && match (self.port, other.port) {
                (Some(a), Some(b)) => a == b,
                (None, None) => true,
                (Some(p), None) | (None, Some(p)) => default_ports(&self.scheme).contains(&p),
            }
    }
}

/// 省略端口时等价于哪些端口。`svn+ssh` 的 realm 里写的是 ra_svn 的 3690 还是 ssh 的 22 没有
/// 实测（开发机没有这种服务器），两个都认：host 已经对上，这里不是判别力所在。
fn default_ports(scheme: &str) -> &'static [u16] {
    match scheme {
        "http" => &[80],
        "https" => &[443],
        "svn" => &[3690],
        s if s.starts_with("svn+") => &[22, 3690],
        _ => &[],
    }
}

/// 一个工作副本的 `wc.db` 的一次 `stat`。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WcDbStamp {
    /// SVN 工作副本根的绝对路径，正斜杠。可以在仓库外（external 指向外部目录）。
    pub wc_root: String,
    /// **IPC 上是十进制字符串。** 当代的纳秒时间戳约 1.8e18，远超 JS 的 2^53，作为 JSON 数字
    /// 到前端会被舍入到 256ns 的倍数，而前端只拿它比相等。
    #[serde(serialize_with = "decimal_string")]
    pub mtime_ns: u128,
    pub size: u64,
}

fn decimal_string<S: serde::Serializer>(v: &u128, s: S) -> std::result::Result<S::Ok, S::Error> {
    s.collect_str(v)
}

/// 各 wc(P) 的 `wc.db` 戳，同一个 wc.db 只返回一条，读不到的跳过。
///
/// wc.db 的 mtime + size 能当信号是因为 jkk 自己发的只读 svn 命令都不写它（E19）；
/// `wc.db-journal` 常驻 0 字节，看它没有用。
pub fn wc_db_stamps(repo: &str) -> Result<Vec<WcDbStamp>> {
    wc_db_stamps_with(repo, enumerate_wcs)
}

/// 一个 wc.db 的位置。
#[derive(Debug, Clone)]
struct WcDb {
    wc_root: String,
    db: PathBuf,
}

/// 上次枚举的结果，以及枚举完那一刻的戳。
#[derive(Debug, Clone)]
struct KnownWcs {
    whitelist: Vec<String>,
    dbs: Vec<WcDb>,
    stamps: Vec<WcDbStamp>,
}

fn known_wcs() -> &'static Mutex<HashMap<String, KnownWcs>> {
    static KNOWN: OnceLock<Mutex<HashMap<String, KnownWcs>>> = OnceLock::new();
    KNOWN.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 同 [`wc_db_stamps`]，枚举函数由调用方给。
///
/// # 为什么缓存枚举结果
///
/// `enumerate_wcs` 每个 wc 发一次 `svn info` 和一次 `svn status`，每次焦点都跑太贵。只从
/// 白名单往上找 `.svn` 又看不到白名单里的 external：往 external 提交只写它自己的 wc.db，
/// 焦点检测会漏掉，正是 Review Focus 1 的形态。所以枚举一次记住位置，之后只 `stat`。
///
/// **失效条件是「有 wc.db 变了」或白名单变了。** external 的增删要经过 `svn update` 或改
/// `svn:externals`，都会写父工作副本的 wc.db，不用单独盯。这次重新枚举与前端随后的第二档
/// 校验同一时机，本来就要发 svn 命令。
///
/// 枚举失败（svn 被占用、不在 PATH）时不缓存，用上次的清单回答，没有就从白名单往上找；
/// 下一次焦点再试。缓存只在进程内，重启后第一次焦点重新枚举。
pub fn wc_db_stamps_with(
    repo: &str,
    enumerate: impl FnOnce(&str, &[String]) -> Result<Vec<WcIdentity>>,
) -> Result<Vec<WcDbStamp>> {
    let cfg = read_config(repo)?;
    let known = known_wcs()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(repo)
        .filter(|k| k.whitelist == cfg.whitelist)
        .cloned();
    if let Some(k) = &known {
        let now = stat_all(&k.dbs);
        if now == k.stamps {
            return Ok(now);
        }
    }
    let wcs = match enumerate(repo, &cfg.whitelist) {
        Ok(wcs) => wcs,
        Err(e) => {
            log::warn!("枚举工作副本失败，这次用已知的 wc.db 位置：{}", e.message);
            let dbs = known.map_or_else(|| dbs_of_whitelist(repo, &cfg.whitelist), |k| k.dbs);
            return Ok(stat_all(&dbs));
        }
    };
    let dbs = dbs_of_wcs(repo, &wcs);
    let stamps = stat_all(&dbs);
    known_wcs()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(
            repo.to_string(),
            KnownWcs {
                whitelist: cfg.whitelist,
                dbs,
                stamps: stamps.clone(),
            },
        );
    Ok(stamps)
}

fn dbs_of_wcs(repo: &str, wcs: &[WcIdentity]) -> Vec<WcDb> {
    let found = wcs.iter().filter_map(|wc| {
        let abspath = wc.wc_root_abspath.as_deref().map(Path::new);
        abspath
            .and_then(wc_db_at)
            .or_else(|| find_wc_db(&Path::new(repo).join(&wc.wc_root)))
    });
    dedupe(found)
}

/// 没有 external 信息时的退路：只看得到白名单所在的工作副本。
fn dbs_of_whitelist(repo: &str, whitelist: &[String]) -> Vec<WcDb> {
    dedupe(
        whitelist
            .iter()
            .filter_map(|w| find_wc_db(&Path::new(repo).join(w))),
    )
}

/// `wc_root` 排序去重：两条白名单可以落在同一个工作副本里。
fn dedupe(dbs: impl Iterator<Item = WcDb>) -> Vec<WcDb> {
    let by_root: BTreeMap<String, WcDb> = dbs.map(|d| (d.wc_root.clone(), d)).collect();
    by_root.into_values().collect()
}

fn wc_db_at(dir: &Path) -> Option<WcDb> {
    let db = dir.join(".svn").join("wc.db");
    db.is_file().then(|| WcDb {
        wc_root: dir
            .to_string_lossy()
            .replace('\\', "/")
            .trim_end_matches('/')
            .to_string(),
        db,
    })
}

/// 从 `start` 往上找最近的 `.svn/wc.db`。SVN 1.7 起只有工作副本根有 `.svn`。
fn find_wc_db(start: &Path) -> Option<WcDb> {
    start.ancestors().find_map(wc_db_at)
}

fn stat_all(dbs: &[WcDb]) -> Vec<WcDbStamp> {
    dbs.iter().filter_map(stat_one).collect()
}

fn stat_one(wc: &WcDb) -> Option<WcDbStamp> {
    let read = || -> std::io::Result<(u128, u64)> {
        let meta = std::fs::metadata(&wc.db)?;
        let mtime = meta
            .modified()?
            .duration_since(UNIX_EPOCH)
            .map_err(std::io::Error::other)?;
        Ok((mtime.as_nanos(), meta.len()))
    };
    match read() {
        Ok((mtime_ns, size)) => Some(WcDbStamp {
            wc_root: wc.wc_root.clone(),
            mtime_ns,
            size,
        }),
        Err(e) => {
            log::warn!("读不到 {} 的 mtime，跳过：{e}", wc.db.display());
            None
        }
    }
}

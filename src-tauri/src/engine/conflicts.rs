//! 冲突文件的信息读取与分类（spec §4.1）。
//!
//! stage、mode、blob、二进制判定、冲突标记、与 HEAD 是否相同都走 libgit2；
//! 生效的属性和 `merge.*` 配置走 git CLI（`check-attr` / `config`），因为真正执行合并的是
//! git CLI，判定要和它的解析结果一致（spec §5）。

use crate::engine::git_cli::{run_git_code, run_git_stdin};
use crate::error::{GitError, Result};
use git2::{DiffOptions, IndexEntry, Repository};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StageInfo {
    pub oid: String,
    pub mode: u32,
}

/// 五种情况，按 spec §4.1 的判定顺序：⑤ → ③ → ④ → ① → ②。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConflictCase {
    /// ⑤ AU / UA / DD，或任一 stage 不是普通文件
    Structural,
    /// ③ DU / UD
    DeleteModify,
    /// ④ git 没做逐行文本合并
    NotTextMerged,
    /// ① 工作区里有冲突标记
    Markers,
    /// ② 以上都不是
    NoMarkers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NotTextReason {
    /// `merge=binary` 或 `-merge`
    BinaryAttr,
    /// `merge=<自定义名字>`，且本机配置里有这个名字
    CustomDriver,
    /// 属性没写，`merge.default` 选了非文本的合并方式
    MergeDefault,
    /// `filter=lfs`
    Lfs,
    /// stage 1 / 2 / 3 里任意一份是二进制（④c）
    BinaryContent,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictInfo {
    pub path: String,
    /// `UU` / `AA` / `DU` / `UD` / `AU` / `UA` / `DD`，与 `git status --short` 一致
    pub code: String,
    pub base: Option<StageInfo>,
    pub ours: Option<StageInfo>,
    pub theirs: Option<StageInfo>,
    pub case: ConflictCase,
    pub reason: Option<NotTextReason>,
    /// 自定义 driver 的名字（CustomDriver，或 `merge.default` 指向自定义名字时）
    pub driver: Option<String>,
    /// 工作区（经过过滤器）与 HEAD 完全相同。④ 用它提示「还没合并」
    pub same_as_head: bool,
}

/// 工作区文件里有没有冲突标记（spec §4.1「冲突标记的判定」）。
///
/// 只认 `<` / `>` 标记行：行首**至少** `marker_size` 个同一种字符，后面跟空格或行尾。
/// 用「至少」是因为 git 遇到改名冲突会把标记加长；只出现一行也算，用户删掉一半标记时
/// 照样拦住。孤立的 `=` / `|` 行不算——Markdown / RST 的标题下划线就是这个样子。
pub fn scan_markers(bytes: &[u8], marker_size: usize) -> bool {
    let size = marker_size.max(1);
    bytes.split(|&b| b == b'\n').any(|line| {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let Some(&first) = line.first() else {
            return false;
        };
        if first != b'<' && first != b'>' {
            return false;
        }
        let run = line.iter().take_while(|&&b| b == first).count();
        run >= size && matches!(line.get(run), None | Some(b' '))
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathAttrs {
    /// `check-attr` 的原始值：`unspecified` / `set` / `unset` / 具体名字
    pub merge: String,
    pub filter: String,
    pub marker_size: usize,
}

const DEFAULT_MARKER_SIZE: usize = 7;

impl Default for PathAttrs {
    fn default() -> Self {
        Self {
            merge: "unspecified".into(),
            filter: "unspecified".into(),
            marker_size: DEFAULT_MARKER_SIZE,
        }
    }
}

/// 查这些路径**生效的** `merge`、`filter`、`conflict-marker-size` 属性。
///
/// 路径从 stdin 喂进去而不是放在命令行上：冲突文件可能有几百个，Windows 的命令行
/// 长度有上限。`-z` 让路径里有什么字符都不影响解析。在 `repo`（可能是 linked worktree）
/// 里执行，`info/attributes`、子目录覆盖、`[attr]` 宏都由 git 算好。
pub fn path_attrs(repo: &str, paths: &[&str]) -> Result<HashMap<String, PathAttrs>> {
    let mut out = HashMap::new();
    if paths.is_empty() {
        return Ok(out);
    }
    let mut input = Vec::new();
    for p in paths {
        input.extend_from_slice(p.as_bytes());
        input.push(0);
    }
    let text = run_git_stdin(
        repo,
        &[
            "check-attr",
            "--stdin",
            "-z",
            "merge",
            "filter",
            "conflict-marker-size",
        ],
        &input,
    )?;
    // 输出是 `<path>\0<attr>\0<value>\0` 的三元组
    let fields: Vec<&str> = text.split('\0').collect();
    for triple in fields.chunks_exact(3) {
        let entry: &mut PathAttrs = out.entry(triple[0].to_string()).or_default();
        match triple[1] {
            "merge" => entry.merge = triple[2].to_string(),
            "filter" => entry.filter = triple[2].to_string(),
            "conflict-marker-size" => {
                entry.marker_size = triple[2].parse().unwrap_or(DEFAULT_MARKER_SIZE)
            }
            _ => {}
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DriverConfig {
    pub driver: Option<String>,
    /// 有 `driver` 以外的键（`name`、`recursive`……）
    pub has_other_keys: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergeConfig {
    pub default: Option<String>,
    /// key 是 `merge.<名字>.*` 里的名字，保留大小写（git 按大小写敏感匹配属性值）
    pub drivers: HashMap<String, DriverConfig>,
}

/// `git config -z --get-regexp ^merge\.` 的原始输出；零匹配（退出码 1）返回空串。
///
/// 必须用 `run_git_code`：零匹配是正常结果，`run_git` 会把退出码 1 当成失败。
pub fn merge_config_raw(repo: &str) -> Result<String> {
    let (code, out) = run_git_code(repo, &["config", "-z", "--get-regexp", r"^merge\."])?;
    match code {
        0 => Ok(out),
        1 => Ok(String::new()),
        n => Err(GitError::new(
            "git_cli",
            format!("读取 merge.* 配置失败（git config 退出码 {n}）"),
        )),
    }
}

pub fn merge_config(repo: &str) -> Result<MergeConfig> {
    Ok(parse_merge_config(&merge_config_raw(repo)?))
}

/// 每条记录是 `key\nvalue\0`，没有值的键是 `key\0`。同一个键出现多次时后面的覆盖前面的，
/// 和 git 读配置的语义一致。
fn parse_merge_config(raw: &str) -> MergeConfig {
    let mut cfg = MergeConfig::default();
    for rec in raw.split('\0').filter(|r| !r.is_empty()) {
        let (key, value) = match rec.split_once('\n') {
            Some((k, v)) => (k, Some(v)),
            None => (rec, None),
        };
        let Some(rest) = key.strip_prefix("merge.") else {
            continue;
        };
        // 名字里可以有点：`merge.a.b.driver` 的名字是 `a.b`，变量名永远在最后一段
        match rest.rsplit_once('.') {
            Some((name, var)) => {
                let entry = cfg.drivers.entry(name.to_string()).or_default();
                match (var, value) {
                    ("driver", Some(v)) => entry.driver = Some(v.to_string()),
                    _ => entry.has_other_keys = true,
                }
            }
            None if rest == "default" => cfg.default = value.map(str::to_string),
            None => {}
        }
    }
    cfg
}

/// git 实际会用哪种方式合并这个路径，照搬 git `ll-merge.c` 的 `find_ll_merge_driver`：
/// `merge` → 文本；`-merge` → binary；没写 → 看 `merge.default`；写了名字 → 先找配置里的
/// 自定义 driver，再找内置的 text / binary / union，都没有就退回文本合并。
enum Resolved {
    Text,
    Binary { via_default: bool },
    Custom { name: String, via_default: bool },
}

fn resolve_driver(attr_merge: &str, cfg: &MergeConfig) -> Resolved {
    let (name, via_default) = match attr_merge {
        "set" => return Resolved::Text,
        "unset" => return Resolved::Binary { via_default: false },
        "unspecified" => match &cfg.default {
            None => return Resolved::Text,
            Some(d) => (d.as_str(), true),
        },
        other => (other, false),
    };
    if cfg.drivers.contains_key(name) {
        return Resolved::Custom {
            name: name.to_string(),
            via_default,
        };
    }
    match name {
        "binary" => Resolved::Binary { via_default },
        // text、union，以及本机完全没配的名字：git 退回逐行文本合并（spec §4.1 表 1 第 3 行）
        _ => Resolved::Text,
    }
}

fn stage(entry: &Option<IndexEntry>) -> Option<StageInfo> {
    entry.as_ref().map(|e| StageInfo {
        oid: e.id.to_string(),
        mode: e.mode,
    })
}

fn is_regular_mode(mode: u32) -> bool {
    mode == 0o100644 || mode == 0o100755
}

fn conflict_code(base: bool, ours: bool, theirs: bool) -> &'static str {
    match (base, ours, theirs) {
        (true, true, true) => "UU",
        (false, true, true) => "AA",
        (true, true, false) => "UD",
        (true, false, true) => "DU",
        (false, true, false) => "AU",
        (false, false, true) => "UA",
        _ => "DD",
    }
}

/// 沿用 git 的判定：前 8000 字节里有 NUL 就是二进制。
fn is_binary_blob(repo: &Repository, info: &Option<StageInfo>) -> Result<bool> {
    let Some(info) = info else {
        return Ok(false);
    };
    let blob = repo.find_blob(git2::Oid::from_str(&info.oid)?)?;
    let content = blob.content();
    Ok(content[..content.len().min(8000)].contains(&0))
}

fn workdir_abs(workdir: &Path, rel: &str) -> PathBuf {
    let mut abs = workdir.to_path_buf();
    abs.extend(rel.split('/'));
    abs
}

/// 工作区里这个路径的状态：不存在、普通文件（附内容）、别的东西（目录、符号链接）。
enum Workdir {
    Missing,
    File(Vec<u8>),
    Other,
}

fn read_workdir(abs: &Path) -> Result<Workdir> {
    // 不跟随链接：工作区里被换成链接或目录的路径，内容不能当作合并结果来看
    match std::fs::symlink_metadata(abs) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Workdir::Missing),
        Err(e) => Err(e.into()),
        Ok(m) if m.file_type().is_file() => Ok(Workdir::File(std::fs::read(abs)?)),
        Ok(_) => Ok(Workdir::Other),
    }
}

/// 这些路径里，工作区（经过过滤器）与 HEAD 完全相同的那些。
///
/// 用 tree 对比工作区（不经过 index）：带上 index 时 libgit2 对冲突条目只给一个
/// `Conflicted`、不比较内容；不经过 index 则按普通改动比较，并且会套用 autocrlf 等过滤器，
/// 不会因为工作区是 CRLF 就判成不同。HEAD 里没有的路径永远不算相同。
fn same_as_head_paths(repo: &Repository, paths: &[&str]) -> Result<HashSet<String>> {
    let mut same = HashSet::new();
    if paths.is_empty() {
        return Ok(same);
    }
    let Ok(head) = repo.head().and_then(|h| h.peel_to_tree()) else {
        return Ok(same);
    };
    let mut opts = DiffOptions::new();
    opts.disable_pathspec_match(true);
    for p in paths {
        opts.pathspec(*p);
    }
    let diff = repo.diff_tree_to_workdir(Some(&head), Some(&mut opts))?;
    let changed: HashSet<String> = diff
        .deltas()
        .flat_map(|d| [d.old_file().path(), d.new_file().path()])
        .flatten()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .collect();
    for p in paths {
        if !changed.contains(*p) && head.get_path(Path::new(p)).is_ok() {
            same.insert(p.to_string());
        }
    }
    Ok(same)
}

/// 读出 index 里所有冲突路径，并按 spec §4.1 分成五种情况。没有冲突时返回空列表。
pub fn read_conflicts(path: &str) -> Result<Vec<ConflictInfo>> {
    let repo = Repository::open(path)?;
    let index = repo.index()?;
    if !index.has_conflicts() {
        return Ok(Vec::new());
    }
    let workdir = repo
        .workdir()
        .ok_or_else(|| GitError::new("bare_repo", "裸仓库没有工作区"))?
        .to_path_buf();

    let mut list = Vec::new();
    for conflict in index.conflicts()? {
        let conflict = conflict?;
        let any = conflict
            .our
            .as_ref()
            .or(conflict.their.as_ref())
            .or(conflict.ancestor.as_ref())
            .expect("冲突条目至少有一个 stage");
        let (base, ours, theirs) = (
            stage(&conflict.ancestor),
            stage(&conflict.our),
            stage(&conflict.their),
        );
        let code = conflict_code(base.is_some(), ours.is_some(), theirs.is_some());
        let all_regular = [&base, &ours, &theirs]
            .into_iter()
            .flatten()
            .all(|s| is_regular_mode(s.mode));
        let case = if !all_regular || matches!(code, "AU" | "UA" | "DD") {
            ConflictCase::Structural
        } else if matches!(code, "DU" | "UD") {
            ConflictCase::DeleteModify
        } else {
            // UU / AA 的细分要等属性和工作区读完，先占个位
            ConflictCase::NoMarkers
        };
        list.push(ConflictInfo {
            path: String::from_utf8_lossy(&any.path).into_owned(),
            code: code.to_string(),
            base,
            ours,
            theirs,
            case,
            reason: None,
            driver: None,
            same_as_head: false,
        });
    }
    list.sort_by(|a, b| a.path.cmp(&b.path));

    let text_paths: Vec<String> = list
        .iter()
        .filter(|c| c.case == ConflictCase::NoMarkers)
        .map(|c| c.path.clone())
        .collect();
    if text_paths.is_empty() {
        return Ok(list);
    }
    let text_refs: Vec<&str> = text_paths.iter().map(String::as_str).collect();
    let attrs = path_attrs(path, &text_refs)?;
    let cfg = merge_config(path)?;
    let same = same_as_head_paths(&repo, &text_refs)?;

    for c in list
        .iter_mut()
        .filter(|c| c.case == ConflictCase::NoMarkers)
    {
        let attr = attrs.get(&c.path).cloned().unwrap_or_default();
        c.same_as_head = same.contains(&c.path);
        if let Some((reason, driver)) = not_text_reason(&repo, c, &attr, &cfg)? {
            c.case = ConflictCase::NotTextMerged;
            c.reason = Some(reason);
            c.driver = driver;
            continue;
        }
        c.case = match read_workdir(&workdir_abs(&workdir, &c.path))? {
            Workdir::File(bytes) if scan_markers(&bytes, attr.marker_size) => ConflictCase::Markers,
            // 用户删掉了工作区文件：没有标记，② 的 diff 会如实显示成删除
            Workdir::File(_) | Workdir::Missing => ConflictCase::NoMarkers,
            // 工作区里被换成了目录或链接：已经不是「内容合并」的问题
            Workdir::Other => ConflictCase::Structural,
        };
    }
    Ok(list)
}

/// ④ 的判定（spec §4.1）。二进制内容排在最前：④c 不给外部 merge 入口，不能被 ④a 盖掉。
fn not_text_reason(
    repo: &Repository,
    c: &ConflictInfo,
    attr: &PathAttrs,
    cfg: &MergeConfig,
) -> Result<Option<(NotTextReason, Option<String>)>> {
    for s in [&c.base, &c.ours, &c.theirs] {
        if is_binary_blob(repo, s)? {
            return Ok(Some((NotTextReason::BinaryContent, None)));
        }
    }
    match resolve_driver(&attr.merge, cfg) {
        Resolved::Binary { via_default: false } => {
            return Ok(Some((NotTextReason::BinaryAttr, None)))
        }
        Resolved::Binary { via_default: true } => {
            return Ok(Some((NotTextReason::MergeDefault, None)))
        }
        Resolved::Custom {
            name,
            via_default: false,
        } => return Ok(Some((NotTextReason::CustomDriver, Some(name)))),
        Resolved::Custom {
            name,
            via_default: true,
        } => return Ok(Some((NotTextReason::MergeDefault, Some(name)))),
        Resolved::Text => {}
    }
    // LFS 冲突后工作区里是什么没法确定（取决于本机有没有装 lfs、smudge 是否成功），
    // 按最保守的 ④ 处理
    if attr.filter == "lfs" {
        return Ok(Some((NotTextReason::Lfs, None)));
    }
    Ok(None)
}

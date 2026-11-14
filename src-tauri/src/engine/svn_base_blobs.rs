//! 跟进录制时，本地改过的文件「服务器版本的 blob」缓存。
//!
//! 录基线时，本地改过的白名单文件要换成 `svn cat -r BASE` 的内容（R2b），每个文件两个进程。
//! 每天整体 update 一次之后，长期改着没提交的文件每次跟进都要重取一遍，几百个文件就是好几秒；
//! 公司机器上起进程更慢。可它们的服务器版本大多没变。
//!
//! 这里记下每次取到的 blob，以及取的时候决定它的全部输入。下一次输入完全相同、blob 还在对象库
//! 里，就直接用。
//!
//! # 输入有哪些
//!
//! `svn cat -r BASE` 这一步由 [`BaseKey`] 决定：URL、仓库 UUID、pristine 校验和、最后修改的
//! revision / 作者 / 时间。只比 revision 不够（独立 review，`tests/svn_follow_reuse_safety.rs`
//! 实测）：目录被复制回旧版本、`svn switch` 到分支之后，同一个路径是另一个节点，最后修改的
//! revision 可能比上次还早，内容却不同。
//!
//! `hash-object --path` 这一步由 git 的过滤规则决定：
//! - 每个路径实际生效的 `text` / `eol` / `crlf` / `filter` / `ident` / `working-tree-encoding`，
//!   用 `git check-attr` 问（工作区的 `.gitattributes`、`.git/info/attributes`、全局
//!   `core.attributesFile` 都算进去，口径与 `hash-object` 相同；libgit2 的属性解析与 git CLI
//!   不保证一致，所以不用它）；
//! - 配置里的 `core.autocrlf` / `core.eol` / `core.checkRoundtripEncoding` 与全部 `filter.*`，
//!   存成 [`config_stamp`]，变了整份作废。
//!
//! 已知没覆盖的：`$Date$` 按本机时区展开，换了时区之后沿用的是旧时区的展开。「重建基线」不读
//! 缓存，可以纠正。
//!
//! # 只是缓存
//!
//! 读不出、对不上、写失败都只是退回 `svn cat`，不影响录出的内容。文件在 `.git` 里，跟进的准备
//! 阶段写它：它不是仓库状态，准备阶段「只建对象、不动 ref」的约定不受影响。

use crate::engine::git_cli::run_git_stdin;
use crate::engine::svn_cli::{bypass_scan_cache, run_svn_targets};
use crate::engine::svn_wc::peg_safe;
use crate::engine::svn_xml::{parse_info_base_keys, BaseKey};
use crate::error::{GitError, Result};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// 缓存文件名，放在 `.git/` 下。
pub const CACHE_FILE: &str = "mygitclient-base-blobs.json";
const VERSION: u32 = 2;

/// 决定 `hash-object --path` 输出的属性。
const FILTER_ATTRS: [&str; 6] = [
    "text",
    "eol",
    "crlf",
    "filter",
    "ident",
    "working-tree-encoding",
];

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CacheFile {
    version: u32,
    stamp: String,
    entries: BTreeMap<String, Cached>,
}

/// 一个文件的全部输入。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Inputs {
    key: BaseKey,
    /// `git check-attr` 给这个路径的 [`FILTER_ATTRS`]，`名=值` 按固定顺序拼起来。
    attrs: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cached {
    #[serde(flatten)]
    inputs: Inputs,
    oid: String,
}

/// 一次录制用的：这批文件此刻的输入，和其中能直接用的 blob。
///
/// 命中在 [`BaseBlobs::load`] 里一次算好：取数是多线程的，`git2::Repository` 不能跨线程共享。
pub struct BaseBlobs {
    file: PathBuf,
    stamp: String,
    inputs: BTreeMap<String, Inputs>,
    hits: BTreeMap<String, String>,
}

impl BaseBlobs {
    /// 读 `paths`（仓库相对、正斜杠）此刻的输入和缓存。`paths` 为空、或者任何一步读不出来时
    /// 返回 `None`：照常 `svn cat`，也不写缓存。
    pub fn load(repo_root: &str, paths: &[&str]) -> Option<Self> {
        if paths.is_empty() {
            return None;
        }
        let inputs = match read_inputs(repo_root, paths) {
            Ok(i) => i,
            Err(e) => {
                log::warn!("读不出脏文件的 BASE 信息，全部重新 svn cat：{}", e.message);
                return None;
            }
        };
        let repo = git2::Repository::open(repo_root).ok()?;
        let file = repo.path().join(CACHE_FILE);
        let stamp = config_stamp(&repo);
        let cached = std::fs::read_to_string(&file)
            .ok()
            .and_then(|t| serde_json::from_str::<CacheFile>(&t).ok())
            .filter(|c| c.version == VERSION && c.stamp == stamp)
            .map(|c| c.entries)
            .unwrap_or_default();
        let odb = repo.odb().ok()?;
        // 输入与上次取的时候完全相同、对象还在（没被 gc 掉）才算命中
        let hits = cached
            .into_iter()
            .filter(|(path, c)| inputs.get(path) == Some(&c.inputs))
            .filter(|(_, c)| git2::Oid::from_str(&c.oid).is_ok_and(|o| odb.exists(o)))
            .map(|(path, c)| (path, c.oid))
            .collect();
        Some(Self {
            file,
            stamp,
            inputs,
            hits,
        })
    }

    /// 能直接用的 blob。
    pub fn reusable(&self, path: &str) -> Option<String> {
        self.hits.get(path).cloned()
    }

    /// 把这次每个文件用的 blob 存下来，整份覆盖：不在这次里的（已经提交、或者不再改着的）丢掉，
    /// 文件不会越攒越大。写失败只记日志。
    ///
    /// 存之前绕过扫描缓存再读一次输入，只存前后相同的：[`BaseBlobs::load`] 与 `svn cat` 之间
    /// TortoiseSVN 正好 update 了这个文件的话，取到的内容配的不是读到的那份输入，存下来就是一条
    /// 错的缓存，将来 BASE 回到那份输入时会被用上。再读不出来就整份不写，留着上次的。
    pub fn save(&self, repo_root: &str, used: &[(&str, &str)]) {
        let paths: Vec<&str> = used.iter().map(|(p, _)| *p).collect();
        let now = match bypass_scan_cache(|| read_inputs(repo_root, &paths)) {
            Ok(i) => i,
            Err(e) => {
                log::warn!(
                    "录完之后读不出 BASE 信息，这次不写 blob 缓存：{}",
                    e.message
                );
                return;
            }
        };
        let entries: BTreeMap<String, Cached> = used
            .iter()
            .filter_map(|(path, oid)| {
                let inputs = self.inputs.get(*path)?;
                (now.get(*path) == Some(inputs)).then(|| {
                    (
                        (*path).to_string(),
                        Cached {
                            inputs: inputs.clone(),
                            oid: (*oid).to_string(),
                        },
                    )
                })
            })
            .collect();
        let doc = CacheFile {
            version: VERSION,
            stamp: self.stamp.clone(),
            entries,
        };
        let written = serde_json::to_string(&doc)
            .map_err(|e| GitError::new("svn_base_blobs", e.to_string()))
            .and_then(|t| crate::engine::session::write_atomic(&self.file, &t));
        if let Err(e) = written {
            log::warn!(
                "写不进脏文件 blob 缓存，下次跟进照常 svn cat：{}",
                e.message
            );
        }
    }
}

/// 每个路径的输入。说不清的路径（[`parse_info_base_keys`] 给 `None` 的）不在结果里。
fn read_inputs(repo_root: &str, paths: &[&str]) -> Result<BTreeMap<String, Inputs>> {
    let keys = read_keys(repo_root, paths)?;
    let mut attrs = read_attrs(repo_root, paths)?;
    Ok(keys
        .into_iter()
        .filter_map(|(path, key)| {
            let attrs = attrs.remove(&path)?;
            Some((path, Inputs { key, attrs }))
        })
        .collect())
}

/// `svn info --xml` 一次（按命令行长度分批）取全部路径的键。entry 按传入顺序给出（实测 M1），
/// 每条的路径必须与传入的对得上，对不上就整批不要。
fn read_keys(repo_root: &str, paths: &[&str]) -> Result<BTreeMap<String, BaseKey>> {
    let args: Vec<String> = paths.iter().map(|p| peg_safe(p)).collect();
    let mut entries = Vec::new();
    for xml in run_svn_targets(repo_root, &["info", "--xml", "--"], &args)? {
        entries.extend(parse_info_base_keys(&xml)?);
    }
    let mismatch = || {
        GitError::new(
            "svn_base_blobs",
            format!(
                "svn info 对 {} 个文件返回的 {} 条结果对不回各自的路径",
                paths.len(),
                entries.len()
            ),
        )
    };
    if entries.len() != paths.len() {
        return Err(mismatch());
    }
    let mut out = BTreeMap::new();
    for (p, (got, key)) in paths.iter().zip(&entries) {
        if got != p {
            return Err(mismatch());
        }
        if let Some(key) = key {
            out.insert((*p).to_string(), key.clone());
        }
    }
    Ok(out)
}

/// `git check-attr -z --stdin` 一次取全部路径的 [`FILTER_ATTRS`]。
///
/// 输出是 `路径\0属性\0值\0` 的三元组，同一路径的属性按命令行上的顺序给出。
fn read_attrs(repo_root: &str, paths: &[&str]) -> Result<BTreeMap<String, String>> {
    let mut stdin = Vec::new();
    for p in paths {
        stdin.extend_from_slice(p.as_bytes());
        stdin.push(0);
    }
    let mut args = vec!["check-attr", "-z", "--stdin"];
    args.extend(FILTER_ATTRS);
    let out = run_git_stdin(repo_root, &args, &stdin)?;
    let mut attrs: BTreeMap<String, String> = BTreeMap::new();
    let fields: Vec<&str> = out.split('\0').collect();
    for [path, name, value] in fields.as_chunks::<3>().0 {
        let line = attrs.entry((*path).to_string()).or_default();
        line.push_str(&format!("{name}={value}\n"));
    }
    Ok(attrs)
}

/// 影响 `hash-object --path` 的配置的原文。读不出的项当成空。
fn config_stamp(repo: &git2::Repository) -> String {
    let Ok(cfg) = repo.config() else {
        return String::new();
    };
    let mut lines: Vec<String> = Vec::new();
    for name in ["core.autocrlf", "core.eol", "core.checkroundtripencoding"] {
        lines.push(format!(
            "{name}={}",
            cfg.get_string(name).unwrap_or_default()
        ));
    }
    if let Ok(mut entries) = cfg.entries(Some("filter\\..*")) {
        while let Some(Ok(e)) = entries.next() {
            lines.push(format!(
                "{}={}",
                e.name().unwrap_or_default(),
                e.value().unwrap_or_default()
            ));
        }
    }
    lines.join("\n")
}

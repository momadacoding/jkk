//! 跟进录制时，本地改过的文件「服务器版本的 blob」缓存。
//!
//! 录基线时，本地改过的白名单文件要换成 `svn cat -r BASE` 的内容（R2b），每个文件两个进程。
//! 每天整体 update 一次之后，长期改着没提交的文件每次跟进都要重取一遍，几百个文件就是好几秒；
//! 公司机器上起进程更慢。可它们的服务器版本大多没变。
//!
//! 这里记下每次取到的 blob 和取的时候 BASE 的 [`BaseKey`]（URL、仓库 UUID、最后修改 revision、
//! pristine 校验和）。下一次 BASE 的键完全相同、blob 还在对象库里，就直接用。
//!
//! # 为什么键要这么多
//!
//! 只比 revision 不够（独立 review 第 1、2 条，`tests/svn_follow_reuse_safety.rs` 实测）：目录被
//! 复制回旧版本、`svn switch` 到分支之后，同一个路径是另一个节点，最后修改的 revision 可能
//! 比旧基线还早，内容却不同。校验和管字节，URL / UUID 管关键字里的 `$HeadURL$` 和「是不是同一个
//! 地方」，revision 管 `$Rev$` / `$Author$` / `$Date$`。eol 与 keywords 属性只能随提交改变。
//!
//! `svn cat` 之后还有一步 `hash-object --path`，它按 git 的过滤规则（`.git/info/attributes`、
//! `core.autocrlf` / `core.eol`）转换。这几样的原文存成 [`stamp`]，变了整份缓存作废。
//! 工作区里的 `.gitattributes` 不在其中：已迁移的仓库由 `.git/info/attributes` 钉住 text / eol
//! （`svn_attributes`），它的优先级最高。
//!
//! # 只是缓存
//!
//! 读不出、对不上、写失败都只是退回 `svn cat`，不影响录出的内容。文件在 `.git` 里，跟进的准备
//! 阶段写它：它不是仓库状态，准备阶段「只建对象、不动 ref」的约定不受影响。

use crate::engine::git_read;
use crate::engine::svn_cli::run_svn_targets;
use crate::engine::svn_wc::peg_safe;
use crate::engine::svn_xml::{parse_info_base_keys, BaseKey};
use crate::error::Result;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// 缓存文件名，放在 `.git/` 下。
pub const CACHE_FILE: &str = "mygitclient-base-blobs.json";
const VERSION: u32 = 1;

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CacheFile {
    version: u32,
    stamp: String,
    entries: BTreeMap<String, Cached>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cached {
    key: BaseKey,
    oid: String,
}

/// 一次录制用的：这批文件此刻 BASE 的键，和其中能直接用的 blob。
///
/// 命中在 [`BaseBlobs::load`] 里一次算好：取数是多线程的，`git2::Repository` 不能跨线程共享。
pub struct BaseBlobs {
    file: PathBuf,
    stamp: String,
    keys: BTreeMap<String, BaseKey>,
    hits: BTreeMap<String, String>,
}

impl BaseBlobs {
    /// 读 `paths`（仓库相对、正斜杠）此刻 BASE 的键和缓存。`paths` 为空、或者任何一步读不出来时
    /// 返回 `None`：照常 `svn cat`，也不写缓存。
    pub fn load(repo_root: &str, paths: &[&str]) -> Option<Self> {
        if paths.is_empty() {
            return None;
        }
        let keys = match read_keys(repo_root, paths) {
            Ok(k) => k,
            Err(e) => {
                log::warn!("读不出脏文件的 BASE 信息，全部重新 svn cat：{}", e.message);
                return None;
            }
        };
        let repo = git2::Repository::open(repo_root).ok()?;
        let file = git_read::git_dir(repo_root).ok()?.join(CACHE_FILE);
        let stamp = stamp(&repo, &file);
        let cached = std::fs::read_to_string(&file)
            .ok()
            .and_then(|t| serde_json::from_str::<CacheFile>(&t).ok())
            .filter(|c| c.version == VERSION && c.stamp == stamp)
            .map(|c| c.entries)
            .unwrap_or_default();
        let odb = repo.odb().ok()?;
        // BASE 的键与上次取的时候完全相同、对象还在（没被 gc 掉）才算命中
        let hits = cached
            .into_iter()
            .filter(|(path, c)| keys.get(path) == Some(&c.key))
            .filter(|(_, c)| git2::Oid::from_str(&c.oid).is_ok_and(|o| odb.exists(o)))
            .map(|(path, c)| (path, c.oid))
            .collect();
        Some(Self {
            file,
            stamp,
            keys,
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
    /// 存之前再读一次键，只存前后相同的：[`BaseBlobs::load`] 读键与 `svn cat` 之间 TortoiseSVN
    /// 正好 update 了这个文件的话，取到的内容配的不是读到的那个键，存下来就是一条错的缓存，
    /// 将来 BASE 回到那个键时会被用上。再读不出来就整份不写，留着上次的。
    pub fn save(&self, repo_root: &str, used: &[(&str, &str)]) {
        let paths: Vec<&str> = used.iter().map(|(p, _)| *p).collect();
        let now = match read_keys(repo_root, &paths) {
            Ok(k) => k,
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
                let key = self.keys.get(*path)?.clone();
                if now.get(*path) != Some(&key) {
                    return None;
                }
                Some((
                    (*path).to_string(),
                    Cached {
                        key,
                        oid: (*oid).to_string(),
                    },
                ))
            })
            .collect();
        let doc = CacheFile {
            version: VERSION,
            stamp: self.stamp.clone(),
            entries,
        };
        let written = serde_json::to_string(&doc)
            .map_err(|e| crate::error::GitError::new("svn_base_blobs", e.to_string()))
            .and_then(|t| crate::engine::session::write_atomic(&self.file, &t));
        if let Err(e) = written {
            log::warn!(
                "写不进脏文件 blob 缓存，下次跟进照常 svn cat：{}",
                e.message
            );
        }
    }
}

/// `svn info --xml` 一次（按命令行长度分批）取全部路径的键。entry 按传入顺序对回（实测 M1），
/// 条数对不上就整批不要。
fn read_keys(repo_root: &str, paths: &[&str]) -> Result<BTreeMap<String, BaseKey>> {
    let args: Vec<String> = paths.iter().map(|p| peg_safe(p)).collect();
    let mut entries = Vec::new();
    for xml in run_svn_targets(repo_root, &["info", "--xml", "--"], &args)? {
        entries.extend(parse_info_base_keys(&xml)?);
    }
    if entries.len() != paths.len() {
        return Err(crate::error::GitError::new(
            "svn_base_blobs",
            format!(
                "svn info 对 {} 个文件返回了 {} 条结果",
                paths.len(),
                entries.len()
            ),
        ));
    }
    Ok(paths
        .iter()
        .zip(entries)
        .filter_map(|(p, (_, key))| Some(((*p).to_string(), key?)))
        .collect())
}

/// `hash-object --path` 用的过滤规则的原文。读不出的项当成空。
fn stamp(repo: &git2::Repository, cache_file: &std::path::Path) -> String {
    let cfg = repo.config().ok();
    let get = |name: &str| {
        cfg.as_ref()
            .and_then(|c| c.get_string(name).ok())
            .unwrap_or_default()
    };
    let attributes = cache_file
        .parent()
        .map(|g| g.join("info").join("attributes"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_default();
    format!(
        "autocrlf={}\neol={}\nattributes=\n{}",
        get("core.autocrlf"),
        get("core.eol"),
        attributes
    )
}

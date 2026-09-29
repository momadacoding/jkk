//! 写前 operation journal（spec §7.6）。`sync_in` 与 `sync_out` 共用。
//!
//! # 「写前」是字面意思
//!
//! 任何一步修改落地**之前**，journal 必须已经在磁盘上。spec §7.3.1 记了反例：
//! 原来的步骤顺序把 `git stash push` 排在写 journal 之前，崩在那条缝里，用户的
//! WIP 就变成一个匿名 stash 条目躺在 reflog 里，客户端重开后对它一无所知。
//!
//! # 所有权由锁界定
//!
//! 拿到 `svn_lock` 才允许读写本文件（§5.1.1）。没有这条，两个进程会互相覆盖
//! journal，被覆盖那一方的 stash OID 全部丢失。本模块**不自己去拿锁**——
//! 那会让「谁持有锁」这件事散到两处；调用方（`sync_in` / `sync_out`）负责。
//!
//! 例外是**只读**的 [`read`]：UI 要在没拿锁的情况下显示「上次同步未完成」
//! （spec §5.1「读操作不阻塞」）。读到的可能是另一个进程正在推进的状态，
//! 所以它的结论只能用来显示，不能用来决定动作。

use crate::error::{GitError, Result};
use std::io::Write;
use std::path::{Path, PathBuf};

const JOURNAL_FILE: &str = "mygitclient-sync.json";

/// 哪条流程留下的 journal。恢复入口据此分流（§7.6：`publishing-baseline` 是两个
/// 流程共用的阶段名，但父提交与返回入口完全不同，不能套用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Flow {
    #[default]
    SyncIn,
    SyncOut,
}

/// 流程停在哪一步。字符串**逐字**取自 spec §7.6 的两张表，不要「优化」拼写：
/// 它是落在用户磁盘上的线序格式，改一个字符就读不回上一版写的 journal。
/// `tests/svn_journal.rs::every_phase_uses_the_exact_string_from_the_spec` 钉着。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
    // —— sync_in ——
    #[default]
    Start,
    Stashing,
    Stashed,
    Rebase,
    /// S10：正在 / 已经把 stash 取回来（计划 A2，spec §7.6 本期补上）。
    ///
    /// 没有它的话，S9 成功之后只剩 `rebase`，崩溃重开时分不清 stash 有没有
    /// apply 过——再 apply 一次会撞上自己留下的改动。配合 [`Journal::stash_applied`]。
    ApplyingStash,
    // —— 两个流程共用的录制阶段 ——
    CapturingBaseline,
    PublishingBaseline,
    BaselineReady,
    // —— sync_out ——
    Prepare,
    BuildingDeliver,
    DeliverBuilt,
    CheckedOut,
    SvnStaged,
    Committing,
    PostCommit,
    RestoringBranch,
}

/// journal 的全部字段（spec §7.6 的 JSON 样例）。
///
/// `sync_out` 的大部分字段现在就定义：本模块拥有这个文件格式，等 Phase 3 再加
/// 会让两期写出的 journal 互相读不了。**空值一律 `skip_serializing_if`**，
/// 让 `sync_in` 写出来的文件保持干净可读。
///
/// # ⚠️ `default` 只加在可选字段上，**绝不加在容器上**
///
/// 2026-09-25 独立 review 抓到：容器级 `#[serde(default)]` 会让**缺字段**的
/// JSON 静默读成默认值。实测 `{"opid":"x","flow":"sync_out","startedAt":7}`
/// 读回来是 `phase: Start`——而界面对 `start` 说的是「还没有动过工作目录里的
/// 任何东西，可以直接当作没发生过」。对一次实际停在 `svn-staged`（`.svn` 里
/// 挂着 add / delete 登记）的流程说这句话，是界面能说出的最有害的一句。
///
/// 下面四个「脊梁」字段没有 `skip_serializing_if`，所以我们写出去的 journal
/// 一定带着它们；读的时候缺任何一个都应当是**硬错误**，走
/// `svn_journal_corrupt` 那条路，与模块开头那条「三种结果必须分开」一致。
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Journal {
    // —— 脊梁四件：缺一即 corrupt，不得有 default ——
    pub opid: String,
    pub flow: Flow,
    pub phase: Phase,
    pub started_at: u64,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orig_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orig_head: Option<String>,
    /// push **之前**的 `refs/stash`。负责区分「push 没落地」和「push 落地了但
    /// reflog 被外部 `stash clear` 抹掉了」——后者要报错停下，不能当成没事
    /// 发生（§7.3.1）。`None` 表示当时还没有 stash。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stash_ref_before: Option<String>,
    /// **不可变 OID**，绝不是 `stash@{N}`（§7.6.1：那是栈位置，不是身份。
    /// 任何一次新的 stash 都会把 `stash@{0}` 挤走，按位置恢复会应用到别人的 WIP）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stash_oid: Option<String>,
    /// S10 的 `git stash apply` 已经执行过（成功或留下冲突）。
    ///
    /// 为真时「继续」只做 drop，**不再 apply**。在 apply 返回之后、drop 之前落盘；
    /// 崩在这两步之间时它仍是 false，继续会再 apply 一次——2026-09-25 实测 git
    /// 此时拒绝（「would be overwritten」）且不动任何文件，所以最坏是一条报错。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stash_applied: bool,
    /// S3 的 checkout 即将执行（或已执行）：HEAD 可能已经离开过原分支。
    ///
    /// 没有可压的改动时 phase 一路停在 `stashed` 走过 S3–S5，单看 phase 分不出
    /// 「HEAD 从没离开」与「去过基线又回来了」，而只有后者会留下 checkout 残留
    /// （PR #49 review #2）。在 checkout **之前**落盘：崩在两步之间时它为真而 HEAD 没动，
    /// 最坏是多做一次残留比对。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub left_orig: bool,
    /// 往回走（回退 / 放弃）时进入的 `applying-stash`：分支已回到更新前的位置。
    ///
    /// `applying-stash` 两个方向共用，继续 / 放弃的结局与横幅文案却完全不同——往回走时
    /// 基线（至少对这条分支）没有推进，报 `Done` 或「分支已在新基线上」都是假话
    /// （PR #49 review #3）。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub rolling_back: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_rev_before: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_oid_before: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_tree: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_baseline_oid: Option<String>,

    // —— 以下只有 sync_out 用（§7.6 末尾三段）——
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub svn_added: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub svn_deleted: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub new_parent_dirs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub local_only_commits: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deliver_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temp_worktree: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_start_rev: Option<u64>,
}

/// `<repo_root>/.git/mygitclient-sync.json`。与 `svn_lock::lock_path` 同一约定。
pub fn journal_path(repo_root: &str) -> PathBuf {
    Path::new(repo_root).join(".git").join(JOURNAL_FILE)
}

/// 原子写入：临时文件 → `sync_all` → rename（§7.6：「半个 JSON 比没有 JSON 更糟」）。
///
/// 临时文件名带 pid + 纳秒，理由同 `svn_attributes::write_attributes`：固定名字
/// 在并发下会互相截断对方写到一半的内容。
pub fn write(repo_root: &str, j: &Journal) -> Result<()> {
    let target = journal_path(repo_root);
    let dir = target.parent().expect("journal 路径一定有父目录");
    std::fs::create_dir_all(dir)?;
    sweep_stale_tmp(dir);

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = dir.join(format!("{JOURNAL_FILE}.{}.{nanos}.tmp", std::process::id()));

    // `GitError` 没有 `From<serde_json::Error>`，必须 `map_err`
    // （与 `svn_config::write_config` 同一写法）。
    let body = serde_json::to_vec_pretty(j)
        .map_err(|e| GitError::new("svn_journal_encode", format!("journal 序列化失败: {e}")))?;
    // 从创建 tmp 起到 rename 为止，任何一步失败都要把 tmp 删掉（2b 第二轮
    // review R7：原来只有 rename 失败那条路径会清）。收进一个闭包，是为了让
    // 「所有 `?` 都在清理范围里」由结构保证，而不是靠每个调用点记得。
    let written = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&body)?;
        // rename 是原子的，但「新内容已经落盘」不是它保证的。
        // 少了这一步，断电后可能 rename 生效了而内容还是空的。
        f.sync_all()
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    std::fs::rename(&tmp, &target).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        GitError::new("io", format!("替换 journal 失败: {e}"))
    })?;
    Ok(())
}

/// 读 journal。
///
/// **三种结果必须分开**：没有（上次干净收尾）、有且能解析、有但坏了。
/// 第三种绝不能退化成第一种——那等于把用户 stash 的 OID 悄悄丢掉，而恢复代码
/// 会以为「上次什么都没做」，磁盘上工作区却已经被清空了。
pub fn read(repo_root: &str) -> Result<Option<Journal>> {
    let p = journal_path(repo_root);
    let bytes = match std::fs::read(&p) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    serde_json::from_slice(&bytes).map(Some).map_err(|e| {
        GitError::new(
            "svn_journal_corrupt",
            format!(
                "上次同步留下的记录（{}）读不出来：{e}。\
                 文件已原样保留——它是判断上次停在哪里的唯一现场，请勿删除，\
                 先把它交给开发者。",
                p.display()
            ),
        )
    })
}

/// 删除 journal。不存在也算成功——收尾路径会无条件调它。
pub fn remove(repo_root: &str) -> Result<()> {
    match std::fs::remove_file(journal_path(repo_root)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// 删掉前人留下的 `mygitclient-sync.json.*.tmp`（2b 第二轮 review R7）。
///
/// 只有持锁者写 journal（模块头「所有权由锁界定」），所以写入这一刻目录里的
/// 其它 tmp 必然是崩溃留下的残骸。删不掉就算了：它只占几百字节，不影响正确性。
fn sweep_stale_tmp(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let prefix = format!("{JOURNAL_FILE}.");
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with(&prefix) && name.ends_with(".tmp") {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

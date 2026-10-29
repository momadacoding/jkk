//! 基线录制：临时索引、`git-svn-id` trailer、`commit-tree` + `update-ref`。
//!
//! **这是本设计里最不能写错的一段。** 基线是「上次与 SVN 对齐时的状态」这条事实
//! 的唯一载体；它一旦录错，之后每一次失同步判定、每一次交付清单都建立在它之上，
//! 而它本身看起来完全正常。
//!
//! 四条硬要求：
//!
//! 1. **全程不碰真实 index、不碰 HEAD、不碰工作目录文件**（spec §6.3.1）
//! 2. **临时索引文件必须清理干净**，包括中途 `?` 早退的路径 → 靠 [`Drop`]
//! 3. **trailer 三字段缺一不可**，解析失败即「这不是 baseline」（spec §6.4.1）
//! 4. **`update-ref` 必须带旧 OID 作比较条件**

use crate::engine::git_cli::{
    run_git, run_git_code, run_git_env, run_git_stdin, run_git_stdin_env,
};
use crate::engine::svn_config::BASELINE_BRANCH;
use crate::error::{GitError, Result};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// `git update-ref` 用来表示「这个 ref 现在必须不存在」的哨兵旧值。
///
/// 2026-09-14 实测：ref 不存在时带这个值退出 0；ref 已存在时退出 128
/// （`cannot lock ref …: reference already exists`）且 ref 不动。
const ZERO_OID: &str = "0000000000000000000000000000000000000000";

/// 白名单条目 → 传给 git 的 pathspec。
///
/// `add_all` 与 `ls_others` 必须用**同一份**：预检靠 `ls_others` 预言 `add_all`
/// 会收什么，两边的 pathspec 一旦写分叉（比如一边去了尾斜杠、一边加了 `:(literal)`），
/// 预检就会给出假通过或假阻断，而且没有任何测试会变红。
fn w_pathspecs(w_paths: &[String]) -> Vec<String> {
    w_paths
        .iter()
        .map(|w| w.replace('\\', "/"))
        .filter(|w| !w.is_empty())
        .collect()
}

/// 同一进程内的序号，保证同一纳秒里建的两个临时索引也不同名。
static SEQ: AtomicU64 = AtomicU64::new(0);

/// 一次录制用的候选索引。
///
/// # 为什么必须是**同一份**索引贯穿核对与录制
///
/// spec §6.3.1：「核对与录制必须使用同一份候选索引」「不得核对一次后又重新
/// `git add` 另录一棵树」。两次 `git add` 之间磁盘可能变了，于是核对通过的是 A、
/// 录进基线的是 B——而两者都不报错。
///
/// # 为什么是 `GIT_INDEX_FILE`
///
/// `git add` 没有 `--index-output`（只有 `git read-tree` 有），环境变量是把
/// `git add` 引到临时索引上的唯一手段。走 [`run_git_env`] 而不是自己起进程，
/// 见 `AGENTS.md` §7.3。
///
/// # 清理
///
/// 靠 `Drop`，不靠调用点记得删：录制序列有五六步，任何一步 `?` 早退都不能在
/// 用户的 `.git/` 里留下一个孤儿索引文件。
#[derive(Debug)]
pub struct TempIndex {
    path: PathBuf,
}

impl TempIndex {
    /// 从**空索引**起步。初始化向导与 Phase 1 的 R2 用它（spec §6.3.1）。
    ///
    /// 不预先创建文件——git 见到 `GIT_INDEX_FILE` 指向一个不存在的路径时会当成
    /// 空索引，第一次写入时再建出来。
    pub fn empty(repo_root: &str) -> Result<Self> {
        let path = unique_index_path(repo_root)?;
        // 理论上不会撞名（pid + 纳秒 + 进程内序号），但真撞上时留着上一份内容
        // 会让这次录制从别人的索引起步，比多删一个文件危险得多。
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(Self { path })
    }

    /// 从**当前 HEAD 树**起步。其余录制入口用它（spec §6.3.1）。
    ///
    /// HEAD 未出生时报错而不是退化成空索引：2026-09-14 实测
    /// `git read-tree HEAD` 在那种情况下退出 128（`fatal: Not a valid object
    /// name HEAD`）。静默当成空会让「从 HEAD 起步」的入口悄悄变成「从空起步」，
    /// 两者录出来的 tree 未必相同——差别恰恰是「HEAD 里有、磁盘上没有」的文件。
    pub fn from_head(repo_root: &str) -> Result<Self> {
        let idx = Self::empty(repo_root)?;
        idx.git(repo_root, &["read-tree", "HEAD"]).map_err(|e| {
            GitError::new(
                "svn_baseline_no_head",
                format!(
                    "读不到 HEAD，没法从 HEAD 树起步建候选索引。\
                     首个基线与重建基线（R2）应当用 TempIndex::empty。原始错误：{}",
                    e.message
                ),
            )
        })?;
        Ok(idx)
    }

    /// 在候选索引上 `git add -A -- <W-path>`。
    ///
    /// # 空列表是 no-op，绝不能退化成「全部」
    ///
    /// `git add -A` 不带 pathspec 会收整个仓库——包括任意深度的 `.svn/`，
    /// 而那玩意被 git 跟踪之后一次 checkout 就能毁掉一个正在用的工作副本
    /// （spec §6.3）。`AGENTS.md` §6.2 记过本仓库已经因空 pathspec 误伤过一次。
    pub fn add_all(&self, repo_root: &str, w_paths: &[String]) -> Result<()> {
        let paths = w_pathspecs(w_paths);
        if paths.is_empty() {
            return Ok(());
        }
        let mut args: Vec<&str> = vec!["add", "-A", "--"];
        args.extend(paths.iter().map(String::as_str));
        self.git(repo_root, &args)?;
        Ok(())
    }

    /// 把这些磁盘上的文件逐个放进候选索引，**不看忽略规则**。
    ///
    /// 只给「尊重 .gitignore」时补回「已在基线里、被后来的 `.gitignore` 挡住」的文件用
    /// （`WScope::hidden_tracked_on_disk`）：`add -A` 收不到它们，而 git 的语义是已跟踪
    /// 文件不受 `.gitignore` 影响，从空索引起步的入口必须把它们补回来。
    ///
    /// 用 `update-index --add -z --stdin`：它本来就不读忽略规则；路径逐条字面处理，
    /// 没有 pathspec 魔法；走 stdin 不受命令行长度限制。内容按路径套用行尾过滤器，
    /// 与 `git add` 得到的 blob 相同。
    ///
    /// 空列表是 no-op。调用方保证每条路径在磁盘上都是文件——不存在的路径
    /// `update-index` 会直接报错。
    pub fn add_paths(&self, repo_root: &str, paths: &[String]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        let mut input = Vec::new();
        for p in paths {
            input.extend_from_slice(normalize_entry_path(p)?.as_bytes());
            input.push(0);
        }
        let idx = self.path.to_string_lossy().to_string();
        run_git_stdin_env(
            repo_root,
            &["update-index", "--add", "-z", "--stdin"],
            &input,
            &[("GIT_INDEX_FILE", &idx)],
        )?;
        Ok(())
    }

    /// 候选索引的**完整** `git ls-files -z` 结果。
    ///
    /// 故意不带 pathspec、也不按扩展名过滤（spec §6.3.1 原文：「不能再按扩展名
    /// 过滤，以免藏掉多收的文件」）。核对要的正是「这个索引里到底有什么」。
    ///
    /// `-z` 不能省：不带它 git 会按 `core.quotePath` 把非 ASCII 路径转义成
    /// `"Code/\350\247\222\350\211\262.cs"`，而期望集合是遍历磁盘得到的原始字符串，
    /// 两者永远对不上——每个装了中文文件名的工程都会在初始化时被误报阻断。
    pub fn ls_files(&self, repo_root: &str) -> Result<BTreeSet<String>> {
        let out = self.git(repo_root, &["ls-files", "-z"])?;
        Ok(out
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect())
    }

    /// 在候选索引的视角下，`<W-path>` 里「未跟踪且没被忽略」的文件——即此刻
    /// `add_all` **将会**收进来的东西，但不写任何对象。
    ///
    /// 只用于 `git add` 之前的预检（[`crate::engine::svn_include_check::dry_run_include_set`]），
    /// 不替代最终核对：最终核对读的是 `add_all` 之后的真索引。
    ///
    /// 嵌套的 git 仓库在输出里是一条带尾斜杠的目录（`Code/Plugin/`），这里去掉尾斜杠，
    /// 与 `git add` 之后索引里那条子模块指针（`Code/Plugin`）同形。
    ///
    /// 空列表返回空集：`ls-files -o` 不带 pathspec 会列出整个仓库。
    pub fn ls_others(&self, repo_root: &str, w_paths: &[String]) -> Result<BTreeSet<String>> {
        let paths = w_pathspecs(w_paths);
        if paths.is_empty() {
            return Ok(BTreeSet::new());
        }
        let mut args: Vec<&str> = vec!["ls-files", "-o", "--exclude-standard", "-z", "--"];
        args.extend(paths.iter().map(String::as_str));
        let out = self.git(repo_root, &args)?;
        Ok(out
            .split('\0')
            .filter(|s| !s.is_empty())
            .map(|s| s.trim_end_matches('/').to_string())
            .collect())
    }

    /// 某个路径在候选索引里已有的 mode（`100644` / `100755` / `120000`…）。
    ///
    /// 不在索引里返回 `None`——**不要编一个默认值出来**：调用方要靠这个区分
    /// 「替换一个已有条目」和「新建一个磁盘上没有的条目」（`deleted` / `missing`
    /// 的形态，spec §16 R2a）。
    pub fn stage_mode(&self, repo_root: &str, path: &str) -> Result<Option<String>> {
        let p = normalize_entry_path(path)?;
        // `-s` 的输出形如 `100644 <oid> 0\tCode/A.cs`。`--` 后面跟精确路径，
        // 不是 glob，所以最多一行。
        let out = self.git(repo_root, &["ls-files", "-s", "--", &p])?;
        Ok(out
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().next())
            .map(str::to_string))
    }

    /// 把 `bytes` 写成一个 blob，并挂到候选索引的 `path` 上。
    ///
    /// spec §16 R2b 的「取回 BASE」动作。**只动索引，不写磁盘**——这是整条重建
    /// 流程对工作目录只读的关键，也是它与 `svn revert` 的本质区别。
    ///
    /// mode 沿用该路径在索引里已有的值；不在索引里（服务器上有、磁盘上没有）
    /// 则用 `100644`。**不自己判断可执行位**：Windows 上 `core.fileMode` 通常是
    /// false，`git add` 一律给 100644，替换动作没有理由顺手改掉一个与本期无关的
    /// 属性。
    pub fn replace_blob(&self, repo_root: &str, path: &str, bytes: &[u8]) -> Result<()> {
        let p = normalize_entry_path(path)?;
        let mode = self
            .stage_mode(repo_root, &p)?
            .unwrap_or_else(|| "100644".to_string());
        // ⚠️ **`--path` 不能省。**
        //
        // `git hash-object --stdin` 默认**不跑任何过滤器**，而 `git add` 会。本机
        // （以及多数 Windows 开发机）`core.autocrlf=true`，于是同一个文件走两条路会
        // 得到两个不同的 blob。2026-09-17 实测，一个 `svn:eol-style=native` 的文件：
        //
        // ```text
        // 磁盘                    = "n1\r\n"   CRLF
        // git add -A 之后的 blob  = "n1\n"     LF    ← autocrlf 归一了
        // hash-object --stdin     = "n1\r\n"   CRLF  ← 没归一
        // ```
        //
        // 后果是**这个文件在基线里长什么样，取决于它当时脏不脏**：在脏工作副本上
        // 重建一次，行尾整个翻一遍，工作分支上凭空出现「每一行都改了」的假 diff。
        //
        // `--path <路径>` 让 hash-object 按该路径**应当适用的**属性跑过滤器
        // （autocrlf、`.gitattributes` 的 text/eol 规则都算），与 `git add` 对齐。
        // 对齐的基准必须是 `git add`——树里绝大部分内容是它录进去的。
        let oid = run_git_stdin(
            repo_root,
            &["hash-object", "-w", "--path", &p, "--stdin"],
            bytes,
        )?
        .trim()
        .to_string();
        if oid.is_empty() {
            return Err(GitError::new(
                "svn_baseline_hash_object",
                format!("`{p}` 的内容写成 blob 之后没拿到 OID"),
            ));
        }
        // `--add` 让索引里没有的路径也能建出来；`--cacheinfo` 直接指定
        // (mode, oid, path) 三元组，不经过磁盘。
        let info = format!("{mode},{oid},{p}");
        self.git(repo_root, &["update-index", "--add", "--cacheinfo", &info])?;
        Ok(())
    }

    /// 把一条路径从候选索引里摘掉。
    ///
    /// spec §16 R2b 的「剔除」动作：服务器 rN 上没有这个文件（本地 `svn add` 了
    /// 没提交，或者压根没版本控制）。**磁盘上的文件原样保留**——它是用户的内容，
    /// 只是不属于基线。
    pub fn remove_path(&self, repo_root: &str, path: &str) -> Result<()> {
        let p = normalize_entry_path(path)?;
        // `--force-remove` 才会在文件**仍然存在于磁盘上**时也把条目摘掉；
        // 不带它的 `--remove` 只对磁盘上已消失的文件生效，而这里正相反。
        self.git(repo_root, &["update-index", "--force-remove", "--", &p])?;
        Ok(())
    }

    /// 把候选索引写成一棵 tree，返回它的 OID。
    pub fn write_tree(&self, repo_root: &str) -> Result<String> {
        Ok(self.git(repo_root, &["write-tree"])?.trim().to_string())
    }

    /// 临时索引文件的位置。测试用来确认它被清掉了。
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn git(&self, repo_root: &str, args: &[&str]) -> Result<String> {
        let p = self.path.to_string_lossy().to_string();
        run_git_env(repo_root, args, &[("GIT_INDEX_FILE", &p)])
    }

    fn git_stdin(&self, repo_root: &str, args: &[&str], stdin: &[u8]) -> Result<String> {
        let p = self.path.to_string_lossy().to_string();
        run_git_stdin_env(repo_root, args, stdin, &[("GIT_INDEX_FILE", &p)])
    }
}

/// 索引条目路径的归一：正斜杠、非空。
///
/// 空路径单独拦住，不交给 git 去解释。`git update-index --force-remove -- ""` 的
/// 行为不该由我们去猜，而本仓库已经因为空 pathspec 误伤过一次（`AGENTS.md` §6.2）。
fn normalize_entry_path(path: &str) -> Result<String> {
    let p = path.replace('\\', "/");
    if p.trim().is_empty() {
        return Err(GitError::new(
            "svn_baseline_empty_path",
            "索引条目的路径是空的。空 pathspec 在本仓库造成过一次误伤，一律拒绝。",
        ));
    }
    Ok(p)
}

impl Drop for TempIndex {
    fn drop(&mut self) {
        // 删不掉也不能 panic——Drop 里 panic 会在展开途中直接 abort。
        // 留一个孤儿索引文件是可容忍的，把应用干掉不是。
        let _ = std::fs::remove_file(&self.path);
    }
}

fn unique_index_path(repo_root: &str) -> Result<PathBuf> {
    let dir = git_dir(repo_root)?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    Ok(dir.join(format!(
        "mygitclient-{}-{}-{}.index",
        std::process::id(),
        nanos,
        seq
    )))
}

/// 临时索引放进 `.git/`：与真实索引同一个文件系统（`write_tree` 之后的对象要落在
/// 同一个对象库），且随仓库一起被清理，不会留在用户的工作目录里被 TortoiseSVN
/// 看见。位置问 git 要而不是拼 `repo_root/.git`——worktree 里 `.git` 是个文件。
/// 持有 git 自己的 `.git/index.lock` —— **索引的互斥量**。
///
/// # 为什么是 git 的锁，不是我们自己发明一把
///
/// 要防的是「校验完索引、还没写进去」这段时间里**别的 git 工具**（IDE 的 git
/// 集成、用户的终端、另一个 GUI）往索引里 `git add` 东西。我们自己的应用级锁
/// 它们根本不看，所以那种锁挡不住这件事——能挡住的只有每个 git 进程都尊重的
/// 那一把。
///
/// 2026-09-25 实测确认这三点成立（probe-lock.sh）：
///
/// ```text
/// 持锁期间 git add           → fatal: Unable to create '…/.git/index.lock'，退出 128，索引没动
/// 持锁期间 git diff-index    → 退出 0，纯读不需要锁，所以校验照常做得了
/// 换索引                     → GIT_INDEX_FILE=<临时> read-tree 生成，再 rename 覆盖
/// ```
///
/// 于是「校验 + 换上」之间没有任何窗口，这是**真正的 compare-and-swap**，
/// 不是把窗口缩小。
///
/// # 崩溃留下的残锁
///
/// 与 git 自己留下的残锁完全同形：下一次任何 git 写操作都会报
/// 「Another git process seems to be running」并指出文件路径。这是用户已经见过
/// 的、有现成处置办法的失败形态，所以这里不另造一套超时或抢占逻辑——
/// 自动抢占别人的索引锁比留一个残锁危险得多。
pub struct IndexLock {
    path: PathBuf,
}

impl IndexLock {
    pub fn acquire(repo_root: &str) -> Result<Self> {
        let path = git_dir(repo_root)?.join("index.lock");
        // `create_new` = O_EXCL：存在即失败，判断与创建是同一次系统调用。
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_) => Ok(Self { path }),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(GitError::new(
                "svn_index_busy",
                format!(
                    "另一个 git 进程正在占用这个仓库的索引（{}）。\
                     请等它结束后重试；如果确认没有 git 在跑，那是上一次崩溃留下的\
                     残锁，手工删掉该文件即可。",
                    path.display()
                ),
            )),
            Err(e) => Err(e.into()),
        }
    }
}

impl Drop for IndexLock {
    fn drop(&mut self) {
        // 删不掉也不能 panic（见 TempIndex::drop 的同一条理由）。
        let _ = std::fs::remove_file(&self.path);
    }
}

/// 把真实索引换成 `target` 树，**但只在它此刻仍等于 `expect` 树时**。
///
/// 全程持有 [`IndexLock`]，所以「校验」与「换上」之间别的 git 进程进不来。
/// 磁盘上的文件一个都不碰：新索引是用 `read-tree`（**不带 `-u`**）在一个独立的
/// 临时索引文件里生成的，再整体 rename 覆盖 `.git/index`。
pub fn swap_index_to_tree(repo_root: &str, expect: &str, target: &str) -> Result<()> {
    {
        let _lock = IndexLock::acquire(repo_root)?;

        // 纯读，不需要索引锁；而锁在我们手里，所以这个结论一直有效到 rename 为止。
        let (code, _) = run_git_code(repo_root, &["diff-index", "--cached", "--quiet", expect])?;
        if code != 0 {
            return Err(GitError::new(
                "svn_index_moved",
                "索引已经不是预期的样子了（有别的 git 操作动过它）。\
                 这次不会覆盖它。",
            ));
        }

        let index = git_dir(repo_root)?.join("index");
        let staging = build_index_keeping_stat(repo_root, &index, target)?;
        // rename 覆盖：锁在手里，没有别的 git 进程会同时读写 .git/index。
        // 这正是 git 自己换索引的做法（写 index.lock 再 rename）。
        std::fs::rename(staging.path(), &index)?;
        // `staging` 的 Drop 会去删一个已经不存在的路径，`let _ =` 吃掉即可。
    }
    refresh_stat(repo_root);
    Ok(())
}

/// 给真实索引里 stat 对不上的条目按磁盘回填 stat。
///
/// 只沿用不回填的话，变了的条目 stat 永远是 0：行尾迁移改写全部 blob、一次大的
/// `svn update`，之后又是每次刷新全量哈希（独立 review）。refresh 只哈希 stat 对不上
/// 的条目，代价与这次变了的文件数成正比；与磁盘不一致（有本地改动）的条目照旧留 0。
/// `-q` 下有修改 / 删除也返回 0（2026-10-30 实测）。
///
/// **放在释放索引锁之后**（第二轮独立 review）：行尾迁移会让它哈希全部文件，在锁里跑
/// 等于让 IDE、终端的 git 写操作在这几十秒里全部撞上 `index.lock`。它只改 stat、不改
/// 条目内容，git 自己会拿锁，与别的进程并发是安全的；拿不到锁就只是这次没刷。
/// stat 只关乎快慢，不关乎对错，所以失败只记 warning。
fn refresh_stat(repo_root: &str) {
    if let Err(e) = run_git(repo_root, &["update-index", "-q", "--refresh"]) {
        log::warn!("刷新索引 stat 失败，之后第一次 status 会慢：{}", e.message);
    }
}

/// 生成一份 `target` 树的临时索引，**沿用真实索引里没变的条目的 stat**。
///
/// # 为什么不能只从空索引 `read-tree`
///
/// 从空索引读出来的条目 ctime / mtime / size 全是 0。git 与 libgit2 都只能逐个读
/// 文件哈希来判断「没改」——2026-10-28 现场一万五千个文件 `git status` 冷缓存 53 秒，
/// 而 libgit2 只读不回写，客户端每次刷新都要重来一遍，Local Changes 长时间刷不出来。
/// 每次同步都会走到这里，等于每次同步都把这个慢重新造一遍。
///
/// # 复制真实索引 + `read-tree -i -m`
///
/// 单树合并：内容与 `target` 相同的条目原样保留（连同 stat），变了的条目换成新
/// blob、stat 清零——旧 stat 对新内容无效，沿用它就是把新内容当成旧内容。变了的
/// 条目由换上之后的 [`refresh_stat`] 回填。2026-10-30 实测（probe-readtree.sh）：
///
/// ```text
/// read-tree -m      → 换掉的条目在工作区有未暂存改动时拒绝：
///                     Entry 'b.txt' not uptodate. Cannot merge.（退出 128）
/// read-tree -i -m   → 不对照工作区；a / c 保留 stat，b 清零
/// ```
///
/// 同步把服务器改动合进一个本地也改过的文件，正是 `-m` 拒绝的形态，所以必须带
/// `-i`。它也不动磁盘（没有 `-u`），与原来的做法一样只写索引。真实索引不存在时
/// git 把 `GIT_INDEX_FILE` 当成空索引，`-i -m` 照常可用（实测），不用另走一条分支。
///
/// # 复制必须保留 mtime
///
/// git 判「racily clean」靠比较条目 mtime 与**索引文件自己的** mtime。复制出来的
/// 文件若拿到「现在」的 mtime，原本该被当成可疑、要重新哈希的条目就会被当成干净的。
/// `std::fs::copy` 在 Windows 上保留 mtime、在 Unix 上不保留，所以显式设回去。
///
/// # 退路
///
/// stat 只关乎快慢，不关乎对错，这一步失败不能让同步失败：换一份**新的**临时索引
/// 从空生成（不去删失败的那份——删不掉的话又是一个失败点，它的 `Drop` 会尽力清理）。
/// 目前没有已知能让它失败而守卫放行的输入：冲突条目会先被 `diff-index --cached` 的
/// 守卫挡下。所以这条退路是防御性的，测试覆盖不到。
fn build_index_keeping_stat(repo_root: &str, index: &Path, target: &str) -> Result<TempIndex> {
    match merge_onto_copy(repo_root, index, target) {
        Ok(idx) => Ok(idx),
        Err(e) => {
            log::warn!("沿用 stat 生成索引失败，退回从空索引生成：{}", e.message);
            let idx = TempIndex::empty(repo_root)?;
            idx.git(repo_root, &["read-tree", target])?;
            Ok(idx)
        }
    }
}

fn merge_onto_copy(repo_root: &str, index: &Path, target: &str) -> Result<TempIndex> {
    let idx = TempIndex::empty(repo_root)?;
    if index.is_file() {
        copy_with_mtime(index, idx.path())?;
    }
    idx.git(repo_root, &["read-tree", "-i", "-m", target])?;
    clear_entry_flags(repo_root, &idx)?;
    Ok(idx)
}

/// 清掉条目上的 skip-worktree / assume-unchanged 标志。
///
/// 2026-10-30 实测：`read-tree -i -m` 把 skip-worktree 留在**内容已变**的条目上，
/// assume-unchanged 也原样保留。同步之后紧接着 checkout / rebase 工作分支，带
/// skip-worktree 的路径不会被写到磁盘，分支内容与磁盘静默分叉（第二轮独立 review）。
/// 原来从空索引生成时这些标志每次都被清掉，同步流程依赖的正是那个形状，这里还原它。
///
/// `ls-files -v` 的标记：`S` / `s` = skip-worktree，小写 = assume-unchanged。两个
/// `--no-*` 选项**必须分两次**：实测写在同一条命令里只有后一个生效。路径走 stdin，
/// 数量可能成千上万（sparse checkout），放命令行会撞上 Windows 的长度上限。
fn clear_entry_flags(repo_root: &str, idx: &TempIndex) -> Result<()> {
    let listing = idx.git(repo_root, &["ls-files", "-v", "-z"])?;
    let (mut skipped, mut assumed) = (Vec::new(), Vec::new());
    for rec in listing.split('\0') {
        let Some((tag, path)) = rec.split_once(' ') else {
            continue;
        };
        if tag.eq_ignore_ascii_case("s") {
            skipped.push(path);
        }
        if tag.chars().all(|c| c.is_ascii_lowercase()) {
            assumed.push(path);
        }
    }
    for (flag, paths) in [
        ("--no-skip-worktree", skipped),
        ("--no-assume-unchanged", assumed),
    ] {
        // 空列表不发命令：不让 git 去解释一个空的路径输入（AGENTS.md §6.2）
        if paths.is_empty() {
            continue;
        }
        let mut stdin = paths.join("\0").into_bytes();
        stdin.push(0);
        idx.git_stdin(repo_root, &["update-index", "-z", flag, "--stdin"], &stdin)?;
    }
    Ok(())
}

fn copy_with_mtime(from: &Path, to: &Path) -> Result<()> {
    std::fs::copy(from, to)?;
    let mtime = std::fs::metadata(from)?.modified()?;
    std::fs::OpenOptions::new()
        .write(true)
        .open(to)?
        .set_modified(mtime)?;
    Ok(())
}

pub fn git_dir(repo_root: &str) -> Result<PathBuf> {
    let out = run_git(repo_root, &["rev-parse", "--git-dir"])?;
    let p = PathBuf::from(out.trim());
    Ok(if p.is_absolute() {
        p
    } else {
        Path::new(repo_root).join(p)
    })
}

/// 基线 commit 里那一行权威记账（spec §6.4.1）。
#[derive(Debug, Clone, PartialEq)]
pub struct Trailer {
    /// **Repository Root**，不是某个工作副本的 URL。
    ///
    /// 白名单可能分布在同一个仓库的多个嵌套工作副本里，各自 URL 不同，没有哪一个
    /// 能代表整体；`Repository Root` + `uuid` 对它们是共有的（spec §6.4.1）。
    pub url: String,
    /// 记账单位 N，取自 `nodeRevs`（不是 `svn info .` 的 revision）。
    pub rev: u64,
    /// 仓库 UUID。URL 会因代理 / 镜像 / `http` 与 `https` 写法不同而不可靠，
    /// UUID 是仓库身份的唯一可靠依据（spec §6.2.1）。
    pub uuid: String,
}

/// 格式化成 `git svn` 的原生 trailer 行。
///
/// 沿用 `git svn` 的格式而不是自定义 `Svn-Rev:`：UUID 白送、
/// `git log --grep=^git-svn-id` 这个标准用法照常可用、将来真要迁移时有现成锚点
/// （spec §6.4.1）。
pub fn format_trailer(t: &Trailer) -> String {
    format!("git-svn-id: {}@{} {}", t.url, t.rev, t.uuid)
}

/// 从 commit message 里读回 trailer；读不出来就是 `None`。
///
/// 对应 spec §6.4.1 的正则 `^git-svn-id: (\S+)@(\d+) ([0-9a-fA-F-]+)$`。
/// **三个字段缺一不可，解析失败即视为该 commit 不是 baseline——不猜、不部分接受。**
/// 手写而不是引 `regex`：只为这一条式子加一个 crate 不划算（`AGENTS.md` §1.2）。
///
/// # 两处容易写错的地方
///
/// - **取最后一个 `@`**。`svn+ssh://user@host/repo` 是完全正常的 URL，
///   按第一个 `@` 切会把 rev 读成 `host/repo@12346`，整条基线被判成「不是 baseline」。
/// - **行尾可能有 `\r`**。message 经不同工具往返时会带上 CR，正则的 `$` 对不上
///   就凭空少一条基线。这里先 `trim_end` 再判。
pub fn parse_trailer(message: &str) -> Option<Trailer> {
    for raw in message.lines() {
        let Some(rest) = raw.trim_end().strip_prefix("git-svn-id: ") else {
            continue;
        };
        // `<url>@<rev> <uuid>`：uuid 前只有一个空格，且 `$` 锚定意味着它后面
        // 不能再有别的东西——所以按第一个空格切，右边整段都必须是 uuid。
        let (left, uuid) = rest.split_once(' ')?;
        if uuid.is_empty() || !uuid.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
            return None;
        }
        // `(\S+)` 是贪婪的，等价于取**最后一个** `@`
        let (url, rev) = left.rsplit_once('@')?;
        if url.is_empty() || url.chars().any(char::is_whitespace) {
            return None;
        }
        return Some(Trailer {
            url: url.to_string(),
            rev: rev.parse::<u64>().ok()?,
            uuid: uuid.to_string(),
        });
    }
    None
}

fn baseline_ref() -> String {
    format!("refs/heads/{BASELINE_BRANCH}")
}

/// 用 `TREE` 建一个基线 commit，**不动任何 ref**。
///
/// - `parent`：首个基线传 `None`（无父提交）；其余入口按各自流程的定义传。
///
/// 与 [`move_baseline_ref`] 分开，是因为 `sync_in` 的 S7 要在两步之间落一次
/// journal（spec §7.6 `publishing-baseline`：创建 commit 前记录 captureTree，
/// 移动 ref 前再记录 newBaselineOid）。崩在两步之间时，重开就知道「commit 已建、
/// ref 未动」，只补 update-ref，不重建另一个 commit。
pub fn create_baseline_commit(
    repo_root: &str,
    tree: &str,
    parent: Option<&str>,
    t: &Trailer,
) -> Result<String> {
    create_baseline_commit_with(repo_root, tree, parent, t, None)
}

/// 交付录的基线额外记的那一行：真正提交出去的 revision。
const DELIVERED_REV_KEY: &str = "Delivered-Rev:";

/// 读回 `Delivered-Rev: N`；不是交付录的基线就是 `None`。逐行完整匹配，与 `parse_trailer` 同样先去行尾 `\r`。
pub fn parse_delivered_rev(message: &str) -> Option<u64> {
    message.lines().find_map(|raw| {
        raw.trim_end()
            .strip_prefix(DELIVERED_REV_KEY)
            .and_then(|r| r.trim().parse::<u64>().ok())
    })
}

/// [`create_baseline_commit`]，交付时多记一行 `Delivered-Rev: N`。
///
/// trailer 记的是 D6 更新到的 revision；交付之后同事紧跟着提交时它是同事的，只靠它
/// 就分不清哪个 revision 才是这次交付（2026-10-16 review）。`git-svn-id` 仍放最后一行。
pub fn create_baseline_commit_with(
    repo_root: &str,
    tree: &str,
    parent: Option<&str>,
    t: &Trailer,
    delivered_rev: Option<u64>,
) -> Result<String> {
    // `commit-tree` 的 message **必须走 `-m`，不能走 stdin**：`run_git*` 用的是
    // `Command::output()`，子进程 stdin 是 null，管道那条路会立刻读到 EOF 并产出
    // 一个 message 为空的 commit——而 trailer 就写在 message 里，等于基线丢掉
    // 记账且不报任何错。
    let delivered = delivered_rev
        .map(|r| format!("{DELIVERED_REV_KEY} {r}\n"))
        .unwrap_or_default();
    let msg = format!("svn: r{}\n\n{delivered}{}\n", t.rev, format_trailer(t));
    let mut args: Vec<&str> = vec!["commit-tree", tree];
    if let Some(p) = parent {
        args.push("-p");
        args.push(p);
    }
    args.push("-m");
    args.push(&msg);
    Ok(run_git(repo_root, &args)?.trim().to_string())
}

/// **以旧 OID 作比较条件**把基线分支移到 `oid`。
///
/// - `expected_old`：调用方读到的基线当前值。`None` 表示「这个 ref 现在不该存在」。
///
/// # 为什么非得带旧值
///
/// 并发的第二个 mygitclient 进程、或用户在 IDE / 命令行里手工动了
/// `svn/base`，都可能发生在「读取 → 核对 → 录制」与「发布」之间。无条件覆盖
/// 等于**悄悄丢掉**那一次录制，而且两边都不报错。
///
/// 这里先自己读一次再比对，而不是只靠 `update-ref` 的报错：
/// 这样常见情况能给出一条说得清的中文错误（`svn_baseline_moved` + 两个 OID），
/// 而不是把 git 的英文 `cannot lock ref` 原样丢给用户。`update-ref` 那一层仍然
/// 带着旧值，负责兜住这两步之间的窄竞态。
pub fn move_baseline_ref(repo_root: &str, oid: &str, expected_old: Option<&str>) -> Result<()> {
    let current = read_baseline(repo_root)?;
    if current.as_deref() != expected_old {
        return Err(moved_error(expected_old, current.as_deref()));
    }
    let old = expected_old.unwrap_or(ZERO_OID);
    run_git(repo_root, &["update-ref", &baseline_ref(), oid, old]).map_err(|e| {
        // 走到这里说明上面那次读取之后、这一行之前有人动了 ref。仍然归成
        // 「基线被挪走了」，但把 git 的原话带上，免得掩盖掉别的失败原因
        // （比如 ref 目录不可写）。
        GitError::new(
            "svn_baseline_moved",
            format!(
                "更新基线分支失败——发布前的一瞬间它被别人改了。\
                 已经建好的 commit 是 {oid}（对象还在，没有丢）。git 原话：{}",
                e.message
            ),
        )
    })?;
    Ok(())
}

/// 建 commit 并发布。`init_hybrid` 与 `rebuild_baseline` 用它，行为与拆分前逐字相同：
/// 先比对旧值，比对不过就不建 commit。
pub fn publish_baseline(
    repo_root: &str,
    tree: &str,
    parent: Option<&str>,
    t: &Trailer,
    expected_old: Option<&str>,
) -> Result<String> {
    let current = read_baseline(repo_root)?;
    if current.as_deref() != expected_old {
        return Err(moved_error(expected_old, current.as_deref()));
    }
    let oid = create_baseline_commit(repo_root, tree, parent, t)?;
    move_baseline_ref(repo_root, &oid, expected_old)?;
    Ok(oid)
}

/// 基线分支现在指着谁；不存在返回 `None`。
pub(crate) fn read_baseline(repo_root: &str) -> Result<Option<String>> {
    let (code, out) = run_git_code(repo_root, &["rev-parse", "-q", "--verify", &baseline_ref()])?;
    match code {
        0 => Ok(Some(out.trim().to_string())),
        // 1 = 「ref 不存在」。**不能把 128 也算进来**：那是「ref 损坏」，
        // 当成「不存在」会让我们以为这是首个基线，然后覆盖掉一段真实历史。
        // 同一个坑 1a 在 `engine::ops::head_is_unborn` 上踩过一次。
        1 => Ok(None),
        _ => Err(GitError::new(
            "svn_baseline",
            format!(
                "读取基线分支 {} 失败（rev-parse 退出 {code}）。\
                 这不是「基线不存在」，更可能是 ref 损坏；\
                 当成不存在会覆盖掉一段真实历史，所以这里直接报错。",
                baseline_ref()
            ),
        )),
    }
}

fn moved_error(expected: Option<&str>, actual: Option<&str>) -> GitError {
    let want = expected.unwrap_or("（不存在）");
    let got = actual.unwrap_or("（不存在）");
    GitError::new(
        "svn_baseline_moved",
        format!(
            "基线分支在本次录制期间被改动过：开始时是 {want}，现在是 {got}。\
             继续写入会悄悄丢掉那一次改动，所以这里停下。\
             请重新读取当前状态后再录一次。"
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::copy_with_mtime;
    use std::time::{Duration, SystemTime};

    /// 复制出来的临时索引必须带着原索引的 mtime：git 拿索引文件自己的 mtime 判
    /// racily clean，变成「现在」会让本该重新哈希的条目被当成干净的
    /// （`std::fs::copy` 在 Unix 上不保留 mtime，Windows 上保留，两边都要成立）。
    #[test]
    fn the_copy_keeps_the_source_mtime() {
        let dir = tempfile::TempDir::new().unwrap();
        let (from, to) = (dir.path().join("index"), dir.path().join("copy"));
        std::fs::write(&from, b"DIRC").unwrap();
        let past = SystemTime::now() - Duration::from_secs(3600);
        std::fs::OpenOptions::new()
            .write(true)
            .open(&from)
            .unwrap()
            .set_modified(past)
            .unwrap();

        copy_with_mtime(&from, &to).unwrap();

        assert_eq!(std::fs::read(&to).unwrap(), b"DIRC");
        assert_eq!(
            std::fs::metadata(&to).unwrap().modified().unwrap(),
            std::fs::metadata(&from).unwrap().modified().unwrap()
        );
    }
}

# 跟进的 git 读操作改走 libgit2：实现日志

目标：自动跟进的关键路径上有 30 个串行的 git 子进程（2 万文件 fixture，PR 173 + 174 之后），每个约 44 ms，几乎全是 Windows 起进程的开销，合计约 1.46 s，是整次跟进（约 4.15 s）里最大的一类。其中 19 个是读操作。按项目分工（读走 libgit2、写走 git CLI），把这 19 个改成进程内读，写操作不动。用户 2026-11-16 要求「做 libgit2 那块」。

没有单独的计划文件，本日志的 Task 划分即计划。

## 那 30 个进程（`git --version`、`--exec-path` 进程内各只问一次，不算）

| 阶段 | 进程 | 读 / 写 | 调用点 |
|---|---|---|---|
| 准备 | `read-tree -i -m` | 写（临时索引） | `TempIndex::from_tree_keeping_stat` |
| 准备 | `ls-files -v -z` | 读 | 同上，记标志 |
| 准备 | `ls-files -z` | 读 | `TempIndex::ls_files`（预测体检清单） |
| 体检之后 | `commit-tree` | 写 | `create_baseline_commit_with` |
| 体检之后 | `diff-tree --name-status` | 读 | `External::read` |
| 体检之后 | `log --reverse --topo-order` | 读 | `svn_replay::commits_between` |
| 体检之后 | `diff-tree --name-only` | 读 | `svn_replay::changed_paths`（`all_committed`） |
| 体检之后 | `log --format=%B` | 读 | `svn_replay::all_committed` |
| 体检之后 | `worktree list --porcelain` | 读 | `worktree_heads`（`plan_others`） |
| 体检之后 | `for-each-ref` | 读 | `local_branches` |
| 发布 | `ls-files -u` | 读 | `prepare_index` |
| 发布 | `diff-index --cached` | 读 | `prepare_index` |
| 发布 | `hash-object -w`、`mktree` ×2、`commit-tree`、`update-ref` | 写 | `pin_raw_index` |
| 发布 | `cat-file --batch-check` | 读 | `existing_blobs` |
| 发布 | `ls-files -v` ×2 | 读 | `prepare_index`、`restore_flags` |
| 发布 | `diff-tree -r`（raw） | 读 | `prepare_index` |
| 发布 | `read-tree -i -m`、`update-index` ×（若干） | 写 | `prepare_index` |
| 发布 | `hash-object --no-filters --stdin` ×2 | 读（不写对象库） | `blob_of`（`record_prepared`） |
| 发布 | `ls-files --stage` | 读 | `record_prepared` |
| 发布 | `update-ref` ×2 | 写 | 发布基线、移分支 |
| 发布 | `worktree list --porcelain` | 读 | `finish` |
| 发布 | `for-each-ref --sort` | 读 | `trim_pins` |

## 原则

- **只换读，写一律留在 git CLI。**
- **认不出来就退回 git CLI**：libgit2 不支持或语义可能不同的形状，一律退回原来的命令，不猜。包括：split index、sparse index、索引里有非 0 stage、提交带非 UTF-8 编码、`log` 的输出编码不是 UTF-8、`linear_commits` 的区间里有 merge、替换对象 / grafts / shallow、gitlink、读不懂的 HEAD 文件、SHA-256 仓库、打不开仓库。
- **每个替换都有等价性测试**：在 fixture 上把新旧两种读法的结果逐个场景比较，比不上就算失败。场景必须覆盖 libgit2 与 git CLI 已知会分歧的地方（AGENTS.md §6.2）：大小写、i-t-a、skip-worktree / assume-unchanged、gitlink、typechange、符号链接、多行 / CRLF 的提交说明。

## 进度总账

| Task | 状态 | 产出 | 验证 |
|---|---|---|---|
| T1 对象与 ref：`hash-object`（不写）×2、`cat-file --batch-check`、`for-each-ref` ×2、`worktree list` ×2 | 完成 | `git_read::{or_cli, blob_oid, present_blobs, local_branches, refs_newest_first, worktree_branches}`；调用点 `blob_of`、`existing_blobs`、`local_branches`、`trim_pins`、`worktree_heads` | `tests/git_read_follow.rs` 6 条 |
| T2 树与提交：`diff-tree` ×3、`log` ×2 | 完成 | `git_read::{tree_changes, linear_commits, bodies}`；调用点 `prepare_index` 的 OLD→NEW、`External::read`、`paths_after_mark`、`svn_replay::{changed_paths, commits_between, all_committed}` | 同上 6 条；`svn_replay.rs` 14/14 |
| T3 索引：`ls-files -v` ×3、`ls-files -z`、`ls-files --stage`、`ls-files -u`（`diff-index --cached` 最后退回 git，见 T4） | 完成 | `git_read::{index_records, has_unmerged, ls_files_v, ls_files_stage, ls_files}`；`svn_baseline::index_listing`；调用点 `clear_entry_flags`、`TempIndex::ls_files`、`prepare_index`、`restore_flags`、`record_prepared` | 同上 7 条；`svn_follow_index.rs` 35/35（新增退回 git 的端到端 1 条） |
| T4 实测、review、PR | 完成 | `or_cli` 记 `kind = "inproc"` 的性能记录；`tree_changes` 改为逐层比；`diff-index --cached` 退回 git；review 修 7 条 | `cargo fmt --check`、`cargo clippy -j 4 --all-targets` 无告警；集成测试 97 个文件全过（内存紧，前台分批 `-j 1`；后台那批被系统以内存不足停过一次）；review 修复之后又重跑了跟进相关的 9 个文件；lib 170/170 |

## 决策条目

### 推翻旧决定的理由

`git_read.rs` 开头原来写着：`diff-tree`（重命名检测）、`log` / `rev-list` 的排序、`worktree list`（prunable 判定）两边语义有差，留在 CLI。这次只在更窄的条件下接：

- `diff-tree`：调用点全带 `--no-renames`，没有重命名检测这回事。
- `log`：只接**线性**区间，有 merge 就交回 CLI；线性时两边的顺序只有一种。
- `worktree list`：调用点只要「哪些分支被检出了」，不要 prunable 判定；直接读 CLI 读的那批 `HEAD` 文件。

### T1 对象与 ref

- **upstream 与 CLI 一致**：远端分支还没取回算「有」，remote 没定义算「没有」，两种都实测过、钉进测试。libgit2 报 NotFound 以外的错时算「有」：`upstream` 为真的分支跟进不移动它，说宽了只是少移一条。
- **计划的错误**：起初以为「remote 没定义」时 git 给空、libgit2 会报错，打算把它当成有意的分歧写进测试。实测两边都说「没有」，测试改成与 CLI 相同。
- `worktree_branches` 直接读 `<commondir>/HEAD` 与 `<commondir>/worktrees/*/HEAD`。目录已删、还没 prune 的 worktree，CLI 照样列出，这里也一样（有测试）。**有意的分歧**：主仓库是 bare 时 CLI 不报主工作区的分支，这里照报（只会让跟进少移一条分支）。混合仓库不会是 bare。
- `refs_newest_first`：`--sort` 写两次时最后一个是主键（提交时间倒序，同一秒按 ref 名倒序），有同一秒、不同秒的测试。

### T2 树与提交

- `tree_changes` 第一版用 libgit2 的 `diff_tree_to_tree`（开 `include_typechange`），T4 因为慢换成了手写的逐层比较，见 T4。两版的输出规则相同：文件 ↔ 符号链接是一条 `T`；文件 ↔ 目录拆成删 + 加；按全路径字节序排，与 CLI 的树遍历顺序相同（目录在比较里按「名字/」，`/` 就是 0x2F）。
- `bodies()` 不排除 merge：调用方只看有没有带 Local-Only 标记的说明，与顺序无关；`git log` 也会列出 merge 的说明。只有 `linear_commits()` 要求区间线性。
- **gitlink 一律交回 CLI**：子模块的 `ignore` 设置会影响 git 的 diff，两边不一定一致；混合仓库里没有子模块，没必要冒这个险。
- `%s` 照 git `pretty.c` 的 `format_subject` 实现：跳过开头只有空白的行，取第一段，每行去掉行尾空白（git 的空白只有空格、`\t`、`\n`、`\r`），用空格连起来。libgit2 的 `summary()` 会挤压行内空白，与它不同，不用。测试覆盖多行主题、CRLF、开头空行、只有空白的说明、中文。
- 带 `encoding` 头（CLI 会转码）或不是合法 UTF-8 的说明交回 CLI。

### T3 索引

- **计划没写的坑（实测）**：libgit2 的 `diff_tree_to_index` **静默跳过 assume-unchanged 的条目**：树里没有、索引里有的 assume-unchanged 文件，`git diff-index --cached` 报 `A`，它不报。与 HEAD 比时两边恰好一样，与更早的提交比才暴露。照这样换上去，跟进会漏看暂存的内容。所以第一版的 `index_vs_tree` 不用 libgit2 的 diff，按 git 的做法自己逐条比（每个路径比 mode 与 OID），i-t-a、大小写不同的路径也都与 git 对上了。但它要把整棵 OLD 树走一遍，比起一个 git 进程还慢，最后退回了 git（T4）。
- `ls-files --stage` 的输出要哈希进 journal，恢复时 `index_was_swapped` 用 `git ls-files --stage -z` 读真实索引再比，所以必须**逐字节相同**；测试比的就是字节。恢复那条路照旧走 git，不改。
- **不接的形状**：split index（`core.splitIndex` 或有 `sharedindex.*`）、sparse index（`index.sparse` 或有目录条目）、未合并条目（`ls-files` 列表时）、gitlink、非 UTF-8 路径。
- **计划没写的坑（既有问题）**：拼索引里原有的 `intent_to_add_paths` 早就用 libgit2 读索引，碰上 split index 报 `unsupported mandatory extension: 'link'`。也就是说**改动前跟进就不支持 split index**（失败时锁随 RAII 删掉，什么都没动）。不在这次范围里，记为既有限制。原来以为 libgit2 会静默忽略 `link` 扩展，实测是直接报错，注释已改正。
- 退回 git 的端到端测试用 `index.sparse = true` 触发：没开 sparse checkout 时 git 仍写普通索引，只有「每一步都退回 git」这一点与平常不同，拼出来的结果与平常一样。

### T4 实测

2 万文件 fixture，与应用相同的启动环境，master 0dc3a2e 与本分支交替跑。

**第一版只省了约 0.5 s 里的一半**：git 进程从 40 个降到 20 个，Rust 自己的时间却多了约 0.28 s。给 `or_cli` 加上每个快路径的计时（`kind = "inproc"`）之后看清了：

| 读 | 原来一个 git 进程 | 第一版进程内 | 原因 |
|---|---|---|---|
| 树比较（`tree_changes`）×3 | 约 45 ms | 64–150 ms | libgit2 的 `diff_tree_to_tree` 把两棵树的每个条目都走一遍，不跳过 OID 相同的子树 |
| 暂存区比 OLD（`index_vs_tree`） | 约 45 ms | 75–100 ms | 手写的比较要把整棵 OLD 树走一遍；git 靠索引的 cache-tree 跳过没变的目录，libgit2 不提供 cache-tree |
| 其余 15 个 | 约 45 ms | 1–39 ms | — |

**偏离计划**：

- `tree_changes` 不再用 libgit2 的 diff，照 git 的做法逐层比（`tree.c` 的排序键：目录按「名字/」），OID 相同的子树整棵跳过。等价性测试原样通过，一次 1–17 ms。
- `diff-index --cached` **退回 git**，`index_vs_tree` 与它的测试删掉。上面 T3 记下的 assume-unchanged 分歧照样成立：以后谁想再把它搬进进程内，不能用 `diff_tree_to_index`。

第二版，各 5 轮（第 1 轮热身不算）：

| | master | 本分支 |
|---|---|---|
| 一次跟进 | 4.06–5.21 s，中位 4.15 s | 3.35–4.19 s，中位 3.64 s |
| 每次跟进起的 git 进程（计时工具里，含进程启动时的 `--version`） | 40 个 | 21 个 |
| 关键路径上的 git 小进程 | 33 个，1.42 s | 15 个，0.74 s |
| 进程内的读 | — | 19 次合计 116 ms，最慢 22 ms（`present_blobs`） |
| Rust 自己的时间 | 0.47 s | 0.57 s |

省约 0.5 s（约 12%）。前几轮机器很忙（同一个 master 跑出 5.8–13 s），只看进程数与关键路径，没有拿那几轮的墙钟下结论。

### 独立 review（subagent，按 AGENTS.md §13）

| 第几条 | 问题 | 处理 |
|---|---|---|
| 1（阻断） | git 的 `log` / `diff-tree` / `cat-file` / `for-each-ref` 的 committerdate 默认按 `refs/replace/` 换对象，libgit2 不换：仓库里有替换时快路径会给出不同的答案 | 已修。`no_replace` 守卫：有 `refs/replace/*`、设了 `GIT_REPLACE_REF_BASE`、有 `info/grafts` 或 `shallow` 时，`tree_changes`、`linear_commits`、`bodies`、`present_blobs`、`refs_newest_first` 一律交回 git。测试先确认替换真的让 git 看到另一个答案，再断言快路径不接 |
| 2（阻断） | `worktree_branches` 只认 `ref: refs/heads/`（一个空格），git 认 `ref:` 后任意空白。别的写法会被当成游离，漏报一条已检出的分支，跟进就可能去移动它 | 已修。`head_branch` 只认 40 位 OID 与 `ref:` + 任意空白 + ref 名，其余（以及符号链接形式的 HEAD）交回 git。测试覆盖无空格、制表符 + CRLF、乱码 |
| 3 | `i18n.logOutputEncoding` / `i18n.commitEncoding` 不是 UTF-8 时 `git log` 会转码 | 已修，这时交回 git（中文 Windows 上可能设成 gbk）。有测试 |
| 4 | `local_branches`、`refs_newest_first` 遍历全部 ref 再按前缀筛：ref 多时白走，别处一个怪名字也会让整次退回 | 已修，改用 `references_glob`；`feature/x` 的测试确认 `*` 跨 `/` |
| 5 | 日志里 T2 还写着 `tree_changes` 用 `diff_tree_to_tree`；没说 `bodies()` 为什么接 merge | 已修 |
| 6 | 排序里「同名文件与目录」的平局规则永远走不到 | 已删，注释改正 |
| 7 | 退回 git 的端到端测试没有断言真的走了退回 | 已补前提断言 |
| 8 | `Item::key()` 每次比较都分配；`index_records` 在主仓库里把同一个目录扫两遍；partial clone 时 `cat-file` 会按需取回缺失对象；畸形 mode 两边归一的方式不同 | 记下不改：都是很小的开销或混合仓库碰不到的形状 |

### 为什么这么改是安全的

- 只换读，写操作（`update-index`、`read-tree`、`commit-tree`、`mktree`、`hash-object -w`、`update-ref`）一个没动。
- 每个快路径只在确定与 CLI 等价时返回 `Ok`；其余一律 `Err`，调用方经 `or_cli` 退回原来那条命令，代码原样保留。
- 每个替换都在 fixture 上与原命令逐个场景比较（`tests/git_read_follow.rs` 22 条）。实现时抓到一处分歧（assume-unchanged，那个读最后退回了 git），独立 review 又找出两处（替换对象、HEAD 的写法），都已拦住、有测试。

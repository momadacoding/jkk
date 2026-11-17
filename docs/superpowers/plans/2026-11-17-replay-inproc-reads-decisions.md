# 重放路径上的只读 git 调用改走 libgit2：实现日志

来源：[2026-11-16-follow-report-usability.md](2026-11-16-follow-report-usability.md) 的 C（§3.3、§8.2）。
没有单独的计划文件，本日志的 Task 划分即计划。A、B 的实现日志在那份计划的 `-decisions.md`；C 单独记一份，
免得两个 PR 同时改同一张总账表。

延续 PR 179（[2026-11-16-follow-git-inproc-reads-decisions.md](2026-11-16-follow-git-inproc-reads-decisions.md)）
的原则：只换读、写一律留在 git CLI；每个读经 `git_read::or_cli`，认不出的形状退回原来的命令；每个函数在 fixture
上与它替换掉的命令逐个场景比。

## 每个提交重放一次时的进程（PR 179 之后）

| 调用点 | 进程 | 读 / 写 | 每个提交 |
|---|---|---|---|
| `svn_replay::merge` | `merge-tree --write-tree` | 写 | 1 |
| `svn_replay::commit_like` | `log -1 --date=raw --format=…` | 读 | 1 |
| `svn_replay::write_commit` | `commit-tree` | 写 | 1 |
| `svn_replay::originally_empty` | `rev-parse X^{tree} P^{tree}` | 读 | 1（`drop_empty`） |
| `svn_replay::replay_onto` | `cur^{tree}`（没缓存时），已经走 `git_read::resolve`，不起进程 | 读 | 0 |
| `svn_replay::ls_entries` | `ls-tree` ×2 | 读 | 有要换回 SVN 版本的路径时 |
| `svn_replay::c_prime_tree` | 临时索引 + `write-tree` | 写 | 同上 |

2026-11-16 Client 仓库那次 11 个提交：`log -1` 11 次 0.9 s，`ls-tree` 9 次 0.7 s，`rev-parse` 等约 12 次 0.8 s。

## 进度总账

| Task | 状态 | 产出 | 验证 |
|---|---|---|---|
| C1 `rev-parse <rev>^{tree}` → `git_read::trees_of` | 完成 | 调用点 `svn_replay::{originally_empty, tree_without}` | `tests/git_read_replay.rs` 2 条 |
| C2 `log -1 --date=raw` → `git_read::commit_meta` | 完成 | 调用点 `svn_replay::commit_like` | `tests/git_read_replay.rs` 4 条；`tests/replay_inproc_identity.rs` 1 条（两条路重放出的 OID 逐个相同） |
| C3 `ls-tree --full-tree -z` → `git_read::tree_entries` | 完成 | 调用点 `svn_replay::ls_entries`、`svn_snapshot::entries_for` | `tests/git_read_replay.rs` 2 条 |
| C4 跟进路径上其余的只读调用 | 完成 | `svn_snapshot::raw_diff` 改用 PR 179 的 `git_read::tree_changes` | `tests/svn_follow_replay_budget.rs` 1 条 |
| C5 实测、review、PR | 完成 | 独立 review 1 阻断、1 建议、6 nit，处理见下 | `cargo fmt --check`、`clippy --all-targets` 无告警；碰到重放的 26 个测试二进制全过；review 修复后重跑了其中改动直接影响的 14 个（lib 177、svn_follow 137、svn_follow_auto 38、svn_sync_in 77 等）。全量 `cargo test` 在这台机器上两次被系统以内存不足停下（PR 183 同样），改为分批 |

## 结果

10 个提交的分支跟进一次（`svn_follow_replay_budget.rs`，2026-11-17 实测）：

| 进程 | 次数 | 说明 |
|---|---|---|
| `git merge-tree` | 10 | 写，每个提交一次 |
| `git commit-tree` | 12 | 写，每个提交一次 + 基线 / 钉住索引 |
| `git log` / `diff-tree` / `rev-list` / `ls-files` / `diff-index` / `hash-object` | 各 1–2 | 只读，与提交数无关 |
| `git rev-parse` / `ls-tree` / `cat-file` | 0 | |

改动之前，同一场景每个提交还有 `log -1`、`rev-parse X^{tree} P^{tree}`、快照规则的 `diff-tree P C` 各一个（第一次跑这条测试时 `diff-tree` 是 12 次，C4 之后是 1 次）。10 个提交约少 30 个进程；按 2026-11-16 的实测每个约 60–80 ms，约 2 秒。

## 决策条目

### C2 `commit_meta`

- **计划没写的坑**：`%ad` 用 `--date=raw` 时 git 把时区当整数按 `%+05d` 重新打印：提交里存的 `-0000` 打成 `+0000`；畸形时区（`+08`、`+9999`）git 有自己的容错。libgit2 的 `Signature::when()` 把时区换算成分钟，反算不回原样。所以不用 libgit2 解析出来的作者，直接取提交头里原样的 `author` 行，照 git 的 `split_ident_line` 拆，只接规整的形状（`名字 <邮箱> 秒数 ±四位`，一个空格分隔），别的交回 git。`-0000` → `+0000` 照 git 处理。
- **文档 / 记忆与实测冲突**：原以为 `log.mailmap` 默认开着时 `%an` 是映射后的名字，准备有 `.mailmap` 就交回 git。实测 git 2.51.2：`.mailmap` 生效时 `%aN` 变了，`%an` 仍是提交里原样的作者。按实测处理，不设 mailmap 的守卫；`commit_meta_ignores_mailmap_like_git_does` 钉住这一点。重放沿用的本来就该是原样的作者，映射之后再写进新提交反而是改了作者。
- **为什么这么改是安全的**：新提交的 OID 只取决于树、父提交、作者（名字、邮箱、日期）、提交者（环境）、说明。树与父提交不变；作者与说明由 `commit_meta` 给出，与原来的 `log -1` 解析逐字段相同（9 种作者 / 时区 / 说明组合 + 畸形 ident 要么交回 git、要么相同）；写入仍是同一个 `write_commit`。`replay_inproc_identity.rs` 用钉住的提交者把同一段分支各走一遍两条路，OID 逐个相同；把时区格式故意改坏，这条测试会变红（已验证，随后恢复）。

### C3 `tree_entries`

- **计划没写的坑**：`ls-tree` 不带 `-r` 时，问到的路径里有在某个目录下面的，它就钻进那个目录、不列目录自己，下面那条存不存在都一样（git 2.51.2 实测）。`svn_replay::ls_entries` 的「目录报内部错误」因此只在单独问到目录时生效，这是原来就有的行为，这里照样保留，没有借机改。
- 末尾带 `/`、空段、`.`、`..`、反斜杠的路径 ls-tree 另有解释，一律交回 git；树里存了非规范 mode 时两边归一的方式不同（PR 179 日志第 8 条），同样交回去。
- 测试里 `[`、`]` 留着（`--literal-pathspecs` 的意义所在），`*`、`?` 在 Windows 上建不出文件，没测。

### C4

- 第一次跑进程预算测试时发现 `diff-tree` 每个提交一次：快照规则（6.10）在有更新的基线时对每个提交比一次父提交。改用 PR 179 的 `tree_changes`（已与这条命令对照过）。
- 计划 §3.3 列的 `merge-base --is-ancestor`、`rev-list` 在这个场景里都是常数次，不随提交数增长，没有动。

### C5

- **偏离计划**：计划 §8.2 说先给 2 万文件 bench fixture 加分支提交再量时间。`tools/bench` 的 path 依赖指向主仓库的 `src-tauri`，量不到 worktree 里的改动；fixture 是共用的，改它会影响以后的对比。改为在测试里装性能记录器、数进程：结果确定，不受机器冷热影响，以后谁在每个提交的循环里加一次 git 读，测试会变红。时间上的收益按 2026-11-16 的实测单价推算，没有在 Client 仓库上复测。

### 独立 review（2026-11-17，opus subagent）

| 条目 | 级别 | 处理 |
|---|---|---|
| 1 提交头里有两条 `author` 时 git 取最后一条，`commit_meta` 取第一条：重放出的 OID 会变 | 阻断 | 已修：恰好一条才接，否则交回 git。`commit_meta_hands_malformed_idents_back_to_git` 加了两种重复写法，并断言 git 确实取最后一条。没有先看到红：旧实现会返回第一条，`check_raw` 的相等断言必然失败 |
| 2 进程预算只查列出来的几种读 | 建议 | 已修：反过来列写操作的白名单，其余任何 `git <子命令>` 都不许随提交数增长 |
| nit 3 头后面没有空行：libgit2 给空说明，git 的 `%B` 读到对象之外 | nit | 已修：原始对象里没有 `\n\n` 就交回 git，测试覆盖 |
| nit 4 CLI 按约 16000 字符分批，目录与它下面的路径分在两批时 CLI 会列出目录、进程内不列 | nit | 不改：只在路径总长超过 16000 字符时出现；进程内的结果反而与一次问完一致，影响只是少换 / 多拦一条路径 |
| nit 5 40 位十六进制走 `revparse_single`，对象不存在时 libgit2 会再试同名 ref | nit | 已修：改用 `Oid::from_str` + `find_object` |
| nit 6 等价重放测试没有走快照规则 | nit | 记下：`raw_diff`、`entries_for` 由逐函数对照测试覆盖（`tree_changes` 在 PR 179 的 `git_read_follow.rs`） |
| nit 7 `write_commit` 给 `GIT_AUTHOR_DATE` 传 `<秒> <时区>` 不带 `@`，1973-03-03 之前的作者日期会被 git 解析错 | nit | 不在本 PR：两条路一样（原有问题），修了会改变那些提交重放后的 OID，单独处理 |
| nit 8 本日志的进程表把 `cur^{tree}` 写成了进程 | nit | 已修 |

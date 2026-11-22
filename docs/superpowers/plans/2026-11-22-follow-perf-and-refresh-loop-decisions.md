# 2026-11-22-follow-perf-and-refresh-loop 实现日志

## 进度总账

| Task | 状态 | 产出 | 验证 |
|---|---|---|---|
| T1 冲突后刷新死循环 | 完成（根因待现场确认，见 T1） | `commands/watch.rs`：`.git` 下的目录事件不叫醒刷新；`repo-changed cause` 打点；`is_relevant` 公开。`tests/svn_follow.rs` 1 个、`watch.rs` 单测 3 个 | `cargo test --lib commands::watch` 13/13（连跑 3 次）；`cargo test --test svn_follow` 144/144；fmt / clippy 干净 |
| T2 手动跟进共用一次扫描 | 完成 | `svn_follow.rs` `manual_follow` 前置三项包进 `with_scan_cache`；新测试二进制 `tests/svn_follow_manual_scan_budget.rs` | 改前 2 遍 → 改后 1 遍；`svn_follow manual` 35/35；`svn_follow_scan_budget` 1/1；fmt / clippy 干净 |
| T3 floor shape 只读一次 | 完成 | `svn_cli::scan_scope_generation`；`svn_wcdb::floor_shapes` 按（代次, 仓库, 单元, floor）缓存；`svn_follow_scan_budget` 加断言；`svn_cli.rs` 1 个测试 | 改前 2 次 → 改后 1 次；svn_cli 29、svn_wcdb 32、svn_health_local_floor 33、svn_health 51、svn_follow_auto 40、svn_follow 144、svn_init 100、svn_baseline 31、reuse_base 1、reuse_safety 2 全过；fmt / clippy 干净 |
| T4 暂存区预判 | 未开始 | | |
| T5 验证、review、PR | 未开始 | | |

## 决策条目

### T1

- **偏离计划**：计划第 2 步预期复现测试是红的。实际是绿的：在 fixture 里停在 rebase 冲突上（journal
  `FollowManual / Rebase`），按前端处理函数调一轮全部读命令（含 `read_conflicts`、冲突文件的
  `read_diff`、第二档 `svn_sync_status`），监听器一条相关事件都没看到。
  逐个核对了现场每轮会跑的读命令，都不写 `.git`：`read_conflicts`（libgit2 + `git check-attr` +
  `git config`）、`read_status`（SVN 后端直接转给 git 后端，libgit2 只读）、`svn_sync_state`（锁探测
  有测试钉住不产生事件）、`check_merge_drivers`（按 mtime 缓存，只读）、`svn_pending_restores`（有 journal
  时直接返回）、`with_watch_suspended`（只在流程结束时补发一次）。所以**现场源头在 fixture 里没复现**。
  Ruling：测试保留为回归守卫（名字说的就是它测的：冲突态一轮读不写监听会认的路径）；根因改由第 5 步的
  `repo-changed cause` 打点在现场确认 —— 代价：如果现场源头不是下面这一类，循环还在，要等下一份
  `--perf` 日志再修。
- **计划没写的坑（实测，2026-11-22，Git for Windows）**：
  1. `.git` 下建了又删一个文件时，Windows 对**目录本身**报一条 `Modify(Any)`。索引已是最新时，一条
     只读的 `git status --porcelain` 也会建了又删 `.git/index.lock`，留下一条 `.git` 的目录事件。
     `is_relevant` 原来把它当相关（`ends_with("/.git")`），分到 `IfNotOwn`，核对发现不是自己写的，于是
     **照常刷新**。IDE 的 git 插件、TortoiseGit 每次刷新都会这样。这是能确认的一类「什么都没变却叫醒刷新」。
  2. 已有的 `stage_and_unstage_only_touch_the_index` 在本机约 60% 失败，失败信息是 `.git/hooks` 目录事件
     （刚 `git init` 完，有进程碰了 hooks 目录）。同一类：目录事件、不带信息。改完之后连跑 3 次全过。
  3. 文件与 index 的 mtime 落在同一秒时（racily clean），之后每次 `git status` 都会**重写 index**。这是真改动，
     不该过滤；测试里要先等过这一秒，否则测到的是它。
- **修法**：`is_relevant` 在 `.git` 分支里，对仍存在的目录返回 `false`。
- **为什么这么改是安全的**：真有改动时被改的那个文件总有自己的事件（`index`、`HEAD`、`refs/heads/x`、
  `packed-refs`、`rebase-merge/*`），`what_our_own_index_write_touches…` 的注释早就这样论证过 gitdir 与
  `objects` 两个目录事件。删掉的目录查不到类型，按文件处理、照常刷新。工作区里的目录不受影响（单测覆盖
  新建空目录 `src`）。`OwnWritePaths` 的 gitdir / objects 两项保留：`--separate-git-dir` 的 gitdir 路径里
  没有 `.git`，走不到这条规则。
- **perf 打点**：每次发 `repo-changed` 写一条 `kind: "watch", name: "repo-changed cause"`，`meta.path` 是这一批
  里第一条 `Wake::Yes` 的路径，没有时取第一条 index 一类的。只在 `--perf` 下攒路径。
- **现场验收（未做）**：报告 §2.1 的验收「停在冲突上空闲 1 分钟 `repo-changed` 为 0」只能在公司机器上带
  `--perf` 验；`repo-changed cause` 会直接给出源头路径。

### T2

- **偏离计划**：无。`require_changes_in_whitelist` 也一起包进范围：它在 `run_preflight_with` 之后、
  `svn_journal::write` 之前，同样只读。
- **计划没写的坑**：改前实测 `manual_follow` 期间真实执行的全量扫描正好 2 遍（与现场一致），stash 与 rebase
  之后的步骤一遍都没有，所以断言写成精确的「1 遍」。
- **为什么这么改是安全的**：范围止于 `svn_journal::write` 之前，`stash_wip` / rebase 改工作副本之后的读取
  不在缓存里。缓存范围内的两次检查之间没有任何写操作。手动跟进的 35 个集成测试全过。

### T3

- **第 1 步查出来的（临时打日志，已删）**：整次自动跟进里两次读来自 `svn_health.rs` 的预跑
  （`local_floor`，第 794 行，判哪些单元本地回答、提前发命令）和体检本身（第 1346 行）；floor 相同
  （fixture 里都是 2），都在 `follow_auto` 的 `with_scan_cache` 范围里。部分跟进的现场
  （`a_replay_conflict_on_one_file_leaves_only_that_file_behind`）一共 4 次，floor 都是 1，也都在同一范围。
  所以本 Task 照做。
- **偏离计划**：缓存键多了仓库根（同一线程理论上可能在一个范围里碰两个仓库）。代次除了「最外层范围开始」
  之外，`forget_node_scans` 也换代 —— Ruling：R3c 用它表达「要读最新的节点 revision」，按节点 revision 算出的
  floor 形状跟着作废 —— 代价：R3c 之后若再体检会多读一次 wc.db（约 1.5 s），不影响正确性。
  `bypass_scan_cache` 期间 `in_scan_cache()` 为假，代次是 `None`，不读也不写缓存。
- **为什么这么改是安全的**：范围的约定本来就是「这段时间里的 `svn info` / `svn status` 是同一份快照」
  （`with_scan_cache` 的注释：范围内不发生 SVN 写操作；外部改动由服务器体检和范围外的锁内复核兜住）。
  wc.db 形状属于同一份快照，与范围内缓存的 status 一致。测试注入点 `FloorShapeRefused` 在缓存之前，
  照常生效。

### T4

### T5

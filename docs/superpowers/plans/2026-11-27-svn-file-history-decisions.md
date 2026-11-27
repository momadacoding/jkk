# 混合仓库文件历史与 Blame：实现日志

计划：`2026-11-27-svn-file-history.md`　设计：`../specs/2026-11-27-svn-file-history-design.md`

## 进度总账

| Task | 状态 | 产出 | 验证 |
|---|---|---|---|
| 0 实现日志 | 完成 | 本文件 | — |
| 1 带超时的 svn 查询 | 完成 | `svn_cli.rs`：`run_svn_deadline`、`classify_svn_error`、`SVN_QUERY_TIMEOUT` | `cargo test --lib deadline_tests` 4/4；fmt、clippy 干净 |
| 2 路径 → URL 共用 | 完成 | `svn_wc.rs`：`unit_of`、`server_url`、`server_url_at`、`strip_prefix_ci`、`units_cached` | `url_tests` 2/2；svn_health 五个集成测试二进制 105/105 |
| 3 基线标注 | 完成 | `file_history.rs`：`EntrySvn` / `BaselineInfo` / `DeliveredState`、`SvnAnnotator`；`tests/file_history_svn.rs` | `file_history_svn` 7/7，`file_history` 27/27；fmt、clippy 干净 |
| 4 分组纯函数 | 完成 | `svn_file_history.rs`：`GroupSpec`、`Cursor`、`SvnRevEntry`、`bucket`、`next_cursor`、`PAGE` | `cargo test --lib svn_file_history` 11/11 |
| 5 `svn_file_history` | 完成 | `svn_file_history.rs`：`svn_file_history`、`page_with_units`、进程内 LRU；命令注册 | `file_history_svn` 18/18、`--lib svn_file_history` 12/12；改坏换 peg 的变异让 2 个测试失败 |
| 6 `svn_rev_file_diff` | 完成 | `svn_file_history.rs`：`svn_rev_file_diff`、`svn_cat`、`tree_with_file`；`svn_cli.rs`：`run_svn_deadline_bytes`；命令注册 | `file_history_svn` 23/23；去掉 mempack 的变异让 `no_object_is_written_to_the_users_git` 失败（10 ≠ 5） |
| 7 `svn_blame_overlay` | 未开始 | | |
| 8 前端 store 与纯函数 | 未开始 | | |
| 9 FileHistoryView | 未开始 | | |
| 10 真实窗口验证与收尾 | 未开始 | | |

## 决策

### Task 0

- **偏离计划（实为偏离 spec §7）：** spec 要求超时时结束 svn 进程「含子进程」。本实现只结束 svn 主进程。结束进程树要给 `windows` crate 加 `Win32_System_JobObjects` feature，计划的全局约束不加 feature。svn+ssh 下的 ssh 子进程预计在管道关闭后自行退出，**未测**，列为风险。

### Task 1

- **计划没写的坑（svn 1.14.5 实测 stderr）：**
  - 拒绝连接的端口：`E170013: Unable to connect to a repository at URL …` 接着 `E730061: Can't connect to host …`
  - `file://` 仓库里不存在的路径：`E160013: File not found: revision 0, path '/nope'`
  - 不回应的服务器：svn 一直等，没有自己的超时，只能靠 `run_svn_deadline` 结束
- **为什么这样分类是安全的：** 认证失败时 svn 也会先报一行 E170013，所以先查认证码（E170001 / E215004，**未实测**），再查路径码，最后才查连接码。归错类只影响提示的那句话，不影响数据。
- **偏离计划：** 多加了一个测试 `success_returns_stdout`，确认正常路径能拿到 stdout。超时后不等读管道的线程结束（孙进程可能还攥着管道）。

### Task 2

- **偏离计划：** `strip_prefix_ci` 也一起挪进 `svn_wc`（`pub(crate)`）。`svn_health` 里还有别处用它，留两份会分叉。
- **偏离计划：** `units_cached` 的缓存键多了一项「基线分支 tip」。更新 / 跟进之后可能多出新的 external，只按白名单做键，新 external 下的文件要等重启才换算得出 URL。缓存只留最近一份。
- **为什么是安全的：** `unit_of` / `server_url_at` 原样搬家，`svn_health` 的行为不变，回归用它的五个集成测试二进制（105 个用例）全部通过。

### Task 3

- **偏离计划：** `BlameCommit` 除了 `baseline` 还加了 `delivered`。spec §6.3 要求 Blame 里已交付的提交显示「已交付 rN」，计划只给了 `baseline`，漏了。
- **偏离计划：** 读提交说明不走 `git_read::commit_message`（每次都重开一次仓库，1000 条就开 1000 次），在同一个 `Repository` 上读 `message_raw_bytes`，与 `commit_message` 读的是同一份原始字节。
- **偏离计划：** 多加了 `a_colleague_update_is_a_baseline_not_a_delivery`。`a_delivered_commit_carries_its_svn_rev` 只断言本地提交的「已交付 rN」：交付录的基线树与交付的提交相同，`git log --follow` 不列它，读不到它的标注；`Delivered-Rev` 的解析改在 `mixed_revision_baseline_uses_rev_of_not_floor` 里用自建基线断言。
- **计划没写的坑：** Blame 里一个提交的 `file_rev` 要按路径取，`commits` 表里没有路径，取该提交第一次出现那一行的 `orig_path`。
- **为什么判定是对的：** `svn/base` 的 first-parent 链是一条祖先线，「能到达某提交的基线」在链上是一段后缀，用二分找第一条，`graph_descendant_of` 只调 O(log n) 次。不是混合仓库时 `read_config` 失败，什么都不标，纯 git 仓库的输出只多一个 `null` 字段（`plain_git_repo_has_no_svn_annotation`）。

### Task 4

- **计划的错误：** 计划要求 `file_rev` 不严格递减时报 `bad_history_args`（测试 `non_decreasing_file_revs_are_rejected`）。这会让某些真实文件的 History 直接报错：重建基线可能录出与更早那条相同的 revision（内容回到旧版、git 里却算一次改动）。改为容错的归属规则：X 归 `file_rev ≥ X` 里最小的那条，相同时归最老的。`file_rev` 严格递减时与 spec §5.3 的半开区间完全等价。测试换成 `equal_file_revs_go_to_the_oldest_of_them`。
- **计划的错误：** 计划写「这一页一条都没查到时 O = `peg_rev + 1`」。翻过几页满页之后，这一段其实已经追到了上一页的最老那条，用 `peg_rev + 1` 会把已经查过的组当成「够不着」再查一遍。改为 O = `next_top + 1`；第一页时 `next_top == peg_rev`，与计划一致。补测试 `an_empty_page_after_full_pages_keeps_what_earlier_pages_reached`。
- **计划没写的坑：** 换 peg 只在当前 peg 那组之后（更老的组）里找。否则一条 peg 什么都没查到时会选回自己，死循环（`an_empty_segment_does_not_requery_its_own_group`）。
- **偏离计划：** 多加了 `a_full_page_ending_at_r1_is_exhausted`：满页但最老的已是 r1，不能再发一次 `-r 0:1`。

### Task 5

- **计划的错误：** `hidden` 只取基线的 `Delivered-Rev` 不够。交付录的基线树和交付的提交相同，`git log --follow` 常常不列它，于是交付的 rN 落进下一条基线的组里，和「已交付 rN」的本地提交重复出现。`hidden` 改为基线的 `deliveredRev` 加上本地提交 `delivered.rev` 的并集（测试 helper `hidden_of` 与前端同口径，Task 8 的 `hiddenRevs` 照此实现）。
- **偏离计划：** 「初始化前已有 60 次提交」的 fixture 做不出来（`hybrid_with_colleague` 在 r1 后立刻初始化），改成初始化后同事提交 60 次再更新，测的是同一件事：一页 50、cursor 取剩下的、跨组取完后为 None（`sixty_revisions_page_by_fifty`）。
- **偏离计划：** 「没有调 svn」用一个不存在的仓库路径证明：没有基线组时连仓库都不碰（`a_file_never_in_a_baseline_makes_no_svn_call`）。
- **偏离计划：** 服务器上这个 peg 处没有该路径（`svn_path_missing`）当作这一段为空、继续看更老的组，不报错。
- **计划没写的坑：** 一页内换 peg 时，后一段的 `-l` 要用这一页剩下的额度，`next_cursor` 判「满页」也用这个额度，否则一页会超过 50 条或提前截断。
- **验证测试有效：** 把换 peg 改成永不发生，`git_rename_delivered_switches_peg`、`delete_and_recreate_switches_peg` 失败；改回后全过。

### Task 6

- **计划没写的坑：** `run_svn_deadline` 把 stdout 有损地转成 UTF-8 字符串。`svn cat` 读的是文件内容（GBK、二进制都有），先转字符串就改了内容。新增 `run_svn_deadline_bytes`，`run_svn_deadline` 改成它的薄包装，Task 1 的 4 个测试照过。
- **计划里「未核实」的那条已核实：** 在这次打开的 `Repository` 的对象库上 `add_new_mempack_backend(1000)` 之后，`blob` / `treebuilder` 的写入都进内存，`.git/objects` 的文件数不变。去掉这一行，同一个测试多出 5 个对象文件，确认测试有效。不需要临时裸仓库的退路。
- **偏离计划：** 多加了 `binary_content_is_reported_as_binary`（含 NUL 的内容按二进制显示），以及参数防护：`peg_path` 为空、`rev == 0`、`peg_rev < rev` 返回 `bad_history_args`。路径不在任何白名单单元底下返回 `svn_path_missing`。
- **为什么是安全的：** 只读。`svn cat` 不碰工作副本，diff 的对象只在内存里，打开的 `Repository` 用完即丢。

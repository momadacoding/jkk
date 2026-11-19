# 2026-11-20-svn-changelist-keep 实现日志

## 进度总账

| Task | 状态 | 产出 | 验证 |
|---|---|---|---|
| T1 wc.db 查 changelist | 完成 | `svn_wcdb::changelists`、`svn_xml::parse_changelist_status`；`tests/svn_wcdb.rs` 8 个 `changelists_*` | `cargo test --test svn_wcdb` 32/32 |
| T2 记录 / 补挂 / 核对 | 完成 | `engine/svn_changelist.rs`；`tests/svn_changelist.rs` 16 个 | `cargo test --test svn_changelist` 16/16 |
| T3 接入流程与命令层 | 完成 | `svn_sync_in.rs`（S0 之后 S1 之前）、`svn_follow.rs`（`manual_follow` stash 之前）、`svn_sync_out.rs`（D0.5 之后 D3.8 之前）、`commands/svn_sync.rs`（`with_watch_suspended`） | 见 T4 |
| T4 端到端测试 | 完成 | `svn_sync_in.rs` 2 个、`svn_sync_out.rs` 2 个、`svn_follow.rs` 1 个 | 5/5 |
| T5 验证、review、PR | 进行中 | | |

## 开工前实测（2026-11-20，svn 1.14.5，临时 fixture）

- `svn revert` **不会**摘掉 changelist：revert 之后 `svn status --xml` 里文件仍在 `<changelist>` 块里，
  `item="normal"`。所以丢 changelist 只可能来自显式的 `svn changelist --remove`（或 commit）。
- `svn status --xml` 会列出挂了 changelist 但没有修改的文件（`item="normal"`）。
- wc.db 格式 31：`ACTUAL_NODE(wc_id, local_relpath, …, changelist TEXT, …)`；
  `EXTERNALS(wc_id, local_relpath, parent_relpath, repos_id, presence, kind, def_local_relpath, …)`。
- 本开发机 ACP 1252：`svn changelist` 的**参数**里有中文（文件名或列表名）时，svn 按 ACP 转换成 `??`，
  报 W155010 / E200009；`--targets` 文件（UTF-8）同样找不到文件。中文列表名作参数时直接报
  E205001「Not enough arguments」。这是 svn.exe 在非中文 ACP 上的限制，与本功能无关，但意味着
  **中文文件名 / 列表名在 ACP 不是 936 的机器上补挂会失败**，只能落日志。

## 决策条目

### T1

- **偏离计划**：无。
- **计划没写的坑**：
  - 不复用 `svn_wcdb::excluded_nodes` 的单元（`WcIdentity`）：拿单元要 `svn_wc::enumerate_wcs`，它起
    `svn info` 并遍历一遍白名单找 external（秒级）。改为从白名单条目往上找 `.svn/wc.db`，external
    从 `EXTERNALS` 表读、逐层打开各自的 wc.db，全程不起进程。
  - 文件 external 没有自己的 wc.db，它的 changelist 在所在工作副本的 `ACTUAL_NODE` 里，子树查询已覆盖；
    目录 external 没检出（没有 `.svn/wc.db`）时跳过。
  - 中文目录下的文件：测试里以那个目录为 cwd 挂 changelist（同 `a_non_ascii_directory` 的做法）。
- **为什么安全**：只读打开（`SQLITE_OPEN_READ_ONLY`），与 `excluded_nodes` 同一套「认不出就不读」：
  格式不是 31、`WORK_QUEUE` 非空、`WCROOT` 不唯一都返回 `Err`，调用方改用 `svn status --xml`。

### T2

- **偏离计划**：
  1. 记录文件独立于 journal（`.git/mygitclient-changelists.json`），不按审阅意见写进 Journal 字段。
     原因：journal 在 `restore_step_with` 的五个收尾点被删，延迟核对发生在那之后；单独一个文件只有
     一个生命周期（record 写、recheck 删），不用改 journal 的线序格式。流程中途崩溃时它照样在磁盘上。
  2. 补挂不在 `restore_step` 里做，而在命令层 `with_watch_suspended` 流程函数返回之后做。原因：收尾
     出口很多（Done / Settled / StageLost / 放弃 / 交付各分支），挂在命令层一处覆盖全部；journal
     还在（停在冲突上）时不补挂，由 `settle` 判断。代价：端到端测试要显式调 `settle`（引擎函数不带它）。
- **计划的错误**：我最初写「已经不在磁盘上的路径会让 `svn status` 整批报错」，据此预先过滤掉了不在
  磁盘上的文件。实测（svn 1.14.5）：不在工作副本里的路径（连父目录都不在）只给空 `<target>` 和一条
  W155010 警告，退出码 0。删掉了预过滤，这样 `svn delete` 掉的文件（不在磁盘上、`item="deleted"`，
  IDE 列成「本地删除」）也能挂回去，补了 `a_file_scheduled_for_deletion_gets_its_list_back`。
- **计划没写的坑**：
  - 列表名以 `-` 开头时会被当成选项：命令写成 `svn changelist -- <名> <路径>…`，测试钉住。
  - 文件名带 `@`：问状态与补挂都过 `peg_safe`，测试钉住。
  - `missing`（登记着、磁盘上没有）不算修改，不补挂；`deleted` 算。属性修改（`props="modified"`）算。
- **为什么安全**：
  - 只对记录里的路径发 `svn changelist <名>`，不发 `--remove`，不碰内容；已经在别的列表里的文件不动。
  - 记录、补挂失败只写日志，不改变流程的结局；recheck 之后无论成败删记录，不会无限重试。
  - `settle` / `recheck` 先拿同步锁，拿不到就跳过；`recheck` 只处理自己 opid 的记录，不会删掉下一次
    流程刚写的那份（`recheck_leaves_a_newer_flows_record_alone`）。

### T3

- **偏离计划**：
  1. `sync_out` 的记录点放在 D0.5（pin 交付 ref）之后、D3.8 之前，不是 D0 之前：D0.5 不动磁盘，
     记录离第一次动磁盘越近，用户在确认框之后再改 changelist 的空档越小。
  2. 边界情况（审阅第 6 条）定为**挂回去**：同一文件既有交付出去的改动、又有 Local-Only 补丁时，
     svn commit 摘掉了它的 changelist，但交付后它仍有修改。用户还有没提交的改动，留在原列表里更符合
     IDE 里的预期。`a_partly_delivered_file_that_still_carries_a_local_only_patch_gets_its_list_back`
     钉住。
- **为什么安全**：三处记录都在持同步锁、journal 已写之后、第一次动磁盘之前；`record` 不返回错误，
  不会让流程在动磁盘之前多一个失败出口。

### T4

- **计划没写的坑**：「IDE 摘 changelist」无法在流程中途注入（`sync_in` 没有 probe）。更新与手动跟进
  用冲突出口停住流程、在暂停期间模拟；交付用现有的 `run_sync_out_probed` 在 `Step::CheckedOut` 模拟，
  顺带断言记录发生在 D3.8 之前（否则记录里没有 B，`settle` 返回 `None`）。

## 遗留

- 补挂失败只写日志，界面上没有提示。中文文件名 / 列表名在 ACP 不是 936 的机器上一定失败（见开工前
  实测），用户看不到。要提示需要前端监听事件，另开任务。
- 10 秒是经验值：IDE 一次 VCS 刷新在大工程上若超过 10 秒，晚到的 `--remove` 仍可能漏掉；下一次流程
  开始时不会再补（那时记录已删）。
- 11-17 那次（IDEA 提交后全部 SVN 标记消失、没有 stash/rebase）不在本次范围，原因仍不明。

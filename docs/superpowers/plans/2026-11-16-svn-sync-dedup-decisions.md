# SVN 更新 / 交付去重（第 77 号 issue，第一批）决策日志

没有单独的计划文件：任务范围直接来自 issue 77 的「重复点」表。约束同 issue：纯重构、行为零变化，
既有 `svn_sync_in` / `svn_sync_out` / vitest 原样通过、不改断言。

## 进度总账

| Task | 内容 | 状态 | 产出文件 | 验证 |
|---|---|---|---|---|
| 1 | `now_secs`、`short` 合成一份 | 完成 | 新 `engine/svn_util.rs`；`svn_deliver` `svn_replay` `svn_snapshot` `svn_follow` `svn_lock` `svn_sync_in` `svn_sync_out` `svn_sync_check` `svn_sync_out_check` `svn_local_only_edit` | fmt / clippy 干净；`svn_util` 单测 |
| 2 | `checkout_verified` / `checkout_detached` 共用「切完核对」 | 完成 | `svn_sync_in::run_checkout_verified`；`svn_sync_out::checkout_detached` 改走它 | fmt / clippy 干净 |
| 3 | svn status 条目的三个判据（本地改动、只因不见、祖先目录）收进 `svn_xml` | 完成 | `UpdateStatusEntry::is_local_change` / `is_only_missing`、`ancestor_dirs`；`svn_sync_check` `svn_sync_out_check` 改用 | `cargo test --test svn_xml` 51 通过（新增 3 个） |
| 4 | 前端两个流程的 `afterSync` / `exclusive` / 结局提示 | 完成 | 新 `src/components/svnFlowShared.ts`；`svnSyncFlow.ts` `svnSyncOutFlow.ts` | `pnpm build` 通过；`pnpm test` 1769 通过，未改任何断言 |

## 本批不做的（与原因）

| issue 里的重复点 | 不在本批的原因 |
|---|---|
| `take_over`（更新 / 交付） | **不是纯重复**：交付侧拿锁后核对 `opid` 与 `flow` 没被换掉，更新侧没有。合成一份要么给更新侧加上核对、要么去掉交付侧的，都改行为。留给第 75 条（更新流程遗留问题）一并决定 |
| `capture_delivered` / `publish_delivered` 与 `sync_in` 的录制 / 发布 | 改动面大、牵涉 journal 的 phase 推进，单独一批 |
| `finish` 与 `commits_between` 各解析一遍 log；A5 回原分支；rebase 的 `-c` 参数 | 同上，下一批 |
| `svn_migrate_eol` 的 `MigrationLock` | 去掉它是**行为变化**（崩溃残锁的处理方式变了），不属纯重构 |
| `git-svn-id` 解析正则 Rust / TS 各一份 | 跨语言，合成一份要把解析挪到一侧、改命令返回形状，是架构改动 |

## issue 里「另一个待确认的点」：交付侧 `refreshing` 是否多余

结论：**对最终状态不是必需的，但不是零作用，本批保留。**

- 后端 `with_watch_suspended`（`commands/svn_sync.rs:192-207`）无论成败都补发 `repo-changed`，交付的三个命令都包着它；
  前端 `repo-changed` 监听（`repoStore.ts` 约 1776 行）会跑 `refreshSvnSyncState`。所以失败之后横幅迟早会跟上。
- `refreshing` 多做的两件事：错误抛回对话框**之前**横幅已经是新 phase（用户看错误时按钮与文案是对的）；
  多跑一次完整的 `refreshSvnSync` 与 `fetchSvnLog`（监听器上只跑分层的 `checkSvnSyncIfChanged`）。
- 去掉它或给更新侧补上都改变可见的先后顺序，属行为变化，不在纯重构范围内。

## 决策条目

### Task 1

- **偏离 issue 的范围**：issue 只点了 `now_secs` 与 `short()`「未复用」。实际找到 `now_secs` 4 份（`svn_follow`、`svn_lock`、
  `svn_sync_in`、`svn_sync_out`）、`short` 3 份（`svn_deliver`、`svn_replay`、`svn_snapshot`），外加 10 处内联的
  `&x[..x.len().min(10)]`，一并换掉。`commands/worktree.rs` 也有一份 `now_secs`，不在 SVN 模块里，没动。
- **为什么是安全的**：三份 `short`、四份 `now_secs` 逐字相同（迁移前用脚本按整段文本匹配删除，匹配不上即中止）。
  内联截取的 10 处逐个看过，截的都是 OID（`rev_parse` 的输出、journal 里的 oid、log 的 `%H`），ASCII，按字节截不会截在
  字符中间；`short` 的文档写明不能拿它截分支名之类的文字。
- **坑**：工作区文件是 CRLF。脚本先统一成 LF 匹配、写回时还原；`cargo fmt` 又把改过的文件写成 LF，git 提交时按
  autocrlf 归一，diff 里不出现整文件的行尾变化。

### Task 2

- **为什么是安全的**：两个函数的步骤与顺序原本完全相同（记 HEAD → 跑 checkout → 判 HEAD 在不在目标 → 列已跟踪改动 →
  列该删没删的 → 报错），差别只有 checkout 的参数、「在不在目标」的判法、报错里的目标名。三样都作为参数传进去，
  「在不在目标」的闭包仍在 checkout 之后、列文件之前调用，短路求值（`current_branch` 不为空就不再 `rev_parse`）不变。

### Task 3

- **没合成一个判据**：两边「被删掉的目录不算改动」的规则**有意不同**——更新侧多一条「目录底下没丢掉任何非 W-file」
  （`svn_sync_check.rs` 的注释写了原因：更新侧只列带 incoming 的条目，目录条目是唯一的提醒）。合成一个会改变其中一边的结论。
  只把两边逐字相同的原子判据收走：`is_local_change`（原来两边各写一遍 `matches!`）、`is_only_missing`、`ancestor_dirs`
  （原来两边各写一个 `rsplit_once` 循环）。更新侧 `lost_non_w` 的触发条件是 `item == "missing"`（不看属性），与
  `is_only_missing` 不同，保持原样没换。

### Task 4

- **为什么是安全的**：`afterSync` 两份逐字相同（只有注释不同），合并后的注释保留了两边的理由；`exclusive` 两份逐字相同，
  且两个流程本来就传进同一个 `syncFlowRunning`（`RepoTab.tsx` 1061、1116 行），共用一个函数不改变互斥范围；
  结局提示只收 `confirm(...)` 那一句，各自选哪个 `describe*` 仍在各自流程里。
- 交付侧的 `refreshing` 留在原处，注释补上了本日志「另一个待确认的点」的结论。

## 独立 review（opus subagent，只读）

结论：逐段对照 HEAD 版本，**未发现行为变化**（求值顺序、短路、集合内容、`short` 的截取对象、前端 null 处理与互斥范围都核对过）。
低危两条已修：`checkout_failed` 的文档改成「经 `run_checkout_verified`」；`createAfterSync` 的注释补回指向
`SvnSyncFlowDeps.refreshSvnSync` 的交叉引用。第三条（新单测只测原子判据、组合判定靠既有 fixture 测试）不需要改：
`cargo test` 全量 100 个 suite / 2123 通过，含 `svn_sync_in`、`svn_sync_out`、`svn_sync_out_check`。

# SVN 更新 / 交付去重（第 77 号 issue，第二批）决策日志

第一批见 `2026-11-16-svn-sync-dedup-decisions.md`（PR 178），本批接它「本批不做的」表里标「下一批」的几项。
约束不变：纯重构、行为零变化，既有 `svn_sync_in` / `svn_sync_out` / vitest 原样通过、不改断言。
报错文字也算行为：两边措辞不同的地方各自保留，只收步骤。

## 进度总账

| Task | 内容 | 状态 | 产出文件 | 验证 |
|---|---|---|---|---|
| 1 | rebase 钉住的 `-c` 配置合成一份 | 完成 | `svn_util.rs`（`PINNED_REBASE_CONFIG`、`pinned_rebase`）；`svn_sync_in` 两处、`svn_sync_out` 两处 | `svn_util` 单测 1 条（新增） |
| 2 | 基线 ref 的 CAS 发布（`finish_publish` / `publish_delivered`）合成一份 | 完成 | `svn_sync_in::publish_baseline_ref` | 见下方验证 |
| 3 | A5「W-file 全部干净才 `-f` 回原分支」合成一份 | 完成 | `svn_sync_in::force_back_when_w_clean` | 见下方验证 |
| 4 | 录制核心「按单值 revision 录、录完复核没被挪」合成一份 | 完成 | `svn_sync_in::record_single_rev`、`RevisionMoved` | 见下方验证 |

验证（四个 Task 一起跑，2026-11-23）：`cargo fmt --check`、`cargo clippy --all-targets` 干净；`cargo test --lib` 179；
`svn_sync_in` 79、`svn_sync_out` 52、`svn_sync` 18、`svn_sync_stash` 19、`svn_sync_check` 23、`svn_sync_out_check` 16、
`svn_deliver` 11、`svn_journal` 17、`svn_baseline_guard` 14、`svn_follow_snapshot` 12、`svn_respect` 10、`svn_write_guard` 9、
`svn_follow` 143，全部通过，没改任何断言。全量 107 个二进制没跑（内存回收），挑的是调到这几个函数的。

## 本批不做的（与原因）

| issue 里的重复点 | 不做的原因 |
|---|---|
| `finish` 与 `commits_between` 各解析一遍 log | **不是同一个解析**：`finish` 按 `git log` 默认顺序（新到旧）取 `%H %s %B`，报错里「没交付的提交」按这个顺序列；`commits_between` 走 libgit2（失败退回 CLI），`--topo-order --reverse`、多取 `%P`，要求两端是 OID，失败时报调用方给的错误码。换过去要先 `rev-parse` 两个 ref、倒序输出，报错顺序在有 merge 的区间里还可能变，错误码也变。收益只是少几行解析，不值得冒改行为的险 |
| 更新侧 `rebase --continue` 没带钉住的 `-c` | 交付侧带、更新侧不带。`--continue` 时 autoStash 已不起作用、updateRefs 读的是 rebase 开始时记下的状态，两边结果多半相同，但统一就要改其中一边的命令行，不属纯重构。本批两边各自保持原样 |
| `take_over`、`MigrationLock`、`git-svn-id` 正则 | 原因同第一批 |

## 决策条目

### Task 1：rebase 钉住的配置

- **为什么是安全的**：四处原来的参数表逐个对过，`pinned_rebase(rest)` 拼出来的与原数组逐项相同、顺序相同
  （钉住的两项在最前，手动跟进多出的两项 `-c` 接在后面，再接 `rebase` 子命令）；单测钉住了拼法。原来 S9 那段
  解释为什么要钉的注释搬进了常量的文档，调用处留一句指过去。
- 更新侧的 `rebase --continue` 原来就**不带**这两项，本批没给它加（见「本批不做的」）。

### Task 2：基线 ref 的 CAS 发布

- **为什么是安全的**：两边的三分支 `match` 逐字相同（已是新的 → 不动；是旧的 → `move_baseline_ref` CAS；
  否则报 `svn_baseline_moved`），只差报错里「更新前 / 交付前」和交付末尾多的一句。两处差异作为参数传入，
  拼出来的报错与原来逐字相同。发布之后的索引处理两边**不同**（更新从旧基线换、交付从交付树换并夹着切分支），
  留在各自原处没动。

### Task 3：A5

- **为什么是安全的**：两边都是「列不干净的 W-file → 有就报错 → 没有才 `checkout -f` 原分支」，顺序相同。
  报错文字两边措辞不同（更新侧用「——」、交付侧用「，」，按钮名不同），所以报错由调用方的闭包构造，原文照搬。
  更新侧只在 `Disk::MaybeUpdated` 时才查，否则直接 `-f`：改成 `if 查 { helper } else { 直接 checkout }`，两条路径与原来相同。

### Task 4：录制核心

- **为什么是安全的**：两边都是「`record_server_tree_into` → 再读 `nodeRevs` → 不等就报错 → 造单值 `BaseMap`」。
  交付侧原来在写 journal **之后**才造 `BaseMap`，现在在录完时就造好：`BaseMap::single` 只组装结构体、不碰磁盘，
  提前不改变任何可观察的东西。`run_preflight` 与 `TempIndex::from_head` 的先后两边各自保持原样（更新侧先建索引、
  交付侧先 preflight），都留在调用方。revision 被挪了时返回 `RevisionMoved`，`Display` 写成 `r{before} → r{now}`，
  与两边原来的 `format!` 片段相同；错误码和整句仍由各自调用方给。

## 独立 review（/code-review high，2026-11-23）

结论：四处抽取逐项核对，**未发现行为变化**（参数表、报错文字、`MaybeUpdated` 分叉、`BaseMap` 提前构造都核对过）。

| 发现 | 处理 |
|---|---|
| 更新侧 `rebase --continue` 不带钉住的 `-c`，与交付侧不一致 | 既有差异，本批有意不动（「本批不做的」第 2 行）；要统一另开行为变更 PR |
| `PINNED_REBASE_CONFIG` 的文档说「一律带上」，言过其实 | 已修：写明只在开始 rebase 与交付侧的继续时带，点名更新侧继续与 `--abort` 是例外 |
| `RevisionMoved` 的报错格式没有测试钉住 | 已修：`svn_sync_in` 单测 `revision_moved_reads_before_then_now` |

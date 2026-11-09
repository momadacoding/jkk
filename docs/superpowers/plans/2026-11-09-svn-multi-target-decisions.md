# SVN 逐目录调用合并成多目标：实现日志

计划：[2026-11-09-svn-multi-target.md](2026-11-09-svn-multi-target.md)。分支 `worktree-perf-svn-multi-target`，基点 `2da929f`。

## 进度总账

| Task | 内容 | 状态 | 产出文件 | 验证 |
|---|---|---|---|---|
| 0 | 实测多目标语义 | 完成 | 临时 `tests/zz_probe_multi_target.rs`（已删） | 结论见下 |
| 1 | `run_svn_targets`、按 `<target>` 判 externals | 完成 | `svn_cli.rs`（`MAX_TARGETS_CMDLINE`、`cmdline_estimate`、`chunk_targets`、`run_svn_targets`）、`svn_xml.rs`（`parse_lists`、守卫按块判）；测试 `svn_cli.rs` +6、`svn_xml.rs` +2 | 守卫测试在旧代码上红、改后绿；`svn_cli` 18、`svn_xml` 47 全过 |
| 2 | `enumerate_wcs` 按层批量 | 完成 | `svn_wc.rs`；测试 `svn_wc.rs` +2 | 新测试在旧代码上也绿（回归保护）；`svn_wc` 9、`svn_sync` 18、`svn_preflight` 17 全过 |
| 3 | 本地读批量 | 完成 | `svn_nodes.rs`、`svn_dirty.rs`、`svn_preflight.rs`、`svn_health.rs`（`whitelist_not_under_base`、`excluded_nodes`） | `svn_nodes` 11、`svn_dirty` 30、`svn_preflight` 17、`svn_health` 43、`svn_health_dirty` 8、`svn_sync` 18、`svn_server_set` 12 全过；守卫变异见下 |
| 4 | 体检 floor diff 合并 | 完成 | `svn_health.rs`（`plan_diffs_for`、`DiffCommand::fallback`、执行队列）、`svn_tombstone.rs`（`Point::MergedDiff`）；测试 `svn_health.rs` +4 | 新测试编译失败（`plan_diffs_for` 不存在）→ 绿；`svn_health` 47 全过；变异「合并失败不退回」被 `a_failed_merged_diff_falls_back_to_one_diff_per_root` 抓到 |
| 5 | 服务器核对 `ls -R` 批量 | 完成 | `svn_server_set.rs`；测试 `svn_server_set.rs` +1 | 新测试在旧代码上也绿（回归保护）；`svn_server_set` 13、`svn_follow_audit` 3 全过；变异「清单倒序对回」被 2 条抓到 |
| 6 | 开发机实测 | 完成 | 临时 `tests/zz_perf_multi_target.rs`（已删） | 结果见下；两版结论逐项相同 |
| 7 | 验证、review、PR | 完成 | review 修复见下表 | review 前：`cargo test --no-fail-fast` 89 个测试二进制全过（1794 passed、0 failed、1 ignored）。review 修复后：`cargo fmt --check` 过；`cargo clippy --all-targets` 无告警；`cargo test --no-fail-fast -j 4 -- --test-threads=4` 88 个 `test result` 全 ok（1796 passed、0 failed、1 ignored） |

## 决策条目

### 计划阶段

#### 偏离报告的方案

- 报告方案 F 写的是 `info -R`、`ls -R`、`diff --summarize` 三项。读代码发现跟进里逐目录的本地读不止 `enumerate_wcs`：`read_node_status`、`read_dirty`、`check_single_node_rev`、`whitelist_not_under_base` 也是每个目录一个进程，跟进开头约 35 s 与发布前复核主要是它们，一并纳入。
- 报告里「逐目录 `svn info -R`（22.9 s）」对应的是 `svn_health::excluded_nodes`，不是 `svn_dirty.rs:193`（那条只在丢失的非 ASCII 文件上才跑）。

### Task 0：实测多目标语义

2026-11-09，SlikSvn 1.14.2，开发机，`file://` 仓库。fixture：`Code`（含 `notes.txt`、被 `--set-depth=exclude` 的 `Code/Deep`、同仓库 external `Code/Vendor`）、`Tools`、`Team@Home`、r3 才加的 `Late`；`Code/A.cs`、`Tools/T.cs`、`Code/Vendor/V.cs` 改脏，`Tools/New.cs` 未版本控制。

#### 计划没写的坑

| # | 命令 | 结论 | 对实现的约束 |
|---|---|---|---|
| M1 | `info --xml -- Code@ Tools@ Code/Vendor@ Team@Home@` | 每个目标一条 entry，**按传入顺序**；`path` 是去掉 `@` 后缀的目标，嵌套的用反斜杠（`Code\Vendor`） | 按顺序对回目标，条数对不上就报错 |
| M2 | `status --xml -- Code@ Tools@` | 每个目标一个 `<target path>` 块，内容与单独跑相同；不带 `--ignore-externals` 时照样下沉进 external | `parse_external_roots` 直接用 |
| M3 | `status -v --no-ignore --ignore-externals --xml -- Code@ Code/Vendor@ Tools@` | `Code` 块里 `Code\Vendor` 是 `item="external"` 标记，`Code\Vendor` 块里是它自己的节点 | **`assert_externals_were_ignored` 按整份输出判会误报**：`Code\Vendor\V.cs` 落在 `Code` 块标出的 external 根之内。必须改成按 `<target>` 块判 |
| M4 | `info -R --xml -- Code@ Tools@` | 两个目标的递归结果依次拼接，`Code\Deep` 的 `<depth>exclude</depth>` 照样在 | `parse_excluded_nodes` 直接用 |
| M5 | `diff --old=<root>@3 --new=. --summarize --xml -- Code Tools Late` 对比每单元 `--old=<root>/Code@3 --new=Code` | 结果是逐单元结果的并集，`Code` 目录的属性差异也在；**不下沉进 `Code/Vendor`**（与逐单元相同） | 合并命令只收 URL 就在 `svnUrl` 底下的单元，external 照旧单独比 |
| M6 | `diff --old=<root>@1 ... -- Code Late`（`Late` 在 r1 不存在） | exit 0，`Late` 下全部报 `added`；单独比 `--old=<root>/Late@1 --new=Late` 结果相同 | 旧 revision 上没有的单元不会让合并命令失败 |
| M7 | `diff ... -- Code Nope`（`Nope` 磁盘上没有） | exit 1、`E155010`，stdout 里有 `Code` 的部分结果 | 一个目标失败整条失败；按 `run_svn` 的规矩丢掉 stdout |
| M8 | `diff ... --ignore-properties -- Code Tools` | 正常，去掉了 `Code` 的属性差异 | 映射非单值时合并命令同样带它 |
| M9 | `ls -R --xml -- <Code>@3 <Tools>@3` | 一个 `<lists>` 下每个目标一个 `<list path=<url>>`，按传入顺序；单文件目标的 `<list path>` 就是文件 URL、`<name>` 是文件名 | `parse_list` 只取第一个 `<list>`，要加 `parse_lists`；单文件特判按每个 list 各做 |
| M10 | `ls -R ... -- <Code>@3 <Late>@1 <Tools>@3` | exit 1，`W160013` + `E200009`，三个 `<list>` 都打出来（`Late` 那个是空的） | 失败照旧整体 NotChecked |
| M11 | `status --xml -- Code@ Nope@ Tools@` | **exit 0**，`Nope` 是一个空的 `<target>` 块 | 与单独跑 `status -- Nope@` 一致（同样 exit 0、空结果），不改变现有行为 |
| M12 | `info --xml -- Code@ Nope@ Tools@` | exit 1，`W155010` + `E200009`，另外两条 entry 照样打出 | 一个目标失败整条失败 |
| M13 | `status --depth=empty --ignore-externals --xml -- <abs>@ <abs>/Code/Vendor@` | 每个目标一个空 `<target>` 块（干净时） | 可以批量，但报错要点名目录，见计划「不在范围」 |

### Task 1：`run_svn_targets` 与按 `<target>` 判 externals

#### 偏离计划

- 计划写的 `parse_lists` 测试放在 Task 1，实际与 `parse_list` 共用 `list_of`，`SvnListResult` 补了 `Debug / Clone / PartialEq`，方便断言两者在单个 `<list>` 上相同。

#### 为什么这么改是安全的

- 守卫改成按块判之后，单 target 的输出只有一个块，判据与原来逐字相同；没有 `<target>` 块的手写输入退回按整份判。漏了 `--ignore-externals` 的那一块自己就含 external 内部的节点（M3 的反方向，测试 `the_guard_judges_each_target_block_on_its_own` 用真实 svn 输出验证），所以拦截能力没变。
- `run_svn_targets` 空列表不起进程（测试用不存在的工作目录证明：真起了进程就会报 io 错误）。

### Task 2：`enumerate_wcs` 按层批量

#### 为什么这么改是安全的

- 原来是一个栈，逐个弹出、逐个展开；现在是按层展开，每层先去重。结果按 `wc_root` 排序后返回，与展开顺序无关。
- `info` 结果按顺序对回、条数不等即报错。实测 M1/M12：一个 target 不存在时 svn 整条 exit 1，不会少一条还 exit 0。
- 回归保护 `many_whitelist_entries_in_one_call_resolve_each_to_its_own_identity`：单文件、带 `@` 的目录、重复条目、跨仓库嵌套 external 同在一批，逐条核对 URL 与 UUID。

### Task 3：本地读批量

#### 计划没写的坑

- 变异：把守卫临时改回按整份判，真实调用路径上 4 个套件共 8 条测试变红（`svn_health` 的嵌套 external 体检、`svn_init` 的同仓库嵌套 external 初始化、`svn_preflight` P1/P2 的 3 条、`svn_sync` 的 3 条）。说明「外层 + external 根同批」在跟进、初始化、预检里都会出现，Task 1 的守卫改动是必需的，不是只为单元测试。

#### 为什么这么改是安全的

- 五处都是「逐个 target 跑同一条命令、结果拼起来」，换成一次传后结果就是同一份拼接（M2/M3/M4）。`read_node_status` 的去重规则（保留带 revision 的、都带时保留先见到的）依赖 target 顺序，批量保持原顺序。
- 失败语义：逐个跑时第一个失败就 `?`；批量时一个失败整批报错，错误同样冒出去。`whitelist_not_under_base` 失败时仍落到 NotChecked。
- 跟进里的 `with_scan_cache` 键是完整参数表，同一组 target 批量后的命令每次相同，缓存照样命中；`forget_node_scans` 按 `-v` 认键，不受影响。

### Task 4：体检 floor diff 合并

#### 偏离计划

- 计划写「实现 `group_floor`」。实际是公开的 `plan_diffs_for`（= `plan_diffs` + 合并），`plan_diffs` 原样保留：已有 6 条测试直接断言它的输出，留着它就能继续钉住逐单元命令的形状，`plan_diffs_for` 的测试再断言合并命令的 `fallback` 恰好等于 `plan_diffs` 的那几条。
- 可合并的单元不到两个时不合并，命令与改动前逐字相同（单白名单根的仓库体检一行都不变）。
- 为了测退回路径加了注入点 `Point::MergedDiff`：只有合并命令会撞上它。没有它的话，`FirstDiff` 注入时合并命令与第一条退回命令都失败，结论同样是 NotChecked，分不出退回到底跑没跑。

#### 计划没写的坑

- 合并命令的 `wc_root` 是空串，`repo_rel_of` 原来的「输出路径等于 `unit_url` 就是单文件条目」那条分支对它不成立（`unit_url` 是 svnUrl，一个目录）。加了「`wc_root` 非空」的前提，剥前缀之后不再拼 `wc_root/`。单文件条目在合并命令里的输出路径是 `<svnUrl>/Code/A.cs`，走通则正确还原（测试 `a_merged_floor_diff_reports_the_same_files_as_one_diff_per_root`）。
- 测试 fixture 里 `svn commit -m tools` 被 svn 拒绝（`E205005: The log message is a pathname`）：Windows 上大小写不敏感，`tools` 撞上了刚建的 `Tools` 目录。

#### 为什么这么改是安全的

- 结论：M5/M6 实测合并命令的结果是逐单元结果的并集，不下沉进 external；只合并 `url == <svnUrl>/<wc_root>` 的单元，`whitelist_not_under_base` 已在前面保证白名单根都满足这条，嵌套 external 一定不满足（它的 URL 在别处），照旧单独比。
- 失败语义：合并命令失败不下任何结论，把它替代的逐单元命令按原顺序插回队头再跑。「第一条」的判据从「下标为 0」改成「一条都还没跑成」——改动前失败即返回，两者对逐单元序列完全等价；退回之后逐单元那几条的 NotChecked / Unconfirmed 与改动前逐条相同。代价是断网时多一条失败的命令。
- 尾随 PATH 不过 `peg_safe`（`svn diff` 的尾随 target 不做 peg 解析），所以含 `@` 的单元不合并；非 ASCII 的单元也不合并（系统代码页）。按 `MAX_DIFF_CMDLINE` 分批，与 revision 组同一口径。

### Task 5：服务器核对 `ls -R` 批量

#### 为什么这么改是安全的

- 任何一个单元列不成，svn 整条 exit 1（M10），落成 NotChecked，与原来「一个单位没列成就整体 NotChecked」相同。失败理由原来点名单元，现在列出全部单元，svn 原话里有具体 URL。
- `<list>` 按顺序对回单元、份数不等即 NotChecked；单文件单元的特判按每份清单各做，逻辑不变。回归保护 `a_depth_excluded_subtree_in_a_second_root_is_reported_under_that_root` 断言漏收点名到第二个根下的那个文件。

### Task 6：开发机实测

2026-11-09，开发机，debug 构建，`file://` 仓库。fixture：23 个白名单目录 `D00`–`D22`，每个 20 个 `.cs`（分 3 个子目录），初始化成混合仓库。每个版本连跑 3 轮读路径；跟进 3 次，每次同事在 `D03`、`D11`、`D19` 各改一个文件提交，本地 `svn update` 后调 `follow`。

改动前的版本：用 `git archive master` 把 `src-tauri` 导出到 scratch 目录单独编译（不在 worktree 里换文件），同一份测试文件两边都编。两个版本交替各跑两遍，下表取第二遍（第一遍改动前那边波动很大，同一项 4–17 s，疑似杀软扫描刚落盘的可执行文件）。

| 调用 | 改动前（3 轮） | 改动后（3 轮） |
|---|---|---|
| `enumerate_wcs` | 1978 / 1705 / 1959 ms | 76 / 79 / 77 ms |
| `check_sync`（第二档） | 2741 / 4664 / 3062 ms | 140 / 143 / 144 ms |
| `check_baseline`（体检） | 4099 / 3067 / 3605 ms | 204 / 386 / 212 ms |
| `audit_server_set`（服务器核对） | 3916 / 6105 / 3501 ms | 447 / 224 / 237 ms |
| `follow`（跟进） | 11399 / 11790 / 10747 ms | 3269 / 2556 / 2570 ms |

结论（体检 Verified、核对 Matched、判定 InSync、跟进 Done）两版逐项相同。

#### 计划没写的坑

- 导出的 `src-tauri` 单独编译时，`tauri.conf.json` 的 `bundle.resources` 引用 `../THIRD_PARTY_NOTICES.md` 与 `../LICENSE`，缺了 build script 就失败，要一并放到导出目录的上一级。
- 开发机上每起一个 svn 进程也要几十毫秒，所以 `file://` 仓库上就有 4–25 倍的差距。公司机器上单进程更贵、服务器调用还要连网与认证，实际收益要按计划 P5 的方法在那里录；这里的数字只说明进程数确实降下来了。

#### 为什么这么改是安全的

- 跟进的 3 次都走完整条链路（判定、预检、录基线、体检、重放、发布前复核、发布），结论与改动前相同；全量 `cargo test` 也覆盖了跟进、初始化、重建、交付的既有用例。

### 独立 review（`/code-review high`，Task 1–6 完成后，提交 `f8cd48d`）

| # | 发现 | 结论 | 处理 |
|---|---|---|---|
| 1 | 合并命令的 `--old` 用配置的 svnUrl，而判定可否合并、`url_mismatch`、`whitelist_not_under_base` 都不分大小写；大小写敏感的服务器上 `<svnUrl>@F` 不存在，按 M6 整片报 `added`、判成 Lying，命令成功、不退回 | 成立 | **已修**：`--old` 与 `unit_url` 用单元**实际** URL 去掉 `/<wc_root>` 的前缀；只合并实际 URL 逐字节等于「同一前缀 + `/` + `wc_root`」的单元（白名单写法与服务器目录只差大小写的也不合并）。测试 `the_merged_floor_diff_uses_the_real_url_not_the_configured_spelling` |
| 2 | 合并命令任何失败都退回逐单元，断网 / 认证失败 / 超时时多等一轮 | 成立 | **已修**：`server_unreachable` 按错误码认「整台服务器够不着」（连不上、认证、`ra_local` 打不开、DNS、WSA 超时 / 拒绝 / 重置），这类失败不退回，按第一条失败落 NotChecked。合并的单元同一前缀、同一台服务器，退回也只会在第一条上再失败一次，结论相同。认错的代价只是多跑一条。测试 `server_unreachable_is_judged_by_error_code_only`、`an_unreachable_server_fails_the_merged_diff_without_falling_back`（删掉 `file://` 仓库，理由点名合并命令的全部根，证明没退回） |
| 3 | `plan_diffs_for` 靠「`plan_diffs` 前 `units.len()` 条就是逐单元 floor 命令」这条没核对的约定 | 成立 | **已修**：合并前逐条核对 `wc_root`、`scope == Floor`、无尾随 `--`、无 `fallback`，对不上就原样返回、不合并 |
| 4 | external 守卫按 `<target>` 块判，依赖「不带 `--ignore-externals` 时 external 的内容落在外层块里」，只在 SlikSvn 1.14.2 上实测过 | 成立，属遗留风险 | 不改：块的布局由 libsvn 决定，不是客户端各自实现；真实 svn 输出的双向测试 `the_guard_judges_each_target_block_on_its_own`、`the_guard_holds_against_real_svn_output_in_both_directions` 在换 svn 版本后会第一时间变红 |
| 5 | 服务器核对失败理由列出全部单元，看不出是哪个 | 成立 | **已修**：`failing_units` 按 svn 报错里的 `'<仓库内路径>'` 认出失败的单元，认不出时才列全部。测试在 `a_depth_excluded_subtree_in_a_second_root_is_reported_under_that_root` 里补了 r1 上 Tools 不存在的断言 |
| 6 | 命令行长度估算与上限三处重复 | 成立 | **已修**：`MAX_DIFF_CMDLINE = MAX_TARGETS_CMDLINE`，`cmdline_len` 改调 `cmdline_estimate`，合并命令分批改用 `chunk_targets`。`push_batches` 的循环带着路径集合一起切，没并进来 |
| 7 | 合并命令靠空 `wc_root` 当哨兵，别处要特判 | 部分成立 | **已修**报错文案（合并命令失败时点名它的全部根，不会打出空名字）；不改成枚举：`DiffCommand` 是公开结构、测试直接断言它的字段，改形状牵动面大于收益 |

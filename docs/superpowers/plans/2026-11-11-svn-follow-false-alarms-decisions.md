# 2026-11-11 SVN 跟进误报三项：实现日志

输入是 2026-11-11 的一份使用报告（真实仓库，0.8.0），它定位了三个现象并给了方案 A / B / C。报告本身不进仓库（含同事姓名与业务路径，§11），这里只记实现需要的结论。没有单独的计划文件，本日志按 AGENTS.md §2.1 兜底。

| 现象 | 报告的方案 | 本次 Task |
|---|---|---|
| 「服务器上有代码文件不在本地」：同事在 floor 之后删的文件，update 删了、还没跟进时被报成漏收 | A：跟进之前不把已消失的文件算漏收 | Task 1 |
| 自动跟进被条件 11 拦下：本地专属提交与 SVN 改了同一批文件，但重放结果与磁盘逐字节相同 | B：重放结果与磁盘一致就放行 | Task 2 |
| 跟进报告里红色「丢失的暂存」：暂存的改动 SVN 新版本里都已经有了 | C：按 hunk 判断是否已被 SVN 取代 | Task 3 |

## 进度总账

| Task | 状态 | 产出文件 | 验证 |
|---|---|---|---|
| 1 方案 A：update 删掉、未跟进的文件不报漏收 | 完成 | `src-tauri/src/engine/svn_server_set.rs`、`svn_health.rs`（`excluded_nodes` 改 `pub(crate)`）、`tests/svn_server_set.rs` | 新增 4 条集成测试先红后绿（3 红 1 绿，绿的那条钉「同目录 exclude 照报」）；1 条单元测试；`cargo fmt --check`、`clippy --all-targets` 通过；全量 `cargo test` 见下 |
| 2 方案 B：重叠文件与磁盘逐字节相同就放行条件 11 | 完成 | `src-tauri/src/engine/svn_follow.rs`（`OverlapFile`、`overlap_files`、`retained_by_path`、`auto_blocker`）、`src/engine/svnFollow.ts`（两节报告）、`tests/svn_follow_auto.rs`、`tests/svn_backend.rs`、`tests/vitest/svnFollow.test.ts` | 条件 11 原有 1 条改写 + 新增 3 条集成测试；契约测试补 `overlap`；vitest 2 条 |
| 3 方案 C：暂存改动全部已在 NEW 里时不报丢失 | 完成 | `src-tauri/src/engine/svn_leak.rs`（`stage_hunks_in_new`）、`svn_follow_index.rs`（`superseded`、`partial`、`stage_tally`）、`src/engine/svnFollow.ts`（三档）、`tests/svn_follow_auto.rs`、`tests/svn_leak.rs`、`tests/svn_backend.rs`、`tests/vitest/svnFollow.test.ts` | 条件 12 新增 2 条集成测试、原有 1 条补断言；判据 5 条单元测试；契约测试补两字段；vitest 2 条 |

全部 Task 与 review 修复完成后：
- `pnpm build` 通过；`pnpm test`（89 个文件、1544 条）通过。
- `cargo fmt --check`、`cargo clippy --all-targets` 没有告警。
- 全量 `cargo test --no-fail-fast`：除 1 条 lib 单元测试外全部通过。这次是分两段跑的：第一段被 Claude Code 以内存紧张回收，`svn_sync_in` 及之后的 16 个测试文件用 `-j 2 --test-threads=4` 补跑。
- 失败的那条是 `commands::watch::tests::stage_and_unstage_only_touch_the_index`，watcher 在 `git add` 时收到 `.git/hooks` 的事件。在未改动的基线 6883b03 上导出构建也一样失败，是本机环境下已有的问题，与本次改动无关。
- 没有改弹框组件与样式，没跑 `pnpm test:layout`。

## Task 1：方案 A

### 偏离计划（报告的方案）

- **剔除的不是判定的 `removed` 全集，而是 `removed − 被排除节点之下`。** 报告写的是「检查额外剔除判定里的 removed 路径」，并声称「同一时段另有目录被 exclude：照样报 Missing」。按代码这条验收会失败：`removed` 是「基线里有、`svn status -v` 没列出」的文件，被 `--set-depth=exclude` 掉的文件同样不在 status 里（`tests/svn_server_set.rs` 的 `local_checks_cannot_tell_a_depth_excluded_subtree_from_a_deletion` 早就钉了这一点）。照报告的做法，这条检查要防的那类漏收只要和一次 update 落在同一个跟进窗口里，就会被一起藏掉。改为再用 `svn info -R`（每个单元一次，纯本地）找出被裁掉的子树，减掉它们下面的（review 之后的最终形态见文末 M2）。
- **候选来自后端自己跑的 `check_sync`，不是前端传入。** 与报告的倾向一致，不改命令签名。报告说的「多读一次 wc.db」不准确：判定走的是白名单内的 `svn status -v --xml`（`svn_nodes::read_node_status`），2 万文件的仓库要几秒；这条检查本来就有一次 `svn ls -R` 走网络，相比之下可以接受，每 5 分钟一次。
- 只改 `audit_server_set_at_baseline`（打开仓库 / 轮询那条路径）。`audit_server_set_map`、`audit_server_set` 行为不变，现有测试照旧覆盖它们。

### 计划的错误

- 报告的验收用例「同一时段另有目录被 exclude：照样报 Missing」与它给的做法矛盾（见上）。测试按验收用例写，做法按验收用例改。

### 计划没写的坑

- 被裁掉的子树读不出来（`svn info -R` 失败、XML 解析失败）时一个都不剔，回到改动前的行为：宁可误报，不藏漏收。
- 判定读不出来（`check_sync` 出错）或判定不是 ExternallyAdvanced 时同样不剔。
- 排除节点与候选按**不区分大小写**的路径前缀比（`Code/Deep` 不覆盖 `Code/DeepX.cs`）。比宽的后果只是多报，比窄会藏掉漏收，所以取宽。
- 同事删了 `Code/Deep/D.cs`、我 update 之后又把 `Code/Deep` 排除：本地分不清，照报（`a_removed_file_under_an_excluded_node_is_still_reported`）。跟进录制时向服务器确认后自然消失。

### 为什么这么改是安全的

- 这条检查只加提示、不进任何门槛（模块头注释）。被剔除的路径没有被放过，只是推迟到跟进录制时向服务器逐个确认：不存在记墓碑，仍存在判 `Excluded`，确认不了判 `Unconfirmed` 并拒绝录制（`svn_health.rs`），自动跟进的条件 5 也挡着。
- 唯一能让一个真正漏收的文件被剔掉的情形，是它在本地「消失」而又不在任何被裁掉的子树下。本地裁掉文件只有两种做法：`--set-depth=exclude`（留 exclude 节点），以及把目录 depth 降成 `empty / files / immediates`（子目录整个消失，父目录的 depth 变了；独立 review M2，fixture 实测）。两种 `svn info -R` 都看得见，`parse_trimmed_nodes` 都收。
- （初版只减 exclude 节点，独立 review M2 指出降 depth 的口子，已修，见文末。）
- 测试：`a_file_removed_by_update_before_follow_is_not_reported_as_missing`（修复的现象）、`an_exclude_in_the_same_window_is_still_reported`（报告方案的漏洞）、`a_removed_file_under_an_excluded_node_is_still_reported`、`a_file_removed_by_update_stays_matched_after_follow`（跟进前后都是 Matched，墓碑接手）。

## Task 2：方案 B

### 偏离计划（报告的方案）

- **tip 与磁盘都没有这个文件算一致。** 报告写「文件不存在……照旧拦下」。两边都没有时「本地改动」里不会出现这个文件，不可能有反向改动，拦下只会让「commit 删了文件、SVN 也删了」这类情形无谓地失效。一边有一边没有、符号链接、子模块、目录照旧算不一致。
- **只差可执行位不算不一致**，与 `drift_paths` 的 review 第 5 条同一口径（`hash-object` 只比内容）。
- **重叠的核对挪到 `prepare` 里做、结果写进 `FollowReport.overlap`**，`auto_blocker` 只读结果。报告第 4 点要求跟进报告逐个标「与磁盘一致 / 不一致」，手动点「跟进」也要有这份清单，所以两种入口共用一次计算。核对出错时报告里为空，`Plan.overlap_err` 记着，自动跟进照旧给 `svn_auto_follow_overlap_unknown`。
- `retained_overlap` 换成 `retained_by_path`：同一条 `git log -z` 多带 `%x01%h%x02%s`，顺手记下每个文件是哪些提交改的，横幅写出提交（短 hash + 说明）。
- 前端把重叠文件分两节：不一致的是 attention（「本地改动」里看到的就是差别，是自己没提交的改动就不用管），一致的是 info。已经在 `drift`（danger）里的不重复列。
- 横幅去掉「跟进之后请对照报告检查这些文件」，改成「报告会逐个标出这些文件是否与磁盘一致」：报告现在真的有这份清单了。

### 计划的错误

- 无。报告的设计要点与验收用例都照做了（「多个保留的 commit 只有一个碰到重叠文件」并进了第一条用例）。

### 计划没写的坑

- 原有的条件 11 用例（`condition_11_a_branch_commit_on_a_file_svn_also_changed_is_not_followed`）正是方案 B 要放行的形状：update 合出来的磁盘与重放结果相同。改写成「放行」用例，「拦下」改用两种真正不一致的形状：重叠文件里还有没提交的改动、W14（用拼接两边的 merge driver 造合错，同 `svn_follow_snapshot.rs`）。
- `git log -z --format=%x01%h%x02%s --name-only`：每个 commit 是 `\x01<h>\x02<s>` NUL，路径以 `\n` 开头、NUL 结尾（2026-11-11 实测 git 2.51.2）。
- 没有先跑红：新字段 `overlap` 不存在时测试编不过，「红」就是编译失败。放行的两条在旧代码下会是 `svn_auto_follow_overlap`，拦下的两条在旧代码下本来就拦。

### 为什么这么改是安全的

- 「本地改动」= 磁盘 − tip。重叠文件的 tip blob 与磁盘内容（按 `git add` 的过滤规则算）逐字节相同时，这些文件在「本地改动」里没有差异，W14 那种反向改动不可能出现。两次合并（svn update 与 git 重放）错得一模一样时，jkk 也只是和磁盘一致，没有额外引入错误。
- `drift` 一定是 tip ≠ 磁盘（磁盘 = NEW ≠ tip），照样拦。
- 时间差：核对在拿同步锁之前做。之后用户改了磁盘，只是用户自己的改动；之后又跑了 `svn update`，`still_valid` 按 svn 节点重算映射，有变化就整轮重来。
- 核对出错（读对象失败、`hash-object` 行数对不上）一律不自动跟进。

## Task 3：方案 C

### 偏离计划（报告的方案）

- **不复用 `svn_leak` 的判定，另写一个更严的 `stage_hunks_in_new`。** 报告说「判据可能直接复用」。泄漏判定宁可多报：映射位置对不上时，把 `+` 行逐条到 NEW 的改动里去找，`end`、`}` 这种行到处都有。这里的「在」会把红色降成灰色，误判的方向正好相反。新判据要同时满足三条：NEW 动过这一片；`+` 行按顺序、连续地出现在重叠区域对应的那一段 NEW 里；暂存删掉的行，NEW 那一段里也删了。判不了的（二进制、删除、mode 变化、读对象出错）照旧算丢失。
- **数据形状**：`StageDecision` 不变（仍是 `LostToNew`，暂存区取 NEW）。`ComposeReport` 加两个 `#[serde(default)]` 字段：`superseded`（全部已在，不进 `lost`）和 `partial`（`lost` 里部分已在的计数）。条件 12 只看 `lost`，所以「全部已在」放行自动跟进、「部分已在」照旧拦。这与报告的倾向一致。
- 前端三档：`lost` 里没有计数的是 danger，有计数的是 attention（「N 处改动里 M 处已在 SVN 上」），`superseded` 是 info，三节都写钉住的 ref。

### 计划的错误

- 报告估计真实现场会落在「部分已在」，按新判据也是这样：同事删了 `<br>` 的那一处，连续块对不上。

### 计划没写的坑

- 判据的取舍：只在重叠区域里找连续块，不要求块前后的行也对得上。消息表那种「暂存加了几条，SVN 在同一处加了这几条，前后还多加了别的」要能算「在」（`stage_change_already_in_new_counts_even_when_new_changed_more_around_it`）。代价是：暂存某处只加了一行通用的行（比如单独一个 `}`），而 SVN 恰好在同一片区域也加了这一行，会被算成「在」。这种情形下原内容照旧钉在 follow ref 下，报告的 info 节写明了怎么取回。
- `spans` 给纯删除的 `new_start` 可能是 0，不能拿它取 `+` 行。

### 为什么这么改是安全的

- 暂存区的处理一个字节都没变：`LostToNew` 照旧取 NEW，原索引照旧钉在 `refs/mygitclient/follow/<opid>`。变的只是报告的级别，以及条件 12 对「全部已在」的放行。
- 只有三条全都满足才降级，任何拿不准的都留在 `lost`（danger）。单元测试钉了三种「泄漏判定会算 Present、这里不能算」的形状：行分散、只在文件别处、删除没在 NEW 里。

## 独立 review（2026-11-12，独立的 Opus subagent，只读）

没有 blocker。下面四条都修了，每条有对应的测试。

| 级别 | 发现 | 处理 |
|---|---|---|
| Medium 1 | C：两处暂存改动可以在同一段 NEW 里各认领同一行（暂存两处各加 `foo`，NEW 只加一处），被判成 `superseded` | 判据加第 4 条：与 h 重叠的 NEW hunk 若还与暂存的别的 hunk 重叠，这几处一律不算在。单元测试 `two_stage_hunks_cannot_share_one_new_change` |
| Medium 2 | A：`--set-depth=empty / files / immediates` 裁掉的子目录不留 exclude 节点，被裁掉的文件会被当成 update 删掉的剔除，原来的安全论证「被排除的节点 `svn info -R` 一定看得见」不成立 | fixture 实测（SlikSvn）：`--set-depth=files Code` 之后 `Code/Deep` 从 wc.db 里消失，`Code` 的 depth 是 `files`，没有 exclude 节点。改用 `svn_xml::parse_trimmed_nodes`：exclude 节点，加上 depth 不是 infinity 的目录，整棵子树都减掉（比宽只是多报）。`svn_health::excluded_nodes` 恢复成私有。测试 `a_directory_reduced_to_depth_{files,immediates}_is_still_reported` |
| Low 1 | 前端 `p in partial` 会命中原型链：叫 `constructor` 的丢失文件会被降一级 | 改成 `Object.prototype.hasOwnProperty.call`（lib 是 ES2020，没有 `Object.hasOwn`）。vitest 补了一条 |
| Low 2 | `stage_hunks_in_new` 缺边界用例 | 补了这几种：空 OLD（新增文件）、文件开头 / 结尾的纯删除、末行没有换行。B 的 `(None, None)` 放行分支没补用例：保留下来的 commit 相对新父提交不可能再「改动」一个两边都删了的路径，`drift` 又要求 tip ≠ NEW，这条分支只是防御性的 |

修 Low 2 时写错了一条期望：NEW 是 `1x\n2\n` 时第 3 行也删了，(1, 1) 才对。已改成 NEW 保留第 3 行，再验一次不在。

Task 1「为什么这么改是安全的」第二条据此更正：被裁掉的成因不止 exclude，还有降 depth，两种都要减掉。

# SVN C1h TortoiseSVN 钩子：实现日志

- 计划：`2026-11-01-svn-c1h-tsvn-hooks.md`（同目录）
- 设计：`docs/superpowers/specs/2026-10-30-svn-mixed-revision-baseline-analysis.md` §6.13（下称「分析」）
- 分支：`worktree-svn-c1h-tsvn-hooks`（基点 `ef762cb`）
- 约定：需要用户拍板的地方，先按保守做法绕过去，条目标【待决】，收尾时按用户的决定回头修

## 进度总账

| Task | 内容 | 状态 | 产出文件 | 验证 |
|---|---|---|---|---|
| 0 | 用真实 TortoiseSVN 实测分析 §11 的钩子未知项 | 完成 | 本文件 Task 0 | H1–H5，四轮手工操作，2026-11-01 |
| 1 | 注册表条目的解析、注册计划与健康判定 | 完成（未 commit） | 新增 `engine/tsvn_hooks.rs`、`tests/tsvn_hooks.rs`（18 条）；改 `engine/mod.rs`、`Cargo.toml`（`windows` 加 `Win32_System_Registry`） | 先红（模块不存在）后绿 18/18；补测的 3 条做了 1 次变异，抓到；`cargo fmt --check` 通过，`cargo clippy --all-targets` 0 告警。注册表读写未跑自动化测试，见 Task 1 小节。Review 1 修复之后 21/21 |
| 2 | 钩子子命令与收件箱写入 | 完成（未 commit） | 新增 `engine/tsvn_hook_cli.rs`、`engine/tsvn_inbox.rs`（事件与写入）、`tests/tsvn_hook_cli.rs`（13 条）；改 `main.rs`、`engine/mod.rs`、`Cargo.toml`（`Win32_Globalization`） | 先红（模块不存在）后绿 13/13；fmt、clippy 干净。release exe 实测：`app.exe --tsvn-hook post_commit …` 退出 0、总耗时 101ms（含 `Start-Process`），收件箱里正好 1 条事件，没有启动 GUI。Review 1 补了进程级测试，之后 14/14 |
| 3 | 收件箱读取、配对与清理 | 完成（未 commit） | 改 `engine/tsvn_inbox.rs`（`read_evidence`、`prune_inbox`、证据结构与常量）；新增 `tests/tsvn_inbox.rs`（14 条） | 先红（缺符号）后绿 14/14；`tsvn_hook_cli` 13/13 仍绿 |
| 4 | Check-commit 按磁盘内容拦截 Local-Only 泄漏 | 完成（未 commit） | 改 `engine/svn_leak.rs`（抽出 `Side` / `judge_sides`，新增 `judge_bytes`、`local_only_leaks_on_disk`、`leak_message`）、`engine/tsvn_hook_cli.rs`（`check_commit`）；测试 `tests/svn_leak.rs` +8（`judge` 辅助函数改为对每个用例交叉核对 `judge_bytes`）、`tests/tsvn_hook_cli.rs` +3 | 先红（缺符号；命中用例在旧行为下失败）后绿：`svn_leak` 27/27、`tsvn_hook_cli` 17/17；1 次变异（忽略 Local-Only 标记）被 `check_commit_without_local_only_commits_exits_zero` 抓到；`svn_follow` 133/133 仍绿；fmt 通过，clippy 0 告警 |
| 5 | 跟进按钩子事件判定 | 完成（未 commit） | 改 `engine/svn_follow.rs`（`Who.events`、`facts_from_events`、`committed_after_mark`、`staged_in_mark`、F2 ① 的 `mine`、`Evidence`、`FollowReport.evidence` / `evidence_cutoff_ms`、`finish` 里清理收件箱、`fault::pretend_hook_health`）、`engine/svn_follow_mark.rs`（`at_ms`）、`engine/tsvn_hooks.rs`（`quick_health`）；新增 `tests/svn_follow_hooks.rs`（11 条）；改 `tests/svn_follow_mark.rs`（+2 断言）、`tests/svn_backend.rs`（契约 + 旧 journal 兼容） | 先红（编译失败；加上字段桩之后 8 条行为测试失败）后绿：`svn_follow_hooks` 11/11、`svn_follow_mark` 45/45、`svn_backend` 35/35；fmt 通过，clippy 0 告警（`is_none_or` 不满足 MSRV，已换）。全量 `cargo test --no-fail-fast`：79 个测试二进制，1566 过 / 0 失败 / 1 ignored（`merge_drivers` 原有）；这次全量是在 Task 6 的 Rust 改动之前启动的 |
| 6 | Tauri 命令与前端入口 | 完成（未 commit；真实窗口里的 invoke 参数名已核对，读写注册表的路径留给 Task 7） | 改 `commands/svn.rs`（`TsvnHooksStatus`、`hooks_target`、`tsvn_hooks_status` / `_register` / `_unregister`）、`lib.rs`（注册命令）、`src/store/repoStore.ts`、`src/engine/svnFollow.ts`（`evidence`、`basis`）、`src/components/BaselineBanner.tsx`、`FollowReportDialog.tsx`、`RepoTab.tsx`；新增 `src/engine/tsvnHooks.ts`、`src/components/TsvnHooksDialog.tsx`；测试 `tests/tsvn_hooks.rs` +3、`tests/vitest/tsvnHooks.test.ts`（4 条）、`TsvnHooksDialog.test.tsx`（9 条）、`BaselineBanner.test.tsx` +3、`svnFollow.test.ts` +1 | 前端先红（缺模块，3 条断言失败）后绿，vitest 63 个文件 1018/1018；`pnpm build` 通过；Rust `tsvn_hooks` 24/24，fmt 通过，clippy 0 告警。命令层测试写在实现之后，1 次变异没被抓到，见 Task 6 小节 |
| 7 | E2E 清单与文档回写 | 完成（未 commit） | 新增 `docs/superpowers/e2e/2026-11-02-svn-c1h-tsvn-hooks-e2e.md`；改分析文档（第八轮）、C1a E2E 清单（场景 13 标已并入）、本计划；代码改 `engine/tsvn_hook_cli.rs`（`encode_thread_acp`）、`Cargo.toml` 注释，`tests/tsvn_hook_cli.rs` +1 | 真实 TortoiseSVN E2E 场景 1–8 通过（7c 未跑）。`stderr_text_follows_the_thread_code_page_not_the_system_one` 先红（`[63, 63]`）后绿。全量（dev 已停）：`cargo fmt --check` 通过，`cargo clippy --all-targets` 0 告警，`cargo test --no-fail-fast` 79 个二进制 1568 过 / 0 失败 / 1 ignored；`pnpm build` 通过，`pnpm test` 63 个文件 1022/1022。Final Review 修复之后再跑一遍：fmt 通过，clippy 0 告警，`cargo test` 1576 过 / 0 失败 / 1 ignored，`pnpm build` 通过，`pnpm test` 1023/1023。**E2E 在这些修复之前跑的，修复没有在真实窗口里重跑**：`cas_write` 的抽取只有内存闭包测试，注册框的提示只有组件测试 |

## 用户决定

| 编号 | 问题 | 决定 |
|---|---|---|
| D1 | 钩子本体的形态（分析 §6.13.2 留给 spec） | **已定（2026-11-01）：** jkk 的 exe 自带 `--tsvn-hook` 子命令，不另外分发小 exe |
| D2 | C1a E2E 的场景 13（真实 TortoiseSVN 重走场景 2）放在哪里做 | **已定（2026-11-01）：** 并入 C1h 的 E2E（计划 Task 7 第 2 条） |

## Task 0：实测

环境：TortoiseSVN 1.14.9.29743，SlikSvn 1.14.2，git 2.51.2，Windows 11。系统 locale en-US（`GetACP` = 1252），用户 locale zh-SG（线程代码页 936）。

> **更正（Task 7，2026-11-02）：** 这里原写「系统代码页 936」，是 Task 0 没有实测的推断，错了。H4 的结论因此也写错了，已在 H4 行内更正，见 Task 7 小节。

fixture 是 scratchpad 里的一次性 `file://` 仓库，含两个工作副本：`wc` 扮演「我」，`wc2` 扮演「同事」。钩子只挂在 `wc` 上。

探针有两种：
- pwsh 脚本：把参数、临时文件的原始字节、`PATH` 里各文件此刻的磁盘内容都落盘；
- GUI 子系统的 Rust exe：形态与 Tauri release 构建相同。

用户在 TortoiseSVN 界面里操作，Claude 读日志。实测用完的钩子由 `restore-hooks.ps1` 删除。

### 计划没写的坑（实测发现）

| 编号 | 实测 | 结论 |
|---|---|---|
| H1 | 钩子存在 `HKCU\Software\TortoiseSVN` 的 `hooks` 值里，类型 REG_SZ。每条 6 行，用 `\n` 分隔，行尾都有 `\n`：`pre_commit_hook` / 工作副本路径 / 命令行 / `true` / `hide` / `ask`。第 6 字段 `ask` 对应编辑框里没勾选的「Always execute the script」。<br>用程序在末尾追加的 6 条（`start_commit_hook`、`check_commit_hook`、`post_commit_hook`、`start_update_hook`、`pre_update_hook`、`post_update_hook`）不用重启 TortoiseSVN 就生效，设置界面里显示正常 | 程序注册可行（分析 §6.13.5 的「待实测」）。<br>在 Claude Code 自动模式下，agent 写这个值会被权限分类器拦下，所以实测时改由用户自己运行脚本。产品里由用户在 jkk 里点确认后写入 |
| H2 | 在 `wc` 根目录上右键提交，只勾选两个文件：<br>`start_commit` 收到 `PATH`（内容是右键选中的根目录）、`MESSAGEFILE`（空）、`CWD`；<br>`check_commit` 收到 `PATH`（勾选的文件）、`MESSAGEFILE`、`CWD`；<br>`pre_commit` 收到 `PATH`、`DEPTH`（`3`）、`MESSAGEFILE`、`CWD`；<br>`post_commit` 收到 `PATH`、`DEPTH`、`MESSAGEFILE`、`REVISION`、`ERROR`（文件路径，成功时 0 字节）、`CWD`。<br>`PATH` 文件是 UTF-8 无 BOM，正斜杠绝对路径，盘符大写，每行以 CRLF 结尾，中文与空格原样保留。<br>没有 TSVN 或 SVN 的环境变量 | **Check / Pre / Post-commit 拿到的是勾选的文件**，按勾选文件拦截可行（分析 §11 第一问）。`ERROR` 是文件，不是字符串 |
| H3 | 只勾选 `Code/B.cs` 时，`CWD` 参数变成 `wc\Code`，也就是勾选文件的公共父目录；挂在 `wc` 上的钩子照样被触发。<br>Check-commit 返回 1：提交框上弹出「Hook script failed」，选项是 **Abort**（回到提交框，消息保留）和 **Proceed anyway**。<br>Pre-commit 返回 1：提交框关闭，进度框列出错误，带 **Retry without hooks** 按钮，点 OK 回到提交框。<br>两种情况都只显示 stderr，不显示 stdout，`\n` 正常换行。<br>Pre-commit 失败时不调用 Post-commit | `CWD` 不能用来定位仓库，计划改成在注册的命令行里带上 `--repo`。<br>TortoiseSVN 按路径前缀匹配钩子。<br>**拦截改在 Check-commit**（分析 §11 第二问）。用户随时可以越过拦截，与 §6.13.4「附加防线」的定位一致 |
| H4 | stderr 按**线程** ANSI 代码页解读（更正，原写「系统 ANSI 代码页」）：同一段中文，写 UTF-8 字节时乱码，写 GBK 字节时正常，两种对话框表现一致。<br>提交因过期（E155011）失败时，check 和 pre 都被调用，**Post-commit 不被调用**。<br>更新时：`start_update` 收到 `PATH`、`CWD`；`pre_update` 收到 `PATH`、`DEPTH`（`-2`）、`REVISION`（`5`，已经解析成具体的号）、`CWD`；`post_update` 收到 `PATH`、`DEPTH`、`REVISION`、`ERROR`、`CWD`、`RESULTPATH`。<br>`RESULTPATH` 列出根目录本身，加上每个实际被动过的文件：冲突、更新、删除、新增；没变化的文件不列。对 `Code` 再更新一次（没有变化）时，`RESULTPATH` 只有 `wc/Code` 这一行。<br>有冲突时 `post_update` 照常调用，`ERROR` 为空 | stderr 要用 `CP_THREAD_ACP` 编码（更正，原写 `CP_ACP`）。<br>收件箱里会出现「有 Pre 无 Post」的孤儿记录，Pre 与 Post 配对要加时间窗。<br>`RESULTPATH` 可以直接当「来自更新」的路径集合（分析 §11 第一问后半）。它包含已经删除的路径，不能用文件是否存在来过滤 |
| H5 | `#![windows_subsystem = "windows"]` 的 Rust exe 当 check_commit 钩子：没有控制台窗口闪现；`std::io::stderr()` 写入成功；退出码 3 被认作失败；GBK 字节正常显示；进程内耗时 0–1ms。pwsh 冷启动每次约 0.35s。<br>「提交后恢复」：用户在提交框里标记文件，手工删掉本地 hunk 后提交。`pre_commit` 看到的磁盘内容是删过之后的版本，`post_commit` 时磁盘仍是提交的版本，恢复发生在 `post_commit` 之后，r7 里没有本地补丁 | jkk 的 exe 可以直接当钩子（分析 §11 第四问）。本仓库没有 single-instance 插件，`main.rs` 也不解析参数，在 Tauri builder 之前分流即可。<br>按 hunk 检查时，输入就是磁盘上的文件（分析 §11「提交后恢复时 Pre-commit 看到的磁盘内容」） |

### 偏离计划（相对分析 §6.13）

- **不注册 Start-update / Pre-update / Start-commit。** 分析 §6.13.1 把 Start-update 列为可选，用途是「更新前钉住索引」，但 C1a 的跟进本来就不在更新时动索引，YAGNI。Start-commit 拿到的只是根目录，没有用处。
- **Local-Only 清单不预先写盘，由 Check-commit 现场用 git2 读**（分析 §6.13.4 原写的是 jkk 在 HEAD 变化时写进 `.git/mygitclient/local-only/`）。这样没有清单过期的问题，分支上没有 Local-Only commit 时只多一次 revwalk。
- **健康检查只核对注册表条目**（三个必选 kind 都在、exe 路径是当前 exe、`--repo` 是本仓库）。分析里设想的「wc.db 显示有过提交，收件箱却没有事件，就判定失效」这次不做。理由：钩子健康时，「作者是我却没有事件」的路径一律按拿不准处理、保留暂存，方向本来就安全，额外的推断只能改善提示，不影响数据。
- **注册的命令行里带 `--repo`**，钩子不再按 `CWD` / `PATH` 往上找仓库（H3：`CWD` 不是工作副本根）。
- **收件箱只在跟进时读。** 分析 §6.13.2 写的是「jkk 运行时立即处理（放行文件监听，或在焦点检查时读）」，但按事件即时触发跟进属于 C2（§6.13.6）。C1h 里发现外部提交仍靠 C1a 的焦点 + wc.db 戳。
- **快照加 `at_ms`。** 分析 §6.13.3 要求比较 M 的 `at` 与 t_commit，而现在的 `at` 是 unix 秒，同一秒内分不出先后。

### 为什么这么改是安全的

- 钩子除了 Check-commit 命中泄漏之外，一律退出 0，panic 也兜住。所以 jkk 有缺陷、仓库坏了、收件箱写不进去，都不会挡住用户提交。
- 拦截可以被用户越过（H3），所以误报的代价只是多点一次「Proceed anyway」。
- 注册只追加、删除 jkk 自己的条目，其他条目逐字节保留；写入用 CAS，写完再读回核对。
- 判定只把事件翻译成 C1a 已有的 `PathFacts`，`decide` 和发布流程都不改。健康以外的任何状态都走 C1a 原来的路径，C1a 的跟进测试一条都不改。

### 执行方式

用户选了 Native，即主会话逐个 Task 实现，并要求中途补独立 review：Task 2、Task 5 之后各一次，收尾再审一次整个分支。superpowers 的 `sdd-workspace` 脚本内部要调 git，被 worktree 隔离拦下，所以 ledger 就用本文件（AGENTS.md §2.1）。

### 未测、留给 E2E（计划 Task 7）

- `%TEMP%` 或工作副本路径含空格时，TortoiseSVN 追加的参数会不会加引号。
- 勾选「Always execute the script」之后，注册表第 6 字段的取值和语义。
- Unity 大工程上的钩子耗时。
- 用户在 TortoiseSVN 设置窗口开着的时候 jkk 去注册，设置窗口关闭时会不会整值写回、覆盖掉 jkk 的条目。

## Task 1：注册表条目的解析、注册计划与健康判定

### 偏离计划

- **新增三个组合函数：`desired_entries`、`registered_value`、`unregistered_value`。** Ruling：计划只列了 `plan_registration` 和 `without_ours`，但注册与撤销命令（Task 6）都需要「从旧值算出新值」这一步。放在 engine 里做成纯函数可以直接单测，命令层只剩读、CAS 写、读回。错了的代价：Task 6 改调用名。
- **注册时整套重写本仓库的条目**：先 `without_ours`，再追加完整的 `desired_entries`，不只追加 `plan.add`。Ruling：本仓库已有的条目可能指向旧 exe（Stale），只追加会留下两条同类型的条目。错了的代价：无，结果的值是确定的。
- **冲突的范围也包括子孙路径**（计划只写了「相同或祖先」）。Ruling：子孙路径上的同类钩子同样会命中这个工作副本里的一部分提交，TortoiseSVN 选哪条没测过，当冲突处理更保守。错了的代价：多拦一次注册，用户能看到冲突的具体条目。
- **`registration_roots` 把 `wc_root_abspath` 的 `/` 转成 `\`**：TortoiseSVN 界面写进去的路径是反斜杠（H1）。比较时两种都认。

### 计划没写的坑

- 少数钩子类型的名字不带 `_hook` 后缀（`manual_precommit`）。`serialize_hooks` 对这种类型原样写回，避免把别人的条目改坏。

### 为什么这么改是安全的

- 只有 `ours_for`（命令行含 ` --tsvn-hook `，且 `--repo` 是本仓库）的条目会被删或改。别人的条目、别的仓库的 jkk 条目都原样保留，有测试 `foreign_entries_survive_a_round_trip`、`without_ours_keeps_entries_of_another_repo`。
- `write_hooks_value` 写之前重读，和预期不同就不写；写完再读回核对。

### 遗留风险

- **注册表读写（`registry::read` / `write`）没有自动化测试**：测试会写开发机的真实 `HKCU`，而且在本会话的自动模式下会被权限分类器拦下。留给 Task 6 的 `pnpm tauri dev` 实测和 Task 7 的 E2E。
- 测试 `registered_value_*`、`unregistered_value_*` 是在实现之后补的，不是先红后绿。用一次变异核对过：把 `without_ours` 换成原样保留，`registered_value_replaces_stale_entries_and_keeps_foreign_ones` 会失败。

## Task 2：钩子子命令与收件箱写入

### 偏离计划

- **Check-commit 不写事件**（计划写的是「暂时只写事件、返回 0」）。Ruling：判定用不上 Check-commit 事件，写了只会在收件箱里堆积，等 TTL 才能清掉。Task 3 的 `check_commit_events_are_ignored_by_evidence` 仍保留，作为防御。错了的代价：以后要用时，在 `run_hook` 里补一行。
- **`parse_hook_args` 不含 argv0，`tsvn_hook_main` 含。** 第一个参数是 `--tsvn-hook`、后面却不完整时，`tsvn_hook_main` 也返回 `Some(0)`，绝不落到 GUI：TortoiseSVN 在等这个进程退出。测试 `malformed_hook_args_still_exit_zero_without_starting_the_gui`。
- **RESULTPATH 里的仓库根本身不记**（`to_repo_relative` 对根返回 `None`）。H4 显示 RESULTPATH 第一行就是根目录。

### 计划没写的坑

- `to_repo_relative` 要逐字符比较大小写：先整体转小写再按字节切片的话，非 ASCII 字符转小写后字节长度可能变化，切出来的位置会错。

### 为什么这么改是安全的

- `tsvn_hook_main` 用 `catch_unwind` 包住整个钩子。除了 Task 4 的拦截，所有出口都是 0。收件箱写不进去（`.git` 不是目录）时也是 0，而且不会凭空造出 `.git`（测试 `unwritable_inbox_still_exits_zero`）。

## Task 3：收件箱读取、配对与清理

### 偏离计划

- **带错误的 `post_update` 仍算更新证据。** Ruling：部分失败的更新，RESULTPATH 里列出的路径确实被更新碰过，变化来自服务器。错了的代价：这些路径走三方合并，而不是按我的提交处理。三方合并本身是保守的方向。
- **`prune_inbox` 删除已处理的 Post 时，连同它配上的 Pre 一起删**（计划只写了「已处理的删除」）。不然 Pre 会变成孤儿，要等 24 小时才清掉。

### 遗留风险（Task 3）

- 配对靠「路径集合相同」加时间窗。两次提交勾了完全相同的文件、在 10 分钟内先后进行，第一次的 Pre 在第二次 Post 之前时，第二次的 Post 会配上最晚的那个 Pre，也就是它自己的 Pre，结果正确。只有第二次的 Pre 丢失时，才会错配到第一次的 Pre：t_commit 偏早几分钟。偏早意味着失焦快照更可能晚于 t_commit，判定落到「拿不准」，方向安全。

## Review 1（Task 1–3，独立 reviewer，opus，2026-11-02）

结论：With fixes。Critical 无。Important 5 条：第 1–4 条已修，第 5 条记为风险并补进计划。

| 条目 | 处理 | 验证 |
|---|---|---|
| I1 `tests/tsvn_inbox.rs` 没过 `cargo fmt --check`，总账 Task 3 也没写 fmt 结果 | 已修：跑了 `cargo fmt`。从这里起，每个 Task 的总账都写上 fmt / clippy 的结果 | `cargo fmt --check` 通过，`cargo clippy --all-targets` 0 告警 |
| I2 解析 → 序列化有损（CRLF、残条、旧版 5 行格式、错位），而注册与撤销都会从解析结果重写整个值，可能改坏别人的钩子；原测试的输入和期望都由 `serialize_hooks` 生成，测不出这个问题 | 已修：新增 `check_recognized`，要求逐字节往返一致，而且类型行、wait、show 都是认得的取值，否则报 `tsvn_hooks_unrecognized`、不写入。`registered_value` 走这道检查，新增 `try_unregistered_value` 给撤销用 | 手写原文的测试：`foreign_raw_value_round_trips_byte_for_byte_including_manual_precommit`（一开始就通过，把 `manual_precommit` 特例钉住了）、`value_that_does_not_round_trip_is_refused_not_rewritten`（先红后绿，覆盖 4 种形状） |
| I3 仓库在盘符根目录时，`hook_command` 生成 `--repo "D:\"`，`\"` 被拆成字面引号，后面的参数全部并进 repo，钩子写不出事件，健康检查却判 `Healthy`；也没有测试走过真实的 Windows 拆参 | 只修了一半（Final Review 第 1 条更正）：`hook_command` 去掉 repo 的结尾分隔符后，拆参对了，但 `D:` 是驱动器相对路径，钩子仍打不开仓库。新增进程级测试：按注册时的命令行拼上仿 TortoiseSVN 的参数，用 `raw_arg` 启动真实的 `app.exe`，30 秒不退出就判失败（这同时覆盖了 `main.rs` 分流后不会落到 GUI） | `hook_command_trims_trailing_separator_of_a_drive_root_repo` 先红后绿；`real_exe_with_registered_command_line_writes_one_event` 通过，撤掉修复做变异时失败（收件箱为空） |
| I4 `check_commit_writes_no_event_and_exits_zero_without_local_only_commits` 名不副实 | 已修：改名为 `check_commit_writes_no_event_and_exits_zero`，删掉误导的注释。「没有 Local-Only commit」留给 Task 4 用真实仓库测 | 14/14 |
| I5 exe 不在了（便携版换目录、卸载前没撤销、开发构建的目录被删）之后，TortoiseSVN 很可能把 `CreateProcess` 失败当成钩子失败，每次提交都要点「Retry without hooks」。「不挡提交」只在 exe 能启动时才成立 | Ruling：Task 1–3 的代码不改，钩子进程本身管不到 exe 不存在的情况。计划 Task 6 补两点：`Stale` 横幅写明「TortoiseSVN 提交时可能报钩子错误」，并且 `Stale` 下也能撤销注册；从开发构建路径注册时显示警告。Task 7 的 E2E 第 7 条实测 exe 不存在时的真实表现。错了的代价：用户移动或卸载 jkk 之后，要到 TortoiseSVN 设置里手工删钩子 | 计划已更新 |

另一处修正：前面「为什么这么改是安全的」里「一律退出 0，不会挡住提交」这句话，前提是 exe 能启动（见 I5）。

### 工作副本在盘符根目录时的 `CWD`

I3 的另一半：工作副本在盘符根目录时，TortoiseSVN 追加的 `CWD` 是 `D:\`。如果 TortoiseSVN 给它加了引号，就成了 `"D:\"`，排在它后面的 RESULTPATH 会被吞掉，更新事件写不出来。这取决于 TortoiseSVN 怎么给参数加引号，这一点没测过。Ruling：留给 E2E（Task 7 第 7 条）。缺了事件只会落到「拿不准」，方向安全。错了的代价：工作副本在盘符根目录的用户得不到更新事件。

### 延后的 Minor

- Final: minor (deferred): 两次 `RegGetValueW` 之间值变长会报 `ERROR_MORE_DATA`，现在直接报错，没有重试。
- Final: minor (deferred): ERROR 文件只按 UTF-8 读；非空但不是 UTF-8 的内容会被记成「没有错误」。按 H4，失败时根本不调用 Post-commit，影响很小。
- Final: minor (deferred): 收件箱里残留的 `.tmp` 和读不懂的 `.json` 永远不会被 prune 掉。
- Final: minor (deferred): 用户后来在子目录上加的同类钩子会遮住 jkk 的钩子，健康判定看不出来（方向安全：只是落到「拿不准」）。
- Final: minor (deferred): 路径为 `*` 的通配钩子（reviewer 凭记忆提出，待核实）。
- Final: minor (deferred): 两个 jkk 仓库共用同一个 SVN 工作副本根时，冲突提示分辨不出对方也是 jkk 的条目。
- Final: minor (deferred): `wc_roots` 为空时 `judge_health` 判 `Healthy`，这个判定是空洞的。由 Task 6 的调用方挡住。

## Task 4：Check-commit 按磁盘内容拦截 Local-Only 泄漏

### 偏离计划

- **没有 `judge_bytes`。** 计划要的这个函数最初写了，但生产路径不走它，只有测试在调（Review 2 第 1 条），已删除。Ruling：钩子路径直接用 `judge_sides`，N 取磁盘字节。错了的代价：无。
- **按 `Side`（不存在 / 二进制 / 文本）重构 `judge_path`**，把按对象和按磁盘的两条路径收到同一个 `judge_sides` 里。磁盘上的 N 用「前 8000 字节里有 NUL」判二进制，与 libgit2 的判据有差异，见 `side_of_bytes` 的注释。
- **不用 `commits_between` 和 `FOLLOW_CODES`，改用 git2 revwalk（`svn/base..HEAD`）加 `graph::parse_local_only`。** Ruling：钩子进程里不起 git 子进程；`commits_between` 要求 oid 参数，还会起 `git log`。Local-Only 的判定规则与交付、跟进是同一个函数，不会漂移。错了的代价：无，`FOLLOW_CODES` 保持原样。
- **判定为 `Unknown`（拿不准）的 hunk 也拦截**，提示里标「（可能）」。Ruling：拿不准时多拦一次，代价只是用户点一次「Proceed anyway」（H3）；放过的话本地补丁可能上服务器。与 F3「拿不准一律给 Unknown、宁可多告警」同一方向。
- 预先生成 Local-Only 清单这一项的偏离，已记在 Task 0「偏离计划」里。

### 计划没写的坑

- TortoiseSVN 给的路径大小写不一定与 git 记录的一致（盘符一定是大写，H2）。`local_only_leaks_on_disk` 比较仓库相对路径时不区分大小写，有测试 `disk_check_matches_paths_case_insensitively`。

### 为什么这么改是安全的

- `judge_path` 的语义没变：重构后 `svn_leak` 原有的 19 条测试和 `svn_follow` 的 133 条全部通过。另外，C1a Review 2 的 9 个用例经由 `judge` 辅助函数逐个核对：钩子走的 `local_only_leaks_on_disk` 与 `local_only_leaks` 的结论一致（本节 C1h Review 2 第 1 条之后才成立，之前核对的是一个生产代码不调的函数）。
- 钩子侧任何错误（`svn/base` 不存在、HEAD 读不出、文件读不了）都放行，有测试 `check_commit_on_error_fails_open`。

## Task 5：跟进按钩子事件判定

### 偏离计划

- **测试放在新文件 `tests/svn_follow_hooks.rs`，不加进 `tests/svn_follow.rs`。** Ruling：`svn_follow.rs` 单个文件就要跑 318 秒，而且 helper 是文件私有的。新文件复制了需要的几个 helper，可以单独快速迭代。错了的代价：两份相似的 helper。
- **清理收件箱的判据改成时间截点，不用计划写的「事件里每条路径在新映射里的 revision ≥ 事件的 revision」。** Ruling：更新事件的 revision 是「更新到哪个版本」（H4 里 `pre_update` 的 `5`），而映射记的是每个文件最后一次修改的 revision，两者不可比：一个在 r4 改过、更新到 r5 的文件永远不会满足条件，只能等 14 天的 TTL。改为：读收件箱之前先记下时刻（`FollowReport.evidence_cutoff_ms`），跟进完成（Done）之后，早于这个时刻的事件都已反映在新基线里，可以删。错了的代价：跟进只覆盖了一部分路径（比如白名单之外）时，那部分的事件也被删了；但它们本来就用不上。
- **`FollowReport` 多了 `evidence_cutoff_ms` 字段**（计划只列了 `evidence`）：截点要随 journal 活过崩溃恢复，`finish` 是在 journal 里读报告的。两个字段都是 `#[serde(default)]`，journal 里的旧报告读回来是 `Heuristic` 和 `None`（契约测试里断言了）。
- **只有 Done 才清理收件箱。** 跟进结果是 NeedsManual 时，基线已经发布、分支还没动，之后的分支跟进还要用这些事件。
- **跟进时的健康检查用 `quick_health`**：只读注册表，工作副本根取本仓库 jkk 条目自己登记的路径，不重新跑 `enumerate_wcs`（那要对每条白名单跑 `svn info`）。代价是看不出「白名单后来多了一个工作副本」，那个子树没有事件，判定落到「拿不准」，方向安全。
- **提交事件的判定不再要求路径在 `after_mark` 里**（C1a 的判定要求）。Ruling：`after_mark` 防的是「快照比那批 SVN 提交还新」（基线已发布、分支落后，C1a Review 2 第 1 条）。有事件时，`mark.at_ms < t_commit` 直接证明快照早于提交，不需要再旁证。F2 ① 的 `replaceable` 仍然与 `after_mark` 求交集，照计划做，更保守。错了的代价：无，比原判据更严。
- `Option::is_none_or` 要 Rust 1.82，本 crate 声明的 MSRV 是 1.77.2，clippy 报 `incompatible_msrv`，改用 `map_or(true, …)`。

### 计划没写的坑

- 不注入健康状态的测试（C1a 原有的全部跟进测试）会经由 `quick_health` **只读**开发机的 `HKCU\Software\TortoiseSVN\hooks`。临时仓库的路径不可能和那里的 `--repo` 对上，结果总是 `NotRegistered`，不影响测试；但测试确实读了开发机的状态。只读，不写。

### 为什么这么改是安全的

- 判定表只把事件翻成 C1a 已有的 `PathFacts`。`decide` 和发布流程都没改。
- 「钩子健康却没有事件、作者是我」从「丢弃」改成「保留、标拿不准」（R22），方向更保守。
- 钩子失效、没注册时，结论与改动前完全相同：`stale_hooks_fall_back_to_heuristic`、`not_registered_reports_heuristic_and_keeps_inbox`。C1a 原有的跟进测试一条都没改。

## Task 6：Tauri 命令与前端入口

### 偏离计划

- **`TsvnHooksStatus` 多了 `exe` 字段**：前端要据此提示开发构建（Review 1 第 5 条补进计划的那一项）。
- **`planned` 由 `desired_entries` 给出整套条目（含 check_commit），前端按勾选过滤**（`entriesToWrite`），不用 `plan.add`。Ruling：注册时后端整套重写本仓库的条目（Task 1 偏离），所以注册框里要列的就是整套。错了的代价：已注册时列表里也会出现已经存在的条目，这是如实的。
- **注册框的主按钮在已注册或已失效时显示「重新注册」**，撤销注册在这两种状态下都能用（Review 1 第 5 条）。
- **`tsvn_hooks_status` 在找不到工作副本根时判 `NotRegistered`**，注册命令直接报 `tsvn_hooks_no_wc`。这条处理了 Review 1 Minor 里的「`wc_roots` 为空时判 `Healthy`」。
- **入口只在「体检基线」那条安静横幅上**（计划写的「手动体检旁边」）。那条横幅只在基线与工作副本一致时出现；不一致时没有入口，但钩子失效横幅不受影响，任何时候都会出现。Ruling：不为它另开一个入口，常态下基线一致。错了的代价：失同步期间要先跟进，才能打开注册框。
- **失效横幅用现有的 `warn` 色调**，不新增 CSS。
- **`pnpm tauri dev` 实测（计划 Step 4）拆成两半。** Ruling：注册会写开发机的真实 `HKCU`，必须由用户在 jkk 里自己点，所以写注册表的路径留给 Task 7 的 E2E。参数名不用等：2026-11-02 在真实窗口里经 CDP 调 `window.__TAURI_INTERNALS__.invoke` 核对过（`app.exe` 路径确认在本 worktree 的 `target\debug` 下）。路径用一个空的临时目录：三个命令用正确的参数名调用，都解析成功、停在 `svn_not_hybrid`，没有碰注册表；故意写错的 `block_leaks` 和 `repo` 报 `missing required key blockLeaks` / `path`，证明上面不是巧合。核对后读注册表确认没有 `--tsvn-hook` 条目，`session.json` 没变。

### 计划没写的坑

- `.git` 是文件（linked worktree）的目录，`detect_repo_kind` 今天就不认它是混合仓库，所以 `hooks_target` 里 `tsvn_hooks_no_git_dir` 这道检查在测试里走不到。核对过程：用变异去掉这道检查，测试照样通过。处理：测试改名为 `hook_commands_refuse_a_linked_worktree_layout`，接受两种错误码之一，并在注释里写明这是纵深防御。
- `worktree` 里没有 `node_modules`，跑 vitest 之前先 `pnpm install --frozen-lockfile --offline`（只用本机 pnpm store，不新增依赖）。

### 遗留风险

- 三个命令真正读写注册表的路径没有自动化测试，留给 Task 7 的 E2E。

## Review 2（Task 4–6，独立 reviewer，opus，2026-11-02）

结论：With fixes。Critical 无，没有发现数据丢失问题。Important 3 条全部已修；Minor 第 8 条按实际影响上调为 Important，一并修了。

| 条目 | 处理 | 验证 |
|---|---|---|
| I1 Task 4 的交叉核对测的不是钩子的真实路径：`judge_bytes` 只有测试在调，`local_only_leaks_on_disk` 直接走 `judge_sides`，缺 `n == b` 这一步。磁盘等于 B 的二进制文件会判 Unknown，被挡下 | 已修：删掉 `judge_bytes`；钩子路径补上等价的比较（`Oid::hash_object` 对比 B 的 oid）；`judge` 辅助函数改成把 N 写到磁盘、调 `local_only_leaks_on_disk`。**更正 Task 4「为什么这么改是安全的」**：那里写的「Review 2 的 9 个用例经由 `judge` 核对 `judge_bytes`」对钩子路径不成立，那时核对的是一个生产代码根本不调的函数。现在这些用例走的才是真实路径 | `disk_equal_to_base_is_not_a_leak` 先红（二进制形状失败，正是 reviewer 说的情形）后绿；`svn_leak` 28/28 |
| I2 健康状态只在打开仓库时读一次，同一会话里钩子失效时横幅不出现；E2E 草稿场景 7 第 5 步按现有代码会失败 | 已修：跟进之后重读一次（`refreshAfterFollow`）。不放进焦点事件：这个命令要跑 `svn info`。E2E 第 5 步改成与实际行为一致。Ruling：`RepoTab` 里这一行接线没有自动化测试，这个文件目前没有组件测试，靠 E2E 场景 7 验证。错了的代价：要重开标签页才看得到失效横幅 | E2E 场景 7 |
| I3 只有 `check_commit` 上有冲突时，取消勾选拦截也注册不了：冲突固定按 `block_leaks = true` 算，后端其实会接受 | 已修：冲突列表也按勾选过滤（复用 `entriesToWrite`） | `冲突只在 check_commit 上时，取消勾选拦截就能注册`、`必选钩子上的冲突不因取消勾选而消失`，前者先红后绿 |
| M8（上调）注册或撤销失败时，错误写在被遮罩盖住的顶部红条上，框里没有任何变化；没有 busy，连点两次第二次会报「被别的程序改过」。Review Focus 第 2 条的那句提示正是靠它传达 | 已修：`TsvnHooksDialog` 加 `error`（`role="alert"`）和 `busy`。从失效横幅撤销时框没开，错误照旧走顶部红条 | 2 条组件测试先红后绿 |
| M1（只改注释）`Who::gather` 的注释「读的过程中新到的事件时刻更晚」不严格成立：钩子的 `at_ms` 取在写文件之前 | 注释改成如实的说明；行为不改，方向安全 | — |

核实记录：reviewer 凭记忆说 libgit2 的 `is_binary` 判据更宽，它自己标了「没核实」。我对照了 `Cargo.lock` 锁定的 `libgit2-sys 0.18.7+1.9.6` 的 `git_str_is_binary`：确实还看 UTF-16/32 BOM 与不可打印字符的比例。`side_of_bytes` 的注释按此写。

### 延后的 Minor

- Final: minor (deferred): `evidence_cutoff_ms` 取在 `check_sync_with_nodes` 之后，InSync 分支跟进入口没有重扫兜底；可以把时刻提到 `prepare` 开头，或只删这次实际读到的文件。后果只是少了证据。
- Final: minor (deferred): 磁盘内容没过 git 过滤器。`has_eol_rule` 为假的旧仓库，Local-Only 碰过的文件可能频繁被误挡（代价是多点一次 Proceed anyway）。读代码推出来的，没有实测。
- Final: minor (deferred): 时钟回拨幅度大于「提交到快照」的间隔时，暂存可能被误丢；概率很低，有钉住的索引兜底。
- Final: minor (deferred): 单个文件读失败（如 Unity 锁着）时整次拦截放行，已查出的泄漏一起丢弃。方向是 fail-open，由 F3 事后兜底。
- Final: minor (deferred): `tsvn_hooks_status` 与 `quick_health` 的根来源不同：白名单新增工作副本后，横幅写「跟进暂按作者 + 快照判定」，而跟进实际还是 Hooks。
- Final: minor (deferred): 按作者 + 快照判定时收件箱从不清理，TTL 与 `MAX_EVENTS` 都不生效；`read_evidence` 也不看 TTL。
- Final: minor (deferred): `roots` 为空且注册表里还有本仓库的旧条目时判 `NotRegistered`，注册框里没有撤销入口，只能去 TortoiseSVN 设置里删。
- Final: minor (deferred): 测试缺口：NeedsManual 不清收件箱、崩溃恢复后清理、InSync 分支跟进入口带事件、`follow_prunes_processed_events` 只数条数；磁盘路径的 detached HEAD 与 merge commit。
- Final: minor (deferred): `TsvnHooksDialog` 没有 Escape 关闭，打开时焦点不进框。

## Task 7：E2E 清单与文档回写

E2E 于 2026-11-02 在真实窗口和真实 TortoiseSVN 上跑完，场景 1–8 通过，7c 未跑。逐条结果与「没验到」的范围见 `docs/superpowers/e2e/2026-11-02-svn-c1h-tsvn-hooks-e2e.md` 的状态表。二进制核对过在本 worktree 的 `src-tauri\target\debug\app.exe`。

### 偏离计划

- **7c（盘符根目录）没跑。** Ruling：`subst J: $WC` 的写法大概率测不到东西：TortoiseSVN 按路径前缀匹配钩子，`J:\` 不匹配注册的 `C:\...\wc1`（推断，未实测）。要验得用 `J:\` 打开并注册，这是一轮额外的注册表写入，而缺事件时判定只落到「拿不准」。错了的代价：工作副本在盘符根目录的用户得不到更新事件。
- **没有单独重走「无钩子 + 暂存 + 真实 TortoiseSVN」。** 场景 7 第 5 步没有暂存文件，只验到判定依据退回。Ruling：判定路径有 `stale_hooks_fall_back_to_heuristic` 与 C1a 原有测试覆盖，失焦快照的时序在场景 2 用同一份快照验过，用户决定 D2 也是把场景 13 并入钩子版本。错了的代价：无钩子时 TortoiseSVN 弹框的焦点时序导致暂存落到「拿不准」，方向安全。
- **E2E 由 Claude 驱动，写注册表的按钮由用户点。** 真实按钮点击和 TortoiseSVN 界面操作都是真实的，Claude 不代点注册类按钮。

### 计划的错误

- **Task 0 的「系统代码页 936」是没有实测的推断，错了，并连带写错了计划和实现。** 本机 `GetACP` = 1252、`GetOEMCP` = 437，系统 locale en-US；用户 locale zh-SG，线程 ANSI 代码页 936，TortoiseSVN 的 `LanguageID` 未设。`WideCharToMultiByte(CP_ACP)` 把「钩子」编成 `3F 3F`，`CP_THREAD_ACP` 才是 `B9 B3 D7 D3`。H5 用的探针直接写 GBK 字节，所以当时看到的「GBK 正常」没能暴露 `CP_ACP` 的问题。这是 fixture 建不起来时查 ACP 才发现的，在场景 5 之前修掉：`encode_acp` 改名 `encode_thread_acp`，先红（`[63, 63]`）后绿。
- **E2E 清单 0.2 的 fixture 脚本跑不起来：** `svnadmin` / `svn.exe` 按 ACP 转换命令行参数，中文绝对路径变 `??`，报 E720123。已改成 SVN 仓库和 `wc2` 放 ASCII 目录、只把 `wc1` 搬到中文路径，辅助函数用工作目录 + 相对路径。jkk 的 `svn_command` 走 `current_dir`，中文路径工作副本实测全流程正常。
- **清单场景 6 造不出过期：** `B.cs` 在 `wc1` 不存在。已改成先由同事种子、`wc1` 更新、跟进，再同事改 + 本地改。
- **清单场景 6 第 3 步：** `svn resolve --accept working` 保留带冲突标记的文件，直接提交会把标记提交上去。已改成先写回干净内容。
- **清单 7c 的 `subst` 写法**，见「偏离计划」。

### 计划没写的坑

- **exe 不存在时要过两道拦截（Review 1 第 5 条的实测结果）：** Check-commit 报「Hook script failed / The system cannot find the file specified」，点 Proceed anyway 之后 Pre-commit 让提交失败（「The operation failed」），要点 Retry without hooks。提交成功后没有事件。
- 第 6 字段 `ask`、`hide`、`true` 注册后 TortoiseSVN 设置界面显示正常，4 条条目可见。
- 失效横幅只列第一条原因（7a 的 `pre_commit`），完整列表在注册框里。
- 健康状态不实时：删掉 exe 后，要跟进之后才出现失效横幅。与 Review 2 第 2 条的设计一致。
- 钩子健康时，报告仍可能问 SVN 用户名：`needsSvnUserPrompt` 在有「拿不准的暂存」时总问，而这时判定不看作者。
- `__TAURI_INTERNALS__.invoke` 不可写，E2E 打桩不了系统文件框；对编辑框 `WM_SETTEXT` 也不改变 IFileDialog 的内部状态，点确定返回的是当前目录。改用 `save_session` 把 fixture 写进最近打开，再点列表项。

### E2E 期间的事故

**误打开了用户的真实仓库。** 为了绕过系统文件框，我用 `WM_SETTEXT` 写路径并点确定（见上一节），框返回的是当前目录，jkk 打开了 `C:\Users\liuhao\workspace\dev\Dev\koodo-reader`。没有做任何 git 写操作，立刻关了标签页，核对 `git status --short` 0 行、分支和 HEAD 未变，并告知了用户。这条路径已弃用。当时 jkk 显示 `Changes (177)` 而 `git status --short` 是 0 行，原因没有查，不能当作 jkk 没有问题的证据。

`session.json` 被 dev 实例改过（最近打开里多了 koodo-reader 和 fixture），收尾时已从备份恢复，核对过文件 hash 一致。

### 为什么这么改是安全的

- `CP_THREAD_ACP` 只影响 stderr 给人看的字节，不影响退出码和拦截判定。
- 测试 `stderr_text_follows_the_thread_code_page_not_the_system_one` 只改测试线程的 locale，结束时恢复；在 ACP 本身是 936 的机器上抓不到退回 `CP_ACP` 的改动，已写在测试注释里。
- E2E 全程只写 fixture 和 `HKCU\Software\TortoiseSVN\hooks`，写注册表由用户点按钮。结束时 `hooks` 值不存在，与开始时相同；fixture 两个目录已删；dev 实例已停，端口已释放。

### 延后的 Minor

- Final: minor (deferred): 用户 locale 不是中文时，线程代码页表示不了中文，拦截提示会变 `?`。文件名和行号（ASCII 部分）还在，退出码不受影响。
- Final: minor (deferred): 钩子健康时，报告仍可能问 SVN 用户名。
- Final: minor (deferred): 失效横幅只列第一条原因。
- Final: minor (deferred): 新测试在 ACP = 936 的机器上抓不到退回 `CP_ACP`。
- Final: minor (deferred): exe 消失后每次提交多两道操作；注册框「先撤销注册」的提示目前只对开发构建显示，安装版卸载前没有提示。

### Task 0「未测」清单的收口

| Task 0 的未测项 | 结论 |
|---|---|
| `%TEMP%` 或工作副本路径含空格时的引号 | 工作副本路径含空格和中文已验（E2E）。`%TEMP%` 含空格未测：拆参失败时只是写不出事件，方向安全 |
| 勾选「Always execute the script」后第 6 字段的取值 | 未测。`check_recognized` 不看这个字段，取值变化不影响解析 |
| Unity 大工程上的钩子耗时 | 未测 |
| 设置窗口开着时注册 | 未测，见 Final Review 第 3 条 |

## Final Review（整分支，独立 reviewer，opus，2026-11-03）

结论：With fixes。Critical 无。Important 3 条全部已修。reviewer 没有跑 cargo / pnpm，结论来自读代码；3 条我都对照代码核对过。

| 条目 | 处理 | 验证 |
|---|---|---|
| I1 盘符根：`--repo "D:"` 是驱动器相对路径，`join(".git")` 得到 `D:.git`，相对钩子的工作目录解析。事件写不出，Check-commit 也打不开仓库放行，界面却是 Healthy，**本地补丁静默不再被拦**。Review 1 第 3 条的两条测试一条只比字符串，一条用的是临时子目录，都测不到 | 已修：`parse_hook_args` 经 `restore_drive_root` 把 `D:` 补成 `D:\`。放在钩子侧，已注册的命令行不用重新注册 | `drive_root_repo_is_restored_to_an_absolute_path` 先红（`"D:"` ≠ `"D:\\"`）后绿；`repo_that_is_not_a_bare_drive_is_left_alone` 防误伤。盘符根的真实 TortoiseSVN 行为（`CWD` 的引号形状）仍未测，见 E2E 7c |
| I2 `with_hooks_intermediate_commit_is_superseded_only_for_event_paths` 名不副实：B 不在 `after_mark` 里，断言永远成立。保护 commit 不被换掉的两道判据（只认事件、快照早于提交）都没有测试 | 已修：原用例改名 `…_for_an_event_path`，去掉假断言。新增两条用例 | `with_hooks_my_author_without_event_commit_path_is_replayed_not_superseded`、`with_hooks_commit_made_after_the_svn_commit_keeps_its_work`。生产代码本来就是对的，所以这两条一写就绿，用两次变异证明：去掉 `committed_after_mark` 过滤，第二条红；`mine` 退回按作者判定，第一条红。变异已撤销 |
| I3 Review Focus 第 2 条与实现不一致且没记：钩子被整值覆盖后判 `NotRegistered` 而不是「失效」，没有横幅；CAS 失败分支没有测试 | 已修 CAS：抽出 `cas_write`，注册表读写作为闭包传入。注册框加提示。Ruling：不做「记住注册过」的标记，条目全部消失时仍判 `NotRegistered`。TortoiseSVN 设置窗口是否整值写回没有实测，为一个未证实的情形加持久化状态不值；方向是退回作者 + 快照、失去拦截，提交不会被挡。错了的代价：用户注册后点了设置窗口的确定，拦截静默关闭，只有跟进报告的判定依据能看出来 | `cas_refuses_to_write_when_the_value_changed_since_it_was_read`、`cas_reports_a_value_overwritten_right_after_the_write` 等 4 条，先红（`cas_write` 不存在）后绿；`提醒先关掉 TortoiseSVN 的设置窗口…` 先红后绿 |

Review 的 Minor 第 6 条（`post_update` 的 REVISION 真实取值）已由 E2E 关闭：场景 4、6 的真实事件 revision 分别是 5 和 9，能解析。

### 未记录的偏离（Minor 第 5 条补记）

- `main.rs` 直接调 `app_lib::engine::tsvn_hook_cli::tsvn_hook_main`，`lib.rs` 没有按计划 Task 2 单独导出。无害。
- 计划 Task 6 的 JSON 形状测试放在 `tests/tsvn_hooks.rs`，不在 `svn_backend.rs`。无害。

### Reviewer 搁置、由我裁定

- Final: Ruling: `*` 通配钩子会不会被 jkk 更具体的条目遮住 — 未实测，本机没有这种配置；保持现状，由用户在 TortoiseSVN 设置里看得到全部条目 — 错了的代价：公司的通配钩子在这个工作副本上静默不跑。
- Final: Ruling: `catch_unwind` 管不到的进程级崩溃（libgit2 段错误、栈溢出）— 不处理；结果是 TortoiseSVN 报钩子失败，用户可以越过 — 错了的代价：多点一次。
- Final: Ruling: 一次提交跨多个工作副本根时 TortoiseSVN 选哪条钩子 — 未实测，不处理；缺事件时落到「拿不准」— 错了的代价：这类提交得不到精确判定。

### 延后的 Minor（Final Review）

- Final: minor (deferred): `tsvn_inbox.rs:22` 的注释写「超过之后按没有事件处理」，而 `read_evidence` 不看 TTL。
- Final: minor (deferred): 分析 §6.13.2 的文件名写 `<时间戳>-<序号>.json`，实际是 `<at_ms>-<pid>-<kind>.json`。
- Final: minor (deferred): 分析 §6.13.3 的「t_commit 之前最近的失焦快照」，实际只有一份快照，晚于 t_commit 时直接判拿不准。
- Final: minor (deferred): `svn_leak.rs` 模块头「全程只读对象」没有提 `local_only_leaks_on_disk` 读磁盘。
- Final: minor (deferred): 上面「磁盘内容没过 git 过滤器」一条可以写得更准：只有 L 的纯删除 hunk 会多拦，修改型 hunk 判定正确（读代码）。
- Final: minor (deferred): E2E 误开 koodo-reader 时看到的 `Changes (177)` 与 `git status --short` 0 行不一致，可能是 jkk 的真实缺陷，与 C1h 无明显关系，要单独跟踪。

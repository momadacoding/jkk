# 建库时可选「尊重已有的 .gitignore」——实现日志

计划：`2026-10-28-svn-init-respect-gitignore.md`；spec：`../specs/2026-10-28-svn-init-respect-gitignore-design.md`。
执行方式：主会话直接施工（用户 2026-10-29 选定，不用 subagent）。分支 `worktree-svn-init-respect-gitignore`，
叠在 PR #90 的 c775857 上。未经用户放行不 commit。

## 进度总账

| Task | 状态 | 产出 | 验证 |
|---|---|---|---|
| 1 报告写完整路径与「第 N 行」 | 完成 | `svn_gitignore.rs`（新）、`svn_include_check.rs`、`tests/svn_exclude.rs`、`tests/svn_init.rs` | RED：编译失败（模块不存在）；GREEN：`svn_exclude` 34/34、`svn_init` 63/63；fmt / clippy 无告警；`cargo test` 全绿 |
| 2 配置字段与版本 2 | 完成 | `svn_config.rs`、`svn_init.rs`（字面量）、`tests/svn_config.rs` | RED：编译失败（字段不存在）；GREEN：`svn_config` 25/25；fmt / clippy 无告警；`cargo test` 全绿 |
| 3 `WScope` 统一口径 | 完成 | `svn_scope.rs`（新）；`svn_include_check` / `svn_dirty` / `svn_health` / `svn_server_set` / `svn_sync_check` / `svn_sync_in` / `svn_sync_out_check` / `svn_init` 换签名；`tests/svn_scope.rs`（新）及 4 个测试文件的调用 | RED：编译失败（模块不存在）；GREEN：`svn_scope` 8/8；fmt / clippy 无告警；`cargo test` 全绿（既有断言一条未改） |
| 4 空索引入口补回基线文件 | 完成 | `git_cli.rs`（`run_git_stdin_env`）、`svn_baseline.rs`（`add_paths`）、`svn_init.rs` / `svn_sync_check.rs` / `svn_include_check.rs` 接入；`tests/svn_respect.rs`（新） | RED：`add_paths` 编译失败；接入前重建 / 手动体检 / 同步前检查 3 条按预期失败（`include_set_mismatch` / `Lying`）；GREEN：`svn_respect` 8/8；fmt / clippy 无告警；`cargo test` 全绿 |
| 5 冲突检测 | 完成 | `svn_gitignore.rs`（探针、结果类型、`detect_conflicts`、`is_versioned`）、`svn_include_check.rs`（`expected_set_folded`）、`svn_baseline.rs`（`w_pathspecs` 公开）、`svn_init.rs`（`InitPreflight` / `init_preflight` / `build_config` 改签名）；`tests/svn_gitignore.rs`（新） | RED：编译失败（类型与函数不存在）；中文用例先后暴露 fixture 与 `is_versioned` 两处代码页问题；GREEN：`svn_gitignore` 11/11 + 模块内单测 1/1；fmt / clippy 无告警；`cargo test` 全绿 |
| 6 建库「尊重」路径 | 完成 | `svn_init.rs`（`GitignoreChoice` / `init_hybrid_with` / `GitignoreOutcome` / `RenamedGitignore`、检测阶段、闸门、「写配置」阶段）；`tests/svn_init.rs`（改写 3 条、新增 5 条）、`tests/svn_backend.rs`（契约） | RED：编译失败（`init_hybrid_with` / `GitignoreChoice` / `InitReport.gitignore` 不存在）；GREEN：`svn_init` 67/67、`svn_backend` 29/29；fmt / clippy 无告警；`cargo test` 全绿 |
| 7 建库「不尊重」路径（改名） | 完成 | `svn_gitignore.rs`（`disable_gitignore` / `restore_gitignores`）、`svn_init.rs`（改名阶段、改名后重查、回退时改回、配置记录、完成页数据）；`tests/svn_gitignore.rs` +3、`tests/svn_init.rs` +5 | RED：改名单元编译失败；编排 4 条失败 + 1 条加了前提断言后失败；GREEN：`svn_gitignore` 14/14、`svn_init` 72/72；fmt / clippy 无告警；`cargo test` 全绿 |
| 8 IPC 与向导界面 | 完成（真实窗口验证在 Task 9） | `commands/svn.rs`（两个命令改签名）、`repoStore.ts`、`RepoTab.tsx`、`svnInit.ts`（类型 + 4 个纯函数 + 3 条引导语）、`SvnInitWizard.tsx`；`tests/vitest/svnInit.test.ts` +9、`tests/vitest/SvnInitWizard.test.tsx`（新，4 条）、`svn_backend.rs`（`InitPreflight` 契约） | RED：vitest 9 条失败、组件 4 条失败；GREEN：`pnpm build` 通过、`pnpm test` 54 文件 / 802 条全绿；fmt / clippy 无告警；`cargo test` 全绿 |
| 9 真实窗口验证与文档 | 完成 | 真实窗口发现并修正 2 个缺陷（见 Task 9 节）、勾选框布局；`svn_gitignore.rs` 影子树与按轮检测、`svn_init.rs` 按轮改名与脏清单补登；新增用例 3 条（Rust 2、vitest 2 组）；总设计 §6.3.1 / §6.5、本功能设计 §5 更新 | 最终：`cargo fmt --check` / `clippy --all-targets` 无告警，`cargo test` 65 个套件全部通过；`pnpm build` 通过，`pnpm test` 54 文件 / 804 条全绿；真实窗口 5 个场景全部符合预期 |

## Pre-flight：任务间接口核对

| 生产 → 消费 | 接口 | 结论 |
|---|---|---|
| 1 → 3 | `query_verdicts`、`GitCtx::Repo`、`Verdict::{ignores, from_worktree_gitignore}` | 一致 |
| 1 → 5 | `GitCtx::Probe`、`display_path` | Task 1 先定义 `Probe` 变体但不用；clippy 可能报 dead_code，Task 1 里用 `#[cfg_attr]` 之外的方式规避（见 Task 1 条目） |
| 2 → 3/6/7 | `respect_gitignore`、`RESPECT_GITIGNORE_VERSION`、`DisabledGitignore` | 一致 |
| 3 → 4 | `WScope::hidden_tracked_on_disk` | 一致 |
| 3 → 5 | `server_expected_set(root, &WScope::plain(..), fixes)` | 一致 |
| 5 → 6 | `detect_conflicts(GitCtx::Repo)`、`GitignoreConflict.path` 作 `acknowledged` | 一致 |
| 6 → 7 | `build_repo` 阶段结构、`GitignoreOutcome.disabled` | 一致 |
| 5/6/7 → 8 | `InitPreflight`、`GitignoreOutcome`、三个错误码 | 一致 |

## 决策条目

### 执行方式

- Ruling：不用 `sdd-workspace` / `task-start` / `task-done` 脚本，账本即本文件——worktree 守卫拒绝运行
  内部调用 git 的脚本；`AGENTS.md` §2.1 本来就要求账本落在 `docs/` 下——代价：无。
- Ruling：最终 review 由我自查（用户明确不用 subagent）；合并前的独立 review 门槛（`AGENTS.md` §4.1）
  仍需在 PR 阶段用 `/code-review` 满足——代价：作者自查有盲区。

### Task 1

- 偏离计划 / Ruling：`Verdict::from_worktree_gitignore` 改名 `in_worktree_gitignore`——clippy 的
  `wrong_self_convention`（`from_*` 不该取 `self`），按 `AGENTS.md` §7.3 不用 allow 掩盖——代价：无，
  后续 Task 的接口名随之改。
- 偏离计划 / Ruling：`GitCtx::Probe` 与 `GitCtx::run_code` 本 Task 暂无调用者，加了带理由注释的
  `#[allow(dead_code)]`，Task 5 用上后删掉——代价：若 Task 5 忘删，残留一个无害的 allow。
- 偏离计划：计划里单列的 `report_names_generated_rules_without_exposing_the_exclude_path` 没有新建，
  而是改写已有的 `the_mismatch_report_names_the_rule_for_files_that_are_tracked_but_should_not_be`
  ——它造的正是同一个形态（`git add -f` 进 HEAD 树、被 `/Code/**` 排除），原断言 `exclude:` 在新格式下
  必然失败，改成断言「本应用生成的规则 `/Code/**`」且不含 `info/exclude`。
- 计划没写的坑：`svn_include_check.rs` 在工作区里是 LF，git 提示下次会转 CRLF（`core.autocrlf`）；diff
  统计正常（只含实际改动），不影响。
- 安全：纯显示层改动，裁决查询的参数与解析逐字搬迁，不改变任何判定。

### Task 2

- 计划的错误 / Ruling：已有用例 `an_unknown_version_is_rejected` 拿 `version = 2` 当「未知版本」，与 spec §4
  冲突；改成 3，断言不变——代价：无，「未知版本被拒」的覆盖面不变。
- 计划的命名：测试名 `a_v1_config_round_trips_byte_identical` 实写为 `..._without_the_new_keys`——断言的
  就是「不多出新键」，不是逐字节相同（serde 会重排、补默认字段），旧名会虚报覆盖面（`AGENTS.md` §8）。
- 安全：新字段 `skip_serializing_if` 为空/false 时不落盘，旧仓库读写往返后文件里没有新键；`disabledGitignores`
  只记录、不进命令行，因此不做路径校验。

### Task 3

- 计划没写的坑（实测，git 2.51.2.windows.1）：`check-ignore --no-index` 对**磁盘上不存在**的
  `Code/Gen/X.cs` 照样按目录规则 `Gen/` 判为忽略——git 判前导目录时按「是目录」处理，不看磁盘。
  所以服务器侧路径（体检 diff、服务器清单）可以直接拿去问，不需要先在磁盘上存在。
  `scope_excludes_a_missing_file_under_an_ignored_directory` 钉住了这条。
- 偏离计划 / Ruling：`changed_files` / `outside_changes` / `dirty_w_files` 保持收 `&SvnConfig` 的签名，在
  函数内部 `WScope::for_repo` 建口径，而不是改成收 `&WScope`——它们还有别的调用点，改签名会把改动
  扩散出去；代价是「尊重」模式下一次交付检查会多读一两次基线树（`ls-tree`，毫秒级）。
- 偏离计划：`retain_not_ignored` 只对**不在基线树里**的路径发问，基线文件直接放行——这是「参照基线树」
  的实现，也省掉了绝大多数路径的查询。
- 新增用例（计划外）：`hidden_tracked_on_disk_skips_files_deleted_from_disk`、
  `rules_outside_worktree_gitignores_do_not_count_as_respected_exclusions`——前者钉住 Task 4
  `update-index --add` 的输入约束，后者钉住「只有工作树 `.gitignore` 算已有规则」。
- 安全：不尊重时 `WScope::for_repo` 等于 `plain`，不起进程，判定逐字等于原 `is_w_file`（函数体原样搬迁）。
  既有测试只改调用形状、不改断言，全部通过，即为「不尊重口径行为不变」的证据。
  `is_w_file` 已删除、`path_is_w` 私有，任何入口想判 W-file 只能经过 `WScope`，编译器保证没有遗漏。

### Task 4

- 计划没写的坑：`svn_respect.rs` 里 5 条「`.gitignore` 挡住未进基线的服务器文件」用例（更新、交付、
  服务器核对、体检过滤）在 Task 4 动代码前就已经通过——它们验的是 Task 3 的 `WScope` 接入，不是补回逻辑。
  在 Task 3 之前它们必然失败：「更新」的纳入集合预检会以 `include_set_mismatch` 拦下。
  Task 4 自己的 RED 是另外 3 条（重建、手动体检、同步前检查）加 `add_paths` 的编译失败，均已亲见。
- 偏离计划 / Ruling：补回放在 `record_server_tree_timed` 的「录入磁盘内容」阶段内，对从 HEAD 起步的
  入口（sync_in S7、sync_out D7a）也执行一次——那些文件在 HEAD 树里本来就有，`update-index --add`
  只是按磁盘内容再写一遍，与 `add -A` 的结果相同。不按起点分支，少一个参数、少一个分叉点——
  代价：「尊重」模式下这两个入口多一次 `check-ignore` + `update-index`。
- 安全（为什么 `update-index --add` 安全）：
  - 只作用于 `GIT_INDEX_FILE` 指向的临时候选索引，不碰真实索引、HEAD、磁盘；
  - 路径集合 = 基线树 ∩ W-file ∩ 磁盘上是文件 ∩ 被工作树 `.gitignore` 挡，不会把非 W-file 或 `.svn`
    放进去（W-file 判据已排除 `.svn`：`expected_set` 跳过它，exclude 末行兜底，而基线树里本来就没有）；
  - `-z --stdin` 逐条字面处理，没有 pathspec 魔法；空列表直接返回，不起进程；
  - blob 与 `hash-object --path` 一致（用例钉住），行尾过滤照常。

### Task 5

- 计划没写的坑（实测，本机 ACP 1252、SlikSvn 1.14.2）：svn 的命令行参数走系统 ANSI 代码页，中文路径
  到了 svn 那里变成 `??`（`E720123` / `W155010`）。两处影响：
  1. 测试 fixture 逐个 `svn add 资源/图.exe` 失败 → 改成从根 `svn add --force .`。
  2. **产品代码**：`is_versioned` 若把 `Code/资源/.gitignore` 当参数传给 svn，会得到「找不到节点」→ 判成
     不受控 → 一个团队提交的 `.gitignore` 会被改名（反目标第 2 条）。改为工作目录设成文件所在目录、
     参数只传 ASCII 的文件名 `.gitignore`（工作目录走 Unicode 的进程 API，不受代码页影响）。
     RED 依据是上面同一次运行里 svn 对中文参数的实际报错；用例
     `conflicts_keep_non_ascii_paths_verbatim_and_still_tell_versioned_ones` 断言中文目录里受控的
     `.gitignore` 判为 `versioned: true`。
  - 既有限制（不在本计划内）：中文**白名单根**在这类机器上连 P1–P4 都过不了（`svn info` 收到 `??`）。
    用户的中文 Windows（ACP 936）不受影响。记为已知限制，用例的白名单因此用 ASCII 的 `Code/`。
- 计划的错误 / Ruling：计划说「探针检测与在真仓库上用 `GitCtx::Repo` 检测，结果相同」，但
  `detect_conflicts` 是 `pub(crate)`，集成测试调不到。改为两条断言：①探针对 `*.EXE` / `*.cs` 与
  `a.exe` / `Up.CS` 全部命中；②同一台机器上一个真 git 仓库对 `*.EXE` 与 `a.exe` 同样判忽略——
  代价：Repo 上下文的一致性要等 Task 6/7 用真仓库检测时再间接验证（重查与闸门都走 `GitCtx::Repo`）。
- 计划没写的坑：`expected_set` 用 `ignore_case(repo_root)` 问 git，第 2 步工作副本里没有 `.git`，问到的是
  全局配置（多半为 false），与建库后真仓库（Windows 上为 true）不一致 → `Up.CS` 这类文件在第 2 步不算
  W-file，建库时才冒出来 → 用户被「请重新确认」送回第 3 步，而第 2 步再查还是看不到，**死循环**。
  拆出 `expected_set_folded(.., fold)`，检测时 `fold` 取探针 git 目录里的 `core.ignoreCase`；探针显式按
  工作副本是否大小写不敏感设置它（`.svn` 存在时看 `.SVN` 能否解析）。
- 偏离计划 / Ruling：探针目录残留的检查从「集成测试里按前缀数系统临时目录」改成模块内单元测试
  `the_probe_directory_is_removed_on_drop`——并行跑的其它用例会同时建删探针目录，按前缀计数会时好
  时坏；集成测试改为断言工作副本 `svn status` 里除了 `.gitignore` 没有别的新东西——代价：无。
- 偏离计划：`GitCtx` 增加会检查退出码的 `run`（`run_git_code` 对任何退出码都返回 `Ok`，`ls-files`
  失败会被当成空输出）；Task 1 留下的两个 `#[allow(dead_code)]` 已删除。
- 安全：探针目录建在系统临时目录、`create_dir` 撞名即失败、值先建出来再做后续步骤（任一步失败由 `Drop`
  清理）；所有 git 调用带 `--git-dir` / `--work-tree`，工作副本里不出现 `.git`（用例断言）。检测只读。

### Task 6

- 偏离计划 / Ruling：没有给 `InitRequest` 加字段，而是新增 `GitignoreChoice { respect, acknowledged }` 与
  `init_hybrid_with(req, choice)`；`init_hybrid(req)` 等于「不尊重、一个都没看过」。`InitRequest { .. }`
  字面量全仓 21 处（多在测试 fixture 里），加字段要逐个改；缺省语义正是「查到起作用的就拦下送回」，
  对既有调用安全——代价：Tauri 命令要调 `_with` 版本（Task 8）。
- 偏离计划：「写配置」从「建仓库、写规则」阶段拆成独立阶段，排在检测（Task 7 起还有改名）之后——
  `respectGitignore` / `version` / `disabledGitignores` 要等检测与改名之后才定。耗时表里因此多一行「写配置」。
  `git init` 与写配置之间的窗口里目录会被识别成 `SvnSuspicious`，这一点与原来相同（原来也在 `git init`
  之后才写），只是窗口多了一次 `check-ignore`，毫秒级。
- 偏离计划 / Ruling：契约测试（`svn_backend.rs`）的 `InitReport` / `GitignoreOutcome` / `GitignoreConflict` /
  `GitignoreRule` / `RenamedGitignore` 键名在本 Task 落地，而不是等 Task 8——类型在这里诞生，编译就要求
  测试补字段；`InitPreflight` 的键名仍在 Task 8——代价：无。
- 计划的错误：PR #90 的三条既有用例（回退、预检、失败耗时）用根 `.gitignore`（`*.cs`）制造失败，新流程下
  它们停在检测阶段、错误码变成 `svn_gitignore_unconfirmed`。处理：
  - 回退用例只改期望的错误码，回退照测（检测同样在 `git init` 之后）；
  - 预检用例改名为 `an_unacknowledged_gitignore_sends_the_user_back_before_anything_is_recorded`，
    原来「预检在 git add 前拦下」的覆盖面交给新用例 `a_nested_repo_is_still_caught_by_the_precheck`
    （没有提交的嵌套 git 仓库）；
  - 失败耗时用例也改用嵌套仓库触发，原断言（预检标失败、没走到 git add、回退有耗时、只有一个失败标记）
    一条不改。
  - 脏工作副本用例改为「尊重 + 确认过」：`A.cs`（svn `M`）被 `.gitignore` 挡，尊重时不进基线。
- 安全：闸门全部在 `git init` 之后、`git add` 之前，失败走既有的唯一回退分支删掉 `.git`；尊重路径不改任何
  文件（用例断言 `.gitignore` 原样在）。未确认的冲突一律拦下，用户永远不会在没看过的情况下按某个
  `.gitignore` 少收文件。

### Task 7

- 偏离计划 / Ruling：`disable_gitignore` / `restore_gitignores` 是 `pub` 而不是 `pub(crate)`——集成测试要直接调它们
  验证撞名与复原；模块本来就是 `pub mod`——代价：多两个公开函数，没有调用方之外的副作用。
- 计划没写的坑：`a_failed_init_puts_the_renamed_gitignore_back` 在实现前就通过了——那时根本没改过名，
  `.gitignore` 自然「还在原处」，属于空过。补了前提断言：错误耗时里必须同时出现「改名」与「纳入集合预检」
  两个阶段（证明是改名之后才失败的），加上之后按预期失败，实现后通过。
- 偏离计划：「受版本控制则拦下」的检查放在改名阶段**开头**、任何改名之前，一个都不改；点名所有受控的
  文件，而不只是第一个。
- 安全（改名 / 改回）：
  - 只在用户确认过（`acknowledged` 覆盖全部冲突）、且全部不受版本控制时才改名；
  - **绝不覆盖**：先用 `create_new` 原子占住候选名，再 `rename` 到自己的占位上——Windows 上 `fs::rename`
    会替换已存在的目标，被替换的只可能是那个空占位（`disabling_never_overwrites_an_existing_disabled_file`
    钉住，`-disabled` 与 `-2` 已存在时得到 `-3`，旧内容不变）；`rename` 失败时删掉占位；
  - 改回时原位置已有文件不覆盖、点名（`restore_never_overwrites_a_gitignore_that_reappeared`）；
  - 回退阶段先改回、再删 `.git`，两步互不依赖；改回失败追加在错误正文里点名，不静默；
  - 改名后重查冒出新的起作用文件 → 全部改回、删 `.git`、`svn_gitignore_unconfirmed` 送回第 3 步
    （`a_gitignore_that_becomes_active_after_renaming_sends_the_user_back`）。

### Task 8

- 计划没写的坑：新 worktree 没有 `node_modules`，先 `pnpm install --frozen-lockfile`（不改锁文件）。
- 偏离计划：`describeGitignoreOutcome` 收 `GitignoreOutcome | undefined`——完成页拿到的 `report` 来自 IPC，
  字段缺席时（旧后端）不应崩；`describeTimings` 同样的写法。
- 偏离计划：完成页里 `.gitignore` 的说明排在体检之前——改名是用户最该知道的一件事（反目标第 1 条：
  改过名要写清楚怎么恢复）。
- 偏离计划：勾选框沿用既有的 `dlg-check` 样式（`DeleteBranchDialog` / `DeliverDialog` 已在用），没加新 CSS。
- IPC 参数名：Rust `respect_gitignore` ↔ JS `respectGitignore`、`acknowledged`、`extensions`。无编译期检查，
  Task 9 在 `pnpm tauri dev` 里实跑（`AGENTS.md` §7.3）。
- 安全：前端的「建库」禁用只是提前提示，后端闸门照旧（Task 6/7）；`acknowledged` 只取第 2 步后端返回的
  `path`，前端不自己拼。

### Task 9：真实窗口验证

做法：一次性 SVN fixture 建在 `%TEMP%\svnui2`（`unver` / `ver` / `nested` / `clean` 四份），
`pnpm tauri dev` 带 `--remote-debugging-port=9222` 与独立的 `WEBVIEW2_USER_DATA_FOLDER`，CDP 驱动
（`Runtime.evaluate` + `Page.captureScreenshot`），复用 PR #90 的脚本手法。`session.json` 事先备份，
结束后恢复并核对哈希一致（`CDFF18…99E1`）；fixture、WebView2 数据目录、验证用的 `target-verify`
全部删除；系统临时目录里没有残留的 `jkk-ignore-probe-*`。应用是从 `session.json` 恢复的旧标签页，
用 `TabBar.onCloseAll` 回到空白页再经 `EmptyState.onOpenPath` 打开 fixture（`Open Repo` 按钮会弹系统
选目录框，CDP 点不了）。

**发现的缺陷与修正（都先写了失败用例）：**

1. **改名后的文件被录进基线**（`the_renamed_gitignore_never_enters_the_baseline`，RED：基线含
   `Code/.gitignore.jkk-disabled`）。扩展名 `*` 时新名字是 W-file，而 svn 脏清单是改名之前读的、
   里面没有它 → 被当成「服务器上有」录进基线；`svn diff` 看不见未版本控制文件，体检拦不住。
   - 修正：`pre` 改成 `&mut`，改名时把新名字以 `unversioned` 补进 `pre.dirty`，录制 / 预检 / 体检
     都从这同一份清单算修正。
   - 计划的错误：spec §5 末条「改名后在 SVN 里仍是 `?`，不影响交付」只看了交付，漏了基线；
     计划的 Task 7 用例全用扩展名 `.cs`，新名字不是 W-file，测不到。
2. **嵌套 `.gitignore` 让用户在第 3 步死循环**（`a_gitignore_hidden_behind_another_is_listed_too`、
   `acknowledging_the_hidden_gitignore_too_disables_both`，RED：第 2 步只列外层；两个都确认过建库
   仍被送回）。回到第 3 步重查时外层已改回原名，内层又被挡住，第 2 步永远看不到它。
   - 修正：
     - 第 2 步改为在探针目录里建**影子树**（只放 `.gitignore`，跳过会被改名的），
       `detect_with_renames` 一轮轮模拟停用到不再冒出新的；冒出来的标 `activeAfterRename`。
     - 建库改名按轮进行：冒出来的在 `acknowledged` 里就接着改，不在才全部改回、送回第 3 步；
       上限 `MAX_RENAME_ROUNDS = 16`。
     - 「多收」候选改由 `scan_tree` 遍历得到（影子树里没有真实文件，`ls-files -o` 用不了；真仓库
       与影子树两种上下文统一一种求法）。
     - 前端：清单标「（外层的 .gitignore 停用之后才会起作用）」；勾「尊重」时这类不参与拦截判断。
   - 计划的错误：spec §5 / 计划 Task 7 的「改名后重查、冒出新的回到第 3 步」与「把新的完整清单给用户看」
     自相矛盾；spec 里「第 2 步提前算清代价高」那条我的决定作废，已在 spec §5 注明。
   - 偏离计划 / Ruling：`GitignoreConflict` 加字段 `activeAfterRename`（IPC 契约随之更新）——代价：
     前后端多一个字段。
3. **勾选框布局**：`.dlg-field` 是横向 flex，说明文字被挤到右边一列、勾选框折成两行。改成与
   `DeleteBranchDialog` 相同的 `<label className="dlg-field dlg-check">`，说明另起一块（截图核对）。

**真实窗口逐项结果**（每项截图在 scratchpad，`r1`–`r10`）：

| 场景 | 结果 |
|---|---|
| `unver`：未受控的 `Code/.gitignore` 挡 5 个已提交 `.exe` | 第 3 步列完整路径、「第 1 行 `*.exe`：挡掉 5 个」、3 个样例；勾选框默认不勾。不勾建库 → 成功；完成页「停用了 1 个」+ 原路径 → 新路径 +「想恢复就把它改回原名」；基线 = `A.cs` + 5 个 `.exe`；`git status` 只有 `?? Code/.gitignore.jkk-disabled`；配置 `version: 1` + `disabledGitignores` |
| `ver`：受控的 `.gitignore` | 标「（受 SVN 版本控制）」，红色说明，「建库」`disabled: true`；勾上后可建 → 成功；完成页「按 .gitignore 的规则，这些文件没有交给 git 管」；基线 = `Code/.gitignore` + `A.cs`；配置 `version: 2`、`respectGitignore: true`；点「关闭」主界面正常打开、「No changes」；界面上点「体检基线」→「基线已通过体检（r1）」 |
| `nested`：`Code/.gitignore` 写 `Sub/`、`Code/Sub/.gitignore` 写 `*.cs` | 修正前：送回第 3 步后清单不变（死循环）。修正后：第 3 步两个都列出，内层带「外层停用之后才会起作用」；不勾建库 → 成功，停用 2 个，基线含 `Code/Sub/X.cs` |
| `clean`：没有 `.gitignore` | 第 3 步没有提示块、没有勾选框 |
| `clean` + 第 2 步之后放进一个 `*.cs` 的 `.gitignore` | 第一次点「建库」→ `svn_gitignore_unconfirmed`，耗时表标在检查阶段，清单刷新出新文件、勾选框出现；第二次点 → 成功，基线 `Code/A.cs` |

- IPC：`svn_preflight_check` 的 `extensions`、`svn_init_hybrid` 的 `respectGitignore` / `acknowledged`
  在真实窗口里都通（上表每一项都经过它们）。
- 计划没写的坑：`pnpm tauri dev` 运行期间再跑 `cargo test` 会抢 `target/` 的构建锁、并触发窗口重编，
  验证中的 Rust 用例改用 `CARGO_TARGET_DIR=target-verify`，结束后删除。
- 计划没写的坑：`tauri dev` 又把 `src-tauri/Cargo.toml` 的行尾改了（内容无差异，
  `git diff --ignore-cr-at-eol` 为空），与 PR #90 那次相同；不提交它。
- 已知限制（不在本计划）：svn 命令行参数走系统 ANSI 代码页，ACP 1252 的机器上中文白名单根过不了
  P1–P4（见 Task 5）。

### 最终 review

- 用户明确不用 subagent，本次为作者自查（executing-plans 的「无 subagent」路径）。合并前的独立 review
  门槛（`AGENTS.md` §4.1）仍需在 PR 阶段用 `/code-review` 满足。
- 自查按 `AGENTS.md` §13 逐条过：回退（多轮改名倒序改回、占位文件清理、探针与影子树由 `Drop` 删除）、
  空输入（空白名单 / 空扩展名 / 空路径列表都提前返回，不起进程）、IPC 签名（真实窗口实跑）、跨平台路径
  （显示一律 Windows 形态；中文路径不经 svn 命令行参数）。Review Focus 五条各有用例。
  没有新的高危问题；Task 9 发现的两个缺陷已先写失败用例再修。
- Final: minor (deferred)：建库与第 2 步的检测各多一次白名单子树遍历（`scan_tree` 与 `expected_set` 各走一遍），
  扩展名不是 `*` 时还要对全部非 W-file 跑一次 `check-ignore`。大型 Unity 工程上可能多几秒，需在公司环境实测。
- Final: minor (deferred)：错误块标题前带 `[svn_gitignore_unconfirmed]` 这样的错误码前缀——既有的
  `toErrorMessage` 格式，与本功能无关。
- 提交状态：未提交（`AGENTS.md` §1.1，等用户放行）。`src-tauri/Cargo.toml` 的行尾改动不纳入提交。

### 提交与换基（2026-10-29，用户放行提交与草稿 PR）

- 计划没写的坑：开工时 PR #90 还没合并，本分支叠在它的 `c775857` 上。提交时 PR #90 已经**压缩合并**进
  `master`（`06b5ff5`），合并时的分支头是 `9802aaa`，比 `c775857` 多一个 review 提交；`master` 上还有 #91。
  直接 `git merge origin/master` 时合并基点退回 `f5baa5d`，两边都「新增」了 PR #90 的内容，冲突 10 个文件、
  自动合并区还出现重复段落——撤销了那次合并（`merge --abort`，提交已在 `026c5fd`，无损）。
- 做法：从 `origin/master` 新开 `feat/svn-init-respect-gitignore`，cherry-pick `026c5fd`（以 `c775857` 为基
  做三方合并），冲突只剩 `git_cli.rs` / `svn_include_check.rs` / `svn_init.rs`。原分支
  `worktree-svn-init-respect-gitignore` 与 `026c5fd` 原样保留。没有用 rebase。
- 移植 `9802aaa` 的改动到本分支的结构里：
  - 裁决查询遇到 fatal 路径时跳过续查：从 `svn_include_check::blame_rules` 移进
    `svn_gitignore::query_verdicts`（经 `GitCtx::run_stdin_partial`，探针与真仓库两种上下文都生效）；
    仍返回 `Result`，`Err` 只剩「进程起不来」，报告一侧照旧吞掉、「尊重」口径一侧照旧上抛。
  - `PhaseClock::new(scope)` / `settle_failure`：沿用 master；「读取工作副本信息」不再计时，「尊重」的
    `version` / `respectGitignore` 设置挪到那段未计时的代码里。
  - `dry_run_before_add`（初始化与重建共用的预检）：沿用 master，内部换成 `WScope` 口径。
  - master 带来的两条断言旧格式 `Code/.gitignore:1:*.exe` 的测试（模块内 fatal 续查、unix 符号链接）
    改成完整路径 + 「第 N 行」。
- 换基后验证：`cargo fmt --check` / `clippy --all-targets` 无告警，`cargo test` 65 个套件全部通过；
  `pnpm build` 通过，`pnpm test` 54 文件 / 807 条全绿（#91 带来 3 条）。换基只动了后端三个文件的合并与
  两条测试断言，前端与 IPC 未变，真实窗口未重跑。

### `/code-review high` 的核实与修复（2026-10-30）

review 由独立的 review agent 完成，10 条。逐条先核实再处理：

| 条 | 结论 | 处理 |
|---|---|---|
| 1 大小写不同的 `.GitIgnore` 第 2 步看不见 | **成立**（实测） | 已修 |
| 2 `is_versioned` 按小写名问 svn，受控的 `.GitIgnore` 会被改名 | **不成立**（实测） | 不改 |
| 3 `restore_gitignores` 判存在后 `rename`，有覆盖窗口 | 成立（读代码） | 已修 |
| 4 只把基线树当已跟踪，`work` 上已提交、基线里没有的文件后来被 `.gitignore` 挡住会被当成白名单外 | 成立，是 spec 没覆盖的缺口 | 用户选方案 B：口径不动，只把提示说对（见下） |
| 5 向导重新检查后清单为空时「尊重」仍生效；重新检查失败盖掉原错误 | 成立（读代码） | 已修 |
| 6 `detect_with_renames` 去重丢掉后几轮的计数 | 前半**不成立**；后半（上限静默退出）成立但只在病态结构下出现 | 不改 |
| 7 `verdict_batch` 对不上号且没有 fatal 时静默丢弃 | 成立，但没见过触发形态 | 只补 `log::warn!` |
| 8 每轮重复遍历目录树 | 成立 | 延后（本日志「最终 review」已记） |
| 9 `WScope::for_repo` 一次操作建多次 | 成立 | 延后（Task 3 有意的取舍） |
| 10 `ignore_case` / 目录遍历两份实现 | 成立 | 延后 |

- 计划没写的坑（实测，git 2.51.2.windows.1，NTFS）：`Code/.GitIgnore` 照样被 git 读到，
  `check-ignore -v` 的来源报 `Code/.gitignore`。`scan_dir` 只认名字恰好是 `.gitignore` 的文件，
  影子树里没有它 → 第 2 步查不出 → 建库时真仓库查出来 → `svn_gitignore_unconfirmed` → 重查还是
  没有，**死循环**。祖先目录那条路走的是 `root.join(".gitignore").is_file()`，本来就大小写不敏感，
  只有子树漏了。
  - 修正：名字大小写不敏感地等于 `.gitignore`、**且**按 `.gitignore` 打得开（大小写敏感的卷上打不开，
    git 也不读）时收下，记成 git 的写法 `目录/.gitignore`——与裁决来源、`acknowledged`、`disabled`
    同一个键；影子树复制与改名都按这个名字打开同一个文件。
  - RED：`a_gitignore_spelled_in_another_case_is_still_detected`（第 2 步得到 `[]`）、
    `a_hidden_gitignore_spelled_in_another_case_is_listed_too`（内层 `.GITIGNORE` 没列出）、
    `confirming_what_step_two_listed_gets_past_a_gitignore_spelled_in_another_case`（按第 2 步的清单
    确认，建库仍报 `svn_gitignore_unconfirmed`）。三条都是 `#[cfg(windows)]`：大小写敏感的卷上
    git 不读 `.GitIgnore`，这个形态不存在。
  - 已知取舍：改名失败回退时，文件改回的名字是 `.gitignore`，原来的大小写丢了。只发生在不受控的
    文件上，内容不变。
- 第 2 条核实（实测，SlikSvn 1.14.2）：受控的 `Code/.GitIgnore`，在 `Code/` 里
  `svn status --xml -v --depth empty -- .gitignore` 返回 `path=".GitIgnore"`、`item="normal"`——Windows 上
  svn 会把参数解析成磁盘上的真实大小写。`is_versioned` 判为受控，不会被改名。
- 第 3 条修正：`hard_link(停用名, 原名)` 在原名已存在时原子地失败（`AlreadyExists` → 照旧点名
  「又出现了一个新文件」），成功后再删停用名，删不掉也点名。卷不支持硬链接（FAT / exFAT）时退回原来的
  「判存在再 rename」。竞态没法写成稳定的失败用例；既有的 `restore_never_overwrites_a_gitignore_that_reappeared`
  与 `disabling_renames_..._restoring_puts_it_back` 覆盖了两条确定性路径，照样通过。
- 第 5 条修正：新增 `effectiveRespect(conflicts, respect)`（`src/engine/svnInit.ts`），清单为空时
  请求里的「尊重」恒为 `false`；重新检查包一层 `try`，失败时留着旧清单、显示建库那次的错误。
  RED：组件用例两条（下一次建库收到 `true`；界面显示的是「重新检查失败」）。
- 第 4 条为什么不顺手改：spec「参照统一是当前基线树里已有的文件」是写明的决定。改成「基线 ∪ `work` 的
  HEAD」会改动体检 / 重建 / 同步前检查 / 交付的口径。现在的表现是拒绝交付，提示里说的「多半是 `git add -f`」
  是误导，但不丢数据。
  - Ruling（用户 2026-10-30）：方案 B——口径不动，只改提示；「基线 ∪ HEAD」留作以后单独设计。spec §4 已补「已知限制」。
  - 做法：`WScope::hidden_reasons` 只对「路径判断是 W、又不在基线里」的路径问裁决，返回挡住它的 `Verdict`；
    `changed_files` 先按原口径算出不是 W-file 的改动，再拆成「被 `.gitignore` 挡住」与「真正在白名单外」。
    只有后者时错误码与文案逐字不变（`a_change_outside_the_whitelist_is_refused_not_filtered` 照旧通过）；
    有前者时报新码 `sync_out_hidden_by_gitignore`，逐个写「路径（完整 .gitignore 路径 第 N 行 `规则`）」，
    后者附在后面另起一段。前端没有按这两个码分支的地方，只显示正文，不用改。
  - 顺带：`changed_files` 里的 `WScope` 建一次、过滤与点名共用（第 9 条的一小部分）。
  - RED：`respect_delivery_names_the_gitignore_line_that_hides_a_committed_new_file`、
    `respect_delivery_still_calls_out_changes_outside_the_whitelist_separately`，均拿到 `sync_out_outside_whitelist`。
  - 安全：判定集合与改动前完全相同（`not_w` 的算法没动），只是报错的分组和文案变了；拒绝仍然拒绝，
    没有任何路径因此变得可交付。`hidden_reasons` 不尊重时直接返回空、不起进程。
- 第 6 条为什么前半不成立：不受控的一经发现就在影子树里停用，后面几轮不会再出现；会再次出现的只有受控的。
  受控的永远不改名，只有「尊重」时能建库，而「尊重」时外层也不停用——第 0 轮的计数就是「尊重」口径下的
  真实数字，后几轮的反而不对。
- 验证：`cargo fmt --check` / `clippy --all-targets` 无告警，`cargo test` 全部套件通过；`pnpm build` 通过，
  `pnpm test` 54 文件 / 809 条全绿。真实窗口未重跑：第 1 条的改动在后端扫描、没有 IPC 变化，fixture 用例
  走的是同一条 `init_preflight` / `init_hybrid_with` 路径；第 5 条由组件用例覆盖。

### 合并 master（2026-10-30，用户放行合并、提交与推送）

- master 新增 PR #93（扩展名支持中文分隔符，`validate_new_extensions`）。照本分支一贯做法用 `git merge origin/master`，
  不 rebase：已推送的历史不动，不需要 force push。
- 冲突 2 个，都是两边各加各的：
  - `src-tauri/tests/svn_config.rs`：`use` 列表两边各加一个名字，取并集（`validate_new_extensions` + `DisabledGitignore`）。
  - `tests/vitest/SvnInitWizard.test.tsx`（add/add，两边都新建了这个文件）：保留 master 的文件头注释，import 取并集，
    两组 `describe` 都留；两边都有叫 `mount` 的辅助函数、签名不同，master 的改名为 `mountForExtPreview`。
- 自动合并的 9 个文件里核对过一处：master 在 `build_config` 里加的 `validate_new_extensions` 仍在 `git init` 之前
  （`init_hybrid_with` 先调 `build_config`），第 2 步预检也走它，两处接受范围一致。
- 验证：`pnpm build` 通过，`pnpm test` 54 文件 / 816 条全绿；Rust 侧见合并提交说明。

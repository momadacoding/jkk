# 2026-09-22 Phase 2a 行尾归一化落地 —— 决策日志

> 分支 `worktree-svn-phase2a-eol`，基点 `ec911eb`（含 spec §7.3.0 的定案与本期计划）。
> 计划：`docs/superpowers/plans/2026-09-22-svn-phase2a-eol-normalization.md`
> spec：`docs/superpowers/specs/2026-09-12-svn-git-hybrid-design.md` §7.3.0
>
> 本文件不是事后总结，是 review 的输入（`AGENTS.md` §2.1）。
> **先读「计划的错误」和「偏离计划」两节再看 diff。**

## 进度总账

| Task | 内容 | 状态 | 产出 | 验证 |
|---|---|---|---|---|
| 0 | 施工前复核计划 | ✅ 完成 | 本文件 | 抓到计划的 2 处错误，见下 |
| 1 | `svn_attributes.rs` 模块 | ✅ 完成 | `svn_attributes.rs`、`tests/svn_attributes.rs`、`engine/mod.rs` | `cargo test` 544 passed（基线 537，新增 7）；变异验证已跑 |
| 2 | `init_hybrid` 建库时写规则 | ✅ 完成 | `svn_init.rs`、`tests/svn_init.rs` | `cargo test` 547 passed（新增 3）；`--test svn_init` 36 passed |
| 3 | `eol_status` 侦测（纯读） | ✅ 完成 | `svn_init.rs`、`commands/svn.rs`、`lib.rs`、`tests/svn_init.rs` | `cargo test` 551 passed（新增 4）；`commands_are_async` 变异验证已跑 |
| 4 | `migrate_eol` 原子迁移 + 回滚 | ✅ 完成 | `svn_init.rs`、`commands/svn.rs`、`lib.rs`、`tests/svn_init.rs` | `cargo test` 556 passed（新增 5）；回滚变异验证已跑 |
| 5 | 前端横幅与接线 | ✅ 完成 | `svnSync.ts`、`repoStore.ts`、`BaselineBanner.tsx`、`RepoTab.tsx`、`svnSync.test.ts` | `pnpm build` OK；`pnpm test` 365 passed（新增 11）；变异验证已跑；**真实窗口 3 个场景 2026-09-24 由用户跑通** |
| 6 | 修入独立 review 的阻断发现 | ✅ 完成 | `svn_init.rs`、`tests/svn_init.rs`、`tests/svn_attributes.rs` | `cargo test` 562 passed（新增 6）；对齐变异验证已跑 |
| 7 | 修入第二轮独立 review 的三条阻断（PR #45） | ✅ 完成 | `svn_init.rs`、`svnSync.ts`、`RepoTab.tsx`、`tests/svn_init.rs`、`svnSync.test.ts` | `cargo test` 564 passed（新增 2）；`pnpm test` 369 passed（新增 4）；变异验证已跑 |
| 收尾 | E2E 清单 + 独立 review | ✅ 完成 | `docs/superpowers/e2e/2026-09-22-svn-phase2a-eol-e2e.md` | E2E 3/3 通过；review 1 条阻断已修 |

## 全量验证

| 命令 | 结果 |
|---|---|
| `cargo fmt --check` | ✅ OK |
| `cargo clippy --all-targets` | ✅ **0 告警** |
| `cargo test` | ✅ **564 passed / 0 failed**（基线 537，新增 27） |
| `pnpm build` | ✅ OK（`tsc` + `vite build`） |
| `pnpm test` | ✅ **369 passed**（基线 354，新增 15） |
| `pnpm tauri dev` 真实窗口 | 🟡 **场景 1、4 已通过；2、3 的行为已通过、改过的文案待重看**（第二轮 review 改了横幅与确认框的文字，那是用户按下按钮前唯一能读到的东西，必须用眼睛确认。清单见 `docs/superpowers/e2e/2026-09-22-svn-phase2a-eol-e2e.md`） |

> 本仓库没有 PR 门禁 CI（`AGENTS.md` §4.1）。以上全部要本地实跑，如实填。

---

## Task 0：施工前复核计划

`superpowers:executing-plans` 的 Step 1 要求先批判性复核计划。复核对象是计划里
两处我自己标了「读了再决定，不要猜」的假设。**两处都是计划写错了**，在动代码
之前就修正，避免按错的计划施工。

### 计划的错误

#### 错误 1：新命令的属性写成了 `#[tauri::command]`，应为 `#[tauri::command(async)]`

计划 Task 3 / Task 4 的代码块里，两条新命令都写成 `#[tauri::command]`。

实测 `src-tauri/src/commands/svn.rs`：既有 10 条命令**全部**是
`#[tauri::command(async)]`，无一例外。

按计划原样写会被 `src-tauri/tests/commands_are_async.rs` 的
`every_command_is_declared_async` 判红。那条测试的理由写在文件头上，值得抄在这里
以免下一个人再踩：**Tauri 2 里没有 `async` 标记的命令在主线程上执行**，而前端
`invoke` 是由 WebView2 的自定义协议处理器在窗口过程内部同步调用的——一个同步命令
只要阻塞（等 git 子进程、等网络），窗口消息泵就停摆，界面直接冻住。仓库里有过实测
记录：`read_worktrees_cmd` 曾让启动后的窗口约 5 秒无法拖动，7 次复现 5 次。

本期两条新命令里，`migrate_eol` 内含重建基线、要跑 `svn cat` 往返，正是会阻塞的那种。

**修正**：两条命令一律写 `#[tauri::command(async)]`。

#### 错误 2：计划说要新加 `baseline_blob_bytes`，但既有的 `baseline_blob` 本来就返回字节

计划 Task 2 Step 1 写着「文件里已有的 `baseline_blob` 返回 `String`，本组测试要比
**字节**，不能复用」，于是要求新增一个 `baseline_blob_bytes`。

实测 `src-tauri/tests/svn_init.rs:708`：

```rust
/// 某个路径在 `svn/base` 树里的 blob 字节。
fn baseline_blob(wc: &str, path: &str) -> Vec<u8> { … }
```

它本来就返回 `Vec<u8>`，走的是 `run_git_bytes` + `cat-file blob`。计划里那个新
helper 是凭空多出来的重复代码。

**修正**：直接复用既有的 `baseline_blob`，不新增 `baseline_blob_bytes`。计划里
用到该名字的三处断言（Task 2 一处、Task 4 两处）一并改名。

### 偏离计划

（本 Task 无实现改动；两条修正在各自的 Task 里落地并各自记录。）

### 计划没写的坑

- `commands_are_async.rs` 是**反射式**的（扫 `src/commands/*.rs` 的源码文本），不是
  硬名单。所以新增命令**不需要**改它——计划里「读了再决定要不要改」的那个悬念，
  答案是「不用改，但属性必须写对」。这两件事容易混为一谈：不用改名单 ≠ 它不管你。

### 为什么这么改是安全的

本 Task 只新增本文件，未触碰任何代码。

---

## Task 1：`svn_attributes.rs` 模块

7 条测试全部按计划落地并通过。`cargo test` 全量 **544 passed / 0 failed**
（基线 537），`cargo fmt --check` 与 `cargo clippy --all-targets` 均零告警。

### 变异验证（已跑）

把 `EOL_RULE` 从 `"* -text"` 改成 `"* text=auto"`：

```text
rule_beats_a_gitattributes_committed_into_the_working_tree ... FAILED
rule_turns_off_eol_conversion_for_every_path               ... FAILED
其余 5 条                                                   ... ok
```

恰好是那两条依赖 git 实际裁决的断言变红，其余 5 条（备份、回滚）与规则内容
无关所以不动——变异的作用域和预期一致，不是「全红」那种无信息的结果。

### 偏离计划

- **测试里的注释措辞微调**：计划写「spec §7.3.0 取 C 不取 B 的决定性理由」，
  实际落成「取本方案而非 `core.autocrlf=false` 的决定性理由」。理由：A/B/C 是
  写 spec 时的临时代号，不进代码——半年后读这行代码的人不该被迫回去查代号表。

### 计划的错误

（本 Task 无。计划里 Task 1 的代码可直接照抄，实现与之一字不差。）

### 计划没写的坑

- **`String::from_utf8_lossy` 用在判断而非写回上，这一点必须在注释里说清。**
  用户的 `.git/info/attributes` 完全可能不是 UTF-8（GBK 注释的中文工程很常见）。
  备份走的是**原始字节** `std::fs::write(&backup, bytes)`，`from_utf8_lossy`
  只参与「要不要备份」的判断。这和 spec §16.2 末尾那条「`svn cat` 必须按字节
  接收、不能过 UTF-8 转换」是同一类坑，实现时补了注释点明。

### 为什么这么改是安全的

- **不动工作目录。** 这个模块只写 `.git/info/` 下的文件，工作树一个字节不碰。
- **不静默吃掉用户数据。** 首次接管时把用户原有的 attributes 备份成
  `attributes.bak`，且备份已存在就不再覆盖（否则第二次运行会把用户的原件
  糊成我们自己生成的内容——`rewriting_our_own_file_does_not_create_a_second_backup`
  就是钉这条的）。
- **写入是原子的。** 临时文件 → `sync_all` → `rename`，且临时文件与目标同目录
  （跨卷 rename 在 Windows 上报 `ERROR_NOT_SAME_DEVICE`）。半个规则文件比没有
  规则更难发现：`has_eol_rule` 会报 false，界面于是提示迁移，而迁移又会因为
  「规则已存在」的分支走岔。

---

## Task 2：`init_hybrid` 建库时写规则

一行实现（`build_repo` 里 `write_exclude` 之后、`record_server_tree` 之前
`write_attributes(root)?`）+ 3 条测试。`cargo test` 全量 **547 passed / 0 failed**，
`--test svn_init` 单独 36 passed（改动前 33）。fmt / clippy 零告警。

### 这一期真正的 bug 长什么样（失败测试的实测输出）

```text
a_new_repo_records_the_baseline_byte_for_byte
  assertion `left == right` failed: 基线里的字节必须与磁盘完全一致
   left: [110, 49, 10]        ← 基线 blob   "n1\n"    LF
  right: [110, 49, 13, 10]    ← 磁盘        "n1\r\n"  CRLF
```

这就是 spec §7.3.0 描述的形状，第一次以**建库路径**的形式被钉住。此前仓库里
唯一相关的测试（`recording_a_file_produces_the_same_blob_whether_it_was_dirty_or_not`）
比的是「两条录制路径彼此一致」，两条**一起**错的时候它照样绿。

### 偏离计划

1. **没有新增 `baseline_blob_bytes` / `git_status_porcelain` / `svn_status_short`
   三个 helper。** 按 Task 0 的错误 2，`baseline_blob` 本来就返回 `Vec<u8>`；
   而 `run_git` / `run_svn` 在该测试文件里早已导入，直接用即可。计划里那三个
   helper 全是多余的。
2. **`a_new_repo_is_clean_on_both_sides_right_after_init` 的 svn 侧断言改成
   「除 `.git` 外干净」**，理由见下面「计划没写的坑」。

### 计划的错误

- 计划 Task 2 Step 2 的预期写的是「两条失败」，实际**三条**都失败了。第三条
  （`a_new_repo_is_clean_on_both_sides_right_after_init`）不是实现的问题，是
  计划里那条断言本身写错了——见下。写计划时没想到 `svn status` 的常驻噪音。

### 计划没写的坑

- **`svn status` 在混合仓库里永远有一行 `?  .git`。** 实测：

  ```text
  建完库后 svn status  →  ?       .git
  ```

  初始化**一条 svn 写命令都不发**（`a_successful_init_emits_no_svn_write_command`
  钉着这条），所以没机会设 `svn:ignore`，svn 必然把 `.git` 报成未版本控制。
  把它当脏会让这条断言**永远红**，而真正要看的是白名单内有没有文件变脏。
  断言因此改成过滤掉以 `.git` 结尾的行。

  这条对 Plan 2b 的 `sync_in` 同样成立：S4 的「`svn status` 扫描后只校验 W-file
  应为 0 修改」如果按「整个输出为空」实现，会在第一次运行时就误判。

- **`a_new_repo_is_clean_on_both_sides_right_after_init` 在改动前后都绿**，不是
  失败测试。原因：`git status` 比对时**也会**跑 autocrlf 转换，所以旧设置下
  「磁盘 CRLF vs 索引 LF」经过归一后仍判为相等。它的价值是回归守卫（防索引
  对齐跟不上 blob 形状变化），不是这一期 bug 的证据——证据是上面那条字节断言。
  计划把它和另两条并列列成「预期失败」，是把守卫误当证据了。

### 为什么这么改是安全的

- **`write_attributes` 的位置有唯一正确解，且已被测试钉死。** 放在
  `record_server_tree` 之前，第一个基线就是字节形态；放在之后，建库当场就造出
  一个需要迁移的旧基线。`a_new_repo_records_the_baseline_byte_for_byte` 会在
  顺序写反时立刻变红。
- **既有 33 条 `svn_init` 测试一条没红**，其中包括本次改动影响面最大的
  `recording_a_file_produces_the_same_blob_whether_it_was_dirty_or_not`
  ——它断言干净路径（`git add`）与脏路径（`hash-object --path`）产出同一个 blob。
  2026-09-21 的探针预测「关掉转换后这条仍成立，且变成平凡成立」，实测确认。
- **建库仍然一条 svn 写命令都不发**：`a_successful_init_emits_no_svn_write_command`
  绿着，`write_attributes` 只碰 `.git/info/`。

---

## Task 3：`eol_status` 侦测（纯读）

`EolStatus` + `eol_status` + `svn_eol_status` 命令 + 4 条测试。`cargo test` 全量
**551 passed / 0 failed**（新增 4），fmt / clippy 零告警。

### Task 0 那条修正被实测坐实

把新命令按**计划原文**改回 `#[tauri::command]`（去掉 `(async)`）：

```text
test every_command_is_declared_async ... FAILED
  svn.rs:70: #[tauri::command]
```

所以那不是纸上谈兵的顾虑，是计划照抄就会红的实错。改回 `(async)` 后通过。

### 偏离计划

1. **`EolStatus` 放在 `RebuildReport` 之前**，不是计划说的「附近」。理由：
   `eol_status` 的函数体要用到 `detect_repo_kind`，和 `rebuild_baseline` 开头
   那段守卫同形，挨着放读起来是一组。
2. **测试里抽了一个 `strip_eol_rule(fx)` 辅助**。计划里三处都写成裸的
   `std::fs::remove_file(...unwrap())`；Task 4 还要再用两次，共五处。抽出来
   顺带能把「这是在模拟 Phase 1 建的库」这句话写在一个地方。
3. **没加 `git_status_porcelain` helper**，直接用既有的 `run_git`（同 Task 2）。

### 计划的错误

（除 Task 0 已记的那条属性错误外，本 Task 无新增。）

### 计划没写的坑

- **`rule_installed` 与 `needs_migration` 恒定互补，看着冗余。** 留两个字段是
  有意的：前端一个用来决定横幅挂不挂、一个用在文案里，让前端自己取反更容易
  写错。已在字段注释里说明，免得下一个人「顺手精简」掉一个。

### 为什么这么改是安全的

- **`eol_status` 不含任何写操作**，函数体只有一次 `detect_repo_kind` 和一次
  `has_eol_rule`（后者只 `read_to_string`）。`detection_writes_nothing` 连调
  两次并断言规则文件仍不存在、`git status` 输出逐字节未变。
- **不是混合仓库时报错而不是返回「不需要迁移」。** 后者会让一个普通 git 仓库
  静默地被当成「已迁移」，将来若有人拿这个字段做别的判断就会踩空。
- **命令写了 `(async)`**，不会阻塞 UI 线程——虽然这条只读、快，但规矩是一致的，
  且 `commands_are_async` 会持续钉住。

---

## Task 4：`migrate_eol` 原子迁移 + 回滚

`migrate_eol` + `svn_migrate_eol` 命令 + 5 条测试。`cargo test` 全量
**556 passed / 0 failed**（新增 5），fmt / clippy 零告警。

### 变异验证（已跑）

把回滚那一臂换成 `match Ok::<(), GitError>(()) { … }`（即失败时**不**还原规则）：

```text
a_failed_rebuild_rolls_the_rule_back              ... FAILED
a_failed_rebuild_restores_a_users_own_attributes_file ... FAILED
其余 43 条                                          ... ok
```

只有这两条回滚断言变红，其余不动——变异作用域与预期一致。

### 偏离计划

1. **测试里抽了 `obstruct(fx)` 辅助**返回被 obstruct 的路径。两条失败路径测试
   都要造同一种脏状态；顺手把「§16.1 之后 `~` 是 R1 唯一仍然阻断的脏状态」
   这句理由写在辅助函数上，而不是抄两遍。
2. **`a_failed_rebuild_restores_a_users_own_attributes_file` 没有先
   `strip_eol_rule`**，而是直接用用户内容覆盖 `.git/info/attributes`。效果一样
   （`has_eol_rule` 对 `*.psd binary\n` 报 false），但更贴近真实场景：用户本来
   就有自己的 attributes，而不是「先有我们的规则再被删掉」。

### 计划的错误

- **`GitError::new` 的第一个参数是 `&str`，不是 `String`。** 计划里回滚失败那一
  支写的是 `GitError::new(e.code, …)`，`e.code` 是 `String`，编译不过：

  ```text
  error[E0308]: expected `&str`, found `String`
  ```

  改成 `&e.code`。计划写代码块时没查 `error.rs` 的签名。

- **计划的代码块不满足 `rustfmt`。** 两处 `assert_eq!(err.code, "…", "实际：{}", err.message)`
  超宽，`cargo fmt --check` 报 diff。跑一次 `cargo fmt` 即可，但说明计划里的
  代码块**不能当作可直接粘贴的成品**——粘完要跑 fmt。

### 计划没写的坑

- **回滚失败时不能把原错误盖掉。** 用户首先要知道重建**为什么**没成；回滚的
  失败是附加信息。更关键的是：若回滚真的失败，仓库会停在「规则装了但基线是
  旧的」状态，而此后 `has_eol_rule` 报 true，界面**不会再提示迁移**——等于
  无人知晓。所以那条消息里必须明确告诉用户手工删掉
  `.git/info/attributes` 后重试。这一点计划提了一句，实现时才看清它的分量：
  它是这条路径上唯一的出口。

### 为什么这么改是安全的

- **失败路径已被两条测试逐一钉住**，且变异验证证明它们有牙齿。回滚用的是
  `write_attributes` 返回的**原始字节**，用户自己的 attributes 逐字节还回去
  （不是「删掉文件」了事）。
- **拒绝重复迁移。** 规则已装时返回 `svn_eol_already_migrated` 而不是再跑一次
  重建——重建会移动 `svn/base` 指针，旧基线只能从 reflog 找回来，没有理由时
  不该动它。
- **拦住之后磁盘原样。** `a_failed_rebuild_rolls_the_rule_back` 末尾断言那个被
  obstruct 的路径仍然是目录——本应用一律不替用户动手（spec §6.5 P1）。
- **`migrate_eol` 自己不实现重建**，直接复用 `rebuild_baseline`。R1–R5 那段是
  最不能写错的代码，抄一遍等于多一处会漂移的实现。

---

## Task 5：前端接线

`svnSync.ts`（类型 + 两个判据函数 + `needsStandaloneCheck` 改签名）、`repoStore.ts`
（两条 invoke + `svnEol` 状态 + `refreshSvnEol`）、`BaselineBanner.tsx`、
`RepoTab.tsx` + 11 条 vitest。`pnpm build` OK，`pnpm test` **365 passed**。

### 真实窗口验证（2026-09-24 由用户实跑，3/3 通过）

`AGENTS.md` §7.3 对新增 Tauri 命令是硬要求：`snake_case` ↔ `camelCase` 这层转换
没有编译期检查，556 + 365 条自动化测试一条都抓不到。三个场景——新建库不出现迁移
横幅、删掉规则重开出现横幅且另外两条让位、点「重建基线…」后横幅消失且两侧干净
——全部通过，两条新命令的参数名转换因此得到实测确认。

### 变异验证（已跑）

删掉 `describeEolMigration` 里 `pinnedReset` 让位那一行：

```text
× describeEolMigration > pinnedReset 时让位——那种状态下重建基线是错的
× 迁移横幅与失同步横幅互斥 > pinnedReset 不让位——那条横幅指的是另一件必须先做的事
其余 363 条 ... ok
```

### 偏离计划

1. **`refreshSvnEol` 放在 `refreshSvnSync` 之后、`refreshSvnLog` 之前，并且
   `await`。** 计划只说「在打开仓库那条路径里顺带读一次」。实现时发现顺序有
   讲究：迁移横幅会让失同步横幅和常驻体检入口**都**让位，所以它必须在第一帧
   之前读到，否则界面会先挂出一条随后又被替换掉的横幅。
2. **多加了一条测试**：`行尾状态还没读到时按「不需要迁移」处理，常驻入口照常`。
   计划没有这条，但 `null` 是打开仓库那一瞬间的真实状态，不钉住的话很容易被
   改成「读不到就藏起来」，表现为每次打开仓库体检入口闪一下。
3. **`BaselineBanner` 里迁移横幅的按钮不走 `a.id` 分发**，直接绑 `onMigrateEol`。
   那个 model 只有一个 action，走分发等于为一个分支写一套 switch。
4. **`RepoTab` 的确认框文案不复用 `handleRebuildBaseline` 那一份。** 计划说
   「和重建基线同等分量」，实现时意识到两者要回答的问题不同：重建那边说的是
   「为什么要重建」，迁移这边要回答「**为什么突然要我重建**」——用户没做任何
   事，横幅自己冒出来的。这与 Phase 1c「文案只应有一份」那条不冲突：那条讲的
   是同一句话不要抄两遍，不是不同的话要合并成一句。

### 计划的错误

- **计划漏了 `pnpm install`。** 新建的 worktree 里没有 `node_modules`，
  `pnpm test` 直接报 `'vitest' is not recognized`。而且 `pnpm install` 走网络，
  在这套环境里**必须用 PowerShell 跑**（Bash 工具没有网络）。任何在新 worktree
  里执行的计划都该把这一步写进去。

### 计划没写的坑

- **`needsStandaloneCheck` 的既有调用点有 7 处**（6 处在 vitest、1 处在
  `BaselineBanner.tsx`）。按计划「不给默认值」办，`tsc` 只逼出了组件那一处
  ——`tests/vitest/**` 不在 `tsconfig.json` 的 `include` 里（`AGENTS.md` §5.1
  已记的覆盖盲区）。**测试里那 6 处是靠 `pnpm test` 跑出来才发现的**，不是
  编译器。这条对 review 有直接影响：不能因为 `pnpm build` 绿就认为签名改动
  的调用点都跟上了。
- 那 6 处统一传 `eolDone`（规则已装）而不是 `null`，因为它们测的是**原来那条
  判据**，传 `eolDone` 才能保证迁移逻辑不介入。传 `null` 效果相同但语义模糊。

### 为什么这么改是安全的

- **判据全部下沉到 `src/engine/svnSync.ts`**，组件只渲染，不自己判断哪种情况给
  哪条出口（`AGENTS.md` §7.2）。三条互斥规则各有 vitest 覆盖。
- **`pinnedReset` 的例外有测试钉住，且方向是两条**：迁移横幅不出现、失同步横幅
  照常出现。只测一条的话，一个「pinnedReset 下什么都不挂」的实现也能骗过去。
- **`refreshSvnEol` 失败时写回 `null` 而不是一个「不需要迁移」的值。** 两者在
  界面上都不挂横幅，但把读失败当成「已经迁移过」会让这个仓库此后再也不提示。
  同 `refreshSvnAudit` 的 `keepOnError=false` 分支的理由。
- **`samePath` 守卫**：读的过程中用户可能已经切到别的仓库，不比路径的话这次结果
  会写进另一个仓库的横幅里。
- **切到非 SVN 仓库时显式清空 `svnEol`**，与既有的 `svnSync` / `svnAudit` 同处
  一个 `setState`——不清的话上一个 SVN 仓库的迁移横幅会挂在下一个普通 git
  仓库上（`refreshPinned` 的注释里记过同一个坑）。

---

## Task 6：修入独立 review 的阻断发现（2026-09-24）

`AGENTS.md` §4.1 的独立 review 门槛，由一个全新上下文的 agent 按 §13 rubric 执行。
**1 条阻断 + 6 条非阻断。** 阻断那条由本会话独立复现后确认成立并修掉。

### 阻断问题：迁移只换了三样东西里的一样

Review 的原话是「迁移对**真实形态**的 Phase 1 仓库并没有把仓库对齐」。本会话
独立复现（新写 `migrating_a_faithful_phase1_repo_leaves_nothing_dirty`）：

```text
迁移后 git status  →  M Code/Nat.cs
```

**根因。** `rebuild_baseline` 按设计全程不动 HEAD 和真实索引（R1–R5 的核心约束），
所以它只把 `svn/base` 换成新形状。而 Phase 1 建库时 `build_repo` 是靠一句
`read-tree` 把**真实索引**也对齐的，工作分支 tip 又恰好指向那个基线 commit
——真实的 Phase 1 仓库里，**基线树、工作分支 tip、真实索引三样同时是旧形状**。
只换第一样，留下的正是本期声称要消灭的那个潜伏故障。

**为什么原来的测试没抓到（rubric #9，虚假信心）。** 原 fixture 只做
「删规则 + `rebuild_baseline`」，而它的真实索引和 work tip 是在**规则已装**的
情况下建的（新形状），于是迁移之后一切恰好对得上。测试名说「重录基线」，
测的却不是 Phase 1 仓库的真实形态。新增 `rewind_to_phase1` 把三样都退回去。

**修法。** 迁移成功后把工作分支与真实索引一起搬到新基线上
（`update-ref` 带旧值守卫 + `read-tree`，**不带 `-u`**，磁盘全程不碰）。

### 被实测推翻的方案：`git add --renormalize`

工作分支上有自己的提交时（情形 B），移 ref 修不了那些 commit 里的旧形状 blob。
用户最初选了「加一笔归一化提交」。**探针把这个方案推翻了：**

```text
Code/Nat.cs    旧 blob=LF  磁盘=CRLF  →  cat-file --filters 给 CRLF  ✅
Code/Plain.cs  旧 blob=LF  磁盘=LF    →  cat-file --filters 给 CRLF  ❌
```

`--renormalize`（及其背后的 `cat-file --filters`）给出的是「git 会往磁盘写什么」，
`autocrlf=true` 下**所有文本文件都是 CRLF**；而磁盘的真实形态由 SVN 决定。
一个 SVN 存 LF 的文件会被它改成 CRLF，**凭空造出一批新的假阳性**。

顺带验证了它的另一半确实没问题：从 HEAD 树构造不会卷进用户的 WIP
（探针里磁盘的 `WIP-NOT-COMMITTED` 既没进提交也没被抹掉）。但取值来源错了，
方案整体不成立。

要做对就得逐路径定目标形态，而目标形态只能靠数 `\r\n` 猜——猜错就是改坏用户
已有的提交。**改选：本期在动任何东西之前拒绝**，理由与出路写进错误消息。
「把一个分支重放到新基线上」本来就是 `sync_in` S9 的职责，那里有 rebase 和
完整的冲突解决流程兜底。

### 计划的错误（本轮新增）

- **「落后于基线」被我误报成「有本地提交」。** 第一版 `check_can_align` 只算
  `base..HEAD`，于是 HEAD 是 base 祖先时报出「有 **0** 笔本地提交」。而
  **落后于基线是 Phase 1 的合法常态**——spec §16 明写「重建基线的代价是工作分支
  不会自动跟进」，界面上那颗「重建基线」按钮每按一次就会造出这个状态。两种成因
  相反，消息必须分开说，否则用户按着提示去找不存在的本地提交。已补
  `migration_says_so_when_the_branch_merely_lags_behind_the_baseline` 钉住。

### 非阻断项的处理

| review 发现 | 处理 |
|---|---|
| 「备份走原始字节」没有测试（变异后 52 条全绿） | **已修**：新增 `a_non_utf8_attributes_file_is_backed_up_byte_for_byte`，用 GBK 字节断言 `.bak` 逐字节相等 |
| 两条 EOL 测试依赖本机全局 `core.autocrlf=true`，别的机器上会**静默平凡通过** | **已修**：显式断言这条前提。`init_hybrid` 自己跑 `git init`，没法在建库前设仓库级配置，所以只能把前提断出来而不是设出来 |
| `write_attributes` 非 rename 失败路径会留下 `.tmp` | 未修。与 `write_exclude` 同病，属既有约定，单独处理 |
| 回滚后 `attributes.bak` 留在原地 | 未修。留着不伤正确性，且它是用户原件的唯一副本，删它风险更大 |
| 只有注释的 attributes 会被无备份覆盖；用户原有规则失效后 UI 未提示 | 未修，记为已知限制。建议在后续把「会接管 `.git/info/attributes`」写进确认框 |
| `has_eol_rule` 对 BOM / 空文件 / 只有 header 的边界无测试 | 未修。三种行为均已确认正确（分别 false/false/false），补测试的收益低于本期收尾成本 |

### 为什么这么改是安全的

- **拒绝是全或全无的**：`check_can_align` 在 `write_attributes` **之前**跑，四条
  判据全是纯读。三条拒绝测试都断言了「规则文件不许出现、基线不许动、工作分支
  不许动、暂存内容原样保留」。
- **对齐不会丢东西**：前置保证分支 tip == 旧基线且索引 == 旧基线树，所以
  `update-ref`（带旧值守卫）不会丢提交，`read-tree` 不会丢暂存。
- **对齐失败会把基线也退回去**，不留「基线新、分支旧」的半迁移状态；退回也失败时
  把原错误保留并给出手工命令。
- **变异验证已跑**：去掉 `align_branch` 之后，恰好那两条对齐断言变红。

---

## Task 7：修入第二轮独立 review 的三条阻断（2026-09-25，PR #45）

Task 6 的修复本身带进了一个新缺口，另外两条是本期一直存在的。三条都由实测坐实，
不是纸面推断。

### 偏离计划

计划里没有这一节——它是 PR #45 的 review 产物。

### 计划的错误

不适用（计划写于这些问题出现之前）。

### review 的三条阻断与处理

| # | 发现 | 处理 |
|---|---|---|
| 1 | `read-tree` 失败时工作分支没回滚 | **已修**：`align_branch` 换序 + 补回滚 |
| 2 | 重建窗口内新产生的暂存会被静默冲掉 | **已修**：覆盖索引前重查一次，缩窗 |
| 3 | 确认框对工作分支变更的说明与实现相反 | **已修**：两处文案改写并共用同一份常量，加断言钉住 |
| 4 | 迁移静默停用用户已有的 attributes 规则（P2） | **已披露**：写进确认框文案，并由断言钉住 |

### 计划没写的坑

**一、`update-ref` 成功、`read-tree` 失败，留下的状态会让仓库永久迁不动。**

原先的顺序是先移分支后 `read-tree`。fixture 复现（`.git/index.lock` 存在即可触发，
任何一个并发 git 进程或崩溃残留都会造出它）：

```text
update-ref ok                        → work = 新基线
read-tree  exit=128                  → 索引还是旧形状
失败路径只退 svn/base                → base = 旧基线

git status --short                   →  M  f.txt      ← 用户从没暂存过
git rev-list --count base..HEAD      →  1
```

最后那个 `1` 才是真正致命的：下一次点迁移，`check_can_align` 会把它当成
「分支上有 1 笔本地提交」而拒绝——**这个仓库从此在界面里迁不动了**，只能手工
`git update-ref` 救。也就是说，一次普通的锁冲突会把可恢复的失败变成不可恢复的死角。

换序之后：`read-tree` 是更容易失败的那一步，而它失败时一个 ref 都还没动，天然
全或全无；后面的 `update-ref` 自带旧值守卫、本身原子，万一失败就把索引 `read-tree`
回旧基线。每一步之前的状态都是可恢复的。

**二、前置检查与真正覆盖索引之间隔着一整趟 SVN 往返。**

`check_can_align` 在开头查一次索引，之后 `rebuild_baseline` 要跑完整的 SVN 网络
往返，最后才 `read-tree`。这中间 `git add` 进来的东西会被冲掉。实测确认影响面：

```text
git add brandnew.txt   →  A  brandnew.txt
git read-tree <new>    →  ?? brandnew.txt     ← 暂存状态没了
cat brandnew.txt       →  new content         ← 磁盘内容还在
```

所以准确说法是**丢暂存状态，不丢文件内容**（`read-tree` 不带 `-u`，磁盘全程不碰）。
另有一处不对称值得记下：同一窗口内如果用户**提交**了，`update-ref` 的旧值守卫会
挡住并安全失败——ref 侧一直有守卫，index 侧一直没有。

本期只做缩窗（覆盖前重查一次），不做锁。真正的解法是仓库级操作锁，那是 Plan 2b
`sync_in` 的既定范围，在这里另造一套会和它打架。

**三、注入测试必须用白名单内已跟踪的文件，否则平凡通过。**

用 `reference-transaction` 钩子在 `rebuild_baseline` 的 R4（`update-ref` 基线指针）
那一刻注入 `git add`，时序正好落在窗口正中，比任何「先 add 再调用」的模拟都真实。

但第一版钩子新建了一个 `INJECTED.txt`——Phase 1 的 `.git/info/exclude` 是**白名单
制**，这个文件被 git 直接忽略，`git add` 什么也没做，`git status` 一片空白，测试
平凡通过（`AGENTS.md` §13 rubric #9 的又一个实例，这次在自己写的测试里）。改成
改写已跟踪的 `Code/Nat.cs` 才真正构造出那个竞态。

**四、后端行为改了，前端的承诺没跟着改。**

Task 6 把迁移从「只重录基线」改成「基线、工作分支、真实索引一起搬」，但横幅和
确认框仍写着「不会推进你的工作分支」——在用户按下按钮之前对他说了反话。E2E 清单
里甚至同时写着「工作分支没有被动过」和「工作分支必须跟着新基线一起前移」，而那条
场景已经被勾成通过：**矛盾摆在眼前也没被读出来**。

这暴露的是结构问题，不是疏忽：同一个机械后果在 `svnSync.ts` 和 `RepoTab.tsx` 各写
了一遍。现在「会发生什么」这一段收成 `MIGRATE_EOL_WARNING` 一个常量，两处共用；
标题和正文仍各写各的（它们回答的是不同问题，横幅说「这是什么」，确认框说「你按下
去会发生什么」）。另加一组断言钉住关键句，下一个改 `align_branch` 的人会在那里被
拦下。

### 变异验证（已跑）

| 变异 | 结果 |
|---|---|
| 把 `align_branch` 换回「先 `update-ref` 后 `read-tree`」 | 只有 `migration_moves_nothing_when_the_index_cannot_be_written` 变红 |
| 去掉覆盖索引前的重查 | 只有 `migration_refuses_when_something_gets_staged_during_the_rebuild` 变红 |
| 把横幅文案改回「也不会推进你的工作分支」 | 恰好两条文案断言变红（「不得再声称」「要说明会前移」），其余 367 条全绿 |

前两条是在写实现之前先让新测试对着旧实现跑出来的红，不是事后人为制造的。

### 为什么这么改是安全的

- **换序不改变成功路径的最终状态**：成功时索引和分支仍然都落在新基线上，
  `migrating_a_faithful_phase1_repo_leaves_nothing_dirty` 与
  `migration_installs_the_rule_and_rerecords_the_baseline` 均未改动且仍通过。
- **每一步之前的状态都可恢复**：`read-tree` 失败 ⇒ 什么都没动；`update-ref` 失败
  ⇒ 索引退回旧基线；两者失败都会把 `svn/base` 和 `attributes` 一并退回。
- **两个回滚都不再吞错误**。原先 `restore_attributes` 的失败被 `let _` 吞掉，而
  被吞掉的恰好是用户此后无从察觉的那一种——界面会因为 `has_eol_rule` 报 `true`
  而不再提示迁移，等于无人知晓。现在两个回滚的失败都追加进错误消息并给出手工命令。
- **缩窗不是消除竞态**，代码注释和本节都写明了这一点，不给下一个人「这里已经安全了」
  的错觉。
- **文案改动由断言覆盖**，不依赖人记得同步。

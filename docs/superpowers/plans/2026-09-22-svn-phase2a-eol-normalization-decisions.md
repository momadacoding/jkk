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
| 4 | `migrate_eol` 原子迁移 + 回滚 | ⬜ 未开始 | | |
| 5 | 前端横幅与接线 | ⬜ 未开始 | | |
| 收尾 | E2E 清单 + 独立 review | ⬜ 未开始 | | |

## 全量验证

| 命令 | 结果 |
|---|---|
| `cargo fmt --check` | 未跑 |
| `cargo clippy --all-targets` | 未跑 |
| `cargo test` | 未跑（基线 537 passed） |
| `pnpm build` | 未跑 |
| `pnpm test` | 未跑（基线 354 passed） |
| `pnpm tauri dev` 真实窗口 | 未跑 |

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

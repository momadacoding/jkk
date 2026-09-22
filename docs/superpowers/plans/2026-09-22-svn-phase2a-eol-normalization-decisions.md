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
| 2 | `init_hybrid` 建库时写规则 | ⬜ 未开始 | | |
| 3 | `eol_status` 侦测（纯读） | ⬜ 未开始 | | |
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

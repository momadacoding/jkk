# SVN Phase 1a 决策日志

计划：`docs/superpowers/plans/2026-09-13-svn-phase1a-foundation.md`
规格：`docs/superpowers/specs/2026-09-12-svn-git-hybrid-design.md`（第四轮 review 修订版）
分支：`worktree-svn-phase1a`（worktree：`.claude/worktrees/svn-phase1a`）
基点：`02f8d45`

> 本文件按 `AGENTS.md` §2.1 建立，**在动第一个 Task 之前**。
> §2 的 checkpoint 只活在对话里，上下文压缩或会话中断就没了；在用户放行 commit
> 之前分支也无法靠 `git log` 恢复进度。**这里是唯一的状态来源。**

## 进度总账

**重构前测试基线**（2026-09-13，本 worktree 内实测）：`cargo test` **226 passed /
0 failed**，22 个测试二进制。Task 9 拿它对账。

> Phase 0 收尾时记的是 216 / 21。差的 10 个来自其后合并的 discard 加固分支
> （`engine::ops` 的空仓库与 ref 损坏回归测试）。对得上，不是漂移。

| Task | 状态 | 产出文件 | 验证结果 |
|---|---|---|---|
| 0 开工自检与基线 | ✅ 完成 | 本文件 | 自检全过；基线 226 passed / 0 failed / 22 个二进制 |
| 1 svn CLI 封装与可用性探测 | ✅ 完成 | `engine/svn_cli.rs`、`engine/mod.rs`、`tests/svn_cli.rs` | fmt ✓ clippy 零告警 ✓ `--test svn_cli` 4 passed ✓ `cargo test` **230 passed / 0 failed**（= 226 + 4） |
| 2 `--xml` 解析（roxmltree） | ⬜ 未开始 | | |
| 3 配置文件读取与校验 | ⬜ 未开始 | | |
| 4 嵌套 WC 枚举与 P4 | ⬜ 未开始 | | |
| 5 exclude 规则生成 | ⬜ 未开始 | | |
| 6 纳入集合核对 | ⬜ 未开始 | | |
| 7 仓库识别 / capabilities / backend_for 分流 | ⬜ 未开始 | | |
| 8 前端按能力隐藏入口 | ⬜ 未开始 | | |
| 9 收尾验证 | ⬜ 未开始 | | |

### 开工自检（`AGENTS.md` §4.2）

```text
- 当前分支 / worktree：worktree-svn-phase1a @ .claude/worktrees/svn-phase1a
- 当前分支是否已合并进 master：否（新开分支）
- 是否允许在当前位置修改：是（已离开 master 与主工作区）
- 目标 worktree：已在动第一行代码之前创建并用 EnterWorktree 切入
- pwd 实测：/c/Users/liuhao/workspace/dev/Dev/mygitclient/.claude/worktrees/svn-phase1a ✅
- 是否存在未提交改动：0 条
- 基点 commit：02f8d45（= origin/master，含本计划与修订后的 spec）✅
```

`.claude/worktrees/` 已在 `.gitignore` 第 2 行（Phase 0 时加的），不需要再处理。

---

## 决策条目

> 四类缺一不可（`AGENTS.md` §2.1）：**偏离计划 / 计划的错误 / 计划没写的坑 /
> 为什么这么改是安全的**。

### 全局

#### 偏离计划

（暂无）

#### 计划的错误

（暂无）

#### 计划没写的坑

- **`EnterWorktree` 的 baseRef 默认是 `fresh`（`origin/<默认分支>`），不是本地 HEAD。**
  这条 Phase 0 已经记过一次，本期沿用同样的处理：切 worktree **之前**先把 spec 修订
  （`150ddad`）与本计划（`02f8d45`）push 到 `origin/master`，走 `AGENTS.md` §4.1 的
  纯文档例外。切入后已实测基点确实是 `02f8d45`，计划与修订后的 spec 都在。

#### 为什么这么改是安全的

- 本期（Phase 1a）**不发出任何 SVN 写操作**。`svn add` / `commit` / `update` /
  `revert` / `delete` / `propset` 一个都不进 `src-tauri/src/`，Task 9 的 Step 9.3
  用 grep 做机械断言。fixture 构造里有这些命令，但那在 `tests/` 下、作用于
  `TempDir` 里自建的一次性仓库。
- 普通 git 仓库的行为**零变化**：`detect_repo_kind` 在没有 `.svn` 时立刻返回
  `Git`，`backend_for` 走的还是 Phase 0 那条分支；`GitBackend::capabilities()`
  返回全 true，前端拿到的与改动前一致。
- 新增的两个 Tauri 命令（`read_capabilities` / `probe_svn_environment`）都是只读的。
- 所有测试在 `TempDir` 里自建 fixture，跑完即删，不碰本仓库、用户仓库或任何固定
  路径（`AGENTS.md` §8 与开篇的最高优先级约束）。

### Task 0：开工自检与基线

#### 计划的错误

- **Step 0.5 写的 `cargo test 2>&1 | tail -40` 量不出基线。** 本仓库有 22 个测试
  二进制，每个各打一行 `test result:`，分散在整个输出里；`tail -N` 只能看到最后
  一两行。我第一次照着跑，拿到「20 passed / 2 个二进制」这个**看起来像结果的
  错数字**，差点当成基线。
  **正确做法**：重定向到文件再 grep 汇总行。Task 9 的 Step 9.1 同理，已在计划里
  就地改掉。

#### 计划没写的坑

- **在 `cargo test` 运行期间创建新测试文件会污染那次运行。** 我在基线跑到一半时
  写了 `tests/svn_cli.rs`。这次侥幸没中（cargo 已经枚举完目标，输出里 `svn_cli`
  出现 0 次），但第二次跑就编译它并以 `unresolved import` 失败了。
  **量基线必须在新测试文件还不存在时做**——我最后是把文件临时挪走重跑才拿到干净
  数字的。

- **满并行的 `cargo test` 在本机会打爆提交内存，表现成一串看起来像代码缺陷的错误。**
  实测这一串是同一个根因：

  ```text
  memory allocation of 1458192 bytes failed
  error: linking with `link.exe` failed: exit code: 0xc000012d   ← STATUS_COMMITMENT_LIMIT
  error: crate `windows_sys` required to be available in rlib format, but was not found in this form
  ```

  Phase 0 把最后那条记成「冷编译竞态，重跑即好」。**那只说对了一半**：重跑如果
  仍然满并行，仍会失败——根因是内存不足，不是竞态。`cargo test -j 2` 一次过
  （23 个二进制、230 passed）。按 `AGENTS.md` §5.3，这类失败一律先判环境。

- **`cargo fmt` 会拆掉链式调用。** `hidden_command("svn").args([...]).output()`
  写成一行过不了 `cargo fmt --check`，它要拆成三行 + 独立的 `{`。先写后 fmt 即可，
  但别把 `cargo fmt --check` 的失败当成代码问题。

- **`cargo fmt --check 2>&1 | head -20; echo $?` 打印的是 `head` 的退出码，恒为 0。**
  要判 fmt 是否通过，命令末尾不能接管道。

#### 为什么这么改是安全的

- Task 0 只读不写：自检、量基线、建本文件。唯一的写操作是创建
  `docs/` 下的本文件与 `tests/svn_cli.rs`，都不影响现有行为。

### Task 1：svn CLI 封装与可用性探测

#### 偏离计划

- 计划里 `run_svn` / `run_svn_code` 各自内联拼 `--non-interactive`，实现抽成了
  共用的 `with_global()`。纯去重，行为一致。

#### 计划没写的坑

- 无（本 Task 的实现与计划给的代码一致）。

#### 已知覆盖盲区（**必须如实记，不得当成测过**）

- **`SvnAvailability::StalePath` 与 `NotInstalled` 两个分支没有测试覆盖。**
  开发机装了 SlikSvn，`probe_svn()` 永远走 `Available`，构造不出另外两条路径而
  不去改 PATH（改 PATH 会影响整台机器上并发的其它会话，不可接受）。
  连带 `persistent_path_has_svn()` / `dir_has_svn_exe()` 也从未被执行过。
  已实测的只有它们依赖的那条外部命令本身：`reg query "HKLM\…\Environment" /v Path`
  退出 0 且输出含 SlikSvn 路径（2026-09-13，PowerShell）。
  **这两条分支要在 Task 9 的真实窗口验证里补一次人工核对**，或者接受它是盲区并
  写进 PR 描述——不能因为 `cargo test` 全绿就认为 §17.1 实现对了。

#### 为什么这么改是安全的

- 本 Task **只新增代码，不修改任何既有路径**。`engine/mod.rs` 只多一行
  `pub mod svn_cli;`，没有任何现有模块引用它，所以既有 226 个测试的行为
  结构上不可能改变——实测也确实是 226 → 230（只多出本 Task 新增的 4 个）。
- `run_svn` / `run_svn_code` 至今**没有任何调用方**，不会发出 svn 命令。
- `probe_svn` 只跑 `svn --version` 与 `reg query`，两个都是只读。

---

## 计划外的实测记录

写计划期间在临时 fixture 上核对过的事实，实施时**不要再猜**：

| 事实 | 实测结果 | 影响 |
|---|---|---|
| `--non-interactive` 的作用域 | **全局选项**，`info`/`status`/`log`/`diff`/`update`/`commit` 六个子命令全部接受 | 可以在 `run_svn` 内部统一注入 |
| `svn info --ignore-externals` | `Subcommand 'info' doesn't accept option '--ignore-externals'`，exit 1 | 绝不能「一律带」，见 spec §4.4 的子命令表 |
| `svn --version --quiet` | 输出 `1.14.2-SlikSvn`，exit 0 | 最便宜的存活探测 |
| `reg query "HKLM\...\Environment" /v Path` | exit 0，含 SlikSvn | §17.1 的持久 PATH 比对可行，不需要注册表 crate |
| Git Bash 下跑 `reg query ... /v Path` | `ERROR: Invalid syntax` | MSYS 把 `/v` 改写成路径。**要手工核对请走 PowerShell**，Rust 的 `std::process::Command` 不受影响 |
| roxmltree 版本与依赖 | 0.21.1，MIT/Apache-2.0，`rust-version 1.60`，依赖 `memchr`（已在树里） | 与 `Cargo.toml` 声明的 `rust-version = "1.77.2"` 兼容 |
| `#[allow(lint, reason = "…")]` | 需要 Rust 1.81 | 本机 toolchain 是 1.95 能编过，但 `Cargo.toml` 声明的 MSRV 是 **1.77.2**，用它等于悄悄抬高 MSRV。**用裸 `#[allow(dead_code)]` + 中文注释** |

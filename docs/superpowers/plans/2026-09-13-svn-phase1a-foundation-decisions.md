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
| 2 `--xml` 解析（roxmltree） | ✅ 完成 | `engine/svn_xml.rs`、`engine/mod.rs`、`Cargo.toml`、`tests/svn_xml.rs` | fmt ✓ clippy 零告警 ✓ `--test svn_xml` **8 passed**（计划写的是 6，对齐真实输出后加了 2）✓ `cargo test` **238 passed / 0 failed**（= 230 + 8） |
| 3 配置文件读取与校验 | ✅ 完成 | `engine/svn_config.rs`、`engine/mod.rs`、`tests/svn_config.rs` | fmt ✓ clippy 零告警 ✓ `--test svn_config` **14 passed**（计划写的是 8）✓ `cargo test` **252 passed / 0 failed**（= 238 + 14） |
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

### Task 2：`--xml` 输出解析

#### 偏离计划

- **测试从 6 个加到 8 个。** 计划的 Step 2.6 只要求「人工核对真实输出的结构，
  对不上就改测试常量」。核对后发现两处常量缺东西，就地补成了断言（见下）。

#### 计划的错误

- 计划给的 `INFO_XML` 常量**漏了 `<entry>` 底下的 `<commit revision>` 子节点**。
  真实 `svn info --xml` 有它，而且它与 entry 自身的 `revision` 属性是两回事
  （前者是「最后被改动于哪个 revision」）。按计划那个常量测，`parse_info` 取哪个
  都能通过——**等于没测到这个取错点**，而它与 §6.6.4 陷阱二是同一类缺陷。
  已补：常量里 entry `revision="9"` 与 commit `revision="4"` 故意不同，
  新增 `info_takes_the_entry_revision_not_the_commit_child` 钉住。

- 计划给的 `STATUS_XML` 里 external 根写的是 `path="Vendor"`，**没有路径分隔符**，
  于是「Windows 上 svn 返回反斜杠」这件事一点都没覆盖到。已改成 `Code\Vendor`
  并新增 `external_root_paths_come_back_with_windows_separators_verbatim`。

#### 计划没写的坑（2026-09-13 真实 svn 输出实测）

在临时 fixture（`svnadmin create` + checkout，跑完即删）上 dump 了两条命令的原始
输出，与计划的常量逐项对照：

| 观察 | 结果 |
|---|---|
| 行尾 | **CRLF**（`od -c` 见 `\r \n`），与 spec §6.6.4 陷阱三一致 |
| 属性排列 | 逐行，`<entry` 与 `path=` 不在同一行 → 行内正则确实会扫空 |
| `svn info` 的 entry 属性顺序 | 实际是 `kind, path, revision`，不是计划写的 `path, revision, kind`。DOM 解析不受影响，但常量已改成真实顺序 |
| `svn info` 的 `<wc-info>` | 除 `<wcroot-abspath>` 外还有 `<schedule>` / `<depth>`。常量已补 |
| `svn info` 的 `<entry>` 直接子节点 | **有 `<commit revision>`**（计划漏了，见上） |
| `svn status -v --xml` 的 `path` 属性 | Windows 上是**反斜杠**（`Code\A.cs`） |
| 目录节点本身 | 也是一条 `<entry>` 且带 revision，所以 `nodeRevs` 会含目录的 revision——这是对的，spec §6.6.2 说的就是「每个已版本控制**节点**」 |

**分隔符规范化放在消费方，不放在解析层。** 解析层的职责是如实反映 svn 说了什么；
`engine::svn_wc`（Task 4）要拿 external 路径和正斜杠形式的白名单比对，不转换就
永远比不中，会把嵌套 external 反复塞回待处理队列。这个约定现在有断言兜着。

#### 为什么这么改是安全的

- 本 Task 仍然**只新增代码**，`svn_xml` 没有任何现有模块引用，既有测试 230 → 238
  只多出本 Task 的 8 个。
- 引入的 `roxmltree` 0.21.1 是**只读 DOM**，不执行任何外部内容；MIT/Apache-2.0，
  `rust-version 1.60` 低于本 crate 声明的 1.77.2，依赖只有已在树里的 `memchr`。
- 解析失败一律返回 `Err`，**绝不降级成空集**——空集会让「nodeRevs 单值」这条
  不变量以看起来通过的方式被绕过，已由
  `parse_node_revs_on_garbage_errors_instead_of_returning_empty` 钉住。

### Task 3：配置文件读取与校验

#### 偏离计划

- **测试从 8 个加到 14 个**，多出来的 6 条都是计划漏掉的路径：
  `read_config` 的四条（文件不存在报 `not_configured`、坏 JSON 报 `svn_config`、
  读盘路径也要规范化后再校验、好文件能读出规范化后的白名单）、
  反斜杠盘符 `C:\Windows`、尾斜杠保留。

- **`validate` 对 whitelist 与 extensions 分别判空，不是「两者皆空才拒」。**
  spec §6.2.1 的原话是「两者皆空 → 阻断」，但任一为空同样不可用：空白名单让每一步的
  空参数保护触发，空扩展名让 W-file 恒为空集（§6.3.2），两种都表现为「功能装上了
  但什么都管不到」。实现按更严的口径走，理由写在代码注释里。

#### 计划的错误

- 计划的 `normalize_whitelist_entry` **没有覆盖反斜杠写法的盘符**。
  `C:\Windows` 只有先转成正斜杠才看得出第二个字符是 `:`；计划里规范化与校验的
  先后顺序没写明，照字面实现很容易把校验放在 `replace('\\', "/")` 之前，于是
  反斜杠写法从盘符检查底下整个溜过去。
  已在实现里把顺序写死（先 replace 再逐条检查），并补
  `a_backslash_drive_letter_is_rejected_too` 与 `read_config_normalizes_before_validating`
  两条断言钉住——后者走的是真实读盘路径，不是只测那个纯函数。

#### 计划没写的坑

- **`//server/share`（UNC）不需要单独判。** 它以 `/` 开头，被「必须是仓库内相对
  路径」那条一并挡掉了。计划把 UNC 单列成一类，实现上是同一条规则。

- **尾斜杠必须原样保留。** 它是「目录（递归）」与「单个文件」在 §6.3 里的唯一
  区分，规范化时顺手 `trim_end_matches('/')` 会让 exclude 生成走错分支。计划没
  提这一点，已补 `a_trailing_slash_is_preserved`。

#### 为什么这么改是安全的

- 仍然**只新增代码**：`svn_config` 没有任何现有模块引用，既有测试 238 → 252
  只多出本 Task 的 14 个。
- 本模块**不执行任何外部命令**，只读一个 JSON 文件并做字符串校验。
- 校验**只拒绝、不改写**用户的配置：`normalize_whitelist_entry` 唯一的改写是
  `\` → `/` 与去首尾空白，其余一律报错让用户自己改（对齐 §6.2.1「不猜、不自动
  改写」）。

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

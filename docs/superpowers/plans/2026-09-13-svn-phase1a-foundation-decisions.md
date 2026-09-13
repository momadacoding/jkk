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
| 4 嵌套 WC 枚举与 P4 | ✅ 完成 | `engine/svn_wc.rs`、`engine/mod.rs`、`tests/svn_wc.rs`、`tests/common/svn_fixtures.rs` | fmt ✓ clippy 零告警 ✓ `--test svn_wc` **6 passed** ✓ `cargo test` **264 passed / 0 failed**（= 226 + 38） |
| — review 修复（7 条） | ✅ 完成 | `svn_cli.rs`、`svn_config.rs`、`svn_xml.rs` + 各自测试 | 见「独立 review」一节。⚠️ **#3 分两次才修完**——第一次只加了文档契约就报了「全部已修」，等于没修。测试数 svn_cli 4→8、svn_config 14→16、svn_xml 8→11；`cargo test` **267 passed / 0 failed** |
| 5 exclude 规则生成 | ✅ 完成 | `engine/svn_exclude.rs`、`engine/mod.rs`、`tests/svn_exclude.rs` | fmt ✓ clippy 零告警 ✓ `--test svn_exclude` **9 passed**（计划写的是 7，多的 2 条覆盖「白名单条目互相包含」，见下）✓ `cargo test` **276 passed / 0 failed / 27 个二进制**（= 267 + 9） |
| 6 纳入集合核对 | ✅ 完成 | `engine/svn_include_check.rs`、`engine/mod.rs`、`tests/svn_exclude.rs`（追加） | fmt ✓ clippy 零告警 ✓ `--test svn_exclude` **17 passed**（9 + 8；计划写的是 3，多的 5 条见下）✓ `cargo test` **284 passed / 0 failed / 27 个二进制**（= 276 + 8） |
| 7 仓库识别 / capabilities / backend_for 分流 | ✅ 完成 | `vcs/mod.rs`、`vcs/git.rs`、`vcs/svn.rs`、`commands/repo.rs`、`lib.rs`、`tests/svn_backend.rs` | fmt ✓ clippy 零告警 ✓ `--test svn_backend` **11 passed**（计划写的是 9）✓ `cargo test` **295 passed / 0 failed / 28 个二进制**（= 284 + 11）。⚠️ 两个新 Tauri 命令的真实窗口验证在 Task 9 |
| 8 前端按能力隐藏入口 | ✅ 完成 | `src/engine/capabilities.ts`、`src/engine/toolbarActions.ts`、`src/engine/commitActions.ts`、`src/store/repoStore.ts`、`RepoTab.tsx`、`BranchView.tsx`、`GraphView.tsx`、`Toolbar.tsx`、`tests/vitest/capabilities.test.ts`、`tests/vitest/commitActions.test.ts` | `pnpm build`（含 tsc）✓ `pnpm test` **266 passed / 20 files**（= 257 + 9；计划写的是 3）。⚠️ 真实窗口验证在 Task 9 |
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

## 独立 review（2026-09-13，Task 1–3 之后）

按 CLAUDE.md「每完成一个非平凡里程碑先跑一次 review 再继续」，在 Task 1–3 落地、
Task 4 开始真正调 svn 之前跑了一次 `/code-review high`。**7 条发现，逐条核实后
全部成立，无一需要反驳，全部已修。**

| # | 级别 | 问题 | 修法 |
|---|---|---|---|
| 1 | Medium | `persistent_path_has_svn` 把整段 `reg query` 输出按 `;` 切，**第一个 chunk 是「注册表键名 + 换行 + `Path REG_EXPAND_SZ ` + 第一个目录」**，`trim()` 去不掉中间的换行，于是 **PATH 的第一个条目被静默吞掉** | 抽出 `path_dirs_from_reg_output()`：先定位含 `REG_EXPAND_SZ` / `REG_SZ` 的那一行，切掉类型标记之前的部分再按 `;` 切。函数**有意不加 `#[cfg(windows)]`**（纯字符串处理），这样测试不必按平台分叉 |
| 2 | Medium | `"./"` 绕过校验：`trimmed == "."` 是精确比较，`"./"` 不等于 `"."`，被原样放行——而 `./` 当 git pathspec 和当 SVN target **两边都等于「全部」** | `.` / `..` 改为**逐段**判断，顺带覆盖 `Assets/./Scripts` |
| 3 | Medium | `parse_node_revs` 的注释说「external 根没有 revision，跳过」，会让人以为 externals 已被处理。实际那只对**根节点**成立，external **内部**节点都带 revision 且来自另一个仓库的序列 | **分两次修**，见下方「#3 第一次只修了一半」 |
| 4 | Medium | **我自己写的假绿测试**：`info_must_not_receive_ignore_externals` 在非工作副本上只断言 `code != 0`，而那种目录下不带该参数的 `svn info` 同样退出 1。就算哪天 svn 开始接受这个参数，测试照样绿 | 改到真实 fixture 工作副本上跑，加**对照组**（不带该参数必须成功）并断言 stderr 含 `doesn't accept option`；另加 `status_and_update_do_accept_ignore_externals` 覆盖表的另一半 |
| 5 | Low-Med | `run_svn_code` 只返回 `(code, stdout)`，照抄了 `run_git_code` 的形状。但 git 靠 1 vs 128 区分失败类型，**svn 基本一律退出 1**，光看退出码分不出任何两种失败——这正是 #4 只能做无判别力断言的原因 | 改成返回 `SvnOutput { code, stdout, stderr }` |
| 6 | Low | `version` / `mode` 声明了字段却从不校验。`"version": 2` 会被当 v1 语义静默接受 | 加 `SUPPORTED_VERSION` / `SVN_GIT_MODE` 两个常量并校验。**与 `engine::custom_config` 有意不同**——那边最坏后果是一个按钮不对，这里版本语义差异会直接改变哪些文件受保护 |
| 7 | Low | `.git` / `.svn` 前缀判断大小写敏感，NTFS 下 `.SVN/wc.db` 会命中真实目录 | 改 `eq_ignore_ascii_case`；顺带补上「不能以 `-` 开头」（当 SVN target 时会被当成选项，git 侧有 `--` 保护但 svn 侧没有） |

### #3 第一次只修了一半

第一轮我只加了文档契约就在 checkpoint 里报了「7 条全部已修」。**那个说法不准确。**
reviewer 的原话是「写进文档**并在类型上区分**……**别只靠调用方记得**」，而一条文档
注释恰恰就是「靠调用方记得」——两个函数仍然都收 `&str`，签名上分不出来，陷阱原封
不动。用户追问「那你全部修复了吗」时复查才发现。

**第二次修法：让 `parse_node_revs` 自己检出违约。** 先实测了判据是否存在
（2026-09-13 临时 fixture，`Code/` 底下一条跨仓库 external `Vendor`）：

| entry path | 带 `--ignore-externals` | 不带 |
|---|---|---|
| `Code` | normal | normal |
| `Code\A.cs` | normal | normal |
| `Code\Vendor` | **external** | **external** |
| `Code\Vendor` | — | normal ← external 自己的 WC 根 |
| `Code\Vendor\V.cs` | — | normal ← external 的内容 |

关键观察：**`item="external"` 两种情况下都出现**，所以第一个想法（靠它的有无判定）
不成立；但带该参数时，external 根**及其底下**只有那一条 `item="external"` 条目。
判据因此是「是否存在位于某个 external 根之内、而自己不是那条 external 条目的 entry」。

实现为 `assert_externals_were_ignored()`，`parse_node_revs` 入口处调用，违约返回
`svn_xml_externals_not_ignored`。配三条测试：手写常量的正反两向，**外加一条拿真实
`svn status` 输出跑的双向测试**——手写常量是照实测抄的，但「照着抄」和「真的成立」
是两回事（`AGENTS.md` §14.1）。

**为什么没做 newtype。** reviewer 给的另一个选项是包两个 newtype。那要么把构造函数
开成 public（测试要能构造，于是又能被误构造，防护退化成命名约定），要么把取数函数
搬进 `svn_xml`——而 Phase 1a 根本没有 `nodeRevs` 的消费方（那是 1b 的失同步检测），
现在搬等于把 1b 的范围提前拉进来。运行时判据在这里更划算：零 API 面积、覆盖所有
调用方（包括将来 1b 新加的），且**有真实输入的测试兜着**。

**#1 与 #4 的共同点值得单独记**：两者都落在我自己在 Task 1 就写明的覆盖盲区上
（`StalePath` / `NotInstalled` 无测试）。盲区如实记了下来，但**记下来不等于挡住了**
——真正让缺陷暴露的是 review。现在 #1 的解析逻辑已有直接测试覆盖，盲区缩小到
「两条分支的端到端行为」，仍待 Task 9 的真实窗口验证。

reviewer 另外排除了两条疑虑，一并记下免得以后重新怀疑：`child_text` 用
`Node::text()` 只取第一个文本子节点，查过 roxmltree 0.21.1 的 `append_text` /
`merge_text`，字符实体会被合并进同一个文本节点，不会截断 URL/UUID；`Cargo.lock`
里 roxmltree 的依赖只有已在树里的 `memchr`，无 ghost dependency。

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

### Task 4：嵌套工作副本枚举与 P4

#### 偏离计划

- **fixture 从 `tests/svn_fixtures.rs` 挪到 `tests/common/svn_fixtures.rs`。**
  cargo 会把 `tests/` 下的每个 `.rs` 都当成一个独立测试目标，而这个文件里一个
  `#[test]` 都没有——照计划放会多出一个 0 测试的幽灵二进制，而 Task 9 要按
  **二进制个数**与基线对账，那个幽灵会把数字搅混。cargo 不递归发现子目录，
  放进 `common/` 既消除幽灵又是 Rust 的惯用做法。引用方式相应改成
  `#[path = "common/svn_fixtures.rs"]`。

- **测试从 4 个加到 6 个。** 多出来的两条都是在补 fixture 自身的可信度与边界：
  - `the_two_nested_wcs_really_belong_to_different_repositories`：如果 fixture
    造错了（external 指回同一个仓库），P4 那条断言会因为「本来就该通过」而假绿。
    这条把 fixture 自身的正确性也钉住。
  - `a_whitelist_entry_that_is_a_file_still_resolves_to_its_working_copy`：
    白名单可以是单个文件（§6.3），`svn info` 对文件同样给 root 与 uuid。

#### 计划没写的坑

- **去重必须同时看 `out` 和 `pending`。** 计划的伪码只查了 `out`，但一个
  external 可能被两条白名单同时覆盖到，两次都还没处理完就会被重复压栈。
  实现里两个都查。

- **`wc_root` 的尾斜杠必须去掉，而配置侧必须保留——两处规则相反，不能互相套用。**
  `wc_root` 是拿来当 svn target 和做相等比较的键，`Code` 与 `Code/` 必须是同一个；
  而配置里的尾斜杠承载「目录 vs 单文件」的语义（§6.3）。已在 `normalize()` 的
  注释里写明这个区别。

#### 为什么这么改是安全的

- `enumerate_wcs` 与 `check_single_repository` **只发 `svn info` 与 `svn status`
  两条只读命令**，不写任何东西。
- 空白名单在函数开头就返回空列表，**一条 svn 命令都不发**（§12.2）。
- P4 失败时**只报错不修改**任何状态——它的定位就是「不让用户白走完整个确认流程
  才在 D5 撞墙」。
- fixture 全部在 `TempDir` 里自建 `svnadmin create` 仓库，跑完即删，不碰本仓库、
  用户仓库或任何仓库外的固定路径。

---

### Task 5：exclude 规则生成

#### 偏离计划

- **`generate_exclude` 的结构从「按条目流水线输出」改成「分三段输出」**
  （① 打开父目录，去重 + 按深度升序 ② 目录条目的叶子规则 ③ 单文件条目）。
  原因见下方「计划没写的坑」——不改的话，白名单一旦出现互相包含的条目就会
  静默丢文件。计划里的输出示例（单条白名单）在改动后**逐字节不变**，
  三条既有测试可以作证。

- **测试从 7 个加到 9 个。** 多出来的两条就是覆盖上述组合：
  `a_narrower_entry_does_not_silently_drop_files_of_the_broader_one` 与
  `a_file_entry_coexists_with_a_directory_entry_that_contains_it`，
  两条都把**书写顺序的两个方向**都跑一遍——顺序不该改变纳入集合。

- **`svn_metadata_never_enters_git_with_the_wildcard_extension` 多加了两条正向断言**
  （`Code/Sub/Deep/Low.cs` 与 `Code/Junk.psd` 必须在）。只断言「没收进 `.svn`」
  的话，一条把所有东西都排除掉的规则同样能过——那是 `AGENTS.md` §13 rubric #9
  说的虚假信心。

#### 计划的错误

**计划给的 `generate_exclude` 会在白名单条目互相包含时静默丢文件。**

git 的裁决规则是「**最后一条**匹配的模式说了算」，所以规则的先后顺序本身就是
语义。计划那版按条目顺序流水线输出：后一个条目为打开自己的父目录要发一条
`/Code/*`，它落在前一个条目的 `!/Code/**/*.cs` **之后**，把前者已经纳入的文件
又剔了出去。2026-09-13 在真实 git 仓库里实测（断言的是 `git ls-files`）：

| 白名单（扩展名 `.cs`） | 计划那版的实际结果 |
|---|---|
| `["Code/", "Code/Sub/"]` | `Code/Top.cs` 消失，只剩 `Sub` 底下两个 |
| `["Code/", "Code/Junk.psd"]` | **只剩 `Code/Junk.psd`**，整个目录条目全没了 |

后一行比我预判的还糟：单文件条目那条 `/Code/*` 把 `Code` 的所有直接子项（含
子目录 `Sub`）都排除了，git 不再往里递归，目录条目的 `!/Code/**/*.cs` 一条都
够不着。

而且**全程没有任何报错**。用户看到的是「我明明把整个 `Code/` 加进了白名单，
顶层的 `.cs` 却没受保护」——切分支丢改动、stash 存不进去、交付清单里不出现，
正是 spec §6.3 说这个模块必须由代码生成的那个失败形态。

修法：三段式。① 把所有条目的非末段前缀收集起来去重、按深度升序，每层发一次
`!/P/` + `/P/*`；去重保证同一层只发一次，深度升序保证外层先于内层。② 再发目录
条目的叶子规则，`/P/**` 与按扩展名的反选成对出现、互不干扰。③ 单文件条目**必须
排在最后**，因为它不过扩展名过滤，排在目录条目之前会被 `/P/**` 重新盖掉。

#### 计划没写的坑

- **「单文件条目」与「目录条目 + 扩展名」不是同一种东西，不能互相替代。**
  单文件条目发的是 `!/Code/Junk.psd`，**不过扩展名过滤**——这是它存在的意义
  （「整个 `Code/` 收 `.cs`，另外单独收一个 `build.gradle`」是合法配置）。
  所以「窄条目被广条目覆盖就可以丢掉」这条捷径只对**目录条目**成立，
  对单文件条目不成立。最终没走「拒绝重叠条目」那条路，就是因为它会误伤
  这种合法配置。

- **`opens.sort_by_key` 必须是稳定排序。** Rust 的 `sort_by_key` 是稳定的，
  所以同深度的前缀保持白名单里的书写顺序；换成 `sort_unstable_by_key` 会让
  生成结果在同深度条目之间不可复现，而这个文件是要写进 `.git/info/exclude`
  再被 diff 的。

- **`write_exclude` 假定 `.git` 是目录。** git worktree 里 `.git` 是一个文件，
  这时 `create_dir_all(".git/info")` 会失败并返回 `io` 错误。Phase 1a 没有任何
  调用方（它要到 Phase 1b 的初始化向导才被用上），且本设计的形态就是 `.git`
  直接落在 SVN 工作副本根上，所以**本阶段不处理，如实记在这里**。与 Task 3 的
  `config_path` 是同一个假设，将来要改得一起改。

#### 为什么这么改是安全的

- **断言对象全部是 `git ls-files` 的实际输出，不是生成出来的规则文本。**
  测规则文本只能证明「生成了我以为的字符串」，证明不了 git 会怎么裁决，
  而这个模块存在的全部理由就是后者。
- `generate_exclude` 是纯函数，不碰磁盘；`write_exclude` 只写
  `<仓库根>/.git/info/exclude` 一个文件，**不动工作树、不动索引、不动 refs**。
  写坏的最坏后果是「某些文件没被 git 跟踪」，不会丢已提交的数据。
- 测试仓库全部是 `TempDir` 里 `git init` 出来的空仓库，`git add -A` 只在那里面
  跑，跑完即删（`AGENTS.md` 最高优先级条款）。
- 末尾那条 `.svn/` 有两条测试兜着（`.db` 在扩展名列表里、以及 `extensions:["*"]`
  两条最容易漏的路径），因为它一旦失效就不是「少跟踪几个文件」，而是 git
  checkout 会在一个**正在使用的** SVN 工作副本底下换掉 `wc.db`。

---

### Task 6：纳入集合核对

#### 偏离计划

- **`actual_set` 改成纯读，不建临时索引。** spec §6.3.1 的原文是「在**临时索引**上
  `git add -A -- <白名单>` 之后读 `ls-files`」，意图是「git 会跟踪谁」而不要弄脏
  用户的暂存区。实现走 `--cached` ∪ `--others --exclude-standard`：

  | | 临时索引方案 | 纯读方案 |
  |---|---|---|
  | 写临时索引文件 | 要（`GIT_INDEX_FILE` + 清理） | 不写 |
  | 往 `.git/objects` 写松散对象 | 会（`git add` 要算 blob） | 不写 |
  | 结果 | —— | **实测相同** |

  两者等价这件事没有当成推断——`the_actual_set_equals_what_git_add_would_have_produced`
  在真实仓库上把两条路径的结果直接对了一遍，且断言「两边都不为空」，
  免得退化成「空集 == 空集」的假绿。

- **`expected_set` 会读 `core.ignoreCase`**（计划里没有这个概念）。理由见下方
  「计划没写的坑」。

- **`ls_files` 一律带 `-z`**（计划里是按行读）。理由见下方。

- **测试从 3 条加到 8 条。** 计划那 3 条里有 2 条是坏的（见「计划的错误」），
  修好之后又补了 5 条：索引已填充的那一路、等价性、空白名单、非 ASCII 文件名、
  大小写敏感性。

#### 计划的错误

**计划给的 3 条测试里，1 条必然失败、1 条会以错误的理由变绿。**

计划的 `include_check_passes_on_a_clean_working_directory` 和
`include_check_reports_unexpected_files_too` 都只 `write_exclude` 就直接
`verify_include_set`，**中间没有 `git add -A`**。而计划版 `actual_set` 读的是
`git ls-files`（索引），在一个刚 `git init` 的仓库里索引是空的：

| 计划的测试 | 实际会发生什么 |
|---|---|
| `..._passes_on_a_clean_working_directory` | expected 有 3 个 `.cs`、actual 空 → `unwrap()` panic，**测试直接挂** |
| `..._reports_unexpected_files_too` | 同样是 actual 空 → 报的是「**该在的没在**」，但它名字叫 *unexpected*。断言只查 `err.code`，于是**以完全错误的理由变绿** |

后一条正是 `AGENTS.md` §13 rubric #9 说的虚假信心：名为「覆盖 X」的测试测的不是
X 的真实形态。修法是让它先真的 `git add -A` 把 `.svn/wc.db` 送进索引，并加一条
**前置条件断言**（`.svn` 下确实有文件进了索引）和两条内容断言（错误信息必须含
「不该被跟踪」和 `.svn/wc.db`），这样它只能因为「多出来」那一侧而通过。

顺带把「纯读」方案确定下来之后，第一条测试反而成立了——所以它改名为
`include_check_passes_before_anything_is_staged`，名字直说它覆盖的是哪个时刻。

#### 计划没写的坑

- **`git ls-files` 默认会转义非 ASCII 路径。** `core.quotePath` 默认 `true`，
  `Code/角色.cs` 读出来是 `"Code/\350\247\222\350\211\262.cs"`（带引号的八进制），
  而期望集合是遍历磁盘得到的原始字符串——两者**永远对不上**。后果是每一个装了
  中文文件名的工程都在初始化时被误报阻断，而这个项目面向的就是中文团队的 Unity
  工程。修法是一律带 `-z`（同时关掉转义并用 NUL 分隔，顺带也扛住文件名里含换行）。
  实测验证：把 `-z` 摘掉，`non_ascii_filenames_do_not_produce_a_phantom_mismatch`
  立刻变红。

- **`core.ignoreCase` 会改变忽略规则的匹配语义。** Windows 上 `git init` 会把它
  探测成 `true`，于是 `!/Code/**/*.cs` 连 `Loud.CS` 一起纳入。期望集合若死板地按
  大小写敏感算扩展名，就会把 `Loud.CS` 报成「不该被跟踪却跟踪了」——在一个完全
  正常的仓库上**误报阻断**。所以 `expected_set` 去问 `git config --get
  core.ignoreCase`，跟 git 保持一致，而**不是按平台猜**（同一台 Windows 上，仓库
  落在大小写敏感的卷、或用户显式设成 `false` 时行为就反过来）。实测验证：强制
  `fold = false`，`extension_matching_follows_gits_own_case_sensitivity` 立刻变红。

- **单文件白名单条目不过扩展名过滤——`expected_set` 必须跟 `svn_exclude` 对齐。**
  计划的 `expected_set` 对 `abs.is_file()` 的分支写了 `if matches_ext(...)`，但
  `svn_exclude` 的第 ③ 段发的是光秃秃的 `!/Code/Junk.psd`，不过滤。两边不一致
  会让「整个 `Code/` 收 `.cs` + 单独收一个 `build.gradle`」这种合法配置在核对时
  被判成「多出来」。已把 `matches_ext` 从单文件分支去掉。

- **白名单条目在磁盘上不存在时不报错。** 本函数的定义就是「磁盘上现在有什么」；
  「配置指向了不存在的路径」是另一条独立校验，属初始化向导（Phase 1b）。
  已写进函数注释，免得下一个人当成漏判补一个不该在这里的分支。

#### 为什么这么改是安全的

- **本模块一条写操作都没有。** `git ls-files` / `git check-ignore` / `git config
  --get` 全是只读，不碰索引、不碰工作树、不碰 refs、连松散对象都不写
  ——这也是放弃「临时索引」方案的主要理由。
- **空白名单在 `actual_set` 开头就返回空集，一条 git 命令都不发**（G6）。
  `an_empty_whitelist_never_degenerates_into_everything` 在一个**确实有被跟踪文件**
  的仓库上断言这一点，所以它不是恒真的。
- **`ls-files` 的参数里有 `--` 分隔符。** 白名单条目是用户手写的，没有它一个以
  `-` 开头的条目会被 git 当成选项解析（`AGENTS.md` §6.2 的空 pathspec 事故同源）。
  Task 3 的 `normalize_whitelist_entry` 已经挡了前导 `-`，这里是第二道。
- 测试仓库全部是 `TempDir` 里 `git init` 出来的，跑完即删，不碰本仓库或用户仓库。
- 两条防御性测试都做过**反向验证**（摘掉保护后必须变红），不是写完就算。

---

### Task 7：仓库识别、`capabilities()` 与 `backend_for` 分流

#### 偏离计划

- **`#[path = "svn_fixtures.rs"]` 改成 `common/svn_fixtures.rs`。** Task 4 已经
  把 fixture 挪进 `common/`（理由见那一节），计划里这条路径没同步。

- **测试从 9 条加到 11 条，且换掉了其中一条。**
  - 删 `a_suspicious_repo_refuses_to_open`，换成
    `the_two_blocking_states_carry_different_error_codes`：原测试只验证了
    `svn_unconfigured_git` 一个码，而这两个状态存在的全部意义就是**引导相反**
    （去初始化 vs 去查那个 `.git` 哪来的）。只测一半，另一半塌缩成同一个码也
    不会有人发现。新测试把两个码都断言，外加一条 `assert_ne!`。
  - 加 `a_directory_with_neither_marker_still_yields_a_backend`：本 Task 给
    `backend_for` 新开了三条 `Err` 分支，而「打开任意文件夹」这条既有路径依赖
    「任何目录都能拿到一个后端」。没有这条，新分支哪天多吃了一种目录也没人发现。
  - 加 `a_hybrid_repo_with_a_broken_config_fails_at_open`：配置坏了既不是
    「可疑」也不该让半个坏配置流到下游，错误码是 Task 3 的 `svn_config`。

- **`hybrid()` 辅助函数会连 exclude 一起写。** 计划的 fixture 只有
  `.svn` + `.git` + 配置，没有 `.git/info/exclude`——那不是混合仓库的真实形态。
  顺带让 `an_svn_backend_still_delegates_git_reads` 能多断言一句
  「`.svn` 不出现在状态列表里」；不写 exclude 的话 SVN 的工作副本数据库会摆在
  「未跟踪文件」里等用户一键 stage 进去，而原测试根本看不见这件事。

#### 计划的错误

- **`expect_err` 在 `Result<Box<dyn Vcs>, _>` 上编译不过。** `expect_err` 要求
  `T: Debug`，而 `Box<dyn Vcs>` 没有 `Debug`（给 `trait Vcs` 加 `Debug` 约束是
  为了一条测试去动公共接口，不值当）。改用 `.err().expect(...)`，它不要求 `Debug`。

- **计划的 Interfaces 块写「其余 16 个方法不变」，实际是 17 个。** `trait Vcs`
  现有 17 个方法，`SvnGitBackend` 委托了全部 17 个。数字已在 `Capabilities`
  的文档注释里改正。

#### 计划没写的坑

- **这里有一处会影响现有用户的行为变化，必须显式说出来。** Phase 0 在
  `backend_for` 上留了一条注释：「**不要在这里加 `.svn` 判断**——现在加会改变
  『一个同时有 `.svn` 和 `.git` 的目录』的现有行为（今天它就是个普通 git 仓库），
  那就不是零行为变化了。SVN 分支在 Phase 1 连同配置文件识别一起加。」

  本 Task 正是它说的那个时机，所以约束解除。**代价是**：一个今天能当普通 git
  仓库打开的 `.svn` + `.git` 目录（比如有人自己在 SVN checkout 里 `git init` 过），
  从此会被 `svn_unconfigured_git` 挡住。spec §6.1 明确要这个结果——那个 `.git`
  的来历不明，猜错的后果是拿另一个人的基线去覆盖工作副本。这条已经写进
  `backend_for` 的文档注释，不让它只活在决策日志里。

- **`.git` 必须用 `exists()` 判断，不能用 `is_dir()`。** git worktree 里 `.git`
  是一个**文件**。计划的代码用的就是 `exists()`，但没说为什么，容易在后续重构里
  被「统一成 is_dir」掉。已补注释。

- **所有 `backend_for` 调用点都用 `?` 直接向上抛**（`commands/{diff,graph,ops,repo,stash}.rs`
  共 19 处），所以三条新 `Err` 分支会原样变成前端的错误横幅，不需要逐个改。
  已 grep 核对过，没有吞错误的调用点。

#### 为什么这么改是安全的

- **`SvnGitBackend` 的 17 个方法全部原样委托给 `GitBackend`，零行为变化。**
  它在 Phase 1a 唯一的自有逻辑是 `capabilities()`（纯返回常量）和 `open()` 里的
  一次配置校验。
- **`capabilities()` 是只读的**，不发任何命令、不碰磁盘。
- 唯一的行为变化就是上面那条新增的阻断，且**是阻断而不是放行**——失败方向是
  「打不开」，不是「用错后端去写」。
- Phase 0 交接里点名的那批仍直连 git 的命令（`checkout_remote_branch` /
  `create_tag` / `cherry_pick` / `save_patch` / `fetch` / `pull` / `push` 与整个
  `commands/worktree.rs`）**本 Task 一行都没动**。它们靠 Task 8 的前端不渲染入口
  来兜。这是一层**表现层**防线，不是接缝——如实记在这里，不要当成已经堵住了。

---

### Task 8：前端按能力隐藏 git 专属入口

#### 偏离计划

- **`visibleSections` 的取值集合与计划不同。** 计划写的是六个平铺分区
  `['branches','remotes','tags','stashes','worktrees','pullRequests']`，但实际的
  组件树不是这个形状（见下方「计划的错误」）。改成与 `RepoTab` 渲染顺序一致的
  四个：`['branches','tags','worktrees','stashes']`。

- **多了两个纯函数，都放在 `src/engine/`**（本仓库的约定是纯逻辑下沉、组件只渲染）：
  - `toolbarActions.ts` 的 `visibleToolbarActions(caps)`
  - `commitActions.ts` 的 `commitMenu` 增加 `ctx.caps`，按项裁剪

- **`CommitMenuContext.caps` 是必填，不是可选。** 设成可选、默认全开的话，新加的
  调用点忘了传就会在 SVN 仓库上静默多出一个直连 git 的菜单项。必填让 `tsc` 在
  编译期报出来——**而它真的报了**：`RepoTab.tsx:262` 那个第二调用点就是 tsc 抓到的，
  不是我翻出来的。代价是 `tests/vitest/commitActions.test.ts` 的 7 处调用要跟着改
  （`tests/vitest/**` 不在 tsc 覆盖范围内，靠 vitest 兜）。

- **多了一个常量 `NO_GIT_ONLY_CAPABILITIES`。** 只有 `ALL_CAPABILITIES` 的话，
  将来加一个新能力字段时，「忘了在 SVN 侧关掉」没有任何东西能抓到。
  测试里有一条专门比对两个常量的键集合。

- **测试从 3 条加到 9 条**：分区 4 条、工具栏 2 条、提交菜单 3 条。

#### 计划的错误

- **计划的 `visibleSections` 模型对不上真实组件树。** 侧边栏实际只有四个
  `SidebarSection`：`BranchView`（BRANCHES）、`TagView`、`WorktreeView`、`StashView`。
  **远程分支和 PR 不是独立分区**——前者是 `BranchView` 内部的 `SidebarGroup "Remote"`，
  后者是本地分支行上的 `PrBadge`。照计划写出来的 `visibleSections` 返回两个
  渲染层根本用不上的字符串，而真正该关掉的两处一个都关不到。
  改法是让 `visibleSections` 只管分区，`remotes` / `pullRequests` 由 `BranchView`
  自己按 props 决定。

- **计划的 Step 8.5 说「在打开仓库的两处 `invoke('open_repo')` 之后各加一次」。**
  实际只有 `openRepo` 一处是「打开仓库」；另一处是 `refreshHead`，它只重读 HEAD，
  在 checkout / pull 之后调用，那里再读一次能力纯属多余的 IPC。

#### 计划没写的坑

- **worktree 里没有 `node_modules`，`pnpm test` 会静默跑到主工作区去。**
  第一次跑 `pnpm test` 的输出里文件路径是
  `.claude/worktrees/svn-phase1a/tests/vitest/...`、`Test Files 167 passed`
  ——pnpm 沿目录向上找到了主仓库的 `node_modules` 和 `package.json`，于是
  **vitest 的 root 是主工作区**：它同时收了主检出和 worktree 两份测试文件，
  而 `@` 别名解析到的是**主检出的 `src/`**。

  后果有两层：跑的不是本分支的代码（AGENTS.md §4.1 要求验证在 worktree 内进行），
  以及新模块「找不到」的报错会被误诊成路径写错。在 worktree 里 `pnpm install`
  之后恢复正常：`Test Files 20 passed / 266 tests`。**下次在新 worktree 里第一次
  跑前端验证，先确认 `node_modules` 在不在。**

- **`state.capabilities` 每次打开仓库都必须写回，不能只在成功时写。**
  上一个仓库是 SVN 时留下的能力集合会把下一个普通 git 仓库的入口一起关掉。
  `readCapabilities` 因此把失败也吃掉并返回 `ALL_CAPABILITIES`，保证 `openRepo`
  的那次 `setState` 一定带着一个明确的值。

- **PR 数据在 SVN 模式下天然是空的，不需要再加一层判断。** `refreshPullRequests`
  只有两个触发点：`openRepo`（已按 `caps.pullRequests` 跳过）和 `runToolbar` 里的
  fetch/pull/push（按钮本身已被 `visibleToolbarActions` 裁掉）。已 grep 核对。

- **`DeleteBranchDialog` 的「Also delete remote branch」不用改。** 它挂在
  `{info?.upstream && …}` 下面，而 SVN 混合仓库的 git 侧没有任何 remote，
  没有分支有上游，勾选框根本不出现。已读代码确认，不是推断。

#### 为什么这么改是安全的

- **只隐藏入口，一行后端代码都没删。** 这些命令在普通 git 仓库上照常要用；
  `ALL_CAPABILITIES` 下渲染出来的东西与改动前逐项一致（`visibleSections`、
  `visibleToolbarActions`、`commitMenu` 三条全开路径都有测试断言）。
- **失败方向是「多显示」而不是「误操作」**：读不到能力就退回全开，受影响的是
  SVN 仓库上多露一个入口；反过来把普通 git 仓库搞成半残要糟得多。
- **这是表现层防线，不是接缝——已经写进 `capabilities.ts` 的模块注释。**
  `checkout_remote_branch` / `create_tag` / `cherry_pick` / `save_patch` /
  `fetch` / `pull` / `push` 与整个 `commands/worktree.rs` 仍然直连 git CLI。
  隐藏入口只是不让人点到它们，不是它们变安全了。谁要是绕过 UI 直接 invoke，
  照样打得进去。
- 已 grep 全部 `src/components` 与 `src/store`，确认这批命令的**唯一**入口都在
  已裁剪的组件里，没有漏网的按钮。

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

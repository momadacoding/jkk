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
| 9 收尾验证 | 🟡 进行中 | 本文件、`svn_xml.rs`、`svn_config.rs`、`svn_wc.rs`、`svn_exclude.rs`、`svn_include_check.rs`、`svn_cli.rs`、`vcs/svn.rs`、`engine/deleteBranchTargets.ts`、`DeleteBranchDialog.tsx`、`BranchView.tsx` + 各自测试 | 9.1 ✓ 9.2 ✓ 9.3 ✓ **9.6 第一轮独立 review：2 条阻断 + 7 条应修，全部核实成立并修完**。**9.7 第二轮独立 review：4 条 P2，全部实测复核成立并修完**。**9.8 master 上的 spec 更新（`310c17d`）已比对，三处落在本期地盘：一处已修入、一处不需改代码、一处记账给 1b**。全套：Rust **316 passed / 0 failed / 28 个二进制**、fmt ✓ clippy exit 0；前端 `pnpm build` ✓ `pnpm test` **272 passed / 21 files**。⬜ **9.4 真实窗口验证待用户执行**（fixture 已备好，见 9.4 一节） |

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

> ⚠️ **下表两列都是带 `-v` 跑出来的**，只有 `--ignore-externals` 在变。9.6 独立
> review 指出原表漏标了这一点——照表复现时不带 `-v` 会得到相反的结论，因为不带
> `-v` 的 status 只报告有改动的节点，干净工作副本上两列都是空的。守卫当时也确实
> 漏了这一半，见 9.6 的 B2。

| entry path | `-v` + `--ignore-externals` | `-v`，不带 `--ignore-externals` |
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

- **每个 Task 的测试数都比计划多，合计 72 对 41**（逐文件对账见 Task 9 的 9.1）。
  没有一处是少的。多出来的绝大部分属两类：给「名字比覆盖范围大」的测试补齐真实
  形态（`AGENTS.md` §13 rubric #9），以及给防御性改动补**反向验证**
  ——摘掉保护后测试必须变红，否则它证明不了任何事。本期有三条做过反向验证：
  `-z`、`core.ignoreCase`、`#[serde(rename_all)]`。

#### 计划的错误

计划里的**测试代码**错得比实现代码多，且集中在同一类：**没有真的跑过**。
逐条记在各 Task 下，共同点在这里说一次——

| Task | 错在哪 | 会怎样 |
|---|---|---|
| 5 | 「按条目流水线输出」exclude 规则 | 白名单条目互相包含时静默丢文件 |
| 6 | 两条测试漏了 `git add -A` | 一条必挂，一条以「报缺失」的错误理由变绿 |
| 7 | `expect_err` 用在 `Result<Box<dyn Vcs>, _>` 上 | 编译不过（`Box<dyn Vcs>` 没有 `Debug`） |
| 7 | `#[path = "svn_fixtures.rs"]` | 路径不对（Task 4 已挪进 `common/`） |
| 8 | `visibleSections` 的六分区模型 | 对不上真实组件树，该关的两处一个都关不到 |

**教训**：计划里的代码块是**未经执行的草稿**，不是可以照抄的实现。尤其是测试——
一条从没跑过的测试，最可能的形态就是「能编译、能变绿、但测的不是它名字说的东西」。

#### 计划没写的坑

- **`EnterWorktree` 的 baseRef 默认是 `fresh`（`origin/<默认分支>`），不是本地 HEAD。**
  这条 Phase 0 已经记过一次，本期沿用同样的处理：切 worktree **之前**先把 spec 修订
  （`150ddad`）与本计划（`02f8d45`）push 到 `origin/master`，走 `AGENTS.md` §4.1 的
  纯文档例外。切入后已实测基点确实是 `02f8d45`，计划与修订后的 spec 都在。

- **SVN 的 peg revision 解析吃掉路径里的 `@`，`--` 挡不住。** 计划全程把
  `--` 当成「target 到此为止」的万能护栏，但那只管「这之后不是选项」。合法目录名
  `Code/Team@Home` 会让 `svn info` 报 `E205000`、`svn status` 报 `E200009`。
  修法是无条件追加空 peg 后缀 `@`——实测不含 `@` 的普通路径也接受这个后缀，
  所以不需要条件分支（条件分支会被忘掉，恒等变换不会）。详见 9.7。

- **git 的布尔配置不只有 `true` 这一种写法。** `1` / `yes` / `on` 都是真值。
  凡是读 git 配置做判断的地方都要让 git 自己解析（`git config --bool`），
  不要字符串比对。详见 9.7。

- **`git check-ignore` 默认先看索引，对已跟踪文件一律吐空裁决。** 想问「这个
  已经进了索引的文件凭什么进来」必须带 `--no-index`。详见 9.7。

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

#### 偏离计划

无。自检与基线按计划执行，唯一的调整是 Step 0.5 的基线读取方式（见下方「计划的错误」）。

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

#### 计划的错误

计划给的 `svn_cli.rs` 代码本身没错。但它给的**测试**有一条名不副实，
2026-09-13 独立 review 抓到：`run_svn_injects_non_interactive_for_every_subcommand`
只断言各子命令的 stderr 不含 `doesn't accept option`——**把 `with_global` 改成
原样返回参数，它照样全绿**。它证明的是「svn 接受这个参数」，不是「我们真的加了」。

漏掉注入没有任何编译期或运行期信号：GUI 应用没有终端，svn 一旦要提示输入凭据
就会永久挂住并占掉 Tauri 的线程池，表现只是「点了没反应」（与 `git_cli` 的
`GIT_TERMINAL_PROMPT=0` 同源）。

修法：`with_global` 改成 `pub`，新增 `with_global_actually_prepends_non_interactive`
直接断言参数表形状（含空参数表那一路）；原测试改名为
`every_subcommand_accepts_the_injected_global_option`，名字回到它真正覆盖的范围。

**把私有函数公开出来是否值得，这里与 review #3 的 newtype 结论不冲突**：那次拒绝
是因为公开构造函数会让防护退化成命名约定；`with_global` 是纯函数，没有需要靠
私有性保护的不变量，公开不损失任何东西。

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

#### 计划的错误

无。计划给的 `svn_wc.rs` 与 P4 的逻辑与实测行为一致，只有 fixture 路径需要调整
（见下方「偏离计划」第一条）。

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
  用户的暂存区。实现走 `--cached` ∪ `--others --exclude-standard` **− `--deleted`**：

  | | 临时索引方案 | 纯读方案 |
  |---|---|---|
  | 写临时索引文件 | 要（`GIT_INDEX_FILE` + 清理） | 不写 |
  | 往 `.git/objects` 写松散对象 | 会（`git add` 要算 blob） | 不写 |
  | 结果 | —— | **实测相同** |

  ⚠️ **这条「实测相同」我一开始写早了，第三项 `− --deleted` 是 9.6 独立 review
  补上的。** 原来只有前两项，而「等价」被当成实测事实写进了文档、测试名和这张表
  ——但当时那条等价性测试跑在**空索引**上，`--cached` 恒为空，真正会分叉的那一半
  根本没执行到。反例见 9.6 的 B1。现在有两条测试，其中
  `the_actual_set_equals_git_add_even_when_the_index_is_already_populated`
  专门跑在索引非空、且存在「删除未 stage」差异的时刻。

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
- 已 grep 全部 `src/components` 与 `src/store`，确认这批命令的入口都在已裁剪的
  组件里。

  ⚠️ **这句话我第一次写成了「唯一入口……没有漏网的按钮」，那是不准确的。**
  9.6 独立 review 找到一个漏网的：`DeleteBranchDialog` 的「Also delete worktree」
  勾选框只看 `info.worktree`、不看 `capabilities.worktrees`，而 `BRANCHES` 分区在
  任何模式下都渲染，所以在 SVN 模式下它是可达的（分支被一个应用外建的 linked
  worktree 检出时就会出现）。已修，见 9.6 的 A5。

  教训：「我 grep 过了」证明的是**我搜的那些关键字**没有漏网，不是没有入口。
  这次漏掉的原因是我按命令名 grep `src/components`，而这个入口是**弹框内部**的一个
  勾选框，它的 `invoke` 在 `RepoTab` 的回调里——命令名和渲染条件隔着两个文件。

---

### Task 9：收尾验证

#### 9.1 两侧全套检查（实测数字）

| | 基线（master） | 本分支 | 增量 |
|---|---|---|---|
| Rust 测试 | 226 passed / 22 个二进制 | **312 passed / 0 failed / 28 个二进制** | +86 / +6 |
| `cargo fmt --check` | — | ✓ 干净 | |
| `cargo clippy --all-targets` | — | ✓ 零告警 | |
| 前端 vitest | 257 passed / 19 files | **266 passed / 20 files** | +9 / +1 |
| `pnpm build`（含 tsc） | — | ✓ 通过 | |

Rust 增量按测试文件拆开对账：

| 文件 | 计划预估 | 实际 |
|---|---|---|
| `svn_cli.rs` | 4 | 9 |
| `svn_xml.rs` | 6 | 13 |
| `svn_config.rs` | 8 | 21 |
| `svn_wc.rs` | 4 | 6 |
| `svn_exclude.rs`（Task 5 + 6） | 7 + 3 | 23 |
| `svn_backend.rs` | 9 | 14 |
| **合计** | **41** | **86** |

计划自己写了「以实测为准，不要照抄这个数字——对不上就说明有 Task 的测试没写全或写多了，两种都要查」。这里是**每个文件都多**，逐条原因已记在各 Task 的「偏离计划」里，没有一处是少的。

⚠️ **上表是 9.6 独立 review 修完之后重跑的数字。** review 前是 298，9 条发现修完
涨到 312（+14）。计划把 9.1 排在 9.6 之前，等于默认 review 不会带来代码改动——
实际它带来了两条阻断修复和六处行为改动，见下方 9.6 一节。

#### 9.2 命令注册对账

```text
grep -rn "#[tauri::command" src-tauri/src/commands/ | wc -l   → 47
grep -c "commands::" src-tauri/src/lib.rs                     → 46
```

差的那 1 个是 `commands/session.rs:9` 的**文档注释**里提到了 `#[tauri::command(async)]`，不是命令定义——Phase 0 记过这个坑，实测确认还在。真实命令数 **46**，与注册数相等。

`git show master:src-tauri/src/lib.rs` 的注册数是 **44**，差值正好 **+2**（`read_capabilities`、`probe_svn_environment`）。

#### 9.3 确认没有 SVN 写操作混进来（G8）

在 `src-tauri/src/engine/svn_*.rs` 与 `src-tauri/src/vcs/svn.rs` 上扫 `add|commit|update|revert|delete|propset|checkout|import|copy|move|mkdir|lock|unlock|merge|switch|relocate|cleanup|resolve|resolved` —— **零输出**。

再正面列一遍这些模块实际发出的 svn 子命令：

```text
"info",   "--xml", "--", <target>
"status", "--xml", "--", <target>
```

**只有这两条，都是只读。** fixture 构造里有 `svn add` / `commit` / `propset`，那在 `tests/common/` 下，不在 `src/`。

#### 9.4 真实窗口验证（**待执行**）

`AGENTS.md` §5.1：改了 Tauri 命令签名就必须实际跑 `pnpm tauri dev` 确认 invoke
真的通。本期新增两个命令。**这一步尚未执行**，fixture 已用脚本备好（scratchpad 下
`make-phase1a-fixtures.ps1`，四个目录覆盖六个场景）：

| # | 目录 | 期望 |
|---|---|---|
| 1/2 | `git-plain` | 与改动前完全一致：worktree / tag / PR / remote 入口**都在**；暂存、提交、切分支、stash 全部照常 |
| 3 | `wc-uninit`（有 `.svn` 无 `.git`） | 报 `svn_not_initialized`，**不崩** |
| 4 | `wc-suspicious`（`.svn` + 无配置的 `.git`） | 报 `svn_unconfigured_git` |
| 5/6 | `wc-hybrid`（完整混合仓库） | 能打开；tag / worktree / remote / PR 入口消失；分支与 stash 还在；暂存 → 提交 → 切分支 → stash 全部照常 |

全部在临时 fixture 上做，**禁止拿任何真实 SVN 仓库试**（`AGENTS.md` 开篇的最高
优先级约束）。

#### 9.6 独立 review（`AGENTS.md` §4.1 的合并前门槛）

由**独立于实现者**的 agent 按 §13 rubric 跑了一遍全量 diff，**2 条阻断 + 7 条应修，
逐条核实全部成立，无一需要反驳**。两条阻断都先写成失败测试复现、再改实现。

| # | 级别 | 问题 | 修法 |
|---|---|---|---|
| B1 | 阻断 | **「`--cached ∪ --others` 与 `git add -A` 等价」这个结论是错的，而它被当成实测事实写进了文档、测试名和本文件。** 反例：已跟踪文件被从工作树删掉、删除还没 stage 时，`--cached` 仍列它而 `git add -A` 之后不列，`expected_set` 走磁盘也不含它 → 核对报「不该被跟踪、实际跟踪了 X」并硬阻断，而用户只是删了个文件还没提交 | `actual_set` 减去 `git ls-files --deleted`。补两条测试，其中一条断言的是**索引非空**时刻的等价性——原来那条跑在空索引上，`--cached` 恒为空，真正会分叉的那一半根本没执行到 |
| B2 | 阻断 | **`parse_node_revs` 的守卫只挡住了 `--ignore-externals` 那一半，漏了 `-v` 时完全不响。** 干净工作副本上不带 `-v` 的 status 只输出 external 标记，守卫沉默、返回 `Ok(空集)`——正是本模块开头写明的那个失败形态：「单值不变量」以看起来通过的方式被绕过 | 空集直接报 `svn_xml_empty_node_revs`。补一条对照测试证明判据不误伤最窄的合法输入（target 就是 external 根本身） |
| A1 | 应修 | **白名单不挡 glob 元字符**（`extensions` 那侧早就挡了）。实测 `whitelist:["*"]` + `extensions:[".cs"]` 会把 `Art/boss.fbx`、`Code/Junk.psd` 一起收进 git——扩展名过滤被整个绕开、白名单退化成「全部」 | `normalize_whitelist_entry` 拒掉 `* ? [ ]`，测试表里加 5 个形态 |
| A2 | 应修 | **目录条目漏写尾斜杠 → 静默变成「整棵树不过扩展名过滤」。** 尾斜杠是「目录 vs 单文件」的唯一区分，而单文件条目不过扩展名过滤，于是 `"Assets/Scripts"`（少一个 `/`）会把整棵子树收进 git。没有任何一层校验它和磁盘上的实际类型对不对得上 | 新增 `validate_whitelist_against_disk`，`SvnGitBackend::open` 里调用 |
| A3 | 应修 | **Windows 大小写只处理了一半。** git 的忽略规则匹配大小写不敏感、**但 pathspec 匹配是敏感的**。`"code/"` 这种写法下 git 照常跟踪 `Code/Top.cs`，而 `git ls-files -- code/` 返回空 → 在一个工作完全正常的仓库上报「应当被跟踪、实际没有」 | 同上那道校验里逐段比对磁盘上的真实大小写。**不能用 `Path::exists()` 判**——NTFS 不区分大小写，只能列目录逐项比 |
| A4 | 应修 | `p.is_dir()` 跟随 junction，junction 成环时递归没有出口 → 栈溢出 → Rust abort → **整个 Tauri 应用凭空消失** | 见下方「计划没写的坑」——reviewer 给的方向对一半，我实测后走了另一条路 |
| A5 | 应修 | **前端能力裁剪漏了一个入口。** `DeleteBranchDialog` 的「Also delete worktree」勾选框只看 `info.worktree`、不看 `capabilities.worktrees`，确认后走 `invoke('delete_worktree')`。而 `BRANCHES` 分区在任何模式下都渲染，所以这个入口在 SVN 模式下是可达的。**Task 8 里我写的「已 grep 全部 src/，这批命令的唯一入口都在已裁剪的组件里」因此不准确** | 把 `capabilities` 传进弹框：不支持时不渲染勾选框、`useEffect` 里不默认勾上、`confirm()` 里再判一次（与已有的 `remoteProtected` 同一个模式），并给一条说明告诉用户该去哪儿处理。⚠️ **这个修法本身也只做了一半**——同一个 `confirm()` 里的远程那半仍然没判能力，下一轮 review 的 P2-2 抓到了，见 9.7 |
| A6 | 应修 | `write_exclude` 用 `File::create` **先截断再写**，且无条件覆盖用户已有的 `.git/info/exclude`。写到一半失败留下的最坏形态是**缺了末尾那行 `.svn/`** ——之后一次 `git add -A` + `git checkout` 就会在正在使用的工作副本底下换掉 `wc.db` | 临时文件 + `sync_all` + `rename` 原子替换；首次接管时把含真实规则的原件备份成 `exclude.bak` |
| A7 | 应修 | 记录瑕疵：四类条目有缺项（Task 0 缺「偏离计划」，Task 1/4/9 缺「计划的错误」）；Task 9「偏离计划」写「从 11 到 13」而实际是 14；`run_svn_injects_...` 名不副实 | 本次一并补齐，测试的修法记在 Task 1 的「计划的错误」里 |

**A4 我没有照 reviewer 给的修法做，因为实测推翻了它的前提。** reviewer 说「git 把
符号链接记成一个 blob、不往里递归，所以按一个文件处理即与 git 对齐」。我先照这个
改了，结果 `a_directory_junction_inside_the_whitelist_does_not_cause_a_phantom_mismatch`
立刻失败——**git 实际上会走进 junction**：

| 实测项（2026-09-13，真实 git fixture） | 结果 |
|---|---|
| `Code/Mirror` 是指向仓库外目录的 junction，`git ls-files --others -- Code/` | 列出 `Code/Mirror/Linked.cs`，**走进去了** |
| `symlink_metadata("Code/Mirror")` | `is_dir=false, is_symlink=true`（junction 也算 reparse point） |
| 本机 `core.symlinks` | `false`；`symlink_dir()` 直接失败（没开开发者模式） |
| `Code/Loop` → `Code` 成环时 git 的行为 | 一路递归到 **64 层**才放弃，吐出 64 条 `Code/Loop/…/Top.cs` 并打印 `could not open directory: Function not implemented`。**不挂、不崩** |
| `canonicalize("Code/Loop")` | 与 `canonicalize("Code")` 相同——能识破环 |

所以「符号链接记成 blob」那条对 `core.symlinks=true` 下的真符号链接成立，对 Windows
的 junction 不成立。按那个方向改**反而制造了新的误报**（期望集合少一批文件）。

最终修法：**跟随 junction（与 git 对齐），用祖先链的规范路径做环保护**。只查祖先链
不做全局已访问集合——两条 junction 指向同一个目标是合法的，git 会把两边都列出来，
全局去重会与它分叉。环存在时本函数与 git 仍会给出不同集合，那种工作树本来就是坏的，
**报一次集合不一致是正确行为**；这里要保证的只是不崩。环保护做过反向验证（关掉它，
loop 测试立刻失败）。

#### 9.7 第二轮独立 review（2026-09-14）

又一轮独立 review，**4 条 P2，逐条实测复核全部成立，无一需要反驳，已全部修完**。
前两条靠读代码就能确认，后两条各自在临时 fixture 上复现过。

| # | 问题 | 修法 |
|---|---|---|
| P2-1 | **`validate_whitelist_against_disk` 遇到不存在的条目会整函数 `return Ok(())`**，把它**后面所有条目**的校验一并放行。反例 `["Missing/", "Code"]` 通过，而 `Code` 单独送进来是被拒的（漏尾斜杠 → 整棵树绕开扩展名过滤，正是上一轮 A2 要挡的东西）。「跳过这一条」写成了「跳过全部」 | 外层循环加标签，`return Ok(())` → `continue 'entries`。测试同时钉住三件事：后续条目的类型错误、大小写错误都要抓到，而缺失条目本身仍不报错 |
| P2-2 | **能力裁剪的另一半**：`confirm()` 里 worktree 那半接了 `capabilities.worktrees`（上一轮 A5），**远程那半只看 `info.upstream`**，`remote` 的构造完全不判能力。混合仓库的 git 侧留着 remote 与上游是当前打开校验允许的，于是**正常的删除确认流程**就能调到 `delete_remote_branch`。这不是 PR 已披露的「绕过 UI 直接 invoke」，是 UI 自己走进去的 | 把两个目标的构造下沉成纯函数 `src/engine/deleteBranchTargets.ts`，勾选框渲染也加上 `capabilities.remotes` 的门与一条说明。**不是补上漏掉的那半就完**——分散写两段三元表达式正是缺口成因，收到一处才有测试可写（6 条 vitest） |
| P2-3 | **SVN 把 target 里最后一个 `@` 之后的部分当 peg revision 解析，`--` 挡不住。** `Code/Team@Home` 是合法目录名、也过得了配置校验（`@` 不是通配符），但 `enumerate_wcs` 要发的两条命令**都**会失败 → 整个白名单条目不可用 | 新增 `peg_safe()`，两个 target 构造点都追加空 peg 后缀 `@`。递归发现的 external 从同一个 `pending` 出队，因此一并覆盖 |
| P2-4 | **`core.ignoreCase` 只认字面量 `"true"`，而 git 的布尔解析还接受 `1` / `yes` / `on`。** 设成 `1` 时 git 照常纳入 `Code/Loud.CS`，期望集合却按大小写敏感算、不含它 → 核对在一个完全正常的仓库上报「不该被跟踪、实际跟踪了」并**硬阻断** | 改用 `git config --bool --get`，让 git 自己归一。测试把 4 个真值 + 3 个假值全跑一遍 |

**实测记录（2026-09-14，临时 fixture，跑完即删）**

| 实测项 | 结果 |
|---|---|
| `svn info --xml -- Code/Team@Home` | exit 1，`E205000: Syntax error parsing peg revision 'Home'` |
| `svn status --xml -- Code/Team@Home` | exit 1，`E200009: a peg revision is not allowed here`（**和 info 不是同一个错**） |
| 两者追加尾缀 `@` | 全部 exit 0，`<url>` 里没有多出来的 `@` |
| **不含 `@` 的普通路径追加尾缀**（`svn info -- Code@`） | exit 0 —— 所以可以**无条件**追加，不必先探测 |
| `git config --bool --get`：`1` / `yes` / `on` / `True` | 全部归一成 `true` |
| 同上：`0` / `no` / `off` / `false` | 全部归一成 `false`；非法值 `banana` → exit 128、stdout 空 |
| `core.ignoreCase=1` 时 `git ls-files --others -- Code` | 列出 `Code/Loud.CS` —— 误报阻断成立 |

**顺带修入的一条 spec 对齐（不在 review 清单里）**：`blame_rule` 加 `--no-index`，
并让「多收」方向也指认规则。实测同一个文件（`Code/Junk.psd`，被 `/Code/**` 排除、
用 `git add -f` 强行入索引）：

| | 输出 | exit |
|---|---|---|
| 无 `--no-index` | `::<TAB>Code/Junk.psd`（**空裁决**） | 1 |
| 有 `--no-index` | `.git/info/exclude:3:/Code/**<TAB>Code/Junk.psd` | 0 |

即：最需要指认规则的那一侧（用户想知道「凭什么它进来了」），不带这个参数恰好
什么也指认不出来。原实现只在「漏收」方向调用 `blame_rule`，而那一侧的文件必然
未跟踪，所以**今天还没踩到**——但它是个哑火的防线。新版 spec §6.3.1 已写明要带。

#### 9.8 master 上的 spec 更新对本期的影响

本分支基于 `02f8d45`（已含第四轮 review 的 `150ddad`），master 此后只多了
**一个** spec 提交 `310c17d`。落在 Phase 1a 地盘的有三处：

| # | spec 改动 | 对本期的影响 | 处置 |
|---|---|---|---|
| 1 | **§6.3.1 的核对机制重写**：从「`git ls-files` 读出实际集合」改成「**临时索引** + `git add -A -- <W-path>` + 同一份索引 `write-tree` 得到 TREE，录制时直接用这个 TREE」，并明确「不得核对一次后又重新 `git add` 另录一棵树」 | `actual_set` 走的是**纯读模拟**（`--cached ∪ --others − --deleted`），是 Task 6 就写明的偏离。Phase 1a 不录基线，TREE 那条耦合还不存在，**今天不产生错误行为** | **本期不改**。理由：临时索引的唯一消费者是 1b 的录制流程，现在建等于把 1b 的基础设施提前塞进这个 PR。但**必须记账**——1b 建了临时索引之后，模拟版就是第二个会分叉的事实源，那时删掉它、核对与录制共用一份索引。B1 已经证明「模拟 `git add -A`」这件事很容易写错 |
| 2 | **§6.3.1 指定 `git check-ignore --no-index`** | 原实现没带，且只在「漏收」方向调用 | **已修入**，见 9.7 末尾 |
| 3 | **§6.3.2 的 W-path / W-file 表重写**：旧硬规则「构造 git 命令参数时用 W-file，构造 SVN 命令 target 时用 W-path」被推翻，改成「**范围由操作目的决定，不由命令叫 git 还是 svn 决定**」；W-file 的定义补上「永久排除任意深度的 `.svn/` 与 `.git/`」 | 本期代码**恰好仍然符合新表**：`svn_wc` 用 W-path 做 status 扫描（新表里这仍是 W-path），`actual_set` 用 W-path 当 pathspec（新表写明「候选索引可以用 W-path 递归执行 add」），`walk` 早就在任意深度跳过 `.svn` 与 `.git` | 不需要改代码。但**旧硬规则不能再被当成判据引用**——1b 写 `revert` / `commit` 的 target 时按旧规则推理会得出错误结论（新表明确要求 `revert` 只取 W-file 且已有备份的具体文件） |

§7.3.2（rebase 前置阻断）、§8（交付）、§9.4（恢复范围）的改动全部落在 Phase 1b/1c，
本期不涉及。

#### 偏离计划

- **多加了两条命令层测试**（`svn_backend.rs` 从 11 到 13，后又加 serde 那条到 14）：
  `the_read_capabilities_command_returns_what_the_backend_says` 与
  `the_probe_svn_environment_command_reports_one_of_three_states`。理由见下方
  「计划没写的坑」的第一条。

#### 计划的错误

计划的 Step 9.6 只写了「跑一次独立 review」，没写**跑完之后 9.1 的数字会变**——
这次修完 9 条发现，Rust 测试从 298 涨到 312，9.1 那张表里的每个数字都要重算一遍。
计划把 9.1 排在 9.6 之前，等于默认 review 不会带来代码改动。下次把「修完 review
之后重跑并更新 9.1」写成一个独立步骤。

#### 计划没写的坑

- **`probe_svn_environment` 在 Phase 1a 没有任何前端调用方。** 它是给 1b 的初始化
  向导准备的，本期只是注册上去。后果是**它的 invoke 路径没办法在真实窗口里验证**
  ——`AGENTS.md` §5.1 要求新命令必须实际跑一次 `pnpm tauri dev` 确认 invoke 真的通，
  而没有调用方就没有可点的入口。

  已补一条 Rust 测试直接调命令体，把三态契约钉住（前端要靠这三个字符串分流
  「去装 / 重启应用 / 正常」，塌缩成两态就等于把「装完了但应用没重启」错报成
  「没装」，spec §17.1）。但**那不能替代 IPC 验证**，如实记在这里：
  **`probe_svn_environment` 的 IPC 往返在本期属未验证**，1b 接上向导时必须补。

- **命令层测试证明不了 snake_case ↔ camelCase。** 那层转换发生在 Tauri 的 IPC
  边界上，Rust 测试直接调函数根本碰不到它。已写进测试注释，免得下一个人看到
  「命令有测试」就以为 §7.3 的要求满足了。

#### 为什么整期这么改是安全的

1. **本期没有任何 SVN 写操作** —— 9.3 的 grep 是证据，且正面列出了实际只发
   `svn info` 与 `svn status` 两条只读命令。

2. **普通 git 仓库的行为零变化。**
   - `detect_repo_kind` 在没有 `.svn` 时**第一个分支**就返回 `Git`，连配置文件都
     不去读；`backend_for` 走的还是原来那条 `GitBackend::new(path)`。
   - `GitBackend::capabilities()` 返回全 true，前端三个裁剪函数在
     `ALL_CAPABILITIES` 下的输出都有测试逐项断言与改动前一致。
   - 既有的 226 条 Rust 测试与 257 条 vitest **一条都没改行为**（只有
     `commitActions.test.ts` 的 7 处调用加了必填参数，断言全部原样）。

3. **唯一的行为变化是一条新增的阻断**（`.svn` + 无配置的 `.git` → `svn_unconfigured_git`），
   spec §6.1 明确要求，方向是「打不开」而不是「用错后端去写」。已写进
   `backend_for` 的文档注释。

4. **新增的两个命令都是只读的**：`read_capabilities` 只返回常量结构体，
   `probe_svn_environment` 只跑 `svn --version --quiet` 和一次 `reg query`。

5. **空列表在每一处都提前返回，不会退化成「全部」**（G6 / `AGENTS.md` §6.2）：
   - `svn_wc::enumerate_wcs`：空白名单直接返回空列表，一条 svn 命令都不发
   - `svn_include_check::actual_set`：空白名单直接返回空集，不发 `git ls-files`
   - `svn_include_check::ls_files`：参数里带 `--` 分隔符
   - `svn_config::normalize_whitelist_entry`：空串、纯空白、`.`、`./`、`..`、
     前导 `-`、`:` 前缀全部拒绝
   - 这三处各有测试，其中 `an_empty_whitelist_never_degenerates_into_everything`
     是在一个**确实有被跟踪文件**的仓库上断言的，不是恒真

6. **所有 fixture 都是临时目录里自建的**（`TempDir` + `git init` / `svnadmin create`），
   跑完即删，不碰本仓库、用户仓库或任何仓库外的固定路径。

7. **前端那层是表现层防线，不是接缝** —— 已在 `capabilities.ts` 的模块注释里
   明说。绕过 UI 直接 invoke 照样能打进那批直连 git 的命令。这一条是**已知缺口**，
   不是已解决的问题。

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

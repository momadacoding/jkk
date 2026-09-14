# Phase 1b 实现日志：建库与基线录制

> 配套 `2026-09-14-svn-phase1b-baseline.md`。按 `AGENTS.md` §2.1 建立。
> **review 请先读每个 Task 的「偏离计划」再看 diff**（§13 rubric #12）。

本文件只有两部分：**进度总账**（每完成一个 Task 立即更新）与**决策条目**
（按 Task 分节，四类缺一不可）。

---

## 进度总账

| Task | 状态 | 产出文件 | 验证结果 |
|---|---|---|---|
| 0 开工自检与基线数字 | ✅ 完成 | 本文件、`2026-09-14-svn-phase1b-baseline.md` | 改动前基线（新 worktree 内实测）：Rust **316 passed / 0 failed / 28 个二进制**、前端 `pnpm test` **272 passed / 21 files**、`pnpm build` ✓。与计划预期一致 |
| 1 `git_cli` 支持环境变量 | ✅ 完成 | `src-tauri/src/engine/git_cli.rs`、`src-tauri/tests/git_cli.rs` | `cargo test --test git_cli` 5 passed；全量 `cargo test -j 2` **318 passed / 0 failed / 28 个二进制**（= 基线 316 + 新增 2）；`cargo fmt --check` ✓、`cargo clippy --all-targets` ✓ |
| 2 `svn_xml` 三个新解析器 | ✅ 完成 | `src-tauri/src/engine/svn_xml.rs`、`src-tauri/tests/svn_xml.rs` | `cargo test --test svn_xml` 27 passed；全量 **332 passed / 0 failed / 28 个二进制**（= 318 + 新增 14）；`cargo fmt --check` ✓、`cargo clippy --all-targets` ✓ |
| 3 P1/P2/P3 硬门槛 | ✅ 完成 | `svn_preflight.rs`（新）、`svn_xml.rs`、`svn_wc.rs`、`engine/mod.rs`、`tests/svn_preflight.rs`（新）、`tests/common/svn_fixtures.rs` | `cargo test --test svn_preflight` 10 passed；全量 **342 passed / 0 failed / 29 个二进制**（= 332 + 新增 10）；`cargo fmt --check` ✓、`cargo clippy --all-targets` ✓；另做了一轮变异测试验证两条关键断言有牙 |
| 4 P5 externals 钉死检测 | ✅ 完成 | `svn_externals.rs`（新）、`engine/mod.rs`、`tests/svn_externals.rs`（新）、`tests/common/svn_fixtures.rs` | `cargo test --test svn_externals` 10 passed；全量 **352 passed / 0 failed / 30 个二进制**（= 342 + 新增 10）；`cargo fmt --check` ✓、`cargo clippy --all-targets` ✓；变异测试确认 `@N` peg 判据有牙 |
| 5 基线录制（临时索引 / trailer） | ✅ 完成 | `svn_baseline.rs`（新）、`svn_include_check.rs`、`engine/mod.rs`、`tests/svn_baseline.rs`（新）、`tests/svn_exclude.rs` | `cargo test --test svn_baseline` 13 passed、`--test svn_exclude` 25 passed；全量 **365 passed / 0 failed / 31 个二进制**（= 352 + 新增 13）；`cargo fmt --check` ✓、`cargo clippy --all-targets` ✓ |
| 6 §6.8 基线体检 | ✅ 完成 | `svn_health.rs`（新）、`svn_wc.rs`（`peg_safe` 文档）、`engine/mod.rs`、`tests/svn_health.rs`（新）、`tests/common/svn_fixtures.rs` | `cargo test --test svn_health` 15 passed；全量 **380 passed / 0 failed / 32 个二进制**（= 365 + 新增 15）；`cargo fmt --check` ✓、`cargo clippy --all-targets` ✓ |
| 7 初始化向导编排与回退 | ⬜ 未开始 | | |
| 8 重建基线 R1–R5 | ⬜ 未开始 | | |
| 9 Tauri 命令与向导 UI | ⬜ 未开始 | | |
| 10 收尾验证 | ⬜ 未开始 | | |

---

## 开工自检（`AGENTS.md` §4.2）

```text
- 当前分支 / worktree：worktree-svn-phase1b
    @ .claude/worktrees/svn-phase1b
- 当前分支是否已合并进 master：否，全新分支
- 是否允许在当前位置修改：是，独立 worktree，已用 EnterWorktree 会话级切入
- 目标 worktree 是否在改动前已创建并切入：是
- Get-Location 实测值：C:\Users\liuhao\workspace\dev\Dev\mygitclient\.claude\worktrees\svn-phase1b ✓
- 基点 commit：ca304d6（Phase 1a 合并进 master 的那个 squash commit）
    其父是 310c17d（spec 的最新修订），所以本 worktree 里的 spec 就是最新版
- 是否存在未提交改动：无（建完即查，干净）
```

**上一期的收尾状态**：Phase 1a 的 PR #41 已 squash 合并为 `ca304d6`。
`worktree-svn-phase1a` 那个 worktree 保留在磁盘上未删（其分支因 squash 合并不是
`master` 的祖先，`git branch -d` 会拒绝，删除需要用户单独授权）。

---

## 全局

### 本期的范围是用户在对话里定的

spec §16 的 Phase 1 是一大坨（向导、基线、体检、重建、失同步检测、SVN log、混合图、
本地补丁标记）。2026-09-14 用户选定切法：**本期 = 建库与基线录制**，剩下的「看得见」
那半（失同步横幅、SVN log、混合图、补丁标记）留 Phase 1c。

理由：录制机制（临时索引 → `write-tree` → `commit-tree` → 体检 → `update-ref`）被
首个基线和重建基线共用，拆开会让这段最不能写错的序列被抄两遍；而失同步检测要有基线
才有东西可比，天然排在后面。

### Phase 1a 遗留的窗口验证并进本期

1a 的两个命令（`read_capabilities` / `probe_svn_environment`）是在 `AGENTS.md` §5.1
要求的真实窗口验证**没有执行**的情况下合并进 master 的（PR #41 里那两个复选框一直
空着，没有谎报）。2026-09-14 用户确认并进本期一起验，清单见
`2026-09-13-svn-phase1a-foundation-decisions.md` 的 9.4 一节。

### 从 1a 记账过来的一条

1a 决策日志 §9.8 记了：新版 spec §6.3.1 把纳入集合核对改成「临时索引 + `git add -A`
+ 同一份索引 `write-tree`」，而 1a 的 `actual_set` 是**纯读模拟**（当时写明的偏离）。
当时判断「1a 不录基线，TREE 那条耦合还不存在，先不改」，并记账给 1b。

本期 spec 的 R2 **字面要求** `GIT_INDEX_FILE`，所以临时索引一定会建起来。届时模拟版
就是第二个会分叉的事实源——1a 的 B1 已经证明「模拟 `git add -A`」很容易写错。
**处置写在计划的 Task 5 Step 5：删掉模拟版，核对与录制共用同一个 `TempIndex`。**

---

## Task 0：开工自检与基线数字

**改动前基线（在本 worktree 内实测，不是抄上一期的）：**

| 项 | 值 |
|---|---|
| `cargo test -j 2` | **316 passed / 0 failed / 28 个二进制** |
| `pnpm test` | **272 passed / 21 files** |
| `pnpm build`（含 `tsc`） | 通过 |

三项与计划里写的预期一致，无需修正计划。

#### 偏离计划

无。Task 0 按计划执行。

#### 计划的错误

无。

#### 计划没写的坑

- **新 worktree 里 `node_modules` 不存在，必须先 `pnpm install`。** 实测
  `Test-Path node_modules` → `False`。1a 踩过一次：不装的话 `pnpm test` 会**静默跑
  主工作区**的测试（`@` 别名解析到主 checkout 的 `src/`），拿到一个看起来正常、
  实际测的是另一份代码的数字。计划的 Task 0 Step 2 只写了跑命令，没写这一步。
  **Task 10 的收尾验证同样适用**，换 worktree 之后要重来一次。

- **PowerShell 的 `Select-String` 默认大小写不敏感。** 统计失败行时用
  `-Pattern 'FAILED'` 会把 28 行 `0 failed;` 全部匹配进来，得到「30 个失败」
  这种吓人又错误的结论。核对测试结果时必须加 `-CaseSensitive`，或者直接判
  「非 `test result: ok.` 开头的行数」。本期后面每次统计数字都走后者。

- **`cargo test` 的正常输出里混着两行看着像错误的东西**，是 fixture 故意造的形态，
  不是失败：`error: cannot delete branch 'feat/busy' used by worktree at …`
  （断言 git 会拒绝删被检出的分支）与 `Automatic merge failed; fix conflicts`
  （冲突 fixture）。判读时不要被它们带偏——以 `test result:` 行和退出码为准。

#### 为什么这么改是安全的

Task 0 只读不写：跑测试、记数字、建两个 `docs/` 下的文件。没有任何代码改动，
也没有发出任何 git / svn 写命令。

---

## Task 1：`git_cli` 支持带环境变量执行

新增 `run_git_env(path, args, envs) -> Result<String>`，`GIT_INDEX_FILE` 有了合规入口。

#### 偏离计划

- **没有照计划那样把 `run_git_bytes` 的函数体抄一份**，而是抽出私有的
  `output_or_error(cmd) -> Result<Vec<u8>>`，让 `run_git_bytes` 与 `run_git_env`
  共用同一套失败映射。计划写的是复制粘贴版。

  原因：两处各写一遍 `if !output.status.success()`，只要有一处日后被改漏，那条路径
  就会把**失败当成空输出**。对本期而言这不是理论风险——基线录制里 `git add -A`
  一旦静默失败，`write-tree` 会照常成功并录出一个**残缺但看起来合法**的 tree，
  体检还未必拦得住（少的文件在 SVN 侧也不在 diff 里）。失败映射只留一份。

  等价性：`run_git_bytes` 改写前后逐字相同（`git_command(path).args(args).output()`
  → 同样的 cmd 交给 `output_or_error`），全量回归 318 passed 无变化。

#### 计划的错误

- **计划里的测试代码引用了三个不存在的 helper**：`repo_with_commit()`、
  `fx.path_str()`、`fx.path()`。计划在旁边挂了个 ⚠️ 说「先读 `fixtures.rs` 确认真实
  签名，不要照抄这里的名字」——这个提醒是对的，照抄会直接编译不过。实际情况是：

  1. `src-tauri/tests/fixtures.rs` **本身就是一个测试文件**，不是共享模块，里面只有
     私有的 `linear_repo() -> TempDir`，没有 `repo_with_commit`。
  2. 更要命的是 `tests/` 下**每个 `.rs` 文件都是独立 crate**，彼此 import 不到。本仓库
     共享 fixture 的唯一办法是 `#[path = "common/svn_fixtures.rs"] mod …`（见
     `tests/svn_wc.rs`）。所以「从 `fixtures.rs` 里拿 helper」这条路根本不存在。

  实际做法：按 `tests/tree.rs` 的既有写法，在 `tests/git_cli.rs` 内自建
  `fixture_git()` + `repo_with_commit()`。目前只有这一个文件需要 git fixture，还不值得
  为它建 `common/git_fixtures.rs`；等第二个文件也要时再抽。

- 计划的测试里 `run_git(...)` 是裸调用，实际必须写成 `git_cli::run_git(...)`——该测试
  文件只 `use app_lib::engine::git_cli;`，没有 glob import。

#### 计划没写的坑

- **fixture 自己发的 git 命令刻意不走被测的 `run_git`。** 用被测对象搭测试脚手架，
  它坏了脚手架跟着一起坏，断言就失去意义（`run_git` 挂了的话 `repo_with_commit`
  先 panic，根本走不到断言）。所以 `fixture_git` 是裸 `Command::new("git")`。

- **rustfmt 会因为中文断言消息把单行 `assert!` 拆成三行**（CJK 字符宽度算 2，单行超
  100）。1a 踩过一次，这次又踩。结论：改完 Rust 代码**先 `cargo fmt` 再
  `cargo fmt --check`**，别只跑后者然后对着 diff 手工排版。

- **`run_git_env` 走的是 `Command::output()`，子进程 stdin 是 null。**
  → **Task 5 必须用 `git commit-tree -m <msg> <tree>`，不能用管道喂 message。**
  走管道会立刻读到 EOF，`commit-tree` 照常成功，产出一个 **message 为空的基线提交**；
  而 trailer 就写在 message 里（spec §6.4.1），等于基线丢掉 `git-svn-id:`，
  `--grep` 再也找不回来，且**不报任何错**。这条现在记下，免得到 Task 5 现场才发现。

#### 为什么这么改是安全的

- 纯新增 + 一次等价重构：没有任何现有调用点的行为改变，`run_git` / `run_git_code`
  完全没动。全量 `cargo test -j 2` = 318 passed / 0 failed / 28 个二进制，
  相对基线 316 多的正好是本任务新增的两条测试，无回归。
- 新入口仍然经过 `git_command()`，`GIT_TERMINAL_PROMPT=0` 与 `hidden_command`
  两条防线都在（`AGENTS.md` §7.3 MUST：不许另起一套进程调用）。
- 两条新测试都在 `TempDir` 里 `git init` 自建仓库，跑完即删，没有对本仓库或任何真实
  仓库发出写命令（`AGENTS.md` 开篇最高优先级约束）。其中一条的断言本身就是
  「真实索引前后逐字相同」，等于把「不碰用户暂存区」这条固化成了回归测试。

---

## Task 2：`svn_xml` 三个新解析器

新增 `parse_status_entries` / `parse_externals_props` / `parse_diff_summary`。

**动手前先抓了一次真实 XML。** `tests/svn_xml.rs` 开头立的规矩是「输入必须是真实
svn 输出的形态，手写理想 XML 等于没测」，而计划里那六段测试 XML 是我写计划时手写的。
于是先在临时 fixture 上把三条命令的真实输出原样抓下来（跑完即删），再照抄进常量。
**抓回来的形状和计划里写的有四处不一样**，下面逐条记。

#### 偏离计划

- **`StatusEntry` 多了一个 `props` 字段**（计划声明的是 `{ path, item }`）。

  实测：只改属性、没改内容的节点是 `item="normal"` + `props="modified"`
  （文本版 status 显示成第二列的 `M`）。计划的结构体只带 `item`，P1 拿到的就是一个
  读起来完全干净的 `normal` 节点，于是**带着一处未提交的属性改动去录基线**。
  `svn_xml` 是全项目唯一接触 XML 的地方，这里丢掉的字段下游再也拿不回来。

- **entry 底下缺 `<wc-status>` 时报错，而不是跳过。** 计划的实现要点没提这件事，
  按最自然的写法（`find(...)` 拿不到就 `continue`）会静默少一条。少一条的含义是
  「这个节点不脏」，而 P1 的全部价值就在于没漏。最现实的触发方式是喂错输入——
  `svn info --xml` 的 `<entry>` 也叫 entry，底下却没有 wc-status。

- **`parse_diff_summary` 的缺字段策略与 `parse_info` 相反**（填空串而不是报错），
  计划就是这么写的，但没写为什么可以。补上理由：体检判「没有差异」的条件是
  `item == "none" && props == "none"`，**空串两个都不等于**，于是缺字段的条目被
  当成「有差异」保守报出来。空串在这里是安全方向，在 `parse_info` 里不是
  （空 uuid 会让 P4 的「所有 UUID 相同」在两个空串上通过）。

#### 计划的错误

- **计划 Task 2 的 status 测试 XML 里一条 `item="external"` 都没有，而真实输出里
  有一大把——即使带了 `--ignore-externals`。** 实测那次挂了五条 external，
  `svn status --no-ignore --ignore-externals --xml` 原样回了五条
  `item="external"` 的 entry。

  **这条直接决定 Task 3 的 P1 写法。** 计划里 P1 写的是「不允许存在 `M`/`A`/`D`/
  `R`/`C`/`!`/`~`/`?` 任何一种」，照字面最容易写成「status 有输出即脏」——而本设计的
  核心场景恰恰是**代码就在 external 里**，那样写向导在目标仓库上**永远过不去**，
  且失败信息会指着五个完全正常的 external 根说「工作副本不干净」。

  处置：解析层原样给出（不偷偷过滤，否则「external 根自己被 `svn add` 了」这种真脏
  也会被吞），并且把这件事同时写进 `parse_status_entries` 的 doc 注释和一条名叫
  `status_entries_keep_external_markers_because_they_are_not_dirt` 的测试里。
  **Task 3 的 P1 必须显式剔除 `item == "external"`。**

- **计划说 `--summarize --xml` 的 `item`/`props`/`kind` 是三个独立属性——对，但漏了
  一件事：属性顺序在同一次输出里就不一致。** 实测同一次输出的前两条是
  `kind, item, props`，第三条是 `item, props, kind`。计划给的示例 XML 三条顺序统一，
  照那个形态写测试就测不出「按序取值」这个错法（第三条会被读成 `item="dir"`）。
  常量里原样保留了顺序差异，并单列一条 `diff_summary_does_not_depend_on_attribute_order`。

#### 计划没写的坑

- **`item="added"` 的 entry 带 `revision="-1"`，`item="unversioned"` 则完全没有
  revision 属性。** 前者有后果：`parse_node_revs` 只跳过「没有 revision 属性」的
  entry，`-1` 会被正常收进集合。也就是说一个只有本地新增文件的工作副本，
  `nodeRevs` = `{-1, 当前rev}` → 多值。

  → **Task 3 的 `run_preflight` 必须 P1 先于 P2**。顺序反了的话，用户看到的是
  「工作副本是 mixed-revision，请先跑一次完整 svn update」——而真实原因是他有个
  没提交的新文件，那句引导做了也没用。顺序不是风格问题，是错误信息对不对的问题。

- **`svn propget svn:externals -R --xml` 的 `<target path>` 是绝对路径 + 正斜杠，
  而 `svn status --xml` 的 `path` 是相对 + 反斜杠。** 计划的 E7 记了这件事，但值得
  在这里重申：同一个 CLI 两种写法，不是笔误。三个解析器的归一策略做成了一张表写进
  `StatusEntry` 的 doc 注释，免得下一个人以为 externals 那个是漏了归一。

- **没有任何 externals 的子树，`propget` 回的是空的 `<properties>`，退出码 0。**
  不是报错、也不是空输出。空列表是正常结果。

- **`propget svn:externals` 只会回这一个属性**，所以「只取 `name="svn:externals"`」
  那条守卫在真实输出里根本不会被触发。它防的是将来有人改成 `proplist -v --xml`。
  这条分支只能用合成 XML 覆盖，测试里写明了它是合成的以及为什么。

#### 为什么这么改是安全的

- 三个函数都是**纯新增**，没有改动 `parse_info` / `parse_node_revs` /
  `parse_external_roots` / `assert_externals_were_ignored` 任何一行。全量
  `cargo test -j 2` = 332 passed / 0 failed / 28 个二进制，相对 Task 1 的 318 多的
  正好是本任务新增的 14 条，无回归。
- 全部是「喂字符串、吐结构体」的纯函数，不发任何命令、不碰文件系统。
- 唯一跑真实 svn 的测试 `externals_props_survive_real_svn_output` 复用 1a 已有的
  `svn_repo_with_cross_repo_nested_external()`，在 `TempDir` 里自建仓库、跑完即删，
  只发 `propget`（只读）。抓形状用的探针脚本也全程在 `%TEMP%` 的一次性 fixture 上，
  `finally` 里 `Remove-Item -Recurse`，没有碰本仓库或任何真实 SVN 工作副本。

---

## Task 3：P1 / P2 / P3 三道硬门槛

新增 `engine/svn_preflight.rs`，对外只有
`run_preflight(repo_root, cfg) -> Result<Preflight>`。

### 一条曾经走错、已撤销的收窄（记在这里防止再犯）

我一度把 P1 对 `ignored` / `unversioned` 的判据收窄成「只有落在 W-file 集合里的
才算脏」，并在报告里用「`Code/bin`、`Code/obj` 这类构建产物几乎必然存在」来论证。
**那个目录结构是我凭空设想的，不来自本仓库、spec 或用户的任何输入。** 用户当场
指出不存在这种结构。

撤销，改回 spec 字面：**`ignored` 与 `unversioned` 一律算脏**。

两个理由，都不依赖对目录结构的任何假设：

1. spec §6.5 的禁止清单里 **`?` 是明写的**。收窄 `unversioned` 不是「按 spec 的理由
   解释 spec」，是直接和明文冲突——我当时把这一点混过去了。
2. 风险方向本来就是这边对。**误拦看得见**：错误信息直接列出路径，用户当场能判断
   是不是误报，代价是一次可见的失败。**放行看不见**：一个 SVN 不管的文件进了基线，
   之后每一次失同步判定、每一次交付清单都建立在它之上，而基线本身一切正常。
   在「看得见的误拦」与「看不见的错基线」之间，只能选前者。

对应的 fixture 与测试也一起改了：`svn_repo_with_ignored_build_output` →
`svn_repo_with_svn_ignored_non_code_file`（`svn:ignore Notes.txt` + 造出该文件），
测试从「必须放行」翻成
`p1_rejects_svn_ignored_files_outside_the_extension_list_too`「必须拦住」。
它和 `p1_must_pass_no_ignore_or_svn_ignored_files_slip_through`（`Generated.cs`，
匹配扩展名）成对，把「不按扩展名区别对待」这条钉死。

**教训**：给设计找理由时，只能用仓库里读得到的、spec 写明的或实测出来的事实。
编一个听起来合理的场景，会把一个本来该问用户的选择伪装成一个已经论证过的结论。

#### 偏离计划

- **`WcIdentity` 加了 `wc_root_abspath: Option<String>`；`svn_wc::peg_safe` 改成
  `pub`。** P3 要查的是**工作副本根**，而 `enumerate_wcs` 里 `parse_info` 早就把
  `<wcroot-abspath>` 解析出来了、然后丢在地上。捡起来是零额外命令；不捡的话每条
  白名单要多发一次 `svn info`。`WcIdentity` 只有一个构造点（`enumerate_wcs`），
  测试里没有结构体字面量，加字段不破坏任何调用方。

- **`StatusEntry` 又加了一个字段 `wc_locked: bool`**（Task 2 刚定的结构体）。
  理由同 `props`：`svn_xml` 是唯一接触 XML 的地方，这里不取，P3 就拿不到。

- **执行顺序不按 P1→P2→P3→P4 的编号，而是
  枚举 → P4 → P3 → P1 → P2。** 理由写在 `run_preflight` 的 doc 表格里。
  其中 **P1 必须先于 P2** 是 Task 2 实测出来的硬约束（`added` 节点带
  `revision="-1"`，会进 `nodeRevs` 把它变成多值 → 用户拿到一条做了也没用的引导）。

- **P3 没有走计划给的降级退路。** 计划写着「造一个中断的 update 很难，若确认造不出
  稳定 fixture，就降级为读 `svn status` 是否报 `L`（locked）并如实写明覆盖范围」。
  实测既造得出来，而且**证明了只读 `L` 是不够的**（见下）。所以没降级。

#### 计划的错误

- **Step 1 的测试代码编译不过。** 原文是：

  ```rust
  let err = run_preflight(...).unwrap_or_else(|e| panic!(...)).err().unwrap_or_else(...)
  ```

  对一个 `Result` 先 `unwrap_or_else` 拿到 `Preflight`，再对它调 `.err()` ——
  类型就不对。实际改成 `let Err(err) = run_preflight(...) else { panic!(...) };`。

- **Step 2 的 P2 测试自相矛盾。** 它同时断言「`nodeRevs` 多值，P2 必须拒绝」和
  `assert_ne!(root_rev, 3)`（潜台词是 `nodeRevs == {3}` 单值）。这是**两个不同的
  形态，需要两个 fixture**，一个计划里的测试同时做不到：

  | fixture | 造法 | 实测结果 | P2 应当 |
  |---|---|---|---|
  | `svn_repo_mixed_revision` | 整体 r3，`Code/A.cs` 单独 `update -r1` | rootRev=3，nodeRevs={1,3} | **拒绝** |
  | `svn_repo_root_rev_behind` | `checkout -r1`，只 `update Code` | rootRev=1，nodeRevs={3} | **放行**，且 `node_rev` 必须是 3 |

  后者才是 spec §6.5.1 那个陷阱的真实形态（「照 rootRev 取 rev，初始化完成的那一
  瞬间就判失同步」），也是更有价值的那条。拆成了两条测试。

#### 计划没写的坑

- **`svn cleanup` 清掉写锁，但清不掉 `incomplete`。** 这是 P3 的全部要害。
  2026-09-14 实测（Windows 上用 `FileShare::None` 独占占住一个待更新的文件，
  让 `svn update` 中途失败）：

  | 状态 | 工作副本根 `--depth=empty` | 白名单根 `Code@` |
  |---|---|---|
  | 干净 | 无 entry | 无 entry |
  | update 半路失败 | `wc-locked="true"` + `item="incomplete"` | 只有 `wc-locked` |
  | **跑完 `svn cleanup`** | **仍是 `item="incomplete"`** | **一条 entry 都没有** |
  | 再跑一次完整 update | 无 entry | 无 entry |

  第三行是最现实的场景：update 失败 → SVN 提示 run cleanup → 用户照做 → 以为好了。
  **那时 P1 全绿、P2 单值、白名单根 status 完全干净**，只有工作副本根还写着
  incomplete。所以 P3 必须（a）查工作副本根而不是白名单根，（b）`wc-locked` 和
  `incomplete` 两个都查。计划给的降级方案「只读 `L`」恰好在这个场景下失效。

- **造「update 半路失败」的稳定办法**：Windows 上
  `OpenOptions::new().share_mode(0).open(victim)` 独占占住一个待更新的文件，
  `svn update` 走到那一项必然失败（`E155009: Failed to run the WC DB work queue`
  + `E720005: Can't move … Access is denied`）。不依赖时序、不用杀进程，每次都复现。
  这个 fixture 是 `#[cfg(windows)]` 的，与 `tests/fixtures.rs` 里
  `make_dangling_link` 的做法一致。

- **P2 的错误信息必须按「白名单是否落在 external 内」分两种说法。** 计划的 ⚠️ 提到了，
  这里记下判据：`WcIdentity.wc_root_abspath` 与仓库根不同即说明落进了 external。
  落在 external 里时「先跑一次完整 svn update」是一条**永远无效**的引导。

- **我这一轮先写测试后写实现，中间没跑红**——十条测试第一次跑就全绿，这本身是个
  不可信的信号。补做了一轮**变异测试**：
  - 把 `item == "incomplete"` 那条判据短路掉 → 只有
    `p3_still_rejects_after_the_user_ran_svn_cleanup` 失败，
    `p3_rejects_a_working_copy_left_mid_update` 仍然通过（它被 `wc_locked` 兜住）。
    这正好证明「cleanup 之后那条」是不可替代的。
  - （当时还有一条针对上面那个已撤销收窄的变异，随收窄一起删了。）

  变异只打掉对应的那一条，别的全绿。变异已撤销，撤销后全量 342 passed。

#### 为什么这么改是安全的

- **本任务不发任何写命令。** `run_preflight` 只跑 `svn info` 和 `svn status`
  （三种参数组合），全是只读；没有 `update`、`revert`、`cleanup`、`commit`。
  spec §6.5 明确要求「不自动规范化——revert 用户的本地修改是破坏性的」，
  实现里没有任何一条会改动用户工作副本的路径。
- 新 fixture 全部在 `TempDir` 里 `svnadmin create` 自建仓库，跑完即删；
  `svn_repo_interrupted_update` 会**故意**让一次 update 失败，但那是在自己刚建的
  一次性工作副本上，且 `assert!` 了「失败」正是预期（成功反而是 fixture 前提不成立）。
- 对 1a 既有代码只做了加法：`WcIdentity` 加字段、`peg_safe` 提升可见性、
  `StatusEntry` 加字段。没有改动任何既有函数的行为。全量
  `cargo test -j 2` = 342 passed / 0 failed / 29 个二进制，相对 Task 2 的 332
  多的正好是新增的 10 条，无回归。

---

## Task 4：P5 —— `svn:externals` 钉死检测

新增 `engine/svn_externals.rs`：`parse_external_line`（纯函数）+ `detect_pinned`。
**全模块不返回阻断性错误**——P5 命中只是切到 pinned 模式（spec §6.5.3）。

#### 偏离计划

- **`detect_pinned` 的过滤条件做成了双向的。** 计划和 spec 的说法都是「白名单
  **所在**的 external」（external 包住白名单）。实现里额外收了反方向：白名单包住
  external 的情况也报。

  理由：那条 external 的内容一样会进基线（白名单覆盖到它），一样会被项目根的递归
  update 退回 peg（§6.5.3 测点 C），一样需要 pinned 模式的两步 update 显式点名它。
  只收单向会在「`Code/` 底下挂着一条被钉死的 external」这种形态下漏掉。
  与白名单**无交集**的 external 仍然不报——报了只是噪音，还会让用户以为得去改一个
  跟自己无关的定义。

- **计划的 Interfaces 里 `parse_external_line` 返回 `Option<ExternalDef>`，但
  `ExternalDef` 本身没定义。** 补上了：`{ local_path, url, pinned_rev }`。
  `url` 是剥掉 peg 后缀的原文，`local_path` 正斜杠归一。

#### 计划的错误

无。Task 4 的判据（`-r <N>` 记号**或** URL 带 `@<N>`）和五种写法的实测结论都对得上。

#### 计划没写的坑

- **`-r` 与 `@` 同时出现时取哪个。** `-r` 是 operative revision、`@` 是 peg，
  内容来自前者，所以 `operative.or(peg)`。计划只说了「二者有其一即算钉死」，
  没说都有时报哪个。补了一条测试 `an_operative_revision_wins_over_the_peg_...`。

- **不能一见 `@` 就当 peg。** `^/team@home/lib` 是合法路径；按「有 `@` 就是 peg」
  写会把 URL 截断成 `^/team`，还凭空报出一条「钉死」。判据必须是「最后一个 `@`
  之后**全是数字**」。这和 1a 的 `peg_safe` 是同一族坑的两面：那边是「`@` 会被
  SVN 当 peg 解析，所以要加空后缀」，这边是「不是每个 `@` 都是 peg」。
  空 peg 后缀（`^/lib@`）也顺带处理成「不是钉死」。

- **哪个 token 是 URL 只能看长相，不能看位置。** 新旧格式字段顺序相反且可以混在
  同一个属性值里。实现用 `looks_like_url`（`^/` / `//` / `/` / `../` / 含 `://`），
  并要求去掉 `-r N` 之后**正好剩两个 token、且其中正好一个像 URL**；
  两个都像或都不像就返回 `None`，不猜。

- **`propget svn:externals -R` 必须从仓库根跑，不能只查白名单子树。** 要找的那条
  external 定义在白名单**上方**（`Code` 本身就是一条 external 时，定义在根上）。
  代价是它会遍历整个工作副本；这是一次性的向导步骤，可接受。

- **fixture 里 `svn update` 会失败，且是预期内的。** 五行定义里
  `pinD -r1 ^/lib` 是旧格式配 `^/` 相对 URL，spec §6.5.2 实测说这种签出直接失败。
  所以 fixture 用 `try_svn` 发这条 update 并忽略结果——检测走的是**属性文本**，
  不是磁盘上签出来的东西，那条失败不影响被测行为。
  用 `run`（断言成功）的话整个 fixture 会 panic。

- **变异测试**：把 `split_peg` 的 peg 分支短路成 `None` →
  只有 `all_five_pinning_forms_are_detected` 和
  `detect_pinned_reports_the_four_pinned_ones_and_not_the_following_one` 失败，
  `an_operative_revision_wins_over_the_peg_when_both_are_present` 仍通过
  （它靠 `-r2`，不靠 peg）——三条的分工确实是分开的。变异已撤销。

#### 为什么这么改是安全的

- **只发一条只读命令**：`svn propget svn:externals -R --xml`。没有 `propset`、
  没有 `update`、没有任何会改动工作副本或仓库的调用。
- **本模块不返回阻断性错误**，符合 spec §6.5 对 P5 的定性（「命中不阻断」）。
  唯一的 `Err` 来源是 `run_svn` 本身失败或 XML 解析失败。
- 新 fixture `svn_repo_with_five_external_forms` 在 `TempDir` 里自建仓库，跑完即删；
  写 externals 属性用 `std::fs::write` 写死 `\n`，避开 E10 那个「混合行尾 →
  `E135000` → 属性静默没设上 → 测试假绿」的坑，而
  `detect_pinned_reports_the_four_pinned_ones...` 这条测试本身就是该坑的守卫
  （属性没设上时它返回空列表，测试立刻红）。
- 纯新增模块，没有改动任何既有代码。全量 `cargo test -j 2` = 352 passed /
  0 failed / 30 个二进制，相对 Task 3 的 342 多的正好是新增的 10 条，无回归。

---

## Task 5：基线录制 —— 临时索引、trailer、`commit-tree`

新增 `engine/svn_baseline.rs`（`TempIndex` / `Trailer` / `publish_baseline`），
并按计划把 `svn_include_check` 的纯读模拟删掉。

动手前先把五件不确定的 git 行为在临时 fixture 上实测掉（跑完即删），**没有一条是
照文档猜的**：

| # | 实测项 | 结果 |
|---|---|---|
| B1 | `GIT_INDEX_FILE` + `read-tree HEAD` 起步 | 成功；真实索引、HEAD、工作目录全不变 |
| B2 | HEAD 未出生时 `read-tree HEAD` | **exit 128**，`fatal: Not a valid object name HEAD` |
| B3 | `commit-tree <tree> -m <多行 message>` | 接受；`%B` 逐字读回，末尾补一个换行 |
| B4 | `update-ref <ref> <new> <40个零>` | ref 不存在 → **exit 0**；ref 已存在 → **exit 128**（`reference already exists`）且 ref 不动 |
| B5 | `update-ref` 旧值不符 | **exit 128**，`is at X but expected Y`，**ref 未被覆盖** |
| B6 | `rev-parse -q --verify <不存在的 ref>` | exit 1，stdout 空 |

#### 偏离计划

- **「多收」的检测路径变了，计划没提这件事，第三条测试也得跟着改造。**

  计划的 Step 5 只说要改写两条等价性测试。实际跑下来红的是第三条：
  `the_mismatch_report_names_the_rule_for_files_that_are_tracked_but_should_not_be`。

  原来它往**用户的真实索引**里 `git add -f -- Code/Junk.psd` 制造多收，1a 的纯读
  模拟通过 `--cached` 能读到。新模型下候选索引从空索引（或 HEAD 树）起步，
  用户暂存区里那一笔根本进不来——**而且确实不该进来**：它永远到不了基线。

  新模型下多收的真实路径是「**HEAD 树里已经躺着**一个不该被跟踪的文件」
  （比如以前被 `add -f` 之后提交了）：`from_head` 把它带进候选索引，
  `add -A` 不会删掉它（磁盘上还在），于是它一路进到新基线里。测试改成这个形态。

  这是**精度提升不是覆盖损失**：核对的对象从「用户的临时暂存区」变成了
  「真正会被录进基线的那份索引」。

- **改写后的两条等价性测试仍然不是恒等式**，值得说清楚。计划说「断言目标仍然成立」
  ——对，但要补一句为什么：候选索引是「从**空**索引 + `-A -- Code`」，对照组是
  「在**用户已有的**索引上 `git add -A`（整个仓库）」，两条路径的起点和 pathspec
  都不同，相等是一个实打实的结论。若两边都写成「临时索引上 `add -A`」，那才成了
  自己比自己。

- **`TempIndex` 加了 `#[derive(Debug)]`**：`Result::expect_err` 要求 `T: Debug`。

- **多写了 7 条计划没列的测试**，都对着实测来的坑：
  `two_temp_indexes_do_not_collide`、`from_head_starts_from_the_head_tree_not_the_
  users_staging_area`、`from_head_on_an_unborn_head_is_a_clear_error_not_an_empty_
  index`、`add_all_with_an_empty_whitelist_adds_nothing`、
  `the_url_may_itself_contain_an_at_sign`、`a_crlf_message_still_parses`、
  `publishing_a_first_baseline_refuses_when_one_already_exists`、
  `recording_the_same_tree_again_is_legal`。

#### 计划的错误

无。这一节的测试代码是对着 B1–B6 的实测写的，计划给的骨架直接可用。

#### 计划没写的坑

- **`commit-tree` 的 message 必须走 `-m`，不能走 stdin。** Task 1 就记过账，这里
  兑现：`run_git*` 用 `Command::output()`，子进程 stdin 是 null，管道那条路会立刻
  读到 EOF 并产出一个 **message 为空**的 commit——而 trailer 就写在 message 里，
  等于基线丢掉全部记账，且**不报任何错**。B3 确认 `-m` 能带多行。

- **`rev-parse -q --verify` 的退出码 1 和 128 必须分开。** 1 是「ref 不存在」，
  128 是「ref 损坏」。把 128 也当成「不存在」，`publish_baseline` 会以为这是首个
  基线，然后用 `expected_old = None` 那条路**覆盖掉一段真实历史**。
  同一个坑 1a 在 `engine::ops::head_is_unborn` 上踩过（那次的后果是删用户的文件）。

- **trailer 里的 URL 自身可能含 `@`。** `svn+ssh://user@host/repo` 是完全正常的
  写法。按第一个 `@` 切会把 rev 读成 `host/repo@12346`、解析失败，整条基线被判成
  「不是 baseline」。spec 的正则 `(\S+)@(\d+)` 里 `\S+` 是贪婪的，等价于取**最后
  一个** `@`——手写解析必须用 `rsplit_once`。

- **commit message 可能带 CRLF。** 经不同工具往返时会带上 CR，正则的 `$` 对不上
  就凭空少一条基线。解析前先 `trim_end`。

- **临时索引的位置问 `git rev-parse --git-dir`，不要拼 `repo_root/.git`。**
  worktree 里 `.git` 是个**文件**不是目录，拼出来的路径写不进去。放 `.git/` 里
  还有两个原因：与对象库同一个文件系统；不会出现在工作目录里被 TortoiseSVN 看见。

- **`Drop` 里不能 panic。** 展开途中再 panic 会直接 abort——在 Tauri 里就是整个
  应用凭空消失。删不掉临时索引文件是可容忍的，把应用干掉不是。

- **文件名唯一性用 pid + 纳秒 + 进程内原子序号。** 只用纳秒不够：Windows 的时钟
  粒度约 100ns，同一个流程里「先核对后录制」两次构造可能落在同一个刻度上，
  撞名的后果是第二份索引从第一份的内容起步。

#### 为什么这么改是安全的

- **「不碰用户的任何东西」这条被三重断言钉死**
  （`recording_touches_neither_the_real_index_nor_head_nor_the_worktree`）：
  录制前后 `ls-files -s` 逐字节相同、`rev-parse HEAD` 的退出码与输出相同、
  工作目录里那个不该被收的 `Code/B.psd` 仍在。B1 的实测和这条测试是两件事，
  前者证明「git 做得到」，后者证明「我们的实现做到了」。
- **本任务新增的写操作只有四类**：写临时索引文件（`.git/` 内，`Drop` 清理）、
  往对象库写 blob/tree/commit（`add` / `write-tree` / `commit-tree`，只增不改）、
  更新 `refs/heads/svn/base`（**带旧值**）。没有 `checkout`、`reset`、`clean`、
  `rm`、`branch -D` 中的任何一个。
- **`update-ref` 失败时不会丢东西**：commit 对象已经写进对象库了，错误信息里带着
  它的 OID（`已经建好的 commit 是 …，对象还在，没有丢`），最坏是一个孤儿对象，
  等 `gc` 回收。
- **`add_all` 的空 pathspec 是 no-op**，不会退化成「收整个仓库」——那会把任意深度
  的 `.svn/` 收进 git，而那玩意被跟踪之后一次 checkout 就能毁掉一个正在使用的
  工作副本（spec §6.3）。有专门的测试守着。
- 删掉纯读模拟属于**减少**事实源：原来「核对读的集合」与「录制写的树」是两条独立
  推导出来的东西，现在是同一份索引的两次读取。全量 `cargo test -j 2` = 365 passed /
  0 failed / 31 个二进制，相对 Task 4 的 352 多的正好是新增的 13 条，无回归。

---

## Task 6：§6.8 基线体检

新增 `engine/svn_health.rs`：`check_baseline` + `Health::{Verified, Lying, NotChecked}`。
spec §6.8.2 的 T1–T7 逐条在临时 fixture 上复现，七条结论与 spec 记的完全一致。

#### 偏离计划

- **白名单根不过 `peg_safe`，与计划的 Interfaces 相反。** 计划写着
  「Consumes: … `svn_wc::peg_safe`」。实测这条会让命令直接失败，见下。

- **多发一条 `svn info` 核对 `svnUrl`。** 计划没有这一步，是写测试时发现的
  真实误报路径，见下。

#### 计划的错误

- **`svn diff` 的尾随 target **不接受**空 peg 后缀。**
  2026-09-14 实测：`svn diff --old=… --new=. --summarize --xml -- Code@` 报
  `E155010: The node '…\wc\Code@' was not found` —— 带 `--old` / `--new` 时尾随
  target 是**纯路径**，`@` 被当成字面字符，不走 peg 解析。

  这推翻了 1a 给 `peg_safe` 写的那句「不含 `@` 的普通路径追加尾缀同样合法，
  无条件追加因此没有代价」——那条结论是在 `info` / `status` 上验的。
  已把这个例外补进 `peg_safe` 的 doc 注释。

  连带的**已知限制**：白名单路径含 `@` 时，`enumerate_wcs` **必须**加后缀而
  `svn_health` **必须不加**，两边都能工作的写法目前不存在，所以那种路径没法做体检。
  记在 `check_baseline` 的 doc 注释里。

#### 计划没写的坑

- **`svnUrl` 配成仓库子路径时会产出一份看起来完全合理的 `Lying` 清单。**
  这是写 `a_url_that_does_not_match_the_output_is_not_checked_rather_than_lying`
  这条测试时撞出来的：我原以为「前缀剥不掉」能拦住它，实测**前缀照样剥得掉**
  ——输出路径就是 `<svnUrl>/<target>/…`，剥掉 `<svnUrl>/` 正好得到 `Code/A.cs`。
  于是 `--old` 那侧指向一个不存在的 `<svnUrl>/Code`，白名单里每个文件都被报成差异。

  按 spec §6.8.6，初始化时 `Lying` 会触发**整个回退**。把一处配置错误变成
  「基线撒谎」是这条检查最坏的失败形态——比漏报糟得多，因为用户完全无从下手。

  处置：跑 diff 之前先 `svn info` 核对 `svnUrl` 与工作副本根的实际 URL，
  对不上返回 `NotChecked` 并说清楚该填什么。测试从「断言剥不掉前缀」改成
  「断言这条真实误报路径被拦住」。

- **命令失败时 svn 仍然往 stdout 打出部分 XML。** 实测 URL 不可达时 stdout 里
  有 `<?xml …?>` 和 `<diff><paths>`，错误只在 stderr 和退出码里。
  「解析得出来就算成功」的实现会在断网时拿到一个空的 paths 列表，于是
  **报告基线健康**——这条检查等于不存在。专门写了一条测试守它，并配了
  「同一个工作副本、同一处真实差异、只把 URL 指歪」的对照组，
  证明 `NotChecked` 不是因为别的原因静默的。

- **`rev` 大于服务器 HEAD 时是 exit 1 + `E160006: No such revision`**，
  同样归 `NotChecked`（最可能的成因是 `svnUrl` 配错或指向了另一个仓库），
  不是「基线撒谎」。

- **W-file 判定必须是纯路径判断，不能走磁盘。** `expected_set` 遍历磁盘，
  而 T7 那类差异（服务器上删掉了 `Code/Deep/D.cs`）的文件在磁盘上根本不存在，
  走磁盘的判断会把它整个漏掉——那正是 §9.4.1 那类漏检要抓的东西。

- **`Code/Art.psd` 确实会出现在输出里。** 计划和 spec 都说了要按扩展名剔除，
  这里补一条实测确认：白名单根**底下**的非 W-file 文件被改过时，
  `svn diff --summarize` 会报它。T3（白名单**外**的美术文件）和 T3b（白名单
  **内**但扩展名不匹配）是两条不同的路径，各写了一条测试。

- **空白名单返回 `Err` 而不是 `Verified`。** 不传 target 时 `svn diff` 会拿整个
  工作副本去比——对 Unity 工程是几个 GB，且任何一处正常的本地改动都会报成撒谎。
  返回 `Verified` 更糟：等于宣布一个空白名单的基线是健康的。

#### 为什么这么改是安全的

- **全部是只读命令**：`svn info --xml` 与 `svn diff --summarize --xml`。
  没有 `update`、`revert`、`commit`、`propset`。
- **三种结果严格分开**，且默认方向是保守的：任何跑不成的情况都落到 `NotChecked`
  （不阻断、如实显示「未经体检」），只有「命令成功且过滤后仍有 W-file 内容差异」
  才是 `Lying`。这与 spec §6.8.6 的三行表逐行对应。
- **两条「不误报」测试（T3 / T3b）和一条「配置错误不当成撒谎」测试**共同守着
  这条检查最危险的方向。误报会让一个完全正常的仓库永远建不起来，而且按 spec
  它会触发整个初始化回退。
- 纯新增模块 + 一处 doc 注释修正，没有改动任何既有函数的行为。全量
  `cargo test -j 2` = 380 passed / 0 failed / 32 个二进制，相对 Task 5 的 365
  多的正好是新增的 15 条，无回归。

---

## 记账给后面（Task 4 遗留）

**spec §6.5.2「手段二：行为校验」本期不做。** 它要在一次真实 `svn update` 成功
之后比对「白名单所属 WC 的 revision」与「该仓库当时的 HEAD」，而本期根本不发
`update`（那是 §7.3 的 S5，属同步流程）。spec 说两条手段「都要实现」，所以这是一处
**有意的未完成**，不是遗漏：手段一负责阻断/判定，手段二负责兜住手段一的盲区。
已写进 `svn_externals.rs` 的模块注释，记账给 Phase 2。

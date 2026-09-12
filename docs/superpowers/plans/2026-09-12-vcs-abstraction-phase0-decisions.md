# VCS 抽象层 Phase 0 决策日志

计划：`docs/superpowers/plans/2026-09-12-vcs-abstraction-phase0.md`
规格：`docs/superpowers/specs/2026-09-12-svn-git-hybrid-design.md`
分支：`worktree-vcs-phase0`（worktree：`.claude/worktrees/vcs-phase0`）
基点：`b5f7f7c`

## 进度总账

**重构前测试基线**（2026-09-12，worktree 内实测）：`cargo test` **200 passed / 0 failed**，
21 个测试二进制。Task 6 拿它对账，最终为 **219**（16 个等价性测试 + review 修复带来的
3 个回归测试）。

> 注：输出里那行 `error: cannot delete branch 'feat/busy' used by worktree` 是
> `worktree_ops.rs` 某条失败路径测试的**预期 stderr**，不是测试失败——21 个二进制
> 全部报 0 failed。

| Task | 状态 | 产出文件 | 验证结果 |
|---|---|---|---|
| 1 只读四件套 | ✅ 完成 | `vcs/mod.rs`、`vcs/git.rs`、`tests/vcs_backend.rs`、`lib.rs`、`commands/{graph,diff,repo}.rs` | fmt ✓ clippy 零告警 ✓ `cargo test` **206 passed / 0 failed**（= 基线 200 + 新增 6） |
| 2 discard 下沉 | ✅ 完成 | `engine/ops.rs`（新建）、`engine/mod.rs`、`commands/ops.rs` | fmt ✓ clippy 零告警 ✓ `--test discard` 9 passed（与搬家前逐条一致）✓ `cargo test` **206 passed / 0 failed** |
| 3 暂存与提交 | ✅ 完成 | `vcs/mod.rs`、`vcs/git.rs`、`commands/ops.rs`、`tests/vcs_backend.rs` | fmt ✓ clippy 零告警 ✓ `cargo test` **210 passed / 0 failed**（= 206 + 新增 4） |
| 4 分支操作 | ✅ 完成 | `vcs/mod.rs`、`vcs/git.rs`、`commands/ops.rs`、`tests/vcs_backend.rs` | fmt ✓ clippy 零告警 ✓ `cargo test` **213 passed / 0 failed**（= 210 + 新增 3） |
| 5 stash | ✅ 完成 | `engine/stash.rs`、`vcs/mod.rs`、`vcs/git.rs`、`commands/stash.rs`、`tests/vcs_backend.rs` | fmt ✓ clippy 零告警 ✓ `cargo test` **216 passed / 0 failed**（= 基线 200 + 新增 16，达标） |
| 6 收尾验证 | ✅ 完成（E2E 点击流程待用户） | 本文件 | 三重对账全过 + 两侧五项检查全过，详见下方「收尾验证结果」 |

### 收尾验证结果

**三重对账（证明「行为零变化」）**

| 对账项 | 期望 | 实测 |
|---|---|---|
| `src/commands/` 的 `#[tauri::command]` 增删 | 0 / 0 | **0 新增 / 0 删除** ✓ |
| 命令定义数 vs `lib.rs` 注册数 | 相等且与 master 一致 | **44 = 44** ✓（`grep` 数到的 45 是 `session.rs:9` 文档注释里的一句提及，非真命令） |
| `git diff master -- src/lib.rs` | 只多 `pub mod vcs;` | **仅此一行**，`invoke_handler!` 44 行一字未动 ✓ |
| `git diff master -- src/commands/` 里的 `pub fn` 行 | 增删配对 | **零输出**——44 个命令签名与 master 逐字节一致 ✓ |
| `git diff master` 前端部分 | 空 | **空** ✓（`src/`、`tests/`、`package.json`、各 config 全无改动） |

**五项检查**

| 检查 | 结果 |
|---|---|
| `cargo fmt --check` | 通过 |
| `cargo clippy --all-targets` | **0 告警 0 错误**（无 `#[allow]`） |
| `cargo test` | **216 passed / 0 failed**（基线 200 + 新增 16） |
| `pnpm build`（含 tsc） | 通过，80 modules |
| `pnpm test` | **257 passed / 19 文件** |
| `cargo build`（应用二进制） | 通过 |
| `pnpm tauri dev` 启动冒烟 | 通过——Vite 就绪、`app.exe` 启动、无 panic 无启动期错误 |

**E2E 点击验证：已完成（2026-09-12，用户在真实窗口执行）**

| # | 操作 | 结果 |
|---|---|---|
| 1–3 | 打开仓库 / 图渲染 / 点提交看 diff / Local Changes | ✅ 会话恢复时自动跑通，0 panic 0 ERROR |
| 4 | 暂存 → 取消暂存 → 再暂存 → 提交 | ✅ |
| 5 | 丢弃改动 | ✅ |
| 6 | 新建分支 → 重命名 → 切过去 → 删掉 | ✅ |
| 7 | stash → 列表 → pop | ✅ |

用户在第 7 条观察到「stash 没有收走未跟踪文件」，**核实为设计行为、非回归**：
`src/engine/toolbarActions.ts:54` 里普通点击传 `includeUntracked: false`，
按住 Ctrl 时按钮变成 "Stash all" 才传 `true`（对应 `git stash push -u`）。
master 上即如此，本次重构未触碰该路径。

至此 `AGENTS.md` §5.1 要求的真实窗口验证已闭环。

### 独立 review 结论（2026-09-12）

按 `AGENTS.md` §4.1 的独立 review 门槛执行，6 条发现（2 medium / 4 low），无 blocking。
reviewer 自行跑了 `cargo test`（216 passed），数字与本文件一致。

**本 PR 内已修**

| # | 问题 | 修法 |
|---|---|---|
| 2 | `unstage` 缺空参数守卫 | `GitBackend::stage` / `unstage` 加 `is_empty()` 提前返回；trait 文档把「空 paths 必须是 no-op」升级为**三个方法一体适用**的契约 |
| 3 | `stage` / `unstage` 缺 `--` 分隔符 | 两处都补上 `--` |
| 5 | `backend_for_rejects_non_repo` 名不符实 | 改名 `open_on_non_repo_reports_not_repo`——`backend_for` 根本不会失败，断言全在 `open()` 上，属 rubric #9 的虚假信心 |
| 6 | 绕过接缝的写命令没记录 | 见上方「给 Phase 1 实现者的交接」整节 |

新增 3 个回归测试（`backend_unstage_with_empty_paths_is_a_noop`、
`backend_stage_with_empty_paths_is_a_noop`、
`backend_stage_and_unstage_handle_dash_prefixed_paths`）。

实测支撑（2026-09-12，临时 fixture）：

```text
git reset HEAD           （无 pathspec）→ 清空整个暂存区，退出码 0
git add "-dash.txt"      （无 --）      → error: unknown switch 'd'
git add -- "-dash.txt"                  → 退出码 0
git add                  （无参数）     → "Nothing specified, nothing added."
```

**不在本 PR 修，另开 behavior-fix PR**

| # | 问题 | 为什么不在这里修 |
|---|---|---|
| 1 | `engine::ops::path_set` 的 `Err(_) => HashSet::new()` 无差别吞掉所有 git 错误。`ls-tree` 探测若意外失败且 `staged=true`，HEAD 里本来存在的文件会落进 `remove` 桶被 `git rm -f`，变成「本想恢复、结果删掉」 | **从 master 原样搬来的既有行为**，修它会打破 Phase 0「行为零变化」这个整套验证赖以成立的前提。另：reviewer 给的触发条件（`index.lock` 竞争）**不成立**——`ls-tree` 读对象库，不拿 index.lock。底层担忧真实，触发路径需重新评估 |
| 4 | `discard` 三阶段（restore → `rm -f` → unlink）无回滚，中途失败停在半丢弃状态 | 同为既有行为。顺序是「最不破坏优先」的有意设计，部分应用可接受，但缺一条说明注释 |

两条都应在 Phase 1 开工前处理掉——它们都落在 `engine::ops::discard` 上，而 Phase 2
的 `sync_in` 会依赖 discard 的可靠性。

#### 2026-09-13 结清：两条都已在分支 `worktree-discard-hardening` 修掉

| # | 结论 | 实测支撑 |
|---|---|---|
| 1 | **确认是真缺陷，比 review 描述的更严重**。触发条件不是 `index.lock`，而是**对象库损坏**：删掉 HEAD 的 tree 对象后 `ls-tree -r HEAD` 报 `fatal: not a tree object`，而 `git rm -f` 照样退出 0 并把文件删掉。旧实现此时**整个调用返回 `Ok(())`**——用户点「丢弃改动」，文件没了，界面上没有任何异常 | 临时 fixture 实测 + 回归测试 `staged_discard_stops_instead_of_deleting_when_head_cannot_be_read`（修复前 `expect_err` 拿到的是 `Ok(())`） |
| 4 | **「加回滚」做不到**，`git checkout` 抹掉的工作区内容没有副本可取回。改为「失败立刻停下 + `discard_partial` 说清已完成 / 可能已部分处理 / 未处理三类路径」。fail-fast 本来就有（靠 `?`），补的是那份交代 | 回归测试 `a_failed_stage_says_what_was_already_done_and_what_was_not`、`a_locked_untracked_file_reports_which_ones_were_already_deleted` |

**空仓库必须与对象库损坏区分开**，这是 #1 修复的核心。不能用「`rev-parse --verify HEAD`
失败」来判空仓库——实测场景 D（tree 对象丢失）里它照样退出 0，而别的损坏形态下它会
失败，于是真损坏又被当成空仓库吞掉。正确判据是正向的：`symbolic-ref --quiet HEAD`
成功且它指向的分支 ref 不存在。detached HEAD 下 `symbolic-ref` 退出 1，判为非空仓库，
正确。

顺带把 `staged == false` 时的 `ls-tree` 探测整条去掉——那种情况下 `in_head` 被
`staged &&` 短路，本来就用不到，少一次进程也少一个失败面。

#### 对抗性 review 的追加修正：判定必须看退出码，不是看成败

第一版修复只判「`show-ref --verify` 有没有失败」，**漏掉了同一缺陷的第二种形态**，
由 Codex 对抗性 review 指出并在 fixture 中复现。四种形态的实测退出码：

| 形态 | `show-ref --verify --quiet` | 正确结论 |
|---|---|---|
| 仓库还没有任何提交（ref 文件不存在） | **1** | unborn |
| 分支 ref 指向不存在的 OID | **128** `bad ref` | **抛出** |
| ref 在 packed-refs 里 | **0** | 非 unborn |

第二行就是漏掉的那条：只看成败会把它当成 unborn，而 `git rm -f` 在该状态下照样
退出 0 并删掉文件。回归测试 `staged_discard_stops_instead_of_deleting_when_the_branch_ref_is_corrupt`
在旧写法下确实失败（`expect_err` 拿到 `Ok(())`，已实际验证过一次）。

`run_git` 把任何非零退出码塌缩成同一个 `GitError`，表达不了这个区别，因此在
`engine::git_cli` 加了同族函数 `run_git_code`（与既有的 `run_git_bytes` 并列，
不是另起一套进程调用，符合 `AGENTS.md` §7.3）。

**否掉的备选方案**：用 `for-each-ref` 判 ref 是否存在。它按**路径前缀**匹配——
HEAD 指向 `refs/heads/svn`（正是 spec D10 的基线分支名）而仓库里有
`refs/heads/svn/base` 时，它会吐出后者，把真 unborn 误判成「ref 存在」。实测确认，
并留了 `an_unborn_head_pointing_at_a_slash_prefixed_name_is_still_unborn` 挡住这条路。
`show-ref --verify` 是精确匹配，无此问题。

#### 第二轮对抗性 review：一条不成立，但根因是我的文档写错了

第二轮 review 判 needs-attention，指「ref 内容是垃圾时仍会删文件」。**该结论不成立**，
但责任在我——它读的是我上一版写错的文档。

初版这里写着「ref 内容是垃圾 → `show-ref` 退出 1 → 按 unborn 处理 → 暂存内容丢失，
有意接受」。**那个后果我从未端到端验证过**：探测脚本是先抓 `$h` 再损坏 ref，单独测了
`show-ref` 这一个零件的退出码，然后就据此推断了整条路径的结果。

补测真实路径后的事实：ref 内容不是合法 OID 时，`git symbolic-ref --quiet HEAD` 就已经
退出 **128**（`fatal: No such ref: HEAD`），**根本走不到 `show-ref`**。判定在第一关就
返回 false，`discard` 抛出，文件与索引都不动。

违反的是 `AGENTS.md` §6.2：测的是零件，写的却是整机的结论。已补回归测试
`staged_discard_stops_instead_of_deleting_when_the_branch_ref_is_malformed`，断言的是
`ops::discard` 的真实返回与磁盘、索引状态，不是某条 git 命令的退出码。

顺带修掉一处**碰巧正确**：第一关 `code != 0` 原本只注释了 detached HEAD，而「HEAD 读
不出来」也落在同一分支、含义相反，只是恰好都返回 `Ok(false)` 才安全。现已写明
`Ok(false)` 的语义是「**不能确认是 unborn**」而非「确认不是 unborn」，两种情况都归它，
是有意设计。

各形态的完整实测表见 `engine::ops::head_is_unborn` 的文档注释。

## 决策条目

### 全局

#### 偏离计划

- **trait 方法不带 `path` 参数**，改为后端构造时绑定工作目录。spec §4.1 的签名里
  每个方法都带 `path: &str`。理由见计划「与 spec 的一处有意偏离」一节：Phase 1 的
  `SvnGitBackend` 要持有每仓库配置（白名单、`svnUrl`、基线分支名，来自
  `.git/mygitclient.json`），按 path 传参会导致每次调用重复读盘解析。

- **`branches()` / `stashes()` 两个子 trait 改为平铺方法。** spec §4.1 写的是
  `fn branches(&self) -> &dyn BranchOps` 这种返回子 trait 的形式。实际只有 4 个分支
  方法和 5 个 stash 方法，为它们各建一层 trait 对象是纯粹的间接，Phase 1 也没有
  需要按能力分组的地方。

- **`capabilities()` / `sync_in` / `sync_out` / `external_changes` 不在 Phase 0 实现。**
  spec §4.1 把它们列进 trait，但 Phase 0 的验收标准是「行为零变化」——
  `capabilities()` 要等 Phase 1 前端消费才有意义，现在加进来是 dead code，会触发
  clippy 告警，而 `AGENTS.md` §7.3 禁止用 `#[allow(...)]` 掩盖；另外三个 Phase 2–4
  才需要，现在加只能写 `unimplemented!()`。

#### 计划的错误

- **Task 1：计划里给的测试代码不是 rustfmt-clean 的。**
  `backend_diff_matches_engine` 里那句
  `assert!(!via_backend.files.is_empty(), "fixture 应当有可 diff 的文件");`
  超出行宽，rustfmt 要求拆成多行。`cargo fmt --check` 因此失败一次。

  影响很小（跑一次 `cargo fmt` 即可），但对后续 Task 的含义是：**计划里的代码块
  是语义正确的参考，不保证格式正确**——每个 Task 的验证步骤里先跑 `cargo fmt`
  再跑 `cargo fmt --check`，不要指望粘贴即合规。

  另注：计划 Step 8 把 `cargo fmt --check && cargo clippy` 用 `&&` 串起来，
  fmt 一失败 clippy 就被短路跳过了。后续 Task 分开跑这两条。

- **Task 3：计划里 `backend_unstage_removes_from_index` 的断言数字错了。**
  计划写的是 unstage 之后 `unstaged.len() == 1`，实测是 **2**。

  原因：本项目的 `engine::diff::read_status` **把未跟踪文件也算进 unstaged**
  （与 `fixtures.rs` 的 `read_diff_includes_untracked_content` 是同一套约定），
  而 fixture 里有一个 `untracked.txt`。是**计划的假设错**，不是实现错。

  修法不是把 1 改成 2，而是改成**按路径断言**
  （`after.unstaged.iter().any(|f| f.path == "a.txt")`）——数个数会随 fixture
  里无关文件的增减而碎，按路径断言才真的在测「a.txt 退回了未暂存」。

- **Task 5：`engine::stash::drop` 直接按计划预判改名为 `drop_entry`。**
  计划 Step 8 里写了「clippy 可能报 `should_implement_trait`，若报了再改名」。
  实现时直接取了 `drop_entry`，没有先撞一次告警——`Drop::drop` 同名冲突是确定的，
  等 clippy 报再改属于明知故犯。trait 侧的方法名仍是 `stash_drop`，不受影响。

#### 给 Phase 1 实现者的交接：接缝**不完整**，以下写命令仍直连 git

这不是疏漏，是 spec §4.3 的有意范围划分。但 decisions 初版只列了
`branch_delete_info` 一条，会让人误以为接缝已覆盖全部写操作。完整清单：

| 命令 | 位置 | 为什么没进 trait |
|---|---|---|
| `checkout_remote_branch` | `commands/ops.rs` | 远程分支，SVN 侧无对应物 |
| `delete_remote_branch` / `delete_remote_ref` | `commands/ops.rs` | 同上 |
| `branch_delete_info` | `commands/ops.rs` | 删除前的护栏查询，位置刻意不动 |
| `create_tag` / `push_tag` | `commands/ops.rs` / `remote.rs` | tag，SVN 侧无对应物 |
| `cherry_pick` / `revert` | `commands/ops.rs` | 依赖 git 的 DAG 语义 |
| `save_patch` | `commands/ops.rs` | `format-patch`，git 专属 |
| `fetch` / `pull` / `push` | `commands/remote.rs` | Phase 2/3 会被 `sync_in`/`sync_out` 取代 |
| `commands/worktree.rs` 全部（1359 行） | — | git worktree，SVN 模式下整块隐藏 |

**Phase 1 的风险点**：一旦 `backend_for` 能返回非 git 后端，上面这些命令仍会对着
同一个工作目录直接 `run_git`。Phase 1 必须同时做的是 `capabilities()` + 前端按能力
隐藏入口，**而不是假定接缝已经拦住了它们**。

另：`backend_for` 返回 `Result` 但当前实现永不失败，10 个调用点的 `?` 是死代码。
这是**有意预留**——Phase 1 要在这里读并解析 `.git/mygitclient.json`，那一步会失败。

#### 计划没写的坑

- **全冷 target 上第一次 `cargo test` 会以 rlib 链接错误失败，重跑即好。**
  新 worktree 没有 `src-tauri/target/`，P6 的第一次 `cargo test` 有 12 个测试二进制
  报 `crate 'windows' / 'tokio' / 'cookie' / 'tauri_runtime' … required to be
  available in rlib format, but was not found in this form`。

  这**不是代码缺陷**，是冷编译竞态：并行构建时部分依赖的 rlib 还没落地，测试二进制
  就开始链接。判定方式——单跑一个最小测试（`cargo test --test git_cli`）立刻通过，
  随后完整 `cargo test` 也全绿（200 passed / 0 failed）。

  对后续每个 Task 的含义：**看到 rlib 类错误先重跑一次再怀疑代码**，按 `AGENTS.md`
  §5.3「检查失败指向环境问题时不要直接判定为代码缺陷」。

- **`EnterWorktree` 默认从 `origin/<默认分支>` 开分支，不是本地 HEAD。**
  切 worktree 前 `origin/master` 停在 `928923f`，而本地 master 已经领先 3 个文档
  提交（spec 的 review 修订、环境更新、本计划）。若直接切，worktree 里会没有计划和
  修订后的 spec。处理：先把这 3 个纯文档提交 push 到 master（`AGENTS.md` §4.1 的纯
  文档例外明确允许），再切 worktree。计划的 P3 检查项（「基点 commit 与 master 的
  HEAD 一致」）正是为抓这个而写的。

#### 为什么这么改是安全的

- Phase 0 不碰任何破坏性 git 操作，不改前端，不改命令签名，不动 `lib.rs` 的
  `invoke_handler!` 注册表。安全性论证的核心是**现有 18 个测试文件 4038 行集成测试
  全绿**——它们同时覆盖命令层（6 个文件直接调 `app_lib::commands::*`）和 engine 层
  （`fixtures.rs` 等直接调 `app_lib::engine::*`）两条路径，任何行为漂移都会被其中
  一侧抓到。**实测：216 passed / 0 failed。**
- 新增的 16 个测试是**等价性测试**而非「功能能跑」测试：每条都拿「经 trait 调用」
  与「直接调 engine」的结果对比。后者一个返回固定值的桩也能通过，前者不能。
- **命令签名逐字节未变**（`git diff master -- src/commands/ | grep '^[-+]pub fn'`
  零输出），`invoke_handler!` 未动，前端零改动——前后端契约结构上不可能漂移。
  这是本次重构最强的一条安全论证：改动全部落在命令函数体内部。
- **`discard` 的空 pathspec 守卫在两处都在**：`engine::ops::discard` 的
  `paths.is_empty()` 提前返回原样保留，另外在 trait 文档里把「空 paths 必须是
  no-op」写成了各实现的契约，并加了 `backend_discard_with_empty_paths_is_a_noop`
  做断言。空 pathspec 退化成「全部」是本项目踩过的不可逆事故。
- **本期没有引入任何破坏性 git 操作的新路径**：`delete_branch` 走的仍是原来的
  `git branch -D`，没有改变确认流程或调用点；`branch_delete_info` 这个删除前的
  信息查询命令刻意留在命令层未进 trait，护栏位置不变。

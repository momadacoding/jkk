# VCS 抽象层 Phase 0 决策日志

计划：`docs/superpowers/plans/2026-09-12-vcs-abstraction-phase0.md`
规格：`docs/superpowers/specs/2026-09-12-svn-git-hybrid-design.md`
分支：`worktree-vcs-phase0`（worktree：`.claude/worktrees/vcs-phase0`）
基点：`b5f7f7c`

## 进度总账

**重构前测试基线**（2026-09-12，worktree 内实测）：`cargo test` **200 passed / 0 failed**，
21 个测试二进制。Task 6 拿它对账，最终应为 216。

> 注：输出里那行 `error: cannot delete branch 'feat/busy' used by worktree` 是
> `worktree_ops.rs` 某条失败路径测试的**预期 stderr**，不是测试失败——21 个二进制
> 全部报 0 failed。

| Task | 状态 | 产出文件 | 验证结果 |
|---|---|---|---|
| 1 只读四件套 | ✅ 完成 | `vcs/mod.rs`、`vcs/git.rs`、`tests/vcs_backend.rs`、`lib.rs`、`commands/{graph,diff,repo}.rs` | fmt ✓ clippy 零告警 ✓ `cargo test` **206 passed / 0 failed**（= 基线 200 + 新增 6） |
| 2 discard 下沉 | ✅ 完成 | `engine/ops.rs`（新建）、`engine/mod.rs`、`commands/ops.rs` | fmt ✓ clippy 零告警 ✓ `--test discard` 9 passed（与搬家前逐条一致）✓ `cargo test` **206 passed / 0 failed** |
| 3 暂存与提交 | 未开始 | | |
| 4 分支操作 | 未开始 | | |
| 5 stash | 未开始 | | |
| 6 收尾验证 | 未开始 | | |

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
  一侧抓到。
- 新增的 16 个测试是**等价性测试**而非「功能能跑」测试：每条都拿「经 trait 调用」
  与「直接调 engine」的结果对比。后者一个返回固定值的桩也能通过，前者不能。

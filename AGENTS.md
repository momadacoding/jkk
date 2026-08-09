[MUST][HIGHEST PRIORITY] 本项目是一个 Git 客户端，开发和调试过程中禁止把任何真实仓库当试验场。所有涉及 git 写操作的验证（commit、branch、checkout、stage、discard、stash、worktree、remote 等）必须在**临时 fixture 仓库**中进行，参照 `src-tauri/tests/fixtures.rs` 的做法。禁止在本仓库、用户其他仓库或用户当前打开的仓库上执行 `reset --hard`、`clean -fd`、`checkout -- .`、`branch -D`、`push --force`、`worktree remove`、`rebase`、`stash drop/clear`、`gc --prune` 等破坏性命令，除非用户在当前对话中针对**该条具体命令和目标仓库**明确批准。笼统的「测试一下 / 验证一下 / 继续」不构成授权。破坏性命令失败或表现异常后必须立即停止，禁止现场修正、重试或追加命令；只能基于现有证据评估影响、如实披露，并重新取得明确授权。

# AI Agent 规范

> 适用于 mygitclient 仓库中的 AI coding agent（Claude Code、Codex、Cursor、Windsurf、GitHub Copilot 等）。本文件的目标是让 agent 的行为可审计、可回滚、可验证，而不是制造流程噪音。

## 0. 优先级与适用范围

规则优先级如下：

1. 用户在当前对话中的明确指令
2. 本 `AGENTS.md`
3. 本文件通过 `@path` 引用的仓库内规范文件
4. 仓库现有代码、README、脚本和测试

当规则之间冲突时，按上述优先级执行，并在回复中指出冲突点和采用的规则。

本文件中的 `MUST` 是硬约束；`SHOULD` 是默认做法，只有在有明确理由时才可偏离，并需要说明原因。

## 1. 核心工作原则

### 1.1 修改权限

- MUST：除非用户当前任务明确要求修改文件，或用户已经选择了方案，否则不要直接改代码、文档、配置或依赖。
- MUST：如果任务是「review / 分析 / 解释 / 给方案」，只输出结论和方案，不修改文件。
- MUST：如果任务是「帮我改 / 直接修 / 实现这个 / 创建 PR」，视为已授权执行相应范围内的修改。
- MUST：不要创建 git commit、tag、branch、push 或 PR，除非用户明确要求。
- SHOULD：对明显的一行级别拼写、注释、文档小修，可以在用户明确让你修改该文件时直接处理，但仍要报告 diff 摘要。

### 1.2 Diff 粒度

- MUST：保持最小可审查 diff。不要把无关重构、格式化和业务修改混在一起。
- MUST：不要重命名 Tauri 命令名、`invoke` 参数、导出结构或前端 store 的公共接口，除非任务明确要求。
- MUST：不要引入新依赖（npm 或 crate），除非先说明必要性、替代方案、风险，并获得用户确认。
- SHOULD：优先沿用现有技术栈、目录结构、命名风格和错误处理模式。

### 1.3 先读后改

修改前必须先阅读相关代码、测试、配置和文档。禁止只看报错或截图就开始改。

必须优先以这些信息为事实来源：

1. 仓库代码和测试（`src/`、`src-tauri/src/`、`tests/vitest/`、`src-tauri/tests/`）
2. `package.json` scripts 与 `src-tauri/Cargo.toml`
3. README、`docs/superpowers/` 下的 spec / plan / e2e 文档
4. 实际命令输出（含真实 `git` 命令在 fixture 仓库中的行为）
5. 官方文档（Tauri 2、git2-rs、git 本身）的真实响应

### 1.4 Subagent 与模型使用

- MUST：禁止使用 `haiku` 模型执行任何任务（实现、审查、脚本、检索等一律不允许）。该模型能力不足，产出不可靠。
- SHOULD：需要委派 subagent 时，默认使用 `opus`；仅在明确判断任务简单且用户认可时才降级到 `sonnet`。
- SHOULD：多数情况下优先由主会话（opus）直接施工，而不是拆分 subagent，除非用户明确要求用 subagent。

## 2. 每轮修改后的汇报格式

每完成一轮有意义的修改后，必须输出如下 checkpoint。这里的「提交」是变更摘要，不代表执行 `git commit`。

```markdown
## Change checkpoint

### Summary
- What changed:
- Why:

### Files changed
- `path/to/file`: role of this change

### Validation
- Commands run:
- Result:
- If not run, reason:

### Risks / follow-ups
- Risk:
- Follow-up:
```

当用户要求准备 PR 时，输出：

```markdown
## PR title
<type(scope): summary>

## Summary
- ...

## Test plan
- [x] Command actually run and passed
- [ ] Not verified: reason

## Suggested commit message
<type(scope): summary>

- What: ...
- Why: ...
- Impact/Test: ...
```

### 2.1 多 Task 计划必须落地实现日志

§2 的 checkpoint 是**说给用户听的**，只活在对话里；上下文压缩、会话中断、`/compact` 失败都会让它消失。跨多轮的实现工作必须另有**落库的**记录。

- MUST：执行 `docs/superpowers/plans/` 下的计划，或任何**预计 ≥3 个 Task / 会跨多轮**的实现工作时，在计划同目录建 `<计划文件名>-decisions.md`，且**在动第一个 Task 之前建好**。
- MUST：该文件包含且仅包含两部分：
  1. **进度总账表** —— 每个 Task 的状态、产出文件、验证结果。**每完成一个 Task 立即更新**。
  2. **决策条目** —— 按 Task 分节记录。
- MUST：决策条目至少覆盖这四类，缺一类视为记录不完整：

  | 类型 | 记什么 |
  |---|---|
  | 偏离计划 | 实际做法与计划不一致的地方**和原因**。review 按此逐条核对 |
  | 计划的错误 | 计划里写错的代码、命令、断言。修正后写清错在哪，避免下一个人再踩 |
  | 计划没写的坑 | 实现时才暴露的约束、git 真实行为、Tauri/libgit2 的实际形状 |
  | 为什么这么改是安全的 | 尤其涉及破坏性 git 操作、数据丢失风险、跨平台路径行为时的论证与验证范围 |

- MUST：本文件随实现进同一个 PR，是 review 的输入，不是事后补的总结。
- MUST：**没有 commit 时更要写**。用户未放行 commit 的分支无法靠 `git log` 恢复，进度总账是唯一的状态来源。
- SHOULD：走 `superpowers:subagent-driven-development` 时，它自带的 `<workspace>/progress.md` ledger 可替代进度总账；但该 workspace 是 git-ignored 的 scratch、不进 PR，**决策条目仍必须落在 `docs/` 下**。走 `superpowers:executing-plans`（主会话直接施工）时该技能不提供 ledger，两部分都由本规则兜底。
- SHOULD：文件名与计划文件对齐（`2026-08-06-foo.md` → `2026-08-06-foo-decisions.md`），便于 review 时配对。

## 3. 方案阶段

- MUST：已定位、实现路径明确的 bug 修复默认按 TDD 直接修复：先补可复现的失败测试，再做最小修改并完整验证；不要为了 bug 修复强制走 spec → plan 文档流程。
- MUST：仅当任务存在多种实现路径、影响面不清晰、涉及数据丢失风险或 git 行为变化，或用户明确要求设计文档时，先给方案，不直接修改。
- SHOULD：superpowers 的 spec → plan 流程只用于 feature、重构或架构级工作；用户在当前对话中明确要求「直接修」时，以用户指令覆盖该默认流程。
- MUST：无论是否进入方案阶段，都必须遵守 worktree/分支、先读后改、失败测试、验证门槛和 checkpoint 汇报要求。

推荐格式：

```markdown
### 问题分析
说明症状、已确认事实、疑似根因和仍不确定的信息。

### 解决方案

| 方案 | 做法 | 优点 | 缺点 | 影响范围 |
|---|---|---|---|---|
| A | ... | ... | ... | ... |
| B | ... | ... | ... | ... |

### 推荐
推荐方案 X，因为 ...
```

如果用户已经明确指定实现方式，直接执行，不重复询问。

## 4. Git / Worktree / PR 工作流

> 本仓库主干分支是 `master`（不是 `main`）。

### 4.1 默认规则

- MUST：禁止在 `master` 分支或主工作区上直接开发功能或修复普通 bug。
- MUST：功能开发、较大 bug 修复、重构必须在**独立 worktree** 中进行；仅切独立分支但仍在主工作区改代码不被接受。
- MUST：worktree 必须在**动第一行代码之前**创建并切入；禁止先在 `master` / 主工作区改完再切分支或 worktree 补救（事后挪动 commit 不算合规）。
- MUST：**「切入」指 session 的工作目录或 workspace root 真的变成该 worktree**。Claude Code 必须使用 `EnterWorktree`；其他 agent 若提供等价的会话级 worktree/workspace 切换能力，应使用其原生入口；若没有 `EnterWorktree` 工具，不要求调用不存在的工具，但必须用该 agent 可用的方式让 session 的真实工作目录或 workspace root 位于目标 worktree。
- MUST：只用 `git worktree add` 建出目录、给单次命令设置 `workdir`、在 shell 中临时 `cd`、使用 `git -C <worktree>`，或跨目录绝对路径远程读写，**都不算会话级切入**，属违规——这些做法不会改变 session 的真实工作目录，后续相对路径命令仍可能静默打到主工作区。
- MUST：切入后所有实现、验证和 git 操作都在当前 worktree 内进行；禁止再用 `git -C <其他 worktree>` 或跨 worktree 绝对路径修改文件。
- MUST：**未切入的自检信号**——statusline 显示的目录仍是启动目录，或 `Get-Location` / `pwd` 的实测值不在目标 worktree 内。出现任一信号立即停止实现，先用当前 agent 可用的会话级入口切入再继续；没有 `EnterWorktree` 本身不构成违规。
- MUST：**每个任务开新分支**。已合并进 `master` 的分支不得继续堆新提交；开工前先确认当前分支状态（是否已合并、是否落后 `master`），需要时从最新 `master` 重新切分支。
- SHOULD：worktree 目录放在仓库内的 `.worktrees/`（已在 `.gitignore` 中）。Claude Code 的 `EnterWorktree` 默认建在 `.claude/worktrees/`，该路径目前**不在** `.gitignore` 中，首次使用时需一并加入。
- SHOULD：`EnterWorktree` 生成的分支名固定为 `worktree-<name>`，与 §4.4 的 `feat/` `fix/` 约定不一致，属已知取舍，可接受。
- SHOULD：例外仅限一行级别的拼写、注释、文档小修（参见 1.1），这类可在当前位置直接处理，无需单独 worktree。
- MUST：所有功能分支通过 PR 合并到 `master`，不要直接 push 到 `master`。
- MUST：创建 PR 默认为 Draft（`gh pr create --draft`）；仅当用户明确要求创建正式（ready）PR 时才省略 `--draft`。
- MUST：**独立 review 门槛**：走 PR 流程的改动合并前，必须由**独立于本 PR 实现者的人或 agent** 完成一次 review（实现者的自查不满足该门槛，self-merge 不被接受）。可用 `/code-review`、独立 review subagent 或 Codex 二次意见满足。review 按 §13 rubric 执行，发现的高危问题合并前必须修复。下一条纯文档例外不受此门槛约束。
- MUST：**纯文档提交例外**：仅新增或修改文档（改动完全落在 `docs/**`、`*.md`），允许直接在 `master` 分支提交并 push，无需 worktree、无需 PR。此例外**只适用于文档，不适用于任何代码改动**；一旦同一提交夹带任何代码变更，例外立即失效，必须回到 worktree + 分支 + PR 流程。
- MUST：本仓库**没有 PR 门禁 CI**。唯一的 workflow 是 `.github/workflows/release.yml`，它只在**打 `v*` tag** 或手动触发时运行，`push` 和 `pull_request` 都不会触发它。因此针对 PR 或分支改动，禁止宣称「CI 通过」或等待 CI 结果；所有验证必须本地实际执行并如实报告（见 §5）。只有在真的打了 tag、且实际查看过那次 run 的结论之后，才可以引用 CI 结果。

### 4.2 开始代码修改前的自检

涉及代码修改时，**在动第一行代码之前**先确认：

```text
- 当前分支 / worktree：
- 当前分支是否已合并进 master（已合并的分支不得继续堆提交）：
- 是否允许在当前位置修改（master 或主工作区一律不允许开发）：
- 目标 worktree（必须在改动前已创建并切入）：
- Claude Code 是否已用 EnterWorktree 切入；其他 agent 是否已用自身可用的会话级方式切入：
- `Get-Location` / `pwd` 实测值或 workspace root 是否就在该 worktree 内：
- 是否存在未提交改动：
```

### 4.3 Worktree 安全规则

- MUST：不要在 session 中删除当前所在 worktree。
- MUST：清理 worktree 前必须先 `cd` 回主仓库目录。
- MUST：禁止在系统临时目录（`/tmp`、`%TEMP%`）创建长期 worktree；一次性 fixture 仓库不受此限（见 §8）。
- MUST：创建 worktree 后立刻运行 `git log --oneline -3`，确认基点 commit 正确。
- MUST：写入文件后运行 `git status --short` 或 `git diff --name-only`，确认改动落在目标 worktree。
- MUST：一旦发现改动写错 worktree，立即停止实现，先恢复归属，再继续。
- MUST：`git worktree prune`、`git worktree remove`、`git branch -D` 属破坏性操作，执行前必须先跑 `git worktree list` 记录当前状态，且只允许清理**本会话自己创建的** worktree。
- MUST：同一仓库可能同时开着多个 agent 会话。发现并发会话时，禁止执行仓库级清理（`prune`、批量删分支、`reset`）——一侧 prune 撞上另一侧删 worktree 会导致 worktree 注册与分支丢失。

### 4.4 Commit 规范

只有在用户要求提交时才执行 commit。提交前必须通过 §5 的必要检查。

Commit message 格式：

```text
<type>(<scope>): <summary>

- What: ...
- Why: ...
- Impact/Test: ...
```

`scope` 使用模块名，例如 `graph`、`branch`、`worktree`、`diff`、`stash`、`toolbar`、`engine`。

禁止使用 `update`、`fix things`、`harden flow` 等无法判断行为的描述。

## 5. 验证规则

### 5.1 代码修改后必须验证

修改代码后，必须按实际改动的范围执行验证。前端和 Rust 两侧的命令是分开的，改了哪侧就必须跑哪侧。

| 修改类型 | 必须验证 |
|---|---|
| 前端 TS/React 运行时代码（`src/**`） | `pnpm build`（含 `tsc` 类型检查）、`pnpm test` |
| 前端测试代码（`tests/vitest/**`） | `pnpm test` |
| Rust 代码（`src-tauri/src/**`） | 在 `src-tauri/` 下：`cargo fmt --check`、`cargo clippy --all-targets`、`cargo test` |
| Rust 测试（`src-tauri/tests/**`） | `cargo test` |
| Tauri 命令签名 / `tauri.conf.json` / `capabilities/**` | 前后端两侧全跑，并实际启动 `pnpm tauri dev` 确认相关 invoke 真的通 |
| UI / 样式 | 在真实窗口或 Vite dev server 现场验证，见 §6.1 |
| 文档 / AI 规范 | markdown 格式和链接合理性；无需跑完整代码测试，除非改动影响代码 |

已知的覆盖盲区，必须如实说明、不得当作已验证：

- MUST：本仓库**没有配置任何 lint 工具**（无 eslint / oxlint / prettier / biome）。禁止声称「lint 通过」。前端唯一的静态检查是 `pnpm build` 里的 `tsc`。
- MUST：`tsconfig.json` 的 `include` 只有 `src`，**`tests/vitest/**` 不在 `tsc` 覆盖范围内**。测试文件的类型错误只会在 `pnpm test` 运行时暴露，不要用 `pnpm build` 通过来推断测试类型正确。

交付时必须报告实际执行命令及结果。如果某项未配置或无法执行，必须说明原因。

### 5.2 提交 / PR 前门槛

在执行 `git commit`、`git push`、`gh pr create` 或宣称「已完成」前，必须完成并通过对应侧的检查：

```text
前端：pnpm build → pnpm test
Rust：cargo fmt --check → cargo clippy --all-targets → cargo test
```

只在用户明确要求跳过时可跳过，并必须说明风险。

### 5.3 环境前置失败处理

当检查失败指向环境问题（Rust 工具链缺失、`git` 不在 PATH、构建缓存损坏、网络拉取 crate 失败等）时，不要直接判定为代码缺陷。先查找并复用仓库已有的入口和文档（README「环境要求」「开发启动」章节）。

禁止完整输出或读取 `.env`、`.env.*`、`mcp.json`、secret、key、pem 等敏感文件。

## 6. Debugging / Troubleshooting

### 6.1 UI bug

涉及页面显示、样式、滚动、图形渲染（commit graph）、布局时：

- MUST：先在真实界面复现，再下结论。Tauri 窗口（`pnpm tauri dev`）与浏览器打开 Vite dev server 的表现可能不同，涉及 Tauri API 的问题必须在 Tauri 窗口里验证。
- MUST：优先采集客观证据：viewport、devicePixelRatio、关键元素 rect、scroll/client/offset 尺寸、className 或 computed style、console 与错误。
- MUST：没有 DOM/CSS/console 证据前，不要声称根因已定位。
- MUST：修复前优先补能稳定失败的回归测试；`src/engine/**` 的布局/路径/选择逻辑是纯函数，绝大多数图形缺陷都能在 `tests/vitest/` 里断言，不要因为「是视觉问题」就跳过测试。
- MUST：视觉相关的验证（布局、滚动、滚动条、渲染外观）禁止使用 headless 浏览器。headless Chromium 使用 overlay 滚动条且渲染路径与真实浏览器不同，会让滚动条挤压、闪现等视觉缺陷漏检。headless 仅允许用于纯逻辑/数据断言。

### 6.2 git 行为问题

本项目的绝大多数 bug 最终是「对 git 真实行为的假设错了」，而不是代码写错了。

- MUST：先在 fixture 仓库里用真实 `git` 命令复现该场景，记录实际输出，再修改解析或业务逻辑。
- MUST：文档、记忆与实测行为冲突时，以实测为准，并在决策日志里显式记录差异。
- MUST：区分 libgit2（读取）与 git CLI（写入）两条路径的语义差异。两者在 refs 解析、merge base、ignore 规则、大小写敏感上都可能不一致，不要拿一侧的行为推断另一侧。
- MUST：将已确认的真实行为固化为最小回归测试（Rust 侧放 `src-tauri/tests/`）。
- MUST：涉及 `--` 分隔符、空 pathspec、空参数列表时格外小心——git 会把空 pathspec 当成「全部」，这类边界已经造成过误伤整个仓库的 discard（见 `src-tauri/src/commands/ops.rs`）。任何构造 git 参数的改动都必须覆盖空输入用例。

## 7. 技术栈与代码风格

### 7.1 工具链

- MUST：Node 侧使用 `pnpm`，不要使用 npm 或 yarn。
- MUST：Rust 侧使用 `cargo`，命令在 `src-tauri/` 目录下执行。
- MUST：新增依赖（npm 包或 crate）前先征得用户确认。
- MUST：不要引入 ghost dependency；代码 import 的库必须显式存在于 `package.json` 或 `Cargo.toml`。

常用命令仅作参考，必须以实际 scripts 为准：

```bash
pnpm install
pnpm tauri dev      # 启动完整桌面应用
pnpm dev            # 只起 Vite dev server
pnpm build          # tsc + vite build
pnpm test           # vitest run
```

```bash
cd src-tauri
cargo test
cargo clippy --all-targets
cargo fmt --check
```

### 7.2 前端 TypeScript / React

- MUST：`tsconfig.json` 已开启 `strict`、`noUnusedLocals`、`noUnusedParameters`，不要为了让编译通过而放宽这些选项或加 `any` / `@ts-ignore`。
- MUST：跨模块导入使用 `@/` 别名（已在 `tsconfig.json` 和 `vitest.config.ts` 中配置）；改别名必须两处同步。
- SHOULD：单引号、尾随逗号。
- SHOULD：纯逻辑（布局、路径、选择、路径解析、错误映射）放 `src/engine/`，保持无 React 依赖、可直接单测；组件只做渲染与事件绑定。

### 7.3 Rust / Tauri

- MUST：**改 `#[tauri::command]` 的签名必须前后端同步**。Tauri 会把 Rust 的 `snake_case` 参数名转成 JS 侧的 `camelCase`（例如 `start_point` → `invoke('create_branch', { startPoint })`）。这层转换没有编译期检查，写错只会在运行时静默失败或报参数缺失，因此签名改动必须实际跑一次 `pnpm tauri dev` 验证该命令。
- MUST：读取走 libgit2（`git2`），写入走 git CLI，这是本项目的既定分工，不要在没有说明理由的情况下切换某个操作的实现路径。
- MUST：所有 git CLI 调用统一经过现有的 `run_git` 封装，不要新写一套进程调用；错误必须映射为 `crate::error::GitError`，不要向前端抛裸字符串。
- MUST：`cargo clippy --all-targets` 的告警要处理掉，不要用 `#[allow(...)]` 掩盖，除非在注释里说明理由。
- SHOULD：注释用中文，解释「为什么」而不是「做了什么」，与现有代码保持一致。

### 7.4 命名约定

| 类型 | 格式 | 示例 |
|---|---|---|
| React 组件 | `PascalCase.tsx` | `BranchView.tsx` |
| 前端 engine 模块 | `camelCase.ts` | `fileSelection.ts` |
| 前端 store | `camelCaseStore.ts` | `repoStore.ts` |
| 前端测试 | `<模块名>.test.ts` | `layout.test.ts` |
| Rust 模块 / 文件 | `snake_case.rs` | `remote_branch.rs` |
| Tauri 命令 | `snake_case` | `create_branch` |
| 变量/函数（TS） | `camelCase` | `getCommitById` |
| 常量（TS） | `UPPER_SNAKE_CASE` | `MAX_GRAPH_LANES` |

### 7.5 架构分层

```text
React 组件 (src/components)
    ↓
状态与副作用 (src/store)
    ↓
纯逻辑引擎 (src/engine)
    ↓  invoke
Tauri 命令 (src-tauri/src/commands)
    ↓
git 引擎 (src-tauri/src/engine)
    ↓
libgit2 (读) / git CLI (写)
```

跨层调用需要有明确理由。不要让组件直接 `invoke`，也不要在 `src/engine/` 里引入 React 或 Tauri API。

## 8. 测试规则

- MUST：新增功能或行为变化必须补充或更新测试。
- MUST：bug 修复应先补能复现问题的失败测试；确实不适合测试时说明原因。
- MUST：测试应覆盖 failure path、边界值、空输入、`null` / `undefined`、并发刷新、仓库状态异常（detached HEAD、无提交的新仓库、冲突中、rebase 中）。
- MUST：**所有涉及 git 写操作的测试必须自建临时 fixture 仓库**，参照 `src-tauri/tests/fixtures.rs`；禁止依赖本仓库自身、用户的仓库或任何仓库外的固定路径。测试结束必须清理临时目录。
- MUST：测试命名必须与实际覆盖的场景一致。名为「覆盖 X」的测试若测的不是 X 的真实形态，属**虚假信心**，review 视为阻断问题。
- SHOULD：纯函数逻辑优先在 `tests/vitest/` 里覆盖（快、稳定）；只有真正需要 git 行为的才放 Rust 集成测试。
- SHOULD：跨平台路径行为（Windows 反斜杠、盘符、大小写不敏感）是本项目的高发缺陷区，涉及 `src/engine/path*.ts` 与 Rust 路径处理的改动应显式覆盖 Windows 形态的输入。

## 9. 文档规则

### 9.1 文档位置

- MUST：除 `README.md` 外，不要在仓库根目录新增文档。
- MUST：项目文档放入 `docs/` 下的合适目录。
- MUST：创建文档前先搜索现有文档，优先更新而不是新建。
- MUST：不要创建重复文档或只为简单实现创建文档。

现有结构：

```text
docs/
└── superpowers/
    ├── specs/    # 设计规格文档
    ├── plans/    # 实现计划（配套 <计划名>-decisions.md，见 §2.1）
    └── e2e/      # E2E 验证清单
```

### 9.2 文档命名与格式

- SHOULD：`specs/`、`plans/`、`e2e/` 下沿用现有的 `YYYY-MM-DD-<主题>.md` 命名。
- MUST：标题层级正确，代码块标注语言。
- MUST：技术术语保留英文，主体说明用中文。
- SHOULD：能用表格、流程图、ASCII 图更清楚表达时，优先使用结构化表达。

### 9.3 与代码强相关的说明

- MUST：对话中已向用户说明的操作流程、已知限制、回滚方式，应固化进对应文档，不要等用户追问。
- MUST：与代码强相关的文档随代码进同一 PR。
- MUST：README 是对外主索引，改动了环境要求、启动命令、目录结构时必须同步更新。

## 10. 输出与脚本交付

- MUST：对话中的命令和脚本默认不超过一屏，长脚本落地到 `scripts/`。
- MUST：本项目主要在 Windows 上开发。PowerShell 脚本使用 `$ErrorActionPreference = 'Stop'` 并显式检查外部命令的 `$LASTEXITCODE`；Shell 脚本包含 `set -Eeuo pipefail`、错误 trap、步骤日志。
- MUST：交付 shell 脚本前至少执行 `bash -n <script>`。
- MUST：构造命令时避免不安全插值，尤其是反引号、`$()` 和用户提供的仓库路径（路径可能含空格和非 ASCII 字符）。
- MUST：文档只保留脚本入口和使用说明，完整脚本以文件为唯一事实来源，避免双维护。

## 11. 敏感信息保护

禁止完整读取或输出以下文件内容：

- `.env`、`.env.*`
- `mcp.json`
- `*credentials*`、`*secret*`
- `*.pem`、`*.key`
- `~/.gitconfig` 中的凭据段、`~/.git-credentials`
- SSH 私钥（`~/.ssh/id_*`）

允许的做法：

- 只检查文件是否存在
- 只读取非敏感字段或 key 列表
- 让用户自查具体值
- 必要时脱敏显示

另外：本项目会读取用户任意仓库的内容。MUST：调试时不要把用户仓库里的提交信息、文件内容、远程地址等原样大段贴进对话或文档，只保留定位问题所必需的最小片段。

## 12. `@path` 引用解析

- MUST：`AGENTS.md` 是唯一入口文件。
- MUST：`@path` 按仓库根目录解析，不按当前工作目录漂移。
- MUST：按出现顺序加载引用文件，并自动去重。
- MUST：支持递归解析和循环保护。
- MUST：禁止引用仓库外文件或敏感文件。
- SHOULD：只在涉及任务时报告已加载和未加载的引用文件；不要在无关闲聊中制造噪音。
- MUST：引用文件缺失时，说明具体路径和原因，但不中断其他可用规则加载。

## 13. Review rubric

> MUST：PR 合并前的 review 必须由**独立于实现者的人或 agent** 完成（见 §4.1 独立 review 门槛）；实现者的自查不满足该门槛。

Review 时优先检查：

1. failure path 是否正确
2. cleanup / rollback 是否缺失（尤其是 git 操作中途失败后仓库停在什么状态）
3. 空输入、`null`、`undefined`、空 pathspec、空参数列表是否安全——本项目已因空 pathspec 造成过误伤
4. 破坏性 git 操作是否有确认流程、是否可撤销、是否会丢用户数据
5. 并发刷新、文件监听重入、重复 invoke 是否安全
6. 跨文件隐藏耦合是否增加（尤其是组件绕过 store 直接 `invoke`）
7. Tauri 命令签名改动是否前后端同步（`snake_case` ↔ `camelCase`，§7.3）
8. 是否缺少回归测试；测试是否真的建了 fixture 仓库而不是依赖现有仓库
9. 测试的**输入是否真是实际场景的形态**：名为「覆盖 X」的测试若测的不是 X 的真实形态，属虚假信心，阻断
10. 是否引入 ghost dependency
11. 跨平台路径行为（Windows 反斜杠、盘符、大小写）是否被覆盖
12. 若本 PR 执行 `docs/superpowers/plans/` 下的计划，是否附带 `<计划文件名>-decisions.md`（§2.1）。**先读该文件的「偏离计划」条目再看 diff** —— 与计划不一致但未记录的改动，视为未经说明的偏离，阻断

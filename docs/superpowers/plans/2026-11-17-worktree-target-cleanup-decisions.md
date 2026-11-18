# worktree 的 target 清理脚本（第 83 条，方案 B）——实现日志

没有对应的计划文件：issue 本身就是需求，按 `AGENTS.md` §2.1「预计 ≥3 个 Task」建本日志。

## 进度总账

| Task | 内容 | 状态 | 产出文件 | 验证 |
|---|---|---|---|---|
| 1 | 纯逻辑：解析 worktree 列表、判定哪些 target 可清 | 完成 | `scripts/worktree-targets.mjs`（`parseWorktreeList`、`latestPrByBranch`、`classify`、`formatBytes`）、`tests/vitest/worktreeTargets.test.ts` | 先红（模块不存在）后绿 |
| 2 | 副作用：量大小、查占用进程、删除、命令行 | 完成 | 同上（`collect`、`cleanTargets`、`main`） | 临时 fixture 仓库（真实 `git worktree add` / `lock`）集成测试；本仓库只读实跑 |
| 3 | 文档：README / AGENTS.md 写入口 | 完成 | `README.md`「清理 worktree 的编译产物」、`AGENTS.md` §4.5 | 文档里的命令都实跑过，见 Task 3 |

## 决策条目

### Task 0：选方案

issue 列了三个方案，评论建议「先做方案 B（清理脚本）就能缓解」。本分支只做 B：

- A（共享 `CARGO_TARGET_DIR`）issue 里已记录会撞 `crate ... required to be available in rlib format`，
  要先实测多 worktree 交替编译，范围大，不在本分支
- C（收尾流程提醒）不解决已经堆着的 77 GB

脚本写成 Node（`.mjs`），不跟 `stale-branches.ps1` 一样写 PowerShell：判定规则是这个脚本里唯一容易错的部分，
写成导出的纯函数就能用 vitest 覆盖（先例：`gen-notices.mjs`、`perf-summary.mjs`）；PowerShell 脚本本仓库没有测试手段。

### Task 1 / 2：脚本

#### 偏离 issue 的提议

- issue 写「列出已合并分支的 worktree 及其 target 大小」。实际列出**所有**带 target 的目录（含主工作区、
  开着 PR 的、残留目录），只把已合并 / 已关闭 / 残留的标为可清：只看已合并的，用户不知道剩下的盘被谁占着
- 「已合并」只按 PR 状态判断，不像 `stale-branches.ps1` 那样逐个核对提交祖先关系：那边要删分支、错了丢提交；
  这里只删构建产物，判错的代价是重编一次
- 已关闭未合并的 PR 也标为可清：分支不会再编译；真要接着用，重编即可

#### 计划没写的坑

- Claude Code 给活跃会话的 worktree 加的锁，`git worktree list --porcelain` 里是一行
  `locked claude session <名字> (pid <n>)`（2026-11-17 实测）。脚本按「有 locked 行」判，不解析原因
- worktree 列表里的路径是正斜杠，扫描 `.claude/worktrees/` 得到的残留目录、进程的可执行文件路径是反斜杠；
  比较前统一成 `path.resolve` + Windows 下小写，显示时统一成反斜杠
- 量大小不跟随符号链接 / junction（`readdir` 的 Dirent 对链接不报 `isDirectory`），不会顺着链接量到 target 外面
- 本仓库实跑（只读，2026-11-17）：16 份 target、14 份可清。`follow-report-wording` 正被另一个会话 lock，
  标为保留；`vcs-phase0` 是 git 不认的残留目录。第一版逐个量、硬链接重复计：101.6 GB / 可清 87.0 GB，95 秒；
  review 修复后（并行量、硬链接只算一次）：90.3 GB / 可清 78.3 GB，18 秒（缓存是热的，两次不是严格对照）
- 实测 git 对经 junction 给出的 `worktree add` 路径会登记成真实路径（别名用例在修复前就通过）；
  仍按 realpath 比较，兜住 8.3 短名、subst 盘符等没测到的别名形态

### 独立 review（`/code-review high`，2026-11-17）

8 条全部成立，全部修复，每条先补失败用例：

| 发现 | 处理 |
|---|---|
| 1 lock / 占用 / 近期写入只在盘点时查一次，删除前不复查 | `makeRecheck`：删每一个之前重读 worktree 列表、进程列表，看 target 往下两层目录的时间戳 |
| 2 别名路径登记的 worktree 会被当成残留目录、绕过 lock | 比较一律走 `realpathSync.native` |
| 3 prunable 但目录还在的 worktree 既不按 worktree 列、也不按残留目录列 | prunable 的不算「已登记」，目录还在就按残留目录列出 |
| 4 硬链接重复计算 | 按 `(dev, ino)` 去重，`lstat` 取 bigint（Windows 的文件编号超过 2^53） |
| 5 「当前」只按 cwd 的 `rev-parse --show-toplevel` 判，站在残留目录里时会落到主工作区 | 当前目录和脚本所在目录都算，取路径最深的候选 |
| 6 target 本身是 junction 时顺着量、进程路径对不上 | target 是链接就不量、不清，标「target 是链接，不处理」 |
| 7 逐个量，太慢 | 各候选并行量 |
| 8 测试里进程路径与 target 都经 `path.join`，测不出混合写法 | 进程路径改成正斜杠 + 整串大写；补 current / prunable / 别名 / 硬链接 / junction / 复查用例 |

### Task 3：文档

按 `writing-user-runbooks` 走了一遍。文档里每条命令都在 PowerShell 里实跑过：

- 主工作区外（`~`）运行：Node 报 `Error: Cannot find module 'C:\Users\liuhao\scripts\worktree-targets.mjs'`，文档里照抄了这句
- 只读盘点：输出节选照抄真实输出（路径换成编造的 `C:\code\mygitclient`）
- `--clean` 在非交互环境：列表之后停在「不是交互终端，没法确认；确定要删就加 --yes」，退出码 2，什么都没删
- **没跑过**：真实终端里输入 `yes` 的交互确认（工具环境的 stdin 不是 TTY）。删除本身由 vitest 的 fixture 用例覆盖；
  提示语原文取自代码

### 为什么这么改是安全的

- 只删 `src-tauri/target`（构建产物，`cargo build` 能重新生成），不删 worktree、不删分支、不碰 git，
  因而不属于 `AGENTS.md` 开篇列出的破坏性 git 操作，也不撞 §4.3 的并发清理限制
- 默认只读，只打印；`--clean` 先列出要删的再要求输入 `yes`，非交互环境必须显式 `--yes`
- 不清：主工作区、脚本所在的 worktree、被 lock 的 worktree（Claude Code 给活跃会话加的锁）、
  有进程从 target 里运行的、target 近期还在写入的（可能正在编译）

# Phase 2c 实现决策日志

> `AGENTS.md` §2.1 要求的落库记录。配套计划：`2026-09-25-svn-phase2c-sync-in.md`
> 分支：`worktree-svn-phase2c-sync-in`，基点 `1db8210`（计划本身提交为 `e334a8a`）

## 进度总账

| Task | 内容 | 状态 | 产出 | 验证 |
|---|---|---|---|---|
| 0 | 决策日志 + 自检 | ✅ 完成 | 本文件 | 自检见下「Task 0」；`pnpm install` 成功 |
| 1 | 2b 遗留：watcher RAII、锁释放重试、journal tmp 清理、持锁时长 | ✅ 完成 | `commands/watch.rs`、`svn_lock.rs`、`svn_journal.rs` + 5 条测试 | RED 已见（2 编译失败 + 1 断言失败）；3 条变异各自只红对应测试；fmt / clippy 0 告警 / `cargo test` 608 passed 0 failed |
| 2 | 前置检查 | ⬜ | | |
| 3 | WIP 压栈与取回 | ⬜ | | |
| 4 | 基线录制零件拆分 | ⬜ | | |
| 5 | 编排主路径 S0–S11 + 冲突出口 | ⬜ | | |
| 6 | 继续 / 放弃（含回原分支） | ⬜ | | |
| 7 | S3 / S4 / S5 失败回退 | ⬜ | | |
| 7A | 外部整体 update 之后由「更新」接管（A16） | ⬜ | | |
| 8 | Tauri 命令 | ⬜ | | |
| 9 | 前端纯逻辑 | ⬜ | | |
| 10 | 前端接线 | ⬜ | | |
| 11 | E2E + spec 修订 + 收尾 | ⬜ | | |

## 全量验证

| 命令 | 结果 |
|---|---|
| `cargo fmt --check` | ⬜ |
| `cargo clippy --all-targets` | ⬜ |
| `cargo test` | ⬜（基线：master `1db8210` 上 598 passed） |
| `pnpm build` | ⬜ |
| `pnpm test` | ⬜（基线 383 passed） |
| `pnpm tauri dev` | ⬜ |

---

## 开工前实测（写计划时在临时 fixture 上跑过，执行者不必重跑）

计划「对 spec 的补足与偏离」表的依据。git 2.51.2.windows.1，svn 1.14.2-SlikSvn。探针脚本在会话 scratchpad（`probe-*.ps1`），不进仓库。

| # | 内容 | 实测依据 |
|---|---|---|
| A1 | pinned 模式下 `sync_in` 动手前拒绝（用户决定） | —— |
| A2 | 新增 phase `applying-stash` 与字段 `stashApplied` | T2：对已 apply 过的 stash 再 apply，git 拒绝且不动文件 |
| A3 | S10 apply 前查未跟踪文件撞名 | R1：撞名时 git 先写已跟踪部分再失败（退出 1，无 `UU`），半套 apply |
| A4 | S5 成败不看退出码 | Q2 / Q3：文本冲突、树冲突时 `svn update --accept postpone` 退出 0；树冲突节点 `item="added" tree-conflicted="true"` |
| A5 | S5 之后放弃：断言 W-file svn 干净，再 `checkout -f` 回原分支（用户决定） | R2：普通 checkout 会把服务器内容带到原分支变成本地修改 |
| A6 | 任何 svn 冲突态都在前置检查拒绝 | W-file 的冲突标记在 stash 后仍留在 `.svn` 里，S4 必败 |
| A7 | `stashed` 且无 `stashOid` ⇔ 没有可压的改动 | P1：无可压内容时 `stash push` 退出 0、`refs/stash` 不变 |
| A8 | S10 不带 `--index`，暂存状态丢失、内容不丢 | R3 |
| A9 | S7 不跑体检，候选索引从 HEAD 树起步 | spec §6.8.5、§6.3.1 |
| A10 | `start` 阶段继续 = 放弃 = 删 journal | 定义 |
| A11 | 从 `stashed` 继续前复核 HEAD / 分支 / 基线 | 崩溃后没有锁 |
| A12 | S3 / S4 失败回退用 `checkout -f`，不做 A5 断言（仅本进程刚走到这里时） | S5 未跑，磁盘被跟踪内容全来自 git |
| A13 | `flow = sync_out` 的 journal 继续 / 放弃一律拒绝 | Phase 3 才有 |
| A14 | 冲突出口放锁，继续 / 放弃重新拿锁并复核 | spec §7.4 |
| A15 | 每次 checkout 后核对真的切干净了 | L1：share=Read 占用时 checkout 退出 0 而文件没写进去；L1′：share=None 时整体拒绝 |
| A16 | 外部整体 update（`nodeRevs` 单值且大于基线）由「更新」接管（用户决定） | `probe-ext-update.ps1`：混在一起 → 重建基线 + S1–S10 后重新分开；相邻行变成 stash 冲突 |
| A17 | 撞名预检放宽：磁盘 / stash / HEAD 三处同一个 blob 不算撞名 | `probe-ext-added.ps1`：服务器新增文件被 `-u` 收进 stash，不处理时 `already exists, no checkout` 半套放回 |

---

## Task 0：开工自检

`AGENTS.md` §4.2（2026-09-26 实测）：

```text
- 当前分支 / worktree：worktree-svn-phase2c-sync-in / .claude/worktrees/svn-phase2c-sync-in
- 是否已合并进 master：否（`merge-base --is-ancestor HEAD origin/master` exit=1）
- 是否基于最新 master：是（`merge-base --is-ancestor origin/master HEAD` exit=0）
- 是否允许在当前位置修改：是（独立 worktree，非 master）
- 目标 worktree 是否已切入：是，Get-Location = C:\Users\liuhao\workspace\dev\Dev\mygitclient\.claude\worktrees\svn-phase2c-sync-in
- 是否存在未提交改动：否
- git log --oneline -3：e334a8a（计划）/ 1db8210（#46）/ 4ae3e34
```

### 偏离计划

- 计划 Step 1 期望「`git log` 第一行是 `1db8210`」。实际是 `e334a8a`：用户要求先单独提交并推送计划文档，所以基点之上多了这一个纯文档提交。不影响任何代码 Task。

### 计划的错误

- 无。

### 计划没写的坑

- PowerShell 下 `git commit -F - @'...'@` 不会把 here-string 送进 stdin，而是当成 pathspec，报 `did not match any file(s)`；提交信息要先写进文件再 `-F <文件>`。
- 本 worktree 的 Bash 工具会拒绝「形式复杂」的 git 命令（worktree 隔离检查），git 操作一律走 PowerShell 的简单命令。

### 为什么这么改是安全的

- 本 Task 只新增文档，不碰代码与仓库状态。

---

## Task 1：2b 遗留——watcher RAII、锁释放重试、journal tmp 清理、持锁时长

### 验证记录

| 步骤 | 实际结果 |
|---|---|
| RED | `svn_journal` 新用例断言失败「残留 tmp 必须被清掉」；`svn_watch_suspend` / `svn_lock` 编译失败（`WatchSuspension`、`describe_age` 不存在），与计划期望一致 |
| GREEN | `cargo test --test svn_watch_suspend --test svn_lock --test svn_journal`：6 / 10 / 11 passed |
| 变异 1：`WatchSuspension::drop` 不调 `resume_watch` | 只红两条 guard 测试 |
| 变异 2：锁 `Drop` 重试 `0..1` | 只红 `release_retries_when_a_reader_briefly_holds_the_file` |
| 变异 3：不调 `sweep_stale_tmp` | 只红 `a_stale_tmp_left_by_a_crash_is_swept_on_the_next_write` |
| 全量 | `cargo fmt --check` 0；`cargo clippy --all-targets` 0 告警；`cargo test` 608 passed / 0 failed（46 个测试二进制） |

### 偏离计划

- 无代码偏离。计划给的两段测试代码不满足 rustfmt（长行），`cargo fmt` 自动折行，语义不变。

### 计划的错误

- 计划写的 `cargo test` 基线是 598 passed；本 Task 加 5 条后实测 608。差出的 5 条没有逐一核对来源（基线数字抄自 2b 决策日志，统计口径可能不同，例如是否计入 doc-test）。**不影响结论**：0 failed。以后以本表的 608 为新基线。

### 计划没写的坑

- `cargo fmt` 会把 CRLF 的测试文件整体写回成 LF。`git diff --stat` 只显示真实增删（autocrlf 在提交时归一），所以没有造成整文件改动；但用脚本处理这些文件前要重新数行尾。
- `release_retries_when_a_reader_briefly_holds_the_file` 在实现前处于编译失败状态，没法先看到它的「断言失败」。用变异 2 补上：重试降到 1 次时它确实红，证明测的是重试本身。

### 为什么这么改是安全的

- `WatchSuspension` 只把既有的 `suspend_watch` / `resume_watch` 包进 RAII，挂起集合的语义不变；Drop 在 panic 展开时也会跑（测试钉住）。
- 锁 `Drop` 的重试最多等 10 × 20ms = 200ms，且只在删除失败、文件仍在时才等；`NotFound` 立刻返回。Drop 里依旧不 panic。
- `sweep_stale_tmp` 只删「`mygitclient-sync.json.` 开头且 `.tmp` 结尾」的文件，同目录的 `.bak` 等不相干文件由测试证明不被误删；只有持锁者写 journal，写入时的其它 tmp 必然是残骸。
- journal 写入失败时现在会删掉自己的 tmp（原来只有 rename 失败那条路径会删），目标文件本身从未被触碰，原子性不变。

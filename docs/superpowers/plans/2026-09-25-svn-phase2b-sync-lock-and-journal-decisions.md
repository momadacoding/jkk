# Phase 2b 实现决策日志

> `AGENTS.md` §2.1 要求的落库记录。§2 的 checkpoint 只活在对话里，上下文压缩、
> 会话中断、`/compact` 失败都会让它消失；这份文件是跨轮次的唯一状态来源。
>
> 配套计划：`2026-09-25-svn-phase2b-sync-lock-and-journal.md`
> 分支：`worktree-svn-phase2b-lock`，基点 `821a735`

---

## 进度总账

| Task | 内容 | 状态 | 产出 | 验证 |
|---|---|---|---|---|
| 0 | 建决策日志 + worktree | ✅ 完成 | 本文件 | §4.2 自检通过：分支 `worktree-svn-phase2b-lock`、基点 `821a735`、`pwd` 实测在 worktree 内、无未提交改动 |
| 1 | 跨进程锁的获取与释放 | ✅ 完成 | `svn_lock.rs`、`tests/svn_lock.rs`、`engine/mod.rs` | `cargo test --test svn_lock` 4 passed（计划里是 3，见下）；fmt / clippy 零告警；变异验证 ×2 |
| 2 | 崩溃后锁真的松开（跨进程实测） | ✅ 完成 | `tests/svn_lock.rs`（+2 条） | `cargo test --test svn_lock` 6 passed；对 spec 原方案（pid 探测）做变异，2 条变红 |
| 3 | 持锁期间拒绝写操作 | ⬜ 未开始 | | |
| 4 | 写前 journal | ⬜ 未开始 | | |
| 5 | 同步期间挂起文件监听 | ⬜ 未开始 | | |
| 6 | 「上次同步未完成」的检测与展示 | ⬜ 未开始 | | |
| 收尾 | E2E 清单 + 独立 review + spec 回写 | ⬜ 未开始 | | |

## 全量验证

| 命令 | 结果 |
|---|---|
| `cargo fmt --check` | ⬜ 未跑 |
| `cargo clippy --all-targets` | ⬜ 未跑 |
| `cargo test` | ⬜ 未跑（基线 567 passed） |
| `pnpm build` | ⬜ 未跑 |
| `pnpm test` | ⬜ 未跑（基线 369 passed） |
| `pnpm tauri dev` 真实窗口 | ⬜ 未跑 |

> 本仓库没有 PR 门禁 CI（`AGENTS.md` §4.1）。以上全部要本地实跑，如实填。
> **本期验收不是「能更新」**——`sync_in` 编排在 2c。

---

## Task 0：施工前复核计划

### 偏离计划

无。

### 计划的错误

计划在写作阶段的自审里已经修掉 5 处（`serde_json` 的 `?`、Rust 断言里未转义的
双引号、两处多余的"补 `pub mod`"指示、不存在的 `<Banner>` 组件、两个只说做什么
不给代码的步骤）。施工阶段若再发现，记在对应 Task 下。

### 计划没写的坑

**一、fresh worktree 没有 `node_modules`。** 2a 踩过一次：`pnpm test` 报
`'vitest' is not recognized`。本次在 Task 1 之前就起了 `pnpm install`（走
PowerShell，Bash 工具没有网络——见 memory `bash-tool-no-network`）。

### 为什么这么改是安全的

本 Task 只新建文档，不动代码。

---

## Task 1：跨进程锁的获取与释放

### 偏离计划

**一、多了一条测试 `a_normal_release_removes_the_lock_file_itself`。** 计划里是 3 条，
实际 4 条。多出来的那条是变异验证逼出来的，见下面「计划没写的坑」第一条。

**二、`assert_not_syncing` 的函数本体在本 Task 就一起写了**（计划把它排在 Task 3
Step 3）。同一个文件，分两次写只是多一次编辑；Task 3 的测试扫的是 `vcs/svn.rs`
的调用点，不是这个定义，所以不会让 Task 3 的测试提前变绿。Task 3 只做接线。

### 计划的错误

**一、`SyncLock` 没有 `Debug`，测试编译不过。**

```text
error[E0277]: `SyncLock` doesn't implement `Debug`
  --> tests\svn_lock.rs:25:40
   |   let err = acquire(&root, "opid-B").expect_err(...)
```

`Result::expect_err` 要求 `Ok` 那一侧 `T: Debug`。`std::fs::File` 自己实现了
`Debug`，所以 `#[derive(Debug)]` 就够。已在结构体上加，并注明是测试要的。

**二、计划预测「变异 B 只有 1 条测试变红」，实测是 2 条。** 多出来的那条不是
测试写松了，是**实现里真有个 bug**——见下。

### 计划没写的坑

**一、`Drop::drop` 里删不掉锁文件，而且失败被吞掉（真 bug，变异测试抓到的）。**

把 `is_held` 变异成「文件存不存在」之后，`a_live_lock_blocks_a_second_acquisition`
也跟着红了，报的是 `Drop 之后必须释放`。顺藤摸下去发现根因与变异无关：

- `Drop::drop(&mut self)` 在**字段被释放之前**运行，所以执行 `remove_file` 那一刻
  文件句柄还开着；
- 而 `create_new_exclusive` 用的是 `share_mode(FILE_SHARE_READ)`，**没有**
  `FILE_SHARE_DELETE`，于是 Windows 上 `remove_file` 以 ERROR_SHARING_VIOLATION 失败；
- 失败又被 `let _ =` 吞掉。

后果：每一次**正常**同步结束后锁文件都留在磁盘上。功能上不致命（`is_held` 看的是
句柄不是文件，所以残锁会被正确判成「无人持有」并接管），但下一次 `acquire` 会打出
`接管了一把无人持有的同步锁（上次同步没有正常收尾）` ——**一条永远为假的告警**。
告警一旦永远为真，它就等于不存在，真出事那次也不会有人看。

修法是在 `Drop` 里先 `self.file.take()` 把句柄关掉再删（字段因此包成 `Option`），
**不是**去加 `FILE_SHARE_DELETE`：加了就等于允许别的进程删掉我们正持有的锁文件，
那会让另一个进程删掉再重建，两边都以为自己拿着锁。

原来的三条测试全都抓不到这件事——它们只问 `is_held`，而 `is_held` 在这个 bug 下
仍然给出正确答案。必须**直接看文件**才看得见，所以新增了那条测试。

**二、rustfmt 会自己把 `pub mod` 重排成字典序。** 我按计划插在
`svn_include_check` 后面，`cargo fmt` 把它挪到了 `svn_init` 之后（正确位置）。
不用手工纠结插哪一行，交给 fmt。

### 变异验证（已跑）

| 变异 | 结果 |
|---|---|
| A：`Drop` 里不先关句柄（恢复成有 bug 的顺序） | 只有 `a_normal_release_removes_the_lock_file_itself` 变红 |
| B：`is_held` 换成 `lock_path().exists()` | 修掉 bug 之后，只有 `a_lock_file_left_behind_by_a_crash_is_not_held` 变红（与计划预测一致） |

两条都精确命中单条断言，没有连带。

### 为什么这么改是安全的

- **锁的正确性不依赖清理代码**。句柄由操作系统在进程结束时回收，这一条已由
  上一轮的三组探针实测（计划头部那张表），Task 2 还会用真外部进程再验一次。
- **`Drop` 不 panic**。展开途中 panic 会直接 abort；删不掉的后果只是留个残锁，
  下次照常接管，所以仍然 `let _ =`，只是把句柄先关了让它真的能删掉。
- **接管路径会写日志**（§5.1.1 要求），且修掉上面那个 bug 之后，这条日志只在
  真正的异常退出后才出现。
- **非 Windows 是明确的降级**，在 `is_held` 的文档注释里写明了「恒返回 false」，
  没有假装有跨进程语义。本项目主力平台是 Windows，与 `file_lock.rs` 同一处理。

---

## Task 2：崩溃后锁真的松开（跨进程实测）

### 偏离计划

无。计划预判「这两条测试在 Task 1 的实现下会直接绿」，实测确实如此——它们不是
驱动新实现的测试，是给「操作系统会替我们松手」这条假设**上锁**的测试。

### 计划的错误

无。

### 计划没写的坑

**一、对 spec 原方案（pid 探测）做变异，红的是两条，不是一条——第二条的成因
比预想的更糟。**

把 `is_held` 换成「读 journal 里的 pid，用 `tasklist /FI "PID eq <pid>"` 判断存活」
之后：

```text
a_stale_lock_naming_a_live_unrelated_pid_is_still_takeable ... FAILED   ← 预期内：pid 复用
a_lock_whose_holder_was_killed_is_not_held                 ... FAILED   ← 预期外
```

第二条失败在**外部进程正持有时本进程却拿到了锁**那一句。成因：测试里的
powershell 只是 `File::Open` 把文件开着，**从没写过 pid 元数据**；pid 探测要先
`holder()` 读出 pid 才能判，读不到就只能当「没人持有」。

这暴露了 pid 探测的一个比 pid 复用更基本的缺陷：**它信的是文件内容，而不是
持有这个事实本身**。真实世界里对应两种情形——

1. 拿到锁与写完元数据之间有一条缝，崩在那里就留下一个「有人持有过但没写名字」
   的锁；
2. 元数据写了一半（断电 / 磁盘满），JSON 解析失败，同样读不出 pid。

两种情形下 pid 探测都会说「没人持有」并**直接接管一把可能真有人在用的锁**，
方向恰好是危险的那一边。句柄探测不受影响：它问的是操作系统，不是文件内容。

这条写进 spec §5.1.1 的回写里（收尾 Task），不能只写「pid 复用」那一条理由。

**二、跨进程测试要轮询，不能固定 sleep。** 子进程起来并真的打开文件需要时间，
机器忙时更久。用 `is_held` 轮询 50×100ms 作为前提检查（`assert!(opened, "测试前提
不成立")`），比 `sleep(800ms)` 稳，也比它诚实——前提没达成时会明确说测试没测到东西，
而不是静默通过。

### 变异验证（已跑）

| 变异 | 结果 |
|---|---|
| `is_held` 换成 spec 原写的 pid 探测（tasklist） | 恰好 2 条变红，成因见上；其余 4 条仍绿 |

### 为什么这么改是安全的

- **这两条测试不改实现，只固定假设**，所以不存在「为了让测试过而放宽行为」的风险。
- **跨进程那条只杀自己 spawn 出来的 powershell**，不碰任何仓库，符合
  `AGENTS.md` 最高优先级那条（不拿真实仓库当试验场）。
- **`#[cfg(windows)]` 圈住**：非 Windows 上没有等价语义，不留一条在别的平台上
  必然假绿的测试。

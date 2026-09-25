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
| 3 | 持锁期间拒绝写操作 | ✅ 完成 | `vcs/svn.rs`、`tests/svn_write_guard.rs`、`tests/svn_lock.rs`（+1） | `cargo test` **576 passed**（基线 567，新增 9）；fmt / clippy 零告警；变异验证已跑 |
| 4 | 写前 journal | ✅ 完成 | `svn_journal.rs`、`tests/svn_journal.rs`、`engine/mod.rs` | `cargo test` **585 passed**（新增 9）；fmt / clippy 零告警；变异验证 ×2 |
| 5 | 同步期间挂起文件监听 | ✅ 完成 | `commands/watch.rs`、`tests/svn_watch_suspend.rs` | `cargo test` **589 passed**（新增 4）；fmt / clippy 零告警；变异验证 ×2 |
| 6 | 「上次同步未完成」的检测与展示 | 🟡 代码完成，待真实窗口验证 | `commands/svn.rs`、`lib.rs`、`svnJournal.ts`、`svnJournal.test.ts`、`repoStore.ts`、`BaselineBanner.tsx`、`RepoTab.tsx`、`tests/svn_backend.rs`、spec §5.1.2 + §18 | `cargo test` **590 passed**；`pnpm test` **378 passed**（新增 9）；`pnpm build` OK。**`pnpm tauri dev` 未跑** |
| 7 | 修入独立 review 的 3 条阻断 + 7 条非阻断 | 完成 | svn_lock.rs、svn_journal.rs、commands/ops.rs、commands/svn.rs、svn_write_guard.rs、svn_lock.rs 测试、svnJournal.ts、repoStore.ts、RepoTab.tsx、E2E | cargo test 595 passed；pnpm test 379 passed；变异验证 x3 |
| 8 | 修入第二轮独立 review 的 4 条（2026-09-25） | ✅ 完成 | `svnJournal.ts`、`svnJournal.test.ts`（+2）、`repoStore.ts`、`BaselineBanner.tsx`、E2E 清单、spec §5.1.2 + §18、本文件 | `pnpm build` OK；`pnpm test` **381 passed**（+2，先红后绿）；Rust 未改，未跑 cargo |
| 收尾 | E2E 清单 + 独立 review | 完成 | E2E 清单已写；review 已跑两轮并修入 | 真实窗口仍待用户实跑 |

## 全量验证

| 命令 | 结果 |
|---|---|
| `cargo fmt --check` | ✅ OK |
| `cargo clippy --all-targets` | ✅ **0 告警** |
| `cargo test` | ✅ **595 passed / 0 failed**（基线 567，新增 28） |
| `pnpm build` | ✅ OK（含 `tsc`） |
| `pnpm test` | ✅ **381 passed**（基线 369，新增 12；Task 8 +2） |
| `pnpm tauri dev` 真实窗口 | ⬜ **未跑** —— 新增命令 `svn_sync_state` 的 invoke 路径尚未验证（`AGENTS.md` §7.3 硬性要求），待用户实跑 |

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

---

## Task 3：持锁期间拒绝写操作

### 偏离计划

**一、`SvnGitBackend` 自己存了一份 `root: String`。** 计划已经预见到这一点并给了
两个选项（「`GitBackend` 若没有公开访问器，就自己存一份」），实测确认
`GitBackend::root` 是私有字段且没有访问器，所以走了后者。不去给 `GitBackend` 加
public 访问器：那会扩大一个与本期无关的公开接口（`AGENTS.md` §1.2）。

### 计划的错误

**一、`extensions: vec!["cs".into()]` 少了前导点。**

```text
GitError { code: "svn_config", message: "扩展名必须以 . 开头：cs" }
```

`svn_config` 在校验阶段就拒了。正确写法是 `".cs"`，与 `tests/svn_init.rs::req`
里那份一致。已修并在测试里就地注明。

### 计划没写的坑

**一、`src/vcs/svn.rs` 是 CRLF，其余被改过的文件是 LF。** 用只含 LF 的模式批量
替换会一条都匹配不上（第一次尝试 12 条全部 `NOT FOUND`）。做法：读进来先把
CRLF 归一成 LF，改完再整体写回 CRLF，并在写回前断言「原文是纯 CRLF」——
混合行尾的文件按这个流程会被悄悄统一，那是不该由本期顺手做的改动。

改完复核 `CRLF=141 bare-LF=0`，没有混入 LF。

**二、两层测试各自只能抓一半，缺一不可。**

| 测试 | 抓得到 | 抓不到 |
|---|---|---|
| `svn_write_guard.rs`（扫源码） | 清单里的方法出现回归 | 加了但没生效 |
| `staging_is_refused_…`（真后端） | 加了但没生效 | 除 `stage` 外其余 11 个方法 |

变异（只去掉 `stage` 的守卫）实测两边都红：扫描那条直接点名 `stage`，行为那条
在真实 `backend_for` 路径上报错。只留任意一条都会留下一个真实的漏洞形态。

> ⚠️ **本节第二条当时写得过于乐观，见 Task 7 的 B2。** 那张表里「新增写方法时
> 忘了加」其实**抓不到**——`WRITE_METHODS` 是硬编码清单，新方法根本不在里面。
> 独立 review 指出后已改成枚举实际存在的方法。上面保留原文不改，因为这正是
> 「自己给自己的测试写了一句它做不到的承诺」的现场。

**三、反向约束和正向一样重要。** `read_methods_are_not_guarded` 守住
「读不阻塞」（spec §5.1）。没有它的话，下一个人「为了保险」给 `status` 也加上
守卫是很自然的动作——而后果是同步期间界面整个瞎掉，恰恰在最需要看清状况的时候。

### 变异验证（已跑）

| 变异 | 结果 |
|---|---|
| 去掉 `stage` 的 `assert_not_syncing` | `every_write_method_checks_the_sync_lock` 点名 `stage` 变红；`staging_is_refused_while_a_sync_holds_the_lock` 同时变红。其余全绿 |

### 为什么这么改是安全的

- **只加前置判断，不改任何既有行为**。12 个方法的实现仍原样委托给 `GitBackend`，
  没持锁时行为与改动前逐字相同（`cargo test` 全量 576 passed，既有 567 条无一变化）。
- **读路径一个字没动**，spec §5.1 的「读不阻塞」由反向测试钉住。
- **守卫用的是同一把 `is_held`**，与 `acquire` 判据一致，不存在「拿锁看一套、
  拒绝看另一套」的分裂。
- **普通 git 仓库不受影响**：守卫只加在 `SvnGitBackend` 上，`GitBackend` 没动。

---

## Task 4：写前 journal

### 偏离计划

**一、多了两条测试。** 计划 7 条，实际 9 条：

- `flow_uses_the_exact_strings_from_the_spec`——`Phase` 的线序名有测试钉着，
  `Flow` 却没有。它同样是落盘格式，且它决定恢复入口走 `sync_in` 还是 `sync_out`
  那条分支（§7.6 明写两者的 `publishing-baseline` 不能互相套用），写错的后果不比
  `Phase` 轻。
- `a_stash_oid_survives_the_round_trip`——§7.6.1 是整份 spec 里点名最重的一条
  （「按不可变 OID 记，不能记 `stash@{N}`」），但计划里没有任何测试碰 `stash_oid`
  字段。它是崩溃恢复唯一的锚点，必须证明它真的落盘并读得回来。

**二、`Default` 用 `#[derive(Default)] + #[default]` 属性，不是手写 `impl`。**
计划给的是手写 `impl Default for Phase / Flow`。手写版会被 clippy 的
`derivable_impls` 盯上；`#[default]` 变体属性从 Rust 1.62 起可用，本 crate 的
`rust-version` 是 1.77.2，够用。

### 计划的错误

无（Task 4 的计划代码一次编译通过）。

### 计划没写的坑

**一、`serde` 的 `skip_serializing_if` 与 `#[serde(default)]` 必须成对。**
结构体上标了 `#[serde(rename_all = "camelCase", default)]`，字段上才能用
`skip_serializing_if` ——否则写出去的 JSON 缺字段，读回来就报 `missing field`。
这一对是 `sync_in` 写出的 journal 能保持干净（不含一堆 `sync_out` 的 null）
同时又能被读回的前提。

**二、计划担心「原子写那条测试可能撞不上窗口」，实测不用担心。**
去掉原子写（改成 `std::fs::write` 直接覆盖）之后连跑 3 次**全红**，且报的是
`EOF while parsing a value at line 1 column 0` ——读到的确实是被截断的文件，
不是别的原因。所以计划里那条「撞不上就删掉并如实说明」的后备方案没有启用，
测试保留。

**三、`read` 是本模块唯一允许不持锁调用的函数。** 已写进模块文档：UI 要在
没拿锁的情况下显示「上次同步未完成」（spec §5.1「读操作不阻塞」），但读到的
可能是另一个进程正在推进的状态，所以它的结论**只能用来显示，不能用来决定动作**。
不写明这一点的话，下一个人很自然会拿 `read` 的结果去驱动恢复逻辑。

### 变异验证（已跑）

| 变异 | 结果 |
|---|---|
| `read` 的坏 JSON 分支改成 `Ok(..ok())`（静默退化成"没有"） | 只有 `a_corrupt_journal_is_an_error_not_a_default` 变红 |
| `write` 去掉临时文件 + rename，改成直接覆盖 | `overwriting_never_leaves_a_half_written_file` 连跑 3 次全红，报 `EOF while parsing` |

### 为什么这么改是安全的

- **纯新增模块**，没有任何既有调用方，全量 585 passed 里原有的 567 条一条没动。
- **坏 journal 绝不静默吞掉**。这是整个模块最关键的一条不变量：吞掉它等于把
  用户 stash 的 OID 丢了，而恢复代码会以为「上次什么都没做」——但磁盘上工作区
  已经被清空了。由变异验证坐实。
- **原子写已由变异坐实**，不是靠「代码看起来写对了」。
- **`sync_out` 的字段现在就定义**，避免 Phase 3 改文件格式导致两期写出的 journal
  互相读不了；但它们全部 `skip_serializing_if`，`sync_in` 写出来的文件不含它们。

---

## Task 5：同步期间挂起文件监听

### 偏离计划

无。四条测试与计划一致。

### 计划的错误

**一、计划 Step 2 说「若 `commands` 模块不是 `pub`，在 `lib.rs` 里改成
`pub mod commands;`」，实际早就是 `pub`。** 这一条在写计划的自审阶段已经改掉了
（连同 `pub mod proc;` 那条），施工时确认无需改动。

### 计划没写的坑

**一、`src/commands/watch.rs` 也是 CRLF。** 与 `src/vcs/svn.rs` 同样的坑
（Task 3 记过）。这次直接用编辑工具而不是脚本批量替换，避开了行尾问题；
改完复核 `CRLF=189 bare-LF=0`。

**二、「丢弃」而不是「攒着」。** 挂起期间收到的事件直接扔掉，不缓存。攒着的话
恢复的瞬间会连发几百条 `repo-changed` ——那正是挂起想避免的刷新风暴，只是推迟了
几秒发生。恢复端由调用方主动发一次（spec §5.2 明写），一条就够。

**三、`suspend` / `resume` 静默吞掉锁中毒。** 这两个函数返回 `()` 而不是
`Result`：挂起失败的后果只是同步期间界面多刷新几次，而在这里往上抛错会让整个
同步流程因为一件无关紧要的事中止。已在函数文档里写明这个取舍。

### 变异验证（已跑）

| 变异 | 结果 |
|---|---|
| A：发事件前不查挂起状态（`if true`） | 只有 `the_debounce_loop_actually_checks_the_suspension` 变红 |
| B：三处 `normalize(path)` 换成 `path.to_string()` | 只有 `suspension_is_path_normalized` 变红 |

变异 A 正是「集合写对了但没人查它」那种无声失效；变异 B 是 Windows 上
`C:epo` 与 `C:/repo` 变成两个键、挂起静默失效。两条都只表现为「界面偶尔闪
一下」，没有断言钉着就不会有人发现。

### 为什么这么改是安全的

- **默认行为完全不变**：`SUSPENDED` 空集时 `is_watch_suspended` 恒为 `false`，
  去抖线程照原样发事件。全量 589 passed 里既有的 567 条一条没动。
- **不碰引用计数**：挂起是另一个独立集合，`WATCHERS` 的登记 / 引用计数逻辑
  一行未改，多标签页共用 watcher 线程的行为不受影响。
- **没有调用方**：本期只把能力做出来，`suspend_watch` / `resume_watch` 的实际
  调用点在 2c 的 S0 / S11。所以这一期不可能因为它改变任何现有行为。

---

## Task 6：「上次同步未完成」的检测与展示

### 偏离计划

**一、多了一条 Rust 测试 `sync_state_serializes_to_camel_case`。**
`commands/svn.rs` 的模块头自己写着「全部结构体都带 `#[serde(rename_all =
"camelCase")]`，并在 `tests/svn_backend.rs` 里用断言钉住序列化后的字段名」——
新增两个结构体就得守这条既有约定，计划漏了它。这条钉住 `startedAt`
（前端 `svnJournal.ts` 直接读这个名字），以及 `flow` / `phase` 的字面值。

**二、多了一条 vitest「stashing 阶段必须明确叫用户不要自己去 apply」。**
spec §7.6 的 phase 表对 `stashing` 写的是「**不静默 apply**」，而用户自己挑一条
stash 去 apply 极可能挑到别人的 WIP（§7.6.1 实测过 `stash@{0}` 会被挤走）。
这是整组文案里唯一一条「不照做会直接丢数据」的，值得单独钉住。

**三、Step 3（后端命令）与 Step 4/5（前端）的顺序调换了。** 先写完前端纯逻辑
和它的测试，再接后端。对结果没有影响——两侧的契约由上面那条序列化断言连起来。

### 计划的错误

无。Task 6 的代码一次通过；`BaselineBanner` 的真实形状（没有通用 `<Banner>`
子组件、让位靠把 model 算成 `null`）在写计划的自审阶段就已核对并改正，
施工时没再踩。

### 计划没写的坑

**一、`cargo fmt` 会把 `use` 语句重排，插在哪一行无所谓。** 我按「相关的放一起」
插了四行 import，fmt 按字典序拆开重排了。与 Task 1 的 `pub mod` 同一回事。

**二、`cargo fmt` 把 `commands/svn.rs` 整个文件的行尾从 CRLF 改成了 LF。**
复核 `git diff --stat` 确认该文件只有 `+50`，没有产生整文件 diff——仓库是
`core.autocrlf=true`，索引里本来就是 LF，磁盘形态的差异由 git 吸收掉了。
（这正是 Phase 2a 那一期在处理的同一层机制。）

**三、`locked` 与 `unfinished` 必须是两个字段，不能合并成一个布尔。**
两者的出口完全相反：`locked` 要用户**等**，`unfinished` 要用户**决定继续还是
放弃**。合并之后，界面会在一次正常同步进行中劝用户去「放弃」。已写进
`SyncState` 的文档注释，并由 vitest 的「正在同步时说的是『进行中』，不是
『未完成』」钉住。

**四、本期的横幅没有按钮，这是有意的。** 「继续 / 放弃」要能真正收尾
（apply stash、切回分支、退基线），那是 2c 的 `sync_in` 编排。挂两颗点不动的
按钮比没有按钮更糟，所以 `actions: []` 并用 `note` 明说入口在下一期。
有一条 vitest 钉住这一点，避免下一个人「顺手」加上去。

### 待办：真实窗口验证（Step 8，**未跑**）

`AGENTS.md` §7.3 硬性要求：新增 Tauri 命令的 `snake_case` ↔ `camelCase` 转换
没有编译期检查，必须实跑一次 `pnpm tauri dev` 确认 invoke 真的通。

`svn_sync_state` 目前**只经过了单测与序列化断言**，invoke 路径没验过。
造现场的步骤写在 E2E 清单里。

### 为什么这么改是安全的

- **纯新增 + 纯读。** `svn_sync_state` 一个字节都不写，也不拿锁（它恰恰是用来
  观察锁的）。既有 567 条 Rust 测试与 369 条前端测试一条没动。
- ~~**坏 journal 不被吞。** 命令里 `svn_journal::read(&path)?` 直接往上抛；
  前端 `refreshSvnSyncState` 的 catch 只影响横幅显示，错误本身照常由通用错误
  提示呈现。~~ **这条论证是错的**（第二轮独立 review 指出，已复核）：那个 catch
  只调了 `console.warn` 并写回 `null`，没有任何报错路径——坏 journal 在界面上
  恰恰被吞成了「一切正常」。修法见 Task 8。
- **让位规则收敛在一处。** `otherBannersAreSuppressed` 与既有的
  `syncBannerIsSuppressed` 同形，由 vitest 钉住，不是散在组件里的 `&&`。
- **前端读失败写 `null` 而不是编造状态**，与既有 `refreshSvnEol` 完全同形。

---

## Task 7：修入独立 review 的发现（2026-09-25）

`AGENTS.md` §4.1 的独立 review 门槛，由一个全新上下文的 agent 按 §13 rubric 执行。
报了 **3 条阻断 + 12 条非阻断**。三条阻断**逐条由我自己复现后确认成立**，
不是照单全收。

### 三条阻断（全部已修）

**B1：三条会写仓库的命令绕过了 `trait Vcs`，完全没有守卫。**

`assert_not_syncing` 只加在 `SvnGitBackend` 的 12 个方法上，而这三条不走那里：

| 命令 | 它干什么 | 为什么可达 |
|---|---|---|
| `ops::revert` | `git revert --no-edit`，移动 HEAD + 重写工作目录 | **自己复核过**：`commitActions.ts` 的能力过滤只关掉 `new-tag` / `cherry-pick` / `save-patch`，`revert` 走 `?? true`，在 SVN 混合仓库上完全可用 |
| `svn_rebuild_baseline` | 移动 `svn/base` 指针 | 失同步横幅上的按钮 |
| `svn_migrate_eol` | 同上 | 行尾迁移横幅上的按钮 |

后两条尤其刺眼：`svnJournal.ts` 里我自己写着「重建会移动基线指针，而此刻正有
另一套流程依赖着那个指针」——**那个危险只在 UI 层挡住了**，后端无条件接受 invoke。

`svn_init_hybrid` 不在此列：它只在 `SvnUninitialized` 上跑，那种目录里还没有
`.git`，也就不可能有锁。

**B2：`every_write_method_checks_the_sync_lock` 抓不到它自己声称要抓的那件事。**

它的文档写着「新增一个写方法时忘记加守卫是无声的」，但 `WRITE_METHODS` 是一份
**硬编码清单**——新增一个方法根本不在清单里，测试照样绿。rubric #9（虚假信心）。
决策日志里那次变异（移除 `stage` 的守卫）只覆盖了「已在清单里的方法出现回归」
这一种，正好是它唯一能覆盖的那种。

改法：先枚举 `impl Vcs for SvnGitBackend` 里**实际存在**的方法，任何一个没归类到
写 / 读的直接失败并要求作者归类。另加两条：`the_guard_runs_before_the_delegation_not_after`
（守卫写在委托之后 = 活儿已经干完，守卫等于没有；原来只有 `stage` 有行为测试兜底，
其余 11 个没人管）、`write_commands_outside_the_vcs_trait_are_guarded_too`（守 B1）。

**B3：容器级 `#[serde(default)]` 让缺字段的 journal 静默读成默认值。**

自己写测试复现，确认成立：

```text
{"opid":"x","flow":"sync_out","startedAt":7,"stashOid":"abc"}  →  Ok(phase: Start)
{}                                                              →  Ok(opid:"", phase: Start)
```

`a_corrupt_journal_is_an_error_not_a_default` 只覆盖了**语法坏掉**的 JSON，
留下「语法合法但缺字段」这一整类没测。而后果更糟：少了 `phase` 读成
`Phase::Start`，界面对 `start` 说的是**「还没有动过工作目录里的任何东西——
可以直接当作没发生过」**。对一次实际停在 `svn-staged`（`.svn` 里挂着 add /
delete 登记）的流程说这句话，是整个界面能说出的最有害的一句。

改法：`default` 从容器挪到每个可选字段上。脊梁四件（`opid` / `flow` / `phase` /
`startedAt`）没有 `skip_serializing_if`，我们写出去的一定带着它们，读的时候缺
任何一个都是硬错误，走 `svn_journal_corrupt`。

### 非阻断里修了 7 条（各有具体失败场景）

| # | 发现 | 处理 |
|---|---|---|
| N1 | `acquire` 在写元数据的四个 `?` 早退路径上**泄漏锁文件**（`file` 掉了但文件留着，没有 guard 去删） | **已修**：先建 `SyncLock` 再写元数据。与 Task 1 修的 `Drop` bug 同一类后果——又一条永远为假的「上次没有正常收尾」 |
| N2 | `is_held` 对 32/33 之外的错误**一律当成「没人持有」**，于是权限错误（errno 5）会让守卫放行写操作 | **已修**：三态 `Held / Free / Unknown`。守卫 fail-closed（判不出当有人持有），接管 fail-safe（判不出就不抢）。新增只读属性造 errno 5 的测试 |
| N4 | 去抖那条源码断言只查子串 `is_watch_suspended`，**去掉 `!` 仍然绿**（功能恰好反过来） | **已修**：断言改成 `!is_watch_suspended` |
| N5 | `refreshSvnSyncState` 只在打开仓库时调一次 | **已修**：挂进 `repo-changed`。原来打开那一刻若读到 `locked: true`，让位规则会把其余横幅**压住整个标签页生命周期**，除非关掉重开 |
| N7 | 前端 `SyncPhase` 联合类型比后端 `Phase` 窄，认不出的 phase 让 `PHASE_DETAIL[...]` 变 `undefined` | **已修**：加兜底文案 + 一条 `phase: 'svn-staged'` 的 vitest |
| N10 | 守卫也会挡住**持锁者自己**，未记录 | **已修**：写进 `assert_not_syncing` 文档——2c 的 `sync_in` 要直接调 `engine` 层，不能用 `backend_for(..)` 干自己的活 |
| N11 | 接管路径的 TOCTOU 落败方抛的是裸 io 错误 | **已修**：`AlreadyExists` 映射回 `sync_in_progress` |
| N12 | E2E 场景 1 用 `Set-Content -Encoding utf8`，PowerShell 5.1 会写 **BOM**，serde_json 直接报错 | **已修**：改用 `[IO.File]::WriteAllText`，并写明为什么 |

### 没修的（各有理由）

| # | 发现 | 为什么不修 |
|---|---|---|
| N3 | watcher 挂起没有 RAII guard | **属 2c**。本期没有任何调用方（`suspend_watch` 的调用点在 S0 / S11），现在造一个 guard 是替一个还不存在的调用方设计。2c 接线时一并做，已记在此处备查 |
| N6 | 让位是「藏起信息」而不是只藏危险按钮 | 有道理，但改成「灰化保留」要动四条横幅的渲染与色调，超出本期范围；且本期 journal 不会自己产生，实际暴露面为零。记为 2c 的待办 |
| N8 | journal 缺 `svnDelta` / `svnStatusBefore` / `captureReturnStep`，且没有 schema `version` | 这三个字段的**语义**要等 `sync_out` 才定得下来（§8.2 / §8.5），现在照 spec 抄一遍字段名而不知道怎么填，是另一种形式的猜。已把注释里「等 Phase 3 再加会读不了」那句改成如实说明 |
| N9 | `lock_path` 硬编码 `<root>/.git`，不像两个兄弟锁那样用 `git_dir()` | 混合仓库的 `.git` **必定**是工作副本根下的真实目录（`init_hybrid` 在那里跑 `git init`，§6.1 的识别也据此），`.git` 是文件的形态（worktree / submodule）进不了混合仓库。与邻居 `svn_attributes::info_dir` 同一约定。热路径省一次子进程是真实收益 |

### 变异验证（本轮新增）

| 变异 | 结果 |
|---|---|
| 从分类表里拿掉 `checkout`（模拟新增方法未归类） | `every_trait_method_is_classified_as_read_or_write` 点名 `checkout` 变红 |
| 把 `checkout` 的守卫挪到委托之后 | 只有 `the_guard_runs_before_the_delegation_not_after` 变红；**`every_write_method_checks_the_sync_lock` 仍然绿**——正是 B2 指出的那个洞 |
| 移除 `revert` 的守卫 | `write_commands_outside_the_vcs_trait_are_guarded_too` 变红 |
| （B3 / N2 两条是先写测试跑出红再修的） | 见上 |

### 为什么这么改是安全的

- **B1 的三条守卫是纯前置判断**，没持锁时行为逐字不变；全量 595 passed 里既有的
  567 条一条没动。
- **B3 收紧了反序列化**，只会把「本来会被静默接受的坏输入」变成报错，不会让
  任何合法 journal 读不回来——`a_journal_round_trips` 与 `a_stash_oid_survives_…`
  仍绿。
- **N2 的三态在两个方向上都选了保守的那边**，且两边保守的方向相反（守卫关门、
  接管不动手），不存在「为了让一边安全而把另一边放开」。
- **`tsc` 在 N5 的接线上抓到了一处漏传**（`RepoTab.tsx` 的 refreshers 对象缺字段），
  说明那条类型约束是有效的，不是摆设。

---

## Task 8：修入第二轮独立 review（2026-09-25）

第二轮 review 报了 8 条（中 3、低 5），**逐条对照代码复核，全部成立**。其中第 1 条的
BOM 部分用 Windows PowerShell 5.1 实测过：`Set-Content -Encoding utf8` 写出的文件
开头是 `EF BB BF`。本轮修 4 条，其余 4 条记为 2c 待办。

### 修了的

| # | 发现 | 处理 |
|---|---|---|
| R1a | E2E 场景 3 仍用 `Set-Content -Encoding utf8` 写锁文件——N12 只修了场景 1。5.1 写出的 BOM 让 `holder()` 返回 `None`，横幅看不到 9999 | 改成 `[IO.File]::WriteAllText`，并在场景 3 重复那条警告 |
| R1b | 场景 3 第 4 步用 Ctrl+C「模拟持锁进程退出」。Ctrl+C 只打断 `Start-Sleep`，`$f` 仍留在会话里开着，**锁根本没释放**——按清单对照表会被误判成「存活判据退化」 | 删掉 `Start-Sleep`（锁就是 `$f` 本身，不需要撑着）；第 4 步改为 `$f.Close()` 或关窗口，并写明为什么不能用 Ctrl+C |
| R2 | Task 6「为什么安全」里「错误照常由通用错误提示呈现」**与代码不符**；`repoStore.ts` 的 JSDoc 有同一句。实际行为：坏 journal → catch 写 `null` → 不挂横幅、不报错 | **代码与文档一起修**，见下 |
| R3 | spec §5.1.2 与 §18 写非 Windows「降级为无法判定存活」，代码返回的是 `Free` | **改 spec，不改代码**，见下 |
| R4 | `stashing` 的文案写着 `**不要**`，横幅按纯文本渲染，用户会看到两对星号 | 去掉星号；新增一条 vitest 扫所有横幅分支的 title / detail / note，不许出现 `**` |

### 没修的（记为 2c 待办）

| # | 发现 | 为什么留到 2c |
|---|---|---|
| R5 | UI 打开锁文件（`probe` 的 `share_mode(0)`、`permissive_open` 的 `READ\|WRITE`）时都没给 `FILE_SHARE_DELETE`。持锁方 `Drop` 里的 `remove_file` 恰好撞上会失败，被 `let _ =` 吞掉，留下残锁，下次 `acquire` 打一条假的「上次没有正常收尾」。同理，接管路径的 `remove_file(&path)?` 撞上时会抛裸 io 错误 | 复核成立，概率低（要撞上微秒级窗口）。**本期没有任何代码调 `acquire`**，暴露面为零；2c 接入 `sync_in` 时一并处理（读方加 `FILE_SHARE_DELETE`，或 `Drop` 删除失败时短重试） |
| R6 | spec §13 要求显示持锁进程的 pid **与开始时间**。横幅只显示 pid；`busy_error` 把 epoch 秒原样打印 | 要定时间格式和时区（前端本地化 vs 后端格式化），属展示层设计，和 2c 的冲突 UI 一起做 |
| R7 | `svn_journal::write` 在 `create` / `write_all` / `sync_all` 失败时留下 `.tmp`；崩溃留下的 `.tmp` 也没有清理 | 本期没有任何代码调 `write`；与 `svn_attributes::write_attributes` 同一写法，2c 接线时两处一起收 |
| R8 | vitest 没有「`sync_out` + 共用 phase」的组合，前端文案不看 `flow` 这件事没有测试挡 | 共用录制阶段在 `sync_out` 下该说什么，要等 `sync_out` 的语义定下来（与 N8 同理）。2c/Phase 3 接 `sync_out` 时补 |

### 偏离计划

- **R2 没有走「通用错误提示」**，虽然那是 Task 6 文档原本声称的做法。原因：N5 之后
  `refreshSvnSyncState` 挂在 `repo-changed` 上，每次保存文件都重读一次；走顶部错误
  横幅的话，用户关掉它、下一次保存又弹回来。改为在同步横幅的位置挂一条**红条**
  「读不出上次同步的记录」，正文接后端报错原文，且**其余横幅同样让位**。
- 为此新增类型 `SyncStateUnreadable = { readError }`。store 字段从
  `SyncState | null` 放宽为 `SyncState | SyncStateUnreadable | null`，只有
  `BaselineBanner` 一个消费方，`describeUnfinishedSync` / `otherBannersAreSuppressed`
  的参数随之放宽。`svnEol` 仍是读失败写 `null`，这里**有意不同**：行尾读不到的
  后果是少一条提示，journal 读不到的后果是把「中间态」显示成「干净」。

### 计划的错误

- **spec §5.1.2 / §18：「非 Windows 明确降级为无法判定存活」**。按 Task 7 的三态模型，
  这对应 `Unknown`：守卫关门、接管不动手，于是非 Windows 上**任何一次崩溃留下的锁
  文件都会永久挡住所有写操作，且没有入口能解开**。代码实际是 `Free`（不做互斥），
  这才是对的取舍：没有互斥 = 与本期之前相同，永久卡死 = 新引入的退化。已按实现
  改写 spec 两处，并写明为什么不能选 `Unknown`。
- 同一段还写着「std 没有等价的跨进程劝告锁」。这话只在本 crate 的 MSRV（1.77.2）
  下成立；std 的 `File::try_lock` 自 1.89 起稳定，本机工具链是 1.95。spec 已改成
  如实表述。`svn_lock.rs` 第 84 行的同一句注释**本轮没改**（只动了 TS 与文档，不为
  一句注释重跑 Rust 全套）；若将来要在非 Windows 上做真互斥，是否提 MSRV 需要单独决定。

### 计划没写的坑

- **Ctrl+C 不等于进程退出。** 在交互式 PowerShell 里，Ctrl+C 只中断当前管道，会话
  变量里的 `FileStream` 仍然开着。手工造锁的步骤必须显式 `Close()` 或关掉会话。
- 后端 `svn_journal_corrupt` 的报错原文已经是完整句子（带路径和「请勿删除」），
  前端再套一层括号会出现「。）。」。红条正文改为原文直接接在一句前缀后面。

### 为什么这么改是安全的

- **R2 只把「不挂横幅」变成「挂红条」**，方向与守卫一致：拿不准时保守。红条上
  **没有任何按钮**，不会引导用户做出任何写操作；唯一的副作用是其余横幅让位，
  也就是在读不出记录时不给出「重建基线」这类会移动基线指针的出口。
- **读成功路径逐字不变**：`try` 分支没动，既有 12 条 vitest 全绿。
- **R3 只改 spec 不改代码**，行为零变化。
- **R4 只删了两对星号**，`stashing 阶段必须明确叫用户不要自己去 apply` 仍绿。

### 验证

| 命令 | 结果 |
|---|---|
| `pnpm vitest run tests/vitest/svnJournal.test.ts`（改代码前） | 新增 2 条**按预期失败**：`expected null not to be null`；`expected '…**不要**…' not to contain '**'` |
| `pnpm build`（含 `tsc`） | ✅ OK |
| `pnpm test` | ✅ **381 passed**（379 + 2） |
| cargo 全套 | 未跑：本轮没有改 `src-tauri/**` |
| `pnpm tauri dev` | ⬜ **仍未跑**。E2E 场景 1 新增第 5 步专验 R2 |

**已知覆盖盲区**：store 里「catch 写 `{ readError }` 而不是 `null`」这一行只有 `tsc`
和 E2E 第 5 步盖着。`null` 也是合法类型，把它改回 `null` 时 `tsc` 不会报错；
vitest 只覆盖纯逻辑层。本仓库目前没有 store 层的测试基建，这一行靠 E2E 兜。

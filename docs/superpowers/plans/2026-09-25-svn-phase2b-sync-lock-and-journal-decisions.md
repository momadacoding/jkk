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
| 6 | 「上次同步未完成」的检测与展示 | ⬜ 未开始 | | |
| 收尾 | E2E 清单 + 独立 review + spec 回写 | ⬜ 未开始 | | |

## 全量验证

| 命令 | 结果 |
|---|---|
| `cargo fmt --check` | 🟡 Task 5 时点：OK |
| `cargo clippy --all-targets` | 🟡 Task 5 时点：**0 告警** |
| `cargo test` | 🟡 Task 5 时点：**589 passed / 0 failed**（基线 567，新增 22） |
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

**一、`src/vcs/svn.rs` 是 CRLF，其余被改过的文件是 LF。** 用 `
` 模式批量替换
会一条都匹配不上（第一次尝试 12 条全部 `NOT FOUND`）。做法：读进来先
`
 → 
` 归一，改完再整体写回 CRLF，并在写回前断言「原文是纯 CRLF」——
混合行尾的文件按这个流程会被悄悄统一，那是不该由本期顺手做的改动。

改完复核 `CRLF=141 bare-LF=0`，没有混入 LF。

**二、两层测试各自只能抓一半，缺一不可。**

| 测试 | 抓得到 | 抓不到 |
|---|---|---|
| `svn_write_guard.rs`（扫源码） | 新增写方法时**忘了加** | 加了但没生效 |
| `staging_is_refused_…`（真后端） | 加了但没生效 | 除 `stage` 外其余 11 个方法 |

变异（只去掉 `stage` 的守卫）实测两边都红：扫描那条直接点名 `stage`，行为那条
在真实 `backend_for` 路径上报错。只留任意一条都会留下一个真实的漏洞形态。

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

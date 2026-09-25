# Phase 2b 同步基础设施 手工测试指引

**对应分支：`worktree-svn-phase2b-lock`。** 配套
`docs/superpowers/plans/2026-09-25-svn-phase2b-sync-lock-and-journal.md`
与同目录的 `-decisions.md`。

> 这份文档假设你**没看过这期的代码**，照着从上往下做就行。

## ⚠️ 这一期的验收**不是**「能更新」

`sync_in` 的编排（S0–S11）在 **2c**，本期只做它的地基：跨进程锁、写前 journal、
watcher 挂起，以及「上次同步未完成」的检测与展示。

所以下面三个场景都要**手工造现场**（手写 journal 文件、手工造一把锁），
因为还没有任何代码会自己产生它们。这是预期之内的，不是测试方法凑合。

## 状态

| 项 | 状态 |
|---|---|
| 自动化验证（`cargo test` / `pnpm test` / `pnpm build` / `fmt` / `clippy`） | ✅ 已跑：590 + 378 passed，fmt / clippy 零告警 |
| 下面 3 个真实窗口场景 | ⬜ **待跑** |

⚠️ **没跑的别勾。** 空着是如实，勾上是谎报。

- [ ] 场景 1：手写 journal → 出现「上次同步没有正常结束」横幅，且其余横幅让位；坏 journal → 「读不出上次同步的记录」红条
- [ ] 场景 2：删掉 journal → 横幅消失，其余横幅回来
- [ ] 场景 3：手工造一把活锁 → 写操作被拒，读操作照常

---

## 0. 准备一个混合仓库

任选一个已经初始化过的 SVN 混合仓库；没有的话按
`docs/superpowers/e2e/2026-09-22-svn-phase2a-eol-e2e.md` §1 造一个 fixture，
走完初始化向导即可。下文用 `<WC>` 指它的工作副本根目录。

```bash
pnpm tauri dev
```

---

## 场景 1：手写 journal → 出现横幅

这条验 `svn_sync_state` 这个**新命令的 invoke 路径真的通**
（`AGENTS.md` §7.3：`snake_case` ↔ `camelCase` 转换没有编译期检查，
写错只会在运行时静默失败，必须实跑）。

1. 先**关掉**应用。

2. 在 PowerShell 里造一份「上次停在 stashed」的记录：

```powershell
$j = '{"opid":"manual-test","flow":"sync_in","phase":"stashed","startedAt":1757000000,"origBranch":"work"}'
[IO.File]::WriteAllText('<WC>\.git\mygitclient-sync.json', $j)
```

> ⚠️ **别用 `Set-Content -Encoding utf8`。** Windows 自带的 PowerShell 5.1 会给
> 它写上 **BOM**（PowerShell 7 不会），而 serde_json 见到 BOM 直接报
> `expected value`——你会看到下面那张表「读不出上次同步的记录」那一行，而不是
> 本场景要验的横幅。
> `[IO.File]::WriteAllText` 两个版本都不写 BOM。

3. 重新 `pnpm tauri dev`，打开 `<WC>`。

**应该看到：**

- 顶部一条**红色**横幅，标题 **「上次同步没有正常结束」**；
- 说明里提到 **stash**，大意是「未提交的改动已经压进 stash，OID 已经记下来了，
  东西没丢，工作目录看起来是空的那是正常的」；
- 下面一段补充说明写着「继续」与「放弃」两个入口随更新流程一起到来；
- 横幅上**没有任何按钮**（本期有意为之，见下表）；
- **其余横幅一条都不在**——包括底部那条常驻的「体检基线」灰条。

**看到别的说明：**

| 现象 | 含义 |
|---|---|
| 没有横幅 | `svn_sync_state` 没被调用，或参数名转换写错了（这正是本场景要抓的） |
| 横幅在，但「体检基线」灰条也还在 | `otherBannersAreSuppressed` 的让位判据没接上 |
| 横幅上有按钮 | 不该有。本期只检测与展示，能收尾的「继续 / 放弃」在 2c |
| 红色横幅，标题「读不出上次同步的记录」 | JSON 写错了（比如带了 BOM）。这条横幅本身是**对的行为**——坏 journal 必须挂出来而不是当成没有（见第 5 步） |

4. 顺带验一下「正在进行中」那条分支与它不同。把 phase 改成 `rebase` 并**同时**
   造一把活锁（见场景 3 第 2 步），重开应用：标题应当变成
   **「这个工作副本正在进行一次同步」**，是**黄色**而不是红色，且说明里带着
   持锁进程的 pid。

5. 验「坏 journal 不被吞」。先按场景 3 第 4 步释放掉第 4 步造的锁，再把记录写成
   一个缺字段的 JSON，重开应用：

```powershell
[IO.File]::WriteAllText('<WC>\.git\mygitclient-sync.json', '{}')
```

   **应该看到**红色横幅，标题 **「读不出上次同步的记录」**，说明里带着后端的
   报错原文（含 `missing field`），且其余横幅同样全部让位。**没有横幅**就说明前端把
   读失败又吞成了「一切正常」。验完回到第 2 步把正常的记录写回去，或者直接进场景 2。

---

## 场景 2：删掉 journal → 横幅消失

```powershell
Remove-Item <WC>\.git\mygitclient-sync.json
```

重开应用打开 `<WC>`。

**应该看到：**

- 红色横幅**消失**；
- 原本该有的其余横幅 / 灰条**回来了**（至少底部的「体检基线」灰条应当在）。

这一步确认让位是**临时**的，不是把其余横幅永久关掉了。

---

## 场景 3：手工造一把活锁 → 写操作被拒，读操作照常

这条验 spec §5.1 的两半：**写拒绝、读不阻塞**。

1. 先关掉应用（避免它自己动那个仓库）。

2. 在一个**单独的** PowerShell 窗口里，开着不要关——它就是「另一个正在同步的
   进程」：

```powershell
$lock = '<WC>\.git\mygitclient-sync.lock'
[IO.File]::WriteAllText($lock, '{"pid":9999,"startedAt":1757000000,"opid":"manual"}')
$f = [System.IO.File]::Open($lock,'Open','ReadWrite','Read')
'锁已持有。这个窗口别关，也别跑别的命令，第 4 步再回来。'
```

> ⚠️ 同场景 1：**别用 `Set-Content -Encoding utf8`**，PowerShell 5.1 写出的 BOM
> 会让 `holder()` 解析失败，横幅只会说「还读不到持有它的进程信息」，看不到 9999。
>
> 锁就是 `$f` 这个句柄本身：只要这个 PowerShell 会话还活着、`$f` 没关，锁就一直
> 被持有，不需要 `Start-Sleep` 撑着。

3. 另起 `pnpm tauri dev`，打开 `<WC>`。

**应该看到：**

- 顶部一条**黄色**横幅，标题 **「这个工作副本正在进行一次同步」**，
  说明里带着 **9999** 这个 pid；
- **读操作照常**：提交图画得出来、文件列表看得见、点开文件能看到 diff；
- **写操作被拒**：勾选一个文件点「暂存」，应当弹出错误，大意是
  「这个工作副本上已经有一次同步正在进行」，并报出持有者 pid。
  切换分支、提交同理。

4. 回到第 2 步那个窗口释放锁（模拟持锁进程退出），二选一：

   ```powershell
   $f.Close()
   ```

   或者直接**关掉那个 PowerShell 窗口**。然后在应用里**重新打开**这个仓库。

   > ⚠️ **不要用 Ctrl+C 代替。** Ctrl+C 只中断正在跑的命令，`$f` 仍留在会话里
   > 开着，锁并没有释放——你会看到横幅还在，并被下表误判成「存活判据退化」。

**应该看到：**

- 黄色横幅消失；
- 暂存、切分支等写操作恢复正常。

5. 确认残留的锁文件**不会**把仓库卡死：

```powershell
Test-Path <WC>\.git\mygitclient-sync.lock
```

即使它返回 `True`（文件还在），上一步的写操作也必须是通的——
**「文件在」不等于「有人持有」**，这正是本期锁设计的核心
（spec §5.1.2）。验完可以手工删掉它。

**看到别的说明：**

| 现象 | 含义 |
|---|---|
| 写操作没被拒 | `assert_not_syncing` 没接上，或 `is_held` 判错 |
| **读**操作也被挡住 / 界面空白 | 守卫加到读方法上了。spec §5.1 明确要求读不阻塞——同步期间恰恰最需要看清状况 |
| 第 4 步之后写操作仍被拒 | 存活判据退化成了「文件存不存在」。那样一次崩溃就让仓库永远同步不了 |
| 第 5 步文件已经不在了 | 也可以接受（说明有人清理过），但那时第 4 步就没验到「残锁不卡死」这一条，请重做一遍 |

---

## 自动化测试覆盖了什么（不用手工重复）

| 场景 | 覆盖它的测试 |
|---|---|
| 崩溃后锁真的被操作系统释放 | `tests/svn_lock.rs::a_lock_whose_holder_was_killed_is_not_held`（真的 spawn 一个进程再 kill） |
| pid 复用不会把残锁判成活着 | `a_stale_lock_naming_a_live_unrelated_pid_is_still_takeable` |
| 正常释放会把锁文件删掉 | `a_normal_release_removes_the_lock_file_itself` |
| 12 个写方法都有守卫、读方法都没有 | `tests/svn_write_guard.rs` 两条 |
| 坏 journal 必须报错而不是当成没有 | `tests/svn_journal.rs::a_corrupt_journal_is_an_error_not_a_default` |
| journal 写入是原子的 | `overwriting_never_leaves_a_half_written_file` |
| 每个 phase 的线序名与 spec 逐字一致 | `every_phase_uses_the_exact_string_from_the_spec` |
| 字段名 `startedAt` 等转换正确 | `tests/svn_backend.rs::sync_state_serializes_to_camel_case` |
| watcher 挂起 / 恢复、路径归一化 | `tests/svn_watch_suspend.rs` 四条 |

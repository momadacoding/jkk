# Phase 2b 同步基础设施 手工测试指引

**对应分支：`worktree-svn-phase2b-lock`。** 配套
`docs/superpowers/plans/2026-09-25-svn-phase2b-sync-lock-and-journal.md`
与同目录的 `-decisions.md`。

> 这份文档假设你**没看过这期的代码**，照着从上往下做就行。
> 所有 PowerShell 代码块都能**原样复制粘贴**，不需要手工替换任何占位符。

## ⚠️ 这一期的验收**不是**「能更新」

`sync_in` 的编排（S0–S11）在 **2c**，本期只做它的地基：跨进程锁、写前 journal、
watcher 挂起，以及「上次同步未完成」的检测与展示。

所以下面三个场景都要**手工造现场**（手写 journal 文件、手工造一把锁），
因为还没有任何代码会自己产生它们。这是预期之内的，不是测试方法凑合。

## 状态

| 项 | 状态 |
|---|---|
| 自动化验证（`cargo test` / `pnpm test` / `pnpm build` / `fmt` / `clippy`） | ✅ 已跑：598 + 383 passed，fmt / clippy 零告警 |
| 下面 3 个真实窗口场景 | ⬜ **待跑** |

⚠️ **没跑的别勾。** 空着是如实，勾上是谎报。

- [ ] 场景 1：手写 journal → 打开时出现「上次同步没有正常结束」横幅，其余横幅让位；拿锁 / 放锁 / 写坏 journal 时横幅**自己**跟着变
- [ ] 场景 2：删掉 journal → 横幅自己消失，其余横幅回来
- [ ] 场景 3：另一个进程拿锁 → 黄条自己出现、写操作被拒、读操作照常；放锁 → 黄条自己消失、写操作恢复

---

## 0. 准备

整个过程要开**三个** PowerShell 窗口，下文按用途称呼：

| 窗口 | 用途 |
|---|---|
| **应用窗口** | 跑 `pnpm tauri dev`，别的不干 |
| **操作窗口** | 造 fixture、写 journal、删文件 |
| **持锁窗口** | 只在场景 1 第 4 步和场景 3 用，扮演「另一个正在同步的进程」 |

⚠️ 按 `AGENTS.md` 开篇的最高优先级约束，场景 3 要在仓库上点「暂存」，属于写操作，
**只能在临时 fixture 里做**。不要拿任何真实的 SVN 工作副本来跑这份清单。

### 0.1 起应用（应用窗口）

```powershell
cd C:\Users\liuhao\workspace\dev\Dev\mygitclient\.claude\worktrees\svn-phase2b-lock
pnpm tauri dev
```

> ⚠️ **必须在这个 worktree 里跑，不是主仓库。** 主仓库的 `master` 上没有这一期的代码。

### 0.2 造 fixture（操作窗口）

```powershell
cd C:\Users\liuhao\workspace\dev\Dev\mygitclient\.claude\worktrees\svn-phase2b-lock
pwsh -File scripts\e2e-svn-fixture.ps1
```

脚本会在 `$env:TEMP\phase1c_xxxxxxxx\` 下建 SVN 仓库和工作副本 `wc1`，最后打印
`wc1` 的路径。

### 0.3 初始化成混合仓库（应用里）

在应用里打开上一步打印的 `...\wc1`，走完初始化向导：**白名单填 `Code/`，扩展名
用默认**（与 2a 清单一致）。这样 `Code\A.cs` 才会进 git，场景 3 要拿它试「暂存」。

走完之后 `wc1` 下才会有 `.git`，下面的场景都要往 `.git` 里写文件。

### 0.4 设 `$WC`（操作窗口）

```powershell
$fix = Get-ChildItem $env:TEMP -Directory -Filter 'phase1c_*' |
    Sort-Object CreationTime -Descending | Select-Object -First 1
if (-not $fix) { throw '没找到 fixture，先做 0.2' }
$WC = Join-Path $fix.FullName 'wc1'
if (-not (Test-Path (Join-Path $WC '.git'))) { throw "$WC 下还没有 .git，先做 0.3" }
"WC = $WC"
```

它自动取 `$env:TEMP` 下**最新建的**那个 fixture，并确认初始化向导已经走完。
打印出来的路径应当就是你在应用里打开的那个 `wc1`。

> `$WC` 只在当前 PowerShell 会话里有效。**操作窗口别关**；持锁窗口里的代码块
> 自带同样的定位逻辑，不依赖这里。

### 0.5 横幅怎么「自己变」

除了场景 1 的第 1–3 步，下面**所有步骤都不要关掉或重开标签页**。另一个进程拿锁、
放锁时不会通知你，界面必须自己发现——这正是要验的：

| 变化 | 界面靠什么发现 | 应当在多久内反映出来 |
|---|---|---|
| journal 被写 / 被删、锁文件出现 | 文件监听（`repo-changed`，300ms 去抖） | 约 1 秒 |
| 锁被释放（关句柄、进程退出或崩溃） | 锁被持有期间每 2 秒轮询一次 | 最多约 2–3 秒 |

---

## 场景 1：手写 journal → 出现横幅

这条验 `svn_sync_state` 这个**新命令的 invoke 路径真的通**
（`AGENTS.md` §7.3：`snake_case` ↔ `camelCase` 转换没有编译期检查，
写错只会在运行时静默失败，必须实跑）。

1. 在应用里**关掉** `wc1` 这个标签页（应用本身不用退出）。

   只有这一处要关：它模拟的是真实场景「上次同步崩溃留下了记录，**下次打开仓库**
   时要发现它」，验的是打开那一刻的读取。

2. 在**操作窗口**造一份「上次停在 stashed」的记录：

```powershell
$j = '{"opid":"manual-test","flow":"sync_in","phase":"stashed","startedAt":1757000000,"origBranch":"work"}'
[IO.File]::WriteAllText((Join-Path $WC '.git\mygitclient-sync.json'), $j)
```

> ⚠️ **别用 `Set-Content -Encoding utf8`。** Windows 自带的 PowerShell 5.1 会给
> 它写上 **BOM**（PowerShell 7 不会），而 serde_json 见到 BOM 直接报
> `expected value`——你会看到下面那张表「读不出上次同步的记录」那一行，而不是
> 本场景要验的横幅。`[IO.File]::WriteAllText` 两个版本都不写 BOM。

3. 在应用里**重新打开** `wc1`。

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

4. 验「正在进行中」与「未完成」不同，而且**不重开标签页**。

   **操作窗口**——把 phase 改成 `rebase`：

```powershell
$j = '{"opid":"manual-test","flow":"sync_in","phase":"rebase","startedAt":1757000000,"origBranch":"work"}'
[IO.File]::WriteAllText((Join-Path $WC '.git\mygitclient-sync.json'), $j)
```

   约 1 秒内红条的说明**自己**变成「停在 git 自己的 rebase 冲突态上……」。

   **持锁窗口**——造一把活锁（与场景 3 第 2 步是同一段）：

```powershell
$fix = Get-ChildItem $env:TEMP -Directory -Filter 'phase1c_*' |
    Sort-Object CreationTime -Descending | Select-Object -First 1
$lock = Join-Path $fix.FullName 'wc1\.git\mygitclient-sync.lock'
$f = [System.IO.File]::Open($lock, 'Create', 'ReadWrite', 'Read')
$b = [Text.Encoding]::UTF8.GetBytes('{"pid":9999,"startedAt":1757000000,"opid":"manual"}')
$f.Write($b, 0, $b.Length); $f.Flush()
"锁已持有：$lock"
```

   约 1 秒内横幅**自己**变成 **「这个工作副本正在进行一次同步」**，是**黄色**而不是
   红色，且说明里带着 **9999**。

5. 验「放锁后自己回来」和「坏 journal 不被吞」，同样**不重开标签页**。

   **持锁窗口**——释放锁：

```powershell
$f.Close()
```

   2–3 秒内黄条**自己**变回红条（journal 还在，停在 rebase）。

   **操作窗口**——把记录写成一个缺字段的 JSON：

```powershell
[IO.File]::WriteAllText((Join-Path $WC '.git\mygitclient-sync.json'), '{}')
```

   约 1 秒内变成红色横幅，标题 **「读不出上次同步的记录」**，说明里带着后端的
   报错原文（含 `missing field`），且其余横幅同样全部让位。

| 现象 | 含义 |
|---|---|
| 第 4 步写完 journal 横幅不变，要重开才变 | journal 的写入没触发 `repo-changed`，或 `repo-changed` 没接 `refreshSvnSyncState` |
| 第 4 步造完锁黄条不出现，要重开才出现 | 锁文件仍被 `watch.rs` 的 `*.lock` 噪声过滤吞掉了 |
| 第 5 步放锁后黄条一直不消失 | 锁被持有期间的轮询没生效 |
| 第 5 步写成 `{}` 后横幅**消失** | 前端把读失败又吞成了「一切正常」 |

---

## 场景 2：删掉 journal → 横幅消失

**不重开标签页**，在**操作窗口**：

```powershell
Remove-Item (Join-Path $WC '.git\mygitclient-sync.json')
```

**应该看到**（约 1 秒内，自己发生）：

- 红色横幅**消失**；
- 原本该有的其余横幅 / 灰条**回来了**（至少底部的「体检基线」灰条应当在）。

这一步确认让位是**临时**的，不是把其余横幅永久关掉了。

---

## 场景 3：另一个进程拿锁 → 写操作被拒，读操作照常

这条验 spec §5.1 的两半：**写拒绝、读不阻塞**，以及锁的出现与消失界面都能
自己跟上。全程**不重开标签页**。

1. 在**操作窗口**先改一个文件，后面要拿它试「暂存」：

```powershell
Add-Content (Join-Path $WC 'Code\A.cs') "`n// phase2b e2e"
```

   文件列表里应当出现 `Code\A.cs` 的改动。**先别暂存。**

2. 在**持锁窗口**造一把活锁（场景 1 第 4 步里造过的话，先确认那把已经
   `$f.Close()` 过，再重新跑这一段）：

```powershell
$fix = Get-ChildItem $env:TEMP -Directory -Filter 'phase1c_*' |
    Sort-Object CreationTime -Descending | Select-Object -First 1
$lock = Join-Path $fix.FullName 'wc1\.git\mygitclient-sync.lock'
$f = [System.IO.File]::Open($lock, 'Create', 'ReadWrite', 'Read')
$b = [Text.Encoding]::UTF8.GetBytes('{"pid":9999,"startedAt":1757000000,"opid":"manual"}')
$f.Write($b, 0, $b.Length); $f.Flush()
"锁已持有：$lock"
```

   打印出来的路径应当就是你在应用里打开的那个 `wc1` 下面的。**这个窗口别关，
   也别跑别的命令，第 4 步再回来。**

> 为什么这么写：
>
> - **先拿住句柄、再写内容**，与应用里真实的拿锁顺序一致。反过来「先写文件再
>   打开」的话，中间有一瞬间文件在、却没人持有，界面恰好在那一瞬间刷新就会
>   判成「没锁」。
> - `[Text.Encoding]::UTF8.GetBytes` 不带 BOM。**别换成 `Set-Content -Encoding utf8`**：
>   PowerShell 5.1 写出的 BOM 会让持有者信息解析失败，横幅只会说「还读不到持有它
>   的进程信息」，看不到 9999。
> - 锁就是 `$f` 这个句柄本身：只要这个 PowerShell 会话还活着、`$f` 没关，锁就一直
>   被持有，不需要 `Start-Sleep` 撑着。

3. 回到应用，**不重开标签页**。

**应该看到：**

- 约 1 秒内，顶部**自己**出现一条**黄色**横幅，标题
  **「这个工作副本正在进行一次同步」**，说明里带着 **9999** 这个 pid；
- **读操作照常**：提交图画得出来、文件列表看得见 `Code\A.cs` 的改动、点开能看到 diff；
- **写操作被拒**：勾选 `Code\A.cs` 点「暂存」，应当弹出错误，大意是
  「这个工作副本上已经有一次同步正在进行」，并报出持有者 pid 9999。

4. 回到**持锁窗口**释放锁（模拟持锁进程退出），二选一：

```powershell
$f.Close()
```

   或者直接**关掉持锁窗口**。

   > ⚠️ **不要用 Ctrl+C 代替。** Ctrl+C 只中断正在跑的命令，`$f` 仍留在会话里
   > 开着，锁并没有释放。

**应该看到**（不重开标签页）：

- 2–3 秒内黄色横幅**自己**消失；
- 再点一次「暂存」，这次成功。

5. 在**操作窗口**确认残留的锁文件**不会**把仓库卡死：

```powershell
Test-Path (Join-Path $WC '.git\mygitclient-sync.lock')
```

应当返回 `True`：造锁那段代码从来没删过它，文件还在。而上一步黄条消失了、暂存也
通了——**「文件在」不等于「有人持有」**，这正是本期锁设计的核心（spec §5.1.2）。

**看到别的说明：**

| 现象 | 含义 |
|---|---|
| 第 3 步黄条不出现，重开标签页才出现 | 锁文件的出现没触发 `repo-changed`（`watch.rs` 的 `*.lock` 过滤没给同步锁开例外） |
| 写操作没被拒 | `assert_not_syncing` 没接上，或 `is_held` 判错 |
| **读**操作也被挡住 / 界面空白 | 守卫加到读方法上了。spec §5.1 明确要求读不阻塞——同步期间恰恰最需要看清状况 |
| 第 4 步之后黄条一直不消失，但点「暂存」能成功 | 锁被持有期间的轮询没生效（守卫是实时判断的，所以写操作不受影响） |
| 第 4 步之后写操作仍被拒 | 先确认释放锁用的是 `$f.Close()` 或关窗口而不是 Ctrl+C；确认过仍被拒，就是存活判据退化成了「文件存不存在」。那样一次崩溃就让仓库永远同步不了 |
| 第 5 步返回 `False` | 有人把文件删了，第 4 步就没验到「残锁不卡死」这一条，请从第 2 步重做 |

---

## 收尾

整个 fixture 是一次性的，测完在**操作窗口**删掉整个目录：

```powershell
Remove-Item -Recurse -Force $fix.FullName
```

删之前先在应用里关掉 `wc1` 标签页，并确认持锁窗口已经 `$f.Close()` 或关掉，
否则文件被占用删不干净。

---

## 自动化测试覆盖了什么（不用手工重复）

| 场景 | 覆盖它的测试 |
|---|---|
| 崩溃后锁真的被操作系统释放 | `tests/svn_lock.rs::a_lock_whose_holder_was_killed_is_not_held`（真的 spawn 一个进程再 kill） |
| pid 复用不会把残锁判成活着 | `a_stale_lock_naming_a_live_unrelated_pid_is_still_takeable` |
| 正常释放会把锁文件删掉 | `a_normal_release_removes_the_lock_file_itself` |
| 12 个写方法都有守卫、读方法都没有 | `tests/svn_write_guard.rs` 两条 |
| 坏 journal 必须报错而不是当成没有 | `tests/svn_journal.rs::a_corrupt_journal_is_an_error_not_a_default` |
| 坏 journal 在界面上挂红条而不是被吞 | `tests/vitest/svnJournal.test.ts`「读不出同步状态 ≠ 干净」（只覆盖纯逻辑层，store 那一行靠场景 1 第 5 步） |
| 锁文件的变动会触发刷新，git 自己的 `*.lock` 仍被过滤 | `commands/watch.rs` 单元测试 `the_sync_lock_file_is_relevant`、`git_own_lock_files_are_still_noise` |
| 界面探测锁本身不产生事件（不会形成刷新回路） | `commands/watch.rs::probing_the_sync_lock_does_not_trigger_a_refresh`（真 notify watcher，带正对照） |
| 只在锁被持有时轮询 | `tests/vitest/svnJournal.test.ts`「只在锁被持有时轮询」（定时器的起停接线只靠场景 1 第 5 步、场景 3 第 4 步） |
| journal 写入是原子的 | `overwriting_never_leaves_a_half_written_file` |
| 每个 phase 的线序名与 spec 逐字一致 | `every_phase_uses_the_exact_string_from_the_spec` |
| 字段名 `startedAt` 等转换正确 | `tests/svn_backend.rs::sync_state_serializes_to_camel_case` |
| watcher 挂起 / 恢复、路径归一化 | `tests/svn_watch_suspend.rs` 四条 |

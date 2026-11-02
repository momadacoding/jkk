# C1h「TortoiseSVN 钩子」手工测试指引

**对应分支：`worktree-svn-c1h-tsvn-hooks`。** 配套 `docs/superpowers/plans/2026-11-01-svn-c1h-tsvn-hooks.md`
与同目录的 `-decisions.md`（下称「日志」），设计见 `docs/superpowers/specs/2026-10-30-svn-mixed-revision-baseline-analysis.md` §6.13。

> 这份文档假设你**没看过这期的代码**，照着从上往下做就行。PowerShell 代码块都能原样复制粘贴。

## 这一期验的是什么

在 jkk 里一键把 TortoiseSVN 客户端钩子挂到自己的工作副本上。之后：

- 用 TortoiseSVN 提交最终版，跟进时「暂存的中间版本」按**真实发生的提交**判定，跟进报告写「判定依据：TortoiseSVN 钩子事件」；
- 不经过钩子的提交（命令行 `svn ci`、另一台机器）不再按作者猜着丢暂存，而是标「拿不准」；
- TortoiseSVN 更新带下来的同事改动与你的暂存做三方合并；
- 带着 Local-Only 补丁整文件提交时，TortoiseSVN 提交框里就被拦下，中文提示正常；
- jkk 换了位置或被删掉时，横幅提示钩子失效，可以重新注册或撤销。

这一期**也是 C1a E2E 场景 13 的落点**（用真的 TortoiseSVN 重走「暂存了中间版本」，日志 D2）：本清单场景 2 就是它。「没钩子退回 C1a」这条路由场景 7 第 5 步验，但那一步**没有暂存文件**，只验到判定依据退回「作者 + 失焦快照」，见下面状态表。

## 安全说明（先读）

- **注册会写你这台机器的 `HKCU\Software\TortoiseSVN\hooks`。** jkk 只追加 / 删除自己为 fixture 仓库挂的条目，你已有的钩子逐字节保留。最后的场景 8 撤销注册；中途放弃时也请在 jkk 里点「撤销注册」，或在 TortoiseSVN 设置 → Hook Scripts 里删掉命令行里带 `--tsvn-hook` 的条目。
- **钩子只挂在 fixture 的工作副本路径上**，你真实工程的提交不受影响。
- 按 `AGENTS.md` 开篇的最高优先级约束，下面所有 git / svn 写操作**只在 fixture 里做**。不要拿公司的 Unity 工程跑这份清单。

## 状态

| 项 | 状态 |
|---|---|
| 下面的真实窗口场景 | ✅ 2026-11-02 场景 1–8 全部通过（7c 未跑）。Claude 在 `pnpm tauri dev` 真实窗口里驱动：WebView2 远程调试端口点按钮、读界面，用系统调用切换前台窗口；TortoiseSVN 的提交 / 更新和 jkk 里写注册表的「注册」「重新注册」「撤销注册」由用户手点 |
| 环境 | TortoiseSVN 1.14.9、SlikSvn 1.14.2，系统 locale en-US（ACP 1252），用户 locale zh-SG（线程代码页 936）。工作副本路径带空格和中文 |
| 没验到 | ① 7c 盘符根目录。② 有暂存文件时走「作者 + 失焦快照」的 TortoiseSVN 提交（场景 7 第 5 步没有暂存）；同一份失焦快照在场景 2 走钩子判定时验过。③ 用户已有钩子时的保留行为：本机注册前没有钩子，由 `foreign_raw_value_round_trips_byte_for_byte_including_manual_precommit` 等自动化测试覆盖。④ Unity 大工程上的钩子耗时 |

- [x] 场景 1：在 jkk 里注册钩子；TortoiseSVN 设置里能看到（同时验 `tsvn_hooks_status` / `tsvn_hooks_register` 的参数名接通）
- [x] 场景 2（= C1a 场景 13）：暂存中间版本，TortoiseSVN 提交最终版 → 跟进后中间版本被丢弃，判定依据是钩子事件
- [x] 场景 3：同一路径用命令行 `svn ci` 提交（不经过钩子）→ 暂存保留、标「拿不准」
- [x] 场景 4：TortoiseSVN 对子目录 Update，带下同事改动 → 与暂存三方合并
- [x] 场景 5：Local-Only 补丁：「提交后恢复」删掉补丁再提交不拦；整文件提交被 Check-commit 拦下，中文正常，Abort 回到提交框
- [x] 场景 6：过期导致提交失败，更新之后再提交 → 跟进正常，孤儿 Pre 不误配对
- [x] 场景 7：jkk 换了位置 → 横幅提示失效；原位置的 exe 不存在时 TortoiseSVN 提交的真实表现；重新注册后恢复
- [ ] 场景 7c（可选）：盘符根目录，未跑，见场景 7 末尾
- [x] 场景 8：撤销注册 → 本机注册前没有钩子，撤销后 `hooks` 值不存在

---

## 0. 准备

两个 PowerShell 窗口加上应用本身：

| 窗口 | 用途 |
|---|---|
| **应用窗口** | 跑 `pnpm tauri dev`，别的不干 |
| **操作窗口** | 造 fixture、扮演同事、用命令行改文件、查结果 |

TortoiseSVN 的操作（提交、更新）**在资源管理器里右键做**，这正是这一期要验的。

### 0.1 起应用（应用窗口）

```powershell
$w = 'C:\Users\liuhao\workspace\dev\Dev\mygitclient\.claude\worktrees\svn-c1h-tsvn-hooks'
# dev 实例与安装版共用 session.json，启动就会改 activeTab：先备份，收尾时恢复
Copy-Item "$env:APPDATA\com.momadacoding.jkk\session.json" "$env:LOCALAPPDATA\jkk-session-backup.json" -ErrorAction SilentlyContinue
# WebView2 配置目录与安装版隔开
$env:WEBVIEW2_USER_DATA_FOLDER = "$env:TEMP\jkk-e2e-webview2"
Set-Location $w
pnpm tauri dev
```

**起来之后先核对跑的是不是这个 worktree（操作窗口）：**

```powershell
Get-Process app -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Path
```

路径必须在 `...\worktrees\svn-c1h-tsvn-hooks\src-tauri\target\` 下。tauri CLI 从 `.claude\worktrees` 启动时可能往上找到主仓库的 `src-tauri`，静默跑 master：那样后面每一步的结果都不算数，新命令还会报 `Command ... not found`。对不上就停下，不要往下做。

> ⚠️ 注册写进 TortoiseSVN 的命令行指向**这个 worktree 的 debug exe**，注册框会提示「这是开发构建」，这是预期的（日志 Review 1 第 5 条）。

### 0.2 造 fixture，路径带空格和中文（操作窗口）

工作副本路径带空格和中文，用来验注册的命令行与 TortoiseSVN 追加的 `CWD` 参数能不能原样拆回来（计划 Review Focus 第 1 条）。

> ⚠️ **fixture 脚本不能直接跑在中文路径下。** ACP 是 1252 的机器上，`svnadmin` / `svn` 把命令行里的中文绝对路径转成 `??`，报 E720123（日志 Task 7）。所以 SVN 仓库和 `wc2` 建在 ASCII 目录，建好之后只把 `wc1` 整体搬到中文路径。工作副本不含自身的绝对路径，搬走后照常可用。

```powershell
$w = 'C:\Users\liuhao\workspace\dev\Dev\mygitclient\.claude\worktrees\svn-c1h-tsvn-hooks'
$ascii = Join-Path ([IO.Path]::GetTempPath()) 'jkk-e2e-fixture'
$base = Join-Path ([IO.Path]::GetTempPath()) 'jkk e2e 钩子'
New-Item -ItemType Directory -Force $ascii, $base | Out-Null
$env:TEMP = $ascii   # 只对这个窗口生效：fixture 脚本建在 $env:TEMP 下
pwsh -NoProfile -File "$w\scripts\e2e-svn-fixture.ps1"
$fix = Get-ChildItem -LiteralPath $ascii -Directory -Filter 'phase1c_*' |
    Sort-Object CreationTime -Descending | Select-Object -First 1
if (-not $fix) { throw '没找到 fixture' }
Move-Item -LiteralPath (Join-Path $fix.FullName 'wc1') (Join-Path $base 'wc1')
```

### 0.3 辅助函数（操作窗口，一次性粘贴）

接着 0.2 的窗口做，要用到那里的 `$fix` 和 `$base`。`svn` 的路径参数一律用**工作目录 + 相对路径**：传 `wc1` 的绝对路径会撞上 0.2 那个问题。

```powershell
if (-not $fix) { throw '没找到 fixture，先做 0.2' }
if ($fix.CreationTime -lt (Get-Date).AddMinutes(-30)) { throw "最新的 fixture 不是刚才建的，先确认 0.2 成功了" }
$WC = Join-Path $base 'wc1'
$WC2 = Join-Path $fix.FullName 'wc2'
$INBOX = Join-Path $WC '.git\mygitclient\tsvn-inbox'
function Wc([string]$rel, [string]$content) {
    $p = Join-Path $WC $rel
    New-Item -ItemType Directory -Force (Split-Path $p) | Out-Null
    [IO.File]::WriteAllText($p, $content)
}
function SvnWc {
    Push-Location -LiteralPath $WC
    try { svn @args; if ($LASTEXITCODE -ne 0) { throw "svn $args 失败" } } finally { Pop-Location }
}
function Colleague([string]$rel, [string]$content) {
    Push-Location -LiteralPath $WC2
    try {
        svn update -q --non-interactive
        $isNew = -not (Test-Path -LiteralPath $rel)
        New-Item -ItemType Directory -Force (Split-Path $rel) | Out-Null
        [IO.File]::WriteAllText((Join-Path $WC2 $rel), $content)
        if ($isNew) { svn add -q --parents --non-interactive $rel }
        svn commit -q --non-interactive --no-auth-cache --username colleague -m "colleague $rel"
        if ($LASTEXITCODE -ne 0) { throw 'svn commit 失败' }
    } finally { Pop-Location }
}
function G { git -C $WC @args; if ($LASTEXITCODE -ne 0) { throw "git $args 失败" } }
# 收件箱里的事件（钩子写的）
function Inbox { Get-ChildItem -LiteralPath $INBOX -Filter *.json -ErrorAction SilentlyContinue | ForEach-Object { [IO.File]::ReadAllText($_.FullName) } }
# 只读：TortoiseSVN 钩子设置里带 --tsvn-hook 的条目数，和总条目数
function Hooks {
    $v = (Get-ItemProperty 'HKCU:\Software\TortoiseSVN' -ErrorAction SilentlyContinue).hooks
    if (-not $v) { 'hooks 值不存在'; return }
    $lines = $v -split "`n"
    "总条目 $([math]::Floor($lines.Count / 6))；jkk 条目 $(@($lines | Where-Object { $_ -match ' --tsvn-hook ' }).Count)"
}
"WC = $WC"; Hooks
```

最后一行先记下来：这是注册前你自己的钩子条目数（场景 8 要对回来）。

### 0.4 初始化成混合仓库（应用里）

在应用里打开 0.3 打印的 `WC` 路径，走完初始化向导：**白名单填 `Code/`，扩展名默认**。走完之后停在 `work` 分支上，顶部有一条灰色的「体检基线」横幅。

### 0.5 界面怎么发现变化

与 C1a 相同：**点回应用窗口的那一刻**检查 SVN；**点离应用窗口的那一刻**拍失焦快照。下面每个「点回应用」都是真实步骤。

---

## 场景 1：注册

1. **应用：** 灰色横幅上点 **TortoiseSVN 钩子…**。
2. 注册框应当：列出 4 条（`check_commit`、`pre_commit`、`post_commit`、`post_update`），工作副本路径就是 `WC`；「提交前拦截本地补丁」默认勾着；有一条黄色的「这是开发构建」。
3. 取消勾选「提交前拦截本地补丁」：列表少了 `check_commit`。**再勾回来**，点 **注册**。
4. **操作窗口：** `Hooks`

**应该看到：** jkk 条目 4 条，总条目 = 0.3 记下的数 + 4。打开 TortoiseSVN 设置 → Hook Scripts，4 条新条目的类型、路径显示正常，你原有的条目都还在。再点一次 **TortoiseSVN 钩子…**，状态是「已注册（提交前拦截本地补丁）」。

**看到别的说明：** 点注册后顶部红条报 `invalid args` / 缺参数：`blockLeaks` 参数名没接通（`AGENTS.md` §7.3），记下原文。

## 场景 2：暂存了中间版本，TortoiseSVN 提交最终版（= C1a 场景 13）

1. **操作窗口：** `Wc 'Code/A.cs' 'class A { int t1; }'`
2. **应用：** 暂存 `Code/A.cs`。
3. **操作窗口：** `Wc 'Code/A.cs' 'class A { int t2; }'`（点到操作窗口的那一刻，jkk 拍了快照）
4. 资源管理器打开 `WC`（`explorer $WC`），右键 → **SVN Commit...**，只勾 `Code\A.cs`，提交。完成后关掉进度框。
5. **操作窗口：** `Inbox`，应当有一条 `pre_commit` 和一条 `post_commit`，`paths` 都是 `["Code/A.cs"]`。
6. **应用：** 点回应用 → 横幅 → **跟进**。

**应该看到：** 跟进报告「丢弃的暂存：`Code/A.cs`」，最下面一行写「判定依据：TortoiseSVN 钩子事件」。`A.cs` 内容是 `t2`，「本地改动」为空。再跑一次 `Inbox`：空了（跟进完成后已处理的事件被清掉）。

**看到别的说明：** 判定依据是「作者 + 失焦快照」：跟进时没把钩子判成健康，记下注册框里显示的状态。A 进了「拿不准的暂存」：对一下第 5 步的事件时刻，以及你点窗口的顺序。

## 场景 3：不经过钩子的提交

1. **操作窗口：** `Wc 'Code/A.cs' 'class A { int t3; }'`
2. **应用：** 暂存 `Code/A.cs`。
3. **操作窗口：** `Wc 'Code/A.cs' 'class A { int t4; }'`，然后用**命令行**提交：`SvnWc commit -q --non-interactive -m cli Code/A.cs`
4. **应用：** 点回应用 → 跟进。

**应该看到：** 「拿不准的暂存：`Code/A.cs`」，暂存区里还是 `t3`。判定依据是钩子事件。（C1a 的推断此时会按「作者是我」丢掉暂存；钩子健康时不再这样猜，日志 Task 5。）

收尾：在应用里取消暂存 `Code/A.cs`。

## 场景 4：TortoiseSVN 更新带下同事改动

1. **操作窗口：** `Wc 'Code/A.cs' "1`n2`n3`n4`n5`n6`n7`n8`n"`，然后命令行提交：`SvnWc commit -q --non-interactive -m seed Code/A.cs`
2. **应用：** 点回应用 → 跟进（把这次提交收进基线）。
3. **操作窗口：** `Colleague 'Code/A.cs' "1`n2`n3`n4`n5`n6`n7`ncolleague`n"`；再 `Wc 'Code/A.cs' "mine`n2`n3`n4`n5`n6`n7`n8`n"`
4. **应用：** 暂存 `Code/A.cs`。
5. 资源管理器里在 `WC\Code` **这个子目录**上右键 → **SVN Update**。完成后关掉。
6. **操作窗口：** `Inbox`，应当有一条 `post_update`，`paths` 含 `Code/A.cs`。
7. **应用：** 点回应用 → 跟进。

**应该看到：** 「合并的暂存：`Code/A.cs`」；暂存区内容是 `mine` 开头、`colleague` 结尾。

## 场景 5：Local-Only 补丁的提交前拦截

1. **操作窗口：**
   ```powershell
   Wc 'Code/L.cs' ((1..20 | ForEach-Object { 'line {0:d2}' -f $_ }) -join "`r`n")
   SvnWc add -q Code/L.cs; SvnWc commit -q --non-interactive -m 'L seed' Code/L.cs
   ```
2. **应用：** 点回应用 → 跟进。
3. **操作窗口：** 把第 2 行改成补丁，做成 Local-Only commit：
   ```powershell
   $t = (Get-Content -LiteralPath (Join-Path $WC 'Code/L.cs')) -replace '^line 02$', 'line 02 DEBUG_PATCH'
   [IO.File]::WriteAllText((Join-Path $WC 'Code/L.cs'), ($t -join "`r`n"))
   G add Code/L.cs; G commit -q -m "调试补丁`n`nLocal-Only: true"
   ```
4. **操作窗口：** 再改第 18 行（功能改动，不 commit）：
   ```powershell
   $t = (Get-Content -LiteralPath (Join-Path $WC 'Code/L.cs')) -replace '^line 18$', 'line 18 FEATURE'
   [IO.File]::WriteAllText((Join-Path $WC 'Code/L.cs'), ($t -join "`r`n"))
   ```
5. 资源管理器 → `WC` 右键 → **SVN Commit...**，只勾 `Code\L.cs`，点 OK。

**应该看到（5a）：** 弹「Hook script failed」，内容是中文：`jkk：这些文件里有 Local-Only 补丁…`、`Code/L.cs：第 2 行起，来自「调试补丁」`，最后一行「取消勾选这些文件，或用「提交后恢复」删掉这些改动再提交。」点 **Abort**：回到提交框，消息还在。

6. 在提交框里右键 `Code/L.cs` → **Restore after commit**；用记事本打开 `WC\Code\L.cs`，把第 2 行改回 `line 02`（只删 ` DEBUG_PATCH`，**别留行尾空格**），保存；回提交框点 OK。

**应该看到（5b）：** 不拦，提交成功；提交完 `L.cs` 第 2 行恢复成 `line 02 DEBUG_PATCH`。

7. **应用：** 点回应用 → 跟进。报告里没有「泄漏告警」，Local-Only commit 还在分支上。

## 场景 6：过期导致提交失败

先造种子：`wc1` 里要有 `B.cs` 的旧版本，提交才会过期（这份清单原来少了这一步，`B.cs` 在 `wc1` 不存在时造不出过期）。

1. **操作窗口：** `Colleague 'Code/B.cs' 'class B { int seed; }'`；`SvnWc update -q Code/B.cs`
2. **应用：** 点回应用 → 跟进。
3. **操作窗口：** `Colleague 'Code/B.cs' 'class B { int c; }'`；`Wc 'Code/B.cs' 'class B { int mine; }'`
4. 资源管理器 → `WC` 右键 → **SVN Commit...**，只勾 `Code\B.cs`，点 OK。

**应该看到（6a）：** 提交失败「out of date」，接着弹「Command failed - Update needed」。点 **Cancel**。`Inbox` 里多了一条 `pre_commit`，**没有** `post_commit`（日志 H4）。

5. 右键 → **SVN Update**，B.cs 冲突时选「稍后解决」。
6. **操作窗口：** 先写回干净内容，再标记解决（`--accept working` 保留文件原样，不写回的话冲突标记会被提交上去）：
   ```powershell
   Wc 'Code/B.cs' 'class B { int mine; }'
   SvnWc resolve -q --accept working Code/B.cs
   ```
7. 再右键 → **SVN Commit...**，只勾 `Code\B.cs`，提交成功。
8. **应用：** 点回应用 → 跟进。

**应该看到（6b）：** 跟进正常完成，判定依据是钩子事件。`Inbox` 里只可能剩下第 4 步那条孤儿 `pre_commit`（它 24 小时后会被清掉），不影响判定。

## 场景 7：jkk 换了位置

1. **操作窗口：** 把当前的 debug exe 复制一份到别处，**另开一个 jkk**：
   ```powershell
   $moved = Join-Path $base 'jkk moved'
   New-Item -ItemType Directory -Force $moved | Out-Null
   Copy-Item 'C:\Users\liuhao\workspace\dev\Dev\mygitclient\.claude\worktrees\svn-c1h-tsvn-hooks\src-tauri\target\debug\app.exe' $moved
   Start-Process (Join-Path $moved 'app.exe')
   ```
   新开的 jkk 用的是同一个 dev server（`pnpm tauri dev` 要一直开着）。在它里面打开 `WC`。

**应该看到（7a）：** 新 jkk 顶部有黄色横幅「TortoiseSVN 钩子已失效」，原因写着「… 钩子指向 …\target\debug\app.exe，当前程序在 …\jkk moved\app.exe」，带「重新注册」「撤销注册」。横幅只列第一条原因，注册框里列全部。

2. 在新 jkk 里点 **重新注册** → 注册框主按钮是「重新注册」→ 点它。横幅消失；`Hooks` 仍是 4 条 jkk 条目（换成了新路径，没有多出 4 条）。
3. 关掉新 jkk，删掉 `jkk moved` 目录：`Remove-Item -Recurse -Force $moved`
4. **操作窗口：** `Wc 'Code/A.cs' 'class A { int t5; }'`，资源管理器 → **SVN Commit...** 只勾 `Code\A.cs`，提交。

**应该看到（7b，2026-11-02 实测）：** exe 不存在时，一次提交要过**两道**拦截：

| 顺序 | TortoiseSVN 的表现 | 操作 |
|---|---|---|
| 1 | Check-commit 弹「Hook script failed」，内容是「The system cannot find the file specified」，选项 **Abort** / **Proceed anyway** | 点 **Proceed anyway** |
| 2 | Pre-commit 让提交失败，进度框显示「The operation failed」和 **Retry without hooks** | 点 **Retry without hooks** |

之后提交成功，`Inbox` 里没有这次提交的事件。

5. **应用（原来那个 jkk）：** 第 4 步的提交之后点回应用 → **跟进**。钩子指向已删的路径，这次跟进的判定依据应当是「作者 + 失焦快照」，跟进完成后顶部出现「已失效」横幅（健康状态在打开仓库时和每次跟进之后读取，不是实时的）。点 **撤销注册** → 确认。`Hooks`：`hooks 值不存在`（或 jkk 条目 0）。

6. 再打开 **TortoiseSVN 钩子…** 重新注册一次，为场景 8 做准备。

**可选（7c，盘符根目录）：未跑。** 想验的是 `CWD` 是盘符根时 TortoiseSVN 追加参数的引号形状（日志 Review 1「工作副本在盘符根目录时的 `CWD`」）。**不能**按原来写的 `subst J: $WC` 再从 `J:\` 提交：TortoiseSVN 按路径前缀匹配钩子，`J:\` 不匹配注册的 `C:\...\wc1`，钩子大概率根本不触发（推断，未实测），`Inbox` 没事件什么也说明不了。要验就用 `J:\` 打开仓库，在那里注册，再提交与更新，做完撤销注册、`subst J: /d`。缺事件时判定落到「拿不准」，方向安全。

## 场景 8：撤销注册

1. **应用：** **TortoiseSVN 钩子…** → **撤销注册** → 确认。
2. **操作窗口：** `Hooks`

**应该看到：** jkk 条目 0，总条目 = 0.3 记下的数。0.3 时 `hooks 值不存在` 的话，现在也应当是 `hooks 值不存在`。TortoiseSVN 设置里只剩你自己的条目。

---

## 收尾

先在应用里关掉 `wc1` 标签页，确认场景 8 已撤销注册，退出应用，再删 fixture、恢复 `session.json`：

```powershell
Remove-Item -LiteralPath $base -Recurse -Force
Remove-Item -LiteralPath $ascii -Recurse -Force
$bak = "$env:LOCALAPPDATA\jkk-session-backup.json"
if (Test-Path $bak) { Move-Item -Force $bak "$env:APPDATA\com.momadacoding.jkk\session.json" }
```

## 自动化测试覆盖了什么（不用手工重复）

| 场景 | 覆盖它的测试 |
|---|---|
| 注册表值的解析与逐字节往返、别人条目保留、冲突、健康判定 | `src-tauri/tests/tsvn_hooks.rs` |
| 认不出的格式（CRLF、残条、错位）拒绝写入 | `value_that_does_not_round_trip_is_refused_not_rewritten` |
| 盘符根的 `--repo "D:"` 拆参与还原成绝对路径（不含 TortoiseSVN 追加 `CWD` 的引号形状，见 7c） | `hook_command_trims_trailing_separator_of_a_drive_root_repo`、`drive_root_repo_is_restored_to_an_absolute_path` |
| 真实 exe 按注册的命令行拆参、不落到 GUI（用的是临时子目录，不是真正的盘符根） | `real_exe_with_registered_command_line_writes_one_event` |
| 写注册表的 CAS：值被别人改过不写，写完被覆盖能发现 | `src-tauri/tests/tsvn_hooks.rs` 的 `cas_*` |
| 钩子参数按位置解析、仓库外路径丢弃、RESULTPATH 含被删文件、错误一律放行 | `src-tauri/tests/tsvn_hook_cli.rs` |
| stderr 按线程代码页编码（系统 ACP 不是 936 的机器上，中文不变 `?`） | `stderr_text_follows_the_thread_code_page_not_the_system_one` |
| Pre / Post 配对、孤儿 Pre 不配对、清理 | `src-tauri/tests/tsvn_inbox.rs` |
| Check-commit 按磁盘判定（提交后恢复、整文件、大小写、fail-open） | `src-tauri/tests/svn_leak.rs` 的 disk_* 与 `tsvn_hook_cli.rs` 的 check_commit_* |
| 跟进按事件判定的每一行（含失效退回、清理收件箱） | `src-tauri/tests/svn_follow_hooks.rs` |
| 注册框、失效横幅、判定依据文案 | `tests/vitest/TsvnHooksDialog.test.tsx`、`BaselineBanner.test.tsx`、`tsvnHooks.test.ts`、`svnFollow.test.ts` |

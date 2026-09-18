# 基线改从服务器录 · 手工测试指引

**对应分支：`feat/svn-baseline-from-server`（PR #44，base 是 `feat/svn-phase1c`）**

## 状态

| | |
|---|---|
| 自动化测试 | Rust 524 passed / 0 failed；前端 336 passed |
| 真实窗口 E2E | ⬜ **全部未执行**，本文件就是那份清单 |

> 本文件里的每一条命令都在真实 fixture 上跑过，输出是实测值不是猜的。
> 场景 10 最初写错过一版（会给出**假的通过**），已按实测改正——见那条里的方框。

---

## 0. 这份清单在测什么

### 一句话

Phase 1c 里，只要你手上有本地修改，**「重建基线」「初始化向导」「体检基线」三个入口
全都点不动**。这个分支把它们改成不再要求你先清干净，办法是**换掉基线内容的来源**：

```text
旧：把磁盘录成基线 → 所以必须先保证「磁盘 == 服务器」→ 要求工作区干净
新：直接照服务器录 → 脏文件去取服务器那一版，干净文件才用磁盘
```

所以整份清单反复在问两件事：

1. **脏工作副本上能不能做成**（旧版会被拒）
2. **做的过程碰没碰你的磁盘**（碰了就等于 `svn revert`，是最严重的缺陷）

### ⚠️ 先确认你测的是对的那个构建

这是最容易浪费一小时的地方。在**终端 A**：

```powershell
cd C:\Users\liuhao\workspace\dev\Dev\mygitclient\.claude\worktrees\svn-baseline-b
git rev-parse --abbrev-ref HEAD
```

必须打出 **`feat/svn-baseline-from-server`**。打出别的就说明你在 1c 的 worktree 里，
下面每条都会「失败」，但那不是 bug。

### 为什么这些必须手工测

自动化测试跑的是后端函数。它证明不了：横幅长什么样、按钮点下去有没有反应、
确认框里的话说得对不对、错误信息在界面上是不是人话。这些只有真实窗口能看见。

### 这份清单不会让你敲任何破坏性命令

没有 `Remove-Item .git`、没有 `git reset --hard`、没有 `svn revert -R`（只有场景 10
用一次，且明确圈定在 fixture 里）。需要「回到初始状态」时一律**重造 fixture**——
它只要几秒钟，而且零风险。

---

## 1. 准备

### 1.1 三个终端

| | 干什么 | 之后 |
|---|---|---|
| **终端 A** | 跑 `pnpm tauri dev` 把应用起起来 | 起完别动 |
| **终端 B** | 在 `wc1` 里敲命令、看结果 | 一直用 |
| **终端 C** | 扮演「同事」，在 `wc2` 里提交 | 几个场景用 |

### 1.2 确认 svn 命令行可用

**终端 B**：

```powershell
svn --version --quiet
```

看到版本号（例如 `1.14.2`）就行。报「无法识别 svn」说明只装了 TortoiseSVN 的图形
界面——重跑安装包选 Modify，把 **command line client tools** 勾上，或者装
[SlikSVN](https://sliksvn.com/download/)。装完**重开终端**。

### 1.3 起应用

**终端 A**：

```powershell
cd C:\Users\liuhao\workspace\dev\Dev\mygitclient\.claude\worktrees\svn-baseline-b
pnpm install
pnpm tauri dev
```

首次编译 Rust 要几分钟。窗口弹出来就算好了。

### 1.4 造 fixture（后面会重复做好几次）

**终端 B**，同一个 worktree 目录：

```powershell
pwsh -File scripts/e2e-svn-fixture.ps1
```

跑完打出两行路径。存成变量（换成你自己的随机后缀）：

```powershell
$fx = "C:\Users\liuhao\AppData\Local\Temp\phase1c_8e0c428e"
cd $fx\wc1
```

后文写 `$fx` 就是指这一段（不含 `\wc1`）。**终端 B 之后一直待在 `$fx\wc1`。**

fixture 里的文件：

| 路径 | 为什么在那儿 |
|---|---|
| `Code\A.cs`、`Code\Deep\D.cs` | 白名单内的代码，会进 git |
| `Code\Art.psd` | 白名单内但扩展名不符——**不该**进 git，也不该进任何差异清单 |
| `Art\boss.psd` | 白名单外 |

`$fx\wc2` 是「同事的副本」，在那里提交 = 外部变更。

### 1.5 每次「重造 fixture」的完整动作

后面好几处会说「重造 fixture」，指的就是这三步：

```powershell
cd ~
Remove-Item -Recurse -Force $fx          # 删的是 %TEMP% 下那个随机目录
cd C:\Users\liuhao\workspace\dev\Dev\mygitclient\.claude\worktrees\svn-baseline-b
pwsh -File scripts/e2e-svn-fixture.ps1   # 重新打出一个新路径，更新 $fx
```

> 删之前**先在应用里关掉这个仓库**（或直接关应用），否则文件被占用删不掉。

---

## 2. 测试场景

每条的格式是「**做什么** → **应该看到** → **看到别的说明什么**」。
不符就把场景号 + 实际现象记下来，不用自己排查。

场景 2–8 是**一条连续的线**，按顺序做，中间不要重造 fixture。

---

### ⬜ 场景 1：干净工作副本上初始化（回归，不能省）

> 测的是「旧路径没被弄坏」。新功能再好，把原来能用的弄坏了也不行。

**做什么**

1. 造一个全新的 fixture（§1.4）。
2. 应用里 `Open Repo` → 选 `$fx\wc1`。
3. 自动弹出向导，标题 **「把这个 SVN 工作副本交给 git 管」**。
4. 第一栏「**交给 git 管的目录（一行一条）**」清空，填一行 `Code/`。
5. 第二栏「**收哪些扩展名**」填 `.cs`。
6. 点 **「检查工作副本」** → **「下一步」** → **「建库」** → **「关闭」**。

**应该看到**

- 「检查工作副本」后显示**检查通过**，`记账 revision：r1`；
- 「建库」后显示**建好了**、`首个基线：r1(...)`、`当前分支已切到 work`；
- 一条 **「基线已通过体检（r1）」**；
- 图里出现一行 SVN 提交，短 hash 的位置显示 **`r1`**；
- 顶部没有警告横幅，但有一条灰色细条 + **「体检基线」** 按钮。

终端 B 核对：

```powershell
git status --short          # 应当**没有任何输出**
```

**看到别的说明什么**

| 现象 | 说明 |
|---|---|
| 向导任何一步报错 | 抄下错误原文 |
| `git status` 有输出 | 干净工作副本上初始化完不该有任何改动 |

---

### ⬜ 场景 2：**带着本地修改**初始化 ← 核心新行为之一

> 旧版在这里会被 `[svn_preflight_dirty]` 直接拒绝。

**做什么**

1. **重造 fixture**（§1.5）——要回到「还没交给 git 管」的状态。
2. 终端 B，造出三种本地修改：

   ```powershell
   cd $fx\wc1
   Set-Content Code\A.cs -Value 'MY LOCAL WORK' -NoNewline
   Set-Content Code\New.cs -Value 'brand new file' -NoNewline
   svn delete Code\Deep\D.cs
   svn status
   ```

   `svn status` 的实测输出（顺序就是这样）：

   ```text
   M       Code\A.cs
   D       Code\Deep\D.cs
   ?       Code\New.cs
   ```

3. 应用里 `Open Repo` → 选 `$fx\wc1`，按场景 1 同样的填法走完向导。

**应该看到**

- **向导正常走完，不再被拒绝**；
- `记账 revision：r1`、`首个基线：r1(...)`、**「基线已通过体检（r1）」**；
- 关闭向导后，顶部**只有**那条灰色细条（「revision 对得上不代表基线记的内容是对的……」
  + 「体检基线」按钮），**没有**任何黄色或红色横幅。

**看到别的说明什么**

| 现象 | 说明 |
|---|---|
| 报 `svn_preflight_dirty` | 你测的是 1c 的构建（回 §0 确认分支） |
| 报「工作副本是 mixed-revision，请先跑一次完整 svn update」 | `Code\New.cs` 那个未提交的新文件把 P2 带歪了——这正是本期修掉的一条 |
| 体检报「基线与服务器不符」 | **严重**：新判据把你自己的本地修改误判成了假基线 |
| **黄色横幅「服务器上有代码文件不在本地」，列着 `Code/Deep/D.cs`** | **假警报**，2026-09-18 已修。那个文件是你自己 `svn delete` 掉的，而且它**就在基线里**（录制时从 BASE 取回来了）；这条提示指控的本来是 `--set-depth=exclude` 那种你不知情的漏收，方向正好相反。又看到就说明修复没生效 |

---

### ⬜ 场景 3：初始化完，`git status` 恰好是你原有的那三处修改

> 接着场景 2，不要重开。验的是「基线树是服务器内容、磁盘是你的内容，两者故意
> 不同，而 git 把这个差异如实显示出来」。

**做什么**

终端 B：

```powershell
git status --short
Get-Content Code\A.cs           # 必须还是 MY LOCAL WORK
Test-Path Code\Deep\D.cs        # 必须是 False（你删掉的，不许被复活）
Test-Path Code\New.cs           # 必须是 True
```

**应该看到**

`git status --short` 的实测输出**恰好**这三行：

```text
 M Code/A.cs
 D Code/Deep/D.cs
?? Code/New.cs
```

内容检查：`MY LOCAL WORK` / `False` / `True`。

> 从这里开始，`svn status` 会多出一行 `?       .git`——那是应用刚建的 git 仓库，
> SVN 不认识它。**属正常**，后面所有 `svn status` 的输出里都会有它。

**看到别的说明什么**

| 现象 | 说明 |
|---|---|
| `git status` 是空的 | 基线把你的修改一起录进去了——**这正是会让 Phase 3 静默丢改动的形态** |
| `Code\A.cs` 变回了 `class A {}` | **最严重**：录制过程写了磁盘，等于 `svn revert`。立刻停下并报告 |
| `Code\Deep\D.cs` 又出现了 | 同上 |
| 多出别的行（比如 `Code/Art.psd`） | 白名单扩展名过滤失效 |

---

### ⬜ 场景 4：基线里存的是**服务器**那一版

> 场景 3 证明「磁盘没被动」，这条证明「基线录对了」。两条缺一不可。

**做什么**

终端 B：

```powershell
"磁盘   : $(Get-Content Code\A.cs -Raw)"
"基线里 : $(git cat-file blob 'refs/heads/svn/base:Code/A.cs')"
git ls-tree -r --name-only refs/heads/svn/base
```

**应该看到**

```text
磁盘   : MY LOCAL WORK
基线里 : class A {}          ← 服务器 r1 的原文
```

`ls-tree` **恰好**是：

```text
Code/A.cs
Code/Deep/D.cs
```

**看到别的说明什么**

| 现象 | 说明 |
|---|---|
| 基线里是 `MY LOCAL WORK` | 录的还是磁盘内容，核心改动没生效 |
| 树里有 `Code/New.cs` | 服务器 r1 上根本没有它，不该进基线 |
| 树里**没有** `Code/Deep/D.cs` | 你本地删了它，但服务器上还有——基线记的是服务器，必须有 |
| 树里有 `Code/Art.psd` | 扩展名不在白名单里 |

---

### ⬜ 场景 5：同事提交 → 失同步 → **带着本地修改**重建，且磁盘分毫未动

> 这就是你最初卡住的那个场景，也是整个分支最关键的一条。

**做什么**

1. **终端 C** 扮演同事。**让他新增一个文件**，不要去改你正在改的那些：

   ```powershell
   cd $fx\wc2
   svn update
   Set-Content Code\Colleague.cs -Value 'class Colleague {}' -NoNewline
   svn add Code\Colleague.cs
   svn commit -m "colleague adds a file"
   ```

   > **为什么必须是「新增」**：你在场景 2 里改了 `Code\A.cs`、删了
   > `Code\Deep\D.cs`。同事若去动这两个文件，下一步的 `svn update` 会产生
   > 冲突（实测会得到 `Tree conflicts: 1`），那是另一条路径，会把这条场景搅浑。

2. **终端 B** 拉下来：

   ```powershell
   cd $fx\wc1
   svn update --accept postpone
   ```

   实测输出应当是干干净净的一条 `A Code\Colleague.cs` + `Updated to revision 2.`，
   **没有 conflict**。

3. 应用里刷新或重新打开仓库。**先别点按钮**，在终端 B 抓一份快照：

   ```powershell
   "=== BEFORE ==="
   svn status
   git status --short
   "hash A.cs = $((Get-FileHash Code\A.cs).Hash)"
   ```

4. 点 **「重建基线」**。确认框里应当提到
   **「你未提交的 SVN 改动不影响重建」**。点确认。

5. 再抓一份：

   ```powershell
   "=== AFTER ==="
   svn status
   git status --short
   "hash A.cs = $((Get-FileHash Code\A.cs).Hash)"
   ```

**应该看到**

第 3 步之前，顶部有红色横幅，标题 **「基线落后于工作副本」**，正文含
`工作副本已经前进到 r2，而基线还记着 r1`，上面有 **「重建基线」** 和
**「体检基线」** 两个按钮。

重建之后：横幅消失，基线走到 r2。

**BEFORE 与 AFTER 三组输出逐字相同。** 实测值：

```text
svn status:              git status --short:
?       .git              M Code/A.cs
M       Code\A.cs         D Code/Deep/D.cs
D       Code\Deep\D.cs   ?? Code/Colleague.cs
?       Code\New.cs      ?? Code/New.cs
```

> `?? Code/Colleague.cs` 出现在 git 侧是**正常**的：同事的新文件已经通过
> `svn update` 到了磁盘上，而你的 `work` 分支还没跟进新基线。
> 「重建不自动推进工作分支」是设计里写明的代价。

再确认基线录对了：

```powershell
"基线里 : $(git cat-file blob 'refs/heads/svn/base:Code/A.cs')"   # class A {}
git ls-tree -r --name-only refs/heads/svn/base
```

树里应当有 `Code/A.cs`、`Code/Colleague.cs`、`Code/Deep/D.cs`，**没有** `Code/New.cs`。

**看到别的说明什么**

| 现象 | 说明 |
|---|---|
| 确认框里还写着「白名单内必须干净」 | 前端文案没跟上 |
| 点确认后报 `svn_preflight_dirty` | 你测的是 1c 的构建 |
| **哈希变了 / `svn status` 少了几行** | **最严重**：重建写了磁盘。立刻停下，把 BEFORE / AFTER 两段原样贴出来 |
| 报「重建过程中工作副本被外部改动了」 | R3c 触发了。你若确实在这期间动过 `wc1`，重点一次即可；没动过就记下来 |
| 第 2 步出现 `Tree conflicts` | 同事动的不是新增文件，回第 1 步 |

---

### ⬜ 场景 6：`~`（obstructed）仍然被拦，且**不自动修复**

> 这是唯一还会阻断的脏状态。必须拦，而且拦住之后不许替用户动手。

**做什么**

终端 B：

```powershell
Remove-Item Code\A.cs
New-Item -ItemType Directory Code\A.cs | Out-Null
svn status                      # 应当出现一行带 ~ 的
```

应用里点 **「重建基线」**（横幅没了的话，先让终端 C 再提交一个新增文件制造失同步）。

**应该看到**

- 报错，错误码 **`svn_preflight_obstructed`**；
- 错误信息里**指名 `Code/A.cs`**；
- 引导提到「本应用不会替你动这些路径」，并说明
  **除了 `~`，其余本地修改都不再阻断**。

磁盘没被动：

```powershell
Test-Path Code\A.cs -PathType Container      # 必须仍然是 True
```

**收拾现场**（进入下一场景前）：

```powershell
Remove-Item -Recurse -Force Code\A.cs
svn revert Code\A.cs
```

**看到别的说明什么**

| 现象 | 说明 |
|---|---|
| 重建居然成功了 | `~` 的拦截没生效——那种位置上取不到可信的服务器内容 |
| 那个目录被应用删掉了 | **严重**：本应用不做破坏性修复 |
| 错误信息只说「工作副本不干净」 | 文案没更新，会把用户引回一个已经不存在的死胡同 |

---

### ⬜ 场景 7：未提交的新文件**不再**被误报成 mixed-revision

> 实现途中发现的一条连锁问题：放松「必须干净」之后，`svn add` 未提交的节点
> （`revision="-1"`）会让 P2 以为工作副本是多 revision 的。

**做什么**

终端 B：

```powershell
Set-Content Code\Fresh.cs -Value 'class Fresh {}' -NoNewline
svn add Code\Fresh.cs
svn status                      # 应当有一行 A  Code\Fresh.cs
```

应用里点 **「重建基线」**（没有入口就先让终端 C 新增一个文件并提交，制造失同步）。

**应该看到**

- 重建**成功**；
- 基线树里**没有** `Code/Fresh.cs`：

  ```powershell
  git ls-tree -r --name-only refs/heads/svn/base | Select-String Fresh
  # 应当**没有输出**
  ```

**看到别的说明什么**

| 现象 | 说明 |
|---|---|
| 报「工作副本里有多个 revision，先跑一次完整的 svn update」 | 本期修的就是这条。而且那句引导做了也没用——真实原因是有个没提交的新文件 |
| 基线树里有 `Code/Fresh.cs` | 服务器上不存在的文件进了基线 |

---

### ⬜ 场景 8：真正的 mixed-revision **仍然**被拦

> 和场景 7 成对。只做场景 7 的话，一个「把 P2 整个删掉」的实现也能通过。

**做什么**

终端 B，把一个子目录退回旧 revision：

```powershell
svn update -r 1 Code\Deep
svn status -v Code | Select-Object -First 8     # 应当看到两个不同的 revision 号
```

应用里点 **「重建基线」**。

**应该看到**

- 被拒绝，错误码 **`svn_preflight_mixed_revision`**；
- 错误信息里**列出具体是哪几个 revision**；
- 引导说的是「先跑一次完整的 svn update」。

**收拾现场**：

```powershell
svn update
```

**看到别的说明什么**

| 现象 | 说明 |
|---|---|
| 重建成功了 | P2 被放松了。它不该放松——取服务器内容用的是 `-r BASE`，而「BASE 就是那个 N」完全依赖工作副本停在单一 revision |
| 错误信息里没有 revision 号 | 用户不知道该更新什么 |

---

### ⬜ 场景 9：脏工作副本上点「体检基线」，能给出**明确结论**

> 1c 里这颗按钮在工作副本脏时永远回答「没法下结论」——和重建那条是同一个死胡同，
> 只是换了颗按钮。

**做什么**

1. 确保工作副本是脏的（接着前面的场景就已经是了，否则）：

   ```powershell
   Set-Content Code\A.cs -Value 'WIP WIP WIP' -NoNewline
   svn status
   ```

2. 应用里点 **「体检基线」**（灰色细条上，或失同步横幅上）。

**应该看到**

- **「基线已通过体检（rN）」**之类的明确结论；
- **不是**「没法下结论 / 未经体检」。

**看到别的说明什么**

| 现象 | 说明 |
|---|---|
| 报「白名单内的磁盘内容与基线树不一致……所以这里不下结论」 | 1c 的旧行为，没改到 |
| 报「基线与服务器不符」并列出你改过的那些文件 | **误报**：本地修改被当成了基线撒谎 |

---

### ⬜ 场景 10：体检抓得住**真的**假基线 ← 必须和场景 9 成对做

> 只做场景 9 的话，一个「永远说通过」的实现也能骗过你。

> ⚠️ **这一条必须在干净的工作副本上做，而且同事要改的是一个干净文件。**
>
> 判据只能对**没被你改过**的文件下结论——你改过的那些，磁盘本来就和服务器不同，
> 体检合法地把它们排除在外（这是设计边界，不是缺陷，见 spec §6.8.7）。
>
> 这份文档最初写错过：让同事去改 `Code\Deep\D.cs`，而那个文件在场景 2 里被你
> 本地删掉了。实测结果是体检报 **通过** —— 一个**假的通过**，照着做会报出一个
> 不存在的 bug。

**做什么**

1. **终端 B**，先把工作副本清干净：

   ```powershell
   cd $fx\wc1
   svn revert -R .
   Remove-Item Code\New.cs, Code\Fresh.cs -ErrorAction SilentlyContinue
   svn status                 # 应当只剩一行 ?  .git
   ```

   > `svn revert -R .` 的作用域被 `cd` 限死在 fixture 的 `wc1` 里。
   > 敲之前确认 `pwd` 真的是 `$fx\wc1`。

2. **终端 C**，让同事改一个**干净**文件：

   ```powershell
   cd $fx\wc2
   svn update
   Set-Content Code\A.cs -Value 'class A { int fromColleague; }' -NoNewline
   svn commit -m "colleague edits A.cs"
   ```

3. **终端 B**：**不要跑 `svn update`**。手工把基线的 revision 号改成服务器那个新
   revision（模拟「基线记错了」或「恢复流程没走完」）：

   ```powershell
   cd $fx\wc1
   $srv  = (svn info --show-item revision (svn info --show-item repos-root-url)).Trim()
   "服务器在 r$srv，而我停在 r$((svn info --show-item revision).Trim())"
   $tree = git log -1 --format=%T refs/heads/svn/base
   $uuid = (svn info --show-item repos-uuid).Trim()
   $url  = (svn info --show-item repos-root-url).Trim()
   $old  = git rev-parse refs/heads/svn/base
   $msg  = "svn: r$srv`n`ngit-svn-id: $url@$srv $uuid"
   $new  = $msg | git commit-tree $tree -p $old
   git update-ref refs/heads/svn/base $new $old
   git log -1 --format=%B refs/heads/svn/base      # 确认 trailer 写的是 @$srv
   ```

   > 这几条只动 fixture 里的 git 分支指针，不碰任何真实仓库。

4. 应用里**重新打开仓库**，点 **「体检基线」**。

**应该看到**

- 报 **基线与服务器不符**，并**指名 `Code/A.cs`**。

实测的后端结论是 `Lying { files: ["Code/A.cs"] }`，界面上应当是对应的红色结论 + 文件名。

**看到别的说明什么**

| 现象 | 说明 |
|---|---|
| 报「通过」 | 先回头确认第 1 步真的清干净了、第 2 步同事改的确实是干净文件。两条都对还报通过，才是**判据没有牙齿**——而那正是它存在的全部意义：`svn status` 在这种形态下什么都看不见 |
| 报「没法下结论」 | 判据被前置门槛挡掉了 |

---

### ⬜ 场景 11：行尾一致性（`core.autocrlf`）

> 实现途中查出的一个真 bug：干净文件走 `git add`（按 autocrlf 归一），脏文件走
> `hash-object`（默认不归一），同一个文件在基线里长什么样竟然取决于它当时脏不脏。

**做什么**

1. **重造 fixture**（§1.5），按场景 1 的方式在**干净**状态下建库。

2. **终端 C** 给一个文件设上 `svn:eol-style=native`：

   ```powershell
   cd $fx\wc2
   svn update
   svn propset svn:eol-style native Code\A.cs
   svn commit -m "set eol-style"
   ```

3. **终端 B**：

   ```powershell
   cd $fx\wc1
   svn update
   ```

4. 应用里点 **「重建基线」**，然后记下 blob 的 SHA：

   ```powershell
   git rev-parse "refs/heads/svn/base:Code/A.cs"
   ```

   记成 **H1**。

5. 改脏这个文件，让终端 C 再新增一个文件并提交（制造失同步），再点一次
   **「重建基线」**：

   ```powershell
   Set-Content Code\A.cs -Value 'dirty now' -NoNewline
   ```

   ```powershell
   git rev-parse "refs/heads/svn/base:Code/A.cs"
   ```

   记成 **H2**。

**应该看到**

**H1 == H2**（形如 `37bdd2221a71b58576da9d3c2dc0ef0998263652` 的 40 位串）。

**看到别的说明什么**

| 现象 | 说明 |
|---|---|
| H1 ≠ H2 | 行尾归一没对齐。后果：在脏工作副本上重建一次，这个文件在工作分支上会凭空出现「每一行都改了」的假 diff |

---

### ⬜ 场景 12：整个过程一条 SVN 写命令都没发

> 本期的硬约束：产品代码只读 SVN。

**做什么**

**终端 B**，在应用里做任何操作**之前**和全部做完**之后**各跑一次：

```powershell
svn info --show-item revision (svn info --show-item repos-root-url)
```

**应该看到**

服务器的 revision **只在你自己用终端 C 提交时才变**。应用里的任何操作
（初始化、重建、体检）都不该让它变。

**看到别的说明什么**

| 现象 | 说明 |
|---|---|
| 你没在终端 C 提交过，revision 却涨了 | **最严重**：应用往服务器写了东西。立刻停下并报告 |

---

## 3. 收尾

### 3.1 删掉 fixture

**先关掉应用里打开的仓库**（或直接关应用），否则文件被占用删不掉。

```powershell
cd ~
Remove-Item -Recurse -Force $fx
```

删的是 `%TEMP%` 下那个随机后缀目录，**跟你任何真实仓库无关**。

顺手看看有没有以前留下的：

```powershell
Get-ChildItem $env:TEMP -Directory -Filter "phase1c_*" | Select-Object FullName
```

### 3.2 停掉应用

终端 A 里 `Ctrl+C`。

### 3.3 记结果

把每个场景的 ⬜ 改成 ✅ 或 ❌。❌ 的写清楚：

```text
场景 5：❌
  期望：BEFORE / AFTER 三组输出逐字相同
  实际：Get-FileHash 的值变了
        BEFORE: A1B2C3...
        AFTER:  D4E5F6...
```

不用自己排查原因。

---

## 附：和 1c 那份清单的关系

| | 测什么 | 文件 |
|---|---|---|
| 1c 清单 | 失同步横幅、SVN log 翻页、混合图、本地补丁标记 | `2026-09-15-svn-phase1c-e2e.md` |
| **本清单** | 脏工作副本上的建库 / 重建 / 体检 | 本文件 |

1c 清单里那节「已知问题：重建基线在真实开发节奏下点不动」**原样保留着**——
它是这个分支存在的全部理由，里面的现象、根因和「为什么不能简单地放行」三段，
正是新做法必须绕开的陷阱。

## 附：这期**不**在范围内的东西

别拿这些来判定失败：

- **`sync_in` / `sync_out`**（在应用里点一下就更新 / 交付）——那是 Phase 2、3。
  这一期更新和交付仍然要回 TortoiseSVN 手工做。
- **本地补丁的摘除与回落**——Phase 3。这一期「仅本地」只是一条标记。
- **白名单外改动的视图与 revert**——Phase 4。
- **Phase 2 的行尾问题**——`sync_in` 的 stash 往返会把没设 `svn:eol-style` 的文件
  还原成 CRLF。已记在 spec §7.3.0，**本期不修**，因为这一期根本没有 `sync_in`。
- **`svn status` / `svn diff` 的「大小 + mtime」捷径盲区**——拿十六进制编辑器改一个
  同长度的字节再把时间戳改回去，体检看不见。这是**既有**盲区，不是本期引入的。

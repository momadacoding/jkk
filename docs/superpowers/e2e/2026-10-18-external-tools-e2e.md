# 外部 diff / merge 工具 E2E 验证清单

- 日期：2026-10-21（第二版，重写原因见决策日志 Task 12「E2E 清单第一版作废」）
- 分支：`worktree-external-tools`
- 设计：`docs/superpowers/specs/2026-10-17-external-diff-merge-tools-design.md`（场景 1–7 来自 §7「真实窗口」）
- 计划 / 决策日志：`docs/superpowers/plans/2026-10-18-external-diff-merge-tools.md` 及同名 `-decisions.md`

## 准备

1. 造现场（只在临时仓库里做，`AGENTS.md` 最高优先级约束）：

   ```powershell
   pwsh -File scripts/e2e-external-tools-fixture.ps1
   ```

   最后一行打印根目录，例如 `C:\Users\<你>\AppData\Local\Temp\jkk-e2e-ext-xxxxxxxx`，下文记作 `<根>`。
   **一个场景一个仓库，目录名以场景编号开头，每个冲突仓库只有一个冲突文件**，走完一条不影响下一条。

2. 启动 `pnpm tauri dev`，用窗口里的 **Open Repo** 打开下文写的仓库。
   dev 版和已安装的 jkk 共用配置，会恢复你平时开着的仓库标签——**只在 `<根>` 下的仓库标签里操作**。

3. 外部工具用默认预设：Beyond Compare 5（`C:\Program Files\Beyond Compare 5\BComp.exe`）和 VS Code（PATH 上的 `code`）。

4. 冲突仓库里三边的内容带固定标记：HEAD（main）那边是 `from-main`，被合并的 feat 那边是 `from-feat`，
   共同祖先是 `base`。下文的「左 / 右 / 中」都按这个认。

5. 全部走完后关掉这些标签，删除 `<根>` 整个目录。

## 总表

| # | 场景 | 仓库 | 结果 | 备注 |
|---|---|---|---|---|
| 1 | 普通文本冲突：BC 解决 → Stage → Commit | `01-merge-text` | ✅ 真机（2026-10-21） | 1–6 步全部符合；仓库里实际生成两个父提交的合并提交、工作区干净、MERGE_HEAD 已清除。第一版清单的失败原因见决策日志 |
| 2 | rebase 冲突：BC 的标题与内容 | `02-rebase` | ✅ 真机（2026-10-21） | 1–4 步全部符合；Stage 后 `a.txt` 为 `M `、无冲突，`.git/rebase-merge` 仍在，顶部只剩 `rebase in progress`（预期） |
| 3a | DU：保留修改后的文件 | `03a-keep` | ✅ 真机（2026-10-21） | 1–4 步全部符合；合并提交有两个父提交，`Code/Gone.cs` 内容是 `from-feat` 那版，工作区干净 |
| 3b | UD：删除文件（先取消、再确认） | `03b-delete` | ✅ 真机（2026-10-21） | 1–5 步全部符合；合并提交有两个父提交，`Code/Gone.cs` 磁盘和 index 里都没有了，工作区干净 |
| 4 | 普通改动和历史提交的外部 diff | `diff` | ✅ 真机（2026-10-19） | 决策日志 Task 4；本轮不用重做 |
| 5 | VS Code 打开对比与三方合并 | `05-vscode` | ✅ 真机（2026-10-21） | 左栏 `P.REMOTE.cs`=feat、右栏 `P.LOCAL.cs`=HEAD；Complete Merge 后 VS Code 自己暂存了文件（VS Code 行为，见决策日志）；合并提交有两个父提交，`notes.txt` 仍是未暂存改动 |
| 6 | 场景文件：自动 driver 没合完 | `06-scene` | 6a ✅ 真机（2026-10-21）；6b ✅ 真机（2026-10-22） | 6a：合并提交有两个父提交，场景文件为 `x: 5`、无冲突标记，工作区干净。6b（真实 UnityYAMLMerge 6000.3.26f1）：首测因 fixture 场景文件不合法、工具工作目录不对失败，修复后复测 1–6 步全部符合——规则命中、自动弹出 BC、无退出码报错；合并提交两个父提交、相对 HEAD 无内容变化（选了 HEAD 那版） |
| 7 | 缺 driver 的横幅与合并前确认 | `07-drivers` | ✅ 真机（2026-10-21） | 三条横幅文案与颜色符合；回车 = Cancel，main 无新提交、无 `CHERRY_PICK_HEAD`。首测看不出焦点，补了焦点样式后复验能看到 Cancel 的蓝框（见决策日志）。另发现 Tab 会把焦点移出弹框，见决策日志 |
| 8 | 二进制冲突 | `08-binary` | ✅ 真机（2026-10-22） | 1–4 步全部符合；Stage 后 Staged 为空（工作区是 HEAD 那版，预期）；合并提交两个父提交，工作区干净 |
| 9 | 特殊路径的冲突文件 | `09-special-path` | ✅ 真机（2026-10-22） | 1–4 步全部符合；`目录 a/-f[1] x.txt` 内容为 `from-feat`，目录里没有多出别的文件；合并提交两个父提交，工作区干净 |
| 10 | 同一个文件重复打开 | `10-busy` | ✅ 真机（2026-10-22） | 1–3 步全部符合；关掉 BC 后 `Code/P.cs` 仍是 `UU`（没保存，预期），`%TEMP%\jkk-external` 下没有残留子目录，没有残留的 BC 进程 |
| 11 | 新命令的 invoke 都能通 | — | ✅ 真机（2026-10-22） | 由 1（`read_conflicts`、`open_external_merge`）、3a / 3b（`resolve_delete_modify`）、7（`check_merge_drivers`）推出 |
| 12 | 大仓库的 driver 检查不卡 | `big`（另造） | ✅ 真机（2026-10-22） | 50001 个被跟踪文件；打开后 1 秒内出现 `e2e-missing` 横幅；保存 `F001.cs` 后出现在 Unstaged，界面不卡、横幅不闪。顺带发现：选中 `big` 提交时详情显示了工作区的改动（大提交 diff 要几十秒，期间显示的是上一份），已修复并真机复验，见决策日志 |

## 逐条步骤

每一步「→」后面是应该看到的结果。和写的不一致就停下，把那一步的编号和实际看到的告诉我。

### 1 普通文本冲突（`<根>\01-merge-text`）

1. 打开仓库 → 顶部显示 `merge in progress · 1 个文件有冲突`。
2. 在左侧 Unstaged 里点 `Code/P.cs`（带红色 `!`）→ 右侧标题是「这个文件有冲突」，有按钮「在 Beyond Compare 中解决」，**下面不显示 diff**。
3. 右键 `Code/P.cs` →「在 Beyond Compare 中解决」→ BC 打开四栏，标题分别是：左 `main（HEAD）`（内容 `from-main`）、
   右 `feat`（内容 `from-feat`）、中 `共同祖先`（内容 `base`）、下方输出 `结果（工作区）`。
4. 在 BC 里第一行选一边（比如 `from-feat`），保存，关掉 BC → 回到应用，右侧**自动**变成「未检测到冲突标记，检查后可以暂存」，
   下面是 diff：`from-main` → `from-feat`。
5. 点右侧的 **Stage** → `Code/P.cs` 移到 Staged，`!` 消失，顶部变成 `merge in progress`（不再说有冲突）。
6. 写提交信息，Commit → 提交成功，顶部的 merge 提示消失，提交图上多一个有两个父提交的合并提交。

### 2 rebase 冲突（`<根>\02-rebase`）

1. 打开仓库 → 顶部显示 `rebase in progress · 1 个文件有冲突`。
2. 右键 Unstaged 里的 `a.txt` →「在 Beyond Compare 中解决」→ BC 标题：
   左 `当前重放结果（HEAD，基于 main）`，右 `REBASE_HEAD <7 位 hash> c2 改第一行`，中 `共同祖先`，输出 `结果（工作区）`。
3. 看左栏内容 → 最后一行是 `line5 c1`（feature 的第一个提交已经重放上去了），第一行是 `line1 main`。
4. 第一行选一边，保存，关掉 BC → 右侧变成「未检测到冲突标记，检查后可以暂存」→ 点 **Stage** → `a.txt` 移到 Staged，
   顶部变成 `rebase in progress`（不再说有冲突；rebase 还没继续，所以提示仍在）。

本条到 Stage 为止。应用里没有「继续 rebase」的入口，不在这次功能范围内。

### 3a DU：保留（`<根>\03a-keep`）

1. 点 Unstaged 里的 `Code/Gone.cs` → 右侧标题「HEAD 这边删除了这个文件，另一边修改了它」，
   说明「选择保留修改后的文件，或删除它。选择后立即暂存。」，两个按钮：「保留修改后的文件」、红色「删除文件」。
2. 右键 `Code/Gone.cs` → 菜单里**没有**「在 … 中解决」和「用其他工具打开 ▸」，只有「编辑外部工具配置…」（加上原有的菜单项）。按 Esc 关掉菜单。
3. 点「保留修改后的文件」→ **不弹确认框**，`Code/Gone.cs` 立刻出现在 Staged 里（新增），Unstaged 里没有它了。
4. Commit → 成功，合并结束。

### 3b UD：删除（`<根>\03b-delete`）

1. 点 Unstaged 里的 `Code/Gone.cs` → 右侧标题「HEAD 这边修改了这个文件，另一边删除了它」，按钮同 3a。
2. 点红色「删除文件」→ 弹出红色确认框，标题「删除 Code/Gone.cs？」。
3. 点 **Cancel** → 什么都没变：`Code/Gone.cs` 仍在 Unstaged、带 `!`，磁盘上的文件还在。
4. 再点「删除文件」→ 这次点确认框里的「删除文件」→ `Code/Gone.cs` 出现在 Staged 里（删除），Unstaged 里没有它了。
5. Commit → 成功，合并结束。

### 5 VS Code（`<根>\05-vscode`）

1. 右键 Unstaged 里的 `notes.txt`（普通改动，没有 `!`）→「用其他工具打开 ▸」→「VS Code」→ VS Code 打开对比：
   左 `note base`，右 `note changed in workdir`。关掉这个对比标签。
2. 右键 `Code/P.cs`（冲突）→「用其他工具打开 ▸」→「VS Code」→ VS Code 打开三方合并编辑器，标签页标题 `Merging: P.cs`。
   命令行打开时两栏上方显示的是**文件名**，不是 git 插件里的「Incoming / Current」：
   **确认左栏是 `P.REMOTE.cs`、内容 `from-feat`，右栏是 `P.LOCAL.cs`、内容 `from-main`**
   （与 VS Code 自己解冲突时「左 Incoming、右 Current」的排法一致）；下方 Result 的路径是工作区里的 `05-vscode\Code\P.cs`。
   Result 一开始是共同祖先的内容（第一行 `base`、上方写 `No Changes Accepted`），这是 VS Code 的行为。
   编辑器里红色的 `; expected` 是 C# 语言服务的语法提示，与本功能无关。
3. 点左栏或右栏上方的「Accept P.REMOTE.cs」或「Accept P.LOCAL.cs」→ Result 第一行变成对应内容 → 点右下角「Complete Merge」
   → VS Code 关闭合并编辑器，**并自己 `git add` 了结果文件**（VS Code 官方文档：「Complete Merge stages that file and closes
   the merge editor」）→ 回到应用，`Code/P.cs` 已经在 Staged 里、没有 `!`，顶部变成 `merge in progress`。
   这一步的暂存是 VS Code 做的，不是应用做的；用 BC 时不会这样（场景 1）。
4. Commit（`notes.txt` 留在 Unstaged，不用管）→ 成功。

### 6 场景文件（`<根>\06-scene`）

6a 和 6b 用两套不同的 fixture：6a 用默认参数造的 `06-scene`，driver 是一个必然失败的假 driver（文件保持 HEAD 那边的内容，
driver 返回 1），专门看「工作区与 HEAD 完全相同」的显示；6b 用真实 UnityYAMLMerge。

**6a（假 driver）**

1. 打开仓库 → 顶部只有 `merge in progress · 1 个文件有冲突`，**没有** merge driver 横幅。
2. 点 `Assets/Scenes/Main.unity` → 右侧标题「这个文件配置了自动合并工具 `unityyamlmerge`，但仍处于冲突状态，说明它没能完全合并」，
   说明「文件内容不能直接当作合并结果（例如 UnityYAMLMerge 会在冲突处取对方的值），需要手动合并」，
   两个按钮「在 Beyond Compare 中解决」「Stage」；按钮下面不是 diff，而是一句「工作区与 HEAD（你这边）完全相同：另一边的改动都没有进来」。
3. 点「在 Beyond Compare 中解决」→ BC 左 `main（HEAD）` 是 `x: 9`，右 `feat` 是 `x: 5`，中 `共同祖先` 是 `x: 1`。
4. 选 `x: 5`，保存，关掉 BC → 右侧那句「完全相同」消失，换成 diff：`x: 9` → `x: 5`；标题不变（这个文件按配置始终算「自动 driver 没合完」）。
5. Stage → Commit → 成功。

**6b（真实 UnityYAMLMerge）**

前置：

- 用本机 UnityYAMLMerge 另造一套 fixture（本机 Unity 装在 `E:\unity`，不在脚本默认的 `Program Files` 下）：

  ```powershell
  pwsh -File scripts/e2e-external-tools-fixture.ps1 -UnityYamlMerge 'E:\unity\6000.3.26f1\Editor\Data\Tools\UnityYAMLMerge.exe'
  ```

  只用新根目录下的 `06-scene`（下文记作 `<根2>\06-scene`）。git 调用真实 UnityYAMLMerge（`--fallback none`）后停在冲突上：
  UnityYAMLMerge 在冲突处取了对方的值、以 2 退出，工作区是 feat 那边的 `x: 5`、没有冲突标记。
- `externalTools.json` 加上 README「按文件类型选工具」里的 `unityyamlmerge` 工具和规则（`program` 改成本机路径）。
- UnityYAMLMerge 自带的 `mergespecfile.txt` 里 `.unity` 的专用后备是占位程序（不存在），会落到 `*` 行里的
  `%programs%\Beyond Compare 5\bcomp.exe`。

步骤：

1. 改完配置后点一下应用窗口（切回窗口时重读配置）→ 打开 `<根2>\06-scene`。
2. 点 `Assets/Scenes/Main.unity` → 标题同 6a，按钮变成「在 UnityYAMLMerge 中解决」「Stage」（规则命中），
   下面是 diff：`x: 9` → `x: 5`（UnityYAMLMerge 自动取了对方的值，正是说明里「文件内容不能直接当作合并结果」的情形）。
3. 右键它 → 第一项「在 UnityYAMLMerge 中解决」，「用其他工具打开 ▸」里有 Beyond Compare 和 VS Code。按 Esc 关掉菜单。
4. 点「在 UnityYAMLMerge 中解决」→ **不再弹「Could not determine the transform parent」报错**；UnityYAMLMerge 合不了
   `m_LocalPosition`，**自动弹出 BC**（BC 是 UnityYAMLMerge 起的，标题是临时文件路径，不是应用写的 `main（HEAD）` 那一套）。
5. 在 BC 里把结果选成 `x: 9`（HEAD 那边），保存，关掉 BC → UnityYAMLMerge 随之退出 → 应用右侧的 diff 消失，
   换成「工作区与 HEAD（你这边）完全相同：另一边的改动都没有进来」。
   顶部如果报了 UnityYAMLMerge 的退出码，记下码值（配置里没写 `successExitCodes`，只认 0）。
6. 点 Stage → 文件从 Unstaged 消失，**Staged 里也看不到它**（结果与 HEAD 相同，暂存区相对 HEAD 没有差别，git 的正常行为），
   顶部变成 `merge in progress` → 写提交信息，Commit（Staged 为空也能点）→ 生成两个父提交、内容不变的合并提交。

### 7 缺 driver（`<根>\07-drivers`）

1. 打开仓库 → 顶部三条横幅：
   - 红色：「.gitattributes 让 Assets/Main.unity 等文件用 merge driver e2e-missing，但本机 git 配置里没有定义它。……」，
     下面带一段配置示例（`[merge "e2e-missing"]` 开头）；
   - 红色：「merge driver e2e-half 只配了一部分，缺少 driver 命令。……」；
   - 琥珀色：「merge driver e2e-badexe 指向的程序不存在：C:/nope/NotThere.exe。……」。
2. 在提交图上右键提交「拿来 cherry-pick 的提交」（分支 `pick` 上那个）→ 菜单项「Cherry Pick <7 位 hash>…」→ 点它
   → 先弹原有的确认「Cherry-pick <hash> onto the current branch?」→ 点 OK。
3. → 接着弹红色的「合并前确认」框，说没有定义 `e2e-missing`；**Cancel 外面有一圈蓝色焦点框**，红色「仍然继续」没有。
4. **直接按回车** → 框关闭，什么都没执行：提交图上 main 没有新提交，顶部没有 `cherry-pick in progress`，三条横幅还在。

### 8 二进制冲突（`<根>\08-binary`）

1. 点 `img.bin` → 右侧标题「二进制文件冲突」，**只有 Stage 一个按钮**，没有 diff。
2. 右键 `img.bin` → 菜单里**没有**「在 … 中解决」和「用其他工具打开 ▸」。按 Esc 关掉菜单。
3. 点 Stage → 文件从 Unstaged 消失，**Staged 里也看不到它**：二进制冲突时 git 把 HEAD 那版原样留在工作区
   （`50 4E 47 00 02`），暂存后与 HEAD 相同，暂存区没有差别（与 6b 第 6 步同理）。
4. 写提交信息，Commit → 生成两个父提交、`img.bin` 保持 HEAD 那版的合并提交。

### 9 特殊路径（`<根>\09-special-path`）

1. Unstaged 里的文件显示为 `目录 a/-f[1] x.txt`，点它 → 右侧「这个文件有冲突」。
2. 右键它 →「在 Beyond Compare 中解决」→ BC 正常打开，左 `from-main`、右 `from-feat`、中 `base`，顶部不报错。
3. 选 `from-feat`，保存，关掉 BC → 右侧变成「未检测到冲突标记，检查后可以暂存」，diff 是 `from-main` → `from-feat`
   （说明结果写回了工作区里那个真实文件，而不是别处）。
4. Stage → Commit → 成功。

### 10 重复打开（`<根>\10-busy`）

1. 右键 `Code/P.cs` →「在 Beyond Compare 中解决」→ BC 打开。**不要关它。**
2. 回到应用，再右键 `Code/P.cs` →「在 Beyond Compare 中解决」→ 顶部报错「Code/P.cs 已在 Beyond Compare 中打开」，**没有第二个 BC 窗口**。
3. 不保存，关掉 BC。再点一次「在 Beyond Compare 中解决」→ 这次正常打开（锁已经放开）。关掉 BC 即可，本条不用提交。

### 12 大仓库

1. 另造一个大仓库（5 万个文件，脚本要跑几分钟）：

   ```powershell
   pwsh -File scripts/e2e-external-tools-fixture.ps1 -Shape Big
   ```

2. 打开它下面的 `big` 仓库 → 顶部出现 `e2e-missing` 的红色横幅。记下从打开到横幅出现大约几秒。
3. 用记事本改 `big\Assets\D0000\F001.cs` 并保存 → 文件出现在 Unstaged，界面不卡，横幅不闪（不消失再出现）。
4. 用完删除这个仓库的根目录。

## 大仓库的 driver 检查耗时（引擎层）

`cargo test --release --test merge_drivers -- --ignored --nocapture perf_50k_files`（5 万个被跟踪文件，500 个目录）：
首次 272 ms，命中缓存 102 ms（2026-10-21，release 构建）。第一次实测时发现 `check-attr --stdin` 管道死锁，
已修复，见决策日志 Task 12。

## 已知未覆盖

- 2：rebase 冲突解决后的「继续 rebase」应用里没有入口，不在本功能范围。
- 非 Windows 平台没有验证。

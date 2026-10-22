# 外部 diff / merge 工具 + 冲突文件显示 设计

> 对应 GitHub 第 64 条（吸收第 63 条，并从第 50、51 条各拿走一部分，见 §9）。
> 2026-10-17 与用户逐段确认；标「实现者决定」的是留给实现者的，标「我的决定」的是设计者替用户拿的主意、用户已看过。

## 1. 为什么做

用户在 Fork 里用了多年，**解冲突一律在 Beyond Compare（BC）里做**，从不用 GUI 内置的冲突面板 / 合并器。mygitclient 目前：

- 冲突文件（`UU` 等）在「本地改动」右侧显示「没有文本改动」、行数 `+0/-0`。原因：`src-tauri/src/engine/diff.rs` 的 `read_diff` 默认分支用 `diff_tree_to_workdir_with_index`，libgit2 对冲突条目只给一个 `Delta::Conflicted`，不带 hunk（第 63 条）。
- 没有任何入口把文件交给外部工具，无论冲突文件还是普通改动。

目标是复刻用户在 Fork 里的真实工作流，而不是做一个「更好的」冲突解决器。

## 2. 目标 / 非目标 / 反目标

**目标**

- 冲突文件选中时，右侧按五种情况显示（§4.1）。
- 冲突文件一键交给外部 merge 工具（默认 BC），解完回来能看到结果 diff。
- 普通改动、已暂存文件、历史提交里的文件一键交给外部 diff 工具。
- 一边删一边改的冲突（DU / UD）在应用内「保留 / 删除」二选一。
- 按文件类型单独指定 diff / merge 工具。典型场景：Unity 的 `.unity` / `.prefab` / `.asset` 冲突用 UnityYAMLMerge 解，看 diff 仍用 BC（§4.4）。用户原话：用 BC 解场景文件冲突「不太靠谱」。
- 检查 `.gitattributes` 引用的自定义 merge driver 在本机是否已配置，缺了就警告，并在应用发起合并前拦一下（§4.6）。用户要求这次一起做完。

**非目标**

- 内置三栏合并器、冲突面板、combined diff（`--cc`）渲染。
- 设置界面。配置走 JSON 文件。
- Continue / Abort、二进制冲突的「用我的 / 用对方的」（留在第 51 条）。
- Conflicts 独立分区、横幅可点击、Stage 前冲突标记确认框（留在第 50 条）。

**反目标**（做出来也算失败）

- 外部工具退出后**自动 `git add`**。冲突文件必须由用户手动 Stage（DU / UD 的二选一除外，见 §4.1）。
- 删掉、覆盖或移动用户工作区里的文件。`$MERGED` 和 diff 的工作区一侧都是用户的真实文件，任何清理逻辑都不能碰。
- BC 启动失败时悄悄换成别的工具。

## 3. 已实测的事实（Fork 行为与 git 状态）

2026-10-17 在一次性 fixture 仓库（merge 冲突 + rebase 冲突）里，用户用 Fork + BC 走了一遍，配合 `git ls-files -u` 核对：

| 观察 | 结论 |
|---|---|
| BC 里解完、关掉后，Fork 右侧显示「File conflicts are resolved. You can stage the file now.」，文件留在 Unstaged | Fork 不自动暂存 |
| 此时 `git status` 仍是 `UU`，`ls-files -u` 仍有 stage 1/2/3 | 「resolved」是 Fork 按文件内容自己判断的（推测依据：文件里已没有冲突标记） |
| 右侧 diff：左 = `HEAD` 版本，右 = 工作区文件。验证方法：把结果改成两边都没有的内容（「手工合并」「line5 手改」），右侧如实显示 | **resolved 之后显示的是 HEAD 对比工作区**，不是 base，也不是 ours 对比 theirs |
| DU 冲突（`Gone.cs` main 删、feature 改）在 Fork 右侧面板勾选保留 → 立刻变成 `A Code/Gone.cs` | DU / UD 的二选一**立即暂存** |
| rebase 中 Fork 左侧（HEAD）是上游 `main` 的内容 | rebase 时 ours 是 HEAD，theirs 是正在重放的提交。这次只重放了一个提交，所以 HEAD 恰好等于上游。重放多个提交时，HEAD 是「上游 + 已经重放成功的那些提交」（Codex review 指出） |

## 4. 设计

### 4.1 冲突文件右侧显示什么

**总原则**（Codex review 后加）：只有**能确认**以下三点都成立时，才显示「可以暂存」：

- git 用的是内置逐行文本合并；
- 三份输入都是文本；
- 文件里检测不到冲突标记。

**确认不了的，一律保守处理，不说「已解决」。** 任何情况下都**不自动暂存**（③ 的二选一除外）。

按下面的顺序判定，命中第一条就停：

| 顺序 | 情况 | 判定 | 右侧显示 |
|---|---|---|---|
| 1 | ⑤ 结构冲突 | AU / UA / DD；或任一 stage 不是普通文件，比如符号链接（mode `120000`）、子模块、文件与目录互换 | 说明是哪种冲突。**不提供任何写操作，也不提供外部工具入口**，留给后续处理 |
| 2 | ③ 一边删一边改 | DU / UD，且留下的那一边是普通文件 | 说明哪一边删了、哪一边改了，给「保留修改后的文件」/「删除文件」两个按钮，**点了立即暂存**。**「删除文件」先弹确认框**，「保留」不弹 |
| 3 | ④ git 没做逐行文本合并 | UU / AA，且满足下面「④ 的判定」任意一条 | 见「情况 ④」 |
| 4 | ① 检测到冲突标记 | 工作区文件里有冲突标记（规则见「冲突标记的判定」） | 「这个文件有冲突」+ 按钮「在 <匹配到的 merge 工具> 中解决」（§4.4 规则）。不显示 diff |
| 5 | ② 未检测到冲突标记 | 以上都不是 | 顶部「未检测到冲突标记，检查后可以暂存」+ Stage 按钮；下面是 **HEAD 对比工作区** 的 diff，与普通文件的 diff 渲染一样。文件留在 Unstaged |

- 判定需要的冲突信息（每个 stage 是否存在、mode、blob id）由后端给出。现在 `diff.rs` 只输出一个 `conflicted` 字符串，不够用，不能从状态字符串反推。
- 文件列表里的冲突行**不显示 `+0/-0`**，只保留冲突角标（我的决定）。
- 工作区变化时状态自动切换：BC 保存后，现有的 notify 文件监听会触发刷新，① 变成 ②。
- ② 的 diff 要绕开 `Delta::Conflicted`，比如对冲突路径单独做 HEAD 树对比工作区文件。读取走 libgit2，具体做法由实现者决定。

#### 情况 ③ 的执行（数据安全）

- 「保留」执行 `git --literal-pathspecs add -- <path>`，「删除」执行 `git --literal-pathspecs rm -- <path>`，都走 `run_git`。
  - `--literal-pathspecs` 必须加：只写 `--` 时，路径仍按 pathspec 模式解释，`f[1].txt` 会匹配到 `f1.txt`（Codex review 指出）。
  - 路径为空时后端直接拒绝。
- 执行前，后端**重新读一遍 index**，确认这个路径仍是 DU / UD；「保留」还要确认工作区里的文件存在，并且是普通文件。不满足就报「状态已变化，请刷新」，不执行。

#### 情况 ④：git 没做逐行文本合并

`merge` 属性的几种取值（同一个文件只会生效一个值）：

| 写法 | 含义 |
|---|---|
| 不写，或 `merge` / `merge=text` | git 内置的逐行文本合并，冲突时写入冲突标记。**例外**：配置里设了 `merge.default` 时，按它选择的方式合并 |
| `merge=binary` 或 `-merge` | git 内置。不合并，工作区保留 HEAD 那一边，不写标记 |
| `merge=<自定义名字>`，例如 `merge=unityyamlmerge` | 自定义 merge driver。git 在 merge / rebase / cherry-pick / stash 等合并过程中**自动**调用配置里 `merge.<名字>.driver` 这条命令 |

**用户项目的选择：Unity 场景文件用 `merge=unityyamlmerge`，作为自动 driver**（2026-10-18 确定）。driver 的配置写法见 §4.6。

**实测**（2026-10-18，git 2.51.2.windows.1，在临时仓库里做）

表 1：同一次 merge 里放了几个文件，两边都改了第 1 行。`faildriver` 是测试时编的假 driver（`echo "driver-left-this" > %A; exit 1`），用来模拟「driver 运行了，但合并失败」：

| `.gitattributes` 和本机配置 | 结果 | 工作区里留下的内容 |
|---|---|---|
| 不写 | `UU` | 有冲突标记 |
| `merge=binary` | `UU` | 没有标记，是 HEAD 那一边的原样 |
| `merge=unityyamlmerge`，本机**完全没配**这个名字 | `UU` | **有冲突标记**（git 退回逐行文本合并） |
| `merge=faildriver`，本机配了，driver 失败 | `UU` | 没有标记，是 driver 写进 `%A` 的内容 |
| `merge=halfdriver`，本机**只配了 `name`、没配 `driver`** | git 报 `fatal: custom merge driver halfdriver lacks command line`，**整个 merge 中止**（退出码 128，没有 `MERGE_HEAD`） | 没有任何变化 |
| `merge=missingexe`，`driver` 指向一个不存在的程序 | `UU` | 没有标记，保持 HEAD 那一边的原样 |

git 调用 driver 的方式：把 base、ours、theirs 写成三个临时文件 `%O`、`%A`、`%B`，driver 把结果写进 `%A`，用退出码报告成败（0 表示成功）。正常返回时，**无论成败，git 都会把 `%A` 拷到工作区文件**；失败时还会标成冲突。

表 2：用本机真实的 UnityYAMLMerge（Unity 6000.6.6f1 自带）做 driver，命令是 `merge -h -p --force --fallback none %O %B %A %A`：

| 场景 | 结果 | 工作区里的场景文件 |
|---|---|---|
| 两边改了同一个物体的不同属性（名字、位置 y） | 自动合并成功，**不进入冲突**，文件直接是已暂存状态 | 两边的改动都在 |
| 两边把同一个属性改成不同值（ours x=9，theirs x=5） | `UU` | 合法的 YAML，没有标记。**冲突处取的是 theirs 的值（x=5）**，其余改动已合并 |
| 场景结构不完整，UnityYAMLMerge 解析失败 | `UU` | 保持 ours 原样 |

表 2 第 2 行就是情况 ④ 必须存在的直接证据：这个文件格式合法，也和 HEAD 不一样，如果按 ② 显示「可以暂存」，用户一暂存，就把 theirs 的值当成了合并结果。

**④ 的判定**：冲突类型是 UU / AA，满足下面任意一条就算 ④：

- 该路径生效的 `merge` 属性是 `binary`、unset，或者是自定义名字；
- `merge` 属性没写，但配置里的 `merge.default` 不是文本合并；
- stage 1 / 2 / 3 里**任意一份**是二进制。git 内置的文本合并遇到这种情况会退回 binary 方式，所以不能只检查工作区里的文件（Codex review 指出）。判定方式沿用 git 的「前 8000 字节含 NUL」；
- `filter=lfs`。LFS 文件冲突后工作区里实际是什么，要实现时用 fixture 实测，结果记进决策日志。在实测之前一律算 ④，按最保守的方式处理。

属性用 git CLI 查：`git check-attr`，加上 `-z` 和要查的路径，在目标 worktree 里执行。`merge.default` 用 `git config` 查。理由见 §5。

**情况 ④ 的显示**

| 子情况 | 右侧提示 | 下面 |
|---|---|---|
| ④a：`binary` / unset / `merge.default` / LFS，三份都是文本 | 「git 没有逐行合并这个文件（按 .gitattributes 或 git 配置），需要手动合并」+ 合并按钮 | HEAD 对比工作区的 diff。两边逐字节相同时，改为提示「工作区与 HEAD（你这边）完全相同：另一边的改动都没有进来」（只说事实：用户在工具里有意选了 HEAD 那版时也是这个状态，不能说「还没合并」，2026-10-22 E2E 修订） |
| ④b：自定义 driver，三份都是文本 | 「这个文件配置了自动合并工具 `<名字>`，但仍处于冲突状态，说明它没能完全合并。文件内容不能直接当作合并结果（例如 UnityYAMLMerge 会在冲突处取对方的值），需要手动合并」+ 合并按钮 | 同上 |
| ④c：任意一份是二进制 | 「二进制文件冲突」，不给外部 merge 入口 | 无。「用我的 / 用对方的」属于第 51 条 |

- ④b 的文案只描述**当前配置**，不说「刚才的合并调用过它」。冲突发生之后，配置可能被改过，所以不能凭当前配置断言历史上发生了什么（Codex review 指出）。
- 合并按钮按 §4.4 的规则选工具，例如场景文件显示「在 UnityYAMLMerge 中解决」。外部工具拿到的是 stage 1 / 2 / 3 的**原始三个版本**，不是 driver 留在工作区里的半成品。
  - UnityYAMLMerge 作为手动合并工具使用时**不带** `--fallback none`：它合不了的部分，会按 Unity 自带的 `mergespecfile.txt` 交给后备工具，这个文件里默认列了 Beyond Compare 5。
- ④ **永远不显示「可以暂存」**，但 Stage 按钮始终可以点。
- 实现时把表 1、表 2 固化成 Rust fixture 测试。真实的 UnityYAMLMerge 不进自动化测试（依赖本机安装），用一个行为相同的假 driver 代替：写入合法内容，然后以 1 退出。

#### 冲突标记的判定（区分 ① 和 ②）

- 在 Rust 侧只对冲突路径读工作区文件。二进制文件已经在 ④c 处理了。
- **标记长度** L 取该路径的 `conflict-marker-size` 属性，没有就用 7。每一行**行首至少 L 个**同一种字符才算标记行。用「至少」，是因为 git 遇到改名冲突时会自动把标记加长（Codex review 指出）。
  - `<` / `|` / `>` 标记行：后面跟空格或行尾；
  - `=` 标记行：后面直接是行尾（允许 `\r`）。
- **只要出现任意一行 `<` 标记或 `>` 标记，就算 ①**，不要求完整的一块。用户只删掉一半标记的情况也会被拦住。
- 孤立的 `=` 行（例如 Markdown / RST 的标题下划线）、孤立的 `|` 行不算。
- 第 50 条的「Stage 前检查冲突标记」以后会复用这个判定，所以接口要能单独调用。

### 4.2 外部 merge 工具（冲突文件）

- **入口**：冲突文件的右键菜单，以及 ① / ④a / ④b 的合并按钮。③、④c、⑤ 没有入口。
- **四个文件**：

| 占位符 | 内容 | 来源 |
|---|---|---|
| `$LOCAL` | ours（stage 2） | 临时文件 |
| `$REMOTE` | theirs（stage 3） | 临时文件 |
| `$BASE` | base（stage 1）；AA 没有 base，用空文件 | 临时文件 |
| `$MERGED` | **工作区里的真实文件**（绝对路径；在 linked worktree 里，按那个 worktree 解析） | 用户文件，不删不动 |

- **启动前**，后端用不跟随符号链接的方式检查 `$MERGED`，它必须是普通文件，否则拒绝。这是防止外部工具顺着符号链接写到别的文件上（Codex review 指出）。
- **临时文件用工作区形态**（我的决定）：内容要经过 autocrlf / eol、`ident`、`working-tree-encoding`、LFS smudge 这些过滤器，和用户 checkout 出来的文件一致。
  - 用 `git cat-file --filters --path=<路径> <blob>` 读出内容，**用 `run_git_bytes` 按字节原样写进文件**。不能用 `run_git`：它会经过 `from_utf8_lossy`，UTF-16、GBK 这类内容会被改坏（Codex review 指出）。
  - 过滤器失败就中止启动，**不能拿空文件顶替**。
  - 注意：`cat-file --filters` 用的是**当前**工作区的过滤器配置，不能保证和那个历史版本当初 checkout 时的环境完全一样。
  - 这是对「读取走 libgit2」的有意偏离：libgit2 跑不了 LFS 这类外部过滤器。
- 临时文件名保留原扩展名，例如 `P.LOCAL.cs`、`P.REMOTE.cs`、`P.BASE.cs`。放在应用自己的临时目录下，每次调用一个独立子目录。具体位置由实现者决定。
- **标题**：BC 支持 `/lefttitle=`、`/righttitle=`、`/centertitle=`、`/outputtitle=`（已查 BC5 官方命令行文档）。各侧用真实名字，不写 ours / theirs：
  - merge 时，左 = 当前分支（HEAD），右 = 被合并的分支（`MERGE_HEAD`）；
  - rebase 时，左 = 「当前重放结果（HEAD，基于 <新基线>）」，右 = 正在重放的提交（`REBASE_HEAD`，短 hash + 标题）；
  - cherry-pick / revert 同理，右边分别用 `CHERRY_PICK_HEAD` / `REVERT_HEAD`。
  - 具体文案由实现者决定，但 rebase 和 merge 必须区分。
- **进程生命周期**：
  - 后端在导出临时文件**之前**，先占住「worktree + 路径」这把锁，所有入口共用。同一个文件已经在外部工具里打开时，新的点击直接报「已在 <工具> 中打开」。进程退出或启动失败时释放锁。
  - 启动失败、进程异常退出、已知的工具故障码（例如 BC 的 102「无法等待 BCompare」、103「找不到 BCompare」）都要报给用户。**退出码永远不用来判断冲突是否已解决。**
  - 进程结束后，只删除本次的临时子目录。删除前校验目标在应用临时根目录内，`$MERGED` 永远不在删除范围里。
- **刷新**：进程结束后，后端发出完成事件，事件里带上 worktree 身份；前端只刷新对应的仓库。新增的刷新路径要校验请求代次和 scope，防止旧请求的结果晚到、覆盖新状态（Codex review 指出：现在 `repoStore.ts` 的 `refreshStatus` / `refreshDiff` 没有这层校验）。
- **不自动暂存。不用 `git mergetool`**：它退出后会自动 `git add`，默认还会留下 `.orig` 备份。

### 4.3 外部 diff 工具（非冲突文件）

- **入口**：「本地改动」Unstaged / Staged 里的文件、提交详情的文件列表（包括 stash 和 SVN 节点），右键菜单。
- **两侧**（用户已确认）：

| 位置 | `$LOCAL`（左） | `$REMOTE`（右） |
|---|---|---|
| Unstaged | index（stage 0） | 工作区真实文件 |
| Staged | HEAD | index |
| 提交详情 `commit:<oid>` | 第一父提交（根提交用空文件） | 该提交 |
| 提交详情 `range:<a>..<b>`（SVN 节点） | a | b |

- 注意：应用内「本地改动」右侧显示的是 HEAD 对比工作区（scope `workdir`），而外部 diff 的 Unstaged 是 index 对比工作区，Staged 是 HEAD 对比 index。这是有意的，和 `git difftool` / `git difftool --cached` 一致，用户已确认。
- 某一侧不存在（新增、删除、未跟踪）时用空临时文件。改名按 `find_similar` 的结果处理：左边用旧路径的内容，右边用新路径的内容。
- 临时文件的导出方式、锁、生命周期、刷新，和 §4.2 相同。
- 工作区那一侧是真实文件，在 BC 里改了会直接存盘，这是期望的行为。工作区这一侧同样要检查不是符号链接。其他各侧都是临时文件，要不要设成只读由实现者决定。
- 冲突文件不提供外部 diff 入口，只提供外部 merge。

### 4.4 配置文件（我的决定，沿用「自定义命令」的机制）

- 路径：`app_config_dir()/externalTools.json`，Windows 上是 `%APPDATA%\com.momadacoding.jkk\externalTools.json`。实现参照 `src-tauri/src/commands/custom.rs` 和 `engine/custom_config.rs`：
  - 文件不存在时写入默认预设；
  - 提供「用系统默认程序打开配置文件」的命令，这个命令**不接受路径参数**。
- 不做设置界面。
- 形状示意（字段名实现者可以调，但这些信息要保留）：

```json
{
  "version": 1,
  "defaultDiff": "bc",
  "defaultMerge": "bc",
  "rules": [],
  "tools": [
    {
      "id": "bc",
      "label": "Beyond Compare",
      "program": "C:\\Program Files\\Beyond Compare 5\\BComp.exe",
      "diffArgs": ["$LOCAL", "$REMOTE", "/lefttitle=$LOCAL_TITLE", "/righttitle=$REMOTE_TITLE"],
      "mergeArgs": ["$LOCAL", "$REMOTE", "$BASE", "$MERGED",
                    "/lefttitle=$LOCAL_TITLE", "/righttitle=$REMOTE_TITLE",
                    "/centertitle=$BASE_TITLE", "/outputtitle=$MERGED_TITLE"]
    },
    {
      "id": "vscode",
      "label": "VS Code",
      "program": "code",
      "diffArgs": ["--wait", "--diff", "$LOCAL", "$REMOTE"],
      "mergeArgs": ["--wait", "--merge", "$REMOTE", "$LOCAL", "$BASE", "$MERGED"]
    }
  ]
}
```

- **按文件类型指定工具**（用户已确认），通过顶层的 `rules`：

```json
"rules": [
  { "patterns": ["*.unity", "*.prefab", "*.asset"], "merge": "unityyamlmerge" }
]
```

  - 从上往下匹配，**第一条匹配的生效**。`diff` / `merge` 都可以不写，没写的那一项用 `defaultDiff` / `defaultMerge`。上面这条只改了 merge，所以场景文件看 diff 仍然用 BC。
  - 模式**只匹配文件名**（不看目录），**不区分大小写**，只支持 `*` 和 `?`（我的决定）。
  - 规则引用了不存在的工具 id，报配置错误并指向配置文件，不回退。
  - 改名的文件按**新文件名**匹配（我的决定）。
- **UnityYAMLMerge 不放进默认预设**：它装在 Unity Hub 下按版本区分的目录里，预设写死的路径迟早会错。用户可见的说明里给出示例（放在哪由实现者决定），用户自己填路径。这台机器上的实际路径是：

```json
{ "id": "unityyamlmerge", "label": "UnityYAMLMerge",
  "program": "C:\\Program Files\\Unity\\Hub\\Editor\\6000.6.6f1\\Editor\\Data\\Tools\\UnityYAMLMerge.exe",
  "mergeArgs": ["merge", "-p", "$BASE", "$REMOTE", "$LOCAL", "$MERGED"] }
```

  参数顺序来自 Unity 官方 Smart Merge 文档，和本机 usage 输出的 `<base> <left=theirs> <right=mine> [dest]` 一致。
- **参数**：参数是数组，占位符在**每个数组元素内部**做字符串替换，所以 `/lefttitle=$LOCAL_TITLE` 这种写法能用。替换完的数组直接交给 Rust 标准库启动，**不自己拼 shell 命令**。
  - 例外：`.cmd` / `.bat`（例如 `code.cmd`）会由 Rust 交给 `cmd.exe` 启动，并按 cmd 的规则转义，这一点 `launcher.rs` 里已有说明。禁止自己拼 `cmd /c`。
  - 参数里含 `"`、`%`、`!`、`&`、空格、中文的情况都要有测试（Codex review 指出）。
- 某个工具没写 `diffArgs` 或 `mergeArgs`，就不出现在对应的菜单里。
- 预设里 BC 的路径写本机实测的 `C:\Program Files\Beyond Compare 5\BComp.exe`（BC 不在 PATH 上）。不探测安装位置，和 `custom_config.rs` 的做法一致。
- 用 `BComp.exe`，不用 `BCompare.exe`：官方文档说，`BCompare.exe` 发现已有实例在运行时，会「tell the existing copy to start a comparison and exit immediately」。`BComp.exe` 是给版本控制系统调用的程序。**它的进程是否会等比较窗口关闭才退出，要在实现时用真实的 BC 实测**；不成立的话改用 `BComp.com` 或别的办法，并记进决策日志。
- VS Code：`--wait` 会等文件关闭才返回。官方 1.70 版本说明里给的合并示例参数顺序是 `$REMOTE $LOCAL $BASE $MERGED`（Codex review 核实），实现时在真实窗口里再看一眼 Incoming / Current 是否和预期一致。

### 4.5 菜单（用户已确认）

- 右键菜单第一项用**这个文件匹配到的工具**：先看 `rules`，没有匹配就用默认工具。例如普通文件显示「在 Beyond Compare 中对比 / 解决」，场景文件冲突时显示「在 UnityYAMLMerge 中解决」。名字取配置里的 `label`。§4.1 里的合并按钮也按同样规则选工具。
- 配置了不止一个可用工具时，再加一个子菜单「用其他工具打开 ▸」。
- 「编辑外部工具配置」的入口放在哪由实现者决定。报错信息里必须写明配置文件路径。
- 工具启动失败时**不回退**到别的工具。

### 4.6 merge driver 配置检查（用户要求本次一起做）

**为什么要做**：`.gitattributes` 跟着仓库走，但 driver 的配置写在**每台机器自己的** git 配置里，不跟着仓库走。实测（§4.1 表 1）有三种出问题的情况：

| 本机配置状态 | 合并时会发生什么 | 级别 |
|---|---|---|
| **完全没配**这个名字 | git 退回逐行文本合并，**往场景文件里写冲突标记，场景被写坏** | 严重 |
| **配置不完整**：有 `merge.<名字>.name` 之类的项，但没有 `driver` | git 报 fatal，**整个 merge 中止**，什么都不改 | 严重（合并做不了，但不会写坏文件） |
| `driver` 指向的**程序不存在**（典型原因：Unity 升级后，带版本号的旧目录被删了） | driver 失败，文件变成冲突状态，保持 ours 原样，自动合并失效 | 警告 |

**检查什么**（所有命令都在目标 worktree 里执行）

1. **实际生效的自定义名字**：用 `git ls-files -z` 列出所有被跟踪的路径，交给 `git check-attr --stdin -z merge`，取每个路径**真正生效**的 `merge` 值。
   - 去掉 `unspecified`、`set`、`unset`、`text`、`binary`、`union`，剩下的就是自定义名字。每个名字记下几条示例路径，用在提示里。
   - 子目录覆盖、`[attr]` 宏、`info/attributes`、全局和系统级属性文件，git 都已经算进去了。这样不会把被覆盖掉的旧规则误报出来（Codex review 指出：直接扫描 `.gitattributes` 原文会误报，也会漏报）。
   - 如果 `merge.default` 设成了自定义名字，同样要检查。
   - 大仓库的耗时要实测。可以按 `.gitattributes` 有没有变化来缓存结果，由实现者决定。
2. **配置**：用 `git config --get-regexp '^merge\.'` 查，要用能拿到退出码的 `run_git_code`。**退出码 1 表示没有任何匹配，这是正常结果，不是错误**；`run_git` 会把它当成失败（Codex review 指出）。同一个键出现多次时，取最后一个值。每个名字归为三类之一：有 `driver`、有其他项但没有 `driver`（配置不完整）、什么都没有（完全没配）。
3. **程序是否存在**：已经配了 `driver` 的，尽力检查命令里的程序是否存在。命令的第一个词（带引号就取引号里的内容）是绝对路径时，检查这个文件存不存在；不是绝对路径就跳过。
4. 检查本身出错时（例如 `check-attr` 失败），显示「merge driver 检查未完成」，**不能显示成配置正常**。

**结果怎么显示**

- **完全没配**：仓库顶部显示红色横幅，例如「`.gitattributes` 让 `Assets/Scenes/Main.unity` 等文件用 merge driver `unityyamlmerge`，但本机 git 配置里没有定义它。在这台机器上合并时，git 会往这些文件里写冲突标记，Unity 场景会被写坏。」并附上要添加的配置示例（见下文）。
- **配置不完整**：红色横幅：「merge driver `unityyamlmerge` 只配了一部分，缺少 `driver` 命令。在这台机器上，所有涉及这类文件的合并都会直接失败。」
- **程序不存在**：琥珀色横幅：「merge driver `unityyamlmerge` 指向的程序不存在：`<路径>`。合并时它会失败，冲突需要手动解决。」
- **不自动修改用户的 git 配置**（我的决定）：只提示，并给出可以复制的配置。

**什么时候检查**

- 打开仓库、`.gitattributes` 有变化时（复用现有的文件监听）。
- **应用自己发起会触发合并的操作之前**：merge、rebase、cherry-pick、revert、stash apply / pop、pull，以及 SVN 更新、交付里的 rebase 和补丁落回、交付慢路径里的 `git merge-tree`。
  - Codex 查了 git 2.51.2 的源码：`merge-tree --write-tree` 和 `stash apply` 也会走 driver 调用链。实现时要用 fixture 实测确认，尤其是 `svn_deliver.rs` 里实际使用的参数。
  - **完全没配**时，弹出危险样式的确认框说明后果，默认按钮是「取消」。
  - **配置不完整**时，git 自己会中止，不另外拦截，横幅已经说明了。
  - **程序不存在**时不拦截。
  - 实现者要把这些入口逐个列出来，记进决策日志。
- 在命令行或 Fork 里发起的合并拦不住，只能靠横幅提醒。

**给用户看的配置示例**（这次实测用的就是这条 driver 命令）：

```text
# .gitattributes（提交进仓库）
*.unity  merge=unityyamlmerge
*.prefab merge=unityyamlmerge
*.asset  merge=unityyamlmerge

# 全局 ~/.gitconfig（每台机器各配一次）
[merge "unityyamlmerge"]
    name = Unity SmartMerge
    driver = 'C:/Program Files/Unity/Hub/Editor/6000.6.6f1/Editor/Data/Tools/UnityYAMLMerge.exe' merge -h -p --force --fallback none %O %B %A %A
```

- 作为自动 driver 时**必须带 `--fallback none`**：driver 是 git 在后台调用的，弹出后备工具窗口会把 merge 卡住。手动解决时（§4.2）不带，好让它把合不了的部分交给 BC。
- `%O %B %A %A` 对应 UnityYAMLMerge 的 `<base> <left=theirs> <right=mine> <dest>`。

## 5. 约束（来自仓库规范）

- **读取默认走 libgit2**：stage 条目、mode、blob id，HEAD / index / 提交里的 blob，二进制判定，冲突标记判定，② 的 diff。
- **以下几处读取走 git CLI**，是有意的偏离，理由都是「真正执行合并的是 git CLI，要和它的解析结果一致」：
  - `git cat-file --filters`：导出临时文件，用 `run_git_bytes`（§4.2）；
  - `git check-attr`：查生效的 `merge`、`filter`、`conflict-marker-size` 属性（§4.1、§4.6）；
  - `git config`：查 `merge.default`、`merge.<名字>.*`，用 `run_git_code`（§4.6）。
- **写入走 git CLI 的 `run_git`**：③ 的 `git --literal-pathspecs add/rm -- <path>`。空路径直接拒绝，并要有测试覆盖（`ops.rs` 出过空 pathspec 的事故）。
- **启动外部工具不是 git 操作**，不走 `run_git`。复用 `engine/launcher.rs` 的做法：`resolve_program` 按 PATHEXT 解析、`hidden_command`、标准流接到 null。另外要新增一个**能在进程退出后清理、释放锁、发事件**的变体，现有的 `spawn_detached` 只在后台线程里 `wait`。所有错误都映射成 `GitError`。
- 新增 `#[tauri::command]` 时前后端要同步（`snake_case` 对 `camelCase`），并且必须在 `pnpm tauri dev` 里实测 invoke。
- 不引入新依赖；确实需要时先征得用户同意。
- 组件不直接 `invoke`，统一走 `src/store/`；纯逻辑放在 `src/engine/`，并补 vitest。

## 6. 边界情况

| 情况 | 处理 |
|---|---|
| UU / AA | 按 §4.1 的顺序依次判定 ④ → ① → ②。AA 外部合并时 `$BASE` 用空文件 |
| DU / UD | ③。没有外部 merge 入口 |
| AU / UA / DD、符号链接、子模块、类型冲突 | ⑤。实现时用 fixture 造出每一种，记录 `ls-files -u` 的实际样子 |
| 二进制（任意一份） | ④c |
| `merge=binary` / `-merge` / 自定义 driver / `merge.default` / LFS | ④a / ④b |
| 没配 `merge=binary`，git 往场景文件里写了冲突标记 | ①。是否要配属性是仓库自己的事，本功能不替用户改 `.gitattributes` |
| 用户只删了一半冲突标记 | ①（见「冲突标记的判定」） |
| `conflict-marker-size` 改过、改名冲突时标记被加长 | 按属性读出长度，并且「至少 L 个」字符就算 |
| 文件名含空格、中文、`[]`、开头是 `-` | 临时文件名、`$MERGED`、标题参数、③ 的 literal pathspec 都要覆盖到，有测试 |
| linked worktree | index、工作区、`info/attributes`、`config.worktree`、`includeIf` 都按那个 worktree 解析，所以命令都要在那个 worktree 里执行 |
| 同一个文件重复点击 | 后端的锁拦住，报「已在 <工具> 中打开」 |
| 工具不存在、路径错误、异常退出码 | `GitError`，信息里带上配置文件路径或退出码 |
| 配置文件 JSON 格式错误 | `GitError`，指向配置文件，不能悄悄回退到预设 |
| 应用崩溃后临时目录残留 | 启动时清理旧的临时子目录，或者不管，由实现者决定。清理只能在应用临时根目录里进行 |
| 刷新结果晚到 | 用请求代次和 scope 校验，旧结果直接丢弃 |

## 7. 验收

- **Rust 集成测试**（`src-tauri/tests/`，自建 fixture 仓库）：
  - **导出**：merge 冲突和 rebase 冲突各一，导出的 `$LOCAL` / `$REMOTE` / `$BASE` 分别等于 stage 2 / 3 / 1 经过过滤器之后的内容（fixture 打开 `core.autocrlf=true`，断言换行是 CRLF）。另外要覆盖：UTF-16 和 GBK 内容逐字节一致；过滤器失败时不启动工具。
  - **rebase 有两个提交**：第一个重放成功，第二个冲突，断言 LOCAL 里**包含第一个提交的改动**。
  - **五种情况的判定**：
    - 表 1 每一行都要复现：`binary`、`-merge`、完全没配、配了但失败的假 driver、只配了 `name`（merge 中止）、程序不存在；
    - `merge.default`；
    - 三份里只有 base 是二进制；
    - AU / UA / DD；
    - 符号链接冲突：链接目标在仓库外，断言目标文件不受影响。
  - **冲突标记**：merge 风格、diff3 风格、只剩开头标记、只剩结尾标记、`conflict-marker-size=32`、改名冲突时加长的标记、孤立的 `=======`（Markdown 下划线）不算。
  - **② 的 diff 是 HEAD 对比工作区**：用「结果和两边都不一样」的内容断言，复刻 §3 的那次实测。
  - **③**：保留 / 删除之后 `ls-files -u` 为空、index 正确；文件名 `f[1].txt` 旁边放一个 `f1.txt`，断言 `f1.txt` 不受影响；空路径被拒绝；执行前状态已经变了时拒绝执行。
  - **外部 diff 四种位置的两侧内容**：包括新增、删除、改名、根提交。
  - **启动**：用一个把 argv 写进文件的假可执行文件，`.exe` 和 `.cmd` 各一个，断言占位符展开、参数顺序，以及含 `"`、`%`、`!`、`&`、空格、中文的参数都作为单个 argv 送到；进程退出后临时目录被清掉、`$MERGED` 还在；重复打开被锁拦住；非零退出码被报告。
  - **driver 检查**：根目录被子目录覆盖、`[attr]` 宏、`info/attributes`、`core.attributesFile`，都以 `check-attr` 的结果为准；`git config` 零匹配被当成正常结果；完全没配、配置不完整、程序不存在三种结果；`merge-tree` 和 `stash apply` 是否调用 driver 的实测结论。
  - **linked worktree** 场景。
- **规则匹配**（放在哪一侧由实现者决定，放在哪侧就在哪侧写单测）：第一条命中生效、大小写不敏感、只看文件名、`diff` / `merge` 分开回落、引用不存在的工具 id 时报错。
- **vitest**：五种情况到界面的映射；菜单项的生成（一个工具、多个工具、某个工具没有 `mergeArgs`、规则命中）；刷新代次校验（旧结果被丢弃）。
- **真实窗口**（`pnpm tauri dev` + 本机 BC5 + 本机 UnityYAMLMerge，用户手工走一遍）：
  - merge 冲突：右键 → BC 解决 → 保存并关闭 → 右侧从 ① 变成 ② → Stage → Commit；
  - rebase 冲突走一遍，检查 BC 里的标题；
  - DU 冲突的保留和删除；
  - 普通改动和历史提交的外部 diff；
  - 用 VS Code 打开（子菜单）；
  - 场景文件：自动 driver 合并失败 → ④b → 「在 UnityYAMLMerge 中解决」→ 合不了的部分自动弹出 BC；
  - 把全局配置里的 driver 临时注释掉 → 红色横幅；在应用里点 merge → 弹出确认框。

## 8. 留给实现者

- 后端 API 的形状：新增几个命令，`StatusData` / `DiffData` 怎么携带冲突信息（只新增字段，不改已有字段）。
- 临时目录的位置、残留怎么清理、临时文件是否只读。
- 标题的具体文案、分支名和提交名怎么取。
- 「编辑外部工具配置」入口放在哪里。
- driver 检查的缓存策略。
- 冲突标记检测的二进制判定细节，以及读多大范围的文件。

## 9. 对 GitHub issue 的影响

- **第 64 条**：正文按本设计更新。第 63 条下面的评论说「不在应用里计算冲突 diff」，已经被 §3 的实测推翻，要在第 63 条补一条评论说明。
- **第 50 条**：冲突标记的判定移到本设计（§4.1），第 50 条的「Stage 前检查」复用它。Conflicts 分区和可点击的横幅仍留在第 50 条。
- **driver 检查**：用户要求这次直接做完，放进第 64 条，不另开 issue。
- **第 51 条**：DU / UD 的二选一移到本设计（§4.1 ③）。第 51 条剩下 Continue / Abort、二进制冲突的「用我的 / 用对方的」，以及 ⑤ 结构冲突的处理。

## 10. 分阶段（Codex review 建议，全部属于本次交付）

每个阶段都能单独验收，按顺序做：

| 阶段 | 内容 | 依赖 |
|---|---|---|
| 1 外部工具基础 | JSON 配置、`rules`、按字节导出、BC / VS Code 启动、锁和清理、完成事件和刷新代次；先做**普通文件的外部 diff** | 无 |
| 2 冲突显示与外部 merge | 后端冲突信息（stage / mode / 属性 / 二进制）、五种情况的判定和显示、冲突标记判定、② 的 diff、外部 merge、标题 | 1 |
| 3 DU / UD 二选一 | literal pathspec、执行前复核、删除确认 | 2 |
| 4 driver 检查 | `check-attr` / `config` 检查、三种横幅、各合并入口之前的确认 | 2 共用属性查询，可以和 3 并行 |

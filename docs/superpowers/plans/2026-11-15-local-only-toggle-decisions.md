# Local-Only 标记可取消、可拆分（2026-11-15）设计与实现日志

输入：2026-11-11 使用报告 9.2。「add @avatarclass」整个标成了 Local-Only（66 个文件），其中 ImpDatabase.lua 那一处要上服务器；结果提交时被钩子拦、跟进时报泄漏、分支被标成落后。标记是 commit message 里的 `Local-Only: true`，原来只能在 Amend 最新一个 commit 时改。报告要求「先出设计：与跟进 journal、其他分支、重放 memo 的关系都要说清」。没有单独的计划文件，设计写在本文件。

## 进度总账

| Task | 状态 | 产出 | 验证 |
|---|---|---|---|
| T1 后端：改写引擎（取消 / 加上标记、拆分） | 完成 | `engine/svn_local_only_edit.rs`；`svn_replay.rs` 开放 `tree_without`、`changed_paths`，从 `commit_like` 拆出 `write_commit` | `tests/svn_local_only_edit.rs` 18 条（含工作区 / 索引不动、作者原样、别的分支不动、目录换文件、各类拒绝）；模块内单元测试 6 条 |
| T2 命令 `svn_set_local_only`、`svn_split_local_only`、`svn_local_only_files` | 完成 | `commands/svn_sync.rs`、`lib.rs` | `svn_write_guard` 9 条；真实窗口里正反例都验过参数名 |
| T3 前端：graph 右键菜单三项、确认框、拆分弹框 | 完成 | `engine/localOnlyEdit.ts`、`engine/commitActions.ts`、`store/repoStore.ts`、`components/SplitLocalOnlyDialog.tsx`、`RepoTab.tsx`；布局场景 `split-local-only-long` | `localOnlyEdit.test.ts` 11 条、`SplitLocalOnlyDialog.test.tsx` 6 条；`capabilities.test.ts` 的 SVN 菜单清单加一项；`pnpm test:layout` 176 条（review 之前跑的） |
| T4 真实窗口验证 | 完成 | — | 见「真实窗口验证」 |
| 独立 review（`/code-review high`） | 10 条：修 9 条、1 条已在 review 期间修掉 | 见「review 处理」 | 新增测试修之前红或编译不过 |
| Rust 全量测试 | 完成 | — | 集成 95 个套件 1869 条通过、0 失败；lib 150 / 151（已知不稳定的 watch 用例），见「Rust 全量测试」 |

## Rust 全量测试（2026-11-16，开发机）

- 本机内存紧（空闲 2.5–7 GB / 31 GB，大头不在普通进程里），后台测试被 Claude Code 按内存压力回收了 4 次；改成逐个套件跑、结果记日志、断点续跑。
- lib 单元测试 151 条里 150 条通过。`commands::watch::tests::stage_and_unstage_only_touch_the_index` 连续失败（多收到一条 `.git/hooks` 目录事件），单独跑通过。本分支没改 `watch.rs`、`vcs/`；这条在 2026-11-08、11-11（未改动的基线 6883b03 上也失败）、11-12、11-14 的实现日志里都记过同样的现象，判断为本机负载下的已知不稳定测试，未修。
- 集成测试：95 个套件全部通过，1869 条通过、0 失败、1 条 ignored（`merge_drivers` 里原本就标了 ignore）。前 54 个单线程、后 41 个 `--test-threads=4`，都是 `-j 1`。PR 先于全量测试开出（用户要求），结果随后补进本节和 PR 描述。

## 设计

### 关键观察：分支末端的树不变

- **取消 / 加上标记**只改 commit C 的说明，C 的树不变。
- **拆分**把 C 换成两步：C1 = C^ 加上选中的文件（普通 commit），C2 = C 的树（仍是 Local-Only）。C2 的树就是 C 的树。

所以 C 之后的每个 commit D 都可以**原样搬树**、只换父提交：新 D 的树 = 旧 D 的树。不需要 merge-tree、不会冲突；分支末端的树与改写前逐字节相同，**工作区、暂存区一个字节都不用动**，最后对分支 ref 做一次 CAS 移动即可。

### 方案取舍

| 方案 | 做法 | 结论 |
|---|---|---|
| A 对象层搬树，CAS 移动分支 | `commit-tree` 逐个重建 C..HEAD，树取原样，作者与说明取原样（只改 C 的说明），`update-ref <branch> <new> <old>` | **采用**。不碰磁盘、索引；失败时没有任何 ref 动过，只多了几个没人指着的对象 |
| B 交互式 rebase（`rebase -i` 的 reword / edit） | 用 git CLI 的 rebase | 不做。要写工作区、会停在中途、Windows 上 sequence editor 难驱动；而这里根本不需要合并 |
| C 只改最新一个 commit（现有 Amend） | 不加新入口 | 不做。报告的例子是一个早就提交、后面还压着别的 commit 的 Local-Only commit |

### 前置条件（任何一条不满足都拒绝，什么都不改）

| 条件 | 原因 |
|---|---|
| 持同步锁；没有未完成的 SVN 同步（journal） | 跟进、更新、交付进行中时分支随时会被它们移动；跟进的崩溃恢复按 journal 里记的 OID 走 |
| git 不在 merge / rebase / cherry-pick 中途 | 同 `check_git_idle` 的其他调用方 |
| HEAD 在分支上（不是游离、不是 `svn/base`） | 改写的是「当前分支」 |
| C 是 HEAD 的祖先，且**不在** `svn/base` 的历史里 | 基线里的 commit 改了说明也没用：基线是按 SVN 录的；改写会让分支脱离基线 |
| C..HEAD 线性（每个 commit 只有一个父提交） | 搬树只对单父链成立；有 merge 时要重建合并关系，不在范围 |
| 取消 / 加上：C 现在的状态与要的相反 | 已经是那个状态时报明白，不生成一模一样内容的新 commit |
| 拆分：C 是 Local-Only；选中的文件非空、都是 C 改过的、不是全部 | 全部选中等于取消标记，引导用户用那一项 |

### 与其他部分的关系（报告要求说清的）

| 部分 | 关系 |
|---|---|
| 跟进 journal | 有 journal 就拒绝（上表）；改写本身不写 journal：只有一步 CAS，要么移动了要么没移动，没有中间态要恢复 |
| 其他分支 | **不动**。别的分支上也有 C 时，它们留着旧的 C（仍是改写前的标记）。结果里列出这些分支，界面提示 |
| 失焦快照（`follow-mark.json`） | 快照里记的 HEAD 是旧 OID，改写后的 commit 不是它的祖先，下一次跟进把它们当「拿不准」，走正常重放（保守分支）。不改快照：只是少用一条线索，不会错 |
| 重放的 memo、快照规则 | 都是一次跟进内部的缓存，按 OID 现算，不跨跟进保存 |
| 待放回清单（`restore-pending.json`） | 记的是路径；放回条件每次现场重验（svn/base..HEAD 里改过它的 commit 是否全是 Local-Only），改写后照常成立 |
| 交付 | 交付时现场读 `svn/base..HEAD` 摘 Local-Only，改写后自然按新标记走 |
| 钩子的泄漏检查 | 按当前分支现场判定，改写后立刻生效 |

### 新 commit 的元数据

- 作者（名字、邮箱、时间）取原 commit；提交者是当前用户、当前时间（与 `git rebase` 一致）。
- 拆分：C1 的说明是 C 的说明去掉 `Local-Only: true`，C2 的说明是 C 原样。两者标题相同：交付时 C1 的说明会进 SVN，不往里加「（拆分）」之类的字样。
- 加上标记时按提交框的规矩单独成段（`buildCommitMessage`）；取消时只删那一行，删完末尾留下的空行一并去掉。

### 恢复

改写前的分支末端在 reflog 里（`mygitclient: ...`），`git reset --keep <旧 OID>` 即可回去；树相同，`--keep` 不会碰工作区的改动。

## 决策条目

### T1 后端

- **为什么这么改是安全的**：新 commit 的树全部取自原 commit（拆分的 C1 例外，它是 C 去掉没选中的文件，用的是手动跟进早就在用的 `tree_without` / `c_prime_tree`），末端的树与改写前逐字节相同，所以不用碰工作区和索引。唯一的写操作是对当前分支的 `update-ref <新> <旧>`：读完 HEAD 之后分支被别人移动了就失败、什么都不改（`local_only_edit_branch_moved`）。前面任何一步失败都只是留下几个没人指着的对象。测试「工作区与索引不动」放了一处已暂存、一处没暂存的改动和一个没跟踪的文件，改写前后 `status --porcelain` 与 `diff --cached` 逐字相同。
- **计划没写的坑**：`commits_between` 只收完整 OID，传 `<oid>~1` 会报「不是一个完整 OID」，先 `rev-parse`。
- **计划没写的坑**：拆分要列「这个 commit 改了哪些文件」，前端原来只能读整份 diff。新增只读命令 `svn_local_only_files`，与后端校验选中项用同一个函数（`--no-renames`），弹框列的就是后端认的。改名在两边都是删 + 加两条。
- **线性判定**：`target^..HEAD` 第一个必须是 target 本身、每个都只有一个父提交、首尾相接。target 之前有从别处合进来的 commit 时这一段会多列，同样判成非线性。

### T3 前端

- **偏离报告的措辞**：报告写「把选中的文件拆成普通 commit」。弹框里勾的是**要上服务器的**文件：报告的情形是 66 个文件里只有一个要交付，勾一个比勾 65 个合理。按钮写「拆出 N 个文件」。
- **只在 SVN 混合仓库、本地提交上出现**：SVN 节点、基线 commit（单值与混合）都不出现这几项；是不是在当前分支上、是不是已在基线里，后端再查一遍（菜单拿不到这些信息）。
- **改写完成后按新的末端重画图并选中它**：旧的选中项已经不在图上了。别的分支上也有这个 commit 时弹一条「别的分支没有跟着改」。

## review 处理（`/code-review high`，2026-11-15）

review 只读代码，没有跑。10 条：

| 条 | 问题 | 处理 |
|---|---|---|
| 1 | CAS 移动分支之后才列别的分支，那一步失败就把一次已经完成的改写报成错误；用户以为没改成、再点一次 | **修**。CAS 之后的步骤失败只记日志，结果照常返回（别的分支列表为空） |
| 2 | 拆分成功之后的刷新是 `void` 出去的，失败成了没人接的 rejection | **修**。`.catch(reportError)` |
| 3 | commit 在同一个位置把目录换成同名文件（或反过来）时拆分报内部错误：临时索引放不下目录与文件，换条目的逻辑也只认文件 | **修**。这种 commit 一律拒绝拆分，写明原因并引导用「取消仅本地」。集成测试三种勾法都拒绝、分支没动 |
| 4 | 改写后图跳到分支末端，改的是下面第 30 个时看不到结果 | **修**。结果里带上改写后的那个 commit 的 OID（拆分时是留下的 Local-Only 那个），图选中它。真实窗口里核对过选中项 |
| 5 | 每个后续 commit 起 3 个进程（`rev-parse ^{tree}`、`log -1`、`commit-tree`），几百个时持锁几十秒 | **修**。一次 `git log` 读出整条链的树与作者，之后每个 commit 只有一次 `commit-tree`。为此从 `svn_replay::commit_like` 拆出 `write_commit`，交付、跟进走的还是原来的 `commit_like`，行为不变（相关测试套件全量重跑，见下） |
| 6 | 只改了一个文件的 commit 勾上之后同一句提示出现两次 | **修**。组件测试钉住只有一行 |
| 7 | 去掉标记时逐行删，标记在中间会留下两个连续空行，交付时原样进 SVN | **修**。按段处理，删空的段整段去掉；CRLF 的说明出来是 LF。单元测试 2 条 |
| 8 | `is_ancestor`、`changed_paths` 各抄了一份 | **修**。用 `git_read::is_ancestor` 和 `svn_replay::changed_paths`（改成 `pub(crate)`） |
| 9 | 实现日志没更新 | review 开始之后就已更新（即本文件） |
| 10 | 新函数插在了 `handleCommitAction` 的文档注释和函数之间 | **修**。挪到注释上面 |

## 真实窗口验证（2026-11-15，开发机）

- 隔离：`pnpm tauri dev --config <json>`，identifier `com.momadacoding.jkk.e2elocalonly`，vite 5194、CDP 9233；核对过窗口属于本 worktree 的 `target\debug\app.exe`。fixture 由 `scripts/e2e-svn-fixture.ps1` 造在 `%TEMP%`，初始化成混合仓库后用 git 造出「普通 A ← Local-Only（改 A.cs、D.cs）← 普通 B」。
- **参数名**：三个命令正确的形状都通过；`svn_local_only_files` 写成 `{ oid }`、`svn_set_local_only` 写成 `{ enabled }`、`svn_split_local_only` 写成 `{ files }` 都被 Tauri 拒绝（`missing required key ...`）。
- **拆分**（右键那一行 → 拆出部分文件上服务器… → 勾 `Code/Deep/D.cs` → 拆出 1 个文件）：历史变成 A ← debug patch（普通，只改 D.cs）← debug patch（Local-Only，只改 A.cs）← B；`git status` 干净；弹框关掉。
- **取消**（右键剩下那个 Local-Only → 取消「仅本地」… → 确认）：标记没了，图上的「仅本地」角标消失；reflog 里有 `mygitclient: 拆分仅本地 …`、`mygitclient: 取消仅本地 …` 两条。
- **review 修完之后重走一遍**（同一个 fixture 上再造一个改两个文件的 Local-Only commit，后面压一个普通 commit）：拆分、取消都照样成功、工作区干净；两次之后图上选中的都是改写后的那个 commit（`HEAD~1`），不是分支末端。
- **没验到的**：「别的分支没有跟着改」的提示（fixture 里只有一条工作分支，集成测试覆盖了后端返回的分支清单）；改写之后紧接着跟进（失焦快照不再认得改写后的 commit，按设计走保守分支，没有实跑）。

# 手动跟进收尾核对磁盘 + 分类冲突文案（2026-11-06）决策日志

没有对应的计划文件。任务来源是 2026-11-05 SVN 跟进事故分析（不在仓库里）的两条待修项：

- **6.9**：手动跟进会写磁盘，但结束时不核对重放结果。11-05 那次 `MetaLeagueDbsApp.cpp` 被合出两份同名函数、写到了磁盘，`sim_lowmem.lua`（SVN 已改名）被重新加回提交，界面照样报「完成」，stash 备份也被删掉——而那份备份可能是正确内容唯一的副本。
- **6.3**：`svn_baseline_classification_conflict` 的文案把原因归给「录制过程中被外部改动，请重试」。PR 116 之后它剩下的主要触发是不带历史的 replaced，重试也没用。

预计 4 个 Task，按 AGENTS.md §2.1 落这份日志。

## 进度总账

| Task | 内容 | 状态 | 产出 | 验证 |
|---|---|---|---|---|
| 1 | 收尾核对（引擎）：跟进前后磁盘比对、被带回的 SVN 已删文件 | 完成 | `engine/svn_follow_audit.rs` | 见 Task 2 |
| 2 | 接进手动跟进收尾：有发现时不报完成、保留备份，新结局 `followDrift` | 完成 | `svn_sync_in.rs`（`restore_step_with`、`finish_forward`、`SyncInOutcome::FollowDrift`）、`tests/svn_follow.rs` 2 条 | `cargo test --test svn_follow` 136 过；两条新测试做过变异验证（关掉核对后都红） |
| 3 | 前端：结局类型与文案 | 完成 | `svnSyncIn.ts`（`followDrift` 类型、`describeFollowDrift`）、`svnFollow.ts`、`svnFollow.test.ts` 5 条 | `pnpm build`；`pnpm test` 72 个文件 1182 条 |
| 4 | 6.3 文案 | 完成 | `svn_init.rs`、`svnInit.ts`、`tests/svn_init.rs` 1 条、`svnInit.test.ts` 1 条 | Rust 测试先红后绿；vitest 见 Task 3 |

## 方案

### 「跟进前的磁盘」从哪来

手动跟进的流程是 stash → rebase → 放回改动。stash 用的是 `git stash push -u -- <白名单>`（`svn_sync_stash.rs`），所以：

- 已跟踪文件跟进前的内容 = stash 提交的树（W）
- 未跟踪文件跟进前的内容 = `stash^3` 的树（U）
- 没有 stash（白名单内没有任何改动）时 = 原分支 HEAD（`origHead`）

### 为什么「磁盘应该不变」是对的

混合仓库里磁盘就是 SVN 工作副本：SVN 带来的新内容、你提交进 git 的改动、没提交的改动，跟进前都已经在磁盘上了。手动跟进只是让 git 的历史追上磁盘，正确的结果是磁盘一个字节都不变。rebase 先按新 tip 写一遍磁盘，再由 stash 放回改动，两步合起来与跟进前不同，就是分析文档 R19 / W14 说的「重放干净，却造出与磁盘不同的内容」。

例外是你自己在 rebase 冲突时的处理：解决出来的内容可能有意与跟进前不同。所以这是**报警**，不是拦截。

### 两项核对

1. **磁盘变了**：`git diff --no-renames --name-status <W 或 origHead>` 列出与跟进前不同的已跟踪文件；状态为 A（跟进前不是已跟踪文件）的，再拿 U 里的同名 blob 与磁盘内容的 hash 比，相同就不算。
2. **被带回的 SVN 已删文件**：重放后的分支里有、新基线里没有，但旧基线或两者之间某个基线里有过的文件。即 SVN 在这段时间里删掉或改名了它，旧快照又把它带了回来。

> 以上是动手前的方案，两项都改过，见下面 Task 1 的「偏离方案」与独立 review 一节。实际实现：
> 1. `git diff --name-only <W 或 origHead>` 只取候选路径，每个候选逐个按 blob 比（跟进前取 W，没有再取 `stash^3`；跟进后取磁盘按过滤器算的 hash）。跟进前等于原分支 HEAD、跟进后等于新基线的不算；符号链接、子模块不判。
> 2. 重放后的分支里有、新基线里没有，且「磁盘上没有」或「分支自己的旧基线里有」。

### 有发现时

- 不报 `done`，报新结局 `followDrift`，列出两类文件
- 有 stash 时不 drop，留在 stash 列表里（拆掉钉子，同 `Settled`）
- 暂存没保住（`stage_lost`）时照旧保留，并把这件事一并告诉用户

## 决策条目

### Task 1 —— 收尾核对

- **偏离方案（被带回的文件）**：原方案是「旧基线或中间某个基线里有过」。fixture 里没有中间基线时这条会漏报（快照提交引入的文件旧基线里本来就没有），改成「重放后的分支里有、新基线里没有、磁盘上也没有」。代价：你自己在 git 里新建、后来从磁盘删掉但没提交删除的文件也会被列出。这种文件「本地改动」里本来就显示为删除，列出来不算误导。
- **计划没写的坑（stale disk）**：第一版按「磁盘应该一字不变」比，`svn_follow` 里 4 条「分支落后、磁盘上是旧基线内容」的现有用例全部误报。那种场景下手动跟进**本来就该**把新基线写回磁盘。加了例外：跟进前等于原分支 HEAD、跟进后等于新基线的文件不报。合错的内容两头都不等，不受影响（测试 1 仍红 / 绿正确）。
- **计划没写的坑（未跟踪文件）**：stash 用 `-u`，跟进前的未跟踪文件在 `stash^3` 里。同事新加的文件跟进前是未跟踪、跟进后进了新基线变成已跟踪，`git diff <stash>` 会把它列成新增，要拿 `stash^3` 里的 blob 比掉。测试 1 带了这个形状。
- **为什么是安全的**：核对只读（`git diff` / `ls-tree` / `hash-object`）。hash 磁盘文件走与 `git add` 相同的过滤器（不带 `--no-filters`），与 `svn_sync_stash.rs` 的 `is_same_file_everywhere` 同一个口径，CRLF 工作区不会误报。

### Task 2 —— 接进收尾

- **为什么是安全的**：核对放在「改动干净放回、`stashApplied` 落盘之后，drop 之前」。核对失败（git 报错）时 journal 停在 `stashApplied=true`，「继续」走已有的「已放回」分支，按 `Kept` 收尾、不 drop，备份不会因为核对没跑完而丢。有发现时同 `Settled`：拆钉子、留在 stash 列表里。
- **范围**：只在手动跟进的往前走（`finish_forward`）里核对。更新（`sync_in`）自己跑了 `svn update`，磁盘本来就该变；放弃（往回走）不核对。冲突后用户处理完再「继续」的那条路（`stashApplied` 已为真）不核对：它本来就按 `Settled` 收尾、留备份并提醒用户核对，而且磁盘经过了用户之手，比不出有意义的结果。
- **测试场景的坑**：
  - `file://` 下所有提交的作者都是系统用户，同事的提交不改 revprop 的话会被当成「我自己的」，F2 ① 直接取 SVN 版本，走不到手动跟进。新增 `colleague()` helper 统一改作者。
  - 只有一个文件在 SVN 上一加一删时净变化为零，`follow` 报「没有要跟进的」，要再给同事一笔别的改动。
  - 「合错」用 `cat %B >> %A` 的 merge driver 模拟，**在对象层跟进报冲突之后才装上**：先装的话对象层也会用它合，见下一条。
- **计划没写的坑（对象层，未修，记为跟进事项）**：横幅上的普通「跟进」（对象层重放，只移 HEAD 不写磁盘）遇到同样的合错**也报 `Done`**：HEAD 停在合错的 tip 上，「本地改动」里多出一条反向修改。只有 Old.cs 被带回的情况同样直接报 `Done`。`stale_disk_paths` 只管「分支落后、磁盘是旧内容」那一种，管不到这类。磁盘本身没被写坏（对象层不写磁盘），用户提交时会把「改回磁盘内容」一并提交，不丢数据，但历史里会留下错误的中间提交。这正是事故分析 6.10（重放时快照内容按基线走）要解决的，不在本 PR。

### 独立 review（第 1 轮）

opus subagent，读代码 + git 源码（会话隔离不让它在临时目录跑 git，没实测）。没有阻断级问题，2 条中危、8 条低危，处理如下：

| 发现 | 处理 |
|---|---|
| 中危：没有 stash 时核对出错会卡死（journal 停在 rebase，「继续」再核对、再出错） | **偏离方案**：核对不再返回错误。出错时记下原因、留备份，报 `followDrift` 并带 `auditError`。没有 stash 时先删 journal 再核对 |
| 中危：磁盘本来就是旧内容时，被带回的文件跟进前后都在磁盘上，漏报 | 被带回的判据加一支：分支自己的旧基线（`baselineOidBefore`）里有它。新增 `tests/svn_follow_audit.rs` 钉住，做过变异验证 |
| 测试 1 覆盖了 `.git/info/attributes`，冲掉了初始化钉的换行规则 | 改成追加 |
| `hash-object` 多参数：输出行数没校验、命令行长度、进程数 | 改用 `--stdin-paths` 并校验行数，与 `stale_disk_paths` 同一口径 |
| 符号链接、子模块会误报 | 树里不是 `100644` / `100755` 的、磁盘上是符号链接的，不判 |
| `git checkout <before> -- <文件>`：跟进前不存在的文件跑不通，还会顺手暂存 | 改成 `git show` 查看、`git restore --source=… --worktree` 写回；有 stash 时说明未跟踪文件用 `^3` |
| `baseline_rev` 在收尾之后失败会丢掉核对结果 | 手动跟进时先读 `new_rev` 再动手 |
| 决策日志「方案」与实现不一致 | 补了上面的说明 |
| 缺「没有 stash 报 FollowDrift」「StageKept + 有发现」「核对出错」的端到端测试 | **未补**：前两种要搭「分支落后 + 合错」的组合现场，第三种要让 git 在核对时稳定出错，fixture 里都很绕。引擎判据由 `tests/svn_follow_audit.rs` 覆盖，结局映射与文案由 vitest 覆盖；端到端只有两条主形状 |
| skip-worktree 文件有本地改动、新基线又改了它时可能误报 | 未处理，概率低，记为已知限制 |

### 独立 review（第 2 轮）

同一个 reviewer 复核第 1 轮的处理。没有阻断或中危问题。已确认：核对出错的所有路径都不 drop 备份、不报 `Done`；`baselineOidBefore` 在唯一入口（`svn_follow.rs` 的 `manual_follow`）是 merge-base 出来的分支自己的基线；跳过 special 路径只会漏掉「类型变过」的路径。处理的低危：

- 文案里的 `<oid>^3` 在 cmd.exe 下 `^` 会被当成转义符：所有带 OID 的命令都加了引号
- 被带回、但磁盘上还在的文件，「删掉并提交」会连 SVN 工作副本里那份一起删：改成建议 `git rm --cached`
- 跟进后新多出来的文件没有旧版本可取：文案注明
- `FollowDrift.before` 的 Rust 注释还写着 `git checkout`：改掉

**已知漏报（未处理）**：文件只在中间某个基线里出现过（快照新加、SVN 之后又删），而磁盘本来就是旧内容时，「被带回」两条判据都不命中。概率低，6.10 的「快照内容按基线走」会从源头处理这类问题。

### 环境

- 本机全量 `cargo test` 并行链接几十个测试二进制时报 `The paging file is too small for this operation to complete (os error 1455)`（内存加页面文件被耗尽），所有测试目标编译失败。不是代码问题，`-j 2` 重跑。之前还出现过一次 `only metadata stub found for rlib dependency core`，重试即恢复，推测是同一原因。

### Task 3 —— 前端

- **为什么是安全的**：新结局只加不改，`describeSyncOutcome`（更新流程）也补了这一支，照实说，不吞掉；组件里只有 `svnSyncFlow.ts` 一处按 kind 分支，判断的是 `FollowOutcome`，不受影响。没改命令签名（入参不变，只是返回的枚举多了一个变体），所以没跑 `pnpm tauri dev`。
- 取回命令里给完整 OID：用户要照抄（同 `stageLostDetail`）。

### Task 4 —— 6.3

- **偏离计划**：没有加新的错误码，只改文案。PR 116 之后走到 `svn_baseline_classification_conflict` 的主要是不带历史的替换，原因能直接说出来；真正的竞态少见，放在第二句。
- **测试顺序**：Rust 那条先红后绿；前端提示的断言是改完文案之后补的，但旧文案里明确有「请重试一次」，`not.toContain` 在旧代码上必红，没另做变异。

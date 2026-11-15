# 部分自动跟进（E）：实现日志

计划：`2026-11-13-partial-auto-follow.md`

## 进度总账

| Task | 状态 | 产出 | 验证 |
|---|---|---|---|
| T1 | 完成 | `svn_follow.rs`：`prepare(.., partial)`、`Partial`、`partial_record`、`tree_diff`；`Plan.rec` | `svn_follow_auto.rs` 末尾 8 条新测试 |
| T2 | 完成 | `prepare_auto`、`held_candidates`、`HeldBack` / `FollowReport.held_back` | 同上；关掉部分跟进分支时 4 条主路径测试变红，去掉墓碑补记时墓碑测试变红；`svn_backend.rs` 钉住 `heldBack` 的 JSON 形状与旧 journal |
| T3 | 完成 | `svnFollow.ts`：`HeldBack` 类型、「留在旧基线上的文件」一节（attention）、摘要「N 个文件留在旧基线上」 | 2 条 vitest 先红后绿；`pnpm build`、`pnpm test` 1630 过 |
| T4 | 完成 | E2E `2026-11-07-svn-auto-follow-e2e.md` 新增场景 8 | 2026-11-14 真实窗口（本 worktree 的 app.exe，identifier 隔离，CDP 派发 focus / blur）：提示「跟进了 1 个文件，1 个文件留在旧基线上」、要人看；报告「留在旧基线上的文件」列 D 与原因，「跟进的文件」只有 A；横幅只剩 D、原因行与整次拦时相同；磁盘不动，`svn/base` 里 A 是新的、D 是旧的，基线只发布一次；之后 10 秒 0 次 IPC，没有反复自动跟进 |
| T5 | 完成 | 独立 review 1 高 1 中 3 低；高 1、中 2、低 3 已修，低 4、低 5 记为取舍 | 墓碑测试加第二轮，换回第一版补法时变红 |
| 后续 1（2026-11-14，另一个 PR） | 完成 | `svnFollow.ts`：`heldBackScene`、`RECHECK_SKIPS`；`svnSyncFlow.ts`：部分跟进后记现场、SVN 冲突时放掉现场、做完叫醒一次；`RepoTab.tsx` 接线。T4 那一行里「原因行与整次拦时相同」从此不成立：原因行改成留下的文件与原因 | vitest 先红后绿（第一轮 7 条、review 中 1 另 3 条）；`pnpm build`、`pnpm test` 过；真实窗口场景 8、9，见「后续」一节 |

全量验证（2026-11-14）：`cargo fmt --check`、`cargo clippy --all-targets` 0 告警、`cargo test --no-fail-fast -j 2` 全过（95 组、1996 条）；`pnpm build`、`pnpm test` 1630 过。

## 决策条目

### 总体

- **偏离使用报告：只做自动跟进，按钮不变。** 报告没区分两条入口。按钮是用户显式要整次跟进，冲突要走手动跟进出口；部分跟进会把「冲突文件」静默留在旧基线上，用户点了按钮却看不到冲突框。
- **偏离使用报告：只排除外部「修改」的文件。** 报告把改名拆半、删目录列为要用 fixture 覆盖的风险。与其分别处理，不如让新增、删除的文件根本不进排除集合，出现在拦截原因里就整次拦。代价：外部新增的文件有问题时仍然整次拦，比报告的效果差一些；这类文件在使用场景里少见（新增文件很少与分支提交重叠）。
- **为什么安全：** 发布、journal、崩溃恢复一行不改；部分基线只是另一个合法的映射（与「SVN 只更新了一部分」同构），被排除的文件在下一次检测里照旧是 ExternallyAdvanced。

### T1

- **为什么锁内复核不用改：** `Plan.recorded` 只给 `still_valid` 的 R3c 用，它按 `tree_files(new_base)` 与重读的节点状态重算映射。部分跟进只留「修改」的文件，部分基线的文件集合与完整录制相同，重算出来的就是完整映射，所以 `recorded` 存完整映射、`still_valid` 一行不改。
- **计划没写的坑：floor 降下来会丢墓碑。** 根目录 update 之后所有节点都在 HEAD，完整录制的 floor 是 HEAD；留下的文件记旧 revision，`compute_map` 算出的 floor 随之降到它。完整录制只为原 floor 体检过，只给删除 revision 高于原 floor 的文件发墓碑；新 floor 与原 floor 之间删掉的文件在新 floor 上服务器还有，服务器文件集合的审计（每五分钟一次）会把它们报成漏收。
  - 第一版补法（已撤）：`old_base → 完整树` 里状态为 D 的补记墓碑 = 原 floor。独立 review 高 1、低 3 指出本地推不全：旧基线自带的墓碑（完整录制不继承，连续两次部分跟进时必丢）、旧基线之后加进来又删掉的文件，都不是 D。
  - 现在的做法：floor 降下来时，按部分基线再体检一次（`svn_init::confirm_tombstones`，`JustRecorded`，`explained` = 完整录制的 `S` + 被留下的文件），由服务器确认墓碑，与录制同一套规则。代价是这种情况多一次联网 `svn diff`；floor 没降时不跑。体检没过（撒谎、漏收、确认不成、没核对成）就放弃部分跟进，整次拦。为此 `Recorded` 多存了录制时的 `S`（`explained`）。
  - 测试 `a_file_deleted_in_the_same_follow_gets_a_tombstone_when_the_floor_drops` 加了第二轮（连续两次部分跟进），换回第一版补法时第二轮变红。
- **为什么选「降 floor」而不是「floor 不动、单列一个低于 floor 的 revision」：** 后者违反「floor 是映射里的最小值」（`svn_basemap` 模块头），体检、交付、提交图都按这个读；前者只是 `compute_map` 的正常结果，与工作副本里真有一个文件停在旧 revision 时完全同形。
- **复用第一遍的录制：** 第二遍不再跑 `record_from_old_base`。两遍准备包在同一个 `with_scan_cache` 里，`svn status` 也不重扫。偏离计划「第二遍不再联网体检」：floor 降下来时要为墓碑体检一次（见上一条）。

### T2

- **第二遍还被拦时返回第一遍的原因。** 第二遍的原因可能只是排除之后的连带结果，用户看不懂；第一遍的才是他要处理的。
- **候选必须是这次的外部修改：** 用 `diff-tree --raw old_base new_base` 判，状态 M 且 mode 不变。只改 mode 的不算（留旧 blob 会配上新 mode）。
- **偏离计划与使用报告：「磁盘上还是分支的旧内容」不按文件留。** 计划表里它是按文件排除的一条。独立 review 中 2 指出：`stale_disk_paths` 看的是分支落后的那一段（`scope..old_base`），与这次的外部变化无关，留下之后第二遍照样报，只是白跑一遍。现在 `held_candidates` 见到 stale_disk 直接返回 `None`，整次拦，文案不变。
- **计划的错误：改名场景里拦下它的不是冲突。** 计划预期分支改了 B、同事把 B 改名为 B2 时重放冲突。实测 git 的 merge-tree 认出了改名，把分支的改动带到 B2 上，拦下它的是「B2 与磁盘对不上」。B2 是新增的，留不住，照样整次不做；测试按实测改了期望的 code。
- **单文件的老用例都没改：** 条件 7、8、11 的老用例里有问题的就是唯一的外部变化，留下之后什么都不剩，照原来整次拦，原因文案不变。

### 独立 review（opus subagent，按 AGENTS.md §13）

1 高、1 中、3 低。高 1（旧墓碑丢失）与低 3（floor 区间内新增又删除的文件缺墓碑）合并修成「floor 降下来时按部分基线再体检」，中 2 改成 stale_disk 整次拦，见上。没改的：

- **低 4：只做一轮。** 第一遍是冲突时不算 overlap 与 leaks，「X 冲突 + Y 与磁盘对不上」第二遍才看见 Y，整次拦、横幅只写 X。照计划「按新基线再判断一次；仍然被拦就整次不做」，不加第二轮：多一轮就多一次重放与可能的联网体检，收益只在两类问题同时出现时。
- **低 5：单列条目越积越多。** 被留下的文件把 floor 钉在旧 revision，其余文件按 `commit_rev > floor` 单列；超过 5000 条时 `compute_map` 报 `svn_map_too_large`，第二遍失败，退回整次拦（日志 info）。方向保守，记为遗留风险。

### T3

- **「留在旧基线上的文件」是 attention：** 它们的新内容没跟，横幅会继续挂，自动跟进的提示因此是要人看的那种（与 PR 163 的「只收安静的提示」配合：这份报告不会被下一次跟进收掉）。

### 后续（2026-11-14）：部分跟进之后不再白跑一遍，SVN 冲突解决后能再自动试

分支 `worktree-auto-follow-retry`，只动前端（`src/engine/svnFollow.ts`、`src/components/svnSyncFlow.ts`、`RepoTab.tsx`）。

- **计划没写的坑：部分跟进做完，现场键跟着变，自动跟进立刻再跑一遍。** 基线推进之后判定只剩留下的那几个文件，
  `autoFollowKey` 与发起时不同，effect 又发起一次 `svn_follow_auto`：重录基线、服务器体检、重放，结论一定还是被同样的
  文件拦下（留下它们的原因一条都没变），白等几秒。改成做完时现读判定与 HEAD，判定里**恰好只有**留下的文件
  （`heldBackScene`：没有新增 / 删除、`changed` 的路径集合与 `heldBack` 相同）就把这个现场直接记成试过，横幅原因取
  报告里的 `heldBack`。集合不相等（又来了新内容、留下的文件已经不在判定里）、读不出现场时不记，由下面那一次叫醒
  照常自动试。
- **计划没写的坑：被 SVN 冲突拦下的现场，Resolve 之后再也不自动。** 现场键只含 HEAD 与判定内容，`svn resolve` 两样
  都不改；`RepoTab` 在发起时就把现场记成试过，解决之后判定重新写回（wc.db 变了）键还是原来那个。改成被
  `svn_preflight_conflicted` 拦下时把这次的现场放掉（`RECHECK_SKIPS`）。
- **计划没写的坑：流程在走时判定写回，effect 那一次被挡掉，之后没人再叫它（独立 review 中 1）。** 跟进后的刷新写回新
  判定，effect 随即运行，这时 `running` 还没放开，`handleAutoFollow` 返回 false、不记现场。原来流程在刷新之后只剩同步
  的收尾，effect 落在 `running` 放开之后，碰巧能发起；第一版在收尾里加了读现场的两次 IPC，effect 就落到了流程里面：
  现场不是「只剩留下的文件」（跟进期间又来了新内容）或者读现场失败时，既没记成试过、也没人再试，新内容不跟、横幅上
  也没有原因。改成不靠时序：自动跟进**做完**（`done`）、`running` 放开之后明确叫醒一次（`nudgeAutoFollow`，
  `RepoTab` 里就是 `autoNudge` 加一）。现场已经记成试过时是空转，整次跟进之后没有现场键也是空转，其余照常自动试。
  被拦下时**不叫醒**：现场没变、`RECHECK_SKIPS` 又刚把记录放掉，叫醒就是原地打转。
- **为什么这么改是安全的。** 三处都只改「要不要发起一次自动跟进」，不改跟进本身：后端的前置检查、录制、体检、
  `auto_blocker` 原样把关。放掉现场不会造成循环：被拦下的那条路径不写 effect 的任何依赖；再试要等判定重新写回
  （wc.db 戳变了、HEAD 动了、强制校验）、切标签、或应用内一次提交结束（`autoNudge`，取消的提交也算）。冲突没解决时
  每次再试是一遍 `svn status`（预检在录制与服务器体检之前就拒绝），外加 `svn_follow_auto` 收尾时照例补发的一次
  `repo-changed`（界面整体刷新一遍，SVN 检测比戳、戳没变就跳过）和「跟进」按钮禁用的那一小会儿。真实窗口实测：
  冲突未解决时再派发 `focus`，9 秒内没有第 2 次 `svn_follow_auto`。
- **遗留风险（review 低 2，没实测）：** 冲突还在、同时有外部 SVN 操作持续写 wc.db 时，「再试 → `repo-changed` →
  比戳发现变了 → 判定写回 → 再试」会跟着那次操作一直转到它结束，每轮一遍 `svn status`。操作一停就停。
- **没改（review 低 1）：`heldBackScene` 只比路径，不比 revision。** 被留下的文件在跟进期间又被更新到更新的 revision 时，
  现场照样记成试过，横幅上的原因是按旧内容判的。窗口很窄：发布前的锁内复核（`still_valid`）按节点状态重算映射，
  对不上就回到 F1，所以它只能发生在发布到刷新之间的不到一秒里；发生了也只是横幅原因偏旧，点「跟进」照常，SVN 侧
  再有变化也照常自动试。
- **没改（review 低 3）：读现场用的是不排队的 `freshSvnSync`。** 跟进后的强制校验刚记过戳，这里通常只是比一次戳；
  跟进期间 wc.db 又变了才会与后台那轮并发一遍扫描，与提交前那一次是同一个取舍。`repoStore.ts` 里「唯一不排队」的
  注释已改。
- **没做：** 只放掉 `svn_preflight_conflicted` 一个原因。同样「原因消失之后现场键不变」的还有断网
  （`svn_auto_follow_not_verified`，网络恢复后不会再试，这是原有行为）和预检的目录写锁
  （`svn_preflight_operation_in_progress`，没实测过它的现场键变不变），不顺手加。
- **独立 review（subagent，按 AGENTS.md §13）：** 0 高、1 中、6 低。中 1 已修（见上，先补了把 effect 接线一起模拟的
  失败用例）；低 4（一条用例测的是走不到的状态）改了用例说明，低 5（本文件的总账）、低 6（横幅那句话开头重复、
  原因之间靠句号分隔）已改；低 1、低 2、低 3 见上。

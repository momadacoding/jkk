# P0 清扫（2026-11-04 ～ 11-05）决策日志

没有对应的计划文件：任务是「修完所有 P0 issue，然后发版」，每条 issue 自带修法与验收，按 AGENTS.md §3 走 TDD 直接修。
跨了多个 PR、多轮会话，按 §2.1 落这份日志。

## 进度总账

| Task | Issue | PR | 状态 | 产出 | 验证 |
|---|---|---|---|---|---|
| 1 | 第 85 条：冲突标记被 Stage | 112 | 已 review（2 轮），待合并 | `src/engine/conflicts.ts`（`describeStageConflicts`）、`LocalChangesView.tsx`（统一 `stage()`）、`vcs/git.rs`（stage / unstage 加 `--literal-pathspecs`） | `pnpm build`、`pnpm test` 1149；cargo fmt / clippy / test 1645 |
| 2 | 新发现：discard 按 glob 匹配，会丢数据 | 114 | 已 review，待合并 | `engine/ops.rs`（四条命令加 `--literal-pathspecs`） | cargo fmt / clippy / test 1646 |
| 3 | 第 72、71、68 条 | 113 | 已 review（2 轮），待合并 | `repoStore.ts`（diff 读失败、`samePath` 守卫、`openedPath`）、`CommitDetailView` / `LocalChangesView`（失败说明）、`trashDirSync.test.ts` | `pnpm build`、`pnpm test` 1146 |
| 4 | 第 69 条：git 专属写命令后端不拦 | 115 | 已 review，按意见修过 | `vcs/mod.rs`（`require_capability`）、`commands/{ops,remote,worktree}.rs`、结构测试 | 见 Task 5 |
| 5 | 第 70 条：基线护栏缺口与恢复路径 | 115 | 已 review，按意见修过 | `vcs/svn.rs`（`create_branch` / `commit` 守卫、`refuse_while_on_baseline`）、`ops::revert`、`svn_init.rs`（`rebuild_or_recover_baseline`）、`describeMissingBaseline` / `BaselineBanner` / store | `pnpm build`、`pnpm test` 1139；cargo 见 PR |
| 6 | 发版 | — | 阻塞：合并需要用户放行 | — | — |

## Task 1 —— 第 85 条

- **计划没写的坑**：`git add -- <path>` 在 `--` 之后**仍按 glob 匹配**，`f[1].txt` 能匹配到 `f1.txt`。前端只看到 `f[1].txt`，于是还留着冲突标记的 `f1.txt` 会绕过确认，被标记为已解决。2026-11-04 在临时 fixture 里实测：`UU f1.txt` 变成 `M  f1.txt`，内容仍以 `<<<<<<<` 开头。由独立 review 发现。
- **偏离 issue**：issue 只要求前端确认，这里另外改了后端 `stage` / `unstage`。不改的话「所有入口都过确认」不成立。
- **为什么是安全的**：`--literal-pathspecs` 只关掉 pathspec magic。review 实测过目录路径、空格与非 ASCII 路径、反斜杠、删除、改名、无提交仓库，行为都不变。
- **接受的风险**：status 已经写回而 conflicts 还没读到的那几毫秒里，旧的 `noMarkers` 分类会放行；确认框开着时，外部新出现的冲突文件会被 Stage All 一起暂存。

## Task 2 —— discard 的 glob

- **计划没写的坑**：同 Task 1 的根因，但在 discard 里是**数据丢失**：`checkout --` 会抹掉没选中文件的改动，`rm -f --` 会删掉没选中的文件。原本不在 P0 列表里，修 85 时发现，单独开 PR。
- **为什么是安全的**：review 逐条实测了 `ls-tree` 在加 flag 前后输出逐字节相同（它本来就不做 glob），`in_head` 分类不会因此误判出新的 `rm -f`。
- **遗留**：前端如果传反斜杠路径，与 git 输出的 `/` 精确比较对不上，已跟踪的文件会被分进 `remove`。前端现在只传 `/`，暂时触发不了，记为潜在隐患。

## Task 3 —— 第 72、71、68 条

- **偏离 issue**：71 的 issue 只点了三个函数，这次另外给 `refreshPinned`、`refreshPullRequests` 补了守卫。它们同样是慢请求，属于同一类问题。
- **计划的错误（我自己的第一版）**：把 `latestVerdict` 放进 `setState` 的 updater 里赋值。React 18 下 updater 可能要到下一次渲染才执行，而 `trackSvnCheck` 在那之前已经把在跑计数归零，失焦快照会拿旧结论去写。独立 review 判为阻断。改成新增 `openedPath` ref，同步核对、同步赋值。
- **计划没写的坑**：`repoStoreFollowFocus.test.ts` 原来只把 `path` 塞给 effect，store 自己的 `state.path` 一直是 `null`。守卫上线后写回全被丢掉，那不是真实形态，所以把 mount 改成真的 `openRepo`。
- **68**：确认框的文案早在第 87 条就改对了，这次只补防漂移测试，并用变异验证过。
- **行为变化**：文件监听那条 diff 刷新失败时，右侧显示「读取改动失败」，不再是 `No changes`，下一次文件事件到来时自动恢复。

## Task 4 —— 第 69 条

- **issue 的待选项**：选了方案 A，即每个命令入口加一行守卫，不把这些命令收进 `trait Vcs`。理由是改动小，而且判据就是前端用的那份 `capabilities()`。
- **偏离 issue**：issue 的表里写的是「`commands/worktree.rs` 全部」，但 `read_worktrees_cmd` **不拦**：`openRepo` 每次打开仓库都会无条件调它，拦了 SVN 仓库就打不开。它在结构测试的放行表里写明了理由。
- **为什么是安全的**：普通 git 仓库的后端声明全部能力，行为不变；还没初始化或来历不明的 SVN 目录，由 `backend_for` 报各自的错误码，与走 trait 的命令一致。结构测试要求 `ops` / `remote` / `worktree` 的每个 Tauri 命令都被归类，用变异验证过。

## Task 5 —— 第 70 条

- **issue 的待用户决定项**：基线丢失后的恢复路径。**由实现者代为选了推荐方案**：「重建基线」在 `svn/base` 不存在时从零录一条。用户此前在会话中明确要求不要停下来等决定（理由见全局 CLAUDE.md），所以没有等待答复。另外两个选项是「基线不存在时放行改名」（但备份分支的内容没经过体检）和「这次不做恢复」，都没选。
- **偏离 issue**：issue 第 2 条（从远程检出 `svn/base`）没有单独加守卫，因为 Task 4 已经让 SVN 混合仓库整个拒掉了 `checkout_remote_branch`。
- **设计**：新增 `rebuild_or_recover_baseline`，只给命令层用。`migrate_eol` 和 `sync_in` 照旧调严格的 `rebuild_baseline`，它们依赖旧基线。恢复时父提交取 HEAD 历史里最近的一条、来自同一个 SVN 仓库的基线 commit，工作分支与新基线仍有共同祖先，后续「更新」的变基才有落脚点；找不到就拒绝（见下面「计划的错误」）。`RebuildReport.old_baseline` 用空串表示从零录（前端不读这个字段）。
- **为什么是安全的**：
  - `read_baseline` 只把「ref 不存在」当成没有，ref 损坏照样报错，不会把一段真实历史当成没有而覆盖掉
  - 发布以「ref 现在不该存在」为条件（`update-ref` 带零 OID），检查之后有人建了 `svn/base` 的话发布失败
  - 预检和体检与平时重建完全相同
  - 恢复路径用变异验证过：把命令改回严格的 `rebuild_baseline` 时测试失败
- **前端**：store 只在 `svn_sync_status` 报 `svn_no_baseline` 时记下 `svnBaselineMissing` 并清掉旧结论，其余失败保留上一次的结论；横幅在「缺失且没有结论」时挂恢复入口，同时不挂常驻体检入口。为了不跟 PR 113 冲突，只改了 `applySvnSync` 的 catch 分支。
- **HEAD 在基线上**：拦 commit（含 amend）和 revert，不拦 checkout，因为查看基线内容是正当操作，同步流程自己也会切过去。普通 git 仓库里叫 `svn/base` 的分支不受影响，有测试覆盖。
- **计划的错误（我自己的第一版，PR 115 独立 review 指出）**：
  - 找不到父基线时，第一版退回成没有父提交的根 commit，`rev-list` 失败也静默降级。根基线一旦发布，之后每次重建都以它为父，工作分支与基线永远没有共同祖先，跟进报 `svn_follow_branch_unrelated`，而护栏又不许删、改基线，用户等于被困死。现在改为：找不到候选或读不出历史时报 `svn_baseline_recover` 并拒绝，不建出 `svn/base`
  - 候选只看「能不能解析出记账」，可能选中别的 SVN 库导入的 git-svn commit；`rev-list` 按提交时间排序，落后分支上更早的基线可能排在前面。现在加了 `--topo-order`，并要求 `repos_root` 与 `uuid` 和本次新录的记账一致（用变异验证过）
  - `svnBaselineMissing` 在判定成功时不清，而 `needsStandaloneCheck` 只看这个标记，重建之后常驻体检入口一直不回来。现在改成「丢了且没有结论」才隐藏；catch 分支只在 `svn_no_baseline` 时置位，别的失败（锁、网络）不再把标记清掉
  - 基线丢失与行尾迁移同时出现时，迁移横幅会把恢复横幅压掉，留下一颗必然报 `svn_no_baseline` 的迁移按钮。现在恢复横幅优先（用变异验证过）
  - 结构测试原来只扫三个文件，并且只认 `pub fn`（会跳过 `pub async fn`）。现在扫描 `commands/` 下除 SVN 自身命令以外的全部文件，按 `fn ` 匹配，放行表支持整个文件放行
- **接受的风险**：从零录基线时，「找不到父基线就拒绝」这一步发生在录制（含服务器体检）**之后**，因为判断要用新录出来的记账里的 `repos_root` / `uuid`。被拒的那次白跑了一遍网络体检，但恢复是罕见操作，可以接受

## 跨 PR 的提醒

- 113 与本 PR 都改了 `repoStore.ts` 的 `applySvnSync`，但改的是不相邻的行（113 改成功分支，本 PR 改 catch 分支）。两边都合并时，后合的那个要按 master 再跑一遍 `pnpm test`
- 没有任何一个 PR 在 `pnpm tauri dev` 的真实窗口里验证过：这几个 PR 都没改命令签名。70 的恢复横幅只在 jsdom 里测过

# 跟进报告可读性与跟进速度：实现日志

计划：[2026-11-16-follow-report-usability.md](2026-11-16-follow-report-usability.md)。本 PR 只做 A。

## 进度总账

| Task | 状态 | 产出 | 验证 |
|---|---|---|---|
| A1 没放回的补丁与放回按钮合成一节 | 完成 | `svnFollow.ts` 的 `pendingRestoreGroups`、`svnLastCommit`；`FollowReportDialog.tsx` 的 `Section` 收 `children`，`RestoreRow` 只剩按钮与结果；`styles.css` 的 `.follow-restore` 去掉边框 | `svnFollow.test.ts` 新增 5 条、改 1 条；`FollowReportDialog.test.tsx` 新增 1 条、改 1 条 |
| A2 「分支与磁盘对不上」三句话 + 提交 | 完成 | `svnFollow.ts` 的 drift 节说明与 `driftItem` | `svnFollow.test.ts` 新增 3 条、改 1 条 |
| A3 泄漏告警标出已从分支移除的提交 | 完成 | `svnFollow.ts` 的 `removedLeakItem`、`REMOVED_LEAKS` | `svnFollow.test.ts` 新增 3 条 |
| A4 自动跟进被挡时的泄漏横幅 | 完成 | `svn_follow.rs` 的 `auto_blocker` 条件 8 | `svn_follow_auto.rs` 改 1 条、新增 1 条，2/2 通过 |
| A5 验证、review、PR | 完成 | 独立 review 1 阻断 + 3 建议 + 3 nit 已修，见下 | 最终代码上：`pnpm build` 通过；`pnpm test` 103 个文件 1783 条全过；`pnpm test:layout` 176 条全过；`cargo fmt --check`、`cargo clippy --all-targets` 无告警；`cargo test --lib` 177/177、`--test svn_follow_auto` 38/38。全量 `cargo test` 在 review 修复之前跑过一次 92 个测试二进制全过；修复后的全量重跑因系统内存不足被回收，只重跑了受影响的两组（后端只改了 `auto_blocker` 条件 8 的文案，只有 `svn_follow_auto.rs` 断言它） |

## 决策条目

### A1 没放回的补丁

- **偏离计划**：计划写「原因只写一次」，没说原因不止一种时怎么办。实际：只有一种原因时写进说明；有几种时按原因分组，组名就是原因，说明里不写原因。后端的原因除了「不是你」，还有「文件被占用」「这次跟进是崩溃后续上的」等（`with_restore`、`RESUMED`），同一次报告里可能混着出现。
- **偏离计划**：「SVN 最后提交者」后端没单独给，取自 `report.changes`（`ExternalChange.author` / `rev`）。理由：能放回的文件 ⊆ drift ⊆ 这次的外部变更（`drift_paths` 只看 `external` 里的路径），所以一定在 `changes` 里；被 SVN 删了的不会进 drift 的修改分支。找不到就不写这一项，匿名提交写「没有作者」。
- **计划的错误**：旧说明写「不要把磁盘版本提交进分支，那会把补丁删掉」。这把「不要补丁了」这条路堵死了：自动放回不做，正是因为补丁可能是用户有意去掉的。新说明把两条路都写上：要补丁就放回；是有意去掉的，就在顶部横幅上点「不放回」，与横幅（`describePendingRestores`）的说法一致。
- **为什么不叫人提交**：初稿写的是「不要了，就把去掉补丁的修改提交进分支」，理由是 `restorable` 意味着磁盘就是 SVN 的版本。独立 review 指出两点：横幅上写的是「不要把它提交进分支」，两处说法相反；而且 `restorable` 只在跟进那一刻成立，没放回的原因可能正是「写回之后磁盘与分支、SVN 都不一样」「暂存区里是别的版本」，这时提交会把不知道是什么的内容带进分支。改成不提提交，第一句加「跟进时」限定时间。
- 已知遗留（改动之前就有）：原因再等也不会变的文件（`retry: false`）不进待放回清单，横幅上看不到它们，「顶部横幅上也能放回 / 点不放回」对它们不成立。这类文件在多原因分组里能看到各自的原因。
- 按钮挪进节里：`restorablePaths(r)` 与这一节的清单是同一个集合（`restorable` 减去 `restore.restored`），有节必有按钮（给了 `restoreLocalOnly` 时），反之亦然。`RestoreRow` 原来自带的说明和文件清单删掉，免得同一批文件列两遍；`commits` 属性随之去掉。
- **计划没写的坑**：节折起来时正文不渲染（`useFocusTrap` 依赖这一点，见 `Section` 的注释），按钮挪进节里之后，`RestoreRow` 自己的状态会随折叠丢掉：放回成功后折起再展开，按钮又回来，再点只会报「已经是分支的版本」（独立 review 第 4 条）。状态提到 `Dialog` 的 `useRestore` 里，`RestoreRow` 只管显示。

### A2 分支与磁盘对不上

- **偏离计划**：计划把「带上造成差异的提交」放在 B。核对代码发现 `overlap_files` 已经把整个 drift 并进 overlap，`commits` 就是 `retained_by_path` 的结果，只是前端没显示。这一步纯前端、与重写这一节是同一处改动，就在 A 里做了。B 只剩行数。
- 例外那句（「差的是 Local-Only 补丁时别提交」）保留：能放回的文件本来已经单列，但 `restorable_paths` 出错时后端给空列表（只记日志），这些文件会落回这一节。
- 旧后端没有 `overlap` 时照原样只列文件名。

### A3 泄漏告警

- `Leak.commit` 与 `report.dropped` 都是重放前的原始 OID：`svn_leak` 用 `c.oid` / 原始 commit 填 `commit`，`replay_branch` 把 `c.oid` 记进 `dropped`；其他分支的泄漏与 `dropped` 在 `plan_others` 里一起并进报告，口径相同。
- **计划没写的坑**：只比 OID 不够。另一条分支带着同一个 Local-Only 提交时，`plan_other` 见到泄漏就返回、不重放，提交还在那条分支上，但当前分支的重放已经把它记进了 `dropped`，那条分支的泄漏会被错标成「已从分支移除」（实现时自己发现，独立 review 第 1 条同样指出）。报告里没有当前分支名，前端改为「在 `dropped` 里，且所在分支不在 `behindBranches` 里」：有泄漏的其他分支一定进 `behindBranches`。后端 `Plan` 有 `branch`，直接比分支名。
- 已丢弃提交上的泄漏单列一组，不再参与「已确认 / 拿不准」的分档：提交重放后变空，说明内容整个在 SVN 上，分档没有意义。
- 节的级别不变（红）：补丁内容上了服务器这件事本身要用户知道，即使分支上已经没有它。

### A4 自动跟进被挡时的横幅

- 条件 8 本身不变（计划 §8.3）。只在文案里补一句：哪些提交的内容都已经在 SVN 上、跟进后会从分支上移除。
- 按提交 OID 去重、按说明点名：同一个提交泄漏多个文件时只点名一次；两个同名提交（「wip」）各算一个，「等 N 个提交」不少算（独立 review nit）。
- 只认当前分支（`l.branch == plan.branch`），理由同 A3。

### 独立 review（2026-11-17，opus subagent）

| 条目 | 级别 | 处理 |
|---|---|---|
| 1 别的分支共享提交时错标「已从分支移除」 | 阻断 | 已修，见 A3；`svnFollow.test.ts` 新增一条 |
| 2 报告让人提交、横幅说别提交 | 建议 | 已修，见 A1 |
| 3 没放回的原因可能是磁盘另有内容，「提交」不安全 | 建议 | 已修，见 A1 |
| 4 折叠后放回结果丢失 | 建议 | 已修，见 A1；`FollowReportDialog.test.tsx` 新增一条 |
| nit 同名提交去重少算 | nit | 已修，见 A4 |
| nit drift 附注没有前缀 | nit | 已修：「改过它的提交：…」 |
| nit 放回后跳过清单双重缩进 | nit | 已修：`.follow-restore .follow-items li` 不再缩进 |
| （自查截图发现）放回结果的红框缩进 54px | — | 已修：`.dlg-warning` 默认让出图标位，在节里改为 0 并占满宽度。有头 Edge 截 `follow-report-long` 场景核对过 |
| nit 没给 `restoreLocalOnly` 时说明仍暗示有按钮 | nit | 不改：改动之前就有；应用里 `RepoTab` 总是传它 |
- **计划没写的坑**：Rust 这条没有先看到红就改了实现。旧文案里没有「从分支上移除」这几个字，断言必然失败，所以判断这条测试确实钉住了改动；新增的「提交还在分支上时不提移除」覆盖了反向。

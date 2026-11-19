# 跟进报告可读性与跟进速度：实现日志

计划：[2026-11-16-follow-report-usability.md](2026-11-16-follow-report-usability.md)。A 在 PR 182，B 在下一个 PR，各自单独 review。

## 进度总账

| Task | 状态 | 产出 | 验证 |
|---|---|---|---|
| A1 没放回的补丁与放回按钮合成一节 | 完成 | `svnFollow.ts` 的 `pendingRestoreGroups`、`svnLastCommit`；`FollowReportDialog.tsx` 的 `Section` 收 `children`，`RestoreRow` 只剩按钮与结果；`styles.css` 的 `.follow-restore` 去掉边框 | `svnFollow.test.ts` 新增 5 条、改 1 条；`FollowReportDialog.test.tsx` 新增 1 条、改 1 条 |
| A2 「分支与磁盘对不上」三句话 + 提交 | 完成 | `svnFollow.ts` 的 drift 节说明与 `driftItem` | `svnFollow.test.ts` 新增 3 条、改 1 条 |
| A3 泄漏告警标出已从分支移除的提交 | 完成 | `svnFollow.ts` 的 `removedLeakItem`、`REMOVED_LEAKS` | `svnFollow.test.ts` 新增 3 条 |
| A4 自动跟进被挡时的泄漏横幅 | 完成 | `svn_follow.rs` 的 `auto_blocker` 条件 8 | `svn_follow_auto.rs` 改 1 条、新增 1 条，2/2 通过 |
| A5 验证、review、PR | 完成 | 独立 review 1 阻断 + 3 建议 + 3 nit 已修，见下 | 最终代码上：`pnpm build` 通过；`pnpm test` 103 个文件 1783 条全过；`pnpm test:layout` 176 条全过；`cargo fmt --check`、`cargo clippy --all-targets` 无告警；`cargo test --lib` 177/177、`--test svn_follow_auto` 38/38。全量 `cargo test` 在 review 修复之前跑过一次 92 个测试二进制全过；修复后的全量重跑因系统内存不足被回收，只重跑了受影响的两组（后端只改了 `auto_blocker` 条件 8 的文案，只有 `svn_follow_auto.rs` 断言它） |
| B1 `git_read::line_stats`：两个提交之间逐路径的增删行数 | 完成 | `git_read::{LineStat, line_stats, line_stat, blob_at}` | `tests/git_read_line_stats.rs` 2 条，逐路径与 `git diff --numstat` 对照，正反两个方向 |
| B2 跟进报告带上 `driftLines` | 完成 | `FollowReport.drift_lines`；`svn_follow.rs` 在 `drift_paths` 之后算 | `svn_follow_snapshot.rs` 新增 1 条端到端（2026-11-16 的形状）、原 drift 用例加 1 条断言；`svn_backend.rs` 序列化与旧 journal |
| B3 前端「对不上」逐行写差几行 | 完成 | `svnFollow.ts` 的 `LineStat`、`driftLinesText`；布局 harness 加 `driftLines` / `overlap` | `svnFollow.test.ts` 新增 5 条 |
| B4 验证、review、PR | 完成 | 独立 review 0 阻断、2 建议、6 nit，处理见下 | `pnpm build`；`pnpm test` 1808 条全过；`pnpm test:layout` 176 条全过；`cargo fmt --check`、`clippy` 无告警；全量 `cargo test` 见 PR |
| B5 每个对不上的文件一个「对比」 | 完成 | `FollowReport.drift_range`（`DriftRange { from, to }`）；`svnFollow.ts` 的 `ReportItem.compare`；`FollowReportDialog.tsx` 的 `CompareAction` 与按钮；`RepoTab.tsx` 的 `compareDrift`；`.follow-compare` | `svn_follow_snapshot.rs` 新增 1 条、原用例加断言（from / to 里的内容就是 SVN 版本与分支版本）；`svn_backend.rs` 序列化与旧 journal；`svnFollow.test.ts` 新增 1 条、`FollowReportDialog.test.tsx` 新增 4 条；`pnpm test` 1813 条、`pnpm test:layout` 176 条全过；有头 Edge 截图。review 修复后 `pnpm build` 通过、`pnpm test` 1814 条、`pnpm test:layout` 176 条全过，再截图核对 |

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

### B1 `git_read::line_stats`

- **偏离计划**：计划 §3.2 写「磁盘内容按 `git add` 的过滤规则取，与 `drift_paths` 同口径」。实际比的是新基线 → tip 两个 blob，不读磁盘（§8.1）：`drift_paths` 挑出的修改类文件，条件就是「磁盘按 `git add` 过滤后的 hash == 新基线的 blob」，所以磁盘内容就是新基线的 blob。计划 §5 里 autocrlf 那条风险因此不存在。
- **计划没写的坑**：git2 0.20 的 `Patch::from_blobs` 两边都要 blob，没有「一边没有」的入口。新增 / 删除的文件改用 `from_blob_and_buffer` 对空内容比；新增的文件反着比再把增删对调。这样两种情形都由仓库里的 blob 带路径，`.gitattributes` 的 `-diff` 照样认成二进制。`tests/git_read_line_stats.rs` 在同一个 fixture 上逐路径与 `git diff --numstat` 对照：改、增、删、CRLF、末尾换行、二进制、`-diff`、中文路径、两边相同、两边都没有。
- 两边内容相同、两边都没有的路径不出现在结果里，与 numstat 一致。子模块指针、目录算没有。

### B2 跟进报告带上 `driftLines`

- `FollowReport.drift_lines`，`#[serde(default)]`：旧 journal 读回来为空（`svn_backend.rs` 的序列化用例覆盖）。
- 只在重放成功（`Replayed::Moved`）那一处算，紧跟 `drift_paths`。出错只记日志、给空表，跟进不因它失败；前端没有行数时照原样只写提交。
- **计划没写的坑**：照 2026-11-16 的形状搭 fixture（一个提交加 7 行、下一个撤销，同事把同样 7 行提交进 SVN），起初不会对不上：分支相对旧基线的净改动为空，「已全部提交」的捷径把两个提交都收掉，分支直接等于新基线，结果是对的。真实现场分支上还有别的、SVN 上没有的改动，捷径不适用，才逐个重放、出现「撤销照样删」。测试里加了一个改别的文件的提交来复现。

### B3 前端

- 附注：「分支比 SVN 多 N 行 / 少 N 行 / 多 N 行、少 M 行」，二进制写「二进制文件，内容不同」，后面接「改过它的提交：…」。说不出来（旧后端、算不出、0/0）时只写提交。
- 布局 harness 的 `follow-report-long`：`restorable` 改成 drift 的一半。原来两者相同，能放回的文件不进「对不上」那一节，那一节是空的，长附注量不到。放回按钮仍在（50 个文件）。
- **偏离计划**：§2 的验收写「分支比 SVN 少 7 行 · c028e264 小婉问答」，实际是「分支比 SVN 少 7 行 · 改过它的提交：c028e264 小婉问答」：A 的 review 给提交加了前缀，B 沿用。
- SVN 上已删、分支把它带回来的文件（`report.removed` 里有它）写「SVN 上已删除，分支里还有这个文件（N 行）」，不写「多 N 行」：后者看不出是整个文件（独立 review nit 5）。分支把文件删了、SVN 上还有的情形前端分不出来，照写「少 N 行」。

### B5 「对比」（2026-11-18，用户反馈）

- **偏离计划**：计划 B 只要求写出差几行。用户看了 PR 之后指出：几处改动混在一起时，「多 12 行、少 9 行」说明不了差在哪，
  也帮不了判断该不该把磁盘的版本提交进分支。复杂的 diff 用文字描述不出来，改为每个文件旁边一个「对比」，用外部对比工具
  （Beyond Compare）直接打开。行数保留，只当大小的提示。
- **为什么比的是两个提交，不是「本地改动」**：跟进刚结束时，这些文件确实出现在「本地改动」里，可那里比的是暂存区与工作区：
  跟进会拼回暂存（`svn_follow_index`），暂存区不一定等于 tip；用户之后改了磁盘，比出来的也不再是报告说的那件事。
  对不上的文件按定义磁盘上就是新基线的 blob（`drift_paths`），所以比「新基线提交 → 重放后的 tip」同一个文件，正好是
  「SVN 的版本 → 分支的版本」，而且不随之后的操作变。复用现有的 `DiffTarget::Range`：左边 from、右边 to，一侧没有这个
  文件（SVN 上已删）时用空文件，过滤器照 `.gitattributes`。
- 两个都是提交 OID，不是 ref：tip 在分支历史里、新基线在 `svn/base` 历史里，之后的提交不会让它们变得不可达。
- 没配对比工具（`toolFor` 找不到）时不显示按钮；工具起不来时把原因写在那一节里；工具退出由 `RepoTab` 原有的
  `useExternalToolExit` 处理。

#### B5 的独立 review（2026-11-18，opus subagent）

0 阻断、3 建议、5 nit。

| 条目 | 级别 | 处理 |
|---|---|---|
| 1 `driftRange` → `DiffTarget` 的拼装写在组件里、没有测试：from / to 写反时所有测试照过，工具里左右与提示相反，用户会把分支版本当 SVN 版本提交 | 建议 | 已修：抽成 `svnFollow.ts` 的 `driftDiffTarget`，vitest 钉住方向 |
| 2 工具起来之后异常退出时，`useExternalToolExit` 走顶部红条，被报告框的遮罩挡住，用户看不见 | 建议 | 不改，记为已知限制：只在对比工具自己崩溃时发生；关掉报告就看得到红条。要在报告框里显示，得把退出事件接进对话框，牵动面大于这个风险 |
| 3 工具配置在点「跟进」那次渲染时就定死了：那一刻配置重读失败是 null，报告里一个按钮都没有 | 建议 | 已修：`RepoTab` 用 ref 存最新的配置与仓库路径，报告框每次渲染时按最新的找工具 |
| nit 1 每个按钮的可访问名都只是「对比」；按钮在路径的 span 里，复制路径会带上「对比」 | nit | 已修：`aria-label="对比 <路径>"`；按钮挪到路径外面，成为 li 的子元素（于是排成一列，位于路径与附注之间，截图核对过） |
| nit 2 错误条吃了 `.dlg-warning` 给图标留的 54px 缩进 | nit | 已修：`.follow-compare-error` 清掉；harness 的「对比」改成点了就报超长错误，截图核对对齐 |
| nit 3 错误不说是哪个文件 | nit | 已修：`<路径>：<原因>` |
| nit 4 工具窗口两侧的标题只有「路径（短 SHA）」，看不出哪边是 SVN | nit | 不改，记为后续：`DiffTarget` 目前不能传自定义标题，方向写在按钮的提示里 |
| nit 5 开工具用 `repoPath`、退出监听比对的是 `state.path` | nit | 已修：随建议 3 改用 `state.path` |

### B 的独立 review（2026-11-17，opus subagent）

| 条目 | 级别 | 处理 |
|---|---|---|
| 1 只有一边有 blob 时的二进制、空文件没测，正是另走一条路的地方 | 建议 | 已修：fixture 加新增 / 删除的二进制、`-diff`、空文件，正反两个方向都与 numstat 对照 |
| 2 实现日志状态没更新、缺 B 的决策条目 | 建议 | 已修，见上 |
| nit 3 注释说「同 numstat」太满；对照测试受全局 `diff.algorithm` 影响 | nit | 已修：注释写明默认配置与不同之处（只差 mode、myers、textconv、bigFileThreshold）；测试里 git 一侧固定 `-c diff.algorithm=myers` |
| nit 4 一条路径读不出来，整份行数都没了 | nit | 已修：逐路径出错只跳过并记日志 |
| nit 5 被带回来的文件只写「多 120 行」 | nit | 已修：见 B3 |
| nit 6 harness 注释说量长附注，场景里却没有 `overlap` | nit | 已修：harness 加两条长提交说明；有头 Edge 截图核对，路径不被挤没 |
| nit 7 测试里开 autocrlf 不起作用 | nit | 改注释：两边都是 blob，autocrlf 本来不参与；真正覆盖 autocrlf 的仍是 `drift_paths` 已有的用例 |
| nit 8 每个 drift 文件做一次完整 diff，没有大小上限 | nit | 不改：drift 通常几个文件；Unity 大 YAML 成批对不上时再加上限，单独写「太大」 |

- **计划没写的坑**：全量 `cargo test` 第一次跑时 `svn_config.rs` 的 `concurrent_set_svn_user_calls_do_not_collide_on_the_temp_file` 失败一次，单独重跑 3/3 通过。本 PR 没碰 `svn_config`：8 个线程在 Windows 上并发 rename，负载高时偶发。cargo 默认在第一个失败的测试二进制处停下，之后改用 `--no-fail-fast` 跑全量。

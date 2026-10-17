# Phase 3a 遗留小修 · 决策日志

没有单独的计划文件：条目来自 `2026-10-07-svn-phase3a-sync-out-decisions.md` 里标为「后续」的观察与 review 结论，用户 2026-10-16 选定「方向 C」打包一个 PR。分支 `worktree-svn-phase3a-followups`，基点 `610fdb0`。

## 进度总账

| Task | 内容 | 来源（3a 决策日志） | 状态 | 产出文件 | 验证 |
|---|---|---|---|---|---|
| 1 | 「更新」成功后刷新失败不能让对话框以为更新失败 | 最终 review 第 3 条 | 完成 | `src/components/svnSyncFlow.ts` | `svnSyncFlow.test.ts` +3，2 条先 RED |
| 2 | 未收尾 / 持锁时，工具栏「更新」「交付」禁用并说明原因 | E2E review 后的观察 | 完成 | `src/engine/svnJournal.ts`、`Toolbar.tsx`、`RepoTab.tsx` | `svnJournal.test.ts` +5、新 `Toolbar.test.tsx` 2 条；先 RED 5 条，review 后再 RED 2 条 |
| 3 | 结局提示框不再借确认框外观（警告三角、多余的 Cancel） | 同上 | 完成 | `ConfirmDialog.tsx`、`svnSyncIn.ts`、`svnSyncOut.ts`、两个 flow、`styles.css` | `ConfirmDialog.test.tsx` +3、`svnSyncIn/Out.test.ts` 各 +1、两个 flow 各 +1，先 RED 6 条 |
| 4 | 顶部错误条保留多行消息的换行 | 同上 | 完成 | `styles.css` | 纯 CSS；真实窗口 ✅（见文末）|
| 5 | 全应用滚动条跟随深色主题 | 偏离计划 · `.deliver-dialog` 条 | 完成 | `styles.css` | 同上 |
| 6 | 「更新」预览里被删目录的误报 | E2E 之后独立 review 第 4 条 | 完成 | `src-tauri/src/engine/svn_sync_check.rs` | fixture 实测 4 个场景；`svn_sync_check.rs` +3；先 RED 1 条，review 后再 RED 1 条 |

## 决策条目

### Task 1 · `svnSyncFlow` 的 `afterSync` 不抛

- **做法**：照搬 `svnSyncOutFlow` 的修法，刷新失败进错误条，`fetchSvnLog` 照常发。
- **偏离（相对 3a 交付侧的做法）**：交付侧还有 `refreshing()`（执行失败时也刷新横幅），这里**不加**。理由：后端 `svn_sync.rs` 的 `with_watch_suspended` 无论成败都补发一次 `repo-changed`，store 的监听会跟着刷新 `svnSyncState`；更新侧失败后横幅本来就会跟上。交付侧的 `refreshing` 是不是同样多余，本 PR 不碰。
- **为什么这么改是安全的**：只改前端刷新的错误去向，不改任何同步命令和它的顺序。

### Task 2 · 工具栏禁用

- **计划没写的坑**：3a 日志写的是「后端会拒，不会出事」。实测代码：两个预览命令（`preview_sync_in`、`preview_sync_out`）**一进门**就查锁和 journal，在走网络之前。所以原来的代价只是「点了才弹错误条」，不是「白等几秒网络」。注释按这个写，起初写成后者，已改正。
- **判据**：`svnSyncBlockedReason` 对持锁（重建基线持锁时单独说「正在重建基线」）、有未收尾记录两种返回悬停说明；`null`（还没读到）和 `readError` 不拦（后者是 review 第 2 条之后改的，见下）。横幅上的按钮不受影响，收尾走的就是它们。
- **测试**：新增 `Toolbar.test.tsx`（jsdom 组件测试，AGENTS.md §8），断言 `disabled` 与 `title`。

### Task 3 · 结局提示

- **做法**：`ConfirmRequest` 加可选 `notice?: 'ok' | 'attention'`。设了就只渲染一个按钮（没有 Cancel），图标按分量选：`ok` 是绿色对勾，`attention` 是琥珀色三角。不设时行为逐字不变（护栏测试「普通确认框照旧有 Cancel」）。
- `OutcomeNotice` 加必填 `tone`，按「还要不要用户动手」分：
  - 更新：`aborted`、`settled` 是 `ok`；`rebaseConflict`、`stashConflict`、`stashBlocked`、`released` 是 `attention`。`settled` 虽然提到 stash 里留着备份，但那是「为防万一」，删不删随用户，不算待办。
  - 交付：`delivered`（含还有本地补丁的情形）、提交前的「已清理」是 `ok`；落回冲突、「已停止收尾」（还欠一次更新，否则下次交付被拒）是 `attention`。
- **未改**：`RepoTab` 里体检结果那个框（`confirmLabel: '重建基线' | '知道了'`）。它在 `offerRebuild` 时确实是二选一，不是纯提示；不在 3a 的观察范围内。

### Task 4 · 错误条多行

- 原因：`.banner-error` 没有 `nowrap`，换行是被默认的 `white-space: normal` 吃掉的。消息 `span` 加 `white-space: pre-wrap`、`overflow-wrap: anywhere`，并限高 30vh 可滚（几十行的 svn stderr 不把标签页顶下去）；`×` 顶部对齐。

### Task 5 · 全局深色滚动条

- **做法**：`:root { color-scheme: dark }`，删掉 `.deliver-dialog` 上那条局部声明（现在继承自 root）。
- **为什么不用 `scrollbar-color`**：它只改滚动条，影响面更小；但 Chromium 设了非 `auto` 的 `scrollbar-color` 会换一套滚动条外观。`color-scheme` 是平台原生的深色画法，和 3a 在交付框里已经验证过的效果一致。
- **影响面（要在真实窗口确认）**：`color-scheme` 也会让**没设样式的原生控件**按深色画。逐个查过：对话框里的按钮、输入框都有显式样式，不受影响；会变的是没有样式的那几个——横幅上的按钮（`BaselineBanner`）、错误条的 `×`、`SvnInitWizard` 的按钮、`StashView` 的按钮、提交区的 Amend 复选框。这几个会从浅灰底变成深色底，与应用其余部分一致，正是想要的；但错误条是红底，`×` 变成深色按钮后的观感要看一眼。

### Task 6 · 「更新」预览里被删目录的误报

- **fixture 实测**（临时探针，跑完已删，不进 PR）。基线里有 `Code/Solo/S.cs`，本地 `git rm` 后 git 删掉了空目录：

  | 场景 | `svn status -u` 里的 `Code/Solo` | 修前 outside |
  |---|---|---|
  | A 同事改了 `S.cs` | `missing`，无 repos-status | `[]` |
  | B 同事在目录里新增 `T.cs` | `missing` + repos `modified` | `["Code/Solo"]` ← 误报 |
  | C 同事改别处 | `missing`，无 repos-status | `[]` |
  | D 目录里还有 `x.psd`，手工删整个目录，同事新增 `T.cs` | `missing` + repos `modified`；`x.psd` 单独列为 missing、无 incoming | `["Code/Solo"]` |

  missing 目录的子项总是单独列出来（`S.cs`、`x.psd`），这条判据才成立。
- **做法**：以交付侧 `svn_sync_out_check::outside_changes` 的判据为底：只跳过「仅因 missing（属性没改）」且是别的条目祖先的目录。review 第 1 条之后又多加一条：目录底下没有丢掉任何非 W-file（见下）。没有抽成公用函数：交付侧按多个 target 分别跑 status 再合并，形状不同；与 3a 日志第 7 条（录制 / 发布代码重复）一起留给后续重构。
- **为什么这么改是安全的**：
  - 测试里真跑了一次更新：同事的 `T.cs` 落地，本地的删除照样重放在新基线上（`git log -1` 仍是 `drop solo`）。update 对 missing 目录只会把它拉回来，不丢东西。
  - 护栏测试：目录被手工删掉、同事又改了里面的 `x.psd` 时，`x.psd` 仍然报出来。
  - 执行端 `run_sync_in` 在锁内用同一个 `check_sync_in` 复核 `confirmedOutside`，预览与执行的判据不会分叉。
- **观察（未改，记为后续）**：场景 D 里 `x.psd` 被本地删除、服务器上没动，现有规则（`local && incoming`）不报它，而 `svn update` 会把 missing 文件恢复回来。恢复不丢数据（只是撤销了删除），但「更新会碰到的本地改动」严格说不止 incoming 这一种。这是 §7.2 第 5 条本身的口径问题，不在本 PR 的范围。

### Review（`/code-review high`，未提交 diff，2026-10-17）

9 条，逐条对照代码核对：

| 条 | 发现 | 判定 | 处理 |
|---|---|---|---|
| 1 | 场景 D：用户在资源管理器里删掉含 `x.psd` 的目录、同事往里加文件时，修前经由目录条目报警，修后一条都不报，update 静默把 `x.psd` 拉回来 | 成立，中 | 目录底下**没有丢掉任何非 W-file**时才跳过它（只从叶子条目往上标，免得嵌套目录被 git 删时又误报）。`a_directory_deleted_with_files_git_does_not_manage_is_still_listed`，先 RED（`[]` vs `["Code/Solo"]`）。上面 Task 6 的「观察」照旧：服务器上没动的 missing 文件不单独列，是 §7.2 第 5 条的口径问题 |
| 2 | `readError` 也禁用按钮：一次偶发读失败就让按钮一直灰着——`readError` 不轮询，横幅也没有按钮 | 成立，中 | `readError` 不拦，真坏了预览会以 journal 错误拒绝。测试改为断言 `null`，先 RED |
| 3 | 重建基线持锁时悬停说明写「同步正在进行」，与横幅「正在重建基线」矛盾，用户可能误以为锁是残留的而重开应用 | 成立，低 | 与 `describeUnfinishedSync` 同一判据（`BASELINE_MOVE_OPID_PREFIX`）。+1 测试，先 RED |
| 4 | 全局 `color-scheme` 没在真实窗口里看过 | 成立 | 本来就标着「待真实窗口看」，交付时如实报告，不当成已验证 |
| 5 | 两个 flow 的 `afterSync` / `exclusive` / `showOutcome` 近乎逐行重复 | 成立，重构 | 不在本 PR 做。与 3a 日志第 7 条一起留给后续的同步代码去重 |
| 6 | 目录判据与交付侧重复；每个条目都分配一个 `Vec` | 前半成立（同上，推迟）；后半成立 | 改为直接插进 `HashSet` |
| 7 | 通用的 `ConfirmDialog` 从 SVN 模块引 `NoticeTone` | 成立，低 | `ConfirmDialog` 自己定义并导出 `NoticeTone`；SVN 的 `OutcomeNotice.tone` 同形，结构类型兼容 |
| 8 | 新加的导入没用 `@/` 别名（AGENTS.md §7.2 MUST） | 成立 | `RepoTab` 的新导入改成 `@/engine/svnJournal`（第 7 条修完后 `ConfirmDialog` 那行已经没了）。**规则冲突**：同文件已有的导入都是相对路径；按 AGENTS.md §0 的优先级，新代码照 MUST 写，旧导入不顺手改（§1.2 最小 diff） |
| 9 | `Toolbar.test.tsx` 有 act() 警告（`CustomCommandBar` 挂载后异步 setState） | 成立 | 把 `CustomCommandBar` mock 成空组件，警告消失。全量跑时剩下的一条 act 警告来自 `DeliverDialog.test.tsx`，本 PR 没碰那个测试和组件，是原来就有的 |

### 真实窗口验证（2026-10-17，`pnpm tauri dev`，本 worktree 的 `app.exe`）

fixture：`scripts/e2e-svn-fixture.ps1`（Simple）建在 `%TEMP%\phase1c_2bd778ae`，用户在应用里初始化（白名单 `Code/`）。造现场的辅助脚本放在会话 scratchpad，不进仓库；冲突现场照 3a E2E 场景 6 的服务器 post-commit hook。用户逐项确认：

| 项 | 现场 | 结果 |
|---|---|---|
| 第 5 条滚动条 | 旁支 `scroll-test` 60 个提交，提交图出滚动条 | ✅ 深色 |
| 第 5 条副作用 | 红色错误条上的 `×` 变成深色原生按钮 | ✅ 可接受 |
| 第 4 条多行错误条 | 旁支 `merge-test` 上有 merge commit，点「更新」 | ✅ 按行显示（`sync_in_merge_in_range`） |
| 第 3 条 ok | 交付 `add P` → 「已交付 r2」 | ✅ 绿色对勾、只有「知道了」 |
| 第 3 条 attention | 交付 `change A`，hook 让同事在 D5 后改同一行 → 「交付成功，本地补丁落回时有冲突」 | ✅ 琥珀色三角、只有一个按钮 |
| 第 2 条 | 停在 `restoring-branch` 时 | ✅ 工具栏「更新」「交付」灰、悬停有说明 |
| 收尾 | 暂存 `P.cs` →「我已解决，继续」→「已交付 r3」 | ✅ 绿色对勾；journal 清掉、回到 `work`，服务器最新仍是 r4（没有重复提交） |

- **发现（既有问题，不在本 PR 修）**：冲突文件（`UU`）在「本地改动」右侧显示「没有文本改动」、行数 `+0/-0`。git 命令行对它输出的是 combined diff（`diff --cc`、`@@@`）；应用读工作区 diff 走 libgit2 `diff_tree_to_workdir_with_index`（`diff.rs`），冲突条目只给一个 `Conflicted` delta，不带 hunk。本 PR 没碰 `diff.rs`，读 diff 这条路径与 master 相同，所以普通 git 仓库里的 merge / rebase 冲突应当同样如此（没有专门在 master 构建上复现）。修法另开任务，候选方案：冲突文件改显示「工作区 vs HEAD」（推荐起步）/ 解析 combined diff / 分别对 ours、theirs 各显示一段。

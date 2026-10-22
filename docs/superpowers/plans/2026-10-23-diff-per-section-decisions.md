# 本地改动的 diff 按分区比较（issue 第 65 条）—— 实现日志

> 没有单独的计划文件：issue 第 65 条正文已写明问题、期望与验收，用户 2026-10-23 在 A / B 两个方案里选了 **A**（`read_diff` 加 `unstaged` / `staged` 两个 scope，前端按选中的分区请求对应那个）。
> 分支 `worktree-diff-per-section`（worktree `.claude/worktrees/diff-per-section`），基点 `e4c2e43`。
> 执行方式：主会话直接施工。

## 进度总账

| Task | 状态 | 产出文件 | 验证结果 |
|---|---|---|---|
| 0 建 worktree 与决策日志 | ✅ 完成 | 本文件 | 纯文档；`git status` 只有本文件 |
| 1 后端 `unstaged` / `staged` 两个 scope | ✅ 完成 | `src-tauri/src/engine/diff.rs`；测试 `src-tauri/tests/fixtures.rs`（+9，冲突 3 例改为两个 scope 都跑） | RED 6 例（新 scope 落进默认分支，两边都是 HEAD 对比工作区）；GREEN 后 `cargo fmt --check` 过、`clippy --all-targets` 无告警、`cargo test -j 4 --no-fail-fast` 62 个二进制 1031 例全过、1 例 ignored；改名守卫做了变异验证 |
| 2 前端：选中的分区决定 scope | ✅ 完成（review 后重做，见 Task 2R） | `src/engine/fileSelection.ts`（`reconcileSelection`、`activeChange`、`changesPaths`、`changesSection`）、`src/components/LocalChangesView.tsx`（受控选中）、`src/store/repoStore.ts`（`useRepoEffects` 加参数、监听器 `disposed`、`refreshStatus` 交回状态）、`src/components/RepoTab.tsx`（持有选中、`refreshChanges`）；测试 `fileSelection.test.ts`、`LocalChangesView.test.tsx`、`repoStoreDiffRefresh.test.ts`、新 `repoEffectsDiffScope.test.ts` | 第一版 RED 16 例；review 后再 RED 21 例；`pnpm build` 过、`pnpm test` 53 文件 778 例全过；`src/` 里不再有 `'workdir'` |
| 2R 独立 review 修复（10 条） | ✅ 完成 | 见 Task 2R；后端 `diff.rs` 的 `staged` 过滤冲突条目 + `fixtures.rs` +1 | Rust 1032 例全过；前端 778 例全过 |
| 2R2 第二轮 review 修复（8 条） | ✅ 完成 | 见 Task 2R2；`diff.rs` 的 pathspec 按字面量 + `fixtures.rs` +1；store 的 `refreshChanges` | Rust 1033 例全过；前端 782 例全过 |
| 3 全量验证与真实窗口实测 | ✅ 完成 | 本文件 | Rust：`cargo fmt --check` 过、`clippy --all-targets` 无告警、`cargo test` 62 个二进制 1033 例全过、1 例 ignored；前端：`pnpm build` 过、`pnpm test` 53 文件 782 例全过；`pnpm tauri dev` 真实窗口 9 项由用户实测全部通过（2026-10-23） |

## 决策条目

### Task 0

#### 方案选择

- A：`read_diff` 加两个 scope，前端按选中分区请求。每次只读一个分区，结果里同一路径最多一条，`DiffView` / `activeDiff` 按路径认文件不用改；代价是「当前选中哪个分区」要从 `LocalChangesView` 提到 `RepoTab` / `useRepoEffects`，写死 `'workdir'` 的刷新都要改，切换分区时重读一次。
- B（未采用）：一次读回两半、每条带 `section`。同一路径会出现两条，前端按路径认文件的三处（`activeDiff`、`DiffView` 的 key、默认文件的分区）都要改成按「分区 + 路径」。
- 用户选 A：同一路径只出现一次，数据形状更简单。

#### 偏离计划 / 计划的错误

- 无计划文件。issue 正文是唯一输入，与它不一致之处在各 Task 下记。

### Task 1 · 后端两个 scope

#### 偏离计划（相对 issue 正文）

- **`staged` 不做改名检测**。issue 的实现提示写「Staged 里的改名可以做 `find_similar`，显示成一条改名」。没照做：Staged 列表来自 `read_status`，它没开 `renames_head_to_index`，`git mv a b` 在列表里是 `a.txt`（删）与 `b.txt`（增）两项。diff 若合成一条改名（路径 `b.txt`、`old_path` `a.txt`），点列表里的 `a.txt` 时右侧找不到它的条目，显示「No changes」。列表与 diff 必须同一套口径；要显示改名，得让 `read_status` 与 diff 一起改，不在本条范围。
  - 守卫：`every_path_in_the_staged_list_has_a_staged_diff_entry`。**变异验证**：给 `staged` 加上 `find_similar` 后它红（diff `["added.txt","b.txt"]`、列表 `["a.txt","added.txt","b.txt"]`），撤掉后绿。
- **`workdir` scope 保留**，前端改完后不再有调用方，但默认分支（认不出的 scope）仍走它，删掉要连带决定「认不出的 scope 报错还是退化」。`a_malformed_range_is_an_error_not_a_workdir_diff` 已记录过「落到默认分支会把工作区改动当成别的东西」这类风险；这次不扩大范围，记为后续。

#### 计划没写的坑（实测）

- **`diff_index_to_workdir` 对冲突条目照样给 `Conflicted` delta**，`fill_conflicted` 能照常补上 HEAD 对比工作区的行。之前不确定这一点（第 63 条的根因是带 index 的 diff 对冲突条目不给 hunk），所以把原来三条冲突用例改成 `workdir` 与 `unstaged` 两个 scope 都跑，而不是另写一份：`conflicted_file_diff_is_head_vs_workdir`、`…_applies_autocrlf`、`…_missing_in_workdir_shows_deletion` 在 `unstaged` 下全绿。
- 「暂存了修改、又在工作区删掉」时，Unstaged 的删除对着的是**暂存区那一版**（`-line 2 staged`），不是 HEAD 那一版。这正是按分区比较的语义，用例 `a_staged_change_deleted_in_the_workdir_shows_on_both_sides` 钉住。
- 改名守卫在实现前就是绿的（两个 scope 当时都落进默认分支，列表与 diff 恰好一致）。它不是 RED → GREEN 的用例，有没有牙靠上面的变异验证证明。

#### 为什么这么改是安全的

- 只读：三个分支都只构造 diff、不写仓库。
- 坏路径、冲突文件的处理抽成 `workdir_side`，`workdir` 与 `unstaged` 共用同一份代码，`workdir` 原有行为不变（原有 21 例 `fixtures` 用例全绿）。`readable_only` 拿空列表重建的那条保护（空 pathspec 等于不限制）原样保留在 `workdir_side` 里。
- `staged` 不读工作区文件，悬空链接与它无关；`unstaged_scope_survives_a_dangling_link_in_the_workdir` 同时断言两个 scope 都不因坏链接失败。
- 命令签名没变（`read_diff(path, scope)`），只是 `scope` 多了两个取值，前后端契约的形状不变。

### Task 2 · 前端按区读

> 本节是 review 修复**之后**的终态。第一版做法（子组件用 effect 把区报给父组件）被独立 review 否掉，原因与改动见下面「Task 2R」。

#### 做法

- **选中放在 `RepoTab`**（`changesSelection`），`LocalChangesView` 改为受控（`selection` / `onSelectionChange`；不传时自己持有，单独挂载与既有测试照旧）。读哪个区在**渲染时**由 `changesSection(selection, status)` 算出，传给 `useRepoEffects`。
- 纯函数都在 `src/engine/fileSelection.ts`：
  - `reconcileSelection`：状态刷新后让选中与两个列表对齐——都还在就原样返回（同一对象）；有的不见了留下其余的；全不见了但有的跑到另一个区（刚被整份暂存 / 取消暂存）就**整批**跟过去（按那个区的显示顺序）；两边都没有（刚提交 / 丢弃）就清空。查找用 Set。
  - `activeChange`：先对齐，再取最后选中的那一个；没选时取 Unstaged 第一个、再取 Staged 第一个（只决定右侧显示什么，**不算选中、不高亮**）。
  - `changesPaths`：两个列表在界面上的顺序（冲突文件排 Unstaged 最前），`RepoTab` 与 `LocalChangesView` 共用，默认项两边算的一致。
  - `changesSection`：上面三者合起来给出区。
- `LocalChangesView` 的高亮、Delete 快捷键、Shift 多选、右键都用**对齐后**的选中。对齐结果由**选中的持有者**存回：受控时是 `RepoTab`（effect 挂在 `state.status` 上，提交图视图里 `LocalChangesView` 没挂载时也照样对齐），不受控时是组件自己。渲染已经按对齐后的走，存回只是让「两边都有」之后不跳回旧区。
- 写操作之后的刷新统一走 store 的 `refreshChanges(path, () => 选中)`：`refreshStatus` 交回读到的状态，按它与最新的选中算区，再读那个区的 diff；状态读取被更晚的刷新盖掉时**不跳过**，按 store 里最新的状态算区。提交、丢弃、Stage / Unstage、DU / UD 二选一、外部工具退出（在本地改动视图时）都走它。
- 文件监听（`repo-changed`）读 diff 时取事件到达时的 `wantedDiffScope`，不取注册监听那一刻的 `diffScope`。
- 读哪个 scope 由 `useRepoEffects`、`refreshChanges` 与文件监听决定，三者都按「当前要的区」；`wantedDiffScope` 与 `currentDiff` 的「只显示当前要的那一份」机制原样复用。

#### 偏离计划（相对 issue 正文 / 原行为）

- **选中的文件换了区时，选中跟过去**（不只是右侧跟过去）。原来点中 Unstaged 的 `a.txt` 再整份 Stage，选中仍记着「Unstaged 的 `a.txt`」，右侧照样显示（原来读 HEAD 对比工作区，与区无关）。按区读之后不跟过去右侧会变成「No changes」，而 Delete 等快捷键也会对着一个已经不在那个区的文件。
- **选中的文件两边都没有了（刚提交、刚丢弃）时清空选中**，右侧回到默认第一个。原来停在一个已经不存在的路径上、显示「No changes」。
- **选中跨视图保留**：选中原来在 `LocalChangesView` 里，切到提交图再切回来就清空；现在放在 `RepoTab`，切回来还是原来那个文件（仍会按当时的状态对齐）。
- 公共接口变化（§1.2）：`useRepoEffects` 多一个必填参数 `changesScope: FileSection`；`refreshStatus` 的返回从 `Promise<void>` 变为 `Promise<StatusData | undefined>`（原调用方都不读返回值，不受影响）；`LocalChangesView` 多两个可选 prop。

#### 计划没写的坑

- **默认项不能高亮**：没选任何东西时右侧显示 Unstaged 第一个，但它不是选中，高亮了会让人以为 Delete / 右键会作用在它上面（review 第 5 条，原行为就是不高亮）。
- 区变化时 `useRepoEffects` 的 effect 与 `refreshChanges` 可能各读一次同一个新区（effect 因 scope 变了重读，回调按新状态读）。代次闸门只留后到的那份，结果正确，代价是一次重复读取。与文件监听在写操作后再读一次是同一类既有的重复，没有为它加额外的去重。
- 全量 `pnpm test` 里有一条 act() 警告（`Harness`），来自本分支没碰过的 `DeliverDialog.test.tsx` 等，是已知的既有问题（issue 第 81 条记过）。

#### 为什么这么改是安全的

- 纯前端读路径，不发任何新的写命令；Stage / Unstage / 丢弃 / 提交的命令与参数一字未改，只改了它们之后刷新哪个 scope。
- 丢弃的目标仍是**对齐后的选中**里的路径，且由用户在确认框里看到具体路径后才执行；对齐只会去掉已经不在该区的路径，或把选中挪到文件所在的那个区——不会让 Delete 作用到用户没选过的文件上（`LocalChangesView.test.tsx`「选中的文件刚被整份暂存……按 Delete 丢的就是它」）。
- 提交详情（`commitScope`）一侧完全没动：`useRepoEffects` 只在 `viewMode === 'changes'` 时用 `changesScope`（「在提交图视图里：区与读哪份 diff 无关」）；外部工具退出在提交图视图里仍按 `commitScope` 刷新。

### Task 2R · 独立 review（`/code-review high`，未提交 diff，2026-10-23）

10 条，逐条对照代码核对：

| # | 发现 | 判定 | 处理 |
|---|---|---|---|
| 1 | `repo-changed` 监听的 effect 依赖 `diffScope`，换区就重新注册；清理时 `listen()` 若还没回来，旧监听器永远不注销，之后每次存盘多跑一整套刷新 | 成立。**换提交时同样会触发**（`diffScope` 随选中的提交变），master 上就有，按区读之后更常见 | 照 `useExternalToolExit` 加 `disposed`：回来时已清理就立刻注销。`repoEffectsDiffScope.test.ts` +1，先 RED（旧监听器 0 次注销） |
| 2 | 区由子组件 effect 报上来，父组件第一次渲染先按旧区读一整份 diff；也是第 1 条的主要触发源 | 成立 | 选中提到 `RepoTab`，区在渲染时算（见上「做法」）。`onActiveSectionChange` 删除 |
| 3 | Stage 等回调用点击那一刻的区刷新，选中的文件换区后读回的整份旧区 diff 必被丢掉 | 成立（结果正确，白读一次；大工作区代价明显） | `refreshStatus` 交回状态，`refreshChanges` 按新状态算区。`repoStoreDiffRefresh.test.ts` +2，先 RED 1 条 |
| 4 | 选中的文件换区后高亮跟过去了，`selection` 没跟，Delete / Shift 多选对着旧状态 | 成立 | 对齐的是选中本身（`reconcileSelection`），并写回父组件。组件测试 +2，先 RED |
| 5 | 没选任何东西时默认第一行也被高亮成 active，与原行为不同，且未记录 | 成立 | 高亮回到「选中所在的区」，默认项不高亮。组件测试 +1，先 RED（高亮了 1 行） |
| 6 | 在两个区之间切换，每次重读整个区的 diff；原来一份 `workdir` 两边共用 | 成立，**属方案 A 的已知代价**：选方案时已向用户说明「切换分区时会重新读一次」，用户选了 A | 不改。若大工作区里明显，可按区缓存最近一份，或改成只读当前文件——与第 73 条（超大 diff 太慢）一起考虑 |
| 7 | 多选的最后一项不见了时跳到 Unstaged 第一个，而不是留下仍选着的其余项 | 成立 | `reconcileSelection` 留下其余项。纯函数测试，先 RED（实际返回 `u1`） |
| 8 | Staged 的 diff 在读时也说「正在读取工作区的改动…」 | 成立 | 按区说「正在读取暂存区的改动…」。组件测试 +1，先 RED |
| 9 | `changesDiffScope` 是恒等函数；`'unstaged' \| 'staged'` 与 `FileSection` 重复声明 | 成立 | 删掉 `changesDiffScope`，直接用 `FileSection` |
| 10 | `staged` 没有「合并冲突中」的用例（AGENTS.md §8） | 成立，且**真有缺陷** | 补 `staged_scope_during_a_merge_conflict_matches_the_staged_list`：先 RED，staged diff 多出冲突的 `c.txt`（`read_status` 只把它放 Unstaged）。`staged` 分支过滤掉 `conflicted` 条目 |

验证：`cargo fmt --check` 过、`clippy --all-targets` 无告警、`cargo test -j 4 --no-fail-fast` 62 个二进制 1032 例全过、1 例 ignored；`pnpm build` 过、`pnpm test` 53 文件 778 例全过。

### Task 2R2 · 第二轮独立 review（`/code-review high`，Task 2R 之后的未提交 diff，2026-10-23）

8 条，逐条对照代码核对：

| # | 发现 | 判定 | 处理 |
|---|---|---|---|
| 1 | `refreshChanges` 的状态读取被盖掉时直接返回、不读 diff；而盖掉它的可能是工具栏、checkout、stash 这些只刷状态不读 diff 的路径，右侧停在旧内容。原来是无条件读 | 成立，**Task 2R 引入的回归** | 挪进 store：被盖掉时按 store 里最新的状态（`latestStatus`）算区照样读。`repoStoreDiffRefresh.test.ts` +2，先 RED（函数不存在） |
| 2 | 有坏路径时按 pathspec 逐个重建 diff，没关通配，`[` `*` 当模式匹配 | 成立。**master 上的 `workdir` 就有**，`unstaged` 继承了它。review 描述的形态（`f[1].txt` 的 diff 不见）实测不复现：libgit2 对字面量相同的路径照样能匹配上；真正的危害是**过度匹配**——`f[1].txt` 当模式会把坏链接 `f1.txt` 拉回来，`print` 中止，`f[1].txt` 列出来了但内容为空 | `workdir_diff_opts` 有 pathspec 时 `disable_pathspec_match(true)`（与 `fill_conflicted` 一致）；`read_status` 的行数统计用同一份选项，一并受益。`rebuilding_around_a_dangling_link_matches_paths_literally`：坏链接就叫 `f1.txt`，先 RED（`Some([])`） |
| 3 | 文件监听按注册时的 `diffScope` 读，选中的文件换区后读回的旧区 diff 必被丢掉 | 成立 | 改读事件到达时的 `wantedDiffScope.current`。`repoEffectsDiffScope.test.ts` +1，先 RED |
| 4 | `changesSection` 每次渲染都在整份列表上重算，`includes` 是 O(k·n) | 成立 | `RepoTab` 里 `useMemo([changesSelection, state.status])`；`reconcileSelection` 改用 Set |
| 5 | 对齐结果只由 `LocalChangesView` 的 effect 写回，提交图视图里它没挂载，选中不对齐，「跳回 Unstaged」又会出现 | 成立 | 写回挪到持有者 `RepoTab`（effect 挂 `state.status`）；组件只在不受控时自己写回。组件测试改 1 条（受控时不写回，先 RED）、+1 条不受控的端到端（暂存后又改、两边都有，仍在 Staged） |
| 6 | 多选整批换区时缩成最后一个，用户手上的那一批没了 | 成立 | 整批跟过去，按那个区的显示顺序。纯函数 +2，先 RED |
| 7 | Delete 快捷键仍各自拼两份路径列表，与 `changesPaths` 是两个事实源 | 成立 | 直接传 `paths` |
| 8 | Task 3（真实窗口验证）还没做 | 成立，本来就排在最后 | 见 Task 3 |

**仍然存在、有意不处理的重复读取**：一次 Stage 之后，`refreshChanges`、`useRepoEffects`（区变了）、文件监听最多各读一次**同一个新区**，代次闸门只留最后一份，结果正确。master 上 Stage 之后也是回调与文件监听各读一次。要彻底去重得给「读到一半的同 scope 请求」加合并，不在本条范围。

验证：`cargo fmt --check` 过、`clippy --all-targets` 无告警、`cargo test -j 4 --no-fail-fast` 62 个二进制 1033 例全过、1 例 ignored；`pnpm build` 过、`pnpm test` 53 文件 782 例全过。

### Task 3 · 真实窗口实测（2026-10-23，`pnpm tauri dev`，本 worktree 的 `app.exe`）

fixture：会话 scratchpad 下的一次性仓库 `diff65-fixture`（造仓脚本只放 scratchpad，不进仓库），只在它的标签页里操作。造出的现场先用 git 命令行核对过：`git diff -U0`（暂存区对比工作区）只有 `a.txt` 第 3 行与 `gone.txt` 的整份删除；`git diff --cached`（HEAD 对比暂存区）只有 `a.txt` 第 2 行与 `gone.txt` 第 2 行。

用户逐项实测，全部通过：

| # | 操作 | 结果 |
|---|---|---|
| 1 | Unstaged 里的 `a.txt`（两区都有） | ✅ 只有第 3 行 |
| 2 | Staged 里的 `a.txt` | ✅ 只有第 2 行，只高亮 Staged 那一行 |
| 3 | Unstaged 里的 `gone.txt`（暂存后又在工作区删掉） | ✅ 对着暂存区那一版的整份删除 |
| 4 | Staged 里的 `gone.txt` | ✅ 只有第 2 行 |
| 5 | 未跟踪的 `untracked.txt` | ✅ 整份新增 |
| 6 | `git mv` 的两项 | ✅ 一删一增各自都有 diff，没有「No changes」 |
| 7 | 选中 `untracked.txt` 后 Stage，再在 Staged 里按 Delete | ✅ 选中与高亮跟到 Staged；确认框点名的是它 |
| 8 | 两个区各自右键用 BC 打开 | ✅ 与应用内一致 |
| 9 | 切到提交图再切回来 | ✅ 回到同一个文件，两区的 diff 仍各不相同 |

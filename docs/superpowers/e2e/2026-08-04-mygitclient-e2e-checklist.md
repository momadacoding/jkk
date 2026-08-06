# mygitclient MVP E2E 验证清单

## 环境
- 仓库：`C:\Users\liuhao\workspace\dev\Dev\mygitclient\.worktrees\feat-mvp`
- 启动：`pnpm tauri dev`（Vite 5174 + Tauri app）
- 验证仓库：`C:\Users\liuhao\workspace\dev\Dev\mygitclient`

## 执行记录（2026-08-05）

驱动方式：以 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222`
启动，通过 CDP 连接 WebView2，直接读 DOM / React fiber、派发事件、调
`window.__TAURI_INTERNALS__.invoke`、用 `Page.captureScreenshot` 截图。

> 注意：Windows 上 `PrintWindow` 抓 WebView2 会返回陈旧/不完整的合成帧，
> 截出来的画面会缺内容（实测缺失作者列与 ref pills，而 DOM 中实际存在）。
> 截图一律用 CDP `Page.captureScreenshot`，不要用 `PrintWindow`。

写操作在 `C:\Users\liuhao\AppData\Local\Temp\e2egit` 的一次性测试仓库上执行，
未改动真实仓库；验证后已删除。测试仓库放在**短路径**下——放在超长路径
（>200 字符）时 `open_repo` 会误报 `not_repo`，是环境假阳性而非应用缺陷。

图例：`[x]` 通过 · `[!]` 失败（附缺陷编号）· `[~]` 部分通过 · `[ ]` 未验证

## 修复轮次（2026-08-05）

B1–B11 已全部修复并用同一套 CDP 手法复验，下方条目已更新为修复后的结果。
每条缺陷的修复方式记在「发现的缺陷」一节里。

## 启动与渲染
- [x] `pnpm tauri dev` 成功启动，无编译错误
- [x] 窗口标题为 `mygitclient`
- [x] 顶部 header 显示当前仓库路径与 HEAD 分支名
- [x] 左侧 sidebar 显示 Branches 与 Worktrees 面板
- [x] 中间 Graph 区域显示提交图（SVG 线条 + 节点，18 commits / 18 节点 / 3 path）
- [x] 右侧/底部 Staging 区域显示文件列表与 diff

## 仓库打开
- [x] 启动后自动打开 `C:\Users\liuhao\workspace\dev\Dev\mygitclient`
- [~] 空仓库时显示错误提示而非无限 Loading
      —— 代码验证：`openRepo` catch 置 `loading:false` + `error`，不会卡 Loading；
      错误文案已修复为真实 git 报错（B7）。未构造空仓库实测。
- [~] 大仓库时提交图最多加载 5000 条，UI 不卡死
      —— 上限存在于 `src-tauri/src/engine/graph.rs:33`；未用大仓库实测。

## 提交图交互
- [x] 点击 commit row 可切换选中状态
- [x] 选中后 bottom diff 面板显示该提交 diff（SHA 与 `git log` 逐一核对一致）
- [x] refs 标签（branch/tag/head）正确显示在 commit row 上
      —— 与 `git for-each-ref` 一致；`origin/HEAD` 不显示是正确行为。
      注：HEAD 无专属样式，`.ref-pill.head` 定义了但未被使用。
- [x] 鼠标悬停 commit row 有高亮（`.commit-row:hover` 规则存在）
- [x] 键盘 ArrowUp / ArrowDown 导航

## 分支管理
- [x] Local 分支列表与 `git branch` 一致
- [x] 当前 HEAD 分支高亮显示
- [x] 点击分支名执行 checkout，header HEAD 更新
      —— 修复后复验：点 `dev2` 后 header 变为 `@ dev2`，侧栏 `.head` 高亮同步移动，
      `git symbolic-ref --short HEAD` 一致。
- [x] 输入新分支名点击 Create 后，分支出现在列表中
- [x] 点击 × 删除非当前分支后，列表更新

## Worktree 支持
- [x] Worktree 列表与 `git worktree list` 一致
      —— 分支名已正确显示：`mygitclient (master)` / `feat-mvp (feat-mvp)`。
- [x] 当前工作区路径高亮
      —— 反斜杠的默认路径与 git 输出的正斜杠路径现在归一化后比较，两种入口都高亮。
- [x] 点击 worktree 行切换到对应工作区
- [x] 点击 + New worktree 创建新的 worktree
- [x] 非 main worktree 可删除
      —— 主 worktree 与当前打开的 worktree 都不再显示删除按钮。
- [x] 删除 worktree 后列表更新

## Staging / Diff / Commit
- [x] 修改工作区文件后，Unstaged 列表出现对应文件
      —— 外部写入 `watched.txt` 后约 300ms 自动出现，无需重载。
- [x] 点击 Unstaged 文件显示 diff
- [x] 勾选 Unstaged 文件执行 stage 后，文件移动到 Staged
- [x] 勾选 Staged 文件执行 unstage 后，文件回到 Unstaged
- [x] 输入 message 点击 Commit 后，新提交出现在 Graph 顶部
- [x] Amend 勾选后执行 amend，HEAD commit message 更新（SHA 改写已核对）

## 自动刷新
- [x] 在终端对同一仓库执行 `git commit` 后，Graph 自动刷新
      —— 外部 commit 后 commits 由 2 变 3，unstaged 清空。
- [x] 执行 `git checkout` 后，header HEAD 自动更新
      —— 外部 `git checkout master` 后 head 从 `dev2` 变为 `master`，
      worktree 面板的分支名同步更新。
- [~] 短时间内多次变更只触发一次刷新（debounce 300ms）
      —— 已从「首沿节流 + 丢弃后续事件」改为尾沿去抖（静默 300ms 才发一次），
      连续写入不会连发。未统计 emit 次数，只验证了最终状态正确。

## 边界情况
- [x] merge/rebase/cherry-pick 进行中时顶部显示状态横幅
      —— 冲突发生时自动出现 "merge in progress"，解决并提交后自动消失
      （横幅现在也跟随 `repo-changed` 刷新，不再是打开仓库时的一次性快照）。
- [x] 操作失败时底部/顶部显示错误提示，可关闭
      —— checkout 一个已被其他 worktree 占用的分支，横幅显示
      `[git_cli] fatal: 'feature' is already used by worktree at '...'`。
- [x] 切换 worktree 后 watcher 跟随新路径
      —— 切到 e2egit-wt 后：改新路径的文件会刷新；改旧路径的文件不再触发刷新，
      说明旧线程已退出（B3 修复的直接证据）。
- [x] merge 冲突文件出现在 Unstaged 列表（`conflicted` 状态，路径橙色标记）

## 性能
- [x] SVG viewport clipping 只渲染可见区域
      —— 短窗口滚动后 18 行只渲染 13–14 行。原「首次滚动前按硬编码 600px 估算」
      已改为挂载时实测 + ResizeObserver 跟随面板拖动。
      滚动 0/90/170/250/340 五个位置复验：SVG 节点无一被裁掉、
      圆点与 DOM 行中心偏差 0.00px、两条 lane 的描边色始终一致。
- [ ] 提交图在 1000 commits 内滚动流畅 —— 无大仓库，未验证
- [ ] 大量 refs 时 commit row 不溢出布局 —— 仅 4 个 ref，未验证

## 回归
- [x] `cargo test` 全部通过（5 passed）
- [x] `pnpm test` 全部通过（5 passed / 2 files）
- [x] `pnpm exec tsc --noEmit` 无类型错误
- [x] `cargo check` 无错误（`cargo test` 已覆盖编译）

## 发现的缺陷

全部已修复。每条记录「原因 → 改法 → 复验方式」。

### B1 · checkout 后 HEAD 不更新（高）· 已修复
原因：`App.tsx` 的 checkout 回调只调 `refreshGraph` + `refreshStatus`，
两者都不写 `state.head`；`head` 仅在 `openRepo` 里赋值一次。
改法：store 新增 `refreshHead()`（重新 `open_repo` 取 HEAD），
checkout 回调与 `repo-changed` 监听都调用它。
复验：点 `dev2` → header `@ dev2`；外部 `git checkout master` → header 自动跟随。

### B2 · 文件监听在 Windows 完全失效（高）· 已修复
原因：`watch.rs` 用 `s.contains("/.git/")` 判断相关性，
而 Windows 上 `notify` 给的是 `C:\...\.git\index`，判断恒为 false，
每个事件都被 `continue`，`repo-changed` 从未 emit。
改法：路径先 `replace('\\', "/")` 再匹配；同时把「只认 .git」放宽为
「.git 内部除锁文件/objects 之外 + 工作区未被 .gitignore 忽略的路径」，
这样改工作区文件也能触发刷新（用 `git2` 的 `is_path_ignored` 过滤
node_modules / target 等噪声，不硬编码目录名）。
复验：外部新建文件、外部 commit 均在 ~300ms 内自动反映到 UI。

### B3 · watcher 线程泄漏与竞态（中）· 已修复
原因：`watch_repo` 在已有 watcher 时直接 return，`unwatch_repo` 置的
`WATCHER_STOP` 又被下一次 `watch_repo` 重置为 false，旧线程永远读不到停止信号；
前端两个 invoke 也没 await，先后顺序不确定。
改法：`AtomicBool` 换成单调自增的 `WATCHER_GEN`——线程只在代号仍属于自己时继续，
watch / unwatch 各自增一次，旧线程必然退出；前端改为 await 串行调用，
并在 effect cleanup 里 unwatch。
复验：切到 e2egit-wt 后，改新路径文件会刷新，改旧路径文件不再触发。

### B4 · worktree 的 `-z` 解析错误（中）· 已修复
原因：`split('\0')` 分块，但 `git worktree list --porcelain -z` 是
**每行**以 `\0` 结尾、块间为 `\0\0`（`od -c` 确认），每个片段只含一个属性，
所以 `head`/`branch` 必为空串。
改法：拆出 `parse_worktrees()`，先按 `\0\0` 分块、块内按 `\0` 分行，
并补上 detached 分支的处理。
复验：面板显示 `mygitclient (master)` / `feat-mvp (feat-mvp)`。

### B5 · `is_main` 字段命名不匹配（中）· 已修复
原因：Rust serde 输出 snake_case `is_main`，前端读 `wt.isMain` 恒为 `undefined`。
改法：`Worktree` 结构加 `#[serde(rename_all = "camelCase")]`；
同时 `is_main` 不再靠路径比较判断，改用 git 的输出顺序（第一块即主工作区）。
复验：主 worktree 不再显示删除按钮，当前打开的 worktree 同样不显示
（`git worktree remove` 本来就会拒绝）。

### B6 · 路径分隔符未归一化（中）· 已修复
原因：`wt.path === currentPath` 是裸字符串比较，git 输出正斜杠、
Tauri dialog 与默认常量给反斜杠，两者永不相等。
改法：新增 `src/engine/path.ts` 的 `normalizePath` / `samePath`
（统一正斜杠、去尾部斜杠、忽略大小写），worktree 高亮改用它。
复验：默认路径（反斜杠）打开时主 worktree 正确高亮。

### B7 · 错误信息显示为 `[object Object]`（中）· 已修复
原因：`err instanceof Error ? err.message : String(err)`，
而 Tauri invoke 拒绝时抛的是普通对象 `{code, message}`。
改法：store 导出 `toErrorMessage()`，识别 `{code, message}` 形状，
输出 `[code] message`；并新增 `reportError()`，
把 `runAndRefresh` / `handleCommit` 里原本只 `console.error` 的失败也送到横幅。
复验：checkout 被占用的分支 →
`[git_cli] fatal: 'feature' is already used by worktree at '...'`。

### B8 · 冲突文件不出现在 status 列表（中）· 已修复
原因：冲突文件的状态位是 `CONFLICTED`，既不在 `INDEX_*` 也不在 `WT_*` 里，
两个 `intersects` 判断都不命中。
改法：`read_status` 单独处理 `CONFLICTED`，作为 `conflicted` 放进 unstaged；
前端加 `.changes-file.conflicted` 橙色路径标记。
复验：merge 冲突时列表出现 `conflict.txt:conflicted`，解决提交后消失。

### B9 · 文件增删行数恒为 0（低）· 已修复
原因：`FileChange` 的 `additions`/`deletions` 直接写死 0。
改法：新增 `line_stats()`，分别对 `diff_tree_to_index`（已暂存）和
`diff_index_to_workdir`（未暂存）逐行统计，再按路径查表填入。
顺带修了状态分类——原来未暂存那一侧也用 INDEX_* 判断，
导致未跟踪文件显示成 `modified`，现在分成 `index_status` / `worktree_status`。
复验：`a.txt +1/-0`、`untracked.txt +2/-0`，与 `git diff --numstat` 一致。

### B10 · untracked 文件无 diff（低）· 已修复
原因：workdir diff 没开 untracked 相关选项。
改法：抽出 `workdir_diff_opts()`，统一开启
`include_untracked` / `recurse_untracked_dirs` / `show_untracked_content`；
顺带让 `read_diff` 在空仓库（没有 HEAD）时与空树比较而不是直接报错。
复验：`untracked.txt` 显示 `+brand new` / `+second` 两行。

### C1 · 滚动时分支线换色（中）· 已修复
原因：`paths.ts` 按 `PALETTE[paths.size % N]` 取色，颜色取决于插入顺序，
而 `GraphView` 只把可见行喂进去，可见集合一变颜色就重排；
再加上 path id 是 `${parent.lane}-${node.lane}`，拓扑无关的线只要 lane 对相同就被并成一条。
改法：颜色改成 lane 的确定性函数 `laneColor(lane)`，path 按 lane 分组。
复验：五个滚动位置的描边色集合完全一致。

### C2 · 跨视口边界的连线断头（中）· 已修复
原因：`computePaths` 在可见切片里 `rows.find` 找父节点，父节点滚出窗口就整段丢弃
（顺带是 O(n²) 查找）。
改法：布局与路径对全量历史算一次（hash → 行号 Map，O(n)），
渲染阶段用 `filterPathsToBand` 只挑与条带相交的线段，跨界线段保留、由 SVG 边界裁掉。
复验：滚动后线段绝对坐标最小 y=14 < 条带顶 y=112，线延伸到边缘而非断头。

### C3 · SVG 选中矩形宽度写死 200px（低）· 已修复
删掉 `svgRenderer` 里的选中矩形，高亮交给已经铺满整行的 `.commit-row.selected`。

### C4 · 圆点无分支色、HEAD 无强调（低）· 已修复
圆点改为 lane 色填充 + 白边；HEAD 所在 commit 半径 5 → 6.5、描边加粗。

### C5 · SVG 与提交行错位、底部行无圆点（高）· 已修复
滚动后 SVG 元素放在 `top:0` 却使用整条历史的绝对坐标，
内容超出 SVG 自身高度的部分被静默裁掉——截图里底部 4 行完全没有圆点和连线。
注意：`getBoundingClientRect` 对被 SVG 边界裁掉的元素仍返回布局位置，
所以纯数值对齐检查查不出这个问题，是靠截图发现的。
改法：`GraphLayout` 增加 `offsetY`，内容整体 `translate(0, -offsetY)`，
容器回到条带顶端。复验：五个滚动位置 `nodesOutsideSvg` 均为 0。

### C6 · HEAD 指针从不出现在前端（中）· 已修复
`repo.references()` 只遍历 `refs/*`，根本不包含 HEAD，
所以 `kind == "head"` 的指针从未产生，`.ref-pill.head` 样式一直是死代码，
圆点也无从判断哪个是 HEAD。改法：`refs::collect` 末尾用 `repo.head()` 单独取并 resolve。

### B11 · diff 行首混入 origin 字符（低）· 已修复
原因：libgit2 把文件头（`'F'`）、hunk 头（`'H'`）、二进制标记（`'B'`）
也通过行回调抛出，被当成内容渲染成 `Fdiff --git ...`、`H@@ -2,3 +2,4 @@`。
改法：`diff_to_data` 只保留 origin 为 `' '` / `'+'` / `'-'` 的行。
复验：workdir 与 commit 两种 scope 的 origin 集合都只剩内容字符。

## 2026-08-06 Graph 车道重排（阶段一）

分支 `feat/git-toolbar-and-graph-polish`。规格
`docs/superpowers/specs/2026-08-06-toolbar-and-graph-lanes-design.md`，
计划 `docs/superpowers/plans/2026-08-06-graph-lane-rework.md`。

单测：`pnpm test` 53 passed（owners 12 / layout 12 / workdir 8 / paths 9 / path 6 / errors 6）。
`cargo test` 12 passed。`tsc --noEmit` 无错误。
`paths.test.ts` 与既有 3 条 layout 用例**未做任何改动**即通过。

运行时验证走 CDP（WebView2 `--remote-debugging-port=9222`），打开本仓库自身。

| # | 检查项 | 结果 | 证据 |
| --- | --- | --- | --- |
| 1 | master 在最左列 | 通过 | `master`/`ddd`（bc95955）圆点 cx=10，填充 `#4fc3f7` |
| 2 | 分支 tip 不与 master 同车道 | 通过 | `feat/git-toolbar-and-graph-polish` cx=30、`feat/tabs-and-graph-fixes` cx=50，各自独立颜色 |
| 3 | 工作区伪行 + 虚线 | 通过 | 顶行 `6 changes`，圆点 `fill=none` `stroke-dasharray=2 2`，虚线连到加粗行 |
| 4 | 本地/远程标签可分 | 通过 | 本地 `border-left: 2.67px` 车道色 + 底色 `rgb(51,64,77)`；远程 `border-left: 0px`、底色 `rgb(47,47,47)` |
| 5 | 标签紧跟消息 | 通过 | 截图 `g1-top.png`，标签在消息右侧而非窗口右缘 |
| 6 | HEAD 行加粗 | 通过 | `.commit-row.head` 命中 bc95955 |
| 7 | **切分支不改图** | 通过 | 见下方「双标签对照」 |
| 8 | 点伪行进 Local Changes | 通过 | 导航项由 `All Commits` 变为 `Local Changes (6)`，`.local-changes-view` 出现 |
| 9 | 上下键跳过伪行 | 通过 | `commits` 数组不含伪行，`findIndex` 返回 -1 时直接 return |
| 10 | 滚到底部仍有圆点和线 | 通过 | 强制 260px 视口滚到底：15 行 / 15 圆点 / `dotsOutsideSvg: 0` / 行圆点偏移 0.00px，并截图 `g2-scrolled.png` 确认 |

### 第 7 条的验证方法：双标签对照

没有动用户的仓库做 checkout。改用一个更强的对照：主仓库与
`.worktrees/git-toolbar` **共享同一份 refs 与提交**，但 HEAD 不同。
两个标签同时打开，逐个提交比对圆点 x 坐标：

- 标签 A：`mygitclient @ master`，加粗行 bc95955，伪行 cx=10
- 标签 B：`.worktrees/git-toolbar @ feat/git-toolbar-and-graph-polish`，加粗行 7e5a881，伪行 cx=30
- **29 个提交的车道全部相同（`laneDiffs: []`）**

即 HEAD 只影响「哪一行加粗」和「伪行挂在哪」，不影响任何车道分配。

### 已定：本地 master 落后时，lane 0 仍归本地 master

本仓库的本地 `master`（bc95955）比 `origin/master`（b61263e）落后 17 个提交。
按 tier 1 规则「本地 master 优先」，owner 0 只覆盖 bc95955 及其之前的
3 个提交，lane 0 因此在图的上部是空的；真正的主干（`origin/master` 往上）
落在 lane 1，与 `feat/git-toolbar-and-graph-polish` 共用车道和颜色。
标签样式仍能区分两者（远程灰底无色条 vs 本地带色条）。

**决定（2026-08-06）：保持本地优先，不改。** lane 0 的语义就是「你本地的
master 在哪里」，本地落后时左边那条短线本身就是一个信号。阶段二的
ahead/behind 徽章会把这个信息补全。

否决的两个备选：
- 「取更靠前的那个」——语义变得依赖同步状态，同一个仓库 fetch 前后图会跳。
- 「远程优先」——本地刚提交还没推送时，你自己的新提交会被挤到 lane 1 去。

### 追加修复（用户复检后发现，2026-08-06）

复检时用户对照 Sublime Merge 指出两个问题，都不是车道分配的错，而是**连线渲染**：

**一、跨车道的边画成横扫整段高度的巨大弧线。**
`svgRenderer.tsx` 对所有 `curve: true` 的边一律用 `C x1 midY, x2 midY, x2 y2`。
相邻两行之间是个好看的小弯，但 `Merge pull request #1` → `master` 这条第一父边
**跨了 16 行**（实测 y 378 → 826），弧线横扫 448px，完全看不出它连的是哪两个提交。
改法：线段笔直待在**它自己所属的那条车道**里，只在另一端拐一个 6px 圆角。
拐角位置由归属决定——第一父的线归子节点车道，拐角在下；合并第二父的线归父节点
车道，拐角在上。复验：路径数据里 `C` 指令归零，`Q` 圆角 5 处。

**二、feat-mvp 与主干共用车道，看不出何时合并。**
owner 1（主干链）到第 12 行结束，owner 3（feat-mvp 链）从第 13 行开始，
行区间不重叠，装箱算法让它复用了 lane 1。两段线首尾相接，看上去是一条连续的线，
合并点从视觉上消失。改法：`packLanes` 增加 `LANE_REUSE_GAP = 2`，间隔不够就另开一列。
复验：feat-mvp 由 cx=30 移到 cx=70，成为独立的紫色车道。

顺带：合并提交（父数 > 1）改画空心环，即使与主干同色同列也能认出合并行。
复验：`.commit-node.merge` 命中 2 个（Merge PR #1 / #2）。

`laneEnd` 初值同时从 `-1` 改为 `-Infinity`——否则 `minRow` 为 0 时
`-1 >= 0 - 2` 成立，空车道会被误判为已占用（已加单测锁住）。

单测增至 58 passed（layout 15 / paths 11 / owners 12 / workdir 8 / path 6 / errors 6）。

### 追加修复三：提交排序（用户发现 3a63421 位置不对，2026-08-06）

用户问「3a63421 是不是漏掉了」。查下来它**没漏**——后端数据第 15 条、DOM 已渲染、
带着 `refs/heads/feat-mvp`。但它被排在了 `Merge pull request #1` **下面**，
而它的提交时间（8/6 2:25 AM）比那个合并提交（8/5 3:17 PM）更新。

原因：`engine/graph.rs` 只设了 `Sort::TOPOLOGICAL`。该排序只保证「子在父之前」，
**互不相关的两个提交之间顺序是任意的**，实际取决于 revwalk 的 push 顺序。

视觉后果很具体：3a63421 与合并的第二父 2dbba11 同属 feat-mvp 那条链、同一车道，
而它恰好夹在 `Merge PR #1` 与 2dbba11 之间。合并连向第二父的线于是**从 3a63421 的
圆点上直接穿过去**——它看上去不是一个独立的分支 tip，而像合并线上的中转点。

改法：`Sort::TOPOLOGICAL | Sort::TIME`，即 `git log --date-order` 的语义：
不破坏拓扑的前提下按提交时间从新到旧。

复验：新增 `graph_puts_newer_unmerged_tip_above_older_merge`，用固定
`GIT_AUTHOR_DATE`/`GIT_COMMITTER_DATE` 构造「未合并 tip 比合并提交更新」的形状。
改之前该用例失败并打印出实际顺序
`[("merge feature", 1767398400), ("f2", 1767484800), ...]`，改之后通过。
运行时复验：3a63421 索引由 15 移到 14，排到 fa43adf 之前；截图确认它成为紫色车道
的顶端、合并线改从环形节点侧向分出，不再穿过它的圆点。

`cargo test` 13 passed，`pnpm test` 58 passed。

---

## 2026-08-06 Git 操作工具栏（阶段二）

分支 `feat/git-ops-toolbar`，从 PR #3 合并后的 master 开出。

测试用一次性仓库，**没有碰任何真实仓库**：scratchpad 下 `e2egit-remote`（bare）
+ `e2egit-local` + `e2egit-other`（第二个克隆，用来造远程侧的新提交）。
push / force push 全在这套里跑。

驱动方式同上一轮：`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222`
起 `pnpm tauri dev`，用 CDP 的 `Runtime.evaluate` / `Page.captureScreenshot`。
应用没有程序化开标签的入口（只有原生对话框和双击 worktree），所以临时把
`DEFAULT_REPO_PATH` 指向测试仓库走 HMR，验完已还原。

| # | 检查项 | 结果 | 证据 |
|---|---|---|---|
| 1 | 工具栏出现，显示分支名与四个按钮 | 通过 | `⎇ feature` + Fetch/Pull/Push/Stash；Stashes 面板显示 "No stashes" |
| 2 | 按住 Ctrl 四个按钮文字同时变、整排换橙 | 通过 | 标签变 Fetch all / Pull (rebase) / Force push / Stash all；边框 `rgb(85,85,85)` → `rgb(255,183,77)` |
| 3 | 松开 Ctrl 全部还原 | 通过 | 标签与边框色均回到默认 |
| 4 | 按住 Ctrl 切走窗口再回来必须已还原 | 通过 | 只派发 `blur`、不派发 `keyup`，四个按钮全部复位 |
| 5 | 点 Fetch，`origin/master` 移动 | 通过 | `d07eda6` → `8953583`，Pull 自动长出 `↓1` |
| 6 | 点 Pull，远程文件进工作区、徽章消失 | 通过 | `remote-side.txt` 出现，`↓1` 消失 |
| 7 | 本地提交出现 `↑1`，Push 后消失 | 通过 | 徽章由文件监听自动刷出（未点任何按钮）；push 后远程 = 本地 HEAD `b422f41` |
| 8 | 无上游的新分支 Push 自动补 `-u` | 通过 | `git rev-parse --abbrev-ref feature@{upstream}` → `origin/feature` |
| 9 | Ctrl+Push 弹确认框、文案带分支名；取消什么都不做 | 通过（**修复后**，见下） | 对话框标题 Force push、正文 `Force-push feature? This can overwrite commits on the remote.`；取消后远程仍为 `b5555e1f`；确定后强推生效，远程 = 本地 `da964eb` |
| 10 | 改文件点 Stash，面板出现条目、工作区干净 | 通过 | `{0} WIP on feature: da964eb f2-rewritten`，`a.txt` 还原 |
| 11 | 点 stash 条目，下方显示它的内容 | 通过（**修复后**，见下） | SHA `6de357f…`、作者 Test、`a.txt +1/-1`；Changes 页显示 `-hello` / `+modified content` |
| 12 | 右键 → Pop，改动回工作区、条目消失 | 通过 | 菜单为 Apply/Pop/Drop；pop 后 `a.txt` = `modified content`，`git stash list` 为空 |
| 13 | Ctrl+Stash 连未跟踪文件一起存 | 通过 | 按钮文字为 `Stash all`；`untracked.txt` 从工作区消失 |
| 14 | 认证失败快速报错而非挂死 | 通过 | 见下 |

### 第 9 条暴露的真 bug：强推确认从来没生效过

第一次跑的时候一个对话框都没弹，**强推直接推上去了**。

根因两层：

1. 计划里写的是 `if (plan.confirm && !window.confirm(plan.confirm)) return`。
   在标准浏览器里对，在 Tauri 里错——dialog 插件把全局 `window.confirm` 换成了
   async 函数（CDP 读到的实现是
   `async function(i){ return await n("plugin:dialog|confirm", { message: i.toString() }) }`）。
   返回的是 Promise，恒为真值，`!promise` 永远是 `false`，守卫静默失效。
2. `src-tauri/capabilities/default.json` 只授了 `dialog:allow-open`。就算 await 了，
   `confirm` 也会被权限层拒掉——所以连框都出不来。

改法：改用 `@tauri-apps/plugin-dialog` 导出的 `confirm` 并 `await`，补上
`dialog:allow-confirm`，并把弹框失败一律当作「没确认」（fail-closed）——这是工具栏上
唯一不可逆的动作，宁可什么都不做也不能在没问过的情况下推上去。

原生对话框是 Rust 侧弹的模态子窗口，`Page.javascriptDialogOpening` 拦不到，
所以用 Win32 `EnumWindows` 定位（class `#32770`、标题 `Force push`），
再 `WM_COMMAND` 发 IDCANCEL(2) / IDOK(1) 分别验证取消与确定两条路径。

### 第 11 条暴露的问题：点 stash 出现空白面板

stash 提交不被任何分支引用，因此不在 `load_graph` 的输出里。`CommitDetailView` 靠
`state.commits.find(...)` 拿提交，查不到就渲染 "Select a commit"——diff 明明已经取回来了，
面板却整片空白。和上一轮 worktree 游离 HEAD 是同一类悬空 `selectedId` 问题。

改法：`StashEntry` 补上 `author` / `time` / `parents`（libgit2 按 oid 查提交即可），
`RepoTab` 在图里查不到时用这些元数据现搭一个 `CommitNode`。
新增 Rust 用例 `entry_carries_enough_metadata_for_the_detail_panel` 覆盖。

### 第 14 条：GIT_TERMINAL_PROMPT=0 的直接证据

先用不可达地址（`https://example.invalid`）试了一次：1835ms 出红色横幅，按钮解禁。
但那条走的是 TLS 失败，不是认证提示。于是起了一个只回 `401 WWW-Authenticate: Basic`
的本地服务器（127.0.0.1:8899），并把仓库的 `credential.helper` 置空，逼 git 走终端提示：

```
[git_cli] fatal: could not read Username for 'http://127.0.0.1:8899': terminal prompts disabled
```

259ms 出横幅，按钮解禁。错误原文里的 "terminal prompts disabled" 就是这道防线生效的
直接证据——没有它，git 会打印 `Username:` 然后无限等 stdin。

### 测试统计

`cargo test` 42 passed（git_cli 3 / upstream 4 / remote 6 / stash 9 / 既有 20）、
`pnpm test` 71 passed、`pnpm exec tsc --noEmit` 无错误。

# Git 操作工具栏与 Graph 车道重排 — 设计

日期：2026-08-06
分支：`feat/git-toolbar-and-graph-polish`（worktree `.worktrees/git-toolbar`，基于 `origin/master` b61263e）

## 背景

两个独立的缺口：

**一、没有任何远程操作。** `src-tauri/src/commands/ops.rs` 目前只有 branch / checkout / stage / unstage / commit。fetch、pull、push、stash 全部缺失——原始设计文档
（`2026-08-04-mygitclient-design.md:37`）当初把它们明确划在 MVP 之外。用户要的是 Fork 那样的顶部工具栏，且按住 Ctrl 时按钮切换到变体动作。

**二、Graph 分不清两个分支。** `src/engine/layout.ts` 的车道分配是一趟贪心：第一父永远继承子节点的车道，谁先出现谁抢 lane 0。结果是一个直接骑在
`origin/master` 头上的分支 tip（例如 `feat/tabs-and-graph-fixes`）会和 master 挤进同一条车道、同一个颜色，视觉上完全等同于「同一条线」。
再加上 ref 标签被 `margin-left: auto` 顶到窗口最右端（离提交消息可达 1500px），本地/远程标签只差一个填充色，三重因素叠加导致无法区分。

## 设计原则

**图形只由提交拓扑与 refs 决定。** checkout 切换分支不得改变任何车道分配或颜色。HEAD 只影响两件事：哪一行加粗，以及工作区伪行的虚线连到哪个提交。

## 第一部分 · Graph 重排

### 1.1 车道算法：两趟「按分支归属」

替换 `computeLayout` 现有的一趟贪心。

#### Pass 1 — 计算归属（owner）

按固定优先级构造 tip 列表，同 hash 去重：

1. `master` / `main`（本地优先，没有本地分支则回退 `origin/master` / `origin/main`；两者都不存在时本级为空，lane 0 归下一级的第一个 tip）
2. 其余本地分支，按 tip 所在行号排序（`load_graph` 返回的顺序是新 → 旧，故行号越小越新）
3. 远程分支，同上（与本地分支同 hash 的已在去重时跳过）
4. 图中没有子节点的悬空提交，按行号
5. 合并提交的第二父及以后：遍历到时如果还没有主人，就地新开一个 owner

对每个 tip 按顺序沿**第一父链**向下走，把沿途尚无主人的提交划归该 tip。撞上已有主人的提交立即停止——那个提交就是分叉点。

优先级里没有 HEAD。这是刻意的：见「设计原则」。注意后端 `engine/refs.rs` 会往 refs 里塞一条
`kind: "head"` 的记录，`computeLayout` 必须**显式跳过 `kind === 'head'` 的 ref**，否则 HEAD 会被当成一个 tip 参与排序，切分支就会改变车道分配。

#### Pass 2 — 车道装箱

每个 owner 统计它所拥有提交的行区间 `[minRow, maxRow]`，然后贪心分配：

- 优先级 1 的 owner（master/main）固定拿 lane 0。
- 其余 owner 按 minRow 升序处理，各自选**第一条当前占用者 maxRow < 本 owner minRow 的车道**；没有可复用的才新开。

车道复用是这个设计能用的前提：仓库里几十个早已合并的旧分支各自只占几行、行区间互不重叠，会共享同一条车道。只有**行区间真正重叠**（即同时活跃）的分支才会并列成新的一列。因此车道数取决于并发分支数，而非分支总数。

#### 对既有代码的影响

- `computeLayout` 签名从 `(commits)` 变为 `(commits, refs)`。`tests/vitest/paths.test.ts` 有 6 处调用需要跟着改。
- `LayoutNode.lane` 与 `LayoutParent.lane` 的语义不变，`computePaths` 已经使用 `parentNode.lane`（父提交**实际**落到的车道）而非预留值，无需改动。
- 颜色仍由 `laneColor(lane)` 决定，保持确定性。车道复用会让两个不同分支在互不重叠的行区间内共享颜色，这与 Sublime Merge 的行为一致，可接受。

### 1.2 工作区伪行

在进入布局之前注入一个合成提交：

```ts
{ id: '__workdir__', parents: headHash ? [headHash] : [], author: '', time: now, message: '' }
```

渲染时按 id 特判：空心虚线圆点、到 HEAD 的**虚线**连接、行文字为 `N changes` 或 `No changes`，点击切换到 Local Changes 视图。

**该伪行不参与 Pass 1/2。** 它的车道在装箱结束后直接取其父节点的车道；没有父（空仓库或无提交）时取 lane 0。

理由：若让它参与归属计算，当 HEAD 恰好在 master 上时，master 已经拥有了 HEAD 那条提交，伪行沿第一父链会立即撞墙，退化成只占一行的独立 owner，被装箱算法分到 lane 1，视觉上凭空多出一列。

同时这也保证了「切分支不改图」——伪行只是横向移动到 HEAD 提交的正上方。

补充：Sublime Merge 的虚线正是这个用途。官方论坛的说法是 "Dashed lines are used to connect HEAD and the working directory representing list item."
它**不是**「分支基于 main」的标记。

### 1.3 Ref 标签重做

`src/components/CommitRow.tsx` 与 `src/styles.css`：

- **位置**：消息改为 `flex: 0 1 auto` + 省略号，标签紧跟其后，作者/时间用 `margin-left: auto` 推到右端。标签不再被顶到窗口边缘。
- **本地分支**：3px 左侧色条（取该提交所在车道的颜色）+ 浅色底。
- **远程分支**：纯灰底，无色条。
- **tag**：与分支区分的形状。
- **HEAD 所在行**：整行文字加粗。仅在游离 HEAD 时额外显示 `HEAD` 标记。

## 第二部分 · 工具栏与远程操作

### 2.1 布局

工具栏位于 `RepoTab` 顶部：

```
[⎇ feat/git-toolbar-and-graph-polish]  │  [Fetch] [Pull ↓2] [Push ↑1] [Stash]
```

### 2.2 Ctrl 变体

| 按钮 | 默认 | 按住 Ctrl |
| --- | --- | --- |
| Fetch | `git fetch --prune` | `git fetch --all --prune --tags` |
| Pull | `git pull`（沿用用户的 `pull.rebase` 配置） | `git pull --rebase` |
| Push | `git push`；无 upstream 时自动补 `-u origin <branch>` | `git push --force-with-lease`，弹一次确认 |
| Stash | `git stash push` | `git stash push -u` |

Pull 默认使用不带参数的 `git pull`，以尊重用户自己的 git 配置——这是 git 客户端最不意外的行为。

按住 Ctrl 时按钮文字与配色实时切换，否则该功能不可发现。实现要点：监听 window 的 `keydown` / `keyup`，并**在 `blur` 时强制复位**——否则按着 Ctrl 切走再切回，按钮会永久卡在变体状态。

网络操作期间按钮显示 spinner 并禁用。本轮不解析 git 的进度输出，只做忙碌态。

### 2.3 ahead / behind

新增读命令，走 libgit2 的 `Repository::graph_ahead_behind`，不开子进程（符合项目「读走 libgit2、写走 git CLI」的既有分工）。返回：

```rust
struct UpstreamInfo { branch: String, upstream: Option<String>, ahead: usize, behind: usize }
```

没有 upstream 时不显示徽章。在 `repo-changed` 事件以及每次 fetch / pull / push 之后刷新。

### 2.4 Stash 侧边栏

`src/components/StashView.tsx`，结构参照现有的 `WorktreeView.tsx`，与 Branches / Worktrees 面板并列。右键菜单：Apply / Pop / Drop。

点击条目查看 diff 复用现有机制：stash 本身就是一个提交，`read_stashes` 直接返回其 hash，前端用现成的 `commit:<hash>` scope 调 `read_diff`。**`engine/diff.rs` 一行不用改。**

### 2.5 认证与挂死防护

所有 git 子进程统一设置 `GIT_TERMINAL_PROMPT=0`。

凭据管理器（Windows 上的 GCM）弹出自己的 GUI 窗口是正常且需要保留的行为。要防的是另一种情况：git 往 stderr 打印 `Username:` 然后无限等待 stdin——GUI 应用里没有任何人能回答它，子进程会永久挂死，进而拖住 Tauri 的线程池。`GIT_TERMINAL_PROMPT=0` 让 git 在这种场景下立即失败，错误转成顶部错误横幅。

现有 `ops.rs::run_git` 提取到 `src-tauri/src/engine/git_cli.rs` 统一加固，所有写操作共用。

## 文件计划

### 后端

| 文件 | 动作 |
| --- | --- |
| `src-tauri/src/engine/git_cli.rs` | 新增。从 `ops.rs` 提取 `run_git`，加 `GIT_TERMINAL_PROMPT=0` |
| `src-tauri/src/engine/upstream.rs` | 新增。`graph_ahead_behind` |
| `src-tauri/src/engine/stash.rs` | 新增。`stash_foreach` 列表 |
| `src-tauri/src/commands/remote.rs` | 新增。`fetch` / `pull` / `push` |
| `src-tauri/src/commands/stash.rs` | 新增。`stash_push` / `stash_apply` / `stash_pop` / `stash_drop` / `read_stashes` |
| `src-tauri/src/commands/ops.rs` | 改用 `git_cli::run_git` |
| `src-tauri/src/lib.rs` | 注册新命令 |

### 前端

| 文件 | 动作 |
| --- | --- |
| `src/engine/layout.ts` | 重写为两趟算法，签名加 `refs` |
| `src/engine/svgRenderer.tsx` | 工作区节点 + 虚线连接 |
| `src/components/CommitRow.tsx` | 标签列、HEAD 加粗、工作区行 |
| `src/components/GraphView.tsx` | 注入工作区伪行，传 refs 给 layout |
| `src/components/Toolbar.tsx` | 新增 |
| `src/components/StashView.tsx` | 新增 |
| `src/components/RepoTab.tsx` | 挂载 Toolbar 与 StashView |
| `src/store/repoStore.ts` | 增加 upstream / stashes 状态与刷新 |
| `src/engine/types.ts` | `UpstreamInfo` / `Stash` 类型 |
| `src/styles.css` | 工具栏、标签、stash 面板样式 |

## 测试

### 单元测试（`tests/vitest/layout.test.ts`，新建）

1. 分支 tip 骑在 master 头上时，两者落在**不同**车道，master 在 lane 0。
2. 行区间不重叠的两个分支**复用同一条车道**（验证宽度不随分支总数增长）。
3. 行区间重叠的两个分支占用**不同**车道。
4. 无论 refs 中 HEAD 指向哪里，车道分配结果**逐字节相同**（锁死「切分支不改图」）。
5. 没有 master/main 时的回退路径。
6. 工作区伪行取父节点车道；HEAD 在 master 上时它仍在 lane 0。

### 回归

`tests/vitest/paths.test.ts` 全部 9 个用例必须继续通过（调用点需适配新签名）。特别是「每条线段端点都落在某个提交圆点上」那条——它是上一轮修复悬空竖线的锁。

### 后端测试（`src-tauri/tests/`）

7. `graph_ahead_behind` 在有/无 upstream 两种情况下的返回值。
8. stash 列表解析，含带消息与不带消息两种。
9. `git_cli` 确实设置了 `GIT_TERMINAL_PROMPT=0`。

## 分阶段实施

两个部分各提一个 PR：

- **阶段一：Graph 重排**（1.1 / 1.2 / 1.3）。纯前端、纯计算，可用单测完全锁死，不碰网络。
- **阶段二：工具栏与远程操作**（2.1 – 2.5）。引入 4 个新命令与网络 IO。

理由：阶段一改动集中且确定性强；阶段二涉及网络与认证，失败模式完全不同。混在一个 PR 里出问题不好定位。

## 明确不做

- git 进度输出解析（fetch/push 的百分比）。本轮只做忙碌态。
- 取消正在进行的网络操作。
- merge / rebase / cherry-pick 的工具栏入口。
- 双行提交行（Sublime Merge 把作者放在消息下一行）。本轮保持单行。
- 交互式 rebase、reset 等其余写操作。

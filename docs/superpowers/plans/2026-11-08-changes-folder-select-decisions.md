# 本地改动：文件夹可选中 — 实现日志

没有单独的计划文件。方案是在对话里定的（方案 A，用户 2026-11-08 选定）：

- 文件夹行本身可以选中，和 Fork 一致
- 单击行选中；单击箭头展开 / 收起；双击 Stage / Unstage 整个文件夹
- 右键菜单与文件行相同，作用于文件夹下所有文件（含收起的部分）
- Shift / Ctrl / 拖选时文件夹也算一行

## 进度总账

| Task | 内容 | 状态 | 产出文件 | 验证 |
|---|---|---|---|---|
| T1 | engine：行 key（文件夹 `dir/`）、选中展开成文件、对齐 / 默认项 / diff scope 认文件夹 | 完成 | `src/engine/fileSelection.ts`、`tests/vitest/fileSelection.folder.test.ts` | 新测试先 18 个失败，实现后 19 + 原有 47 全过 |
| T2 | 组件：文件夹行选中、箭头单独收起、右键、双击、拖选、Ctrl+A、Delete、区头按钮、diff 面板说明 | 完成 | `src/components/LocalChangesView.tsx`、`src/styles.css` | `pnpm build` 通过 |
| T3 | RepoTab 对齐选中改用行列表 | 完成 | `src/components/RepoTab.tsx` | `pnpm build` 通过 |
| T4 | 组件测试 + 真实窗口验证 | 组件测试完成；真实窗口待用户看 | `tests/vitest/LocalChangesView.test.tsx`（「选中文件夹」一组 + 改了 6 条旧用例） | `pnpm test` 80 files / 1364 tests 全过 |

## 决策条目

### T1

- **选中里放「行」，不另开一个文件夹字段**：`FileSelection.paths` 里文件是路径，文件夹是 `目录/`。
  锚点、Shift 范围、Ctrl 切换、拖选、对齐都只是在一个有序列表上算，原来的函数不用改，换传行列表即可。
  另开 `dirs` 字段的话，Shift 从文件拖到文件夹的范围就要在两个列表之间拼。
- **为什么 `目录/` 不会和文件撞**：后端 `read_status` 开着 `recurse_untracked_dirs(true)`
  （`src-tauri/src/engine/diff.rs`），未跟踪目录也逐个文件列出，文件路径不会以斜杠结尾。
- **动作只认文件**：stage / unstage / discard 一律先 `selectedFiles` 展开，不把 `目录/` 交给 git。
  也考虑过直接把目录交给 `git add -- dir`，但 Unstaged 里 `dir` 下的文件和 `git add dir` 实际会碰的不一定一致
  （比如冲突文件要先确认，`describeStageConflicts` 按文件判断），刷新时的 `touched` 也要文件列表。
- **展开含收起子文件夹里的文件**：与原来「选中只含看得见的文件」的规则看似冲突。原规则防的是
  Shift 选一段把没点到、看不见的文件夹进来；选中文件夹是用户点了那一行、它高亮着，作用到它下面的全部文件是用户要的。
  收起文件夹时，它**里面**的行照旧退出选中（`collapseDir` 不变，只是换成按行过滤）。
- **Ctrl+A 选看得见的行，文件夹行也在里面**：偏离原来「Ctrl+A 不选收起文件夹里的文件」。收起的文件夹那一行看得见、
  会被选中、会高亮，按下去 Stage / Delete 会作用到它下面的全部文件。和 Fork 一致；Delete 仍要过丢弃确认框（列出文件）。
- **最后点的是文件夹：右侧不读 diff**。`changesDiffScope` 返回 `null`，组件显示「选中了文件夹 X，N 个文件」的说明。
  一次读整个文件夹的 diff 会退回 6.13 方案 C 之前「整区读」的开销，而右侧本来就只显示一个文件。
- **默认项跳过文件夹行**：没选时右侧显示第一个**文件**；行列表里文件夹排在它的内容前面，不能直接取 `rows[0]`。
- **保留 `changesPaths`**（只给文件）：现有测试和「整区 Stage All」还要文件列表；新增 `changesRows` 给对齐选中用。

### T2

- **展开 / 收起只认箭头**（`.changes-dir-toggle`）：单击行变成选中。箭头只有 10px，用内边距撑大点击区域、等量负外边距抵掉，
  文件夹名和文件名仍对齐。箭头上的 mousedown / dblclick 都 `stopPropagation`：快速点两下箭头是「收起又展开」，
  冒到行上就成了「双击暂存整个文件夹」。
- **双击文件夹 = Stage / Unstage 它下面的全部文件**（含收起的子文件夹）。原有测试「双击目录行不暂存任何东西」按方案 A 改写。
- **Shift / 拖动跨过收起的文件夹：那一行进选中**。偏离原有测试「Shift 选一段跳过收起目录里的文件」。
  原规则防的是看不见的文件被夹进来；现在被夹进来的是看得见、高亮着的文件夹行，和 Fork 一致——「高亮了什么就动什么」。
  它里面的行不单独进选中（展开后不会显示为选中）。
  **为什么这么改是安全的**：Stage / Unstage 可逆；Discard（右键或 Delete）都走 `describeDiscard` 确认框，
  逐个列出要丢的文件、未跟踪文件单独红条，默认焦点在 Cancel。
- **右键菜单**：数字 `(n)` 是展开后实际会动到的文件数；Copy path 复制选中的行本身（文件夹给文件夹路径，不展开）；
  文件夹行不给外部 diff / merge 工具（没有单个文件可对比）。右键文件夹前 WebView 自带菜单会弹出来，是因为文件夹行原来没有 `onContextMenu`。
- **Delete**：先 `selectedFiles` 展开再交给 `discardShortcut`，与右键 Discard 丢的是同一批。`discardShortcut` 本身不改。
- **右侧**：最后点的是文件夹时显示「已选中文件夹 X，N 个文件」，不读 diff。
- **样式**：`.changes-dir.selected` / `.changes-dir.active::before` 照文件行写了一份，仍用 `--selected-bg`。
  PR 135（文件行改实色 `#3a6ea5`）合并后要跟着改文件夹行，否则两种行选中色不一样——rebase 时处理。

### T3

- RepoTab 状态变化时对齐选中改用 `changesRows`。只用文件列表的话，选中的文件夹每次刷新都会被当成「不在列表里」清掉。
- `changesDiffScope` 对文件夹返回 `null`：`useRepoEffects` 在 scope 为 null 时不发读取（与「两边都空」同一路径），
  `currentDiff` 给 null，组件在判断「正在读取」之前先判断是不是文件夹，不会误报正在读取。

### T4

- 组件测试里 6 条旧用例改了：4 条把「点目录行收起」换成点箭头（行为本意不变）；
  「双击目录行不暂存」「Ctrl+A 跳过收起目录」「Shift 跳过收起目录里的文件」按方案 A 改写断言（见 T1 / T2 的偏离说明）。
- **计划没写的坑**：多选时 active 是**显示顺序里的最后一个**，不是最后点的那个（Ctrl 多选会按显示顺序重排，
  `nextFileSelection` 的 `sortByList`）。第一版测试按「最后点的」写错了，已改。
- 组件测试是在实现组件之前写的，但写完没有先跑一遍看它失败（engine 那组是先看到 18 个失败再实现的）。

### 独立 review（`/code-review high`，PR 137）

10 条，第 1、2、3、5、6、7、8、10 条修了（补的 8 条测试先全部失败再修），第 4、9 条接受并说明。

| # | 问题 | 处理 |
|---|---|---|
| 1 | 收起任意文件夹时 `toggleDir` 传给 `onCollapse` 的还是**文件**列表，选中的文件夹行全被清掉。决策日志 T1 写了「改成按行过滤」，这一处漏了 | 改用 `treeRowKeys`。补测试：选中文件夹再收起它 / 收起别的文件夹，仍选中 |
| 2 | Shift / 拖选跨过**展开着的**文件夹头，那一行进选中，动作覆盖整个文件夹，包括范围之外、没扫到也没高亮的文件 | **偏离 T2 的原决定**：范围中间扫过的展开文件夹头不进选中（端点是用户点的，照算）；Ctrl+A 同样不选展开着的文件夹头。收起的文件夹头照旧进选中——那一行代表它的全部内容。另加 `in-selected`：选中的文件夹下面的行用淡一档的底色标出，看得出动作会覆盖哪些文件 |
| 3 | 文件夹整个暂存走后，选中按 key 跟到另一个区的同名文件夹，那边可能装着更多文件，再按 Delete / Unstage 会动到没选过的 | 对齐时只有文件跨区跟随，文件夹行不跟。代价：整个暂存一个文件夹后选中清空，不能「马上整个 Unstage 回去」 |
| 4 | 选中的文件夹之后又多出新文件，动作会带上它们 | **接受**：文件夹选中的语义就是「这个文件夹」，Fork 同样如此。Discard 的确认框逐个列出文件、未跟踪文件单独红条、默认焦点在 Cancel |
| 5 | `discardShortcut` 对展开后的上万个文件逐个 `includes`，平方级 | 改用 Set |
| 6 | 每次按键都展开整棵树 | 先判断是 Delete 再展开 |
| 7 | 右侧「已选中文件夹」的文件数每次渲染都走一遍树 | `useMemo` |
| 8 | `changesRows` 每次调用都走一遍树，一次刷新要调 4 次 | 按树 `WeakMap` 缓存（树本身已按列表缓存） |
| 9 | `FileSelection.paths` 混着文件和 `目录/`，靠注释约束「交给 git 之前先展开」 | **接受，记录**：已核对全部出口——右键菜单、区头按钮、Delete、双击都经过 `expandRowKeys` / `selectedFiles`，`repoStore` 的 `touched` 收到的是展开后的文件。改成带类型的 RowKey 要动 `fileSelection` 全部函数签名和 RepoTab，超出本 PR |
| 10 | 双击文件夹用 `treeFilePaths(node.children)`，和其他入口不是同一套展开 | 统一用 `expandRowKeys([key], tree)` |

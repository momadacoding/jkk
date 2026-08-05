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

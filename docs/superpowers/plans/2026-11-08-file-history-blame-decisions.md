# 文件历史与 Blame：决策日志

计划：`2026-11-08-file-history-blame.md`

## 进度总账

| Task | 状态 | 产出 | 验证 |
|---|---|---|---|
| 1 后端 | 完成 | `engine/file_history.rs`、`commands/history.rs`、`tests/file_history.rs`；`diff_to_data_only` 改 `pub(crate)` | `cargo test --test file_history` 20/20，lib 单测 2/2，clippy 无告警 |
| 2 前端纯逻辑 | 完成 | `src/engine/fileHistory.ts`、`tests/vitest/fileHistory.test.ts` | vitest 17/17 |
| 3 store + 视图 | 完成 | `repoStore.ts` 三个包装、`FileHistoryView.tsx`、`styles.css`、`layoutMemory.ts` 新 id | `pnpm build` 通过 |
| 4 入口接线 | 完成 | `RepoTab.tsx`（第三种视图）、`CommitDetailView.tsx`、`LocalChangesView.tsx`、`fileHistoryItems.ts`、`tests/vitest/FileHistoryView.test.tsx` | `pnpm test` 84 文件 / 1427 用例全过 |
| 5 验证与 review | 完成 | 独立 review 8 条全部处理；真实窗口发现并修复 Blame 横向吸附 | `pnpm build` 通过、`pnpm test` 84 / 1434、`cargo fmt --check` / `clippy --all-targets` 干净、`cargo test` 82 个测试二进制 1704 通过；`pnpm tauri dev` 真实窗口 16 项全过 |

## 决策条目

### 全局

- **D0 不追问，直接定方案**：用户明确要求「直接按照最优方案开工」。入口、布局、命名都是本次自定，用户可推翻。
- **D1 历史与 blame 走 git CLI，不走 libgit2**（偏离「读走 libgit2」的默认分工，AGENTS §7.3 要求说明理由）：
  - libgit2 没有 `--follow`，跨重命名追溯要自己逐个提交做改名检测，既慢又和 git 的判定不一致。`git_read.rs` 开头已经写明 `log` 的语义留在 CLI。
  - libgit2 的 blame 在长历史上明显比 `git blame` 慢，且不认 `blame.ignoreRevsFile` 这类用户配置；`git blame` 不带 rev 时还能直接给出工作区里「未提交」的行。
  - 两条命令都只读，走现有 `run_git` 封装，错误映射为 `GitError`。
  - 单个文件在某提交里的 diff 仍走 libgit2（和 `read_diff` 同一套 `DiffData` 产出）。
- **D2 历史起点 = `HEAD` + 打开它的那个提交**：只从 HEAD 走，别的分支上的提交打开历史会是空的；只从那个提交走，又看不到它之后的改动。两个都作为起点，`git log` 取并集。Local Changes 进来只有 `HEAD`。
- **D3 文件历史作为主区域的第三种视图**，而不是弹框：历史 + diff + blame 需要大面积，弹框放不下，也不用走 §5.1 的弹框布局门槛。侧边栏的 Local Changes / All Commits 点一下就回去。

（下面按 Task 记录偏离计划 / 计划的错误 / 计划没写的坑 / 安全性论证。）

### Task 1 后端

- **坑（实测 git 2.5x，fixture）**：`git log -z --format=<x> --name-status` 的形状是 `\x1e<头>\0\n<状态>\0<路径>\0[<新路径>\0]`；改名 / 复制是 `R100\0旧\0新\0` 三段。头和 name-status 之间多一个 `\n`，解析时要去掉。从来没有过的路径 `git log --follow` 退出 0、输出为空——要当成「没有历史」而不是错误。
- **坑**：`git blame --porcelain` 每个提交的作者等信息只在**第一次出现**时给，后面同一提交的行只有 `<sha> <orig> <final> [<n>]` 和 `\t内容`；`filename` 也只在第一次出现时给。工作区未提交的行 sha 是 40 个 0，作者是 `Not Committed Yet`。
- **安全性**：三个命令全是只读，不改 index、不改工作区；空 `file` 与以 `-` 开头的 rev 在进 CLI 之前挡掉。
- **坑**：porcelain 的 `filename` / `previous` 在 `core.quotePath=true`（git 默认）下会把中文路径转成带引号的八进制。命令里固定 `-c core.quotePath=false`，剩下的引号 / 控制字符转义由 `unquote` 还原（lib 单测覆盖）。
- ~~**坑**：合并提交在 `git log --name-status` 里没有 name-status，只有头~~ —— **这条原先写错了**（独立 review 第 2 条指出，fixture 实测确认）：`--follow` 下没有 diff 输出的合并提交 git **整条不输出**。只在解冲突时改了这个文件的合并提交因此不在历史里，Blame 却会把那些行归到合并提交。处理：Blame 点这种块时退回「在提交图中定位」；从合并提交打开历史时列表上方说明「打开时的提交不在列表里」。真实形态固化在 `history_and_blame_around_a_conflict_resolving_merge`；lib 里那条手写输入的单测改名为 `parse_log_tolerates_a_record_without_name_status`，注明只是防御、不是真实形态。
- **偏离计划**：二进制判定放在 blame 之前（libgit2 读 blob 的 `is_binary`；工作区读前 8000 字节找 NUL，与 git 同一判据），而不是从 blame 输出里找 NUL：大二进制文件 `git blame` 要算很久，没必要跑。
- **偏离计划**：TDD 顺序上测试与实现是同一轮写完再跑的（先用一个临时探针测试摸清了 git 的真实输出，探针已删），没有单独跑出一轮红灯。

### Task 2 前端纯逻辑

- **坑**：从 Local Changes 打开时，新增文件（含 `git mv` 的新路径——`read_status` 把它列成一删一增）HEAD 里没有，`git blame` 报 `no such path in HEAD`、历史为空。菜单项置灰并写明原因（`workdirHistoryAvailable`），而不是点了再报错。工作区里删掉的文件 Blame 看 HEAD 那一版。
- **决策**：Blame 色条按提交时间的**名次**而不是时间线性插值着色：长寿仓库里按时间算，近几年的提交会挤在同一种颜色里。

### Task 3 / 4 视图与入口

- **决策**：菜单文字用英文 `File History` / `Blame`，与同一菜单里已有的 `Stage` / `Copy path` 一致。
- **决策**：文件历史视图里 `useRepoEffects` 按「要回去的那个视图」读 diff，返回时不用再等一次；视图自己的历史 / Blame / diff 由组件直接经 store 包装读，跟随 `headHash` 重读（提交、切分支之后）。
- **决策**：「在提交图中定位」在混合仓库里先把基线 oid 换成 `svn:N` 节点再找（基线提交已被接合掉）；不在图上时报错说明，不留悬空选中（同 `handleSelectWorktree`）。

### Task 5 验证与 review

独立 review（独立 subagent，按 §13）没有高危问题，下列全部已修：

| # | 级别 | 问题 | 处理 |
|---|---|---|---|
| 1 | 中 | 混合仓库里交付过的本地提交收在 svn 节点里、不在 `state.commits`，「在提交图中定位」报不在图中 | 抽成 `graphSelectionFor`，`findDelivered` 找得到也算；vitest 覆盖 |
| 2 | 中 | 合并提交的真实形态与日志 / 单测不符（见 Task 1 被划掉的那条） | fixture 实测固化、改日志、加界面说明 |
| 3 | 中 | `git log` 把 `a[1].txt` 当通配符，会混进 `a1.txt` 的提交 | 加 `--literal-pathspecs`（实测与 `--follow` 可同用）；fixture 覆盖 |
| 4 | 低 | 暂存了新增又改过的文件在 Unstaged 里是 `modified`，菜单没置灰，点了 blame 报错 | 两个区里任一是新增都按新增算；组件测试覆盖 |
| 5 | 低 | 外部工具退出时按原始 `viewMode` 刷新，历史视图下回到 Changes 是旧的 | 与 `useRepoEffects` 同样按「要回去的视图」 |
| 6 | 低 | svn 节点一律退回 HEAD | 有 `baselineOid` 时按基线提交算，删除时 Blame 前一版基线 `diffBase`；vitest 覆盖 |
| 7 | 低 | 从旧提交、按旧文件名打开时，HEAD 那一侧追不到改名之后的历史，未记录 | 记在这里；列表上方的说明同第 2 条 |
| 8 | 低 | blame 头只认 40 位 sha，SHA-256 仓库会静默显示「空文件」 | 认 40 / 64 位；有输出却一行没认出来时报 `blame_unparsed`；前端 `isFullOid` / `isUncommitted` 同步 |

- **计划没写的坑**：`tests/svn_write_guard.rs` 要求 `commands/` 下每个命令要么走 `trait Vcs`、要么 `require_capability`、要么进 `NOT_GIT_ONLY` 写明理由。`history.rs` 整个文件按只读放行。
- **真实窗口才发现的坑**（`pnpm tauri dev`，独立 identifier，CDP 驱动 WebView2，非 headless）：Blame 横向滚动时，短行的提交栏和行号跟着内容滚走，只有最长那一行吸住了。原因是短行只有视口那么宽，sticky 元素在它的包含块里没有可以位移的余地。修法同 `.diff-file`：外面包一层 `width: max-content; min-width: 100%` 的 `.fh-rows`。jsdom 不做布局，组件测试测不到这一点。
- **真实窗口验证范围**：工作区 Blame（含未提交行）、横向吸附、跨改名历史、旧路径 diff、改名配对、Blame 旧版本 / 之前的版本、在提交图中定位、File Tree 入口、返回、新文件置灰、`oldFile` 参数名（带一条写成 `old_file` 的反例：被忽略、改名退化成新增）、空文件名被后端拒绝。fixture 仓库建在会话 scratch 目录，验完删除；独立 identifier 的配置目录也已删除。
- **计划没写的坑**：`commands::watch::tests::stage_and_unstage_only_touch_the_index` 在本机时好时坏（一轮里连续 3 次失败，后来整跑一次通过）（unstage 时看到临时 fixture 的 `.git/hooks` 目录事件，git 2.51.2）。本 PR 没有改 watch / stage / `git_cli`，判断是环境问题；没在 master 上复跑确认（worktree 隔离下跑不了别的检出）。

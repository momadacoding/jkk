# Stage 后的刷新太重（6.13）：实现日志

没有单独的计划文件。输入是 2026-11-05 排查报告第 9 节（结论：每点一次 Stage，对整个工作区全量扫描 2–3 次，
真实工作副本上一次 1.35–2.33 秒）。用户要求「修复 6.13」，按报告推荐的 A + B + C 一起做，D（读状态改走 git CLI）不做。

上一轮相关工作：`2026-10-30-stage-refresh-perf-decisions.md`（`read_status` 只扫一遍）、
`2026-10-30-refresh-single-flight-decisions.md`（刷新单飞）。

## 进度总账

| Task | 内容 | 状态 | 产出 | 验证 |
|---|---|---|---|---|
| A1 | Rust：只读这几个路径的状态（`read_status_paths`） | 完成 | `src-tauri/src/engine/diff.rs`、`vcs/{mod,git,svn}.rs`、`commands/diff.rs`、`lib.rs`、`tests/status_paths_contract.rs` | 新测试 9/9（实现前编译失败）；`status_contract` 7/7 不变 |
| C1 | Rust：`read_diff` 支持只读一个文件（`staged:<path>` / `unstaged:<path>`） | 完成 | `src-tauri/src/engine/diff.rs`、`tests/diff_file_scope.rs` | 实现前 4/4 失败、实现后 4/4 通过；`fixtures` 33/33 不变 |
| B1 | Rust：自己写 index 引起的监听事件不再触发整轮刷新 | 完成（review 后改为核对 index 指纹） | `src-tauri/src/commands/watch.rs`、`commands/ops.rs`、`engine/watch.rs` | `commands::watch` 9/9，含真实 watcher + 真实 `git add` / `git reset` + 外部 `git add` 反例 |
| R | 独立 review 发现的问题 | 完成 | 见「独立 review」 | `pnpm test` 1215/1215，`cargo test` 见 V |
| A2 | 前端：Stage / Unstage 后就地合并列表 | 完成 | `src/engine/statusPatch.ts`、`src/store/repoStore.ts`、`src/components/RepoTab.tsx`、`tests/vitest/statusPatch.test.ts`、`tests/vitest/repoStoreStatusPatch.test.ts` | 新 store 测试实现前 5/10 失败、实现后 10/10 |
| C2 | 前端：右侧 diff 按选中的单个文件读 | 完成 | `src/engine/fileSelection.ts`、`src/store/repoStore.ts`、`src/components/{RepoTab,LocalChangesView}.tsx`、`src/styles.css`、相关测试 | 实现前 11 条失败、实现后全过；`pnpm test` 1211/1211；`pnpm build` 通过 |
| V | 前后对比基准 + `pnpm tauri dev` 实测 invoke | 完成 | scratchpad 基准（不进仓库） | 见下文「V」；公司机器真实工作副本上的复测未做 |

## 决策条目

### A1

**做法**：新命令 `read_status_paths(path, paths)` 返回整份 Staged（HEAD 对比 index，不碰工作区）+ 只有点名路径的
Unstaged（index 对比工作区，字面量 pathspec）+ `ignoreCase`。`read_status` 拆成 `staged_list` / `unstaged_list`
两个函数复用，输出不变（`status_contract.rs` 7/7）。

**契约**（`status_paths_contract.rs`）：Staged 与全量读完全相同；Unstaged 恰好是全量读里点名路径的那几项（内容、顺序）。
每个场景对列表里出现的每个路径逐个核对，再核对全部路径、干净路径、不存在的路径。

**计划没写的坑**

- `git add` 有连带改动（实测，`a_side_effect_on_an_untouched_path_shows_up_in_staged`）：已跟踪的文件 `t` 换成同名目录后
  暂存 `t/inner.txt`，git 会把 index 里的文件 `t` 一起删掉。`t` 没被点名，局部重读看不到它在 Unstaged 那一项已经没了。
  这种改动一定反映在 Staged 上，所以前端比对「没点名的路径在 Staged 里变没变」，变了就退回全量读（A2）。
- 空路径列表在 libgit2 里是「全部」。`read_status_paths` 对空列表直接不读 Unstaged，否则前端会把整份 Unstaged
  追加到原列表上，每项出现两次（`an_empty_path_list_reads_no_unstaged_entries`）。
- `ignoreCase` 取 `core.ignorecase`：libgit2 打开 index 时按同一个配置项决定 index 是否不区分大小写，diff 的顺序随之变。

**偏离报告**

- 报告写的是「只重读这几个文件的状态，加一次不碰磁盘的 HEAD↔index 比对，就地更新列表」。合并放在前端
  （`src/engine/statusPatch.ts`）：Rust 侧没有上一次的列表。为此 Rust 多返回一个 `ignoreCase`。
- `Vcs` trait 加了 `status_paths`，两个后端都是委托；`tests/svn_write_guard.rs` 把它归为读方法。

### C1

**做法**：`read_diff` 认 `staged:<路径>` / `unstaged:<路径>` 两种 scope，只读这一个文件，结果与整区读里这个路径的那一项
相同（`diff_file_scope.rs`，含冲突文件补行、二进制、坏路径、`f[1].txt` 字面量匹配）。没有改 Tauri 命令签名。

- 空路径（`unstaged:`）报 `bad_diff_scope`：退化成整区读的话，这条快路径会悄悄变回它要替换的那次全量读。
- `staged` 整区读也走新的 `staged_side`，行为不变（原来传 `None` 选项，现在传默认 `DiffOptions`，两者等价）。

### B1

**计划没写的坑（实测，`stage_and_unstage_only_touch_the_index`）**

- 报告只提到 `.git/index`。真实 watcher 上，`git add` 还报了 **`.git` 目录本身**和 **`.git/objects` 目录**的变动
  （`index.lock` 建了又改名、新对象落进新的两位前缀目录引起的目录项变化）。现有过滤把这两个当相关事件，
  所以只豁免 index 的话方案 B 完全不生效。第一版测试就是在这里红的。

**做法（第二版：核对 index 指纹）**

- `OwnIndexWrite` 守卫：`stage` / `unstage` 命令开头登记「本应用在写 index」，Drop 时记下 index 的指纹
  （大小 + 修改时间）。计数而不是布尔：两次 Stage 重叠时，先结束的那次不能让还在写的那次被当成写完了。
- watcher 把相关事件分两类：gitdir 下的 `index`、gitdir 本身、commondir 下的 `objects` 这三条是「自己写 index
  也会碰到的」，其余一律要刷新。攒着的变化只有前一类时，去抖到点才核对：还在写 → 下一个 tick 再看；
  index 还是最后记下的那份 → 不发；没登记过、读不到或者指纹变了 → 照常发。
- 三条路径比较前把事件路径的父目录 `canonicalize`（8.3 短名、大小写、分隔符，同 `watch_targets` 的坑），
  先按文件名预筛，绝大多数事件不付这个代价。linked worktree 的 index 在 per-worktree gitdir 里、对象库在
  commondir 里，所以分开取。

**偏离 / 改过的设计**

- 第一版按时间窗口豁免（写的过程中加之后 1 秒）。独立 review 第 6 条与用户都指出：窗口内外部进程写 index
  （终端 `git add`、mergetool 标记已解决）会被一起吞掉。用户提议改为核对 index 的时间戳和大小，采纳。
  另一个好处：迟到的事件也认得出，不再依赖余量取多大（review 第 10 条担心的正是余量）。

**为什么这么改是安全的**

- 只影响「要不要发 `repo-changed`」，不碰任何 git 操作。
- 外部写 index 一定改变修改时间（git 先写 `index.lock` 再改名，NTFS 精度 100ns），所以会照常刷新，
  实测见 `stage_and_unstage_only_touch_the_index` 末尾的外部 `git add`。剩下的缝只有「本应用的 git 退出」到
  「Drop 里记下指纹」之间那几微秒里恰好有外部写入，这时会把外部那份当成自己的。
- `.git` 目录本身和 `objects` 目录的事件只是目录项变化。外部真有改动时总有自己那条事件（index、refs、HEAD……），
  所以这一类只看 index 指纹就够了。
- 前端自己的刷新（A2）照常跑，冲突分类也跟着它走（`refreshStatus` 里的 `refreshConflicts`）。原来那一轮刷新顺带做的
  merge driver 重查、以及同一仓库其它标签的刷新，由 `afterIndexWrite` 补上（见「独立 review」）。

### A2

**做法**

- `applyStatusPatch(prev, touched, patch)`：没点名的路径在 Staged 里变了（连带改动）→ 交回 `null`；否则 Staged 换成
  重读的整份，Unstaged 去掉点名路径的旧项、把新项按 git 的顺序归并回去（`gitPathCompare`：按码位比，不区分大小写时只
  折叠 ASCII 字母，与 libgit2 `git__strcasecmp` 一致）。不追加到末尾：下一次全量读时列表会跳。
- `refreshStatus(path, touched?)`：先把需求登记进 `statusWanted`（要不要全量、要局部读哪些路径），单飞开跑时把排队期间的
  需求合起来读一次。局部重读只在 `knownStatus` 里有这个仓库「上一次读到的状态」时才走；合并交回 `null` 或局部重读失败，
  退回全量读。
- `refreshChanges(path, selection, touched?)`：Stage / Unstage 传路径；丢弃、提交、DU/UD 二选一会动工作区，照常全量。

**计划没写的坑**

- 不能沿用单飞「补跑用最后一个任务」的规则：全量刷新（外部改动）排在前、局部重读排在后时，补跑只读几个路径，
  排队的外部改动就漏了。所以需求单独登记、开跑时取走，同一轮里只要有一个调用方要全量就全量
  （`repoStoreStatusPatch.test.ts` 的两条「排队」用例）。
- 全量读可能是 Stage 之前发起的（文件监听）。局部重读和它同一个单飞 key、排在它后面、在它的结果上合并，
  不会先合并再被它晚到的旧结果盖掉。

### C2

**做法**：`changesDiffScope(selection, status)` 给出 `<区>:<路径>`，两边都空时为 `null`（原来固定读整个 Unstaged，
没有改动时也白扫一遍工作区）。`changesSection` 只剩它一个调用方，换掉后删除，测试改成 `changesDiffScope` 的。

**计划没写的坑**

- 原来在同一个区里换选中不用重读（整区的 diff 已经在手里），现在每换一次都读一次。两处处理：
  1. 单文件 scope 全部排进同一个单飞 key（`diff:<path>:changes-file`）：方向键一路按下去，同时只读一个，补跑只读最后
     一个，中间跳过的不读。写回仍只认当前视图要的 scope。
  2. 「正在读取…」提示延迟 200ms 出现（`.diff-note-pending`），单个文件几十毫秒就回来，不让它一闪而过。
- 没有文件时 `diff` 一直是 `null`，`LocalChangesView` 原来会说「正在读取」并且永远读不完；改成按空 diff 显示。
- `repoEffectsDiffScope.test.ts` 的 mock 原来让 `read_diff` 永不返回；单文件读取排同一队之后，换选中的那次只会排着
  不发出去。改成 `read_diff` 立即返回，并在换选中前先让上一轮跑完。

**偏离 / 没做的**

- 监听 `repo-changed` 的 effect 依赖仍是 `[state.path, diffScope]`，所以现在每换一个文件就重新注册一次监听
  （原来是每换一个区）。处理函数读的是 `wantedDiffScope` ref，本可以去掉这个依赖；没去掉是为了不碰
  「换区快过 IPC 往返」那条已有保护的前提。代价是注销与重新注册之间那一次 IPC 往返内的事件会丢，等下一个事件补上。

### V

**基准**（release 构建，开发机 SSD、文件缓存是热的）：50 × 20 × 15 = 15,000 个跟踪文件，30 处修改、8 个未跟踪文件，
`.git/info/exclude` 460 条 `**/genN/**` 规则（模拟公司仓库的白名单规则数）。每项 7 轮取中位数。

| 环节 | 旧 | 新 |
|---|---|---|
| Stage 后读状态 | `read_status` 全量 122 ms | `read_status_paths` 1 个路径 12 ms，30 个路径 46 ms |
| 右侧 diff | 整区 `unstaged` 110 ms / `staged` 12 ms | `unstaged:<文件>` 9 ms / `staged:<文件>` 8 ms |
| 监听补发的那一轮 | 又一次全量读 + 提交图、stash、merge driver、svn 检查 | 不再发生 |

- 30 个路径的 46 ms 大部分是这 30 个文件的行数统计（逐个读内容），全量读同样要付这部分。Stage All 的情形下
  这些路径暂存之后在 Unstaged 一侧已经没有差异，不产生补丁，所以局部重读不会比全量读更慢（推理，没有单独测）。
- 绝对值远小于报告里公司仓库的 1.35–2.33 秒/次全量：fixture 复刻不了真实仓库的规模和白名单结构。
  **公司机器上的实际收益仍需在那台机器上用报告留下的 `bench.py` 复测**，复测后删掉 `%TEMP%` 里的工具。

**`pnpm tauri dev` 实测**（核对过运行的是本 worktree 的 `src-tauri\target\debug\app.exe`；session.json 先备份、结束后恢复，
哈希一致；WebView2 配置目录隔开）。fixture 仓库建在本 worktree 被忽略的 `src-tauri/target/v-fixture`。经 CDP：

- `read_status_paths` 正常返回（含 `ignoreCase: true`），空列表不读 Unstaged；参数名写错（`pathList`）报
  `missing required key paths`，证明前一次确实走到了新命令。
- `read_diff` 的 `unstaged:a.txt` 只返回这一个文件；`unstaged:` 报 `bad_diff_scope`。
- 界面上 Unstage a.txt：146 ms 内列表更新，a.txt 按 git 顺序插回第一位，与随后全量 `read_status` 的结果一致；
  连着 Stage 两个文件同样一致。**两次操作之后 2.5 秒内 `repo-changed` 一次都没发**（方案 B 生效）。
- 正对照：外部改一个工作区文件、外部终端 `git add`，各收到一次 `repo-changed`，列表随之刷新。
- `.diff-note-pending` 在 WebView2 里 0 / 120 ms 时 `visibility: hidden`，320 ms 时 `visible`。
- B1 改为核对 index 指纹之后在真实窗口复验：界面上 Unstage、Stage a.txt 各一次，2 秒内 `repo-changed` 0 次，
  列表与全量读一致；随后外部终端 `git add -- b.txt`，收到 1 次事件，列表刷新。「自己写完后紧接着外部写入」这个
  紧贴的时序在窗口里卡不准，由 `the_verdict_follows_the_index_fingerprint` 覆盖。
- 最终：`cargo fmt --check`、`cargo clippy --all-targets` 无告警；`cargo test` 83 个目标 1678 条全过；
  `pnpm build` 通过；`pnpm test` 74 个文件 1215 条全过。

**计划没写的坑**

- `pnpm tauri dev` 把 `src-tauri/Cargo.toml` 的换行改成了 LF（内容没变），提交前用 `unix2dos` 改回。
- `window.__TAURI_INTERNALS__.invoke` 不可写（`writable: false`），想在页面里拦截 invoke 记调用顺序做不到，
  改为订阅 `repo-changed` 并比对界面列表与全量读的结果。

### 独立 review（`/code-review high`，2026-11-06）

| 条目 | 结论 | 处理 |
|---|---|---|
| 1 字面量 pathspec 仍按目录前缀匹配，点名 `t` 带出 `t/x`，合并后重复 | 成立，fixture 复现 | `read_status_paths` 只留点名路径的项（`a_named_path_does_not_bring_in_entries_under_it`，先失败后通过）；`applyStatusPatch` 见到没点名的路径交回 `null`，再挡一道 |
| 2 Stage 之后不再重查 merge driver（原来靠监听那一轮） | 成立 | `afterIndexWrite` 里补调 `refreshMergeDrivers` |
| 3 `git add` 写了一部分再以非 0 退出时不刷新 | 成立 | `RepoTab` 的 `writeIndex`：失败也刷新，且按全量读 |
| 4 监听 effect 依赖 `diffScope`，换一个文件就重新注册，空档里的事件会丢 | 成立 | 依赖改为 `[state.path, state.capabilities.vcs]`；测试改成「换选中不重新注册」+「换仓库快过 IPC 往返」 |
| 5 Stage All 点名几千个路径可能比全量还慢 | 实测不成立 | 3000 个改动文件（release）：未暂存时局部 3023 ms vs 全量 3418 ms，暂存后 1262 vs 1307 ms，不加阈值 |
| 6 窗口内外部写 index 被吞；同一仓库开在别的标签时那些标签收不到刷新 | 成立 | 前半：B1 改为核对 index 指纹；后半：`afterIndexWrite` 进程内通知其它标签全量刷新 |
| 7 单文件 diff 共用一个单飞 key，被顶替的调用方拿到后一个文件的错误 | 不改 | 错误条是一份 state，同一条错误设置两次不会显示两遍；被顶替的读取本来就不再需要 |
| 8 `gitPathCompare` 每次比较分配两个数组 | 成立 | 改为按下标逐码位比 |
| 9 每个事件都 normalize + 加锁，登记表不清理 | 成立 | 复用线程里的 key；只在攒着的变化全是那三条路径、到点时才查一次登记表。登记表每个 Stage 过的仓库一项，第二版没有过期的概念，不清理 |
| 10 实测测试硬编码豁免为真，没走真守卫 | 成立 | 测试改为走 `OwnIndexWrite` 和 `own_write_verdict`，并加外部 `git add` 的反例 |

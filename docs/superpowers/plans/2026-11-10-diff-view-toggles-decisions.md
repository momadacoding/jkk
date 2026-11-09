# Diff 面板开关（左右分栏 / 忽略换行符等）实现日志

需求（2026-11-10 用户）：diff 面板支持左右分栏；默认忽略换行符改动；做成 Fork 那样一排 toggle
（截图：空白 ⌴、不可见字符 ¶、自动换行 ↩、减少上下文、增加上下文、整个文件、左右分栏）。

没有单独的计划文件，按 §2.1「≥3 个 Task」规则建本日志。

## 进度总账

| Task | 内容 | 状态 | 产出 | 验证 |
|---|---|---|---|---|
| 1 | Rust：`read_diff` / `commit_file_diff` 收 `options`（上下文行数、忽略空白） | 完成 | `engine/diff.rs`（`DiffViewOptions`、`read_diff_with`）、`engine/file_history.rs`（`commit_file_diff_with`）、`vcs/*`、`commands/{diff,history}.rs`、`tests/diff_view_options.rs` | `cargo test --test diff_view_options` 9/9 |
| 2 | 前端 engine：偏好、忽略换行符、分栏配对、不可见字符 | 完成 | `src/engine/{diffPrefs,diffEol,diffSplit}.ts` + 三个同名 vitest | 31/31 |
| 3 | store：偏好存储；diff 读取带上选项，换选项重读 | 完成 | `src/store/diffPrefsStore.ts`、`repoStore.ts`（`refreshDiff` / `useRepoEffects` / `readCommitFileDiff`）、`FileHistoryView.tsx`、`tests/vitest/repoStoreDiffOptions.test.ts` | 5/5；`FileHistoryView.test.tsx` 两条断言补上 `options` |
| 4 | UI：工具栏 + 分栏 / 换行 / 不可见字符渲染 + CSS | 完成 | `DiffView.tsx`、`styles.css`、`tests/vitest/DiffView.test.tsx` | 9/9 |
| 5 | 验证：两侧门槛 + `pnpm tauri dev` 实测 + 独立 review | 完成 | review 修复见下 | 最终：`pnpm build` ✓、`pnpm test` 1507/1507、`cargo fmt --check` ✓、`cargo clippy --all-targets` ✓、`cargo test` 93 个测试文件全过（内存紧张被回收一次，`-j 2 -- --test-threads=4` 重跑）；真实窗口逐项实测 |

## 决策条目

### 总体方案

| 开关 | 在哪做 | 理由 |
|---|---|---|
| 忽略换行符（默认开） | 前端后处理 | libgit2 1.9 没有 `IGNORE_CR_AT_EOL`（vendored xdiff 有 `XDF_IGNORE_CR_AT_EOL`，但 `diff_xdiff.c` 不映射它）。`IGNORE_WHITESPACE_EOL` 会连行尾空格的改动一起吞掉，不等于「只忽略换行符」。所以拿到普通 diff 后在前端把「只差行尾 `\r`」的一删一增配成上下文行，再按上下文行数重新切 hunk |
| 忽略空白 | 后端 libgit2 `ignore_whitespace`（= `git diff -w`） | libgit2 原生支持，语义与 git 一致，比前端后处理准 |
| 上下文行数 / 整个文件 | 后端 `context_lines` | 要的是 diff 里本来没有的行，只能后端给 |
| 不可见字符 / 自动换行 / 左右分栏 | 纯前端渲染 | 不改数据 |

- 偏好是全局的（所有标签、所有 diff 视图共用），存 `localStorage`，与布局记忆同一取舍（见 `layoutStore.ts` 注释）。
- 「整个文件」的上下文行数用 `1 << 24` 而不是 `u32::MAX`：xdiff 里 `ctxlen` 是 `long`，Windows 上 `long` 是 32 位，`u32::MAX` 转过去成了 -1。

### Task 1：Rust

- **设计取舍**：`read_diff` / `commit_file_diff` 原签名保留，新增 `read_diff_with` / `commit_file_diff_with`。原函数在 Rust 测试里有 30 多处调用，改签名只会制造无关 diff。
- **Tauri 命令签名**：两个命令各加一个可选参数 `options: Option<DiffViewOptions>`。JS 侧参数名 `options`（单个词，没有 snake/camel 转换问题）；不传 = 原来的默认行为，旧调用方不会坏。`DiffViewOptions` 的字段 serde 用 `camelCase` + `default`，前端发 `{ contextLines, ignoreWhitespace }`。
- `trait Vcs::diff` 加了 `view` 参数，SVN 后端照旧转给 git 后端。
- **状态列表的增删行数不受选项影响**：`read_status` 不变。列表上的 +/- 数仍是原样的改动，只有右侧 diff 按开关显示。
- **计划没写的坑**：原来 `commit:` 和 `range:` 两个 scope 给 `diff_tree_to_tree` 传的是 `None`（libgit2 默认选项），要换成带选项的 `DiffOptions`；`fill_conflicted` 另外构造一次 diff，也要套上选项，否则冲突文件不跟着开关走。`context_lines_reach_every_scope` / `ignore_whitespace_hides_whitespace_only_changes_on_every_scope` 对六种 scope 逐一核对。
- **钉住前端依赖的 libgit2 行为**：`crlf_changes_keep_the_carriage_return_in_line_content`：LF→CRLF 时 libgit2 给「全删再全加」，新版行内容带 `\r`（`diff_to_data_only` 只去掉 `\n`）。

### Task 2：前端 engine

- 忽略换行符的做法：每个 hunk 里每段连续的删/增，按「去掉行尾一个 `\r`」后的内容跑 Myers（`diffSequences`），配上的变成上下文行（内容取新版那行，两个行号都填），剩下的每段先删后增；再按当前上下文行数重切 hunk（改动前后各留 N 行，两段上下文挨上就合并，同 git）。
- Myers 先剥掉两端相同的部分，整文件换换行符的情形基本在这一步做完；剩下的改动数超过 500 就放弃重比、原样显示（Myers 回溯的内存随改动数平方增长）。
- 重切出来的 hunk 头自己拼 `@@ -a,b +c,d @@`；只有从原 hunk 开头切出的那段保留原头里的函数上下文。纯新增且上下文为 0 时旧侧起始行号按 git 的写法用前一行。
- 「文件末尾有没有换行」的差别在行内容上看不出来（`\n` 已去掉、`\ No newline` 标记行被过滤），也一并当作换行符改动藏掉。可接受：它本来就是换行符的改动。
- 一个 hunk 里一处都没藏的话原样返回同一个对象，不重切（保留 libgit2 的原样头）。
- 不可见字符：空格 `·`、制表符 `→` 画在绝对定位的伪元素里，不占宽度，复制出去还是原字符；行尾 `\r` 画 `␍`。开关关着时行尾 `\r` 从显示文本里去掉。

### Task 3：store

- **偏好全局一份**（`diffPrefsStore.ts`，模块级 + `useSyncExternalStore`），存 `localStorage` 键 `mygitclient:diff-prefs`；读存档逐字段校验，坏的回默认。
- **只有上下文行数和忽略空白要重读**：`backendDiffOptions` 只取这两项，`diffOptionsKey` 是它的键。`useRepoEffects` 里按选中读 diff 的 effect 把这个键加进依赖；分栏、换行、不可见字符、忽略换行符切换不发请求。
- **为什么并发安全**：`refreshDiff` 在真正开读那一刻取开关（排在单飞补跑里的那一轮可能是开关变之前登记的）；写回前核对「开关的键还是开读时那个」，不是就丢掉。连按两下「增加上下文」时，第一轮读回来不会盖掉第二轮。`joinSince` 复用在跑那轮的条件加上键相同。测试 `repoStoreDiffOptions.test.ts`。
- 开关变了时 store 里那份旧 diff 留着（`diffScope` 没变，`currentDiff` 照样认），新的回来再换：界面不会闪成「正在读取」，工具栏也不会消失。`FileHistoryView` 的单提交 diff 同样处理：只有换了提交/文件才清空。

### Task 4：UI

- 工具栏放在 `DiffView` 自己顶上，`.diff-view` 改成上下两段（工具栏 + `.diff-scroll`），滚动从外层挪到里层，工具栏不随内容滚走。三个用到 `DiffView` 的地方（本地改动、提交详情、文件历史）一起生效，调用方不用改。
- 按钮顺序参照 Fork 截图：空白 ⌴、不可见字符 ¶、自动换行、减少上下文、增加上下文、整个文件、左右分栏；最左边多一个 `EOL`（忽略换行符）。Fork 没有这个按钮，但用户要求它默认开，得单独给一个开关才能关掉。上下文档位 `0/1/3/5/10/25/50/100`，中间显示当前行数；「整个文件」开着时 ± 禁用。
- **左右分栏的两种画法**：
  - 不换行：左右两个窗格各自滚动，`scroll` 事件里互相同步 `scrollTop` / `scrollLeft`（只在值不同时赋，不会来回推）。一行一个格子的画法下长行要么截断、要么把右栏推出视口，所以不用它。两侧逐行对齐靠每行高度固定（`.diff-line` 加了 `min-height: 18px`，占位行也一样高）。两个窗格都 `overflow-x: scroll`：只有一侧出横向滚动条的话两侧可视高度不同，滚到底时最后一行会错开。
  - 换行：一行一个两列的 grid，两侧折行后取较高的那一侧，天然对齐，单一滚动容器。
- 配对规则同 Fork / GitHub：一段删除和一段新增按顺序一一对上，多出来的那一侧对面画斜纹占位。
- 新的空状态文案：只有换行符改动时说「只有换行符（CRLF / LF）改动，已按设置忽略」；开着忽略空白且没剩行时说「没有文本改动（已忽略空白）」。

### Task 5：验证 + 独立 review（`/code-review high`，fork 出的独立 agent）

真实窗口（`pnpm tauri dev --config` 换 identifier，核对过 `app.exe` 在本 worktree 的 `target\debug\` 下；fixture 在 scratchpad）实测：

- 提交「整个文件 LF→CRLF + 改一行」：默认只显示那一行的一删一增和上下文。
- 分栏不换行：两侧 7 行、头和行的 `top` 逐个相等（0 处错位）；纵向、横向滚动跟随；对面多出来的行画斜纹；两条横向滚动条都在视口底部。
- 只换了 CRLF 的文件显示说明；关掉 EOL 后变成 3 删 3 增；开不可见字符后 3 个 `␍`。
- 忽略空白：重读后只改缩进的那行成了上下文，真改的照样显示。

review 共 10 条，处理如下：

| 条目 | 结论 | 处理 |
|---|---|---|
| 第 1 条：分栏横向跟随会把宽的一侧拉回窄的一侧的尽头 | 成立（双向赋值，窄侧被夹住后它的 scroll 又反推回来） | 改成单向：被代码改动的那一侧的下一次 scroll 不再反推；只在值真的变了时才记这个标记，否则会吞掉用户下一次滚动 |
| 第 2 条：补上文件末尾换行也被当成换行符改动藏掉 | 成立，推翻了 Task 2 里「可接受」的判断：git `--ignore-cr-at-eol` 照样显示它 | 原样相同的一对不再配成上下文；Rust 测试 `adding_the_final_newline_gives_identical_line_content` 钉住 libgit2 的这个形态 |
| 第 3 条：提交详情一次读整个提交，「整个文件」会把每个文件全文都读出来 | 成立（几万文件的提交会序列化几百 MB） | 后端 `WIDE_CONTEXT_FILE_LIMIT = 200`：一次读出的文件超过 200 个且上下文调大时，按默认 3 行重建，`DiffData.contextNarrowed` 告诉前端，diff 上方一条说明。单文件 scope 不受影响 |
| 第 4 条：画法开关让每个标签都重渲染 | 成立 | `useDiffOptionsKey` 只订阅后端选项的键（字符串快照），`useRepoEffects` 和 `FileHistoryView` 改用它 |
| 第 5 条：提交详情的文件 +/- 数随忽略空白变 | 部分成立：与右侧 diff 一致是合理的，错的是 Rust 注释里「列表行数不受影响」的说法 | 改注释，写清「本地改动」列表不受影响、提交详情列表与 diff 一致 |
| 第 6 条：读取失败时工具栏跟着消失，调坏的开关调不回去 | 成立 | `DiffView` 加 `note` 参数，三个调用方的读取失败说明都放进 `DiffView`，工具栏照样在 |
| 第 7 条：重切出来的后几个 hunk 头没有函数名 | 成立但不改 | 函数名是 git 按 hunk 起点往前找的，前端拿不到原文无法重算；沿用原头的函数名可能是错的函数。只在第一段保留 |
| 第 8 条：`DiffView` 跨模块导入没用 `@/` | 成立（AGENTS.md §7.2） | 改成 `@/` |
| 第 9 条：三份一样的「文件 → hunk」循环，分栏配对每个窗格各算一次 | 成立 | 合成一个 `FileList`（传 `renderRows`），配对在 `useMemo` 里每个 hunk 算一次、两个窗格共用 |
| 第 10 条：保留了 `read_diff` / `commit_file_diff` 的默认参数包装 | 不改 | 见 Task 1 的取舍；包装只是一行转发，测试覆盖的逻辑相同 |

review 修复后再进真实窗口复验：

- 分栏横向：右侧（更宽）一次滚到 1707，左侧停在自己的尽头 1654，右侧不再被拉回；接着从左侧往回滚，右侧跟过来；再滚右侧也不被吞。
- **实测抓到的坑**：201 个文件的提交开「整个文件」，后端确实退回了 3 行上下文，但说明条没出现。`CommitDetailView` 只留选中文件时把 diff 重新拼成 `{ files }`，`contextNarrowed` 丢了。改成一起带过去，补了组件测试；复验说明条出现、关掉「整个文件」后消失。

### 遗留

- 「整个文件」打开超大单文件（几十万行）时 DOM 行数很多，与原来上下合栏显示大改动时一样没有虚拟滚动。
- 分栏不换行的两个窗格都常驻横向滚动条（为了滚到底时两侧对齐），没有长行时也占一条滚动条的高度。

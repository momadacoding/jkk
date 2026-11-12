# 提交慢（2026-11-11 使用报告）实现日志

输入是 2026-11-11 在公司机器（SVN 混合仓库，git 跟踪 17,602 个文件）上录的进程时间线：Ctrl+Enter 提交约 2 s 才完成，`git commit` 本身 255 ms。报告提出 H1–H4，本次实现 H1 与读代码新增的 H5。没有对应的计划文件。

## 进度总账

| Task | 状态 | 产出 | 验证 |
|---|---|---|---|
| H1 提交后不再跑 SVN 第二档 | 完成（按独立 review 重做过一次） | `engine/repo.rs`（`head_parents`）、`engine/svnSync.ts`（`sameHeadForSync` / `isPlainCommitOn`）、`store/repoStore.ts`（`noteOwnCommit` / `forceCheckAtHead`）、`RepoTab.tsx` | `fixtures.rs` 新增 1 个（含序列化键名）；`svnHeadMove.test.ts` 12 个；`repoStoreHeadMove.test.ts` 10 个。让 `noteOwnCommit` 什么都不做时 4 个变红，去掉第一轮第 8 条的衔接时 1 个变红，第二轮第 1 条的补跑在修之前红 |
| H5 提交后状态与图并发读、状态先发 | 完成 | `RepoTab.tsx` `handleCommit` | 无自动化测试，原因见下 |
| 独立 review（`/code-review high`） | 8 条全部处理 | 见下「review 处理」 | — |
| 公司机器复测 | 未做 | — | 要用户在 qj-liuhao 上用新构建提交 3–4 个文件 |

## H1

### 为什么这么改是安全的

- 第二档（`svn_sync::check_sync_with_nodes`）里只有 `behind_fork` 看 HEAD：当前处境（游离 / 基线分支 / 其余分支）+ `merge-base(HEAD, svn/base)`。其余输入（`svn/base` 的 OID、基线映射、`svn status` 节点、W-scope）与 HEAD 无关。
- 本应用刚做的单父提交是新对象，不可能是 `svn/base` 的祖先，所以 `merge-base(新, base) = merge-base(旧, base)`，结论不变。
- **形状相同不等于是新提交**：快进到 `svn/base` 也是「旧 HEAD 的单父子提交」，merge-base 却变成新基线，落后横幅要撤。所以只在本应用提交**成功之后紧接着**读到的 HEAD 上用这条判定（`noteOwnCommit`），不按形状认任意 HEAD。
- 其余情形一律照旧校验：amend（不调 `noteOwnCommit`，父提交也不是旧 HEAD）、解决 merge 后的提交（两个父提交）、在 `svn/base` 本身上提交、从没校验成功过。

### 计划没写的坑（git 真实行为）

- **提交期间分支被挪走，`git commit` 会失败**。fixture 实测（scratch 脚本：pre-commit hook 里 `git update-ref` 把分支挪到另一个 P 的子提交）：`fatal: cannot lock ref 'HEAD': is at <B> but expected <P>`，退出码 128，分支停在 B。所以提交成功就说明新提交叠在提交开始时的 HEAD 上，紧接着读到的 HEAD 就是它。剩下的窗口只有「提交进程退出」到「`open_repo` 读 HEAD」之间的几毫秒。
- 监听去抖 300 ms（`watch.rs` 的 `DEBOUNCE`），`noteOwnCommit` 在 `commit` 的 invoke 返回后立刻发，多半早于监听。晚于监听时，监听已为新 HEAD 排了一轮，`noteOwnCommit` 看到 `headRequested` 就是新 HEAD 本身，什么都不做，只是没省下。
- 原来比 HEAD 只比 hash，现在按「hash + 处境」比（`sameHeadForSync`）：同一个提交切到 `svn/base` 或游离时结论可能变，原来漏校验；`checkout -b` 一条新分支不会变，照旧不跑。

### 偏离（相对报告第 5 节）

- 报告只说「去掉提交后的强制第二档」。实际另做了两处：
  - 打开仓库时那次强制校验改走 `forceCheckAtHead`，校验前现读 HEAD，写回后记下。不记的话打开后第一次 HEAD 事件（常常就是第一次提交）因为记录为 `null` 还会再跑一遍。读不出 HEAD 时退回原来的 `refreshSvnSync`。
  - 提交时 HEAD 的那一轮校验还没落定（刚切分支、刚打开就提交），那一轮写回后直接把这次提交记成「上次校验的 HEAD」（`ownCommitOnPending`），不再为它排一轮。
- `open_repo` 的返回多了 `headParents`。命令参数没变，只是返回结构加字段；前端读时 `?? []` 兜底，旧后端返回里没有它时判定为「不是普通提交」，照旧校验。
- `RootRevMemo` 改名为泛型的 `ObservationMemo<T>`：它原本只剩 HEAD 在用，HEAD 的观测值从字符串变成了对象。

## H5

### 读代码确认的时序（未计时）

1. `handleCommit` 原来先 `await refreshGraph` 再 `await refreshChanges`。
2. 提交写 refs 后约 0.59 s，监听发起一轮全量 `refreshStatus`。
3. 状态读取走单飞（`singleFlight.ts`），在跑时来的请求排补跑。`refreshChanges` 晚于读图才发，多半撞上监听那一轮，于是要等「监听那轮 + 自己补跑的那轮」两轮全量状态。

### 做法与取舍

- 改成 `Promise.all` 并发、状态先发：提交返回时监听多半还没到，状态这轮先开读，监听那轮排到它后面补跑。提交只等 `max(读图, 一轮状态)`。
- 没有做「复用在跑的那轮」（像 `refreshDiff` 的 `joinSince`）：要在单飞外再记一份「这一轮何时开读、是不是全量」，改动面大；并发先发已经覆盖常见时序。监听早于 invoke 返回就开读的少见情形仍会等两轮，但结果正确。
- 监听那轮补跑仍在后台跑一次，不挡提交。

### 为什么没有自动化测试

时序取决于真实监听的去抖与 IPC 往返，`handleCommit` 在 `RepoTab` 组件里，jsdom 下要挂整个 RepoTab 并伪造监听时序，测到的只是假时序。改动本身是把几个独立刷新从串行改成并发，各自 `.catch`，写的是 state 里互不相关的字段（`status`/`diff`、`commits`/`refs`/`selectedId`、`head`/`headHash`）。效果靠公司机器复测确认。

## review 处理（`/code-review high`，第一版实现）

| 条 | 问题 | 处理 |
|---|---|---|
| 1 | 提交**前**登记、第一个不同的 HEAD 就被认领：提交期间（hook、gpg 签名）外部快进到基线也会被当成本应用的提交，落后横幅不撤 | 重做：改成提交**成功后**现读 HEAD 再判定（`noteOwnCommit`），依据是上面实测的 `cannot lock ref` 行为 |
| 2 | 登记没有过期，监听没收到新 HEAD 时会一直留着、认领以后无关的 HEAD 变化 | 随第 1 条消失：不再有提交前的登记 |
| 3 | 改成比分支名后，`checkout -b` 也要跑第二档 | 按处境比（`sameHeadForSync`） |
| 4 | 打开时记下的 HEAD 取自打开开头那次读取，可能过时；校验期间没设 `headRequested` | 校验前现读 HEAD，走 `forceCheckAtHead`（会设 `headRequested`） |
| 5 | 登记用的是渲染时闭包里的 `state.head` / `state.headHash`，可能落后 | 随第 1 条消失：改为现读 |
| 6 | `RootRevMemo` 没有生产代码在用，注释已不对 | 泛化为 `ObservationMemo<T>`，HEAD 记录用它 |
| 7 | 打开仓库内联了一份 `refreshSvnSync` 的逻辑 | 提取 `forceCheckAtHead`，打开与 `checkSvnSyncIfHeadMoved` 共用；`refreshSvnSync` 的注释改成它现在的用途 |
| 8 | 提交时上一轮还没落定，这次提交之后还会再跑一轮 | `ownCommitOnPending`：那一轮写回后直接记成这次提交 |

## 第二轮 review 处理（`/code-review high`，重做后的版本）

| 条 | 问题 | 处理 |
|---|---|---|
| 1、2 | 还没落定的那一轮失败或被顶替时，本应用那次提交的事件已经被吞掉，没人再为新 HEAD 校验，横幅停在切换前的结论上 | 那一轮没写回、也没有更晚的请求接手时，替那次提交补一轮（`forceCheckAtHead` 的 finally）。补测试「那一轮失败了」，修之前红 |
| 3 | `noteOwnCommit` 在 `await` 之后没再核对路径 | 读完 HEAD 再核对一次 `openedPath` |
| 4 | 层次：HEAD 变化只该重算 `behind_fork`，不该特判本应用的提交 | **不采纳**。第二档的 W-scope 在「尊重 `.gitignore`」时按工作区的 `.gitignore` 判定（`svn_scope.rs` 的 `WScope::for_repo` → `Respect`），checkout 会改它，切分支时仍要整轮校验。本应用的提交是唯一确知不碰工作区的 HEAD 变化，只对它省是最小的安全切口 |
| 5 | 衔接按 `own.parents[0] === head.hash` 认那一轮，同一个提交上的另一轮也会被认成它 | 登记时记下那一轮的对象（`on`），按 `===` 认 |
| 6 | 打开 SVN 仓库多一次 `open_repo`；每次 `open_repo` 多一次 `find_commit` | 接受：都是 libgit2 本地读，毫秒级 |
| 7 | `repo.rs` 注释引用了已删掉的 `headMoveNeedsCheck` | 改成 `isPlainCommitOn` / `noteOwnCommit` |
| 8 | 注释说「状态先发」，代码里 `noteOwnCommit` 排在第一个 | 注释改成实际顺序与理由 |
| 9 | `open_repo` 返回结构变了，没跑 `pnpm tauri dev` 验证 | 补 Rust 断言序列化后的键名是 `headParents`。**`pnpm tauri dev` 仍未跑**，见遗留 |

## 遗留

- 报告的 H2（`refreshChanges` 本身多慢）、H4（文件数影响）仍要 performance monitor 才能拆清。
- 报告第 7 节的空闲时每 5 分钟约 18 s 的服务器核对，与提交慢无关，未处理。
- `open_repo` 返回多了 `headParents`，AGENTS.md §5.1 要求的 `pnpm tauri dev` 实机验证没做：只有 Rust 侧序列化键名的断言和前端 mock 测试。键名错了的后果是优化静默失效（`?? []` → 照旧校验），不会出错。公司机器复测时顺带确认：提交后进程时间线里不再有 `svn info` + 两次 `svn status`。
- `commands::watch::tests::stage_and_unstage_only_touch_the_index` 在本机时好时坏（连跑 5 次 2 次过），失败时多收到一条 `.git/hooks` 事件。本次没有改 `watch.rs` 及它调用的代码；没在 master 上复现确认。

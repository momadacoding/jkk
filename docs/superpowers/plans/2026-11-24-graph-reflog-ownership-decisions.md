# 分叉方向按 reflog 判定（graph 车道归属）实现日志

计划来源：对话里选定的方案 B（没有单独的计划文件）。

起因：readwise-alternative 里两条本地分支共享一段不在 main 上的提交。
`claude/design-draft-discrepancy-257468` 是从 `claude/mobile-ai-gen-ui-design-de480b` 的 tip 切出来的，
图上却画成了反方向。原因是本地分支按 tip 行序认领：前者的 tip 新 78 秒，先认领，把共享的那段带走了。
只看拓扑判断不了谁是原分支，git 也不记录提交是在哪条分支上做的，唯一的来源是本机 reflog。

方案 B：分支 reflog 里「在这条分支上新建了提交」的条目作为证据。有证据时按证据排认领顺序；
证据缺失（过期、新 clone、换机器）时退回原来的行序，结果和改动前完全一样。

## 进度总账

| Task | 内容 | 状态 | 产出 | 验证 |
|---|---|---|---|---|
| 1 | 在 fixture 仓库里实测 git 往分支 reflog 写什么 | 完成 | 见 Task 1 | 一次性脚本，结论固化进 Task 2 的测试 |
| 2 | 后端：本地分支的 `RefPointer` 带上 reflog 证据 | 完成 | `src-tauri/src/engine/refs.rs`（`made_here`、`creates_commit`） | `tests/refs.rs` 新增 6 条，26 条全过 |
| 3 | 前端：本地分支按证据排认领顺序 | 完成 | `src/engine/owners.ts`（`orderByMadeHere`、`firstParentChains`）、`src/engine/types.ts`（`madeHere?`） | `owners.test.ts` 新增 7 条，22 条全过 |
| 4 | 验证：两侧门槛 + `pnpm tauri dev` 实测 | 完成 | — | `pnpm build`、`pnpm test`（104 文件 1889 条）、`cargo fmt --check`、`cargo clippy --all-targets` 无告警、`cargo test` 分三批（lib + 108 个集成测试二进制）全过 |

## 决策条目

### Task 1：git 的真实行为（git 2.51.2.windows.1，2026-11-24 实测）

在临时 fixture 里依次做 commit、amend、cherry-pick、revert、`merge --no-ff`、`worktree add -b` 后在 linked worktree 里 commit、rebase，
各分支 reflog 的 `%gs`：

| 操作 | 分支 reflog 消息 |
|---|---|
| 普通提交 | `commit: <标题>` |
| 第一个提交 | `commit (initial): <标题>` |
| amend | `commit (amend): <标题>` |
| cherry-pick | `cherry-pick: <标题>` |
| revert | `revert: <标题>` |
| `merge --no-ff` | `merge <分支>: Merge made by the 'ort' strategy.` |
| 建分支 | `branch: Created from HEAD` / `branch: Created from <起点>` |
| rebase | 分支上只有一条 `rebase (finish): refs/heads/<b> onto <oid>`，中间各个 pick 记在 HEAD 的 reflog 里 |
| linked worktree 里提交 | 记在共享的 `logs/refs/heads/<b>` 里，和主工作区一样 |

- **计划没写的坑**：readwise-alternative 那条 `claude/design-draft-discrepancy-257468` 的 reflog 只有两条 `commit:`，
  没有 `branch: Created from`。所以「按分支创建时间排」（被否掉的方案 A）连这个例子都只是碰巧对。
- **计划没写的坑**：amend 之后旧提交的 `commit:` 记录还在 reflog 里，但它已经不在分支上了；
  `pull: Fast-forward` 拉下来的提交没有 `commit:` 记录。证据只能当「这条分支在这里做过这个提交」，
  使用前要确认提交确实在分支的第一父链上。
- **计划没写的坑**：rebase 后的提交在分支 reflog 里没有逐个记录，证据缺失，按行序退化。
- 过期规则来自本机 git 自带文档 `git-gc`：`gc.reflogExpire` 默认 90 天，`gc.reflogExpireUnreachable` 默认 30 天。

### Task 2：后端

- **做法**：`RefPointer` 加 `made_here: Vec<String>`，只给本地分支填。读 `repo.reflog(<完整 ref 名>)`，
  消息以 `commit:`、`commit (`、`cherry-pick:`、`revert:` 开头，或 `merge ` 开头且含 `: Merge made by` 的条目，取 `id_new`，去重。
  为空时不序列化（`skip_serializing_if`），前端字段是可选的 `madeHere?`，现有测试手写的 `RefPointer` 不用改。
- **为什么不在后端判断「是否还在分支上」**：后端照实报 reflog，amend 掉的旧提交也列着；是否在第一父链上由前端判断，
  前端手里本来就有整张图。
- **读不到 reflog 时返回空、不报错**：证据缺失是常态（过期、新 clone、`core.logAllRefUpdates=false`），不能让整张图加载失败。
- **计划没写的坑**：linked worktree 打开时 libgit2 也能从 commondir 读到分支 reflog，测试 `made_here_sees_commits_made_in_a_linked_worktree` 两种打开方式都覆盖了。
- **已知代价**：每次读 refs 都要把每条本地分支的 reflog 读一遍。90 天内提交很多的分支会多传一串 oid（每个 40 字节，
  1000 个提交约 40 KB）。没做截断：截断会让最老的那段证据丢失，而分叉点恰恰可能在那里。

### Task 3：前端

- **做法**：`collectTips` 里本地分支这一级（`tiers[2]`）原来按 tip 行序，现在经过 `orderByMadeHere` 重排：
  1. 每条本地分支 tip 沿第一父链往下走，主干（`tiers[0]`、`tiers[1]` 的第一父链）上的提交不算。
  2. 任意两条分支的链第一次汇合的那个提交就是两者的分叉点。分叉点在 B 的 `madeHere` 里、不在 A 的里，就让 B 排在 A 前面。
  3. 按这些先后关系做拓扑排序，同级按 tip 行序；成环时取剩下的第一个，也就是退回行序。
- **为什么只看分叉点**：分叉点之下的历史两条分支都有，更老的提交是谁做的说明不了谁从谁切出来。测试「只看分叉点」覆盖了
  「分叉点证据过期、只剩更老的证据」这种情况：不重排。
- **为什么主干不参与**：主干固定先认领，是 lane 0 的锚点（`collectTips` 里原有的规则）。在 feature 上做、后来 master 快进到的提交，
  照样归主干。
- **为什么是安全的**：只改认领顺序，不改认领算法本身；没有证据时 `orderByMadeHere` 原样返回，结果和改动前逐位一致
  （「没有证据时退回按 tip 行序」那条测试，以及原有 1889 条前端测试全过）。不依赖 HEAD，checkout 不会让图重排。
- **偏离讨论时的说法**：讨论时说过「证据缺失时和 `git log --graph` 一致」，不准确。准确的说法是退回改动前的画法；
  改动前的布局本来就和 `git log --graph` 不完全一样（主干固定 lane 0、分级认领、车道复用）。
- `laneBranch.ts`（侧边面板显示提交在哪条分支的车道上）复用同一个 `computeOwners`，自动一致，测试里顺带断言了。

### Task 4：验证

- 两侧门槛见进度总账。`cargo test` 没有一次跑完整套：全量会被内存回收，按测试二进制分三批（lib + 非 SVN 48 个、SVN 前 30 个、SVN 后 30 个），每批都是 0 失败。
- **真实窗口**：scratchpad 里用脚本建了和 readwise-alternative 同形状的 fixture（mobile 上做 e、f；design 从 f 切出做 x1；mobile 再做 y1；design 再做 x2，tip 最新）。
  这个 fixture 上 `git log --graph` 把 f、e 画在 design 那一列。`pnpm tauri dev --config <只改 identifier 的 json 文件>` 打开它，
  核对过进程是本 worktree 的 `src-tauri\target\debug\app.exe`：f、e 画在 mobile 的车道上，design 在 f 处分出去。验完关掉进程，删掉隔离 identifier 的两个配置目录。
- **计划没写的坑**：PowerShell 里把 JSON 直接写在 `--config` 参数上会被多转义一层，tauri 报 `key must be a string`；改成传文件路径。

### 独立 review（`/code-review high`）发现与处理

5 条，都是 nit，没有高危。处理如下：

| 发现 | 处理 |
|---|---|
| 没有 master/main 时，`orderByMadeHere` 每条链走到窗口底部，再两两比较，开销约 B²·L | 改写：① 分叉点一定有至少两个以它为第一父的子，先按这条筛证据；② 链只走到最老的候选分叉点那一行；③ 只给做过候选分叉点的分支建查找集合，判断「第一次汇合」改成看前一个提交在不在对方链上，不再两两遍历。scratchpad 量最坏形状（无主干、100 条分支、4000 个共享提交、每条都有证据）：44 ms/次降到 3 ms/次，无证据时 2.3 ms/次 |
| `git pull` 产生的合并（`pull <参数>: Merge made by …`）和 `am:` 没算证据 | 先在 fixture 里实测这两种消息，再加进 `creates_commit`，补测试 `made_here_counts_am_and_merges_made_by_pull`。review 提到的 `In-index merge` 这次没复现出来，**未核实**，不处理 |
| `made_here` 把 reflog 全量发给前端，从不 gc 的仓库会一直长 | `graph::load` 在 revwalk 之后只留图里有的 oid，补测试 `graph_load_keeps_only_made_here_commits_that_are_in_the_graph`。`refs::collect` 只有 `graph::load` 一个调用方 |
| 注释把「默认 90 天」写得像自动过期 | 改成：`git gc` / `git reflog expire` 运行时才删，从不 gc 的仓库一直留着 |
| vitest fixture 说 y1 是 pull 快进来的，Task 4 的真实窗口 fixture 里 y1 是本地提交，两处说法不一致 | 两种都对：vitest 照 readwise-alternative 的现场不给 y1 证据，scratchpad 的脚本 fixture 在本地提交了 y1。判定只看分叉点 f，y1 有没有证据都不影响结果；测试注释里写明了这一点 |

- **偏离 TDD 顺序**：`am` / `pull` 合并这两条测试是和实现一起写的，没有先单独跑一次看它失败。它们断言的是改动前确实不认的消息形式（改动前的 `creates_commit` 只认 `merge ` 开头），按代码推断改动前会失败，但没实际跑过。

### review 修复后的复验

- `pnpm build`、`pnpm test`（104 文件 1891 条）、`cargo fmt --check`、`cargo clippy --all-targets` 全过。
- `cargo test`：lib + 非 SVN 48 个二进制（含 `refs`、`fixtures`）全过；SVN 第一批跑完 15 个、0 失败时，后台任务因系统内存不足被 Claude Code 回收。
  剩下 45 个 SVN 二进制改成前台、每批 9 个、`--test-threads=4` 分 5 批重跑，全部 0 失败。

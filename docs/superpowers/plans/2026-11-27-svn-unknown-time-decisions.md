# SVN 没返回时间的 revision：实现日志

来源：2026-11-26 使用报告第 6 节（对话中提供，未单独落库）。r67823 是锚点，`svn log --xml -r 67823`
只有 revision 号，没有 author / date / msg。前端 `toEpochSeconds` 把空时间当 0，显示 1970，
`dateOrder` 还把 10-30 的分支提交排到它上面、把它下面的主线截断。

与「提交图显示作者时间」（PR 第 207 号）分开做：原因不同、代码位置不同，可以各自回退。

## 进度总账

| Task | 状态 | 产出文件 | 验证 |
|---|---|---|---|
| 1. 排序：借相邻 revision 的时间，标 `timeUnknown` | 完成 | `src/engine/svnGraph.ts`、`src/engine/types.ts` | 5 条新用例先失败后通过 |
| 2. 显示：「时间未知」「SVN 未返回」、详情说明 | 完成 | `svnGraph.ts`（`commitDateText` / `commitAuthorText`）、`CommitRow.tsx`、`CommitDetailView.tsx` | 5 条新用例先失败后通过 |
| 3. 钉住 SVN 的真实返回形状 | 完成 | `src-tauri/tests/svn_xml.rs` | svnserve + authz 实测 XML；`cargo test --test svn_xml` 52 条通过 |
| 4. 全量验证 + 真实窗口 | 完成 | — | `pnpm build`、`pnpm test`（1977 条）、`cargo fmt --check`、`cargo clippy --all-targets`；真实窗口见下 |

## 决策条目

### Task 1

- **计划的错误**：报告写「借下一个更早 revision 的时间不会排错」。不准确：`svnRevs` 里只有动过
  白名单的 revision 和锚点，是跳号的，数组里的下一条可能早好几天。真实时间夹在两个邻居之间，
  借哪边都只在这段间隙里可能排偏。这次 r67822 正好紧挨着，所以没问题；一般情况下是近似。
- **计划没写的坑**：
  - 最老的一条没有时间时下面没得借 → 改借上面那条。
  - 连着几条都没时间 → 一直往下找到第一条有时间的。
  - 整条都没时间 → 留 0（和原来一样），不报错。
- **偏离报告**：报告只说「借时间」，没说显示怎么区分。排序和显示读的是同一个 `time`，借来的值写进
  `time` 后显示就是借来的时间，不是「未知」。所以加了 `timeUnknown` 标记，显示先看它。
- **为什么安全**：只改 svn 节点的 `time` 取值和一个新的可选字段；git 提交不受影响。查过没有任何
  代码靠 `time === 0` 判断「未知」。后端不动：`svn_xml.rs` 的 `parse_log` 对空 logentry 给空字符串，
  正是新用例的输入形状。删掉了不再使用的 `toEpochSeconds`（它的注释说「图的拓扑不依赖日期」，
  拓扑确实不依赖，但行序依赖，这正是这个缺陷）。

### Task 2

- 作者写「SVN 未返回」只对 svn 节点生效；git 提交作者为空时原样（不是同一回事）。
- 详情里三样都空时，说明行替代空的说明块。文案写「常见原因是……没有读权限」，没写死：
  报告里这条是推断，没核对权限配置。
- 与 PR 第 207 号都改了 `CommitRow.tsx` 取日期那一行和 `CommitDetailView.tsx` svn 分支的日期；
  后合的那个需要解一次冲突（两边都换成了函数调用，合并后应为
  「作者时间」+「时间未知」都生效）。

### Task 3：SVN 的真实返回

报告说「多半是没有读权限（推断，未核对）」。用 svnserve + authz 实测证实（`file://` 不走 authz，
必须起 svnserve）：匿名可读 `/trunk`、读不了 `/secret`，r2 只改 `/secret`。

```text
svn log --xml -r 2 svn://127.0.0.1:3699/    → <logentry revision="2"></logentry>   三样全无
svn log --xml -r 1 svn://127.0.0.1:3699/trunk → 有 author、date，没有 msg（r1 部分路径读不了）
```

- **计划没写的坑**：部分可读的 revision 有时间、没有说明。这种不会显示 1970，不在本次范围；
  只是说明为空，和用户自己提交了空说明看起来一样。
- 用这段 XML 加了 `svn_xml.rs` 的契约用例。它一加上就通过：后端本来就把缺失元素当空字符串，
  缺陷全在前端。用例的作用是把「这条要留下、三样给空」钉住，前端 `svnTimes` 依赖这个形状。

### Task 4：真实窗口

fixture（全部在 scratch 目录、本机 svnserve）：匿名 checkout `/trunk` 到 r1 → `svn_init_hybrid`
（基线 r1）→ admin 提交 r3（改 Code）、r4（只改 `/secret`）→ 匿名 update 到 r4 → `svn_follow`
（基线记在 r4，正是报告里的「锚点 revision 读不了」）→ r5 改 Code → update → follow。
最后用 `git commit-tree` 从 r1 基线挂一条 10-30 的三提交分支（不碰工作区，跟进不会重放它）。

- **计划没写的坑**：只 update 到一个没改白名单的 revision 时，`svn_follow` 回「没有要跟进的」，不录基线。
  报告里的锚点是因为那次 update 同时带来了更早的白名单改动、基线落在 HEAD 上。fixture 照这个形状造。

| 看什么 | 结果 |
|---|---|
| 行序 | svn:5 → svn:4 → svn:3 → kairos 3/2/1（10-30）→ svn:1。主线连续，10-30 的提交不再顶到 r4 上面 |
| r4 那一行 | 作者「SVN 未返回」、日期「时间未知」 |
| r4 详情 | 同上，加一段「SVN 只返回了 revision 号……」说明；页面上没有 1970 |
| 运行的二进制 | `app.exe` 在本 worktree 的 `src-tauri\target\debug` 下，隔离 identifier，验完已删配置目录 |

没有在 master 的构建上复现原缺陷：失败用例（`不再把更早的 git 提交顶到它上面`）在改动前失败，
已覆盖同一形状。

### Task 5：独立 review（`/code-review medium`）

1 条 important、5 条 nit：

| 条目 | 处理 |
|---|---|
| important：「Copy Commit Info」仍读 `time`、`author`，把借来的时间当真实时间复制出去（改之前至少是一眼假的 1970） | 已修：`formatCommitInfo` 改用 `commitDateText` / `commitAuthorText`，先补失败用例 |
| 共用的 `CommitHeader` 仍直接读 `time`、`author` | 已修，统一走两个函数 |
| `svnTimes` 逐条往两边 `slice` 查找，空记录多时 O(n²) | 已修，改成两遍线性扫，原 5 条用例照过 |
| 头像取「SVN 未返回」的「S」，像真实作者首字母 | 已修，作者为空时显示「?」，用例覆盖 |
| `/1970/` 断言依赖时区（UTC 以西是 1969-12-31） | 已修，改成断言 `new Date(0).toLocaleString()` 不出现 |
| 借较早邻居的时间会和它同秒并列，建议 +1 或取中点 | **不改**。并列时按输入顺序，svn 节点排在 git 之后，影响只在同一秒；真正的误差来自「真实时间在两邻居之间未知」，+1 或中点都不消除，只是换一种近似。已在 `svnTimes` 注释和 Task 1 条目里写明是近似 |

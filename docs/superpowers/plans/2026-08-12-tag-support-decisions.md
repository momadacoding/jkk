# 2026-08-12 Tag 支持 — 实现决策日志

对应计划：`~/.claude/plans/tag-commit-graph-tag-4ca4dc356a452302dd-deep-corbato.md`（plan mode 产物，不在仓库内，故决策日志按 `AGENTS.md` §2.1 落在本目录）。

分支：`worktree-tag-support`，worktree：`.claude/worktrees/tag-support`，基点 `8755dde`。

## 进度总账

| # | Task | 状态 | 产出文件 | 验证 |
|---|---|---|---|---|
| 1 | Rust：annotated tag peel 到 commit | 完成 | `src-tauri/src/engine/refs.rs`、`src-tauri/tests/refs.rs` | 新增 2 条测试，改前 `annotated_tag_…` 实测失败（left `d7985f3` 是 tag object，right `acd3c8c` 是提交），改后 7/7 通过 |
| 2 | Rust：tag 目标推进 revwalk | 完成 | `src-tauri/src/engine/graph.rs`、`src-tauri/tests/fixtures.rs` | 新增 1 条测试，改前失败（图里只有 `["c2","c1"]`），改后通过 |
| 3 | Rust：新增 `push_tag` 命令 | 完成 | `src-tauri/src/commands/remote.rs`、`src-tauri/src/lib.rs`、`src-tauri/tests/remote.rs` | 新增 3 条测试，`cargo test --test remote` 9/9 通过 |
| 4 | 前端：`tagOrder` 纯逻辑 | 完成 | `src/engine/tagOrder.ts`、`tests/vitest/tagOrder.test.ts` | 9/9 通过 |
| 5 | 前端：`TagView` + 接进 `RepoTab` | 完成 | `src/components/TagView.tsx`、`src/components/RepoTab.tsx` | `pnpm build`（tsc）通过 |
| 6 | 验证 + 独立 review | 代码侧完成 | 见下方 review 小节 | `cargo fmt --check` / `cargo clippy --all-targets`（0 warning）/ `cargo test` **163 passed** / `pnpm build` / `pnpm test` **212 passed**。独立 review 报 5 条，全部核实属实并修完后复跑全绿。**真实窗口手工验证待做** |

## 决策条目

### 立项：根因实测

用户报告 graph 上看不到 `4ca4dc3` 的 tag。在本仓库跑只读命令确认：

```
git for-each-ref --format='%(refname) %(objecttype) %(objectname) -> %(*objectname)' refs/tags
refs/tags/v0.1.0 tag 35330adbfc96f7ed52dde1d5cbafbb0aaeab1d6d -> 4ca4dc356a452302dd10f469e299e04573fbe7a6
```

**计划没写的坑**：`objecttype` 是 `tag` 而不是 `commit` —— 这是**附注 tag**。`refs.rs:47` 的 `reference.target()` 返回 tag object 的 OID，而前端 `GraphView.tsx:165-171` 用 `ref.hash` 与 `CommitNode.id` 精确匹配来分组，于是这条 ref 永远匹配不到任何一行，pill 不渲染。轻量 tag 走同一条路径却是对的，所以这个 bug 只影响附注 tag —— 这也解释了为什么 `.ref-pill.tag` 的 CSS（`styles.css:482`）早就存在却从没人见过它生效。

侧边栏没有 tag 是**另一个独立问题**（`BranchView.tsx:76-77` 只筛 branch/remote，无任何组件消费 `kind === 'tag'`），不是同一个根因的两个表现。

### 范围决策（用户在对话中确认）

| 决定 | 内容 | 理由 |
|---|---|---|
| 不做 Delete Tag | 侧边栏只有 Checkout / Push Tag | 用户明确排除。附注 tag 删掉不可恢复，少一个危险入口 |
| 不区分 annotated / lightweight | `RefPointer` 不加字段 | 保持最小 diff；加下划线字段会强制给结构体补 `#[serde(rename_all = "camelCase")]`，牵连所有既有消费方 |
| tag 推进 revwalk | `graph.rs` 额外 push tag 目标 | 否则只被 tag 指向的 commit 不在图里，侧边栏点它必定定位失败 |

### Task 1 — annotated tag peel

**为什么这么改是安全的**：只对 `kind == "tag"` 走 `peel_to_commit()`，分支/远程分支的取值路径一字未动，既有 5 条 ahead/behind 测试全部保持通过。peel 失败时**跳过该 ref** 而不是 `?` 传播 —— 一个指向 blob/tree 的畸形 tag 不该让整张图加载不出来。`RefPointer` 的字段和 serde 形状没动，前端 `types.ts` 无需同步。

**计划没写的坑**：轻量 tag 的回归测试必须同时断言 `hash == rev-parse v2^{commit}` **和** `hash == rev-parse v2`。附注 tag 的用例反过来必须断言 `!= rev-parse v1`。少了那条 `assert_ne!`，一个什么都不做的实现也能让用例变绿 —— 那就是 `AGENTS.md` §13.9 说的虚假信心。

### Task 2 — tag 推进 revwalk

**偏离计划**：计划说「把 `refs::collect` 提前到 revwalk 之前」，实际这么做了，但多了一步计划没提的转换：`RefPointer.hash` 是 `String`，`walk.push()` 要 `git2::Oid`，中间需要 `Oid::from_str`。解析失败一并跳过。

**为什么这么改是安全的**：只 push `kind == "tag"`，明确不含 `other` —— `refs/stash` 和 `refs/notes` 都指向提交，push 进去会把 stash 的提交和 notes 拉进主图，那是可见的行为退化。`walk.push` 的错误被吞掉（`let _ =`）而不是传播，理由同上：单条坏 tag 不该拖垮整张图。

### Task 3 — `push_tag`

**计划没写的坑**：空名的危险比计划里写的更具体。`refs/tags/{name}:refs/tags/{name}` 在 name 为空时变成 `refs/tags/:refs/tags/`，**冒号左边为空正是 git 删除远端引用的写法**。所以空名守卫不只是「参数校验」，它挡的是一个删远端引用的形态。守卫用 `trim()` 后判空，纯空白也挡。

**偏离计划**：计划只要求测空名守卫和一条推送成功。实际多加了 `push_tag_does_not_push_a_same_named_branch` —— 同时建同名的分支 `v1` 和 tag `v1`，断言只有 `refs/tags/v1` 上了远端、`refs/heads/v1` 仍为空。这条才真正证明全限定 refspec 有用；只测「推上去了」的话，用短名的实现也能通过。

**为什么这么改是安全的**：不带 `--force`，与既有 `create_tag` 不带 `-f` 同一理由 —— 静默改写别人已经拉走的 tag 是不可接受的。测试全程走 bare 远程的本地路径（复用 `remote.rs` 既有的 `trio()` fixture），不碰网络、不碰真实仓库。

### Task 4/5 — 前端

**偏离计划**：计划说排序「时间相同或 commit 不在图里时按名字**倒序**兜底」。实际实现里把「不在图里」和「同时间」拆成了两条独立规则：不在图里的一律沉底（无论名字），同时间的才比名字。计划的写法会让一个不在图里的 `zzz` 排到在图里的 `aaa` 前面。

**计划的错误**：计划说 `TagView` 的短名用「组件内本地 helper」。实际把 `tagShortName` 放进了 `src/engine/tagOrder.ts` —— 因为 `sortTags` 的测试本来就要断言短名，helper 留在组件里测试就够不着。这不违反分层（`engine/` 仍然无 React、无 Tauri 依赖）。

**为什么这么改是安全的**：`state.refs` 里本来就有 tag，接 `TagView` **没有新增 store slice、没有新增 refresher** —— `refreshGraph` 已经覆盖，不存在漏刷新的路径。`handleSelectTag` 照 `handleSelectWorktree` 的先例先校验提交在不在图里，避免留悬空 `selectedId`。checkout 传全限定 `refs/tags/<name>`：同时存在同名分支时 git 会优先选分支，短名会检出到完全不同的地方。

### 这个 bug 为什么一直没被发现

`docs/superpowers/e2e/2026-08-04-mygitclient-e2e-checklist.md:48` 有一条已勾选的
「refs 标签（branch/tag/head）正确显示在 commit row 上 —— 与 `git for-each-ref` 一致」。
那次验证的仓库里只有轻量 tag，而轻量 tag 走的路径本来就是对的，所以这条勾是**真空通过**的
—— 正是 `AGENTS.md` §13.9 说的「测的不是 X 的真实形态」。附注 tag 是 `git tag -a` 和
GitHub Release 的默认产物，实际用起来反而更常见。

不改那份 checklist（它是当时那次验证的历史记录），但下次补 e2e 时该条必须拆成
annotated / lightweight 两种形态分别验。

### 真实仓库上的端到端确认（只读）

用一个临时的一次性测试对本仓库跑 `engine::graph::load`（纯读，不做任何写操作），
确认用户报的症状确实修好了，跑完即删、不进提交：

```
v0.1.0 -> 4ca4dc356a452302dd10f469e299e04573fbe7a6
matched commit row: 4ca4dc3 fix(release): publish job 缺少 GH_REPO 导致 gh CLI 找不到仓库 (#23)
```

即 tag 的 hash 现在等于用户点名的那条提交，且该提交确实在 `commits` 里有对应行 ——
这正是前端 `refByHash[node.commit.id]` 画 pill 所需的全部条件。

### Task 6 — 独立 review 发现并修复的问题

`/code-review high`（独立于实现者，满足 §4.1 门槛）报了 5 条，全部核实属实、全部已修：

| # | 问题 | 修法 |
|---|---|---|
| 1 | `push_tag` 硬写 `origin`，远端叫 `upstream` 的 fork 仓库永远推不出去，且界面无处可改 | 新增 `tag_remote()`：有 `origin` 用 `origin`；否则唯一远端就用它；没有远端报 `no_remote`；多远端且无 `origin` 报 `ambiguous_remote`，不猜 |
| 2 | **我引入的回归**：peel 失败时整条 ref 被丢掉，TAGS 数量会和 `git tag` 对不上且无任何解释 | peel 失败退回 `reference.target()`。这类 tag 列得出来、点击提示「不在提交图中」，比静默消失准确 |
| 3 | 双击 tag 必先触发 onClick，会先弹一条「不在当前提交图中」再成功 checkout，红条自相矛盾 | `handleCheckoutTag` 开头 `clearError()` |
| 4 | `handleCheckoutTag` 漏了 `refreshUpstream`，进游离 HEAD 后工具栏仍显示原分支的 ↑↓ | 补上 `refreshUpstream` |
| 5 | `defaultOpen={sorted.length > 0}` 永远为 false —— `SidebarSection` 只在挂载时读一次，而挂载时 refs 还没加载 | 去掉该 prop，跟 BRANCHES 一样默认展开 |

**计划的错误**：计划在步骤 3 明确写了「remote 硬编码 `origin`，与既有 `push()` 一致」。这个类比是错的 —— `push()` 只在补 `-u` 时才写死 `origin`，平时靠 git 解析分支自己配的远端；`push_tag` 硬写就是真的只能推 origin。按计划原样实现会留一个 fork 工作流下必现的 bug。

**计划的错误（2）**：计划步骤 1 写「peel 失败时**跳过这条 ref**」。这条指示本身会造成回归 —— 改之前那条 ref 至少还在 payload 里。已改为退回 `target()`。

新增回归测试 4 条：`a_tag_on_a_blob_is_still_listed`（用 `git hash-object -w` 造真 blob，再 `git tag` 挂上去，覆盖 #2 的真实形态）、`push_tag_works_when_the_remote_is_not_named_origin`、`push_tag_without_any_remote_reports_a_clear_error`、`push_tag_refuses_to_guess_between_several_remotes`。

clippy 在这轮报了一条 `using contains() instead of iter().any()`，按 §7.3 直接改代码，没有用 `#[allow]` 掩盖。

### 待做：真实窗口验证

`push_tag` 是新增的 Tauri 命令签名，`AGENTS.md` §7.3 要求实际在 `pnpm tauri dev` 里 invoke 一次 —— `snake_case` ↔ `camelCase` 的转换没有编译期检查。本次命令只有 `path` 和 `name` 两个单词参数，不存在下划线转换，但仍需实跑确认。**在此之前不得声称本功能已完整验证。**

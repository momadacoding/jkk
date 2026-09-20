# 2026-09-21 独立 review 发现的修复 —— 决策日志

> 分支 `fix/svn-baseline-junction`，基点 `2ea08f5`（`feat/svn-phase1c`，含已合并的 PR #44）。
> 起因：对 PR #43（`feat/svn-phase1c` → `master`）跑的 Codex 对抗式 review，
> 三条发现全部由本会话在临时 fixture 上实测复核过（复核记录见下方各 Task）。
>
> 本文件不是事后总结，是 review 的输入（AGENTS.md §2.1）。

## 进度总账

| Task | 内容 | 状态 | 产出 | 验证 |
|---|---|---|---|---|
| 1 | 白名单内的 junction 会把服务器上没有的文件录进基线 | ✅ 完成 | `svn_dirty.rs`、`tests/svn_dirty.rs`、`tests/svn_init.rs`、`tests/common/svn_fixtures.rs` | `cargo test --test svn_init` 33 passed；`--test svn_dirty` 9 passed；正交变异两轮 |
| 2 | 第二档同步校验失败后，同一 revision 不再重试 | ✅ 完成 | `svnSync.ts`、`repoStore.ts`、`tests/vitest/svnSync.test.ts` | `pnpm test` 354 passed；正交变异 |
| 3 | 体检「通过」的文案越权：没验过的路径不告诉用户 | ✅ 完成 | `svn_health.rs`、`svn_init.rs`、`svnInit.ts` + 4 个测试文件 | `cargo test` 537 passed；`pnpm test` 354 passed；正交变异 |
| 4 | `repoStore.ts` 里一段悬空 JSDoc | ✅ 完成 | `repoStore.ts` | `pnpm build` |

## 全量验证（2026-09-21，全部本地实跑）

| 命令 | 结果 |
|---|---|
| `cargo fmt --check` | OK |
| `cargo clippy --all-targets` | **0 告警** |
| `cargo test` | **537 passed / 0 failed**（基线 530，新增 7） |
| `pnpm build` | OK（`tsc` + `vite build`） |
| `pnpm test` | **354 passed**（基线 345，新增 9） |

> 本仓库没有 PR 门禁 CI（AGENTS.md §4.1），以上全部是本地实际执行的结果。

## Task 1 —— junction 混进基线

### 复核实测（2026-09-21，临时 SVN fixture + 临时 git 库，跑完即删）

三段链路逐段量过，全部成立：

| 量的是什么 | 实测结果 |
|---|---|
| `svn status --no-ignore -v -- Code`，`Code/Mirror` 是指向库外目录的 junction | 只有一行 `?  Code\Mirror`，里面的 `Linked.cs` 一行都没有 |
| Rust `symlink_metadata("Code/Mirror").is_dir()` | **`false`**（`is_symlink()` 为 `true`，`Path::is_dir()` 为 `true`） |
| `git add -A -- Code/`（exclude 规则照 `svn_exclude.rs` 的形状写） | 索引里出现 **`Code/Mirror/Linked.cs`** |

于是 `is_plain_dir` 返回 `false` ⇒ `expand_dir` **根本不会被调用** ⇒ 没有任何 `Fix::Drop`
⇒ 服务器上不存在的文件留在一棵自称「等于服务器 rN」的树里。

### 四道检查为什么一道都不响

| 检查 | 为什么不响 |
|---|---|
| P1 `svn status` | `?` 不阻断；§16.1 之后只剩 `~`（obstructed）阻断 |
| 集合核对 | `expected_set` 的 `walk` **故意跟随 junction**（`svn_include_check.rs` reparse 一节），两边一致 |
| `Fix::Drop` 的执行 | `remove_path` 走 `update-index --force-remove -- Code/Mirror`，**不带 `-r`**；索引里的条目是 `Code/Mirror/Linked.cs`，这一条是 no-op |
| 基线体检 | svn 看不见那个文件 ⇒ 它不在 `D` 里 ⇒ `D \ S == ∅` ⇒ `Verified` |
| 服务器侧核对 | `svn_server_set.rs` 明写「**只报这一个方向**」（服务器有、本地没有），反方向不报 |

### 这是本 PR 开出来的口子

§16.1 之前「录基线要求工作副本干净」，一条未版本控制的 junction（`? Code/Mirror`）
会直接把初始化拦下。放松之后它畅通无阻。而 `svn_dirty.rs` 自己的注释写着
「Unity 工程里 junction 并不罕见」。

### 怎么修的

`svn_dirty.rs` 两处，都是「与 git 对齐」这一条原则的贯彻：

1. `is_plain_dir` → `is_dir_on_disk`：判据从 `symlink_metadata().is_dir()`（不跟随）
   换成 `Path::is_dir()`（跟随）。原来的注释把「防环」当成不跟随的理由，**但防环
   该在遍历里做，放在这道门上等于把整扇门关死**。
2. `expand_dir` 由迭代改成递归的 `walk_expand`，带 `chain` 祖先链，与
   `svn_include_check::walk` **逐行同构**——两侧本来就该看同一棵树。

### 计划没写的坑（实测）

- **环形 junction 的每一层 `Path::is_dir()` 都是 `true`。** 所以这个遍历除了祖先链
  守卫之外**没有任何终止条件**。git 自己也只是撞上 Windows 重解析点 63 层的上限
  才停的（独立探针：65 个索引条目，最深一条 63 层）。**不能拿「操作系统恰好会
  拦住」当终止性论证**，那是别人的实现细节。
- **端到端那条用例的结局不稳定。** 同一份 fixture 反复跑，多数次是「成功且基线树
  干净」，也见过一次 `include_set_mismatch` 阻断——差别在 `git add` 那一侧到底
  递归多深。所以 `tests/svn_init.rs` 那条只断言两种结局**都**必须成立的性质
  （会返回、基线树里没有环形路径），确定性的终止性断言下沉到 `tests/svn_dirty.rs`
  直接打在 `read_dirty` 上，不经过 git。
- ⚠️ **`Copy-Item` / `Move-Item` 会保留 mtime，cargo 因此不重编译。** 做正交变异
  验证时用它还原源文件，之后几轮跑的其实都是**变异后的二进制**，一度让我得出
  「守卫开关都不改变结果」这个完全相反的结论。变异与还原之后都必须显式
  `(Get-Item $f).LastWriteTime = Get-Date`。

### 正交变异验证（重做过一次，第一次因上面那个 mtime 坑作废）

| 变异 | 结果 |
|---|---|
| 把 `if chain.contains(&c)` 改成恒假（拆掉环守卫） | `--test svn_dirty` **恰好 1 条红**，正是环那条 |
| 还原 | 9 passed |
| 修复前（`symlink_metadata` 那版） | `--test svn_init` 两条 junction 用例红，环那条绿 |

## Task 2 —— 同步重检卡死

### 复核

`checkSvnSyncIfChanged` 在 `await refreshSvnSync(path)` **之前**就把 `lastRootRevs`
推进到新值，而调用点（`repoStore.ts` 的 `repo-changed` 监听器）是
`.catch(ignore)`。完整校验一抛错，观测值已经前进，下一次事件看到同一个 revision
就直接返回——失同步状态停在旧值，直到 revision 再变一次或重开仓库。

`tests/vitest/` 里搜不到任何 `checkSvnSyncIfChanged` / `lastRootRevs` /
`svnRootRevs` 的用例，**零覆盖**。

### 怎么修的（偏离 review 建议之处）

review 建议「仅在完整成功后提交 `lastRootRevs`」。照做了，但**没有**按它说的
「在完整检查前后各读一次 rootRevs、变化时重试」——那会把一次本地读变成三次，
而同样的保护用**更简单的方式**就能拿到：只记「开跑**之前**」那一次观测值。
校验期间工作副本要是又前进了，记下的偏旧值会让下一次事件判不等、再跑一遍。
多跑一次本地读，远好过漏一次。这也顺带解决了 review 点的 `Promise.all`
快照不一致——那两条命令现在是顺序发的，不再并发取两个时刻的事实。

规则抽到 `src/engine/svnSync.ts` 的 `recordAfterSuccess` / `syncIfRootRevChanged`：
`repoStore` 里的闭包函数在 vitest 中测不到（§7.2），而「什么时候允许写观测值」
恰恰是这条缺陷的全部内容。`repoStore` 现在只是适配器，`lastRootRevs` 从
`useRef<string | null>` 变成 `useRef<RootRevMemo>`。

### 正交变异验证

| 变异 | 结果 |
|---|---|
| 把 `memo.value = observed` 挪到 `await runFullCheck()` **之前** | **恰好 2 条红**，正是那对顺序用例 |
| 还原 | 354 passed |

## Task 3 —— 体检文案越权

### 复核

`check_baseline` 的判据 `D \ S == ∅` 把 `S` 里的路径**整批豁免**；手动体检那条
路径上的第二份证据（基线树 vs 磁盘树）**同样**把 `explained` 滤掉。所以 `S` 里
每一条都没被任何证据碰过。而 `Health::Verified { rev }` 只带一个 revision 回前端，
`describeHealth` 于是无条件显示「磁盘内容与服务器上该 revision 一致。」

`svn_init.rs` 里本来就有一段 ⚠️ 写明了这个代价（spec §6.8.7 的已知取舍），
**缺的不是判据，是如实告知**。2026-09-19 用户在 1c 场景 5c 被绊到的就是这句文案。

### 偏离 review 建议之处（重要）

review 要求「对 explained 路径直接比较基线 blob 与服务器/BASE 内容」。**没有照做**，
采用的是它自己给出的备选：「列出未覆盖路径，禁止在存在未验证路径时返回 Verified」。
理由两条，都不是嫌麻烦：

1. **它会在最常见的流程上变成误报。** 基线 blob 是 `git hash-object --path` 处理过
   的字节（`core.autocrlf=true` 下 git 会做 EOL 转换，这在本项目已经踩过），
   而 `svn cat -r BASE` 给的是原始字节。直接比会在每个 CRLF 仓库上报假 `Lying`。
2. **改用服务器侧 `svn cat @rev` 逐文件比**能绕开 EOL，但那是**每个脏文件一次网络
   往返**，而 `check_baseline` 也跑在初始化那条路径上。几十个脏文件就是几十次
   往返，把「建库」从秒级拖成分钟级。

所以这一期只把「覆盖不到哪些」如实报出去。真正的逐文件验证留作后续，**前提是先
解决 EOL 归一化**——见文末遗留。

### 为什么不是一刀切地报「未覆盖」

初始化 / 重建那条路径上，`S` 里的 blob 是几毫秒前由 R2b 从 `svn cat -r BASE`
写进去的，而 P2 要求 `nodeRevs` **单值**、`rev` 取的就是那个值 ⇒ `BASE == rev`
⇒ 那些 blob 就是服务器 `rev` 上的内容，**由构造保证正确**。在那里报「有 N 个文件
未覆盖」是纯噪音，而且每次建库都会出现。

所以加了 `ExplainedOrigin::{JustRecorded, Unknown}`，**不给默认值**——每个调用点
必须表态。手动体检是 `Unknown`（基线是谁、什么时候录的都不知道）。

## Task 4 —— 悬空 JSDoc

`repoStore.ts` 里「重新读取被钉死的 external」那段 JSDoc 后面紧跟着另一段 JSDoc，
于是它谁也没文档到，而它描述的函数 `refreshSvnExternals` 在文件里根本不存在。
查下来它是 `refreshPinned` 的文档，被后来插入的代码挤散了（`refreshPinned` 自己
反而一句注释都没有）。搬回去即可。独立 review 和 Codex 都没抓到这条。

## 操作笔记（踩过的，不是代码问题）

- **`Set-Location ..; pnpm test` 写在同一次调用里会跑错包根。** 实测两次得到
  「208 个测试文件 / 2671 条」——那是把主仓库底下**所有** worktree 的测试都收进来了。
  以 worktree 根目录作为调用自身的 cwd 时稳定是 24 文件 / 354 条。
- 新 worktree 要先 `pnpm install`，否则 `pnpm test` 会静默落到祖先目录的 vitest 上。

## 遗留（本期**没做**，理由已论证）

| 项 | 为什么留着 |
|---|---|
| 对 `S` 里的路径做真正的逐文件验证 | 需要先解决 blob 与 pristine 的 EOL 归一化（`core.autocrlf`），以及网络往返的开销。见 Task 3 |
| `refreshSvnSync` 的 `setState` 没有 `samePath` 守卫 | 与 `refreshSvnAudit` 同一类跨仓库竞态，但**改动前就有**，不在本次 review 的三条发现里。最小 diff 原则，单独处理 |
| 环形 junction 下 `init_hybrid` 的结局不稳定 | git 自己的递归深度不稳定所致，而两种结局都是安全的（阻断 / 干净基线）。`svn_include_check.rs` 早有结论：那种工作树本来就是坏的 |

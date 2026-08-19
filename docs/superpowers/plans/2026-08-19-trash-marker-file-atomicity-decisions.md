# 回收站状态标记从目录名迁到标记文件 — 决策日志

> 对应工作：修 Windows 上 `delete_worktree` 落名失败（`trash_commit` / `Access is denied. (os error 5)`）。
> 无独立 plan 文件：根因已定位、实现路径明确，按 `AGENTS.md` §3 走 TDD 直接修复；本文件按 §2.1 承担进度总账与决策记录。

## 背景与根因

用户在 `readwise-alternative` 删除 worktree `review-screen-safe-area` 时报：

```
[trash_commit] 回收站条目落不了名：Access is denied. (os error 5)
已经原样放回，什么都没改。
```

实测确认（`MoveFileExW`，与 Rust `fs::rename` 同一底层调用）：

| 场景 | 结果 |
|---|---|
| 子孙文件被打开（share=Read） | `err=5` |
| 子孙文件被打开（share=ReadWrite,Delete） | `err=5` |
| 目标目录已存在（REPLACE_EXISTING） | `err=5` |
| 子目录是别的进程 cwd | `err=5` |
| 上述句柄关闭后重试 | `ok` |

**Windows 上重命名目录要求整棵子树没有任何打开的句柄**（连带 `FILE_SHARE_DELETE` 的规矩开法也挡）。
而原设计把回收站槽位的状态编码在**目录名后缀**上，四处状态转换全是「重命名一个装着整棵工作树的目录」：

1. `commit_slot`：`.partial` → 正式（删除的提交点）
2. `claim_slot`：正式 → `.partial`（撤销认领）
3. `claim_for_pruning`：正式 → `.pruning`（清理认领）
4. `restore_bail_with_error`：`.partial` → 正式（撤销失败复原）

POSIX 的 `rename` 不看子树句柄，所以这个设计在 Linux/macOS 上永不暴露，在 Windows 上是概率性失败。

**这是设计缺陷，不是环境问题。**

## 方案

状态标记从**目录名**迁到**槽位内的小文件**。槽位目录名一经创建永不改变；
所有状态转换退化为「同目录内重命名一个小文件」——原子、跨进程互斥，且不受子树句柄影响。

| 标记文件 | 含义 | 对应旧目录后缀 |
|---|---|---|
| `meta.json.partial` | 删除搬移进行中 | `.partial`（删除中） |
| `meta.json` | 正式条目 | 无后缀 |
| `meta.json.restoring` | 撤销认领中 | `.partial`（撤销中） |
| `meta.json.pruning` | 清理认领中 | `.pruning` |

兜底：对**不可避免要重命名带内容目录**的地方（数据搬移本身）加短重试退避。

## 进度总账

| # | Task | 状态 | 产出 | 验证 |
|---|---|---|---|---|
| 1 | 决策日志建档 | ✅ 完成 | 本文件 | — |
| 2 | 写失败测试 | ✅ 完成 | `worktree.rs` 三个句柄形态用例 | — |
| 3 | 确认测试变红 | ✅ 完成 | — | 三个用例全部以 `Access is denied. (os error 5)` 失败，与用户报的错**逐字一致** |
| 4 | 实现标记文件状态机 | ✅ 完成 | `worktree.rs` | `cargo test --lib` 18 passed |
| 5 | 重试兜底 `rename_retry` | ✅ 完成 | `worktree.rs` | `rename_retry_waits_out_a_transient_lock` |
| 6 | 全套验证 | ✅ 完成 | — | `cargo fmt --check` OK / `cargo clippy --all-targets` 0 warning / `cargo test` 全绿（含 `worktree_ops` 24 passed） |
| 7 | 独立 review（`/code-review high`） | ✅ 完成 | 3 项 low，全部处理 | 复验：`cargo fmt --check` OK / clippy 0 warning / `cargo test` 全绿 |

## 独立 review 的处理

review 独立复跑了 fmt / clippy / `--lib` 19 项 / `worktree_ops` 24 项，判定核心变更 sound：
逐条走过状态转换后确认认领互斥不变量成立，失败方向全部落在「不删」一侧，
并确认了新槽位名（`{name}-{stamp}[-{n}]`，恒以数字结尾）不可能撞上旧后缀检查。

| # | findings | 处理 |
|---|---|---|
| 1 | 被清理认领的条目撤销时返回 `trash_state`「稍后再试」，但它注定要没，重试永远不会成功（旧方案因为目录改名会正确返回 `trash_gone`） | **已修**：`Pruning`/`Unknown` → `trash_gone`，`Restoring`/`Deleting` → `trash_state`。测试断言同步收紧 |
| 2 | 六处 doc 注释仍在描述已被本次移除的目录名后缀机制 | **已修**：`delete_worktree` 两处行内注释、`RESIDUE_TTL_SECS`、`should_reap_partial`、`should_reap_pruning`、`spawn_prune`、一处测试 doc 全部改写 |
| 3 | 进程崩在撤销中途会把槽位永久留在 `Restoring`：撤不了、也不被回收，回收站可无限增长 | **不在本 PR 修**（见下） |

顺带修掉 review 的 cosmetic nit：`rename_retry` 实际是 1 次首发 + 10 次重试（10 段退避、合计 1 秒），
注释原来含糊写成「最多重试几次」，已写精确。

## Task 9 — 版本隔离：新槽位改写到 `.trash-v2`

第二轮 review 指出一个**我漏掉的兼容方向**：Task 4 只保证了「新代码读旧槽位」，
没保证「**旧代码读新槽位**」。核实后成立，且比「竞态」这个说法更确定。

#### 问题（已用 `git show 809f1bb` 逐字核实）

旧版 `prune_trash` 只按**目录名后缀**分类，其余一律进选举池：

```rust
if name.ends_with(PARTIAL_SUFFIX)  { ...; continue; }
if name.ends_with(PRUNING_SUFFIX)  { ...; continue; }
let ts = read_meta(&path).map(|m| m.deleted_at).unwrap_or(0);  // 读 meta.json
slots.push((ts, path));
```

新版进行中槽位目录名不带 `.partial`，里面只有 `meta.json.partial` →
旧版读不出 `meta.json` → `unwrap_or(0)` → 排序后是「最旧」→ 回收站一超限就
**第一个**被认领并 `remove_dir_all`。

危害面（这一层 review 意见没展开，实际比「全盘不兼容」窄）：

| 新版状态 | 旧版拿到的 ts | 后果 |
|---|---|---|
| `Committed` | 真实时间戳 | ✅ 行为完全正确 |
| `Deleting` | **0** | ❌ 超限时第一个被删 |
| `Unknown` | **0** | ❌ 同上 |
| `Pruning` | 0 | 被重复认领删除，本来就该死，无害 |

两个窗口，严重度差很多：搬移中途只是毫秒级竞态；真正要命的是**回滚失败后
保留的槽位**——长期停在 `Deleting`、装着未提交内容的唯一副本、新版承诺永久保留，
而旧进程**每次启动清理都会命中**。那不是撞概率，是必然。

触发前提是有 0.2.1 及更早的进程仍访问同一仓库（版本回退、两个安装并存）。
0.2.1 已发布（`caec3b9`），旧二进制在野。

#### 修法：目录隔离
- 新槽位一律进 `.worktrees/.trash-v2/`（`TRASH_DIR`），旧版扫的是 `.trash`，
  根本不会打开它 → 零歧义、零残留风险。
- 新版认得两边：`prune_trash` 收**主仓库目录**，同时扫 `.trash-v2` 和 `.trash`
  （旧格式条目在 `slot_state` 下判定本来就正确）。

#### 计划没写的坑
- **两个目录必须合并参与同一次选举**，不能各留 5 个。分开的话旧目录会永远
  卡在 5 个不动、永不排空（`slots.len() <= TRASH_KEEP` 直接 return）。
  合并之后旧目录里时间戳更早的条目先被清，旧回收站自然排空。
  钉子：`both_trash_dirs_share_one_election`。
- 备选方案都更差，记下来免得下一个人重走：让目录名永远带 `.partial`（旧版会
  跳过）会产生新歧义——`.partial` + 只有 `meta.json` 分不清「新版正式条目」和
  「旧版半截残留」，得再改标记名才能消歧；双写一份 `meta.json` 当诱饵只能把
  「必定第一个被删」降级成「按时间轮换最终被删」，对「永久保留」的承诺仍不成立。

#### 为什么这么改是安全的
- 隔离方向是**新躲开旧**，不需要旧版配合任何事——旧版代码一行不用改就安全了。
- 旧回收站只读不写，存量条目继续参与选举，不会因为改版而变成孤儿。
- 新增两个钉子：`new_slots_never_land_in_the_legacy_trash_dir`（隔离不变量）、
  `both_trash_dirs_share_one_election`（合并选举 + 排空顺序）。
- 验证：`cargo fmt --check` OK、clippy 0 warning、`cargo test` 全绿。

## Task 8 — 删掉整条撤销链（用户决定）

review 收尾后用户提出撤销功能多余、要求删掉，理由之一是「界面上没有操作入口」。

#### 偏离计划 / 前提更正
- **该前提不成立**：`RepoTab.tsx` 删完 worktree 后会渲染一条撤销横幅（「已删除 worktree
  「x」，目录还在仓库的回收站里。」+ 撤销按钮），代码注释还特意写了它不自动消失。
  已把这一点连同「删除对话框正向用户承诺可撤销」一并向用户说明，用户在知情后
  仍选择**整条链删干净**。照办。

#### 删除范围
| 层 | 删掉的东西 |
|---|---|
| UI | 撤销横幅 JSX、`undoDelete` state、`handleUndoDelete`、`.banner-undo*` 三条 CSS |
| 跨组件 | `onRepoRestored` prop 与 `App.handleRepoRestored` |
| 命令 | `restore_worktree` + `lib.rs` 里的注册 |
| 状态机 | `MARK_RESTORING`、`SlotState::Restoring`、`claim_slot`、`begin_restore`、`restore_bail*`、`rollback_restored_worktree` |
| 测试 | 5 个撤销集成用例 + 3 个撤销单测 |
| 文案 | 删除对话框、`worktree_dirty` 错误里「可以撤销」的承诺 |

#### 计划没写的坑
- **`onRepoRemoved` 的 `boolean` 返回值只为撤销服务**（`reopen`：删的时候有没有标签开着它，
  撤销时要不要把标签开回来）。删掉撤销后它退化成 `void`，顺带消掉了 `App.tsx` 原注释里
  描述的那个隐患（删分支那条路没有撤销入口，记下的路径永远没人消）。
- **`TrashMeta` 必须继续写，哪怕程序不再消费它**。手工把 worktree 捞回去需要知道原路径，
  尤其是 `admin`（`.git/worktrees/<name>`）——光看槽位内容猜不出来。`deleted_at` 也仍是
  清理选举的排序依据。已在类型注释里写明用途转变。
- **`DeletedWorktree` 保留**：前端不再消费，但它是删除的事实记录，且集成测试靠
  `trash_path` 定位槽位做断言。删掉它会让一批测试改用猜路径。
- 删掉 `undo_restores_the_exact_state_including_the_index` 会**连带拔掉一颗仍然有效的钉子**
  ——「工作树和管理目录必须成对搬移」（index 在管理目录里，只搬工作树会丢暂存状态）。
  没有撤销之后这条不变量反而更要紧：回收站里那份副本必须完整，用户才捞得全。
  已补 `the_worktree_and_its_admin_dir_move_together` 顶上，直接断言 `slot/admin/index` 存在。
- 删除对话框的文案改成写出回收站路径（`.worktrees/.trash`）：没有撤销入口之后，
  它是用户自救的唯一线索。

#### 为什么这么改是安全的
- 删的全是**恢复路径**，删除路径本身一行未动：两次搬移、回滚、落名、清理选举、
  认领互斥全部原样保留，对应测试也全部保留。
- 回收站没有降级为「直接删」：东西照样搬进 `.worktrees/.trash`，`TRASH_KEEP` 保留策略
  不变，含内容的进行中槽位仍然永不自动删。丢失的只是**一键**恢复，不是恢复的可能性。
- `claim_for_pruning` 现在是唯一的认领者，互斥语义从「撤销 vs 清理」简化为
  「清理 vs 清理」，仍由同一个 `rename` 的原子性保证，`a_victim_can_be_claimed_only_once`
  钉着。
- 验证：`cargo fmt --check` OK、`cargo clippy --all-targets` 0 warning、`cargo test` 全绿
  （`worktree_ops` 20 passed）、`pnpm build`（含 tsc）通过、`pnpm test` 218 passed。
- 全仓库 grep 确认代码里零残留引用（只有历史 decisions 文档提到，属历史记录，不改）。

#### 已知遗留（Task 8 后已消失）：中断的撤销会把槽位卡死在 `Restoring`

review 自己确认**这不是本次回归**——旧方案里同样的中断会把目录永久留在 `.partial`，
症状一模一样。按 `AGENTS.md` §1.2 的最小可审查 diff 原则，当时没把它混进这个修复。

**Task 8 之后这个遗留自动消失**：`Restoring` 状态本身已经不存在了。

| 8 | 按用户要求删掉整条撤销链 | ✅ 完成 | 见下节 | 前后端全套复跑，全绿 |
| 9 | 版本隔离：新槽位改写到 `.trash-v2` | ✅ 完成 | 见下节 | `cargo fmt/clippy/test` 全绿 |

Task 1–7 期间前端零改动；Task 8 才动了 `src/**`（见下节）。

## 决策条目

### Task 1 — 建档

#### 偏离计划
- 无 plan 文件可偏离；本次按 §3「已定位的 bug 修复走 TDD」直接实现。

#### 计划的错误
- 暂无。

#### 计划没写的坑
- 用户报的那次失败**回滚成功**（`worktree/`、`admin/` 都搬回原位、空槽位也删掉了），
  说明挡路的句柄是**毫秒级瞬时**的：t3 落名被挡，t4/t6 回滚和删空槽位都通过。
  只读核查 `readwise-alternative`：`git worktree list` 14 个 worktree 完好、`.trash` 5 个条目
  且无 `.partial`/`.pruning` 残留 —— **没有数据损失**。
- 排除项（只读验证）：Defender 实时防护已关闭；目标名冲突不成立（`.trash` 里没有同名条目）；
  应用自身 watcher 有 300ms 去抖 + `is_path_ignored` 过滤，且 `.worktrees/`、`.claude/` 都在该仓库
  `.gitignore` 里，不是它。合成树（5000 文件）在同一父目录跑 14 轮 0 失败 —— 空转不会自发失败，
  必须有真实进程插一手（该机 130 个 node 进程、~110 个 worktree 各带 watcher、WSearch 索引开着）。

#### 为什么这么改是安全的
- 只读核查，未对任何仓库做写操作；实验一律在临时目录内，探针目录已删除。

### Task 4/5 — 实现

#### 偏离计划
- **`claim_slot` / `claim_for_pruning` / `commit_slot` 的返回类型保持不变**（前两者仍返回
  `PathBuf` / `Option<PathBuf>`，只是返回的路径恒等于入参）。本可以简化成 `bool`，
  但保持签名让**同一个测试能在新旧实现上都编译**，红→绿才是行为差异而不是编译差异。
- `restore_bail` / `restore_bail_with_error` 的 `original` 参数删掉了：目录名不再变化，
  「改回原名」这个概念随之消失，复原动作变成「把 `MARK_RESTORING` 改回 `MARK_COMMITTED`」。

#### 计划的错误
- 无 plan 文件。

#### 计划没写的坑
- **`new_trash_slot` 原本是 `exists()` 再 `create_dir`，中间有 TOCTOU 窗口**，两个并发删除
  能拿到同一个槽位路径。顺手改成用 `create_dir` 的 `AlreadyExists` 避让，占位与判重合一。
- **`commit_slot` 的 `-c{n}` 兜底不再需要**：它存在的理由是「创建时避让的是带后缀的名字，
  去掉后缀后的名字可能被同秒同名的前一次删除占掉」。目录名不再变化之后，`create_dir`
  一次就把最终名占住了，不存在二次避让。
- `rename_retry` **分不清「瞬时句柄」和「目标已被占」**——Windows 上两者都是
  `ERROR_ACCESS_DENIED`。代价：真正的永久失败要多花 1 秒才报错（撞在 `rollback_restored_worktree`
  这类失败路径上）。取舍是有意的：数据搬移的成功率比失败路径的延迟重要，而失败路径罕见。
- clippy 的 `needless_borrows_for_generic_args` 会对 `std::fs::rename(&p.join(..), ..)` 报警，
  `join` 出来的 `PathBuf` 直接传即可。

#### 升级兼容（自查发现的高危问题，已修）
换状态编码方式必然要面对「升级前留下的槽位怎么读」。第一版实现漏了这一条，差点造成**数据丢失**：

- 旧槽位里只有一个 `meta.json`，状态全靠目录名后缀。
- 带 `.partial` 后缀的**半截**槽位（删除崩在搬移中途）里**同样有 `meta.json`**。
- 只看标记文件的话，它会被判成 `Committed` → 参与清理选举 → 时间戳最旧 → 第一个被
  `claim_for_pruning` 认领 → `remove_dir_all` 掉，而里面躺着未提交内容的唯一副本。

修法：`slot_state` 里**后缀的优先级高于标记文件**——带 `.partial` 判 `Deleting`，
带 `.pruning` 判 `Pruning`，与旧版本的处理逐条对齐。旧的**正式**条目（无后缀 + `meta.json`）
天然就该判 `Committed`，可撤销可清理，认领时 `meta.json` 被改名成新标记，自动完成迁移。
钉子：`legacy_suffixed_residue_is_never_treated_as_committed`。

#### 行为变化（review 重点看这两条）
1. **无标记槽位不再被当作正式条目清掉。** 原来「无后缀 + 读不出 meta」的目录被当成最旧的
   victim 优先清；现在它落进 `SlotState::Unknown`，按残留最保守的一档处理（含 `worktree/`
   或 `admin/` 就永不删）。**必须这么改**：新设计里「目录已建、标记还没落盘」是删除流程的
   正常瞬时中间态，沿用旧规则会让并发的清理线程误删一个**正在进行的删除**。
   代价是「meta 丢了但有内容」的异常残留会永久占地方——与既有的「含内容的 `.partial`
   永久保留」是同一个取舍方向（宁可漏清不可误删）。
2. **`claim_slot` 对「目录在、但一个标记都没有」返回 `trash_gone`**（原来会认领成功）。
   这种槽位没有 meta，本来也无从撤销。

#### 为什么这么改是安全的
- **状态机是等价映射**，四个状态一一对应，没有新增或丢失的状态：
  `.partial`(删除中)→`meta.json.partial`、无后缀→`meta.json`、`.partial`(撤销中)→`meta.json.restoring`、
  `.pruning`→`meta.json.pruning`。原设计把「删除中」和「撤销中」挤在同一个后缀里，
  新设计把它们分开了，清理规则反而更精确。
- **认领互斥的正确性没有被削弱**：撤销和清理抢的仍是同一个源（现在是 `meta.json` 这个文件
  而不是槽位目录），仍靠单次 `rename` 的原子性，谁先成功谁拿到，另一方必得 `NotFound`，
  跨进程仍然成立。`a_slot_can_be_claimed_only_once` 钉着这一条。
- **失败方向逐条审过**：`slot_state` 探测出错（权限、盘在抖）滑向 `Unknown`，那是最保守的一档；
  `has_entry` 探测出错按「有内容」处理；`dir_age` 读不出一律不清。错的方向全部是「不删」。
- **数据搬移路径本身没动**：工作树和管理目录的两次搬进、两次搬回、以及回滚，顺序和条件
  一字未改，只是套上了 `rename_retry`。回滚失败时槽位停在撤销中状态、清理器结构性跳过，
  这条不变量由 `failed_worktree_rollback_keeps_incomplete_slot_claimed` 钉着。
- **破坏性操作的范围没有扩大**：唯一的删除动作仍是 `prune_trash` 里对 victim 的
  `remove_dir_all`，且仍然只对**自己认领成功**的槽位执行。
- 测试证据：三个句柄形态用例先红（`os error 5`）后绿；`cargo test` 全绿，其中
  `worktree_ops` 的 24 个集成用例全部建临时 fixture 仓库，未依赖本仓库或任何固定路径。

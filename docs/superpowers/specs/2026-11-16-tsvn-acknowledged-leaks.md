# TortoiseSVN 提交时已确认的 Local-Only 泄漏不再拦自动跟进

状态：已选定方案 A，2026-11-19 开始实现（实现日志见
[`../plans/2026-11-16-tsvn-acknowledged-leaks-decisions.md`](../plans/2026-11-16-tsvn-acknowledged-leaks-decisions.md)）　日期：2026-11-16　仓库：mygitclient（master）

来源：用 TortoiseSVN 提交时被 jkk 的 Check-commit 钩子拦下（「Hook script failed / Local-Only patches」），用户确认要提交，
点了 Proceed anyway。之后 jkk 的跟进又拦了一次。

一句话：Check-commit 每次都往收件箱写一条事件，记下警告过的 hunk；跟进时如果这些 hunk 能与一次成功的 TortoiseSVN 提交对上，
就当作「用户已确认」，不再拦自动跟进，报告里降为非红色分节。对不上的泄漏照旧拦。

## 1. 背景与问题

### 1.1 现象

点 Proceed anyway 提交之后，下次跟进时：

- 自动跟进：泄漏的文件留在旧基线，顶部横幅显示「分支的 Local-Only 补丁可能已经被带上了 SVN 服务器，要你先看一眼。」，
  必须手动点「跟进」。
- 手动跟进：报告里出现红色「泄漏告警」，写着「只是提醒，不影响这次跟进」；跟进照常完成，Local-Only commit 重放后变空、被丢弃。

手动跟进的结果与「不拦的自动跟进」完全相同，只多一个报告弹窗。用户在提交框里已经确认过一次，等于问了两遍。

### 1.2 根因（已读代码确认）

| 位置 | 事实 |
|---|---|
| `src-tauri/src/engine/tsvn_hook_cli.rs` `run_hook` | Check-commit 不写事件（注释：「判定用不上它，只会在收件箱里堆垃圾」）。跟进无从得知用户点过 Proceed anyway |
| `src-tauri/src/engine/svn_follow.rs` `auto_blocker` | `report.leaks` 非空就返回 `svn_auto_follow_leak`，自动跟进不做 |
| `src-tauri/src/engine/svn_follow.rs` `held_candidates` | 部分自动跟进把泄漏文件留在旧基线，原因 `HELD_LEAK` |
| `docs/superpowers/plans/2026-11-01-svn-c1h-tsvn-hooks-decisions.md` | 设计时的代价估计：「误报的代价只是多点一次 Proceed anyway」。漏算了跟进侧的第二次确认 |

自动跟进遇到泄漏就停，本意是防「补丁在用户不知情时上了服务器」：命令行 `svn ci`、另一台机器、钩子未注册或失效、
Check-commit 出错放行。问题在于跟进区分不了「不知情」和「已确认」。

## 2. 目标与非目标

### 2.1 目标

- 用户在 TortoiseSVN 提交框里看过泄漏警告、点 Proceed anyway 提交的 hunk，跟进时不拦自动跟进，也不把文件留在旧基线。
- 这些 hunk 在报告里仍然可见，从红色「泄漏告警」移到非 danger 的分节。
- 不知情的泄漏保持现在的拦截，行为不变。

### 2.2 非目标

- 不改 Check-commit 的判定逻辑和弹框文案。
- 不改手动跟进的流程。
- 本期不处理其他分支上同一补丁的「落后基线」（见 §8 第 1 条）。
- 不从 SVN 服务器撤回已提交的内容。

## 3. 约束与不变量

- 误判方向：只放行「确实看过警告」的 hunk；拿不准一律按现在的拦截处理。
- 钩子 fail-open 不变：写事件失败、算不出泄漏，都照常放行提交；新增的记录不能挡住提交。
- 分工不变：钩子只读 git 对象（libgit2）和磁盘，只写收件箱；不起 git 进程。
- 收件箱向后兼容：`EVENT_VERSION` 保持 1，新字段可选。
- 已实测的 TortoiseSVN 行为（C1h 决策日志 H2、H3）：Check / Pre / Post-commit 拿到的 PATH 都是勾选的文件；Check-commit
  返回 1 时弹框，Abort 回到提交框、Proceed anyway 继续；Pre-commit 失败时不调用 Post-commit。

## 4. 备选方案与取舍

| 方案 | 做法 | 优点 | 否决 / 选定理由 |
|---|---|---|---|
| A | Check-commit 记下警告过的 hunk，与同一批文件的成功提交配对；配上的算已确认 | 只放行确实弹过警告的情况 | 选定。代价是改动跨钩子、收件箱、跟进、报告四处 |
| B | 不新增记录：有 TortoiseSVN 提交事件且注册了 Check-commit，就当已确认 | 改动小，约 2 小时 | 否决。Check-commit 出错时 fail-open，注册了不等于弹过警告，会放过真泄漏 |
| C | 自动跟进不再因泄漏停下，只在报告里提醒 | 最简单 | 否决。不知情的泄漏会被一次自动跟进带过去 |
| D | 横幅加「已知晓，跟进」按钮，不弹报告 | 改动最小 | 否决。仍要手动一次，没解决重复确认 |

## 5. 设计

### 5.1 Check-commit 每次都写一条事件

- `kind = "check_commit"`，`paths` 为勾选的文件（仓库相对路径，与 Pre / Post 相同的转换）。
- 新增可选字段 `warned`：警告过的 hunk 列表。没有泄漏也写，写空列表。
- 算不出泄漏（`local_only_leaks_on_disk` 返回 `Err`）时不写：没有 check 事件就不算确认，跟进照拦。
- 写事件失败不影响退出码：照常按判定结果返回 0 或 1。

为什么没有泄漏也要写：用户 Abort → 用「提交后恢复」删掉补丁 → 再次提交时，第二次 Check-commit 是干净的。只记有泄漏的那次，
配对会错配到第一次的警告上。

```rust
pub struct TsvnEvent {
    // ……现有字段不变
    /// 只有 check_commit 有：Some(空) 表示检查过、没有泄漏
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warned: Option<Vec<WarnedHunk>>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WarnedHunk {
    pub commit: String,  // Local-Only commit 的 OID
    pub path: String,    // 仓库相对路径
    pub old_start: u32,  // 与 HunkLeak 同一坐标（L 的父提交 B 侧）
    pub lines: u32,
}
```

### 5.2 配对规则

一次 TortoiseSVN 提交在收件箱里留下 C（`check_commit`）→ P（`pre_commit`）→ Q（`post_commit`）。现有 `pair_of` 已经配 P↔Q：
路径集合相同、P 不晚于 Q、相隔不超过 `PAIR_WINDOW_MS`（10 分钟），取最晚的 P。

1. 新增 C↔P：路径集合相同、C 不晚于 P、相隔不超过 `PAIR_WINDOW_MS`，取 P 之前最晚的那条 C。
2. 一个泄漏 hunk 算「已确认」，当且仅当：存在成功的 Q（有 revision、没有 error），它配上的 P 又配上了 C，且 C 的 `warned`
   里有一项的 `(commit, path, oldStart, lines)` 与跟进算出的这个 hunk 完全相同。
3. 按 hunk 判定：同一个文件里只要有一个 hunk 没对上，这个 hunk 照旧算泄漏、照旧拦。

| 场景 | 收件箱 | 跟进结果 |
|---|---|---|
| 被警告后点 Proceed anyway 提交 | C(warned=X) → P → Q | X 已确认，不拦 |
| 被警告后 Abort，没再提交 | C(warned=X)，孤儿 | 服务器上没有补丁，无泄漏；C 按孤儿 TTL 清掉 |
| Abort → 提交后恢复删掉补丁 → 提交 | C1(warned=X)、C2(空) → P → Q | 服务器上没有补丁，无泄漏；即使有，配上的是最晚的 C2，不算确认 |
| Abort → 命令行 `svn ci` 提交 | C(warned=X)，没有 P / Q | 泄漏，拦（与现在相同） |
| 关掉「提交前拦截本地补丁」后用 TortoiseSVN 提交 | P → Q，没有 C | 泄漏，拦 |
| Check-commit 算不出（fail-open），窗口内也没有更早的 C | P → Q，没有 C | 泄漏，拦 |
| 同一文件另有一个没被警告的 hunk 也上了服务器 | C(warned=X) → P → Q | X 已确认；另一个 hunk 照旧拦 |
| C 之后、跟进之前 Local-Only commit 被重写（OID 变了） | C(warned=旧 OID) | 对不上，拦（安全方向） |

### 5.3 跟进侧改动

- 读证据：`read_evidence` 额外产出已确认集合 `acknowledged`：`(commit, path, old_start, lines)`。
- 拆分泄漏：`local_only_leaks` 算完后按已确认集合拆开，`report.leaks` 只留未确认的 hunk，新增 `report.acknowledgedLeaks`
  （同 `Leak` 结构）。冲突路径（`ManualNeeded.leaks`）同样拆分。
- 拦截自然生效：`auto_blocker` 和 `held_candidates` 只看 `report.leaks`，已确认的不再拦、不再留在旧基线。
- 其他分支：`plan_others` 本期不拆分。
- 清理：`prune_inbox` 删掉已处理的 Q 时，连同配上的 P 和 C 一起删；孤儿 C 按 `ORPHAN_PRE_TTL_MS`（24 小时）删。
- 前端：`src/engine/svnFollow.ts` 新增分节「提交时已确认的 Local-Only 改动」，level 用 `info` 或 `attention`（见 §8 第 2 条），
  说明这些改动是在 TortoiseSVN 提交时看过警告后提交的。
- 接口：Tauri 命令签名不变；`FollowReport` 多一个字段，前端类型同步。

### 5.4 兼容

- 旧版 jkk 读新事件：`check_commit` 事件没有 revision，`read_evidence` 跳过；`pair_of` 只看 `pre_commit`；多出的 `warned`
  字段被 serde 忽略。旧版只会把它当垃圾按 TTL 清掉。
- 新版读旧收件箱：没有 C，全部按现在的规则拦。
- 跟进 journal（崩溃恢复）：`FollowReport` 的新字段加 `#[serde(default)]`，旧 journal 照常读出。

## 6. 失败、安全与回滚

- 危险方向是错误放行：不知情的泄漏被自动跟进带过去。防线有四道：路径集合相同、10 分钟窗口、hunk 坐标与 commit OID 完全相同、Q 成功。
- 已知残余风险：10 分钟内先被警告并 Abort、再关掉拦截、再用 TortoiseSVN 提交同一批文件，旧的 C 会配上。这种情况下用户确实
  看过同一批 hunk 的警告，接受。
- 同级的残余风险（实现时的独立 review 第 2 条）：被警告并 Abort 之后 10 分钟内再提交同一批文件，这次 Check-commit 算不出或写不进事件，
  P 会配上前一次的 C。用户看过的仍是同一批 hunk（commit OID 与坐标都相同），多出来的 hunk 不在那条 C 的 `warned` 里，照旧拦。接受。
- 不涉及 git 写操作：钩子只多写一个收件箱 JSON；跟进的写入路径不变。
- 回滚：回退代码即可。收件箱里多出的 `check_commit` 事件会被旧版忽略并按 TTL 清掉；journal 新字段旧版不认，崩溃恢复时丢掉的只是这个分节。

## 7. 测试与验收

### 7.1 自动化测试

| 文件 | 覆盖 |
|---|---|
| `src-tauri/tests/tsvn_hook_cli.rs` | 有泄漏时写带 `warned` 的事件且仍退出 1；无泄漏写空 `warned`；算不出时不写事件且放行；写事件失败仍按判定结果返回 |
| `src-tauri/tests/tsvn_inbox.rs` | C↔P 配对（路径集合、窗口、取最晚）；孤儿 C 清理；处理 Q 时连带删 P 和 C；旧格式事件兼容 |
| `src-tauri/tests/svn_follow_hooks.rs` | §5.2 表里每个场景一条；自动跟进对已确认泄漏不拦、不留旧基线；一个确认、一个未确认时只留未确认的文件 |
| `tests/vitest/svnFollow.test.ts`、`FollowReportDialog.test.tsx` | 新分节的 level 与文案；只有已确认泄漏时不出现红色卡片 |

### 7.2 真实窗口 E2E

在 `docs/superpowers/e2e/2026-11-02-svn-c1h-tsvn-hooks-e2e.md` 场景 5 之后加 5d（fixture 工作副本，真 TortoiseSVN）：整文件提交被拦
→ Proceed anyway → 点回应用 → 自动跟进不被拦，报告里是「提交时已确认」，Local-Only commit 被丢弃。顺带实测 §8 第 4 条。

### 7.3 验收标准

- [ ] Proceed anyway 提交后，自动跟进完成，没有「泄漏」横幅，文件不留在旧基线
- [ ] 命令行 `svn ci` 带上同样的补丁，自动跟进照旧被拦
- [ ] 关掉拦截后用 TortoiseSVN 提交，自动跟进照旧被拦
- [ ] Rust：`cargo fmt --check`、`cargo clippy --all-targets`、`cargo test` 通过
- [ ] 前端：`pnpm build`、`pnpm test` 通过
- [ ] 独立 review（AGENTS.md §13）无未修复的高危问题

## 8. 未决问题

1. 其他分支：另一条分支带着同一个 Local-Only commit 时要不要也放行？倾向 OID 相同就放行，OID 不同（各自重写过）照旧。
2. 报告分节的 level：`info` 还是 `attention`？用户已经知道，倾向 `info`，但保留分节作留档。
3. C↔P 的时间窗口：用户盯着弹框多久没有实测数据，先沿用 10 分钟。
4. 待实测：Proceed anyway 之后 Pre-commit 的 PATH 与 Check-commit 完全相同。H2 是在一次正常提交里分别记录的，没覆盖 Proceed anyway 这条路。

实现时的取舍见实现日志的决策条目。

## 9. 实现路线

在独立 worktree 里按 TDD 做；超过 3 个 Task，按 AGENTS.md §2.1 先建 `-decisions.md`。预计半天到一天（含 E2E）。

1. `TsvnEvent` 加 `warned`、`WarnedHunk`；Check-commit 写事件
2. 收件箱 C↔P 配对、已确认集合、清理
3. 跟进拆分 `leaks` / `acknowledgedLeaks`；journal 兼容
4. 前端报告分节
5. E2E 5d、独立 review

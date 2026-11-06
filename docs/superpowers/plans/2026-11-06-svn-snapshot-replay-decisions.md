# 重放时快照内容按基线走（6.10） 实现日志

方案：`docs/superpowers/specs/2026-11-06-svn-snapshot-replay-design.md`。属于 2026-11-05 事故分析剩余待修项的 Task 2，总账见 PR 120 的 `2026-11-06-svn-incident-followups-decisions.md`。两个 PR 各自从 master 切出、互不依赖，所以本 PR 单独记一份。

## 进度总账

| Task | 内容 | 状态 | 验证 |
|---|---|---|---|
| 1 | 失败测试：快照、删除、混真冲突、放弃、Local-Only 与退回旧版本、drift | 完成 | `tests/svn_follow_snapshot.rs` 6 条；变异验证见下 |
| 2 | `svn_snapshot.rs`：按分支基线算命中路径 | 完成 | 同上 |
| 3 | 对象层跟进（当前分支、其他分支）接入；报告 `snapshot` / `drift` | 完成 | 同上；`svn_backend.rs` 序列化形状 |
| 4 | 手动跟进：rebase -i + 预先写好的 todo | 完成 | 同上（含放弃） |
| 5 | 前端报告文案 | 完成 | vitest 3 条；`pnpm test:layout` 140 条 |

## 决策条目

### 偏离方案

- 方案 §5 列了「退回到分支自己基线版本的改动不按本规则换」的端到端测试。改成直接测 `SnapshotRule::paths`：退回旧版本那条 commit 在端到端里的结局（冲突或不冲突）有没有这条规则都一样，测不出区别。
- Local-Only 同上，并入同一条规则级测试。

### 计划没写的坑

- **master 上 `cargo clippy --all-targets` 本来就过不了**：PR 119 新加的 `tests/svn_follow_audit.rs` 没有测试文件惯用的 `#![allow(clippy::unwrap_used, clippy::panic)]`，PR 118 的 lint 护栏随后合入，两者一碰就报错。本 PR 补上这一行（与其他测试文件相同），否则 §5.2 的门槛过不了。
- 手动跟进不能只靠对象层：对象层重放把快照内容换掉之后，冲突只剩真冲突，用户点「手动跟进」，原来的 `rebase <新基线>` 又会原样 pick 快照提交，35 个冲突在磁盘上回来。所以手动跟进必须用同一份规则改 todo。
- `rebase -i` 的 todo 必须只写 OID：`rebase.instructionFormat` 会改说明文字，pick 只认第一个字段。
- 合并结果按「两条规则都命中算 F2 ①」分节：那是我自己的提交，「你已经提交到了 SVN」比「多半是快照」更准。

### 为什么这么改是安全的

- 本规则只往 F2 ① 已有的 `superseded` 集合里加路径，重放引擎、索引拼装、发布流程一行没改。被换掉的是 commit 里与某个较新 SVN 版本**逐字节相同**的内容，换成的是同一路径在 NEW 上的版本：NEW 要么就是那个版本，要么是它之后 SVN 上的演进。丢不掉 SVN 上没有的东西。
- 唯一会丢的情形是用户有意停在某个比分支基线新、比 NEW 旧的 SVN 版本，报告单独列出（`snapshot` 节，提示从原 commit 取回）。原 commit 在 reflog 里。
- 退回到分支自己基线版本的改动不算命中（`read_versions` 去掉与 scope 相同的条目）；Local-Only 不命中。
- 判定出错时这条 commit 不命中，退回原行为（多一个冲突，不会合错）。
- 手动跟进：继续、放弃、冲突出口仍是 `git rebase` 自己的状态机；C' 只是对象，没有 ref 指着，放弃时 `rebase --abort` 回到原分支（测试覆盖）。todo 在写 journal 之前算好、一起落盘，崩在 rebase 之前时继续用同一份。
- drift 只告警不拦：按分析 §6.12，手动点「跟进」时这种情形可以完成，问题是原来不提示。

### 变异验证

| 变异 | 结果 |
|---|---|
| `SnapshotRule::paths` 永远返回空 | 6 条里 4 条红（快照、删除、混真冲突、规则级） |
| 手动跟进忽略 `rebase_todo` | 「混真冲突」红：手动跟进的 rebase 冲突里又出现 B |

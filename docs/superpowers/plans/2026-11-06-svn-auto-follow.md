# 自动跟进（事故分析 6.12；分析文档 §6.12，C2） 实现计划

状态：计划，未实现。前置：PR 121（6.10 快照内容按基线走、对象层 drift 告警）与 PR 123（冲突框上下文）合入；PR 119（手动跟进收尾核对）已合入。

## 1. 目标

TortoiseSVN 一更新完就把 SVN 内容收进基线，「本地改动」里不再出现它们，从源头杜绝 11-05 那种快照提交。只自动执行**对象层跟进**（不写磁盘、不发 SVN 写命令）；会写磁盘的手动跟进永远不自动。

## 2. 触发

| 信号 | 现状 | 要做的 |
|---|---|---|
| 文件变动 / 尾随复查 / 焦点：wc.db 戳变了 → 第二档 → ExternallyAdvanced | PR 120 已有 | 判定写回后，若开关开着且条件满足，调用自动跟进命令 |
| TortoiseSVN `post_update` 钩子写收件箱 `tsvn-inbox` | 钩子已有（C1h），只用作判定证据 | watcher 放行收件箱目录的事件（或轮询它的 mtime），收到 `post_update` 立即跑第二档 |

单飞：自动跟进与横幅上的「跟进」共用一个 `exclusive`（`svnSyncFlow.ts`），正在跑时不重入。

## 3. 13 条条件与现有代码

| # | 条件 | 现有落点 | 缺口 |
|---|---|---|---|
| 1 | 无未收尾 journal，同步锁空闲 | `prepare`：`svn_journal::read`、`svn_lock::assert_not_syncing` | 无 |
| 2 | git / SVN 空闲 | `check_git_idle`、`require_index_free`、判定 `WcLocked` | 无 |
| 3 | HEAD 附着在非基线分支；需要重放时线性 | `attached_branch`、`require_linear` | 无 |
| 4 | 白名单内无 SVN 冲突、无 `~` | 录制时的 `run_preflight_with` | 无 |
| 5 | 「消失」的文件都已由服务器确认删除 | 录制的体检 | 核对体检结果里有无未确认的删除 |
| 6 | 体检 Verified | `FollowReport.verified` | 自动时 NotChecked 不发布 |
| 7 | 当前分支重放无冲突 | `Target::Tip` | 无 |
| 8 | 当前分支无 Local-Only 泄漏 | `FollowReport.leaks` | 自动时非空不发布 |
| 9 | 映射条目没有超限 | `BaseMap` 条目数 | 加上限常量 |
| 10 | git ≥ 2.43 | `check_git_version` | 无 |
| 11 | 重放后保留的 commit 仍改动的路径与本次外部变更无交集 | 无（PR 121 的 `drift_paths` 是它的子集） | 新写：对 `scope..tip` 每个保留 commit 的 `diff-tree` 与 `External::paths()` 求交集 |
| 12 | 拼索引无三方合并、无「作者是我但先后拿不准」 | `ComposeReport.decisions` / `uncertain` / `lost`，在 `publish` 的 PrepareIndex 步骤才算出 | **结构性缺口**：`publish` 先 CAS 移动 `svn/base` 再拼索引。要么把拼索引的判定前移到 `prepare`（只算不写），要么在 PrepareIndex 之后、换入之前回退基线。推荐前者 |
| 13 | 用户没有撤销过针对同一份映射的跟进 | 无 | 撤销功能做了才需要；不做撤销时这条恒真 |

实现形态：新命令 `svn_follow_auto(repo)`：`prepare` → 按上表逐条判 → 全部满足才 `publish`，否则返回 `Skipped { reason }`，前端改挂横幅并写明是哪一条。

## 4. 前端

- 设置里「自动跟进」开关，默认开（用户 2026-10-30 选定）
- 完成后一条不打断操作的提示，可展开 `describeFollowReport`
- 撤销：本期不做（分析文档写明「做不做留给 spec 决定」）；不做则条件 13 恒真

## 5. 测试

- 每条条件一个「不满足就不自动、横幅写明原因」的 fixture 用例
- 全部满足时自动发布、磁盘一个字节不动（`wc_snapshot` 前后比对）
- 与横幅「跟进」、同步、交付并发时不交错

## 6. 风险

- 条件 12 的前移改动 `publish` 的结构，是全计划里唯一动到既有发布流程的地方，单独一个 Task、单独 review
- 钩子路径：收件箱在 `.git` 下，watcher 目前过滤 `.git`，要单独放行，注意不要让 jkk 自己写收件箱触发循环

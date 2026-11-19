# SVN changelist 在更新 / 手动跟进 / 交付之后补挂回去 实现计划

## 背景

jkk 的「更新」「手动跟进」「交付」会暂时把白名单内的文件换回 SVN 原版（stash、checkout 基线、切交付树）。
IDEA / Rider 的 SVN 插件刚好在这段时间刷新时，会把这些「已还原」的文件当成改动消失，执行
`svn changelist --remove`（`SvnChangelistListener.changesRemoved`）。改动回来以后，IDE 把它们放进
默认列表，SVN 侧的 changelist 就丢了。

完整的事故分析在对话里（2026-11-19），结论：jkk 的钩子不碰磁盘，问题出在三个流程改写磁盘的这段时间。

## 目标

三个流程动磁盘之前记下「文件 → changelist」，流程结束（含放弃）之后，把没挂 changelist、仍有修改的
文件按原来的列表名挂回去；约 10 秒后再核对一次，补上 IDE 晚到的 `--remove`。

## 不做

- 不改钩子，不改三个流程本身的 git 步骤。
- 不做前端提示（补挂失败只写日志；见决策日志「遗留」）。

## Task

| # | 内容 | 产出 |
|---|---|---|
| T1 | 从 `wc.db` 只读查出白名单（含其中的目录 external）里所有挂了 changelist 的文件 | `svn_wcdb::changelists` + `tests/svn_wcdb.rs` |
| T2 | 记录文件、补挂、延迟核对 | `svn_changelist.rs`、`svn_xml::parse_changelist_status` + `tests/svn_changelist.rs` |
| T3 | 接入三个流程（记录）与命令层（流程结束后补挂 + 10 秒后核对） | `svn_sync_in.rs`、`svn_follow.rs`、`svn_sync_out.rs`、`commands/svn_sync.rs` |
| T4 | 端到端：更新冲突暂停期间模拟 IDE 摘掉 changelist；交付时 Local-Only 文件；手动跟进 | `tests/svn_changelist.rs` |
| T5 | 全量验证、独立 review、PR | — |

## 关键设计

- **记录文件独立于 journal**：`.git/mygitclient/changelists.json`（review 后从 `.git/` 根挪进来），带 opid。journal 在 S11 / 各种收尾点被删
  （`restore_step_with` 里有五处），延迟核对发生在 journal 删掉之后，记录必须活得比 journal 长。
- **记录走 wc.db**：`svn status` 遍历白名单在公司机器上约 4 秒；`ACTUAL_NODE.changelist` 一条查询。
  认不出 wc.db（格式不是 31、WORK_QUEUE 非空、打不开）时退回 `svn status --xml`。
- **补挂条件**：现在没有 changelist、且仍有修改（文本或属性）。已经在别的列表里的不动；已经不再修改的
  不挂（交付提交掉的文件，svn commit 本来就会摘掉 changelist）。
- **补挂的时机**：命令层 `with_watch_suspended` 在流程函数返回之后调用 `svn_changelist::settle`。
  journal 还在（停在冲突上）时不补挂：那时文件可能仍是原版，挂上也会被 IDE 再摘掉。
- **失败不影响流程结局**：记录或补挂失败只写日志；补挂失败时记录留着，10 秒后的核对再试一次，
  之后无论成败都删记录，不无限重试。
- **并发**：（原计划：补挂与核对都先拿同步锁。review 第 1 条指出这会挡住自动跟进与 git 写操作，改为进程内
  互斥 + 只探测同步锁，见决策日志 T5。）有流程在跑就跳过，记录留着，下一次流程开始记录前先补挂旧记录。

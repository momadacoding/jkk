# 跟进：补上 F2 ① 的快照空窗，换成 SVN 版本前先核对

来源：2026-11-26 使用报告「jkk 0.10.3 跟进时，TortoiseSVN 里已处理的冲突还要再解一遍」（飞书），
第 5 节改法 ① + ②。③（冲突框支持多个提交改过同一文件）延后，④（冲突框里磁盘 = SVN 新版本）不做。

## 背景

F2 ①（`svn_follow.rs` 的 `superseded`）：失焦快照之前的本地提交里，你在快照之后用 TortoiseSVN
提交过的文件，重放时整个换成 SVN 版本。两个缺口：

1. **快照空窗。** 跟进收尾时清掉快照（`svn_follow.rs` `finish`、`svn_sync_in.rs` `finish_forward`），
   之后只有失焦且判定为一致时才再写。跟进完马上去 TortoiseSVN update / 提交，失焦那一刻判定不是
   一致，快照一直没有，F2 ① 静默失效，已经提交到 SVN 的文件掉进冲突框。
2. **换之前不核对。** 「部分提交」事故：TortoiseSVN 只提交了文件的一部分，F2 ① 把整个文件换成
   SVN 版本，没提交的那部分从 work 分支上没了。

## Task 1：跟进收尾时写一份只含 commit 的快照

- `svn_follow_mark` 加 `write_commit_mark`：与 `write_mark` 相同，但 `staged` 记空。
- `finish`（自动跟进与按钮跟进）、`finish_forward`（手动跟进落到基线上）把 `clear_mark` 换成它。
- 测试：跟进完成后不失焦、直接用 TortoiseSVN 提交中间版本所在的文件，下一次跟进 F2 ① 生效。

## Task 2：换成 SVN 版本前逐个「提交 × 文件」核对

- `svn_follow_disk` 加 `supersede_safe(repo, commit, parent, new_base, paths)`，与冲突框磁盘核对共用
  `merge-file` 两段式标记与 `outside_hunks_match`。
- 对提交 C 改过的文件 p：以 `C^:p` 为合并基础、`NEW:p` 为 ours、`C:p` 为 theirs 合一次。
  - 干净：结果必须等于 `NEW:p`。
  - 有冲突：`NEW:p` 在冲突块以外必须与合并结果逐行一致。
  - 不通过就不换，C 照常重放（干净就保留改动，冲突就进冲突框）。
- `svn_follow.rs` 的 `superseded` 按提交调用它，结果缓存。
- 测试：只提交了一部分（干净合并、冲突合并两种）不换；中间版本（W10-R2 形态）照旧换。

## Task 3：现有测试与文档

- 改断言「收尾清掉快照」的现有用例。
- 跑全量验证，独立 review，开 Draft PR。

# 手动跟进放回改动的假冲突（2026-11-19 使用报告问题 4、5）实现日志

没有单独的计划文件：输入是 2026-11-19 的使用报告（问题 4 `map_resource.lua`、问题 5
`SkillDetailInfoBaseUI.lua`）和对话里的 review 结论。按 AGENTS.md §2.1 落这份日志。

## 进度总账

| Task | 状态 | 产出 | 验证 |
|---|---|---|---|
| 1. fixture 复现问题 4、5 | 完成 | `src-tauri/tests/svn_follow.rs` 新增 5 个用例 | 修复前 3 个复现用例全部失败（`StashConflict`，问题 4 报在新路径 `Code/Sub/Old.cs`） |
| 2. 放回冲突后按跟进前的磁盘解决 | 完成 | `svn_sync_stash.rs` `settle_untouched_conflicts` 等；`svn_sync_in.rs` 接入 | `svn_follow` 142 个用例通过 |
| 3. 问题 4 的提示文字 | 完成 | `src/engine/conflictView.ts`、`tests/vitest/conflictView.test.ts` | `pnpm build`、`pnpm test` 通过 |
| 4. 全量验证 + 独立 review | 完成 | review 第 1–8 条已处理 | `cargo fmt --check`、`clippy`、全部测试二进制分批通过 |

## 决策条目

### Task 1

- **为什么用真 SVN fixture**：问题 4 的形状（SVN 移动文件 → git 视为「删除 + 未跟踪」→ stash 分两部分收）
  只有真的 `svn mv` + `svn update` 才搭得出来，纯 git 仓库搭的是另一种形状（AGENTS.md §13 第 9 条）。
- **计划没写的坑**：问题 4 里如果用户还改过挪过去的新文件，未跟踪部分与 HEAD 不再是同一个 blob，会先被
  A3 拦成 `StashBlocked`，走不到这里。这是已有行为，本次不改。

### Task 2

- **偏离 review 建议：不加「SVN pristine == 新基线」这条判定。** review 时担心分支落后、磁盘是旧基线内容
  加本地编辑（stale disk）时，取跟进前的磁盘会把 SVN 更新冲掉。复查后不加，原因：
  1. 这种情况 pristine 也等于新基线（svn update 早就做过，是 git checkout 把旧内容写回了磁盘），
     pristine 判定分不出来，加了也白加。独立 review 同意。
  2. 收尾核对 `svn_follow_audit` 已经把「有未提交改动的文件，跟进后应等于跟进前的磁盘」定为正确结果，
     合出别的内容会报「分支与磁盘对不上」。按跟进前的磁盘解决与核对口径一致。
  3. 结果就是用户点「跟进」之前磁盘上的内容，TortoiseSVN 看到的本地修改与跟进前相同，什么都不丢。
- **不改基准**：`git stash apply` 不能指定基准；改用新基线当基准还会让问题 3（本地提交改过的文件）静默
  合掉，原本该由人判断的冲突消失。
- **判定条件**：冲突文件在新 HEAD 与新基线里相同（`diff-tree --no-renames <新基线> HEAD` 不列出它）。
  它不完全等于「本地提交没碰过」：本地提交改过、用户在 rebase 冲突里解成 SVN 版本的文件也满足，放回后
  那笔提交的改动变成未提交改动。不丢东西，也是「同一改动既提交进 git 又提交进 SVN」这种常见情况的
  正确结果（独立 review 第 5 条，注释已改成按实际条件描述）。
- **删除只在 stash 明确记下删除时才做**：旧 HEAD 里有、stash 两部分都没有。原先「两部分都没有就删」在
  Windows 上只差大小写的改名会删掉用户的文件（独立 review 第 1 条）。
- **一个文件出错只留它在冲突里**，不让整步报错（独立 review 第 2 条）。磁盘上是目录时不删。
- **回滚时不自动解决**：回滚没有收尾核对，`checkout` 静默没写进去（Unity 占用文件，本文件已记过的
  行为）时没人发现，而后面紧跟着 drop（独立 review 第 3 条）。
- **全部解决之后**：把 apply 留在暂存区的东西退回新 HEAD（新增文件除外，与 git 干净放回一致），再走
  干净放回的路（核对、drop）。跟进前真有暂存时 `stage_lost`，备份留着，走 `StageLost` 出口。

### 为什么这么改是安全的

- 写下的内容只有两种来源：stash 的已跟踪树、stash 的未跟踪树；删除只发生在 stash 记下了删除时。都是跟进前
  用户磁盘上的样子，经过与任何 stash apply 相同的换行 / 过滤器转换（不保证与带混合换行的原始字节逐字相同，
  这一点和原来的 stash apply 一样）。
- **收尾核对对这些文件是循环的**：它们从 stash 取出、再拿去和 stash 比，核对不提供独立保证（独立 review
  第 6 条）。安全性来自上面这条：来源只有 stash，认不准时不动。
- `journal.stash_applied` 在自动解决**之前**落盘：崩在中途重开时走「已 apply」分支，要么报还有未解决的
  冲突，要么按 `Kept` 收尾、备份留在 stash 列表里，不会 drop。
- 所有带路径的命令都设 `GIT_LITERAL_PATHSPECS=1`；`unstage_applied` 没有路径时不调用（空 pathspec 是全部）。

### Task 3

- **只能改说法，不能识别改名**：冲突索引里只有这一个路径的几个阶段（问题 4 的 stage 1 记在新路径上），
  分不出删除与改名。标题改成「另一边没有它」，正文提示可能是改名、删除前先确认。

## 遗留

- 未覆盖的分支：符号链接 / 子模块（`Entry::Other`）、文件名带通配符的路径。Windows fixture 建符号链接要权限，
  通配符路径的效果在这里观察不到（写回的都是跟进前的内容），只靠 `GIT_LITERAL_PATHSPECS` 保证。
- 自动解决了哪些文件没有单独告诉用户：结果与跟进前一致，和合得上的文件一样不报。

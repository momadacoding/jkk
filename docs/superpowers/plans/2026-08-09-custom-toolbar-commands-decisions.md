# 工具栏自定义命令 实现决策日志

计划：`docs/superpowers/plans/2026-08-09-custom-toolbar-commands.md`
规格：`docs/superpowers/specs/2026-08-09-custom-toolbar-commands-design.md`
worktree：`.claude/worktrees/custom-toolbar-commands`，分支 `worktree-custom-toolbar-commands`，基点 `a0e1bb3`

## 进度总账

| Task | 状态 | 产出文件 | 验证结果 |
|---|---|---|---|
| 0 准备 worktree | 完成 | 本文件 | 切入确认：`pwd` 在 worktree 内，基点 `a0e1bb3` |
| 1 配置解析 | 未开始 | | |
| 2 launcher | 未开始 | | |
| 3 Tauri 命令层 | 未开始 | | |
| 4 前端纯逻辑 | 未开始 | | |
| 5 前端接线与 UI | 未开始 | | |
| 6 真机验证与文档 | 未开始 | | |

## 决策条目

### Task 0：准备 worktree

**计划的错误**

计划 Step 4 要求检查 `.claude/worktrees/` 是否在 `.gitignore` 中，并称需要补上——
这条来自 AGENTS.md §4.1 的「该路径目前**不在** `.gitignore` 中」。**实际已经在了**，
`.gitignore:2` 就是 `.claude/worktrees/`（`.worktrees/` 在第 1 行）。
Step 4 是 no-op，没有改动 `.gitignore`。

AGENTS.md 里那句话已过时。本次不顺手改 AGENTS.md——那是规范文件，
改它超出本 PR 范围，留作后续单独处理。

**计划没写的坑**

`git check-ignore .claude/worktrees` **在 worktree 内部跑会给出误导性的答案**。

`.gitignore` 里的模式带尾部斜杠（`.claude/worktrees/`），只匹配目录。
而当前 session 的工作目录是 `.claude/worktrees/custom-toolbar-commands`，
在它内部并不存在 `.claude/worktrees` 这个路径，git 无法确认它是目录，
带斜杠的模式便匹配不上，`check-ignore` 返回「未忽略」。

回到主工作区跑同一条命令才得到正确结果：

```
.gitignore:2:.claude/worktrees/	.claude/worktrees
```

结论：**验证 `.gitignore` 规则必须在该路径真实存在的工作区里做**，
不能在一个恰好把该路径当作自身父目录的 worktree 里做。

**偏离计划**

无。

**为什么这么改是安全的**

Task 0 除新增本文件外没有任何改动，不涉及 git 写操作，无风险。

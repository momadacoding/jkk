# 外部 diff / merge 工具 —— 实现日志

> 计划：`docs/superpowers/plans/2026-10-18-external-diff-merge-tools.md`
> 设计：`docs/superpowers/specs/2026-10-17-external-diff-merge-tools-design.md`
> 分支：`worktree-external-tools`（worktree `.claude/worktrees/external-tools`），基点 `dd8911c`
> 执行方式：主会话直接施工（`superpowers:executing-plans`），不用 subagent（用户 2026-10-18 指定）

## 进度总账

| Task | 状态 | 产出文件 | 验证结果 |
|---|---|---|---|
| 0 建 worktree 与决策日志 | ✅ 完成 | 本文件 | 纯文档，无代码验证；`git status` 只有本文件 |
| 1 外部工具配置与规则匹配 | 未开始 | | |
| 2 外部进程启动、锁与临时目录 | 未开始 | | |
| 3 按字节导出与外部 diff 命令 | 未开始 | | |
| 4 前端外部 diff 入口与刷新代次 | 未开始 | | |
| 5 冲突信息读取与五种情况分类 | 未开始 | | |
| 6 冲突文件的 HEAD 对比工作区 diff | 未开始 | | |
| 7 外部 merge 命令与各侧标题 | 未开始 | | |
| 8 前端冲突提示与 merge 入口 | 未开始 | | |
| 9 DU / UD 二选一 | 未开始 | | |
| 10 merge driver 检查（后端） | 未开始 | | |
| 11 横幅与合并入口前的确认 | 未开始 | | |
| 12 用户文档、E2E 清单与最终验证 | 未开始 | | |

## 决策条目

### Task 0

#### 偏离计划

- 无。

#### 计划的错误

- 无。

#### 计划没写的坑

- `EnterWorktree` 建出来的分支，在没有任何新提交时，`git branch --merged master` 也会把它列出来（分支指针和 master 指向同一个提交）。这不代表「已合并的旧分支」，这个分支是本任务新开的。
- `superpowers:executing-plans` 自带的 scratch 台账放在 `<仓库根>/.superpowers/sdd/`，但这个目录不在 `.gitignore` 里，建了会变成未跟踪文件。

#### 为什么这么改是安全的

- Ruling：不建 `.superpowers/sdd/` 下的 scratch 台账，进度总账和决策条目都只记在本文件里。理由是 AGENTS.md §2.1 规定，主会话直接施工时这两部分都要落在 `docs/` 下，本文件已经同时承担两种职责。代价（如果这个决定错了）：skill 的 `task-start` / `task-done` 脚本用不了，需要手工维护上面的总账表。

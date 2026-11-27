# 混合仓库文件历史与 Blame：实现日志

计划：`2026-11-27-svn-file-history.md`　设计：`../specs/2026-11-27-svn-file-history-design.md`

## 进度总账

| Task | 状态 | 产出 | 验证 |
|---|---|---|---|
| 0 实现日志 | 完成 | 本文件 | — |
| 1 带超时的 svn 查询 | 未开始 | | |
| 2 路径 → URL 共用 | 未开始 | | |
| 3 基线标注 | 未开始 | | |
| 4 分组纯函数 | 未开始 | | |
| 5 `svn_file_history` | 未开始 | | |
| 6 `svn_rev_file_diff` | 未开始 | | |
| 7 `svn_blame_overlay` | 未开始 | | |
| 8 前端 store 与纯函数 | 未开始 | | |
| 9 FileHistoryView | 未开始 | | |
| 10 真实窗口验证与收尾 | 未开始 | | |

## 决策

### Task 0

- **偏离计划（实为偏离 spec §7）：** spec 要求超时时结束 svn 进程「含子进程」。本实现只结束 svn 主进程。结束进程树要给 `windows` crate 加 `Win32_System_JobObjects` feature，计划的全局约束不加 feature。svn+ssh 下的 ssh 子进程预计在管道关闭后自行退出，**未测**，列为风险。

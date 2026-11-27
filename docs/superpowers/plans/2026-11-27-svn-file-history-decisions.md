# 混合仓库文件历史与 Blame：实现日志

计划：`2026-11-27-svn-file-history.md`　设计：`../specs/2026-11-27-svn-file-history-design.md`

## 进度总账

| Task | 状态 | 产出 | 验证 |
|---|---|---|---|
| 0 实现日志 | 完成 | 本文件 | — |
| 1 带超时的 svn 查询 | 完成 | `svn_cli.rs`：`run_svn_deadline`、`classify_svn_error`、`SVN_QUERY_TIMEOUT` | `cargo test --lib deadline_tests` 4/4；fmt、clippy 干净 |
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

### Task 1

- **计划没写的坑（svn 1.14.5 实测 stderr）：**
  - 拒绝连接的端口：`E170013: Unable to connect to a repository at URL …` 接着 `E730061: Can't connect to host …`
  - `file://` 仓库里不存在的路径：`E160013: File not found: revision 0, path '/nope'`
  - 不回应的服务器：svn 一直等，没有自己的超时，只能靠 `run_svn_deadline` 结束
- **为什么这样分类是安全的：** 认证失败时 svn 也会先报一行 E170013，所以先查认证码（E170001 / E215004，**未实测**），再查路径码，最后才查连接码。归错类只影响提示的那句话，不影响数据。
- **偏离计划：** 多加了一个测试 `success_returns_stdout`，确认正常路径能拿到 stdout。超时后不等读管道的线程结束（孙进程可能还攥着管道）。

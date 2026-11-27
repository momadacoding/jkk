# 混合仓库文件历史与 Blame：实现日志

计划：`2026-11-27-svn-file-history.md`　设计：`../specs/2026-11-27-svn-file-history-design.md`

## 进度总账

| Task | 状态 | 产出 | 验证 |
|---|---|---|---|
| 0 实现日志 | 完成 | 本文件 | — |
| 1 带超时的 svn 查询 | 完成 | `svn_cli.rs`：`run_svn_deadline`、`classify_svn_error`、`SVN_QUERY_TIMEOUT` | `cargo test --lib deadline_tests` 4/4；fmt、clippy 干净 |
| 2 路径 → URL 共用 | 完成 | `svn_wc.rs`：`unit_of`、`server_url`、`server_url_at`、`strip_prefix_ci`、`units_cached` | `url_tests` 2/2；svn_health 五个集成测试二进制 105/105 |
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

### Task 2

- **偏离计划：** `strip_prefix_ci` 也一起挪进 `svn_wc`（`pub(crate)`）。`svn_health` 里还有别处用它，留两份会分叉。
- **偏离计划：** `units_cached` 的缓存键多了一项「基线分支 tip」。更新 / 跟进之后可能多出新的 external，只按白名单做键，新 external 下的文件要等重启才换算得出 URL。缓存只留最近一份。
- **为什么是安全的：** `unit_of` / `server_url_at` 原样搬家，`svn_health` 的行为不变，回归用它的五个集成测试二进制（105 个用例）全部通过。

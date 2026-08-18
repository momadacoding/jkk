# fix/async-trash-prune 决策日志

> 配套 PR #29。本文件按 §2.1 记录进度与决策；文件建于第二轮修复开工时
> （第一轮单修复当时判定为单轮小修未建，此为先建后补的偏离）。

## 进度总账

| Task | 状态 | 产出 | 验证 |
|---|---|---|---|
| 1 诊断删除慢根因 | 完成 | 探针逐步计时 | 用户真实仓库实测：rename 26ms / 同步 prune 129s（10 万文件）；合成 2 万文件 6.4s |
| 2 prune 后台化 + 启动清理 + 文案 | 完成 | worktree.rs / session.rs / DeleteBranchDialog.tsx / worktree_ops.rs | 新回归先红（14.4s）后绿；cargo fmt/clippy/test、pnpm build/test 全绿；commit c17ed3e |
| 3 prunable worktree 删除（io 267） | 完成 | worktree.rs / worktree_ops.rs | 两条失败测试转绿；cargo fmt/clippy/test 全绿 |
| 4 review 修复：prune/delete 竞态（blocking） | 完成 | worktree.rs | cargo fmt/clippy/test 全绿 |

## Task 3 补充决策（实现落地时）

- **计划没写的坑**：review 指出除 `admin_dir` 外还有两处必须跟着目录是否存在走——
  脏检查 `is_dirty` 同样在缺失目录里起 git 进程（且「目录已丢」时没有可保护的
  用户数据，本就不该查）；`restore_worktree` 原先无条件搬 `slot/worktree`，对
  只含管理目录的槽位必须改成按 `slot/worktree` 是否存在走条件搬移。
- **计划没写的坑**：管理目录反查靠扫 `<common>/worktrees/*/gitdir` 文件（内容=
  工作树 `.git` 文件路径，父目录即工作树）。目录已丢后 `same_path` 的
  canonicalize 兜底失效，只剩归一化字符串比较；前端路径来自 `git worktree
  list`，与 gitdir 内容同出一族，正常写法都能匹配，局限已写进函数注释。
- **为什么这么改是安全的**：主工作区否决在两条路径上都成立——目录在时按
  `absolute-git-dir == git-common-dir` 判；目录不在时主工作区不在
  `.git/worktrees/` 登记里，扫不到只会以 `worktree_not_registered` 被拒，
  方向仍是拦。

## Task 4 决策（来自独立 review 的 blocking 发现）

- **问题**：后台 prune 与进行中的 delete 存在竞态。`new_trash_slot` 建槽位即出现在
  `read_dir` 里，但 `meta.json` 原先在搬移完成后才写；空窗内并发 prune 读不出
  元数据，把该槽位当 `ts=0`（最旧）优先清，`remove_dir_all` 会删掉正在搬入的
  用户工作树——回收站要保护的数据恰好在这一条路上丢。
- **方案**：`meta.json` 在任何搬移之前落盘。ts 变为当前时刻后，该槽位按最旧优先
  的排序不再是 victim；残留空窗缩到一次小文件写。剩余的同秒 tie 需要同一秒内
  6 次删除且 prune 恰好命中微秒级空窗，UI 路径（每次删除都要过确认对话框和多次
  git 调用）达不到这个速率，作为已记录的残余风险接受。
- **为什么这么改是安全的**：meta 内容（worktree/admin 路径、时间戳）在搬移前
  已全部可知，提前写不改变内容；写失败时槽位仍为空，直接报错当作没删过，比
  原先「搬完发现写不出再回滚」更早失败、更少状态要收拾。附带收益：管理目录
  搬移失败且工作树回滚也失败时，残留槽位现在带 meta，仍是可撤销的回收站条目。
- **review 附带处理**：`src-tauri/Cargo.toml` 的行尾空 churn 已 `git checkout`
  还原；测试注释「撤销对…的撤销」语病已修。
- **验证**：`cargo fmt --check`、`cargo clippy --all-targets`、`cargo test -j 2`
  全绿（含两条「目录已丢」新测试由红转绿）。首轮 `cargo test` 默认并发撞
  Windows 页面文件上限（os error 1455，环境前置问题，§5.3），降并发后通过。

## Task 1 决策

- **计划没写的坑**：删除慢不在删除本身。`delete_worktree` 的搬移是 rename（毫秒级），
  慢的是 `prune_trash` 同步 `remove_dir_all` 最旧槽位——用户的每次删除在替之前攒下的
  回收站买单。Fork 没有回收站设计，所以没有这笔账。
- **为什么这么改是安全的**：诊断全程只读用户仓库；唯一一次真实删除由用户在对话中
  针对 `worktree-blog-kindle-obsidian` 与 `readwise-alternative` 仓库明确批准，且走的是
  应用自身的回收站搬移（可撤销）。

## Task 2 决策

- **偏离计划**：无事前计划文件；按 §3 已定位 bug 直接 TDD。
- **为什么这么改是安全的**：后台清理与删除主路径无共享可变状态；并发清理选中同一
  victim 时慢者 `remove_dir_all` 静默失败；强退残留有界（≤6 槽）且下次启动/删除自收。
  不采用并行删除/第三方 crate：无成熟并行删除库，手写会重造「解链接不跟进 junction」
  不变量，而清理速度在后台化后已不在用户路径上。
- **计划没写的坑**：`DeleteBranchDialog` 的进度文案把同步清理的耗时当固有成本写进了
  界面（「要等一两分钟」），修后端必须同步修文案，否则留下死注释与误导文案。

## Task 3 决策

- **计划没写的坑**：目录已丢的 worktree（prunable）删分支时，`delete_worktree` 第一步
  `admin_dir` 在不存在目录里起 git 进程，Windows 上直接 io error 267
  （`The directory name is invalid`）。实测 git 真实行为：prunable 登记仍会拒绝
  `branch -D`；`git worktree remove --force` 对缺失目录返回 0 并清登记。
- **为什么这么改是安全的**：目录已丢时没有可丢的用户数据（未提交内容随目录已失），
  仍把管理目录搬进回收站使撤销可回到「原样 prunable」状态；管理目录不靠目录名拼
  （同名后缀会错），扫 `<common>/worktrees/*/gitdir` 文件按 `same_path` 匹配，
  与现有「问 git/问文件系统，不拼名字」的安全风格一致。
- **偏离计划**：此修复是 PR 测试中暴露的独立 failure path，与 Task 2 同属
  `delete_worktree`，随同一 PR 第二个 commit 提交，保持分支内聚。

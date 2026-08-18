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
| 5 第二轮 review：三个 P1 数据安全问题 | 完成 | worktree.rs / worktree_ops.rs | cargo fmt/clippy/test 全绿（lib 18/18、worktree_ops 23/23） |
| 6 第三轮 review：三个 P1（TTL 误删 / restore 竞态 / 按目录大小写敏感） | 完成 | worktree.rs | cargo fmt/clippy/test 全绿（lib 21/21、worktree_ops 23/23） |
| 7a 第四轮 review：restore 先认领再读取 meta | 完成 | worktree.rs | 回归先因缺少 `begin_restore` 编译失败，后定向测试 1/1 通过 |
| 7b 第四轮 review：回滚失败保留 partial | 完成 | worktree.rs | 回归先因缺少安全状态参数编译失败，后定向测试 1/1 通过 |
| 7c 第四轮 review：同时间戳 victim 确定性排序 | 完成 | worktree.rs | 回归先因缺少 `sort_slots` 编译失败，后定向测试 1/1 通过 |

## Task 7 决策（第四轮 review，完成）

- **已核实的根因**：restore 的原子认领发生在读取 meta 和目标占位检查之后，路径在
  这段窗口里可能经历「旧槽位被清理 → 同名槽位重建」的 ABA；admin 搬回失败时又
  忽略 worktree 回滚结果，无条件把 claimed 槽位改回正式名；prune 只按秒级时间戳
  排序，相同时间戳依赖未定义的 `read_dir` 顺序，并发清理可能认领不同 victim。
- **实施约束**：保持现有原子 rename 状态机，不新增进程内锁或依赖；三个问题分别
  先补失败测试，再做最小修改，每项完成后立即更新本表。
- **7a 为什么安全**：restore 先把正式槽位原子改名为 `.partial`，再从该已认领目录
  读取 meta 并检查目标路径。之后 prune 无法再选中它；读取或占位探测失败时改回
  原名，并保留原错误码。这样路径名不会在「读 meta → 认领」之间被复用成另一条目。
- **7b 为什么安全**：admin 恢复失败后显式记录 worktree 反向 rename 是否成功；仅在
  成功恢复原始槽位结构时解除认领。反向 rename 失败则保留 `.partial`，错误信息同时
  给出仍在原位置的 worktree 和 partial 槽位，避免 prune 把不完整状态当正式条目删除。
- **7c 为什么安全**：victim 由 `(deleted_at, 完整路径)` 全序排列；同一回收站的并发
  清理者即使拿到不同 `read_dir` 顺序，也会按相同顺序竞争同一组槽位。原子 rename
  仍是最终认领门槛，因此每个应清条目最多被一个清理者拿到，不会因平局各删一组。
- **偏离计划**：无实现方向偏离；三个问题均按「失败测试 → 最小生产修改 → 定向转绿」
  顺序完成，未新增依赖或进程内锁。
- **计划的错误**：本轮没有独立计划文件，review 三条即任务输入；未发现 review 给出的
  复现条件或安全断言有误。
- **计划没写的坑**：7b 最初的测试只把 `false` 状态传给收尾函数，能证明收尾安全，
  但不能证明真实反向 rename 会生成该状态。完整 diff 自查时改为创建已占用的
  `slot/worktree`，让同一个生产辅助函数真实执行并失败，避免测试形态产生虚假信心。
- **验证**：三个新增定向回归均先编译失败后转绿；`cargo fmt --check`、
  `cargo clippy --all-targets`、`cargo test -j 2` 全绿（lib 24/24；其余 fixture 集成
  测试全部通过，命令退出码 0）。

## Task 6 决策（第三轮独立 review，三条 P1 逐条核实后采纳）

- **三条均核实属实**，未驳回：
  1. `.partial` 按 TTL 无条件清：崩溃在搬移中途 / 回滚失败时里面是未提交内容的
     唯一副本，24h 后自动删等于定时销毁恢复现场；
  2. restore 操作正式槽位期间始终是 prune 的合法 victim，两次 rename 之间被清
     会留下无登记的坏 worktree（同步 prune 时代两条命令并发即可触发，后台化
     只是放大）；
  3. `cfg(windows)` ≠ 文件系统不区分大小写：Windows 支持按目录开大小写敏感，
     那种目录里 Foo/foo 两条登记共存，折叠会任意命中。我上一轮新加的 Windows
     测试还把折叠固定成了预期行为，一并纠正。
- **P1-1 方案：自动清理只碰「能证明为空」的残留**。`.partial` 需同时满足
  年龄 ≥24h 且无 `worktree/`、无 `admin/` 子项才被回收；探测出错按「有内容」
  处理（宁可漏清）。含内容的 `.partial` 永久保留——崩溃残留有界（每次崩溃
  最多一个），占盘换不丢数据。
- **P1-2 方案：原子 rename 当认领锁，不用互斥锁**。restore 开始前把槽位改名
  加 `.partial`（prune 结构性跳过）；prune 删 victim 前先改名加 `.pruning`。
  谁先改名成功谁拿到，另一方 rename 必 NotFound——竞态从结构上不存在，
  跨进程成立，且不会有「restore 被分钟级清理卡住」的锁等待。restore 失败时
  把槽位改回原名（`restore_bail`），前端持有的 trash_path 可原样重试；
  改不回就在错误消息里给出新路径（带后缀不会被清理碰）。
- **P1-3 方案：登记反查全平台精确匹配，彻底去掉折叠**。Windows 只归一分隔符
  （反斜杠不可能是文件名字符），非 Windows 连分隔符都不动（反斜杠是合法
  文件名字符）。曾考虑「折叠但要求全表唯一命中」，但登记损坏 + 双大小写名 +
  目录已丢的组合下唯一折叠命中仍可能是别人——精确匹配零误命中，匹配不上
  按 `worktree_not_registered` 拒删（安全方向）。多重精确命中 = 登记损坏，
  以 `worktree_ambiguous` 拒绝。`same_path` 保持不动（服务锁/主工作区否决，
  折叠方向 over-refusal 安全）。
- **计划没写的坑（测试形态）**：Windows 默认卷造不出两个只差大小写的兄弟
  目录（`create_dir_all` 落进同一个目录），双大小写反查用例只能
  `cfg(not(windows))`；Windows 上钉的是「大小写对不上必须找不到」这条
  不折叠保证。第一轮写反的测试（把折叠当预期）已拆除。
- **Limitation**：`registry_lookup_distinguishes_case_variants` 与
  `dir_probe_errors_instead_of_assuming_missing` 为 Unix-only，本机未执行
  （仓库无 PR CI，首跑要等 Unix 环境）。
- **Follow-up（同类未修）**：`lock_reason` 在非 Windows 双大小写登记下可能
  查到错的块（同源于 `same_path` 折叠）。需目录存在才会触发 lock 语义、且
  要双大小写登记并存，比本轮三个面窄得多；超出本轮 review 范围，记录在案。
- **验证**：`cargo fmt --check`、`cargo clippy --all-targets`、`cargo test -j 2`
  全绿（lib 21/21 含新增认领互斥/残留判定/登记精确匹配用例，worktree_ops 23/23）。

## Task 5 决策（第二轮独立 review，三条 P1 逐条核实后采纳）

- **三条均核实属实**，未驳回任何一条：
  1. meta-first 不够——`std::fs::write` 非原子（create+truncate+write），且 `read_dir`
     到写完之间仍有空窗，半截/缺失 JSON 会被 prune 当 ts=0 清掉；
  2. `Path::exists()` 把 I/O 错误折叠成 false（Rust 文档明确），断连网络盘/未挂载/
     无权限会被当成「目录已丢」，跳过脏检查只搬登记，真实工作树变无主数据；
  3. `same_path` 无条件 `to_lowercase`，目录已丢时 canonicalize 兜底失效，大小写
     敏感平台上 `/wt/Foo` 与 `/wt/foo` 两条登记会按 read_dir 顺序撞错，搬错管理
     目录会弄坏另一个还活着的 worktree。
- **P1-1 方案：结构性区分 committed/in-progress，不靠时序余量**。槽位创建即带
  `.partial` 后缀（后缀而非前缀：worktree 名是用户输入，可能以前缀开头，撞不上
  我们追加在时间戳后的后缀）；meta + 两次搬移全部完成后 `commit_slot` 同卷原子
  rename 摘后缀，清理线程只见到两种完整状态之一。`prune_trash` 对 `.partial`
  一律跳过且不计入条数；超过 24h 的 `.partial` 视为崩溃残留回收（正常进行中窗口
  是毫秒级，24h 同时兜住双实例开同一仓库的场景）；年龄读不出宁可漏清。
- **P1-1 附带设计点**：`commit_slot` 去后缀后的名字可能与同秒同名的前一次删除
  撞上，兜底序号用 `-c{n}` 与创建时的 `-{n}` 区分；落名失败时两道工序都搬回原位
  报错。meta 提前写保留为第二层防护（摘后缀那一刻内容必须完整）。
- **P1-2 方案：`dir_exists` 严格探测**。`metadata` 的 NotFound 才算目录已丢，其他
  错误一律以 `dir_probe` 拒绝。三处应用：`delete_worktree` 的 worktree 探测、
  `restore_worktree` 的 `has_worktree`、restore 的两处占位检查（折叠成 false 会丢
  meta 走人，残留槽位变成无元数据正式条目被清掉）。
- **P1-3 方案：`registry_path_matches` 平台分治**。非 Windows 只做精确匹配
  （外加尾部斜杠归一）；Windows 保持折叠并回退 `same_path` 的 junction/短名识别。
  `same_path` 本身不动：它的折叠服务锁/主工作区否决，误判方向是 over-refusal
  （安全）；登记反查误判方向是搬错目录（不安全），两条路径的安全方向相反。
- **Limitation（如实记录）**：两个 Unix-only 行为在本机（Windows）无法执行——
  `dir_probe_errors_instead_of_assuming_missing`（Windows 无不提权的便携造错法）
  与 `registry_path_matches` 的 `cfg(not(windows))` 精确分支。代码已写、本机未跑；
  Windows 无 CI，这两个分支首次真实执行要等 Unix 环境。root 下权限用例会假红。
- **Limitation（已知取舍）**：macOS 大小写不敏感卷上，大小写拼写不同的同一目录
  在登记反查中匹配不上，按「找不到登记」拒删——over-refusal 是安全方向，且
  gitdir 与前端路径同出 git 一族写法，正常情形拼写一致。

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

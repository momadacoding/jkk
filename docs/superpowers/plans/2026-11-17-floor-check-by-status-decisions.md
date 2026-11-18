# 体检的 floor 比对改用本地状态：实现日志

来源：[2026-11-16-follow-report-usability.md](2026-11-16-follow-report-usability.md) §7、§8.4。没有单独的计划文件，
本日志的 Task 划分即计划。

## 问题

跟进的体检对每个白名单单元跑 `svn diff --old=<url>@F --new=<单元> --summarize`（floor 组），2026-11-16 公司机器上
7.4–15.8 s，是跟进里最大的一块。§7 的实测：svn.exe 自身 CPU 14.1 s、其中内核 12.9 s，网络往返 0.15 s——慢在本地
逐个文件，不在服务器。同样 23 个目录的 `svn status` 1.4 s。

§7 原来的方向是「拆回每个目录一条」（实测快 36–49%）。这次换一个更大的方向：floor 组里能由工作副本自己回答的单元，
不再问服务器。

## 为什么本地能回答（论证，T1 实测支撑）

`svn diff --old=URL@F --new=WC` 是 repos→wc 比对：工作副本把每个节点的 BASE revision 报给服务器，服务器只发
「从这些 BASE 到 F」的变化，再叠上本地修改。所以：

1. 一个文件节点 `n`（BASE revision）、`c`（最后修改 revision）满足 `c ≤ F ≤ n` 时，它在 F 上的内容就是 pristine，
   服务器不会为它发任何变化。它在 diff 里出现与否，只取决于本地有没有改——这正是 `svn status` 的 `modified`。
2. 目录同理：`c ≤ F ≤ n` 时，F 上它的子项清单与 BASE 相同（子项增删会让目录的 committed-rev 前进）。
3. 判「本地有没有改」，diff 与 status 用的是同一套（时间戳 + 大小没变就不读内容）：T1 里内容改了、时间与大小
   不变的文件，两条命令都不报；真改了的，两条都报。

不满足上面形状的（混合 revision 的目录、稀疏检出、switch、not-present、本地 add/delete/copy、冲突、file external、
正在进行的操作），这个单元照旧跑 diff。

## 进度总账

| Task | 状态 | 产出 | 验证 |
|---|---|---|---|
| T1 在临时 SVN 仓库里实测 status 与 floor diff 的结论 | 完成 | 见「T1」 | scratch 里的 fixture，手工命令 |
| T2 wc.db 判形状 | 完成 | `svn_wcdb::floor_shapes` / `FloorShape` | `tests/svn_health_local_floor.rs` 里每个不规整场景断言了原因 |
| T3 体检接入：形状规整的单元用 status 结果，其余照旧 diff；预跑同步改 | 完成 | `svn_health` 的 `LocalFloor`、`local_floor_commands`、`read_local_floor`、`plan_diffs_for_except`；`svn_cli::run_svn_fresh`；`fault` 可同时 arm 多个点 | 同上 |
| T4 等价测试：同一工作副本走两条路，体检结论逐项相同 | 完成 | `tests/svn_health_local_floor.rs` 28 条 | 28/28 |
| T5 进程预算：规整的跟进里不再有 floor `svn diff` | 完成 | `tests/svn_follow_scan_budget.rs` 改写 diff 那段断言 | 1/1 |
| T6 review、PR | 进行中 | | `cargo fmt --check`、`clippy --all-targets` 无告警；33 个 SVN 测试二进制全过（见 T6） |

## 决策条目

### T1 实测（2026-11-17，SlikSvn 1.14.2，file:// 仓库）

- 内容改了、时间与大小不变（`touch -r` 还原 mtime）：`svn status` 不报，`svn diff --old=URL@2 --new=.` 也不报。
  diff 慢不是因为它更严。
- 真改了内容：两者都报（status `M`，diff `modified`）。
- 文件、目录被删（missing）：diff 不报，status 报 `missing`——本地路径要把 `missing` 当成「不在 D 里」。
- 只改属性：diff 报 `item="none"`（体检本来就剔掉），status 报 `item="normal" props="modified"`。
- 未版本化文件：diff 不报，status 报 `unversioned`，忽略。

### T2 wc.db 判形状

- 判据只读 `NODES` / `ACTUAL_NODE` / `WC_LOCK` / `WORK_QUEUE`，与 `svn_wcdb::excluded_nodes` 同一套「认不出来就不读」：
  格式不是 31、WORK_QUEUE 非空、根查不到，一律 `Err`，整次体检照旧 diff。
- 不规整的条件，任一条成立这个单元就照旧 diff：工作副本被锁；子树里有 `op_depth > 0` 的行（本地 add / delete / copy /
  move）；`conflict_data` 非空（冲突）；BASE 行的 presence 不是 `normal` / `excluded`（not-present 等）；file external；
  kind 不是 file / dir；`repos_path` 与「单元根 + 相对路径」不一致（switch）；目录 depth 不是 infinity（稀疏）；
  目录不满足 `c ≤ F ≤ n`。
- 文件不满足 `c ≤ F ≤ n` 不直接判不规整，记进 `stale`：映射单列的文件、不是 W-file 的文件，diff 报不报它们都不影响结论
  （前者归自己的 revision 组、后者被 W-file 筛掉）。只有剩下的 W-file 才让单元改回 diff。用户提交过文件、没 update 的
  常见形态因此仍能走本地（`a_w_file_committed_without_update_is_left_to_its_own_group`）。
- `excluded` 的行跳过：整单元 diff 本来就不报被排除的节点，体检另外点名比，那部分照旧。
- **计划没写的坑**：判形状用的是 BASE 层（`op_depth = 0`），`excluded_nodes` 用的是最上层（`TOP`）。两者不冲突：有
  `op_depth > 0` 的行时整个单元已经改回 diff 了。

### T3 体检接入

- **为什么这么改是安全的**：
  - 结论层面：本地 floor 只产生 floor 组的 `modified` 差异，之后的 W-file 筛选、`D \ S`、墓碑、点名比对全部照旧，
    代码路径与 diff 的结果汇合在同一个 `diffs` 里。规整的单元里 diff 不可能报 `added` / `deleted`（那需要 WORKING 层或
    F 上的子项与 BASE 不同，都已判成不规整），所以墓碑候选不受影响。
  - 失败语义：原来 floor diff 是体检第一次连服务器，断网 = `NotChecked`，录制放行；比对开始之后失败 = `Unconfirmed`。
    本地 floor 先对每个仓库根做一次 `svn info --depth empty <仓库根>@F`，用同一个注入点 `FirstDiff`，再读 status。
    这样断网仍是 `NotChecked`，不会因为本地答得出来就报 `Verified`；之后的失败照旧算「比对开始之后」。探测仓库根，
    不探测单元的 URL：单元在 F 上可能还不存在，原来的 diff 那时报 added 而不是失败（M6）。
  - 时刻：status 读磁盘，与 diff 一样必须在 `git add` 之后。体检里走 `run_svn_fresh`（不读扫描缓存：缓存里的 status
    是准备阶段、`git add` 之前读的），预跑在 `git add` 刚结束时起跑，与原来 diff 的起跑点相同。
    `svn_follow_scan_budget.rs` 钉住了这条 status 排在 `git add` 之后、写树之前。
  - 任何一步读不了（wc.db、W-file 筛选、status 命令失败、status 里出现意外的条目）都只是让单元改回 diff，记日志，
    不改变结论。本地 floor 是提速，不是判据。
- **计划没写的坑**：第一版用 `parse_node_status` 解析 status，它是给 `-v` 的输出写的，空结果（干净的工作副本）当错误。
  错误被「读不了就改回 diff」吞掉，结论照样对，只是永远走不到本地。等价测试因为同时断言了「走没走本地」才抓到，
  改用 `parse_status_entries`。所以每条等价测试都必须断言走了哪条路。
- 放回 diff 的单元没有提前预跑的命令时（预测的映射与实际不同、status 里出现意外条目），体检自己跑，只是慢。
- 加了性能打点：`query` 记录 `wc.db floor shape`（查询耗时）与 `health floor answered locally`（`local` / `server`
  两个数）。不规整的原因写进 `log::info`。公司机器上跟进一次，就能从性能日志读出这次有几个单元走了本地。

### T4 等价测试

- 每条都在同一个工作副本上跑两遍体检：一遍注入 `FloorShapeRefused`（全部照旧 diff），一遍放开；断言结论逐项相同
  （失败原因的措辞除外），并断言放开那遍走没走本地。不规整的场景另外断言 `FloorShape::Irregular` 的原因，免得「没走
  本地」其实是因为别的地方读不了。
- **计划没写的坑**：测试里给 svn 命令拼 `--non-interactive` 时放到了 `--` 后面，`svn update --set-depth=empty -- Code/Deep@
  --non-interactive` 把它当成路径、报「Skipped」、退出码 0，稀疏检出的场景就空跑了。改为紧跟子命令。原因断言加上之后
  这类空跑会变红。
- `tests/svn_health.rs` 里合并 floor diff 那一节（合并、退回逐单元、服务器不可达不退回）用的工作副本是规整的，本地
  floor 之后一条 floor diff 都不发，有一条变红、两条空过。`check_roots` 改为注入 `FloorShapeRefused`。为此
  `fault::arm` 改成可以同时 arm 几个点（原来一次只能一个）。

### 偏离计划

- §7 写的是「拆成每个目录一条，改之前在服务器慢的时段再测」。改为本地回答：拆分最多省到 8–10 s，本地回答把 floor 组
  从服务器比对变成一次 `svn status` + 一次 wc.db 查询。§7 担心的「服务器慢时连接开销」在新做法里不存在（规整的单元
  只连一次服务器，见 T3 的可达性探测）。
- 没有在 Client 仓库复测耗时：仓库在公司机器上，这台开发机没有。改为加性能打点（每个单元走了哪条路、为什么），
  在公司机器上跟进一次即可读到。

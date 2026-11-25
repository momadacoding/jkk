# 跟进提速 PR2：跟进命令带回发布后的判定

来源：2026-11-24 对 `--perf` 记录的分析，方案 4。PR1（PR 198）已合并。

## 目标

跟进后的刷新（`follow.refresh`，慢时段中位 9.4 秒）里，关键路径是 `svn_sync_status`：一遍全量 `svn status`
加判定。发布前的锁内复核（R3c）刚读过同一份节点，发布后用它就能算出判定，不必再扫一遍。

- `svn_follow` / `svn_follow_auto` 在发布成功后带回 `sync`：新基线下的 `SyncStatus`，和算它时读到的 HEAD。
- 前端拿到 `sync` 就直接写回判定、记下 HEAD，不再调 `svn_sync_status`。自动跟进的完成提示因此早挂出。
- 验收：`follow.refresh` 里不再有 `svn_sync_status`。

## 不变量

- 判定与紧接着跑一次 `check_sync` 逐项相同：同一个判定函数，只把「读节点」换成 R3c 读到的那份。
- R3c 不删、不挪：节点仍在锁内、发布前读；不新增 `svn status`（`svn_follow_scan_budget.rs` 的遍数不变）。
- 前端记录的「上次观测值」不晚于判定的输入（`recordAfterSuccess` 的规矩）：
  - wc.db 戳**不记**：留着跟进前那次检测记的戳。它早于 R3c，之后 wc.db 有任何变化，下一次比戳都会重跑。
  - HEAD 记后端在判定之前读到的那个；有别的 HEAD 在排队时不记。
- 算不出（没录制的分支跟进入口、读出错）就不带，前端照原来刷新。

## Tasks

### Task 1：后端拆出「按给定节点判定」

- `svn_sync::check_sync_with_nodes` 的判定部分拆成 `pub(crate) fn judge_nodes(repo, &[NodeStatus]) -> SyncStatus`；
  `check_sync` 读节点后调它。
- 验收：现有 `svn_sync*` 测试全过。

### Task 2：跟进带回判定

- `still_valid` 交出 R3c 读到的节点。
- `follow` / `follow_auto` 在发布（含放回 Local-Only 补丁）之后、放锁之前：读 HEAD（`repo::open`），再 `judge_nodes`。
- 引擎新增 `follow_with_sync` / `follow_auto_with_sync` 返回 `(结局, Option<SyncAfterFollow>)`；原来的
  `follow` / `follow_auto` 只丢掉第二项，七十多处测试不动。
- 命令层返回 `Followed<T> { #[serde(flatten)] outcome, sync }`：JSON 仍是 `{ kind, report, ... }`，多一个 `sync`。
- 测试（`tests/svn_follow_carry_sync.rs`）：整次跟进、部分跟进、点「跟进」冲突（基线发布、分支落后）三种，
  断言带回的 `SyncStatus` 与紧接着的 `check_sync` 逐项相同、HEAD 与 `repo::open` 相同；不跟进、
  被拦下时不带；序列化里有 `kind` 和 `sync`。

### Task 3：前端用带回的判定

- `repoStore` 新增 `applyFollowedSync(path, sync)`：领 `svnSync` 闸的号（顶掉更早开跑的检测）、写判定、
  按上面的规矩记 HEAD。
- `createAfterSync` 允许跳过复查；`svnSyncFlow` 的 `afterFollow` 拿到 `sync` 且写回成功时跳过 `refreshSvnSync`。
- 测试：`svnSyncFlow.test.ts`（带回时不调 `refreshSvnSync`、没带或写不回时照旧调），`repoStore` 侧的
  HEAD 记录规矩按纯函数抽出来测。

### Task 4：门槛、review、PR

- 两侧门槛；签名改了，要跑 `pnpm tauri dev` 实测一次跟进。
- 独立 review。

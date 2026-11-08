# SVN 扫描性能 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把公司机器上 jkk 的后台 SVN 扫描和「跟进」耗时降下来；本计划先落地第一优先级（后台扫描），其余按优先级排队。

**Architecture:** 第一档检测只比 wc.db 戳（纯 `stat`，后端已有枚举缓存），rootRevs 退出前端热路径；四个检测入口（打开仓库、文件变动、焦点、HEAD 变化）共用一个按仓库路径单飞的队列，排队时合并「是否强制跑第二档」。

**Tech Stack:** React 18 + TypeScript（`src/engine`、`src/store`）、Vitest；Rust 侧只改注释。

**Spec:** 本文 §1 的问题清单（来源：2026-11-07 在公司机器上对 jkk 0.7.3 便携版的 ETW 录制报告，报告本身不入库，关键数字摘在 §1）。

## 1. 问题清单

环境：Windows 11、24 核；Client 仓库 git 跟踪 17,602 个文件，白名单 23 个目录，`.svn\wc.db` 117 MB；D 盘挂着深信服（file_filter / load_driver / sfaioguard）、火绒 sysdiag、Defender WdFilter 四层文件过滤驱动。

| # | 问题 | 实测 | 根因 | 状态 |
|---|---|---|---|---|
| Q1 | 电脑每十几秒卡一下 | ccstatusline 每 10 s 跑一次 `git diff --shortstat`，单次 0.35–0.9 s 墙钟、3.5–6 s CPU，内核态 93–99% | 过滤驱动让每次 `stat` 约 0.2 ms（正常十几 µs） | 已缓解（不在本仓库：TTL 改 60 s、删 git-changes widget） |
| Q2 | 文件一动 jkk 就在后台扫 | 每次 `repo-changed` 跑两轮第一档（立即 + 尾随复查），约 138 个 svn 进程、约 16 s | `read_root_revs`（`svn_sync.rs:412`，注释写「O(1)」）每轮都重新 `enumerate_wcs`：23×`info --xml` + 23×`status --xml` + 23×`info --show-item`，不用 `wc_db_stamps` 已有的枚举缓存 | **本计划 Task 1** |
| Q3 | 检查入口叠跑，最多 8 路并发 | 第一次录制 10 分钟 2842 个进程、约 1035 s 进程时间 | 只有 `checkSvnSyncIfChanged` 单飞；焦点、HEAD 变化、打开仓库都不走；`refreshGate` 只丢弃旧结果，不阻止旧请求跑完 | **本计划 Task 2** |
| Q4 | 点「跟进」约 200 s | 979 个子进程（svn 521、git 458）；git 对象层不到 5 s | 逐目录扫描 × 23、逐文件起进程 × 192 个脏文件（R2b `rewrite_to_server`，`svn_init.rs:721`，单这步约 65 s）、逐目录问服务器（`info -R` 22.9 s、`diff --summarize` 33.3 s） | 排队：P2、P4 |
| Q5 | 跟进发布后又起 472 个进程 | — | jkk 自己写 ref / index 触发的 `repo-changed` 刷新 | P1 落地后复测，见 P3 |
| Q6 | 外部 SVN 更新后几分钟才出「跟进」 | 未录到 | 假设 H1 检测链路慢；H2 「最后一次生效」被持续触发拖住；H3 最后一轮落在更新未结束时（撞锁失败保留旧结论），之后没有新触发，要等焦点或 5 分钟服务器核对 | 待验证，见 P5 |
| Q7 | jkk 进程内 libgit2 的 status 开销 | ETW 只统计子进程，没量到 | 17,602 个文件 × 0.2 ms 的 stat 也落在 jkk 主进程上 | 待测量，见 P6 |

## 2. 修复优先级

| 优先级 | 内容 | 预期收益 | 前置 / 风险 |
|---|---|---|---|
| P0（不改代码） | 火绒信任区加 `D:\Program\trunk`；深信服排除找 IT | 所有起进程的开销整体下降，最大杠杆 | 深信服可能不可控 |
| **P1（本计划）** | A'：第一档只比 wc.db 戳；C：检测入口共用带「强制」标记的单飞队列 | 文件变动时 svn 进程 138 → 0（戳没变时）；任何时刻最多一轮在跑 + 一轮排队 | 见 Review Focus |
| P2 | 跟进 R2b：`ls-files -s` 一次读全、`update-index` 合成一次 `--index-info`（含删除）、`svn cat` + `hash-object --path` 有界并行（4–8 路） | 65 s 那一步预计降到十几秒；约 500 个进程省下约 290 个 | `hash-object` 必须带 `--path`（`svn_baseline.rs` `replace_blob` 注释）；`update-index` 必须串行 |
| P3 | 跟进成功后复用它得出的结论，不再为自己引起的 `repo-changed` 跑第二档 | 发布后的 472 个进程 | 先在 P1 之后复测还剩多少 |
| P4 | B：`enumerate_wcs` 一次传全部目录；F：`info -R` / `ls -R` / `diff --summarize` 一次传多目标，语义不成立时退到按目录并行。另：`wc_db_stamps` 现在任一 wc.db 变了就整套重新枚举（约 46 个进程），能否只在 external 集合可能变化时才重新枚举，需单独论证（P1 独立 review 第 5 条） | `info` 1251 → 82 ms、`status` 8.1 → 4.1 s；跟进里约 90 s 的服务器调用 | 初始化、更新、跟进、交付共用，影响面大；每条先在 fixture 上实测多目标语义 |
| P5 | Q6 验证：录一次外部更新到按钮出现，对照 H1 / H3（P1 落地后 H2 在结构上不再成立）。需要的话给 release 构建开文件日志 | 定位剩余延迟 | release 开日志要定日志位置与保留策略 |
| P6 | 量 jkk 主进程自身 CPU（libgit2 status） | 确认 Q7 是否显著 | — |
| 降级 | G：`obj/`、`bin/` 加进忽略——P2 合并删除后只剩界面去噪，由用户决定；H：按 BASE revision 复用旧 blob——`svn update` 会把整棵树的 BASE revision 升到新值，跟进场景几乎命中不了，要改用 pristine checksum 才有意义，需单独设计 | — | — |

## Global Constraints

- 不新增依赖（npm / crate）。
- 不改 Tauri 命令名与参数；`svn_root_revs` 保留（Rust 测试在用、PR 129 在并行改 `commands/`），前端不再调用，删除另起清理。
- 第二档成功写回之后才记观测值（`recordAfterSuccess` 的不变量），被顶替（返回 `false`）或抛错都不记。
- 记的观测值必须是**开跑之前**读到的。
- `svnChecksRunning` 覆盖从读戳开始到结论写回为止（失焦快照靠它）。
- 前端验证：`pnpm build`、`pnpm test`；Rust 只改注释也跑 `cargo fmt --check`、`cargo clippy --all-targets`、`cargo test`。

## Review Focus

1. 外部更新进行中文件事件、焦点事件连续到达：同一仓库任一时刻最多一轮检测在跑、一轮排队，不出现并发的 `svn_sync_status`。
2. 一轮「比戳」检测在跑时 HEAD 动了：排队的那一轮必须强制跑第二档，否则切到落后分支没有「跟进」（§6.11.3 的老问题）。
3. 打开仓库：这次就记下 wc.db 戳，之后第一次存盘不再白跑第二档；打开时读戳失败仍要跑第二档（只是不记）。
4. 提交前的 `freshSvnSync` 撞上一轮在跑的检测：要等一轮在它请求**之后**开跑的检测写回，不能拿在跑那轮（可能早于 TortoiseSVN 更新完）的结论就走。
5. 第二档被顶替或失败：戳与 HEAD 都不记，下一次事件重试。

---

### Task 1: 第一档只比 wc.db 戳

**Files:**
- Modify: `src/engine/svnSync.ts`（`recordAfterSuccess` 泛型化；删 `syncIfRootRevOrWcDbChanged`；新增 `runSvnSyncCheck`）
- Modify: `src/store/repoStore.ts`（删 `lastRootRevs`、`readRootRevs`、`serializeRootRevs`、`svnRootRevs`；三个入口改走 `runSvnSyncCheck`）
- Modify: `src/components/RepoTab.tsx`（`repo-changed` 监听器里「O(1) 第一档」的注释）
- Modify: `src-tauri/src/engine/svn_sync.rs`、`src-tauri/src/commands/svn.rs`（`read_root_revs` / `svn_root_revs` 的「O(1)」注释改成实际成本，注明前端不再调用）
- Test: `tests/vitest/svnSync.test.ts`、`tests/vitest/repoStoreFollowFocus.test.ts`、`tests/vitest/repoStoreBaselineMissing.test.ts`、`tests/vitest/repoStoreSvnSamePath.test.ts`

**Interfaces:**
- Produces:
  - `recordAfterSuccess<T>(memo: { value: T | null }, observed: T, runFullCheck: FullCheck): Promise<boolean>`（行为不变，只放宽类型）
  - `type SvnCheckOutcome = 'skipped' | 'superseded' | 'applied'`
  - `runSvnSyncCheck(memo: WcDbStampMemo, force: boolean, readStamps: () => Promise<WcDbStamp[]>, runFullCheck: FullCheck): Promise<SvnCheckOutcome>`
    - 非强制：读戳失败抛错；戳没变返回 `'skipped'`。
    - 强制：读戳失败也跑第二档，只是不记。
    - 第二档返回 `false` → `'superseded'`，不记；否则记戳（读到了的话）→ `'applied'`；第二档抛错原样抛出，不记。

- [x] **Step 1: 写失败测试（`tests/vitest/svnSync.test.ts`，`describe('runSvnSyncCheck')`）**
  - `'非强制、戳没变：不跑第二档，返回 skipped'`
  - `'非强制、戳变了：跑第二档，写回后记下开跑前读到的戳'`
  - `'强制、戳没变：照样跑第二档'`
  - `'强制、读戳失败：照样跑第二档，记录不动'`
  - `'非强制、读戳失败：抛错，不跑第二档'`
  - `'第二档被顶替（返回 false）：返回 superseded，记录不动'`
  - `'第二档抛错：一并抛出，记录不动'`
  - 原 `syncIfRootRevOrWcDbChanged` 的测试删掉。
- [x] **Step 2: 跑 `pnpm test tests/vitest/svnSync.test.ts`，确认因 `runSvnSyncCheck` 不存在而失败**
- [x] **Step 3: 在 `svnSync.ts` 实现 `runSvnSyncCheck`，泛型化 `recordAfterSuccess`，删 `syncIfRootRevOrWcDbChanged`**
- [x] **Step 4: store 测试改成新形状（先红）**
  - `repoStoreFollowFocus.test.ts` 加 `'文件事件、wc.db 没变：不调 svn_root_revs，也不跑第二档'`：`repo-changed` 之后 `count('svn_root_revs') === 0`、`count('svn_sync_status') === 0`。
  - 加 `'打开仓库时记下戳：之后第一次文件事件 wc.db 没变就不跑第二档'`。
  - `repoStoreBaselineMissing.test.ts`、`repoStoreSvnSamePath.test.ts` 里按顺序应答 `svn_root_revs` 的地方改成应答 `svn_wc_db_stamps`。
- [x] **Step 5: `repoStore.ts` 改接线**：`refreshSvnSyncReporting` 读戳后 `recordAfterSuccess(lastWcDbStamps.current, …)`；`checkSvnSyncIfChanged` 与 `checkSvnSyncOnFocus` 都走 `runSvnSyncCheck(…, false, …)`；删 rootRevs 相关代码与注释。
- [x] **Step 6: Rust 注释改成实际成本**
- [x] **Step 7: 验证**：`pnpm build`、`pnpm test`；`src-tauri/` 下 `cargo fmt --check`、`cargo clippy --all-targets`、`cargo test`。

### Task 2: 检测入口共用带「强制」标记的单飞队列

**Files:**
- Create: `src/engine/svnCheckQueue.ts`
- Modify: `src/store/repoStore.ts`（四个入口改走队列）
- Test: `tests/vitest/svnCheckQueue.test.ts`、`tests/vitest/repoStoreFollowFocus.test.ts`

**Interfaces:**
- Consumes: Task 1 的 `runSvnSyncCheck`、`SvnCheckOutcome`；`createSingleFlight`（`src/engine/singleFlight.ts`）。
- Produces:
  - `createSvnCheckQueue(run: (path: string, force: boolean) => Promise<SvnCheckOutcome>): { request(path: string, force: boolean): Promise<SvnCheckOutcome> }`
    - 同一路径任一时刻只有一轮 `run` 在跑；在跑时来的请求合并成一轮补跑，所有合并进来的调用方拿到补跑那轮的结果。
    - 补跑那轮的 `force` = 合并进来的请求里任一个是 `true`。`force` 在一轮**开跑时**取走并清掉，开跑之后再来的强制请求进下一轮。
    - 不同路径互不影响。
  - store 入口：
    - `refreshSvnSyncReporting(path)` = `request(path, true)`，返回 `outcome === 'applied'`
    - `checkSvnSyncIfChanged(path)` / `checkSvnSyncOnFocus(path)` = `request(path, false)`，返回 `outcome !== 'skipped'`
    - `checkSvnSyncIfHeadMoved(path, head)`：`head === lastCheckedHead.value` 时直接返回 `false`；否则 `request(path, true)`，`'applied'` 时才记 `head`

- [x] **Step 1: 写失败测试（`tests/vitest/svnCheckQueue.test.ts`）**
  - `'空闲时请求立即开跑'`
  - `'在跑时连来三次：只补跑一轮，三个调用方拿到同一个结果'`
  - `'在跑一轮非强制时来了强制请求：补跑那轮是强制'`
  - `'强制标记开跑时取走：开跑之后再来的非强制请求补跑时不强制'`
  - `'这一轮抛错：补跑照样开跑，错误只交给这一轮的调用方'`
  - `'不同路径各跑各的'`
- [x] **Step 2: 跑 `pnpm test tests/vitest/svnCheckQueue.test.ts`，确认失败**
- [x] **Step 3: 实现 `createSvnCheckQueue`**：内部 `createSingleFlight()` + `Map<string, boolean>` 记待取的强制标记，同 `repoStore` 里 `statusWanted` 的做法。
- [x] **Step 4: store 测试（先红）**（`repoStoreFollowFocus.test.ts`）
  - `'焦点检测与文件事件同时来：第二档不并发'`：`svn_sync_status` 挂起时再发焦点与 `repo-changed`，在跑的 `svn_sync_status` 始终 ≤ 1。
  - `'一轮比戳的检测在跑时 HEAD 动了：补跑那轮强制跑第二档'`。
  - `'提交前的 freshSvnSync 撞上在跑的检测：等补跑那轮写回'`。
- [x] **Step 5: `repoStore.ts` 接队列**，`trackSvnCheck` 包在传给队列的 `run` 里；删掉 `checkSvnSyncIfChanged` 上原来的 `refreshFlights.run`。
- [x] **Step 6: 验证**：`pnpm build`、`pnpm test`。
- [ ] **Step 7: 独立 review（§4.1 门槛），修完高危后提交、推送、开 Draft PR**

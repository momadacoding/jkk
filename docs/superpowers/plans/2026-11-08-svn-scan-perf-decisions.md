# SVN 扫描性能：实现日志

计划：[2026-11-08-svn-scan-perf.md](2026-11-08-svn-scan-perf.md)

## 进度总账

| Task | 内容 | 状态 | 产出文件 | 验证 |
|---|---|---|---|---|
| 1 | 第一档只比 wc.db 戳 | 完成 | 改 `src/engine/svnSync.ts`（`runSvnSyncCheck`、`recordAfterSuccess` 泛型化，删 `syncIfRootRevOrWcDbChanged` / `syncIfWcDbChanged`）、`src/store/repoStore.ts`（删 rootRevs 一整套）、`src-tauri/src/commands/svn.rs`、`src-tauri/src/engine/svn_sync.rs`（只改注释）；测试 `svnSync.test.ts`、`repoStoreFollowFocus.test.ts`、`repoStoreBaselineMissing.test.ts`、`repoStoreSvnSamePath.test.ts` | 引擎测试先红（9 条，函数不存在）后绿；store 测试先红后绿；变异「打开仓库不记戳」被 `打开仓库（refreshSvnSync）记下戳` 抓到；`pnpm build`、`pnpm test` 通过；`cargo fmt --check`、`cargo clippy --all-targets` 通过，`cargo test` 见 Task 2 |
| 2 | 检测入口共用带「强制」标记的单飞队列 | 完成 | 新增 `src/engine/svnCheckQueue.ts`、`tests/vitest/svnCheckQueue.test.ts`；改 `src/store/repoStore.ts`、`repoStoreFollowFocus.test.ts` | 队列测试先红（模块不存在）后绿；store 4 条新测试在旧代码上全红、改后全绿；变异「补跑不合并强制标记」被队列测试与 store 测试各抓到一次；`pnpm build` 通过，`pnpm test` 79 个文件 1293 条通过 |

## 决策条目

### 计划阶段

#### 偏离报告的方案

- 报告的方案 A 是「第一档复用枚举缓存，23 个目录合成一次 `svn info --show-item revision`」。计划改成 A'：第一档只比 wc.db 戳，rootRevs 退出前端热路径。理由：revision 存在 wc.db 里，`svn update` 改 revision 必然写 wc.db，戳的信号覆盖 rootRev 的信号（反过来不成立：只提交不更新时 rootRev 不动、wc.db 变）。A' 在戳没变时一个 svn 进程都不起，也不用改 `--show-item` 多目标输出的解析。
- 读戳的后备：原来戳读失败时还能按 rootRevs 判断。`wc_db_stamps_with` 只有 `read_config` 失败才报错，枚举失败时退回已知位置或白名单所在的 wc.db；`read_config` 失败时 rootRevs 也读不出来（同一个入口），所以这个后备没有实际覆盖面，去掉不损失什么。

#### 计划没写的坑

- 实现日志 `2026-11-06-svn-incident-followups-decisions.md` 记过「两样观测值并行读……总成本与顺序读相同」。不对：rootRevs 这一侧不走枚举缓存，每次都重新枚举，戳那一侧缓存失效时又枚举一遍，外部更新期间一轮就是两次枚举。3.2 节「枚举 41 轮 vs `show-item` 22 轮」就是这个差。
- 同一份日志记过「打开仓库时不记戳」的次要原因是「会打乱 3 个按 invoke 顺序逐个应答的 store 测试」。A' 去掉 rootRevs 之后那几个测试本来就要改，这个原因不再成立。

#### 为什么这么改是安全的

- 只改前端检测的**触发条件与调度**，第二档（`svn_sync_status`）本身、跟进、录基线都不动；不发任何 git / svn 写命令。
- 「记观测值」的不变量原样保留：开跑前读、写回后才记、被顶替或失败不记（`recordAfterSuccess` 只放宽类型，不改行为）。
- 合并请求时强制标记取「或」：最坏是多跑一轮第二档（本地读），不会少跑。

### Task 1：第一档只比 wc.db 戳

#### 偏离计划

- 计划只说删 `syncIfRootRevOrWcDbChanged`。实现时 `syncIfWcDbChanged` 也一并换成 `runSvnSyncCheck`：两者只差「强制」分支，留着就是两份「什么时候记戳」的实现。它的测试场景（第一次没有记录、戳没变、读戳失败、记开跑前的值、失败重试、review 第 1 条的顶替 + 失败）全部改写到 `runSvnSyncCheck` 下保留。
- 计划没写 `markOnBlur` 的行为变化：打开仓库现在会记下戳，之后失焦时 `markOnBlur` 先读一次戳核对（原来记录为空、直接按判定写）。这是更保守的方向：TortoiseSVN 在 jkk 前台时完成的提交从打开仓库起就能挡住。`repoStoreSvnSamePath.test.ts` 的「同一个仓库：结论写回后失焦照常写快照」因此要多应答一次 `svn_wc_db_stamps`。

#### 计划没写的坑

- 去掉 `svn_root_revs` 这一跳之后，测试夹具里第一次焦点检测比 `openRepo` 记下 `openedPath` 还早写回，被 `samePath` 守卫当成「已切到别的仓库」丢掉，戳没记上，后面的计数全错。`repoStoreFollowFocus.test.ts` 加了 `opened()`，`settleOn` 先等打开走完。真实应用里打开仓库后第一帧之前的焦点检测同样会被丢掉，下一次事件重跑，与改动前相同。
- 「上一次校验失败：没有判定，不写」原来靠 rootRevs 变化触发一轮失败的校验、同时保持戳不变。rootRevs 没了，改用 HEAD 变化触发。

#### 为什么这么改是安全的

- 第一档的信号只多不少：rootRev 变 ⇒ wc.db 变（revision 存在 wc.db 里）。原来「戳读失败时退回看 rootRevs」的后备在实际中覆盖不到任何情形（两者读失败的条件相同，见计划阶段条目）。
- `svn_root_revs` 命令与 `read_root_revs` 保留，Rust 侧零行为改动。

### Task 2：检测入口共用带「强制」标记的单飞队列

#### 偏离计划

- 计划写 `checkSvnSyncIfHeadMoved` 自己比 HEAD、自己记。实现保留了 `syncIfRootRevChanged` + `recordAfterSuccess`，只把第二档换成 `requestSvnCheck(path, true)` 并把结局映射成「写回了没有」。这样「成功才记」仍只有一份实现，`§6.6.3 两档同步检测` 那组测试原样有效，diff 更小。
- 计划说删 `checkSvnSyncIfChanged` 上的 `refreshFlights.run`：已删，单飞由 `svnCheckQueue` 承担，`refreshFlights` 的 `svnCheck:` key 不再使用。

#### 计划没写的坑

- 「先发起的校验晚到：不能把新结论盖掉」原来断言 B 在 A 还挂着时就写回了 ADVANCED。串行之后 B 排在 A 后面，这条断言不再成立，改写成「B 排队、A 写回后补跑、最终 ADVANCED、失焦不写快照」。原来要防的「A 晚到盖掉 B」在串行下不可能发生（A 一定先写回），`refreshGate` 仍保留，兜住 `applySvnSync` 的其他调用方。
- `HEAD 动了 → 补跑强制` 这条 store 测试第一版没抓住「补跑不合并强制标记」的变异：最后合并进来的恰好就是 HEAD 那次（自带 `force = true`）。在 HEAD 变化之后补一次焦点事件，让最后一次请求是非强制的，变异就被抓到了。
- 「`freshSvnSync` 撞上在跑的检测」与「HEAD 动了 → 补跑强制」在旧代码上也成立（旧代码并发跑，不会丢）；它们是给合并逻辑的回归保护，不是先红后绿。先红的是「第二档不并发」与「排队、补跑」两条。

#### 为什么这么改是安全的

- 串行只会让结论晚到，不会让结论错：每一轮都在开跑时读戳、按 `runSvnSyncCheck` 的规矩记录；排队期间来的请求全部合并进一轮在它们之后开跑的补跑，所以 `freshSvnSync` 拿到的结论一定晚于它的请求。
- 失焦快照的前提不变：`trackSvnCheck` 包在开跑的那一轮上；排队的请求必然对应一轮正在跑的检测，计数不为 0。一轮收尾到补跑开跑之间只隔微任务，焦点 / 失焦是宏任务，插不进去。

### 独立 review（`/code-review high`，Task 1–2 完成后）

| # | 发现 | 结论 | 处理 |
|---|---|---|---|
| 1 | 所有入口排一个队，一轮卡住（svn 进程不退出）后面全部卡住，提交前的 `freshSvnSync` 跟着无限等 | 成立：旧代码里焦点检测不排队，提交不依赖更早的那轮 | **已修**：`freshSvnSync` 不排队，直接 `runSvnCheckNow`；后台那轮晚到时被 `refreshGate` 顶替。测试「提交前的 freshSvnSync 不排在卡住的检测后面」先红（超时）后绿 |
| 2 | `freshSvnSync` 要等在跑那轮 + 补跑一轮，提交延迟最多翻倍 | 成立，与第 1 条同源 | 同第 1 条 |
| 3 | HEAD 在一轮排队 / 在跑时 a → b → a：记录最后被写成 b，横幅停在按 b 算的结论 | 成立：旧代码也有这个窗口，排队后变宽 | **已修**：`headRequested` 记「已请求、未落定」的 HEAD，比较时优先比它。测试「一轮在跑时 HEAD 切走又切回」先红后绿，变异「只比记录」被它抓到 |
| 4 | 非强制时读戳失败直接抛错，没有后备 | 不改：`wc_db_stamps_with` 只有 `read_config` 失败才报错，此时第二档同样读不了配置，后备覆盖不到任何情形（计划阶段条目）。没有专门的测试 | — |
| 5 | wc.db 任一变化都清掉枚举缓存、重新枚举（约 46 个进程） | 成立，属后端，超出 P1 范围 | 记入计划 P4 |
| 6 | 队列按原始路径字符串分 key，同一仓库两种写法会绕过单飞 | 不成立：所有调用点传的都是本标签的 `state.path`（`repo-changed` 处理函数用的是 `state.path!`，不是事件 payload），旧代码的 `svnCheck:${path}` 同样按原始字符串 | 不改 |
| 7 | 打开仓库的强制校验可能排在一轮已在跑的焦点检测后面，首帧前多等一轮 | 成立但罕见：首次挂载不触发焦点检测（「标签没切换的重渲染不算」），只有窗口焦点事件恰好落在打开过程中才会碰上；代价是多一轮本地读 | 记为已知代价 |
| 8 | `svn_root_revs` / `read_root_revs` 成了死代码 | 有意保留（Global Constraints），另起清理 | — |
| 9 | `RootRevMemo` 名字与用途不符 | 外观问题；文档已改写，不改名以免扩大 diff | — |
| 10 | `refreshSvnSyncReporting` 只剩一层包装 | 成立 | **已修**：并进 `refreshSvnSync` |

顺带：第 3 条改完后 `syncIfRootRevChanged` 没有生产代码在用，连同它的 5 条测试删除；「失败不记、同一个值必须重试」由保留的 `recordAfterSuccess` 测试守着（HEAD 变化直接用 `recordAfterSuccess`）。

### 真实窗口 E2E（2026-11-08，合并后的 master `3e68e0b`，含 P1 与 P2）

驱动方式：在 worktree `e2e-svn-scan-perf` 里跑 `pnpm tauri dev --config`（换 identifier 隔离配置；核对过 `app.exe` 在该 worktree 的 `target\debug` 下），通过 WebView2 的 CDP 端口读 DOM、点按钮。焦点用页面内派发的 `window` blur / focus，和真实切窗口走同一组监听。fixture 用 `scripts/e2e-svn-fixture.ps1` 建在 `%TEMP%`，跑完已删。初始化直接调 `svn_init_hybrid`，没走向导（向导不在本轮范围）。暂存用 fixture 里的 `git add`。

#### 计划没写的坑

- `__TAURI_INTERNALS__.invoke` 是不可写、不可配置的属性，包不上。要统计调用了哪些命令，只能包 `window.fetch`：Tauri 2 在 Windows 上把每次 invoke 发成 `http://ipc.localhost/<cmd>`。

#### 结果

按 `e2e/2026-11-01-svn-mixed-baseline-follow-e2e.md` 跑了场景 1、2、4、8、11、12，各项核对全部通过。

| 核对 | 结果 |
|---|---|
| wc.db 没变时切走、切回 5 次 | 10 次 `svn_wc_db_stamps`，0 次 `svn_sync_status` |
| 改白名单内的文件（文件监听） | 2 次 `svn_wc_db_stamps`（立即一次加补查一次），0 次 `svn_sync_status` |
| 场景 1、2、4、12：外部提交 / 局部更新 / 根目录整体更新 | 每次都出横幅，列的文件、revision、作者都对；跟进耗时 2.3～3.3 s。改文件时，监听在切回窗口之前就跑完了完整检查，切回时只读了戳 |
| 场景 8：「仅切换」到落后分支（HEAD 变化入口） | 0.6 s 出「当前分支落后基线」，只跑了 1 次 `svn_sync_status`；跟进时转手动跟进，冲突解掉、继续之后分支到了新基线上 |
| 场景 11：工作副本被锁时切回 | 灰色「SVN 正在更新…」、没有按钮。锁解除后切走再切回，0.2 s 变成黄色横幅，只跑了 1 次完整检查。这次磁盘文件没变，只能靠焦点检测的戳 |
| 压力测试：外部 5 次 `svn update` 加改文件，同时连发 10 次焦点事件 | 5 次 `svn_sync_status` 依次执行，0 次重叠，每次都对应 wc.db 确实又变了；横幅结论正确 |

#### 没覆盖

- TortoiseSVN 钩子路径（`e2e/2026-11-02-svn-c1h-tsvn-hooks-e2e.md`）：注册钩子要写开发机的 `HKCU`，留给用户手动跑。
- 公司机器上的实际耗时（计划 P5）。

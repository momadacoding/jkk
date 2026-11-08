# 跟进 R2b 批量化：实现日志

计划：[2026-11-08-svn-follow-batch.md](2026-11-08-svn-follow-batch.md)

## 进度总账

| Task | 内容 | 状态 | 产出文件 | 验证 |
|---|---|---|---|---|
| 1 | `TempIndex::apply_edits` | 完成 | 改 `src-tauri/src/engine/svn_baseline.rs`（`IndexEdit`、`hash_blob`、`apply_edits`、`stage_modes`；`replace_blob` / `remove_path` 改成薄包装）；测试 `tests/svn_baseline.rs` +7 | 先编译不过后绿；`svn_baseline` 30 条全过（含原有 `replace_blob_*` / `remove_path_*`）；变异「关掉 D/F 检查」被 `apply_edits_refuses_a_file_directory_conflict` 抓到 |
| 2 | `rewrite_to_server` 并行取数 + 一次写索引 | 完成 | 改 `src-tauri/src/engine/svn_init.rs`（`BASE_FETCH_WORKERS`、`fetch_base_blobs`、`fetch_base_blob`）；测试 `tests/svn_init.rs` +1 | 新测试在旧代码上也绿（回归保护）；`svn_init` 100、`svn_sync_in` 136、`svn_follow` 77、`svn_baseline` 30 全过；开发机实测见下；全量 `cargo test` 见 PR |

## 决策条目

### Task 0：实测 `update-index -z --index-info`

在临时 fixture 仓库里跑（`core.autocrlf=true`，临时集成测试 `tests/zz_probe_index_info.rs`，跑完已删）。git 版本同开发机（Git for Windows 2.51）。

#### 计划没写的坑

| # | 场景 | 结论 | 对实现的约束 |
|---|---|---|---|
| X1 | 一批里替换已有条目、新建条目、`0 <零 OID>` 剔除一个磁盘上还在的文件 | 全部生效，exit 0；磁盘文件不动 | 一次调用就能代替逐条 `--cacheinfo` 与 `--force-remove` |
| X2 | 不带 `--add` 新建路径 | 照样建出来 | 仍显式带 `--add`，不依赖这一点 |
| X3 | D/F：索引里有文件 `Code/F`，放 `Code/F/Y.cs`（先放后删、先删后放两种顺序） | **静默替换**，`Code/F` 被挤掉，exit 0 | 旧写法 `--cacheinfo` 同样情形 exit 128（`appears as both a file and as a directory`）。必须自己查冲突，否则基线里会少一个不在 `S` 里的条目 |
| X4 | 剔除一个不在索引里的路径 | exit 0 | 与旧 `--force-remove` 相同 |
| X5 | 空 stdin | exit 0 | 仍在调用前 no-op，不起进程 |
| X6 | 某行路径为空 | 只打印 `Ignoring path`，exit 0 | 空路径必须调用前拒绝，不能交给 git |
| X7 | `Code/角色 x.cs`（`-z`） | 正常写入，`ls-files -s -z` 原样读回 | 两头都用 `-z` |
| X8 | 某一行格式不对 | 整批 fatal（exit 128），同一批里排在前面的合法行也**没有**写入 | 一批是原子的：失败时候选索引不动 |
| X9 | `ls-files -s -z` 的格式 | `<mode> <oid> <stage>\t<path>\0` | 按第一个 `\t` 切路径 |

### Task 1：`TempIndex::apply_edits`

#### 偏离计划

- 计划写的 D/F 判定是「祖先或后代是现有条目、且不在本批的剔除里」。实现改成**按编辑顺序逐条模拟**索引里有哪些路径（`present` 集合，剔除就移出、放入就加进）。这与旧的逐条 `--cacheinfo` / `--force-remove` 严格等价：同一批里「先放子路径、后剔除文件」旧写法会在放的那一步报错，按集合判定却会放行；两个 `Put` 互相冲突（`Code/F` 与 `Code/F/Y.cs`）按集合判定也查不到。`rewrite_to_server` 的编辑按 `BTreeMap` 路径顺序排，`Code/F` 总在 `Code/F/Y.cs` 前面，所以「文件变目录」的合法情形照样通过。
- 计划里 `stage_modes` 没列成公开接口；实现成 `pub`，给 `apply_edits` 用，也方便以后排查。

#### 为什么这么改是安全的

- 录出的 tree 不变：mode 规则（沿用已有、否则 `100644`）、`hash-object --path` 都原样保留；剔除的语义（磁盘上还在也摘掉）由实测 X1 确认。
- 失败时候选索引不动：空路径、D/F 冲突在写之前就拒绝；git 自己遇到坏行也是整批不写（X8）。候选索引本来就是临时文件，失败后整条录制回退，`Drop` 删掉它。

### Task 2：`rewrite_to_server` 并行取数 + 一次写索引

#### 偏离计划

- 无。并行度取计划写的 6。

#### 计划没写的坑

- 恢复备份做对照实验时要刷新 mtime（`Copy-Item` 会连 mtime 一起复制，cargo 以为没变、不重编译）。这次用 `cp`（不保留 mtime）再 `touch`，变异检查用 PowerShell 还原后显式改 `LastWriteTime`。

#### 开发机实测

临时测试 `tests/zz_perf_r2b.rs`（跑完已删）：fixture 上 150 个文件改脏（`TakeBase`）、30 个未版本控制（`Drop`），`rebuild_baseline` 连跑 3 次，debug 构建。改动前把 `svn_init.rs`、`svn_baseline.rs` 临时换回 `master` 的版本。

| 版本 | 3 次耗时 | 录出的 tree |
|---|---|---|
| 改动前（逐条串行） | 29.1 s / 28.7 s / 28.3 s | `9daaec5a69…` |
| 改动后（6 路并行 + 一次写索引） | 3.36 s / 3.35 s / 3.63 s | `9daaec5a69…` |

tree OID 三次、两版完全相同。重建里除 R2b 外还有预检、`git add`、体检等固定开销，所以 R2b 本身的降幅比整体的 8 倍还大。公司机器的数字要等实际跟进时再录（计划 P5 的 ETW 方法）。

#### 为什么这么改是安全的

- 并行的只有只读取数：`svn cat -r BASE` 只读 pristine 与 wc.db（实测上表 150 个文件 6 路并行、连跑 3 次无报错，结果与串行逐字节相同；没有专门验证它与 TortoiseSVN 同时写 wc.db 时的表现，那种情形串行版本同样会撞上，由录制后的复核兜住）；`hash-object -w` 写对象是「临时文件 + 改名」，同一内容写两次得到同一个对象。写索引仍只有一次、在所有取数完成之后。
- 任何一个文件取不到就整批报错、不写索引，与原来「取到一半报错」相比更干净（原来已改写的那几条会留在候选索引里，随后整条录制回退，结果相同）。
- `NotOnServer` 的错误码与文案不变；报出的错与串行时**相同**，是路径顺序里第一个失败的：任务按下标单调领取，失败之后只是不再领新任务，已领的照常跑完，而下标最小的失败项一定在第一个报错的之前或就是它，早已领走（`run_parallel` 文档；单元测试 `the_error_is_the_first_failing_item_in_input_order_like_a_serial_run`）。初稿这里写的是「可能与串行不同」，推理错了，review 后改正。

### 独立 review（`/code-review high`，Task 1–2 完成后）

| # | 发现 | 结论 | 处理 |
|---|---|---|---|
| 1 | 并行时最多 6 个文件的完整内容同时在内存里，大文件会放大峰值内存 | 成立；白名单按扩展名收代码类文件，单个通常很小 | 记为已知代价（`BASE_FETCH_WORKERS` 文档）。若以后白名单收大资源，再改成按体积限流或流式 |
| 2 | Windows 上内容相同的两个文件并发 `hash-object -w` 同一个对象，改名可能撞上另一个进程 / 杀软报权限错，整批失败 | 有可能（串行碰不到）；没在本机复现 | **已修**：`hash-object` 失败重试一次（幂等，重试得到同一个 OID） |
| 3 | 同一批「先剔除 p 再放 p」沿用了旧 mode，与文档的「逐条等价」不符；mode 若是 gitlink 会写坏条目 | 成立 | **已修**：只在 p 此刻仍在 `present` 里时沿用 mode。测试 `apply_edits_does_not_carry_a_mode_over_a_remove_in_the_same_batch` |
| 4 | 并行的失败路径（提前停、取哪个错误、没领到的槽位）没有测试 | 成立 | **已修**：抽出泛型 `run_parallel`，`svn_init.rs` 内 4 条单元测试（乱序完成仍按序返回、报的是第一个失败项、失败后不再领新任务、单线程 / 空输入） |
| 5 | OID 先进 `BTreeMap` 再取，留了一条永远走不到的错误分支 | 成立 | **已修**：剔除项一并进 `run_parallel`（返回 `None`），结果与 `todo` 直接 zip |
| 6 | 每次 `apply_edits`（含单条包装）都读全量索引、复制全部键 | 生产路径一次跟进只调一次；单条包装现在只有测试在用 | 不改 |
| 7 | D/F 判定依赖编辑顺序 | 有意：与旧的逐条执行严格等价（Task 1 偏离计划条目）；举的例子（服务器是文件、本地是目录）在上游 `svn status` 是 obstructed，早被拒绝 | 不改 |
| 8 | `stage_mode` 没有生产调用方；`replace_blob` 归一化了两次路径 | 前者是公开接口、测试在用，保留；后者成立 | **已修**后者 |
| 9 | 固定 6 线程，不看核数 | 成立 | **已修**：`fetch_workers()` 取 `min(6, available_parallelism)` |
| 10 | 新错误码 `svn_baseline_index_conflict` 前端没有提示文案 | 成立 | **已修**：`src/engine/svnInit.ts` 加提示，`tests/vitest/svnInit.test.ts` +1 |

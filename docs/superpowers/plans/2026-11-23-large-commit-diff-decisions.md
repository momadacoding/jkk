# 超大提交的 diff（第 73 条）实现日志

计划来源：GitHub 第 73 条正文里的四步方案（没有单独的计划文件）。

## 进度总账

| Task | 内容 | 状态 | 产出 | 验证 |
|---|---|---|---|---|
| 1 | 分步计时，定位时间花在哪 | 完成 | 计时用的 `probe_50k_steps`（数字记在下面，探针本身没留，验收测试 `perf_50k` 取代它） | 见 Task 1 |
| 2 | 后端：文件太多时只给列表；按文件读 diff 的新命令 | 完成 | `engine/diff.rs`（`SUMMARY_FILE_LIMIT`、`tree_diff`、`summary_data`、`read_diff_file`）、`commands/diff.rs`、`lib.rs` | `tests/diff_large_commit.rs` 10 条（含 review 补的 1 条） + `perf_50k`（release） |
| 3 | 前端：只给列表时按选中文件单独读 diff，行数一栏不显示 | 完成 | `CommitDetailView.tsx`（`FileDiffOnDemand`）、`repoStore.ts`（`readDiffFile`）、`types.ts` | `CommitDetailView.test.tsx` 新增 6 条 |
| 4 | 前端：Commit 标签平铺列表截断、Changes 树默认收起 | 完成 | `CommitDetailView.tsx`（`COMMIT_LIST_LIMIT`） | 同上 |
| 5 | 验收：5 万文件目标值、`pnpm tauri dev` 实测新命令 | 完成 | — | 见 Task 5 |

## 决策条目

### Task 1：计时

5 万个新增小文件的根提交，loose 对象（fixture 刚 `git commit` 完），release 构建，2026-11-23 本机：

| 步骤 | 耗时 |
|---|---|
| `diff_tree_to_tree`（只建 delta） | 12 ms |
| `find_similar(None)` | 1 ms |
| `Diff::stats()` | 23.2 s |
| 逐文件 `Patch::line_stats` | 22.0 s |
| `read_diff` 整体（含 `print(Patch)`） | 22.3 s |
| `serde_json::to_string` | 21 ms，14.0 MB |

- **计划的错误**：方案第 3 步假设改名检测是大头，实测只有 1 ms（根提交没有删除项，没有可配对的来源）。
  时间几乎全在**读 blob**：只要算行数或出 hunk，就得把 5 万个 blob 全部解压一遍，
  `Diff::stats()` 和逐文件 `line_stats` 一样慢。所以第 2 步「summary 只返回文件列表与增删行数」
  省不下时间，行数本身就是那 22 秒。
- **偏离计划**：大提交的列表**不带增删行数**，只有路径、状态、改名前路径。行数要等点到那个文件、
  读出它的 diff 才有。
- 序列化不是瓶颈（21 ms），但 14 MB 过 IPC 再在前端 `countChanges` 一遍也要时间；去掉 hunk 后体积随之下降。

### Task 2：后端

- **做法**：`commit:` / `range:` 先只建 delta 列表（不读 blob）数一下，超过 `SUMMARY_FILE_LIMIT`（1000）个
  就只回列表（`summaryOnly: true`，每个文件 `hunks` 为空、没有行数）；没超过的走原来的路，代码与结果都不变。
  新命令 `read_diff_file(path, scope, file, oldFile?, options?)` 读其中一个文件。
- **阈值为什么是 1000**：普通提交的行为必须不变（验收要求）。1000 个文件以内全读，loose 对象下最坏约 0.45 秒；
  上了 1000 才换成只列文件。与第 73 条方案第 3 步建议的数字相同。
- **按改名配对之前的 delta 数算**：600 个改名是 1200 个 delta，也会走只列文件。这时改名照样配（见下一条），
  只是没有行数，可接受。
- **计划没写的坑：libgit2 的 exact 改名配对是逐对比较，不是哈希。** `find_similar` 即使
  `EXACT_MATCH_ONLY`，也是对每个新增项从头试来源，最多试 `diff.renameLimit`（默认 1000）个就停
  （`libgit2/src/libgit2/diff_tform.c` 的 `find_best_matches`，libgit2-sys 0.18.8 源码核对）。5 万个文件挪目录，
  实测只配上前一千来个，其余摊成一删一增。所以只列文件时**不用 `find_similar`**，自己按 blob OID 用 HashMap
  配：O(n)，不读 blob。5 万文件挪目录 63 ms。
- **偏离计划（改名口径）**：只列文件时只认内容完全相同的改名；内容也改了的改名要读 blob 算相似度，摊成一删一增。
  空文件不配（内容全是空的，配成哪对都是猜），只配普通文件（与 libgit2 `GIT_MODE_ISBLOB` 同口径）。
- **按文件读与列表口径一致**：`read_diff_file` 把 `oldFile` 和 `file` 都放进字面量 pathspec，再按 `renames(true)`
  检测。列表里配成对的（内容相同）这里一定配得上；列表里摊开的，前端只传新路径，读出来也是新增。
- **为什么这么改是安全的**：
  - 空 `file` 直接报 `bad_diff_scope`：空 pathspec 在 libgit2 里是「全部」，不拦就成了读整个提交（§6.2）。
    `oldFile` 为空串当没有。
  - 只认 `commit:` / `range:`，工作区类 scope 一律报错：这个命令只读对象库，不碰工作区。
  - 字面量 pathspec 仍按目录前缀匹配，结果只留路径恰好等于 `file` 的那项（复用 `diff_to_data_only`），
    点名一个目录不会读出目录下所有文件（`reading_one_file_is_an_exact_match_not_a_directory_prefix`）。
  - 纯读取，没有写操作，失败只影响右侧那一个文件的显示。
- 命令不经过 `Vcs` trait：与 `commit_file_diff` 一样只读对象库，git 与 svn 混合仓库是同一条路。

### Task 3、4：前端

- 只列文件时，Changes 右侧换成 `FileDiffOnDemand`，按选中文件调 `read_diff_file`，写法照搬文件历史的 `EntryDiff`
  （diff 开关变了按新选项重读，换文件才说正在读取）。右侧带路径头：树默认收起，选中的文件多半藏在没展开的目录里。
- Commit 标签：只列文件时不显示 `+0/-0`（会被读成「没改」），顶上写明为什么没有行数；平铺列表最多画
  `COMMIT_LIST_LIMIT`（1000）行，超过的写明「完整列表在 Changes 标签页」。普通提交不超过 1000 个文件，不受影响。
- **偏离计划**：方案第 4 步提的虚拟滚动没做，换成截断 + 树默认收起（File Tree 标签页已有同样的先例）。
  遗留：单个目录里直接放几千个文件时，展开那个目录仍会一次画几千行。

### Task 5：验收

- `perf_50k`（release，fast-import 造的 packed 仓库，2026-11-23 本机）：5 万新增的列表 **24 ms / 4.2 MB**
  （Task 1 的 22 s / 14 MB 是 loose 对象下量的，两边对象存储不同，不是严格对照；新路径只读 tree 不读 blob，
  loose 下也只多读 501 个 tree 对象），读其中一个文件 **8.5 ms**，5 万文件挪目录的列表 **63 ms**。
  目标值写进测试：各自 1 秒以内。
- `pnpm tauri dev`（`--config` 换 identifier 隔离，核对运行的是本 worktree 的 `target\debug\app.exe`），
  5000 文件 fixture，CDP 调 `__TAURI_INTERNALS__.invoke`：
  - `read_diff` 5000 文件：26 ms，`summaryOnly=true`、5000 项、0 个 hunk；挪目录：4999 条改名，改过内容的那个一删一增
  - `read_diff_file`：11 ms，读出那一个文件的两行；`range:` 带 `oldFile`、不传 `options` 读出改名
  - 反例：参数名写成 `file_path` 报 `missing required key file`；空 `file` 报 `bad_diff_scope`
  - 界面：Commit 标签说明文字、平铺 1000 行、无行数栏；Changes 树收起、右侧显示第一个文件的 diff；
    普通的 2 文件提交仍是 `+2/-0`、`+1/-0`，没有说明文字

### Review（/code-review high，2026-11-23）

没有高危问题。处理结果：

| 发现 | 处理 |
|---|---|
| 只列文件时改名配对不看 `diff.renames`，与没超上限时不一致 | 已修：读 `diff.renames`，关掉就不配；补 `diff_renames_false_turns_off_pairing_for_large_commits_too` |
| 空 `file` 报「无法解析的 diff 范围」，指错了对象 | 已修：改为「没有指定要读的文件」，错误码不变；测试断言文案 |
| 文件直接放在仓库根目录时，树默认收起也挡不住：根这一层照样一次画几万行 | 遗留：Unity 工程的文件都在 `Assets/` 等目录下，实际少见；要彻底解决得给 `FileTree` 做按层截断或虚拟滚动 |
| 只列文件时改 diff 开关，整份列表也重读一次（5 万文件约 4 MB 过 IPC），列表并不依赖开关 | 遗留：后端 24 ms，实际可感知的是前端重建 5 万节点的树；要改 `refreshDiff` 的重读条件，另开 |
| 改 diff 开关时，后台标签里的 `FileDiffOnDemand` 也会重读当前文件 | 遗留：只读一个文件（约 10 ms）；要做的话把标签是否在前台传进去 |
| 类型变化（文件变链接）摊成同路径一删一增时，按路径去重会丢掉一半 | 既有问题，`diff_to_data_only` 同样如此，不在本条范围 |

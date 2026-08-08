# worktree 删除改为可撤销 — 决策日志

> 配套实现：`worktree` 删除从「永久删除」改为「移入仓库内回收站 + 撤销窗口」，并补删除确认框。
> 本文件按 `AGENTS.md` §2.1 维护，随实现进同一个 PR，是 review 的输入。

## 背景

用户提问「worktree 的删除是有没有走回收站？」，答案是没有：`git worktree remove` 永久删除，
失败时我们用 `std::fs::remove_dir_all` 兜底，也是永久删除。方案讨论后选定 **B + C**：

- **B**：同卷 rename 到仓库内 `.worktrees/.trash/`，给撤销窗口
- **C**：补删除确认对话框（原先 `WorktreeView.tsx` 的 ✕ 一键无确认永久删除）

被否掉的 A（引入 `trash` crate 走系统回收站）：`trash` crate 对 junction 的行为无文档
（docs.rs 顶层未写，ctx7 未收录该 crate），而这一条破了就是删掉仓库外数据的事故；
另有跨卷不可用、回收站配额超限静默转永久删除、十几万文件入回收站很慢等问题。

## 进度总账

| # | Task | 状态 | 产出文件 | 验证 |
|---|---|---|---|---|
| 0 | 前置行为验证（git 真实行为） | ✅ 完成 | 见「Task 0」决策条目 | fixture 实测，7 组实验 |
| 1 | 决策日志建档 | ✅ 完成 | 本文件 | — |
| 2 | Rust：删除改为搬入回收站 | ✅ 完成 | `src-tauri/src/commands/worktree.rs` | `cargo test` 101 passed |
| 3 | Rust：撤销命令 | ✅ 完成 | 同上 + `src-tauri/src/lib.rs` | 同上 |
| 4 | Rust：回收站清理策略 | ✅ 完成 | 同上 | 同上 |
| 5 | Rust 集成测试 | ✅ 完成 | `src-tauri/tests/worktree_ops.rs` | 18 passed（该文件） |
| 6 | 前端：确认框 + 撤销 UI | ✅ 完成 | `src/components/DeleteWorktreeDialog.tsx`、`WorktreeView.tsx`、`RepoTab.tsx`、`src/engine/errors.ts`、`src/styles.css` | `pnpm build` ok、`pnpm test` 131 passed |
| 7 | 占用诊断：查出是谁占着 worktree | ✅ 完成 | `src-tauri/src/engine/file_lock.rs`（新）、`Cargo.toml` | `cargo test` 108 passed；稳定性连跑 20 次 0 失败 |
| 8 | 独立 review（§4.1 门槛） | ⬜ **未做** | — | 合并前必须补，实现者自查不满足门槛 |
| 9 | `pnpm tauri dev` 真实窗口验证 | ⬜ **未做** | — | 命令签名改了，§7.3 要求实跑一次 |

---

## Task 0：前置行为验证

所有实验在 `scratchpad/` 下的一次性 fixture 仓库里做，未碰任何真实仓库（`AGENTS.md` §8）。

### 计划没写的坑（实测才暴露）

| # | 实测结论 | 对设计的影响 |
|---|---|---|
| 0-1 | 工作树目录被外部移走后，`git worktree remove <path>` 仍 **exit 0 并干净解除登记**，不需要 `prune` | 「先搬走目录再解除登记」这条路成立 |
| 0-2 | `git worktree lock` 的保护**不随目录移走失效**（git 从登记读，不看目录）；`git worktree list --porcelain` 有 `locked <reason>` 行 | 锁不会被绕过，但必须**预检**，否则会出现「目录已进回收站、解除登记却被拒」的不一致状态 |
| 0-3 | 脏检查可用 `git -C <wt> status --porcelain` 预检（未跟踪文件显示为 `?? x`） | 倒转顺序后 git 的脏检查不再自动生效，必须自己补上 |
| 0-4 | 同卷 rename 含 junction 的目录：**15ms**，junction 保持为链接未被跟进，仓库外的目标文件毫发无伤 | 方案 B 的安全性与 O(1) 成本的直接证据 |
| 0-5 | `git worktree add` 对**已存在的目录一律拒绝**，`--force` 也不行（`fatal: '...' already exists`） | 撤销不能走「搬回原位 + 重新 add」 |
| 0-6 | `git worktree repair` 救不回来：管理目录 `.git/worktrees/<name>` 被 `remove` 删掉后无法重建 | 撤销不能走 repair |
| 0-7 | **worktree 的 index 存在管理目录 `.git/worktrees/<name>/index` 里，不在工作树目录内** | 只搬工作树必然丢暂存状态：实测 `A s.txt` 恢复成了 `?? s.txt` |
| 0-8 | 推迟解除登记（只搬走、不 remove）撤销是 O(1)，但会留 `prunable` 登记，**挡住同名分支重建**（`fatal: 'keep' is already used by worktree at ...`） | 否掉该方案，删除时就要让 git 状态干净 |
| 0-9 | **工作树 + 管理目录一起搬走** → git 彻底不认，`list` 零残留，同名分支可立刻重建；两者一起搬回 → `status` 与删除前**逐字一致**（含暂存状态），可正常提交 | 最终采用的方案 |
| 0-10 | 撤销时 `.git/worktrees/` 父目录可能已因为空而不存在，直接 rename 回去会 `No such file or directory` | 撤销前必须 `create_dir_all` 父目录 |

### 计划的错误

- 方案讨论阶段设想的撤销是「rename 回原位即可」，这是错的 —— 0-5/0-6/0-7 三条共同证明只搬工作树
  既恢复不了登记、也恢复不了暂存状态。正确做法是 0-9：**工作树和管理目录成对搬移**。
- 管理目录**不能靠目录名去猜**（同名冲突时 git 会加后缀）。必须用
  `git -C <wt> rev-parse --absolute-git-dir` 解析。

### 为什么这么改是安全的

倒转「谁来删文件」这件事本身就消除了两条既有的数据丢失路径：

1. `worktree_ops.rs:250-253` 记录了一条已验证的事实：**`git worktree remove` 会跟进 junction
   把目标里的东西删掉**。改造后 git 不再负责删文件，这条路径消失。
2. `worktree.rs:59-77` 记录的 pnpm junction bug（git 递归删除栽在死链接上、删一半放弃）同样消失，
   因为不再有「git 删一半」这个状态。

代价是绕过了 git 自带的两道保护（脏检查、lock 检查），二者都已验证可预检（0-2、0-3），
必须在搬移**之前**做，否则会留下「目录已进回收站但登记还在」的不一致状态。

---

## Task 2-6：实现决策

### 偏离计划

| # | 实际做法 vs 方案讨论时说的 | 原因 |
|---|---|---|
| 2-1 | 方案说「rename 到 `.trash` 后再 `git worktree remove` 解除登记」。实际**不跑 `remove`**，而是把管理目录也搬进回收站 | 0-7/0-9：登记就是管理目录本身，搬走它即等于解除登记，还顺带把 index 一起保住了。多跑一次 `remove` 反而会把管理目录删掉，撤销就没了 |
| 2-2 | 方案说撤销是「rename 回原位」。实际是**两个目录成对搬回**，且搬之前要 `create_dir_all` 父目录 | 0-7、0-10 |
| 2-3 | 方案没提 force。实际加了 `force: bool` 参数，脏的默认拦、用户确认后放行 | 倒转顺序后 git 的脏检查不再自动生效（0-3），这道保护必须自己补，而补了就必须给用户一个越过它的方式，否则脏 worktree 永远删不掉 |
| 2-4 | 保留 `.trash` 的清理上限定为 5 个，不是按时间过期 | 按时间过期要么需要后台任务，要么在启动时扫；按个数在每次删除时顺手做掉，简单且够用 |

### 计划的错误

- 方案对比表里写方案 B「不是系统回收站，用户在资源管理器里找不到」，这条仍然成立；但当时把
  「顺带修掉 pnpm bug」列为唯一附加收益，**漏了更重要的一条**：`git worktree remove` 会跟进
  junction 删掉链接指向的外部数据（`worktree_ops.rs` 旧注释里已有记录，改造前单独验证过）。
  这是一条真实的数据丢失路径，改造后一并消失，已由
  `deleting_never_touches_what_a_junction_points_at` 钉住。

### 计划没写的坑

| # | 坑 | 处理 |
|---|---|---|
| 2-5 | 管理目录不能靠工作树目录名去拼——同名冲突时 git 会加后缀 | 用 `git -C <wt> rev-parse --absolute-git-dir` 问 git 要 |
| 2-6 | 锁的判据在登记里，不预检的话会出现「目录已进回收站、解除登记却被拒」的不一致状态 | `lock_reason()` 在搬移前预检，force 也不放行，由 `a_locked_worktree_is_refused_even_with_force` 钉住 |
| 2-7 | `handleDeleteBranch` 里也调 `delete_worktree`，签名改了必须同步（§7.3） | 已传 `force: false`，保持与侧边栏删除一致的保护强度 |
| 2-8 | 确认框要判断「脏不脏」，但为此加一个查询命令会让弹框和真正执行删除用两套判据 | 改成先按 `force: false` 删一次，撞上 `worktree_dirty` 再把按钮升级成「仍要删除」。判据只有一处 |
| 2-9 | 前端靠 message 文案判断错误类型太脆 | 新增 `toErrorCode()`（`src/engine/errors.ts`），有 vitest 覆盖 |
| 2-10 | **自查发现的漏洞**：「不能删主工作区」原先是 git 给的保护（`fatal: '.' is a main working tree`）。git 不再参与删除后这道保护凭空消失了，而主工作区的管理目录**就是仓库的 `.git` 本身**。界面上禁着按钮拦不住直接调命令的路径 | 在所有检查最前面加一票否决，判据是 `--absolute-git-dir` == `--git-common-dir`（实测：主工作区两者相等，linked worktree 不等）。由 `the_main_worktree_can_never_be_deleted` 钉住 |

> 2-10 的测试做过**反向验证**：临时摘掉保护后该测试确实失败（拿到 `worktree_dirty` 而非
> `worktree_is_main`）。顺带暴露一件事——没有专门保护时，主工作区只是**偶然**被脏检查拦住
> （仓库根通常有未跟踪的 `.worktrees/`），干净的主工作区会一路走到 rename。这条保护不是冗余的。

### 为什么这么改是安全的

1. **两条既有的数据丢失通道被消除**（见上文「计划的错误」和 Task 0「为什么这么改是安全的」）。
2. **保护没有变弱**：git 原本的脏检查、锁检查都由预检补上，且都在搬移**之前**执行，
   失败时目录一动未动（`a_dirty_worktree_is_refused_and_left_untouched` 断言了连回收站里都
   不留空槽位）。
3. **每一步失败都有回滚**：管理目录搬不动 → 工作树搬回原位；`meta.json` 写不成 → 两个目录
   都搬回去；撤销时管理目录搬不回 → 工作树退回回收站。回滚失败的情况下错误信息里带上东西
   现在在哪、怎么手工搬回去。
4. **回收站清理仍然只用 `remove_dir_all`**，它解链接而不跟进目标，这条不变量由
   `trash_pruning_unlinks_junctions_instead_of_following_them` 继续钉着。
5. **撤销不会覆盖任何东西**：原位置或管理目录只要已被占用就拒绝
   （`undo_refuses_to_overwrite_whatever_took_the_place`），且只接受本仓库回收站底下的路径
   （`undo_only_accepts_paths_inside_this_repos_trash`）。

---

## Task 7：占用诊断（「移不动」时查出是谁占着）

用户反馈：撞上 `worktree_busy` 时只有一句 os error 32，看不出是哪个进程。

### 计划没写的坑

| # | 坑 | 处理 |
|---|---|---|
| 7-1 | **Windows 上有两种占用，难度差一个数量级**。打开了里面的文件 → Restart Manager 能给进程名+PID；把目录当**工作目录**（cwd）→ RM 完全无效，要枚举进程 cwd 得走 `NtQueryInformationProcess`。而删 worktree 最常撞上的恰恰是后者 | 分两步：先「独占打开」探测出**是哪个路径**（两种都测得出），再对该路径问 RM 要进程（只有前者拿得到）。查不出进程时明说「多半是有程序的工作目录停在这里」——这本身就是可动手的结论 |
| 7-2 | 原方案 A 打算「逐层 rename 定位」。实际改用**独占打开探测** | rename 探测要真的移动目录，中途失败会把目录打散；独占打开是纯只读，且实测能精确命中 cwd 占用（造了个 cwd 停在子目录的进程，探测准确报出该子目录，同级目录报 FREE，而整体 rename 确实失败） |
| 7-3 | 探测**绝不能跟进 junction**，否则会走到仓库外面、甚至绕成环 | `symlink_metadata` 判断，链接一律不进；由 `probing_does_not_follow_junctions` 钉住 |
| 7-4 | **杀软/索引服务会在文件刚落盘时瞬时握住其所在目录**，只测一次会误报。实测这让测试 6 次里 flaky 1 次 | 确认前隔 30ms 复检一次，两次都占才算数。指着一条无辜路径让人去找，比不报还糟 |
| 7-5 | 装了依赖的 worktree 有十几万文件，全测一遍太慢 | `MAX_PROBES` 上限 20000，最多报 3 个；真占用通常几百个条目内就撞上 |

### 计划的错误（自己写出来的 bug）

加复检（7-4）时写错了控制流：复检发现是瞬时占用后**直接 `return`**，导致不再遍历子目录，
真正被占的文件反而永远探测不到。表现是测试从「偶尔报错路径不对」变成「一个都找不到」，
20 次里失败 4 次。正确写法是「当作没占用，继续往里探」。修完连跑 20 次 0 失败。

### 为什么这么改是安全的

- 探测**全程只读**：只尝试独占打开，不移动、不修改、不删除任何东西。而且它只在 rename
  **已经失败之后**才跑，那时目录一动未动。
- 只把 os error 32/33（共享冲突、拒绝访问）算作占用。找不到、权限不足之类一律不报，
  免得把无关路径塞进错误信息。
- `windows` crate 本来就在依赖树里（tao → tauri v0.61.3），这里只是显式声明并打开
  `Win32_System_RestartManager` feature，不新增下载；且用 `[target.'cfg(windows)'.dependencies]`
  声明，非 Windows 平台不引入。
- RM 那段 unsafe 全程持有一个已开好的 session，末尾无条件 `RmEndSession` 收尾；
  所有指针都指向本函数的局部缓冲区。
- **有真实占用的端到端测试**：`a_held_file_is_found_and_its_holder_is_named` 在测试进程里
  握住一个文件，断言 RM 报出的 PID 就是自己——这是 RM 那半边真的工作的唯一证据；
  `a_directory_held_as_a_working_directory_is_found` 起一个 cwd 停在子目录的子进程，
  断言路径被定位到。

### 已知遗留

- `cargo fmt --check` 在本分支不通过，但**基线 master 上就有 70 个 diff 块**，本仓库从未 fmt
  干净过。单独格式化本次改动的文件会与周围代码风格冲突，按 §1.2 保持最小 diff，未处理。
- `cargo clippy` 有一条 `incompatible_msrv` 告警，位置在 `src/engine/branch.rs:342`
  （`is_none_or`），**非本次改动引入**，未处理。
- 命令签名改了（`delete_worktree` 加 `force`、新增 `restore_worktree`），§7.3 要求实际跑一次
  `pnpm tauri dev` 验证 invoke 真的通，**尚未执行**。
- §4.1 的独立 review 门槛尚未满足。
- 占用诊断查不出「把目录当 cwd」的那个进程是谁（7-1）。要做到得走
  `NtQueryInformationProcess` 枚举所有进程的 cwd，既底层又要权限，Sysinternals 的 handle.exe
  是靠内核驱动做的。当前实现明确告诉用户「是这一类占用」，已足够动手。

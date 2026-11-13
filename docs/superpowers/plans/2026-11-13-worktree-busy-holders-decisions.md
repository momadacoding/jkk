# worktree 被占用：查出进程并允许结束 —— 实现日志

> 没有独立计划文件。需求来自对话：删 worktree 撞上 `worktree_busy`，弹框只说「查不出是哪个进程，
> 多半是有程序的工作目录停在这里」。用户选了方案 **B + A + 结束进程按钮**：
>
> - B：读每个进程 PEB 里的 `CurrentDirectory`，查出「工作目录停在 worktree 里」的进程
> - A：读每个进程的命令行，查出「启动参数指向 worktree」的进程，兜底
> - 弹框里列出这些进程，每个带「结束进程」按钮
>
> 触发这件事的真实案例：一个 Codex 插件的 `app-server-broker.mjs`（node.exe）以
> `--cwd <worktree>` 启动，父进程早已退出，孤儿状态挂了 16 天，worktree 一直删不掉。

## 进度总账

| Task | 内容 | 状态 | 产出文件 | 验证 |
|---|---|---|---|---|
| 1 | Rust：枚举进程的 cwd / 命令行，判定谁占着 worktree；结束进程 | ✅ | `src-tauri/src/engine/process_holders.rs`（新）、`Cargo.toml`（4 个 feature） | 单测 15 个；把 PEB 读取改成恒 `None` 后 4 个 cwd 测试全挂（变异检查） |
| 2 | Rust：`describe` 在查不出 RM 进程时报出 cwd 持有者 | ✅ | `src-tauri/src/engine/file_lock.rs` | `a_directory_held_as_a_working_directory_is_found` 改为断言 PID 与文案 |
| 3 | Tauri 命令 `worktree_holders` / `terminate_worktree_holder` | ✅ | `commands/worktree.rs`、`lib.rs`、`tests/worktree_ops.rs` | 集成测试 30 passed，含「占用 → 点名 → 结束 → 删成功」整条路 |
| 4 | 前端：占用进程面板 + 两个删除弹框接入 | ✅ | `src/engine/worktreeHolders.ts`、`WorktreeHoldersPanel.tsx`、`DeleteWorktreeDialog.tsx`、`DeleteBranchDialog.tsx`、`WorktreeView.tsx`、`repoStore.ts`、`styles.css` | `pnpm build` ok；vitest 新增 16 个 |
| 5 | 布局场景 + `pnpm test:layout` | ✅ | `tests/layout/{scenarios.ts,harness.tsx,dialogs.layout.ts}` | 164 passed（新场景 800×600 / 800×400 各 6 项） |
| 6 | 全量验证 + `pnpm tauri dev` fixture 实测 | ✅ | — | 真实窗口走通；实测抓到 StrictMode bug（6-1），已修 |
| 7 | 独立 review（§4.1 门槛） | ✅ | 见下「Task 7」 | 独立 opus agent 只读 review：无阻断；4 条应修 + 5 条小问题已修，1 条记录不修 |

## 决策条目

### Task 1：进程枚举与结束

#### 偏离 / 取舍

| # | 决定 | 理由 |
|---|---|---|
| 1-1 | 命令行用 `NtQueryInformationProcess(ProcessCommandLineInformation)`，**不读 PEB** | 只要 `PROCESS_QUERY_LIMITED_INFORMATION`，提权进程也读得到；PEB 那条要 `PROCESS_VM_READ` |
| 1-2 | 不开 windows crate 的 `Win32_System_Kernel`（PEB 结构体），自己按偏移读 | 只要 `PebBaseAddress` 一个字段，按 6 个指针宽整数读 `PROCESS_BASIC_INFORMATION` 即可 |
| 1-3 | 判定结果分三档 `cwd` / `openFile` / `commandLine`，命令行一档在界面上明说「可能无关」 | 命令行只是旁证。E2E 时驱动脚本自己的 argv 带着 worktree 路径，就被列了进来——这正是该档的真实误报形态 |
| 1-4 | 结束按钮对本应用自己、RM 标成 `RmCritical` 的进程不给 | 自杀没有意义；系统关键进程结束了会出更大的事 |

#### 计划没写的坑

| # | 坑 | 处理 |
|---|---|---|
| 1-5 | **WOW64 进程有两份 PEB**，32 位的 `SetCurrentDirectory` 只改 32 位那份 | 先问 `ProcessWow64Information`，非 0 就按 32 位布局读；`a_32_bit_process_whose_cwd_is_inside_is_named` 用 `SysWOW64\cmd.exe` 钉住 |
| 1-6 | 偏移量 | x64：`PEB+0x20` → params，`params+0x38` → `CurrentDirectory.DosPath`（UNICODE_STRING：Length u16 @0，Buffer u64 @8）。x86：`PEB32+0x10` → params，`params+0x24` → `CurrentDirectory`（Length u16 @0，Buffer u32 @4） |
| 1-7 | PEB 里的工作目录带尾反斜杠，偶尔带 `\\?\` | `clean()` 去前缀后走 `engine::path::normalize` |
| 1-8 | 退化的根（空串、`C:\`）会让「在它底下」对所有进程成立 | `usable_root` 要求归一化后仍含 `/`，否则什么都不匹配；`degenerate_roots_match_nothing` 钉住 |
| 1-9 | `C:\wt` 不能匹配兄弟目录 `C:\wt2`、`C:\wt-old`、`C:\wt.bak` | 前后都按「能接在名字里的字符」判边界，命中兄弟后继续往后找 |
| 1-10 | 子目录被当 cwd 时，整目录 rename 报的是 **os error 5**（拒绝访问），不是 32 | E2E 实测。错误码仍是 `worktree_busy`，不影响流程；`file_lock` 的探测本来就把 32/33 都算占用 |

#### 为什么这么改是安全的

- **结束进程只在用户点两下之后发生**，而且后端：
  1. 只认这个仓库**登记在册的 linked worktree**（`require_registered_worktree` 复用删除流程的 `admin_from_registry`），主工作区和任意目录一律拒绝——前端传个 `C:\Users\me` 进来不会把半个系统的进程都算成持有者；
  2. **先拿进程句柄、用这个句柄读到的信息核对它确实占着 worktree、再用同一个句柄结束**。句柄握着，PID 就不会在核对和结束之间被复用；
  3. 核对不过（已退出、换了目录、根本不相干）返回 `holder_not_found`，什么都不做。`a_process_that_does_not_hold_the_root_is_left_alone` 断言给了 PID 也不会结束不相干的进程。
- 读别的进程只用 `ReadProcessMemory` / `NtQueryInformationProcess`，不写；对方地址无效时 API 返回错误而不是崩。读回的命令行 `Buffer` 必须落在本地缓冲区内且按 u16 对齐，否则丢弃。
- 新开的四个 feature（`Win32_System_Threading`、`Win32_System_Diagnostics_Debug`、`Win32_System_Diagnostics_ToolHelp`、`Wdk_System_Threading`）都属于已在依赖树里的 `windows 0.62.2`，`Cargo.lock` 不变，不新增下载。

### Task 2：`file_lock::describe`

- `BlockingProcess` 加了 `via_cwd`、`critical` 两个字段。RM 对某个**目录**答不出进程时，再看谁的工作目录落在里面；全是这种时文案写「工作目录停在这里：…」。
- 读不到 PEB 的（提权进程）仍然报「查不出是哪个进程」，文案补了「可能以管理员身份运行」。

### Task 3：命令

- 两个命令都是 `#[tauri::command(async)]`：要遍历全部进程，可能还要探测目录，不能卡界面线程。
- 参数名 `worktree_path` → JS 侧 `worktreePath`。真实窗口里用 `worktree_path` 调，后端报 `missing required key worktreePath`（反例），证明正例里的 camelCase 真被认了。

### Task 4：前端

| # | 决定 | 理由 |
|---|---|---|
| 4-1 | 面板挂在错误下面，不改后端错误文案的结构，不解析 message | 错误文案已经点名 PID（Task 2）；按钮靠单独的 `worktree_holders` 查，前后端不靠字符串耦合 |
| 4-2 | 结束要点两下（第一下变「确认结束」） | 结束进程会丢掉那个进程里没保存的东西；再弹一层确认框又会打断删除弹框本身 |
| 4-3 | 结束后不自动重试删除 | 可能还有别的占用或别的失败原因；让用户自己点 Delete，结果在框里看得清 |
| 4-4 | 每次撞上 `worktree_busy` 换 key 重建面板 | 重试失败时重新查，不沿用旧列表 |
| 4-5 | `DeleteWorktreeDialog` 新增 `repoPath` 属性 | 后端要拿主仓库核对 worktree 登记；`WorktreeView` 传 `currentPath`，与 `RepoTab.handleDeleteWorktree` 用的 `state.path` 是同一个值 |
| 4-6 | `Holder` 加了 `isSelf` | 界面要分清「本应用自己（关标签页）」和「系统关键进程」，前端拿不到自己的 PID |

### Task 6：真实窗口实测

fixture：scratchpad 下建仓库 + linked worktree，用隐藏的 `cmd /k` 把工作目录停在 worktree 的 `src` 里，复刻孤儿进程。
`pnpm tauri dev --config`（独立 identifier `com.momadacoding.jkk.e2eholders`），CDP 驱动真实窗口，确认 `app.exe` 是本 worktree 的 `target\debug` 产物。

结果：删除 → `worktree_busy`，错误里写着「src —— 工作目录停在这里：cmd.exe (PID …)」→ 面板列出 cmd.exe（工作目录停在 src）和驱动脚本自己的 node.exe（启动参数一档）→ 点两下结束 cmd.exe → 再点 Delete 成功，worktree 进了 `.trash-v2`，`git worktree list` 只剩主工作区。验完删掉了两个配置目录。

#### 6-1 实测抓到的 bug（单测没抓到）

面板一直停在「正在查是哪些进程占着它…」，而直接调 `worktree_holders` 178ms 就返回了正确结果。
原因：防卸载后写 state 的 `alive` ref 只在 effect 的清理函数里写 `false`。dev 构建开着 React StrictMode，effect 会挂载→卸载→再挂载，第二次挂载后它一直是 `false`，查回来的结果全被丢掉。
RTL 的 `render` 默认不包 StrictMode，所以组件测试是绿的——**测试形态和真实形态不一致**。
修复：effect 体里先写 `alive.current = true`。先补了包 `<StrictMode>` 的失败测试，再修。

### Task 7：独立 review 及修复

reviewer 另外用 C# P/Invoke 按同样的偏移独立读了一遍 x64 / WOW64 的 PEB，确认偏移正确，
并实测 WOW64 进程的 64 位 PEB 工作目录恒为 `C:\WINDOWS\`——32 位分支的测试不是虚假信心。

| # | 级别 | 问题 | 处理 |
|---|---|---|---|
| R1 | 应修 | 命令行一档能结束，但界面看不到命令行，只能盲点 | `Holder.commandLine`：命中处附近最多 200 字符；面板等宽小字显示 |
| R2 | 应修 | 后端 `by_cwd \|\| by_cmdline` 短路，RM 标 critical 的进程绕过界面直接 IPC 也能被结束 | 先问 RM，critical 一律拒绝（`holder_critical`），与界面一致 |
| R3 | 应修 | `is_self` 只比自己的 PID。从停在 worktree 里的终端启动本应用时，WebView2 等子进程继承了工作目录，会被列成可结束 | Toolhelp 快照建父子表（`own_tree`），自己和全部后代都标 `isSelf`、不给结束，后端也拒绝；cwd 一档的文案改成「从别处重新启动本应用」 |
| R4 | 应修 | 重复探测：`find_holders` 里 `find_blockers` 又为每个被占目录枚举全部进程；`terminate_holder` 再整树探测一遍 | `find_blockers_with(.., cwd_fallback=false)`；结束时前端回传 `heldPath`，后端只对这一个路径问 RM（要求它落在 worktree 底下） |
| R5 | 小 | 命令行缓冲区范围检查 `start + len` 可能回绕 | `checked_add` |
| R6 | 小 | 查回空列表时面板一片空白 | 提示「多半是以管理员身份运行的程序」 |
| R7 | 小 | 重查失败会盖掉结束失败的原因 | 两种错误分开存 |
| R8 | 小 | 结束进程过程中 Delete 仍可点 | 面板经 `onEndingChange` 上报，两个弹框据此禁用 Delete |
| R9 | 小 | 「确认结束」永不过期 | 5 秒不点复原（`ARM_TIMEOUT_MS`） |
| R10 | 小 | `admin_from_registry` 不认相对路径的 `gitdir`（git 2.48+ `worktree.useRelativePaths`） | **不修**：删除流程本来就有同样的局限，失败方向是拒绝，安全；不属于本 PR 引入 |

#### R3 带出的计划没写的坑

- 测试进程直接起的子进程现在算「本应用自己的」，原来那批 live 测试全都要换成**非后代**进程：
  经 PowerShell 的 `Start-Process` 起，起它的 powershell 随即退出，正好复刻真实案例里的孤儿。
- 带 `-RedirectStandardOutput` 的 `Start-Process` 会让子进程继承 powershell 的句柄，
  测试要是用管道读 PID，就得等那个子进程退出才读得到 EOF（实测卡满 60 秒）。改成 PID 写文件、三个标准流都不接管道。
- `NamedTempFile` 开着文件，`Set-Content` 写不进去；改用 `TempDir` 里的普通路径。
- 「开着文件」那一档的非后代进程：`cmd /k` 没有自己的控制台，起它的 powershell 一退它就退；改用绝对路径的 `PING.EXE -n 60`，标准输出重定向进 worktree 里的文件。

## 遗留

- 读不到提权进程的 PEB：它们的工作目录占用仍只能报「查不出」，要用户自己去任务管理器。
- 8.3 短名 / junction 别名拼出来的工作目录和 worktree 路径对不上时不会被认出（与删除流程里 `spelling_equal` 的局限相同）。
- 命令行一档有误报（只要 argv 里写着这个路径就算）；界面已明说「可能无关」，结束前仍要点两下。

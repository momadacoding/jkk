# 工具栏自定义命令 设计规格

- 日期：2026-08-09
- 状态：已评审通过，待实现
- 相关：`docs/superpowers/specs/2026-08-06-toolbar-and-graph-lanes-design.md`

## 1. 目标

在工具栏右侧提供一组用户可自定义的按钮，用于把「当前打开的 worktree」交给外部程序处理，
形态参考 Fork 客户端右上角功能区。典型用途：

- 在 VS Code / Zed 中打开当前 worktree
- 在当前 worktree 目录打开终端
- 在文件管理器中定位当前 worktree

### 1.1 非目标

明确排除在本期之外，不要顺手实现：

- 应用内的命令管理表单（增删改查 UI）
- 命令可用性探测（检测本机是否装了 VS Code 并隐藏按钮）
- 提交级 / 文件级占位符（`$SHA`、`$FILE`）与对应的右键菜单入口
- 配置文件的自动监听热重载
- 捕获子进程输出并在应用内展示

## 2. 配置

### 2.1 位置与格式

配置文件位于 `app_config_dir()/customCommands.json`。
Windows 下即 `%APPDATA%\com.tauri.dev\customCommands.json`。

```json
{
  "version": 1,
  "commands": [
    { "id": "vscode",   "label": "VS Code",  "program": "code",     "args": ["$WORKTREE"] },
    { "id": "terminal", "label": "Terminal", "program": "wt",       "args": ["-d", "$WORKTREE"] },
    { "id": "explorer", "label": "Explorer", "program": "explorer", "args": ["$WORKTREE"] }
  ]
}
```

字段定义：

| 字段 | 类型 | 必填 | 说明 |
|---|---|---|---|
| `id` | string | 是 | 稳定标识，用作 React key 与 `data-command` |
| `label` | string | 是 | 按钮文字 |
| `program` | string | 是 | 可执行文件名或绝对路径 |
| `args` | string[] | 否，默认 `[]` | 参数数组。**逐项**做占位符替换 |
| `cwd` | string | 否，默认 `"$WORKTREE"` | 子进程工作目录 |

顶层 `version` 目前恒为 `1`，仅供将来迁移使用；解析时不校验其值。

边界情况：

- `commands` 为空数组 → 合法，工具栏只显示 `⚙`
- `id` 重复 → **不报错**，全部保留，React key 用 `${id}-${index}`。
  为一个小瑕疵丢掉整份配置不划算，与 §2.4「坏 JSON 不覆盖」是同一个取向

### 2.2 占位符

| 占位符 | 值 | 来源 |
|---|---|---|
| `$WORKTREE` | 当前打开的 worktree 绝对路径 | `RepoState.path` |
| `$BRANCH` | 当前分支名 | `RepoState.head` |
| `$HEAD` | HEAD 的完整 oid | `RepoState.headHash` |

规则：

- 替换是**整串子串替换**，不是 shell 展开；`"$WORKTREE/src"` 会被正确替换
- 未知占位符（如 `$FOO`）**原样保留**，不报错、不替换成空串
- 路径值本身含 `$` 时不会被二次展开（先替换后不再扫描）
- `$WORKTREE` 为 `null`（未打开仓库）时，整组按钮禁用，不执行替换

### 2.3 各平台默认预设

首次读取时文件不存在，按 `cfg!(target_os)` 写出对应预设：

| 平台 | 编辑器 | 终端 | 文件管理器 |
|---|---|---|---|
| Windows | `code $WORKTREE` | `wt -d $WORKTREE` | `explorer $WORKTREE` |
| macOS | `code $WORKTREE` | `open -a Terminal $WORKTREE` | `open $WORKTREE` |
| Linux | `code $WORKTREE` | `x-terminal-emulator`（cwd=`$WORKTREE`） | `xdg-open $WORKTREE` |

预设中的程序在本机不存在是**预期情况**，见 §5.3 的错误处理。

### 2.4 生命周期

| 环节 | 行为 |
|---|---|
| 读取 | Tauri 命令 `read_custom_commands` 解析 JSON |
| 文件不存在 | 写出当前平台的默认预设，再返回该预设 |
| JSON 语法错误 / 结构不符 | 返回 `GitError`，**绝不覆盖或重写用户文件**；工具栏退化为只剩 `⚙`，错误走顶部横幅 |
| 未知字段 | serde 默认忽略，保持向前兼容 |
| 编辑 | `⚙` 菜单中「Edit commands…」，用系统默认程序打开该 JSON |
| 重新加载 | `⚙` 菜单中「Reload」。不做文件监听，避免与既有 `repo-changed` 监听重入纠缠 |

「JSON 坏了不覆盖」是硬要求：用户手写配置写错一个逗号就丢掉全部自定义命令是不可接受的。

## 3. 架构分层

```text
Toolbar.tsx
    │  渲染按钮、禁用态、⚙ 菜单
    ↓
src/engine/customCommands.ts        纯函数：占位符展开、可用性判定（无 React / 无 Tauri）
    ↓
src/store/repoStore.ts              invoke 出口
    ↓  invoke('run_custom_command', { program, args, cwd })
    │  invoke('read_custom_commands')
    │  invoke('open_in_default_app', { path })
    ↓
src-tauri/src/commands/custom.rs    #[tauri::command] 入口，错误映射到 GitError
    ↓
src-tauri/src/engine/launcher.rs    程序解析 + spawn
src-tauri/src/engine/custom_config.rs  配置读写
    ↓
std::process::Command
```

占位符展开放在**前端纯函数**，与既有 `src/engine/toolbarActions.ts` 同构，可直接用 vitest
覆盖 Windows 反斜杠、含空格、路径带 `$` 等形态。后端不认识占位符，只负责按 argv 启动进程。

### 3.1 Tauri 命令签名

Rust 侧 `snake_case` 参数会被转成 JS 侧 `camelCase`，本设计中三个命令的参数均为单词，
不存在转换差异，但签名改动仍须按 AGENTS.md §7.3 实跑 `pnpm tauri dev` 验证。

```rust
#[tauri::command] fn read_custom_commands(app: AppHandle) -> Result<Vec<CustomCommand>>
#[tauri::command] fn run_custom_command(program: String, args: Vec<String>, cwd: Option<String>) -> Result<()>
#[tauri::command] fn open_custom_commands_config(app: AppHandle) -> Result<()>
```

第三个命令**不接受路径参数**：它自己解析配置文件位置、确保文件存在（缺失时先写默认预设），
再交给系统默认程序打开。不暴露通用的「打开任意路径」命令，前端也就没有能力
让系统去打开一个它自己拼出来的路径。

## 4. 执行层

### 4.1 程序解析（Windows 关键点）

`Command::new("code")` 在 Windows 上必然返回 `NotFound`——`CreateProcess` 查找 PATH 时
只补 `.exe`，而 VS Code 提供的是 `code.cmd`。

```text
resolve_program(program):
    含路径分隔符，或已带扩展名  → 原样返回
    否则                       → 遍历 PATH × PATHEXT，返回第一个存在的文件
    找不到                     → GitError("launcher", "找不到可执行文件 `code`，请检查 customCommands.json")
```

非 Windows 平台不做此解析，直接交给 `Command::new`。

### 4.2 待实测的假设

**Rust 1.77.2 起，std 对 `.bat`/`.cmd` 目标会自动经 `cmd.exe` 启动并做参数转义**
（CVE-2024-24576 / BatBadBut 的修复）。本项目 MSRV 恰为 1.77.2。

这是本设计唯一依赖「记忆中的行为」的地方，按 AGENTS.md §6.2 必须实测：

- 实现的第一步，在临时 fixture 目录里实测
  `Command::new("<绝对路径>\\code.cmd").arg("含 空格&的路径")` 的真实行为
- 若行为与假设不符，退回显式构造 `cmd.exe /c <resolved> <args...>`，
  并把该转义路径的正确性单独固化成测试
- 实测结论与偏离记入 `<计划名>-decisions.md`

### 4.3 spawn 语义

- 用 `.spawn()`，**不 `.wait()`**。等待子进程退出会让点一下 VS Code 就把 UI 卡到编辑器关闭为止
- stdin / stdout / stderr 全部 `Stdio::null()`，不继承句柄
- **不**加 `CREATE_NO_WINDOW`——终端类命令本来就需要窗口
- 全程 argv 数组直传，**不经 shell 解释**。含空格、`&`、中文的仓库路径不会被二次解析成新命令

最后一条是本功能的核心安全防线：`$WORKTREE` 是用户仓库路径，是最容易出事的输入。

### 4.4 用默认程序打开

「Edit commands…」复用同一条 spawn 路径，不需要引入任何新能力：

| 平台 | 命令 |
|---|---|
| Windows | `explorer.exe <file>`（会用默认关联程序打开） |
| macOS | `open <file>` |
| Linux | `xdg-open <file>` |

## 5. UI

### 5.1 布局

```text
[⎇ master] │ Fetch  Pull  Push  Stash ←── flex spacer ──→ VS Code  Terminal  Explorer  ⚙
```

沿用既有 `.toolbar-button` 样式，不新造视觉语言。flex spacer 把自定义命令组推到右侧。

### 5.2 状态

- `state.path == null`（未打开仓库）→ 自定义命令按钮全部禁用。
  **`⚙` 始终可用**——编辑配置不需要先打开仓库
- 配置解析失败 → 只渲染 `⚙`，错误横幅说明原因
- spawn 是毫秒级返回，**不做 busy 态**（与 fetch/push 不同，那些要等网络）

### 5.3 错误呈现

程序找不到时的错误必须能直接指导用户修复，格式：

```text
找不到可执行文件 `code`，请检查 customCommands.json
```

错误经 `GitError` 映射后走顶部错误横幅，与现有 git 操作失败一致。

## 6. 测试

| 层 | 文件 | 覆盖 |
|---|---|---|
| vitest | `tests/vitest/customCommands.test.ts` | 占位符展开：Windows 反斜杠路径、含空格路径、路径本身含 `$`、未知占位符原样保留、`args: []`、`$WORKTREE` 为 `null` 时命令不可用 |
| cargo | `src-tauri/tests/custom_commands.rs` | 配置解析：文件缺失→写默认；坏 JSON→报错**且原文件字节未变**；未知字段忽略；`args` / `cwd` 缺失走默认 |
| cargo | 同上 | launcher：在 `tempfile` fixture 目录里 spawn `git --version` 成功；spawn 不存在的程序→`GitError`；Windows 下 `resolve_program("git")` 命中 `git.exe` |

配置解析逻辑实现为 `load_from(&Path)`，测试通过 `tempfile` 注入路径，
**绝不触碰真实的 app config dir**，符合 AGENTS.md §8。

launcher 测试选 `git --version` 作被测程序：它一定存在（整个项目依赖它），
执行无副作用，且不产生窗口。

## 7. 风险与取舍

1. **按设计执行任意本地程序。** 这就是功能本身，风险等同用户自己在终端敲命令。
   两条防线：不经 shell 解释；配置只来自用户自己写的本地文件。
2. **默认预设中的程序可能未安装。** 不做可用性探测（YAGNI），点击后报错并指明改哪个文件。
3. **`explorer.exe` 退出码常年非 0。** 因为不 `wait`，不受影响。
4. **配置文件位于 app config dir，同机其他程序可写。** 不额外加固，与桌面应用惯例一致。
5. **不做热重载。** 用户改完 JSON 需手动点 Reload；换取的是不与既有文件监听逻辑纠缠。

## 8. 实现约束

- 不引入任何新的 npm 包或 crate（评审时已排除 `tauri-plugin-shell` 与 `tauri-plugin-opener`）
  - `tauri-plugin-shell` 的 scope 是编译期白名单，与「运行时用户自定义任意命令」语义冲突
  - `tauri-plugin-opener` 无法带参数启动指定程序，覆盖不了编辑器与终端场景
- 实现必须在独立 worktree 中进行（AGENTS.md §4.1）
- 因涉及多个 Task 跨轮，必须建 `<计划文件名>-decisions.md`（AGENTS.md §2.1）

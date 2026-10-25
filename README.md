# mygitclient

一个面向个人使用的 Git GUI 客户端，目标是把 Fork 的好用 worktree 支持、Sublime Merge 级别的 commit graph，以及编辑器/终端整合放到一起。当前 MVP 聚焦在「Graph 专家版」：高质量提交图 + staging/diff/commit + 分支/worktree 管理。

## 技术栈

- **Tauri 2** + **Rust**：跨平台桌面外壳与后端命令
- **React 18** + **TypeScript**：前端 UI
- **libgit2（git2-rs）**：高性能读取（revwalk、diff、refs）
- **git CLI**：写入操作（commit、branch、checkout、stage、worktree 等），保证与原生 git 行为一致
- **SVG**：commit graph 渲染（可替换为 Canvas）
- **notify**：文件系统监听，自动刷新

## 环境要求

- [Rust](https://rustup.rs/) >= 1.77.2
- [Node.js](https://nodejs.org/) + [pnpm](https://pnpm.io/)
- Windows / macOS / Linux 均可（当前主要在 Windows 11 开发）
- 本地已安装 `git`

## 目录结构

```
.
├── .github/workflows/
│   └── release.yml   # 打 v* tag 触发构建并发 Release（不是 PR 门禁）
├── docs/superpowers/
│   ├── specs/    # 设计规格文档
│   ├── plans/    # 实现计划
│   └── e2e/      # E2E 验证清单
├── scripts/          # 一次性/维护脚本（图标生成、CI 签名等）
├── src/              # React 前端
│   ├── components/   # UI 组件
│   ├── engine/       # 提交图布局/路径/SVG 渲染
│   ├── store/        # 全局状态与副作用
│   └── App.tsx
├── src-tauri/        # Rust 后端
│   ├── src/
│   │   ├── commands/ # invoke 命令
│   │   ├── engine/   # git 读取引擎
│   │   └── main.rs
│   └── tests/        # Rust fixtures 测试
└── tests/vitest/     # 前端单元测试
```

## 安装依赖

```bash
pnpm install
```

Rust 依赖在首次 `cargo build`/`pnpm tauri dev` 时自动拉取。

## 开发启动

```bash
pnpm tauri dev
```

这会同时启动：
- Vite 开发服务器（默认 `http://localhost:5174`）
- Tauri 桌面窗口
- Rust 后端热重载

> 如果端口 5174 被占用，先杀掉占用进程再启动。

## 构建

```bash
pnpm tauri build
```

产物在 `src-tauri/target/release/bundle/`。

## 发布

打 `v*` tag 会触发 `.github/workflows/release.yml`：在 `windows-latest` 上跑一遍
测试与 lint 门槛、`pnpm tauri build`，然后把安装包传上去建一个 **draft** Release，
由人过目后再点发布。

```bash
# tag 必须打在 master 上。在功能分支或 worktree 里打 tag 同样能构建成功，
# 发出去的却是没经过 review 的代码，而且从安装包上完全看不出来。
git checkout master
git pull --ff-only

# 版本号直接在 master 上改，不走 PR（范围见 AGENTS.md「发版提交例外」）。
# 四处必须一致：package.json / src-tauri/tauri.conf.json / src-tauri/Cargo.toml，
# 以及 src-tauri/Cargo.lock 里 app 那一条
node scripts/gen-notices.mjs --check   # 依赖变过就去掉 --check 重新生成，一起提交
git commit -am "chore(release): 版本号升到 0.1.0"
git push origin master

git tag v0.1.0
git push origin v0.1.0
```

Release 说明开头的「新功能 / 修复」由 `scripts/release-notes.sh` 自动生成：取上一个正式版
tag 以来的提交标题，`feat` 进新功能、`fix` 进修复，其余类型不列。所以提交标题就是更新日志，
要写成用户看得懂的话。打 tag 之前可以先预览（需要 `gh` 已登录，比对的是 origin 上的 master）：

```bash
# 第一个参数填这次真正要发的版本号：脚本按它找「上一个正式版」当比对起点，
# 填小了起点就跟着错（填 v0.1.0 这种最早的号则什么也不输出）
bash scripts/release-notes.sh v0.7.0 master
```

workflow 会在构建前自行校验三件事，任何一项不过都在几秒内失败：

| 校验 | 拦住什么 |
|---|---|
| tag 名是严格 SemVer | 畸形 ref（含引号、分号的 tag 名是合法的 git ref） |
| tag 版本 == 三个清单文件 | 漏改版本号导致 `v0.2.0` 的 Release 里装着 `0.1.0` 的包 |
| tag 指向的提交已在 master 上 | 在功能分支上误打 tag，发出未经 review 的代码 |
| 第三方许可清单与锁文件一致 | 依赖变动后忘了重新生成 `THIRD_PARTY_NOTICES.md` |
| 目标 Release 不是「已发布」状态 | 重跑 workflow 覆盖掉已经公开的安装包 |
| `git describe` 恰好等于 tag | 浅克隆没取到 tag，正式版窗口标题带着 hash 发出去 |

同名 Release 若还是 draft（上次传到一半），重跑会复用它并覆盖资产；
**若已经发布则直接失败** —— 同一版本号对应两份不同的二进制会让审计和回滚失去依据。
要重发得先手动删掉那个 Release，或换新版本号。

产物含 `SHA256SUMS.txt`，可用来核对下载到的是不是这次构建的东西。

真打 tag 之前可以先在 Actions 页面手动触发一次（`workflow_dispatch`）：
只构建、不发 Release，用来确认 CI 环境编得出来。

> 这个 workflow **不是 PR 门禁**——`push` 和 `pull_request` 都不会触发它。
> 日常改动的验证仍然全靠本地手工执行，见 `AGENTS.md` §5。

### 从窗口标题认出是哪个构建

窗口标题由 `src-tauri/build.rs` 在编译期执行
`git describe --tags --always --match 'v[0-9]*'` 得出，同时开着几个 jkk 时靠它区分：

| 标题 | 含义 |
|---|---|
| `jkk 0.5.0` | release 构建，且正好在 `v0.5.0` 这个 tag 上（正式版） |
| `jkk 0.5.0 (v0.5.0-3-g610fdb0)` | release 构建，在 `v0.5.0` 之后第 3 个提交 `610fdb0` |
| `jkk 0.5.0 (610fdb0)` | 仓库里没有任何 `v*` tag（如 `workflow_dispatch` 的浅克隆） |
| `jkk 0.5.0 (v0.5.0-3-g610fdb0) [dev · <目录名>]` | `pnpm tauri dev` 等 debug 构建，带上所在 checkout / worktree 的目录名 |
| `jkk 0.5.0 (未知提交)` | 编译时取不到 git 信息（没装 git / 不在仓库里） |

几点限制：

- 标题**不带** `-dirty`：build.rs 只在 HEAD、当前分支、tag 变化时重跑，盯不住工作区改动，
  标出来反而会过时。所以在 tag 上带着未提交改动本地 `pnpm tauri build`，标题同样是
  `jkk 0.5.0`——正式版只认 Release 页面上的安装包。
- 同一个 Release 的安装版和绿色版是同一个二进制，标题相同。
- release.yml 设了 `JKK_REQUIRE_GIT_DESCRIBE=1`，CI 上取不到 describe 会直接构建失败。

### 绿色版（免安装）

每个 Release 另附 `jkk_<版本>_x64-portable.zip`：解压即用，不走安装流程。
两点注意：

- 未签名时首次运行同样会弹 SmartScreen（见下节）；
- 绿色版不会像安装包那样自动补装 WebView2 运行时——Win11 和保持更新的
  Win10 都自带；干净的旧系统双击无反应的话，装一下
  [WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/)。

### 代码签名：流水线已就位，证书未采购

流水线支持「有证书就签」：secrets 配好 `SIGNING_CERT_PFX_BASE64` 和
`SIGNING_CERT_PASSWORD` 后，构建会自动用 signtool 给 exe 和安装包签名
（脚本：`scripts/sign-windows.ps1`）；没配就发未签名包。

签名之前，Windows 会对安装器和绿色版弹 SmartScreen「未知发布者」警告，
点「更多信息」→「仍要运行」即可。

### 应用图标

`src-tauri/icons/` 下的整套图标由 `src-tauri/icons/source.png` 派生，**不要手工改**：

```bash
pwsh -File scripts/make-icon.ps1        # 重新生成 1024x1024 源图
pnpm tauri icon src-tauri/icons/source.png   # 派生全套平台图标
```

`tauri icon` 会顺带生成 iOS / Android 图标，本项目只做桌面端，那两个目录已在
`.gitignore` 里。

## 测试

前端单元测试：

```bash
pnpm test
```

Rust 测试：

```bash
cd src-tauri
cargo test
```

类型检查：

```bash
pnpm exec tsc --noEmit
cd src-tauri && cargo check
```

## MVP 功能范围

- [x] 打开本地仓库，读取 HEAD
- [x] 会话还原：重启后自动打开上次的标签，空态页列出最近打开过的仓库
- [x] Sublime Merge 风格的 commit graph（SVG 渲染、viewport 裁剪）
- [x] 点击提交查看 diff
- [x] Staging / unstaging / commit / amend
- [x] 分支查看、checkout、创建、删除
- [x] Worktree 查看、切换、创建、删除
- [x] 文件系统监听自动刷新
- [x] merge / rebase / cherry-pick 状态横幅
- [x] 大仓库提交数上限保护（5000 commits）
- [x] 工具栏自定义命令（见下节）

## 工具栏自定义命令

工具栏右侧可以配置任意本地命令，用来把当前 worktree 交给外部程序，
比如在编辑器里打开、在该目录起一个终端、或者在文件管理器里定位。

配置文件位置：

| 平台 | 路径 |
|---|---|
| Windows | `%APPDATA%\com.momadacoding.jkk\customCommands.json` |
| macOS | `~/Library/Application Support/com.momadacoding.jkk/customCommands.json` |
| Linux | `~/.config/com.momadacoding.jkk/customCommands.json` |

首次启动会写出一份当前平台的默认预设。点工具栏最右侧的 `⚙` →
**Edit commands…** 用系统默认程序打开它，改完点 **Reload** 生效（不会自动重载）。

```json
{
  "version": 1,
  "commands": [
    { "id": "vscode", "label": "VS Code", "program": "code", "args": ["$WORKTREE"] }
  ]
}
```

| 字段 | 必填 | 说明 |
|---|---|---|
| `id` | 是 | 稳定标识 |
| `label` | 是 | 按钮上的文字 |
| `program` | 是 | 可执行文件名或绝对路径 |
| `args` | 否 | 参数数组，默认 `[]`。逐项做占位符替换 |
| `cwd` | 否 | 子进程工作目录，默认 `"$WORKTREE"` |

可用占位符：`$WORKTREE`（当前 worktree 绝对路径）、`$BRANCH`、`$HEAD`（完整 oid）。
未知占位符原样保留。

### `args` 必须逐项拆开

参数以数组形式直接传给系统，**不经过 shell 解释**。因此 `args` 的**每个元素就是一个
命令行参数**，不能把一整条命令行塞进一个字符串——那会被目标程序当成**单个**参数。

这是最容易踩的坑，且失败时界面**不会有任何提示**（原因见下）。

❌ 错误——整串塞进一个元素：

```json
"args": ["-NoProfile -Command \"Write-Output hi\""]
```

PowerShell 收到的是一个参数，于是把整串当成脚本文件名去找：

```
The argument '-NoProfile -Command "Write-Output hi"' is not recognized as
the name of a script file.
```

✅ 正确——逐项拆开：

```json
"args": ["-NoProfile", "-Command", "Write-Output hi"]
```

带空格的**单个**参数仍然写成一个元素，不需要加引号——不经过 shell，也就不存在
被空格拆开的问题：

```json
"args": ["--folder-uri", "$WORKTREE", "--goto", "src/main.rs:42"]
```

### 已知限制

- **写不了复合命令。** `cd x && y`、管道、重定向都依赖 shell，这里没有 shell。
  需要的话把它包成一个 `.ps1` / `.cmd` / `.sh` 脚本文件再调用那个文件。
- **程序启动之后的失败是静默的。** 本功能只负责把进程拉起来就返回，不等它退出
  （否则点一下编辑器就会把界面卡到编辑器关闭为止），子进程的 stdout / stderr 也被丢弃。
  所以「可执行文件找不到」会报错，但「程序起来了、自己参数没解析对」**不会有任何提示**，
  表现就是点了按钮什么都没发生。遇到这种情况，先把同样的 `program` + `args`
  在终端里手动跑一遍，看真实报错。
- **JSON 写错时程序不会覆盖你的文件**，只会在 `⚙` 上显示 `⚠` 并把解析错误显示出来。

## 外部 diff / merge 工具

右键文件可以交给外部工具对比或解决冲突，默认 Beyond Compare。设计见
`docs/superpowers/specs/2026-10-17-external-diff-merge-tools-design.md`。

- **对比**：「本地改动」的 Unstaged / Staged 文件、提交详情（Changes 标签页，含 stash 与 SVN 节点）
  的文件，右键「在 X 中对比」。Unstaged 是暂存区对比工作区，Staged 是 HEAD 对比暂存区
  （同 `git difftool` / `git difftool --cached`）；应用内右侧显示的是 HEAD 对比工作区，两者有意不同。
- **解决冲突**：冲突文件右键「在 X 中解决」，或选中后点右侧提示里的按钮。工具拿到的是三个原始版本
  （`$LOCAL` / `$REMOTE` / `$BASE` 为临时文件），结果写进工作区的真实文件（`$MERGED`）。
  **应用不会在工具关闭后自动暂存**，回到应用里看过结果再手动 Stage。例外是 VS Code：它的合并编辑器点
  「Complete Merge」时会自己 `git add` 结果文件并关闭编辑器（VS Code 的行为，不经过应用），回到应用时文件已在 Staged。
- **一边删一边改（DU / UD）**：在右侧选「保留修改后的文件」或「删除文件」，立即暂存；删除前会确认。
- 临时文件放在 `%TEMP%\jkk-external\` 下每次一个子目录，工具退出后删除。同一个文件已经在工具里
  打开时，再点会提示「已在 X 中打开」。

配置文件位置（首次使用时写出默认预设，右键菜单最后一项「编辑外部工具配置…」打开它，
切回窗口时自动重读）：

| 平台 | 路径 |
|---|---|
| Windows | `%APPDATA%\com.momadacoding.jkk\externalTools.json` |
| macOS | `~/Library/Application Support/com.momadacoding.jkk/externalTools.json` |
| Linux | `~/.config/com.momadacoding.jkk/externalTools.json` |

默认预设（Beyond Compare 5 与 VS Code）：

```json
{
  "version": 1,
  "defaultDiff": "bc",
  "defaultMerge": "bc",
  "rules": [],
  "tools": [
    {
      "id": "bc",
      "label": "Beyond Compare",
      "program": "C:\\Program Files\\Beyond Compare 5\\BComp.exe",
      "diffArgs": ["$LOCAL", "$REMOTE", "/lefttitle=$LOCAL_TITLE", "/righttitle=$REMOTE_TITLE"],
      "mergeArgs": ["$LOCAL", "$REMOTE", "$BASE", "$MERGED",
                    "/lefttitle=$LOCAL_TITLE", "/righttitle=$REMOTE_TITLE",
                    "/centertitle=$BASE_TITLE", "/outputtitle=$MERGED_TITLE"],
      "successExitCodes": [0, 1, 2, 11, 12, 13, 14]
    },
    {
      "id": "vscode",
      "label": "VS Code",
      "program": "code",
      "diffArgs": ["--wait", "--diff", "$LOCAL", "$REMOTE"],
      "mergeArgs": ["--wait", "--merge", "$REMOTE", "$LOCAL", "$BASE", "$MERGED"]
    }
  ]
}
```

（实际预设里 BC 还带 `exitCodeMessages`：BComp.exe 用退出码报比较结果，1–14 都是正常结束，
100 起才是故障。）

| 字段 | 说明 |
|---|---|
| `program` | 可执行文件绝对路径或 PATH 上的名字。BC 不在 PATH 上，预设写死安装路径，不探测 |
| `diffArgs` / `mergeArgs` | 参数数组，逐项做占位符替换，不经过 shell。没写的那一项，这个工具就不出现在对应菜单里 |
| `successExitCodes` | 哪些退出码算正常结束，缺省只认 0。其他退出码会在顶部报错，但**从不用来判断冲突是否已解决** |
| `exitCodeMessages` | 已知故障码的说明，键是十进制退出码 |

占位符：`$LOCAL` `$REMOTE` `$BASE` `$MERGED`，以及对应的 `$LOCAL_TITLE` 等标题
（merge 时写真实名字：当前分支 / 被合并的分支；rebase 时写「当前重放结果」和 `REBASE_HEAD`）。

### 按文件类型选工具

`rules` 从上往下匹配，第一条命中的生效；模式只看文件名、不区分大小写，只支持 `*` 和 `?`。
没写的那一项用 `defaultDiff` / `defaultMerge`。规则引用了不存在的工具时报配置错误，不回退。

Unity 场景用 UnityYAMLMerge 解冲突、看 diff 仍用 BC 的写法（路径按本机 Unity 版本改）：

```json
"rules": [
  { "patterns": ["*.unity", "*.prefab", "*.asset"], "merge": "unityyamlmerge" }
],
"tools": [
  { "id": "unityyamlmerge", "label": "UnityYAMLMerge",
    "program": "C:\\Program Files\\Unity\\Hub\\Editor\\6000.6.6f1\\Editor\\Data\\Tools\\UnityYAMLMerge.exe",
    "mergeArgs": ["merge", "-p", "$BASE", "$REMOTE", "$LOCAL", "$MERGED"] }
]
```

手动解决时**不带** `--fallback none`：合不了的部分会按 Unity 自带的 `mergespecfile.txt` 交给后备工具
（默认列了 Beyond Compare 5）。

### 冲突文件右侧显示什么

| 显示 | 含义 | 操作 |
|---|---|---|
| 这个文件有冲突 | 工作区里有冲突标记 | 在 X 中解决 |
| 未检测到冲突标记，检查后可以暂存 | 下面是 HEAD 对比工作区的 diff | Stage |
| git 没有逐行合并这个文件… | `merge=binary`、`-merge`、`merge.default`、LFS | 在 X 中解决、Stage |
| 这个文件配置了自动合并工具 `X`，但仍处于冲突状态… | 自定义 merge driver 没能合完 | 在 X 中解决、Stage |
| 二进制文件冲突 | 任意一个版本是二进制 | Stage |
| 一边删一边改 | DU / UD | 保留 / 删除（立即暂存） |
| 结构冲突 | 改名冲突、符号链接、子模块等 | 无，请在命令行里处理 |

任何情况下都不显示「已解决」：index 里仍是冲突，只有手动 Stage 才算解决。

### merge driver 配置检查

`.gitattributes` 跟着仓库走，`merge=<名字>` 对应的 driver 却要在每台机器自己的 git 配置里定义。
打开仓库时会检查，有问题就在顶部挂横幅：

| 本机状态 | 横幅 | 合并时会怎样 |
|---|---|---|
| 完全没配这个名字 | 红色，附配置示例 | git 退回逐行文本合并，往场景文件里写冲突标记 |
| 只配了 `name` 等、没有 `driver` | 红色 | 整个合并直接 fatal，什么都不改 |
| `driver` 指向的程序（绝对路径）不存在 | 琥珀色 | driver 失败，冲突需要手动解决 |

「完全没配」时，在应用里点 Pull、Cherry-pick、Revert、Stash Apply / Pop、SVN 更新 / 交付（含继续）
之前会弹确认框，默认按钮是取消。在命令行或其他客户端里发起的合并拦不住。应用不会改你的 git 配置。
`merge=lfs`（`git lfs track` 写的标准属性）不检查。

作为**自动 driver** 的配置示例（必须带 `--fallback none`，否则 git 在后台调用时会弹出后备工具窗口卡住合并）：

```text
# .gitattributes（提交进仓库）
*.unity  merge=unityyamlmerge
*.prefab merge=unityyamlmerge
*.asset  merge=unityyamlmerge

# 全局 ~/.gitconfig（每台机器各配一次）
[merge "unityyamlmerge"]
    name = Unity SmartMerge
    driver = 'C:/Program Files/Unity/Hub/Editor/6000.6.6f1/Editor/Data/Tools/UnityYAMLMerge.exe' merge -h -p --force --fallback none %O %B %A %A
```

## 设计文档

- 设计规格：`docs/superpowers/specs/2026-08-04-mygitclient-design.md`
- 实现计划：
  - `docs/superpowers/plans/2026-08-04-mygitclient-engine-and-skeleton-plan.md`
  - `docs/superpowers/plans/2026-08-04-mygitclient-graph-rendering-plan.md`
  - `docs/superpowers/plans/2026-08-04-mygitclient-ui-closure-plan.md`
- E2E 验证清单：`docs/superpowers/e2e/2026-08-04-mygitclient-e2e-checklist.md`
- 工具栏自定义命令：`docs/superpowers/specs/2026-08-09-custom-toolbar-commands-design.md`、
  `docs/superpowers/plans/2026-08-09-custom-toolbar-commands.md`、
  `docs/superpowers/e2e/2026-08-09-custom-toolbar-commands.md`

## 会话还原

关闭应用时会记下当前打开的标签和活动标签，下次启动原样还原。一个标签都没有时
（首次启动、或把标签全关掉了）显示空态页，上面有 **Open Repository…** 和最近打开列表。

会话存在 `session.json`，和 `customCommands.json` 同一个目录（见上表）。这份文件由程序
自己维护，**不需要手动编辑**；删掉它等于清空「上次会话 + 最近列表」，不影响任何仓库。

| 行为 | 说明 |
|---|---|
| 标签还原 | 已经不是 git 仓库的路径会被跳过，不会还原出一个必然报错的标签 |
| 最近列表 | 最多 10 条，最新在前。目录暂时不在（移动硬盘、网络盘没连上）只置灰，不删除 |
| 路径去重 | 反斜杠/正斜杠、大小写、尾部斜杠视为同一个仓库 |

## 注意事项

- 写入操作均通过 git CLI 执行，因此要求工作目录在有效的 git 仓库内。
- 自动刷新监听整个工作区，但只响应 `.git/` 下的变更事件。

## 许可

专有软件，保留所有权利。详见 [LICENSE](LICENSE)。

第三方组件的完整清单与许可正文见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)，
由锁文件生成，随安装包一起分发（MIT / Apache-2.0 都要求分发物保留这些声明）：

```bash
node scripts/gen-notices.mjs          # 依赖变动后重新生成
node scripts/gen-notices.mjs --check  # CI 用：检查有没有漂移
```

其中 libgit2 是 GPL-2.0 **带链接例外**，该例外正是本项目得以按专有条款分发二进制的依据。
注意 `git2` / `libgit2-sys` 这两个 crate 自己声明的是 `MIT OR Apache-2.0`，
那只覆盖 Rust binding，不覆盖它 vendor 进来的 C 代码。

# SVN + Git 混合模式 设计规格

- 日期：2026-09-12
- 状态：设计已确认，未实现。已过三轮对抗性 review + 一次 `git svn` 方案复核，结论全部修入，见 §18
- 分期：见 §16。本文覆盖全部 5 期的架构，各期另出实现计划

## 1. 背景与目标

公司的 Unity 游戏项目用 SVN 管理，同一个仓库里既有代码也有几十 GB 美术资产。
SVN 对美术、策划职能足够好用，不可能整体迁移到 git；但对程序来说缺了太多东西：
没有本地暂存区、没有廉价分支、没有 stash、无法把一堆改动拆成几次干净的提交。

本项目要做的是：**让程序同学在不改变团队 SVN 工作流的前提下，本地获得完整的
git 体验。**

### 1.1 核心命题

方案的全部基础是这一条：

> **工作目录同时是一个 SVN 工作副本和一个 git 仓库，两者互不知晓，只通过磁盘上
> 真实的文件内容交汇。**

git 管白名单内的代码文件，负责暂存、分支、stash、本地提交；SVN 管整个工作副本，
只在「更新」和「交付」两个时刻出场。

在 SVN 眼里，你永远只是「一个停在 r12345、有一批本地修改的工作副本」——它看不到
你的 git commit、分支和 stash。

### 1.2 非目标

明确排除，不要顺手实现：

- 给美术、策划使用。本客户端的 SVN 模式只服务程序职能
- SVN 分支管理（`svn switch`、`svn merge`、分支创建）
- SVN 文件锁（`svn lock` / `unlock`）、externals
- git 历史与 SVN revision 的双向映射（git-svn 语义）。**不用 `git svn` 本身的完整论证见 §2.2**
- 把本地 git 历史共享给同事（不需要 git remote）
- 在 SVN 模式下提供 worktree、tag、PR、cherry-pick、revert
- `svn move`：git 的 rename 一律拆成 delete + add，见 §15

## 2. 已确认的决策

这些是设计的输入，不是本文推导出来的结论。改动其中任何一条都需要重新评估架构。

| # | 决策 | 影响 |
|---|---|---|
| D1 | 只给程序 / 代码同学用 | 砍掉文件锁、externals、大资产工作流 |
| D2 | 一份完整工作目录，代码 + 美术都在，可能几十 GB | git 只能覆盖代码子集。⚠️ 这条**不构成**否决 git-svn 的理由，真正的死因见 §2.2 |
| D3 | 本地 git 历史是脚手架，用完即弃，最终 squash 成一次 `svn commit` | 不需要 revision 映射；同步模型大幅简化 |
| D4 | 只需本地多任务并行 + 实验性改动 | 砍掉全部 SVN 分支功能 |
| D5 | 主视图是混合图：SVN 线性历史 + 本地 git 分支 | 见 §10 |
| D6 | 白名单手写配置，纯白名单语义 | 见 §6.3 |
| D7 | 白名单**不含** `.prefab` / `.unity` / ScriptableObject | 避免 git 行合并产出语义损坏的 YAML |
| D8 | 采用 Overlay：`.git` 直接放在 SVN 工作副本根 | 见 §2.1 |
| D9 | 交付时硬性要求工作区干净，不自动 stash | 见 §8.1 |
| D10 | 基线用普通分支 `refs/heads/svn/base`（IDE 可见） | 见 §6.4 |
| D11 | 长期本地修改（原 `ignore-on-commit` 的用途）用带 `Local-Only` trailer 的 commit 表达，不用文件级清单 | 见 §6.7、§8.2.3 |

### 2.1 为什么是 Overlay 而不是 Sidecar

备选方案是把 gitdir 放在 app data、用 `--work-tree` 指向工作副本（Sidecar），
好处是对 SVN 工作副本零侵入。落选原因：

1. Overlay 让 Rider / VS 的 git 集成**白送**可用——这是程序同学的高频价值，
   而 Sidecar 下 IDE 完全看不见这个仓库
2. Sidecar 规避的风险（误把 `.git` 提交上去）是本机、本人的手滑，可以用 §12.3
   的三层防护覆盖；而它牺牲的是每天都会用到的能力
3. Overlay 下 `run_git(path, args)` 原样可用，现有 40 多个命令零改造

被否决的第三方案是「不用 git，用 SVN 自己的 changelist + shelve 实现暂存区」：
changelist 无法让两份不同的文件内容同时存在，做不到分支，与 D4 直接冲突。
（`ignore-on-commit` changelist 的**那个具体用途**——长期保留不打算交付的本地修改——
是真需求，由 D11 单独承接，见 §6.7。）

### 2.2 为什么不是 `git svn`

git 自带 `git svn`，它能直接读写 SVN 仓库。本设计不用它。这一节把理由写透，因为
**原来 D2 给的理由是错的**，而错的理由必然会被反复重新提出。

#### 原来的理由站不住

D2 曾写「几十 GB → 判死 `git svn clone`」。但 `--include-paths=<regex>` /
`--ignore-paths=<regex>` 是 `git svn` 的正式选项，对 fetch / clone / rebase / dcommit
全程生效（配置键 `svn-remote.<name>.include-paths`）：

```gitconfig
[svn-remote "svn"]
    include-paths = ^Assets/Scripts
```

美术资产本来就永远不会进 git 对象库。**体积拦不住 `git svn`。**

#### 真正的死因：`git svn` 没有工作副本

`git svn` 通过 SVN 的 RA 层直连服务器，`git svn clone` 出来的目录里**没有 `.svn`**。

而本设计的物理前提是「一个 Unity 能打开的目录，代码 + 美术都在」（D2），美术只能来自
SVN 工作副本。于是同一批代码文件会被 `.svn` 和 `git svn` 同时认领，各有一套「磁盘现在
是哪个 revision」的记账；而为了更新美术必跑的 `svn update` 会在 `git svn` 背后改写代码
文件，`git svn` 对此一无所知。

唯一的出路是把代码目录从 SVN 工作副本里摘掉——`svn update --set-depth=exclude <代码目录>`
——让 svn 只管美术、`git svn` 只管代码。这条路在**结构上**能走通，但有一个致命细节：

> **depth-exclude 的粒度是目录，而 D6 的白名单语义是「路径 AND 扩展名」。**
>
> `Assets/Scripts/` 底下那个别人扔进来的 `.psd`：被 depth-exclude 移出工作副本，
> 又被 `include-paths` 的正则挡在 git 外——**它会直接从磁盘上消失**，两边都不认领。
> 要避免就只能放弃扩展名过滤（`include-paths` 只有路径正则，没有扩展名维度），
> 于是 D6 专门要防的「误吸一个 500MB 文件进对象库」原样回来。

此外还丢掉两样东西：TortoiseSVN 对**代码**的兜底能力（§16 Phase 1 / Phase 2 的自洽前提
就是「用户继续用 TortoiseSVN 更新和交付」），以及新增代码目录时 exclude 网会漏的问题。

#### 实测：`git svn` 会打掉现有的测试策略

2026-09-13 本机实测（临时 fixture，跑完即删）：

| 观察 | 结果 |
|---|---|
| `git svn --version` | `2.51.2.windows.1 (svn 1.14.2)`，命令存在 |
| `git svn clone file:///C:/…` | ❌ `Unable to open repository`（`Git/SVN.pm line 148`） |
| 换 4 种 URL 写法（`file://localhost/`、小写盘符、尾斜杠、标准三斜杠） | ❌ 全部失败 |
| 换 fsfs format 6 / 7 / 8（`svnadmin create --compatible-version 1.8/1.9/1.10`） | ❌ 全部失败，**不是仓库格式问题** |
| 同一批 URL 用 SlikSvn 的 `svn info` | ✅ 全部成功 |
| 本机 `svnserve` | ❌ 不存在（SlikSvn 只装 `svn` + `svnadmin`） |

即 **Git for Windows 自带的 `git svn` 打不开 `file://` 仓库**。含义是致命的：§14.1 的
fixture 策略（`svnadmin create` + `file://` + 两个工作副本）对任何以 `git svn` 为核心的
方案**当场失效**，必须另起一个真实 SVN 服务端（svnserve 或 Apache）才能跑自动化测试，
而 `AGENTS.md` §8 把「git 写操作测试必须自建临时 fixture」定成硬约束。这笔成本是引入
`git svn` 最贵的一项。

**未验证**：公司服务器走 https（ra_serf），本机没有服务器可测，`git svn` 在 https 下的
可用性与性能**没有实测依据**。不要把上表的结论外推成「`git svn` 完全不能用」——上表只
证明了 `file://` 这一条路不通。

#### 但 `git svn` 确实结构性地消灭了两个失败模式

这点必须如实承认，不能因为最终没选它就略过：

| 本设计的失败模式 | `git svn` 下为什么不存在 |
|---|---|
| §8.3 空 target 的 `svn commit` 提交整个工作副本 | `git svn dcommit` 不吃 pathspec，它从 git 对象算 delta 经 RA 提交，没有「目标列表」这个概念 |
| §8.2.5 D3.8→D5 之间磁盘字节被外部 IDE 改掉 | 同上：dcommit 提交的是 git commit 的内容，不是磁盘当时的字节 |

这两条是本设计后果最重的两个风险，在本方案里只能靠断言（§8.3、§12.2、D4.9）挡住，
而不是结构上不可能发生。**这是选择 Overlay 路线所付的代价，写在这里以免将来误以为
两边等价。**

#### 从 `git svn` 借来的东西

不引入这个工具，但拿走它的两条设计性质：

1. **基线必须能被服务器证伪。** `git svn` 的基线是从服务器 fetch 下来的，天然不会
   撒谎；本设计的基线由 `git add -A` 收磁盘而来，磁盘不能自证等于服务器——这正是
   §6.5.1 / §9.2 / §16 那一整族缺陷的共同根因。对策是 §6.8 的基线体检
2. **记账用 `git svn` 的原生 trailer 格式**（§6.4），白送 UUID 记账与工具互操作

## 3. 总体架构

```text
D:\Game\trunk\                    ← SVN 工作副本根，同时是 git 仓库根
├── .svn\                         ← SVN 元数据（git 完全不碰）
├── .git\                         ← git 元数据
│   ├── info\exclude              ← 由白名单生成的忽略规则
│   ├── mygitclient.json          ← 白名单配置（手写）
│   ├── mygitclient-svnlog.json   ← SVN 历史缓存
│   └── mygitclient-sync.json     ← 同步中断状态（流程完成后删除）
├── Assets\
│   ├── Scripts\Player.cs         ← svn 和 git 都管
│   └── Art\boss.fbx              ← 只有 svn 管
└── ProjectSettings\
```

工作目录里**只多出 `.git` 一个条目**：白名单配置和生成的忽略规则都放在 `.git/`
内部，不用工作目录里的 `.gitignore`。

## 4. 抽象层

### 4.1 trait Vcs

```rust
// src-tauri/src/vcs/mod.rs
pub trait Vcs {
    // ── 两个后端都实现；SvnGitBackend 全部直接委托给内部的 GitBackend ──
    fn open(&self, path: &str) -> Result<RepoInfo>;
    fn status(&self, path: &str) -> Result<StatusData>;
    fn diff(&self, path: &str, scope: &str) -> Result<DiffData>;
    fn stage(&self, path: &str, paths: &[String]) -> Result<()>;
    fn unstage(&self, path: &str, paths: &[String]) -> Result<()>;
    fn discard(&self, path: &str, paths: &[String], staged: bool) -> Result<()>;
    fn commit(&self, path: &str, msg: &str, amend: bool) -> Result<()>;
    fn branches(&self) -> &dyn BranchOps;
    fn stashes(&self) -> &dyn StashOps;

    // ── 两侧语义真正不同的 5 个 ──
    fn graph(&self, path: &str) -> Result<GraphData>;
    fn sync_in(&self, path: &str) -> Result<SyncReport>;
    fn sync_out(&self, path: &str, msg: &str) -> Result<SyncReport>;
    fn capabilities(&self) -> Capabilities;
    fn external_changes(&self, path: &str) -> Result<Vec<ExternalChange>>;
}
```

```text
              ┌─ GitBackend ──── 现有 engine/* 搬家，行为零变化
   trait Vcs ─┤
              └─ SvnGitBackend ─┬─ git: GitBackend    ← 上半部分逐条委托
                                ├─ svn: SvnClient     ← svn CLI 包装
                                └─ whitelist: Whitelist
```

### 4.2 「委托」的确切含义

不是把 git 操作翻译成 SVN 的等价物，是**字面上执行同一条 git 命令**：

```rust
impl Vcs for SvnGitBackend {
    fn stage(&self, path: &str, paths: &[String]) -> Result<()> {
        self.git.stage(path, paths)   // 跑的就是 `git add <paths>`
    }
}
```

因为暂存区本来就是 git 的暂存区，它就在 `.git/index` 里。

| 用户操作 | SVN 模式下实际执行 | SVN 感知 |
|---|---|---|
| 暂存 / 取消暂存 | `git add` / `git reset HEAD` | 无 |
| 看 diff | `git diff` / `git diff --cached` | 无 |
| 本地提交 | `git commit` | 无 |
| 新建分支 | `git branch` | 无 |
| 切分支 | `git checkout` | 看到文件内容变了，当成本地修改 |
| stash | `git stash` | 同上 |
| **更新** | `svn update` + 前后的 git 编排 | ✅ |
| **交付** | `svn commit <显式路径>` | ✅ |

日常操作全程不跑任何 svn 命令，不需要连公司网络。

### 4.3 不进抽象层的东西

worktree、tag、remote 分支、PR、cherry-pick、revert 在 SVN 模式下没有对应物。
它们保持现有的 git 专属命令，不进 trait；由 `capabilities()` 告诉前端不要渲染
对应 UI。现有 `commands/worktree.rs`（1359 行）等文件一个字不用动。

### 4.4 SvnClient

形状对齐现有的 `engine/git_cli.rs`：

- `run_svn(path, args) -> Result<String>`，走 `hidden_command` 避免闪 console 黑框
- 所有调用带 `--non-interactive`。理由与 `GIT_TERMINAL_PROMPT=0` 完全相同：
  GUI 应用没有终端，svn 一旦等待 stdin 输入凭据就会永久挂死并占住 Tauri 线程池
- 需要机器可读输出的一律用 `--xml`（`info` / `status` / `log`）
- 错误映射为 `crate::error::GitError`，不向前端抛裸字符串

## 5. 并发与原子性

三层，能控制的只有前两层。

### 5.1 仓库级操作锁

`sync_in` / `sync_out` 是多步编排，必须整体原子。加一个按工作副本路径的独占锁：

- 同步流程持有锁期间，所有写操作（stage / commit / checkout / discard / stash）
  直接返回 `GitError::new("sync_in_progress", ...)`
- **拒绝而不是排队**。排队会让用户点了没反应、几秒后突然生效，比明确报错更糟
- 读操作（status / diff / graph）不阻塞，但要能返回「同步进行中」状态供 UI 显示

#### 5.1.1 这把锁必须是跨进程的

**`std::sync::Mutex` 不够——它的作用域只到进程边界。** 现有代码里的锁全是这一类
（`commands/session.rs` 的 `SAVE_LOCK`、`commands/watch.rs` 的 `WATCHERS`），而且仓库
**没有引入 `tauri-plugin-single-instance`**，所以同一个工作副本上完全可能开着两个
mygitclient 进程。

两个进程各自拿到自己那把进程内锁之后会发生什么：

- 同时写 `.git/mygitclient-sync.json`，后写的把前一个的 journal 整个覆盖掉——被覆盖
  那一方的 stash OID、`deliverRef`、`svnDelta` 全部丢失，等于失去恢复依据
- 同时 `git checkout` / `svn add` / `svn commit`。git 自己的 `index.lock` 只保护
  **单条命令**，保护不了整段编排；SVN 侧更是直接对着同一个 `.svn` 乱来

所以锁必须落在**文件系统**上：

```text
锁文件：.git/mygitclient-sync.lock
获取：  以「原子创建且必须不存在」的语义打开（POSIX O_EXCL / Windows CREATE_NEW）
        写入 { pid, startedAt, opid }
释放：  流程收尾时删除（§8.2.4 ⑤ / S11 / D10 都要包含这一步）
陈旧锁：只有当记录的 pid 已不存在时才允许接管，接管必须写日志并告知用户
        ⚠️ 不能只看时间戳——一次 svn commit 卡在网络上几分钟是正常的
```

**journal 的所有权由这把锁界定**：拿到锁才允许读写 journal。没有这条，§7.6 的写前
日志在多进程下是没有意义的——另一个进程随时可能把它覆盖掉。

### 5.2 文件监听重入

`svn update` 一次改动成百上千个文件。现有 `commands/watch.rs` 是 300ms 尾沿去抖，
流程跑到一半就会发 `repo-changed`，前端刷新后读到「已切到基线分支但还没落新基线」
的中间态，界面上会闪出「改动全没了」的假象。

**同步流程开始时挂起该仓库的 watcher，结束后恢复并主动发一次 `repo-changed`。**

顺带一个协同效应：`is_relevant()` 用 `r.is_path_ignored(rel)` 过滤事件，而白名单
机制让美术目录全部处于 ignored 状态——**几十 GB 美术资产的 update 不产生任何刷新
事件**，这条现有逻辑白送了一层性能保护。

### 5.3 外部进程（无法控制）

TortoiseSVN、命令行 `svn`、IDE 的 git 集成都能绕过本客户端直接操作工作副本。
SVN 没有类似 `.git/index.lock` 的工作副本级长效锁（`.svn/wc.db` 的 SQLite 锁只在
单次操作期间持有）。

**所以设计是「检测 + 恢复」，不是「预防」。** 见 §9。

## 6. 仓库状态模型

### 6.1 仓库识别

打开目录时按此顺序判定，**必须先查 `.svn`**，因为混合仓库两者都有：

| 条件 | 判定 | 行为 |
|---|---|---|
| `.svn` + `.git` + `.git/mygitclient.json` | SVN 混合仓库 | 走 `SvnGitBackend` |
| `.svn`，无 `.git` | 未初始化的 SVN 工作副本 | 引导走初始化向导 |
| `.svn` + `.git`，无配置文件 | 可疑 | 阻断并提示（可能是别人留下的 `.git`） |
| 只有 `.git` | 普通 git 仓库 | 现有行为，零变化 |

### 6.2 配置文件

`.git/mygitclient.json`，用户手写，客户端只读（除初始化向导那一次）：

```json
{
  "version": 1,
  "mode": "svn-git",
  "svnUrl": "https://svn.company.com/game/trunk",
  "whitelist": ["Assets/Scripts/", "Assets/Plugins/", "Packages/", "ProjectSettings/"],
  "extensions": [".cs", ".shader", ".cginc", ".asmdef", ".json", ".xml", ".meta"],
  "baselineBranch": "svn/base"
}
```

客户端提供「打开配置文件」入口，沿用现有 `open_custom_commands_config` 的模式。

#### 6.2.1 配置是不可信输入，必须校验

这个文件**用户手写**，所以它和用户在输入框里敲的字符串是同一个可信级别。每一个字段
都会被送进 git 或 svn 的命令行，不校验就等于把命令构造交给了手滑。

**`svnUrl` 是后果最重的一个。** 它只用于「问服务器」的那些操作（`remoteRev`、历史、
**断网查证**），而提交是对着当前工作副本做的。两者指向不同仓库时不会有任何报错：

```text
svnUrl 写错（复制了另一个项目的地址）
  → D3.5 读到的 expectedStartRev 是另一个仓库的 revision
  → §8.6.2 的查证在另一个仓库的 log 里找路径集合
  → 完全可能"找到"一个路径集合相同的 revision  → 判定本次提交成功
  → 推进基线到一个根本没发生过的交付上          → 基线撒谎
```

所以：**`svnUrl` 必须与 `svn info` 读到的工作副本 URL 一致，并且比对 repository
UUID**（`svn info --xml` 的 `<uuid>`）。UUID 才是仓库身份，URL 可能因为代理、镜像、
`http` / `https` 而写法不同。不一致一律阻断，不猜、不自动改写。

其余字段：

| 字段 | 规则 |
|---|---|
| `baselineBranch` | **固定为 `svn/base`**，不再声称可配。全文所有流程都硬编码了这个名字，留一个可配字段只会让两者不一致。字段保留是为了向前兼容，值不是 `svn/base` 就阻断并说明 |
| `whitelist` | 必须是**仓库内相对路径**。拒绝：绝对路径、盘符（`C:`）、UNC、`..` 段、空串、`/`、`.`、以 `.git` 或 `.svn` 开头的路径、以 `:` 开头的 pathspec magic（`:(exclude)` 之类）。分隔符统一规范化为 `/` |
| `extensions` | 必须以 `.` 开头、不含路径分隔符和通配符 |
| 两者皆空 | 阻断。空白名单会让 §12.2 的空参数保护在每一步都触发，等于功能不可用 |

白名单条目同时会被当成 **git pathspec** 和 **SVN target** 用，两套语义都有自己的魔法
字符。`AGENTS.md` §6.2 记的空 pathspec 教训就是这一类——git 把空 pathspec 当"全部"，
而这里一个 `:(exclude)` 或一个 `..` 就能让命令作用到预期之外的范围。**校验必须在配置
加载时一次做完，不要散落到各个调用点。**

### 6.3 白名单语义

**纳入条件是 `whitelist` AND `extensions`**，两者都满足才归 git 管。

选 AND 而非 OR 的理由：`Assets/Scripts/` 底下迟早会混进别人扔的 `.psd`、`.zip`、
临时的 `.bak`——目录白名单挡不住，扩展名能。代价是偶尔漏管该管的文件（如无扩展名
的配置），那时改一行配置即可，比误吸一个 500MB 文件进对象库可逆得多。
`"extensions": ["*"]` 是全纳入的写法。

`whitelist` 路径相对工作副本根，统一正斜杠；以 `/` 结尾表示目录（递归），
否则表示单个文件。

由配置生成 `.git/info/exclude`：

```gitignore
/*
!/Assets/
/Assets/*
!/Assets/Scripts/
/Assets/Scripts/**
!/Assets/Scripts/**/          ← 必须有！重新纳入所有中间目录
!/Assets/Scripts/**/*.cs
!/Assets/Scripts/**/*.meta
```

#### 为什么 `!/Assets/Scripts/**/` 这一行不能省

git 的硬规则：**父目录被排除后，其中的文件无法再被重新纳入**。`/Assets/Scripts/**`
会把 `Assets/Scripts/Player/` 这个**目录**一起排除掉，git 就不再往里递归，后面的
`!/Assets/Scripts/**/*.cs` 对深层文件永远不会生效。

2026-09-12 在临时 fixture 上实测（三层嵌套 + 干扰文件）：

| 规则 | `Scripts/Top.cs` | `Scripts/Player/Mid.cs` | `Scripts/Player/Deep/Low.cs` | `Player/Junk.psd` | `Art/boss.fbx` |
|---|---|---|---|---|---|
| 缺 `!**/` | ✅ | ❌ **静默丢失** | ❌ **静默丢失** | 排除 ✓ | 排除 ✓ |
| 补上 `!**/` | ✅ | ✅ | ✅ | 排除 ✓ | 排除 ✓ |

这是本设计里最危险的一类缺陷：**用户以为文件受 git 保护，实际上没有**——切分支会
丢改动、stash 存不进去、交付清单里不出现。Unity 工程的 `Assets/Scripts/` 底下必然
有多层子目录，所以这条一定会被踩到。

补上之后剪枝仍然有效：`Assets/Art/boss.fbx` 在 `/Assets/*` 这一层就被挡住，git
根本不会递归进美术目录（`git check-ignore -v` 实测命中的是第 3 行）。被重新纳入的
只有白名单**内部**的子目录，那里都是代码，递归代价可忽略。

这套反转写法极易写错，因此**由代码生成、绝不让人手写**，并有独立单元测试：给定
配置 → 用真实 git fixture 断言 `git ls-files` 的结果，**必须覆盖至少两层嵌套目录**，
含 Windows 反斜杠形态的输入。断言对象是 git 的实际行为，不是生成出来的规则文本——
测规则文本等于没测。

### 6.4 基线分支

`refs/heads/svn/base`——普通分支，IDE 可见。

- 每次成功 `svn update` 后在其上落一个 commit，内容 = 白名单内文件在该 revision
  的状态
- commit message 携带权威记账，**沿用 `git svn` 的原生 trailer 格式**：

```text
svn: r12346

git-svn-id: https://svn.company.com/game/trunk@12346 8f3a1c7e-4b21-0410-9d2f-1a7b3c5d9e01
```

#### 6.4.1 为什么用 `git-svn-id:` 而不是自定义的 `Svn-Rev:`

格式固定为一行：`git-svn-id: <url>@<rev> <repository-uuid>`，解析正则
`^git-svn-id: (\S+)@(\d+) ([0-9a-fA-F-]+)$`。三个字段缺一不可，解析失败即视为该
commit 不是 baseline（不猜、不部分接受）。

换用这个格式的收益：

- **UUID 白送**。§6.2.1 要求比对 repository UUID（URL 可能因代理 / 镜像 / `http` 与
  `https` 写法不同而不可靠），原来的 `Svn-Url:` 记的是 URL，帮不上忙；`git-svn-id`
  自带 UUID，基线自己就带着仓库身份
- **工具互操作**。`git log --grep=^git-svn-id` 是 `git svn` 文档里给出的标准用法，
  IDE 与第三方工具也按这个格式识别 SVN 映射
- **迁移锚点**。将来若 §2.2 的结论被推翻、真要迁到 `git svn`，历史里已有现成的映射

⚠️ **代价**：一个用户若在本仓库里手工跑 `git svn init`，`git svn` 会把这些 trailer
当成自己的映射来解释，而 `.git/svn/` 里并没有对应的 `rev_map`。这不会损坏数据，但
会产生令人困惑的行为。记为已知限制（§15 第 19 条）。

- **客户端 UI 禁止在基线分支上开发提交。** 流程内部会临时切上去，但任何流程结束时
  必须回到原分支

选可见分支而非 `refs/mygitclient/base` 的理由：D8 选 Overlay 的核心收益就是 IDE
可用，而「我相对上次更新改了什么」是最常看的 diff，把基线藏起来等于削掉这个收益。
误操作防护改由 UI 限制 + §9 的失同步检测承担；真删了也能从 reflog 恢复，最坏
重建一个基线。

已知小坑：分支名带斜杠意味着 git 会建 `.git/refs/heads/svn/` 目录，此后不能再有
一个名为 `svn` 的分支。

### 6.5 初始化向导

**动手之前必须先校验工作副本，否则会造出一个虚假基线。**

```text
P1  白名单内 svn status 必须干净
      不允许存在 M / A / D / R / C / ! / ~ / ? 任何一种
      ⚠️ 必须带 --no-ignore：svn status 默认隐藏 SVN-ignored 文件，而那些
         文件 git 照收，不带这个参数就会漏检
      ⚠️ 不自动规范化——revert 用户的本地修改是破坏性的，只拦住并引导去
         TortoiseSVN 自己处理（提交或还原），对齐 §8.1 #2 的做法
P2  nodeRevs 必须是单元素集合 {N}
      多值说明工作副本是 mixed-revision，引导用户先跑一次完整 svn update
P3  不在 svn update / merge 中途（.svn 里没有未完成的操作）
```

三条全过之后才建库：

```text
git init -b svn/base                 ← 显式指定，不吃 init.defaultBranch 的默认值
写入 .git/mygitclient.json（用户编辑白名单）
生成 .git/info/exclude
git add -- <白名单> && git commit    ← 首个基线，git-svn-id 的 rev 取自 P2 读到的 N
基线体检（§6.8）                      ← 有差异则整个回退，删掉刚建的 .git
git checkout -b work                 ← 立刻离开基线分支
```

#### 6.5.1 为什么这三条前置不能省

`svn/base` 的语义是「白名单内的文件在 revision N 时**服务器上的样子**」。初始化直接
`git add` 当前磁盘内容，等于宣称「磁盘 == 服务器」，而这个等式只有在工作副本干净且
单一 revision 时才成立。

不校验的后果，按状态分：

| 初始状态 | 被写进基线的东西 | 后果 |
|---|---|---|
| `M` 本地修改 | 改过的内容被当成服务器原文 | 这些改动**永远不会**出现在交付的净变更里（基线和工作分支都有它），静默漏交付 |
| `?` 未版本控制 | git 收了，SVN 不知道 | 第一次交付时 D1 算不出它是新增 |
| `A` / `D` 待提交登记 | 登记状态没进 git | §8.5 的差量基准从一开始就是脏的（§8.5.1） |
| mixed-revision | —— | 见下 |

mixed-revision 那条最隐蔽：原设计「rev 取自 `svn info`」，读的是**根节点**的
revision，而 §6.6.2 的判据比的是**白名单 `nodeRevs`**。两者在 mixed-revision 工作副本
里本来就不相等——

```text
实测（2026-09-13，真实 SVN fixture）：
  rootRev（svn info .）  = 1
  白名单 nodeRevs        = {3}
  → 若照原设计取 rootRev 作 git-svn-id 的 rev，baselineRev=1、nodeRevs={3}，
    初始化**完成的那一瞬间**就判失同步
```

所以 `git-svn-id` 的 rev 必须取自 `nodeRevs`，而且 `nodeRevs` 必须先是单值——这也是 P2 存在的
全部理由。

### 6.6 状态判定

#### 6.6.1 SVN 工作副本是 mixed-revision 的

这是 SVN 与 git 最容易被忽略的差异，也是本节设计的全部理由。

`svn commit` **只推进被提交节点的 revision**，工作副本里其余节点（包括根目录）留在
原 revision。所以一次「提交了 `Assets/Scripts/Player.cs`」之后，工作副本是这样的：

```text
根目录 .              r12345      ← 没动
Assets/Scripts/Player.cs  r12346  ← 只有它前进了
Assets/Art/boss.fbx       r12345
```

**因此读工作副本根的单个 `<entry revision>` 不能证明任何事。** 外部只提交一个深层
文件时，根的 revision 纹丝不动，`baselineRev == rootRev` 照样成立——§9 那道失同步门禁
会被完整绕过，基线开始撒谎而界面上一片祥和。

#### 6.6.2 分层检测

```text
baselineRev = svn/base 顶端 commit 的 git-svn-id trailer 的 rev  ← 权威记账
rootRev     = svn info --xml 的 <entry revision>          ← O(1)，本地读，毫秒级
nodeRevs    = svn status -v --xml -- <白名单路径>          ← 白名单内每个已版本控制
                                                             节点的 revision 集合
                                                             本地读，范围限于代码
remoteRev   = svn info --xml <URL> 的 revision            ← 走网络，异步，失败不阻塞
```

**正常状态的不变量**：`nodeRevs` 是单元素集合，且等于 `{baselineRev}`。

失同步的判据**只有这两条**：

| 观察 | 含义 |
|---|---|
| `nodeRevs` 有多个值 | mixed revision，外部提交或局部 update 过 |
| `nodeRevs = {N}` 但 `N != baselineRev` | 整体被外部推进或回退 |

**`rootRev` 不是判据，只是变化提示。** §6.6.1 已经论证过「读根的单个
`<entry revision>` 不能证明任何事」；把 `rootRev != baselineRev` 当成失同步的判据，是在
用一个自己刚否定掉的信号下结论。更要命的是它在**正常路径上就必然成立**：

```text
实测（2026-09-13，真实 SVN fixture）：一次完全成功的交付之后
  rootRev（svn info .）        = 1     ← D6 只更新白名单，根节点纹丝不动
  nodeRevs（白名单 status -v） = {3}
  baselineRev                  = 3
  服务器 HEAD                  = 3
```

`rootRev != baselineRev` 成立，但仓库状态**完全正常**——这正是 §8.2.1 主动选择的
mixed-revision 结果。拿它报失同步，等于每次交付成功后都给用户挂一个红色横幅。

所以 `rootRev` 的正确用法是**与上次观测值比**：变了就说明「有人动过工作副本」，
触发一次完整的 `nodeRevs` 校验；没变就跳过。它回答的是「要不要花钱查」，不是
「是不是坏了」。

未版本控制的文件（git 新增但还没 `svn add`）不参与 `nodeRevs` 集合；已 `svn add` 尚未
提交的节点没有 revision，同样跳过——但它们的存在本身就说明有外部登记，单独报警。

#### 6.6.3 检查频次分层

`svn status -v` 要走一遍白名单子树，比 `svn info` 贵得多（但仍是纯本地、范围限于
代码文件，不是几十 GB 的美术）。所以分两档：

| 时机 | 检查 | 理由 |
|---|---|---|
| 每次 `repo-changed` 刷新 | `rootRev` 与**上次观测值**比（O(1)） | 高频；变了才跑下一档，代价可忽略 |
| 打开仓库、`sync_in` / `sync_out` 第一步、恢复向导前 | **完整 `nodeRevs` 校验** | 这几处是正确性的闸门，必须问到底 |

**第一档比的是「上次观测值」，不是 `baselineRev`。** 交付之后根节点会长期停在旧
revision（§6.6.2 实测），若拿它跟 `baselineRev` 比，第一档就会**每次刷新都判不等**，
于是每次刷新都触发第二档的全量 `svn status -v`——分层的意义直接归零。与上次观测值比
则不会：根节点没被动过就一直相等，安静跳过。

上次观测值存在内存里即可，进程重启后第一次刷新当作「变了」跑一次全量，这正是打开
仓库本来就要做的事。

顺带记一个**考虑过但否掉**的方案：`svn update --depth=empty .` 实测确实能只推进根节点
的 revision 而不碰白名单外的内容（本地修改的美术文件原样保留、状态仍是 `M`）。但它把
`rootRev` 推到的是**当时的服务器 HEAD**，而 `baselineRev` 是白名单节点的 revision，
同事在这之间提交一次两者就又不等了——恢复不了等式，却要在「已不可撤销」的 D5 之后
再加一次 SVN 写操作，不划算。

漏检的代价只是**延迟发现**——真正会造成数据损坏的操作全都在第二档后面，不存在
「绕过闸门直接交付」的路径。
「绕过闸门直接交付」的路径。

`remoteRev` 走网络，按现有 `refreshPullRequests` 的模式处理：不 await、失败静默
（读不到就不显示落后数，不弹错）。`remoteRev > rootRev` 时显示「落后 N 个 revision」。

### 6.7 本地补丁：Local-Only commit

白名单内的某个 `.cs` 受 SVN 管理，但开发者要长期保留一些**不打算交付**的本地改动：
注释掉刷屏的 log、塞一段临时测试代码、改一个只在本机用的调试常量。SVN 里的对应做法
是把文件扔进 `ignore-on-commit` changelist。

本设计**不照搬 changelist**，而是把这类改动放进带 trailer 标记的 git commit：

```text
local: mute spawn log

Local-Only: true
```

判定只认这条 trailer，**不新增任何配置**（§6.2 的配置文件不加字段）。客户端在提交框
提供「仅本地，不交付」勾选框，勾上就追加该 trailer，别的什么都不做。

#### 6.7.1 关键不变量：补丁活在 commit 里，不活在工作区

这一条是整个方案能与现有流程共存的前提，逐条对照：

| 流程 | 为什么不受影响 |
|---|---|
| §7 `sync_in` | **零改动**。S9 的 `git rebase --empty=drop svn/base` 会把 Local-Only commit 和普通 commit 一起搬到新基线上 |
| §8.1 前置检查第 2 条「工作区必须干净」 | 依然成立。补丁已经 commit 了，工作区本来就是干净的 |
| §9.4 恢复流程 | 第 ③ 步 `git checkout svn/base` 之后磁盘上不含补丁，第 ④ 步的 `svn status` 不会把它误判成需要 `svn revert` 的 `M` 文件 |
| §6.6 失同步检测 | 按 `nodeRevs`（revision 号）判定，与文件内容无关 |

反过来说：**如果允许本地补丁以未提交改动的形态长期存在，上面四条全部失效**——这正是
不能照搬 changelist 的直接原因。

**稳态**：一次交付完成后，工作分支相对 `svn/base` 只剩下 Local-Only commit（已交付的
commit 被 rebase 按 patch-id 丢弃）。补丁自动跟着每一次 `sync_in` / `sync_out` 向前走，
用户不需要做任何维护动作。

#### 6.7.2 为什么不是文件级清单

| | Local-Only commit（本设计） | `localOnly` 路径清单（≈ changelist） | 把文件移出白名单 |
|---|---|---|---|
| 粒度 | **hunk** | 文件 | 文件 |
| 同一文件既有本地补丁又有待交付改动 | 支持 | **二选一** | 不支持 |
| 跨 `sync_in` / `sync_out` 存活 | 自动 | 只能是未提交改动，随时被 checkout / stash 波及 | 存活，但代价见右 |
| 对基线正确性的影响 | 无 | **破坏**，见下 | 无 |
| 该文件还能交付真实改动吗 | 能 | 能（但会连补丁一起交付） | **永远不能** |
| 白名单外的 `svn update` 冲突 | 不涉及 | 不涉及 | 产生 `.mine` / `.rN`，客户端无恢复手段（§15.9） |

文件级清单破坏基线正确性的具体路径：§8.2.2 的 D8c 是 `git add -A -- <白名单>`，收的是
**磁盘内容**。若某文件被排除在 `svn commit` 之外却带着补丁躺在磁盘上，这个补丁会被写进
基线 commit，而该 commit 声称自己等于服务器的 `r_new`——此后所有「我相对服务器改了什么」
的判断全错。这正是 §9.2 的「基线撒谎」，只不过是自己造出来的。

另一条绕不过去的物理事实：**`svn commit` 提交的是磁盘上的字节，不是 git 的某棵树。**
要让 SVN 看不见补丁，磁盘在那一刻就必须真的没有它——这决定了交付流程的形状（§8.2.3）。

### 6.8 基线体检：让基线可被服务器证伪

#### 6.8.1 为什么需要它

第二轮 review 的「四个切面」表（§18）已经把根因写出来了：**`svn/base` 是由
`git add -A` 收磁盘而来的，而磁盘不能自证等于服务器。** §6.5 的初始化前置、§6.6 的
`nodeRevs` 判据、§9.4 的恢复流程都是在**防止**假基线产生，但它们全是**间接**判据——
检查的是 revision 号和工作副本的登记状态，**没有任何一条直接比对过「基线里的字节」与
「服务器上的字节」**。

于是每发现一个新的边界，就得再补一条间接判据，而且永远无法证明补全了。`git svn`
没有这个问题（§2.2），因为它的基线是从服务器 fetch 下来的。本设计拿不到那个性质，
但可以拿到它的**可证伪性**：加一条直接问服务器要事实的检查。

#### 6.8.2 怎么做

前提：**磁盘上白名单内的内容等于基线树的内容**。满足它的时刻有两类，§6.8.5 列的四个
时机全部落在这两类里：

- HEAD 在 `svn/base` 上且工作区干净（§6.5 首个基线之后、§9.4 第 ⑥ 步之后）
- 基线树刚刚从磁盘录出来（§16 Phase 1 的 R2–R3，全程不动 HEAD）

前提成立时就可以直接拿磁盘去跟服务器比：

```bash
svn diff --old=<svnUrl>@<baselineRev> --new=<工作副本根> --summarize -- <白名单根...>
```

有输出即「基线与服务器不符」，**直接证伪**，不需要任何关于 revision 号的推断。

实测（2026-09-13，真实 SVN fixture）：

| # | 场景 | 结果 |
|---|---|---|
| T1 | 基线 = r1，工作副本干净 | 无输出 ✅ |
| T2 | 白名单内一个 `.cs` 被本地改过 | `M <url>/Assets/Scripts/A.cs` ✅ |
| T3 | **美术文件被改**，体检只传白名单根 | 无输出 ✅ 不误报 |
| T4 | 磁盘已是 r2，仍拿 `baselineRev=1` 比（假基线） | 报出差异 ✅ 正是要抓的那类 |
| T5 | 同一时刻用正确的 `baselineRev=2` 比 | 无输出 ✅ |
| T6 | 一次传入多个白名单根 | 接受多 target ✅ |
| T7 | 服务器删掉 `Deep/D.cs` 后拿旧 rev 比 | `D <url>/Assets/Scripts/Deep/D.cs` ✅ 覆盖 §9.4.1 那类漏检 |

#### 6.8.3 这条检查证明不了什么

它比的是**磁盘**与服务器，不是**基线树**与服务器。两者之间隔着一个假设：「git 跟踪的
文件集合 == 白名单内应跟踪的集合」，而这个假设正是 §6.3 那条 exclude 规则可能打破的
（漏掉 `!/Assets/Scripts/**/` 时，深层 `.cs` 在磁盘上好好的，只是 git 不管）。

所以分工是明确的，不要互相冒充：

| 缺陷类型 | 谁抓 |
|---|---|
| 基线 rev 与服务器内容不符（假基线的四个切面，§18） | **本节的体检** |
| git 跟踪的文件集合不对（exclude 规则写错） | §14.2 第 6 项，断言 `git ls-files` 的真实结果 |

要一条命令同时覆盖两类，得把服务器子树 `svn export` 下来再与基线树逐文件比对，
成本从「传一个 diff」变成「下一整棵子树」。**本期不做**，记在这里是为了说明边界在
哪，而不是留一个 TODO。

#### 6.8.4 解析输出的两个坑（实测）

`--summarize` 的输出形态与直觉不一样，照着字面解析会错：

```text
M       file:///…/repo/Assets/Scripts/A.cs        ← 第 1 列 = 内容变更
 M      file:///…/repo/Assets/Scripts             ← 第 2 列 = **属性**变更，且是**目录**
```

1. **路径是 URL，不是工作副本相对路径。** 必须剥掉 `<svnUrl>` 前缀才能和白名单、
   `git ls-tree` 的输出对齐
2. **输出里会混进目录条目，以及只有属性变更的条目。** 体检**只看第 1 列（内容）且
   只看文件**；目录条目一律剔除——与 §8.6.2「比对前必须先剔除目录路径」同源，那里
   不剔除会把真成功判成失败，这里不剔除会把正常仓库判成基线撒谎

#### 6.8.5 什么时候跑

只在**降级路径**和用户主动要求时跑，不进 `sync_in` / `sync_out` 热路径：

| 时机 | 理由 |
|---|---|
| 初始化向导，`git commit` 首个基线之后、`git checkout -b work` 之前（§6.5） | 这是假基线的最大来源；此刻还没建工作分支，不通过就整个回退，代价最小 |
| §9.4 恢复流程第 ⑥ 步之后、第 ⑦ 步之前 | 恢复的全部目的就是修好基线，不验证等于没修 |
| §16 Phase 1「重建基线」R3 之后、R4 之前（R3b） | 同上；此时指针还没动，不通过就中止，代价为零 |
| 用户手动点「体检」 | 怀疑基线不对时的唯一直接手段 |

**不在 `sync_in` 的 S7 之后跑**：S5 的 `svn update` 刚成功，磁盘按构造就是服务器内容，
再问一次是冗余的网络开销。

成本是白名单子树的一次 diff——白名单里没有美术资产，量级是 MB 不是 GB。

#### 6.8.6 失败与「没跑成」必须分开

| 结果 | 处理 |
|---|---|
| 无输出 | 基线可信，记下体检时间与 revision |
| 有输出（真差异） | **基线撒谎**。阻断并列出差异文件；初始化时整个回退，恢复流程时停在第 ⑥ 步报错 |
| 命令失败（断网、认证、URL 不可达） | **不是基线有问题**。不阻断，标记「基线未经体检」并在 UI 上如实显示，对齐 §6.6.2 `remoteRev` 的「不 await、失败静默」模式 |

把第三行单独列出来是必须的：把「没能验证」当成「验证不通过」会让离线场景寸步难行，
当成「验证通过」则等于这条检查不存在。

## 7. 更新流程 sync_in

### 7.1 为什么不能直接 svn update

工作目录此刻的内容 = SVN base + 用户 git 分支上的全部改动，SVN 把这些统统当作
「本地修改」。直接 `svn update` 会有三个问题：

1. SVN 用三方合并处理 git 分支的最终状态，冲突标记是 `.mine` / `.rN` 文件
2. 没有 `--abort`，合并到一半的状态难以回退
3. **想放弃时 `svn revert` 会把文件恢复到 SVN base 而不是 git 分支的状态——
   未提交的改动会被直接吃掉**

设计目标因此是：**让 `svn update` 永远不可能冲突，把全部冲突逼进 git 里解决。**

### 7.2 前置检查

任一不过则中止，且不做任何修改：

1. 不在 rebase / merge 中途
2. 基线未失同步（走 §6.6.3 的**完整 `nodeRevs` 校验**，不是 O(1) 那档）
3. 当前不在 `svn/base` 上，且 **HEAD 必须附着在分支上**（detached HEAD 拒绝，见 §8.1.1）
4. `svn status` 里白名单外没有冲突态文件（`C`）
5. **白名单外若有本地修改（`M` / `!`），列出来并要求用户确认**——见下
6. 拿到该工作副本的独占锁

#### 7.2.1 为什么第 5 条是确认而不是阻断

白名单外的文件（美术资产、Unity 生成的 `.asset`）**git 完全不管**，客户端对它们
没有任何恢复手段。S5 的全量 `svn update` 会把远端改动合并进这些本地修改，冲突时
只会留下 SVN 的 `.mine` / `.rN` 文件，得回 TortoiseSVN 处理。

但**不能因此阻断**：更新美术资产是程序同学的刚需（不更新 Unity 工程就跑不起来），
而工作副本里常年挂着几个被 Unity 碰过的 `.meta` 是常态。阻断等于让这个功能不可用。

所以：**如实列出来，说明风险，让用户决定**。确认后继续；S5 真冲突了就按 §13 报错
并明确指出是白名单外的文件，引导去 TortoiseSVN——**不假装客户端能修**。

### 7.3 步骤

```text
S0   挂起 watcher
     写 operation journal（见 §7.6），phase=start          ← 任何修改之前
     **origBranch / origHead 此刻就落盘**（不是等到 S2）
S1   工作区不干净时：
     ① phase=stashing，stashRefBefore = git rev-parse refs/stash
        （不存在则记 null）                                ← 必须早于 push，见 §7.3.1
     ② git stash push -u -m "mygitclient-sync:<opid>" -- <白名单>
                                                        ← 第一次改动工作区。message 里的
                                                          opid 是恢复时的精确锚点，见 §7.3.1
     ③ 立刻 git rev-parse refs/stash 取不可变 OID
     ④ git update-ref refs/mygitclient/sync/<opid> <OID>
     ⑤ OID 写进 journal，phase=stashed                     ← 不记 stash@{0}，见 §7.6.1
S2   （origBranch 已在 S0 落盘，此处只推进 phase）
S3   git checkout svn/base
S4   svn status --xml 校验白名单内应为 0 修改              ← 双保险
S5   svn update --accept postpone
S6   svn info --xml + svn status -v --xml 读新 revision
S7   git add -A -- <白名单> && git commit --allow-empty     新基线
S8   git checkout ORIG_BRANCH
S9   git rebase --empty=drop svn/base                       ← 冲突在这里爆
S10  git stash apply <OID>（若 S1 有 stash），成功后
     git stash drop 对应条目 + 删掉 refs/mygitclient/sync/<opid>
S11  删除 journal；恢复 watcher + 主动发 repo-changed
```

#### 7.3.1 S1 的 stash 崩溃窗口：写前日志必须写在 push 之前

§7.6 开篇把这个窗口作为写前日志存在的理由写了出来——「S1 完成 stash 之后、S2 写状态
之前崩溃，用户的 WIP 就只剩一个匿名 stash 条目，客户端重开后对它一无所知」。但原来的
步骤顺序**原样保留了它**：`git stash push` 先改工作区，之后才 `rev-parse` → `update-ref`
→ 写 journal，`ORIG_BRANCH` 更是到 S2 才落盘。写前日志的原则恰好在最关键的那个字段上
没有执行。

崩在 ② 和 ⑤ 之间会怎样：

```text
journal:  phase=start，没有 stashOid
磁盘:     工作区已经被 stash 干净了
refs/stash: 多了一条谁也不认识的条目，里面是用户的 WIP
```

`phase=start` 的语义是「还没动任何东西」，恢复代码据此什么都不会做，用户的未提交改动
就这样变成一个匿名 stash 条目躺在那里。

**修法有两半。** 一半是把可先知的信息全部前置（`origBranch` / `origHead` 提到 S0，
`stashRefBefore` 提到 push 之前）；另一半是**给 stash 本身打上 opid 标记**，这样
「栈顶那条是不是我的」就不再需要猜。

`opid` 在 push 之前就已经存在，所以可以直接写进 stash 的 message：

```text
git stash push -u -m "mygitclient-sync:<opid>" -- <白名单>
```

恢复时按 opid 在 stash 的 reflog 里精确定位，**不依赖栈位置**：

```text
恢复时读到 phase=stashing：
  git log -g --format='%H %gs' refs/stash | grep "mygitclient-sync:<opid>"
  ├─ 找到 → 就是本流程压的那条。取它的 OID，补上 update-ref 和 journal，
  │          继续按 stashed 处理
  └─ 没找到 → 再比对 refs/stash 与 stashRefBefore 确认 push 确实没落地
              （两者相等 ⇒ 工作区未被改动，直接按放弃收尾）
```

实测（2026-09-13，临时 git fixture）——**外部工具在窗口内又压了一个 stash** 的场景：

```text
git stash push -m "mygitclient-sync:0f3adead-beef"    ← 我们的
git stash push -m "someone else WIP"                  ← 外部工具的，挤到了栈顶

git stash list
  stash@{0}: On work: someone else WIP                ← 栈顶不是我们的
  stash@{1}: On work: mygitclient-sync:0f3adead-beef

git log -g --format='%H %gs' refs/stash | grep "mygitclient-sync:0f3adead-beef"
  → bf4757c…  On work: mygitclient-sync:0f3adead-beef ✅ 精确命中
  → git show bf4757c…:a.txt = WIP-MINE                ✅ 内容正确
```

所以这**是**确定性判定，不需要降级成人工辨认。`stashRefBefore` 仍然要记：它负责区分
「push 没落地」和「push 落地了但 reflog 被外部 `stash clear` 抹掉了」这两种都找不到的
情况——后者要报错停下，不能当成没事发生。

剩余的真实风险只有一个：外部工具跑了 `git stash clear` 或 `drop`。那种情况下对象可能
还在（我们在 ④ 用 `refs/mygitclient/sync/<opid>` 钉过），但若崩在 ④ 之前就没钉上，
对象会被 gc 回收。这是 §7.6.1 那条「钉住」规则要尽早执行的原因。

### 7.4 不显然的点

- **S4 是双保险，不能省。** revision 比对能抓住「外部提交过」，抓不住「外部手工
  改了 base 文件」。这一步直接问 SVN 要事实
- **S5 的 `--accept postpone` 是硬要求。** S4 保证了内容冲突不可能发生，但
  **SVN 树冲突**（服务器删掉某个目录、或把目录换成文件）与工作副本干净与否无关。
  `postpone` 让它停下来报错，而不是弹交互提示卡死子进程，也不替用户自动选
- **S7 白名单内无变化时也要落 commit**（`--allow-empty`）。基线记账必须跟上
  revision，否则下次 §6.6 的检测会误报失同步
- **S9 / S10 的冲突不是失败，是正常出口之一。** 此时必须**释放锁**（用户要编辑
  文件解决冲突），仓库进入「同步未完成」状态
- **本地补丁（§6.7）在这里不需要任何特殊处理。** Local-Only commit 就是普通 commit，
  S9 的 rebase 把它和其余 commit 一起搬到新基线上。它与服务器改动真冲突时，冲突就在
  S9 爆出来，和普通代码冲突走同一条路——这正是选 commit 而非未提交改动的收益

### 7.5 代价

S2–S7 之间工作目录会短暂回到基线状态，Unity 会多触发一次编译 / reimport。
只影响白名单内的代码文件，是一次编译的量级。相对于「SVN 冲突文件满地 + 可能丢
改动」，这个代价可以接受。

### 7.6 Operation journal（写前日志）

`sync_in` 和 `sync_out` 共用这一套。它不是「冲突时才写的状态文件」，而是**写前
日志**：任何一步修改落地之前，journal 必须已经在磁盘上。

否则存在无法恢复的窗口——S1 完成 stash 之后、S2 写状态之前崩溃，用户的 WIP 就
只剩一个匿名 stash 条目，客户端重开后对它一无所知；D4 登记完 `svn add` / `svn delete`
之后崩溃，`.svn` 里挂着一批登记而没有任何记录说明它们是谁留下的。

`.git/mygitclient-sync.json`：

```json
{
  "opid": "0f3a…",
  "flow": "sync_in",
  "phase": "rebase",
  "startedAt": 1757000000,
  "origBranch": "work",
  "origHead": "e5d3f77…",
  "stashRefBefore": "9c1e4b2…",
  "stashOid": "f2208125aad7d35d63ee9495ad0a6cc3c1ce21be",
  "baselineRevBefore": 12345,
  "svnAdded": ["Assets/Scripts/JumpConfig.cs"],
  "svnDeleted": ["Assets/Scripts/Old.cs"],
  "svnStatusBefore": [{"path": "…", "status": "…"}],
  "svnDelta": [{"path": "…", "status": "A"}],
  "expectedStartRev": 12345,
  "commitMessage": "…",
  "deliverRef": "refs/mygitclient/deliver/0f3a…",
  "tempWorktree": "C:/Users/x/AppData/Local/Temp/mygitclient-deliver-0f3a…",
  "localOnlyCommits": ["a1b2c3d…"]
}
```

最后三个字段只有 `sync_out` 用：

- `deliverRef` 是 D0.5 构造出的交付树（§8.2.3），D3.8 detached 切上去的就是它；
  早退、回滚与崩溃恢复都要靠它和 `origBranch` 收尾（§8.2.4、§8.5）
- `tempWorktree` 只有慢路径有，是 cherry-pick 重放用的临时目录。**必须在建 worktree
  之前就落盘**，否则崩在 `git worktree add` 和写 journal 之间就会留下一个谁也不认识
  的 worktree 注册项
- `localOnlyCommits` 记下本次被摘掉的 commit，用于崩溃重开后如实告诉用户「这 N 个
  本地补丁没有被交付，它们还在 `origBranch` 上」——不记的话这个信息在 `deliverRef`
  被删之后就再也拼不回来了

`sync_out` 的 phase 取值与含义：

| phase | 已经发生了什么 | 放弃时怎么收尾 |
|---|---|---|
| `prepare` | 只挂了 watcher、拿了锁；`origBranch` / `origHead` 已落盘 | §8.2.4（①②③ 均为 no-op） |
| `building-deliver` | 可能有临时 worktree、可能有半途的 cherry-pick | §8.2.4 |
| `deliver-built` | `deliverRef` 已存在，主工作副本未动 | §8.2.4 |
| `checked-out` | D3.8 已切走，主工作副本是交付树的内容 | §8.2.4（② 会真的切回去） |
| `svn-staged` | `.svn` 里挂着 add / delete 登记，`svnDelta` 已落盘 | §8.5（先撤 SVN 登记，再复用 §8.2.4 ②～⑤） |
| `committing` | `svn commit` 已发出，结果未知 | §8.6.2 查证，**不得盲目回滚** |
| `post-commit` | 服务器已落地，基线未推进 | 不回滚，提供「重试 D6 起」入口 |

`sync_in` 另有两个 phase 与 stash 相关，见 §7.3.1：

| phase | 已经发生了什么 | 放弃时怎么收尾 |
|---|---|---|
| `stashing` | `stashRefBefore` 已落盘，`git stash push` 可能落地也可能没有 | 比对当前 `refs/stash` 与 `stashRefBefore` 判定，**不静默 apply** |
| `stashed` | stash 已落地且 OID 已钉住 | 按 `stashOid` 恢复 |

`building-deliver` 与 `deliver-built` 必须分开：前者意味着可能存在半途状态（worktree、
sequencer），后者意味着构造已完成、只剩一个干净的 ref。合成一个 phase 的话，恢复代码
就得靠探测磁盘来猜自己停在哪，而写前日志的全部意义就是不用猜。

规则：

- **原子写入**：写临时文件 → `fsync` → rename。半个 JSON 比没有 JSON 更糟
- **每进入一个 phase 前先更新 `phase` 字段**，而不是事后补记
- 流程正常收尾时删除；打开仓库时发现它存在 → 界面顶部常驻「上次同步未完成」，
  提供继续 / 放弃两个入口
- rebase 中途的状态**不重复记**：git 自己记在 `.git/rebase-merge/`，现有
  `repo_state` 命令已能报出，复用它

#### 7.6.1 stash 必须按不可变 OID 记，不能记 `stash@{N}`

`stash@{0}` 是**栈位置，不是身份**。任何一次新的 stash（你自己、IDE、另一个打开
同一仓库的标签页）都会把它挤走，随后按 `stash@{0}` 恢复会应用到**别人的 WIP**。

2026-09-12 实测：

```text
git stash push -u          → refs/stash = f2208125…   （记下这个 OID）
（模拟 IDE 又压了一个）     → stash@{0}  = 5072d1f2…   ⇒ 已漂移
git stash apply f2208125…  → 取回的仍是正确的 WIP     ✓
```

所以：push 之后**立刻**取 `git rev-parse refs/stash` 得到 OID，用
`git update-ref refs/mygitclient/sync/<opid> <OID>` 钉住（防止 stash 被外部 drop
后对象被 gc），OID 写进 journal；恢复时用 `git stash apply <OID>`。

恢复前必须先校验该 OID 对象仍然存在且是一个 stash 形状的 commit，不存在就报错停下，
**不猜、不退回用 `stash@{0}`**。

## 8. 交付流程 sync_out

### 8.1 前置检查

1. 基线未失同步
2. **工作区必须干净**。这里**不自动 stash**：交付的是 commit 过的内容，未提交的
   改动不该被悄悄排除、也不该被悄悄带上。硬拦住，让用户自己决定。
   若那些改动本来就不打算交付（注释掉的 log、临时测试代码），正确做法是**提交成
   Local-Only commit**（§6.7），不是留在工作区里
3. 不在 rebase / merge 中途
4. 当前分支不是 `svn/base`，且 **HEAD 必须附着在分支上**——detached HEAD 一律拒绝，见 §8.1.1
5. `remoteRev == baselineRev`（走一次网络）；不等则要求先更新
   ⚠️ 比的是 `baselineRev`，**不是 `rootRev`**。交付之后根节点会长期停在旧
      revision（§6.6.2 实测），拿它来比会让**之后每一次交付都被这条挡住**。
      第 1 条已经保证 `nodeRevs == {baselineRev}`，所以 `baselineRev` 就是
      「工作副本当前所处的 revision」的正确表述
6. **需要走慢路径时，待交付历史必须是线性的**：`localOnlyCommits` 非空且不全部
   位于分支顶端（即 §8.2.3 的慢路径）时，`git rev-list --merges svn/base..HEAD`
   必须为空。非空则拒绝交付，指出是哪个 merge commit，引导用户把本地补丁移到分支
   顶端（转成快路径，merge 不受影响）或先线性化历史。
   理由见 §8.2.3 的 T4 / T5：`cherry-pick` 对 merge commit 要么硬失败，要么在加
   `-m` 之后重复应用已摘取过的父提交改动，没有安全的自动解法。**快路径不做这个
   检查**——它只是给一个已有 commit 起名字，历史里有没有 merge 都无所谓
7. 拿锁

#### 8.1.1 为什么 detached HEAD 必须拒绝而不是支持

Overlay 让外部 git 工具可用，所以 detached HEAD 是**合法可达**的状态：用户在 Rider
里点开一个历史 commit、或跑了 `git checkout <sha>`，就会停在那里。

但本流程从头到尾都建立在「有一个可以推进的分支」之上：

| 依赖点 | detached 时会怎样 |
|---|---|
| journal 的 `origBranch` | `git rev-parse --abbrev-ref HEAD` 返回字面量 `HEAD` |
| §8.2.4 ② 的 `git checkout <origBranch>` | 变成 `git checkout HEAD`，**不会**回到原来的位置 |
| §8.2.4 ④ 的 `HEAD == origBranch` 断言 | 无从判定 |
| D8d 的 `merge --ff-only` / `rebase` | 没有分支可推进，基线推进不下去 |
| D9 的「work 分支已交付」提示 | 无意义 |

理论上可以改成「detached 时按 `origHead` 恢复位置」——`origHead` 确实在 journal 里。
但 D8d 需要的是**把工作分支推进到新基线**，这件事没有分支就做不了；硬撑的话得临时
造一个分支，等于替用户做了他没要求的事。

交付本来就该从一个分支上做。所以：**明确拒绝，并告诉用户当前 HEAD 指向哪个 commit、
建议 `git switch -c <name>` 或切回原分支**，而不是假装支持。`sync_in`（§7.2 #3）同理。

### 8.2 步骤

```text
D0   挂起 watcher；写 journal，phase=prepare                ← 任何修改之前
     origBranch / origHead 此刻就落盘
D0.5 deliverRef / localOnlyCommits / tempWorktree 先写进     ← 写前日志，见 §7.6
     journal，phase=building-deliver，再构造交付树
     refs/mygitclient/deliver/<opid>：从 HEAD 摘掉 Local-Only
     commit。没有 Local-Only commit 时它等于 HEAD。
     **主工作副本磁盘全程不动**，慢路径在临时 worktree 里
     重放。构造完成后 phase=deliver-built                   ← 见 §8.2.3
D1   git diff --name-status --no-renames
       svn/base..refs/mygitclient/deliver/<opid>            ← 净变更清单
D2   按白名单过滤清单                                        ← 双保险
D3   把清单给用户确认（A/M/D 分类、文件数、提交信息）         ← 不可撤销前的关口
     **同时列出被摘掉的 Local-Only commit**                  ← 防漏标，见 §8.2.3
D3.5 再查一次 remoteRev，写进 journal 的 expectedStartRev    ← 压缩竞态窗口 + 查证锚点
D3.8 git checkout --detach refs/mygitclient/deliver/<opid>  ← 磁盘上本地补丁消失。
     phase=checked-out                                         这是全流程**第一次**动
                                                               主工作副本，在用户确认之后
D4   phase=svn-staged
     先记 svnStatusBefore = svn status --xml -- <白名单>      ← 差量的基准，见 §8.5
     A → svn add --parents <files>
     D → svn delete <files>
     M → 无需操作
     再记 svnStatusAfter；svnDelta = After - Before 写进 journal
     ⚠️ 差量里会包含 --parents 带出的**父目录**，回滚要靠它，见 §8.5.1
D4.9 提交前最后一道断言（纯本地，毫秒级）：
       git rev-parse HEAD == journal.deliverRef 指向的 OID
       git status --porcelain -- <白名单> 为空
     任一不满足 → **中止**，按 §8.5 撤销 D4 登记，报「工作副本在你确认之后
     被外部工具改动过」。见 §8.2.5
D5   phase=committing
     svn commit --non-interactive --with-revprop mygitclient:opid=<opid>
       -m <msg> -- <显式 target 列表>
D6   phase=post-commit  ← 过了这一步服务器状态已不可撤销
     svn update --accept postpone -- <白名单路径>            ← 只更新白名单，见 §8.2.1
D7   svn status -v --xml 读新的 nodeRevs → r_new
D8   推进基线到交付树（见 §8.2.2）
D9   提示「work 分支已交付，可删除」——不自动删
     **分支上还有 Local-Only commit 时禁止给出这个提示**，见 §8.2.3
D10  收尾：先跑成功路径自己的断言，再复用 §8.2.4 的 ③ 和 ⑤
       断言：HEAD 在 origBranch 上，且 svn/base..origBranch 只剩
             Local-Only commit（D8d 已经检查过，这里是收尾前复核）
       ⚠️ **不能套用 §8.2.4 的 ④**——那条断言 origHead 未变，而 D8d 刚刚
          推进过 origBranch。见 §8.2.4 末尾
```

**D0.5 到 D3.5 之间的任何退出都走 §8.2.4 的统一收尾**，包括 D1/D2 空清单、D3 用户
取消、D3.5 失败和进程崩溃。这些出口都在 D5 之前，代价为零，但**不清理就会留下临时
ref 和临时 worktree**。

#### 8.2.1 D6 为什么只更新白名单路径

前置检查只要求 **git 工作区**干净，白名单外的文件（美术资产、Unity 生成的 `.asset`）
完全可能有本地修改——§7.2.1 已经说明这是常态，不能阻断。

D5 成功之后服务器状态**已经不可撤销**。此时若再跑一次全量 `svn update`，它会把
别人的改动合并进那些白名单外的本地修改，冲突就留下 `.mine` / `.rN` 文件，而客户端
对这些文件没有任何恢复手段——等于在「已经无法回头」之后又主动引入一类没法收拾的
失败。

所以 D6 用 `svn update -- <白名单路径>`：范围限定在 git 管得住的文件上，这些文件
此刻的内容与服务器 base 完全一致（刚提交完），合并必然干净。

代价是工作副本变成 mixed-revision，美术资产停在旧 revision。这**不是问题**：
§6.6 的检测本来就按白名单节点判定，美术更新交给下一次 `sync_in`（那里有 §7.2.1
的确认流程兜底）。

#### 8.2.2 D8 推进基线：必须显式编排分支

D5 成功之后，磁盘上白名单文件的内容 = **交付树 tip 的内容**（D3.8 已经切
上去了）= 服务器 r_new 的内容。新基线的树就应该是它。

`svn/base` 是交付树的祖先，所以基线本身可以纯快进：

```text
D8a  git branch -f svn/base HEAD        ← HEAD 此刻 detached 在 deliverRef 上；只动指针，
                                           工作目录一个文件都不碰
D8b  git checkout svn/base              ← 与 HEAD 同点，工作目录无变化
D8c  git add -A -- <白名单>              ← 收进 D6 拉下来的他人改动（若有）
     git commit --allow-empty -m "svn: r_new … git-svn-id: <svnUrl>@r_new <uuid>"
D8d  git checkout <ORIG_BRANCH>
     无 Local-Only commit：git merge --ff-only svn/base   ← 纯快进，不可能冲突
     有 Local-Only commit：git rebase --empty=drop svn/base
                                        ← 已交付的 commit 按 patch-id 被丢弃，
                                          只剩本地补丁落到新基线之上
```

D8a 先动指针再 checkout 是关键：**反过来先 `git checkout svn/base` 会把工作目录
退回旧基线内容**，而此刻 SVN 的 base 已经是刚提交的新内容——SVN 会把这看成「你
把自己刚提交的东西全撤了」，正是 §9.2 描述的基线撒谎场景，只不过是自己造出来的。

D8d 的 `--ff-only` 是断言而非优化：**在没有 Local-Only commit 的前提下**走到这里必然
是快进，不是快进就说明前面某步的假设崩了，应当报错停下而不是自作主张 merge。有
Local-Only commit 时工作分支必然不是快进（它带着基线上没有的补丁），断言改由 rebase
之后的检查承担：**`svn/base..ORIG_BRANCH` 里必须只剩 Local-Only commit**，出现任何
没标记的 commit 都说明有该交付的东西没交付出去，报错并导出 journal。

#### 8.2.3 D0.5 构造交付树：把本地补丁摘掉

目标：得到一个「等于 HEAD 但不含任何 Local-Only commit」的 commit，并在 D3.8 把它切到
磁盘上，让 `svn commit` 看到的字节里没有本地补丁（§6.7.2 结尾那条物理事实）。

**两条硬约束**（下面所有设计都是为了同时满足它们）：

1. **D0.5 绝不改动主工作副本的磁盘内容。** 磁盘切换是 D3.8 的事，必须晚于用户确认。
   D0.5 在确认之前就把本地补丁从磁盘上抹掉，等于让用户对着一个已经被改过的工作副本
   做决定，而且 Unity 会在确认对话框还开着的时候开始重编译
2. **D0.5 之后的任何退出路径都必须经过 §8.2.4 的统一收尾。** 构造完成后 journal 里
   多了一个临时 ref（可能还有一个临时 worktree），任何早退如果不清理它们，用户就会
   在下一次交付时撞上残留

**存放位置：`refs/mygitclient/deliver/<opid>`，不是分支 `deliver/<opid>`**

用普通分支名会与用户自己的分支撞车。git 的 ref 是文件系统式的层级命名空间，
`refs/heads/deliver` 和 `refs/heads/deliver/<opid>` 不可能共存：

```text
（实测 2026-09-13，临时 git fixture）
已有 refs/heads/deliver → git checkout -b deliver/abc123
  fatal: cannot lock ref 'refs/heads/deliver/abc123':
         'refs/heads/deliver' exists; cannot create 'refs/heads/deliver/abc123'
已有 refs/heads/deliver → git branch deliver/abc123            同样 fatal
已有 deliver/abc123     → git branch deliver                   反向也 fatal
git pack-refs --all 之后                                       依然 fatal
```

后果是**全部 `sync_out` 在 D0.5 硬失败**（快慢两条路径都要建这个 ref），而且反向也成立：
一次崩溃泄漏出 `deliver/<opid>` 之后，用户从此再也建不出名为 `deliver` 的分支，错误
信息还完全不指向本客户端。

§6.4 给基线选可见分支 `refs/heads/svn/base` 的理由是「IDE 里能看见、能 diff」，那条
理由在这里**不成立**：交付树是单次操作内创建并删除的中间产物，D9 也从不让用户与它
交互。所以放进 `refs/mygitclient/` 内部命名空间，与 §7.6.1 钉 stash 用的
`refs/mygitclient/sync/<opid>` 同一套做法。该命名空间不在 `refs/heads/` 下，与用户
分支不可能冲突。

初始化向导（§6.5）仍应保留 `svn` 这个分支名不被占用——那是 §6.4 已经记过的坑，与本节无关。

**快路径（Local-Only commit 连续位于分支顶端）**

```text
git update-ref refs/mygitclient/deliver/<opid> <最后一个非 Local-Only commit>
```

只是给一个已有 commit 起个名字，**不动 HEAD、不动磁盘、结构上不可能冲突**。客户端应
提供一个「把本地补丁移到分支顶端」的整理操作（本质是一次交互式 rebase 的重排），把
慢路径的风险变成用户可主动规避的。

**慢路径（Local-Only commit 夹在中间）：在临时 worktree 里重放**

```text
git worktree add --detach <tmp> svn/base          ← <tmp> 必须在 SVN 工作副本之外
git -C <tmp> cherry-pick <按序的全部非 Local-Only commit>
git update-ref refs/mygitclient/deliver/<opid> <tmp 的 HEAD>
git worktree remove <tmp>
```

**为什么不在主工作副本上直接 `git checkout -b`**：那会在 D3 确认之前就把磁盘切走，
违反上面第 1 条硬约束；改成「切过去构造完再切回来」倒是能补救，但代价是主工作副本多
翻转两次，按 §7.5 的口径就是**多两次 Unity 全量重编译**，而且全程发生在用户还没决定
要不要交付的时候。临时 worktree 把重放隔离到另一个目录，主工作副本一个字节都不动，
换来的成本只是一份白名单内代码文件的额外拷贝（白名单里没有美术资产，量级是 MB 不是 GB）。

`<tmp>` 路径写进 journal 的 `tempWorktree` 字段，崩溃重开后按它清理；清理用
`git worktree remove --force <tmp>` + `git worktree prune`，且**只允许清理 journal 里
记着的这一个路径**——对齐 `AGENTS.md` §4.3「只清理本会话自己创建的 worktree」。

**冲突必须中止整次交付**：报出是哪个 Local-Only commit 与哪个待交付 commit 打架，
提示用户去拆分或重排，然后 `git -C <tmp> cherry-pick --abort`，再走 §8.2.4 收尾。
**绝不自动解冲突**——此时还没碰 SVN、也没碰主工作副本，中止是零代价的，而猜错的后果
是把本地补丁送上服务器。

**慢路径要求待交付历史是线性的**，这条在 §8.1 #6 前置拦截，理由见下。

两条路径都在 D5 之前，此刻回滚不涉及任何 SVN 状态。

**实测**（2026-09-13，临时 git fixture）：

| # | 场景 | 结果 |
|---|---|---|
| T1 | 摘掉中间的 Local-Only commit，其 hunk 与后续 commit **不重叠** | `cherry-pick` 退出 0；交付树上 log 行恢复为原文 ✅ |
| T2 | 摘掉中间的 Local-Only commit，其 hunk 与后续 commit **紧邻/重叠** | `cherry-pick` 退出 1，`CONFLICT` ⚠️ 必须中止 |
| T3 | 基线推进到交付树后 `git rebase svn/base` | 退出 0；`svn/base..HEAD` **只剩那个 Local-Only commit**，磁盘上补丁回来了，基线里是干净原文 ✅ |
| T4 | 待交付历史里有一个 **merge commit** | `cherry-pick` 退出 128：`is a merge but no -m option was given` ❌ 见 §8.1 #6 |
| T5 | 对 T4 的 merge 补 `-m 1`（其两个父提交已被逐个摘过） | `CONFLICT` —— 改动被重复应用 ❌ 加 `-m` 不是解法 |
| T6 | 慢路径在临时 worktree 里重放并制造 T2 冲突 | 主工作副本**磁盘内容、HEAD、HEAD OID 三者全部未变** ✅ 这是本节第 1 条硬约束的直接证据 |
| T7 | worktree 里挂着半途的 cherry-pick 时直接 `git worktree remove --force` | 退出 0，`git worktree list` 干净；**不需要先 abort，也不需要 `prune`** ✅ |

T3 是稳态的证据：已交付的 commit 被 rebase 按 patch-id 自动丢弃，补丁无需人工搬运。

T4 / T5 是 §8.1 #6 存在的理由：`cherry-pick` 对 merge commit 没有「自动选一条边」的
安全语义。不加 `-m` 直接硬失败；加 `-m 1` 则把 merge 的第一父 diff 整个重放一遍，而那
部分改动在前面逐个摘取父提交时**已经应用过**，必然冲突。两条路都走不通，所以只能在
动手之前拦住。

**D9 不得提示删除带补丁的分支。** 交付后工作分支上仍挂着 Local-Only commit，它们是
**唯一副本**——删掉分支就等于删掉用户攒了几个月的本地补丁。所以 `svn/base..ORIG_BRANCH`
非空时，D9 改为提示「该分支还有 N 个本地补丁，未随交付删除」，并提供「移到专用的
`local/<name>` 分支」入口，而不是「可删除」。

**漏标是这套方案的主要人为风险。** 忘记勾「仅本地」，补丁就当成普通改动交付上去。
两道防护：D3 的确认对话框必须**显式列出被摘掉的 Local-Only commit**（列表为空时也要
说明「本次没有本地补丁被排除」，让用户在预期之外时能察觉）；以及 D3 本来就要展示完整
的 A/M/D 文件清单与 diff 预览。

#### 8.2.4 D0.5 之后所有早退共用一条收尾路径

D0.5 建出临时 ref（慢路径还建过临时 worktree）之后，流程有**六个**出口会在碰 SVN 之前
就结束：D0.5 冲突、D0.5 非冲突失败、D1/D2 空清单（§8.3）、D3 用户取消、D3.5 复查失败、
以及任意时刻的进程崩溃。它们必须走**同一段**收尾代码，而不是各写各的：

```text
cleanup_before_svn(journal):
  ① 若 journal.tempWorktree 存在
       git worktree remove --force <tempWorktree>
       ← 实测：worktree 里挂着半途的 cherry-pick 也能直接 remove，不必先 abort
       ← **绝不跑仓库级的 git worktree prune**：用户可能同时开着 Rider 的
         worktree，prune 会清掉别人的注册项。只按 journal 记着的这一个路径动
  ② 若当前 HEAD 不在 journal.origBranch 上
       git checkout <origBranch>                   ← 只有 D3.8 之后才会发生
  ③ 若 journal.deliverRef 存在且前缀是 refs/mygitclient/deliver/
       git update-ref -d <deliverRef>
  ④ 断言：HEAD == origBranch 且 origHead 未变；不满足则**停下报错并导出 journal**，
     不自作主张继续
  ⑤ 删除 journal；释放锁；恢复 watcher 并发一次 repo-changed
```

每一步都必须**幂等**，因为崩溃恢复会把它重跑一遍：worktree 可能已经不在、ref 可能
已经删掉、HEAD 可能本来就在 `origBranch` 上，这些都不算失败。

④ 的断言是这段收尾的价值所在：它把「用户莫名其妙停在 detached HEAD、本地补丁看起来
消失了」这个最吓人的失败形态挡在门外。`origHead` 在 D0 就写进了 journal，是唯一可信
的对照基准。

D5 之后的失败**不走**这条路径——那时 SVN 侧已经落地，收尾语义完全不同，见 §8.5（它在
清完 SVN 登记之后复用本段的 ②～⑤）。

**④ 只属于早退路径，成功路径不能套用。** 早退的定义就是「什么都没发生过」，所以
`origHead 未变` 是对的不变量。但走到 D10 时 D8d 刚刚用 ff-only 或 rebase 把
`origBranch` 推进到了新基线之上，`origHead` **必然**已经变了——把 ④ 原样套到 D10 上，
每一次**成功**的交付都会在最后一步报错，然后留下 journal、锁和 watcher 不收。

所以两条路径的断言必须分开：

| 路径 | 断言什么 | 为什么 |
|---|---|---|
| 早退（§8.2.4 ④） | `HEAD == origBranch` 且 `origHead` **未变** | 什么都不该发生过 |
| 成功（D10） | `HEAD` 在 `origBranch` 上，且 `svn/base..origBranch` 只剩 Local-Only commit | 该发生的都发生了，且没有漏交付 |

共用的只有 ③（删 `deliverRef`）和 ⑤（删 journal、放锁、恢复 watcher）。①（临时
worktree）在成功路径上早已在 D0.5 收尾时清掉，②（切回 `origBranch`）由 D8d 承担。

#### 8.2.5 D4.9 为什么必须有：确认之后磁盘仍然可能被改

D3.8 到 D5 之间有一个无法用锁关上的窗口。§5.1 的独占锁是**进程内**的——它拦得住
mygitclient 自己的 stage / commit / checkout，拦不住 Rider、Visual Studio 或命令行
git。而 §2.1 选 Overlay 的核心理由恰恰就是「让 IDE 的 git 集成白送可用」，所以这不是
理论风险，是设计主动换来的。

具体怎么出事：

```text
D3   用户看着清单点了确认
D3.8 磁盘切到 deliverRef（detached），本地补丁从磁盘上消失
     ↓ 用户切到 Rider，看见「detached HEAD」觉得不对劲，顺手 checkout 回 work
     ↓ 本地补丁**重新回到磁盘**
D4   svn add / delete —— 只改登记，不看内容，毫无察觉
D5   svn commit -- <targets>  ← 提交的是**磁盘当前的字节**，补丁跟着上服务器
```

结果正是 §6.7 整节要防的那件事：注释掉的 log、临时调试代码被交付给全组，而且
`svn commit` 一旦落地就不可撤销。§9 的失同步检测也救不了——它按 revision 号判定，
而这次提交的 revision 号完全正常。

D4.9 把这个失败模式从「静默提交错误内容」降级成「中止并报错」。代价是两条纯本地的
git 命令，毫秒级。放在 D4 **之后**而不是之前，是为了让窗口尽可能窄：D4 的
`svn add --parents` 要逐个路径跑，本身就有耗时。

**它消除不了竞态本身**——断言通过之后、`svn commit` 读盘之前仍有一个更窄的窗口。
真要根治，交付得从一个隔离的临时 SVN 工作副本里做（把 `deliverRef` 的内容导出到那里
再提交），那是另一个量级的改动，记为 §15 第 15 条已知限制。

### 8.3 空清单必须中止

D1 / D2 任一步得到空清单立即中止，**绝不进入 D5**。

空 target 的 `svn commit` 会提交整个工作副本——包括几十 GB 美术里任何一个被误改
的文件，而且**不可撤销、直接影响全组**。这是本方案后果最重的失败模式，与
`AGENTS.md` §6.2 记录的空 pathspec 教训同源但更严重（SVN 提交直接上服务器）。

代码里有独立断言 + 专门测试用例，见 §14.2。

### 8.4 竞态窗口

前置检查里的 `remoteRev == baselineRev` 只是**快速失败的优化，不是保证**。真正的窗口在
D3 的确认对话框——用户可能开着它看几分钟，同事完全有时间提交两三次。

**服务端的检查才是唯一真正的门。** D3.5 再查一次能砍掉这个大窗口，但消除不了竞态
（网络往返期间仍可能有人提交），所以 D5 的失败必须当作常规路径处理好。

### 8.5 D5 失败后的回滚

救命的性质：**SVN 提交在服务端是原子的**——要么整个 revision 落地，要么什么都没
发生。不存在「提交了 3 个文件、第 4 个失败」的半吊子状态。这让失败处理从灾难恢复
降级成清理本地登记。

D4 的两类登记，撤销方式**不对称**：

| D4 做了什么 | git 侧的事实 | `svn revert` 的效果 | 够不够 |
|---|---|---|---|
| `svn add NewFile.cs` | git 有这个文件 | 取消 add 登记，文件留在磁盘 | ✅ |
| `svn delete Old.cs` | git **没有**这个文件 | 取消 delete 登记，**把文件从 base 恢复到磁盘** | ❌ 磁盘凭空多出一个文件 |

第二行是真实的坑：回滚后工作目录冒出一个 `Old.cs`，git 显示为未跟踪文件；随后跑
更新流程时 `git stash push -u` 会把它 stash 走，`git checkout svn/base` 又会把它
作为跟踪文件拉回来，状态越滚越乱。

**回滚的不变量**：工作目录和 `.svn` 的登记都必须回到 D4 之前的样子。

**撤销的依据是 D4 前后的 `svn status` 差量，不是「我打算 add 哪些文件」的清单。**
理由见 §8.5.1：`svn add --parents` 会连带 schedule 未版本控制的父目录，那些目录不在
任何文件清单里。

```text
D4 之前：svnStatusBefore = svn status --xml -- <白名单>        ← 写进 journal
D4 之后：svnStatusAfter  = svn status --xml -- <白名单>        ← 写进 journal
svnDelta = svnStatusAfter - svnStatusBefore                    ← 本流程新增的全部登记

撤销时遍历 svnDelta，**按路径深度从深到浅**（子 → 父）：
  新增的 A 登记（文件或目录） → svn revert <path>
  新增的 D 登记              → svn revert <path>，然后删掉磁盘上被恢复出来的那个文件
                               （用显式文件删除，不用 git clean —— 那条命令语义太宽）
  A 登记被 revert 之后文件会留在磁盘上变成 ?，这是对的：它本来就是 git 侧的新文件

撤销完断言：svn status 与 svnStatusBefore 逐条一致（不只是「git status 没变」）
然后走 §8.2.4 的统一收尾（切回 origBranch、删 deliverRef、删 journal、
放锁、恢复 watcher）
```

逐条精确撤销，不用任何批量恢复命令。从深到浅是必须的：先 revert 父目录会让子路径的
登记状态变得不确定。

**回到 `origBranch` 这一步不能省。** D3.8 之后会话是 detached 在 `deliverRef` 上的，
D5 失败若只清 SVN 登记就收手，用户会发现自己莫名其妙处于 detached HEAD、本地补丁
凭空消失——而补丁其实好好地躺在 `origBranch` 里。`origBranch` 与 `deliverRef` 在
**D0.5 就写进了 journal**（早于任何 ref 操作），崩溃重开后同样按这两个字段收尾。
`refs/mygitclient/deliver/<opid>` 是本流程自建的内部 ref，删它不需要额外授权；但
**删除前必须断言它确实在 `refs/mygitclient/deliver/` 前缀下**，别的 ref 一律不碰。

#### 8.5.1 `svn add --parents` 会带出父目录的登记

实测（2026-09-13，真实 SVN fixture）：

```text
svn add --parents Assets/Scripts/Deep/Nested/New.cs
  A  Assets\Scripts\Deep            ← 未版本控制的父目录，被一并 schedule
  A  Assets\Scripts\Deep\Nested     ← 同上
  A  Assets\Scripts\Deep\Nested\New.cs

只按「svnAdded 里的文件」revert（原设计的字面做法）：
  svn revert Assets/Scripts/Deep/Nested/New.cs
残留：
  A  Assets\Scripts\Deep            ← 没撤掉
  A  Assets\Scripts\Deep\Nested     ← 没撤掉
  ?  Assets\Scripts\Deep\Nested\New.cs
```

两条 `A` 目录登记留在 `.svn` 里，直接违反本节开头那条不变量。后果不只是「不干净」：
下一次交付的 D4 会在一个已经带着陌生登记的工作副本上开始，`svnDelta` 算出来的差量不再
等于本次流程的改动，回滚的精度从此失效——**一次没撤干净，之后每一次都不可靠**。

这也是为什么断言要比 `svn status` 而不是 `git status`：git 完全看不见 `.svn` 里的登记，
拿 git 的干净当证据是**虚假信心**。

### 8.6 D5 的五类失败

| 失败 | 典型错误码 | 服务器状态 | 处理 |
|---|---|---|---|
| out-of-date（同事抢先提交） | E155011 / E160028 | 无变化 | 撤销 D4 → 提示先更新 → **保留用户输入的提交信息** |
| 认证失败 | E170001 | 无变化 | 同上，原样展示 stderr |
| 服务端钩子拒绝 | E165001 | 无变化 | 同上，原样展示 stderr |
| 路径权限不足（SVN authz 按路径配） | E170001 | 无变化 | 同上。原子性保证：一个路径没权限则整个提交被拒 |
| **传输中途断网** | 连接重置 / 超时 | **未知** | 见下 |

传输中断最棘手：数据传完之后、回执送达之前断线，客户端不知道服务器到底提交成功
没有。盲目回滚是错的——如果实际成功了，撤销 D4 会导致下次交付把同样内容再提交
一遍。

#### 8.6.1 查证必须靠幂等键，不能靠「作者 + 提交信息」

「查最近 5 条里有没有作者是你、message 相同的 revision」两个方向都会错：

| 误判 | 场景 | 后果 |
|---|---|---|
| 假阳性 | 你之前提交过同样的 message（重试过、或用了 `修复编译` 这类通用信息） | 把**真失败**当成功，跳过回滚并推进基线 → 基线撒谎 + `.svn` 里挂着未撤销的登记 |
| 假阴性 | 断网时间长，服务器期间新增了超过 5 个 revision | 把**真成功**当失败，执行回滚 → 同样内容被重复交付一遍 |

两者都会直接损坏本设计的核心不变量。所以查证必须建立在**唯一标识**上。

#### 8.6.2 查证流程

D3.5 / D4 已经把以下内容写进 journal（§7.6），它们是查证的全部依据：

- `opid`：本次交付的 UUID
- `expectedStartRev`：D3.5 读到的 remoteRev，即「提交前服务器的 HEAD」
- `svnAdded` / `svnDeleted` + M 清单：本次提交的完整路径集合

```text
D5 失败且错误属网络类（连接重置 / 超时 / 回执缺失）
   ↓
恢复连通后，查 expectedStartRev+1 .. HEAD 的**完整区间**（不是「最近 N 条」）
   svn log --xml -v --with-all-revprops -r <expectedStartRev+1>:HEAD <URL>
   ↓
① revprop mygitclient:opid == 本次 opid
   ├─ 命中 → **判定成功**。不回滚，记下命中的 revision，直接跳到 D6
   └─ 区间内无任何 revision 带 opid，且服务器确认支持 revprop（§8.6.3 的开仓自检）
         → **判定失败**，按 §8.5 回滚
   ↓（服务器不支持 revprop，只能走兜底）
② 作者相同 且 changed-paths 的**文件部分**与本次清单一致
   ⚠️ 只能得出「疑似命中」，**不足以单独判定成功**，见下
   ├─ 疑似命中 → 再校验内容：svn diff --summarize -c <疑似 revision> 的路径集合
   │              与 deliverRef 树对比；逐个 svn cat 校验白名单文件的内容
   │              与 deliverRef 一致
   │      ├─ 内容一致 → 判定成功，跳到 D6
   │      └─ 内容不一致 → 判定**状态未知**，不回滚也不推进
   └─ 无疑似命中 → 判定失败，按 §8.5 回滚
   ↓
仍连不上，或判定为「状态未知」
   → 什么都不做，phase 停在 committing，
     界面常驻「交付状态未知」，列出疑似 revision 供人工核对，
     等网络恢复或用户确认后再走一次
```

查**完整区间**而不是「最近 N 条」是硬要求：断网可能持续几小时，N 取多大都是猜。
区间的起点来自提交前就已落盘的 `expectedStartRev`，不依赖任何事后推断。

**兜底 ② 为什么只能是「疑似」**

§8.6.1 已经否掉了「作者 + message」，理由是同一个用户可能提交过同样的 message。
「作者 + changed-paths」有**同样的结构性弱点**，只是窄一些：用户在 D5 断网之后，
完全可能自己打开 TortoiseSVN 把同一批文件提交一遍——路径集合会**完全一致**。此时
本次 D5 其实失败了，兜底却判成功，于是跳过回滚、直接推进基线，`.svn` 里还挂着未撤销
的 D4 登记。这正是 §8.6.1 那张表里「假阳性」那一行的后果，一个字都不差。

所以路径集合只能用来**缩小候选**，判定成功必须再看内容。内容比对是有意义的：
`deliverRef` 是一个固定的 git 树，它每个白名单文件的字节都是确定的，拿服务器该
revision 的内容逐个比对是**可判定**的，不依赖任何关于用户行为的假设。

**比对前必须先剔除目录路径。** 实测（2026-09-13，真实 SVN fixture）：

```text
svn add --parents Assets/Scripts/Deep/Nested/New.cs 之后提交，svn log -v 给出：
   A /Assets/Scripts/Deep          ← 目录
   A /Assets/Scripts/Deep/Nested   ← 目录
   A /Assets/Scripts/Deep/Nested/New.cs
```

git 不跟踪空目录，所以本次交付的文件清单里只有 `New.cs`。不剔除目录就拿去比「完全
一致」，**任何涉及新增嵌套目录的交付都会判定为不命中**——把真成功当失败，重复交付
一遍。这是假阴性，与假阳性同样致命。

#### 8.6.3 关于 revprop 幂等键

`svn commit --with-revprop mygitclient:opid=<uuid>` 在提交时写入自定义 revprop。

**实测确认**（2026-09-13，SlikSvn 1.14.2 + 全新 `svnadmin create` 仓库，**未配置任何
钩子**）：

```text
svn commit --with-revprop mygitclient:opid=abc-123 -m test -- a.txt
  → Committed revision 1.                                        ✅ 提交成功
svn propget --revprop -r 1 mygitclient:opid <URL>
  → abc-123                                                      ✅ 写进去了
svn log --xml -v --with-all-revprops -r 1 <URL>
  → <property name="mygitclient:opid">abc-123</property>          ✅ 查证读得到

对照组——事后修改 revprop 才是受钩子限制的那个：
svn propset --revprop -r 1 mygitclient:opid xyz <URL>
  → svn: E165006: Repository has not been enabled to accept revision
    propchanges; ask the administrator to create a pre-revprop-change hook
```

所以 §8.6 的设计前提成立：`pre-revprop-change` 管的是**事后修改**，提交时携带 revprop
不受它限制。

**但公司服务器仍可能另有限制**（自定义 `pre-commit` 钩子拒绝未知 revprop、或走某种
代理）。这里有两种完全不同的失败形态，必须分开处理：

**形态 A：提交成功，但 revprop 被悄悄丢掉。** 开仓自检能抓到：

```text
第一次交付成功之后（D6 之前，此时 revision 号已知）：
   svn propget --revprop -r <r_new> mygitclient:opid <URL>
   ├─ 读回本次 opid → 记 supportsRevprop=true 进 .git/mygitclient.json
   └─ 读不到      → 记 supportsRevprop=false，一次性告知
                     「本仓库不支持提交幂等键，断网查证精度降低」
```

**形态 B：提交直接被钩子拒绝。** 这一种**开仓自检永远跑不到**——D5 根本没成功过，
`supportsRevprop` 也就永远写不进去。原来那句「客户端降级」在这种服务器上是空话：
每一次交付都会失败，用户被卡死。

形态 B 必须单独作为一类 D5 失败处理：

```text
D5 失败，且 stderr 指向 revprop / pre-commit 拒绝（而非网络类）
   ↓
按 §8.5 完整回滚 D4 登记（服务器侧什么都没发生，这是确定的）
   ↓
把 svn 的原始 stderr 原样展示给用户，并给出一个**需要用户明确点确认**的选项：
   「本服务器拒绝提交幂等键。不带幂等键重试一次？
     代价：断网时无法用幂等键查证，只能靠路径集合 + 内容比对（§8.6.2 兜底）」
   ├─ 用户确认 → 不带 --with-revprop 重试一次，成功后记 supportsRevprop=false
   └─ 用户不确认 → 停在这里，什么都不做
```

**这不违反 §8.7。** §8.7 禁止的是**自动**重试、以及「顺手修一下再试一次」。这里是把
失败原因原样摆出来、把代价说清楚、由用户决定要不要换一种方式再来一次——正是 §8.7
那句「一律停下来把事实摆给用户看」要求的行为。区别在于**谁做的决定**。

因此实现上：revprop 是**首选**路径，changed-paths + 内容校验是**必须存在的兜底**，
而形态 B 的用户确认重试是**让兜底真的可达**的那一步。

### 8.7 绝不自动重试

任何一类失败都不重试、不「顺手修一下再试一次」，一律停下来把事实摆给用户看。
直接对齐 `AGENTS.md` 开篇那条：破坏性命令失败或表现异常后必须立即停止，禁止现场
修正、重试或追加命令。`svn commit` 正是该规则指向的那类命令。

## 9. 外部改动检测与基线恢复

### 9.1 场景

用户在 TortoiseSVN 里对整个工作副本点了「SVN Commit」，此时本地有 3 个 git
commit、1 个新增文件、1 个未提交改动。

TortoiseSVN 的提交对话框会显示：

```text
M  Assets/Scripts/Player.cs        ← 3 个 commit 叠加后的最终内容
M  Assets/Scripts/PlayerState.cs   ← 没写完的改动
?  Assets/Scripts/JumpConfig.cs    ← 未版本控制，默认不勾选
?  .git                            ← 未版本控制，默认不勾选
```

### 9.2 后果分三层

**① 数据层面：无损失。** 这是本方案的结构性优点。`.git` 一个字节没动——commit、
分支、stash、reflog 全在。`svn commit` 也不改工作目录的文件内容，它只做两件事：
上传 diff、把 `.svn` 的 base revision 从 12345 改成 12346。

**② 社交层面：半成品上线了。** 那个没写完的改动全组都会拉到。客户端救不了，只能
靠及时发现 + 补一次提交。

**③ 记账层面：这才是设计必须处理的。** 客户端认为基线是 r12345，`.svn` 实际已是
r12346，而 r12346 的内容 = 用户工作分支的当前状态。若察觉不到，下次更新流程会：

```text
git checkout svn/base   → 磁盘回到 r12345 的内容
svn status              → 显示一堆「修改」（实为把 r12346 撤回 r12345）
svn update              → 服务器没新东西，无事发生
git add && git commit   → 落「新基线」，内容是旧的 r12345，却打上 …@12346
                          ⚠️ 基线从此撒谎
后续交付                → 净变更含「把已提交过的内容再提交一遍」
                          → 可能覆盖别人在 r12346 之后的改动
```

**真正的风险不是那次误提交，而是它之后基线静默错位、几天后覆盖掉同事的代码。**

### 9.3 检测

按 §6.6.2 的两条判据比对 `nodeRevs` 与 `baselineRev`。检查点：打开仓库时、每次
`repo-changed` 刷新时（先过 §6.6.3 的 `rootRev` 变化提示）、更新 / 交付的第一步。
`svn status` 纯本地读、不联网，成本可忽略。

同一条检查能抓住命令行 `svn update`、TortoiseSVN 的 update 和 commit——这些都推进
revision。

**但它抓不到 `svn revert`。** 实测（2026-09-13，真实 SVN fixture）：

```text
revert 前: rootRev=1  nodeRevs={1}
（改文件，然后 svn revert -R）
revert 后: rootRev=1  nodeRevs={1}     ← 完全没变
```

`svn revert` 只改磁盘内容，不动任何 revision，所以 `nodeRevs == baselineRev` 照样
成立。**这类外部操作只能由 git 侧看见**：`svn revert` 把文件改回服务器 base 内容，
在 git 那边表现为工作区出现修改（相对当前分支）或者「改动凭空消失」。

所以检测分两条独立的路，不能互相冒充：

| 外部操作 | 会不会推进 revision | 靠什么发现 |
|---|---|---|
| `svn update` / `svn commit`（含 TortoiseSVN） | 会 | §6.6.2 的 `nodeRevs` 判据 |
| `svn revert` | **不会** | git 工作区状态——现有的 watcher + `git status` 已经能报出来，UI 上表现为「改动被外部还原」 |

第二条不需要新机制，但**必须写明它不归失同步检测管**，否则容易误以为那道门禁什么都
拦得住。

### 9.4 恢复流程

```text
⓪ 写 journal（flow=recover），phase=start            ← 任何修改之前，见 §7.6
① git stash push -u -- <白名单>；立刻取 OID 并用
   refs/mygitclient/sync/<opid> 钉住，写进 journal   保护未提交改动，可逆
   （S1 那套写前顺序同样适用，见 §7.3.1）
② 原分支与 origHead 写进 journal
③ git checkout svn/base                 磁盘 = 旧基线内容
④ 分两步定出要动的东西，见 §9.4.1
   ④a svn status -v --xml --no-ignore -- <白名单>，按状态分类：
        (a) M / !            → svn revert   恢复服务器 base 内容
        (b) R（replace）     → svn revert   同上
        (c) A / A+（外部登记）→ svn revert   取消登记
   ④b 「服务器已删除」的集合**不看 svn status 的 ? 条目**，改用两个集合作差：
        oldBaselineFiles = git ls-tree -r --name-only svn/base -- <白名单>
        svnVersioned     = ④a 里 SVN 仍在版本控制的路径集合
        toDelete = oldBaselineFiles - svnVersioned      ← 全是**具体文件**，没有目录
   ⚠️ 每一类都独立判空：列表为空则跳过，绝不执行空参数的 svn 命令或删除
⑤ 按 ④ 的结果执行：(a)(b)(c) 用显式文件列表 svn revert；
   toDelete 逐个**文件**删除，**绝不递归删目录**；全部删完后可以清理变空的目录本身
⑥ git add -A -- <白名单> && git commit --allow-empty
                                        新基线，rev 取自 ⑤ 之后重读的 nodeRevs
⑥b 基线体检（§6.8）                     此刻 HEAD 在 svn/base、工作区干净，正是
                                        体检的前提。有差异则停在这里报错，
                                        **不继续 ⑦**——恢复流程的全部目的就是修好
                                        基线，不验证等于没修
⑦ git checkout <原分支>
   git rebase --empty=drop svn/base      已被误传上去的 commit 会变成空 commit，自动丢弃
⑧ git stash apply <OID>；成功后 drop 该条目并删掉钉住的 ref
⑨ 删除 journal
```

#### 9.4.1 为什么必须按「版本控制集合的变化」分类，而不是只看 M / !

原设计只收集 `M` / `!` 两种状态。**外部提交删除了白名单内的文件时，这套枚举会整个
漏过去**，而且后果是把服务器上已经不存在的文件写进标着新 revision 的基线。

实测（2026-09-13，真实 SVN fixture）：

```text
同事 svn delete Assets/Scripts/B.cs 并提交
用户在 TortoiseSVN 里点 update   → B.cs 从磁盘消失，SVN 不再版本控制该路径
③ git checkout svn/base          → 旧基线把 B.cs 拉回磁盘
④ svn status                     → ?  Assets\Scripts\B.cs
                                     ↑ 不是 M，也不是 ！
```

于是 ⑤ 不 revert，⑥ 的 `git add -A` 把它写进新基线。这个文件之后会**永久滞留**：
基线和工作分支都有它，交付的净变更 diff 里永远不会出现，客户端也就永远不会告诉用户
「你这里有个服务器上早就删掉的文件」。

**为什么删除集合要从旧 git tree 反推，而不是用 `svn status` 的 `?`**

第一版这里写的是「`?` → 从磁盘删除」，论证是「① 的 `git stash push -u` 保证磁盘上
白名单内的文件 = 旧基线内容，一个不多一个不少，所以每个 `?` 必然是服务器删掉的」。

**这个论证是错的，前提不成立。** 两处实测（2026-09-13，真实 SVN + git fixture）：

```text
场景：服务器删掉整个 Assets/Scripts/Feature 目录，
      用户在该目录下有一个 git-ignored 的 local.log

① git stash push -u -- <白名单>
     local.log 仍在磁盘  ← -u 只收未跟踪文件，**不收 ignored 文件**（那要 -a）
③ git checkout svn/base
     磁盘: F1.cs  F2.cs  local.log
④ svn status
     ?  Assets\Scripts\Feature     ← 只有一个**目录**条目，不是逐个文件
```

两个洞叠在一起：

1. **`-u` 不收 ignored 文件**，所以「磁盘 = 旧基线内容」不成立，磁盘上可能有 git
   完全不管、stash 也没带走的用户数据
2. **SVN 按目录汇报**，`?` 条目可能是一个目录。按目录递归删就会连带删掉上面那个
   `local.log`——那是用户的文件，不是服务器删掉的东西

所以删除集合必须从**旧基线的 git tree** 反推（④b）：`git ls-tree -r` 给出的全是**受
git 管理的具体文件**，一个目录都没有。差集里的每一条都同时满足：

- 它在旧基线里 —— 所以它确实是「曾经的服务器内容」
- SVN 现在不再版本控制它 —— 所以服务器把它删了

这两条推出的结论只有一个：删掉它是把磁盘对齐到服务器。而 `local.log` 这类文件**根本
不在 `git ls-tree` 的输出里**，因此永远不会进入删除集合——不管它躺在哪个被删掉的目录
底下。

**绝不递归删目录**这条要写死在实现里：删完文件之后若某个目录变空，删空目录本身是安全
的（git 不跟踪空目录，SVN 也已经不版本控制它）；但「删目录及其全部内容」这个动作在
本流程里永远没有正当理由。

用户自己新建、还没交付的 `.cs` 文件同样不受影响：它不在旧基线的 tree 里，所以不在差集
里；它在 ① 被 `-u` 收进 stash，第 ⑧ 步原样还回来。

`A` 登记（(c) 类）单独列出来的理由：`svn revert` 只取消登记，文件会留在磁盘上。它是否
该被删，由 ④b 的差集独立判定——如果它本来就不在旧基线里（用户新建的），差集里没有它，
它就留在磁盘上，这是对的。

### 9.5 第 ⑤ 步为什么安全

涉及破坏性命令，论证必须显式：

- 只作用在白名单内的文件上，美术资产一个都不碰
- 此刻这些文件的内容 = `svn/base` 分支的内容，而那个 commit 就在 git 对象库里，
  随时能取回，因此**可逆**
- 目标是从 `svn status` **显式枚举**出来的文件列表，不是 `svn revert -R <目录>`
  ——递归 revert 会波及同目录下白名单外的文件
- 空列表短路，不执行
- **本地补丁不会被误伤**：第 ③ 步已经切到 `svn/base`，Local-Only commit 活在工作分支
  上、不在磁盘上（§6.7.1），第 ④ 步的 `svn status` 根本看不到它；第 ⑦ 步的 rebase 再
  把它搬回来。若当初允许补丁以未提交改动的形态存在，这里就会把它当成「被外部改脏的
  文件」直接 revert 掉，且不可逆

## 10. 混合图

### 10.1 SVN 没有本地历史

工作副本里只有 base 版本的 pristine 副本，没有任何过往 revision 的记录。所以
`svn log` **每次都走网络**，哪怕只查最近 10 条。

| | Git 模式 | SVN 模式 |
|---|---|---|
| 历史来源 | 本地对象库，毫秒级 | 服务器，几百毫秒到几秒 |
| 离线可用 | 完全可用 | 只能看缓存 |
| 加载方式 | 一次性全量 | 分页 + 增量 + 缓存 |

### 10.2 缓存

`.git/mygitclient-svnlog.json`。SVN 历史不可变（除非有人改 revprop，罕见到可忽略），
已拉取的 revision **永久缓存**，只增量拉新的：

```text
缓存最大 revision = 12340，服务器 HEAD = 12360
  → 只拉 svn log -r 12360:12341
```

打开仓库时**先用缓存把图画出来**，后台异步拉新的，拿到再刷新——对齐现有
`refreshPullRequests` 的「不 await、失败静默」模式。拉不到就在角落挂一个低调的
「SVN 历史可能不是最新」标记，**不弹错误横幅**。离线时本地 git 部分照常可用。

分页第一期做简单版：首屏最近 200 条，底部「加载更多」按钮。不做无限滚动。

### 10.3 两条历史的接合

```text
① 从缓存拿 SVN 序列，节点 ID = "svn:12346"，parents = ["svn:12345"]

② git log --all 拿本地全部 commit
   凡 message 带 `git-svn-id: <url>@N <uuid>` 的（baseline commit）
     → 建立映射 <git hash> → "svn:N"

③ 重写所有 git commit 的 parents，按映射表替换
   baseline commit 自身被替换掉，其 git parent 关系丢弃
   （它在图上的位置由 SVN 序列决定）
```

结果：工作分支第一个 commit 的 parent 本指向 baseline commit，替换后自然指向
`svn:12346` 节点，分支就挂上去了。

```text
                        ● work tip
                        │
                        ● 修空引用
                        │
   ● svn:12348 ─────────┤
   │                    │
   ● svn:12347          │
   │                    │
   ● svn:12346 ─────────●  加跳跃
   │
   ● svn:12345
```

**边界：孤儿 baseline。** 若某个 baseline 的 revision 太老、不在已拉取范围内，
`svn:N` 节点就没有主干可挂。处理办法是**强制补拉**——baseline commit 的 revision
列表已知且有限（几十个），拉取时显式包含。

### 10.4 数据模型改动

```ts
export interface CommitNode {
  id: string            // git: oid；svn: "svn:12346"
  parents: string[]
  author: string
  time: number
  message: string
  vcs?: 'git' | 'svn'   // 新增；缺省视为 git，现有 Git 模式零影响
  svnRev?: number       // 新增；仅 svn 节点有
  localOnly?: boolean   // 新增；message 带 Local-Only trailer（§6.7），不会被交付
}
```

`engine/layout.ts`、`engine/svgRenderer.tsx` **一行不改**——它们只认 `parents`。

## 11. 前端改动

| 组件 | 变化 |
|---|---|
| `GraphView` / `layout.ts` / `svgRenderer.tsx` | **零改动** |
| `DiffView` / `FileTree` | **零改动**（就是 git status / git diff） |
| `LocalChangesView` | 文件列表与 diff 零改动；只有提交框多一个勾选框，见下 |
| `StashView` | 近乎零改动 |
| `BranchView` | 基线分支加特殊标记 + 禁止在其上提交 |
| `Toolbar` | 按 `capabilities`：Fetch / Pull / Push → Update / Deliver；隐藏 Worktree / Tag / PR |
| `CommitRow` | `vcs === 'svn'` 时显示 `r12346` 而非短 hash，换配色；`localOnly` 加「仅本地」角标（§6.7） |
| `LocalChangesView` 的提交框 | 新增「仅本地，不交付」勾选框，勾上即在 message 追加 `Local-Only: true` trailer |
| `CommitDetailView` | svn 节点的 diff 走 `svn diff -c N`（**需要网络**）。第一期只对白名单内路径显示内容，美术资产只列文件名和状态 |
| 新增 `SvnExternalView` | 白名单外的 svn 改动，只读列表 + revert 入口 |
| 新增 `SyncDialog` | 更新 / 交付流程 UI，含冲突解决引导与交付确认关口 |
| 新增 `BaselineBanner` | 失同步 / 同步未完成 / 交付状态未知 |
| 新增初始化向导 | 白名单编辑 + 首个基线 |

`store/repoStore.ts` 按 `kind` 分流 invoke，新增 `capabilities` 字段。纯展示逻辑
（capabilities → 按钮显隐、svn / git 节点标签格式化）下沉到 `src/engine/`，保持无
React 依赖、可直接单测。

## 12. 安全边界

本方案的破坏力集中在三处，每处都有结构性防护。

### 12.1 白名单外的文件绝不进入任何命令参数

所有构造 git / svn 参数的地方走**同一个过滤函数**，然后专门测那个函数。这不是
「测一下功能」，是给整个方案的安全边界上一道断言。

### 12.2 空参数一律拒绝

| 位置 | 空参数的后果 |
|---|---|
| D5 `svn commit` | 提交整个工作副本，**不可撤销，影响全组** |
| §9.4 ⑤ `svn revert` | 恢复整个工作副本，吃掉全部本地改动 |
| S7 / D1 的路径列表 | 基线或净变更算错 |

每处独立断言 + 独立测试用例。

### 12.3 防 `.git` 被误提交

最坏情况：在 TortoiseSVN 里勾了「显示未版本控制文件」并全选，`.git` 整个对象库
（几百 MB）被传上服务器，全组下次 `svn update` 都会拉到一坨。恢复要 `svn delete`
+ 提交，而且**服务器历史里永远留着**——SVN 没有 `filter-branch`。

三层防护：

1. 打开仓库时跑一次 `svn status`，发现 `.git` 处于已 add / 已版本控制状态 →
   红色横幅阻断
2. 首次配置时询问是否往 `~/.subversion/config` 的 `global-ignores` 加 `.git`
   （改用户全局配置，**必须问**）
3. 在 `.git` 旁放一个 `DO-NOT-COMMIT-THIS.txt` 说明文件，给 TortoiseSVN 的误操作
   留一个人眼可读的提示

### 12.4 失败只回滚自己

所有失败路径**只回滚本流程自己做的修改**，绝不「顺手修复」发现的其他问题。
对齐 `AGENTS.md` 开篇的破坏性命令规则。

## 13. 失败路径总表

| 失败点 | 原因 | 回滚到 | 用户看到 |
|---|---|---|---|
| S3 `git checkout` 失败 | **Unity 开着锁住了 .cs / .dll** | `git stash apply <OID>` | 「Unity.exe (PID 1234) 占用了 X 文件」——复用现有 `engine/file_lock.rs` |
| S4 校验不通过 | 外部改过工作副本 | 切回 ORIG_BRANCH + stash apply | 基线失同步，进 §9.4 恢复向导 |
| S5 网络 / 认证失败 | VPN 没连、凭据过期 | 切回 ORIG_BRANCH + stash apply | 原样展示 svn stderr |
| S5 树冲突 | 服务器删了 / 换掉了某个目录 | 同上 | 明确说明是树冲突，给出受影响路径 |
| S5 白名单**外**文件冲突 | 美术资产有本地修改（§7.2.1 已确认过） | 同上 | 明确指出是 git 管不到的文件，**引导去 TortoiseSVN**，不假装能修 |
| S9 rebase 冲突 | 正常的代码冲突 | **不回滚**，停在冲突态 | 进入冲突解决模式 |
| S10 stash apply 冲突 | 未提交改动与新基线冲突 | 不回滚，**stash 与钉住的 ref 都保留不删** | 同上 + 给出 stash 的 OID |
| D5 各类失败 | 见 §8.6 | 见 §8.5 | 见 §8.6 |
| **D6 update 失败 / 冲突** | 服务器在 D5 之后又有提交，且与白名单文件冲突 | **不可回滚**——D5 已落地 | 明确告知「提交已成功、基线尚未推进」，journal 停在 `post-commit`，提供「重试 D6 起」的入口 |
| **D0.5 前置：待交付历史含 merge commit** | 用户在 IDE 里 merge 过，且 Local-Only commit 夹在中间需走慢路径（§8.1 #6） | **不做任何修改**——检查在动手之前 | 指出是哪个 merge commit，引导把补丁移到顶端（走快路径）或线性化历史 |
| **D0.5 cherry-pick 冲突** | Local-Only commit 与待交付 commit 改到相邻/同一处（§8.2.3 T2） | 在临时 worktree 里 `cherry-pick --abort`，然后走 §8.2.4 统一收尾，**零代价**（还没碰 SVN，主工作副本未动） | 指出是哪两个 commit 打架，引导拆分或把补丁移到分支顶端 |
| **D0.5 非冲突失败**（worktree 建不出、磁盘满、ref 写不进） | 环境问题 | 同上，走 §8.2.4 统一收尾 | 原样展示 git stderr，不猜原因 |
| **D1 / D2 空清单** | 净变更为空（§8.3） | 走 §8.2.4 统一收尾 | 说明「没有可交付的净变更」，并提示是否全部改动都被标成了 Local-Only |
| **D3 用户取消** | 用户看了清单决定不交付 | 走 §8.2.4 统一收尾 | 正常退出，不报错；保留用户输入的提交信息 |
| **D3.5 复查 remoteRev 失败 / 已变化** | 网络问题，或同事在确认对话框期间提交了（§8.4） | 走 §8.2.4 统一收尾 | 提示先更新再交付，**保留用户输入的提交信息** |
| **D4.9 断言失败** | D3.8 之后有外部 git 工具动过工作副本（§8.2.5） | 按 §8.5 撤销 D4 登记，再走 §8.2.4 收尾 | 明确告知「工作副本在你确认之后被改动过」，列出 HEAD 与期望值的差异，**保留提交信息**，引导重新走一次 |
| **初始化前置未过** | 工作副本脏或 mixed-revision（§6.5） | **不做任何修改**，不创建 `.git` | 按类型分别提示：有本地修改 → 去 TortoiseSVN 提交或还原；mixed-revision → 先跑一次完整 `svn update` |
| **前置检查发现 detached HEAD** | 用户在 IDE 里切到了某个历史 commit（§8.1.1） | 不做任何修改 | 说明当前 HEAD 指向哪个 commit，建议 `git switch -c <name>` 或切回原分支 |
| **D5 被 pre-commit 拒绝（revprop）** | 服务器不接受自定义 revprop（§8.6.3 形态 B） | 按 §8.5 完整回滚 D4 登记 | 原样展示 svn stderr，给出**需要用户明确确认**的「不带幂等键重试一次」选项；不确认就停下 |
| **配置校验未过** | `svnUrl` UUID 不符、`baselineBranch` 非 `svn/base`、白名单含非法路径（§6.2.1） | **不做任何修改**，仓库不进入可用状态 | 指出是哪个字段、哪一条规则；`svnUrl` 不符时同时显示配置值与 `svn info` 实际值 |
| **基线体检不通过**（§6.8） | 基线内容与服务器在 `baselineRev` 上的内容不符——假基线已经产生 | 按时机分：初始化时**整个回退、不创建 `.git`**；§9.4 第 ⑥b 步**停在那里不继续 ⑦**；Phase 1 R3b **中止，连 `svn/base` 指针都不动** | 列出差异文件（已剥掉 URL 前缀、剔除目录条目与纯属性条目，§6.8.4），说明是内容不符而非 revision 不符 |
| **基线体检跑不成功** | 断网、认证失败、`svnUrl` 不可达 | **不阻断**，不做任何修改 | 标记「基线未经体检」并如实显示；**不得当成体检通过**（§6.8.6） |
| **拿不到跨进程锁** | 同一工作副本上另一个 mygitclient 进程正在同步（§5.1.1） | 不做任何修改 | 显示持锁进程的 pid 与开始时间；pid 已不存在时提供「接管」入口并写日志 |
| **D8d 不是快进**（无 Local-Only commit 时） | 前面某步的假设崩了 | 停下不动 | 报错并导出 journal，**不自动 merge** |
| **D8d rebase 后仍有未标记 commit** | 该交付的东西没进交付树 | 停下不动 | 报错并导出 journal，列出这些 commit |
| **D8d rebase 冲突**（有 Local-Only commit 时） | 本地补丁与他人在 D6 拉下来的改动冲突 | **不回滚**，停在冲突态 | 进入冲突解决模式；提交已成功，基线已推进，只剩补丁没落回去 |
| 任意步骤客户端崩溃 | — | 靠 §7.6 的写前 journal | 重开后按 `phase` 给出对应的继续 / 放弃入口；`prepare` / `building-deliver` / `deliver-built` / `checked-out` 四个 phase 的放弃入口一律走 §8.2.4 统一收尾 |

## 14. 测试策略

### 14.1 fixture

对齐现有 `src-tauri/tests/fixtures.rs`：

```text
svnadmin create   <tmp>/repo
svn checkout file:///<tmp>/repo  <tmp>/wc1     ← 「你」
svn checkout file:///<tmp>/repo  <tmp>/wc2     ← 「同事」，用来制造并发
在 wc1 上跑初始化向导（git init + exclude + 首个基线）
```

两个工作副本指向同一个本地仓库，「同事抢先提交导致 out-of-date / 冲突」可以完整
复现，全程无网络、可重复、跑完即删。

⚠️ Windows 上 `file://` URL 是 `file:///C:/path`（三斜杠 + 盘符 + 正斜杠）。
这是已知跨平台坑，fixture 里要专门处理，不能靠路径拼接碰运气。

真正需要线上 SVN server 的只有三件事，都是边缘路径，不进自动化测试：凭据提示行为、
网络超时、大仓库性能。

### 14.2 必测场景

按 `AGENTS.md` §13 rubric 排：

| # | 场景 | 对应 rubric |
|---|---|---|
| 1 | **空清单的交付必须被拒绝** | 3、4 |
| 2 | **空文件列表的 `svn revert` 必须被拒绝** | 3、4 |
| 3 | 交付含新增 + 删除，`svn commit` 因 out-of-date 失败后**回滚的精确性** | 1、2 |
| 4 | 更新流程遇 rebase 冲突时停在正确状态，abort 能回到原样 | 2 |
| 5 | 外部 `svn commit` 后的失同步检测 | 1 |
| 6 | **exclude 规则：至少两层嵌套目录的源码必须真的被 git 跟踪**（§6.3），断言 `git ls-files` 而非规则文本 | 9、11 |
| 7 | exclude 规则生成：Windows 反斜杠、盘符、大小写 | 11 |
| 8 | 基线映射 → svn 节点的图合成，含孤儿 baseline | 8 |
| 9 | `svn update` 带来的树冲突（wc2 删掉一个目录） | 1 |
| 10 | 未初始化的 SVN 工作副本（有 `.svn` 无 `.git`）的识别 | 3 |
| 11 | **白名单外的文件绝不出现在任何 git / svn 命令的参数里** | 3、4 |
| 12 | 同步进行中的并发 invoke 被正确拒绝 | 5 |
| 13 | **mixed revision 检测**：wc2 只提交一个深层文件，wc1 的根 revision 不动，`nodeRevs` 校验必须报失同步（§6.6.1） | 1、3 |
| 14 | **断网查证**：制造「提交成功但客户端认为失败」，用 opid / changed-paths 在完整区间内正确判定（§8.6.2）；另测假阳性场景——先用同样的 message 提交过一次 | 1、2 |
| 15 | **D6 白名单外有本地修改时交付仍安全**：wc1 改一个美术文件不提交，wc2 提交别的内容，走完整交付，断言美术文件的本地修改**未被合并、未产生冲突文件**（§8.2.1） | 2、4 |
| 16 | **逐 phase 崩溃恢复**：在 S1 后、**D0.5 构造中途**、**D0.5 完成后**、D3.8 后、D4 后、D5 后各杀一次进程，重开必须能按 journal 给出正确的继续 / 放弃路径；D0.5 那两次要断言临时 worktree 被清理、`deliverRef` 被删、HEAD 仍在 `origBranch`；D3.8 后那次要断言能切回 `origBranch` 并清掉 `deliverRef`（§7.6、§8.2.4、§8.5） | 2、5 |
| 17 | **stash OID 抗漂移**：stash 之后由外部再压一个 stash，恢复必须仍取回正确的 WIP（§7.6.1） | 1、5 |
| 18 | **D8 分支编排**：交付后基线确实推进到新内容，且工作分支是快进；断言不会出现「基线内容是旧的却标着新 revision」（§8.2.2） | 1 |
| 19 | **Local-Only commit 绝不被交付**：同一文件里既有本地补丁又有待交付改动，走完整交付后断言服务器上是干净原文、磁盘上补丁还在、`svn/base..HEAD` 只剩那个补丁（§8.2.3 T1+T3） | 1、4 |
| 20 | **Local-Only 与待交付 commit 重叠时交付必须中止**：断言 SVN 侧一个字节都没动、`refs/mygitclient/deliver/<opid>` 与临时 worktree 已清理、原分支原样、**主工作副本磁盘内容全程未变**（§8.2.3 T2） | 1、2、4 |
| 21 | **本地补丁跨 `sync_in` 存活**：wc2 提交别处改动，wc1 走 `sync_in`，断言补丁被 rebase 到新基线上且内容不变（§6.7.1） | 1 |
| 22 | **恢复流程不误伤本地补丁**：带补丁的仓库触发 §9.4，断言 `svn revert` 的文件列表里没有补丁所在文件，且 ⑦ 之后补丁完好（§9.5） | 2、4 |
| 23 | **早退闭环**：D1/D2 空清单、D3 用户取消、D3.5 失败三种早退各跑一次（其中至少一次走慢路径），断言 HEAD 在 `origBranch`、`deliverRef` 已删、临时 worktree 已清、journal 已删、锁已释放、watcher 已恢复（§8.2.4） | 2、5 |
| 24 | **待交付历史含 merge commit 必须前置拒绝**：构造「Local-Only commit 夹在中间 + 历史里有一个 merge commit」的分支，断言 §8.1 #6 拒绝且**没有发生任何 git / svn 写操作**；再断言把补丁移到顶端后（走快路径）同一个仓库能正常交付（§8.1、§8.2.3） | 1、2、4 |
| 25 | **临时 ref 不与用户分支冲突**：仓库里预先建一个名为 `deliver` 的普通分支，断言整条交付流程照常成功（§8.2.3 为什么不用 `deliver/<opid>` 分支） | 4 |
| 26 | **交付成功后不得误报失同步**：走完整交付，断言 `rootRev != baselineRev`（必然）但状态判定为**正常**；再断言第一档只在 `rootRev` 真的变化时才触发全量校验（§6.6.2、§6.6.3） | 1、3 |
| 27 | **初始化前置校验**：分别在「白名单内有 `M`」「有 `?`」「有 `A` 登记」「mixed-revision」四种工作副本上跑初始化，断言**全部被拒绝**且未创建 `.git`；再断言干净仓库上 `git-svn-id` 的 rev 取自 `nodeRevs` 而非根 revision（§6.5、§6.5.1） | 3 |
| 28 | **外部删除后的恢复**：wc2 删除白名单文件并提交，wc1 外部 `svn update`，走 §9.4，断言该文件**不在**新基线里、磁盘上也没有；另测「用户自建未交付的新文件」在同一流程后**完好无损**（§9.4.1） | 1、2、4 |
| 29 | **`svn add --parents` 的父目录被完整撤销**：新增三层嵌套目录下的文件，制造 D5 失败，断言回滚后 `svn status` 与 `svnStatusBefore` **逐条一致**（§8.5.1） | 2、4 |
| 30 | **D4.9 拦住确认后的外部改动**：D3.8 之后、D5 之前用外部 git 切回 `origBranch`，断言 D4.9 中止交付、SVN 侧一个字节没动、D4 登记被完整撤销（§8.2.5） | 1、2、4 |
| 31 | **detached HEAD 被拒绝**：detached 状态下分别触发 `sync_in` 与 `sync_out`，断言前置检查拒绝且不做任何修改（§8.1.1） | 3、5 |
| 32 | **S1 stash 窗口**：在 `git stash push` 之后、写 `stashOid` 之前杀进程，重开后断言能按 `stashRefBefore` 正确判定栈顶归属，且**不静默 apply**（§7.3.1） | 2、5 |
| 33 | **查证不得被相同路径集合骗过**：制造「D5 真失败 + 用户随后用 TortoiseSVN 提交了同一批文件」，断言兜底 ② 只给「疑似」，内容校验后判定为**状态未知**而非成功（§8.6.2） | 1、2 |
| 34 | **查证不得被目录路径骗过**：交付一个新增嵌套目录下的文件，断言 changed-paths 里的目录被剔除后仍能正确匹配（§8.6.2） | 1、2 |
| 35 | **revprop 开仓自检**：断言首次交付后能读回 `mygitclient:opid`；再用一个拒绝 revprop 的仓库断言降级到兜底且界面有提示（§8.6.3） | 1、2 |
| 36 | **成功路径能走完 D10**：跑一次完整成功交付，断言 D10 不报错、journal 已删、锁已释放、watcher 已恢复；显式覆盖「有 Local-Only commit（D8d 走 rebase）」和「没有（走 ff-only）」两种，两者 `origHead` 都变了（§8.2.4 末尾） | 2、5 |
| 37 | **Phase 1 重建基线录的是新内容**：外部 TortoiseSVN update 之后跑重建，断言新基线 tree == 当前磁盘内容（**不是**旧基线）、HEAD 与工作目录全程未动、工作分支上的未提交改动完好（§16） | 1、3 |
| 38 | **服务器删目录 + 目录内有 git-ignored 文件**：走 §9.4，断言被删的受管文件从新基线和磁盘上消失，而 `.gitignore` 掉的 `local.log` **原样还在**；断言实现从未对目录发起递归删除（§9.4.1） | 1、2、4 |
| 39 | **revprop 被 pre-commit 拒绝**：给 fixture 仓库装一个拒绝未知 revprop 的 `pre-commit` 钩子，断言 D5 失败被归为形态 B、D4 登记完整回滚、**不自动重试**；模拟用户确认后重试成功并记 `supportsRevprop=false`（§8.6.3） | 1、2 |
| 40 | **stash 按 opid 精确取回**：压完我们的 stash 后由外部再压一个，断言恢复时按 `mygitclient-sync:<opid>` 在 reflog 里命中的是我们那条、内容正确，且**不碰**外部那条（§7.3.1） | 1、5 |
| 41 | **跨进程锁**：起两个进程同时对同一工作副本发起 `sync_out`，断言第二个被锁文件挡住并给出明确错误，journal 不被覆盖；再断言 pid 已消失的陈旧锁可被接管、pid 仍在的不可（§5.1.1） | 5 |
| 42 | **配置校验**：`svnUrl` 指向另一个仓库（UUID 不符）、`baselineBranch` 非 `svn/base`、白名单含 `..` / 绝对路径 / `:(exclude)` / 空串——逐个断言在配置加载时就被拒绝，且**不发生任何 git / svn 命令**（§6.2.1） | 3、4 |
| 43 | **`svn revert` 不被失同步检测发现**：外部 `svn revert` 后断言 `nodeRevs == baselineRev` 仍成立（即失同步检测**不报**），而 git 工作区状态变化能被报出（§9.3） | 1、3 |
| 44 | **基线体检抓住假基线**：构造「磁盘已到 r2 但 `baselineRev` 仍是 1」，断言体检报出差异、流程按 §13 停在正确状态（初始化不创建 `.git`／§9.4 停在 ⑥b／Phase 1 不动指针）；同一仓库里只改**美术**文件时断言体检**不误报**（§6.8 T3／T4） | 1、3 |
| 45 | **体检输出解析**：断言实现只取第 1 列（内容）、**剔除目录条目与纯属性条目**、剥掉 URL 前缀还原成仓库相对路径；用「服务器删掉嵌套目录下的文件」断言 `D` 条目被正确识别（§6.8.4、T7） | 1、3、11 |
| 46 | **「跑不成」不等于「不通过」**：把 `svnUrl` 指向不可达地址，断言初始化与恢复流程**不被阻断**、状态标为「基线未经体检」，且**没有**被当成通过（§6.8.6） | 1、3 |
| 47 | **`git-svn-id` trailer 解析**：断言三字段缺一（少 uuid／rev 非数字／无 `@`）即视为**非 baseline**，不猜、不部分接受；断言 trailer 里的 UUID 与 `svn info` 不符时按 §6.2.1 阻断（§6.4.1） | 1、3 |

### 14.3 前端

vitest 覆盖纯展示逻辑：`capabilities` → 按钮显隐、svn / git 节点的标签格式化。
exclude 生成与图合成都在 Rust 侧，由 cargo test 覆盖。

## 15. 已知限制

写进文档，不假装没有：

1. **重命名血缘丢失。** git 的 rename 在 SVN 侧拆成 delete + add，SVN 历史里看不出
   是同一个文件的延续。做 `svn move` 需要匹配 git 的 rename 检测与 SVN 的 copy
   语义，是独立的一块复杂度，不在本设计范围内
2. **外部工具无法阻止。** TortoiseSVN / 命令行 svn 可以绕过客户端，只能检测和恢复
3. **SVN 历史必须联网。** 离线时图上只有缓存 + 本地 git 部分
4. **切 git 分支会触发 Unity reimport。** 白名单内文件变化的必然结果
5. **`.prefab` / `.unity` 不归 git 管**（D7），这类文件的并行修改仍需团队人工协调
6. **不能有名为 `svn` 的 git 分支**（§6.4）
7. **本仓库没有 PR 门禁 CI**，所有验证靠本地实际执行
8. **交付后工作副本是 mixed-revision**：D6 只更新白名单路径（§8.2.1），美术资产
   停在旧 revision，要等下一次 `sync_in` 才跟上。这是刻意的取舍——换来的是「提交
   已不可撤销之后不再引入无法收拾的冲突」
9. **白名单外文件的冲突客户端修不了**：git 不管这些文件，`svn update` 在它们上面
   产生的冲突只能回 TortoiseSVN 处理。客户端负责如实报告，不负责修复
10. **本地补丁与待交付改动挨得太近时交付会被拦下**（§8.2.3 T2 实测）。这是刻意的：
    中止的代价是零（还没碰 SVN），猜错的代价是把调试代码送上服务器。缓解手段是把
    Local-Only commit 移到分支顶端，那条路径结构上不可能冲突
11. **用 TortoiseSVN 直接提交会把本地补丁带上去**。`ignore-on-commit` changelist 原本
    还能挡一下，本设计挡不住——补丁就在磁盘上，外部工具看到的就是改过的文件。与第 2 条
    「外部工具无法阻止」同源；交付走客户端是这套方案的前提
12. **`sync_in` 的 rebase 会压平 merge commit**。S9 和 D8d 用的都是
    `git rebase --empty=drop svn/base`，不带 `--rebase-merges`，所以用户在 IDE 里
    合出来的 merge 点会在下一次同步时被线性化。这不会报错也不会丢改动，但会**静默
    改写历史形状**。之所以接受：基线侧是 SVN 的线性历史，工作分支迟早要被压成线性
    才能交付；真要保留 merge 形状，就得先解决 §8.1 #6 拦下的那类问题
13. **待交付历史含 merge commit 且需走慢路径时会被拒绝**（§8.1 #6）。`cherry-pick`
    对 merge 没有安全的自动语义（§8.2.3 T4 / T5 实测）。缓解手段同第 10 条：把
    Local-Only commit 移到分支顶端转成快路径，快路径不受 merge 影响
14. **`rootRev` 只是提示，不是判据**（§6.6.2）。交付后根节点长期停在旧 revision 是
    正常的（§8.2.1 的刻意取舍，2026-09-13 实测确认），所以「根 revision 与基线不等」
    既不能证明有问题、也不能证明没问题。真正的判据只有白名单 `nodeRevs`
15. **D3.8 到 D5 之间无法完全排除外部 git 工具**（§8.2 D4.9）。应用锁是进程内的，
    而 Overlay 的核心收益恰恰是 Rider / VS 的 git 集成可用。D4.9 的提交前断言能把
    「提交了未经确认的字节」变成「中止并报错」，但消除不了竞态本身——真要根治得从
    隔离的临时 SVN 工作副本交付，那是另一个量级的改动，不在本设计范围内
16. **`svn commit` 传输中断且服务器不支持 revprop 时，状态可能长期停在「未知」**
    （§8.6.2）。这是刻意的：把「疑似成功」当成功会推进假基线，当失败会重复交付，
    两者都比挂着一个「需要人工确认」的状态更糟
17. **`svn revert` 不在失同步检测的能力范围内**（§9.3 实测）。它不推进任何 revision，
    只能由 git 工作区状态间接看出来。客户端不会因此报「基线失同步」，这是正确的——
    基线没错，错的是磁盘
18. **Phase 1 的「重建基线」不会推进工作分支**。用户下次得自己 rebase 到新基线上。
    Phase 2 的 §9.4 才做完整编排
19. **baseline 上的 `git-svn-id` trailer 不是 `git svn` 生成的**（§6.4.1）。用这个格式
    是为了白送 UUID 记账和工具互操作，但 `.git/svn/` 里没有对应的 `rev_map`。用户若
    自己在本仓库跑 `git svn init`，`git svn` 会按这些 trailer 解释历史，产生令人困惑
    的行为。不损坏数据，但客户端不支持这种混用，也不检测它
20. **基线体检只证明「磁盘 == 服务器」，不证明「git 跟踪的文件集合是对的」**
    （§6.8.3）。exclude 规则写错那一类由 §14.2 第 6 项覆盖。两条检查不可互相冒充，
    也不要因为体检通过就认为 §6.3 的规则一定正确
21. **体检需要联网**。离线时基线只能停在「未经体检」状态（§6.8.6）。这是刻意的：
    把「没能验证」当成「验证通过」等于这条检查根本不存在

## 16. 分期

每期独立 spec → plan → PR。

| 期 | 内容 | 验收 |
|---|---|---|
| **Phase 0** | `trait Vcs` + `GitBackend`（现有代码搬家，**行为零变化**） | 现有全部测试绿 + 手工 E2E 与改动前无差异 |
| **Phase 1** | 仓库识别、配置、exclude 生成、初始化向导、`SvnClient`（info / status / log 的 XML 解析）、状态判定与失同步检测（**只报警**）、**「重建基线」最小手段**、**基线体检（§6.8）**、混合图只读 | 能打开 SVN 仓库，本地 git 的暂存/分支/stash 可用，看得到 SVN 历史。**还不能 update / deliver**；基线会因外部 TortoiseSVN 操作而失同步，此期只报警 + 提供「重建基线」 |
| **Phase 2** | `sync_in` 完整流程 + 全部失败路径 + 中断持久化 + 冲突解决 UI + **§9.4 基线恢复向导**（复用同一套 stash / checkout / rebase 编排） | 能更新 |
| **Phase 3** | `sync_out` + 确认关口 + 精确回滚 + 传输中断查证 + **本地补丁的摘除与回落**（§8.2.3） | 能交付，闭环 |
| **Phase 4** | 白名单外改动视图与 revert、`.git` 误提交三层防护、svn 节点 diff | 补齐日常缺口 |

**Phase 0 单独拆出是有意的。** 它是纯重构、零行为变化，可独立验证，风险最低；
混进 Phase 1 的话，一旦现有 Git 模式出问题，就分不清是重构引入的还是新功能引入的。
对齐 `AGENTS.md` §1.2「不要把无关重构和业务修改混在一起」。

**Phase 1 的「重建基线」是这一期自洽的前提。** Phase 1 没有 `sync_in` / `sync_out`，
用户必须继续用 TortoiseSVN 更新和交付；而任何一次外部 update / commit 都会推进
`nodeRevs`，让 `svn/base` 立刻失同步（§6.6.2）。此时若只有红色横幅、没有任何出口，
「相对上次更新改了什么」这个 §6.4 的核心收益就永久失效了。

「重建基线」的关键是**不要碰工作目录**。此时 `svn status` 干净，意味着磁盘上白名单
内的内容**已经就是**新的服务器内容——要做的只是把它录成一个新的基线 commit：

```text
R1  校验：白名单内 svn status --no-ignore 干净、nodeRevs 单值 {N}
      不干净就引导去 TortoiseSVN 处理，不自动 revert（同 §6.5 P1）
R2  用一个**临时索引**把磁盘内容录成 tree，全程不碰真实 index、不碰 HEAD：
      GIT_INDEX_FILE=<tmp> git add -A -- <白名单>
      GIT_INDEX_FILE=<tmp> git write-tree                → TREE
R3  git commit-tree TREE -p svn/base -m "svn: rN … git-svn-id: <svnUrl>@N <uuid>"  → NEWBASE
R3b 基线体检（§6.8）                                    ← 此刻 TREE 就是磁盘内容，
                                                          前提成立；**有差异就中止**，
                                                          连指针都还没动，代价为零
R4  git update-ref refs/heads/svn/base NEWBASE          ← 只动指针
R5  删掉临时索引文件
```

**为什么不能照抄 §9.4 的 `git checkout svn/base`。** §9.4 之所以先 checkout 旧基线是
因为它后面紧跟着 `svn revert`——由 revert 把服务器内容重新铺回磁盘，checkout 只是把
git 侧的状态清空。极简版没有 revert（前提就是工作副本干净），此时先 checkout 会把磁盘
**换回旧基线内容**，随后录下来的就是旧内容却标着新 revision，正好造出这一节要防的那种
撒谎基线。这条捷径删错了步骤：删掉的是让序列成立的那一步。

顺带两个 checkout 自带的麻烦也一并没了：工作分支上若有未提交改动，`git checkout` 会
直接失败；而 R1–R5 全程不动 HEAD，也就不存在「流程结束忘了切回原分支」。

代价是工作分支不会自动跟进新基线，用户下次自己 rebase——Phase 1 本来就是手工期，这个
代价可以接受。

**Phase 1 结束时已是一个有意义的交付点**——本地 git 的暂存区、分支、stash 全部可用，
只是 update / deliver 还得回 TortoiseSVN 手工做。

本地补丁（§6.7）横跨两期：**标记**属于 Phase 1（提交框的勾选框 + 图上的角标，本质
只是一条 message trailer，此时还没有交付流程会读它），**摘除与回落**属于 Phase 3。
Phase 1 到 Phase 3 之间用户手工走 TortoiseSVN 交付时，本地补丁没有任何保护——这段
窗口期必须在 UI 上写明，不能让用户以为勾了「仅本地」就安全了。

## 17. 环境前置

- **`svn` CLI 必须在 PATH 上。** 开发机现状（2026-09-12 实测）：SlikSVN `1.14.2`，
  装在 `C:\Program Files\SlikSvn\bin`，已进 **Machine PATH**，`svn` 与 `svnadmin`
  均可用
- TortoiseSVN 默认**不装**命令行工具，需在安装器里勾选「command line client tools」；
  SlikSVN 自带 `svnadmin`，fixture 测试需要它
- 打开 SVN 仓库时若检测不到 `svn`，客户端必须给出明确的安装引导，而不是抛裸错误

### 17.1 「装了但当前进程看不见」是一个必须单独处理的状态

安装器写的是**持久** PATH（Machine / User），已经在跑的进程仍持有启动时的环境快照。
2026-09-12 实测：SlikSVN 装好之后，本会话的 shell 与新起的 PowerShell 子进程
**都**报 `svn: command not found`，而 Machine PATH 里明明已有该条目——因为子进程
继承的是父进程的旧环境块。

对客户端的含义：Tauri 应用如果在用户装 SVN **之前**就已启动，它同样看不见新装的
`svn`，而用户会认为「我明明装了」。所以检测不到 `svn` 时的提示必须区分两种情况：

- 持久 PATH 里**也没有** → 引导去安装
- 持久 PATH 里**有**，但当前进程环境里没有 → 提示**重启应用**，不要让用户重装

判定方式：读注册表 / `[Environment]::GetEnvironmentVariable("Path","Machine")` 对应的
持久 PATH，与进程 PATH 比对。这条不实现的话，最常见的首次使用路径就会卡死在一个
误导性的错误上。

## 18. 修订记录

### 2026-09-13（四）· 重新审视 `git svn`：论证被推翻，机制被借走

用户指出「git 本身就能操作 SVN 仓库」（`git-svn` 文档）。核实之后：**D2 用来否决
`git svn` 的理由是错的，结论侥幸是对的。**

| | 内容 |
|---|---|
| 错在哪 | D2 写「几十 GB → 判死 `git svn clone`」。但 `--include-paths` / `--ignore-paths` 是 `git svn` 的正式选项，美术本来就不会进 git 对象库。**体积拦不住它** |
| 真死因 | `git svn` **没有工作副本**，它直连服务器。而本设计要一个 Unity 能打开的目录、美术只能来自 SVN 工作副本，于是两套系统争夺同一批代码文件。唯一出路 `svn update --set-depth=exclude` 与 D6 的「路径 AND 扩展名」语义冲突——`Assets/Scripts/` 底下的 `.psd` 会**两边都不认领、直接从磁盘消失** |
| 新发现的硬约束 | **Git for Windows 的 `git svn` 打不开 `file://` 仓库**（4 种 URL 写法 × fsfs format 6/7/8 全败，同一批 URL 用 `svn` CLI 全部成功；本机无 `svnserve`）。任何以 `git svn` 为核心的方案都会让 §14.1 的 fixture 策略当场失效 |
| 必须承认的收益 | `git svn dcommit` 不吃 pathspec，**§8.3 空 target 与 §8.2.5 D4.9 竞态在它那里结构上不存在**。本方案只能靠断言挡住，这是选 Overlay 付的代价 |

这一轮的教训与第三轮同类：**论证不写透，等于没写。** 一条只写了「几十 GB」的否决理由，
半年内就被重新提出来一次；补上真死因和实测之后才算关上。

借走的两条性质（不引入 `git svn` 本身）：

- **基线必须能被服务器证伪**。`git svn` 的基线是 fetch 下来的，天然不撒谎；本设计的
  基线由 `git add -A` 收磁盘而来，§18 第二轮那张「四个切面」表全是这一个根因。新增
  §6.8 的基线体检，用 `svn diff --old=<url>@<rev> --new=<wc> --summarize` 直接证伪，
  7 组 fixture 实测通过
- **记账改用 `git svn` 的原生 trailer**：`Svn-Rev:` / `Svn-Url:` → `git-svn-id:
  <url>@<rev> <uuid>`，白送 §6.2.1 要的 UUID 记账

| 改动点 | 内容 |
|---|---|
| §1.2 / §2 D2 | 指向 §2.2；D2 那条影响改为「不构成否决 `git svn` 的理由」 |
| §2.2（新增） | 为什么不是 `git svn`：错误的旧理由、真死因（无工作副本 + depth-exclude 与 D6 冲突）、`file://` 实测表与测试成本、如实承认它消灭的两个失败模式、借走的两条性质 |
| §6.4 / §6.4.1（新增） | trailer 换成 `git-svn-id: <url>@<rev> <uuid>`，给出解析正则与三字段全有才算 baseline 的规则；写明混用 `git svn init` 的代价 |
| §6.8（新增） | 基线体检：为什么需要、怎么做（含 7 组实测）、**证明不了什么**、输出解析的两个坑（URL 前缀、目录与纯属性条目）、四个时机、「跑不成 ≠ 不通过」 |
| §6.5 / §9.4 / §16 | 初始化在首个基线之后、`checkout -b work` 之前体检；§9.4 新增 ⑥b；Phase 1 新增 R3b（在动指针之前，中止代价为零） |
| 全文 | `Svn-Rev` / `Svn-Url` 的 10 处引用同步改为 `git-svn-id`（§18 历史记录里的原文不动） |
| §13 | 新增「基线体检不通过」「基线体检跑不成功」两行 |
| §14.2 | 新增测试项 44–47 |
| §15 | 新增已知限制 19–21 |
| §16 | 基线体检归入 Phase 1 |

### 2026-09-13（三）· 第三轮 review：修洞时新捅的洞

第三轮 review 提了 10 条，全部成立。**其中 3 条是上一条修订记录（第二轮）引入的回归**
——这一点比那 3 条本身更值得记：跨节复用一段逻辑时，没有逐条核对被复用方的前提。

| 回归 | 怎么坏的 |
|---|---|
| D10 套用 §8.2.4 ④ | ④ 断言 `origHead 未变`，那是**早退**路径的不变量；D8d 刚推进过 `origBranch`，所以**每一次成功交付**都会在收尾时报错 |
| Phase 1「重建基线」 | 照抄 §9.4 时删掉了 `svn revert`（让序列成立的那一步），却保留了 `git checkout svn/base`（让序列出错的那一步），结果把旧内容录成了新基线 |
| 初始化前置漏 `?` | 同一节的风险表和测试项 27 都要求拒绝 `?`，唯独可执行的状态列表里没写 |

另外 7 条中，两条推翻了上一轮写下的**论证**，不只是实现：

- §9.4.1 原来的安全性论证建立在「`git stash push -u` 保证磁盘 = 旧基线内容，一个不多
  一个不少」上。实测发现 **`-u` 不收 git-ignored 文件**，前提不成立；同时 SVN 对被删
  目录只报一个 `?` 目录条目，按目录递归删会删掉用户的 ignored 文件。删除集合改为从
  `git ls-tree svn/base` 反推，全是受管的具体文件，绕开这两个洞
- §8.6.3 声称服务器拒绝 revprop 时「客户端降级」。但自检写在「第一次交付**成功之后**」，
  而钩子拒绝的场景下 D5 永远不成功——降级路径根本不可达。补上形态 B：归为专门的失败
  类型、完整回滚、由**用户明确确认**后不带 revprop 重试一次（这不违反 §8.7，§8.7 禁的
  是自动重试）

`wcRev` 那条 review 标的是 P2，我按 P1 处理：`§8.1 #5` 的 `remoteRev == wcRev` 里若
`wcRev` 取 `rootRev`，交付之后根节点长期滞后，**之后每一次交付都会被这条前置检查挡住**
——与 D10 那条同属「正常路径被自己的断言堵死」。

| 改动点 | 内容 |
|---|---|
| §5.1.1（新增） | 锁必须跨进程：锁文件 + 原子创建 + pid 陈旧判定；journal 所有权由它界定。记录现状（无 `tauri-plugin-single-instance`，现有锁均为 `std::sync::Mutex`） |
| §6.2.1（新增） | 配置是不可信输入：`svnUrl` 比对 repository UUID；`baselineBranch` 固定；白名单拒绝绝对路径 / `..` / pathspec magic / 空值；校验集中在加载时 |
| §6.5 | P1 补上 `?`，并要求 `--no-ignore` |
| §7.3 / §7.3.1 | stash message 写入 `mygitclient-sync:<opid>`；恢复改为在 stash reflog 里按 opid 精确定位（实测：外部 stash 挤到栈顶也不受影响），不再降级成人工辨认 |
| §8.1 #5 | `wcRev` → `baselineRev`，并写明为什么不能用 `rootRev` |
| §8.2 D10 / §8.2.4 | 成功路径改用自己的断言；§8.2.4 末尾写明 ④ 只属于早退路径，并列表对照两条路径的断言 |
| §8.6.3 | 拆成形态 A（提交成功但 revprop 被丢）与形态 B（提交被拒），补形态 B 的用户确认重试路径 |
| §9.3 | 删掉「revision 校验能发现 `svn revert`」这个假保证（实测 revision 完全不变），改为两条独立的检测路径 |
| §9.4 / §9.4.1 | ④ 拆成 ④a（状态分类）与 ④b（删除集合从 `git ls-tree svn/base` 反推）；⑤ 明确逐文件删、绝不递归删目录；重写安全性论证 |
| §16 | Phase 1「重建基线」改用临时索引 + `commit-tree` + `update-ref`，全程不动 HEAD 与工作目录 |
| §13 / §14.2 / §15 | 新增 3 条失败路径、测试项 36–43、已知限制 17–18 |

### 2026-09-13（二）· 第二轮 review：基线真实性的四个漏口

第二轮独立 review 提了 12 条，其中 3 条与上一条修订记录重复（已修），9 条成立。本机
装有 SlikSvn 1.14.2，所以这一轮**全部用真实 SVN fixture 实测**，不再靠语义推断。

九条里有四条是同一个根因的不同切面：**`svn/base` 这个「git 分支扮演 SVN 基线」的抽象
在边界上会造出一个与服务器不符、却标着权威 `Svn-Rev` 的基线**。

| 切面 | 什么时候造出假基线 | 实测 |
|---|---|---|
| 初始化（§6.5） | 工作副本本来就脏或 mixed-revision 时，本地内容被当成服务器基线 | `rootRev=1` 而节点 `=3`，初始化完成即判失同步 |
| 交付后（§8.2.1 / §6.6.2） | D6 只更新白名单，根节点不动，`rootRev != baselineRev` 必然成立 | 交付**完全成功**后 `rootRev=1`、`baselineRev=3` → 立即误报失同步 |
| 外部删除后（§9.4） | 恢复流程只枚举 `M` / `!`，漏掉 `?` | 服务器删掉的文件被 `git checkout svn/base` 拉回磁盘，`svn status` 报 `?`，随后写进新基线 |
| Phase 1（§16） | 只报警不恢复，且 §9.4 没有分配给任何一期 | —— |

**`rootRev` 的定位是这轮最关键的修正。** §6.6.1 花了整节论证「读根的单个
`<entry revision>` 不能证明任何事」，§6.6.2 却把 `rootRev != baselineRev` 当成失同步的
**判据**——自己推翻自己。现在 `rootRev` 降级为**变化提示**（与上次观测值比，变了就跑
完整 `nodeRevs` 校验），不再是判据。

顺带否掉一个看似可行的方案：`svn update --depth=empty .` 实测确实能只推进根节点
revision 而不碰白名单外的美术文件（本地修改原样保留、状态仍是 `M`），但它把 `rootRev`
推到的是**当时的服务器 HEAD**，而 `baselineRev` 是白名单节点的 revision，两者仍然可能
不等。既然恢复不了等式，就不值得在「已不可撤销」的 D5 之后再加一次 SVN 写操作。

| 改动点 | 内容 |
|---|---|
| §6.5 | 初始化前新增前置校验：白名单内 `svn status` 干净、`nodeRevs` 单值；`Svn-Rev` 改取自 `nodeRevs` 而非 `svn info` 的根 revision；不满足则拒绝并引导，**不自动规范化** |
| §6.6.2 | `rootRev` 从判据降级为变化提示；失同步判据只剩 `nodeRevs` 两条 |
| §6.6.3 | O(1) 档改为「`rootRev` 与**上次观测值**比」，交付后根节点长期滞后不再导致每次刷新都跑全量校验 |
| §7.2 | 新增前置检查：HEAD 必须附着在分支上，拒绝 detached |
| §7.3 / §7.6 | S0 增记 `origBranch` / `origHead` / `stashRefBefore`，新增 `stashing` phase，堵上 S1 的 stash 崩溃窗口 |
| §8.1 | 新增前置检查：HEAD 必须附着在分支上 |
| §8.2 | D4 改为记录 `svn status` 差量（含 `--parents` 带出的父目录）；新增 D4.9 提交前一致性断言 |
| §8.5 | 撤销 D4 登记改为按差量、从深到浅逐条 revert |
| §8.6.2 | changed-paths 兜底降级为「疑似命中」，必须再校验内容；目录路径不参与比对；无法判定时保持「状态未知」 |
| §9.4 | ④ 改为按 SVN 版本控制集合的变化分类处理，覆盖 `?` / `A` / `D` / `R`；新增 §9.4.1 说明为何删 `?` 是安全的 |
| §15 | 新增第 14～16 条已知限制 |
| §16 | §9.4 恢复流程明确归属 Phase 2；Phase 1 补「重建基线」这一最小恢复手段，验收语加限定 |

### 2026-09-13 · review 核实后修正 D0.5 的三处缺陷

对上一版新增的 D0.5 做了一轮独立 review，三条意见全部在临时 git fixture 里复现确认，
全部成立。核心问题是 **D0.5 被当成「顺手加一步」写进流程，没有按它实际具备的破坏面
配齐前置检查、早退路径和崩溃语义**。

| # | 缺陷 | 实测证据 | 修正 |
|---|---|---|---|
| 1 | D0.5 之后缺早退与崩溃闭环，且 §13 已写的那条回滚步骤本身跑不通 | `cherry-pick --abort` 后 HEAD 仍在 `deliver/op1`，`git branch -D deliver/op1` → `cannot delete branch used by worktree` | 新增 §8.2.4 统一收尾；D0.5 拆出 `building-deliver` / `deliver-built` 两个 phase；慢路径改到临时 worktree 里重放，主工作副本全程不动 |
| 2 | 慢路径遇 merge commit 直接崩 | 不加 `-m`：`is a merge but no -m option was given`（exit 128）；加 `-m 1`：`CONFLICT`，父提交改动被重复应用 | §8.1 新增第 6 条前置检查，在动手之前拒绝；快路径不受影响 |
| 3 | `deliver/<opid>` 会与用户已有的 `deliver` 分支撞车 | `fatal: cannot lock ref 'refs/heads/deliver/abc123': 'refs/heads/deliver' exists`；反向、`pack-refs` 之后同样失败 | 改用内部 ref `refs/mygitclient/deliver/<opid>`，与 §7.6.1 钉 stash 的做法同一套 |

三条里第 1 条的性质最重：它不只是漏了几个早退分支，而是**推翻了 D3.8「磁盘切换尽量晚做、
要在用户确认之后」这条自己写下的不变量**——慢路径的 `git checkout -b` 在 D3 确认之前就
把磁盘切走了，D3.8 在慢路径下退化成 no-op。所以修法不是补几条 `git checkout`，而是把
构造过程整个挪出主工作副本。

**顺带发现、一并记入**：S9 / D8d 的 `git rebase` 不带 `--rebase-merges`，会静默压平用户
的 merge commit（§15 第 12 条）。与第 2 条同源，但不阻断流程，作为已知限制记录。

| 改动点 | 内容 |
|---|---|
| §7.6 | `deliverBranch` → `deliverRef`；新增 `tempWorktree`；新增 `sync_out` 的 phase 取值表与「为何 `building-deliver` / `deliver-built` 必须分开」 |
| §8.1 | 新增第 6 条：慢路径要求待交付历史线性 |
| §8.2 | D0.5 改为构造交付树（不动磁盘）、标注两个 phase；D1 / D3.8 / D8 / D10 改用 `deliverRef`；补「D0.5→D3.5 之间所有退出走 §8.2.4」 |
| §8.2.3 | 重写：两条硬约束、为何用内部 ref（含实测）、慢路径改临时 worktree、实测表补 T4 / T5 |
| §8.2.4（新增） | 六个早退出口共用的 `cleanup_before_svn`，五步幂等，含 `origHead` 断言 |
| §8.5 | 收尾改为复用 §8.2.4 ②～⑤；detached HEAD 的措辞修正 |
| §13 | 修正 D0.5 冲突行；新增 merge 前置拒绝、D0.5 非冲突失败、空清单、用户取消、D3.5 失败五行；崩溃行补 phase 清单 |
| §14.2 | 第 16、20 项补强；新增第 23（早退闭环）、24（merge 前置拒绝）、25（`deliver` 分支不冲突） |
| §15 | 新增第 12、13 条已知限制 |

### 2026-09-13 · 补上「长期本地修改」这个缺失场景

用户指出：受 SVN 管理的代码文件常有**不打算交付**的本地改动（注释掉刷屏的 log、临时
测试代码），他此前靠 SVN 的 `ignore-on-commit` changelist 处理。初版设计**完全没有这个
场景**——`changelist` 只在 §2.1 作为被否决的方案出现过一次。

初版撞上该场景的两条死路：

| 用户的放法 | 初版的行为 |
|---|---|
| 不 commit，留在工作区 | §8.1 前置检查第 2 条「工作区必须干净」直接拦死，进不了交付 |
| commit 进 git | §8.2 D1 的 `svn/base..HEAD` 把它算进净变更清单，**注释掉的 log 直接上服务器** |

即：白名单内的文件不存在「长期本地修改」这个状态。这是设计缺口，不是措辞问题。

**未采用文件级清单**（直接照搬 changelist）的两个原因：粒度是文件而非 hunk，同一个
`.cs` 里既有补丁又有真实改动时只能二选一；且它会让 D8c 的 `git add -A` 把补丁写进基线
commit，制造 §9.2 的「基线撒谎」。详见 §6.7.2。

**采用的方案**：本地补丁放进带 `Local-Only: true` trailer 的 commit，交付时先构造不含
这些 commit 的交付树并在确认后切到磁盘上，`svn commit` 之后再 rebase 回来。不新增
任何配置字段。

| 改动点 | 内容 |
|---|---|
| §6.7（新增） | 模型、关键不变量（补丁活在 commit 里而非工作区）、与另两个方案的对比 |
| §7.4 | 补一条：`sync_in` 零改动，S9 的 rebase 自动搬运补丁 |
| §8.1 | 第 2 条补上「不打算交付的改动应提交成 Local-Only」 |
| §8.2 | 新增 D0.5 / D3.8；D1 改为 diff `deliver/<opid>`；D3 须列出被摘掉的 commit；D9 禁止提示删除带补丁的分支；D10 清理 `deliver/<opid>` |
| §8.2.2 | D8a 的 HEAD 明确为 `deliver/<opid>`；D8d 按有无补丁分两条路，`--ff-only` 断言改为「剩余 commit 必须全部带标记」 |
| §8.2.3（新增） | 快路径 / 慢路径、冲突必须中止、fixture 实测三组、漏标的两道防护 |
| §9.5 | 补一条：恢复流程为何不误伤补丁 |
| §10.4 / §11 | `CommitNode.localOnly`；`CommitRow` 角标；提交框勾选框 |
| §13 | 新增 D0.5 冲突、D8d rebase 冲突、D8d 残留未标记 commit 三行 |
| §14.2 | 新增必测场景 #19–#22 |
| §15 | 新增限制 10（重叠时交付被拦）、11（TortoiseSVN 直接提交挡不住） |
| §16 | 标记属 Phase 1、摘除与回落属 Phase 3；写明两期之间的窗口期无保护 |

**实测支撑**（2026-09-13，临时 git fixture，跑完即删）：T1 摘掉不重叠的中间补丁
`cherry-pick` 退出 0；T2 重叠时退出 1 并 `CONFLICT`；T3 基线推进后 `rebase` 退出 0，
`svn/base..HEAD` 只剩 Local-Only commit，磁盘有补丁而基线是干净原文。三组均见 §8.2.3。

### 2026-09-12 · 对抗性 review 后的五处修正

初版（commit `928923f`）经 Codex 对抗性 review，五条 high 全部成立，已修入本文。
这些都是**会破坏数据保护与恢复不变量**的缺陷，不是措辞问题。

| # | 缺陷 | 核验方式 | 修正 |
|---|---|---|---|
| 1 | 反向 ignore 规则漏掉嵌套源码：`/Assets/Scripts/**` 排除了子目录，git 不再递归，后续 `!…/*.cs` 对深层文件永不生效 | **临时 git fixture 实测**：3 个 `.cs` 只纳入 1 个 | §6.3 补 `!/Assets/Scripts/**/`；测试改为断言 `git ls-files`，必须覆盖两层嵌套 |
| 2 | 单个根 revision 无法表示 SVN 的 mixed-revision 工作副本，外部提交深层文件时失同步门禁被完全绕过 | 2026-09-13 用真实 SVN（SlikSvn 1.14.2）补测确认：只提交深层文件后根 revision 确实纹丝不动 | §6.6 改为白名单 `nodeRevs` 集合校验 + 检查频次分层 |
| 3 | 断网后靠「作者 + message」查证不是唯一标识，假阳性会把真失败当成功、假阴性会重复交付 | 逻辑核验 | §8.6 改为 opid revprop 幂等键 + `expectedStartRev` 完整区间 + changed-paths 兜底 |
| 4 | D5 之后的全量 `svn update` 会在「已不可撤销」之后合并白名单外的本地修改，且失败路径表未覆盖 D6 | 逻辑核验 | §8.2.1 D6 改为只更新白名单路径；§7.2.1 新增白名单外修改的确认关口；§13 补 D6 / D8 行 |
| 4b | **review 未点破的更深缺陷**：原 D7「落新基线 commit」没写落在哪个分支，按字面会提交到工作分支上，基线根本不推进 | 自查时发现 | §8.2.2 新增显式分支编排（先 `branch -f` 再 checkout，`--ff-only` 作断言） |
| 5 | 中断状态不是写前日志（S1→S2、D4→D5 之间崩溃无记录），且记录可变的 `stash@{0}` 会被外部 stash 挤走 | **实测确认漂移** | §7.6 改为原子写入的 operation journal；stash 按不可变 OID + 专用 ref 钉住 |

§14.2 相应新增 6 个必测场景（#13–#18），覆盖 mixed revision、嵌套 ignore、断网
查证的两个方向、D6 白名单外安全性、逐 phase 崩溃恢复、stash OID 抗漂移。

**仍需 fixture 实测确认的假设**（本机尚无 `svn`，见 §17）：

- `svn commit --with-revprop` 是否受服务器钩子限制（§8.6.3 已设计降级路径）
- `svn status -v --xml` 在白名单子树上的实际性能与 `<wc-status revision>` 字段形状
- `svn update -- <paths>` 造成的 mixed-revision 对后续 `svn commit` 的影响
- §8.6 表中的 SVN 错误码（E155011 / E160028 / E170001 / E165001）按记忆写入，须校准

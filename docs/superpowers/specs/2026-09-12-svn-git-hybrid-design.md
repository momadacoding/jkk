# SVN + Git 混合模式 设计规格

- 日期：2026-09-12
- 状态：设计已确认，未实现
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
- git 历史与 SVN revision 的双向映射（git-svn 语义）
- 把本地 git 历史共享给同事（不需要 git remote）
- 在 SVN 模式下提供 worktree、tag、PR、cherry-pick、revert
- `svn move`：git 的 rename 一律拆成 delete + add，见 §15

## 2. 已确认的决策

这些是设计的输入，不是本文推导出来的结论。改动其中任何一条都需要重新评估架构。

| # | 决策 | 影响 |
|---|---|---|
| D1 | 只给程序 / 代码同学用 | 砍掉文件锁、externals、大资产工作流 |
| D2 | 一份完整工作目录，代码 + 美术都在，可能几十 GB | 判死 `git svn clone`；git 只能覆盖代码子集 |
| D3 | 本地 git 历史是脚手架，用完即弃，最终 squash 成一次 `svn commit` | 不需要 revision 映射；同步模型大幅简化 |
| D4 | 只需本地多任务并行 + 实验性改动 | 砍掉全部 SVN 分支功能 |
| D5 | 主视图是混合图：SVN 线性历史 + 本地 git 分支 | 见 §10 |
| D6 | 白名单手写配置，纯白名单语义 | 见 §6.3 |
| D7 | 白名单**不含** `.prefab` / `.unity` / ScriptableObject | 避免 git 行合并产出语义损坏的 YAML |
| D8 | 采用 Overlay：`.git` 直接放在 SVN 工作副本根 | 见 §2.1 |
| D9 | 交付时硬性要求工作区干净，不自动 stash | 见 §8.1 |
| D10 | 基线用普通分支 `refs/heads/svn/base`（IDE 可见） | 见 §6.4 |

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
!/Assets/Scripts/**/*.cs
!/Assets/Scripts/**/*.meta
```

这套反转写法极易写错，因此**由代码生成、绝不让人手写**，并有独立单元测试
（给定配置 → 断言哪些路径被纳入、哪些被剪掉，含 Windows 反斜杠形态的输入）。

目录级剪枝是性能的关键：git 不会递归进几十 GB 的美术目录，`git status` 保持秒回。

### 6.4 基线分支

`refs/heads/svn/base`——普通分支，IDE 可见。

- 每次成功 `svn update` 后在其上落一个 commit，内容 = 白名单内文件在该 revision
  的状态
- commit message 携带权威记账：

```text
svn: r12346

Svn-Rev: 12346
Svn-Url: https://svn.company.com/game/trunk
```

- **客户端 UI 禁止在基线分支上开发提交。** 流程内部会临时切上去，但任何流程结束时
  必须回到原分支

选可见分支而非 `refs/mygitclient/base` 的理由：D8 选 Overlay 的核心收益就是 IDE
可用，而「我相对上次更新改了什么」是最常看的 diff，把基线藏起来等于削掉这个收益。
误操作防护改由 UI 限制 + §9 的失同步检测承担；真删了也能从 reflog 恢复，最坏
重建一个基线。

已知小坑：分支名带斜杠意味着 git 会建 `.git/refs/heads/svn/` 目录，此后不能再有
一个名为 `svn` 的分支。

### 6.5 初始化向导

```text
git init -b svn/base                 ← 显式指定，不吃 init.defaultBranch 的默认值
写入 .git/mygitclient.json（用户编辑白名单）
生成 .git/info/exclude
git add -- <白名单> && git commit    ← 首个基线，Svn-Rev 取自 svn info
git checkout -b work                 ← 立刻离开基线分支
```

### 6.6 状态判定

每次刷新计算：

```text
baselineRev = svn/base 顶端 commit 的 Svn-Rev trailer   ← 权威记账
wcRev       = svn info --xml 的 <entry revision>        ← 本地读，不联网，毫秒级
remoteRev   = svn info --xml <URL> 的 revision          ← 走网络，异步，失败不阻塞
```

| 判定 | 含义 | 界面 |
|---|---|---|
| `baselineRev == wcRev` | 正常 | — |
| `baselineRev != wcRev` | **基线失同步**（外部动过工作副本） | 红色横幅，阻断同步操作，提供恢复向导 |
| `remoteRev > wcRev` | 有新提交 | 「落后 N 个 revision」 |

`remoteRev` 走网络，按现有 `refreshPullRequests` 的模式处理：不 await、失败静默
（读不到就不显示落后数，不弹错）。

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
2. 基线未失同步
3. 当前不在 `svn/base` 上
4. `svn status` 里白名单外没有冲突态文件（`C`）
5. 拿到该工作副本的独占锁

### 7.3 步骤

```text
S0   挂起 watcher
S1   工作区不干净 → git stash push -u -- <白名单>      记下 stashRef
S2   记下 ORIG_BRANCH
S3   git checkout svn/base
S4   svn status --xml 校验白名单内应为 0 修改          ← 双保险
S5   svn update --accept postpone
S6   svn info --xml 读新 revision
S7   git add -A -- <白名单> && git commit               新基线
S8   git checkout ORIG_BRANCH
S9   git rebase --empty=drop svn/base                   ← 冲突在这里爆
S10  git stash pop（若 S1 有 stash）
S11  恢复 watcher + 主动发 repo-changed
```

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

### 7.5 代价

S2–S7 之间工作目录会短暂回到基线状态，Unity 会多触发一次编译 / reimport。
只影响白名单内的代码文件，是一次编译的量级。相对于「SVN 冲突文件满地 + 可能丢
改动」，这个代价可以接受。

### 7.6 中断状态的持久化

S9 / S10 卡在冲突时，若客户端崩溃或被关闭，重开后必须知道上次同步没走完。

- rebase 中途：git 自己记在 `.git/rebase-merge/`，现有 `repo_state` 命令已能报出，
  **复用**
- stash 未 pop：git 不记，落 `.git/mygitclient-sync.json`：

```json
{ "phase": "rebase", "origBranch": "work", "stashRef": "stash@{0}", "startedAt": 1757000000 }
```

流程正常收尾时删除。打开仓库时发现它存在 → 界面顶部常驻「上次同步未完成」提示，
提供继续 / 放弃两个入口。

## 8. 交付流程 sync_out

### 8.1 前置检查

1. 基线未失同步
2. **工作区必须干净**。这里**不自动 stash**：交付的是 commit 过的内容，未提交的
   改动不该被悄悄排除、也不该被悄悄带上。硬拦住，让用户自己决定
3. 不在 rebase / merge 中途
4. 当前分支不是 `svn/base`
5. `remoteRev == wcRev`（走一次网络）；不等则要求先更新
6. 拿锁

### 8.2 步骤

```text
D0   挂起 watcher
D1   git diff --name-status --no-renames svn/base..HEAD    ← 净变更清单
D2   按白名单过滤清单                                       ← 双保险
D3   把清单给用户确认（A/M/D 分类、文件数、提交信息）        ← 不可撤销前的关口
D3.5 再查一次 remoteRev                                     ← 压缩竞态窗口
D4   A → svn add --parents <files>
     D → svn delete <files>
     M → 无需操作
D5   svn commit --non-interactive -m <msg> -- <显式 target 列表>
D6   svn update（拿到自己刚提交的 revision）
D7   落新基线 commit（--allow-empty）
D8   提示「work 分支已交付，可删除」——不自动删
D9   恢复 watcher
```

### 8.3 空清单必须中止

D1 / D2 任一步得到空清单立即中止，**绝不进入 D5**。

空 target 的 `svn commit` 会提交整个工作副本——包括几十 GB 美术里任何一个被误改
的文件，而且**不可撤销、直接影响全组**。这是本方案后果最重的失败模式，与
`AGENTS.md` §6.2 记录的空 pathspec 教训同源但更严重（SVN 提交直接上服务器）。

代码里有独立断言 + 专门测试用例，见 §14.2。

### 8.4 竞态窗口

前置检查里的 `remoteRev == wcRev` 只是**快速失败的优化，不是保证**。真正的窗口在
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

```text
每条 svn add     → svn revert <file>
每条 svn delete  → svn revert <file>，然后删掉磁盘上被恢复出来的那个文件
                   （用显式文件删除，不用 git clean —— 那条命令语义太宽）
撤销完断言：git status 与 D5 之前一致
```

逐条精确撤销，不用任何批量恢复命令。

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

**先查证，再决定**：

```text
D5 失败且错误属网络类
   ↓
恢复连通后，svn log --xml -l 5 <URL>
   ↓
查最近几条 revision 里有没有「作者是你、提交信息是你刚输入的那条」
   ├─ 有   → 提交其实成功了。不回滚，直接跳到 D6
   ├─ 没有 → 真失败，按 §8.5 回滚
   └─ 查不到（仍连不上）→ 什么都不做，进入「交付状态未知」，
                          界面常驻警示，等网络恢复后再点一次查证
```

「交付状态未知」要落进 `.git/mygitclient-sync.json`，客户端重开也要记得。

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
git add && git commit   → 落「新基线」，内容是旧的 r12345，却打上 Svn-Rev: 12346
                          ⚠️ 基线从此撒谎
后续交付                → 净变更含「把已提交过的内容再提交一遍」
                          → 可能覆盖别人在 r12346 之后的改动
```

**真正的风险不是那次误提交，而是它之后基线静默错位、几天后覆盖掉同事的代码。**

### 9.3 检测

按 §6.6 比对 `baselineRev` 与 `wcRev`。检查点：打开仓库时、每次 `repo-changed`
刷新时、更新 / 交付的第一步。`svn info` 纯本地读、不联网，成本可忽略。

同一条检查还能抓住另外两种外部操作：在命令行跑了 `svn update`、用 TortoiseSVN 做了
`svn revert`。

### 9.4 恢复流程

```text
① git stash -u                          保护未提交改动，可逆
② 记下当前分支
③ git checkout svn/base                 磁盘 = 旧基线内容
④ svn status --xml，取出白名单内、状态为 M/! 的**显式文件列表**
   ⚠️ 列表为空 → 跳到 ⑥，绝不执行空参数的 svn revert
⑤ svn revert <显式文件列表>              恢复到 r12346 的 base 内容
⑥ git add <白名单> && git commit         新基线，Svn-Rev: 12346
⑦ git checkout <原分支>
   git rebase --empty=drop svn/base      已被误传上去的 commit 会变成空 commit，自动丢弃
⑧ git stash pop
```

### 9.5 第 ⑤ 步为什么安全

涉及破坏性命令，论证必须显式：

- 只作用在白名单内的文件上，美术资产一个都不碰
- 此刻这些文件的内容 = `svn/base` 分支的内容，而那个 commit 就在 git 对象库里，
  随时能取回，因此**可逆**
- 目标是从 `svn status` **显式枚举**出来的文件列表，不是 `svn revert -R <目录>`
  ——递归 revert 会波及同目录下白名单外的文件
- 空列表短路，不执行

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
   凡 message 带 Svn-Rev: N 的（baseline commit）
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
}
```

`engine/layout.ts`、`engine/svgRenderer.tsx` **一行不改**——它们只认 `parents`。

## 11. 前端改动

| 组件 | 变化 |
|---|---|
| `GraphView` / `layout.ts` / `svgRenderer.tsx` | **零改动** |
| `LocalChangesView` / `DiffView` / `FileTree` | **零改动**（就是 git status / git diff） |
| `StashView` | 近乎零改动 |
| `BranchView` | 基线分支加特殊标记 + 禁止在其上提交 |
| `Toolbar` | 按 `capabilities`：Fetch / Pull / Push → Update / Deliver；隐藏 Worktree / Tag / PR |
| `CommitRow` | `vcs === 'svn'` 时显示 `r12346` 而非短 hash，换配色 |
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
| S3 `git checkout` 失败 | **Unity 开着锁住了 .cs / .dll** | `git stash pop` | 「Unity.exe (PID 1234) 占用了 X 文件」——复用现有 `engine/file_lock.rs` |
| S4 校验不通过 | 外部改过工作副本 | 切回 ORIG_BRANCH + stash pop | 基线失同步，进 §9.4 恢复向导 |
| S5 网络 / 认证失败 | VPN 没连、凭据过期 | 切回 ORIG_BRANCH + stash pop | 原样展示 svn stderr |
| S5 树冲突 | 服务器删了 / 换掉了某个目录 | 同上 | 明确说明是树冲突，给出受影响路径 |
| S9 rebase 冲突 | 正常的代码冲突 | **不回滚**，停在冲突态 | 进入冲突解决模式 |
| S10 stash pop 冲突 | 未提交改动与新基线冲突 | 不回滚，**stash 保留不删** | 同上 + 明确告知 stash 还在 |
| D5 各类失败 | 见 §8.6 | 见 §8.5 | 见 §8.6 |
| 任意步骤客户端崩溃 | — | 靠 `mygitclient-sync.json` | 重开后提示「上次同步未完成」 |

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
| 6 | exclude 规则生成：Windows 反斜杠、盘符、大小写 | 11 |
| 7 | 基线映射 → svn 节点的图合成，含孤儿 baseline | 8 |
| 8 | `svn update` 带来的树冲突（wc2 删掉一个目录） | 1 |
| 9 | 未初始化的 SVN 工作副本（有 `.svn` 无 `.git`）的识别 | 3 |
| 10 | **白名单外的文件绝不出现在任何 git / svn 命令的参数里** | 3、4 |
| 11 | 同步进行中的并发 invoke 被正确拒绝 | 5 |

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

## 16. 分期

每期独立 spec → plan → PR。

| 期 | 内容 | 验收 |
|---|---|---|
| **Phase 0** | `trait Vcs` + `GitBackend`（现有代码搬家，**行为零变化**） | 现有全部测试绿 + 手工 E2E 与改动前无差异 |
| **Phase 1** | 仓库识别、配置、exclude 生成、初始化向导、`SvnClient`（info / status / log 的 XML 解析）、状态判定与失同步检测（**只报警不恢复**）、混合图只读 | 能打开 SVN 仓库，本地 git 全功能可用，看得到 SVN 历史。**还不能 update / deliver** |
| **Phase 2** | `sync_in` 完整流程 + 全部失败路径 + 中断持久化 + 冲突解决 UI | 能更新 |
| **Phase 3** | `sync_out` + 确认关口 + 精确回滚 + 传输中断查证 | 能交付，闭环 |
| **Phase 4** | 白名单外改动视图与 revert、`.git` 误提交三层防护、svn 节点 diff | 补齐日常缺口 |

**Phase 0 单独拆出是有意的。** 它是纯重构、零行为变化，可独立验证，风险最低；
混进 Phase 1 的话，一旦现有 Git 模式出问题，就分不清是重构引入的还是新功能引入的。
对齐 `AGENTS.md` §1.2「不要把无关重构和业务修改混在一起」。

**Phase 1 结束时已是一个有意义的交付点**——本地 git 的暂存区、分支、stash 全部可用，
只是 update / deliver 还得回 TortoiseSVN 手工做。

## 17. 环境前置

- **`svn` CLI 必须在 PATH 上。** 开发机（2026-09-12 实测）当前没有 `svn` /
  `svnadmin`，需先安装。TortoiseSVN 默认不装命令行工具，需在安装器里勾选
  「command line client tools」；或用 `scoop install svn`（SlikSVN，自带 `svnadmin`）
- 打开 SVN 仓库时若检测不到 `svn`，客户端必须给出明确的安装引导，而不是抛裸错误

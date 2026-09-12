# SVN + Git 混合模式 设计规格

- 日期：2026-09-12
- 状态：设计已确认，未实现。已过一轮对抗性 review，五条 high 全部修入，见 §18
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
文件时，根的 revision 纹丝不动，`baselineRev == wcRev` 照样成立——§9 那道失同步门禁
会被完整绕过，基线开始撒谎而界面上一片祥和。

#### 6.6.2 分层检测

```text
baselineRev = svn/base 顶端 commit 的 Svn-Rev trailer     ← 权威记账
rootRev     = svn info --xml 的 <entry revision>          ← O(1)，本地读，毫秒级
nodeRevs    = svn status -v --xml -- <白名单路径>          ← 白名单内每个已版本控制
                                                             节点的 revision 集合
                                                             本地读，范围限于代码
remoteRev   = svn info --xml <URL> 的 revision            ← 走网络，异步，失败不阻塞
```

**正常状态的不变量**：`nodeRevs` 是单元素集合，且等于 `{baselineRev}`。

任何偏离都判为**基线失同步**：

| 观察 | 含义 |
|---|---|
| `nodeRevs` 有多个值 | mixed revision，外部提交或局部 update 过 |
| `nodeRevs = {N}` 但 `N != baselineRev` | 整体被外部推进或回退 |
| `rootRev != baselineRev` | 根目录被动过 |

未版本控制的文件（git 新增但还没 `svn add`）不参与该集合；已 `svn add` 尚未提交的
节点没有 revision，同样跳过——但它们的存在本身就说明有外部登记，单独报警。

#### 6.6.3 检查频次分层

`svn status -v` 要走一遍白名单子树，比 `svn info` 贵得多（但仍是纯本地、范围限于
代码文件，不是几十 GB 的美术）。所以分两档：

| 时机 | 检查 | 理由 |
|---|---|---|
| 每次 `repo-changed` 刷新 | 只查 `rootRev`（O(1)） | 高频，抓得到大部分情况，代价可忽略 |
| 打开仓库、`sync_in` / `sync_out` 第一步、恢复向导前 | **完整 `nodeRevs` 校验** | 这几处是正确性的闸门，必须问到底 |

漏检的代价只是**延迟发现**——真正会造成数据损坏的操作全都在第二档后面，不存在
「绕过闸门直接交付」的路径。

`remoteRev` 走网络，按现有 `refreshPullRequests` 的模式处理：不 await、失败静默
（读不到就不显示落后数，不弹错）。`remoteRev > rootRev` 时显示「落后 N 个 revision」。

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
3. 当前不在 `svn/base` 上
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
S1   工作区不干净 → git stash push -u -- <白名单>
     立刻 git rev-parse refs/stash 取不可变 OID
     git update-ref refs/mygitclient/sync/<opid> <OID>
     OID 写进 journal                                      ← 不记 stash@{0}，见 §7.6.1
S2   ORIG_BRANCH 写进 journal
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
  "stashOid": "f2208125aad7d35d63ee9495ad0a6cc3c1ce21be",
  "baselineRevBefore": 12345,
  "svnAdded": ["Assets/Scripts/JumpConfig.cs"],
  "svnDeleted": ["Assets/Scripts/Old.cs"],
  "expectedStartRev": 12345,
  "commitMessage": "…"
}
```

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
   改动不该被悄悄排除、也不该被悄悄带上。硬拦住，让用户自己决定
3. 不在 rebase / merge 中途
4. 当前分支不是 `svn/base`
5. `remoteRev == wcRev`（走一次网络）；不等则要求先更新
6. 拿锁

### 8.2 步骤

```text
D0   挂起 watcher；写 journal，phase=prepare                ← 任何修改之前
D1   git diff --name-status --no-renames svn/base..HEAD     ← 净变更清单
D2   按白名单过滤清单                                        ← 双保险
D3   把清单给用户确认（A/M/D 分类、文件数、提交信息）         ← 不可撤销前的关口
D3.5 再查一次 remoteRev，写进 journal 的 expectedStartRev    ← 压缩竞态窗口 + 查证锚点
D4   phase=svn-staged，svnAdded / svnDeleted 写进 journal
     A → svn add --parents <files>
     D → svn delete <files>
     M → 无需操作
D5   phase=committing
     svn commit --non-interactive --with-revprop mygitclient:opid=<opid>
       -m <msg> -- <显式 target 列表>
D6   phase=post-commit  ← 过了这一步服务器状态已不可撤销
     svn update --accept postpone -- <白名单路径>            ← 只更新白名单，见 §8.2.1
D7   svn status -v --xml 读新的 nodeRevs → r_new
D8   推进基线（见 §8.2.2）
D9   提示「work 分支已交付，可删除」——不自动删
D10  删除 journal；恢复 watcher
```

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

D5 成功之后，磁盘上白名单文件的内容 = **当前工作分支 tip 的内容** = 服务器 r_new
的内容。新基线的树就应该是它。

`svn/base` 是工作分支的祖先，所以可以纯快进：

```text
D8a  git branch -f svn/base HEAD        ← 只动指针，工作目录一个文件都不碰
D8b  git checkout svn/base              ← 与 HEAD 同点，工作目录无变化
D8c  git add -A -- <白名单>              ← 收进 D6 拉下来的他人改动（若有）
     git commit --allow-empty -m "svn: r_new …  Svn-Rev: r_new"
D8d  git checkout <ORIG_BRANCH>
     git merge --ff-only svn/base       ← 纯快进，不可能冲突
```

D8a 先动指针再 checkout 是关键：**反过来先 `git checkout svn/base` 会把工作目录
退回旧基线内容**，而此刻 SVN 的 base 已经是刚提交的新内容——SVN 会把这看成「你
把自己刚提交的东西全撤了」，正是 §9.2 描述的基线撒谎场景，只不过是自己造出来的。

D8d 的 `--ff-only` 是断言而非优化：走到这里必然是快进，不是快进就说明前面某步的
假设崩了，应当报错停下而不是自作主张 merge。

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
按优先级匹配：
   ① revprop mygitclient:opid == 本次 opid      ← 首选，精确幂等键
   ② 无 revprop 时：作者相同 且 changed-paths 集合与本次清单完全一致
   ↓
   ├─ 命中 → 提交其实成功了。不回滚，记下命中的 revision，直接跳到 D6
   ├─ 区间内无命中 → 真失败，按 §8.5 回滚
   └─ 仍连不上 → 什么都不做，phase 停在 committing，
                 界面常驻「交付状态未知」，等网络恢复后再点一次查证
```

查**完整区间**而不是「最近 N 条」是硬要求：断网可能持续几小时，N 取多大都是猜。
区间的起点来自提交前就已落盘的 `expectedStartRev`，不依赖任何事后推断。

匹配 changed-paths 而不是 message，是因为路径集合由本次交付的净变更唯一确定，而
message 是用户自由输入的文本。

#### 8.6.3 关于 revprop 幂等键

`svn commit --with-revprop mygitclient:opid=<uuid>` 在提交时写入自定义 revprop。
按 SVN 的设计，`pre-revprop-change` 钩子管的是**事后修改** revprop，提交时携带
revprop 不受该钩子限制——但**这一条必须在 fixture 里实测确认**，公司服务器也可能
另有限制。

因此实现上：revprop 是**首选**路径，changed-paths 匹配是**必须存在的兜底**。
若服务器拒绝 revprop，客户端应降级到 ② 并在界面上说明查证精度降低，而不是让整个
交付流程失败。

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
⓪ 写 journal（flow=recover），phase=start            ← 任何修改之前，见 §7.6
① git stash push -u -- <白名单>；立刻取 OID 并用
   refs/mygitclient/sync/<opid> 钉住，写进 journal   保护未提交改动，可逆
② 原分支与 origHead 写进 journal
③ git checkout svn/base                 磁盘 = 旧基线内容
④ svn status -v --xml，取出白名单内、状态为 M/! 的**显式文件列表**
   ⚠️ 列表为空 → 跳到 ⑥，绝不执行空参数的 svn revert
⑤ svn revert <显式文件列表>              恢复到服务器 base 内容
⑥ git add -A -- <白名单> && git commit --allow-empty
                                        新基线，Svn-Rev 取自 ⑤ 之后重读的 nodeRevs
⑦ git checkout <原分支>
   git rebase --empty=drop svn/base      已被误传上去的 commit 会变成空 commit，自动丢弃
⑧ git stash apply <OID>；成功后 drop 该条目并删掉钉住的 ref
⑨ 删除 journal
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
| S3 `git checkout` 失败 | **Unity 开着锁住了 .cs / .dll** | `git stash apply <OID>` | 「Unity.exe (PID 1234) 占用了 X 文件」——复用现有 `engine/file_lock.rs` |
| S4 校验不通过 | 外部改过工作副本 | 切回 ORIG_BRANCH + stash apply | 基线失同步，进 §9.4 恢复向导 |
| S5 网络 / 认证失败 | VPN 没连、凭据过期 | 切回 ORIG_BRANCH + stash apply | 原样展示 svn stderr |
| S5 树冲突 | 服务器删了 / 换掉了某个目录 | 同上 | 明确说明是树冲突，给出受影响路径 |
| S5 白名单**外**文件冲突 | 美术资产有本地修改（§7.2.1 已确认过） | 同上 | 明确指出是 git 管不到的文件，**引导去 TortoiseSVN**，不假装能修 |
| S9 rebase 冲突 | 正常的代码冲突 | **不回滚**，停在冲突态 | 进入冲突解决模式 |
| S10 stash apply 冲突 | 未提交改动与新基线冲突 | 不回滚，**stash 与钉住的 ref 都保留不删** | 同上 + 给出 stash 的 OID |
| D5 各类失败 | 见 §8.6 | 见 §8.5 | 见 §8.6 |
| **D6 update 失败 / 冲突** | 服务器在 D5 之后又有提交，且与白名单文件冲突 | **不可回滚**——D5 已落地 | 明确告知「提交已成功、基线尚未推进」，journal 停在 `post-commit`，提供「重试 D6 起」的入口 |
| **D8d 不是快进** | 前面某步的假设崩了 | 停下不动 | 报错并导出 journal，**不自动 merge** |
| 任意步骤客户端崩溃 | — | 靠 §7.6 的写前 journal | 重开后按 `phase` 给出对应的继续 / 放弃入口 |

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
| 16 | **逐 phase 崩溃恢复**：在 S1 后、D4 后、D5 后各杀一次进程，重开必须能按 journal 给出正确的继续 / 放弃路径（§7.6） | 2、5 |
| 17 | **stash OID 抗漂移**：stash 之后由外部再压一个 stash，恢复必须仍取回正确的 WIP（§7.6.1） | 1、5 |
| 18 | **D8 分支编排**：交付后基线确实推进到新内容，且工作分支是快进；断言不会出现「基线内容是旧的却标着新 revision」（§8.2.2） | 1 |

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

### 2026-09-12 · 对抗性 review 后的五处修正

初版（commit `928923f`）经 Codex 对抗性 review，五条 high 全部成立，已修入本文。
这些都是**会破坏数据保护与恢复不变量**的缺陷，不是措辞问题。

| # | 缺陷 | 核验方式 | 修正 |
|---|---|---|---|
| 1 | 反向 ignore 规则漏掉嵌套源码：`/Assets/Scripts/**` 排除了子目录，git 不再递归，后续 `!…/*.cs` 对深层文件永不生效 | **临时 git fixture 实测**：3 个 `.cs` 只纳入 1 个 | §6.3 补 `!/Assets/Scripts/**/`；测试改为断言 `git ls-files`，必须覆盖两层嵌套 |
| 2 | 单个根 revision 无法表示 SVN 的 mixed-revision 工作副本，外部提交深层文件时失同步门禁被完全绕过 | 未实测（本机无 svn），按 SVN 既定语义接受 | §6.6 改为白名单 `nodeRevs` 集合校验 + 检查频次分层 |
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

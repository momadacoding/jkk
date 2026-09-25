# SVN + Git 混合模式 设计规格

- 日期：2026-09-12
- 状态：设计已确认，未实现。已过五轮对抗性 review + 三轮外部事实复核（`git svn` 方案 / 真实远端工作副本 / externals 布局），结论与修法边界见 §18
- 前置可行性（P4 不跨仓库、P5 代码 external 未被钉住）**已于 2026-09-13 由用户确认通过**，见 §16.0
- 分期：见 §16。本文覆盖全部 5 期的架构，各期另出实现计划

## 1. 背景与目标

公司的 Unity 游戏项目用 SVN 管理，同一个仓库里既有代码也有几十 GB 美术资产。
SVN 对美术、策划职能足够好用，不可能整体迁移到 git；但对程序来说缺了太多东西：
没有本地暂存区、没有廉价分支、没有 stash、无法把一堆改动拆成几次干净的提交。

本项目要做的是：**让程序同学在不改变团队 SVN 工作流的前提下，本地获得完整的
git 体验。**

### 1.1 核心命题

方案的全部基础是这一条：

> **工作目录同时被 SVN 和 git 覆盖，两者互不知晓，只通过磁盘上真实的文件内容
> 交汇。**

git 管白名单内的代码文件，负责暂存、分支、stash、本地提交；SVN 管整个工作目录，
只在「更新」和「交付」两个时刻出场。

在 SVN 眼里，你永远只是「一个停在 r12345、有一批本地修改的工作副本」——它看不到
你的 git commit、分支和 stash。

#### 1.1.1 「SVN 侧」不一定只有一个工作副本

初版这里写的是「工作目录**同时是一个** SVN 工作副本」。这个「一个」在真实项目里
**不成立**：`svn:externals` 会把工作目录拼成**多个互相独立的嵌套工作副本**，每个有
自己的 `.svn`、自己的 URL、自己的 revision，跨仓库时还有自己的 UUID（§6.6.4 实测）。

Unity 工程常见的拼法就是这样——项目根之下 `Assets/`、`Code/`、`Documents/` 各是一条
external，指向不同位置。**代码很可能根本不在项目根那个工作副本里。**

所以准确的表述是：

> git 仓库覆盖**整个工作目录**（`.git` 放项目根），而它管的白名单文件可能分布在
> **若干个嵌套 SVN 工作副本**里。git 不在乎这件事——它只看磁盘；但**所有 SVN 侧的
> 记账都必须落到「白名单文件实际所属的那个工作副本」上，而不是项目根**。

由此派生出两条硬约束，见 §6.5 P4 / P5：白名单必须全部属于**同一个仓库**，且所在
external 不得被 `-r` 钉死。两条都有实测支撑（§6.6.5）。

### 1.2 非目标

明确排除，不要顺手实现：

- 给美术、策划使用。本客户端的 SVN 模式只服务程序职能
- SVN 分支管理（`svn switch`、`svn merge`、分支创建）
- SVN 文件锁（`svn lock` / `unlock`）；externals 的**管理**（创建 / 修改 / 删除
  `svn:externals` 定义）。⚠️ 但 externals 本身**必须被正确处理**，不能无视：真实
  Unity 工程常把代码放在 external 里，白名单很可能**整个落在其中**。见 §1.1.1、
  §6.6.4、§6.6.5
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

##### ⚠️ 2026-09-13 补：上面这条 depth-exclude 论证在真实布局下已被削弱

用户随后指出：公司 Unity 工程**本来就是用 externals 拼起来的**，项目根之下 `Assets/`、
`Code/`、`Documents/` 各指向不同位置。也就是说，**代码与美术的目录边界是团队早就画好
的**，根本不需要 `--set-depth=exclude` 去人为切分——上面那个核心反对理由
（「`Assets/Scripts/` 下的 `.psd` 会两边都不认领、直接从磁盘消失」）在这种布局下
**不成立**。

诚实记账：在 externals 拼装的布局下，否决 `git svn` 的理由**只剩两条**——

1. **测试侧**：`file://` 不通，fixture 策略要推倒重来（不受布局影响，仍然成立）
2. **丢掉 TortoiseSVN 对代码的兜底**（不受布局影响，仍然成立）

而「depth-exclude 与 D6 的 AND 语义冲突」这一条**作废**。

**本轮不做重新评估**——现有 Overlay 方案已经走得很深，而 `git svn` 路线的测试成本
仍然是硬的。但这条必须写在这里，否则就是重蹈 D2 的覆辙：留一条自己已经知道站不住的
论证，等着下一个人再问一遍。重估的触发点是 §16.0 核对完成之后、Phase 1 实现计划开始
之前。

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
| **`git svn clone -r HEAD svn://svn.code.sf.net/…/trunk/contrib`** | ✅ **成功**，11.4s（521 KB 子树，公网真实服务器） |
| 克隆产物的 trailer | `git-svn-id: svn://…/contrib@29785 cc8e0003-…`，UUID 与 `svn info` 完全一致 |
| `git svn find-rev r29785` | ✅ 返回对应 sha |

结论必须说**精确**：不是「`git svn` 在本机不能用」，而是 **只有 `file://` 不能用**。
同一个 `git svn` 对着真实远端（`svn://`）工作完全正常。

这条边界决定了成本落在哪：

- **生产侧：可用。** `-r HEAD` 浅克隆实测可行，D2 担心的「全量历史太贵」本来就有现成
  解法。这一侧不构成否决理由
- **测试侧：不可用，且不会随环境改善。** `file://` 不通意味着 §14.1 的 fixture 策略
  （`svnadmin create` + `file://` + 两个工作副本）对任何以 `git svn` 为核心的方案
  **当场失效**，必须另起一个真实 SVN 服务端（svnserve 或 Apache），而 `AGENTS.md` §8
  把「git 写操作测试必须自建临时 fixture」定成硬约束。**这才是引入 `git svn` 最贵的
  一项**

**仍未验证**：https（ra_serf）下的行为。`svn://` 与 https 同属网络 RA 模块，风险已大幅
下降，但没实测就是没实测。

⚠️ 本设计**不采用** `git svn`，所以 §14.1 的 fixture 策略**照旧有效**。不要把上面这段
读成「现有测试方案有问题」——它说的是「如果改用 `git svn`，测试方案要推倒重来」。

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
│   ├── info\attributes           ← `* -text`，关掉 git 的 EOL 转换（§7.3.0）
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
- **另有一个只返回字节的变体 `run_svn_bytes(path, args) -> Result<Vec<u8>>`**，专供
  `svn cat`。`run_svn` 走 `String::from_utf8_lossy`，那是给 `--xml` 输出准备的；源码
  文件很可能不是 UTF-8（GBK 的 `.cs` 在国内 Unity 工程里很常见），lossy 会把每个非法
  字节换成 U+FFFD，录进基线就是一棵被悄悄改坏的树。见 §16.2 末
- 所有调用带 `--non-interactive`。理由与 `GIT_TERMINAL_PROMPT=0` 完全相同：
  GUI 应用没有终端，svn 一旦等待 stdin 输入凭据就会永久挂死并占住 Tauri 线程池
- 需要机器可读输出的一律用 `--xml`（`info` / `status` / `log`）
- **`--ignore-externals` 按子命令区分，不能「一律带」。** 实测（2026-09-13，
  SlikSvn 1.14.2）各子命令的参数集：

  | 子命令 | 接受 `--ignore-externals` | 本设计怎么用 |
  |---|---|---|
  | `status` | ✅ | **必须带**。不带会让 externals 指向的**另一个仓库**的节点混进 `nodeRevs`，后果见 §6.6.4 |
  | `update` | ✅ | **必须带**（仅 pinned 模式下的根级 update，见 §6.5.3）；D6 / S5 的常规用法见各自章节 |
  | `info` | ❌ **拒绝** | **绝不带**。它默认就不遍历 externals，externals 要靠 opt-in 的 `--include-externals` 才进来，所以本来就不需要这个参数 |
  | `log` | ❌ 无此选项 | 不适用 |
  | `diff` | ❌ **拒绝** | 不需要（实测不递归进 externals） |
  | `cat` | ❌ 无此选项 | 不适用——它按单个文件取，不遍历目录。**只用 `-r BASE` 形式**，见 §16.2 |

  ```text
  svn info --xml --ignore-externals .
    → Subcommand 'info' doesn't accept option '--ignore-externals'   exit 1
  ```

  这不是可有可无的写法问题：`svn info` 是仓库识别（§6.2.1 的 UUID 校验）、
  `rootRev`（§6.6.2）和记账单位判定（§6.6.5）的唯一入口，**多带一个参数会让这三处
  全部以 exit 1 失败**，仓库根本打不开。
  ⚠️ `status` / `update` 那条的作用域是「**在白名单所属的那个工作副本内部**」——白名单
  本身可能就落在某个 external 里，那时从项目根加 `--ignore-externals` 会把白名单整个
  跳过，见 §6.6.5
- **`--xml` 的输出必须用真正的 XML parser 解析，禁止行内正则。** svn 把每个属性单独
  放一行，`<wc-status` 与 `revision="…"` 不在同一行，行内正则会静默扫出空集，见 §6.6.4
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
获取：  以「原子创建且必须不存在」的语义打开（POSIX O_EXCL / Windows CREATE_NEW），
        并**一直持有这个句柄**直到流程结束
        写入 { pid, startedAt, opid }
释放：  关闭句柄并删除文件（§8.2.4 ⑤ / S11 / D10 都要包含这一步）
陈旧锁：只有当**独占打开成功**（= 没有任何进程还持有那个句柄）时才允许接管，
        接管必须写日志并告知用户
        ⚠️ 不能只看时间戳——一次 svn commit 卡在网络上几分钟是正常的
        ⚠️ 也不能只看文件在不在——崩溃之后文件还在，但已经没人持有了
```

#### 5.1.2 存活判据为什么是「能不能独占打开」，不是「pid 还在不在」

**2026-09-25 修订。** 本节原写的是 pid 探测（「只有当记录的 pid 已不存在时才允许
接管」）。Phase 2b 施工时在 Windows 上实测，改成句柄探测，理由是实测出来的三条。

实测（三组探针 + 一条跨进程集成测试）：

| 观察 | 结果 |
|---|---|
| 持锁进程活着时，另一方尝试以 `share_mode(0)` 打开 | `BLOCKED`（`ERROR_SHARING_VIOLATION`，`raw_os_error() == 32`） |
| 持锁进程被 `Stop-Process -Force` 杀掉后，锁文件 | **仍然存在** |
| 同上，另一方再尝试独占打开 | `ACQUIRED` —— 句柄已由操作系统释放 |
| 持锁期间，第三方以 `FILE_SHARE_READ\|FILE_SHARE_WRITE` 只读打开 | **读得到** `{pid,…}`，满足 §13「显示持锁进程的 pid 与开始时间」 |

三条理由：

1. **崩溃安全由操作系统保证**，不依赖我们的清理代码跑到——而崩溃恰恰是清理代码
   跑不到的那种情况。
2. **没有 pid 复用问题。** pid 探测会把「一个无关新进程恰好复用了死进程的 pid」
   误判成「持锁者还活着」，那把锁于是**永远**接管不了，用户被永久卡死。
3. **pid 探测信的是文件内容，而不是「持有」这个事实本身。** 施工时的变异测试
   暴露出这一条比 pid 复用更基本：持锁者在「拿到锁」与「写完元数据」之间还有
   一条缝，元数据也可能写了一半（断电、磁盘满）。这两种情形下读不出 pid，
   pid 探测会判成「没人持有」并**直接接管一把真有人在用的锁**——方向恰好是
   危险的那一边。句柄探测问的是操作系统，不受影响。

`pid` / `startedAt` 仍然落盘，§13 要求显示给用户，只是不再作为判据。

非 Windows 上拿不到等价的跨进程语义（本 crate 声明的 `rust-version` 是 1.77.2，
std 的 `File::try_lock` 1.89 才稳定），按 `engine/file_lock.rs` 已有的方式
拆 `#[cfg(windows)]` / `#[cfg(not(windows))]`。非 Windows 分支**不做互斥**：
独占打开恒成功，探测恒为「没人持有」（`Liveness::Free`）——守卫放行、残锁可接管。

**不是**「无法判定存活」（`Liveness::Unknown`）。那条路守卫关门、接管不动手，
于是一次崩溃留下的锁文件会在非 Windows 上**永久**挡住所有写操作，且没有任何
入口能解开。「没有互斥」的代价是两个进程可能并发同步，与本期之前的状况相同；
「永久卡死」是新引入的退化。本项目主力平台是 Windows（`AGENTS.md` §10）。

⚠️ 实现上还有一个**顺序**要求：释放时必须**先关句柄再删文件**。`Drop` 在字段释放
之前运行，此刻句柄还开着，而锁不能给 `FILE_SHARE_DELETE`（给了就等于允许别的进程
删掉我们正持有的锁），于是 `remove_file` 会以共享冲突失败。失败若被吞掉，每一次
**正常**同步结束后都会留下一个残锁，下一次获取就会打出「上次同步没有正常收尾」
——一条永远为假的告警。这条同样由变异测试抓到。

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

⚠️ **白名单落在多个嵌套工作副本里时，`svnUrl` 只是其中一个 WC 的 URL**（§6.6.5），
它代表不了整体。所以：

| 用途 | 用哪个 URL |
|---|---|
| 配置校验（UUID 比对） | **每个 `wc(P)`** 各比一次，全部 UUID 必须相同（§6.5 P4、§6.5.5） |
| `remoteRev`（`svn info --xml <URL>`） | 任一 WC 的 URL 都可以——它返回的是**仓库 HEAD**，与子树无关 |
| §6.8 基线体检的 `--old=` | **每个 `wc(P)` 用它自己的 `url(P)`**（§6.6.5） |
| **§8.6.2 的断网查证** | **`Repository Root`**，不是 `svnUrl`。子树 URL 的 `svn log` 看不到只碰了别的 WC 的那次提交，实测见 §8.6.2 |
| 基线 trailer 的 `<url>` | `Repository Root`（§6.4.1） |

`svnUrl` 因此退化成「一个用来做 UUID 交叉校验的锚点」，**不要**再把它当成「这个仓库
的地址」直接拼进命令。

其余字段：

| 字段 | 规则 |
|---|---|
| `baselineBranch` | **固定为 `svn/base`**，不再声称可配。全文所有流程都硬编码了这个名字，留一个可配字段只会让两者不一致。字段保留是为了向前兼容，值不是 `svn/base` 就阻断并说明 |
| `whitelist` | 必须是**仓库内相对路径**。拒绝：绝对路径、盘符（`C:`）、UNC、`..` 段、空串、`/`、`.`、以 `.git` 或 `.svn` 开头的路径、以 `:` 开头的 pathspec magic（`:(exclude)` 之类）。**允许**落在 `svn:externals` 内（那正是目标场景），但受两条额外约束：所有白名单根的 `Repository UUID` 必须相同，且所在 external 不得被 `-r` 钉死——见 §6.5 P4 / P5、§6.6.5。分隔符统一规范化为 `/` |
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
.svn/                         ← 必须有，且必须在最后一行！见下
```

#### 为什么 `.svn/` 必须排在最后一行

白名单**可以**落在 external 内（§6.5 P4），而 external 根自己就有一个真实的 `.svn`
目录（§6.6.5 E1）。上面那条 `!/Assets/Scripts/**/` 重新纳入了白名单内部的**所有**
子目录，`.svn` 也在其中；之后只要有任何一条再入规则命中它里面的文件，SVN 的元数据
就会进 git 对象库。

`git check-ignore` 的裁决规则是**最后一条匹配的模式说了算**，所以 `.svn/` 必须放在
所有再入规则**之后**；不带前导 `/` 则在任意深度生效。

2026-09-13 临时 git fixture 实测，白名单 `Code/`：

| 配置 | 无 `.svn/` 兜底时 `git ls-files` | 补上之后 |
|---|---|---|
| `extensions: [".cs", ".db"]` | 含 `Code/.svn/wc.db` ❌ | 只有 `.cs` ✅ |
| `extensions: ["*"]`（§6.3 明文推荐的全纳入写法） | 含 `Code/.svn/wc.db`、`Code/.svn/pristine/aa/*.svn-base` ❌ | 只有代码文件 ✅ |

补上之后 `git check-ignore -v Code/.svn/wc.db` → `.git/info/exclude:6:.svn/`。

**后果不是「恢复一份旧的 SVN 数据库」，而是直接损坏工作副本。** 一旦 `wc.db` 被 git
跟踪，任何一次 `git checkout` 都会在一个**正在使用的** SVN 工作副本底下把它换成另一个
版本。§6.2.1 那条「拒绝以 `.svn` 开头的白名单路径」拦不住这种情况——它校验的是用户写的
条目本身，而这里是递归再入规则在深层命中的。

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

#### 6.3.1 工作目录里的 `.gitignore` 优先级高于 `.git/info/exclude`

§3 那句「白名单配置和生成的忽略规则都放在 `.git/` 内部，**不用**工作目录里的
`.gitignore`」说的是**我们不写**它，不是 git 不读它。git 的忽略源优先级是：

```text
工作树里的 .gitignore（越深越优先）  >  .git/info/exclude  >  core.excludesFile
```

`git add` 的 pathspec 限定扫描范围，不会取消上述忽略规则。`core.excludesFile` 的
优先级较低，**工作树里的 `.gitignore` 却能盖掉我们生成的再入规则**。SVN 工作副本
里完全可能已经躺着一个——团队提交过、或者从别处拷来的 Unity 模板；最终都以实际
候选索引的集合核对为准。

2026-09-13 临时 git fixture 实测，白名单 `Code/` + `extensions: [".cs"]`：

```text
Code/.gitignore 内容为一行 *.cs
  git ls-files                     → 空
  git check-ignore -v Code/Top.cs  → Code/.gitignore:1:*.cs   Code/Top.cs
```

这正是上面刚说过的「最危险的一类缺陷」的另一条路径：**用户以为源码受 git 保护，
实际上一个都没进去**。而且它不会让初始化失败——只要白名单里还剩别的可提交文件，
`git commit` 照样成功；§6.8 的基线体检也查不出来（它比的是磁盘与服务器，不比
git 跟踪了谁，见 §15 第 20 条）。

**每次录制基线都必须做「纳入集合核对」**，不能只在初始化时检查。正常初始化之后，
同事也可能通过 SVN 新增 `Code/.gitignore`（`*.cs`）和 `Code/New.cs`；S5 拉下两者后，
`git add -A` 仍会成功，却漏收 `New.cs`，而 `nodeRevs` 单值、§6.8 体检无差异。

核对与录制必须使用同一份候选索引：

```text
期望集合 = **服务器在 rN 上**白名单内的 W-file，永久排除任意深度的 .svn/ 与 .git/ 元数据
候选索引 = 临时索引；初始化 / Phase 1 R2 从空索引开始，其余录制入口从当前 HEAD 树开始
           在该索引上 git add -A -- <W-path>，不修改真实 index、HEAD 或工作文件
实际集合 = 该索引的完整 git ls-files -z 结果（不能再按扩展名过滤，以免藏掉多收的文件）
两者不一致 → 不录制、不推进基线；对漏收文件用
              git check-ignore --no-index -v --non-matching -- <差异文件> 定位原因
两者一致   → 该索引 git write-tree 得到 TREE，录制时直接使用这个 TREE
```

**期望集合是「服务器上的」，不是「磁盘上的」。** 初版写的是磁盘，因为那时每个录制入口
都要求工作副本干净，两者恒等。§16.1 放松之后它们会分开，而基线的语义（§6.4）从头到尾
说的都是服务器，所以这里必须写服务器。求法**不走网络**，由本地推导：

```text
期望集合 = 磁盘 W-file 集合  −  { 状态为 A / ? 的 }  +  { 状态为 D / ! 的 }
                                  服务器 rN 上还没有       磁盘上没有但服务器有
```

工作副本干净时后两项为空，退化成初版的「磁盘 W-file」，S5 之后的 S7、恢复⑥、D7a 三个
入口都落在这一档，行为逐字不变。真正会用到加减项的只有初始化与 Phase 1 的重建。

`svn ls -R <url>@N` 那条**服务器侧**核对（`svn_server_set`）是独立的第二道，走网络、
可能失败，所以它只出提示、不参与上面的判定——两者不能互相顶替：本地推导挡的是「录错了
哪些文件」，服务器侧挡的是「`--set-depth=exclude` 这类磁盘上压根看不见的漏收」。

路径统一为仓库相对形式再比较，集合必须同时检查**漏收**和**多收**。不使用 `git add -f`
绕开检查，也不自动修改团队的 `.gitignore`。录制基线用 `git commit-tree TREE` 创建
commit（父提交保持各流程原有定义，首个基线无父），再以旧 OID 作比较条件更新
`refs/heads/svn/base`；**不得核对一次后又重新 `git add` 另录一棵树**。创建相同 tree 的
新 revision 基线仍然合法，`commit-tree` 不要求内容变化。

HEAD 已附着在 `svn/base` 的入口在发布后用 `git read-tree TREE` 对齐真实索引，**不带
`-u`、不改磁盘**；Phase 1 R2–R5 全程不碰真实索引。`TREE`、新 commit OID 和发布前的
基线 OID 在使用 operation journal 的流程中必须先落盘，重开时按 OID 判断发布是否已经
完成，不重复录制。初始化与 Phase 1 的创建 / 更新也必须比较旧基线 OID，失败就停止。
临时索引只在本次操作自己的路径下创建、清理。

| 核对时机 | 失败时的状态 |
|---|---|
| 初始化，首个基线之前 | 不创建基线，回退本次初始化创建的 `.git` |
| `sync_in` / `sync_out` / 恢复的前置检查，以及重新加载配置时 | 用真实索引的**副本**预演 add 后核对；失败就不启动流程。预演不代替 stash，也不能复用为后续基线的 TREE |
| S7，S5 完成之后 | 保留旧基线与 journal / stash，停在录制阶段，提示修正规则后继续 |
| D7a，D6 完成之后、D8 移动任何基线指针之前 | D5 已成功；停在 `post-commit`，保留交付 ref，不回滚、不重复提交 |
| §9.4 ⑥，⑤b 完成之后 | 不推进旧基线，保留恢复 journal / stash，等待继续 |
| Phase 1 R2，R3 创建 commit 之前 | 中止，只清理临时索引，HEAD / 真实 index / 工作目录及基线指针均不变 |

录制阶段失败时保留原流程的恢复信息，释放操作锁、恢复 watcher 并显示「同步未完成」。
继续入口在拿锁后复核原分支 / 基线 OID、W-file 的 SVN 内容状态及记录的单值 revision；
有新代码修改或 revision 变化就停止，先处理这些变化，不能把等待期间的 WIP 录成基线。
满足前提后才重建候选索引、重新核对；不能用一次新的 S0 / D0 覆盖旧 journal。
已完成 SVN update 的流程不能把「停止录制」说成「工作副本没变」。

这条核对是**行为断言**，不是针对 `.gitignore` 这一种成因写的特判——`core.excludesFile`、
用户手工改过的 `.git/info/exclude`、规则生成器自己的 bug，全都会在这里暴露。与 §6.8
的基线体检**互不替代**：体检证明「磁盘 == 服务器」，这条证明「git 跟踪的集合 == 配置
声明的集合」（§15 第 20 条）。

#### 6.3.2 「白名单」是两个不同的集合，全文必须分清

这是本文档已经踩过的一个坑（§8.2.1 的论证就是这么写错的）。**「白名单」在两种粒度上
被使用，它们不相等**：

| 记号 | 定义 | 谁用它 |
|---|---|---|
| **W-path** | 配置里 `whitelist` 声明的**路径**（目录递归） | SVN 状态扫描、revision 归一的 update 范围；扫描结果还要按操作目的过滤 |
| **W-file** | W-path **AND** `extensions` 命中的**文件**，永久排除任意深度的 `.svn/` 与 `.git/` 元数据 | git 的跟踪、基线 tree、stash 范围；SVN 交付文件和恢复时自动 revert 的范围 |

`W-file ⊊ W-path`，差集是**真实存在**的一类文件：§6.3 开头就说了 `Assets/Scripts/`
底下迟早会混进 `.psd`、`.zip`、`.bak`。这个差集有一个要命的性质：

> **它归 SVN 管，但 git 完全不管，客户端对它没有任何恢复手段。**

也就是说它和「白名单外的美术资产」是同一类东西（§7.2.1），只是**位置在白名单目录
里面**，所以极容易被误当成「git 保护得住的文件」。

实现上的硬规则：

- **范围由操作目的决定，不由命令叫 git 还是 svn 决定。** SVN 接受显式文件 target，
  客户端可以先按扩展名过滤，再传入文件列表。`status` 扫描 W-path；恢复的 `revert`
  必须从结果中取 W-file 且已有备份的具体文件，绝不把整个扫描结果交给它
- git 的跟踪与快照限定为 W-file；候选索引可以用 W-path 递归执行 `add`，但必须通过
  §6.3.1 的集合核对。`svn commit` 用交付文件清单，加必要新增父目录及 `--depth=empty`
  （§8.2.6）。`svn update` 的 W-path 归一会碰到差集，事前确认适用 §7.2.1、§8.1 #2
  及 §9.4；确认 update **不授权 revert** 这些文件
- 任何一句形如「这些文件 git 管得住 / 有基线兜底 / 冲突能在 git 里解决」的论证，
  **必须先确认说的是 W-file**。落在差集里的文件一条都不成立
- §12.1 那个统一过滤函数要**同时导出两个集合**，不要让调用点自己拼

### 6.4 基线分支

`refs/heads/svn/base`——普通分支，IDE 可见。

- 每次成功 `svn update` 后在其上落一个 commit，内容 = 白名单内文件在该 revision
  的状态
- commit message 携带权威记账，**沿用 `git svn` 的原生 trailer 格式**：

```text
svn: r12346

git-svn-id: https://svn.company.com/game@12346 8f3a1c7e-4b21-0410-9d2f-1a7b3c5d9e01
                            ↑ Repository Root，不是工作副本 URL，见 §6.4.1
```

#### 6.4.1 为什么用 `git-svn-id:` 而不是自定义的 `Svn-Rev:`

格式固定为一行：`git-svn-id: <url>@<rev> <repository-uuid>`，解析正则
`^git-svn-id: (\S+)@(\d+) ([0-9a-fA-F-]+)$`。三个字段缺一不可，解析失败即视为该
commit 不是 baseline（不猜、不部分接受）。

**`<url>` 取 `Repository Root`，不是某个工作副本的 URL。** 白名单可能分布在同一个仓库
的**多个**嵌套工作副本里（§6.6.5），各自的 URL 不同，没有哪一个能代表整体；而
`Repository Root` + `uuid` 对它们是共有的，`<rev>` 则是 §6.6.5 那个推广后的单值 `N`。
具体路径本来就记在配置的白名单里，不需要 trailer 再说一遍。

⚠️ 这是与 `git svn` 约定的一处**有意偏离**——`git svn` 写的是分支 URL。格式仍可被
同一个正则解析，`git log --grep=^git-svn-id` 照常可用，但不要据此认为本仓库的 trailer
能被 `git svn` 当成自己的映射使用（本来也不能，见 §15 第 19 条）。

换用这个格式的收益：

- **UUID 白送**。§6.2.1 要求比对 repository UUID（URL 可能因代理 / 镜像 / `http` 与
  `https` 写法不同而不可靠），原来的 `Svn-Url:` 记的是 URL，帮不上忙；`git-svn-id`
  自带 UUID，基线自己就带着仓库身份
- **工具互操作**。`git log --grep=^git-svn-id` 是 `git svn` 文档里给出的标准用法，
  IDE 与第三方工具也按这个格式识别 SVN 映射
- **迁移锚点**。将来若 §2.2 的结论被推翻、真要迁到 `git svn`，历史里已有现成的映射

格式不是照文档猜的，是拿真实 `git svn` 的产物核对过的（2026-09-13，`svn://` 真实远端）：

```text
git-svn-id: svn://svn.code.sf.net/p/tortoisesvn/code/trunk/contrib@29785 cc8e0003-5e82-43e2-9771-63201f7d8d9c
                                                                         ↑ 与 svn info 的 Repository UUID 逐字符一致
git svn find-rev r29785  →  a5a1476e3af2ae852bd2a62fcca3084a308ea31d     ← 互操作确认
```

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
P1  白名单内 svn status **只拦 `~`（obstructed）**
      M / A / D / R / C / ! / ? 一律放行——录制走 §16 的 R2a–R2c，脏文件改取
      服务器 rN 的内容，不需要工作副本干净。理由与实测见 §16.1
      `~` 单独拦是因为 svn 自己都说不清那个位置上是什么，取不到 BASE
      ⚠️ 必须带 --no-ignore：svn status 默认隐藏 SVN-ignored 文件，而那些
         文件 git 照收，不带这个参数就会漏检。这份清单同时就是 R2a 的输入
      ⚠️ 同时必须带 --ignore-externals，理由见 §6.6.4
      ⚠️ 仍然绝不自动规范化——revert 用户的本地修改是破坏性的，对齐 §8.1 #2。
         现在连拦都不拦了，但那是因为换了录制来源，不是因为松了对 revert 的态度
P2  nodeRevs 必须是单元素集合 {N}
      读法按 §6.6.2：--ignore-externals，且取 <wc-status revision>
      多值说明工作副本是 mixed-revision，引导用户先跑一次完整 svn update
      ⚠️ 读法错了这一条会**永远过不去**，且「先跑一次 svn update」是一条
         **永远无效**的引导（external 的 revision 来自别的仓库），见 §6.6.4
P3  不在 svn update / merge 中途（.svn 里没有未完成的操作）
P4  白名单**可以**落在 external 内，但白名单**递归覆盖到的每一个工作副本**
    都必须属于**同一个仓库**
      ⚠️ 只查白名单根是不够的，见 §6.5.5。必须先枚举白名单子树内的**全部**嵌套
         工作副本，再对每一个取 (Working Copy Root Path, URL, Repository UUID)
      所有 UUID 必须相同；不同则拒绝，并指出是哪两条分属哪两个仓库
      理由：svn commit 跨仓库会被**直接拒绝**（E200007，§6.6.5 实测），交付永远
      做不成。前置拦住是为了不让用户白走完整个确认流程才在 D5 撞墙
P5  检测白名单所在的 external 有没有被 `-r` 钉死
      检测方式见 §6.5.2。命中**不阻断**，改为进入「pinned 模式」，见 §6.5.3
      （初版写的是硬阻断 + 要求团队改 `svn:externals`。那条路走不通——改动别人的
        externals 定义不是一个客户端能要求的事——而且实测表明客户端自己就能绕开，
        见 §6.5.3）
```

P1–P5 全部走完之后才建库（P1–P4 是硬门槛，P5 命中不阻断、只切换到 pinned 模式）：

```text
git init -b svn/base                 ← 显式指定，不吃 init.defaultBranch 的默认值
写入 .git/mygitclient.json（用户编辑白名单）
生成 .git/info/exclude               ← 含末尾的 .svn/ 兜底行，见 §6.3
按 §16 R2a–R2c 录出服务器 rN 的树    ← 脏文件取 BASE 内容、A/? 剔除；含集合核对（§6.3.1）
用该 TREE 录制首个基线并对齐真实索引 ← git-svn-id 的 rev 取自 P2 读到的 N；无父提交
基线体检（§6.8.7）                    ← 判据 D \ S == ∅；不通过则整个回退，删掉刚建的 .git
git checkout -b work                 ← 立刻离开基线分支
```

⚠️ 「三条全过」是初版留下的说法，那时只有 P1–P3。现在是五条，**P4 与 P1–P3 一样
是阻断级门槛**，不要照字面只跑前三条；P5 是分流不是门槛。

#### 6.5.1 为什么 P1–P3 不能省

`svn/base` 的语义是「白名单内的文件在 revision N 时**服务器上的样子**」。初始化直接
`git add` 当前磁盘内容，等于宣称「磁盘 == 服务器」，而这个等式只有在工作副本干净且
单一 revision 时才成立。

下面这张表原本是「P1 为什么必须拦住每一种脏」的论证。§16.1 换掉录制来源之后，**后果
那一列仍然成立，但拦法变了**：不再靠拒绝用户，而是靠在录制时把每一种脏还原成服务器
rN 的样子。P1 因此收窄到只拦 `~`，而不是取消。

| 初始状态 | 照磁盘录会写进基线的东西 | 后果（不处理的话） | §16 R2a 的处理 |
|---|---|---|---|
| `M` 本地修改 | 改过的内容被当成服务器原文 | 这些改动**永远不会**出现在交付的净变更里（基线和工作分支都有它），静默漏交付 | 取回 BASE |
| `?` 未版本控制 | git 收了，SVN 不知道 | 第一次交付时 D1 算不出它是新增 | 从树里剔除 |
| `A` / `D` 待提交登记 | 登记状态没进 git | §8.5 的差量基准从一开始就是脏的（§8.5.1） | `A` 剔除；`D` 取回 BASE |
| `R` / `C` / `!` | 同 `M`，磁盘上不是服务器内容（`!` 是干脆没有） | 同上 | 取回 BASE |
| `~` obstructed | svn 说不清那里是什么 | 无法判定 | **仍然阻断**——取不到 BASE |
| mixed-revision | —— | 见下 | P2 仍然硬拦，且它是 `-r BASE` 的授权条件（§16.2） |

处理完之后有一个顺带的好处值得记下来：初始化结束切到 `work` 分支时，`git status` 显示的
**恰好就是用户原有的那些本地 SVN 修改**——`M` 显示为已修改、`?` 显示为未跟踪、`D` / `!`
显示为已删除。旧做法下这里永远是空的（因为强制要求干净），现在它第一眼就是对的。

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

#### 6.5.2 怎么检测一个 external 有没有被 `-r` 钉死

P5 只有在「能可靠检测」的前提下才算一条规则，否则就是一句空话。两条独立手段，
都要实现。

**手段一：解析 `svn:externals` 属性文本（离线、快，负责阻断）**

`svn propget svn:externals -R --xml` 拿原文逐行解析。实测（2026-09-13，临时 SVN
fixture）——**四种写法都能钉住 revision**，解析器必须全认：

| 写法 | 是否钉住 |
|---|---|
| `^/lib lib` | ❌ 跟随 HEAD |
| `-r2 ^/lib lib` | ✅ 钉在 r2 |
| `-r 2 ^/lib lib`（`-r` 与数字之间有空格） | ✅ 钉在 r2 |
| `^/lib@2 lib`（peg 语法，不用 `-r`） | ✅ 钉在 r2 |
| `lib -r2 ^/lib`（旧格式，本地路径在**前**） | ✅ 钉在 r2 |

⚠️ **两种格式的字段顺序是相反的**——新格式是 `[-r N] URL LOCAL`，旧格式是
`LOCAL [-r N] URL`。所以**不能按列取字段**，判据只能是「这一行里出现了 `-r <N>`
记号，或 URL 带 `@<N>` 后缀」。漏掉 `@<N>` 那一种是最容易犯的错，因为它不含 `-r`。

实测顺带发现：旧格式**不带** `-r` 时（`lib ^/lib`）签出直接失败——旧格式与 `^/`
相对 URL 不兼容。这不影响检测（那种定义本来就没法用），但解析器不要因此 panic。

**手段二：行为校验（在线、权威，负责发现手段一的盲区）**

`svn update` 成功之后，**未被钉住的 external 必然停在该仓库当时的 HEAD**。于是：

```text
sync_in 的 S5 之后：
  比对 svn info <白名单所属 WC> 的 Revision 与该仓库当时的 HEAD
  不等 → 该 external 疑似被钉住
```

它不依赖任何文本解析。代价是有一个窄竞态（查 HEAD 与 fetch external 之间有人提交），
所以它的结论是**「疑似，需复核」，不单独用来阻断**——阻断归手段一，手段二只负责在
手段一漏判时把问题暴露出来。

#### 6.5.3 白名单所在的 external 被钉住时怎么办（pinned 模式）

初版 P5 的处理是「硬阻断 + 请团队改 `svn:externals` 定义」。**那条路走不通**——改别人
的 externals 定义不是一个客户端能要求的事，而且实测表明**根本不需要**。

**实测**（2026-09-13，临时 SVN fixture，`externals = -r2 ^/code-trunk code`，
服务器上该路径 HEAD 内容是 `v3`）：

| # | 操作 | 结果 |
|---|---|---|
| A | 从父目录 `svn update code`（**显式指名** external 目录） | → r4 / `v3`，**不认 peg**，走到该仓库 HEAD ✅ |
| B | `svn update --ignore-externals <根>` + `svn update art` | → 被钉住的 `code` 纹丝不动，`art` 正常更新 ✅ 可选择性更新 |
| C | 在 `code` 里提交后，在项目根跑**递归** `svn update` | → `code` 退回 r2 / `v2`，`svn status` **仍报干净** ❌ 无声回退 |
| D | 有**未提交**改动时跑 C | → 产生冲突标记 `.mine` / `.r4` / `.r2`，**不是静默丢失** |

结论很直接：**peg 只在「父级递归处理 externals 定义」时生效。显式指名那个目录去
update，就绕过去了。**

**于是 pinned 模式只改三件事：**

1. **update 拆成两步**（§7.3 S5）：项目根用 `--ignore-externals`，然后对每个
   external **显式**跑一次 `svn update <目录>`。对用户可见的行为不变（美术照常
   更新），变的只是执行方式。⚠️ 这是 pinned 模式**专属**的形式，不是默认，见下
2. **新增一条失同步信号**（§6.6.2）：`nodeRevs` **掉回 peg revision**，说明有人在
   项目根跑了递归 update。此时提供一键修复——就是 `svn update <白名单所属目录>`，
   把它重新拉到 HEAD
3. **UI 常驻说明**这条残余风险，见下

**残余风险，以及为什么可以接受**

无法阻止外部工具（TortoiseSVN 的「SVN Update」、命令行 `svn up`）在项目根跑递归
update，那会把代码回退到 peg，而且工作副本看起来干净（测点 C）。

但三条理由让它可接受：

- **这个风险今天就存在，不是本设计引入的。** 若团队把承载代码的 external 钉死了，
  任何人提交代码后再 update 都会看到自己的提交从工作副本消失
- **双通道可检测。** SVN 侧 `nodeRevs` 掉回 peg（§6.6.2）；git 侧磁盘内容与
  `svn/base` 不符，`git status` 直接显示出来。两条互相独立
- **一条命令可修复**，且内容从未真正丢失——它在服务器上，也在 git 对象库里

#### 6.5.4 为什么两步形式不做默认

初版把 §6.5.3 第 1 条的两步 update 定成了**无条件**的做法。这是错的，理由不是复杂度，
而是它**引入了一个原本不存在的静默失败面**：

> 两步形式要求客户端**自己枚举所有 external**。一旦枚举漏掉一条（嵌套 external、
> 新增的 external、解析 `svn:externals` 时没覆盖到的写法），那条 external 就会
> **永远静默地不再更新**——没有报错，只是美术资产停在某个旧 revision，几周后才有人
> 发现「我的 Unity 工程和别人的不一样」。

而项目根的递归 `svn update` **在结构上不可能有这个 bug**：externals 的枚举是 SVN
自己做的，客户端一行都不参与。

所以分工是：

| 场景 | S5 用哪种 |
|---|---|
| 默认（白名单所属 external 未被钉住） | **项目根递归 `svn update`**，简单且不可能漏 |
| 检测到白名单所属 external 被 `-r` 钉住 | §6.5.3 的两步形式 |

⚠️ **D6 不受这个分歧影响。** 它本来就是 `svn update -- <白名单路径>`，target 显式指
到白名单（可能在 external 内），不依赖任何枚举，所以两种情形下写法相同。

顺带一提：**一条承载活跃开发代码的 external 实际上很难是被钉住的**，因为那会让全组
每个人的代码提交在下一次 update 后从工作副本消失，团队第一天就会发现。所以 pinned
模式大概率只会作用在美术 / 文档那类 external 上，而本设计从不往那些地方写。

**2026-09-13 用户确认：公司项目的 `code` / `script` 等目录确实没有被钉住**（理由正是
上面这条——开发时要不停改代码）。所以 pinned 模式对该项目**不会触发**，它在本设计里的
定位是**防御性路径**：检测保留、诊断保留（§6.6.2 那条「`N` 恰等于 peg」），但正常流程
一步都不走它。

#### 6.5.5 P4 必须枚举白名单子树内的**全部**工作副本，不能只查白名单根

P4 初版写的是「对每个白名单根跑 `svn info`」。**白名单根底下还可以再挂 external**，
那时只查根会得到根所属仓库的 UUID，P4 干净通过，而一个**另一个仓库**的子树已经进了
白名单。

2026-09-13 临时 SVN fixture 实测（`repo1` 的 `Code/` 底下挂一条指向 `repo2` 的
external `Vendor`）：

```text
svn info --show-item repos-uuid Code          → 0f098dbb-…      ← repo1，P4 通过 ✅
svn info --show-item repos-uuid Code/Vendor   → 35647ab0-…      ← repo2！
svn status -v --ignore-externals -- Code
     9   9  Code           9  3  Code\A.cs      9  1  Code\B.cs
     X       Code\Vendor                        ← 内容整个不在 nodeRevs 里
磁盘实际有:  Code/A.cs  Code/B.cs  Code/Deep/Z.cs  Code/Vendor/V.cs
```

三个后果叠在一起：

| 层面 | 发生了什么 |
|---|---|
| git | `Code/Vendor/V.cs` 命中白名单 + 扩展名，**被 git 跟踪**，进基线、进交付清单 |
| 失同步检测 | §6.6.5 规定在 `wc(P)` 内部带 `--ignore-externals`，于是 `Vendor` 的节点**从不参与 `nodeRevs` 校验**——别人改了它，客户端一无所知 |
| 交付 | D5 的 target 列表会同时含两个仓库的路径，`svn commit` 以 **E200007** 拒绝（§6.6.5 E5）——正是 P4 存在的意义，但 P4 没拦住 |

**所以 P4 的枚举对象是「白名单递归覆盖到的全部工作副本」**：

```text
对每个白名单根 P：
  svn status --xml -- P     （**不带** --ignore-externals）
  取 item="external" 的条目 → 这些就是 P 底下的嵌套 external 根
  对每一个再跑 svn info，取 (Working Copy Root Path, URL, Repository UUID)
  递归，直到不再出现新的 external 根
所有 UUID 必须相同；不同则拒绝，并指出是哪条嵌套 external 属于哪个仓库
```

枚举方式沿用 §6.6.4 那条结论（`item="external"` 比解析 `svn:externals` 属性文本可靠）。

**同仓库的嵌套 external 不阻断**，但它必须被纳入 §6.6.5 的记账单位集合——也就是
`nodeRevs` 要在它上面也跑一次并取并集，否则它就是一块 git 管着、SVN 侧却没人盯的
盲区。这条与「在 `wc(P)` 内部带 `--ignore-externals`」不冲突：`--ignore-externals`
负责不让**别的 WC** 的 revision 混进当前 WC 的集合，而每个 WC 自己都要被跑到。

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
nodeRevs    = svn status -v --xml --ignore-externals       ← 白名单内每个已版本控制
                  -- <白名单路径>                             节点的 revision 集合。
                                                             本地读，范围限于代码。
                                                             取 <wc-status revision>，
                                                             **不是** <commit revision>；
                                                             `--ignore-externals` 也不能
                                                             省。两条都见 §6.6.4
remoteRev   = svn info --xml <URL> 的 revision            ← 走网络，异步，失败不阻塞
```

**正常状态的不变量**：`nodeRevs` 是单元素集合，且等于 `{baselineRev}`。

⚠️ **`nodeRevs` 的粒度是「白名单路径」，不是「白名单路径 AND 扩展名」。** `svn status`
的 target 只能是路径，SVN 不认识扩展名过滤，所以 `Assets/Scripts/` 底下那个被 §6.3
排除掉的 `.psd` **也在 `nodeRevs` 里**。全文出现「白名单」三个字的地方必须分清是哪一种
（§6.3.2）——混用已经造成过 §8.2.1 的错误论证。

失同步的判据**只有这两条**：

| 观察 | 含义 |
|---|---|
| `nodeRevs` 有多个值 | mixed revision，外部提交或局部 update 过 |
| `nodeRevs = {N}` 但 `N != baselineRev` | 整体被外部推进或回退 |

第二条在 pinned 模式下有一个**具体且常见**的成因，UI 要能分辨并给出对应的修复：
`N` 恰好等于该 external 的 peg revision ⇒ **有人在项目根跑了递归 `svn update`**，
把白名单所在的 external 拽回了 peg（§6.5.3 测点 C）。此时不要引导用户进 §9.4 的
完整恢复流程，一条 `svn update <白名单所属目录>` 就能拉回 HEAD（测点 A）。

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

`remoteRev` 走网络，按现有 `refreshPullRequests` 的模式处理：不 await、失败静默
（读不到就不显示落后数，不弹错）。`remoteRev > rootRev` 时显示「落后 N 个 revision」。

#### 6.6.4 两个会让 `nodeRevs` 整个失效的读法

`nodeRevs` 是本节唯一的失同步判据，它一旦算错，§6.6 / §9 / §6.5 全线失效。
2026-09-13 在**真实远端工作副本**（TortoiseSVN 源码树，`svn://svn.code.sf.net/p/tortoisesvn/code/trunk`
@ r29825，16945 个文件，只读）上实测出两个读法陷阱，两个都会让判据永久误报。

**陷阱一：externals 会把另一个仓库的 revision 混进来**

```text
svn status -v --xml .                    → entry 14689 个
  wc-status@revision 取值 = {29825, 1938156}   ← 2 个值！
svn status -v --xml --ignore-externals . → entry 10940 个
  wc-status@revision 取值 = {29825}            ← 1 个值 ✅
```

`1938156` 来自 `svn:externals` 指向的 `https://svn.apache.org/repos/asf/...`——**另一个
仓库**，revision 号在百万量级。该工作副本有 6 个 external（`ext/Subversion`、`ext/apr`、
`ext/apr-util`、`ext/serf`、`ext/TortoiseOverlays`、`ext/CrashServer/external/zlib`）。

不加 `--ignore-externals` 的后果，逐条对照：

| 位置 | 后果 |
|---|---|
| §6.6.2 的判据「`nodeRevs` 有多个值 ⇒ mixed revision」 | **永久误报失同步**，红色横幅永远挂着 |
| §6.5 P2「`nodeRevs` 必须是单元素集合」 | 初始化向导**永远过不去**，而它给的引导是「先跑一次完整 `svn update`」——**这条引导永远无效**，因为 external 的 revision 来自别的仓库，update 多少次都不会相等 |
| §6.6.3 第二档的完整校验 | 每次都判失败 |
| §9.3 / §9.4 | 恢复向导被无意义地反复触发 |

**同仓库 external 更隐蔽。** 上面那 6 个里有一个是 `^/trunk/ext/zlib zlib`（相对 URL，
指向本仓库），它的 revision 就在 29xxx 量级，靠「数值看起来不对」根本发现不了；若它
带了 `-r` peg 固定在某个旧 revision，就会产生一个**稳定的、看起来很合理的**多值
`nodeRevs`。所以判据不能靠数值启发式，只能靠 `--ignore-externals` 从源头排除。

**§1.2 把 externals 列为非目标，但「不支持」不等于「可以无视」。** 它们物理存在于真实
工作副本里，会主动污染状态判定。必须**主动屏蔽**：

- 所有 `svn status` / `svn update` 调用带 `--ignore-externals`（§4.4）。
  ⚠️ **`svn info` 不在此列**——它拒绝这个参数（实测 exit 1），而且默认本来就不遍历
  externals，见 §4.4 那张子命令表
- §6.2.1 的白名单校验**拒绝落在 external 内（或本身就是 external 根）的路径**。
  枚举方式：`svn status --xml`（**不带** `--ignore-externals`）里 `item="external"`
  的条目就是 external 根，实测能干净列出那 6 个；这比解析 `svn:externals` 属性文本
  可靠得多——后者要处理 `-r` peg、`^/` 相对 URL、引号和多行格式

**陷阱二：一个 entry 里有两个 revision，取错就全盘误报**

`svn status -v --xml` 的每个 entry 长这样：

```xml
<entry path=".clang-format">
  <wc-status item="normal" revision="29825" props="none">   ← 节点基线 revision ✅ 要它
    <commit revision="29293">                               ← 最后修改 revision ❌ 不是它
```

同一实测里两者的分布差了三个数量级：

| 属性 | 含义 | 不同取值个数 | 用它做 `nodeRevs` 会怎样 |
|---|---|---|---|
| `<wc-status revision>` | 该节点在工作副本里处于哪个 revision | **1**（`{29825}`） | 正确，不变量成立 |
| `<commit revision>` | 该节点最后被改动是哪个 revision | **2194**（1640–1938129） | 每次都多值 → 永久误报 |

**陷阱三（连带）：XML 属性是逐行排列的，行内正则会静默扫空**

```text
<wc-status⏎   item="normal"⏎   revision="29825"⏎   props="normal">
```

`<wc-status` 与 `revision="…"` **不在同一行**（且是 CRLF）。`grep -o '<wc-status[^>]*revision="[0-9]*"'`
这类写法**不会报错，只会返回空集**——于是 `nodeRevs` 变成空集合，「单值不变量」以一种
看起来通过了的方式被绕过。所以 §4.4 规定 `--xml` 输出必须走真正的 XML parser。

#### 6.6.5 白名单落在 external 里时，记账单位是哪个工作副本

§6.6.4 得出的对策是「统一加 `--ignore-externals`」，那条**只对「白名单不在 external
里」的情形成立**。真实 Unity 工程常见的拼法恰恰相反：项目根之下 `Assets/`、`Code/`、
`Documents/` 各是一条 external，**代码根本不在项目根那个工作副本里**。此时
`--ignore-externals` 会把白名单整个排除掉，比污染更糟。

**实测**（2026-09-13，临时 SVN fixture：项目根 + `code` 同仓库 external + `asset`
跨仓库 external；另在真实工作副本上只读复核）：

| # | 问题 | 结果 |
|---|---|---|
| E1 | external 是什么 | **完全独立的工作副本**：自己的 `.svn`、`Working Copy Root Path`、URL、`Repository UUID`、Revision |
| E2 | 项目根 `svn commit`（无 target） | **不下沉进 external**，里面的改动一个都不提交 |
| E3 | 从项目根**显式指定** external 内的路径提交 | ✅ 成功 |
| E4 | 跨「根 WC + **同仓库** external」多 target 提交 | ✅ **单个 revision，原子** |
| E5 | 跨**不同仓库**多 target 提交 | ❌ `E200007: Commit can only commit to a single repository at a time` —— **前置拒绝** |
| E6 | 项目根 `svn update` | ✅ 连带更新 externals |
| E7 | 往被 `-r` 钉住的 external 提交后再 `svn update` | ⚠️ 见下 |
| E8 | `svn status --xml` 的结构 | 扁平的**单个** `<target>`；external 根标 `item="external"`，其**内容**条目混在同一层，从条目本身看不出它属于哪个工作副本 |

真实副本佐证：TortoiseSVN 源码树里 `ext/apr` 的 UUID 是 `13f79535-…`，而项目根是
`cc8e0003-…`——同一个磁盘目录树下并存着两个仓库。

**E5 其实是好消息。** §8.5 全部的失败处理都建立在「SVN 提交在服务端原子」之上，
而 SVN 对跨仓库提交是**动手之前就拒绝**，不会出现「提交了一半」。所以那条论证**不需要
重写**，只需补一条前置约束：**白名单必须全部属于同一个仓库**（§6.5 P4）。

**E7 是这一节最危险的一条**，实测全过程：

```text
externals 定义 = "-r5 ^/code-trunk code"
  提交我的改动        → Committed revision 8.   磁盘 = 我的新改动
  再跑 svn update     → 磁盘变回旧内容
                         svn info code 的 Revision 仍是 5
                         svn status code 显示 **干净**
```

提交**确实上了服务器**，但下一次 update 把它从磁盘无声抹掉，而且工作副本报干净。
放进本设计：若 D6 使用这种更新方式，D7a / D8 的录制会把**旧内容**录成标着
**新 revision** 的基线——自己造出 §9.2 的基线撒谎，且 §6.6.2 的所有间接判据都显示正常。
没有缓解手段，只能按 §6.5 P5 硬阻断。

**记账规则**

对每个白名单根 `P`，先用 `svn info P` 取三元组：

```text
wc(P)   = Working Copy Root Path     ← 它真正属于哪个工作副本
url(P)  = URL
uuid(P) = Repository UUID
```

然后：

| 原来读哪里 | 改成读哪里 |
|---|---|
| §6.2.1 的 `svnUrl` / UUID 校验：项目根的 `svn info` | **每个 `wc(P)`**；所有 `uuid(P)` 必须相同（§6.5 P4） |
| §6.6.2 的 `nodeRevs`：项目根跑一次 | **在每个 `wc(P)` 上各跑一次**，带 `--ignore-externals`（挡住更深层的嵌套 external），取并集 |
| §6.6.3 的 `rootRev`（O(1) 提示档） | 每个 `wc(P)` 的根 revision，任一变化即触发全量校验 |
| §6.8 体检的 `--old=<svnUrl>@<rev>` | 每个 `wc(P)` 各体检一次，用它自己的 `url(P)` |

⚠️ 这里的「每个 `wc(P)`」指**白名单递归覆盖到的全部工作副本**，包括白名单根**底下**
再挂的嵌套 external，不只是白名单根自己所在的那个 WC。枚举方式与判据见 §6.5.5——漏掉
嵌套 WC 会造出一块「git 管着、SVN 侧没人盯」的盲区。

**不变量相应地推广**：所有白名单所属工作副本的 `nodeRevs` **并集**必须是单元素集合
`{N}`。E4 + E6 保证这是可达的——同仓库的多个 WC 在一次提交后拿到同一个 revision，
update 之后一起收敛到 `N`。

⚠️ `--ignore-externals` 的用法要分清楚：**在 `wc(P)` 内部**用它（挡住更深的嵌套
external）是对的；**从项目根**用它去覆盖白名单是错的（会把白名单整个跳过）。
§6.6.4 那条「统一加」要按这一节理解。

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
| §7 `sync_in` | 通过 §7.3.2 的线性区间检查后，S9 将 Local-Only commit 和普通 commit 一起 rebase 到新基线上；不按 trailer 改变同步语义 |
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

文件级清单破坏基线正确性的具体路径：§8.2.2 的候选基线收的是**磁盘内容**。若某文件
被排除在 `svn commit` 之外却带着补丁躺在磁盘上，这个补丁会被写进
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

⚠️ **第二类在 §16.1 之后不再无条件成立。** R2b 会把本地脏的那几条换成服务器内容，此时
基线树与磁盘**故意**不同。那种时刻改用 §6.8.7 的判据，它是本节这条的严格推广，不是替换。

前提成立时就可以直接拿磁盘去跟服务器比：

```bash
svn diff --old=<svnUrl>@<baselineRev> --new=<工作副本根> --summarize -- <白名单根...>
```

输出还原为仓库相对路径后，**只检查 W-file 的文件内容差异**（§6.8.4）。存在这种差异
即「基线与服务器不符」，直接证伪，不需要关于 revision 号的推断。

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

在**真实远端服务器**上复测（2026-09-13，TortoiseSVN 源码树 @ r29825，只读，
`svn://svn.code.sf.net/…/trunk/contrib`，521 KB 子树，公网）：

| # | 场景 | 结果 |
|---|---|---|
| R1 | `baselineRev = 29825` 对干净工作副本 | 无输出，**3.1s** ✅ |
| R2 | 拿旧的 `r29000` 比 | 报出 29 行差异，**3.3s** ✅ 抓住假基线 |
| R3 | 对着**含 6 个 external 的目录**（`ext/`）体检 | 无输出，3.5s ✅ **`svn diff` 不递归进 externals** |

R3 值得单独记：`svn diff` **不接受** `--ignore-externals`（`Subcommand 'diff' doesn't
accept option '--ignore-externals'`），一开始以为是缺口，实测发现它**根本不需要**——
URL 与工作副本目录比对时不会走进 externals。

⚠️ 当时由此得出的「所以这条只约束 `status` / `info` / `update`」**把 `info` 算错了**：
`svn info` 和 `diff` 一样会拒绝这个参数，同样也不需要它。正确的子命令表见 §4.4。

三条的耗时都在 3 秒级，与 §6.8.5 末尾「成本是 MB 级而非 GB 级」的判断一致；公司服务器
在内网，只会更快。

#### 6.8.3 这条检查证明不了什么

它比的是**磁盘**与服务器，不是**基线树**与服务器。两者之间隔着一个假设：「git 跟踪的
文件集合 == 白名单内应跟踪的集合」，而这个假设正是 §6.3 那条 exclude 规则可能打破的
（漏掉 `!/Assets/Scripts/**/` 时，深层 `.cs` 在磁盘上好好的，只是 git 不管）。

所以分工是明确的，不要互相冒充：

| 缺陷类型 | 谁抓 |
|---|---|
| 基线 rev 与服务器内容不符（假基线的四个切面，§18） | **本节的体检** |
| git 跟踪的文件集合不对（exclude / `.gitignore` 等规则影响） | §6.3.1 在每次录制前核对候选索引；§14.2 第 6、60、70 项覆盖真实行为 |

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
   只看 W-file 文件**；目录、纯属性变更和扩展名排除文件一律剔除。否则恢复流程有意保留的
   `Code/Art.psd` 本地修改会让体检误报。与 §8.6.2「比对前必须先剔除目录路径」同源，那里
   不剔除会把真成功判成失败，这里不剔除会把正常仓库判成基线撒谎

#### 6.8.5 什么时候跑

只在**降级路径**和用户主动要求时跑，不进 `sync_in` / `sync_out` 热路径：

| 时机 | 理由 |
|---|---|
| 初始化向导，首个基线录制并发布之后、`git checkout -b work` 之前（§6.5） | 这是假基线的最大来源；此刻还没建工作分支，不通过就整个回退，代价最小 |
| §9.4 恢复流程第 ⑥ 步之后、第 ⑦ 步之前 | 恢复的全部目的就是修好基线，不验证等于没修 |
| §16 Phase 1「重建基线」R3 之后、R4 之前（R3b） | 同上；此时指针还没动，不通过就中止，代价为零 |
| 用户手动点「体检」 | 怀疑基线不对时的唯一直接手段 |

**不在 `sync_in` 的 S7 之后跑**：S5 的 `svn update` 刚成功，磁盘按构造就是服务器内容，
再问一次是冗余的网络开销。

成本是白名单子树的一次 diff——白名单里没有美术资产，量级是 MB 不是 GB。

#### 6.8.6 失败与「没跑成」必须分开

| 结果 | 处理 |
|---|---|
| 命令成功，按 §6.8.4 过滤后无 W-file 内容差异 | 基线可信，记下体检时间与 revision |
| 命令成功，过滤后仍有 W-file 内容差异 | **基线撒谎**。阻断并列出差异文件；初始化时整个回退，恢复流程时停在第 ⑥ 步报错 |
| 命令失败（断网、认证、URL 不可达） | **不是基线有问题**。不阻断，标记「基线未经体检」并在 UI 上如实显示，对齐 §6.6.2 `remoteRev` 的「不 await、失败静默」模式 |

把第三行单独列出来是必须的：把「没能验证」当成「验证不通过」会让离线场景寸步难行，
当成「验证通过」则等于这条检查不存在。

#### 6.8.7 工作副本不干净时的判据：`D \ S == ∅`

§16.1 之后，录制基线不再要求工作副本干净，于是体检也不能再要求「一条差异都没有」——
用户自己的修改本来就会差出来。判据推广为：

```text
D = svn diff --old=<svnUrl>@N --new=<工作副本根> --summarize -- <白名单根...>
    按 §6.8.4 过滤后剩下的 W-file 内容差异集合
S = 本次录制时实际动过的路径（R2a 分出的「取回 BASE」∪「从树里剔除」两类）

Verified  ⟺  D \ S == ∅
```

读作：**磁盘与服务器的每一处差异，都必须能被一条已知的本地修改解释。** 解释不了的差异
就是基线在撒谎。

工作副本干净时 `S = ∅`，判据退化成 `D == ∅`，与 §6.8.2 逐字等价——所以这是推广，
两套判据不会打架，实现上也只有一条代码路径。

`S` 的取法有硬要求，见 §16.1：**必须恰好是实际动过的那批路径，不能取超集**，否则判据
被悄悄削弱。

**实测（2026-09-16，临时 fixture，一次性探针跑完即删）：**

不误报——工作副本停在 rN，手上四种常见的脏：

| 集合 | 内容 |
|---|---|
| D | `{A.cs, Added.cs, B.cs}` |
| S | `{A.cs, Added.cs, B.cs, New.cs}` |
| `D \ S` | `{}` ✅ 本地改动没有被判成撒谎 |
| `S \ D` | `{New.cs}`（只在 S 里，不影响判定） |

有牙齿——基线声称 r2、磁盘其实停在 r1（`svn update` 漏做 / 基线 rev 记错的真实形态）：

| 观察点 | 结果 |
|---|---|
| `svn status` | `{}` —— 看不见，相对自己的 BASE 确实没改 |
| `svn diff --old=<url>@2` | `A.cs [item=modified]` —— 看得见 |
| `D \ S` | `{A.cs} ≠ ∅` ✅ **判据成立并报警** |

第二组是这条判据存在的意义：它抓的正是 `svn status` 这类「相对本地 BASE」的间接判据
**结构上看不见**的那一类缺陷。

⚠️ **一个既有盲区，B 没有引入也没有修掉。** 试图用「同长度篡改一个字节 + 还原 mtime」
去证明判据的牙齿**没有证成**：`svn status` 与 `svn diff` **双双**被 svn 的
「大小 + mtime 相同就跳过」捷径绕过，两条命令都看不见这次篡改。

这不是新判据的缺陷——**今天的 §6.8 体检跑的就是同一条 `svn diff`，同样看不见**。记在
这里是为了不把体检的能力说得比实际强：它防的是流程性错误（rev 记错、update 漏做、
外部提交），不防有人拿十六进制编辑器精确伪造一个同长度的文件再把时间戳改回去——那个
威胁模型在本项目的场景里也不现实。真要堵上得加 `svn cleanup --vacuum-pristines` 之类的
强制重扫或逐文件比哈希，成本与收益不成比例，**本期不做**。

## 7. 更新流程 sync_in

### 7.1 为什么不能直接 svn update

工作目录此刻的内容 = SVN base + 用户 git 分支上的全部改动，SVN 把这些统统当作
「本地修改」。直接 `svn update` 会有三个问题：

1. SVN 用三方合并处理 git 分支的最终状态，冲突标记是 `.mine` / `.rN` 文件
2. 没有 `--abort`，合并到一半的状态难以回退
3. **想放弃时 `svn revert` 会把文件恢复到 SVN base 而不是 git 分支的状态——
   未提交的改动会被直接吃掉**

设计目标因此是：**先移走 W-file 的本地改动，让代码内容冲突由 git rebase 处理。**
SVN 树冲突，以及未受 git 保护文件的 update 冲突，仍按 §13 单独处理。

### 7.2 前置检查

任一不过则中止，且不做任何修改：

1. 不在 rebase / merge 中途
2. 基线未失同步（走 §6.6.3 的**完整 `nodeRevs` 校验**，不是 O(1) 那档）
3. 当前不在 `svn/base` 上，且 **HEAD 必须附着在分支上**（detached HEAD 拒绝，见 §8.1.1）
4. `svn status` 里 W-file 外没有冲突态文件（`C`），包含 W-path 内被扩展名排除的文件
5. **本次 update 会触及的 W-file 外文件若有本地修改（`M` / `!`），列出来并要求用户确认**——见下
6. 待重放区间 `svn/base..HEAD` 没有 merge commit（§7.3.2），且通过当前磁盘的纳入集合预检（§6.3.1）
7. 拿到该工作副本的独占锁；锁内复核 HEAD / 基线 OID 与上述检查时一致，变化则停止

#### 7.2.1 为什么第 5 条是确认而不是阻断

W-file 外的文件（含白名单目录内的 `.psd`，以及其他美术资产、Unity 生成的 `.asset`）**git 完全不管**，客户端对它们
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
S4   svn status --xml 扫描后只校验 W-file 应为 0 修改        ← 不把已确认保留的差集修改当作失同步
S5   svn update --accept postpone                        ← 在项目根递归跑，
                                                              连带更新所有 external
     **仅当**检测到白名单所属 external 被 `-r` 钉住时，改走 §6.5.3 的两步形式。
     默认不走两步，理由见 §6.5.4
S6   svn info --xml + svn status -v --xml 读新 revision
S7   临时索引纳入集合核对 → 用通过核对的 TREE 录制新基线   ← §6.3.1；失败保留旧基线和 journal
     按 §7.6 写入录制 / 发布阶段；完成索引对齐后才进入 S8
S8   git checkout ORIG_BRANCH
S9   复核 §7.3.2 的待重放区间，再 git rebase --empty=drop svn/base
                                                         ← 普通内容冲突在这里处理，无 merge-tree 前置
S10  git stash apply <OID>（若 S1 有 stash），成功后
     git stash drop 对应条目 + 删掉 refs/mygitclient/sync/<opid>
S11  删除 journal；恢复 watcher + 主动发 repo-changed
```

#### 7.3.0 行尾归一化：混合仓库一律关掉 git 的 EOL 转换（2026-09-21 实测定案）

2026-09-17 发现，2026-09-21 在临时 fixture 上量完并定案。**这是 Phase 2 的前置**：
`sync_in` 的 S1 / S3 / S10 每一步都会由 git 往磁盘写文件。

##### 问题

本机 `core.autocrlf=true`（多数 Windows 开发机的默认状态）。git 在 checkout /
stash 还原时会把 LF 转成 CRLF，而 SVN 的 pristine 里存的可能是 LF。实测一个
**没设 `svn:eol-style`** 的 LF 文件走完一次 stash 往返：

| 时点 | 磁盘 | `svn diff` |
|---|---|---|
| 改出 WIP，未 stash | LF | 1 行——就是那一行真改动 |
| S1 `git stash push` 之后 | **CRLF** | 文件变脏（内容没动，只有行尾） |
| S10 `git stash apply` 之后 | **CRLF** | **整文件重写** + 那 1 行真改动 |

最坏的下游后果在 Phase 3：交付清单按「工作分支 vs 基线」算，一整批只有行尾变化的
文件会被算成真改动**提交到 SVN**。

**顺带修正 2026-09-17 那条记录的一处夸大。** 受影响的不是「白名单内每个 W-file」，
而是**被 git 写过磁盘的那些**：stash push / apply 只碰进出 stash 的文件，S3
`git checkout svn/base` 只碰两棵树有差异的文件。范围小于原记录，性质一样——
只要 git 写过，那个文件对 svn 就变脏了。

设了 `svn:eol-style=native` 的文件不受影响（svn 在 Windows 上自己就放 CRLF，git
转出来的也是 CRLF，两边一致）。风险集中在**没设 eol-style 且 pristine 是 LF** 的
文件上，而 SVN 默认不设——要靠 `auto-props` 或手工 `propset`，所以真实工程里两种都有。

##### 定案：`.git/info/attributes` 写 `* -text`

三条出路的实测对比（同一份 fixture，只换这一项）：

| 出路 | S1/S3/S10 之后 `svn status` | 进工作目录 | 会被提交进 SVN | 挡得住工作树里的 `.gitattributes` |
|---|---|---|---|---|
| 维持现状（`autocrlf=true`） | **脏** | — | — | — |
| **B** `.git/config` 设 `core.autocrlf=false` | 干净 | 否 | 否 | ❌ **挡不住** |
| **C** `.git/info/attributes` 写 `* -text` | 干净 | 否 | 否 | ✅ 挡得住 |
| 逐文件读 `svn:eol-style` | 同 C，代价高得多 | 否 | 否 | 取决于实现 |

**取 C。** 三条理由：

1. **优先级最高。** gitattributes(5) 规定 `$GIT_DIR/info/attributes` 压过工作树里的
   `.gitattributes`。实测：工作树里放一个 `* text=auto`（比如别人真把它提交进了
   SVN），C 下 `git check-attr text` 仍然是 `unset`，**B 下变成 `auto`**——
   `core.autocrlf=false` 被覆盖，归一化又回来了。B 是一道能被用户的仓库内容掀翻的闸。
2. **不进工作目录，所以不会被提交进 SVN。** 与 §3「工作目录里只多出 `.git` 一个条目」
   一致。「在工作树里写 `.gitattributes`」这条出路正是因此被排除的。
3. **与既有做法同形。** `.git/info/exclude` 已经由 `svn_exclude.rs` 生成（含
   `exclude.bak` 备份约定），`.git/info/attributes` 是它的邻居，走同一套写法和同一个
   修复时机。

逐文件读 `svn:eol-style` 不取。我们要的是**字节级相等**：磁盘上就是 svn 放下的那些
字节。这一条 `* -text` 免费拿到；逐文件判别最好也只能追平它，换不来任何额外正确性，
却要为每个文件读一次属性，还得跟着属性的变化重算。

##### 对 R2b 没有副作用（实测）

`svn_baseline.rs` 的 `hash-object` 之所以带 `--path`，是为了让「取回 BASE」与
`git add` 落在同一套过滤器上（§16 R2b）。关掉转换之后这条对齐**仍然成立**，而且
变成平凡成立：

| arm | 文件 | `git add` 的 blob | `svn cat` 走 `hash-object --path` 的 blob | |
|---|---|---|---|---|
| A | `svn:eol-style=native` | LF | LF | 对得上 |
| A | 无 eol-style（磁盘 CRLF） | LF | LF | 对得上 |
| C | `svn:eol-style=native` | CRLF | CRLF | 对得上 |
| C | 无 eol-style（磁盘 CRLF） | CRLF | CRLF | 对得上 |

与 §18（十一）B4 那条实测一致，顺带把它的含义讲透：**`svn cat` 吐的不是 pristine 的原始字节，是翻译后的字节。**
`svn:eol-style=native` 的文件 pristine 里存的是 LF，而 `svn cat -r BASE` 在 Windows
上给的是 **CRLF**。所以 C 下两条录入路径仍然同形，不存在「干净文件录 CRLF、脏文件
录 LF」的撕裂。

##### 迁移：切换的那一刻必须强制重建基线

**这是本条决策唯一有代价的地方，不能省。** 基线 blob 的形状由**录制时**的转换设置
决定：`autocrlf=true` 录的基线全是 LF，关掉转换之后工作树读回来的是 CRLF。实测
（5 个 CRLF 文件 + 2 个 LF 文件）：

| 场景 | `git status` 脏文件数 |
|---|---|
| A 下录的基线，维持 A | 0 |
| A 下录的基线，翻到 B | **5**（全部 CRLF 文件） |
| A 下录的基线，翻到 C | **5** |
| **C 下重录的基线，C** | **0**（`svn status` 也是 0） |

⚠️ **这个故障是潜伏的，比立刻炸更糟。** 翻完设置立刻跑 `git status` 会报 **0**
——git 的 stat cache 认为文件没动过，压根不去重算 hash。要等到某个文件的 mtime
被碰一下（编辑一次、`svn update` 一次），它才单独冒出来。用户看到的因此是「文件
一个一个地无故变脏」，而不是一次性的批量变更。**任何「切完看一眼 `git status`
没事就放行」的验收都是假的。**

所以切换必须与**重建基线**绑死，不能只翻配置。R1–R5 对工作目录是只读的（§16），
代价只是一次 `svn cat` 往返，正是为这种场合准备的手段。落到实现上：

- `init_hybrid` 建库时就写好 `.git/info/attributes`，此后录的基线天然是对的；
- 打开一个**已存在**的混合仓库时，若 `.git/info/attributes` 里缺那条规则，说明这个
  基线是旧设置下录的：补写规则 **+ 要求重建基线**，不能只补规则就放行；
- 判据不能用 `git status` 干净与否（stat cache 会骗人）。要么看规则在不在，要么在
  判定前先 `git update-index --really-refresh`。

Phase 2 的 plan 必须把这条迁移列成一个独立 Task，并在 E2E 清单里覆盖「旧基线打开
→ 被要求重建 → 重建后 `svn status` 干净」这条路径。

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

#### 7.3.2 重放范围内的 merge commit 必须在修改前阻断

普通 `git rebase` 会跳过 merge commit；只存在于合并结果中的额外修改可能无冲突地
消失。上一版试图在 rebase 后与 `git merge-tree --write-tree` 的结果整树比对，
但第五轮 fixture 复核推翻了这个判据：

| 输入 | 实测结果 | 对设计的含义 |
|---|---|---|
| 纯线性分支把 `A.cs` 从 `0→1→0`，新基线把它改成 `1` | merge 与 rebase 均退出 0，结果分别为 `1` 与 `0` | 无冲突不保证两树相等；差异不能证明 merge 内容丢失 |
| 两边改同一行 | `merge-tree` 退出 1，输出的树含冲突标记 | 该树不能作为用户解完冲突后的预期结果，也不能提前挡住正常的 rebase 冲突流程 |
| mixed-revision 恢复的原分支含 merge 独有的 `Evil.cs` | §9.4 ⑦ rebase 退出 0，但文件消失 | 保护必须覆盖恢复流程，不能只加在 S9 / D8d |

因此本期**取消 S8b / S9b 及 D8d 的整树守恒断言**，采用修改前可判定的保守门槛。
不尝试区分「这次 merge 恰好安全」与「包含独有改动」，也不以 `--rebase-merges` 自动
重试；后者重建 merge 时同样不能保证保留原来的额外修改或冲突解决结果。

```text
输入：已解析并固定的 targetOid、tipOid
git rev-list --merges <targetOid>..<tipOid>
  命令成功且输出为空 → 待重放区间线性，可以进入普通 rebase
  输出非空           → 列出 merge OID，停止；不执行 stash / checkout / rebase / SVN 写操作
  命令失败           → 检查未完成，停止；不能把失败当作空集合
```

| 调用入口 | 检查区间 | 必须在什么时候检查 |
|---|---|---|
| `sync_in` | 当前 `svn/base` OID .. 当前分支 OID | S0 / S1 之前；S9 前对新基线再复核 |
| §9.4 恢复 | 当前 `svn/base` OID .. 当前分支 OID | ⓪ / ① 之前；⑦前再复核 |
| `sync_out` 的 D8d（有 Local-Only commit） | `deliverRef` OID .. `origHead` | D0.5 构造完成后、D3 确认及 D5 之前；D8d 前对实际新基线再复核 |

S7 和恢复⑥的新基线都以旧基线为祖先，D8a 的新基线以 `deliverRef` 为祖先，因此正常
推进不会把预检区间之外的 merge 突然加入重放范围。journal 记录检查用的不可变 OID；
继续操作前若分支或基线被外部改动，停止并保留 journal，不能沿用旧检查结果。

**只检查需要重放的区间，不扫描整条历史。** 已经在目标基线里的 merge 不受影响；
没有 Local-Only 的快路径交付不做 rebase，也不受这条门槛限制。快路径顶端的 Local-Only
后缀必须由单父提交组成；后缀内含 merge 时，D0.5 前即拒绝，不能把它当作普通补丁摘掉。
慢路径仍受 §8.1 #6 的线性历史要求约束。

阻断提示应说明「本期不自动重放 merge 历史」，保留原分支及工作内容，供用户保留原历史
后手工整理。**不能建议直接再跑一次普通 rebase，也不自动执行 reset 或历史改写。**

线性区间继续使用 `git rebase --empty=drop svn/base`：退出 0 后进入下一步；非零时检查
git 的 rebase 状态与未合并索引，确为冲突则释放操作锁、保留 journal / stash，进入
现有冲突解决流程。用户明确继续 / 放弃时分别走 rebase 的 continue / abort；没有
rebase 冲突状态的非零结果按命令错误处理。**不先运行 merge-tree，也不在用户解完冲突
后拿含冲突标记的树作等值断言。** 这条分流由 S9、D8d、恢复⑦共同使用。

### 7.4 不显然的点

- **S4 是双保险，不能省。** revision 比对能抓住「外部提交过」，抓不住「外部手工
  改了 base 文件」。这一步直接问 SVN 要事实
- **S5 的 `--accept postpone` 是硬要求。** S4 只验证 W-file 的内容干净；W-file 外的
  本地修改仍可能在 update 时产生内容冲突（§7.2.1），而
  **SVN 树冲突**（服务器删掉某个目录、或把目录换成文件）与工作副本干净与否无关。
  `postpone` 让它停下来报错，而不是弹交互提示卡死子进程，也不替用户自动选
- **S7 白名单内无变化时也要落 commit**（§6.3.1 的 `commit-tree` 允许相同 TREE）。基线记账必须跟上
  revision，否则下次 §6.6 的检测会误报失同步
- **S9 / S10 的冲突不是失败，是正常出口之一。** 此时必须**释放锁**（用户要编辑
  文件解决冲突），仓库进入「同步未完成」状态
- **本地补丁（§6.7）在这里不需要任何特殊处理。** Local-Only commit 就是普通 commit，
  S9 的 rebase 把它和其余 commit 一起搬到新基线上。它与服务器改动真冲突时，冲突就在
  S9 爆出来，和普通代码冲突走同一条路——这正是选 commit 而非未提交改动的收益
- **merge 历史门槛必须前置到 S0 之前，并在恢复入口同样执行**（§7.3.2）。线性分支的
  正常冲突由 rebase 自己产生和记录，不设置另一套 merge 参考树作为完成条件

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
  "newParentDirs": ["Assets/Scripts/Deep", "Assets/Scripts/Deep/Nested"],
  "expectedStartRev": 12345,
  "commitMessage": "…",
  "deliverRef": "refs/mygitclient/deliver/0f3a…",
  "tempWorktree": "C:/Users/x/AppData/Local/Temp/mygitclient-deliver-0f3a…",
  "localOnlyCommits": ["a1b2c3d…"]
}
```

`newParentDirs` 是 `svnDelta` 里状态为 `A` 的**目录**条目。它有两个用途，都不能省：
D5 的 target 列表必须带上它们，否则提交被 `E200009` 拒（§8.2.6）；回滚时要靠它把
`svn add --parents` 带出来的父目录登记撤干净（§8.5.1）。

每次准备录制基线时，journal 另记 `baselineOidBefore`、`captureTree`、`newBaselineOid`
及准备返回的流程阶段（§6.3.1）；这些是发布与重开恢复的锚点，不是 merge 参考树。
rebase 预检使用的目标 OID 与 `origHead` 同样落盘，禁止用可能漂移的 `ORIG_HEAD`
伪引用替代 journal 中的原始 OID。

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
| `post-commit` | 服务器已落地，基线未推进；D7a 可能因漏收文件而中止 | 不回滚；D6 失败可重试 D6，D7a 失败修正规则后重新核对候选 TREE，不重复 D5 |
| `publishing-baseline` | TREE 已核对；新基线 commit 可能已创建或已发布，工作分支尚未恢复 | 比对 journal 的新旧 OID 判定发布状态，按 §8.2.2 继续；不重新提交 SVN、不覆盖外部推进的基线 |
| `restoring-branch` | 基线已发布且索引已对齐；D8d 可能已切回原分支、正在 rebase 或已完成 | 按 HEAD、原分支 OID 与 Git 的 rebase 状态继续 D8d 或进入 D10；不得回到 D6 / D8c 覆盖冲突 |

`sync_in` 另有两个 phase 与 stash 相关，见 §7.3.1：

| phase | 已经发生了什么 | 放弃时怎么收尾 |
|---|---|---|
| `stashing` | `stashRefBefore` 已落盘，`git stash push` 可能落地也可能没有 | 比对当前 `refs/stash` 与 `stashRefBefore` 判定，**不静默 apply** |
| `stashed` | stash 已落地且 OID 已钉住 | 按 `stashOid` 恢复 |

S7 与恢复⑥另共用以下录制阶段，以 `flow` 及 `captureReturnStep` 区分后续入口：

| phase | 写前记录与继续规则 |
|---|---|
| `capturing-baseline` | 核对前记录旧基线 OID、目标 revision 与返回步骤；核对失败留在此处，继续时按 §6.3.1 重新验证前提、重建候选索引 |
| `publishing-baseline` | 创建 commit 前记录通过核对的 `captureTree`、父 OID；移动 ref 前再记录 `newBaselineOid`。重开比较新旧 OID，仅补尚未完成的发布 / 索引对齐；无新 OID 表示尚未发布，为其他 OID 则停止 |
| `baseline-ready` | 发布并对齐索引后、切走之前写入；S7 返回 S8，恢复⑥返回⑥b。此阶段起不得重新录制或对齐旧索引，后续 rebase 冲突由 Git 自身状态接管 |

这些继续入口还要确认 HEAD、磁盘与记录阶段相符；外部改写时停下，不用 `read-tree`
覆盖外部暂存，也不重新执行已经完成的 SVN 操作。`publishing-baseline` 是两个流程
共用的阶段名，但 `sync_out` 的父提交与返回入口按 §8.2.2 解释，不能套用 S7 的旧基线父。

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

0. **当前分支必须已经包含最新的 `svn/base`**：`git merge-base --is-ancestor svn/base HEAD`
   必须成立。不成立则拒绝交付，要求先对该分支跑一次 `sync_in`（同样受 §7.3.2 的 merge 门槛约束）。
   **这是全部前置检查里唯一挡得住「拿旧基线的分支交付」的一条**，见 §8.1.2
1. 基线未失同步
2. **工作区必须干净**，包含两层：
   - **W-file（git 侧）**：`git status --porcelain -- <白名单>` 为空。这里**不自动 stash**：
     交付的是 commit 过的内容，未提交的改动不该被悄悄排除、也不该被悄悄带上。硬拦住，
     让用户自己决定。若那些改动本来就不打算交付（注释掉的 log、临时测试代码），正确
     做法是**提交成 Local-Only commit**（§6.7），不是留在工作区里
   - **W-path 减 W-file（SVN 侧、git 管不到的那部分）**：白名单目录里那些被扩展名过滤
     排除掉的文件（`.psd` / `.zip` / `.bak`，§6.3.2）若有本地修改，**列出来并要求用户
     确认**，形式与 §7.2.1 一致。理由见 §8.2.1：D6 的 `svn update` 会走到它们头上，而
     那时已经在 D5 之后、不可撤销。初始化的 §6.5 P1 本来就是 W-path 粒度的干净要求，
     这一条只是把同样的粒度补回交付侧
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
   顶端（转成快路径，已在交付树内的 merge 不受影响），或保留原分支后手工整理历史。
   理由见 §8.2.3 的 T4 / T5：`cherry-pick` 对 merge commit 要么硬失败，要么在加
   `-m` 之后重复应用已摘取过的父提交改动，没有安全的自动解法。**快路径不做这个
   检查**——它只是给一个已有 commit 起名字，历史里有没有 merge 都无所谓
7. 快路径顶端的 Local-Only 后缀必须是**单父提交**，其中有 merge 则前置拒绝（§7.3.2）
8. 通过纳入集合预检（§6.3.1），再拿锁；锁内复核 HEAD / 基线 OID 没有变化

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

#### 8.1.2 拿旧基线的分支交付会把别人刚交付的文件当成删除提上去

§8.1 第 0 条是这一轮加的，它挡的是本设计**后果最重**的一条路径。

D4 是「本地多任务并行」（§2 D4），所以同时存在两个从同一个旧基线分出来的任务分支是
**正常用法**，不是误用：

```text
svn/base@r1 ──┬── 分支 A ── 交付成功 ── svn/base 前进到 r2（含 A 新增的 ANew.cs）
              └── 分支 B ── 还停在 r1 的基线上
```

此刻切到分支 B 交付，**原有的前置检查一条都拦不住**：

| 检查 | 为什么过得去 |
|---|---|
| #1 基线未失同步 | D6 已经把白名单更新到 r2，`nodeRevs == {r2} == baselineRev` ✅ |
| #5 `remoteRev == baselineRev` | 没有别人提交，服务器 HEAD 就是 r2 ✅ |
| SVN 自己的 out-of-date 检查 | 工作副本基线**确实是最新的**，服务器没有理由拒绝 ✅ |

然后 D1 那条两点 diff 就出事了。`git diff A..B` 是**树对树**比较，不是三点的
merge-base 比较：

```text
（2026-09-13 临时 git fixture 实测）
D1: git diff --name-status --no-renames svn/base..B
      D  ANew.cs       ← 分支 A 刚交付的文件，被算成「删除」
      A  BOnly.cs
对照 三点 diff（svn/base...B）：
      A  BOnly.cs      ← 只有这一条才是分支 B 真正做的事
```

于是 D4 `svn delete ANew.cs`、D5 把删除提交回服务器——**把同事（或自己）上一次的交付
撤销掉，且 D5 之后不可撤销**。

**改成三点 diff 不是正确的修法。** 三点 diff 只是把删除藏起来，交付内容仍然是基于
旧基线算出来的。修订前 D8a 的 `git branch -f svn/base HEAD` 还会把基线**倒退**回
分支 B 的树，`ANew.cs` 从基线里消失，而 D8d 的 `--ff-only` 反而成立、抓不到。
现行 §8.2.2 从已核对的 TREE 录制基线，但仍以交付树为父提交，仍依赖第 0 条祖先检查；
不能靠换一种 diff 或发布顺序替代这道前置。

所以唯一可靠的办法是在动手之前要求「当前分支已经包含最新基线」，也就是把
§8.2.2 第一句「`svn/base` 是交付树的祖先」从**假设**变成**被检查过的前提**。
用户看到的提示是「分支 B 还停在 r1，请先更新这个分支」。线性历史可走 `sync_in`；
待重放区间含 merge 时先按 §7.3.2 保留原分支并人工整理。

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
     有 Local-Only 时检查 deliverRef..origHead 无 merge      ← D8d 的提前门槛，§7.3.2
     不过则走 §8.2.4 收尾，绝不进入 D3 / D5
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
     **newParentDirs = svnDelta 里状态为 A 的目录条目**，写进 journal ←
       它们必须进 D5 的 target 列表，否则提交会被拒，见 §8.2.6
D4.9 提交前最后一道断言（纯本地，毫秒级）：
       git rev-parse HEAD == journal.deliverRef 指向的 OID
       git status --porcelain -- <白名单> 为空
     任一不满足 → **中止**，按 §8.5 撤销 D4 登记，报「工作副本在你确认之后
     被外部工具改动过」。见 §8.2.5
D5   phase=committing
     svn commit --non-interactive --depth=empty              ← --depth=empty 见 §8.2.6
       --with-revprop mygitclient:opid=<opid>
       -m <msg> -- <newParentDirs + 显式文件 target 列表>
D6   phase=post-commit  ← 过了这一步服务器状态已不可撤销
     svn update --accept postpone -- <白名单路径>            ← W-path 粒度，不是 W-file。
                                                               为什么只能这样、以及它
                                                               带来的残余风险，见 §8.2.1
     ⚠️ 白名单落在 external 里时，target 必须**显式指到那个 external 目录**
        （或其内部路径）。实测：显式指名的 update **不认 peg**，会走到该仓库
        HEAD，这正是我们要的；而在项目根跑递归 update 会把它拽回 peg，把刚
        交付的内容从磁盘抹掉。见 §6.5.3
D7   svn status -v --xml 读新的 nodeRevs → r_new
D7a  临时索引纳入集合核对 → TREE（§6.3.1）                  ← 不过则留在 post-commit，基线不动
D8   将 D7a 的 TREE 发布为新基线，并恢复原分支（见 §8.2.2）
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

所以 D6 用 `svn update -- <白名单路径>`，把范围从「整个工作副本」收窄到白名单。

代价是工作副本变成 mixed-revision，美术资产停在旧 revision。这**不是问题**：
§6.6 的检测本来就按白名单节点判定，美术更新交给下一次 `sync_in`（那里有 §7.2.1
的确认流程兜底）。

##### 原来那句论证是错的，残余风险必须写出来

初版这里写的是「范围限定在 git 管得住的文件上，这些文件此刻的内容与服务器 base
完全一致（刚提交完），合并必然干净」。**这句话把 W-path 当成了 W-file**（§6.3.2）。

`svn update` 的 target 只能是路径，所以 `svn update -- Assets/Scripts/` 会**递归更新
该目录下的一切**，包括被扩展名过滤排除掉的 `.psd` / `.zip` / `.bak`。那些文件 git
一个字节都不管，"刚提交完、内容与服务器一致" 对它们不成立——它们可能带着用户的本地
修改在那儿放了几个星期。

2026-09-13 临时 SVN fixture 实测（`Code/Art.psd` 在白名单目录内、被扩展名排除，本地
有修改，服务器上有另一次改动）：

```text
svn update --accept postpone -- Code
  不重叠的改动 → G    Code\Art.psd                     ← 被合并了
  重叠的改动   → C    Code\Art.psd
                 ?    Code\Art.psd.mine
                 ?    Code\Art.psd.r5
                 ?    Code\Art.psd.r6
```

也就是说本节开头那条「绝不在 D5 之后引入没法收拾的失败」的目标，在这个差集上**并没有
达成**。

**为什么不能改成按文件 target 更新。** 只更新本次交付的文件，白名单里其余节点会停在
`baselineRev`，`nodeRevs` 立刻变成 `{baselineRev, r_new}` 多值，§6.6.2 的失同步判据
当场误报。W-path 粒度的 update 在结构上是必需的。

**所以修法在前置，不在 D6**：§8.1 第 2 条把「W-path 减 W-file 的本地修改」补成一次
显式确认（形式同 §7.2.1）。这样用户在**动手之前**就知道哪些不受保护的文件可能被
合并，而不是在不可撤销之后才看到 `.mine` / `.rN`。

**剩下的窗口是 D3.5 到 D6 之间的窄竞态**，消不掉：§8.1 #5 与 D3.5 都确认过
`remoteRev == baselineRev`，所以只有「同事在我们 D5 之后、D6 之前提交了一次、且恰好
碰了白名单目录里某个我们也改过的非 W-file 文件」才会触发。命中时按 §13 报错，明确
指出是 git 管不到的文件，引导去 TortoiseSVN——与 §7.2.1 的收口一致，不假装客户端
能修。记为 §15 第 25 条。

#### 8.2.2 D8 推进基线：必须显式编排分支

D5 成功后，D6 可能又拉到他人的改动，所以新基线的内容取自 D7a **已核对的 TREE**，
父提交取交付树 `deliverRef`。`svn/base` 是交付树的祖先（§8.1 第 0 条），因此新 commit
仍是旧基线的后代，同时保留本次已交付的 git 历史。整个录制过程不得先 checkout 旧基线。

```text
D8a  先写 journal：phase=publishing-baseline、TREE、OLD_BASE_OID、deliverOid
     git commit-tree TREE -p <deliverOid>
        -m "svn: r_new … git-svn-id: <Repository Root>@r_new <uuid>" → NEWBASE
     将 NEWBASE 写入 journal 并落盘，之后才允许移动 ref
D8b  git update-ref refs/heads/svn/base NEWBASE OLD_BASE_OID  ← 比较旧 OID，变化则停止
D8c  git read-tree TREE                  ← 只对齐索引，不带 -u，磁盘已是候选快照
     git checkout svn/base               ← 此刻指向 NEWBASE，不会换回旧基线内容
D8d  先写 journal：phase=restoring-branch、targetBaselineOid=NEWBASE
     git checkout <ORIG_BRANCH>
     无 Local-Only commit：git merge --ff-only svn/base   ← 纯快进，不可能冲突
     有 Local-Only commit：
       复核 svn/base..origHead 无 merge，且原分支 OID 未被外部改写（§7.3.2）
       git rebase --empty=drop svn/base
                                         ← 已交付的 commit 按 patch-id 被丢弃，
                                           只剩本地补丁落到新基线之上
```

先核对 TREE、从 TREE 创建 commit，再发布指针与对齐索引，保证 D7a 失败时旧基线还在。
`publishing-baseline` 重开若尚未记录 NEWBASE，说明 ref 尚未发布，可从已记录的 TREE
重建 commit；若 `svn/base == NEWBASE`，只继续索引对齐与 D8d；仍为 `OLD_BASE_OID`
则尚未发布；为其他 OID 则停止，不覆盖外部改动。索引对齐前还须确认磁盘仍匹配已核对
的候选快照，否则停下保留现场。`restoring-branch` 重开则按 D8d 当前状态继续，不能再
从 D6 或 D8c 重放并覆盖用户正在解决的冲突。

D8d 的 `--ff-only` 是断言而非优化：**在没有 Local-Only commit 的前提下**走到这里必然
是快进，不是快进就说明前面某步的假设崩了，应当报错停下而不是自作主张 merge。有
Local-Only commit 时，先由 §7.3.2 检查待重放区间；rebase 完成后仍要求
**`svn/base..ORIG_BRANCH` 里只剩 Local-Only commit**（出现未标记 commit 就停止并导出
journal）。不与另一种合并算法的结果做整树等值断言。rebase 冲突按 §7.3.2 分流，进入
现有的用户解决 / 继续 / 放弃流程。

⚠️ 走到 D8d 时 D5 已经成功、基线已经推进，所以这里报错**不回滚**，语义与 §13 那行
「D8d rebase 冲突」一致：提交是好的，只是补丁没落回工作分支。保留 journal 中的
`origHead` 与 Git 的 rebase 状态，由用户选择后续操作，不自动 reset 或重试。

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

**快路径（全部 Local-Only commit 构成分支顶端连续的单父后缀）**

```text
git update-ref refs/mygitclient/deliver/<opid> <最后一个非 Local-Only commit>
```

只是给一个已有 commit 起个名字，**不动 HEAD、不动磁盘、结构上不可能冲突**。若用户
需要整理成本地补丁后缀，应先保留原分支，再用熟悉的工具人工调整；本期不自动执行
交互式 rebase，也不靠重排历史来绕过 merge 门槛。

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

#### 8.2.6 D5 的 target 列表必须带上新增的父目录

D1 的清单来自 `git diff --name-status`，**git 不跟踪空目录，所以清单里只有文件**。
而交付一个位于新目录下的文件时，`svn add --parents` 会把那些父目录也登记成 `A`。
只把文件放进 D5 的 target 列表，`svn commit` 会**直接拒绝**：

```text
（2026-09-13 临时 SVN fixture 实测）
svn add --parents Code/Deep/Nested/New.cs
svn commit -m "only file" -- Code/Deep/Nested/New.cs
  → svn: E200009: Commit failed (details follow):
    svn: E200009: '…\Code\Deep\Nested' is not known to exist in the repository
         and is not part of the commit, yet its child '…\New.cs' is part of the commit
```

这不是数据风险（服务器侧什么都没发生），但它让**任何涉及新增嵌套目录的交付永久失败**。

**本文档自己早就有证据。** §8.6.2 那段「比对前必须先剔除目录路径」记的是一次**成功**
提交之后 `svn log -v` 给出 `A /Assets/Scripts/Deep`、`A /…/Nested`、`A /…/New.cs`
三条——那次提交的 target 里一定包含了父目录，否则根本提交不出来。两处对不上。

所以 D4 把 `svnDelta` 里状态为 `A` 的**目录**条目单独记成 `newParentDirs`，D5 的
target = `newParentDirs` + 文件清单。

**必须同时加 `--depth=empty`。** `svn commit` 对目录 target 默认是递归的，直接把目录
放进 target 会顺带提交该目录下**所有**已登记但不属于本次交付的东西——正好违反 §12.1
「白名单外的文件绝不进入任何命令参数」的精神。实测 `--depth=empty` 精确到每个 target
自身：

```text
已登记：Code/Deep2、Code/Deep2/Nested、Code/Deep2/Nested/New.cs、Code/Deep2/Stray.cs
svn commit --depth=empty -- Code/Deep2 Code/Deep2/Nested Code/Deep2/Nested/New.cs
  → Adding  Code\Deep2 / Code\Deep2\Nested / Code\Deep2\Nested\New.cs
  → Committed revision 10.
  svn log -v -r HEAD 只有那三条；Stray.cs 仍停在 A 状态未被提交 ✅
```

`newParentDirs` 本来就要写进 journal 供 §8.5.1 回滚用，这里是同一份数据的第二个
用途，不额外增加状态。

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

这条在「白名单分布于多个嵌套工作副本」时**依然成立**，前提是它们同属一个仓库：
跨同仓库多个 WC 的 target 列表产生**单个** revision（§6.6.5 E4 实测）；而跨**不同**
仓库的 target 列表被 `svn commit` **动手之前就拒绝**（E200007，E5 实测），同样不会
留下半吊子状态。§6.5 P4 把后者提前到配置校验拦住，是为了不让用户白走完确认流程。

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

⚠️ **同一批父目录在成功路径上也必须被处理，而且方向相反**：回滚时要把它们的登记撤掉，
成功时要把它们**放进 D5 的 target 列表**，否则提交根本发不出去（`E200009`，§8.2.6）。
本节只写了失败侧，初版就漏了成功侧——所以 D4 把它们单独记成 `newParentDirs`（§7.6），
一份数据两处用，不要各算各的。

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
   svn log --xml -v --with-all-revprops
           -r <expectedStartRev+1>:HEAD <Repository Root>
                                        ↑ **必须是仓库根**，不是 svnUrl，见下
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

**查证的 URL 必须是 `Repository Root`，不能用 `svnUrl`**

`svn log <URL>` 只返回**该 URL 子树**的历史，它不跟随 externals。而 §6.2.1 的 `svnUrl`
是一个**工作副本 URL**，§16.0 已经确认公司项目的代码分布在 `code` / `script` 两条
不同 URL 的 external 里（§6.6.5）。于是：本次交付只碰了另一个 WC 的文件时，那个**真的
成功了**的 revision 根本不会出现在 `svn log <svnUrl>` 的结果里。

2026-09-13 临时 SVN fixture 实测（一次只碰 `Script/` 的提交，带 `mygitclient:opid`）：

```text
svn log --with-all-revprops -r 1:HEAD <Repository Root>        → opid 命中 1 次 ✅
svn log --with-all-revprops -r 1:HEAD <Repository Root>/Code   → opid 命中 0 次 ❌
逐条对照：root 列出 r1..r8；Code 只列出 r1..r6（r7 / r8 只碰了 Script/）
```

落进本节流程就是最坏的那条路径：①「区间内无任何 revision 带 opid」→ **判定失败** →
按 §8.5 回滚 → 同一批内容被**重复交付**一遍。这正是 §8.6.1 那张表里「假阴性」那一行。

`Repository Root` 本来就已经是基线 trailer 的 `<url>`（§6.4.1），取它零成本；P4 保证
全部白名单同属一个仓库（§6.5 P4 / §6.5.5），所以「一个 Repository Root」这件事是成立的。

⚠️ 兜底 ② 的 changed-paths 比对同理：路径要按 `Repository Root` 的仓库相对形式还原，
不能按 `svnUrl` 剥前缀（§6.8.4 那条剥前缀规则是对 `svn diff --old=<url(P)>` 说的，
两处的基准 URL 不同，不要互相套用）。

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

**先做前置检查，再开始下面的修改。** HEAD 必须附着在非基线分支上，且不在现有的
merge / rebase 中途；执行 §7.3.2 对 `svn/base..HEAD` 的 merge 检查与 §6.3.1 的纳入
集合预检。任一不过，不 stash、不 checkout、不动 SVN。拿锁后复核检查用的 OID 没变。

自动恢复只处理 **W-file 中已受保护的文件**。W-path 减 W-file 的本地修改不进入
revert / 删除清单；若本次需要⑤b的目录 update，必须在⓪之前列出它会触及的差集本地
修改（含 `M` / `!`）并按 §7.2.1 确认，已有冲突则先停止。该确认只授权 update 的
合并风险，不能用它授权还原原文。开始⑤b前重新核对差集，出现未确认的新改动时停止。

```text
⓪ 挂起 watcher；写 journal（flow=recover），phase=start
   origBranch / origHead / baselineOidBefore 此刻落盘 ← 任何修改之前，见 §7.6
① git stash push -u -- <白名单>；立刻取 OID 并用
    refs/mygitclient/sync/<opid> 钉住，写进 journal   保护未提交改动，可逆
    （S1 那套写前顺序同样适用，见 §7.3.1）
② 核实 W-file 的恢复前状态已由旧基线 / origHead / 本次 stash 记录
   （含 staged / 未跟踪 / 删除状态）；无法取回的文件禁止进入自动恢复清单
③ git checkout svn/base                 磁盘 = 旧基线内容
④ 分两步定出要动的东西，见 §9.4.1
   ④a 在各 wc(P) 扫描 svn status -v --xml --no-ignore --ignore-externals -- <W-path>
      先保留完整结果供 revision / 版本控制集合使用；revert 候选只取
      W-file 内且②确认已有备份的具体文件，剔除目录和扩展名排除文件，再按状态分类：
        (a) M / !            → svn revert   恢复服务器 base 内容
        (b) R（replace）     → svn revert   同上
        (c) A / A+（外部登记）→ svn revert   取消登记
   ④b 「服务器已删除」的集合**不看 svn status 的 ? 条目**，改用两个集合作差：
        oldBaselineFiles = 旧基线 tree 中属于 W-file 的文件
        svnVersioned     = ④a 完整结果里 SVN 仍在版本控制的路径集合（不是 revert 候选）
        toDelete = oldBaselineFiles - svnVersioned      ← 全是**具体文件**，没有目录
   ⚠️ 每一类都独立判空：列表为空则跳过，绝不执行空参数的 svn 命令或删除
⑤ 按 ④ 的结果执行：(a)(b)(c) 用已过滤的显式文件列表 svn revert -- <files>；
   toDelete 逐个**文件**删除，**绝不递归删目录**；全部删完后可以清理变空的目录本身
⑤b **revision 归一**：重读 nodeRevs（全部 wc(P) 取并集，§6.6.5）
      单值 {N} → 直接进 ⑥
      多值     → 确认差集保护前置仍成立后，svn update --accept postpone -- <W-path>
                 再读状态及 nodeRevs；失败、冲突或仍多值均停止，不录基线（§9.4.2）
⑥ 临时索引纳入集合核对 → 使用通过核对的 TREE 录制新基线（§6.3.1）
                                        父提交为旧基线，rev 取⑤b归一后的 N；
                                        按 §7.6 记录录制 / 发布阶段，完成索引对齐后进⑥b
⑥b 基线体检（§6.8）                     此刻 HEAD 在 svn/base、W-file 的 Git 状态干净，正是
                                        体检的前提。有差异则停在这里报错，
                                        **不继续 ⑦**——恢复流程的全部目的就是修好
                                        基线，不验证等于没修
⑦ git checkout <原分支>
   复核待重放区间及 origHead 未被外部改写（§7.3.2）
   git rebase --empty=drop svn/base      已被误传上去的 commit 会变成空 commit，自动丢弃
                                        冲突走 §7.3.2；保留 journal / stash，等待用户解决
⑧ git stash apply <OID>；成功后 drop 该条目并删掉钉住的 ref
⑨ 删除 journal；释放锁；恢复 watcher 并发一次 repo-changed
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

#### 9.4.2 ⑤b 归一不能省：恢复流程的主场景本身就是 mixed-revision

⑥ 原来写的是「rev 取自 ⑤ 之后重读的 nodeRevs」，而 `nodeRevs` 是个**集合**——这句话
默认了它此刻是单值。**在本节的主场景里它不是。**

§9.1 描述的就是「用户在 TortoiseSVN 里对整个工作副本点了 SVN Commit」。`svn commit`
**只推进被提交节点的 revision**（§6.6.1），所以提交之后工作副本立刻是 mixed-revision。
而 ①–⑤ 这几步**一个都不会动 revision**：`git checkout` 只改磁盘，`svn revert` 只把
内容改回各节点自己的 base，删文件更不会。

2026-09-13 临时 SVN fixture 实测：

```text
基线      nodeRevs = {1}
外部只提交 Code/A.cs        → Committed revision 3
                             nodeRevs = {1, 3}     ← mixed
改一个文件再 svn revert -R   → nodeRevs = {1, 3}     ← **纹丝不动**
svn update -- Code          → nodeRevs = {3}        ← 只有它能归一
```

不加 ⑤b 的后果：⑥ 拿不到一个合法的单值 `N` 写进 trailer。无论怎么凑（取最大值、取
根的 revision），录下来的基线都会在**下一次刷新时立刻被 §6.6.2 判成失同步**——恢复
向导跑完，红色横幅原地复现，而这正是它要修的东西。

**⑤b 为什么放在 ⑤ 之后而不是之前。** `svn update` 会用三方合并处理本地修改；必须先由
⑤ 把已受保护的 W-file 内容还原成各节点的 base。这个结论**只覆盖 W-file**，不能把
「整个 W-path 干净」当作⑤的目标，否则会顺带 revert 没有 Git 备份的 `.psd`。

**⑤b 的 target 是 W-path，所以它带着 §8.2.1 同样的残余风险**：白名单目录里那些被扩展名
排除掉的文件会被一起更新。§9.4 的前置确认必须覆盖这些本地修改；⑤b仍可能合并它们、
产生冲突或部分完成 update，不能宣称可由 Git 还原、或失败「只是白跑一趟」。失败时保留
journal / stash，停止录制基线，指出受影响的 WC / 文件，交由 TortoiseSVN 处理。

仍多值时**停下报错**，不要自作主张再 update 一次或换个 revision 凑数（§8.7）。
常见成因是白名单跨了多个 WC 而其中一个 update 失败，错误信息要指明是哪个 WC。

### 9.5 第 ⑤ 步为什么安全

涉及破坏性命令，论证必须显式：

- 自动 revert / 删除只作用在 **W-file 且已有恢复快照**的具体文件上。W-path 只是状态
  扫描范围，`Code/Art.psd` 这类扩展名排除文件即使状态为 `M` 也不进入⑤
- 恢复前的代码状态由旧基线、`origHead` 及钉住的 stash（含未跟踪文件）记录，②检查后
  才可操作。`stash -u` 不保护 ignored 文件，所以不能仅凭「执行过 stash」声称可逆
- 目标由 `svn status` 枚举后按上述条件**再次过滤**，不是 `svn revert -R <目录>`；
  对 W-file 外文件的 update 确认不能替代 revert 所需的内容保护
- 空列表短路，不执行
- **本地补丁不会被误伤**：第 ③ 步已经切到 `svn/base`，Local-Only commit 活在工作分支
  上、不在磁盘上（§6.7.1），因此不会因本地补丁本身而 revert 文件。第⑦步仅重放通过
  §7.3.2 门槛的区间；merge 历史在⓪前就被阻断，不能恢复完基线后再静默丢掉合并结果

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

### 12.1 按操作选择白名单粒度

所有构造 git / svn 参数的地方走**同一个过滤函数**，然后专门测那个函数。这不是
「测一下功能」，是给整个方案的安全边界上一道断言。

⚠️ 这个函数必须**同时导出 W-path 与 W-file 两个集合**（§6.3.2），并且让调用点在签名
层面就选定一个，不要让它自己从路径列表里现拼。混用这两个集合已经造成过 §8.2.1 的
错误论证。只检查命令退出码的测试发现不了它，必须用差集文件的真实本地修改验证：
update 要事前确认，自动 revert 不得触及这些文件（§14.2 第 65、67 项）。

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
| S9 / 恢复⑦ rebase 冲突 | 线性提交与新基线发生正常的内容冲突 | **不回滚**，保留 journal / stash，停在 Git 的 rebase 冲突态并释放锁 | 进入冲突解决模式，按未合并索引列出文件；用户解决后继续，不做 `merge-tree` 整树等值断言 |
| S10 stash apply 冲突 | 未提交改动与新基线冲突 | 不回滚，**stash 与钉住的 ref 都保留不删** | 同上 + 给出 stash 的 OID |
| D5 各类失败 | 见 §8.6 | 见 §8.5 | 见 §8.6 |
| **D6 update 失败 / 冲突** | 服务器在 D5 之后又有提交，且与白名单文件冲突 | **不可回滚**——D5 已落地 | 明确告知「提交已成功、基线尚未推进」，journal 停在 `post-commit`，提供「重试 D6 起」的入口 |
| **D0.5 前置：慢路径历史含 merge，或 Local-Only 后缀含 merge** | 不满足 §8.1 #6 / #7 的单父提交约束 | **不做任何修改**——检查在动手之前 | 列出 merge commit；由用户保留原分支、确认改动后人工整理为受支持的历史，不自动重写 |
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
| **白名单跨仓库** | 白名单根的 `Repository UUID` 不全相同（§6.5 P4） | **不做任何修改**，仓库不进入可用状态 | 指出是哪两条分属哪两个仓库；说明 `svn commit` 会以 E200007 直接拒绝，不是本客户端的限制 |
| **白名单所在 external 被 `-r` 钉死** | `svn:externals` 里带 `-r N` 或 `URL@N`（§6.5 P5、§6.5.2） | **不阻断**，进入 pinned 模式（§6.5.3） | 说明客户端会改用「根 `--ignore-externals` + 显式更新各 external」的方式，并提示残余风险：外部工具在项目根跑递归 update 会把代码退回 peg |
| **代码被外部递归 update 退回 peg** | 用户或 TortoiseSVN 在项目根跑了普通 `svn update`（§6.5.3 测点 C） | 不做任何修改 | `nodeRevs` 掉回 peg revision 时明确报出成因，提供一键「`svn update <目录>` 拉回 HEAD」；说明内容没丢（在服务器上，也在 git 对象库里） |
| **行为校验发现疑似钉住的 external** | `svn update` 之后白名单所属 WC 的 revision ≠ 该仓库 HEAD（§6.5.2 手段二） | 不阻断当前流程 | 提示「疑似被钉住，请核对 `svn:externals`」，**不单独据此阻断**（存在窄竞态） |
| **基线体检不通过**（§6.8） | 基线内容与服务器在 `baselineRev` 上的内容不符——假基线已经产生 | 按时机分：初始化时**整个回退、不创建 `.git`**；§9.4 第 ⑥b 步**停在那里不继续 ⑦**；Phase 1 R3b **中止，连 `svn/base` 指针都不动** | 列出差异文件（已剥掉 URL 前缀、剔除目录条目与纯属性条目，§6.8.4），说明是内容不符而非 revision 不符。走 §6.8.7 判据时只列 `D \ S` 里的那些——用户自己的本地修改不是差异，列出来会把人引向错误的方向 |
| **重建 / 初始化时工作副本 obstructed**（`~`） | svn 说不清那个位置上是什么，取不到 BASE 内容，无法录出服务器 rN 的树（§16.1） | 不录制、不动基线指针；这是 P1 收窄后**唯一**仍然阻断的脏状态 | 指名那几条路径，引导去 TortoiseSVN 处理（`cleanup` 或手工还原），**不自动 revert** |
| **重建过程中 `nodeRevs` 被外部推走**（R3c） | R2a 之后有人在 TortoiseSVN 里 commit / update，BASE 已不是 N，取回的字节不再对应 trailer 里的 revision | R3c 中止，指针不动；R3b 的 `D \ S` 通常已先一步拦下 | 说明工作副本在重建过程中被外部改动，请重新点一次「重建基线」 |
| **基线体检跑不成功** | 断网、认证失败、`svnUrl` 不可达 | **不阻断**，不做任何修改 | 标记「基线未经体检」并如实显示；**不得当成体检通过**（§6.8.6） |
| **拿不到跨进程锁** | 同一工作副本上另一个 mygitclient 进程正在同步（§5.1.1） | 不做任何修改 | 显示持锁进程的 pid 与开始时间；pid 已不存在时提供「接管」入口并写日志 |
| **D8d 不是快进**（无 Local-Only commit 时） | 前面某步的假设崩了 | 停下不动 | 报错并导出 journal，**不自动 merge** |
| **D8d rebase 后仍有未标记 commit** | 该交付的东西没进交付树 | 停下不动 | 报错并导出 journal，列出这些 commit |
| **D8d rebase 冲突**（有 Local-Only commit 时） | 本地补丁与他人在 D6 拉下来的改动冲突 | **不回滚**，保留 journal，停在冲突态并释放锁 | 进入 Git 的冲突解决流程；提交已成功，基线已推进，只剩补丁没落回去，不从 D6 重新执行 |
| **交付分支不含最新基线** | 从旧基线分出的另一个任务分支已经交付过，当前分支还停在旧基线（§8.1 #0、§8.1.2） | **不做任何修改**——检查在动手之前 | 说明当前分支落后于 `svn/base`，提示先对这个分支跑一次 `sync_in`；明确告知若强行交付会把上一次交付的文件当成删除提上去 |
| **白名单目录内有 git 管不到的本地修改** | W-path 减 W-file 的差集里有 `M` / `!`（§6.3.2、§8.1 #2、§9.4 前置） | 未确认前**不做任何修改**；已存在冲突则先停止处理 | 列出 D6 / 恢复⑤b 会碰到的文件，说明 Git 没有恢复副本；确认只授权 update，不能据此对这些文件执行 revert |
| **待重放区间含 merge commit** | 普通 rebase 可能跳过 merge 独有内容（§7.3.2） | `sync_in` / 恢复在 stash 前拒绝；交付在 D0.5 后、D3 / D5 前拒绝并清理本流程临时 ref，主工作副本不动 | 列出 merge OID，说明当前版本不能自动重放；保留原历史，由用户人工整理，不先 rebase 再尝试恢复丢失内容 |
| **rebase 前复核失败** | 目标或原分支被外部工具改写，或 merge 查询失败 | 停止，保留 journal / stash；D5 之后不回滚 SVN | 如实说明状态变化或命令错误；不能把查询失败当作「没有 merge」，不能自动 reset 或换算法重试 |
| **恢复的自动还原清单不受 Git 保护** | 文件不在 W-file 内，或旧基线 / origHead / stash 中没有可恢复状态（§9.4 ② / ④a） | 不把它加入 `svn revert` / 删除清单；受保护状态无法确认则停止 | 列出不能自动还原的文件，由用户先处理；目录 target 和扩展名排除项永不递归 revert |
| **恢复⑤b 前出现新的未确认差集修改，或 update 产生差集冲突** | 编辑器 / 外部 SVN 在流程期间改变了 Git 不管理的文件 | 新修改在 update 前停止并重新确认；已有 update 冲突保留现场，不继续录基线 | 明确指出这些文件没有 Git 恢复副本；冲突转 TortoiseSVN 处理，不套用 Git stash 恢复承诺 |
| **D5 被拒：父目录不在提交里** | `newParentDirs` 没进 target 列表（§8.2.6） | 服务器侧什么都没发生；按 §8.5 完整回滚 D4 登记 | 原样展示 `E200009`；这是实现缺陷而非用户问题，导出 journal |
| **⑤b revision 归一失败** | `svn update` 之后 `nodeRevs` 仍多值（§9.4.2） | **停在 ⑤b，不录基线** | 指出是哪个 `wc(P)` 没收敛、当前的多值集合是什么；引导去 TortoiseSVN 完整更新该目录后重来 |
| **纳入集合核对不通过** | 候选临时索引的完整文件集合与 W-file 不符，可能漏收或多收（§6.3.1） | 不发布候选基线：初始化只回退本次创建的 `.git`；S7 / 恢复⑥保留旧基线、journal / stash；D7a 停在 `post-commit`，不回滚或重复 D5；Phase 1 R2 不动 HEAD / 真实索引 / 基线 | 列出漏收、多收文件及 `git check-ignore --no-index -v --non-matching` 的规则来源；修正后从对应录制阶段重新核对，不另起同步覆盖旧 journal |
| **白名单子树内有跨仓库的嵌套 external** | `Code/Vendor` 之类指向另一个仓库（§6.5 P4、§6.5.5） | **不做任何修改**，仓库不进入可用状态 | 指出是哪条嵌套 external 属于哪个仓库；说明它会被 git 跟踪却不受 `nodeRevs` 监控，且 D5 会以 E200007 被拒 |
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

⚠️ **fixture 必须能造出 `svn:externals`**（测试项 48 / 49 依赖它）。用
`svn propset svn:externals` 指向**同一个 fixture 仓库**的另一路径即可，不需要第二个
仓库、不需要网络；加 `-r` peg 就能复现 §6.6.4 那种「稳定且看起来合理」的多值
`nodeRevs`。

⚠️ **fixture 是按设计者的预期造出来的**，它不会包含设计者没想到的东西。§18（五）那条
阻断级缺陷（externals 打穿 `nodeRevs`）在自造 fixture 上跑了三轮都没露头，是拿真实远端
工作副本一比就暴露的。所以自动化测试之外，**至少要在一个真实工作副本上做一次只读
核对**，尤其是状态判定这类「读」的逻辑。

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
| 13 | **mixed revision 检测**：**在 wc1 自己身上**用外部 `svn` 命令只提交一个深层文件（`svn commit -- <单个文件>`），wc1 的根 revision 不动，断言 `nodeRevs` 变成**多值**、校验报失同步（§6.6.1）。⚠️ **不能用「wc2 提交、wc1 不动」来造这个场景**——实测那样做 wc1 的 `.svn` 元数据纹丝不动，`nodeRevs` 仍等于基线，正确实现**本就不该报**；那测的是「服务器有新提交」，不是本地 mixed-revision（`AGENTS.md` §8 测试名称与真实形态一致） | 1、3、9 |
| 13b | **「服务器有新提交」不得被误报成失同步**：wc2 提交、wc1 完全不动，断言 `nodeRevs == {baselineRev}` 且**不报**失同步（这是 13 的对照组，两个方向都要测） | 1、3 |
| 14 | **断网查证**：制造「提交成功但客户端认为失败」，用 opid / changed-paths 在完整区间内正确判定（§8.6.2）；另测假阳性场景——先用同样的 message 提交过一次 | 1、2 |
| 15 | **D6 白名单外有本地修改时交付仍安全**：wc1 改一个美术文件不提交，wc2 提交别的内容，走完整交付，断言美术文件的本地修改**未被合并、未产生冲突文件**（§8.2.1） | 2、4 |
| 16 | **逐 phase 崩溃恢复**：在 S1 后、**D0.5 构造中途**、**D0.5 完成后**、D3.8 后、D4 后、D5 后各杀一次进程，重开必须能按 journal 给出正确的继续 / 放弃路径；D0.5 那两次要断言临时 worktree 被清理、`deliverRef` 被删、HEAD 仍在 `origBranch`；D3.8 后那次要断言能切回 `origBranch` 并清掉 `deliverRef`（§7.6、§8.2.4、§8.5） | 2、5 |
| 17 | **stash OID 抗漂移**：stash 之后由外部再压一个 stash，恢复必须仍取回正确的 WIP（§7.6.1） | 1、5 |
| 18 | **D8 分支编排**：交付后基线确实推进到新内容，且工作分支是快进；断言不会出现「基线内容是旧的却标着新 revision」（§8.2.2） | 1 |
| 19 | **Local-Only commit 绝不被交付**：同一文件里既有本地补丁又有待交付改动，走完整交付后断言服务器上是干净原文、磁盘上补丁还在、`svn/base..HEAD` 只剩那个补丁（§8.2.3 T1+T3） | 1、4 |
| 20 | **Local-Only 与待交付 commit 重叠时交付必须中止**：断言 SVN 侧一个字节都没动、`refs/mygitclient/deliver/<opid>` 与临时 worktree 已清理、原分支原样、**主工作副本磁盘内容全程未变**（§8.2.3 T2） | 1、2、4 |
| 21 | **本地补丁跨 `sync_in` 存活**：wc2 提交别处改动，wc1 走 `sync_in`，断言补丁被 rebase 到新基线上且内容不变（§6.7.1） | 1 |
| 22 | **恢复流程不误伤本地补丁**：在补丁文件没有其他外部修改的前提下触发 §9.4，断言 `svn revert` 清单不含该文件、⑦之后补丁完好；另造同文件有外部提交的情形，允许恢复已受保护的基线内容，但断言原分支 / stash 的补丁仍可取回（§9.5） | 2、4 |
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
| 37 | **Phase 1 重建基线录的是新内容**：外部 TortoiseSVN update 之后跑重建，断言新基线 tree == 通过集合核对的当前磁盘 W-file tree（**不是**旧基线）、HEAD / 真实索引 / 工作目录全程未动、工作分支上的未提交改动完好；核对失败时旧基线指针也不动（§16） | 1、3 |
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
| 48 | **externals 不得污染 `nodeRevs`**：fixture 里给白名单**外**的目录挂一个 `svn:externals`（指向同一 fixture 仓库的另一路径，并用 `-r` peg 固定在旧 revision），断言 `nodeRevs` 仍是单值、失同步检测**不报**；再断言去掉 `--ignore-externals` 时会误报——**两个方向都要测**，只测「加了之后对」证明不了这个参数是必需的（§6.6.4） | 1、3 |
| 49 | **白名单落在 external 内必须能用**：fixture 造「项目根 + `code` 同仓库 external」，白名单只写 `code/`，断言初始化、`sync_in`、`sync_out` 全流程走通，且记账取的是 `code` 那个 WC 的 URL/revision 而非项目根（§6.6.5） | 1、3 |
| 51 | **白名单跨仓库必须被拒**：白名单同时覆盖主仓库路径与跨仓库 external，断言配置加载时就拒绝、**不发生任何 git / svn 命令**；再单独断言真去提交会得到 E200007（证明这条前置拦的是真问题，不是想象出来的）（§6.5 P4、§6.6.5 E5） | 3、4 |
| 52 | **pegged 的四种写法都要被识别**：`-r2 URL LOCAL`、`-r 2 URL LOCAL`、`URL@2 LOCAL`、`LOCAL -r2 URL` 各造一个，断言**全部**被判定为钉住并进入 pinned 模式；再断言无 peg 的定义**不被误判**（§6.5.2） | 3、11 |
| 53 | **pinned 模式下交付仍然走得通**：白名单落在被钉住的 external 里，走完整 `sync_in` + `sync_out`，断言 update 走的是「根 `--ignore-externals` + 显式指名 external」、交付内容在流程结束后**仍在磁盘上**、基线记的是 external 自己的 revision（§6.5.3 测点 A、B） | 1、2 |
| 55 | **外部递归 update 导致的回退可被检出并修复**：pinned 模式下交付成功后，用外部 `svn update <项目根>` 制造回退，断言 ① `nodeRevs` 掉回 peg 且被识别为**这个具体成因**（而不是泛泛的失同步）② git 侧也能看出磁盘与基线不符 ③ 一键 `svn update <目录>` 之后两侧都恢复正常（§6.5.3、§6.6.2） | 1、2、5 |
| 56 | **未提交改动时的递归 update 不是静默丢失**：pinned external 里留未提交改动，跑根级递归 update，断言产生冲突标记而非无声回退——两种形态要分别处理，不能合并（§6.5.3 测点 D） | 1、2 |
| 54 | **跨多个同仓库 WC 的交付是原子的**：白名单同时覆盖项目根与同仓库 external，一次交付触及两侧，断言产生**单个** revision，且 `nodeRevs` 并集在 update 之后收敛到同一个 N（§6.6.5 E4、E6） | 1、2 |
| 50 | **`svn status -v --xml` 的解析**：断言取的是 `<wc-status revision>` 而非 `<commit revision>`——构造一个「所有文件 base 都是 rN、但最后修改 revision 各不相同」的 fixture，断言 `nodeRevs == {N}`；并断言解析走 XML parser（属性跨行的输入不会解析成空集）（§6.6.4） | 1、3 |
| 57 | **旧基线分支交付必须被前置拒绝**：从同一基线开 A / B 两个分支，A 走完整交付推进 `svn/base`，再切到 B 发起交付。断言 §8.1 #0 拒绝、**SVN 侧一个字节没动**；再断言若绕过该检查，D1 的两点 diff 会把 A 新增的文件列成 `D`（**这一条作为反向证据单独断言，证明这道门禁拦的是真问题**）（§8.1.2） | 1、3、4 |
| 58 | **`svn info` 不得带 `--ignore-externals`**：断言实现发出的 `svn info` 命令行里**没有**这个参数；再断言带上它时 svn 返回非零并被识别为实现缺陷而非仓库问题（§4.4） | 1、3 |
| 59 | **`.svn` 在任何深度都不进 git**：白名单覆盖一个 external 根，分别用 `extensions: [".cs", ".db"]` 和 `extensions: ["*"]` 生成规则，断言 `git ls-files` 里**没有**任何 `.svn/` 下的路径（含 `wc.db` 与 `pristine/**/*.svn-base`），且 `git check-ignore -v` 指向兜底那一行（§6.3） | 4、9 |
| 60 | **初始化检测工作树 `.gitignore` 的覆盖**：在白名单目录里预置一个 `.gitignore`（内容 `*.cs`），断言初始化的纳入集合核对**阻断**、不创建基线，且错误信息里含 `git check-ignore --no-index -v --non-matching` 指认的那个文件与行号（§6.3.1） | 1、3、9 |
| 61 | **白名单子树内的跨仓库嵌套 external 必须被拒**：fixture 造「`Code/` 是 repo1 的 external，`Code/Vendor` 是 repo2 的 external」，断言 P4 在**枚举到嵌套 WC 之后**拒绝、不发生任何 git / svn 写操作；再单独断言「只查白名单根」的旧写法会**放行**（反向证据，证明 §6.5.5 的枚举不是过度设计） | 3、4 |
| 62 | **交付新增嵌套目录下的文件**：交付 `Code/Deep/Nested/New.cs`（两层父目录都是新的），断言 D5 成功、`svn log -v` 里三条 `A` 都在；再断言同目录下另一个**已 `svn add` 但不属于本次交付**的文件**没有**被顺带提交（`--depth=empty` 的作用）（§8.2.6） | 1、3、4 |
| 63 | **同步在修改前拒绝重放 merge**：构造含独有文件的 merge commit，使其位于 `svn/base..HEAD`；调用 `sync_in`，断言在 S0 / stash 之前被拒绝，HEAD、索引、磁盘文件与 SVN 节点 revision 均未变化，独有文件仍在。另测 merge 查询非零退出必须停止，不能当作空结果（§7.3.2） | 1、2、4、9 |
| 64 | **恢复流程能修好 mixed-revision**：**在 wc1 自己身上**外部只提交一个文件造出 `nodeRevs` 多值，走 §9.4，断言 ⑤b 之后 `nodeRevs` 收敛到单值、新基线的 rev 取的就是它，且**流程结束后立刻重新检测不再报失同步**（§9.4.2） | 1、2、3 |
| 65 | **D6 会碰到 git 管不到的文件**：白名单目录里放一个被扩展名排除的文件（`.psd`），wc1 改它不提交，断言 §8.1 #2 把它列进确认清单；再构造「D5 之后 wc2 提交了该文件的冲突改动」，断言 D6 产生 `C` + `.mine`/`.rN` 时按 §13 明确报成「git 管不到的文件」而不是普通冲突（§8.2.1、§6.3.2） | 1、2、4 |
| 66 | **断网查证用的是 Repository Root**：fixture 造两条同仓库 external（`code` / `script`），只交付 `script` 下的文件并制造「提交成功但客户端认为失败」，断言查证**命中** opid、判定成功、不回滚；再断言若用 `svnUrl`（`code` 的 URL）查则**查不到**——两个方向都要测（§8.6.2） | 1、2、9 |
| 67 | **恢复不 revert 扩展名排除的本地文件**：在 wc1 自己局部提交造 mixed-revision，同时修改 `Code/Art.psd`（W-path 内、W-file 外）；确认⑤b 的 update 后恢复，断言 revert targets 只有受保护的代码文件、`.psd` 字节仍为原本地修改、`nodeRevs` 已归一且按 W-file 体检通过。另测未确认 / 差集已有冲突时修改前拒绝，以及无法证明恢复状态的文件不进入自动还原清单（§9.4） | 1、2、4、9 |
| 68 | **恢复也在 stash 前拒绝重放 merge**：在 wc1 自己局部提交造 mixed-revision，Git 原分支含 merge 独有文件；调用恢复入口，断言在⓪ / ①之前被拒绝，原分支、索引、文件、SVN 节点 revision 均未变化（§9.4 前置、§7.3.2） | 1、2、4、9 |
| 69 | **线性 rebase 的普通冲突可解决并继续**：分别在 S9、含 Local-Only 的 D8d、恢复⑦构造同一行的真实内容冲突；断言进入 Git 的 rebase 状态、索引存在未合并条目、锁释放且 journal / stash 保留；用户解决后继续成功，不拿含冲突标记的 `merge-tree` TREE 校验解决结果（§7.3.2） | 1、2、5、9 |
| 70 | **后续新增 `.gitignore` 不能让基线漏文件**：初始化成功后由 wc2 提交 `Code/.gitignore`（`*.cs`）与 `Code/New.cs`，在 wc1 实际拉取后分别走 S7、D7a、恢复⑥与 Phase 1 R2；每个入口都必须检出遗漏、保持旧基线。D7a 保持已提交状态且不重复 D5；修正规则后从原录制阶段重新核对并纳入 New.cs。另测实际索引多收文件同样阻断，不能仅核对期望集合的子集（§6.3.1） | 1、2、3、9 |
| 71 | **线性往返修改不被误报为内容丢失**：基线 A 为 `0`，本地两个普通 commit 依次 `0→1→0`，新基线 A 为 `1`；断言前置允许，rebase 成功后的 A 为 `0`，不要求等于端点 merge 的 `1`。另测已交付的相同补丁正常丢弃，不误报失败（§7.3.2） | 1、3、9 |
| 72 | **merge 门槛只覆盖待重放区间**：① 无 Local-Only 时交付树包含 merge，仍可按快路径交付并快进；② merge 已在 deliverRef 内、其后只有连续单父 Local-Only 提交时，D8d 可成功重放且独有文件保留；③ Local-Only 后缀自身含 merge，或 D0.5 后待重放区间仍有 merge，必须在 D3 / D5 前拒绝。不能把「历史中存在 merge」一律当错误（§8.1、§7.3.2） | 1、2、4、9 |
| 73 | **发布的基线必须使用核对过的同一 TREE**：在 D6 后引入服务器新文件，断言 D7a 的 TREE 与 D8 新基线 tree 相同、对齐索引不改磁盘，分别覆盖快进与 Local-Only rebase。注入旧基线 OID 改变时 CAS 拒绝覆盖；分别在 D8b 前后、D8d 切分支前后与冲突中中断重开，断言只续对应阶段、不重做 SVN commit、不用 D6 / D8c 覆盖冲突；S7 / 恢复⑥同样核对后不重新录另一棵树（§6.3.1、§7.6、§8.2.2） | 1、2、3、5、9 |

| 74 | **脏工作副本上重建基线录的是服务器内容**：wc 停在 rN，白名单内分别造出 `M` / `A` / `?` / `D` / `!` / `R` 各一例，重建后逐条断言基线树里的 blob **等于 `svn cat -r BASE` 的字节**（`M`/`D`/`!`/`R`）、`A` 与 `?` 不在树里；并断言**磁盘一个字节没动**（重建前后逐文件哈希相同）与 `git status` 在重建前后不变（§16.1） | 1、4、8、9 |
| 75 | **`~`（obstructed）仍然阻断**：把一个白名单内的 `.cs` 换成同名目录，断言重建与初始化都拒绝、指名该路径、基线指针不动，且**没有**发生任何 `svn revert`（§16.1） | 1、2、4 |
| 76 | **体检新判据两头都站得住**：(a) 不误报——wc 停在 rN 且手上有上面那几种脏时 `D \ S == ∅`，体检通过；(b) 有牙齿——构造「基线声称 r2、磁盘停在 r1」，断言 `svn status` 为空而 `D \ S == {A.cs}`，体检**阻断**。两例必须在同一个测试文件里成对出现，只写 (a) 属虚假信心（§6.8.7、`AGENTS.md` §13 第 9 条） | 1、9 |
| 77 | **`S` 不能取超集**：人为把 `S` 放大成「白名单内全部文件」，断言测试 76(b) 由通过变为**失败**——这是判据被削弱时唯一会响的警报（§16.1「S 必须恰好」） | 9 |
| 78 | **`-r BASE` 而不是 `-r N`**：把仓库目录改名模拟离线，断言重建仍然成功；再断言实现里没有出现 `-r <数字>` 形式的 `svn cat`（§16.2） | 1、3 |
| 79 | **字节不过 UTF-8 转换**：白名单内放一个 GBK 编码的 `.cs`（含非 UTF-8 字节序列），本地改脏后重建，断言基线里的 blob 与 `svn cat` 的原始字节**逐字节相同**、不含 U+FFFD（§16.2 末） | 1、9、11 |
| 80 | **mixed-revision 仍然硬拦**：P2 不单值时重建与初始化都拒绝，且错误信息指向「先跑一次完整 svn update」而非 `~` 那条（§16.2 的授权条件） | 1、4 |
| 81 | **重建窗口期的外部改动只会误拦**：分别在 R2a 之后注入「改脏一个干净文件」与「外部 commit 一个文件」，断言两种都中止且基线指针不动；再注入「还原一个脏文件」，断言正常成功且树内容仍是服务器内容（§16.1 竞态表） | 1、2、5 |
| 82 | **初始化后 `work` 分支的 `git status` 恰好是原有的本地 SVN 修改**：带 `M` / `?` / `D` 各一例初始化，断言切到 `work` 后 git 分别显示为已修改 / 未跟踪 / 已删除，且内容与初始化前逐字节一致（§6.5.1 末） | 1、8 |

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
   在这些路径之外的会停在旧 revision，要等下一次 `sync_in` 才跟上。D6 缩小了更新范围，
   但白名单目录内仍可能有被扩展名排除的资产，其本地冲突风险见第 25 条
9. **白名单外文件的冲突客户端修不了**：git 不管这些文件，`svn update` 在它们上面
   产生的冲突只能回 TortoiseSVN 处理。客户端负责如实报告，不负责修复
10. **本地补丁与待交付改动挨得太近时交付会被拦下**（§8.2.3 T2 实测）。这是刻意的：
    中止的代价是零（还没碰 SVN），猜错的代价是把调试代码送上服务器。缓解手段是把
    Local-Only commit 移到分支顶端，那条路径结构上不可能冲突
11. **用 TortoiseSVN 直接提交会把本地补丁带上去**。`ignore-on-commit` changelist 原本
    还能挡一下，本设计挡不住——补丁就在磁盘上，外部工具看到的就是改过的文件。与第 2 条
    「外部工具无法阻止」同源；交付走客户端是这套方案的前提
12. **本期不自动重放区间内的 merge commit**（§7.3.2）。普通 rebase 可能静默丢掉 merge
    独有的改动，端点 `merge-tree` 也不能作为逐提交 rebase 的正确结果断言。因此
    `sync_in` / 恢复在 stash 前拒绝此类分支；有 Local-Only 的交付在 D3 / D5 前拒绝
    需要重放 merge 的历史。用户须保留原分支并人工整理后再继续；客户端不自动改写。
    已包含在目标基线或交付树中的 merge 不受这条门槛影响，无 Local-Only 的快路径
    仍可直接交付 merge 的完整结果，不声称所有工作分支都必须压平
13. **待交付历史含 merge commit 且需走慢路径时会被拒绝**（§8.1 #6）。`cherry-pick`
    对 merge 没有安全的自动语义（§8.2.3 T4 / T5 实测）。缓解手段同第 10 条：把
    Local-Only commit 整理成分支顶端的连续单父后缀，且 merge 已在交付树内时可走快路径。
    整理由用户在保留原分支后进行，不能让客户端自动线性化来绕过门槛
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
18. **Phase 1 的「重建基线」不会推进工作分支**。用户下次需自己同步工作分支；准备
    rebase 前同样要检查 §7.3.2 的待重放区间，含 merge 时先保留原历史并人工整理。
    Phase 2 的 §9.4 才做完整编排
19. **baseline 上的 `git-svn-id` trailer 不是 `git svn` 生成的**（§6.4.1）。用这个格式
    是为了白送 UUID 记账和工具互操作，但 `.git/svn/` 里没有对应的 `rev_map`。用户若
    自己在本仓库跑 `git svn init`，`git svn` 会按这些 trailer 解释历史，产生令人困惑
    的行为。不损坏数据，但客户端不支持这种混用，也不检测它
20. **基线体检只证明「磁盘 == 服务器」，不证明「git 跟踪的文件集合是对的」**
    （§6.8.3），且只比较 W-file 的文件内容。文件集合另由 §6.3.1 在**每次录制**的
    临时索引上核对，初始化检查不能代替后续检查；两条验证不可互相冒充
21. **体检需要联网**。离线时基线只能停在「未经体检」状态（§6.8.6）。这是刻意的：
    把「没能验证」当成「验证通过」等于这条检查根本不存在
22. **白名单不能跨仓库**（§6.5 P4）。`svn commit` 对跨仓库的 target 列表直接拒绝
    （E200007，§6.6.5 实测），所以一次交付覆盖不了分属两个仓库的代码。若项目把代码
    拆在两个仓库里（例如 `Code/` 与 `Script/` 是指向不同仓库的 external），本方案
    只能覆盖其中一个，另一个继续走 TortoiseSVN——或者为它单独再开一套（两个独立的
    `.git` + 两份配置），但那超出本设计范围
23. **白名单所在的 external 被 `-r` 钉死时，客户端挡不住外部工具的递归 update**
    （§6.5.3）。⚠️ 该情形对当前公司项目**不触发**（§16.0 已确认代码 external 未被
    钉住），这一条是防御性路径的已知边界，不是当前的实际限制。客户端自己的 update 会绕开 peg（显式指名目录即可，实测不认 peg），
    但 TortoiseSVN 的「SVN Update」在项目根跑递归 update 会把代码退回 peg，且工作
    副本看起来干净。**可检测**（`nodeRevs` 掉回 peg，且 git 侧磁盘与基线不符）、
    **可一键修复**（`svn update <目录>`）、内容**从未真正丢失**。注意这个风险在没有
    本客户端时同样存在——它是团队 externals 定义的固有行为
24. **`.git` 放项目根，而白名单文件可能在嵌套工作副本里**（§1.1.1）。git 不在乎这件事，
    但这意味着 §12.3 那三层「防 `.git` 被误提交」的防护要挂在**项目根**那个工作副本上，
    而失同步检测挂在**白名单所属**的工作副本上——两者不是同一个 WC，实现时不要合并
25. **白名单目录里那些 git 管不到的文件，客户端保护不了**（§6.3.2 的 W-path 减 W-file
    差集）。D6 与 §9.4 ⑤b 为了归一目录节点 revision 使用 W-path update，会递归走到
    它们头上；这不允许恢复⑤也用目录 target revert。已做的是把差集本地修改在
    **动手之前**列进确认清单（§8.1 #2、§9.4 前置）；消不掉的是 D3.5 到 D6
    之间那个窄竞态——同事恰好在此期间提交了其中某个文件的冲突改动时，`.mine` / `.rN`
    会出现在 D5 之后，客户端只能如实报告，引导去 TortoiseSVN。缩小这个差集的办法在
    用户手里：把 `extensions` 配得更准，或者不要把非代码文件放进白名单目录
26. **新增的 `svn:externals` 不在检测范围内。** P4 / P5 与 §6.5.5 的枚举都只在
    **初始化时**跑一次。此后团队往白名单子树里新挂一条跨仓库 external，客户端不会
    主动发现；表现是 git 开始跟踪一批不受 `nodeRevs` 监控的文件，直到某次交付撞上
    E200007 才暴露。§16.0 建议的那条只读核对命令要在「团队改过 externals 之后」
    重跑一次，这件事目前靠人记得，没有自动化
27. **线性 rebase 按逐提交语义执行，不保证等于端点 merge 的整树结果**（§7.3.2）。
    已应用的补丁可以正常丢弃，普通冲突由用户解决后继续；不能把与 `merge-tree` 的
    差异都当作内容丢失，也不能把含冲突标记的 TREE 当成用户应得到的结果。本设计
    不再依赖 `merge-tree` 的能力探测或「未经校验仍继续」的降级路径；merge 门槛查询
    失败必须停止

## 16. 分期

### 16.0 开工前要在真实项目上核对的事实（**已确认通过**）

§6.5 的 P4（白名单不得跨仓库）与 P5（所在 external 是否被 `-r` 钉死）决定本方案在
公司这个 Unity 工程上能不能用。**2026-09-13 由用户确认，两条都通过。**

| 条目 | 结论 | 依据与强度 |
|---|---|---|
| **P4** 白名单不跨仓库 | ✅ 通过 | 用户**曾把 `code`、`script` 等目录一次提交成功**。这不是印象——跨仓库的 target 列表会被 `svn commit` 以 `E200007` **前置拒绝**（§6.6.5 E5 实测），所以「一次提交成功」本身就构成「同属一个仓库」的证明 |
| **P5** 代码 external 未被钉住 | ✅ 通过 | `code` / `script` 承载日常开发，不可能钉住——钉住会让全组每个人的提交在下一次 update 后从工作副本消失（§6.5.3 测点 C）。与 §6.5.4 末尾那条推断一致 |

**顺带确认了一件事：§6.6.5 的多工作副本推广是必要的，不是过度设计。** `code` 与
`script` 是同一个仓库里的**两条** external，也就是**两个独立工作副本**——「所有白名单
所属 WC 的 `nodeRevs` 并集必须单值」「trailer 的 `<url>` 取 `Repository Root`」这两条
正是为这种形态准备的。

仍然**建议在 Phase 1 开工时机械地跑一次**下面的命令。不是因为上面的结论可疑，而是
成本近乎为零，且能顺带抓到三件本文档没问过的事：`documents` 之类非代码 external 有没有
被钉住、白名单目录底下有没有嵌套的第三方 external、以及此后**新增**的 external。

只读命令：

```powershell
cd <项目根>
svn propget svn:externals -R .
foreach ($d in 'Assets','Code','Script','Documents') {   # 换成实际的顶层目录名
  "--- $d"; svn info $d | Select-String 'Repository UUID|^URL|^Revision'
}
```

判读：

| 观察 | 含义 |
|---|---|
| 代码相关目录的 `Repository UUID` **全部相同** | P4 通过，方案可用；记账单位按 §6.6.5 取那个 WC |
| 代码分属**两个**仓库（例如 `Code/` 与 `Script/` 的 UUID 不同） | P4 阻断。本方案只能覆盖其中一个，另一个继续走 TortoiseSVN（§15 第 22 条） |
| `svn:externals` 里出现 `-r N` 或 `URL@N` 且命中代码目录 | 进入 pinned 模式（§6.5.3），**不阻断**；update 改走显式指名的方式，并向用户说明外部递归 update 的残余风险（§15 第 23 条） |

判读表保留在这里，是为了让**下一个人**（或换了项目之后）知道该看什么、以及看到
不同结果时该走哪条路——不是因为当前项目还悬着。

每期独立 spec → plan → PR。

| 期 | 内容 | 验收 |
|---|---|---|
| **Phase 0** | `trait Vcs` + `GitBackend`（现有代码搬家，**行为零变化**） | 现有全部测试绿 + 手工 E2E 与改动前无差异 |
| **Phase 1** | 仓库识别、配置、exclude 生成、初始化向导、`SvnClient`（info / status / log 的 XML 解析）、状态判定与失同步检测（**只报警**）、**「重建基线」最小手段**、**基线体检（§6.8）**、**纳入集合核对（§6.3.1）**、**P4 的嵌套 WC 枚举（§6.5.5）**、混合图只读 | 能打开 SVN 仓库，本地 git 的暂存/分支/stash 可用，看得到 SVN 历史。**还不能 update / deliver**；基线会因外部 TortoiseSVN 操作而失同步，此期只报警 + 提供「重建基线」 |
| **Phase 2** | `sync_in` 完整流程 + 全部失败路径 + 中断持久化 + 冲突解决 UI + **§9.4 基线恢复向导**（仅还原受保护 W-file、⑤b 前确认差集修改并归一 revision）+ **stash 前的 merge 门槛（§7.3.2）** + **S7 / 恢复⑥候选 TREE 核对（§6.3.1）** | 能更新 |
| **Phase 3** | `sync_out` + 确认关口 + 精确回滚 + 传输中断查证（按 `Repository Root` 查，§8.6.2）+ **本地补丁的摘除与回落及提前检查重放区间**（§8.2.3、§7.3.2）+ **基线祖先前置**（§8.1.2）+ **`newParentDirs` 进 target**（§8.2.6）+ **D7a 核对同一 TREE、D8 按 journal 发布与恢复** | 能交付，闭环 |
| **Phase 4** | 白名单外改动视图与 revert、`.git` 误提交三层防护、svn 节点 diff | 补齐日常缺口 |

**Phase 0 单独拆出是有意的。** 它是纯重构、零行为变化，可独立验证，风险最低；
混进 Phase 1 的话，一旦现有 Git 模式出问题，就分不清是重构引入的还是新功能引入的。
对齐 `AGENTS.md` §1.2「不要把无关重构和业务修改混在一起」。

**Phase 1 的「重建基线」是这一期自洽的前提。** Phase 1 没有 `sync_in` / `sync_out`，
用户必须继续用 TortoiseSVN 更新和交付；而任何一次外部 update / commit 都会推进
`nodeRevs`，让 `svn/base` 立刻失同步（§6.6.2）。此时若只有红色横幅、没有任何出口，
「相对上次更新改了什么」这个 §6.4 的核心收益就永久失效了。

「重建基线」的关键是**不要碰工作目录**。要录的是**服务器在 rN 的内容**——不是磁盘
内容。工作副本干净时这两者恰好相同，不干净时不同，而不干净才是常态（§16.1）：

```text
R1  校验：nodeRevs 单值 {N}（§6.6.2）、不在 svn update / merge 中途
      白名单内**允许**存在 M / A / D / R / C / ! / ? ——那是程序员的日常
      只有 `~`（obstructed）阻断：svn 自己都说不清那个位置上是什么，取不到 BASE
      记录不可变 OLD_BASE_OID；全程持锁，不允许覆盖尚未结束的同步 journal
R2a 读一次 svn status --no-ignore --ignore-externals，得到白名单内的本地脏清单，
    按状态分成三类：
      取回 BASE  = { M, R, C, D, ! }   磁盘上的不是服务器内容（或根本没有）
      从树里剔除 = { A, ? }            服务器 rN 上根本没有它们
      原样保留   = 其余（干净文件）     磁盘就是服务器内容
R2b 用一个**临时索引**把磁盘内容录成 tree，全程不碰真实 index、不碰 HEAD：
      GIT_INDEX_FILE=<tmp> git read-tree --empty
      GIT_INDEX_FILE=<tmp> git add -A -- <W-path>
    再按 R2a 的分类逐条改写，把这棵「磁盘树」变成「服务器 rN 的树」：
      取回 BASE：svn cat -r BASE -- <path>@  →  git hash-object -w --stdin  → OID
                 GIT_INDEX_FILE=<tmp> git update-index --add --cacheinfo <mode>,<OID>,<path>
                 <mode> 取该路径在临时索引里已有的 mode，不在索引里（D / !）则用 100644
      剔除    ：GIT_INDEX_FILE=<tmp> git update-index --force-remove -- <path>
      ⚠️ **必须写 `-r BASE`，不能写 `-r N`**，理由见 §16.2
      ⚠️ cat 的结果必须按**字节**接收，不能过 UTF-8 转换，见 §16.2 末
R2c 核对 git ls-files 集合 == **服务器 rN 的** W-file 集合（§6.3.1），漏收 / 多收均停止
      该集合由本地推导，不走网络：磁盘 W-file 集合 − { A, ? } + { D, ! }
      GIT_INDEX_FILE=<tmp> git write-tree                → TREE
R3  git commit-tree TREE -p OLD_BASE_OID
      -m "svn: rN … git-svn-id: <Repository Root>@N <uuid>" → NEWBASE
R3b 基线体检（§6.8）                                    ← 判据用 §6.8.7 的 D \ S == ∅，
                                                          S 取 R2a 那份清单；
                                                          **不通过就中止**，连指针
                                                          都还没动，代价为零
R3c 复查 nodeRevs 仍是 {N}（纯本地，不走网络）           ← 防 R2a 到此刻之间有人在
                                                          TortoiseSVN 里提交，把 BASE
                                                          推到了 N+1，见 §16.1 末
R4  git update-ref refs/heads/svn/base NEWBASE OLD_BASE_OID ← 只动指针；旧 OID 变化则拒绝
R5  无论成功或失败都清理本次创建的临时索引文件，释放锁并恢复 watcher
```

**为什么不能照抄 §9.4 的 `git checkout svn/base`。** §9.4 之所以先 checkout 旧基线是
因为它后面紧跟着 `svn revert`——由 revert 把服务器内容重新铺回磁盘，checkout 只是把
git 侧的状态清空。极简版没有 revert，此时先 checkout 会把磁盘**换回旧基线内容**，随后
录下来的就是旧内容却标着新 revision，正好造出这一节要防的那种撒谎基线。这条捷径删错了
步骤：删掉的是让序列成立的那一步。

R2b 的取回 BASE **不是** revert 的变体，这一点必须分清：`svn revert` 会**写磁盘**，把
用户的改动抹掉；`svn cat` 只往 stdout 吐字节，磁盘一个字节不动（2026-09-16 实测，B1）。
整条 R1–R5 对工作目录仍是只读的，这正是「重建基线」这一期能安全存在的前提。

顺带两个 checkout 自带的麻烦也一并没了：工作分支上若有未提交改动，`git checkout` 会
直接失败；而 R1–R5 全程不动 HEAD，也就不存在「流程结束忘了切回原分支」。

代价是工作分支不会自动跟进新基线。用户下次自己同步分支；若选择 rebase，应先检查
§7.3.2 的待重放区间，含 merge 时保留原分支并人工整理，不能给出无条件裸 rebase 的引导。

**Phase 1 结束时已是一个有意义的交付点**——本地 git 的暂存区、分支、stash 全部可用，
只是 update / deliver 还得回 TortoiseSVN 手工做。

本地补丁（§6.7）横跨两期：**标记**属于 Phase 1（提交框的勾选框 + 图上的角标，本质
只是一条 message trailer，此时还没有交付流程会读它），**摘除与回落**属于 Phase 3。
Phase 1 到 Phase 3 之间用户手工走 TortoiseSVN 交付时，本地补丁没有任何保护——这段
窗口期必须在 UI 上写明，不能让用户以为勾了「仅本地」就安全了。

### 16.1 重建基线为什么不再要求工作副本干净

初版的 R1 要求「白名单内 `svn status` 干净」。**这条门槛让重建基线在真实开发节奏下
永远点不动**，2026-09-16 实测确认：

```text
init → 改一个白名单内的 .cs → git commit  →  git status 空，svn status 仍是 `M`
                              svn update  →  svn status 依然 `M`
                              点「重建基线」→  svn_preflight_dirty
```

根因是 **git 与 SVN 共用同一个工作目录**。`git commit` 只动 git 的索引与对象库，
磁盘内容照旧，SVN 那边看到的还是「相对 BASE 改过」。所以只要用户**干过活**，白名单
就永远是脏的；而失同步横幅偏偏在推荐「重建基线」——一条推荐用户去点、却永远点不动的
按钮，是死胡同，不是保护。

用户的原话值得原样记下来：「我一个写代码的程序员，怎么可能工作区是 clean 的呢，
99% 的情况下都是有本地修改的啊」。这条质疑成立。

**但直接放行是更糟的选择。** 那条门槛不是洁癖，它是初版录制方式的**必要条件**：初版
R2 把**磁盘**录成 tree，而 trailer 声称「这棵树 == 服务器 rN」。只有白名单干净时这句
声称才成立。放行而不改录制方式，会把用户自己的改动算进基线——到了 Phase 3 交付时，
这些改动因为「和基线一样」而不在交付集合里，于是**静默消失**。那比拒绝糟得多。

所以真正的修法是**换掉录制来源**，而不是拆掉门槛：把脏文件那几条换成服务器 rN 的内容
（R2a–R2c）。干净文件仍从磁盘录——它们本来就等于服务器内容，绕道取一遍只是白花时间。

**为什么这样录出来的树真的等于服务器 rN。** 记 S = R2b 实际动过的那批路径（取回 BASE
的 ∪ 剔除的），D = 体检报出的「磁盘与服务器 rN 有内容差异」的 W-file 集合：

- 对 `f ∉ S`：树里是磁盘内容。体检判据 `D \ S == ∅` 意味着 `D ⊆ S`，于是 `f ∉ D`，
  即磁盘内容 == 服务器内容 ✓
- 对 `f ∈ S`：树里是 `svn cat -r BASE` 的字节，按构造就是服务器内容；被剔除的那些在
  服务器 rN 上本来就不存在，树里没有它们同样正确 ✓
- 文件**集合**对不对由 R2c 单独管（§6.3.1），不靠上面两条

两条合起来：树 == 服务器 rN。这就是 §6.8.7 那条新判据存在的全部理由——它不是一条
额外的保险，它是**这套录制方式成立所依赖的那个前提本身**。

**S 必须恰好是「实际动过的路径」，不能是它的超集。** S 越大，`D \ S` 越小，判据越弱。
所以 S 只能取 R2a 那份清单在 R2b 里真正生效的那部分，不能顺手拿一个「反正都算脏」的
更宽的集合去凑。

**窗口期的竞态。** R2a 读 S 与 R3b 跑体检之间隔着若干次子进程调用，期间用户可能在
外部编辑器或 TortoiseSVN 里继续动。逐种数一遍，结论是**只会误拦，不会静默录错**：

| 窗口内发生了什么 | 结果 |
|---|---|
| 某个干净文件被改脏 | 它进 D、不进 S → `D \ S ≠ ∅` → **中止**（误拦，安全） |
| 某个脏文件被还原干净 | 树里已是服务器内容，本来就对；它也不再进 D ✓ |
| 用户在 TortoiseSVN 里**提交**了某个文件 | 该文件 BASE 被推到 N+1，磁盘内容 ≠ 服务器 rN 且 svn 认为它干净 → 它进 D、不进 S → **中止**；R3c 复查 `nodeRevs` 再兜一层 |
| 用户在 TortoiseSVN 里 **update** 了 | `nodeRevs` 不再是 {N} → R3c **中止** |

误拦的代价是用户重点一次按钮；静默录错的代价是 Phase 3 丢改动。方向选对了。

**同样的墙也堵在初始化向导上**，所以 §6.5 的 P1 一并按同样的方式放松：一个刚装上客户端
的程序员手上同样有本地修改，让他先去 TortoiseSVN 把活清干净才准建库，和上面那条死胡同
是同一个问题的第一现场。P1 改为只拦 `~` 与「update / merge 中途」，录制走 R2a–R2c 的
同一段实现。

### 16.2 为什么是 `-r BASE` 而不是 `-r N`

两者在联网时取到的字节完全相同（`BASE == N` 由 P2 的 `nodeRevs` 单值保证），但
**离线时只有 `-r BASE` 还活着**。2026-09-16 实测（wc 停在 r1、服务器已到 r2、
A.cs 本地改过、B.cs 本地 `svn delete`）：

| 命令 | 联网 | 仓库移走（模拟离线） |
|---|---|---|
| `svn cat Code/A.cs@`（不带 `-r`） | `a-r1` | `a-r1` ✅ |
| `svn cat -r BASE Code/A.cs@` | `a-r1` | `a-r1` ✅ |
| `svn cat -r 1 Code/A.cs@` | `a-r1` | ❌ `E170013: Unable to connect to a repository` |
| `svn cat -r 2 Code/A.cs@` | `a-r2` | ❌ 同上 |
| `svn cat -r BASE Code/B.cs@`（磁盘上已没有） | `b-r1` | `b-r1` ✅ |

不带 `-r` 时默认就是 BASE，但仍**显式写 `-r BASE`**：默认值是哪一个不该让读代码的人去
猜，而且 `-r N` 和它长得太像，写错一个字就把一条本地操作变成了联网操作。

⚠️ **`-r BASE` 与 `-r N` 的等价性完全依赖 P2。** 一旦允许 mixed-revision 工作副本进入
这条流程，BASE 就不再是单一的 N，取回来的字节会来自若干不同 revision，而 trailer 只写
得下一个数——那正是撒谎基线。P2 在这里不是「顺手做的校验」，是 `-r BASE` 的授权条件。

**字节，不是字符串。** `svn cat` 的 stdout 必须原样按 `Vec<u8>` 接收。现有的
`run_svn` 走 `String::from_utf8_lossy`，那是给 XML 输出准备的；源码文件很可能是 GBK
（本项目面向的是国内 Unity 工程），过一遍 lossy 转换会把每个非 UTF-8 字节换成 U+FFFD，
录进基线的就是一棵被悄悄改坏的树。所以需要一个只返回字节的变体。

已实测的三件相关事实（2026-09-16）：

| # | 问题 | 结果 |
|---|---|---|
| B4 | `svn:eol-style=native` 的文件，磁盘与 `svn cat` 的字节一致吗 | ✅ 一致，都是 CRLF |
| B5 | `svn:keywords` 会展开吗 | ✅ 会，`$Id: Kw.cs 1 … $` 与磁盘逐字节相同 |
| B6 | 磁盘是 LF 的文件经 stdout 出来会被加 CR 吗 | ✅ 不会，4 字节进 4 字节出 |

B5 值得多说一句：`svn cat` 展开的是**该 revision 的**关键字值（revision 号、作者、
日期都取自服务器那一版），这正是基线该记的东西；若它不展开，基线里的 `$Id$` 会和同目录
其他干净文件形态不同，工作分支上凭空多出一堆假 diff。

#### 16.2.1 `hash-object` 必须带 `--path`，否则 `core.autocrlf` 会让两条录制路径分叉

上面 B4–B6 比的都是「`svn cat` 的字节 vs 磁盘的字节」，**漏了一环**：那些字节最终要
变成 git 的 blob，而两条录制路径对此的处理不一样。

```text
干净文件： git add -A            → 跑过滤器（autocrlf、.gitattributes 的 text/eol）
脏文件：   hash-object --stdin   → **默认一个过滤器都不跑**
```

2026-09-17 实测（本机 `core.autocrlf=true`，多数 Windows 开发机的默认状态），
一个设了 `svn:eol-style=native` 的文件：

| | 字节 |
|---|---|
| 磁盘 | `"n1\r\n"` CRLF |
| `git add -A` 进 blob | `"n1\n"` LF —— 归一过 |
| `hash-object --stdin` | `"n1\r\n"` CRLF —— 没归一 |

后果是**这个文件在基线里长什么样，取决于它当时脏不脏**：在脏工作副本上重建一次，
行尾整个翻一遍，工作分支上凭空出现「每一行都改了」的假 diff。

修法是 `git hash-object -w --path <该路径> --stdin`——`--path` 让它按该路径**应当
适用的**属性跑过滤器。对齐的基准必须是 `git add`，因为树里绝大部分内容是它录进去的。

由 `recording_a_file_produces_the_same_blob_whether_it_was_dirty_or_not` 钉住：同一个
文件分别在干净与脏的状态下录一次，blob 必须逐字节相同。拿掉 `--path` 该测试立刻变红
（实测 `left: [110,49,13,10]` vs `right: [110,49,10]`）。

⚠️ **由此引出一条对整份设计成立的限定。** `autocrlf` 打开时，基线树里存的是 LF 归一
后的内容，而服务器上可能是 CRLF。所以 §16.1 那句「树 == 服务器 rN」严格说是「**在
git 自己的 eol 归一之下**相等」。这不是本期引入的——`git add -A` 从 Phase 1b 起就是
这么录的——但既然这里查到了，就写明白：**凡是比较「基线树的字节」与「服务器的字节」
的推理，都要先过一遍这层归一。**本设计里没有这样的比较（体检比的是磁盘与服务器，
§6.8.3），所以结论不受影响。

#### 16.2.2 为什么不能靠 `git stash` 让工作副本变干净

一个自然的替代方案：录基线之前先 `git stash` 把本地改动收起来，工作副本干净了就能照旧
从磁盘录，录完再 `git stash pop`。**实测证明这条路走不通**（2026-09-17，混合仓库
fixture，白名单 `Code/`，四种脏各一例）：

```text
              stash 之前              git stash push -u -- Code/ 之后
  M  Code/A.cs           →      M  Code/A.cs        ← 还在
  A  Code/Added.cs       →      !  Code/Added.cs    ← 变得更糟
  M  Code/Art.psd        →      M  Code/Art.psd     ← 还在
  D  Code/Deep/D.cs      →      D  Code/Deep/D.cs   ← 还在

  git status →  (空)                                ← git 这边确实干净了
```

四条一条都没解决，原因各不相同：

| 条目 | 为什么 stash 清不掉 |
|---|---|
| `M Code/Art.psd` | **不是 W-file，git 根本不跟踪它**。而 P1 检查的是 `svn status` 的全部条目，不按扩展名放行（§6.5 P1） |
| `A Code/Added.cs` | 文件被 `-u` 收走了，但 `A` 这个**登记状态记在 `.svn/` 里**，git 碰不到。结果从「已登记新增」退化成 `!`（已登记但文件没了）——比原来更糟 |
| `D Code/Deep/D.cs` | 同上，删除登记在 `.svn/` 里 |
| `M Code/A.cs` | 最意外的一条：git 确实把它还原成了 HEAD 的内容，但 `core.autocrlf=true` 让还原出来的是 **CRLF**，而 svn 的 pristine 是 LF → svn 照样报 `M`（`svn diff` 打出 `-v1` / `+v1`，内容一样行尾不同） |

根因是一句话：**`git stash` 操作的是 git 的状态，而 P1 检查的是 SVN 的状态，两者共用
一个工作目录但互不相通。** SVN 侧的「登记」（`A` / `D`）压根不在 git 的管辖范围内。

`stash pop` 之后还留下了一条**新的**损坏：`! Code/Deep`（目录丢失）——git 在重放删除时
把空目录一并删了，而 svn 还指望它在。

所以 stash 这条路对**录基线**不成立。它对 **`sync_in`** 是成立的，而且本设计一直就是
这么做的（§7.3 的 S1 / S9 / S10）——那里要清空的本来就是 git 侧的工作树，好让
`svn update` 与随后的 rebase 各自在干净的地基上跑，不是为了骗过 P1。两件事只是看起来像。

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

### 2026-09-25（十三）· 同步锁的存活判据从 pid 探测改为句柄探测

Phase 2b（同步基础设施）施工时改的，依据是 Windows 上的实测，不是纸面推断。
细节与实测表见新增的 §5.1.2，这里只记结论与影响面。

| 原设计（§5.1.1） | 改成 | 为什么 |
|---|---|---|
| 「记录的 pid 已不存在」才允许接管 | 「独占打开成功」才允许接管 | 崩溃安全由操作系统保证；没有 pid 复用导致的永久卡死；不依赖文件内容 |

三条实测结论：

1. **持锁进程被杀之后，锁文件仍然存在**——所以「文件在不在」不能当判据。
2. **但此时独占打开会成功**——句柄由操作系统回收，这才是可靠的判据。
3. **持锁期间用宽容共享模式仍然读得到 pid**，§13 要求的「显示持锁进程的 pid
   与开始时间」不受影响。

施工时的变异测试还暴露出 pid 探测的一条更基本的缺陷：它信的是文件内容。持锁者在
「拿到锁」与「写完元数据」之间有一条缝，元数据也可能写了一半；那两种情形下 pid
读不出来，pid 探测会判成「没人持有」并直接接管一把真有人在用的锁。

另记一条实现顺序要求：释放时必须先关句柄再删文件，否则 `remove_file` 会因共享
冲突失败，每次正常同步都留下残锁，把「上次没有正常收尾」变成一条永远为假的告警。

`pid` / `startedAt` 字段保留，用途从「判据」降为「展示」。非 Windows 平台按
`engine/file_lock.rs` 已有方式降级为**不做互斥**（恒判「没人持有」），**不是**
「无法判定存活」——后者会让残锁永久挡住写操作，理由见 §5.1.2。
（2026-09-25 第二轮独立 review 指出此处原写法与实现方向相反，已按实现更正。）

### 2026-09-21（十二）· 行尾归一化定案：混合仓库关掉 git 的 EOL 转换

Phase 1 全部合入 `master` 之后、Phase 2 开工之前的前置。§7.3.0 从 2026-09-17 起挂着
「尚未验证也未修复 / Phase 2 开工时必须先定」，本轮在临时 fixture 上把三条出路逐条
量完并定案。四支探针，跑完即删，不碰任何真实仓库。

| 问题 | 本轮决策 | 修改位置 |
|---|---|---|
| `autocrlf=true` 下 git 写磁盘会把无 eol-style 的 LF 文件变成 CRLF，最坏让 Phase 3 把纯行尾变化提交进 SVN | 在 `.git/info/attributes` 写 `* -text`，混合仓库一律按字节读写 | §7.3.0、§3 架构图 |
| 备选 B（`.git/config` 设 `core.autocrlf=false`） | **不取**。实测：工作树里一个 `* text=auto` 就能把它覆盖掉（`check-attr` 从 `unset` 变 `auto`）；`$GIT_DIR/info/attributes` 的优先级则压得住 | §7.3.0 对比表 |
| 备选「工作树写 `.gitattributes`」 | **不取**。会进工作目录、会被提交进用户的 SVN 仓库，与 §3 冲突 | §7.3.0 |
| 备选「逐文件读 `svn:eol-style`」 | **不取**。我们要的是字节级相等，`* -text` 免费拿到；逐文件判别最好也只能追平，代价却高得多 | §7.3.0 |
| 关掉转换会不会打破 R2b「`svn cat` 与 `git add` 对齐」 | 不会，而且变成平凡成立。四种组合实测两条路产出同一个 blob | §7.3.0 对齐表 |
| 已存在的基线怎么办 | **必须强制重建**。基线 blob 的形状由录制时的设置决定，旧基线在新设置下会让每个 CRLF 文件整文件变脏 | §7.3.0 迁移一节 |

**两条实测纠正了原记录：**

1. **范围被夸大了。** 原文写「S1 的 stash push 会把白名单内每个 W-file 还原成 CRLF」。
   实测只有**被 git 写过磁盘的那些**才会——stash 只碰进出 stash 的文件，
   `git checkout svn/base` 只碰两棵树有差异的文件。性质不变，范围小于原记录。
2. **迁移故障是潜伏的，这一条原文完全没有。** 翻完设置立刻跑 `git status` 报 **0 个
   脏文件**——git 的 stat cache 认为文件没动过，不去重算 hash。要等某个文件的 mtime
   被碰一下才单独冒出来。**任何「切完看一眼 `git status` 没事就放行」的验收都是假的**，
   判定前必须 `git update-index --really-refresh`，或者干脆只看规则在不在。

本轮只动 spec，未改代码。实现（`init_hybrid` 写规则、打开旧仓库时补规则并要求重建）
与配套的 E2E 路径列入 Phase 2 的 plan。

### 2026-09-16（十一）· 基线改从服务器录，不再要求工作副本干净

**触发这一轮的是一条用户实测反馈，不是 review。** Phase 1c 手工 E2E 走到「重建基线」
时用户报告：点不动，提示 `svn_preflight_dirty`；并质疑「我一个写代码的程序员，怎么可能
工作区是 clean 的呢，99% 的情况下都是有本地修改的啊」。

复现确认质疑成立，而且比反馈里说的更严重——不是「不方便」，是**死胡同**：git 与 SVN
共用同一个工作目录，`git commit` 之后 `svn status` 仍然是 `M`，所以只要用户干过活，
白名单就永远脏，重建基线**永远**点不动；而失同步横幅偏偏一直在推荐它。

| 问题 | 本轮决策 | 修改位置 |
|---|---|---|
| 要求白名单干净才能重建 / 初始化，在真实开发节奏下等于永远不可用 | 换掉**录制来源**而不是拆掉门槛：干净文件仍从磁盘录，脏文件改取 `svn cat -r BASE` 的字节，`A` / `?` 从树里剔除。P1 收窄到只拦 `~` | §6.5 P1、§6.5.1、§16 R1–R5、§16.1 |
| 直接放行会让用户自己的改动被算进基线，Phase 3 交付时静默消失 | 正是不放行的理由，原样记在 §16.1；新做法从构造上不可能把本地改动录进去 | §16.1 |
| 体检的前提「磁盘 == 基线树」在新做法下不成立 | 判据推广为 `D \ S == ∅`——每处与服务器的差异都必须被一条已知本地修改解释。干净时 `S = ∅`，退化成原判据，**是推广不是替换** | §6.8.2、§6.8.7、§13 |
| 期望集合写的是「磁盘 W-file」，脏工作副本上会误判 | 改为「服务器 rN 的 W-file」，由本地推导：磁盘集合 − `{A, ?}` + `{D, !}`，不走网络 | §6.3.1 |
| `-r N` 会让重建变成联网操作 | 改用 `-r BASE`（离线可用，走 `.svn/pristine`）；等价性由 P2 的 `nodeRevs` 单值授权 | §16.2 |
| `run_svn` 的 `from_utf8_lossy` 会把 GBK 源码里的字节换成 U+FFFD | `svn cat` 必须走只返回 `Vec<u8>` 的变体 | §16.2 末；测试 79 |
| R2a 读脏清单与 R3b 体检之间有窗口期 | 逐种数过：只会误拦，不会静默录错；再加 R3c 复查 `nodeRevs` | §16.1 竞态表 |

**八条未知在动 spec 之前全部实测过**（2026-09-16，临时 fixture，一次性探针跑完即删）：

| # | 问题 | 结果 |
|---|---|---|
| B1 | `svn cat` 会不会写磁盘 | ✅ 不写，磁盘一个字节没动 |
| B2 | 已 `svn delete` 的文件还取得到吗 | ✅ 取得到，磁盘上没有照样取 |
| B3 | `svn add` 未提交的怎么判 | ✅ `E195002: has no committed revision`，可明确判别 |
| B4 | `svn:eol-style=native` 的字节一致吗 | ✅ 磁盘与 cat 都是 CRLF |
| B5 | `svn:keywords` 会展开吗 | ✅ 会，且展开的是该 revision 的值 |
| B6 | LF 文件经 stdout 会被加 CR 吗 | ✅ 不会，4 字节进 4 字节出 |
| B7 | `-r BASE` 离线可用吗 | ✅ 可用；`-r N` **不可用**（E170013） |
| B8 | `D \ S == ∅` 会误报吗 / 有牙齿吗 | ✅ 不误报；✅ 对「基线声称 r2、磁盘停在 r1」报警 |

⚠️ **一个负面结果照实记下。** 原本设计来证明判据有牙齿的那条测试（同长度篡改 + 还原
mtime）**没有证成**：`svn status` 与 `svn diff` 双双被 svn 的「大小 + mtime」捷径绕过。
这是**既有**盲区——今天的 §6.8 体检跑的就是同一条 `svn diff`——不是本轮引入的，也不
打算在本轮修掉，理由见 §6.8.7 末。

### 2026-09-13（十）· 第五轮 review：修复恢复范围与 rebase 校验，持续核对录制集合

本轮修订只修改设计文档。下表记录当前采用的规则；后面的历史记录保留当时的决策，
其中第四轮引入的 S8b / S9b 和 D8d `merge-tree` 守恒断言已被本轮取代。

| 问题 | 本轮决策 | 修改位置 |
|---|---|---|
| 恢复把 W-path 下所有 `M` 都交给 `svn revert`，覆盖未受 Git 保护的 `.psd` | W-path 只用于扫描与 revision 归一；自动还原限于有恢复状态的 W-file 显式文件清单。⑤b update 前确认差集修改，确认不授权 revert；体检只看 W-file 内容 | §6.3.2、§6.8、§7.2、§9.4–9.5、§13；测试 67 |
| 恢复⑦漏掉 merge 独有内容的保护 | 与 `sync_in` 统一，在写 journal / stash 前检查待重放区间，含 merge 则拒绝；rebase 前按实际目标 OID 再复核 | §7.3.2、§9.4；测试 63、68、72 |
| `merge-tree` 普通冲突退出 1 阻断正常 rebase；冲突标记 TREE 也不能校验用户解决的结果 | 取消整树等值断言。受支持的线性历史直接进入 Git rebase，真实冲突保留 journal / stash 和未合并索引，由用户解决后继续 | §7.3、§7.3.2、§8.2.2、§13、§15 第 12 / 27 条；测试 69 |
| 即便无冲突，线性 rebase 也可能与端点 merge 不同 | 不把两个算法的不同结果报作内容丢失；用 `0→1→0` 对新基线 `1` 的反例界定逐提交语义 | §7.3.2；测试 71 |
| 只在初始化检查 `.gitignore`，后续录制仍可能漏收代码 | 所有录制入口在临时索引核对磁盘 W-file 与完整实际集合，直接从通过检查的同一 TREE 创建 commit；旧基线 OID 用 CAS 发布。失败保留对应阶段，D5 后绝不重复提交 | §6.3.1、S7、D7a / D8、恢复⑥、Phase 1 R2；测试 70、73 |

**为什么采用前置拒绝 merge。** 这一版不尝试自动推断 merge 独有改动应如何拆成线性
提交，也不把「rebase 退出 0」等同于保护了 merge 内容。只检查即将重放的区间，目标
中已有的 merge 和无需 rebase 的快路径仍可使用。门槛查询失败即停止，不保留旧版
「工具不可用就标记未经校验并继续」的分支。需要整理历史时，先保留原分支，由用户处理。

**本轮实际验证。** 在临时 Git / SVN fixture 中通过 10 项命令级检查，结束后全部清理：

- Git 侧 5 项：更新与恢复的 merge 门槛各自修改前拒绝；线性往返修改 rebase 成功且
  结果为 `0`；普通冲突产生真实 rebase 状态和未合并索引；交付树已含 merge 时，单父
  Local-Only 后缀可重放且 merge 独有文件保留。
- SVN / Git 侧 3 项：在 **wc1 自己局部提交**制造 mixed-revision，按过滤后的文件清单
  恢复并归一 revision，保留排除的 `.psd` 本地字节；后续拉到 `.gitignore` 与新 `.cs`
  时核对阻断、不发布漏收基线；修正规则后重新核对可继续发布。
- D8 发布 2 项：分别覆盖普通分支快进与 Local-Only rebase。`commit-tree TREE`、CAS
  更新基线、`read-tree TREE`（不带 `-u`）及分支恢复均成功，D6 模拟拉下来的文件保留，
  发布时磁盘字节不变且索引与基线 TREE 一致。

普通冲突的 rebase 退出 1 是预期结果，验证的是冲突现场，没有自动 abort / reset 重试。
这些检查证明了修法所依赖的 CLI 行为；§14.2 新增的是后续实现必须落地的回归用例，
包含 journal 中断与故障注入，**不代表应用层流程或完整测试套件已经实现并通过**。

### 2026-09-13（九）· 第四轮对抗性 review：11 条全部复核成立并修入

外部 review 提了 8 条 P1 + 3 条 P2。**逐条在临时 fixture 上实测复核，11 条全部成立，
没有一条是误报**，全部修入。实测环境：SlikSvn 1.14.2 + `svnadmin create` 临时仓库、
git 2.51.2 临时仓库，跑完即删。

| # | 问题 | 实测证据 | 修在哪 |
|---|---|---|---|
| P1-1 | §4.4 要求 `svn info` 带 `--ignore-externals`，而该子命令**拒绝**这个参数 | `Subcommand 'info' doesn't accept option '--ignore-externals'`，exit 1 | §4.4 改成按子命令区分的表；`info` 默认就不遍历 externals，本来也不需要。§6.6.4 / §6.8.2 的重述同步修正 |
| P1-2 | 从旧基线分出的另一个分支交付时，D1 的**两点** diff 把别人刚交付的文件算成删除并提交回服务器 | `git diff svn/base..B` → `D ANew.cs`；三点 diff 只有 `A BOnly.cs` | **新增 §8.1 #0**（`merge-base --is-ancestor`）与 §8.1.2。附带发现 D8a 会把基线**倒退**，而 D8d 的 `--ff-only` 此时反而成立、抓不到 |
| P1-3 | §8.2.1 声称 D6 「范围限定在 git 管得住的文件上」——把 W-path 当成了 W-file | `svn update -- Code` 对被扩展名排除的 `.psd` 产出 `G`；重叠改动产出 `C` + `.mine`/`.r5`/`.r6` | **新增 §6.3.2** 把两种粒度定死；§8.2.1 重写论证并写出残余风险；§8.1 #2 补 W-path 侧的确认 |
| P1-4 | §15 第 12 条声称压平 merge「不丢改动」 | evil merge 的 `evil.txt` 被 `rebase --empty=drop` **无冲突、无告警**吃掉 | **新增 §7.3.2** 的内容守恒断言（`merge-tree --write-tree` 当参考树），S9b / D8d 各挂一道；§15 第 12 条改写 |
| P1-5 | D5 的 target 只有文件，`svn add --parents` 带出的父目录没进去 | `svn: E200009: '…Nested' is not known to exist in the repository and is not part of the commit, yet its child '…New.cs' is part of the commit` | **新增 §8.2.6**：D4 记 `newParentDirs`，D5 带上它并加 `--depth=empty`（实测不会顺带提交同目录下别的登记） |
| P1-6 | §9.4 ⑥ 假定重读的 `nodeRevs` 是单值，而恢复流程的**主场景**本身就是 mixed-revision | 局部提交后 `{1,3}`；`svn revert -R` 后**仍是** `{1,3}`；只有 `svn update` 收敛到 `{3}` | **新增 ⑤b 归一步骤 + §9.4.2**；仍多值则停下报错，绝不硬造基线 |
| P1-7 | P4 只查白名单根，白名单子树内的跨仓库嵌套 external 整个漏过 | `svn info Code` → repo1 的 uuid（P4 通过）；`svn info Code/Vendor` → repo2 的 uuid；`status --ignore-externals` 输出 `X Code\Vendor`，其内容既不在 `nodeRevs` 里、又被 git 跟踪 | **新增 §6.5.5**，P4 改成枚举白名单递归覆盖到的**全部** WC |
| P1-8 | §8.6.2 的断网查证绑在 `svnUrl`（工作副本 URL）上，而代码分布在多条 external | 只碰 `Script/` 的提交：`svn log <Root>` 命中 opid 1 次，`svn log <Root>/Code` 命中 **0** 次 | §8.6.2 改用 `Repository Root`；§6.2.1 补一张「哪个用途用哪个 URL」的表 |
| P2-9 | exclude 的递归再入规则会把 external 根里的 `.svn` 一起纳入 | `extensions: ["*"]`（§6.3 明文推荐的写法）下 `git ls-files` 含 `Code/.svn/wc.db` 与 `pristine/**/*.svn-base` | §6.3 的生成规则末尾**必须**有 `.svn/` 兜底行（最后一条匹配的模式说了算）。后果不只是恢复旧数据，是在活着的工作副本底下换掉 `wc.db` |
| P2-10 | 工作目录里的 `.gitignore` 优先级高于 `.git/info/exclude`，§3 那句「不用它」只是声明 | `Code/.gitignore` 一行 `*.cs` → `git ls-files` **空**，`check-ignore -v` 指向它 | **新增 §6.3.1**：初始化加一道「纳入集合核对」（行为断言，顺带覆盖 `core.excludesFile` 与生成器 bug） |
| P2-11 | 测试 13 名为「mixed revision 检测」，造的却是「服务器有新提交」 | wc2 提交后 wc1 的 `svn status -v` 全部仍是 revision 1，`nodeRevs == baselineRev`，正确实现**本就不该报** | 测试 13 改成「在 wc1 自己身上局部提交」，并**新增 13b 作为对照组**（`AGENTS.md` §8 测试形态一致性） |

同时新增测试 57–66 覆盖以上每一条，多数带**反向断言**（旧写法必须放行 / 新检查必须
不误报）——只测「改完之后对」证明不了这些门禁是必需的。

两条需要留意的**修法边界**，不要照 review 的字面理解：

- **P1-3 不能靠「D6 改成按文件 target 更新」解决。** 只更新交付过的文件会让白名单里
  其余节点停在旧 revision，`nodeRevs` 立刻多值、失同步误报。W-path 粒度的 update 在
  结构上是必需的，所以修法落在**前置确认**，残余竞态如实写进 §15 第 25 条
- **P1-4 的触发面比字面窄。** 冲突解决型 merge **不**会静默丢（线性重放会再撞一次
  同样的冲突），真正会丢的只有 evil merge。§15 第 12 条按这个精度改，不要写成
  「所有 merge 都危险」

### 2026-09-13（八）· 前置可行性确认，并回退一次过度概括

用户回答了 §16.0 挂着的两个问题，两条都通过；同时这两个答案暴露出上一轮（七）的一个
过度概括，一并回退。

**① P4（白名单不跨仓库）通过——而且依据比预期的强**

用户说「没有跨仓库的，我之前试过同时提交 `code` `script` 等目录」。这不是印象，是
**实证**：跨仓库的 target 列表会被 `svn commit` 以 `E200007` **前置拒绝**（§6.6.5 E5
实测），所以「一次提交成功」本身就证明了它们同属一个仓库。

顺带确认 §6.6.5 的多工作副本推广是**必要的**：`code` 与 `script` 是同一仓库里的
**两条** external = **两个独立工作副本**，「`nodeRevs` 并集单值」与「trailer 取
`Repository Root`」正是为这种形态准备的。

**② P5（代码 external 未被钉住）通过**

用户的理由与 §6.5.4 末尾那条推断一字不差：开发时要不停改代码，钉住就意味着全组每个人
的提交在下一次 update 后从工作副本消失。pinned 模式因此降级为**防御性路径**——检测和
诊断保留，正常流程一步都不走它。

**③ 回退（七）的一个过度概括：S5 不该无条件改成两步**

上一轮为了支持 pinned，把 §7.3 S5 的 update **无条件**改成了「根 `--ignore-externals`
+ 逐个显式更新 external」。这是错的，而且错法很典型——**为一个边缘场景改掉了主路径**。

代价不是复杂度，是**引入了一个原本不存在的静默失败面**：两步形式要求客户端自己枚举
所有 external，一旦漏掉一条（嵌套的、新增的、解析没覆盖到的写法），那条 external 就
**永远静默地不再更新**，没有报错，只是美术停在旧 revision，几周后才被发现。

而项目根的递归 `svn update` **在结构上不可能有这个 bug**——externals 的枚举是 SVN 自己
做的，客户端一行都不参与。所以 S5 回退为默认递归，两步形式收进 pinned 模式专用
（§6.5.4）。D6 不受影响，它本来就是显式 target，不依赖枚举。

这一条与（四）（五）（六）是同一类教训的第三次出现：**为了修一个洞而改动主路径时，
要先问「这个改动自己会不会开一个新洞」。** 前两次是跨节复用逻辑没核对前提，这次是
把特例的做法提升成了通则。

| 改动点 | 内容 |
|---|---|
| §7.3 S5 | 回退为项目根递归 `svn update`；两步形式改为「仅当检测到 pinned 时」 |
| §6.5.3 | 第 1 条标注为 pinned 模式专属；末尾补用户确认（代码 external 未被钉住），并把 pinned 定位为防御性路径 |
| §6.5.4（新增） | 为什么两步形式不做默认：静默失败面的论证 + 两种场景的分工表 + D6 不受影响 |
| §16.0 | 从「尚未核对」改为「已确认通过」，记录两条结论各自的依据与强度；保留判读表与开工时的机械确认，说明保留理由 |
| §15 第 23 条 | 标注该情形对当前项目不触发，是防御性路径的边界而非实际限制 |
| 文档头 | 去掉「有硬阻断尚未核对」的警示 |

### 2026-09-13（七）· P5 从硬阻断降级为 pinned 模式

用户否掉了上一轮 P5 的处理方式：**「让团队改 `svn:externals` 定义」这条路走不通。**

这个否定是对的，而且指向一个更一般的毛病：**上一轮把「客户端做不到」直接翻译成了
「让组织去改」**。那不是一个客户端能开的处方。正确的问法是「在不改变任何外部约定的
前提下，客户端自己能做到什么」——一问就发现能做的比想象中多。

**实测**（临时 SVN fixture，`externals = -r2 ^/code-trunk code`）：

| # | 操作 | 结果 |
|---|---|---|
| A | 从父目录 `svn update code`（**显式指名**目录） | → HEAD，**不认 peg** ✅ |
| B | `svn update --ignore-externals <根>` + `svn update art` | → 选择性更新成立，被钉的 `code` 不受影响 ✅ |
| C | 提交进被钉的 external 后，在根跑**递归** update | → 退回 peg，`svn status` 仍报干净 ❌ |
| D | 有**未提交**改动时跑 C | → 产生冲突标记，**不是**静默丢失 |

**A 是关键**：peg 只在「父级递归处理 externals 定义」时生效。显式指名目录去 update
就绕过去了。于是 pinned 只需要改三件事——update 拆成「根 `--ignore-externals` + 逐个
显式更新 external」、把「`nodeRevs` 掉回 peg」识别成一个**具体成因**并给一键修复、
以及把残余风险如实写在 UI 上。**对用户可见的行为不变，美术照常更新。**

残余风险是外部工具在项目根跑递归 update（测点 C）。可接受的三条理由：这个风险**在
没有本客户端时同样存在**；**双通道可检测**（SVN 侧 `nodeRevs` 掉回 peg，git 侧磁盘与
基线不符）；**一条命令可修复**，内容从未真正丢失（在服务器上，也在 git 对象库里）。

顺带记一条判断：**承载活跃开发代码的 external 实际上很难是被钉住的**——那会让全组
每个人提交完代码、一 update 就看到自己的提交从工作副本消失。所以 pinned 模式大概率
只作用在美术 / 文档那类 external 上，而本设计从不往那里写。

测点 D 也单独记：**已提交**与**未提交**两种形态的后果完全不同（静默回退 vs 冲突标记），
实现里不能合并处理。

| 改动点 | 内容 |
|---|---|
| §6.5 P5 | 从「硬阻断 + 要求团队改定义」降级为「检测并进入 pinned 模式」，并写明初版为什么错 |
| §6.5.3（新增） | pinned 模式的完整方案：四组实测、三处改动、残余风险论证 |
| §7.3 S5 / §8.2 D6 | update 改为「根 `--ignore-externals` + 显式指名各 external」；D6 补「白名单在 external 里时 target 必须显式指到该目录」 |
| §6.6.2 | 第二条判据补一个具体成因：`N` 恰等于 peg ⇒ 有人跑了根级递归 update，给一键修复而不是完整恢复流程 |
| §13 | pinned 那行从「阻断」改为「进入 pinned 模式」；新增「代码被外部递归 update 退回 peg」一行 |
| §14.2 | 第 52 / 53 项重写（从「必须被拒」改为「必须能用」）；新增 55、56 |
| §15 | 第 23 条重写：不再是「无缓解手段」，而是「可检测、可一键修复、内容不丢」 |
| §16.0 | 判读表第三行改为「进入 pinned 模式，不阻断」 |

### 2026-09-13（六）· 核心命题被推翻：代码本来就在 external 里

用户指出：**公司的 Unity 工程整个就是用 externals 拼的**——项目根之下 `Assets/`、
`Code/`、`Script/`、`Documents/` 各是一条 external，指向不同位置。

这一条直接打穿了 §1.1 的核心命题（「工作目录**同时是一个** SVN 工作副本」），也当场
作废了上一轮（五）刚加进去的 §6.5 P4「白名单不得落在 externals 内」——**那条规则会把
本方案的主要用例整个挡死**。

连续两轮踩同一个坑，性质值得单独记：上一轮在真实副本上看到 externals，却只把它当成
「需要屏蔽的干扰」（因为那个副本里 externals 确实只放第三方依赖）；**没有问一句「如果
白名单本身就在 external 里会怎样」**。样本对了，从样本推出的结论仍然可能是错的。

**实测**（临时 fixture：项目根 + `code` 同仓库 external + `asset` 跨仓库 external）：

| # | 结论 |
|---|---|
| E1 | external 是**完全独立的工作副本**：自己的 `.svn` / WC root / URL / **UUID** / revision |
| E2 | 项目根 `svn commit`（无 target）**不下沉进 external** |
| E3 | 从项目根显式指定 external 内路径提交 ✅ 成功 |
| E4 | 跨「根 WC + **同仓库** external」多 target 提交 ✅ **单个 revision，原子** |
| E5 | 跨**不同仓库**多 target 提交 ❌ `E200007`，**动手之前就拒绝** |
| E6 | 项目根 `svn update` ✅ 连带更新 externals |
| E7 | 往 `-r` 钉住的 external 提交 → 成功（r8），**下次 update 把内容从磁盘无声抹掉**，`svn status` 报干净 |
| E8 | `svn status --xml` 是扁平单 `<target>`，从条目本身看不出它属于哪个 WC |

**E5 救了 §8.5。** 那一整节的失败处理都建立在「SVN 提交原子」之上。跨仓库不是「拆成
两次提交」而是**直接拒绝**，所以论证不用重写，只需补一条前置约束。

**E7 是本轮最危险的发现**，且**没有缓解手段**：提交真的上了服务器，但工作副本会退回
钉住的 revision，随后 D8c 的 `git add -A` 会把旧内容录成标着新 revision 的基线——
自己造出 §9.2 的基线撒谎，而所有间接判据都显示正常。只能硬阻断。

为了让 P5「不得钉死」可实现，另测了 peg 的全部写法：`-r2 URL LOCAL`、
`-r 2 URL LOCAL`、`URL@2 LOCAL`、`LOCAL -r2 URL` **四种都钉住**，而且新旧两种格式的
**字段顺序相反**，不能按列取字段（§6.5.2）。

**顺带一条诚实记账**：既然代码本来就是独立 external，§2.2 里否决 `git svn` 的那条
核心理由（depth-exclude 会让 `.psd` 从磁盘消失）在这种布局下**不成立**。已在 §2.2
显式标注作废，剩下两条理由（测试成本、TortoiseSVN 兜底）仍然成立。本轮不重估，但
不能装作那条论证还站得住——那正是 D2 犯过的错。

| 改动点 | 内容 |
|---|---|
| §1.1 / §1.1.1（新增） | 核心命题改为「工作目录同时被 SVN 和 git 覆盖」；新增「SVN 侧不一定只有一个工作副本」，点明记账必须落到白名单实际所属的 WC |
| §1.2 | externals 从「非目标」改为「**管理**是非目标，但必须正确处理」 |
| §2.2 | 新增作废声明：depth-exclude 论证在 externals 布局下不成立；列出仅存的两条理由与重估触发点 |
| §6.2.1 | 白名单规则**撤销**「拒绝 externals」，改为「允许，但同仓库 + 不得钉死」 |
| §6.4.1 | trailer 的 `<url>` 改取 `Repository Root`（白名单可能跨同仓库多个 WC），并标注这是与 `git svn` 约定的有意偏离 |
| §6.5 | **P4 重写**（可以在 external 内，但必须同仓库）；**新增 P5**（不得被 `-r` 钉死） |
| §6.5.2（新增） | peg 检测的两条手段：解析四种写法（离线、负责阻断）+ update 后比对 HEAD（在线、负责发现盲区） |
| §6.6.5（新增） | 记账单位规则、E1–E8 实测表、不变量推广为「所有白名单所属 WC 的 `nodeRevs` 并集单值」；澄清 `--ignore-externals` 该在哪一层用 |
| §8.5 | 补原子性论证在多 WC 下依然成立的依据（E4 / E5） |
| §13 | **删除**「白名单落在 externals 内」那行；新增跨仓库、被钉死、行为校验疑似三行 |
| §14.2 | 第 49 项**反转**（从「必须被拒」改为「必须能用」）；新增 51–54 |
| §15 | 第 22 条**重写**；新增 23、24 |
| §16.0（新增） | 开工前必须在真实项目上核对的两条只读命令与判读表，标注**尚未核对** |

### 2026-09-13（五）· 真实远端工作副本复核：externals 会打穿 `nodeRevs`

用户提供了一个真实的 SVN 工作副本做验证：TortoiseSVN 官方源码树
（`svn://svn.code.sf.net/p/tortoisesvn/code/trunk` @ r29825，16945 个文件，922 MB，
UUID `cc8e0003-…`）。**全程只读**——没有在其中创建 `.git`，没有跑任何 `revert` /
`commit` / `update` / `switch`。

在此之前所有实测都跑在自己造的 fixture 上，而 fixture 是**按设计者的预期造出来的**，
它天然不会包含设计者没想到的东西。这一轮的三个发现里，最重的一个正是 fixture 永远
造不出来的。

**① 关掉了两条「未验证」**

| 原标注 | 实测结果 |
|---|---|
| `git svn` 在非 `file://` 下能不能用（§2.2） | ✅ `svn://` 完全正常，`-r HEAD` 浅克隆 11.4s。结论收窄为「**只有 `file://` 不能用**」，生产侧不构成否决理由，测试侧的成本结论不变 |
| §6.4 选的 trailer 格式是不是真的（§6.4.1） | ✅ 真实 `git svn` 产物逐字符一致，UUID 与 `svn info` 相同，`git svn find-rev` 可用 |
| §6.8 体检命令只在 `file://` 上验过 | ✅ 真实服务器上正向 3.1s 空输出、负向 3.3s 报 29 行差异，输出形态与 §6.8.4 记的两个坑完全吻合 |

**② 发现一个阻断级缺陷：externals 污染 `nodeRevs`**（新增 §6.6.4）

```text
svn status -v --xml .                     → wc-status@revision = {29825, 1938156}  ← 2 个值
svn status -v --xml --ignore-externals .  → wc-status@revision = {29825}           ← 1 个值
```

`1938156` 来自 `svn:externals` 指向的 `https://svn.apache.org/repos/asf/...`，**另一个
仓库**。后果不是「显示不好看」，而是：

- §6.6.2 的判据「`nodeRevs` 多值 ⇒ mixed revision」**永久误报**
- §6.5 P2 的初始化前置**永远过不去**，而它给的引导「先跑一次完整 `svn update`」
  **永远无效**——update 多少次，external 的 revision 都不会变成本仓库的 revision

即：**带 externals 的仓库上，这个客户端连初始化都完成不了**，而提示还会把用户引向一个
死路。§1.2 早就把 externals 写成「非目标」，但那句话被当成了「可以不管」——**「不支持」
与「可以无视」是两件事**，物理存在的东西会主动污染判定。

同仓库 external（`^/trunk/ext/zlib`）更隐蔽：revision 在同一量级，靠数值启发式发现
不了；带 `-r` peg 时还会产生一个**稳定且看起来合理**的多值 `nodeRevs`。

**③ 顺带发现两个解析陷阱**（并入 §6.6.4）

- 一个 entry 里有**两个** revision：`<wc-status revision>`（节点基线，要它，实测 1 个
  取值）与 `<commit revision>`（最后修改，实测 **2194** 个取值）。取错 = 永久误报
- `svn status -v --xml` 把每个属性**单独放一行**（CRLF），`<wc-status` 与
  `revision="…"` 不同行。行内正则**不报错、只返回空集**——`nodeRevs` 变成空集合，
  「单值不变量」以一种看起来通过了的方式被绕过。这是本轮踩到的第一个坑，我自己就是
  先写了行内 grep 才发现的

另外确认：`svn diff` **不接受** `--ignore-externals`，但也**不需要**——实测它不递归进
externals（对着含 6 个 external 的 `ext/` 体检，输出 0 行）。所以「统一加
`--ignore-externals`」只约束 `status` / `info` / `update`。
~~（**这句把 `info` 算错了，已于下一轮（九）作废**：`svn info` 与 `diff` 一样拒绝这个
参数，正确的子命令表见 §4.4）~~

| 改动点 | 内容 |
|---|---|
| §1.2 | externals 那条加注「不支持 ≠ 可以无视」，指向 §6.6.4 |
| §2.2 | 实测表补 `svn://` 成功的三行；结论收窄为「只有 `file://` 不通」，并拆成生产侧／测试侧两段；写明本设计不用 `git svn`、§14.1 照旧有效 |
| §4.4 | `SvnClient` 新增两条硬规则：`status` / `info` / `update` 一律带 `--ignore-externals` ~~（**`info` 已于下一轮（九）作废**，该子命令拒绝此参数，见 §4.4 的子命令表）~~；`--xml` 必须用真 XML parser，禁止行内正则 |
| §6.2.1 | 白名单拒绝落在 `svn:externals` 内的路径 |
| §6.4.1 | 补 trailer 格式的真实产物核对 |
| §6.5 | P1 补 `--ignore-externals`；P2 写明读法与「无效引导」；**新增 P4**：白名单不得落在 externals 内 ~~（**已于下一轮（六）作废并重写**，见 §6.5 P4 / P5）~~ |
| §6.6.2 | `nodeRevs` 的定义补全：`--ignore-externals` + 取 `<wc-status revision>` |
| §6.6.4（新增） | 三个读法陷阱的完整论证与实测 |
| §6.8.2 | 补真实服务器上的 R1–R3 实测，含 `svn diff` 与 `--ignore-externals` 的关系 |
| §13 / §14.2 / §15 | 新增失败路径 1 行、测试项 48–50、已知限制 22 |
| §6.6.3 | 去掉一处重复行 |

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

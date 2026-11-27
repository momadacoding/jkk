# 混合仓库的文件历史与 Blame：git 打底、SVN 补全（设计）

日期：2026-11-27　状态：设计，未实现　前置：`docs/superpowers/plans/2026-11-08-file-history-blame.md`

## 1. 背景与目的

混合仓库（SVN 工作副本 + jkk 的 git overlay）里，History 和 Blame 只读 git 历史（`file_history.rs` 的
`git log --follow` / `git blame --porcelain`）。代码文件在 git 里的历史大部分是 jkk 录的**基线提交**：

- 作者是本机 git 用户、时间是录制时间、标题是 `svn: rN`
- 两次基线之间同事的多次 SVN 提交压成一条
- 初始化时整个工作副本作为一条「新增」导入，之前的历史全部看不到

实测（2026-11-27，`BuildPlayerConfig.cs`）：SVN 上 85 次提交，jkk History 只有 3 条；Blame 2614 行作者全是本人。

看历史是为了回答「谁、在哪个版本、为什么改」。只有 SVN 日志能回答。本设计让混合仓库的 History / Blame
显示真实的 SVN 提交，同时保留 git 那一侧（没交付的本地提交、没提交的改动）的信息。

这改变的是 2026-11-08 计划「非目标」里那一条（「混合仓库里看的是 git 这边的历史（含基线提交）」）。

## 2. 目标 / 非目标 / 反目标

**目标**

1. History：每条基线提交展开成它实际包含的 SVN 提交（版本号、作者、时间、提交说明）；初始化那条展开成导入之前的全部 SVN 历史（分页）。
2. History：没交付的 git 提交照常显示并标「未交付」；已交付的 git 提交标「已交付 rN」，SVN 那边的 rN 不再重复列出。
3. 点一条 SVN 提交，右侧 diff 显示这个文件在该版本的改动，diff 选项（上下文行数、忽略空白）与 git 提交一致。
4. Blame：原来归到基线提交的行，改归到真实的 SVN 版本和作者；没交付的提交、没提交的改动保持 git 的归属。
5. 离线或 SVN 出错时，git 那一层照常显示，只是不展开，并明确告诉用户原因。

**非目标**

- 服务器上比当前基线更新、还没「更新」下来的 SVN 提交，不显示（那是 Show Log 的事，不是「这个文件在我这里的历史」）。
- SVN 条目上不提供「Blame 此版本」「Blame 此改动之前的版本」（第一版禁用，菜单置灰并说明原因）。
- 不做 SVN 结果的磁盘缓存；只在进程内缓存。
- 不改纯 git 仓库的任何行为。
- 不改提交图、不改 `svn_log_cache`（不给它加 `-v`）。

**反目标（做到了也算失败）**

- 把一行代码归给**错误**的 SVN 作者或版本。宁可退回「基线 rN」，也不能错配（见 §6.4 的行数核对）。
- 打开 History 要等网络：git 列表必须先出来，SVN 补全异步到达。
- 连不上服务器时卡住几十秒没有反馈。
- 往用户的 `.git` 写任何东西（对象、ref、配置）。本功能全程只读。

## 3. 现状（实现者必须先读的代码）

| 内容 | 位置 |
|---|---|
| History / Blame / 单文件 diff | `src-tauri/src/engine/file_history.rs`，命令在 `src-tauri/src/commands/history.rs` |
| 基线记账：floor + 每文件 revision + 墓碑 | `src-tauri/src/engine/svn_basemap.rs`（`BaseMap::rev_of`、`read_base_map`） |
| 交付录的基线额外记 `Delivered-Rev: N` | `src-tauri/src/engine/svn_baseline.rs:979` 起 |
| 基线分支名 `svn/base` | `src-tauri/src/engine/svn_config.rs:18` |
| 仓库相对路径 → 服务器 `<url>@<rev>`（按白名单单元 / external 选 URL） | `src-tauri/src/engine/svn_health.rs` 的 `unit_of`、`server_url_at`；编码用 `svn_tombstone::encode_url_path` |
| svn 调用封装（统一注入 `--non-interactive`，**没有超时**） | `src-tauri/src/engine/svn_cli.rs:35` |
| 交付后新基线以 deliverRef 为祖先（交付的 git 提交留在历史里） | `docs/superpowers/specs/2026-09-12-svn-git-hybrid-design.md` 的 D8a |
| 前端 | `src/components/FileHistoryView.tsx`、`src/engine/fileHistory.ts`、`src/store/repoStore.ts` |
| SVN fixture | `src-tauri/tests/common/svn_fixtures.rs`（`svnadmin create` + `file://`） |

已核实的 svn 行为（`svn help`，svn 1.14.5）：

- `svn log` 对本地路径默认范围是 `BASE:1`；对 `URL@REV` 默认 `REV:1`。默认跟着复制历史往前查（不加 `--stop-on-copy`）。
- `svn blame TARGET@REV` 不带 `-r` 等于 `-r 0:REV`。

## 4. 方案比较

| 方案 | 做法 | 为什么选 / 不选 |
|---|---|---|
| A（原报告推荐） | 混合仓库直接换成 `svn log <工作副本路径>` / `svn blame <工作副本路径>` | 不选。①工作副本路径默认查到 `BASE`，从提交详情打开旧提交或别的分支时，会混进那之后的提交；②svn blame 把没交付的 git 提交和没提交的改动混成一片「本地改动」，比现在丢信息；③离线时要另写一整套退回 |
| B | 保留 git 列表，每条基线旁标出包含哪些 SVN 提交，看改动仍走 git | 不选。初始化之前的历史看不到，Blame 不解决 |
| C | History 顶部加一行说明 | 不选。问题不解决 |
| D | 给 `svn_log_cache` 加 `-v`，离线按文件查 | 不选。缓存体积没量过，初始化之前的历史仍要联网 |
| **A'（选定）** | git 历史打底，基线提交按记账展开成 SVN 提交；Blame 里归到基线的行按原始行号映射到 `svn blame` | 选定（用户 2026-11-27 确认）。没交付的提交保留 git 信息；从任何 git 提交打开都有正确的上界；离线时自然退化成现在的样子 |

## 5. History 设计

### 5.1 数据流

```text
前端打开 History
  ├─ invoke file_history(...)            ← 现有命令，立即出列表（多了 §5.2 的字段）
  └─ 若是混合仓库：invoke svn_file_history(...)   ← 新命令，异步；结果到了再把基线条目展开
       └─ 「加载更多」：带上一页给的 cursor 再调一次
```

### 5.2 `file_history` 的新增字段（只加不改）

`FileHistoryEntry` 加一个可选字段 `svn`，纯 git 仓库恒为 `None`（序列化为缺省 / `null`）：

```rust
pub struct EntrySvn {
    /// 这条是不是基线提交（能被 `svn_basemap::parse_message` 解析出记账）
    pub baseline: Option<BaselineInfo>,
    /// 非基线提交：是否已交付（见 §5.4）；基线提交为 None
    pub delivered: Option<DeliveredState>,
}
pub struct BaselineInfo {
    /// 这个文件在该基线里对应的 revision：`BaseMap::rev_of(entry.path)`，不是 floor
    pub file_rev: u64,
    pub floor: u64,
    pub delivered_rev: Option<u64>,
}
pub enum DeliveredState { Undelivered, Delivered { rev: u64 } }
```

- 判定基线：对每条 entry 用 libgit2 读 message（`git_read::commit_message`）再 `parse_message`。最多 1000 条，只在混合仓库做。
- 判定「未交付」：该提交不是 `svn/base` 当前 tip 的祖先（`graph_descendant_of` 或等价）。
- 「已交付 rN」：该提交是某个带 `Delivered-Rev: N` 的基线 B 的祖先，且不是 B 的上一条基线的祖先。实现可以沿 `svn/base` 的 first-parent 链找到这样的 B。

`file_rev` 为什么用 `rev_of(path)` 而不是 floor：混合版本基线里，各文件停在不同的 revision（`svn_basemap.rs` 的 `revs`）。标签上的 `svn: rN` 是 floor。

### 5.3 展开规则：哪个 SVN 提交归哪条基线

把这个文件 git 历史里的基线条目从新到旧记为 B1, B2, …, Bk（只含 `git log --follow` 列出来的、即改动了这个文件的基线），各自的 `file_rev` 为 R1 > R2 > … > Rk。

- SVN 提交 X 归 Bi，当且仅当 R(i+1) < X ≤ Ri；最老的 Bk 的区间是 (0, Rk]。
- 等于某个可达基线 `delivered_rev` 的 X 不单独列出：它已经由「已交付 rX」的 git 提交代表。
- X 超过 R1 的不显示（非目标：服务器上更新的提交）。

### 5.4 SVN 查询与分页

一次查询：`svn log --xml <url>@<R> -r <R>:1 -l 50`，`<url>` 由 `server_url_at` 的同一套逻辑从 `entry.path` 算出（要把它从 `svn_health.rs` 提成共用函数，不要复制一份）。

- 从 B1 开始，peg 用 (B1.path, R1)。拿回的条目按 §5.3 分到各组。
- 返回 `cursor`：下一页从「本页最老的 revision − 1」接着查，peg 不变。
- **一段查完了**（svn 返回少于 50 条）：记这一段查到的最老 revision 为 O（这个节点在 SVN 上的起点）。区间上界 ≥ O 的组都已完整（查不到的就是 0 条）；区间整个落在 O 之下的组是这条 peg 够不着的，取其中最新的一条基线的 (path, file_rev) 作新 peg 继续。这条规则同时覆盖：git 里改名后交付（SVN 那边是删除 + 新增，复制历史断了）和删除后重建。
- 同一个 revision 号出现两次（svn 自己跟着复制历史查到了，换 peg 后又查到）按版本号去重。
- 一页固定 50 条 SVN 条目；git 条目不计入。

命令签名（JS 侧 camelCase）：

```text
svn_file_history(path, file, baselines: [{ id, path, fileRev }], cursor?: Cursor)
  → { groups: { [baselineId]: SvnRevEntry[] }, cursor: Cursor | null, error: SvnFetchError | null }
SvnRevEntry = { rev, author, time, message }   // message 是完整说明，列表里显示第一行
```

前端把它已经拿到的基线条目传过去，后端不再重跑 `git log`。

### 5.5 显示

```text
● a1b2c3d  修改打包参数                 你      11-26   未交付
● 9f8e7d6  svn: r70424                  基线    11-21   ▾ 1 次 SVN 提交
    r70193  调整 BuildPlayerConfig      gaozhaotai  11-20
● 5e4d3c2  svn: r67184                  基线    11-07   ▾ 1 次 SVN 提交
    r67080  ……                          panyixiao   11-06
● 1a2b3c4  svn: r65816（初始化导入）    基线    10-30   ▾ 50 次 SVN 提交（已加载 50）
    r65228  ……
    …
    [加载更多]
```

- 基线行保留（它是 git 里真实存在的提交，点它看的仍是 git diff），默认展开。
- 某组区间里一条 SVN 提交都没有（例如只是 svn update 把版本号推上去、内容是同事改了又改回）：显示「0 次 SVN 提交」，不报错。
- 还在查：基线行显示「正在读取 SVN 日志…」。

### 5.6 点 SVN 条目看改动

新命令 `svn_rev_file_diff(path, url, pegRev, rev, options)`：

- `svn cat -r <rev-1> <url>@<pegRev>` 与 `svn cat -r <rev> <url>@<pegRev>`（操作版本从 peg 往回追，跟得上复制）。`rev` 是这个文件的新增版本时旧侧为空。
- 两份内容在内存里做 diff，产出与 `commit_file_diff_with` 同形的 `DiffData`，`DiffViewOptions` 同样生效。**不往 `.git` 写 blob**（反目标）。具体用 `git2::Patch::from_buffers` 还是别的方式，由实现者决定，但输出必须走现有的 `DiffData` 结构，前端 `DiffView` 不改。
- 二进制按现有规则显示「二进制文件」。

## 6. Blame 设计

### 6.1 数据流

```text
invoke blame_file(...)            ← 现有，立即出结果；BlameCommit 多一个 baseline 字段（同 §5.2 的 BaselineInfo）
若有任何行归到基线提交：
  invoke svn_blame_overlay(path, targets: [{ baselineId, origPath, fileRev }])
    → { [baselineId + origPath]: { lines: (rev|null)[], commits: { [rev]: { author, time } } } | { error } }
```

### 6.2 映射

`git blame --porcelain` 每行给出 `(commit, orig_line, orig_path)`。对 commit 是基线 B 的行：

1. 查 `svn blame --xml <url(orig_path)>@<B.file_rev(orig_path)>`
2. 取第 `orig_line` 行的 revision、作者、时间
3. 前端把这行显示成 `rN · 作者`（`svn blame --xml` 自带作者和时间）。提交说明 blame 不给，从 §5.4 的 `svn log` 结果里取（同一 peg、同一进程内缓存）；缓存里没有的版本显示「（说明未加载）」，不为它单独发请求。一条 `svn log` 能否一次查多个不连续 revision（`-c a,b,c`）未核实，实现时实测再定

成立的前提：基线提交里这个文件的内容 = 服务器上 `path@file_rev` 的内容（基线不变量）。

### 6.3 不归到基线的行

没交付 / 已交付的 git 提交、`UNCOMMITTED` 的行，保持 git blame 原样。已交付的提交额外显示「已交付 rN」（同 §5.2）。

### 6.4 防错配

- `svn blame` 返回的行数 ≠ 基线里该文件的行数（`git cat-file` 读那个 blob 数行）：这个 (基线, 路径) 整体不映射，这些行保持「基线 rN」并在 Blame 顶部提示「有 N 行没对上 SVN 版本，保持基线显示」。`svn:keywords` 展开、`svn:eol-style` 都不改变行数，这个核对足以挡住错配。
- 一次 Blame 涉及的不同基线超过 8 个：只映射行数最多的 8 个，其余保持基线显示并提示。（8 是本设计的取值，不是 svn 限制。）

## 7. 离线、超时与缓存

- 新命令的所有 svn 调用带超时，**默认 15 秒**（本设计取值）。到点杀掉 svn 进程（含子进程），返回 `svn_unreachable`。`svn_cli.rs` 现在没有带超时的入口，需要新增一个，仍走 `svn_command` / `--non-interactive` 那套，不另写进程调用。
- 出错时：History 的基线行显示「SVN 明细没取到：<原因>」，列表顶部一行「连不上 SVN，基线条目显示的是快照」；Blame 顶部同样一行。git 部分一律正常。
- 区分错误：连不上 / 超时、认证失败（`--non-interactive` 下 svn 直接报错）、路径在服务器上不存在。各给一句话原因，不给原始 stderr 大段输出。
- 缓存：进程内，键 = (仓库 uuid, URL 路径, peg, 查询参数)。`R ≤ 某个已存在的 revision` 的结果不会再变，可以一直留到进程退出；设个总条数上限（例如 200 项，LRU）。

## 8. 范围之外的文件

- 不在白名单 / external 单元里的路径（`unit_of` 返回 None）、Local-Only 文件、git 历史里一条基线都没有的文件：不调用 SVN，行为与现在完全相同，不显示任何提示。
- 纯 git 仓库：不调用新命令。

## 9. 测试

Rust 集成测试，全部在 `svn_fixtures.rs` 风格的临时 `file://` 仓库里，跑完即删（AGENTS.md §8）：

| 场景 | 断言 |
|---|---|
| 两次基线之间同事提交 3 次 | 后一条基线展开成这 3 个版本，作者、说明与 `svn log` 一致 |
| 初始化前已有 60 次提交 | 初始化那条第一页 50 条，cursor 再取 10 条，之后 cursor 为 null |
| 混合版本基线（文件 rev ≠ floor） | 区间按 `rev_of(path)` 算，不按 floor |
| 交付后同事紧跟着提交 | 交付的 rN 不单列；本地提交标「已交付 rN」；同事的版本归到那条基线 |
| 没交付的提交 | 标「未交付」，不调 SVN 也能出现 |
| SVN 上 `svn mv` 改名 | 一段查询跟着复制历史查到改名前 |
| git 里改名后交付（SVN 删除 + 新增） | 换 peg 继续，改名前的组也填上 |
| 删除后重建 | 同上，按 peg 换段 |
| 从提交详情打开旧提交 | 不出现该提交基线之后的 SVN 版本 |
| 断网：指向不存在的端口（立即拒绝） | 返回 `svn_unreachable`，git 部分不受影响 |
| 超时：本地起一个 accept 后不回应的 TCP 端口 | 15 秒（测试里调小）内返回超时 |
| Blame 映射 | 归到基线的行与 `svn blame` 逐行一致；未提交的行仍是 `UNCOMMITTED` |
| Blame 行数不等（人为构造） | 该组保持基线显示，返回提示 |
| `svn:keywords` / `svn:eol-style=CRLF` 的文件 | 映射正确 |
| 路径含空格、中文、`@` | URL 编码正确（复用 `encode_url_path`） |

前端：`src/engine/` 里的分组、合并、去重逻辑做成纯函数并写 vitest；`FileHistoryView` 的加载中 / 出错 / 加载更多状态写 RTL 组件测试（`// @vitest-environment jsdom`）。

真实仓库验收：只跑只读命令（`svn log` / `svn blame` / `svn cat` 和打开 History），不做任何写操作（AGENTS.md 首条）。

## 10. 验收

1. `BuildPlayerConfig.cs` 的 History：r70424 那条展开出 r70193 · gaozhaotai，初始化那条展开出 r65228 及更早，条目与 `svn log` 一致。
2. 没交付的提交在最上面，标「未交付」。
3. 点 r70193，diff 与 `svn diff -c 70193 <文件>` 的改动行一致。
4. Blame：原来算在 r65816 的 2608 行改成实际作者与版本，与 `svn blame` 一致；本地未交付 / 未提交的行保持 git 归属。
5. 拔网线：git 列表照常立即出现，基线行显示原因，顶部有提示，15 秒内结束等待。
6. 纯 git 仓库的 History / Blame 行为不变（现有测试全过）。

## 11. 交给实现者决定的

- 「已交付」的祖先判定用 libgit2 还是一次 `git rev-list`，按 1000 条的耗时选。
- 内存 diff 的具体实现（§5.6）。
- 超时入口的函数名、放在 `svn_cli.rs` 的哪里。
- 前端展开 / 折叠的交互细节，跟随现有 `FileHistoryView` 的样式。

## 12. 本设计的取值（可推翻）

| 取值 | 理由 |
|---|---|
| 每页 50 条 SVN 条目 | 与 TortoiseSVN Show Log 默认一致；实测 0.6 秒 |
| 超时 15 秒 | 比正常查询（< 1 秒）大一个数量级，又比 Windows TCP 连接超时（约 21 秒，未实测）短 |
| Blame 最多映射 8 个基线 | 每个约 0.4 秒，8 个串行约 3 秒，可以并行 |
| SVN 条目不提供 Blame 此版本 | 第一版控制范围；需要时另开 |

## 13. 估时

后端约 1.5 天，前端约 1 天，fixture 测试约 1 天，合计 3 到 4 天。

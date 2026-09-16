# 基线改从服务器录 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: 用 `superpowers:executing-plans` 逐 Task 实现。步骤用 `- [ ]` 跟踪。
> 配套决策日志：`2026-09-17-svn-baseline-from-server-decisions.md`（`AGENTS.md` §2.1）。

**Goal:** 让「重建基线」和「初始化向导」在**有本地修改的工作副本上**也能用——录进基线的是服务器在 rN 的内容，不是磁盘内容。

**Architecture:** 干净文件仍从磁盘录（它们本来就等于服务器内容）；脏文件逐条换成 `svn cat -r BASE` 的字节；`svn add` 未提交的和未版本控制的从树里剔除。体检判据随之从「一条差异都没有」推广成「每处差异都能被一条已知本地修改解释」。全程对工作目录只读。

**Tech Stack:** Rust（`src-tauri/src/engine/svn_*`）、git plumbing（`hash-object` / `update-index` / `write-tree`）、svn CLI（`cat` / `status` / `diff`）、React + TypeScript 前端文案。

**Spec:** `docs/superpowers/specs/2026-09-12-svn-git-hybrid-design.md`，本期对应 §6.5 P1、§6.5.1、§6.3.1、§6.8.7、§16.1、§16.2、§18（十一）。

## Global Constraints

- 本期**仍然零 SVN 写操作**。新增的只有 `svn cat`，只读。`svn revert` / `update` / `commit` 一行都不许出现在 `src-tauri/src/` 里。
- `svn cat` 一律写 **`-r BASE`**，禁止出现 `-r <数字>` 形式（§16.2：`-r N` 联网才能用）。
- `svn cat` 的结果一律走 `run_svn_bytes`，**禁止**经过 `String::from_utf8_lossy`（§16.2 末：GBK 源码会被换成 U+FFFD）。
- 体检判据里的 `S` 必须**恰好**是 R2b 实际动过的路径集合，不许取超集（§16.1：超集会静默削弱判据）。
- `git add -A` / `git update-index` 的 pathspec **永远不许为空**（`AGENTS.md` §6.2，本仓库已因空 pathspec 误伤过一次）。
- 所有 git / svn 写操作的验证都在临时 fixture 仓库里做（`AGENTS.md` 开篇最高优先级约束）。
- 主干是 `master`；本分支 `feat/svn-baseline-from-server` 基于 `feat/svn-phase1c`（1c 重写过 `svn_health.rs`，B 建在它上面）。

---

## File Structure

| 文件 | 本期职责 |
|---|---|
| `src-tauri/src/engine/svn_cli.rs` | 加 `run_svn_bytes`（Task 1） |
| `src-tauri/src/engine/svn_pristine.rs` | **新建**。`svn cat -r BASE` 取单个文件的服务器字节，区分「取到了」与「服务器上没有」（Task 1） |
| `src-tauri/src/engine/svn_xml.rs` | `parse_node_revs` 忽略 `revision="-1"`（Task 2）；status entry 暴露分类需要的字段（Task 4） |
| `src-tauri/src/engine/svn_preflight.rs` | P1 收窄到只拦 `~`，并把脏清单当**数据**返回（Task 3） |
| `src-tauri/src/engine/svn_dirty.rs` | **新建**。R2a 的纯分类逻辑：status 条目 → 取回BASE / 剔除 / 不动（Task 4） |
| `src-tauri/src/engine/svn_baseline.rs` | `TempIndex` 加 `replace_blob` / `remove_path` / `stage_mode`（Task 5） |
| `src-tauri/src/engine/svn_init.rs` | `init_hybrid` 与 `rebuild_baseline` 共用新的 `record_server_tree`（Task 6）；R3c（Task 9） |
| `src-tauri/src/engine/svn_include_check.rs` | 期望集合改服务器口径：磁盘集合 − `{A,?}` + `{D,!}`（Task 7） |
| `src-tauri/src/engine/svn_health.rs` | `check_baseline` 收一个 `explained` 集合，判据改 `D \ S == ∅`（Task 8） |
| `src/components/BaselineBanner.tsx`、`src/engine/*` | 按钮与文案：不再说「需要工作区干净」（Task 10） |
| `docs/superpowers/e2e/2026-09-15-svn-phase1c-e2e.md` | 「已知问题：重建基线点不动」改写成已解决 + 新场景（Task 11） |

`svn_dirty.rs` 单独建而不是塞进 `svn_preflight.rs`：分类是纯函数、要被 init 和 rebuild 两条路径共用，而且 Task 4 的测试矩阵（八种 status）值得独占一个文件。

---

## Task 1: `run_svn_bytes` 与 `svn cat -r BASE`

**Files:**
- Modify: `src-tauri/src/engine/svn_cli.rs`
- Create: `src-tauri/src/engine/svn_pristine.rs`
- Modify: `src-tauri/src/engine/mod.rs`
- Test: `src-tauri/tests/svn_pristine.rs`（新建）、`src-tauri/tests/common/svn_fixtures.rs`（加 fixture）

**Interfaces — Produces:**
```rust
pub fn run_svn_bytes(path: &str, args: &[&str]) -> Result<Vec<u8>>;

/// `svn cat -r BASE` 的三种结局。
pub enum BaseContent {
    /// 服务器 rN 上有这个文件，这是它的字节。
    Bytes(Vec<u8>),
    /// `E195002: has no committed revision` —— 本地 add 了还没提交，
    /// 服务器 rN 上根本没有它。**不是错误**，是一种正常分类结果。
    NotOnServer,
}
pub fn cat_base(repo_root: &str, rel_path: &str) -> Result<BaseContent>;
```

- [ ] **Step 1** 写失败测试 `cat_base_returns_server_bytes_for_a_locally_modified_file`：fixture 里改脏 `Code/A.cs`，断言 `cat_base` 返回服务器字节且**磁盘未变**（前后哈希相同）。
- [ ] **Step 2** 跑，确认因函数不存在而失败。
- [ ] **Step 3** 实现 `run_svn_bytes`（照 `run_svn` 的形状，只把 stdout 原样返回）与 `cat_base`。`E195002` 从 stderr 识别，走 `run_svn_code` 拿 stderr。
- [ ] **Step 4** 补齐其余测试：已 `svn delete` 的文件取得到；`svn add` 未提交的返回 `NotOnServer`；**GBK 编码的 `.cs` 字节逐字节相同且不含 U+FFFD**（测试 79）；仓库目录改名模拟离线后仍成功（测试 78）。
- [ ] **Step 5** `cargo fmt` → `cargo clippy --all-targets` → `cargo test`，全绿后 commit。

**验证要点：** `-r BASE` 写死在实现里；测试里额外断言实现不产生 `-r <数字>` 形式的参数（可用一个把 args 暴露出来的小函数，或在集成测试里跑离线场景间接证明）。

---

## Task 2: P2 必须忽略 `revision="-1"`

**这是 Task 3 的前置。** 放松 P1 之后，任何一个 `svn add` 未提交的节点都会让 `parse_node_revs` 多出一个 `-1`，`check_single_node_rev` 于是报「mixed-revision，请先跑一次完整 svn update」——一条做了也没用的引导。现有代码靠「P1 先拦住」才没暴露这个坑，注释里写得很明白。

**Files:**
- Modify: `src-tauri/src/engine/svn_xml.rs`（`parse_node_revs`）
- Modify: `src-tauri/src/engine/svn_preflight.rs`（`check_single_node_rev` 里那段关于负数的注释与错误分支）
- Test: `src-tauri/tests/svn_xml.rs`、`src-tauri/tests/svn_preflight.rs`

- [ ] **Step 1** 先用**真实 svn** 确认 `svn add` 未提交的节点在 `status -v --xml` 里确实是 `revision="-1"`——这是从注释里读来的，不是本会话实测过的，写进测试之前必须自己看一眼（`AGENTS.md` §6.2）。
- [ ] **Step 2** 写失败测试 `node_revs_ignores_uncommitted_additions`：真实 fixture，`svn add` 一个新 `.cs` 不提交，断言 `parse_node_revs` 返回 `{N}` 而不是 `{-1, N}`。
- [ ] **Step 3** 实现：跳过负 revision，并在注释里写清楚为什么（未提交的节点没有服务器 revision，不该参与「工作副本是不是单一 revision」的判断）。
- [ ] **Step 4** 改 `check_single_node_rev` 里 `u64::try_from` 那条错误分支的说明——旧文案说「走到这里说明 P1 没有先于 P2 执行」，现在不成立了。
- [ ] **Step 5** 回归：mixed-revision fixture 仍然被拦（测试 80），错误信息仍指向 svn update 而不是 `~`。
- [ ] **Step 6** 三件套 + commit。

---

## Task 3: P1 收窄到只拦 `~`，脏清单作为数据返回

**Files:**
- Modify: `src-tauri/src/engine/svn_preflight.rs`
- Test: `src-tauri/tests/svn_preflight.rs`、`src-tauri/tests/svn_init.rs`

**Interfaces — Produces:**
```rust
pub struct Preflight {
    pub node_rev: u64,
    pub repos_root: String,
    pub uuid: String,
    /// 新增：白名单内所有非 pristine 的条目，R2a 的输入。
    pub dirty: Vec<DirtyEntry>,
}
pub struct DirtyEntry { pub path: String, pub item: String, pub props: String }
```

- [ ] **Step 1** 写失败测试 `preflight_passes_on_a_dirty_working_copy`：`svn_repo_dirty` 各种 kind 都应通过，且 `dirty` 里含对应条目。
- [ ] **Step 2** 写失败测试 `preflight_still_blocks_obstructed`（测试 75）：把一个白名单内的 `.cs` 换成同名目录，断言拒绝、错误码 `svn_preflight_obstructed`、指名该路径。
- [ ] **Step 3** 实现：`check_clean` 改名 `collect_dirty`，只在遇到 `item == "obstructed"` 时报错，其余收集返回。`item == "external"` 仍然跳过。
- [ ] **Step 4** 删掉 `svn_preflight_dirty` 那段长文案（三条出路的引导）——它的前提没了。**不要留成死代码**。
- [ ] **Step 5** 三件套 + commit。

⚠️ 这一步之后 `init_hybrid` 会在脏工作副本上**录出错误的基线**（磁盘内容），直到 Task 6 接上。Task 3–6 之间的中间状态不可交付，这一段不 push 给别人用。

---

## Task 4: R2a 分类

**Files:**
- Create: `src-tauri/src/engine/svn_dirty.rs`
- Test: `src-tauri/tests/svn_dirty.rs`

**Interfaces — Produces:**
```rust
pub enum Fix {
    /// 磁盘上不是服务器内容（或根本没有）：M / R / C / D / !
    TakeBase,
    /// 服务器 rN 上没有它：A / ?（unversioned / ignored）
    Drop,
}
/// 返回值是**有序**的，且只含需要动的路径——干净文件不出现。
pub fn classify(entries: &[DirtyEntry]) -> Result<BTreeMap<String, Fix>>;
```

- [ ] **Step 1** 写表驱动测试，八种输入各一行：`modified` → TakeBase、`replaced` → TakeBase、`conflicted` → TakeBase、`deleted` → TakeBase、`missing` → TakeBase、`added` → Drop、`unversioned` → Drop、`ignored` → Drop。
- [ ] **Step 2** 补边界：`normal` + `props != "none"`（纯属性变更）→ **不进结果**（内容没变，换不换都一样，而进了 `S` 会削弱体检判据——§16.1「S 必须恰好」）。`external` → 不进结果。未知 item → `Err`，不许静默当成不用动。
- [ ] **Step 3** 实现。
- [ ] **Step 4** `cargo test` + commit。

---

## Task 5: `TempIndex` 的 blob 替换与条目删除

**Files:**
- Modify: `src-tauri/src/engine/svn_baseline.rs`
- Modify: `src-tauri/src/engine/git_cli.rs`（若还没有「向 stdin 喂字节」的入口）
- Test: `src-tauri/tests/svn_baseline.rs`

**Interfaces — Produces:**
```rust
impl TempIndex {
    /// 该路径在候选索引里已有的 mode；不在索引里返回 None。
    pub fn stage_mode(&self, repo_root: &str, path: &str) -> Result<Option<String>>;
    /// 把 bytes 写成 blob 并挂到该路径上。mode 缺省 100644。
    pub fn replace_blob(&self, repo_root: &str, path: &str, bytes: &[u8]) -> Result<()>;
    /// 从候选索引里摘掉一条。
    pub fn remove_path(&self, repo_root: &str, path: &str) -> Result<()>;
}
```

- [ ] **Step 1** 先确认 `git_cli` 里有没有能向 stdin 喂**字节**的封装；没有就加一个，**不要绕过 `run_git` 另起进程**（`AGENTS.md` §7.3）。
- [ ] **Step 2** 写失败测试：`replace_blob` 之后 `write_tree` 出来的树里该路径的 blob 内容 == 喂进去的字节（用 `git cat-file blob` 读回来逐字节比）。
- [ ] **Step 3** 写失败测试：`remove_path` 之后 `ls_files` 里没有它。
- [ ] **Step 4** 写失败测试：非 UTF-8 字节（GBK）原样往返。
- [ ] **Step 5** 实现（`hash-object -w --stdin` → `update-index --add --cacheinfo`；删用 `--force-remove`）。
- [ ] **Step 6** 边界：空路径一律拒绝；`replace_blob` 对不在索引里的路径（`D` / `!`）要能新建条目。
- [ ] **Step 7** 三件套 + commit。

---

## Task 6: 串成 `record_server_tree`，init 与 rebuild 共用

**Files:**
- Modify: `src-tauri/src/engine/svn_init.rs`
- Test: `src-tauri/tests/svn_init.rs`

**Interfaces — Produces:**
```rust
/// 录出「服务器 rN 的树」。返回 TREE 与**实际动过的路径集合**（体检的 S）。
fn record_server_tree(
    repo_root: &str,
    whitelist: &[String],
    extensions: &[String],
    dirty: &[DirtyEntry],
) -> Result<(String, BTreeSet<String>)>;
```

- [ ] **Step 1** 写失败测试（测试 74）：wc 停在 rN，白名单内造出 `M` / `A` / `?` / `D` / `!` / `R` 各一例，重建后逐条断言基线树里的 blob == `svn cat -r BASE` 的字节，`A` 与 `?` 不在树里。
- [ ] **Step 2** 同一测试里断言**磁盘一个字节没动**：重建前后逐文件哈希相同，`git status --short` 前后一致。这条是 §16.1 安全性论证的核心，不能省。
- [ ] **Step 3** 实现：`TempIndex::empty` → `add_all` → 按 `classify` 的结果逐条 `replace_blob` / `remove_path`。
- [ ] **Step 4** 把 `init_hybrid` 与 `rebuild_baseline` 都改成走它，**不要复制两份**。
- [ ] **Step 5** 测试 82：初始化后切到 `work`，`git status` 恰好显示原有的本地 SVN 修改（`M` 已修改 / `?` 未跟踪 / `D` 已删除），内容与初始化前逐字节一致。
- [ ] **Step 6** 三件套 + commit。

---

## Task 7: 期望集合改服务器口径

**Files:**
- Modify: `src-tauri/src/engine/svn_include_check.rs`
- Test: `src-tauri/tests/svn_include_check.rs`

- [ ] **Step 1** 写失败测试：脏工作副本上，期望集合 == 磁盘集合 − `{A, ?}` + `{D, !}`。
- [ ] **Step 2** 写回归测试：**干净**工作副本上期望集合与改动前逐条相同（S7 / 恢复⑥ / D7a 三个入口靠这条保持行为不变）。
- [ ] **Step 3** 实现：`expected_set` 多收一个 `dirty` 参数；干净时传空切片，行为退化。
- [ ] **Step 4** 三件套 + commit。

---

## Task 8: 体检判据 `D \ S == ∅`

**Files:**
- Modify: `src-tauri/src/engine/svn_health.rs`
- Test: `src-tauri/tests/svn_health.rs`

**Interfaces — Produces:**
```rust
pub fn check_baseline(
    repo_root: &str, svn_url: &str, rev: u64,
    whitelist: &[String], extensions: &[String],
    /// 本次录制实际动过的路径（§6.8.7 的 S）。干净路径传空集，判据退化成 D == ∅。
    explained: &BTreeSet<String>,
) -> Result<Health>;
```

- [ ] **Step 1** 写测试 76(a) 不误报：wc 停在 rN、手上有 Task 6 那几种脏，断言 `Verified`。
- [ ] **Step 2** 写测试 76(b) 有牙齿：构造「基线声称 r2、磁盘停在 r1」，断言 `svn status` 为空而体检**阻断**、差异列表里是 `A.cs`。**两例必须在同一个文件里成对出现**——只写 (a) 属虚假信心（`AGENTS.md` §13 第 9 条）。
- [ ] **Step 3** 实现：从 `D` 里减掉 `explained` 再判空；`Health::Drifted` 的文件列表只列 `D \ S`。
- [ ] **Step 4** **变异测试**（测试 77）：临时把 `explained` 改成「白名单内全部文件」，确认 76(b) 由绿转红；改回。把这次变异的结果记进决策日志——判据被削弱时唯一会响的警报就是它。
- [ ] **Step 5** 回归：1b / 1c 原有的体检测试（干净工作副本）全部保持绿。
- [ ] **Step 6** 三件套 + commit。

---

## Task 9: R3c 复查 `nodeRevs`

**Files:**
- Modify: `src-tauri/src/engine/svn_init.rs`
- Test: `src-tauri/tests/svn_init.rs`

- [ ] **Step 1** 写测试 81：分别在 R2a 之后注入「改脏一个干净文件」与「外部 commit 一个文件」，断言两种都中止、基线指针不动；再注入「还原一个脏文件」，断言正常成功且树内容仍是服务器内容。
- [ ] **Step 2** 实现 R3c：`write_tree` 之后、`update-ref` 之前重读 `nodeRevs`，不等于 `{N}` 就中止。纯本地，不走网络。
- [ ] **Step 3** 三件套 + commit。

---

## Task 10: 前端文案

**Files:**
- Modify: `src/components/BaselineBanner.tsx`、相关 `src/engine/*`、`src/styles.css`
- Test: `tests/vitest/*`

- [ ] **Step 1** 把「需要工作区干净」类文案全部删掉；重建按钮不再有 disabled 前提。
- [ ] **Step 2** 新错误码 `svn_preflight_obstructed` 的中文映射。
- [ ] **Step 3** `pnpm build` + `pnpm test`。
- [ ] **Step 4** **在真实 `pnpm tauri dev` 窗口里点一次重建**（`CLAUDE.md`：涉及 Tauri API 的行为必须在真实窗口验证）。
- [ ] **Step 5** commit。

---

## Task 11: 文档收口

**Files:**
- Modify: `docs/superpowers/e2e/2026-09-15-svn-phase1c-e2e.md`
- Modify: `docs/superpowers/plans/2026-09-17-svn-baseline-from-server-decisions.md`

- [ ] **Step 1** 「已知问题：重建基线在真实开发节奏下点不动」改写成「已解决」，保留原始复现记录（它是这轮存在的理由），补上现在的做法与新场景步骤。
- [ ] **Step 2** 场景 4 重写：在**有本地修改**的状态下点重建，验「磁盘没变 + 基线是服务器内容」。
- [ ] **Step 3** 决策日志补完四类条目。
- [ ] **Step 4** commit + push，PR base 指向 `feat/svn-phase1c`。

---

## Self-Review

**Spec 覆盖：** §6.5 P1 → Task 3；§6.5.1 末 → Task 6 Step 5；§6.3.1 → Task 7；§6.8.7 → Task 8；§16 R2a → Task 4；R2b → Task 1+5+6；R2c → Task 7；R3c → Task 9；§16.2 字节 → Task 1 Step 4 + Task 5 Step 4；§4.4 `run_svn_bytes` → Task 1。测试 74–82 全部有归属。

**已知缺口（有意留下）：**
- §9.4 恢复流程未改。它自带 `svn revert`，前提本来就是「把磁盘铺回服务器内容」，与本期无关。
- §6.8.7 末记的那个既有盲区（同长度篡改 + 还原 mtime）本期不修，理由见 spec。
- `svn_server_set` 仍然只出提示、不参与判定。Task 7 用的是本地推导，两者分工写在 §6.3.1。

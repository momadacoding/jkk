# 基线改从服务器录 · 决策日志

> 配套计划：`2026-09-17-svn-baseline-from-server.md`。按 `AGENTS.md` §2.1，本文件在动第一个 Task 之前建好，随实现进同一个 PR，是 review 的输入。
> **Review 请先读「偏离计划」条目再看 diff。**

## 进度总账

| Task | 内容 | 状态 | 产出文件 | 验证 |
|---|---|---|---|---|
| 0 | spec 修订（§6.5 P1 / §6.3.1 / §6.8.7 / §16.1 / §16.2 / §13 / §14.2 / §18） | ✅ `19ed981` | `specs/2026-09-12-svn-git-hybrid-design.md` | 纯文档；八条未知已实测（见下） |
| 1 | `run_svn_bytes` + `svn cat -r BASE` | ✅ | `svn_cli.rs`、`svn_pristine.rs`（新）、`tests/svn_pristine.rs`（新，6 例）、fixture `svn_repo_with_gbk_source` | fmt ✓ / clippy 0 警告 / cargo test **497 passed, 0 failed, 39 个二进制**（基线 491/38） |
| 2 | P2 忽略 `revision="-1"` | ✅ | `svn_xml.rs`、`svn_preflight.rs`、`tests/svn_xml.rs`（+2 例） | fmt ✓ / clippy 0 警告 / cargo test **499 passed, 0 failed** |
| 3 | P1 收窄到只拦 `~` | ✅ | `svn_preflight.rs`、`tests/svn_preflight.rs`、`tests/svn_init.rs`、`tests/svn_backend.rs`、fixture `svn_repo_obstructed` | fmt ✓ / clippy 0 警告 / cargo test **501 passed, 0 failed** |
| 4 | R2a 分类 | ✅ | `svn_dirty.rs`（新）、`tests/svn_dirty.rs`（新，8 例） | 见下方合并验证 |
| 5 | `TempIndex` blob 替换 / 条目删除 | ✅ | `git_cli.rs`（`run_git_stdin`）、`svn_baseline.rs`、`tests/svn_baseline.rs`（+6 例） | 同上 |
| 6 | `record_server_tree`，init 与 rebuild 共用 | ✅ | `svn_init.rs`、`tests/svn_init.rs`（+2 例） | 同上 |
| 7 | 期望集合改服务器口径 | ✅ | `svn_include_check.rs`（`server_expected_set`）、`tests/svn_exclude.rs` | 同上 |
| 8 | 体检判据 `D \ S == ∅` | ✅ | `svn_health.rs`、`tests/svn_health_dirty.rs`（新，5 例）、`tests/svn_health.rs` | 同上；**变异测试已跑**，见下 |
| 9 | R3c 复查 `nodeRevs` | ✅ | `svn_preflight.rs`（`read_node_rev`）、`svn_init.rs`、`tests/svn_preflight.rs`（+1 例） | 同上 |

Task 4–9 合并验证（它们是一个不可分割的交付单元，见下方「偏离计划」）：
`cargo fmt` ✓ / `cargo clippy --all-targets` **0 警告** / `cargo test` **523 passed, 0 failed, 41 个二进制**（Task 3 时 501/39，改动前 491/38）。
零 SVN 写操作审计复跑：产品代码实际发出的 svn 子命令是 `--version` / `cat` / `diff` / `info` / `log` / `ls` / `propget` / `status`，全部只读；grep 到的 `add` / `checkout` 经逐条核对都是 **git** 调用（`svn_baseline.rs:105` 走 `run_git_env`，`svn_init.rs:316` 走 `run_git`）。
| 10 | 前端文案 | ✅ | `svnInit.ts`、`RepoTab.tsx`、`tests/vitest/svnInit.test.ts` | `pnpm build` ✓ / `pnpm test` **336 passed / 24 文件**（改动前 333/24） |
| 11 | 文档收口 | ✅ | `e2e/2026-09-15-svn-phase1c-e2e.md`（新增场景 4′ / 5′）、本文件 | 纯文档 |

---

## Task 0（spec 修订）

### 偏离计划

- **本轮没有走 brainstorming → spec → plan 的完整流程。** 方案 A/B/C 的比较、B 的选定都发生在 Phase 1c 的会话里（用户先问「解释一下方案 B」，再选「现在就开 B 的分支」），所以本轮直接从 spec 修订起步。
- **计划文档没有按 `superpowers:writing-plans` 的惯例把每个 Step 的完整代码写进去。** 那个惯例是为「交给一个对本仓库零上下文的工程师」准备的；本期由本会话自己实现，且 §16.1/§16.2 已经把命令形状逐条写死在 spec 里，再抄一遍是双维护。计划里保留的是文件映射、接口签名和测试清单——那三样才是 review 真正要对的东西。

### 计划的错误

（本轮由 spec 直接生成计划，尚无。）

### 计划没写的坑

- **放松 P1 会直接打坏 P2。** `svn_preflight::check_single_node_rev` 把 `svn add` 未提交节点的 `revision="-1"` 收进 `nodeRevs`，靠「P1 先拦住这类节点」才没暴露；代码注释原文：「负数只可能来自 `svn add` 了没提交的节点，而那种节点 P1 已经拦住了」。P1 一放松，任何一个未提交的新文件都会让用户看到「mixed-revision，请先跑一次完整 svn update」——一条做了也没用的引导。因此计划里把它提成独立的 Task 2，排在 Task 3 之前。写 spec 时没看见这条，是读实现才发现的。
- **`-r N` 会让重建变成联网操作。** 实测 `svn cat -r 1` 在仓库不可达时报 `E170013`，而 `-r BASE` 走 `.svn/pristine`、离线照常返回。差点按直觉把 `-r N` 写进 spec（trailer 里记的就是 N，看起来最自然）。等价性由 P2 的 `nodeRevs` 单值授权，已写进 §16.2。
- **`svn cat` 会展开 `svn:keywords`，且展开的是该 revision 的值。** 原本担心它返回未展开的 `$Id$`，那样基线里的这类文件会和同目录干净文件形态不同，工作分支上凭空多出假 diff。实测不存在这个问题。
- **`run_svn` 的 `String::from_utf8_lossy` 不能用于 `cat`。** 它是给 `--xml` 输出准备的。国内 Unity 工程的 `.cs` 很可能是 GBK，lossy 会把每个非法字节换成 U+FFFD，录进基线就是一棵被悄悄改坏的树，而且体检查不出来（`svn diff` 比的是磁盘与服务器，不比基线树）。
- **`svn:executable` 与文件 mode。** Windows 上 `core.fileMode` 通常为 false，`git add` 一律给 100644。`replace_blob` 因此不自己判断 mode，而是沿用该路径在候选索引里已有的 mode，`D` / `!` 这类索引里没有的才退回 100644——避免替换动作顺手改掉一个与本期无关的属性。

### 为什么这么改是安全的

- **对工作目录全程只读。** 新增的唯一 svn 子命令是 `cat`，它只往 stdout 吐字节（B1 实测：磁盘前后逐字节相同）。本期仍然零 SVN 写操作，`revert` / `update` / `commit` 一行都不出现在 `src-tauri/src/` 里。这条要在每个 Task 结束时用 grep 复核，不能只在开头声明一次。
- **「录出来的树 == 服务器 rN」有完整论证，不是直觉。** 见 §16.1：对 `f ∉ S` 由体检判据 `D \ S == ∅` 推出磁盘 == 服务器；对 `f ∈ S` 由构造保证；文件集合由 §6.3.1 单独管。三条合起来才成立，缺一条都不行——所以 Task 6、7、8 必须一起做完才算一个可交付点。
- **窗口期竞态只会误拦，不会静默录错。** 四种情形逐一数过（§16.1 的表），最坏是用户重点一次按钮。再加 R3c 复查 `nodeRevs` 兜一层。
- **判据被削弱时有唯一的警报。** Task 8 Step 4 的变异测试（把 `S` 放大成全集）必须能让测试 76(b) 由绿转红。这是「S 必须恰好」这条约束唯一的自动化保障，变异结果要记进本文件。
- **为什么不直接放行脏工作副本（把磁盘录进去）。** 那会让用户自己的改动被算进基线，Phase 3 交付时因为「和基线一样」而不在交付集合里，**静默消失**。比拒绝糟得多。这是初版那条 clean 门槛存在的真正理由，本轮换的是录制来源，不是拆门槛。

---

## Task 1（`run_svn_bytes` + `cat_base`）

### 偏离计划

- 计划里 `cat_base` 的返回类型写的是 `Result<BaseContent>`，实现照此；但 `run_svn_bytes` 的返回从计划里的 `Result<Vec<u8>>` 改成了 `Result<SvnBytes>`（带 `code` / `stdout` / `stderr`）。原因见下——不给出 stderr 就分不出「服务器上没有这个文件」和「取数失败」，而这两者的处理完全相反。

### 计划的错误

- 计划 Task 1 Step 3 写着「`E195002` 从 stderr 识别」。**错的，而且是我自己的 B3 实测结论误导了自己。** B3 当时跑的是 `svn cat -r 1`，得到 `E195002: has no committed revision`；实现改用 `-r BASE` 之后错误码变成了 **`E200009: has no pristine version until it is committed`**。照 B3 的结论写死 `E195002`，一个未提交的新文件就会掉进「取数失败」分支，整条重建在最常见的开发状态下直接崩。
  - 现在两个码都认，并在注释里写清它随 `-r` 的写法变化。
  - 教训写在这里是因为它有普遍性：**换了参数形式就等于换了一条命令，之前的实测结论不自动适用。**

### 计划没写的坑

- **`E200009` 是个偏通用的错误码（ILLEGAL_TARGET），不能拿它当分类的权威来源。** 所以 `NotOnServer` 被降级成一层**交叉校验**：真正决定「哪些文件从树里剔除」的是 `svn status`（R2a，Task 4），那些文件根本不会走到 `cat_base`。调用方若对一个「本该在服务器上」的文件拿到 `NotOnServer`，说明分类与实际不符，必须**中止**而不是静默把文件丢掉。这一条要在 Task 6 的实现里兑现。
- **`svn cat` 接受 `--` 分隔符**（实测通过），所以路径以 `-` 开头也安全。
- 空路径显式拒绝（`svn_pristine_empty_path`）。`AGENTS.md` §6.2 记过本仓库因空 pathspec 误伤过一次，这类边界一律不交给外部命令去猜。

### 我自己写错的测试（照实记）

`cat_base_preserves_non_utf8_bytes` 第一版加了一条 `assert!(!String::from_utf8_lossy(&got).contains('\u{FFFD}'))`。**这条断言是反的**：那串字节本来就不是合法 UTF-8，对它做 lossy 转换必然产生 U+FFFD，任何正确实现都过不去。真正要证的是「没走 lossy 那条路」，所以改成比对 `got` 与「把 got 过一遍 lossy 之后的样子」不相等，再加一条长度断言（lossy 会把 2 个非法字节膨胀成 3 字节）。

同一个文件里字节比对那条断言从头到尾是绿的——也就是说**实现一次就对了，红的是我的断言**。记下来是因为它正好是 `AGENTS.md` §13 第 9 条的反面教材：一条名字叫「保持非 UTF-8 字节」的断言，测的却不是那件事。

### 为什么这么改是安全的

- `cat_base` 只读。`cat_base_returns_server_bytes_for_a_locally_modified_file` 用「取数前后磁盘逐字节相同」+「用户的本地修改原样还在」两条断言钉住这一点，不是靠声明。
- 离线可用由 `cat_base_works_without_the_server` 钉住：把仓库目录移走之后仍然返回服务器内容。这条测试同时也是「实现没写成 `-r <数字>`」的证明——写成数字的话它会是 `E170013`。

---

## Task 2（P2 忽略 `revision="-1"`）

### 计划没写的坑

- **「全部节点都还没提交」需要一条独立的错误。** 把负数滤掉之后，如果白名单指向的内容整个都是本地新增的（还没进过 SVN），`nodeRevs` 会变成空集，于是掉进既有的那条空集守卫——而它的文案是「最常见的原因是漏了 `-v`」。那句话在这里完全是误导。新增错误码 `svn_xml_all_nodes_uncommitted`，说清真实原因是「这些文件还没提交到 SVN，录不出服务器在 rN 的样子」。
- **过滤放在 `parse_node_revs` 而不是调用点。** 理由是语义：`nodeRevs` 问的是「工作副本停在哪个**服务器** revision」，一个未提交的节点对这个问题无可奉告，它就不该进这个集合。放在调用点的话，将来任何一个新调用点都要自己记得滤一遍。

### 实测（先测后改）

计划 Step 1 要求先用真实 svn 确认形状，因为「未提交的 add 带 `revision="-1"`」是从代码注释里读来的、不是本会话验过的。实测确认属实：

```xml
<entry path="Code\Added.cs">
<wc-status item="added" revision="-1" props="none">
```

`parse_node_revs` 改动前对这份真实输出返回 `[-1, 1]`，改动后返回 `[1]`。

### 为什么这么改是安全的

- `svn_preflight::check_single_node_rev` 里那条 `u64::try_from` 的失败分支**保留着**，只是含义变了：它从前意味着「P1 没有先于 P2 执行」，现在意味着「`parse_node_revs` 的过滤被人删了」——一个实现缺陷的警报，不是用户工作副本的问题。注释已同步改写，没有留下一句过期的论证。
- mixed-revision 的正常拦截不受影响：39 个 `svn_xml` 测试与全量 499 个测试全绿，其中包含既有的 mixed-revision 回归。

---

## Task 3（P1 收窄到只拦 `~`）

### 偏离计划

- **没有新建 `DirtyEntry` 类型，直接复用 `svn_xml::StatusEntry`。** 它已经带着 `path` / `item` / `props` / `wc_locked` 四个字段，而且 `path` 已经是**仓库相对 + 正斜杠**（实测：从 wc 根以 `Code@` 为 target 跑 status，得到的是 `Code\Added.cs`，解析层归一成 `Code/Added.cs`）——正好就是 R2a 要的形状。再造一个平行类型只会多一次转换和一处可能走偏的地方。
- **`dirty` 字段带 `#[serde(skip)]`，不过 IPC。** 计划里没提这一层。真实工程上这份清单可能有几千条（白名单目录底下每个未版本控制文件都算），前端一条都不需要。`svn_backend.rs` 里那条 JSON 形状测试已改成**构造一个非空的 `dirty` 并断言它不出现在 JSON 里**——空 `Vec` 序列化出来也是看不见的，用空值测证明不了任何事。

### 计划没写的坑

- **有四条既有测试编码的是旧行为，必须翻面而不是删掉。** 它们每一条都还承载着一个仍然成立的不变量，删掉就等于把覆盖率悄悄降下去：

  | 测试 | 原来证明什么 | 翻面后证明什么 |
  |---|---|---|
  | `p1_rejects_every_dirty_state` | 四种脏都被拒 | 四种脏都**放行**且如实进清单（覆盖面不变，仍是四种真实形态） |
  | `p1_must_pass_no_ignore_…` | `--no-ignore` 没被删 | 同左——被 `svn:ignore` 挡住的 `.cs` 仍必须**被看见**，否则会被录进基线 |
  | `p1_rejects_svn_ignored_files_outside_the_extension_list_too` | 非代码文件也拦 | 非代码文件也**进清单**（过滤是下游的事） |
  | `p1_catches_a_dirty_file_inside_a_same_repo_nested_external` | `check_targets` 覆盖了嵌套 external | 同左，而且**比从前更要紧**——清单现在是 R2a 的输入，漏掉这条会让 external 里的脏文件被原样录进基线，直接产出假基线，不再只是「少了一道拦截」 |

- **`item="normal" props="modified"`（纯属性变更）要进 `dirty`、但不能进 `S`。** 清单是「非 pristine」的如实转述，所以它进清单；而 R2a 会把它判成「不用动」——属性变了内容没变，换不换字节都一样，让它进 `S` 会**削弱**体检判据（§16.1「S 必须恰好」）。这两层的分工写在 `collect_dirty` 的注释里。
- **`svn status` 报 obstructed 的 item 字面量就是 `"obstructed"`**（实测，`svn_repo_obstructed` fixture：把 `Code/A.cs` 换成同名目录）。

### 为什么这么改是安全的

- 放行本身不会让基线变松。真正的把关从「事前拒绝用户」移到了「事后证伪基线」——录制来源改成服务器内容（Task 6），体检判据改成 `D \ S == ∅`（Task 8）。**这三条必须一起成立**，所以 Task 3 单独落地时 `init_hybrid` 在脏工作副本上会录出错误的基线，这一段中间状态不可交付，已在计划里写明。
- `~` 拦住之后**磁盘原样**，由 `p1_still_blocks_an_obstructed_node` 与 `rebuild_refuses_only_when_a_node_is_obstructed` 两条各自断言（前者查目录还在，后者查目录还在）。不自动修复这条立场没有松动。
- `a_clean_working_copy_reports_an_empty_dirty_list` 钉住「干净时 `S = ∅`」——这是 §6.8.7 那条判据退化成 `D == ∅` 的前提，也是 S7 / 恢复⑥ / D7a 三个既有入口行为不变的依据。

---

## Task 4–9（录制改从服务器取 + 新判据）

### 偏离计划

- **Task 6 与 Task 7 合并成一个提交。** `record_server_tree` 的测试离不开集合口径的改动：脏工作副本上 `verify_include_set` 会立刻报「多收 / 漏收」。硬拆的话 Task 6 只能带着一个红测试进仓库。
- **Task 9（R3c）提前并入同一批。** 它只有几行，而且和 rebuild 的编排在同一个函数里，单独拆一个提交没有任何 review 收益。
- **多出一个计划里没有的公开函数 `svn_dirty::explained_paths`。** 计划里「算 `S`」这件事散在三处（录制、录制后体检、手动体检）。三份各写一遍，迟早有一份会和另外两份不一致，而不一致的方向若是「多算了」，判据就被削弱且没有信号。抽成一个函数之后**录制和体检用的是同一个集合对象**，不是两次独立计算的结果。
- **多出一个 `svn_dirty::read_dirty`（把读 status 从 `svn_preflight` 下沉）。** 原因见下面「计划没写的坑」第一条。

### 计划的错误

- **计划没发现「手动体检」有和重建一模一样的病。** `check_current_baseline` 的前置门槛是「磁盘内容必须与基线树逐字节相同，否则返回 `NotChecked`」——工作副本一脏就永远是「没法下结论」。那和 §16.1 里那条「工作副本必须干净」是**同一个死胡同，只是换了颗按钮**；Phase 1c 刚加上去的「体检基线」按钮等于从一开始就不可用。
  - 已一并改用 §6.8.7 的判据，而且**比原来更强**：原来「基线树与磁盘不一致」一律落 `NotChecked`，现在解释不了的那些会被算成证据（`Health::Lying`）。
  - 相应地，`the_manual_health_check_does_not_call_ordinary_wip_a_lying_baseline` 从「断言 NotChecked」改成「断言 Verified」——测试的用意（别把普通 WIP 指控成假基线）没变松，反而更强：从前只要求「别说它坏」，现在要求「能给出一个肯定的答案」。

### 计划没写的坑

- **`read_dirty` 必须与「判 `~`」分开。** 服务器侧核对（`svn_server_set`）也要这份清单，但它跑在「打开仓库」这条热路径上，**不能**因为一处 obstructed 就让仓库打不开。所以读取下沉到 `svn_dirty::read_dirty`，阻断判定留在 `svn_preflight`。
- **`-r BASE` 下未提交的 add 报的是 `E200009` 而不是 `E195002`**（Task 1 已记）。由此引出一条设计决定：`BaseContent::NotOnServer` **不作为分类的权威来源**，因为 `E200009` 是个偏通用的错误码（ILLEGAL_TARGET）。真正决定「哪些文件剔除」的是 `svn status`；`record_server_tree` 若对一个「本该在服务器上」的文件拿到 `NotOnServer`，会以 `svn_baseline_classification_conflict` **中止**，而不是猜一个。
- **`read-tree` 绝对不能带 `-u`。** 基线树现在是服务器内容、磁盘是用户改过的内容，带 `-u` 会拿前者覆盖后者——本期最不能犯的错。已由 `after_init_git_status_is_exactly_the_users_existing_local_svn_changes` 钉住（断言磁盘内容逐字节原样、被删的文件不许被复活）。
- **`git diff-tree` 必须带 `-z`。** 不带的话 `core.quotePath` 会把非 ASCII 路径转义成 `"Code/\350\247\222..."`，与 svn 侧算出来的 `S` 永远对不上。

### 为什么这么改是安全的

- **`S` 只有一个来源。** `record_server_tree` 先用 `explained_paths` 算出 `S`，**再拿它驱动改写**（`if !touched.contains(path) { continue }`），最后把同一个集合交给体检。不存在「改写用一份、体检用另一份」的可能。
- **变异测试已跑，结果与 §16.1 的预测逐字吻合。** 把 `S` 从 `∅` 放大成 `{Code/A.cs}`（模拟「取超集」这个最容易犯的错）之后：

  ```text
  a_baseline_claiming_a_revision_the_disk_never_reached_is_caught ... FAILED
      实际：Verified { rev: 2 }          ← 谎话被 S 盖住了
  其余三条 ... ok                        ← 包括「不误报」那条
  ```

  也就是说**只有有牙齿那条会响**。这正是这两组断言必须成对写在同一个文件里的理由：单独留下「不误报」那半，判据被削弱时没有任何信号。变异已撤销。
- **磁盘只读有直接证据，不是靠声明。** `rebuild_records_server_content_on_a_dirty_working_copy` 对白名单子树做了完整的「重建前后逐文件字节快照」比对，外加 `git status --porcelain` 前后一致。
- **既有行为没有被改松。** 干净工作副本上 `S = ∅`，判据退化成 §6.8.2 的 `D == ∅`，由 `the_criterion_degenerates_to_the_original_one_on_a_clean_working_copy` 钉住；`svn_health.rs` 里 11 处既有调用全部显式传空集并保持绿，`svn_exclude.rs` 的两处同理。

### 没有自动化覆盖的部分（如实说明）

**测试 81「重建窗口期的外部改动」只覆盖了一半。** `rebuild_baseline` 是一次调用，中间没有注入点，无法在「读完脏清单」和「跑体检」之间插入一次外部 commit。与其写一条名字叫「覆盖竞态」而实际测不到竞态的测试（`AGENTS.md` §13 第 9 条明写这属于阻断级的虚假信心），拆成两半各自真测：

| 竞态表里的行 | 覆盖方式 |
|---|---|
| 干净文件在抓拍后被改脏 | `a_file_that_got_dirty_after_s_was_captured_still_blocks`——直接喂一份**过期的 `S`**，断言判据中止。这就是该场景的真实形态 |
| 外部 commit 把 BASE 推到 N+1 | `read_node_rev_reports_the_current_value_without_running_the_other_checks`——只验 R3c 这道守卫本身读得对、且不会被 obstructed 之类的无关状态干扰 |
| 两者串起来的真实交错 | **没有自动化覆盖。** 需要在 `rebuild_baseline` 里开一个测试专用的注入点，那个代价（往产品代码里加一个只为测试存在的 seam）大于收益 |

---

## Task 10–11（前端文案与文档）

### 偏离计划

- 计划 Task 10 Step 4 写着「在真实 `pnpm tauri dev` 窗口里点一次重建」。**没有执行**，理由见下方「没有执行的验证」。

### 计划没写的坑

- **`svn_preflight_dirty` 这个错误码整条删掉了，而不是改文案。** 它对应的状态已经不存在。留着一条说「请先去 TortoiseSVN 提交或还原」的引导，会把用户推回那个已经不存在、而且从来就走不通的死胡同。加了一条 `不再有 svn_preflight_dirty 这个错误码的条目` 的断言守着。
- **「后端消息已说过的就不重复」那条抑制逻辑按前 12 个字判重**，所以翻新测试时喂进去的后端消息必须与表里那条**开头一致**，否则测的就不是抑制逻辑而是别的东西。翻新时改对了，记在这里免得下一个人踩。
- **`svn_baseline_lying` 的文案要先撇清「不是你的本地修改」。** §6.8.7 之后本地修改已被 `S` 排除在外，但一个手上有改动的用户看到「基线与服务器不符」会立刻把两件事联系起来，然后去做一件完全没用的事（清理工作区）。

### 没有执行的验证（如实说明）

**没有跑 `pnpm tauri dev` 的真实窗口。** 判断依据：

- **没有任何 `#[tauri::command]` 的签名改动。** `Preflight` 新增的 `dirty` 字段带 `#[serde(skip)]`，IPC 上的 JSON 形状逐字不变，并由 `preflight_serializes_to_camel_case_and_keeps_the_dirty_list_off_the_wire` 断言（构造非空 `dirty` 再断言它不出现在 JSON 里）。所以 `AGENTS.md` §7.3 那条「签名改动必须实际跑一次 `pnpm tauri dev`」的触发条件不成立。
- 改动的 UI 是一段确认对话框文案和一张错误码→引导语的映射表，不涉及布局、滚动或渲染，vitest 已覆盖文本本身。
- 真实窗口的点击验证是用户正在做的 E2E，对应新写的**场景 4′ / 5′**。

不写成「已验证」是因为它确实没验过。

---

## 补 Task（2026-09-18）：`hash-object` 漏了 `--path`，两条录制路径分叉

### 计划的错误 / 我自己的错误

用户提出「能不能靠 `git stash` 让工作副本干净，从而照旧从磁盘录基线」，去实测这条路的
过程中**查出我已经写完并推上去的 B 里有一个真 bug**：

```text
干净文件： git add -A            → 跑过滤器（autocrlf 等）
脏文件：   hash-object --stdin   → **默认一个都不跑**
```

本机 `core.autocrlf=true`。一个 `svn:eol-style=native` 的文件，磁盘是 `"n1\r\n"`，
`git add` 进 blob 是 `"n1\n"`，`hash-object --stdin` 是 `"n1\r\n"`——**同一个文件在
基线里长什么样，取决于它当时脏不脏**。在脏工作副本上重建一次就会把行尾整个翻一遍，
工作分支上凭空出现「每一行都改了」的假 diff。

**为什么 B4/B6 没抓住它**：那两条比的是「`svn cat` 的字节 vs 磁盘的字节」，而 bug 在
下一环——那些字节变成 git blob 的过程。测到了链条的前半截就以为覆盖了整条链，这是
这次的教训。

修法：`git hash-object -w --path <该路径> --stdin`。对齐基准必须是 `git add`，因为树里
绝大部分内容是它录进去的。

回归测试 `recording_a_file_produces_the_same_blob_whether_it_was_dirty_or_not`：同一个
文件分别在干净与脏的状态下各录一次，blob 必须逐字节相同。**变异验证已跑**——拿掉
`--path` 立刻变红（`left: [110,49,13,10]` vs `right: [110,49,10]`）。

### 计划没写的坑

- **`git stash` 清不掉 SVN 侧的脏**，实测四种脏一种都没解决：`.psd` git 根本不跟踪；
  `A` / `D` 的登记记在 `.svn/` 里 git 碰不到（`A` 还退化成了更糟的 `!`）；`M` 的那个
  被 autocrlf 改了行尾，svn 照样报 `M`。`stash pop` 之后还多出一条新的损坏
  `! Code/Deep`（git 重放删除时把空目录一并删了）。已写进 spec §16.2.2。
- **同一个发现对 Phase 2 的 `sync_in` 也成立**：S1 的 `git stash push` 与 S10 的
  `git stash apply` 都会把没设 `svn:eol-style` 的文件还原成 CRLF，最坏后果是 Phase 3
  把一整批纯行尾变化提交到 SVN。已写进 spec §7.3.0，标注**尚未验证也未修复**，
  列了三条可能的出路，要求 Phase 2 开工时先定。**不在本期修**——那是另一条流程。

### 为什么这么改是安全的

- 改动只有一处（多一个 `--path` 参数），影响面是「脏文件替换进去的 blob 会跑与
  `git add` 相同的过滤器」，方向是让两条路径**一致**，不是引入新行为。
- 全量 524 passed / 0 failed；clippy 0 警告。

---

## 实测记录（Task 0 之前完成）

八条未知全部在临时 fixture 上实测，探针用完即删，`git status` 已复核干净。

| # | 问题 | 结果 |
|---|---|---|
| B1 | `svn cat` 会不会写磁盘 | ✅ 不写 |
| B2 | 已 `svn delete` 的文件还取得到吗 | ✅ 取得到，磁盘上没有照样取 |
| B3 | `svn add` 未提交的怎么判 | ✅ `E195002: has no committed revision` |
| B4 | `svn:eol-style=native` 的字节一致吗 | ✅ 磁盘与 cat 都是 CRLF |
| B5 | `svn:keywords` 会展开吗 | ✅ 会，`$Id: Kw.cs 1 … $` 与磁盘逐字节相同 |
| B6 | 磁盘是 LF 的文件经 stdout 会被加 CR 吗 | ✅ 不会，4 字节进 4 字节出 |
| B7 | `-r BASE` 离线可用吗 | ✅ 可用；`-r N` 报 `E170013` |
| B8 | `D \ S == ∅` 会误报吗 / 有牙齿吗 | ✅ 不误报；✅ 对「基线声称 r2、磁盘停在 r1」报警 |

⚠️ **一个负面结果照实记下。** 原本用来证明判据有牙齿的那条测试（同长度篡改一个字节 + 还原 mtime）**没有证成**：`svn status` 与 `svn diff` 双双被 svn 的「大小 + mtime 相同就跳过」捷径绕过。这是**既有**盲区——今天的 §6.8 体检跑的就是同一条 `svn diff`——不是本轮引入的。记在这里是为了不把体检的能力说得比实际强。

# 「更新」时 git 不管的文件冲突：在 jkk 里解决

日期：2026-11-25（2026-11-28 复核补测）　状态：预研完成，推荐方案 B，用户决定待定，尚未实现　仓库：mygitclient（主干 `master`）　版本：jkk 0.10.2 起草，行号按 0.10.4 核对；svn 1.14.5（2026-11-28 补测用 1.14.2-SlikSvn）

相关文档：`docs/superpowers/specs/2026-11-25-follow-resolve-with-disk-design.md`（反方向：在 TortoiseSVN 里解决过的冲突，jkk 跟进时复用）

一句话：git 管的代码文件，jkk「更新」时已经在 jkk 里解决冲突，TortoiseSVN 不用管。git 不管的文件（美术、`.prefab`、`.meta` 等）冲突时，jkk 现在整次回退、让你去 TortoiseSVN。能改成在 jkk 里解决：内容冲突和属性冲突已实测可行；树冲突第一版仍交给 TortoiseSVN。推荐方案 B（照常做完更新，之后在 jkk 里解决），约 2 天。

本文的「git 管的文件」= 白名单目录里、扩展名在 jkk 扩展名列表中的文件（主要是代码）；其余都是「git 不管的文件」。

> **2026-11-28 复核的改动**
>
> 1. 第 7 节「再点一次更新」原来写「不受影响」，实测不对：还在冲突的文件会被 svn 跳过，白名单变成混合版本（§5.2）。第 7.1 节第 2 处改动因此是 B 的硬前提。
> 2. 第 10 节第 1 条的两项补测已完成，都可行（§5.1）。
> 3. 新增 §7.3：报告原来没写到的三个坑。
> 4. 行号按 0.10.4 核对修正。

## 1. 需求

在 jkk 里点「更新」后如果有文件冲突，就在 jkk 里解决；解决完，TortoiseSVN 里不用再处理冲突。

## 2. jkk「更新」实际做的 6 步

1. 把你还没提交的代码改动先收到一边
2. 把代码文件换回 SVN 原版
3. 跑 `svn update`，从服务器拿新内容。git 不管的文件在这一步冲突
4. 记下这次更新到的 SVN 版本（基线）
5. 把你在 jkk 里提交过的代码改动接到新版本上。代码冲突在这一步，现在就在 jkk 里解决
6. 把第 1 步收起来的改动放回来

`svn update` 本身不会被打断，总是一口气跑完：没冲突的文件更新好，冲突的文件被 SVN 标成「冲突」留在磁盘上，旁边多出 `.mine`、`.r1` 这类临时文件。下面说的「回退」「停下来」，都是指第 3 步跑完之后 jkk 怎么处理第 4～6 步。

代码：`src-tauri/src/engine/svn_sync_in.rs:265`～`:298`（第 2～4 步）。

## 3. jkk 怎么知道 git 不管的文件冲突了

jkk 不只用 git，也直接问 SVN。「不关心白名单之外的文件」指的是这些文件不进 git，不是 jkk 看不见它们。

- 第 3 步之后，jkk 对整个工作副本跑一次 `svn status`（不带目录参数，不限白名单），挑出冲突的文件（`svn_conflicts`，`svn_sync_in.rs:1147`）。
- 点「更新」时，jkk 先对整个工作副本问一次服务器（`svn status -u`，`src-tauri/src/engine/svn_sync_check.rs:141`）。确认框里「git 管不到、你本地改过、这次更新又会碰到」的文件清单就来自这里。

## 4. 现状（已读代码与测试确认）

| 冲突的文件 | 现在 | 依据 |
|---|---|---|
| git 管的代码 | 不会出 SVN 冲突：第 2 步先换回原版并核对干净，第 3 步才跑。冲突出现在第 5 步，在 jkk 里解决，TortoiseSVN 只看到「已修改」 | 测试 `continue_after_resolving_a_rebase_conflict`（`src-tauri/tests/svn_sync_in.rs:685`） |
| git 不管的文件：内容冲突 | jkk 放弃第 4～6 步、撤销第 1～2 步（这就是「回退」）。第 3 步撤不了，冲突留在磁盘上。提示去 TortoiseSVN 处理，处理完再「重建基线」 | `svn_sync_in.rs:343`、`:415`；测试 `svn_sync_in.rs:1398` |
| 属性冲突 | 同上 | 测试 `svn_sync_in.rs:1420` |
| 树冲突（目录被删、同名文件被新增等） | 同上 | 测试 `svn_sync_in.rs:1476` |

## 5. 实测：SVN 冲突能不能不用 TortoiseSVN 解决

### 5.1 解决命令

环境：系统临时目录里的 `file://` 仓库，两个工作副本模拟「我」和「同事」，测完已删。2026-11-25 用 svn 1.14.5；标「补测」的两行是 2026-11-28 用 1.14.2-SlikSvn 测的。

| 冲突 | 命令 | 结果 |
|---|---|---|
| 文本文件内容 + 属性同时冲突（`.prefab`） | 把合并好的内容写回文件，再 `svn resolve --accept working` | 可行。状态从 `CC` 变成 `MM`（已修改，不再冲突），`.mine`、`.r1`、`.r2`、`.prej` 自动删掉。属性保留本地的值 |
| 二进制文件（`svn:mime-type` 为 `octet-stream`） | `svn resolve --accept theirs-full` | 可行。内容换成服务器版本，文件变回未修改 |
| 文本文件内容冲突，取我的（补测） | `svn resolve --accept mine-full` | 可行。内容是我的，状态 `M`，`.mine`、`.r1`、`.r2` 自动删掉 |
| 属性冲突，取服务器的（补测） | `svn resolve --accept theirs-full` | 可行。属性变成服务器的值，`.prej` 自动删掉，文件状态变干净 |
| 树冲突：本地改了目录里的文件，服务器删了目录 | `--accept theirs-conflict` / `working` | `theirs-conflict` 报 `W195024`（不适用）。`working` 只是标成已解决，留下 `A +`（重新添加整个目录）这种半成品 |
| 树冲突：服务器新增目录，本地有同名未版本控制目录 | `--accept mine-full` / `working` | `mine-full` 报 `W195024`。`working` 留下 `D`（计划删除服务器刚加的目录） |

结论：内容冲突、属性冲突可以在 jkk 里用 `svn resolve` 解决；树冲突在命令行上没有干净的非交互解法，第一版交给 TortoiseSVN。

### 5.2 冲突没解决又更新一次（补测，2026-11-28）

场景：`Art/x.prefab` 内容冲突留着不解决，同事又提交了 r3（改了 `x.prefab`，新增 `z.prefab`），我再跑一次 `svn update --accept postpone`。

```text
Skipped 'Art\x.prefab' -- Node remains in conflict
A    Art\z.prefab
Updated to revision 3.
Summary of conflicts:
  Skipped paths: 1
exit=0

x.prefab 的 revision: 2    z.prefab 的 revision: 3
```

实测结论：

1. 还在冲突的文件被跳过，停在旧版本；其他文件到了新版本。**白名单变成混合版本**，退出码仍是 0。
2. 之后在 `x.prefab` 上 `svn resolve --accept mine-full`，它的 revision 仍是 2。解决的是对 r2 的冲突，服务器已经是 r3。

## 6. 三个方案

例子：你改了 `Art/UI_Main.prefab`，同事也改了同一处并提交了。你点「更新」，第 3 步这个文件冲突。

| 方案 | jkk 怎么做 | 你看到什么、要做什么 |
|---|---|---|
| 现在 | 回退：放弃第 4～6 步，代码换回你的分支、改动放回 | 弹错误「去 TortoiseSVN 处理」。去 TortoiseSVN 解决，再回 jkk 点「重建基线」 |
| A 停下来 | 第 3 步之后先暂停，等你解决完再做第 4～6 步（和代码冲突在第 5 步暂停是同一种做法） | jkk 列出冲突文件，每个给「用我的」「用服务器的」「用 Beyond Compare 合并」；全部解决后点「继续」 |
| B 先做完再解决（推荐） | 不暂停，直接做完第 4～6 步：这些文件 git 本来就不管 | 更新照常完成，之后 jkk 挂出冲突列表，什么时候有空什么时候解决，按钮同 A。但冲突没解决前交付会被拦，见 §7.3 第 2 条 |
| C 照旧回退，换个地方解决 | 和现在一样放弃第 4～6 步 | 冲突改在 jkk 里解决；解决完还要再让 jkk 补一次第 4 步 |

树冲突在三个方案里都照旧回退、交给 TortoiseSVN。

## 7. 方案 B 的安全核对（读代码，没有运行；§5.2 为实测）

问题：工作副本里还有 git 不管的文件处于冲突状态时，jkk 后面的环节会不会出错。

| 后面的环节 | 带着这类冲突时 | 结论 |
|---|---|---|
| 第 4 步，所有文件在同一版本（第一次更新出冲突时） | 录之前只拦 obstructed；冲突文件不进 git，不影响记录（`svn_sync_in.rs:466` `capture_single`、`src-tauri/src/engine/svn_preflight.rs:413` `collect_dirty`） | 不受影响 |
| 第 4 步，文件停在不同版本 | 录之前的检查拦白名单目录里的任何冲突，包括 `.prefab`、`.meta`（`svn_preflight.rs:161` `check_recordable`）。它的理由「冲突标记会被当成服务器内容录进去」只对 git 管的文件成立 | 要改：只拦 git 管的文件和目录。自动跟进用的是同一道检查，也会跟着放行 |
| 第 5、6 步 | 这些文件 git 不碰 | 不受影响 |
| 之后点「交付」 | 白名单目录里有没解决的冲突时，开始前就拒绝，不会交付到一半（`src-tauri/src/engine/svn_sync_out_check.rs:387`） | 安全；要改提示，现在写的是「去 TortoiseSVN 解决」。另见 §7.3 第 2 条 |
| 再点一次「更新」（冲突还没解决） | 实测（§5.2）：冲突文件被 svn 跳过、停在旧版本，白名单变成混合版本，第 4 步走混合录制，撞上上面 `check_recordable` 那一行 | **受影响**：不做第 7.1 节第 2 处改动，这次更新会失败回退。选 B 后这是最常见的情况，不是少见情况 |

### 7.1 B 要改的 3 处

1. 第 3 步之后的判断：git 不管的文件出现内容冲突或属性冲突时，不回退，继续做完。`svn_conflicts`（`svn_sync_in.rs:1147`）现在只返回路径，要按四类分开：git 管的文件、树冲突、目录上的属性冲突、其余。分类要用的字段 `StatusEntry` 里有（`tree_conflicted`、`props`）。
2. 第 4 步录之前的冲突检查（`svn_preflight.rs:161`）：只看 git 管的文件和目录。**这是 B 的硬前提**：冲突留着再更新一次，白名单必然是混合版本（§5.2），不改这处就会回退。
3. 冲突列表：更新完成后在 jkk 里列出这些文件，每个给「用我的」「用服务器的」「用 Beyond Compare 合并」（文本文件才有合并）；交付的提示改成指向这个列表。

### 7.2 仍然回退的情况

- 树冲突（实测没有干净的非交互解法）。
- 目录上的属性冲突：可能改变哪些文件受 SVN 管理（如 `svn:externals`、`svn:ignore`），不能放过。
- 理论上不会出现的「git 管的文件」SVN 冲突：兜底。

### 7.3 写 spec 时要定的三个坑（2026-11-28 补）

1. **解决完的文件仍是旧版本。** 被跳过的文件解决后还停在旧 revision（§5.2 第 2 条）。「用我的」之后交付会被 SVN 报 out-of-date；「用服务器的」拿到的也是旧 revision 的内容。二选一：冲突列表提示「解决后再更新一次」，或解决后由 jkk 对这个文件补一次 `svn update`（可能再冲突一次）。A 方案只在第一次更新时有冲突，文件在最新 revision 上，这个坑主要是 B 的。
2. **「什么时候有空什么时候解决」被交付挡住。** 白名单里有任何冲突，交付就拒绝（`svn_sync_out_check.rs:387`），`.prefab` 冲突也算。美术冲突不解决，代码也交付不出去。二选一：spec 里写明这一点并在冲突列表上提示；或把交付的冲突检查收窄到这次要提交的文件（需要另核对 `svn commit` 的提交范围）。
3. **Beyond Compare 三方合并的三个文件别按文件名猜。** `.rN` 的 N 随版本变。用 `svn info --xml` 的 conflict 段取 base / mine / theirs 三个文件的路径。

## 8. 推荐与估时

- 推荐 B：安全核对通过，用起来比 A 顺，更新不被打断。§7.3 的第 1、2 条会削弱「随时解决」的体验，但不改变推荐。
- 估时：约 2 天，大头是冲突列表（A、B 都要做）。A 约 2～3 天。§7.3 第 1 条若选「解决后 jkk 补一次 update」，B 另加约半天。
- 这是新功能，选定后按项目规范先写设计文档（spec），再写实现计划（plan）。
- 用户决定：待定。

## 9. 测试与验收

| 位置 | 覆盖 |
|---|---|
| `src-tauri/tests/svn_sync_in.rs`（临时 fixture） | 把 `an_update_conflict_outside_git_rolls_back_and_points_to_tortoisesvn`（`:1398`）、属性冲突（`a_property_conflict_during_update_rolls_back`，`:1420`）改写成新行为：更新完成、基线推进、冲突留在工作副本里、jkk 能列出；树冲突（`a_tree_conflict_rolls_back`，`:1476`）保持回退 |
| 同上 | 混合版本下白名单目录里有 `.prefab` 冲突，第 4 步不再被拦；git 管的文件冲突仍被拦 |
| 同上（2026-11-28 补） | `.prefab` 冲突留着不解决，服务器再出一个 revision，再「更新」一次：更新完成、基线推进、该文件仍是冲突且停在旧 revision |
| 冲突解决命令 | 「用我的」「用服务器的」「合并后标记已解决」三种，各在 fixture 上验证 svn 状态变回非冲突、临时文件被删 |
| `tests/vitest/`、`pnpm test:layout` | 冲突列表的文案、按钮状态；新弹框的布局 |

- [ ] fixture 工作副本 + 真 TortoiseSVN：jkk 更新后 prefab 冲突 → 在 jkk 里解决 → TortoiseSVN 里显示「已修改」而不是冲突
- [ ] Rust：`cargo fmt --check`、`cargo clippy --all-targets`、`cargo test` 通过
- [ ] 前端：`pnpm build`、`pnpm test`、`pnpm test:layout` 通过
- [ ] 独立 review 无未修复的高危问题

## 10. 未决

1. ~~补测：文件上的 `--accept mine-full`；属性冲突能否取服务器的值。~~ 2026-11-28 已补测，都可行，见 §5.1。
2. 冲突列表放哪、平时是否常驻。参考 jkk 常驻「体检基线」横幅后来去掉、改成出问题时主动提示的教训：只在有冲突时出现。
3. 用 Beyond Compare 合并时，三方文件取 SVN 留下的文件（路径从 `svn info --xml` 取，见 §7.3 第 3 条）。现有外部合并只支持 git 冲突，要新增一种入口。
4. Unity 开着时，带冲突标记的文本 prefab 会导入报错。这与用 TortoiseSVN 时相同，但要不要在列表里提示。
5. 解决完的文件停在旧 revision 怎么处理（§7.3 第 1 条）。
6. 有 git 不管的文件冲突时，交付拦全部还是只拦要提交的文件（§7.3 第 2 条）。

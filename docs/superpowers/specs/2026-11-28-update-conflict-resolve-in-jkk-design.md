# 「更新」时 git 不管的文件冲突：在 jkk 里解决（设计）

日期：2026-11-28　状态：方案已选定（预研的方案 B），实现中　预研：`docs/superpowers/specs/2026-11-25-update-conflict-resolve-in-jkk-analysis.md`

实现计划：`docs/superpowers/plans/2026-11-28-update-conflict-resolve-in-jkk.md`。

一句话：jkk「更新」时，git 不管的文件（白名单外的文件，或白名单里扩展名不在列表中的文件）出现内容冲突或属性冲突，不再整次回退。更新照常做完、基线照常推进，之后 jkk 弹出 SVN 冲突列表，每个文件可以「用我的」「用服务器的」，文本文件还可以「用外部工具合并」后「标记已解决」。树冲突、目录上的冲突、git 管的文件冲突仍然整次回退，交给 TortoiseSVN。

## 1. 术语

- **W-file**：git 管的文件。白名单目录里、扩展名在 jkk 扩展名列表中的文件（`WScope`）。
- **可放行的冲突**：同时满足三条：不是树冲突；路径在磁盘上不是目录；不是 W-file。其余都是**要回退的冲突**。

## 2. 行为

### 2.1 更新（S5 之后）

`svn update --accept postpone` 之后，读整个工作副本的冲突（同现在）：

| 冲突 | 行为 |
|---|---|
| 没有 | 同现在 |
| 全部可放行 | 不回退，继续 S6–S11。结局与没有冲突时相同（`Done`、`RebaseConflict` 等），不新增结局种类 |
| 至少一个要回退 | 整次回退，同现在。错误文案按两类分别列出：要回退的（去 TortoiseSVN），以及一起留下的可放行的 |

结局不带冲突清单。前端在更新流程走完（含「继续」「放弃」之后）调一次 `svn_update_conflicts`，有冲突就弹冲突列表（§3）。理由：冲突可能跨过一次 rebase 冲突、应用重启，结局里带清单会过期；冲突的事实来源只有 `svn status`。

### 2.2 再点「更新」

更新预检（`svn_sync_check.rs` 的 `sync_in_svn_conflicted`）**照旧拒绝**工作副本里的任何 SVN 冲突。理由（预研 §5.2 实测）：冲突还在时 svn 会跳过这个文件，它停在旧 revision，解决的就是对旧版本的冲突，之后交付会 out-of-date。先解决再更新，文件始终在最新 revision 上。

文案改成：可放行的冲突在 jkk 的冲突列表里解决；要回退的冲突去 TortoiseSVN。前端遇到这个错误码时，除了报错，再弹一次冲突列表。

### 2.3 录基线前的冲突检查（`check_recordable`）

**不改**，照旧拦白名单里的任何冲突。只改文案：可放行的冲突指向 jkk 的冲突列表。

S5 放行之后，单值录制（`run_preflight`）不查冲突，照常录。白名单是混合版本时（external 没跟着更新，少见）走混合录制，会被这道检查拦下：`capture_and_publish` 报错、journal 停在 capturing-baseline、横幅给「继续 / 放弃」。用户在冲突列表里解决之后点「继续」，从录制重来即可走完。前端遇到 `svn_preflight_conflicted` 时同样弹冲突列表。

不收窄的理由见决策日志：收窄会连带改变初始化、重建、跟进的行为，超出这次的需求。

### 2.4 交付

照旧在开始前拒绝白名单里的任何 SVN 冲突（`sync_out_svn_conflicted`）。`svn commit` 只提交改动的文件，但提交之后的 D6 要对白名单 `svn update`，冲突文件会被跳过，D6 / D7 在提交已经发出去之后失败（决策日志）。文案改成指向 jkk 的冲突列表，前端遇到这个错误码时同样弹冲突列表。

## 3. 冲突列表

### 3.1 后端

`svn_update_conflicts(path) -> Vec<SvnConflict>`：

| 字段 | 来源 |
|---|---|
| `path` | `svn status --xml` 的冲突条目，正斜杠 |
| `text` / `props` / `tree` | `item == conflicted` / `props == conflicted` / `tree-conflicted` |
| `dir` | 磁盘上是目录 |
| `gitManaged` | W-file |
| `resolvable` | 可放行的冲突（§1） |
| `mergeable` | 有文本冲突且 `svn info --xml` 的 `<conflict type="text">` 带 `prev-wc-file`。二进制冲突不带它（2026-11-28 实测） |

`svn_resolve_conflict(path, file, choice)`，`choice` 为 `mine` / `theirs` / `merged`：

| choice | 命令 |
|---|---|
| `mine` | `svn resolve --accept mine-full -- <file>` |
| `theirs` | `svn resolve --accept theirs-full -- <file>` |
| `merged` | `svn resolve --accept working -- <file>`（用户已在外部工具里存好合并结果） |

执行前现读一次这个文件的冲突状态：不是冲突、或不是可放行的冲突时拒绝，不信任前端手里的列表。`merged` 还要求它有文本冲突且磁盘上已经没有冲突标记行（`<<<<<<< .mine`、`>>>>>>> .r` 开头），防止把带标记的文件标成已解决。文件路径必须是相对路径、不含 `..`（同 `check_rel_path`）。

`open_svn_conflict_merge(path, toolId, file)`：用外部合并工具打开。`$BASE` / `$LOCAL` / `$REMOTE` 取 `svn info --xml` 的 `prev-base-file` / `prev-wc-file` / `cur-base-file`，`$MERGED` 是工作副本里的文件。路径来自 svn，不按 `.rN` 文件名猜（预研 §7.3 第 3 条）。工具退出后不自动标记已解决。

### 3.2 前端

`SvnConflictsDialog`：

- 只在有冲突时弹（预研 §10 第 2 条）：更新流程结束后、更新或交付报 `sync_in_svn_conflicted` / `svn_preflight_conflicted` / `sync_out_svn_conflicted` 时。不常驻。
- 每个可放行的冲突一行：路径、冲突种类（内容 / 属性 / 内容和属性），按钮「用我的」「用服务器的」；`mergeable` 的再加「用外部工具合并」（配置里有 `mergeArgs` 的工具）和「标记已解决」。
- 要回退的冲突单列一组，不给按钮，写明去 TortoiseSVN 处理。
- 每次操作之后重读列表；全部解决后显示「冲突都解决了」。
- 提示一句：Unity 开着时，带冲突标记的文本文件导入会报错，与用 TortoiseSVN 时相同（预研 §10 第 4 条）。

「用我的」「用服务器的」会改写文件且不能撤销（`mine-full` 丢掉服务器这次的改动，`theirs-full` 丢掉本地改动），点击时先确认。

## 4. 不做

- 树冲突的解决（预研 §5.1：命令行没有干净的非交互解法）。
- 冲突时自动再 update 一次被跳过的文件：预检不放行，就不会出现被跳过的文件。
- 交付只拦要提交的文件：见 §2.4。

## 5. 测试

见实现计划。Rust 侧在 `src-tauri/tests/` 用临时 fixture；前端纯逻辑放 `src/engine/svnConflicts.ts` 并补 vitest；新弹框进 `tests/layout/` 的超长场景。

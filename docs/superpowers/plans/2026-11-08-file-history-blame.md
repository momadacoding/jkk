# 文件历史与 Blame 实现计划

> 2026-11-08。用户反馈「没有 blame 功能，也没有查看文件历史的功能」，要求直接按最优方案做。
> 决策与进度见同目录 `2026-11-08-file-history-blame-decisions.md`。

## 目标

- 任意文件都能看**文件历史**：只列改过这个文件的提交，跨重命名追溯；选中一条看这个文件在那次提交里的 diff。
- 任意文件都能看 **Blame**：每一行最后是哪个提交改的（作者、时间、标题），能从这一行跳到那个提交，能对「这次改动之前的版本」继续 Blame。
- 入口：提交详情 Changes / File Tree 页、Local Changes 的文件右键菜单。

## 非目标

- 不做行内 blame（编辑器里那种悬浮注释），不做 blame 的 `-C` / `-M` 跨文件移动检测参数配置。
- 不做「历史里多选两条互比」。
- SVN 节点（`svn:N`）本身不作为起点：它不是 git 提交。混合仓库里看的是 git 这边的历史（含基线提交）。

## 设计

### 后端（Rust）

新模块 `src-tauri/src/engine/file_history.rs`，三个只读函数，命令放 `src-tauri/src/commands/history.rs`：

| 命令 | 实现 | 返回 |
|---|---|---|
| `file_history(path, file, revs)` | `git log --follow -z --name-status` | 提交列表，每条带**当时的路径**、状态、改名前路径；最多 1000 条，多了置 `truncated` |
| `blame_file(path, file, rev)` | `git blame --porcelain [rev] -- file`；`rev` 为 null 时 blame 工作区 | 行列表 + 提交表（作者、时间、标题、`previous`）；最多 5000 行 |
| `commit_file_diff(path, rev, file, old_file)` | libgit2 tree 对 tree，pathspec 限定到新旧两个路径 + 改名检测 | 与 `read_diff` 同一个 `DiffData` |

读历史与 blame 走 CLI 而不是 libgit2 的理由见决策日志 D1。

参数防护：`file` 为空直接报错（空 pathspec = 全部，AGENTS §6.2）；`rev` / `revs` 以 `-` 开头报错（防止被当成选项）；`revs` 为空报错。

### 前端

- `src/engine/fileHistory.ts`：纯逻辑——起点 rev 列表、blame 分块、按时间算热度、某条历史该在哪个版本上 Blame、初始选中项。
- `src/store/repoStore.ts`：三个 invoke 包装。
- `src/components/FileHistoryView.tsx`：主区域的第三种视图（与 Commits / Local Changes 并列），两个标签页：
  - **History**：左边提交列表，右边这个文件在选中提交里的 diff；按钮「Blame 这一版」「在提交图中定位」。
  - **Blame**：按块显示（同一提交连续的行合成一块，块首行显示短 hash / 作者 / 日期），左侧色条按提交新旧着色；单击块首跳到 History 里那一条，右键菜单：查看这个提交 / 在提交图中定位 / Blame 此改动之前的版本 / 复制 hash。
- 入口：右键菜单加 `File History`、`Blame` 两项。

## Task

1. 后端 engine + 命令 + fixture 集成测试（重命名、删除、工作区未提交行、空参数、选项注入、二进制、截断）
2. 前端纯逻辑 `fileHistory.ts` + vitest
3. store 包装 + `FileHistoryView` 组件 + 样式
4. 接入口（CommitDetailView 两处、LocalChangesView）与 RepoTab 视图切换
5. 全量验证（`pnpm build`、`pnpm test`、`cargo fmt --check`、`cargo clippy --all-targets`、`cargo test`）、真实窗口验证、独立 review

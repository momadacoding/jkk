# 跟进完成后自动放回 Local-Only 补丁（使用报告 2026-11-12，A～D）

前一版：`2026-11-12-restore-local-only-patches.md`（PR 159，报告里的按钮）。本计划推翻其中「不自动写、做成按钮」那一条。

## 问题

固定工作流：分支上有 Local-Only 提交（如 `add @avatarclass` 改了 `ImpChat.lua`）；提交 SVN 前在磁盘上去掉补丁 → TortoiseSVN 提交 → `svn update`。之后：

| 位置 | 内容 |
|---|---|
| 分支 | SVN 新版本 + 补丁（重放正确） |
| 磁盘 | SVN 新版本，没有补丁 |
| 「本地改动」 | 显示删掉了补丁 |

暴露的问题：

1. 自动跟进被拦：拦截判断只看「重放结果与磁盘是否逐字节相同」（`svn_follow.rs` `auto_blocker` 的 `overlap` 那一段）。
2. 补丁要手动点按钮才放回；关掉报告就没有入口。
3. 报告把这个文件列进红色「分支与磁盘对不上」，文案让用户「把磁盘版本提交进分支」——照做补丁就丢了。
4. 放回按钮不说是哪个文件、哪个补丁；横幅原因行没提 Local-Only 这种情况。

## 做法

### A. 跟进完成后在同一把锁里放回

- `restore_local_only` 拆出 `restore_locked`（调用方持同步锁，journal 已删）。
- `follow` / `follow_auto` 在发布返回 `Done` 后、同步锁还没放时调用它，结果写进 `FollowReport.restore`。
- 只自动放回**「SVN 上最后一次改它的是我」**的文件（`Who::is_me(path) == Some(true)`）。其余可放回的文件不自动写，进报告和横幅，由人点。
  - 理由：用户有意去掉补丁、还没提交时，`svn update` 带来别人对这个文件的改动，跟进后它同样满足放回条件；自动写回会撤销用户的准备，接着整包提交就把补丁带上 SVN。自己刚提交过的文件不存在这个问题。
- 不变量改写：跟进模块「journal 存续期间不写磁盘」。放回发生在 journal 删除之后，崩溃恢复不受影响。
- 崩溃恢复（`resume_follow`）走完的那条路不自动放回，只把可放回的文件记进待放回清单。

### B. 自动跟进的拦截判断排除「我的」可放回文件

- `auto_blocker` 的 `bad` 去掉 A 里会自动放回的那些文件。
- 别人最后改的可放回文件照旧拦，原因行写明「分支只比磁盘多 Local-Only 补丁，但 SVN 上最后改它的不是你，可能是你有意去掉的」。

### C. 常驻横幅：待放回清单

- 不按「当前状态」现场推断（会在用户提交前去掉补丁时误报），只认**跟进记下的**文件：`.git/mygitclient/restore-pending.json`。
- 每次跟进结束：清单 = （旧清单 ∪ 这次可放回但没放回的）− 这次放回的。
- 新命令 `svn_pending_restores`：读清单，逐个按放回条件现场重验（含索引），返回能放回的文件和对应的 Local-Only 提交；永久不满足的条目顺手删掉，暂时性的（文件被占用、暂存了别的版本、分支落后）留着。
- 新命令 `svn_dismiss_pending_restores`：横幅上的「不放回」。
- `svn_restore_local_only` 放回成功的文件同时从清单删除。
- 前端：状态刷新后查一次清单，非空时挂横幅「N 个文件缺 Local-Only 补丁」，列出「文件 ← 提交」，按钮「放回」「不放回」。

### D. 报告分节

- 新增「已放回的 Local-Only 补丁」（info），写明路径和提交。
- 新增「没放回的 Local-Only 补丁」（attention），写明原因；下面接放回按钮（列出文件与提交）。
- 红色「分支与磁盘对不上」去掉可放回的文件；文案补一句：被 Local-Only 提交改过的文件，不要直接把磁盘版本提交进分支。
- 自动跟进完成的提示里写放回结果。

### 不做

- E（部分跟进）另开 PR。
- 删除型 Local-Only 照旧只报告。
- 不加设置开关。

## 安全条件

沿用 `svn_restore_local_only` 模块头的五条，写盘前在同步锁内现场重验。新增：自动写只对「我最后提交的」文件。

## Task

| Task | 内容 | 验证 |
|---|---|---|
| T1 | engine：`restore_locked`；`check_paths` 带出 Local-Only 提交；`RestoreOutcome.commits` | `cargo test --test svn_restore_local_only` |
| T2 | engine：follow / follow_auto 串联放回；B；报告 `restore` 字段 | fixture：本例复现（我的提交 → 自动跟进 → 磁盘带回补丁、状态干净）；别人最后改的不自动写且拦自动跟进；磁盘有编辑不写 |
| T3 | engine + 命令：待放回清单、`svn_pending_restores`、`svn_dismiss_pending_restores` | fixture，含空输入 |
| T4 | 前端：报告分节、横幅、自动跟进提示 | `pnpm build`、`pnpm test`、组件测试、`pnpm test:layout` |
| T5 | E2E 清单；独立 review | 文档；review |

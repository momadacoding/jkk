# 跟进后把 Local-Only 补丁放回磁盘（使用报告 9.1）

## 问题

2026-11-11 的使用报告：跟进报告「分支与磁盘对不上」列出 `ImpChat.lua`。分支比磁盘多一行 `---@class Avatar`，来自 Local-Only commit「add @avatarclass」；磁盘就是 SVN r67973，`svn status` 干净。

成因：普通 commit「小婉问答」在 git 里删了这一行，这个文件随 r67973 上了 SVN。跟进时，普通 commit 改过的路径换成 SVN 版本，于是这个 commit 变空、被丢弃；Local-Only commit 不换，照常把这一行加回分支。

按「Local-Only 改动永远留在本地」的语义，分支是对的，缺的是磁盘。现在只报告，用户得自己去「本地改动」里丢弃这个文件，补丁才回到磁盘。

## 做法

跟进模块头写着「全程不写磁盘」，这条不变量不破。写磁盘做成报告里的一个按钮，由用户点。

1. **跟进（只读）**：算出 `drift` 之后，再挑出能安全放回的子集，记进 `FollowReport.restorable`。
2. **新命令 `svn_restore_local_only`**：参数是路径列表。持同步锁，**每条条件现场重验**，不信报告；全部满足的文件用 `git checkout-index -f -z --stdin` 写回磁盘。返回放回了哪些、跳过了哪些以及原因。
3. **报告里的按钮**：在「分支与磁盘对不上」那一节，`restorable` 非空时显示「放回 Local-Only 补丁（N 个文件）」，点完显示结果并刷新状态。

### 能放回的条件（每个文件都要全部满足）

| 条件 | 证明了什么 |
|---|---|
| 不在 SVN 同步中途（没有 journal）、git 空闲、没有 `index.lock`、HEAD 在分支上 | 不和别的写操作撞车 |
| `HEAD:p` 是普通文件（100644 / 100755），且不等于 `svn/base:p` | 有东西可放回，放回的是文件内容 |
| 索引里 `p` 只有 stage 0，且等于 `HEAD:p` | 用户没暂存别的版本；写完后磁盘 = 索引 = HEAD，状态干净 |
| 磁盘上 `p` 是普通文件，按 `git add` 的过滤规则算出的 hash 等于 `svn/base:p` | 磁盘就是 SVN 的版本，没有用户自己的编辑 |
| `svn/base..HEAD` 里改过 `p` 的 commit 至少一个，而且全是 Local-Only | 分支比 SVN 多出来的只有 Local-Only 改动 |

### 不做

- 不自动写：跟进不写磁盘的不变量保留，用户点了才写。
- 删除型的 Local-Only（`HEAD` 里没有 `p`）不处理，照旧只报告。
- 自动跟进不碰：它没有报告弹窗。

## Task

| Task | 内容 |
|---|---|
| T1 | engine：`restorable_paths`（只读）+ 跟进报告加 `restorable` |
| T2 | engine：`restore_local_only`（重验 + 写回），fixture 集成测试 |
| T3 | command + store `invoke` |
| T4 | 前端：纯逻辑进 `src/engine/`，报告弹框加按钮；vitest、组件测试、layout 场景 |
| T5 | E2E 清单补场景 |

验收：fixture 复现本例（Local-Only commit 加一行 → 普通 commit 删掉 → 只把这个文件提交到 SVN → 跟进）。点按钮后磁盘带回这一行；磁盘上有用户编辑时不写。

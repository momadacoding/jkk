# 发布流水线（图标 / LICENSE / release workflow）— 决策日志

> 配套 `2026-08-09-release-readiness-decisions.md`（Task 1，会话还原）。
> 那份记在 `worktree-session-restore` 分支上，本分支从 master 切出，
> 两份分开放是为了避免同路径文件产生合并冲突。

## 进度总账

| # | Task | 状态 | 产出 | 验证 |
|---|---|---|---|---|
| 3 | 替换默认 Tauri 图标、补 LICENSE | 已完成 | `scripts/make-icon.ps1`、`src-tauri/icons/*`、`LICENSE` | 16–256px 逐尺寸目视；`pnpm tauri build` 产物 |
| 4 | release workflow | 已完成 | `.github/workflows/release.yml` | YAML 解析通过；`clippy -D warnings` 与 `pnpm tauri build` 本地实跑 |
| 5 | 打 v0.1.0 | 待 PR 合并后执行 | — | — |

「仓库转 public」一项**取消**：用户选择保持 private。

---

## Task 3 决策条目

### 偏离计划

- 计划里 Task 3 含「仓库转 public」，用户明确选择**保持 private**，该项从范围里去掉。
- LICENSE 选的是**专有声明**（保留所有权利），不是最初推荐的 MIT。

### 计划的错误

无配套 plan 文件。

### 计划没写的坑

1. **`pnpm tauri icon` 会顺带生成 iOS / Android 整套图标。**
   本项目只做桌面端（没有 `src-tauri/gen/`），那是 40 多个一个也用不上的文件，
   而且每次重跑都会重新冒出来。已加进 `.gitignore` 而不是删掉——删掉下次还会有。

2. **crate 声明的 license 不等于它实际分发的东西的 license。**
   `git2` 和 `libgit2-sys` 的 Cargo 元数据都写着 `MIT OR Apache-2.0`，
   照抄就会把 LICENSE 写错。`libgit2-sys` **vendor 了 libgit2 的 C 源码**，
   实测 `~/.cargo/registry/src/.../libgit2-sys-0.18.7+1.9.6/libgit2/COPYING`
   是 GPL-2.0 + 链接例外。元数据那行只覆盖 Rust binding。
   同理 `tao` 是 Apache-2.0 **单许可**，不是 `MIT OR Apache-2.0`。
   LICENSE 里的每一条都按 `cargo metadata` 和实际 COPYING 文件核过。

3. **「jkk」三个字母在 16px 下必然糊。**
   逐尺寸渲染在 `#202020`（任务栏底色）上看过：32px 起可读，24px 勉强，
   16px 只是一块有辨识度的蓝色形状。`tauri icon` 从单一源图派生所有尺寸，
   要让 16px 也清楚就得为小尺寸单独做一版源图。当前接受这个取舍。

### 为什么这么改是安全的

- 图标与 LICENSE 都不含可执行逻辑，不影响任何 git 操作路径。
- 图标全套由 `source.png` 派生且脚本进了仓库，改坏了可以一条命令重来。

---

## Task 4 决策条目

### 偏离计划

- 没有用官方的 `tauri-apps/tauri-action`。它一个 action 就把「构建 + 建 Release +
  传产物」全包了，但也把这几步藏进了黑盒。这里用 `pnpm tauri build` +
  `gh release create` 手写，步骤都看得见，出问题能直接对着日志改。
  代价是多写二十行 YAML。

### 计划没写的坑

1. **PowerShell 的 here-string 放不进 YAML 块标量。**
   `run: |` 要求每一行都带缩进，而 here-string 的结束符 `"@` 必须顶格，
   两者直接冲突——整个 workflow 会在 `YAMLParseError: Implicit keys need to be
   on a single line` 上炸掉，而且**是整份文件解析失败**，不是那一步失败。
   已改成单引号字符串数组 `-join "\`n"`。
   这一条是实际踩到的：第一版写完用 `pnpm dlx yaml@2` 解析当场报错。

2. **workflow 的存在会让 `AGENTS.md` / `CLAUDE.md` 里「没有 CI」那句话变成假的。**
   两处都已改成「没有 **PR 门禁** CI」，并写明这个 workflow 只在
   `v*` tag 或手动触发时跑，`push` / `pull_request` 都不触发。
   措辞要精确：说成「有 CI 了」会让人以为 PR 被验证过，那是更糟的错误。

3. **CI 的 clippy 比本地严。**
   workflow 用 `-D warnings`，本地约定只是 `cargo clippy --all-targets`。
   这意味着一条本地只是警告的 lint 会让**发版**失败。
   本地按 CI 的写法实跑过一次（exit 0），且 `AGENTS.md` §7.3 本来就要求
   把 clippy 告警处理掉，所以保留 `-D warnings`。

4. **`setup-node` 的 `cache: pnpm` 必须排在 `pnpm/action-setup` **之后**。**
   顺序反了会报找不到 pnpm。当前顺序已是 action-setup → setup-node。

5. **Release 建成 draft。** 产物传上去之后由人过目再点发布，
   避免一次手滑的 tag 直接对外可见。

### 为什么这么改是安全的

- workflow 的触发条件只有 `v*` tag 和手动触发，不会因为日常 push 而消耗额度。
  仓库是 private，Actions 按分钟计费（Windows runner 2 倍系数），
  限制触发条件在这里既是行为正确性也是成本控制。
- `permissions: contents: write` 是建 Release 所需的最小权限，没有开 packages / id-token。
- 发布步骤带 `if: startsWith(github.ref, 'refs/tags/v')`，
  手动触发时只构建、不会误建 Release。

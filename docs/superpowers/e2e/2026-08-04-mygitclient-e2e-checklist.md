# mygitclient MVP E2E 验证清单

## 环境
- 仓库：`C:\Users\liuhao\workspace\dev\Dev\mygitclient\.worktrees\feat-mvp`
- 启动：`pnpm tauri dev`（Vite 5174 + Tauri app）
- 验证仓库：`C:\Users\liuhao\workspace\dev\Dev\mygitclient`

## 启动与渲染
- [ ] `pnpm tauri dev` 成功启动，无编译错误
- [ ] 窗口标题为 `mygitclient`
- [ ] 顶部 header 显示当前仓库路径与 HEAD 分支名
- [ ] 左侧 sidebar 显示 Branches 与 Worktrees 面板
- [ ] 中间 Graph 区域显示提交图（SVG 线条 + 节点）
- [ ] 右侧/底部 Staging 区域显示文件列表与 diff

## 仓库打开
- [ ] 启动后自动打开 `C:\Users\liuhao\workspace\dev\Dev\mygitclient`
- [ ] 空仓库时显示错误提示而非无限 Loading
- [ ] 大仓库时提交图最多加载 5000 条，UI 不卡死

## 提交图交互
- [ ] 点击 commit row 可切换选中状态
- [ ] 选中后 bottom diff 面板显示该提交 diff
- [ ] refs 标签（branch/tag/head）正确显示在 commit row 上
- [ ] 鼠标悬停 commit row 有高亮

## 分支管理
- [ ] Local 分支列表与 `git branch` 一致
- [ ] 当前 HEAD 分支高亮显示
- [ ] 点击分支名执行 checkout，header HEAD 更新
- [ ] 输入新分支名点击 Create 后，分支出现在列表中
- [ ] 点击 × 删除非当前分支后，列表更新

## Worktree 支持
- [ ] Worktree 列表与 `git worktree list` 一致
- [ ] 当前工作区路径高亮
- [ ] 点击 worktree 行切换到对应工作区
- [ ] 点击 + New worktree 创建新的 worktree
- [ ] 非 main worktree 可删除

## Staging / Diff / Commit
- [ ] 修改工作区文件后，Unstaged 列表出现对应文件
- [ ] 点击 Unstaged 文件显示 diff
- [ ] 勾选 Unstaged 文件执行 stage 后，文件移动到 Staged
- [ ] 勾选 Staged 文件执行 unstage 后，文件回到 Unstaged
- [ ] 输入 message 点击 Commit 后，新提交出现在 Graph 顶部
- [ ] Amend 勾选后执行 amend，HEAD commit message 更新

## 自动刷新
- [ ] 在终端对同一仓库执行 `git commit` 后，Graph 自动刷新
- [ ] 执行 `git checkout` 后，header HEAD 自动更新
- [ ] 短时间内多次变更只触发一次刷新（debounce 300ms）

## 边界情况
- [ ] merge/rebase/cherry-pick 进行中时顶部显示状态横幅
- [ ] 操作失败时底部/顶部显示错误提示，可关闭
- [ ] 切换 worktree 后 watcher 跟随新路径

## 性能
- [ ] 提交图在 1000 commits 内滚动流畅
- [ ] SVG viewport clipping 只渲染可见区域
- [ ] 大量 refs 时 commit row 不溢出布局

## 回归
- [ ] `cargo test` 全部通过
- [ ] `pnpm test` 全部通过
- [ ] `pnpm exec tsc --noEmit` 无类型错误
- [ ] `cargo check` 无错误

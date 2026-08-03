# mygitclient 设计文档 — Git 客户端（Graph 专家版 MVP）

> 日期：2026-08-04
> 状态：已确认设计，待实现
> 技术栈：Tauri 2 + Rust + React + TypeScript

## 1. 项目背景与目标

### 痛点

用户目前的 Git 工作流是 **Fork + Sublime Merge 组合**：

- Fork：worktree 支持好、Ctrl+P 命令面板好、编辑器与终端整合好——但 **commit graph 很差**
- Sublime Merge：commit graph 极佳——但需要两个工具切换
- GitKraken：满足所有需求，但收费，免费版不支持私有仓库

### 目标

构建一个自用的 Git 图形客户端，**commit graph 达到 Sublime Merge 级别**，覆盖日常操作闭环，替代 Fork + Sublime Merge 组合。定位**先个人后开源**：先做个人可用，架构上预留开源扩展余地。

## 2. 范围

### MVP（Graph 专家版，约 6-7 周）

1. **Commit graph 可视化**（核心价值，对标 Sublime Merge 质量）
   - 布局与可视化：紧凑 lane 分配、分支线颜色、优雅的合并/分叉连接线、分支/标签/HEAD/远程指针标记
   - 指针状态显示：当前 checkout 分支高亮、worktree 关联分支、本地 vs 远程分支区分
   - 交互体验：悬停、点击选中联动、键盘导航、右键菜单
   - 性能基线：5 万 commit 内流畅（视口裁剪）
2. **Commit 详情 + 行级 diff**
3. **分支操作**：checkout / 新建 / 删除 / 重命名 / 从此 commit 创建分支
4. **Staging + Commit**：unstaged/staged 两栏、逐文件行级 diff、commit message、Amend
5. **Worktree 管理**：创建 / 切换 / 删除，graph 上显示 worktree 关联分支

### 明确不做（MVP 之外，用 Fork 顶替）

- push / pull / fetch（含远程认证、SSH 代理）
- Ctrl+P 命令面板
- 终端集成、编辑器集成
- 字符级 diff、冲突解决 UI
- rebase / merge / cherry-pick / stash（仅展示进行中状态，不提供操作）
- 浅色主题（预留 token 体系）
- 标签编辑、子模块、blame、历史搜索

## 3. 技术栈与关键决策

| 决策 | 选择 | 理由 |
|---|---|---|
| 桌面框架 | **Tauri 2 + Rust** | 包体小、内存低、性能好；直接集成 libgit2 |
| 前端 | React + TypeScript | 用户已有 TS 技术栈 |
| git 读取 | **libgit2 (git2-rs)** | commit 图、diff 高性能读取（GitKraken 同路线） |
| git 写入 | **git CLI** | rebase/merge 等写操作行为与 git 100% 一致，规避 libgit2 写操作缺口 |
| graph 渲染 | **自研布局 + SVG**（渲染器可替换） | 布局纯函数与渲染解耦；SVG 开发快、交互天然、可调试；预留 Canvas 替换接口 |
| Rust 经验 | 零基础 | 计划内置学习阶段；Rust 侧保持无状态命令式薄层 |

## 4. 架构总览

```
┌────────────────────────────────────────────────────────┐
│  React UI 层 (TS)                                       │
│   ├─ GraphView    —— SVG 渲染器 + 视口裁剪              │
│   ├─ StagingView  —— 工作区/暂存区 + 提交面板           │
│   ├─ CommitPanel  —— commit 详情 + 行级 diff            │
│   ├─ BranchView   —— 分支列表 + worktree 列表           │
│   └─ store        —— 仓库状态模型（单一数据源）         │
└──────────────┬─────────────────────────────────────────┘
               │ Tauri IPC（invoke 命令 / 事件推送）
┌──────────────▼─────────────────────────────────────────┐
│  Rust git 引擎层                                         │
│   ├─ repo.rs   —— 打开仓库、libgit2 打开/发现            │
│   ├─ graph.rs  —— revwalk：commit 拓扑序列 + 父链        │
│   ├─ refs.rs   —— 分支/标签/HEAD/worktree 指针读取       │
│   ├─ diff.rs   —— libgit2 生成结构化行级 diff            │
│   ├─ worktree.rs —— worktree 管理（CLI）                 │
│   └─ ops.rs    —— 写操作：commit/checkout/branch（CLI）  │
└──────────────┬─────────────────────────────────────────┘
               │ std::process::Command
┌──────────────▼─────────────────────────────────────────┐
│  git CLI (写操作)    git 仓库 (.git 目录)               │
└────────────────────────────────────────────────────────┘
```

**关键设计决策：**

1. **Rust 侧无状态命令式 API**：每个 invoke 命令独立、无全局状态，仓库句柄按需打开（`Repository::open` 开销极小）。避免新手期处理 unsafe 和长生命周期引用。
2. **数据流**：打开仓库 → libgit2 全量 walk（含 refs 指针）→ 紧凑 JSON（commit id、父链、作者、时间、消息、分支指针）→ 前端 store 持有 → 布局引擎纯函数 → SVG 渲染。
3. **刷新机制**：后端监听 `.git` 目录变化，事件推送 → 前端增量更新（保留滚动位置和选中态）。
4. **核心纯函数前置**：lane 布局、指针解析、diff 数据转换全部是 TS 纯函数，可脱离 Tauri 用 Vitest 测试。

## 5. Graph 布局算法

### 输入数据

```ts
interface CommitNode {
  id: string
  parents: string[]        // merge 提交可多个
  author: string
  time: number             // 秒级时间戳
  message: string
  refs: RefPointer[]       // 分支/标签/HEAD/远程指针
}
```

### Lane 分配算法（纯函数，核心）

```
遍历拓扑序 commits，维护"活跃 lane"数组：
  1. commit 归属：若某 lane 的头是这个 commit → 该 lane 归它
  2. 父继承：主父 commit 沿用当前 lane（直线向下）
  3. Merge 提交：主父保持当前 lane，次要父分配到空闲 lane
  4. 分叉点：新分支占新 lane，画分叉标记
  5. 分支结束：该 lane 无后续 commit → 释放（线在此处收尾）
  6. 无活跃分支的行 → 不渲染线（lane 宽度 0，保持紧凑）
```

### 绘制规则

- **合并线**：merge 提交的次要父线，从圆点向左上斜拐进其 lane，圆角路径
- **颜色**：按 lane 分配，高区分度 12 色调色板；同一条拓扑线的所有线段保持同色
- **指针标记**：commit 右侧小标签——本地分支实色填充、远程分支描边透明底、HEAD 加粗 + 星标
- **紧凑性**：无分支段不占 lane 宽度；多分支时 lane 按需增长

### 行布局（像素级约定）

```
[lane 区: 20px/lane] [圆点 10px] [消息...] [分支标签] [时间]
```

- 圆点：中空圆（白边 + 分支色填充）；HEAD 所在 commit 圆点加大加粗
- 选中行：圆角矩形高亮覆盖整行，圆点不变形
- 分离 HEAD：HEAD 标记虚线框样式

### 性能

- 布局 O(n) 纯函数：1 万 commit ≈ 10-30ms，空闲帧计算
- 视口裁剪：SVG 只渲染可见行（~100 行窗口 + 缓冲），DOM 节点恒定

## 6. UI 布局

单窗口三区布局（融合 Sublime Merge 形态 + Fork 左栏习惯）：

```
┌───────┬─────────────────────────────┬──────────┐
│ 左栏   │   Commit Graph（核心，最大）│ 右栏     │
│ 分支   │   • 视口裁剪 SVG 渲染       │ Commit   │
│ 列表   │   • ↑↓ 键盘导航            │ 详情     │
│ (本地/ │   • 右键菜单               │ (作者/   │
│  远程) │                            │  消息/   │
│       │                            │  文件列表)│
│ Work  │                            │          │
│ tree  │                            │          │
│ 列表  │                            │          │
├───────┴─────────────────────────────┴──────────┤
│ 底部面板：Staging / Diff                        │
│   [工作区变更] [暂存区] [选中 commit 的 diff]   │
│   + 文件行级 diff 视图 + 提交输入框             │
└────────────────────────────────────────────────┘
```

- 左栏：分支列表（本地/远程分组，点击 checkout，右键新建/删除/重命名）+ worktree 列表（当前标记、关联分支、新建/删除/切换）
- 中央 graph：右键菜单 = checkout / 从此创建分支 / 复制 hash / 查看 diff
- 右栏 commit 详情：作者、时间、完整消息、变更文件列表（点击跳转 diff）
- 底部面板三态：工作区（staged/unstaged + diff + commit 输入 + Amend）、历史 diff、冲突（后续版本）
- 交互：↑↓ 移动选中、Enter 展开 diff、Esc 收起；分隔条可拖拽
- 提交成功 → graph 顶部插入新节点（保留滚动位置），不整体重载
- 主题：深色为主，颜色 token 集中在 CSS 变量

### Diff 视图（行级）

- 顶部文件列表（文件名 + 增删计数），点击切换
- 行级增删高亮（绿底 + 左侧粗竖线 / 红底 + 左侧粗竖线）、代码着色、改动块折叠
- 文件变更类型图标：M / A / D / R

## 7. 数据流与状态管理

### IPC 命令清单（全部无状态）

| 类别 | 命令 |
|---|---|
| 读 | `open_repo(path)`、`load_graph()`、`read_status()`、`read_diff(scope)`、`read_commit_detail(hash)` |
| 写(CLI) | `stage/unstage(paths)`、`commit(msg, amend)`、`checkout(rev)`、`create/delete/rename_branch`、`create/delete_worktree` |
| 事件(Rust→前端) | `refs:changed`（refs/HEAD 变化）、`status:changed`（工作树变化） |

### 刷新机制

```
Rust 侧：notify crate 监听
  ├─ .git/refs/ 、.git/HEAD 、packed-refs  → refs:changed
  └─ 工作树根目录                          → status:changed
        │ 300ms 防抖合并
        ▼
前端按事件类型分流刷新：
  refs:changed     → 重拉 load_graph（保留选中 + 滚动位置）
  status:changed   → 重拉 read_status（staging 面板原地更新）
自己发起的写操作  → 操作成功立即定向刷新（不依赖监听，避免竞态）
```

### Store 数据模型

```ts
interface RepoState {
  path: string
  head: string                    // commit hash
  commits: CommitNode[]           // 拓扑序
  refs: Map<string, string>       // 分支名 → hash
  worktrees: Worktree[]
  status: { staged: FileChange[]; unstaged: FileChange[] }
  selection: string | null
}
```

### 错误处理

- Rust 错误统一 `Err(code, message)`，前端顶部提示条展示
- CLI 写操作失败：捕获 stderr 原文 + 建议操作（如冲突提示"工作树有冲突"）
- 每次 graph 重载带 seq 号，过期结果丢弃（防竞态）

## 8. 边界情况

| 场景 | 处理 |
|---|---|
| 打开非仓库目录 | 提示"初始化 Git 仓库?"（git init） |
| 合并/rebase/cherry-pick 进行中 | graph 顶部横幅提示 + 底部面板展示冲突文件 |
| 分离 HEAD | HEAD 标记虚线框样式，右键"在此创建分支" |
| 空仓库（无 commit） | 空态引导页：创建第一个 commit |
| >50k commits | 全量加载 + 进度条；>200k 降级为最近 5000 条 + 提示 |
| 操作竞态 | seq 号机制丢弃过期结果 |
| 中文/非 UTF-8 路径 | libgit2 原始 bytes → UTF-8 解码，非法编码显示替换符 |
| git 不在 PATH | 启动时检测，提示安装 |
| 写操作失败 | 统一 GitError，提示条展示 stderr + 建议 |
| 远程指针 | 直接读本地 refs，无需网络（push/pull 不在 MVP） |

## 9. 测试策略

```
Vitest（TS 纯函数，主战场）
  layout：lane 分配/合并路径/颜色稳定
  视口裁剪：窗口边界、空数据、单行
  指针解析：refs → 标签渲染数据

cargo test（Rust 引擎层）
  fixture 仓库（测试内 git init + 造图）：
  线性/分叉/合并/空仓库/分离 HEAD
  graph.rs 输出与预期拓扑比对

手动 E2E 清单（MVP 阶段）
  打开→提交→分支→worktree→切换 全流程
```

## 10. 里程碑与 Rust 学习路径

### 里程碑（6-7 周）

| 里程碑 | 内容 | 时长 | 产出 |
|---|---|---|---|
| M0 骨架 | Rust 入门 → Tauri 2 最小应用，invoke 往返跑通 | ~1 周 | hello 窗口 |
| M1 git 引擎 | git2-rs：open_repo / load_graph 返回真实 commit 拓扑 JSON | ~1.5 周 | 命令 + fixture 测试 |
| M2 Graph 渲染 ⭐ | 布局纯函数 + SVG 渲染器 + 视口裁剪 + 交互 | ~2 周 | 对齐截图的 graph |
| M3 操作闭环 | staging/commit + 行级 diff + 分支/worktree + 刷新机制 | ~2 周 | 完整 MVP |
| M4 打磨 | 边界情况、错误处理收尾、手动 E2E 清单 | ~1 周 | v0.1 可日常使用 |

### Rust 学习路径

```
第 3-6 章  所有权/结构体/枚举       ← 必读
第 7 章    模块组织                ← 必读
第 9 章    错误处理 (Result/?)     ← 必读，写 git 引擎的前提
第 10/17 章 泛型/trait            ← 了解即可
第 13 章   迭代器/闭包             ← 写 layout 前再看
其余章节    跳过，按需查阅
并行：Tauri 快速入门 + git2-rs revwalk 示例照抄改写
```

MVP 的 Rust 代码模式高度重复（每个 handler = 打开仓库 → git2-rs → 序列化 → 返回），第 9 章学完即可开写；复杂算法全部在 TS 侧，避开 unsafe 与生命周期。

## 11. 风险与应对

| 风险 | 应对 |
|---|---|
| lane 算法效果不达预期 | 纯函数快速迭代，先基础版再对齐截图，不阻塞其他部分 |
| Rust 卡壳 | Rust 侧最小化 + git2-rs 文档照抄，社区答案多 |
| SVG 性能不足 | 渲染器接口预留 Canvas 替换，布局算法不动 |

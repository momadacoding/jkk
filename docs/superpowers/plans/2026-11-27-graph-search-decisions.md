# 提交图搜索 — 实现日志

用户原话：「现在 jkk 缺少搜索功能啊」。没有单独的计划文件：按 [功能需求直接开工] 的约定，范围与取舍直接记在这里。

## 范围（第一版）

- 提交图顶部常驻一条搜索栏；`Ctrl+F` 聚焦，`Enter` / `F3` 下一个，`Shift+Enter` / `Shift+F3` 上一个，`Esc` 先清空、再按一次离开输入框。
- 匹配字段：提交标题、作者、hash 前缀（≥4 位）、指向它的分支 / 远程 / tag 名；SVN 节点额外认 `r1234` / `1234`，以及收在它里面的已交付本地提交的标题、作者、hash。
- 多个词用空格分开，全部命中同一条提交才算（AND），大小写不敏感。
- 命中的行高亮、其余行变暗，**不过滤掉行**；计数显示「第 i / 共 n」。
- 输入时 150ms 防抖后跳到「当前选中处及之后」的第一个命中（到底就绕回顶部）。

## 进度总账

| Task | 状态 | 产出 | 验证 |
|---|---|---|---|
| 1 匹配与跳转的纯逻辑 | 完成 | `src/engine/commitSearch.ts`、`tests/vitest/commitSearch.test.ts` | 先失败（模块不存在），实现后 20 条通过 |
| 2 GraphView 搜索栏 | 完成 | `src/components/GraphView.tsx`、`src/components/CommitRow.tsx`、`src/styles.css` | `pnpm build` 通过 |
| 3 组件测试 | 完成 | `tests/vitest/GraphView.test.tsx`（新增 12 条） | `pnpm test` 107 个文件 / 1998 条全过 |
| 4 真实窗口 E2E | 完成 | scratch 里的 40 条提交 fixture 仓库 + `pnpm tauri dev`（独立 identifier、5175 端口，5174 被并发会话占着），CDP 发真实按键 | 9/9：Ctrl+F 聚焦、fix 命中 5 条并自动跳、Enter / Shift+Enter、`fix bob` 多词、tag、分支名、7 位 hash、无匹配、Esc 两段；截图确认高亮 / 变暗、图线不断 |
| 5 独立 review 修复 | 完成 | 见下方「独立 review」 | 新增 14 条测试先失败后通过；`pnpm build` 过；`pnpm test` 107 个文件 / 2012 条全过；E2E 重跑 10/10（多了「切到 Local Changes 再回来」） |

## 决策

### 总体方案取舍

| 方案 | 做法 | 为什么没选 / 选了 |
|---|---|---|
| A（选） | 前端在已加载的提交上匹配，高亮 + 跳转 | 零后端改动、即时；图不断线 |
| B | 命中之外的行过滤掉 | 父子关系被切断，图上的线没法画对；Fork 也是高亮跳转 |
| C | 新增后端 `search_commits`（`git log --grep`，搜正文、超出 5000 条） | 要新 Tauri 命令和分页；第一版先不做，见「遗留」 |

### 计划没写的坑

- 图最多只加载 5000 条（`src-tauri/src/engine/graph.rs` 的 `limit`），`message` 只有标题（`summary()`）。所以第一版搜不到正文、也搜不到更早的提交；搜索栏的提示文字里写明了这一点，不让人误以为「没搜到 = 不存在」。
- hash 只按前缀、且至少 4 位才参与匹配：任意子串匹配时输入 `a` 几乎命中每一条，`add` 这样的英文词也全是十六进制。
- 已交付、收进 SVN 节点的本地提交不在图上（`deliveredFrom`），命中它们时跳到收着它的 SVN 节点。
- 跳到的命中就是当前选中的那条时不再 `onSelect`（会白读一遍 diff），改发一次组件内的定位请求；
  **换了选中时不发**：store 晚一拍更新 `selectedId` 的话，定位 effect 会先对着旧选中跑一遍，画面先跳到旧的那条。
- 自动跳转只跟着输入走（effect 依赖只有搜索词），图刷新、用户点了别的提交都不把选中拽回命中上；有组件测试钉住。
- SVG 节点和文字行都带 `data-id`，组件测试里按 `data-id` 找行会先拿到 SVG 节点，要加 `.commit-row` 限定。

### 为什么这么改是安全的

- 只读：不新增 Tauri 命令、不碰任何 git 写操作；Rust 侧零改动。
- `GraphView` 只新增两个 props（`searchQuery` / `onSearchQueryChange`），`RepoTab` 只多一个 `useState`；不涉及 store、不涉及 invoke。`.graph-view` 仍是滚动容器，外面多套一层 `.graph-pane`，现有的定位 / 键盘测试只补了这两个 props、断言没改。
- 方向键处理本来就跳过输入框里的按键，焦点在搜索框时 ↑ / ↓ 不会换选中。

### 交互细节的取舍

- 搜索栏常驻而不是 Ctrl+F 才弹出：用户的反馈是「缺少搜索功能」，说明入口要看得见。代价是图少一行高度（约 26px）。
- 输入防抖 150ms 再跳：每跳一次都会读那条提交的 diff。高亮和计数不防抖，即时更新。
- 第一版不在标题里标出命中的子串：整行高亮已能看出哪条命中，标子串要改 `CommitRow` 的标题渲染（还有混合基线角标那条分支），留到有需要时再做。

### 独立 review（`/code-review high`，fork 出的独立 agent）

| 条目 | 级别 | 处理 |
|---|---|---|
| 第 1 条：搜索框的 Enter / Esc 没排除输入法组词，打拼音上屏会跳命中、取消候选会清空搜索词 | important | 修。按键判定下沉为纯函数 `inputSearchKey`，组词和自动重复一律不管 |
| 第 2 条：按住 F3 的自动重复每一下都换选中、读 diff | nit | 修。`windowSearchKey` 排除 `repeat` 与组词；Enter 同样排除自动重复 |
| 第 3 条：搜索词放在 GraphView 里，切到 Local Changes / 文件历史再回来就没了 | nit | 修。提到 `RepoTab`（按标签各一份），GraphView 改为受控；重新挂载时不自动跳 |
| 第 4 条：只多打空格或改大小写也会重跑自动跳转，把滚走的画面拽回去 | nit | 修。自动跳转按「词」的字符串判断，不按数组引用 |
| 第 5 条：Ctrl+F 只认 `key === 'f'`：非拉丁布局失效、Ctrl+Shift+F 也被占、会从别的输入框抢焦点 | nit | 修。照 `isSelectAllShortcut` 退回看 `code`；排除 Shift / Alt；焦点在别的输入框时不抢 |
| 第 6 条：选中的命中和鼠标停着的命中底色一样 | nit | 修。选中的命中加一圈描边、底色更深，悬停的变浅 |

### 遗留

- 后端全文 / 全历史搜索（方案 C）。
- 按文件路径搜提交（相当于文件历史，已有「文件历史」入口）。

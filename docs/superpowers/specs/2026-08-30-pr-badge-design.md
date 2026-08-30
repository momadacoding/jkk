# 分支上的 PR 编号徽章 设计规格

- 日期：2026-08-30
- 状态：已实现（PR #37，分支 `worktree-pr-badge`）
- 相关：`docs/superpowers/e2e/2026-08-30-pr-badge-e2e.md`

## 1. 目标

在侧边栏分支列表和 graph 的分支标签上，标出该分支对应的 GitHub Pull Request 编号，
让「这条分支开没开 PR / 合没合」不必切到浏览器才能知道。

两类信息同等重要：

| 状态 | 回答的问题 | 为什么需要 |
|---|---|---|
| open / draft | 这条分支正在走 review 吗 | 最初的需求 |
| merged | 这条分支还留着，但内容进主干了吗 | squash 合并后原分支不会自动消失，**比 open 更难从别处看出来** |

### 1.1 非目标

明确排除在本期之外，不要顺手实现：

- 点击徽章打开浏览器（`PullRequest.url` 已取回但前端未使用，是为这一步预留的）
- PR 的 review 状态、CI 状态、评论数
- 非 GitHub 托管（GitLab MR、Gitea）
- 应用内创建 / 合并 PR
- 对已合并分支给出「可以删了」的清理引导

## 2. 数据来源

### 2.1 为什么是 gh CLI

复用 `gh` 而不是自己调 GitHub REST API：token 存取、SSO、代理、企业版主机名全部由 gh
管理，本项目一行凭据代码都不用写，也不新增任何 HTTP 依赖（`serde_json` 已在
`Cargo.toml` 中）。代价是没装 gh 的机器上看不到徽章——见 §4 的降级约定。

### 2.2 查询

`src-tauri/src/engine/github.rs` 对每种状态各跑一次：

```bash
gh pr list --state open   --limit 100 --json number,headRefName,title,isDraft,state,url
gh pr list --state merged --limit 100 --json number,headRefName,title,isDraft,state,url
```

子进程环境（与 `git_cli::git_command` 同源的理由）：

| 设置 | 为什么必须 |
|---|---|
| `hidden_command` | release 构建带 `windows_subsystem = "windows"`，没有 console；gh 是 console 程序，不加 `CREATE_NO_WINDOW` 每次刷新都闪黑框。**dev 构建看不到这个缺陷** |
| `GH_PROMPT_DISABLED=1` | GUI 进程没有终端，gh 一旦转成交互式提问就会等 stdin，子进程永久挂死并占住 Tauri 线程池 |
| `GH_NO_UPDATE_NOTIFIER=1` | 否则 gh 顺手查一次自己的新版本，白白多一次网络往返 |

**为什么不用 `--state all`：** all 会把「关闭但未合并」的 PR 也捞回来，而那些对
「这条分支现在什么状态」没有信息量；更要命的是它们会占掉 100 条的额度——长期仓库里
废弃 PR 可能比合并的还多，真正要显示的反而被挤出去。分开查，每种状态各有自己的额度。

第一种状态就查不到时直接 `break`：那说明 gh 本身不可用，再为后面的状态各起一个注定
失败的子进程只是每次刷新都白付开销。

## 3. 刷新时机

```text
openRepo / fetch / pull / push  →  invoke('list_pull_requests')  →  store.pullRequests
```

**只在这四个时机拉取，绝不挂进 `repo-changed` 那条文件监听通道。** 那条通道一次编辑
就触发一次，而这里要走网络（实测一两秒）。调用点一律 `void` 不 await——徽章晚一两秒
出现无所谓，但不能让 push 的反馈被 gh 拖住。

`fetch` / `pull` / `push` 之外的工具栏动作（stash、cherry-pick 等）不碰远端，
不触发拉取。

## 4. 失败降级

**gh 侧的失败一律降级成空列表，不返回 `Err`。**

| 情况 | 行为 |
|---|---|
| 没装 gh（子进程起不来） | 空列表 + `log::info!` |
| 没登录 / 远端不是 GitHub / 没有远端（gh 非零退出） | 空列表 + `log::info!` |
| gh 成功退出但 JSON 解析不了 | `GitError { code: "gh_json" }`，前端 `console.warn`，**不弹错误横幅** |

理由：这是锦上添花的信息。让它冒泡到顶部错误横幅的话，真正的 git 错误反而看不见。

成功路径也记一条 `log::info!("gh pr list 读到 N 条 PR（open + merged）")`——所有失败
都是静默的，没有这行日志，「徽章为什么没出来」在装机版上完全无从下手，分不清是没读到
还是读到了 0 条。

## 5. 匹配与挑选规则

纯逻辑全部在 `src/engine/prs.ts`，由 `tests/vitest/prs.test.ts` 覆盖。

### 5.1 哪条 ref 配得上编号

`pullRequestForRef` —— **只有本地分支**（`kind === 'branch'`）。

远程分支被排除不是因为匹配不上：`origin/feat/x` 剥掉前缀后正好也是 PR 的 head 分支名，
反而会命中。排除是因为同一个提交上经常同时挂着 `feat/x` 和 `origin/feat/x` 两个标签，
两边都标就成了重复噪音。tag 和 HEAD 伪 ref 同理。

分支名匹配是**精确的，不做大小写折叠**：GitHub 分支名大小写敏感，折叠会把 `Feat/X`
的 PR 错挂到 `feat/x` 上。徽章指错 PR 比不显示徽章更糟。

### 5.2 一个分支对应多条 PR 时选哪条

`indexPullRequestsByBranch` 的规则，按优先级：

1. **CLOSED（关闭未合并）直接丢掉。** 被否掉的 PR 对当前状态没有信息量，标出来只会误导。
2. **OPEN 优先于 MERGED，哪怕编号更小。** 老分支合过一次又重开 PR 时，当下正在进行的
   那个才是用户关心的。
3. 同状态下取编号最大的，即最新的一条。这样也不依赖 gh 的返回顺序。

## 6. 视觉

### 6.1 两处形态

| 位置 | 形态 | 元素 |
|---|---|---|
| 侧边栏分支行 | 独立胶囊徽章，自带底色 | `.sb-pr` + `.open` / `.draft` / `.merged` |
| graph 分支标签 | **内嵌在分支 pill 内部**，共用底色 | `.ref-pill-pr` + 同样三个修饰类 |

graph 上不另起 pill：那要多一份内边距和间隙，而那一行还挤着提交信息、作者、hash 和日期。

### 6.2 颜色是算出来的，不是挑出来的

graph 里徽章的底色是**车道色的 22% 叠加**，12 条车道各不相同，选中行还要再叠一层
10% 白。任何配色都必须在 `12 车道 × {普通, 选中}` 共 24 种组合上成立。

最差的一条永远是黄色车道 `#fff176` 的选中行。实测：

| | 最初试的值 | 对比度 | 现值 | 对比度 |
|---|---|---|---|---|
| open | `#7ee08a` | 4.05 | `#9ae8a4` | **4.53** |
| draft | `#9aa4ae` | **2.60** | `#d0d8e0` | **4.57** |
| merged | — | — | `#e2d0fc` | **4.60** |
| （参照）分支名 | `#f0f4f8` | 5.96 | 同左 | 5.96 |

`#9aa4ae` 的 2.60 连次要信息的 3.0 都不到——**这是在单一车道上用眼睛看永远发现不了的
缺陷**，只有把 24 种组合全算一遍才暴露。改配色前请重算，不要凭观感判断。

### 6.3 merged 为什么带 `✓` 前缀

颜色在 graph 里**当不了 merged 的唯一区分手段**：

- 能过 4.5 对比度的紫已经淡到与 draft 灰分不开（`#dfcbfb` 只有 4.41，再深就更低）
- 试过给 merged 一个不透明小块以摆脱车道色影响，但小块底色与某些车道的 pill 底色
  **亮度完全一致**（对比度 1.00），边界会消失

所以由 `prBadgeLabel` 给 merged 加 `✓` 前缀（`✓#33`），颜色只作辅助。符号这一路对
色盲同样成立。侧边栏的底色由我们自己定、不受车道色摆布，紫可以更饱和一档
（`#c8a8f5` on `rgba(137,87,224,0.20)`，最差 5.08），但 `✓` 照打，两处形态一致。

## 7. 分层

```text
RepoTab (state.pullRequests)
  ├→ BranchView ──┐
  └→ GraphView ───┤  各自 useMemo 建一次索引
                  └→ CommitRow
                        ↓
                  engine/prs.ts   ← 纯逻辑，全部 vitest 覆盖
                        ↑ invoke
                  commands/pr.rs → engine/github.rs → gh CLI
```

`PullRequest` **刻意不挂进 `RefPointer`**：PR 走的是会失败、会超时的网络通道，塞进
graph 数据结构会让本地读图也背上网络状态。

## 8. 已知限制

| 限制 | 影响 | 取舍理由 |
|---|---|---|
| 每种状态各 100 条上限 | PR 极多的仓库里，靠后的分支没有徽章 | 翻页要多跑网络往返，而这只是个角标 |
| 只支持 GitHub | 其他托管平台无徽章 | gh 本身只支持 GitHub |
| 需要本机装了 gh 并登录 | 不满足时静默无徽章 | 见 §4 |
| 徽章不可点击 | 看到编号后仍需手动去浏览器 | 见 §1.1，`url` 已预留 |
| 不显示 CLOSED | 被否掉的 PR 不可见 | 见 §5.2 |

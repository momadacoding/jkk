# 弹框溢出防线：实现日志

没有对应的计划文件：范围由对话确定（PR #105 的 A + C 方案），按 AGENTS.md §2.1 落这份日志。

## 进度总账

| Task | 内容 | 状态 | 产出 | 验证 |
|---|---|---|---|---|
| 1 | Playwright 布局测试：配置、复现页、场景清单 | 完成 | `playwright.config.ts`、`tests/layout/` | 首跑 4 个框的按钮打开时点不到（红） |
| 2 | `.dialog-actions` 固定在框底 | 完成 | `src/styles.css` | 布局测试转绿；变异验证：删 `max-height` 后布局测试与守卫都变红 |
| 3 | 「丢弃改动」改用应用内确认框 | 完成 | `src/engine/discardConfirm.ts`、`RepoTab.tsx` | `discardConfirm.test.ts`、场景 `confirm-discard-long` |
| 4 | 守卫测试：场景覆盖、`askConfirm` 白名单、基类规则 | 完成 | `tests/vitest/dialogGuards.test.ts` | 先红（指向 `RepoTab.tsx`）后绿 |
| 5 | SVN 初始化向导实测 | 完成 | `src/styles.css`、`tests/layout/wizard.layout.ts` | 800×400 下按钮在 y=420（红），修后绿 |
| 6 | 文档：AGENTS / CLAUDE / README | 完成 | 三个文件 | 人工核对 |
| 7 | 处理 review（第二轮，10 条） | 完成 | 见下方「Review 2」 | 新测试先红后绿；全套门槛重跑 |

## 决策

### Task 1

- **偏离方案：** 方案里没提端口。最初写死 5199，改成 `--port 0` 让系统分配。
- **没写的坑：** 这台开发机的动态端口范围覆盖到五千多。`netstat` 看到 5199 正被别的进程当出站连接的本地端口，vite 报 `EACCES`（不是 `EADDRINUSE`，所以 vite 自带的换端口也不管用）。写死任何端口都可能撞上。`webServer.wait` 的命名捕获组写入 `HARNESS_PORT`，worker 重新加载配置时读到。
- **没写的坑：** 文件名不能用 `*.spec.ts`，会被 vitest 默认的 include 收走。用 `*.layout.ts` 加 `testMatch`，不用改 vitest 配置。
- **为什么这样是安全的：** 系统分配端口后，不可能连到别的 worktree 开着的服务器，不会出现「测了别人的代码还是绿的」。
- **选型：** `channel: 'msedge'` + 有头。§6.1 禁止 headless 做布局验证；WebView2 是 Edge 内核，比 Chrome 接近真实窗口，且 Windows 自带。不下载 Playwright 浏览器。
- **没写的坑：** 按钮「在视口里」不够，用 `elementFromPoint` 判断，被盖住也算够不着。

### Task 2

- **偏离方案：** 方案没提清单被压扁。review 说「压到约 100px」，实测是 **0**：滚动容器的自动最小高度是 0，长内容时 `.deliver-files` 被 flex 压没，交付确认框看不到要提交的文件。改动前就存在（原 `.deliver-dialog` 同样的 `max-height`）。按钮贴底后修法只要 `flex: none`，一并做了；上一轮的 PR 描述把它列为遗留问题，已更正。
- **没写的坑：** `.dialog` 的底部内边距要归给 `.dialog-actions`。留在 `.dialog` 上的话，贴底的按钮条下面会露出 14px，滚过去的内容从那里透出来。
- **保留 `.dialog-body`：** `ConfirmDialog` 的滚动区没有被贴底规则取代。它让错误和进度也留在滚动区外，普通弹框的错误仍在滚动内容里。

### Task 3

- **偏离方案：** 除了换入口，把文案抽成纯函数 `describeDiscard`（§7.2），并把「永久删除」的文件放进红色警告条，与能恢复的分开。
- **行为变化：** 默认按钮从原生框的默认变成 Cancel（`focusCancel`），回车不会丢东西；按钮标签变成 `Discard` / `Cancel`。
- **为什么这样是安全的：** `invoke('discard', ...)` 的调用与参数没动，只有确认来源变了。`confirm` 取消时返回 false，直接 return，与原来 `askConfirm` 的语义相同。原生框失败时按取消处理的行为不再需要：应用内框不会弹不出来。
- **验证范围：** 文案和布局有测试；`RepoTab` 里这几行没有端到端测试，未在 Tauri 真实窗口点过。
- **没验证的前提：** 「原生框放不下」是读 `rfd` 源码（`common-controls-v6` → `TaskDialogIndirect`）加经验的判断，没有实测原生框。这次改动不依赖它成立：不可逆操作的清单放进可滚动的框，本身就更好。

### Task 4

- **没写的坑：** 守卫读的是源码文本，只能拦写法，拦不住布局。规则靠 `dialog-overlay` 这个字符串识别弹框，换类名就会失效。真实布局由 `pnpm test:layout` 负责，它不在 `pnpm test` 里。

### Task 5

- **计划的错误：** 上一轮表格把交付和钩子标成「够得着」，实测打开时按钮点不到，要先滚动。向导当时标「未实测」，实测 800×600 正常、800×400 按钮出界且页面不能滚。
- **没写的坑：** 向导不是 `.dialog`，`.svn-wizard` 和 `.dlg-actions` 此前都没有样式。复现页要套上 `.app > .repo-tab` 才是真实容器。
- **验证范围：** 只量了「预检失败、错误很长」这一步。预检通过页和完成页的内容没有单独场景，靠的是同一条容器规则。

### Review 2（提交前，独立于实现的 /code-review）

| 条目 | 结论 | 处理 |
|---|---|---|
| 1 丢弃框：红条在长清单最下面，Discard 贴底不用滚就点得到 | 成立，贴底按钮引入的回归 | 首行写明「N 个未跟踪文件会永久删除」；`discardConfirm.test.ts` 先红后绿 |
| 2 获焦控件被贴底按钮条盖住 | 交付框的输入框实测**没被盖住**（布局测试新增断言，一直是绿的） | 仍加 `scroll-padding-bottom`：第 3 条的 `scrollIntoView` 需要它才停在按钮条上面 |
| 3 非确认框的错误 / 进度出在视野外 | 成立，贴底按钮引入的回归。新场景 `deliver-short-error`：800×400 下从顶部点交付，错误 viewport ratio 为 0 | 新组件 `Reveal`：出现时滚进视野；用在交付、删分支、删 worktree、钩子四个框 |
| 4 `toBeVisible` 不要求在视口内 | 成立 | 改用 `toBeInViewport`，并断言不在按钮条下面 |
| 5 关框后焦点落到 body，Del / Ctrl+A 失效 | 成立，丢弃改成应用内框后暴露 | `useFocusTrap` 关框时把焦点还给打开前的元素；首次渲染时记录，躲开 `autoFocus` |
| 6 `useConfirm` 单槽，新请求顶替丢弃框 | 成立但不处理 | 所有确认框共有的既有设计；顶替时丢弃不会执行，不丢数据 |
| 7 丢弃流程没在 Tauri 真实窗口验证 | 成立 | 无法在本会话点真实窗口，列入未验证项 |
| 8 `includes` 平方级 | 成立 | `describeDiscard` 与 `handleDiscard` 改用 `Set` |
| 9 文档把未实测的「TaskDialog 没有滚动区」当事实；守卫不查 harness | 成立 | 改写理由为「布局不归我们管、测不到」；守卫新增「每个场景在 harness 里有实现」 |
| 10 向导场景少了标签栏；`tests/layout` 没有类型检查 | 成立 | 复现页照 `App.tsx` 套上 `TabBar` 与 `.tab-panel`；新增 `tsconfig.layout.json`，`test:layout` 先跑 `tsc`。用探针文件验证过它能报出类型漂移 |

- **为什么焦点归还是安全的：** 只在焦点丢到 `body` 时归还，焦点已被下一个框接走时不抢；目标元素已从 DOM 移除时不动。

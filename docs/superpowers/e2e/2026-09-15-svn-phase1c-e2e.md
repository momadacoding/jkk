# Phase 1c E2E 验证清单

> 配套 `docs/superpowers/plans/2026-09-15-svn-phase1c-visible.md` 的 Task 14 Step 4。
>
> **本期新增 5 个 Tauri 命令并改了 `Capabilities` 的形状。**
> Tauri 的 `snake_case` → `camelCase` 参数名转换**没有编译期检查**（`AGENTS.md` §7.3），
> 写错只会在运行时静默拿到 `undefined`。集成测试是直接调命令函数本体的，
> **碰不到 IPC 那一层**，所以下面这些必须在真实窗口里跑。
>
> 2026-09-16 第二轮独立 review 之后，`svn_read_log` / `svn_fetch_log` 两个命令的
> **参数与返回形状都改了**（`beforeRev: number | null` → `older: boolean`，
> 返回 `SvnRevNode[]` → `{ revs, complete }`），场景 8 因此是眼下最该先跑的一条。

## 状态

| 项 | 状态 |
|---|---|
| 自动化验证（`cargo test` / `pnpm test` / `pnpm build` / `fmt` / `clippy`） | ✅ 已跑，见计划的 Task 14 |
| 下面 10 个真实窗口场景 | ⬜ **未执行** |

⚠️ **未执行的不要勾。** 1a 的 PR 里那两个复选框一直空着是对的——空着是如实，
勾上是谎报。

## 前置

```powershell
pnpm install          # 新 worktree 里必须先装，否则 pnpm test 会静默跑主工作区
pnpm tauri dev
```

⚠️ 场景 3 / 6 / 9 要发 SVN 写操作。按 `AGENTS.md` 开篇的最高优先级约束，
**必须在临时 fixture 仓库里做**，不得拿本仓库或任何真实仓库试。

### 造一个 fixture 仓库

脚本在 `scripts/e2e-svn-fixture.ps1`（完整内容以文件为唯一事实来源，
这里只写入口，`AGENTS.md` §10）。

```powershell
# 场景 1-7、9、10 用这个：服务器和 wc1 都在 r1
pwsh -File scripts/e2e-svn-fixture.ps1

# 场景 8 用这个：服务器走到 r230，而 wc1 **停在 r1**
pwsh -File scripts/e2e-svn-fixture.ps1 -Shape Deep
```

脚本最后会打出三样东西：在应用里要打开的 `wc1`、扮演同事的 `wc2`
（在那里提交 = 外部变更），以及删除整个 fixture 的命令。用完删掉。

两种形态都会造出这些文件，后三个是**故意**放的：

| 路径 | 作用 |
|---|---|
| `Code/A.cs`、`Code/Deep/D.cs` | 白名单内的代码，应当进 git |
| `Code/Art.psd` | 白名单内但扩展名不符 —— 场景 6 的漏收提示**不能**报它 |
| `Art/boss.psd` | 白名单外 —— 同上 |

向导里白名单填 `Code/`、扩展名填 `.cs`。

## 场景

- [ ] **1. 打开一个普通 git 仓库**（回归，**不能省**）

  本期改了 `Capabilities` 的形状。最容易出的回归是普通 git 仓库被误判成 SVN，
  然后每次刷新都发几个必然失败的 invoke。

  预期：无失同步横幅、图上没有 svn 节点、提交框**没有**「仅本地」勾选框、
  图底部**没有**「加载更多 SVN 历史」按钮、DevTools 控制台无 SVN 相关报错。

- [ ] **2. 打开 fixture 的 `wc1`，走完初始化向导**

  预期：向导正常完成；随后图上出现 `r1` 的 svn 节点（配色与 git 提交不同、
  hash 位置显示 `r1`）；无横幅。

- [ ] **3. 外部提交一个深层文件后回到窗口**（用 Simple 形态）

  `<fixture>` 是脚本最后打出来的那个根目录。

  ```powershell
  Set-Location "<fixture>\wc2"
  svn update --non-interactive
  Set-Content "Code\Deep\D.cs" -Value 'class D { int y; }' -NoNewline
  svn commit -m "r2 外部改深层文件" --non-interactive
  ```
  再回到 wc1 窗口，随便存一下某个文件触发 `repo-changed`。

  预期：**不**出现横幅（wc1 还没 update，nodeRevs 仍是 1 == baseline）。
  然后在 wc1 里 `svn update`，再触发一次刷新 → 出现 danger 横幅，
  判成「基线落后于工作副本」，列出 r1 / r2。

- [ ] **4. 点「重建基线」**

  预期：先弹确认框，写明「不会修改工作目录、不会推进工作分支」；
  确认后横幅消失；`git status` 干净、当前分支未变、工作目录文件未变。

- [ ] **5. 点「体检基线」**

  预期：弹出体检结果。三态都要试到：
  - 正常时 → `基线已通过体检（rN）`
  - 断网 / 把 `<fixture>\repo` 临时改名 → `基线未经体检`，并说明原因
  - 手改一个白名单内的 `.cs` 再 commit 到 git（不 svn commit）→ 应当仍是
    `NotChecked`（磁盘与基线树不一致，前提不成立），**不是** `Lying`

- [ ] **6. depth-exclude 一个白名单子目录**

  ```powershell
  Set-Location "<fixture>\wc1"
  svn update --set-depth=exclude --non-interactive -- "Code/Deep@"
  ```

  ⚠️ `Code/Deep@` 末尾那个 `@` 不能省：SVN 把最后一个 `@` 之后的内容当
  peg revision 解析，加一个空的后缀才是「这个路径本身」。

  预期：**失同步横幅不出现**（判据看不见它，这是设计如此），
  但**漏收提示出现**，点名 `Code/Deep/D.cs`，并说明 git 没在保护它。

- [ ] **7. 断网后打开仓库**

  把 `<fixture>\repo` 临时改名，模拟服务器不可达。

  预期：图仍能从缓存画出来；**不弹错误横幅**；控制台有一条 `console.warn`。
  漏收提示不出现（`NotChecked` 不打扰用户）。

- [ ] **8. 点「加载更多 SVN 历史」**（本轮改动最大，**优先跑**）

  用 `-Shape Deep` 的 fixture（服务器 r230、wc1 停在 r1），在 wc1 上走完初始化。

  `PAGE_SIZE` 是 200，所以这个形态下每一步的预期是**可以精确对数**的：

  | 步骤 | 图上应当有 | 「加载更多」 |
  |---|---|---|
  | 刚初始化完 | r230…r31（首屏 200 条）**加一个孤零零的 r1**（基线靠锚点补进来） | 可点 |
  | 点一次「加载更多」 | r230…r1 **连号无缺口** | 变灰，显示「已到最早的 revision」 |

  ⚠️ **要看的是中间那段 r2..r30 有没有被填上**，不是只看按钮。
  第二轮 review 的 A2 正是「游标取自含锚点的并集」：从 r1 往下翻 →
  一条命令都不发 → 按钮直接变灰，而 **r2..r30 永远拿不到**。
  只盯按钮的话，坏的和好的都表现为「点一下就变灰」，完全分不出来。

  对数的办法：随便点开一个 SVN 节点看 revision，或者数一下图上 SVN 节点的总数
  （修好了应当是 230 个）。

- [ ] **9. 勾上「仅本地，不交付」提交**

  预期：勾上后下方立刻出现黄色警告，写明「交付流程要到 Phase 3 才会读它」；
  提交后图上那一行出现「仅本地」角标；
  `git log -1 --format=%B` 能看到 trailer **单独成段**。

- [ ] **10. checkout 到 `svn/base`**

  预期：侧边栏里 `svn/base` 带「基线」标记、右键菜单的 Delete 置灰；
  能 checkout 上去；切上去之后 Local Changes 的 Commit 按钮禁用、
  下方出现红色说明，且 **Ctrl+Enter 也提交不了**。

## 已知不在本期范围

- svn 节点的文件 diff（`svn diff -c N`）属 **Phase 4**（spec §16 分期表）。
  点开 svn 节点只显示元信息 + 一句说明，这是预期行为，不是缺陷。
- `SvnExternalView`（白名单外改动视图）属 Phase 4；`SyncDialog`（更新 / 交付）
  属 Phase 2。

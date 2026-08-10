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

6. **tag 和清单里的版本号会各说各话。** 三个文件（`package.json` /
   `tauri.conf.json` / `Cargo.toml`）里漏改任何一个，构建照样成功，
   结果是一个叫 `v0.2.0` 的 Release 里装着 `jkk_0.1.0_x64-setup.exe`——
   而且要等十几分钟构建跑完、Release 都建出来了才看得见。
   已在 checkout 之后加一步版本一致性检查，对不上就在几秒内失败。
   两个方向都实测过：`v0.1.0` 通过，`v0.2.0` 报出三个文件各自的实际版本。

7. **`dtolnay/rust-toolchain@stable` + `-D warnings` 意味着 Rust 发新版可能挡住发版。**
   新 stable 引入一条新 lint，就会让**已经推上去的 tag** 构建失败，
   得删 tag 重来。缓解办法是发 tag 前先 `workflow_dispatch` 干跑一次
   （只构建、不发 Release）。彻底解决要把工具链钉死，那又会让 CI 长期停在旧版本，
   当前选择保留 `stable` + 干跑习惯。

### 为什么这么改是安全的

- workflow 的触发条件只有 `v*` tag 和手动触发，不会因为日常 push 而消耗额度。
  仓库是 private，Actions 按分钟计费（Windows runner 2 倍系数），
  限制触发条件在这里既是行为正确性也是成本控制。
- 发布步骤带 `if: startsWith(github.ref, 'refs/tags/v')`，
  手动触发时只构建、不会误建 Release。
- 权限与 tag 校验见下面的 review 轮次——初版在这两点上都不安全。

---

## Task 4 review 轮次（Codex adversarial review，独立于实现者）

4 条全部核实属实并已修复。**初版 workflow 不该合并**。

### 1 [high] `${{ github.ref_name }}` 直接展开进 PowerShell → 命令注入

初版第 39 / 117 行写的是 `$tag = '${{ github.ref_name }}'`。
GitHub 的 `${{ }}` 是在 shell 启动**之前**做纯文本替换的，
所以 tag 名里的单引号会闭合那个字符串，后面的内容变成可执行代码。

**实测确认前提**（一次性 fixture 仓库，`git check-ref-format` + `git tag`）：

| tag 名 | 结果 |
|---|---|
| `v1.0.0'x` | **可创建** |
| `v0.1.0';iwr$u;'` | **可创建** |
| `v1'; echo PWNED; '` | 被拒 —— 因为**空格**，不是引号 |
| `v1.0.0;whoami` / `v1.0.0$(whoami)` / ``v1.0.0`whoami` `` | 可创建 |

也就是说 `v0.1.0';iwr$u;'` 是合法 tag，展开后得到
`$tag = 'v0.1.0';iwr$u;''` —— 赋值，然后执行 `iwr$u`。注入成立。

注意这里的细节：因为用的是**单引号**字符串，`;`、`$()`、反引号在其中都是字面量，
唯一能逃出去的就是单引号本身；而单引号恰恰是 git 允许的。

修复：`REF_NAME` 经 `env:` 传入，脚本只读 `$env:REF_NAME`；
再加严格 SemVer 正则，畸形 ref 直接拒。publish job 同样处理。

### 2 [high] `contents: write` 挂在 workflow 顶层

初版整条构建链（checkout、pnpm 生命周期脚本、build.rs、一串用可变 tag 的
第三方 action）都跑在能写仓库的权限边界里，而 `actions/checkout` 默认还会
把凭据留在 `.git/config`。`workflow_dispatch` 明明不发布，也拿到同样的权限。

修复：顶层降为 `contents: read`；拆出独立的 `publish` job 只在它上面开
`contents: write`，它不 checkout、不构建，只把上一个 job 的产物挂上去；
checkout 加 `persist-credentials: false`。
publish 跑 ubuntu 而不是 windows —— 只是调一次 `gh`，而私有仓库的
Windows runner 按 2 倍分钟数计费。

**未采纳的建议**：把第三方 action 钉到完整 commit SHA。
钉死之后没有 dependabot 的仓库会长期停在旧版本，对私有个人仓库
不划算。保留 `@v4` 这类可变 tag，属明知的取舍。

### 3 [high] 第三方许可材料不足以支撑二进制分发

初版 LICENSE 自己写着「这份清单不完整，完整集合请自行从锁文件解析」——
那正好是不合规的自白。MIT 要求「在所有副本或实质性部分中包含版权声明和许可声明」，
Apache-2.0 §4 要求向接收者提供许可副本；而我们分发的 `.msi` / `.exe` 里
编译进了这些代码，接收者拿不到 `node_modules` 和 `Cargo.toml`。

修复：新增 `scripts/gen-notices.mjs`，从锁文件生成 `THIRD_PARTY_NOTICES.md`
（257 个组件），并经 `bundle.resources` 随安装包分发。

**收录范围**只取真正进二进制的：Rust 侧从 root package 出发只跟 normal 依赖
（dev-dependencies 只在测试链接、build-dependencies 只在构建期跑，都不分发），
npm 侧用 `pnpm list --prod`。实测核对：`git2` / `libgit2-sys` / `tauri` / `wry` /
`tao` / `react` 等在列；`vitest` / `vite` / `typescript` / `esbuild` / `tempfile`
均不在列。

**生成器自己先踩了一次同样的坑**：第一版只扫 crate 根目录，结果
`libgit2-sys` 只贡献了 `LICENSE-APACHE` / `LICENSE-MIT`（Rust binding 的），
而真正约束分发的 GPL-2.0 + 链接例外在 `libgit2/COPYING` 这个**子目录**里——
生成出来的 notices 里 GPL 正文一个字都没有，偏偏那是整棵树里最关键的一份。
改成额外扫一层子目录（跳过 src/tests/examples 等）后确认 GPL 正文已收录。
这正是 Task 3「crate 声明的 license 不等于它实际分发的东西的 license」那条坑的复发。

CI 加 `node scripts/gen-notices.mjs --check`：依赖动了却忘了重新生成就失败。
两个方向都实测（一致时 exit 0，篡改后 exit 1）。

### 4 [medium] 任意分支上的 `v*` tag 都能成为发布来源

`on: push: tags` 不区分 tag 打在哪个提交上。初版 README 还直接写
「`git tag v0.1.0`」，而我当时正在一个功能分支的 worktree 里——
照着 README 做就会发出未经 review 的代码，且从安装包上看不出来。

修复：构建前用 `gh api repos/{repo}/compare/master...{sha}` 断言该提交
与 master 的关系为 `identical` 或 `behind`（即已并入）。
走 API 而不是 `git merge-base`：上面关掉了 `persist-credentials`，
私有仓库的 `git fetch` 没有凭据可用。README 补上 `git checkout master` 与校验说明。

### 修复后的复验

```
workflow YAML 解析                    ✓ 顶层 contents: read；build=read / publish=write
所有 ${{ }} 均在 env / if / concurrency ✓ 逐行扫描确认
gen-notices --check 两个方向           ✓ 一致 exit 0，篡改 exit 1
pnpm test / cargo fmt / clippy -D warnings / cargo test  ✓ 全部 exit 0
pnpm tauri build                      ✓ msi + nsis
MSI 内文件表                           ✓ jkk.exe + THIRD_PARTY_NOTICES.md + LICENSE
```

`tauri.conf.json` 本次只改了 `bundle.resources`（构建期配置，不涉及任何
invoke 或 capabilities），配置写坏会直接让 `tauri build` 失败——
而它已成功产出两个安装包，这比 dev 模式更能证明该键有效。

---

## Task 4 第二轮 review（Codex adversarial review）

3 条全部核实属实并修复。**上一轮的修复本身又引入/遗留了问题**，这轮是冲着它们来的。

### 1 [high] 许可检查会把「缺正文」判成合格

上一轮加的 `--check` 只比对生成物与提交物是否一致，**不检查内容完不完整**。
生成器遇到没带许可正文的包时，只是把它写进一个说明章节然后照常返回 0。
实际有 **17 个组件**处于这个状态，其中包括 **`tauri` 本身**；
而 LICENSE 里却写着 notices 包含「分发所需的全部许可正文和版权声明」——
直接矛盾。CI 全绿的同时，安装包缺着 MIT / BSD 要求保留的材料。

**修复**：新增 `scripts/license-texts/`，收录 4 份标准正文；
包里没带正文时用标准正文补上，版权归属取自包元数据
（Cargo.toml `authors` / package.json `author`）；
`A OR B` 双许可按 Apache-2.0 → MIT → BSD-3-Clause → MPL-2.0 择一
（双许可本来就允许择一，且 Apache-2.0 正文不含逐项目版权行，最干净）。
**补不上就抛错**，生成与 `--check` 两个模式都失败。

标准正文的来源分两类，都做了核对：

| 许可 | 来源 | 核对方式 |
|---|---|---|
| Apache-2.0 / MPL-2.0 | 从依赖树里现成的副本复制（`adler2` / `cssparser`） | 固定文本，正文里没有逐项目字段 |
| MIT / BSD-3-Clause | 手写模板 + `{{COPYRIGHT}}` 占位 | 与树里真实分发的副本逐字比对：MIT **11/12 一致**（唯一不一致的 `android_system_properties` 自己的文件是残缺的），BSD-3-Clause 与 `alloc-no-stdlib` 一致——正是缺正文的 `alloc-stdlib` 的同作者 |

阻断行为实测：抽掉 `MPL-2.0.txt` → `selectors` 补不上 → 生成和 `--check`
都报错，`--check` **退出码 1**（专门单独验过退出码：先前用管道看时被
`Select-Object -First N` 截断成空，差点误判成不会阻断 CI）。

### 2 [high] 可变 action 跑在 `contents: write` 边界内

上一轮把权限收进了独立的 publish job，但那个 job 里的
`actions/download-artifact@v4` 仍是可变 tag。tag 一旦被重新指向，
换掉的代码就直接运行在能改 Release 的令牌权限下——拆 job 并没有消除这个信任边界。

**修复**：把它钉到完整 commit SHA `d3f86a1…`（经 `gh api` 解析 `v4` 当前指向）。
build job 的 action 仍用可变 tag：那边只有 `contents: read`，
收益不足以抵消没有 dependabot 时手工追 SHA 的维护成本——
这是上一轮「不钉 SHA」判断的**修正版**：不是全不钉，而是只钉写权限边界内的。

### 3 [medium] 发布流程无法安全重试

`gh release create` 在同名 Release 已存在时直接失败。
若 draft 建好了、资产传到一半挂掉，重跑 job 只会一直报 already exists，
留下一个资产不全的 draft 需要人工收拾。

**修复**：改成幂等——「有就复用、没有才建」+ `gh release upload --clobber`，
上传后再核对 `.msi` 与 `setup.exe` 都在，缺了就 `::error::` 并失败。

用桩 `gh` 跑了三种情形（`bash -n` 先过语法）：

| 情形 | 结果 |
|---|---|
| A 首次发布（Release 不存在） | create → upload → 校验通过，exit 0 |
| B 重跑（Release 已存在、资产不全） | 跳过 create、`--clobber` 覆盖，**自愈**，exit 0 |
| C 资产缺 `.msi` | 报 `::error::缺少 .msi 资产`，exit 1 |

### 修复后的复验

```
gen-notices                    ✓ 257 组件，17 个走标准正文兜底
gen-notices --check            ✓ exit 0；抽掉标准正文时 exit 1
YAML + 权限 + 钉定情况          ✓ publish 的 action 已 PINNED，build 的仍 mutable
publish 脚本三情形（桩 gh）      ✓ A/B 自愈，C 失败
pnpm test / fmt / clippy -D warnings / cargo test  ✓ 全 exit 0
pnpm tauri build               ✓ msi + nsis
MSI 文件表                      ✓ jkk.exe + THIRD_PARTY_NOTICES.md(564KB) + LICENSE
```

---

## Task 4 第三轮 review（Codex adversarial review）

2 条全部核实属实并修复。两条都直接推翻了上一轮我自己的判断。

### 1 [high] 可变 action 能替换最终发出去的安装包

上一轮我只钉了 publish job 的 action，理由是「build job 只有 `contents: read`，
风险不足以抵消维护成本」。**这个理由是错的**，因为它衡量错了爆炸半径：
build job 里的 action 经手工作区和 artifact，而 publish job 会**无条件**把
artifact 发布出去。所以那里的可变 tag 被重指，能直接换掉用户装的二进制，
`contents: read` 完全拦不住这条路径——它限制的是仓库 API 权限，不是产物内容。

**修复**：全部 7 个 action 一律钉到完整 commit SHA（经 `gh api` 解析各自当前指向）。

一个必须显式处理的细节：`dtolnay/rust-toolchain@stable` 里的 `stable` 是**分支名**，
该分支的 `action.yml` 给 `toolchain` 输入配了默认值，所以平时能省略。
按 SHA 引用之后那个默认值不再可依赖，而该输入是 `required: true`
（实测其 action.yml：为空会直接 `exit 1`），因此必须补 `with: toolchain: stable`。
另外钉的是 **action 代码**，不是 Rust 版本——装哪个 toolchain 仍由运行时的
`stable` 决定。

**但要清楚这买到的是什么**：构建期本来就有 257 个依赖的 `build.rs` 和 npm
生命周期脚本在跑，钉定 action 只关掉了其中一扇门，不等于产物可信。
决策日志里写明这点，免得下次有人以为「钉了 SHA 就安全了」。

### 2 [high] 重跑会覆盖已经公开发布的资产

上一轮为了让「传到一半」可重试，改成了「存在就复用 + `upload --clobber`」。
但它**只判断存在与否，不看是不是还在 draft**。于是：Release 已经发布出去、
有人下载过之后，重跑 workflow（或删 tag 重推）会静默替换掉那些安装包。
叠加 Rust 用的是 `stable`、同一 tag 重建结果本就不保证一致，
后果是「同一个版本号对应两份不同的二进制」，审计和回滚都失去依据。

**修复**：改成三态判断——

| 目标 Release 状态 | 行为 |
|---|---|
| 不存在 | 建 draft |
| 存在且仍是 draft | 复用并 `--clobber` 覆盖（修复传到一半的中间态） |
| 存在且已发布 | **直接失败**，提示先删 Release 或换版本号 |

同时把 `SHA256SUMS.txt` 作为资产一并发布，便于核对下载到的是不是这次构建的产物。

`|| echo absent` 那一步不能省：release 不存在时 `gh` 返回非零，
而脚本开着 `set -e`，不兜住的话会在判断状态之前就整步失败。

### 修复后的复验

桩 `gh` 覆盖四种状态（`bash -n` 先过语法）：

| 情形 | 结果 |
|---|---|
| A 不存在 | create draft → upload → 校验通过，exit 0 |
| B 仍是 draft、资产不全 | 跳过 create、`--clobber`，**自愈**，exit 0 |
| C **已发布** | exit 1，且日志里**没有任何 upload**——在动资产之前就拦住了 |
| D draft 但缺 `SHA256SUMS.txt` | exit 1 |

```
YAML + 权限 + 钉定           ✓ 7/7 action 全部 PINNED；build=read / publish=write
publish 脚本四情形（桩 gh）    ✓ 见上表
```

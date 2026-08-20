# 绿色版 zip + 条件签名钩子 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **本仓库覆盖约定（AGENTS.md §1.4 + subagent-safety）**：主会话 inline 执行（executing-plans），不派 subagent。执行前必须先建 `2026-08-18-portable-and-signing-decisions.md`（AGENTS.md §2.1）。

**Goal:** Release 构建附带免安装绿色 zip；流水线支持「secrets 有证书就自动签名、没有就保持现状」。

**Architecture:** 只动发布链路：`release.yml` 加打包步骤（zip 原始 `jkk.exe` + 许可文件 + 内联生成的使用说明）和条件签名接线（cert 探测 → pfx 导入用户证书库 → `tauri build -c` 注入 `signCommand`）；签名逻辑收敛在 `scripts/sign-windows.ps1`。应用代码零改动。

**Tech Stack:** GitHub Actions（windows-latest）、Tauri 2 `signCommand`、signtool、PowerShell 7。

**Spec:** `docs/superpowers/specs/2026-08-18-portable-and-signing-design.md`（已确认）

**工作目录：** `.worktrees/feat-portable-signing`（分支 `feat/portable-zip-signing`，基点 `809f1bb`）

---

### Task 1: `scripts/sign-windows.ps1` + 本地自签端到端验证

**Files:**
- Create: `scripts/sign-windows.ps1`
- Create: `docs/superpowers/plans/2026-08-18-portable-and-signing-decisions.md`（§2.1 决策日志，先于第一行实现代码）

- [ ] **Step 0: 建决策日志骨架**

写入 `docs/superpowers/plans/2026-08-18-portable-and-signing-decisions.md`：

```markdown
# 绿色版 zip + 条件签名钩子 决策日志

计划：2026-08-18-portable-and-signing.md

| Task | 状态 | 产出 | 验证 |
|---|---|---|---|
| 1 sign-windows.ps1 + 本地自签验证 | 进行中 | | |
| 2 release.yml 绿色 zip 打包 | 未开始 | | |
| 3 release.yml 条件签名接线 | 未开始 | | |
| 4 README 下载/签名说明 | 未开始 | | |
| 5 分支 workflow_dispatch 空跑 + 产物检查 + PR | 未开始 | | |

## Task 1 决策
```

- [ ] **Step 1: 探测本机 signtool（决定本地验证深度）**

```powershell
Get-ChildItem 'C:\Program Files (x86)\Windows Kits\10\bin' -Directory -ErrorAction SilentlyContinue |
  Sort-Object Name -Descending |
  ForEach-Object { Join-Path $_.FullName 'x64\signtool.exe' } |
  Where-Object { Test-Path $_ } | Select-Object -First 1
```

Expected: 输出一个 signtool.exe 路径。若为空（本机没装 Windows SDK），本地只能验证
「找不到 signtool 的报错分支」，签名机制验证留到 secrets 就位后的 dispatch——
在决策日志里如实记为 Limitation。

- [ ] **Step 2: 写签名脚本**

`scripts/sign-windows.ps1`：

```powershell
#Requires -Version 7.0
<#
.SYNOPSIS
  给单个文件做 Authenticode 签名（release.yml 的 tauri signCommand 调用）。
.DESCRIPTION
  证书预先由 CI 导入 CurrentUser\My（pfx 即导即删，不落盘），本脚本只从
  环境变量 CERT_THUMB 读指纹——密码/指纹/证书路径都不进命令行
  （signtool 的 /p 会暴露在进程列表里），signCommand 配置因此可以完全静态。
  时间戳不可省：没有它，证书过期后签名一起作废。
.PARAMETER Target
  待签文件路径（tauri 把 %1 替换成它；exe 与安装包各调一次）。
.PARAMETER SkipVerify
  仅本地自签机制验证用：自签证书不在受信根里，verify /pa 必然失败。
  CI 永远不传这个开关。
#>
param(
    [Parameter(Mandatory)][string]$Target,
    [switch]$SkipVerify
)
$ErrorActionPreference = 'Stop'

$thumbprint = $env:CERT_THUMB
if ([string]::IsNullOrEmpty($thumbprint)) { throw 'CERT_THUMB 未设置（证书未导入？）' }
if (-not (Test-Path $Target)) { throw "待签文件不存在：$Target" }

# 定位 signtool：Windows SDK kits 目录按版本倒序取第一个
$kitsRoot = 'C:\Program Files (x86)\Windows Kits\10\bin'
$signtool = Get-ChildItem $kitsRoot -Directory -ErrorAction SilentlyContinue |
    Sort-Object Name -Descending |
    ForEach-Object { Join-Path $_.FullName 'x64\signtool.exe' } |
    Where-Object { Test-Path $_ } |
    Select-Object -First 1
if (-not $signtool) {
    throw "在 $kitsRoot 下找不到 signtool.exe（需要 Windows SDK；CI runner 自带）"
}

& $signtool sign /fd sha256 /td sha256 /tr http://timestamp.digicert.com /sha1 $thumbprint $Target
if ($LASTEXITCODE -ne 0) { throw "signtool sign 失败（exit $LASTEXITCODE）：$Target" }

if (-not $SkipVerify) {
    & $signtool verify /pa $Target | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "签完校验失败：$Target" }
}
```

签名参数只收 `-Target`：tauri 把 `%1` 展开为目标路径作为最后一个实参，
具名参数不受位置影响；指纹走环境变量，命令行里零秘密。

- [ ] **Step 3: 本地端到端验证（自签证书）**

```powershell
# 造自签证书（仅本机用户 store，不进受信根）
$cert = New-SelfSignedCertificate -Type CodeSigningCert -Subject 'CN=jkk-local-sign-test' `
  -CertStoreLocation Cert:\CurrentUser\My
$env:CERT_THUMB = $cert.Thumbprint

# 拿一个现成的未签名 exe 副本当靶子（不碰系统原文件）
$target = Join-Path $env:TEMP 'jkk-sign-probe.exe'
Copy-Item "$env:SystemRoot\notepad.exe" $target -Force

# 签名（自签不在受信根，跳过 verify；机制验证看 Get-AuthenticodeSignature）
pwsh -NoProfile -File scripts/sign-windows.ps1 -Target $target -SkipVerify
if ($LASTEXITCODE -ne 0) { throw "脚本签名失败" }

# 断言：签名存在且指纹就是我们的测试证书
$sig = Get-AuthenticodeSignature $target
if ($sig.SignerCertificate.Thumbprint -ne $cert.Thumbprint) { throw "签错证书了" }

# 再验证「不带 -SkipVerify 时 verify 分支真的在工作」：自签不受信，应当抛错
pwsh -NoProfile -File scripts/sign-windows.ps1 -Target $target
if ($LASTEXITCODE -eq 0) { throw "自签证书 verify 居然过了，verify 分支是摆设" }

# 清理：删测试证书、删靶子
Remove-Item "Cert:\CurrentUser\My\$($cert.Thumbprint)"
Remove-Item $target
```

Expected: 无异常抛出；`Get-AuthenticodeSignature` 指纹匹配；第二跑（无 SkipVerify）
非零退出。把结果记进决策日志 Task 1。

- [ ] **Step 4: Commit**

```powershell
git add scripts/sign-windows.ps1 docs/superpowers/plans/2026-08-18-portable-and-signing-decisions.md docs/superpowers/plans/2026-08-18-portable-and-signing.md
git commit -m "feat(release): 签名脚本 sign-windows.ps1（证书后补，机制先验证）

- What: signtool 签名脚本，指纹走环境变量不进命令行，时间戳不可省，签完 verify 自检
- Why: 流水线「有证书就签」的执行端；证书采购是独立决策，机制先用自签证书验证
- Impact/Test: 本地 New-SelfSignedCertificate 端到端跑通；CI 真证书路径待 secrets 就位"
```

---

### Task 2: release.yml 绿色 zip 打包 + publish 资产核对

**Files:**
- Modify: `.github/workflows/release.yml`（build job 加打包步骤；publish job 加核对与 notes）

- [ ] **Step 1: build job 加打包步骤**

在 `- name: tauri build` 步骤之后、`- name: Collect installers` 之前插入：

```yaml
      # 绿色版：原始 exe 本身就是完整应用，zip 起来直接发。
      # 打包发生在 build 之后——有证书时 zip 里的 exe 也是签过名的。
      - name: Pack portable zip
        shell: pwsh
        run: |
          $ErrorActionPreference = 'Stop'
          $version = (Get-Content src-tauri/tauri.conf.json -Raw | ConvertFrom-Json).version
          $exe = 'src-tauri/target/release/jkk.exe'
          if (-not (Test-Path $exe)) { throw "没找到 $exe" }

          $stage = Join-Path $env:RUNNER_TEMP 'portable-stage'
          New-Item -ItemType Directory -Path $stage -Force | Out-Null
          Copy-Item $exe $stage
          Copy-Item LICENSE, THIRD_PARTY_NOTICES.md $stage

          # 使用说明内联生成，不进 repo（避免双维护）；\r\n + BOM 是旧版记事本兼容
          $readme = @(
            'jkk 绿色版（免安装）'
            '===================='
            ''
            '解压后直接运行 jkk.exe 即可。'
            ''
            '* 本软件未做代码签名，首次运行 Windows 会弹 SmartScreen'
            '  「未知发布者」警告：点「更多信息」→「仍要运行」。'
            '* 若双击无反应或报错，说明系统缺少 WebView2 运行时'
            '  （Win11 和保持更新的 Win10 自带），请从微软官网安装：'
            '  https://developer.microsoft.com/microsoft-edge/webview2/'
            '* 第三方组件许可声明见 THIRD_PARTY_NOTICES.md。'
          ) -join "`r`n"
          [IO.File]::WriteAllText(
            (Join-Path $stage 'README-绿色版.txt'),
            $readme,
            [Text.UTF8Encoding]::new($true)
          )

          New-Item -ItemType Directory -Path artifacts -Force | Out-Null
          $zip = "artifacts/jkk_${version}_x64-portable.zip"
          Compress-Archive -Path "$stage\*" -DestinationPath $zip -Force
          Get-Item $zip | Select-Object Name, Length
```

注意：`Collect installers` 步骤只从 bundle 目录收 `*.exe, *.msi`，zip 在
`artifacts/` 下不会被它误收，也不用改它。

- [ ] **Step 2: publish job 加资产核对与 notes**

资产核对段（现有三条 grep 之后）加一条：

```bash
          echo "$assets" | grep -q 'portable\.zip$' || { echo "::error::缺少 portable zip 资产"; exit 1; }
```

notes 的产物列表改成：

```bash
          notes='安装包**未做代码签名**时，Windows 会弹 SmartScreen「未知发布者」警告。
          点「更多信息」→「仍要运行」即可。

          产物：
          - `*-setup.exe` —— NSIS 安装器
          - `*.msi` —— MSI 安装包
          - `*-portable.zip` —— 免安装绿色版（解压即用；无 WebView2 运行时的旧系统
            需先装 https://developer.microsoft.com/microsoft-edge/webview2/ ）
          - `SHA256SUMS.txt` —— 校验和

          第三方组件的许可声明见安装目录下的 THIRD_PARTY_NOTICES.md。'
```

注意：notes 里「未做代码签名**时**」的措辞改了——签名钩子就位后是否签名取决于
secrets，文案不能再说死。

- [ ] **Step 3: YAML 语法检查**

```powershell
npx -y js-yaml .github/workflows/release.yml | Out-Null
```

Expected: 无输出无报错（有语法错会抛）。

- [ ] **Step 4: Commit**

```powershell
git add .github/workflows/release.yml docs/superpowers/plans/2026-08-18-portable-and-signing-decisions.md
git commit -m "feat(release): 构建附带免安装绿色 zip

- What: build job 打包 jkk_<版本>_x64-portable.zip（exe + 许可 + 使用说明）；
  publish 资产核对加 zip，notes 补绿色版条目
- Why: 安装包之外提供免安装形态；Tauri 产物本身不依赖安装
- Impact/Test: js-yaml 语法检查过；端到端靠 Task 5 的分支 dispatch 空跑验证"
```

---

### Task 3: release.yml 条件签名接线

**Files:**
- Modify: `.github/workflows/release.yml`（build job：探测/导入/双 build 分支/清理）

- [ ] **Step 1: 加证书探测步骤**

在 `Validate tag` 之后、`pnpm/action-setup` 之前插入（签名决策要在构建前定）：

```yaml
      # secrets 不能用在 if 里，先把「在不在」映射成 output。
      # 只判存在性，值本身绝不写日志。
      - name: Detect signing certificate
        id: cert
        shell: pwsh
        env:
          PFX_B64: ${{ secrets.SIGNING_CERT_PFX_BASE64 }}
          PFX_PASSWORD: ${{ secrets.SIGNING_CERT_PASSWORD }}
        run: |
          $has = -not [string]::IsNullOrEmpty($env:PFX_B64) -and -not [string]::IsNullOrEmpty($env:PFX_PASSWORD)
          "has_cert=$($has.ToString().ToLower())" >> $env:GITHUB_OUTPUT
```

- [ ] **Step 2: 加导入步骤 + 双 build 分支 + 清理**

`- name: tauri build` 一步替换为以下四步：

```yaml
      # pfx 解码后即导入用户证书库并立刻删文件——证书文件在盘上的存活窗口
      # 压到这一步内部。密码全程只经 PowerShell 变量（SecureString），不进任何
      # 命令行（signtool /p、certutil 都会把密码暴露给进程列表）。
      - name: Import signing certificate
        if: steps.cert.outputs.has_cert == 'true'
        shell: pwsh
        env:
          PFX_B64: ${{ secrets.SIGNING_CERT_PFX_BASE64 }}
          PFX_PASSWORD: ${{ secrets.SIGNING_CERT_PASSWORD }}
        run: |
          $ErrorActionPreference = 'Stop'
          $pfx = Join-Path $env:RUNNER_TEMP 'signing.pfx'
          [IO.File]::WriteAllBytes($pfx, [Convert]::FromBase64String($env:PFX_B64))
          $secure = ConvertTo-SecureString $env:PFX_PASSWORD -AsPlainText -Force
          $imported = Import-PfxCertificate -FilePath $pfx -CertStoreLocation Cert:\CurrentUser\My -Password $secure
          Remove-Item $pfx -Force
          "CERT_THUMB=$($imported.Thumbprint)" >> $env:GITHUB_ENV

      - name: tauri build (signed)
        if: steps.cert.outputs.has_cert == 'true'
        shell: pwsh
        run: |
          $ErrorActionPreference = 'Stop'
          # signCommand 完全静态：脚本路径是 checkout 下的绝对路径，指纹由脚本
          # 自己从 CERT_THUMB 环境变量读。配置里零秘密。
          $cfg = @{
            bundle = @{ windows = @{ signCommand = @{
              cmd  = 'pwsh'
              args = @('-NoProfile', '-File', "$PWD/scripts/sign-windows.ps1", '-Target', '%1')
            } } }
          } | ConvertTo-Json -Depth 6 -Compress
          $cfgPath = Join-Path $env:RUNNER_TEMP 'tauri-sign.conf.json'
          [IO.File]::WriteAllText($cfgPath, $cfg)
          pnpm tauri build -c $cfgPath
          if ($LASTEXITCODE -ne 0) { throw 'tauri build (signed) 失败' }

      # 无证书路径必须与接线前逐字节一致
      - name: tauri build (unsigned)
        if: steps.cert.outputs.has_cert == 'false'
        run: pnpm tauri build

      # runner 是一次性 VM，但证书不该陪跑到 job 结束
      - name: Remove signing certificate
        if: ${{ always() && steps.cert.outputs.has_cert == 'true' }}
        shell: pwsh
        run: |
          Remove-Item "Cert:\CurrentUser\My\$env:CERT_THUMB" -Force -ErrorAction SilentlyContinue
```

- [ ] **Step 3: 静态核对「无证书路径不变」**

```powershell
git diff master -- .github/workflows/release.yml
```

人工核对：unsigned 分支 = 原来的 `pnpm tauri build`；其余原步骤零改动。
再跑一遍 `npx -y js-yaml .github/workflows/release.yml | Out-Null`。

- [ ] **Step 4: Commit**

```powershell
git add .github/workflows/release.yml docs/superpowers/plans/2026-08-18-portable-and-signing-decisions.md
git commit -m "feat(release): 条件签名钩子——secrets 就位即签，没配保持现状

- What: cert 探测 → Import-PfxCertificate 入用户库（pfx 即导即删）→
  tauri build -c 注入静态 signCommand；无证书走原样 pnpm tauri build；
  job 末尾清证书
- Why: 签名可用性的流水线侧就位；证书采购是独立决策，不配 secrets 行为不变
- Impact/Test: js-yaml 语法检查 + diff 人工核对无证书路径不变；
  端到端由 Task 5 dispatch 验证（无证书）"
```

---

### Task 4: README 下载/签名说明

**Files:**
- Modify: `README.md`（发布节：绿色版段 + 签名小节改写 + 目录结构 scripts 注释）

- [ ] **Step 1: 发布节加绿色版段**

在「产物含 `SHA256SUMS.txt`……」段之后插入：

```markdown
### 绿色版（免安装）

每个 Release 另附 `jkk_<版本>_x64-portable.zip`：解压即用，不走安装流程。
两点注意：

- 未签名时首次运行同样会弹 SmartScreen（见下节）；
- 绿色版不会像安装包那样自动补装 WebView2 运行时——Win11 和保持更新的
  Win10 都自带；干净的旧系统双击无反应的话，装一下
  [WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/)。
```

- [ ] **Step 2: 改写「已知：安装包未签名」小节**

整节替换为：

```markdown
### 代码签名：流水线已就位，证书未采购

流水线支持「有证书就签」：secrets 配好 `SIGNING_CERT_PFX_BASE64` 和
`SIGNING_CERT_PASSWORD` 后，构建会自动用 signtool 给 exe 和安装包签名
（脚本：`scripts/sign-windows.ps1`）；没配就发未签名包。

签名之前，Windows 会对安装器和绿色版弹 SmartScreen「未知发布者」警告，
点「更多信息」→「仍要运行」即可。
```

- [ ] **Step 3: 目录结构节 scripts 行微调**

`├── scripts/          # 一次性/维护脚本（图标生成等）` 改为：

```
├── scripts/          # 一次性/维护脚本（图标生成、CI 签名等）
```

- [ ] **Step 4: Commit**

```powershell
git add README.md docs/superpowers/plans/2026-08-18-portable-and-signing-decisions.md
git commit -m "docs(readme): 绿色版下载说明，签名小节改为「流水线就位、证书未采购」

- What: 发布节加绿色版段；签名小节从「已知未签名」改为条件钩子说明
- Why: README 是对外主索引，产物形态变了要同步
- Impact/Test: 纯文档"
```

---

### Task 5: 分支 workflow_dispatch 空跑验证 + 产物检查 + PR

**Files:**
- 无新改动；本任务是验证与交付

- [ ] **Step 1: push 并触发分支空跑**

```powershell
git push -u origin feat/portable-zip-signing
gh workflow run release.yml --ref feat/portable-zip-signing
gh run list --workflow release.yml --limit 1
```

记下 run id。workflow_dispatch 走分支上的 workflow 文件；publish job 的
`if: startsWith(github.ref, 'refs/tags/v')` 在分支 ref 下为假，只构建不发布。

- [ ] **Step 2: 盯到结束**

```powershell
gh run watch <run-id> --exit-status --interval 30
```

Expected: build 绿、publish skipped。日志确认：`Detect signing certificate` 输出
`has_cert=false`，走了 `tauri build (unsigned)`，签名/清理步骤 skipped。

- [ ] **Step 3: 下载 artifact 核内容**

```powershell
gh run download <run-id> -n jkk-windows -D "$env:TEMP\jkk-portable-check"
Expand-Archive "$env:TEMP\jkk-portable-check\jkk_0.2.1_x64-portable.zip" "$env:TEMP\jkk-portable-check\zip"
Get-ChildItem "$env:TEMP\jkk-portable-check\zip"
Get-Content "$env:TEMP\jkk-portable-check\zip\README-绿色版.txt"
```

Expected: zip 里有 `jkk.exe`、`LICENSE`、`THIRD_PARTY_NOTICES.md`、
`README-绿色版.txt` 四件；txt 中文不乱码（BOM 生效）。

- [ ] **Step 4: 绿色 exe 真实启动一次**

```powershell
$p = Start-Process "$env:TEMP\jkk-portable-check\zip\jkk.exe" -PassThru
Start-Sleep 8
if ($p.HasExited) { throw "绿色 exe 启动后退出了（exit $($p.ExitCode)）" }
$p | Stop-Process -Force
Remove-Item -Recurse -Force "$env:TEMP\jkk-portable-check"
```

Expected: 8 秒后进程仍在（窗口起来了）。若本机从未装过该应用依赖的 WebView2
会在此暴露——本机开发环境自带，预期通过；如实记录。

- [ ] **Step 5: 决策日志收尾 + Draft PR**

决策日志各 Task 状态更新为完成、验证结果写实。然后：

```powershell
git add docs/superpowers/plans/2026-08-18-portable-and-signing-decisions.md
git commit -m "docs(plan): 决策日志收尾（dispatch 空跑绿、zip 内容核对、exe 启动验证）"
git push
gh pr create --draft --base master --title "feat(release): 绿色版 zip + 条件签名钩子" --body-file <正文文件>
```

PR 正文要点：spec 链接、决策日志链接、dispatch run 链接与结论、证书接入手册
（spec 末尾已有）、「对 0.2.1 不回溯补发，自下一个 tag 起生效」。

---

## Self-Review 结论（写完后自查）

- **Spec 覆盖**：spec §1 绿色 zip → Task 2；§2 publish → Task 2 Step 2；
  §3 条件签名 → Task 3；§4 签名脚本 → Task 1；§5 验证策略 → Task 1 Step 3 +
  Task 5；README → Task 4。无遗漏。
- **占位符**：无 TBD；所有代码块是完整可执行内容。
- **类型/命名一致**：`CERT_THUMB`（Task 3 写入 GITHUB_ENV ↔ Task 1 脚本读取）；
  `has_cert` output 名前后一致；zip 文件名 `jkk_<version>_x64-portable.zip`
  Task 2 产出 ↔ Task 5 核对一致。
- **已知遗留**：signCommand 的 `%1` 展开与 args 传递形态在 Task 1 Step 2 注释里
  说明；真证书路径的端到端验证只能等 secrets 就位，属 spec 已声明的边界。

# 绿色版（免安装 zip）+ 条件签名钩子 设计

日期：2026-08-18
状态：已确认（用户批准方案 A）

## 背景与目标

v0.2.1 发布只有 NSIS/MSI 安装包，且未做代码签名，Windows SmartScreen 会弹
「未知发布者」警告，可用性差。两个诉求：

1. **免安装版**：把构建出的原始 `jkk.exe` 打成 zip 直接发——Tauri 产物本身
   不依赖安装，安装包只多做「解压到固定目录 + 检测/补装 WebView2 运行时」。
2. **签名可用性**：流水线做成「secrets 里有证书就签、没有就保持现状」，
   证书采购（OV 证书 / Azure Trusted Signing）是之后的独立决策，不阻塞本需求。

### 必须如实告知用户的边界

- **免安装 ≠ 免 SmartScreen**：MotW 对下载的 exe/zip 同样生效，绿色版首次
  运行仍会弹警告。根治只有签名 + 声誉积累。
- **WebView2 依赖不因绿色版消失**：安装包会自动补装 WebView2 运行时，绿色版
  不会。Win11 和保持更新的 Win10 自带；干净的旧系统上双击会启动失败，
  使用说明里必须给出官方下载链接。

## 改动面

- `.github/workflows/release.yml`：加打包/签名步骤（主体）
- `scripts/sign-windows.ps1`：新增（签名脚本）
- `README.md`：下载一节补绿色版说明
- 应用代码（`src/`、`src-tauri/src/`）零改动

## 设计

### 1. 绿色 zip 打包（build job，`tauri build` 之后）

产物 `jkk_<version>_x64-portable.zip`，内容：

| 文件 | 来源 | 理由 |
|---|---|---|
| `jkk.exe` | `src-tauri/target/release/jkk.exe` | 绿色版本体；找不到则 fail-fast |
| `LICENSE` / `THIRD_PARTY_NOTICES.md` | 仓库根 | MIT/Apache-2.0 要求随分发保留声明 |
| `README-绿色版.txt` | workflow 步骤内联生成 | 使用说明，不进 repo（避免双维护） |

`README-绿色版.txt` 内容三条：

1. 免安装，解压即用；
2. 未做代码签名，首次运行 Windows 会弹 SmartScreen「未知发布者」，
   点「更多信息 → 仍要运行」即可；
3. 若双击无反应/报错，说明系统缺 WebView2 运行时，给出微软官方下载链接
   （https://developer.microsoft.com/microsoft-edge/webview2/）。

版本号从 `src-tauri/tauri.conf.json` 读（与 Validate tag 步骤同源）。
用 `Compress-Archive` 打包，zip 随 artifacts 一起上传（`jkk-windows`）。
打包发生在 `tauri build` 之后：有证书时 zip 里的 `jkk.exe` 与安装包内的
一样，都是签过名的。

### 2. publish job

- `SHA256SUMS.txt`：现有 `sha256sum ./*` 是全量的，zip 自动被覆盖，不改；
- 资产核对加一条：缺少 `*-portable.zip` 即失败（与 `.msi`、`setup.exe` 同级）；
- Release notes 模板的产品列表加一行：`*-portable.zip` —— 免安装绿色版，
  并保留 SmartScreen 提示。

### 3. 条件签名钩子（build job）

secrets 设计：

| secret | 内容 |
|---|---|
| `SIGNING_CERT_PFX_BASE64` | 代码签名证书 pfx 的 base64 |
| `SIGNING_CERT_PASSWORD` | pfx 密码 |

GitHub Actions 不允许在 `if` 里直接引用 secrets，因此加一个前置 pwsh 步骤
把「两个 secret 都存在」映射成 step output（`has_cert`），后续步骤按它分支：

- **有证书**：
  1. pfx 解码到 `$RUNNER_TEMP\signing.pfx`（不落 workspace；runner 是一次性
     VM，步骤末仍显式删除）；
  2. `pnpm tauri build -c <json>` 注入 `bundle.windows.signCommand`（CLI 的
     `-c/--config` 合并行为已从本机安装的 CLI 帮助文本确认；`signCommand`
     字段已从本机 tauri-utils 2.9.3 源码确认）。对象记法：
     `cmd` = 仓库 `scripts/sign-windows.ps1` 的**绝对路径**（相对路径基准
     不确定，用绝对路径直接消解），`args` = pfx 路径与 `%1`（tauri 会把 `%1`
     替换成待签文件，exe 与安装包都会调用）；
  3. 密码**不进命令行**：脚本从 `env:SIGNING_CERT_PASSWORD` 读取
     （signtool `/p` 会暴露在进程列表）。
- **没证书**：`pnpm tauri build` 原样执行。此路径行为必须与现状逐字节一致，
  也是本需求能完整端到端验证的部分。

### 4. `scripts/sign-windows.ps1`

遵循 §10 PowerShell 规范（`$ErrorActionPreference = 'Stop'`、显式检查外部
命令 `$LASTEXITCODE`）。职责：

1. 定位 signtool（Windows SDK kits 目录按版本倒序找 `signtool.exe`；
   找不到给出明确报错）；
2. `signtool sign /fd sha256 /td sha256 /tr http://timestamp.digicert.com
   /f <pfx> /p $env:SIGNING_CERT_PASSWORD <目标文件>`——**时间戳不可省**，
   否则证书过期后签名作废；
3. 签完 `signtool verify /pa <目标文件>` 自检。

本地验证机制：`New-SelfSignedCertificate` 造自签证书导出 pfx，对一个测试
exe 端到端跑通脚本（自签不解决 SmartScreen，只验证机制；CI 上换成真 OV
证书时机制相同）。

### 5. 验证策略

| 路径 | 怎么验 |
|---|---|
| 无证书（现状保持） | `workflow_dispatch` 手动空跑：产物 = 现有两个安装包 + 新增 zip |
| zip 内容 | CI 步骤列出包内容；发布前人工解压运行一次（E2E 清单） |
| 签名脚本 | 本地自签证书端到端跑通（机制验证） |
| 真证书路径 | 用户采购证书、配好 secrets 后，打 tag 前的 workflow_dispatch 验证 |

### 6. 明确不做（YAGNI）

- Tauri updater / 自动更新；
- Azure Trusted Signing 接入（等证书决策）；
- 绿色版启动时自动检测并引导安装 WebView2（属应用代码改动，超出本次范围）。

## 风险与对策

| 风险/边界 | 类型 | 对策 |
|---|---|---|
| signCommand 相对路径基准不确定 | Risk | 注入绝对路径，不确定性直接消失 |
| pfx 落盘泄露 | Risk | `$RUNNER_TEMP`（一次性 VM）+ 步骤末显式删除 |
| 密码泄进日志/进程列表 | Risk | 走环境变量不进命令行；GitHub 对 secret 值自动 mask 兜底 |
| 时间戳服务器暂时不可用 | Risk | signtool 失败即构建失败（fail-fast），不静默产出未签名包 |
| 绿色版无 WebView2 时启动失败 | Limitation | 使用说明给出症状描述与官方下载链接 |
| 绿色版仍弹 SmartScreen | Limitation | 使用说明与 Release notes 都如实写明 |

## 后续（证书接入手册，买证书后执行）

1. 导出/下载 pfx，base64 后写入 secrets `SIGNING_CERT_PFX_BASE64`，
   密码写入 `SIGNING_CERT_PASSWORD`；
2. `workflow_dispatch` 手动跑一次，确认产物带有效签名（下载后
   `signtool verify /pa` 或右键属性看「数字签名」页）；
3. 正常打 tag 流程不变。

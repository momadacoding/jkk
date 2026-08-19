# 绿色版 zip + 条件签名钩子 决策日志

计划：2026-08-18-portable-and-signing.md

| Task | 状态 | 产出 | 验证 |
|---|---|---|---|
| 1 sign-windows.ps1 + 本地自签验证 | 完成 | scripts/sign-windows.ps1 | 自签证书端到端跑通（指纹匹配；verify 分支如期拦截不受信根） |
| 2 release.yml 绿色 zip 打包 | 完成 | .github/workflows/release.yml | js-yaml 语法检查过；pwsh 打包块在本地沙盒完整跑通（zip 命名/四件内容/UTF-8 BOM/CRLF 均验证） |

## Task 2 决策

- **偏离计划**：无。计划中的 here-string 版本在写作时已改为数组 join（避免 YAML 缩进
  污染 txt 内容），实现与计划终稿一致。
- **计划没写的坑**：pwsh 控制台输出中文显示为 `?` 是控制台 GBK 显示问题，文件字节
  本身是正确的 UTF-8（BOM EF BB BF 已字节级验证）；`Compress-Archive` 与
  `Collect installers` 无冲突（后者只从 bundle 目录收 *.exe/*.msi）。
- **为什么安全**：zip 找不到 exe 时 fail-fast；publish 资产核对把 zip 列为必需，
  缺了会阻断发布而不是发残缺 Release。
| 3 release.yml 条件签名接线 | 完成 | .github/workflows/release.yml | js-yaml 语法检查过；本地模拟 CI 形态跑通 base64→pfx→Import-PfxCertificate→指纹交接→signCommand 配置生成；真证书 verify 链路见 Limitation |

## Task 3 决策

- **本地模拟 CI 形态验证**：自签证书导出 pfx → base64（模拟 secrets）→ 照 Import
  步骤原文执行 → pfx 即导即删、指纹入库交接成功；signCommand 配置 JSON 生成
  正确（静态、零秘密）。
- **计划没写的坑**：想把自签证书加进 `CurrentUser\Root` 让 `verify /pa` 本地全链
  路通过——Windows 对 Root 库导入强制弹交互确认，非交互会话报
  "UI is not allowed in this operation"。**放弃该路径**，不是脚本缺陷。
- **Limitation**：真证书下 `signtool verify /pa` 的正向通过未在本地验证
  （自签证书无法进入受信根）。已验证的部分：签名本身（Task 1 指纹匹配）、
  verify 对不受信链的拦截（Task 1）、CI 形态的导入与交接（本 Task 模拟）。
  真 OV 证书链接受信根，verify 预期通过；首次真证书构建时留意此点。
- **为什么安全**：无证书路径与接线前逐字节一致（`run: pnpm tauri build` 原样
  保留在 unsigned 分支）；证书文件存活窗口压在一个步骤内；job 末尾
  `always()` 清理用户证书库。
| 4 README 下载/签名说明 | 未开始 | | |
| 5 分支 workflow_dispatch 空跑 + 产物检查 + PR | 未开始 | | |

## Task 1 决策

- **完成**：产出 `scripts/sign-windows.ps1`。本机有 Windows SDK
  （10.0.26100.0 的 signtool），自签证书端到端跑通：未签名靶子签名后
  Get-AuthenticodeSignature 指纹匹配；不带 `-SkipVerify` 时 verify 分支对
  不受信根如期拦截（exit 1）。
- **计划没写的坑（测试靶子选择）**：计划用 notepad.exe 拷贝当靶子，结果 signtool
  报「Successfully signed」但 `Get-AuthenticodeSignature` 仍显示微软原签名
  （Status Valid）——已签名的系统文件不适用「替换成我们的签名」的假设。
  改用未签名的 Rust debug 构建（app.exe）当靶子后一切符合预期。
  **教训：签名机制的测试靶子必须选从未签过名的二进制。**
- **为什么安全**：密码/指纹/证书路径全程不进命令行（CERT_THUMB 走环境变量），
  signCommand 配置完全静态、零秘密；时间戳强制（证书过期后签名仍有效）；
  `-SkipVerify` 仅供本地自签验证，CI 不传。
- **Limitation**：真 OV 证书路径（含 GITHUB_ENV 传递、tauri signCommand 真实
  调用链）只能等 secrets 就位后验证；本地验证的是脚本机制本身。

# 绿色版 zip + 条件签名钩子 决策日志

计划：2026-08-18-portable-and-signing.md

| Task | 状态 | 产出 | 验证 |
|---|---|---|---|
| 1 sign-windows.ps1 + 本地自签验证 | 完成 | scripts/sign-windows.ps1 | 自签证书端到端跑通（指纹匹配；verify 分支如期拦截不受信根） |
| 2 release.yml 绿色 zip 打包 | 未开始 | | |
| 3 release.yml 条件签名接线 | 未开始 | | |
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

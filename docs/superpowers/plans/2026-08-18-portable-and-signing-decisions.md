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
| 4 README 下载/签名说明 | 完成 | README.md | 三处按计划原文：发布节加绿色版段、「已知未签名」小节改写为「流水线就位、证书未采购」、目录结构 scripts 行 |
| 5 分支 workflow_dispatch 空跑 + 产物检查 + PR | 完成 | 分支已推送 | dispatch run 32229587339：build 绿、publish 按设计 skipped；has_cert=false 走 unsigned；签名/清理步骤 skipped；产物含 zip（四件齐全、txt 编码正确）；绿色 exe 真实启动 8 秒存活 |
| 6 review 修复（7 项） | 完成 | release.yml / sign-windows.ps1 / README.md | 6 项代码缺陷各补了 pwsh 断言测试（含旧写法 bug 复现）：19+13 项全绿；signtool 定位在本机真实 SDK 布局上复跑 |

## Task 5 决策

- **dispatch 验证（run 32229587339）**：`gh workflow run release.yml --ref
  feat/portable-zip-signing` 空跑——build success、publish skipped（分支 ref 下
  `startsWith(github.ref, 'refs/tags/v')` 为假，与设计一致）。步骤行为：
  Detect signing certificate → has_cert=false；Import/sign build/Remove 全部
  skipped；走了 `tauri build (unsigned)`；Pack portable zip success。
- **产物核对**：下载 `jkk-windows` artifact，zip 内含 jkk.exe（12MB release
  构建）、LICENSE、THIRD_PARTY_NOTICES.md、README-绿色版.txt 四件；
  txt 为 UTF-8 BOM + CRLF，中文内容字节级正确。dispatch 不发 Release，
  故无 SHA256SUMS（publish 才生成），属预期。
- **绿色 exe 真实启动**：下载的 zip 解压后运行 jkk.exe，8 秒后进程存活
  （窗口起来）。Stop-Process 后文件句柄短暂残留导致首次清理被拒，等待后
  清干净——测试善后现象，非产物问题。
- **Limitation（重申）**：真证书路径（signed 分支、tauri 真实调用 signCommand、
  verify /pa 正向通过）未验证，等 secrets 就位后的首次 tag 构建见分晓；
  已在 Task 1/3 记录机制层面的本地验证覆盖。

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

## Task 6 决策（独立 review 修复）

`/code-review` 在 Task 1~5 收尾后跑出 7 项，全部核实成立并修复。这一节记的是
**为什么前面的验证没拦住它们**——比修复本身更值得留档。

### 计划没写的坑（也是前序验证的盲区）

- **自签证书没有链，掩盖了 `CERT_THUMB` 的数组 bug**：Task 3「本地模拟 CI 形态」
  用的是自签证书，`Import-PfxCertificate` 只返回**一张**证书，
  `"$($imported.Thumbprint)"` 插值正常。而 CA 签发的代码签名 pfx 默认带链，
  返回**数组**，同样的插值会成员枚举成 `AAAA BBBB`，`signtool /sha1` 直接报
  "No certificates were found"。也就是说**第一次真正配上证书的 tag 构建必死**，
  而本地怎么测都测不出来。修法：`$imported | Where-Object HasPrivateKey |
  Select-Object -First 1`，并在取不到叶子时 fail-fast。
  **教训：用自签证书验证签名流水线时，「单证书 vs 带链」是结构性差异，不是细节。**
- **`signtool` 定位按字符串排序**：`Windows Kitsin` 下混着 `x64`/`x86`/`arm64`
  这些非版本目录，且字符串序会把 `10.0.9.0` 排到 `10.0.26100.0` 前面。本机
  7 个 SDK 版本恰好都是 `10.0.<5位>.0`，新旧写法选中同一个 signtool——
  **潜伏缺陷，本机复跑无法暴露**，只能靠构造 `10.0.9.0` 的假目录证伪。
- **时间戳服务器是单点**：`/tr` 只有一个 TSA、无重试。TSA 限流/瞬时超时会让
  `signtool sign` 失败 → bundler 失败 → 整个 tag 构建失败；而 `publish` 只在
  `v*` tag 上跑，恢复要重跑那个 tag。改为 3 轮 × 2 个 TSA，轮间退避 5s/10s。
  连续失败仍然抛出——不静默放行未签名产物。
- **PowerShell 原生命令的退出码语义**：`$PSNativeCommandUseErrorActionPreference`
  是 PS 7.3+ 引入的，部分版本默认 `$true`，此时 `& $signtool` 非零退出会**直接抛
  异常**，重试循环形同虚设（本机 7.6.1 实测为 `False`，但 CI runner 的 pwsh 版本
  不由我们控制）。脚本里显式设为 `$false`，退出码一律自己判。

### 偏离计划

- 计划里没有「review 修复」这一步，本节是 Task 5 之后按 §4.1 独立 review 门槛
  追加的。修的是既有实现的缺陷，没有引入新功能、没有改变签名流水线的设计。

### 为什么这么改是安全的

- **无证书路径逐字节未动**：`tauri build (unsigned)` 步骤、`Detect signing
  certificate` 的判定逻辑都没碰。Task 5 的 dispatch 空跑结论（has_cert=false
  路径）继续成立，不需要重跑。
- **明文私钥的存活窗口真正收敛了**：原实现「写 pfx → 导入 → 删文件」没有
  try/finally，密码填错（首次配 secrets 时很可能）会让 `Import-PfxCertificate`
  抛在 `Remove-Item` 之前，明文 pfx 留在 `RUNNER_TEMP` 里陪跑到 job 结束，
  后面没有任何步骤兜底——这与该步骤注释里「存活窗口压到这一步内部」的声明
  直接矛盾。改为 try/finally 后，导入失败也删。已用「模拟 Import 抛异常」
  的测试双向验证（新写法删干净 / 旧写法确实留下文件）。
- **清理步骤不再瞄准整个证书存储**：`Remove signing certificate` 的守卫是
  `always() && has_cert == 'true'`，不是「导入成功」。导入抛异常时 `CERT_THUMB`
  从未写入 `GITHUB_ENV`，路径塌缩成 `Cert:\CurrentUser\My\`——指向整个 `My`
  容器而不是一张证书，且 `-ErrorAction SilentlyContinue` 会把后果吞掉。
  加空值 `return` 守卫。runner 是一次性 VM，实际影响有限，但这种「删错目标还
  静默」的形状不该留在仓库里。
- **绿色版说明不再撒谎**：`Pack portable zip` 没有 `if:`，两条路径都跑，有证书时
  zip 里的 exe 是签过名的（bundler 有二进制签名 pass），但生成的
  `README-绿色版.txt` 恒定写「本软件未做代码签名」并教用户点「仍要运行」。
  secrets 一落地，每个分发出去的绿色版都带着这句假话。改为由
  `steps.cert.outputs.has_cert` 经 `HAS_CERT` 环境变量推导。

### 验证

真证书路径依旧无法在本地端到端验证（见 Task 3 Limitation），所以这轮把每项修复
都拆成**可断言的最小片段**，从 workflow / 脚本里正则抽出真实文本执行，而不是
复制一份代码去测：

| 测试 | 覆盖 |
|---|---|
| T1 指纹选取 | 带链数组取到叶子；**旧写法确实产生空格拼接**（bug 复现）；单证书 pfx 不回归 |
| T2 清理守卫 | `CERT_THUMB` 为空时不执行 `Remove-Item`；有值时照常删 |
| T3 绿色版 README | `HAS_CERT=true/false` 两条路径的措辞；WebView2 段与许可声明在两条路径下都保留 |
| T4 signtool 定位 | 构造含 `10.0.9.0` / `x64` / `x86` / `arm64` 的假 kits 目录，新写法选中 `10.0.26100.0`；**旧写法确实选错**（bug 复现） |
| T5 pfx 清理 | 模拟 `Import-PfxCertificate` 抛异常：新写法删干净且异常不被吞；**旧写法确实留下明文**（bug 复现） |
| T6 时间戳重试 | 桩 signtool：失败 2 次后第 3 次成功且不抛；持续失败时抛出且尝试上限为 6 次（3 轮 × 2 TSA） |

结果 19 + 13 项全 PASS。另外：三个改动过的 pwsh 块 + `sign-windows.ps1` 过
`[Parser]::ParseFile` 语法检查；signtool 定位在本机真实 SDK 布局（7 个版本目录）
上复跑，选中 `10.0.26100.0\x64\signtool.exe` 并能正常调起。

**仍未验证**：真 CA 证书下的完整链路（带链 pfx 的实际导入结果、tauri 真实调用
signCommand、`verify /pa` 正向通过、真实 TSA 的重试行为）。等 secrets 就位后的
首次 tag 构建见分晓。

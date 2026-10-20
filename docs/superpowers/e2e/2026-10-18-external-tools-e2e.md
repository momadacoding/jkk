# 外部 diff / merge 工具 E2E 验证清单

- 日期：2026-10-21
- 分支：`worktree-external-tools`
- 设计：`docs/superpowers/specs/2026-10-17-external-diff-merge-tools-design.md`（场景 1–7 照抄 §7「真实窗口」）
- 计划 / 决策日志：`docs/superpowers/plans/2026-10-18-external-diff-merge-tools.md` 及同名 `-decisions.md`

## 验证方式说明

| 标记 | 含义 |
|---|---|
| ✅ 自动化 | 有对应的 `cargo test` / `pnpm test` 用例，可重复执行 |
| ✅ 真机 | 在 `pnpm tauri dev` 真实窗口里由人操作确认 |
| ⬜ 待人工 | 需要在窗口里点击才能确认，**尚未验证** |

## 造现场

只在临时仓库里做（`AGENTS.md` 最高优先级约束）：

```powershell
# 本机 UnityYAMLMerge 的路径按实际安装改；找不到时 scene 仓库会改用一个必然失败的假 driver，
# 场景 6 的「④b」提示照样能看到，但「合不了的部分自动弹出 BC」那一步就验证不了
pwsh -File scripts/e2e-external-tools-fixture.ps1 -UnityYamlMerge 'C:\Program Files\Unity\Hub\Editor\<版本>\Editor\Data\Tools\UnityYAMLMerge.exe'
```

脚本最后一行打印根目录，下面有 `merge`、`rebase`、`scene`、`drivers`、`diff` 五个仓库（各自的形状见脚本头注释）。
用完整个目录删掉。

场景 6 还需要在 `externalTools.json` 里加上 UnityYAMLMerge 工具和规则（写法见 README「按文件类型选工具」）。

## 清单

| # | 场景 | 仓库 | 结果 | 证据 |
|---|---|---|---|---|
| 1 | merge 冲突：选中 `Code/P.cs` 右侧显示「这个文件有冲突」、不显示 diff → 右键「在 Beyond Compare 中解决」→ BC 的四个标题是 `main（HEAD）` / `feat` / `共同祖先` / `结果（工作区）` → 在 BC 里解完、保存并关闭 → 右侧自动变成「未检测到冲突标记，检查后可以暂存」，下面是 HEAD 对比工作区的 diff → Stage → Commit | merge | ⬜ 待人工 | 判定与 diff 内容由 `tests/conflicts.rs`、`conflicted_file_diff_is_head_vs_workdir` 覆盖；**① → ② 的自动切换、BC 标题未在窗口确认** |
| 2 | rebase 冲突：`a.txt` 用 BC 解决，BC 左边标题是 `当前重放结果（HEAD，基于 main）`、右边是 `REBASE_HEAD <短 hash> c2 改第一行`；左边内容里有 `line5 c1`（第一个提交已重放） | rebase | ⬜ 待人工 | 标题与内容由 `rebase_titles_mention_rebase_head`、`rebase_second_commit_local_contains_first` 覆盖 |
| 3 | DU 冲突的保留和删除：`Code/Gone.cs` 右侧说明「HEAD 这边删除了这个文件，另一边修改了它」→「保留修改后的文件」立即变成已暂存的新增；另造一次，点「删除文件」先弹红色确认框（标题带文件名），取消什么都不变，确认后变成已暂存的删除 | merge | ⬜ 待人工 | 后端行为由 `keep_resolves_du`、`delete_resolves_ud`、`literal_pathspec_spares_lookalike` 等覆盖；确认框由 `LocalChangesView.test.tsx` 覆盖 |
| 4 | 普通改动和历史提交的外部 diff：Unstaged / Staged 的 `a.txt`、`目录 a/f[1] x.txt`、提交详情里改名的 `new-name.txt`；关掉 BC 后 `%TEMP%\jkk-external` 下没有残留、顶部不报错 | diff | ✅ 真机（2026-10-19） | Task 4 Step 5 实测，见决策日志 Task 4 |
| 5 | 用 VS Code 打开：Unstaged 文件右键「用其他工具打开 ▸ → VS Code」能打开对比；冲突文件右键「用其他工具打开 ▸ → VS Code」打开三方合并，**确认 Incoming 是 feat 那边、Current 是 HEAD 那边**（预设参数顺序 `$REMOTE $LOCAL $BASE $MERGED`） | diff / merge | ⬜ 待人工 | VS Code `--merge` 的左右顺序只能在真实窗口里看 |
| 6 | 场景文件：`Assets/Scenes/Main.unity` 右侧显示「这个文件配置了自动合并工具 `unityyamlmerge`，但仍处于冲突状态…」→ 按钮是「在 UnityYAMLMerge 中解决」（规则命中）→ UnityYAMLMerge 合不了的部分自动弹出 BC | scene | ⬜ 待人工 | 分类由 `failing_custom_driver_is_not_text_merged` 覆盖，按钮选工具由 `LocalChangesView.test.tsx` 覆盖 |
| 7 | 缺 driver：打开仓库顶部出现三条横幅（`e2e-missing` 红色附配置示例、`e2e-half` 红色、`e2e-badexe` 琥珀色）→ 提交图上右键分支 `pick` 的提交 → Cherry-pick → 先是原有的确认，再弹「合并前确认」红框，**焦点在 Cancel 上**，取消后什么都没执行 | drivers | ⬜ 待人工 | 文案由 `mergeDrivers.test.ts` 逐字覆盖，焦点由 `ConfirmDialog.test.tsx` 覆盖 |
| 8 | 二进制冲突 `img.bin`：右侧「二进制文件冲突」，只有 Stage，右键菜单里没有「中解决」 | merge | ⬜ 待人工 | `conflictView.test.ts`、`LocalChangesView.test.tsx` |
| 9 | 特殊路径冲突 `目录 a/-f[1] x.txt`：BC 正常打开、临时文件名是 `-f[1] x.LOCAL.txt` 这种形状、`$MERGED` 是工作区里的那个文件 | merge | ⬜ 待人工 | `chinese_space_bracket_dash_path`（Rust 两处） |
| 10 | 同一个文件 BC 还开着时再点一次「在 Beyond Compare 中解决」，顶部报「已在 Beyond Compare 中打开」 | merge | ⬜ 待人工 | `second_open_of_same_file_is_busy` |
| 11 | 新命令的 invoke 都能通：`read_conflicts`（场景 1 的提示能出来）、`open_external_merge`（场景 1）、`resolve_delete_modify`（场景 3）、`check_merge_drivers`（场景 7 的横幅） | — | ⬜ 待人工 | 参数名前后端同步：`{ path }`、`{ path, toolId, file }`、`{ path, file, keep }`、`{ path }` |
| 12 | 大仓库：`-Shape Big` 造 5 万文件仓库，打开后记录横幅出现的耗时；保存一个文件后界面不卡、横幅不闪 | big | ⬜ 待人工 | 引擎层耗时见下 |

## 大仓库的 driver 检查耗时

引擎层实测（`cargo test --release --test merge_drivers -- --ignored --nocapture perf_50k_files`，
5 万个被跟踪文件，500 个目录）：首次 272 ms，命中缓存 102 ms（2026-10-21，release 构建）。
第一次实测时发现 `check-attr --stdin` 管道死锁，已修复，见决策日志 Task 12。

## 已知未覆盖

- 第 1–3、5–12 条需要在真实窗口里点击确认，实现过程无法操作 GUI。
- 本机没有找到 UnityYAMLMerge（`C:\Program Files\Unity\Hub\Editor\*\Editor\Data\Tools\` 下没有），
  第 6 条「自动弹出 BC」那一步要在装了 Unity 的机器上验证。
- 非 Windows 平台没有验证。

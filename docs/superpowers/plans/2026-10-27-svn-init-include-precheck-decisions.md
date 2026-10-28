# 建库纳入集合：报错可读 + 提前失败 + 分阶段耗时 —— 实现日志

> 无独立计划文件：本次是用户在对话中选定的 A + B + 耗时日志三项修复（2026-10-27），
> 按 `AGENTS.md` §2.1「≥3 个 Task」落这份日志。

## 背景（用户在真实 Unity 工程上的报告）

- 扩展名填 `*`，「确认并建库」跑了约半小时后报 `include_set_mismatch`，漏收几百个 `.exe` / `.pyc`。
- 报错只看得到前 20 条路径，看不到裁决规则。
- 用户此前在同一目录里用过本地 git，后来删掉了 `.git`。

## 进度总账

| Task | 内容 | 状态 | 产出文件 | 验证 |
|---|---|---|---|---|
| 1 | A：不一致报告按裁决规则分组汇总，裁决规则一次批量查 | 完成 | `svn_include_check.rs`、`git_cli.rs`（`run_git_stdin_code`） | 新增 3 条 `svn_exclude` 用例先红后绿；原有 28 条不变 |
| 2 | B：`git add` 之前的只读预检，命中即失败（回退代价小） | 完成 | `svn_include_check.rs`（`dry_run_include_set`）、`svn_baseline.rs`（`ls_others`）、`svn_init.rs` | 新增 4 条 `svn_exclude` + 1 条 `svn_init` 用例 |
| 3 | 分阶段耗时：成功进 `InitReport.timings`，失败附在错误正文末尾，同时 `log::info!` | 完成 | `svn_init.rs`（`PhaseClock`） | 新增 2 条 `svn_init` 用例；`svn_backend` 序列化契约加 `timings` |
| 4 | 前端：错误正文保留换行可滚动；成功页显示耗时；引导语改写 | 完成 | `svnInit.ts`、`SvnInitWizard.tsx`、`styles.css` | `svnInit.test.ts` 新增 7 条先红后绿 |
| 5 | spec §6.5 序列补预检一行 + 说明段 | 完成 | `2026-09-12-svn-git-hybrid-design.md` | — |
| 6 | 处理 `/code-review high` 的 9 条（见下方「review 处理」） | 完成 | 同上 | 新增 1 条 `svn_init` 回归（先红后绿）、1 条 junction 实测、Rust 内联单测 1 条、vitest 1 条 |

全量验证（2026-10-28，review 修复之后重跑）：`cargo fmt --check` / `cargo clippy --all-targets`
无告警、`cargo test` 62 个套件全绿；`pnpm build`、`pnpm test`（53 文件 / 789 条）全绿。
**未做**真实窗口验证（见末尾）。

## 决策条目

### 计划没写的坑（实测，git 2.51.2.windows.1，2026-10-27）

1. **全局 ignore（`core.excludesFile`）挡不住白名单内的文件。** git 优先级是
   工作树 `.gitignore` > `.git/info/exclude` > `core.excludesFile`。我们生成的
   `!/<白名单>/**/*`（或按扩展名的反选）在 `info/exclude` 里，压过全局规则。
   实测：全局文件写 `*.pyc`，`Code/Sub/m.pyc` 的裁决是 `.git/info/exclude:5:!/Code/**/*`；
   对照组去掉 `info/exclude` 后裁决才变成全局文件的 `*.pyc`。
   → 用户那几百个 `.exe` / `.pyc` 只可能来自**工作树里的 `.gitignore`**（白名单内或其祖先目录），
   与「以前本地用过 git、只删了 `.git`」吻合。
2. **`git check-ignore -z -v -n --stdin` 的输出**是四段一组：`<source>\0<line>\0<pattern>\0<path>\0`。
   没匹配任何规则时前三段为空；被 `!` 规则反选时 pattern 以 `!` 开头（即「没被忽略」）。
   父目录被忽略时它会指认父目录那条规则（`.gitignore:2:[Bb]uild/` → `Code/Build/B.cs`）。
3. **嵌套 git 仓库**：`git ls-files -o --exclude-standard` 对它只列一条 `Code/Nested/`（带尾斜杠），
   不列里面的文件；里面的文件在 `check-ignore` 下是「没被忽略」。没有提交的嵌套仓库会让
   `git add -A` 直接失败（`'Code/Nested/' does not have a commit checked out`）。
4. **Rust 日志插件只在 debug 构建启用**（`lib.rs`），装机版里 `log::info!` 不落地。
   用户是在公司装机版上复现的，所以耗时必须进 UI，不能只写日志。
5. **`check-ignore --stdin` 在「一个都没被忽略」时退出 1**（多收一侧的常态），
   原 `run_git_stdin` 把任何非零当失败。新增 `run_git_stdin_code`：0/1 正常，≥128 仍是错误
   ——把 fatal 当成「没结果」会让整批裁决静默变成「查不到」。
6. **junction 底下的路径不会让 `check-ignore` fatal。** 实测与普通目录一样给出裁决
   （`a_junction_inside_the_whitelist_does_not_break_the_batch_rule_lookup` 钉住）。
7. **Rust `{:.1}` 对恰在中间的值按银行家舍入，JS `toFixed(1)` 四舍五入。** 实测
   `1.25 → 1.2` / `1.3`、`0.25 → 0.2` / `0.3`。两侧耗时格式化都改成整数四舍五入。

### 偏离 / 取舍

- **耗时进 UI，不改日志配置。** 备选是让日志插件在 release 也启用、写文件。没选：那会改变
  装机版行为（多一个日志目录），而且用户得自己去找文件。耗时在成功页 / 错误正文末尾直接可见。
- **预检复用错误码 `include_set_mismatch`，靠标题区分。** 两道核对含义相同、处理方式相同，
  前端引导语一份即可；新错误码只会让前端多一张表项。测试按标题里的「预检」断言是哪一道拦的。
- **预检函数名叫 `dry_run_include_set`，不叫 `precheck_include_set`。** 后者在
  `svn_sync_check.rs` 里已经存在，走的是真 `git add`、不套脏清单修正，给同步前用。同名不同义
  极易引错（review 指出）。
- **预检只接进初始化，没接进「重建基线」与同步前检查。** 重建走同一条 `git add` + 最终核对；
  `svn_sync_check::precheck_include_set`（更新 / 交付前都会跑）同样在空索引上做一次完整
  `git add`，大工程上每次同步都要把白名单读一遍。用户这次报的是建库，范围先收住，列为后续项。
- **前端引导语改写为「怎么处理」**，不再复述「不一致」本身：原引导开头 12 个字与最终核对的
  标题重合会被去重吞掉，而与预检标题不重合会显示——两种标题下行为不一致。改写后两种都显示，
  内容只讲处理办法（`.gitignore` 挪走 / 嵌套仓库移出白名单），由测试钉住。
- 后端报告在有 `.gitignore` 被点名时追加一句处理建议，与前端引导语有部分重叠，接受：
  后端那句是「上面点名的这个文件」，前端是通用说明。
- **成功路径多走两遍目录（review 第 6 条，接受）。** 预检做一次 Rust 遍历 + 一次 `ls-files -o`，
  最终核对再做一次 Rust 遍历。都只读目录项、不读文件内容，与 `git add` 的哈希压缩相比是小头；
  而且现在有了分阶段耗时，「纳入集合预检」那一行会直接给出它在真实工程上的代价。真成问题时
  再把磁盘期望集合算一次传给两道核对。
- **预检用临时空索引而不是真实索引（review 第 9 条，保留）。** 初始化时真实索引确实是空的，
  但写成「显式从空索引起步」不依赖调用时机，以后挪到别的入口也不会悄悄变成「相对用户暂存区
  的未跟踪文件」。代价只是一个不落盘的临时路径。

### review 处理（`/code-review high`，2026-10-28）

| 条 | 结论 | 处理 |
|---|---|---|
| 1 预检不套脏清单修正，误报阻断 | **成立，是本次引入的回归** | 预检两侧逐项复刻录制的修正：期望用 `server_expected_set`，实际对 `touched` 内的 `TakeBase` 加入、`Drop` 摘掉。`the_precheck_does_not_block_what_the_final_check_accepts_on_a_dirty_working_copy`（`?` 与 `M` 文件都被 `.gitignore` 挡住）先红后绿 |
| 2 批量查规则一处 fatal 全批丢失 | 触发条件实测不成立 | junction 实测不 fatal（坑 6），以测试钉住；`:` 开头的文件名在 Windows 上建不出来。未加逐个回退 |
| 3 与 `svn_sync_check::precheck_include_set` 同名 | 成立 | 改名 `dry_run_include_set`；同步路径换快实现列为后续 |
| 4 两侧取整不一致 | 成立（坑 7 实测） | 两侧都改整数四舍五入、按取整后的值分档；Rust 内联单测与 vitest 用同一张对照表 |
| 5 阶段之间的错误没有失败标记、回退不计时 | 成立 | Lying 判定挪进「基线体检」阶段内；「占有 .git」「回退」各成一个阶段；测试断言回退被列出且只有一个失败标记 |
| 6 成功路径多两遍目录遍历 | 接受，见上 | — |
| 7 查不到规则时引导语仍说「按规则分组」 | 成立（小） | 措辞补「查不到规则的单独成组」 |
| 8 `ls_others` 复制了 `add_all` 的 pathspec 处理 | 成立 | 抽出共用的 `w_pathspecs` |
| 9 预检的临时索引多余 | 保留，见上 | — |

### 为什么这么改是安全的

- **预检是只读的。** `ls-files -o` 只遍历目录，不写对象、不碰真实索引（候选索引从空起步，
  文件根本没有被创建）；测试 `the_precheck_catches_a_gitignore_without_writing_any_object`
  断言 `.git/objects` 前后都是 0 个文件。
- **预检在回退区内。** 它排在 `git init` 之后、`build_repo` 里，失败走原有的唯一回退点删掉
  刚建的 `.git`；`a_gitignore_is_caught_by_the_precheck_before_anything_is_recorded` 断言 `.git` 已删。
- **不会误报阻断正常仓库。** `the_precheck_passes_whenever_the_final_check_passes` 用三种白名单 /
  扩展名组合（含中文文件名、单文件条目、`*`）两道都跑；脏工作副本由 review 第 1 条的回归用例
  覆盖；原有 `svn_init` 全部成功路径用例（干净、脏、external、junction、中文、GBK）仍全绿。
- **空白名单不会退化成「全部」。** `ls_others` 空列表直接返回空集（AGENTS.md §6.2），有用例钉住。
- **最终核对的判定逻辑没动**，只改了失败时的报告文本；录进基线的 TREE 与改动前逐字相同。
- 计时包装 `PhaseClock::run` 只在闭包外计时，不改变任何步骤的顺序与错误传播；
  `record_server_tree_into`（sync_in / sync_out 用）行为不变，只是内部换成带计时器的同一份实现。
  回退阶段的闭包恒返回 `Ok`，`rollback` 自身的行为（吞掉删除失败）没有改变。

## 未验证 / 后续

- **真实窗口未验证。** 错误详情的滚动区、成功页的耗时块只在 `pnpm build` 与纯逻辑测试层面
  验证过，没有在 `pnpm tauri dev` 里看过实际渲染（AGENTS.md §6.1）。
- **半小时的真正大头还没实测。** 计时是为了下次在公司真实工程上拿到数字；在那之前
  「`git add` 是大头」仍是推测。
- 「重建基线」与同步前检查（`svn_sync_check::precheck_include_set`）换成只读预检，视用户需要再做。

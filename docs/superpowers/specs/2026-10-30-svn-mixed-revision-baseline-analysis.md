# SVN 混合版本基线与「跟进外部提交」——设计分析

- 日期：2026-10-30（2026-10-31 第三至六轮修订，2026-11-01 第七轮修订）
- 状态：C1a 已实现（未合并），实际范围见 §9。本文已按实现回写（第七轮，§12）；C1h、C1b、C2、C3 仍是计划。2026-10-30 用户已回答全部问题（§10）：选定 C-α，自动跟进，交付不强制更新整个项目，允许混合版本初始化。实现过程的全部细节见 `docs/superpowers/plans/2026-10-31-svn-mixed-baseline-c1a-decisions.md`（下称「实现日志」）
- 关联：主规格 `2026-09-12-svn-git-hybrid-design.md` 的 §6.3.1、§6.4、§6.6、§6.8、§7.3、§8.1、§8.2.1、§9.4、§10.3、§15、§16.1、§16.2
- 已过两轮独立 review（2026-10-30），并按 2026-10-31 与用户的讨论做了第三至五轮修订（第四轮新增 §6.13 TortoiseSVN 钩子；第五轮按用户的日常工作流逐项核对，新增 §1.5、§6.14；第六轮补充子目录局部更新的场景 U7，失同步判定改看最后修改 revision）；第七轮把实现中的修正回写到各章节，见 §12

## 1. 背景

### 1.1 用户的真实工作流

2026-10-30 用户反馈，原话要点：

- 日常：改自己的代码，在 TortoiseSVN 里只勾自己的文件提交
- 每周一次：对项目根目录 Update。不更频繁，因为全量 Update 会拉下美术、配置等改动，「本地环境很不稳定」
- 诉求：「我日常工作只需要保证我的修改是正常提交就行了」

### 1.2 现象

用户在 TortoiseSVN 里提交了 7 个 Lua 文件之后：

| # | 现象 | 原因 |
|---|---|---|
| 1 | 7 个文件仍在 jkk 的「本地改动」里 | `svn commit` 不碰 `.git`，工作分支仍停在旧基线上 |
| 2 | 「体检基线」弹框列出的恰好是这 7 个文件 | 基线树与磁盘不同，而这 7 个文件在 svn 看来是干净的，解释不了（`svn_init.rs:1535` 那份证据），拿单一 `baselineRev` 跟服务器比也对不上 |
| 3 | 「更新」报 `sync_in_out_of_sync`，红色横幅「工作副本处于多个 revision」 | 白名单 nodeRevs 多值，A16 只接管「整体前进」（`svn_sync_check.rs:88`） |

### 1.3 根因：基线模型假设白名单停在单一 revision

主规格 §6.6.2 的不变量是 `nodeRevs == {baselineRev}`；§16.2 进一步把「nodeRevs 单值」定为 `svn cat -r BASE` 录制的授权条件：

> 一旦允许 mixed-revision 工作副本进入这条流程，BASE 就不再是单一的 N，取回来的字节会来自若干不同 revision，而 trailer 只写得下一个数——那正是撒谎基线。

而 §6.6.1 自己也写明：`svn commit` 只推进被提交节点的 revision。所以对「只提交、不更新」的用户来说，mixed-revision 不是事故，是每天的常态。现有设计的每一条出口都要求先 Update：

| 出口 | 要求 | 位置 |
|---|---|---|
| 红色横幅的引导 | 对工程根目录（或白名单目录）跑 Update | `svn_preflight.rs:468`、`src/engine/svnSync.ts:95` |
| 「重建基线」 | nodeRevs 单值，否则 `svn_preflight_mixed_revision` | `svn_init.rs:1361` |
| 「更新」 | 自己就在项目根跑 `svn update` | `svn_sync_in.rs:240` |
| 「交付」 | `remoteRev == baselineRev`；提交后 D6 对白名单跑 `svn update` | `svn_sync_out_check.rs:123`、`svn_sync_out.rs:476` |
| §9.4 恢复流程（未实现） | 第 ⑤b 步对 W-path 跑 `svn update` 归一 | 主规格 §9.4 |

用户拒绝的不是「Update」这个动作本身，而是它会顺带拉下同事的改动。只更新白名单目录也一样：同事的 Lua 改动可能依赖还没拉下来的资源或配置。

### 1.4 顺带发现的两个检测盲区

**一、外部提交之后，jkk 在重新打开仓库之前不会发现：**

- `.git/info/exclude` 里有 `.svn/`，文件监听按 ignore 规则过滤掉了 `.svn` 下的全部事件（`commands/watch.rs:117`）；而 `svn commit` 不改工作文件，于是一个相关事件都没有
- 即使收到事件，第一档检测比的是工作副本根的 revision（`repoStore.ts:692`），而提交深层文件时根 revision 纹丝不动（§6.6.1，本次实测 E1 再次确认）

**二、只提交删除时，现行 nodeRevs 判据完全看不见。** E7 表明提交删除后节点从 `svn status -v` 里整个消失，其余节点的 revision 都不变，nodeRevs 仍是 `{N}`，判为 InSync。基线里却还留着这个服务器上已经不存在的文件。这是现行设计已有的漏洞，C-α 的映射比较顺带把它补上（§6.2）。

### 1.5 日常工作流（2026-10-31 用户补充）

| # | 场景 |
|---|---|
| U1 | Unity 工程的整个 SVN 仓库超过 500GB。每周一在项目根目录更新一次，约半小时；之后一周内不再在根目录更新 |
| U2 | 改一个文件后在 git 里暂存，再改、看第二次修改的 diff、再暂存，反复几轮；改完最终版后用 TortoiseSVN 或 jkk 提交到 SVN |
| U3 | 同 U2，但每轮做一个 git commit：SVN 上的一个改动，在 git 里对应好几个 commit |
| U4 | 开发中 stash，去修 bug，修完再继续之前的工作 |
| U5 | 建 feature 分支做大功能，开发一两周；中途切回 main 修 bug 并提交到 SVN，再切回 feature 继续；完成后整体提交到 SVN，或先 merge 回 main 再提交 |
| U6 | TortoiseSVN 里有一个 ignore 开头的 changelist，放着永远不能提交的本地改动（例如注释掉的 log 输出）；同一个文件里也可能有要提交的功能改动，需要像 git 的 hunk 一样区分 |
| U7 | 根目录一周只更新一两次，但子目录可能每天局部更新一次：例如开发道具功能时，会更新邮件、背包、时装等相关模块的代码。更新之后要在 git 里分清哪些是自己的改动，不能把从 SVN 拉下来的同事改动也算在自己身上 |

逐项核对见 §6.14。

## 2. 目标与非目标

| # | 目标 |
|---|---|
| G1 | TortoiseSVN 提交之后，不跑任何 `svn update`，「本地改动」就只剩还没提交到 SVN 的改动 |
| G2 | 基线不撒谎：每个文件的字节都标着它真实来源的 revision，体检仍能拿服务器证伪 |
| G3 | 每周一次的「TortoiseSVN 根目录 Update → jkk「更新」」从混合版本基线出发照常工作 |
| G4 | （后续阶段）在 jkk 里「交付」也能只提交、不更新 |

非目标：

- 改变 SVN 的行为，或替用户合并同事的改动
- 「跟进」被回退的工作副本（revision 变小）：仍按现状报红，多半是误操作
- 提交粒度小于文件（SVN 本来就没有 hunk 级提交）

## 3. 实测（临时 fixture，跑完即删）

E1–E11、W1–W14：2026-10-30，SlikSvn 1.14.5、git 2.55.0.windows.5、`file://` 仓库。W15–W16：2026-10-31，开发机 git 2.51.2.windows.1，scratchpad 下的纯文件与临时仓库。E12：2026-10-31，开发机 SlikSvn 1.14.2，scratchpad 下的 `file://` 仓库。探针脚本是一次性的，不入库；关键输出原样摘录如下。SVN 侧编号 E，git 侧编号 W，避免与 §2 的目标 G 混淆。

### 3.1 SVN 侧

| # | 场景 | 结果 |
|---|---|---|
| E1 | 改 a、b，只提交 a | a → r2；b（modified）、c、目录 `LuaScripts` 仍是 r1；根 revision 仍是 1 |
| E2 | 各节点 `svn cat -r BASE` | a: a-mine（r2 内容）、b: b-r1、c: c-r1——每个节点给出自己 revision 的内容 |
| E3 | 单一 `@1` 对白名单根做 `svn diff --summarize` | 报 a.lua（已提交）与 b.lua（本地修改），外加目录的属性行——与用户截图的形状一致。探针用的是 `--new=. -- LuaScripts`；现行代码按工作副本单元发 `--new=<wc_root>`、不带尾随 target（`svn_health.rs:159`），比对范围相同 |
| E4 | 同事提交 c（r3）后按 revision 分组体检：floor 组 `@1` 显式列 b c，a 组 `@2` 显式列 a | floor 组只报 b（本地修改，属于 S 集合）；a 组无输出 ✅ |
| E5 | 我改 c（BASE=r1，服务器已 r3）再提交 | `E155011: File '...c.lua' is out of date`——SVN 自己按节点拒绝过期提交；`svn status -u` 给 c.lua 标 `*` |
| E6 | 新增 d.lua 并提交 | d → r4，父目录仍是 r1 |
| E7 | `svn delete b.lua` 并提交 | b 从 `svn status -v` 里整个消失，父目录仍是 r1 |
| E8 | `svn status -v --xml`（含先 touch 一个文件）前后的 `.svn/wc.db` mtime | 不变——status 不写 wc.db。⚠️ 只测了 status，jkk 发的其他 svn 读命令见 §11 |
| E9 | `svn commit` 前后的 `.svn/wc.db` mtime | 变了 |
| E10 | 混合状态下整体 `svn update` | 全部节点归一到 HEAD（r6） |
| E11 | 文件放进 `ignore-on-commit` changelist，再经 `git stash push -u` / `apply`、来回切分支 | 每一步之后都仍在 changelist 里；stash 收走改动、文件回到未修改时，成员关系照样保留 |
| E12 | 两份工作副本：同事改 `Bag/b1.lua` 并提交为 r2；我只对 `Bag` 目录局部更新，`Mail` 下另有本地修改 | `svn status -v`：b1 节点 r2、最后修改 r2、作者 colleague；b2、b3 内容没变，节点也被推到 r2，最后修改仍是 r1、作者 me；`Mail` 仍是 r1。即**局部更新会推高整个目录下所有文件的节点 revision，内容变没变要看最后修改 revision**，见 §6.2 |

### 3.2 git 侧：工作分支跟进新基线的几种做法

W1–W9 的场景统一为：a 已提交 SVN、b 改了没提交、新增 d 已提交 SVN。新基线按 §16.1 的 R2a–R2c 录成 BASE 快照（干净文件取磁盘，脏文件取 `svn cat -r BASE`，`A` / `?` 剔除）。

| # | 做法 | 结果 |
|---|---|---|
| W1 | stash 路线（sync_in 的 S1 + S8–S10，不跑 `svn update`） | a 干净、b 仍是修改 ✅；但 `git stash apply` 报 `d.lua already exists`——正是 A17 处理的形状（三处同 blob 先从磁盘拿掉再 apply），探针没实现 A17，生产代码已有 |
| W2 | `git merge --ff-only svn/base` | ❌ `Your local changes ... would be overwritten`，不可用 |
| W3 | `git reset --mixed svn/base` | 只剩 b 与 c（c 原先是暂存的）✅；磁盘一个字节不动；代价是暂存状态丢失、内容仍在 |
| W4 | 工作分支上已有改 a 的 git commit，a 又被 TortoiseSVN 提交 | `rebase --empty=drop` 之后该 commit 变空被丢弃，`svn/base..work` 剩 0 个 ✅ |
| W5 | 快路径改用 `git read-tree -m <旧基线> <新基线>`（不带 `-u`），再 CAS 移分支；暂存区里：a 整文件暂存后被 svn 提交、c 暂存了没提交、e 未暂存 | a 变干净，c 仍是暂存，e 仍是未暂存 ✅；磁盘不动。⚠️ 只因为 a 恰好已暂存才成立，见 W8 |
| W6 | 同上，但 b 只暂存了一部分，之后整文件被 svn 提交 | `read-tree -m` 整体拒绝：`Entry 'LuaScripts/b.lua' would be overwritten by merge`；索引与磁盘原样未动 |
| W7 | 慢路径（现为手动跟进，§6.5）用 `git stash apply --index` | 暂存与未暂存的区分都保住了 ✅ |
| W8 | 同 W5，但 a 没暂存（最常见：改完直接在 TortoiseSVN 提交） | ❌ `Entry 'L/a.lua' not uptodate. Cannot merge.`——不带 `-u` 也检查工作区，`read-tree -m` 这条路走不通 |
| W9 | 按规则拼索引（§6.11.1）：临时索引先读入新基线，再把「已暂存、且新基线没改这个文件」的条目写回；换入真实索引后 CAS 移分支。场景：a 未暂存、已提交、提交后又改了一行；b 部分暂存、整文件已提交；c 暂存未提交；d 新增已提交；e 未暂存未提交 | a 只剩提交后改的那一行；b 变干净并报告「暂存已过期」；c 仍暂存；d 变成已跟踪且干净（不需要 A17）；e 仍未暂存 ✅；磁盘一个字节不动 |
| W10 | 分支上有自己的 commit：`git replay --onto <NEW> <旧基线>..work`。R1：C1 改 a 后被 TortoiseSVN 原样提交，C2 改 e 没提交，工作区另有暂存与未暂存改动。R2：C1 改 a 的第 2 行，之后工作区继续改同一行，TortoiseSVN 提交的是最终版 | R1：C1 变空被丢弃，只剩 C2；工作区、索引、磁盘全部未动 ✅。R2：冲突，整体失败，分支指针未动。⚠️ 本机 git 2.55 的 `git replay` 标着 `EXPERIMENTAL!`，且默认直接改 ref（`--ref-action=update`） |
| W11 | 同 W10，改用稳定的底层命令逐个重放：`merge-tree --write-tree --merge-base=<C^> <onto> <C>` + `commit-tree`，结果树与当前父提交（上一个重放结果）的树相同就丢弃 | R1：结果的树与 `git replay` 完全一致，C1 被丢弃，不改任何 ref ✅。R2：在 C1 处报冲突 ✅。jkk 已有同一套实现（`svn_deliver.rs:155`），只是没有「丢弃变空」这一步 |
| W12 | W8 的形态（没暂存、整文件已进 NEW）改用 `git read-tree -i -m <旧> <新>` | ✅ 成功，索引取 NEW。`-i` 关掉了工作区检查；W6 的形态（部分暂存、整文件已进 NEW）带 `-i` 仍然拒绝（独立 review 实测） |
| W13 | 暂存 line10=A，之后磁盘改成 B；同事改 line50，经局部 update 进了 NEW。即「索引 ≠ 旧 ≠ 新、索引 ≠ 新」，但变化**不是自己提交的** | 若按「重置为 NEW」处理，A 只剩旧暂存 blob 里的一份，**会丢**。改用 `git merge-file`（base = 旧、ours = 新、theirs = 索引）是干净合并，得到「line10 A + line50 COLLEAGUE」✅ |
| W14 | commit C 在 line10 后加 `foo()`；工作区把它挪到 line20 后，TortoiseSVN 提交的是挪过的版本；对象层重放 C 到 NEW 上 | `merge-tree` 不冲突，但结果里 `foo()` 出现 2 次，磁盘只有 1 次。「本地改动」会出现一条反向的 `-foo()`，而 `svn status` 是干净的 |
| W15 | 「暂存中间版、继续改、TortoiseSVN 提交最终版」的三种形态做 `git merge-file`（base = 旧、ours = 新、theirs = 索引）：① 暂存 line10=A，提交 line10=B；② 暂存 line10=A，提交 line10=A + line11 改动；③ 暂存 line10=A，提交 line10=A2 + line11 改动 | 三种全部冲突（exit=1）。②的提交内容包含暂存的改动，仍然冲突：diff3 下两侧的改动区间只要重叠或相邻、内容不同，就判冲突。**否定了第二轮修订里「自己提交的那种，合并结果通常就等于新 HEAD」的假设**，见 §6.11.1 |
| W16 | 临时仓库：`git add a`（内容 A）后等 2 秒，磁盘改成 B，再跑 `git status` 与 `git update-index --refresh` | `git ls-files --debug a` 的 mtime 仍是暂存那一刻文件的 mtime，没有被刷新成磁盘的新 mtime。当时的推论「暂存条目的 mtime ≤ 暂存时刻」只对这个形态成立，**已被 W26 推翻**，见 §3.3、§6.11.1 |

补充：现有 sync_in 的 `git stash apply` 也不带 `--index`（`svn_sync_stash.rs:161`），所以「暂存状态不保留」是现有流程的既有行为，不是 W3 新引入的。W7、W9 说明两条路径都能做到保留（W5 的 `read-tree -m` 被 W8 否决），见 §6.11.1。

### 3.3 C1a 实现前的第二批实测

E13–E21、W17–W28：2026-10-31 C1a 的 Task 0，开发机 SlikSvn 1.14.2、git 2.51.2.windows.1，临时 fixture，跑完即删。下表只记结论，场景与原始结论的唯一记录在实现日志「Task 0：实测」。

| # | 结论 | 用在 |
|---|---|---|
| E13 | `added`、`replaced` 没有可用的 `<commit>`；树冲突是 `item="added" copied="true" tree-conflicted="true"` | §6.2 |
| E14 | changelist 成员在 `</target>` 之后的 `<changelist>` 块里，要用 `descendants()` 遍历 | §6.11.2 |
| E15 | 工作副本被锁时 `svn status -v --xml` 仍 rc=0，只有目录 entry 带 `wc-locked="true"`；分不出活锁与陈旧锁 | §6.6 |
| E16 | `svn info --xml --targets` 对不存在的 target 报 W170000 + 末尾 E200009，存在的照常输出，要按 URL 对回；E160006 是 revision 不存在 | §6.4 |
| E17 | `--set-depth=exclude` 的节点从 `status -v` 消失，`svn info` 仍带 `Depth: exclude` | §6.2、§6.3 |
| E18 | 整单元 diff 根目录总有假的属性行，`--ignore-properties` 去掉；可带多个尾随 PATH，共用一个 OLDREV；PATH 未版本化时整条 E155010；没有 `--targets` | §6.4 |
| E19 | `info`、`diff`、`propget`、`ls`、`status`、`cat` 都不写 wc.db；`changelist`、`cleanup` 写 | §6.6 |
| E20 | 只改关键字展开值或换行符时 svn 视为干净，但磁盘 ≠ BASE。现行录制已有的盲区 | §7 R34 |
| E21 | 命令行 `svn ci` 会把文件移出 changelist（`--keep-changelists` 除外），且不认 `ignore-on-commit`；update、冲突、revert 不改成员 | §6.11.2 |
| W17 | 对象层重放的重命名、二进制、mode 边界形状已摸清；`-z` 下中文路径原样 UTF-8 | §6.5 |
| W18 | 「变空」只能比当前父的树，比 onto 的树两个方向都错 | §6.5 F2 ② |
| W19 | C' 的构造：临时索引 `read-tree C` → 写 C^ 的条目 → `write-tree` → `commit-tree -p C^`；P 只能放文件路径 | §6.5 F2 ① |
| W20 | 两次 `diff-tree -r -z --name-only --no-renames`（O→HEAD、HEAD→NEW）交集为空 ⇔「已全部提交」 | §6.5 F2 |
| W21 | Local-Only 泄漏只有「`diff -U0` 做行号映射」判得准，三方重放与 `git apply --unidiff-zero` 都不行 | §6.5 F3 |
| W22 | `merge-file --object-id`：rc = 冲突块数，二进制 255，不做 eol 规范化 | §6.11.1 |
| W23 | 持 `index.lock` 时副本上的索引操作正常，外部 add / commit / checkout 已有分支立即失败；`checkout -b`、`switch -c`、`symbolic-ref` 挡不住，`update-ref` 也不需要它 | §6.5 F5 |
| W24 | 外部进程开着 `.git/index` 时 rename 报 os error 5，要退避重试 | §6.5 F5 |
| W25 | `read-tree -i -m` 遇到「已暂存 ∩ NEW 改过」且结果不同就整体拒绝；assume-unchanged 会丢 | §6.11.1 |
| W26 | `add -p`、`--cacheinfo`、read-tree 取来的条目 mtime 全是 0，touch 后 status 会刷新成更晚；整文件 add 记的是文件自己的 mtime。**暂存条目 mtime 判据不成立** | §6.11.1 |
| W27 | `commit-tree -F` UTF-8 逐字节往返，无长度限制；不依赖 `interpret-trailers` | §6.1 |
| W28 | write-tree 包 commit 再还原会丢标志与 stat；要钉原始 index blob，且索引引用的暂存 blob 要单独挂进树里，否则 gc 之后内容没了 | §6.5 F5 ② |

## 4. 核心思路：基线 = SVN 的 BASE 快照

把基线的定义从「白名单在服务器 rN 的内容」放宽成：

```text
对每个受版本控制的 W-file f：
  基线里 f 的字节 = pristine(f) = 服务器上 f 在 r(f) 的内容
  r(f)            = f 的内容来源 revision，取值规则见 §6.1（E2 已证：BASE 按节点给各自 revision 的内容）
对每个已提交删除、不在工作副本里的文件 g：
  基线里没有 g，并记一条墓碑 g → d(g)：服务器在 d(g) 上确实没有 g
基线同时记下映射 R = { f → r(f) } ∪ { g → 墓碑 d(g) }
```

单一 revision 是 R 全部取同一个值、没有墓碑的特例，格式与行为完全不变。

### 4.1 为什么它不撒谎

撒谎基线的定义是「标签与字节不符」。现在每个字节都带着自己的标签，每个「缺席」也带着：

- `pristine(f)` 是服务器上 `f@r(f)` 的内容，而 `f@r(f)` 永不改变
- 第六轮：r(f) 可以小于 f 当前的节点 revision。只要 f 的最后修改 revision ≤ r(f)，f 在 r(f) 与当前节点 revision 之间就没改过，`f@r(f)` 的内容仍等于当前的 pristine，标签仍然是真的（E12、§6.2）
- 墓碑 `d(g)` 是一句可被服务器证伪的话：`svn info <url>/g@d(g)` 必须报不存在
- 录制方式不用改：§16.1 的 R2a–R2c 本来就按节点取 `-r BASE`；§16.1 的正确性论证（`D \ S == ∅ ⇒ 树 == 服务器`）只要逐 revision 组各跑一遍，再加上墓碑的核对，就原样成立（E4）
- §16.2 担心的「trailer 只写得下一个数」由映射 R 解决，不再需要单值授权条件

### 4.2 为什么它正好满足 G1

工作分支跟进到这个基线后，「本地改动」= 磁盘 − pristine = `svn status` 里白名单 W-file 的 `M` / `A` / `D`，也就是还没提交到 SVN 的改动。这正是用户脑子里的模型（W3 实测）。

前提：这个等式在「分支上没有自己的 commit」时严格成立。分支上有 commit 时，「本地改动」= 磁盘 − NEW_TIP，而 NEW_TIP 是把这些 commit 重放到新基线上的结果。如果某个 commit 改过的文件恰好又被外部提交过，重放即使不冲突，也可能造出与磁盘不同的内容（W14：一行代码挪了位置，重放后出现两份）。§6.5 F2 的「自己提交的路径取 SVN 版本」先把最常见的来源消掉；剩下的情形由 §6.12 条件 11 拦住，不自动跟进，交给人看。

## 5. 候选方案对比

| 方案 | 做法 | 优点 | 缺点 | 结论 |
|---|---|---|---|---|
| C-α | 基线 = BASE 快照 + 每文件 revision 映射 + 删除墓碑；新增「跟进外部提交」操作 | 满足 G1–G3；跟进时不发任何 SVN 写命令；单 revision 格式不变 | 改动面广：记账格式、失同步判定、体检、服务器侧核对、图、横幅都要改 | 用户需要多分支、提交图、在 jkk 里交付时推荐（§5.3） |
| C-β | 不动基线，只加一个「SVN 视角」视图，列 `svn status` 的 W-file | 成本最低、零风险 | 「本地改动」、暂存、分支 diff、红色横幅全都不变，只是多一个地方看 | 可作过渡，不解决问题 |
| C-γ | 只对白名单目录 Update，再走现有 A16 | 不用改代码 | 仍会拉下同事的代码，用户已明确拒绝 | 否决 |
| C-δ | 实现主规格 §9.4 恢复流程 | 设计已有 | 第 ⑤b 步同样 `svn update` W-path，本质与 C-γ 相同 | 否决（不解决本诉求） |
| C-ε | 把用户此前的 `.svnwip` 存档模型做成 jkk 的另一种 SVN 模式（§5.2） | TortoiseSVN 怎么操作都不受影响，失同步问题根本不存在；存档历史永久保留 | 不满足 G1（「本地改动」不等于「未提交到 SVN」）；jkk 要同时维护两套 SVN 模型 | 用户主要用存档 / 回退时推荐（§5.3） |

G4（在 jkk 里交付也不更新）依赖 C-α 的记账模型：交付后的新基线本来就是混合版本，没有 C-α 就无从表达。所以 C-α 是地基，G4 排在它后面（§9）。

### 5.1 与 `.svnwip` 存档方案的对比

`.svnwip` 是用户在 jkk 的 SVN 模式之前（2026-09-11）自用的一套脚本，放在公司工程里，不在本仓库：README 加 7 个 shell 脚本，约 400 行。它在 SVN 工作副本上叠一层纯本地 git：

| 命令 | 做什么 |
|---|---|
| `git wip` | `git add -A` + commit，随手存档，不碰 svn，瞬时完成 |
| `git up [svn 参数]` | 先存档 → `svn up --accept postpone`（参数原样透传，可局部、可指定版本）→ 查冲突 → 无冲突再记一笔「svn up」存档 |
| `git ci -m` | 用命令行 `svn ci` 提交，自动排除 `ignore-on-commit` changelist（命令行不认这个约定）；提交后再记一笔对齐存档 |
| `git ck` | 5 项体检：冲突、git 与 svn 增删调度一致性、changelist 分类、保护覆盖率、存档状态 |
| `git undo` | `git reset --hard` 到某个存档点，回退前检查 SVN 的 A / D 调度 |
| `clean.sh` | 清理未跟踪文件，保护 `.svn` 与 SVN 管理的空目录 |

#### 5.1.1 核心区别：git 里那条「基准线」代表什么

| | `.svnwip`（存档模型） | C-α（基线模型） |
|---|---|---|
| git 的 HEAD 代表 | 上一次存档时的磁盘 | 服务器内容（SVN 的 BASE） |
| `git status` 的意思 | 上次存档之后又改了什么 | 还有什么没提交到 SVN |
| 一个 git commit 代表 | 一个存档点，永久保留 | 一笔待交付的改动，交付或跟进后会被收掉 |
| 和 SVN 版本号的关系 | 完全不管 | 每个文件记自己的版本号 |

两者回答的是不同的问题：存档模型回答「我什么时候把文件改成了什么样、怎么退回去」，基线模型回答「我相对服务器改了什么、分成了哪几笔」。

#### 5.1.2 放进用户的工作流

| 场景 | `.svnwip` | C-α |
|---|---|---|
| TortoiseSVN 只提交、不更新 | 零摩擦。git 不管版本号，混合版本对它不存在 | 自动跟进（§6.12）；条件不满足时点一下「跟进」 |
| 想知道还有什么没提交到 SVN | git 回答不了，要看 TortoiseSVN 或 `svn st`（该工程全量约 14 秒） | 直接看「本地改动」 |
| 每周更新，分开「我的」和「别人的」 | 先存档再 `svn up` 再记一笔，两边各是一个 commit | 「更新」流程；别人的改动在图上是 SVN 节点，与 SVN 历史对齐 |
| 局部更新、按版本更新 | `git up LuaScripts`、`git up -r N` | 局部 update 后点「跟进」；「更新」本身只做全量 |
| 存档与回退 | 强项。随手存档，存档点永久保留 | 可以在工作分支上提交，但跟进时内容已进 SVN 的 commit 会被收掉（W4），存档历史留不住 |
| 并行多个任务（分支、stash） | 基本做不到：切分支会改磁盘，SVN 会看成本地修改 | 支持，分支的基准就是服务器内容 |
| 本地补丁 | `ignore-on-commit`，TortoiseSVN 提交时自动跳过 | jkk 的 Local-Only commit 只在 jkk 交付时有效，TortoiseSVN 看不见（主规格 §15 第 11 条）；继续用 `ignore-on-commit` 照样有效 |
| 能否证明基准正确 | 不需要，它从不声称自己代表服务器 | 体检拿服务器核对，可以证伪 |
| 实现规模 | 约 400 行 shell；删掉 `.git` 与 `.svnwip` 即完全退出 | 与 Phase 2c 相当，改十几个模块（§8） |

#### 5.1.3 各自的主要风险

`.svnwip`：

- **回退跨过一个「svn up」存档点时，会撤销同事的代码。** `git undo HEAD~2` 跨过了一次更新，`reset --hard` 把同事改过的文件退回旧内容，而 SVN 的 BASE 已是新版本，TortoiseSVN 会把它们列成「已修改」。只勾自己的文件时无害；全选提交就把同事的改动回退到了服务器上。`undo.sh` 只检查 A / D 调度，不检查这一点
- 全靠人守纪律：「五条红线」、更新前先存档等规则要自己记住，没有界面或流程兜底
- 存档提交里记的版本号取自根目录（`svn info --show-item revision`），混合版本与 externals 下不准。只影响提交说明，不影响数据

C-α：

- 复杂度高，状态与失败路径多，实现与维护成本远高于脚本
- 跟进后切到仍停在旧基线上的分支，旧内容被写回磁盘（§6.5 F5 ④、§6.11.3）。与上面「回退撤销同事代码」是同一类问题：git 改了磁盘，SVN 把它看成本地修改。这是 Overlay 结构的固有代价，两种模型都绕不开
- 多一个「跟进」步骤，以及外部提交能否被及时发现（§6.6）

#### 5.1.4 判断

用户的诉求是「日常只需要保证我的修改正常提交」，加上随手存档、改错能回退。就这组诉求而言，存档模型比 C-α 更贴合：它的优势恰好来自不建模 SVN，TortoiseSVN 的任何操作都碰不到它，本次的失同步问题在它那里根本不会出现。

C-α 的价值在别处：「本地改动」直接等于未提交到 SVN 的部分、多分支并行、与 SVN 历史接合的提交图、将来在 jkk 里交付。用户用不用得上这些，决定了 C-α 值不值得做。

### 5.2 C-ε：把存档模型做成 jkk 的另一种 SVN 模式

SVN 混合仓库初始化时，或之后在设置里，选择「基线模式」（现状 + C-α）或「存档模式」：

| 功能 | 存档模式下的做法 |
|---|---|
| 存档 | 白名单内 `git add -A` + commit；提交框即存档框 |
| 本地改动 | 上次存档之后的改动；界面上写明「不代表未提交到 SVN」 |
| 回退 | 回到某个存档点，回退前做两项检查：SVN 增删调度（同 `undo.sh`）；区间里有「svn up」存档时阻断或强提示（补上 §5.1.3 的第一条风险） |
| 更新 | 先自动存档 → `svn update --accept postpone`（全量 / 指定目录 / 指定版本）→ 查冲突 → 无冲突记一笔「svn up」存档 |
| 提交到 SVN | 继续用 TortoiseSVN；可选提供一个排除 `ignore-on-commit` 的提交入口（同 `ci.sh`） |
| 体检 | 冲突、调度一致性、changelist、保护覆盖率、存档状态（同 `check.sh`） |
| 不提供 | 基线、失同步判定、跟进、基线体检、交付、Local-Only；分支切换要么禁用，要么切换前提示「会改磁盘，SVN 会看成本地修改」 |

- 可复用：exclude 生成、`* -text` 行尾规则、`.svn` 保护、SVN 冲突检测、watcher 挂起、冲突解决界面
- 代价：两套 SVN 模型并存，每个功能在哪个模式下可用要划清；两种模式下「本地改动」与「commit」的语义不同，要在界面上写明；测试矩阵翻倍
- 规模：引擎侧明显小于 C-α（不碰基线记账与同步流程），主要成本在界面与模式隔离。正式评估要等 §5.3 选定之后

### 5.3 怎么选

先回答 §10 Q0：平时在 jkk 里实际用到哪些功能？

| 回答 | 推荐 |
|---|---|
| 主要看 diff、存档、回退，SVN 提交全在 TortoiseSVN 里 | C-ε，或者干脆继续用 `.svnwip` 脚本，jkk 只当 git 图形界面用 |
| 会用分支 / stash 并行做几件事，想在图上看 SVN 历史 | C-α |
| 将来想在 jkk 里交付，用 Local-Only 补丁保护调试代码 | C-α，后续做 C3 |
| 都想要 | 先 C-α 的 C1；存档需求用「在工作分支上提交」部分满足，接受跟进时存档被收掉 |

2026-10-30 用户的回答：平时用的是「看 diff」「存档（暂存区，以及 `ignore-on-commit` 之类的 changelist）」「创建分支」。

- 用到分支 → 选 C-α
- 用户的「存档」指暂存区与 changelist，不是用 git commit 留存档点。所以 W4 那种「跟进时 commit 被收掉」对用户影响小；但暂存区必须在跟进前后保持不变
- 由此带出三项要求，见 §6.11

## 6. C-α 详细设计

### 6.1 记账格式

单一 revision、没有墓碑：一字不改，仍是 `git-svn-id: <Repository Root>@N <uuid>`（§6.4.1）。

其余情况：不写 `git-svn-id`，改写一组新 trailer：

```text
svn: r1234（另有 7 个文件在更新的 revision，1 个文件已删除）

Svn-Base-Repo: <Repository Root> <uuid>
Svn-Base-Floor: 1234
Svn-Base-Rev: 1240 LuaScripts/Battle/Settle.lua
Svn-Base-Rev: 1240 LuaScripts/Battle/Reward.lua
Svn-Base-Rev: 1241 LuaScripts/UI/Panel/ShopPanel.lua
Svn-Base-Gone: 1242 LuaScripts/UI/Old/Legacy.lua
```

- **取值规则：** F = 白名单内所有文件节点与目录节点 revision 的最小值（不只看文件：目录 revision 更低时只看文件会发布假的单值）；c(f) ≤ F 的文件隐含 F，不列；其余文件记 n(f) 逐行列出。这样根目录更新后映射自动回到单值、旧格式，局部更新只让内容真的变了的文件进映射。正确性：f 在 c(f) 之后没改过，对任意 r ∈ [c(f), n(f)]，`f@r` 都等于当前的 pristine，而 F 与 n(f) 都落在这个区间里。`compute_map`（`svn_nodes.rs`）是这条规则的唯一实现，`added` / `replaced` / copied 的 c(f) 缺失时取 n(f) 作保守标签，算不出 n(f) 报 `svn_map_unknown_node`
- Floor 取最小而不是众数，是为了让「混合 ⇒ floor < 服务器 HEAD」、摘要行、图上的标注始终是同一个口径。代价是只要有一个文件长期停在旧 revision，floor 就会被它拖低，超限的报错要列出拖低 floor 的那几个文件
- 映射的键集合 = 基线树里的文件列表 + 墓碑，不从 `svn status` 推导。理由见 §6.2
- 墓碑由 R3b 产生（§6.2、§6.3），一直带着，直到 floor ≥ 墓碑的 revision。每周根目录 Update 之后 floor 前进，墓碑自然清空，trailer 回到 `git-svn-id` 旧格式
- 路径对空格、`%`、控制字符做百分号编码，其余字节原样 UTF-8。解析不能 `trim_end`：路径末尾可能是 U+3000
- 条目数上限 `MAX_MAP_ENTRIES = 5000`（`svn_basemap.rs`）。超限拒绝录制（`svn_map_too_large`），报错列出 revision 最低的 10 个文件。第六轮起映射只因内容真的变了的文件增长（§6.2），一周下来应远低于上限。**目录压缩没有实现**：第六轮设想的「同目录同 revision 压成一行」在 C1a 未做，上限够不够等真实使用（实现日志 D2）。原写「超限时引导用户对白名单目录 Update 归一」，与 U7 冲突，已删去
- message 改用 `git commit-tree -F <临时文件>`：现在用 `-m` 传（`svn_baseline.rs:684`），Windows 命令行上限约 32K 字符，几百条路径就会超；`-F` 读文件，不受上限影响，也不走 stdin（该文件 677 行注释说的那个坑）
- 最后一段只要有 `Svn-Base-` 行，就按新格式严格解析，失败返回 None，不退回旧格式：改坏的 message 里残留的 `git-svn-id` 不能被读成单值基线。重复路径、既是 Rev 又是 Gone、Rev 小于 floor、不认识的行一律读不懂。摘要行以 `git-svn-id:`、`Svn-Base-`、`Delivered-Rev:` 开头时垫两个空格，防伪造记账。写入前自检，读不回原值报 `svn_map_unreadable`

为什么混合时不保留 `git-svn-id: …@<floor>`：那一行的语义是「这棵树 == 服务器 r<floor>」，混合时它是假的。不认识新格式的读者（旧版 jkk、`git log --grep=^git-svn-id`、第三方工具）会信以为真；不写它，这些读者只会「读不出基线」。宁可读不懂，也不要读错。旧版 jkk 的实际表现见 §7 R2，§11 还要逐个核实。

备选（第三轮记下，C1a 未采用）：映射不写进 message，而是放在 git notes 或一个指向 blob 的 ref 上。实现用的是 trailer，5000 条约 293KB、解析约 4.9ms（实现日志 Task 12），没有必要换。

### 6.2 失同步判定（`check_sync`）

原判定（nodeRevs 多值 → `MixedRevision`）已删除。现在比较两张文件级的映射（`check_sync`，`svn_sync.rs`）：

```text
R = 基线记下的映射（键 = 基线树的文件列表 ∪ 墓碑；旧格式基线 = 树里每个文件都映射到 N）
C = 对 R 的每个键 f，取它当前的节点 revision n(f) 与最后修改 revision c(f)
    （svn status -v --xml 的 <wc-status revision> 与 <commit revision>，与现在同一次调用）；
    不在工作副本里、或条目是 unversioned / ignored / external 的记为「消失」
    另外收集 status 里有 revision、是 W-file、磁盘上是文件、但不在 R 里的条目（外部新增）

逐个键判断（第六轮修订，E12；取路径序第一个回退文件作 Diverged 的数值）：
  n(f) < R(f)                         → 回退
  item 是 added / replaced、copied、或没有 n(f) → 本地改动，不比 c(f)（E13）
  c(f) > R(f)                         → 内容变了：前进（changed）
  其余（n(f) ≥ R(f) 且 c(f) ≤ R(f)）   → 内容没变：视同相等，即使 n(f) 被局部更新推高

全部相等（墓碑对应的文件仍不在工作副本里） → InSync（即使是混合版本）
差异全部是「前进」：
  内容变了 / 外部新增 / 从工作副本中消失       → ExternallyAdvanced { changed, added, removed }  ← 可跟进
任一键回退                                → Diverged（回退，保持红色，不变）
目录节点带 wc-locked                       → WcLocked（SVN 操作进行中）
优先级：WcLocked > 回退 > ExternallyAdvanced > InSync
PinnedReset 的识别                          → 不变
```

几个定义上的硬要求：

- **键集合必须取基线树的文件列表，不能从 `svn status` 推。** `svn status --xml` 不给 kind（`svn_journal.rs:67`，M2-E1 实测）；扩展名配成 `*` 时 `path_is_w` 对目录也返回真（`svn_scope.rs:215`），而真实用户用的正是 `*`。从 status 推导会把目录混进映射。外部新增的条目需要判断是文件还是目录时，看磁盘
- **旧格式基线的 R 是「树里每个文件 → N」。** 这样只提交删除时（E7 形态），被删的文件在 R 里、不在工作副本里，判成 removed，补上了 §1.4 的第二个盲区
- **为什么比最后修改 revision（第六轮，U7）：** E12 表明局部更新会把整个目录下所有文件的节点 revision 推上去，不管内容变没变。按节点 revision 比较时，更新一个几百个文件的模块目录，横幅会报几百个文件「前进了」，映射要逐行列出整个目录、很快超过上限，每次局部更新还会录出一个树完全不变的基线 commit。改为：只有 c(f) > R(f) 才算内容变了；c(f) ≤ R(f) 时 f 在 R(f) 与 n(f) 之间没改过，`f@R(f)` 的内容就是当前的 pristine，映射保留 R(f) 不动，标签仍然是真的（§4.1）。跟进录新基线时不沿用旧映射，按 §6.1 的规则重算：c(f) ≤ F 隐含 F，其余记 n(f)
- **为什么只看文件：** 基线只包含 W-file 的字节。白名单目录里一个 `.png` 被外部提交、或目录节点的 revision 变化，基线内容都不受影响，不该挂横幅。这是对现行 nodeRevs（W-path 粒度，主规格 §6.6.2）的有意收窄，spec 里要写成显式偏离
- **removed 必须经服务器确认才能变成墓碑：** E7 表明提交删除后节点整个消失，但 `svn update --set-depth=exclude` 也会让节点消失（E17，主规格 §6.3.1 的漏收场景），本地分不开，所以横幅写「工作副本里已没有」，不写「已删除」。墓碑**不**从「旧映射有、现在消失」推导，统一由 R3b 产生（§6.3）：初始化没有旧映射（Q4）也走同一条路径
- ExternallyAdvanced 区分不了「自己提交的」与「局部 update 拉下的同事改动」，两者对基线的处理完全相同，但横幅措辞不能写成「你提交了」（§6.10）。对暂存区与分支 commit 的处理则要区分，按每个文件的最新提交作者判断（§6.11.1）
- A16 的「整体前进」成为 ExternallyAdvanced 的特例。sync_in 对任何 ExternallyAdvanced 都走 A16 接管（`rebuild_baseline`）。`removed` 的 depth-exclude 到接管的 R3b 才报 `svn_baseline_excluded_paths`，此时什么都没动

### 6.3 录制：泛化「重建基线」

在 R1–R5（主规格 §16）上做最小改动：

| 步骤 | 现在 | C-α |
|---|---|---|
| R1 | nodeRevs 单值，否则拒绝 | 跟进入口与初始化向导都去掉单值要求（`run_preflight_with(.., allow_mixed = true)`，初始化见 §10 Q4）；`~`、update / merge 中途仍拒绝；SVN 冲突（含树冲突、只有属性冲突）报 `svn_preflight_conflicted`；任一目录 `wc-locked` 报 `svn_preflight_operation_in_progress` |
| R2a–R2c | 按节点 `-r BASE` | 不变。⚠️ 现在 conflicted 按「取回 BASE」处理（`svn_dirty.rs:320`），带冲突标记的文件会被当成普通本地改动收下，所以冲突必须在 R1 拦住 |
| R3 | 写 `git-svn-id` | 按 §6.1，单值且无墓碑写旧格式，否则写新格式 |
| R3b | 单一 `@N` 体检 | 按 revision 分组体检 + 墓碑核对，见 §6.4；floor 组没被解释的 D 逐个向服务器 HEAD 确认，新墓碑进映射后重算一次 `compute_map`。初始化的体检挪到发布之前 |
| R3c | 复查 nodeRevs 仍是 `{N}` | 按同一规则重算映射，与 R2 时的比较。比映射而不是逐节点 revision：局部 update 只推高没改过的文件时映射不变，录下的内容仍对，不该中止 |
| R4 | CAS 移动 `svn/base` | 不变 |

R3b 的结论：比对或确认中途失败时立刻返回 `Unconfirmed`，不再往下判；全部做成时按 Lying > 已有墓碑被证伪（Lying）> `Excluded` > `Verified { new_gone }` 的顺序给结论，有 Lying 时不再问 HEAD。除 `Verified` 外都拒绝录制，所以先后只影响报错内容，不影响是否发布。

- **墓碑：** D 向服务器确认 `<url>/<rel>@<HEAD>` 不存在，HEAD 用同一个数字作 peg，记 d = 这个 HEAD
- **`Excluded`（depth-exclude，E17）：** 整单元 diff **根本不报**被排除的节点，所以不能只看 D。录制时每单元跑一次 `svn info -R --xml`（本地读）找 `<depth>exclude</depth>`，用 floor 把它们点名 diff；点不了名的（单元根、非 ASCII 路径）改用 `svn ls -R --xml <编码 URL>@<F>` 列出，按 W-file 口径筛。筛出来的与普通 D 走同一条 HEAD 确认：服务器上还在 → 拒绝（`svn_baseline_excluded_paths`），已删 → 墓碑；不是 W-file 的不阻断
- **`Unconfirmed` 拒绝：** 录制时 D 已知、确认却失败（HEAD 查询、`confirm_absent`、排除扫描、`svn ls`、第二条起的 diff、路径还原、旧墓碑核对），报 `svn_baseline_unconfirmed`，不当成 `NotChecked` 放行，否则已知的 D 会静默丢失。第一条 diff 就失败时一条都没比成（通常是断网），仍是 `NotChecked`、基线照常发布（报告里 `verified: false`）。离线录制丢已提交删除的风险见 R35

可选优化（不进第一期）：改成增量录制，只重录映射有变化的节点。正确性论证是「revision 没变 ⇒ pristine 没变」（服务器上 `f@r` 不可变）。但它绕开了 R2 从空索引起步、靠这一步甩掉已删文件的设计（主规格 §9.4.1），要单独论证，所以先用全量录制。

### 6.4 体检：按 revision 分组 + 墓碑核对

`check_baseline_map`（`svn_health.rs`）按单元、按 revision 双层循环发命令，发什么由纯函数 `plan_diffs` 决定。单值映射发的命令与改造前逐字相同。

```text
floor 组：每单元一条，svn diff --old=<unit_url>@<floor> --new=<wc_root> --summarize --xml
          映射不是单值时加 --ignore-properties（去掉根目录那条假的属性行，E18）
          结果里剔除映射单列的文件（它们归各自的组）
          结果里的 D 必须每一条都有墓碑；录制时交给 §6.3 确认，否则计入差异
其余每组 r：svn diff --old=<unit_url>@<r> --new=<wc_root> --ignore-properties --summarize --xml -- <PATH...>
          PATH 相对单元根，只放纯 ASCII 的，按命令行长度分批（MAX_DIFF_CMDLINE = 24000）
          含非 ASCII 的文件不走尾随 PATH：该组加一条整单元 diff，只看这些文件
          发之前用 status 剔除未版本化、ignored、丢失的 PATH，报进 unverified，单独按 status 判
墓碑：    svn info --xml --targets <文件> <编码 URL>@<d>，按 (url, revision) 对回，W170000 才算不存在
判据：    各组 D \ S 的并集 == ∅，且墓碑全部成立（主规格 §6.8.7 的逐组推广）
```

- E4 已验证分组的形状可行
- **没有 `--targets`：** `svn diff` 不支持（E18），只能分批。「逐文件 / 整单元按组大小选」的两种发法没有实现，也不需要切换阈值
- **非 ASCII 回退：** svn.exe 按系统代码页取命令行参数，代码页表示不了的字符变成 `?`，报 E155010。所以尾随 PATH 只放 ASCII；`--targets` 文件内容是百分号编码的 URL，文件放 `.git/` 下、传相对路径（不是 git 仓库才退回系统临时目录）
- 结果路径是 URL 编码的，剥掉单元前缀后要解码；单值体检也受益
- 代价：每个不同的 revision 至少一次 `svn diff`。一周的日常提交大约十来组，每组几个文件，全部走内网
- 服务器侧核对改为 `audit_server_set_map`：按 floor 列服务器清单时，已提交删除的文件会被误报成「服务器上有、本地没有」，要用墓碑剔除

### 6.5 新操作：「跟进外部提交」

两个入口，共用后半段：

| 入口 | 触发 | 走哪几步 |
|---|---|---|
| 跟进外部提交 | 判定为 ExternallyAdvanced | F1–F6 全部 |
| 分支跟进 | 判定为 InSync，但当前分支落后基线（`head_behind_base`：附着的非基线分支，`merge-base HEAD svn/base` 不是 `svn/base`，且是一条基线 commit） | 跳过 F1，以现有的 `svn/base` 为 NEW，单独跑一次 R1，走 F2–F6。旧基线取 merge-base（下称 scope）；普通入口同样用它，否则分支落后时会漏掉上一个基线带进来的路径 |

第二个入口兜住几种「基线已前进、分支没跟上」的状态：发布途中崩溃后重新跟进、分支 CAS 失败、F2 冲突只发布了基线、切换到落后的分支。没有它，这些状态只剩全量「更新」能出去，而那正是用户拒绝的。C1a 没有放弃入口，未收尾的跟进只能「继续」（`resume_follow`）。

共同的前置条件与「更新」相同（`svn_sync_check.rs`）：git 空闲、HEAD 附着在非基线分支、需要重放时待重放区间线性（`assert_linear`；F2 的「已全部提交」捷径不需要）、没有未收尾的 journal、工作副本没有被 svn 锁住（更新进行中，§6.6）、白名单内没有 SVN 冲突（与「更新」的前置检查同一道，`svn_sync_check.rs:159`）、git ≥ 2.43（`merge-file --object-id` 从 2.43 才有，`merge-tree --merge-base` 的 2.40 不够；在 F1 之前预检，报 `svn_follow_git_too_old`。到 F5 ② 才发现时基线已经动过了）。

```text
—— 准备阶段：只建对象，不拿同步锁，不改任何 ref ——
F1  按 §6.3 的 R1–R3b 录出新基线 commit NEW（含联网体检）
F2  在对象层把 svn/base..HEAD 上自己的 commit 逐个重放到 NEW 上，得到 NEW_TIP
      复用 svn_deliver.rs:155 的 replay（merge-tree --write-tree --merge-base），加两个开关：
      ① 「自己提交的路径取 SVN 版本」：对每个不是 Local-Only 的 commit C，按 §6.11.1 的作者 + 失焦快照规则
         找出「已被自己更晚的 SVN 提交取代」的路径集合 P；Local-Only commit 的 P 恒为空
         得到 C'，再重放 C'。merge-tree 对 P 看到的是「theirs 没改」，直接取 onto（NEW）的版本，
         不会冲突，也不会出现 W14 那种重复内容
      ② 「丢弃变空」：重放结果的树与**当前父提交**（上一个重放结果）的树相同 → 丢弃（W10 / W11）
         原本就是空的 commit 保留，与 git rebase 的默认行为一致
      「已全部提交」捷径（第五轮，U5）：scope..HEAD 改过的每个路径，HEAD 里的内容与 mode（含删除）
         都与 NEW 相同，且区间里没有 Local-Only commit → NEW_TIP = NEW，不重放，也不要求线性
         （区间里可以有 merge commit）。先于逐个重放判断
      分支上没有自己的 commit 时 NEW_TIP = NEW
      当前分支有任何一个 commit 冲突 → 不移分支、不碰索引，只发布基线，返回 NeedsManual，转「手动跟进」（见下）
      重放无冲突，但分支落后（scope ≠ 旧基线），而 scope..旧基线 改过、HEAD 与 NEW_TIP 不同的路径里
         磁盘仍是旧 HEAD 的内容（git 看着干净、SVN 看着已修改；`hash-object --stdin-paths` 按 attributes
         转换后与 HEAD 的 blob 比）→ 同样只发布基线，返回 NeedsManual（`staleDisk`，点名这些文件）。
         只移 HEAD 会让「本地改动」变成把 SVN 内容撤回去，`commit -a` 造出回退 commit；手动跟进的 rebase
         会把新内容写回磁盘（实现日志 Review 3 第 2 条）
      其他分支按 §6.11.3 各自重放，同一个原始 commit 只重放一次（记旧 → 新映射）
F3  检查：Local-Only 泄漏（见下）。§6.12 条件 11 属自动跟进，C1a 没有实现

—— 发布阶段：持锁，尽量短 ——
F4  拿同步锁、写 journal（flow=follow，记下全部旧 / 新 OID）。watcher 的挂起由命令层包住整个 follow，含准备阶段
    锁内复核：映射仍等于 F1 读到的（R3c）；HEAD 的符号引用与 OID、svn/base、各分支的 OID 都没变
    任何一项变了 → 释放锁，回到 F1（自动模式下最多重来一次，再不行就挂横幅）
F5  发布，顺序固定，每步先记 journal：
      ① CAS 移动 svn/base：<NEW> <旧基线>
      ② 建 .git/index.lock（create_new，建好立刻记 lock_created），**从这里一直持有到 ③ 的 rename**：
         钉住原始 index blob 与全部已暂存 blob，挂在 refs/mygitclient/follow/<opid>；
         按 §6.11.1 把新索引拼进 index.lock，记下它的 blob OID 与 `ls-files --stage` 指纹
      ③ 核对 HEAD 的符号引用 → CAS 移动当前分支：<NEW_TIP> <旧 HEAD>
         → 再核对 HEAD 解析到 NEW_TIP → rename index.lock 为 .git/index，释放 IndexLock
      ④ CAS 移动其他本地分支（§6.11.3；移动之前重查 worktree list，跳过在任何 worktree 里是 HEAD 的分支）
F6  清失焦快照、修剪钉住的 ref（保留最近 20 个，`KEEP_PINS`）、删 journal、释放同步锁、发 repo-changed
```

F2 ① 的取舍：被换掉的是 C 里中间版本的那部分改动，原始 commit 仍在 reflog 里，C 里其他路径的改动照常保留。这是按文件处理，不是整个丢掉 C：一个 commit 改了 a 和 e、只有 a 被 TortoiseSVN 提交时，C' 只剩 e 的改动，照常重放（与 W4 / W10-R1 的结果一致，但不再依赖重放碰巧不冲突）。P 为空时 C' = C，行为与原来相同。

整个操作不写磁盘，也不发任何 SVN 写命令（没有 update / revert / commit）：重放在对象层完成，工作区与磁盘从头到尾不动（W10 / W11 实测）。这是混合仓库里风险最低的写流程，也是 §6.12 敢让它自动执行的前提。

**为什么联网和重放都放在锁外。** 体检要联网，几秒是常态。同步锁拿着的时候，用户在 jkk 里的暂存、提交都会被 `assert_not_syncing` 挡回来（`vcs/svn.rs:93` 起）。准备阶段只建对象、不改任何 ref，放在锁外没有风险；发布阶段在锁内复核一遍再动手，持锁时间只剩几次 ref 与索引写入。

**为什么 IndexLock 要从读索引一直拿到 rename。** 在锁外读索引、拼好再换入，中间外部（或 jkk 自己）的一次 `git add` 会被覆盖；持锁之后外部 add、commit、checkout 已有分支都立即失败（W23）。`checkout -b`、`symbolic-ref` 绕得过，由 ③ 的两次 HEAD 核对兜住；`update-ref` 绕得过，由 CAS 兜住。④ 在 rename 之后，锁已放开，所以移动前重查 worktree，留一次 git 进程的窗口（§7 R36）。

**手动跟进（冲突兜底，`manual_follow`）。** F2 冲突时，说明同一处内容在 git commit 里和提交到 SVN 的版本里不一样，而 F2 ① 又没能认定 SVN 版本是自己更晚的版本（作者不是自己，或先后拿不准）。这需要人来判断，走会动磁盘的老路：基线已在 F5 ① 发布，这里复用 sync_in 的 S1 + S8–S10（stash → `rebase --empty=drop svn/base` → apply，含 A17），不跑 `svn update`，不切基线。journal 用独立的 `Flow::FollowManual`（线序 `follow_manual`），接 sync_in 的继续 / 放弃，放回步骤与 sync_in 只差三处：接到 `from_baseline_ready`、带 `--index`、收尾清快照。前置条件都在写 journal 之前：判定必须是 InSync（`svn_follow_manual_stale`）、R1、分支线性、白名单外没有已跟踪改动（`svn_follow_manual_outside_changes`，否则 stash 收不走）。apply 先带 `--index`（W7）；未合并就走冲突出口，不再 apply；status 没变才退回不带 `--index`，结局 `SyncInOutcome::StageLost`，这时**不 drop stash、不拆钉子**：暂存版本与磁盘不同的路径，暂存那一份只在 stash 里（比 `stash^2` 与 stash 的树），结局带上 stash OID 与这些路径，前端告诉用户怎么取回（实现日志 Review 3 第 1 条）；status 变了报 `sync_stash_apply_failed`、保留 stash。手动跟进永远不自动执行，入口只有确认框上的 `svn_follow_manual`。

**发布顺序为什么是「基线 → 拼索引进 index.lock → 分支 → rename → 其他分支」（第七轮修订）。** 原设计先换索引、后移分支，是怕「分支已移、索引是旧的」的反向暂存窗口。但拼好的索引留在 index.lock 里、整段持锁时，外部 git 提交不了，这个窗口不存在；先 CAS 分支还有一个好处：CAS 失败时什么都没动，删锁即可。每两步之间都可能崩溃：

| 窗口 | 窗口期状态 | 风险与恢复 |
|---|---|---|
| ① 之后、② 拼完之前 | 基线已前进，HEAD 与索引都没动 | 无。残留的 index.lock 记过 `lock_created` 才是自己的半成品，只能删，不能 rename；没记过就是别人的，报 `svn_index_busy`（活着的 `git commit` 等编辑器时 index.lock 删得掉，「删不掉才是别人的」不成立）。之后由「分支跟进」入口接手 |
| 分支 CAS 之后、rename 之前 | HEAD 已是新的，索引还是旧的，但 index.lock 在 | 外部 git 进不来。崩溃后重开：锁内容与 journal 记的 blob 对得上就往前补做 rename |
| 同上，但锁没了（用户手工删） | 比真实索引的 `ls-files --stage` 指纹（stat 不进去）：等于拼好的那份说明 rename 发生过，往前；否则把分支移回去重新跟进 | 这是唯一可能出现反向暂存的形状，恢复必须把它消掉：用户此时在 Rider / VS 里提交，会产生一个真实的回退 commit |

journal 记下每一步之前的 OID，重开时从断点补做。不变量：**绝不留下「分支已移、索引是旧的、index.lock 不在」**。拿不准时往回走是安全的：就算 rename 其实发生过，索引 == NEW_TIP、分支 == OLD，重新跟进时这些路径都走「索引正好等于新 HEAD」。

③ 的 CAS 失败、第二次核对失败（HEAD 不在 NEW_TIP 上）或 rename 失败（W24，os error 5 / 32 退避重试 1–2 秒后）：先把分支 CAS 回去，再删锁，`svn/base` 保持前进，交给「分支跟进」入口。分支 CAS 不回去时留着锁和 journal。

**例外：`checkout -b` 落在 CAS 之后。** 新分支建在 NEW_TIP 上，HEAD 随之在 NEW_TIP 上却不在原分支上（`head_left`）。这时把原分支移回去动不了 HEAD，回滚反而造出反向暂存，所以一律往前：rename 失败时留着锁和 journal 等「继续」；锁被手工删了就照 OLD → NEW_TIP 重拼一份（全按拿不准），再往前。

**F3：Local-Only 泄漏检查（`local_only_leaks`，`svn_leak.rs`）。** 主规格 §15 第 11 条：用 TortoiseSVN 直接提交会把本地补丁带上去。跟进时这件事第一次可以被看见。判据不能只看「commit 是否变空」：一个 Local-Only commit 改了多个文件，只有一部分被提交时不会变空。也不能只看路径交集（第五轮）：U6 里同一个文件既有 Local-Only 的 hunk、也有功能 hunk，路径必然有交集。三方重放也判不出：相邻行合成一个冲突块（W21）。实现按 hunk：对「变更集合 ∩ L 改过的路径」，用 `diff -U0 B NEW`（context 0，`Patch::from_buffers`）做 B → NEW 的行号映射，在映射位置比较 L 的 `+` 行，结论三值：映射位置逐字相等 → Present；有起点更早且改变行数的重叠 hunk → Unknown；否则按条消费重叠 NEW hunk 的 `+` 行，全部查到 Present、查到一部分 Unknown、一条都没有才 Absent。不重叠 → Absent。二进制或非 blob 且 NEW 改过 → 整文件 Unknown。方向是宁可多报。命中就在跟进报告里醒目告警，列出分支、commit、文件与 hunk，当前分支命中时不自动跟进（§6.12 条件 8）；其他分支命中时该分支不移动、标「落后基线」（§6.11.3）。F3 按原始 commit 计算，不受 F2 ① 换路径的影响。`NeedsManual` 只带当前分支的 leaks，其他分支要等下一次跟进。注册了 TortoiseSVN 的钩子时，这项检查前移到提交之前（§6.13.4），F3 仍保留。

**F5 ④：为什么其他分支也要处理。** 跟进前，已提交的文件相对旧基线是脏的，`git checkout B` 会带着它走，磁盘不变。跟进后它们变干净了，而分支 B 的树里还是旧内容，切过去时 git 会静默把旧内容写回磁盘，TortoiseSVN 随即显示「已修改」。对靠勾文件提交的用户来说，这等于一个现成的「回退自己的提交」。现有的「更新」之后也有同样的问题，但跟进每天都会发生，而用户又会建分支（§5.3），这不是边角情况。细节见 §6.11.3。

### 6.6 检测与触发

第三轮修订：第一期改用「窗口焦点 + wc.db 戳」，不再监听文件事件。C1a 已按此实现（`svn_wc_db_stamps`、`syncIfWcDbChanged`），焦点用 DOM 的 `focus` / `blur`，切标签等同于失焦 / 拿到焦点。

- **触发：** jkk 主窗口拿到焦点时，读每个 wc(P) 的 `.svn/wc.db` 的戳（mtime + size），与上次比较；变了才跑一次第二档完整校验。mtime 走 IPC 时是十进制字符串（纳秒超过 2^53）。wc(P) 清单在进程内缓存，戳变了或白名单变了才重新枚举，每次焦点只 `stat`；external 的增删一定写父 wc 的 wc.db。用户的操作顺序是「在 TortoiseSVN 里提交 → 切回 jkk」，焦点事件正好落在需要检测的时刻。另外 `repo-changed` 读到 HEAD 变了也跑第二档：切分支不写 wc.db，而「当前分支落后基线」是第二档的结论
- **为什么不在第一期用 notify：** 省掉了 Windows notify 下 wc.db 的事件形状、TSVNCache 后台进程会不会高频改写 wc.db、仓库外的工作副本根（`svn_wc.rs:23`，例如白名单落在指向外部目录的 external 里）要单独挂监听这三个未知数。E19 已证明 jkk 发的只读 svn 命令都不写 wc.db，第二档校验不会自激；`changelist`、`cleanup` 会写，只多一次无害的校验
- **失焦快照（第七轮，取代 T0）：** 最近判定是 InSync、没有校验在跑、wc.db 戳没变时，jkk 失焦就写一份快照 `FollowMark { at, head_oid, branch, staged, base_oid }` 到 `.git/mygitclient/follow-mark.json`（`git diff-index --cached -z --ita-invisible-in-index HEAD`，不扫工作区；先写临时文件再 rename；watcher 过滤这个目录），供 §6.11.1 判断暂存与 commit 是否早于外部提交。快照能挺过重启；跟进完成后清掉。四道闸都只会让它少写，少写的后果是走「拿不准」。校验结果用代次闸，晚到的旧结果不能把 ExternallyAdvanced 盖回 InSync
- **更新进行中（第五轮，U1）：** 每周根目录更新要半小时，期间用户切回 jkk，焦点检测会看到 wc.db 在变。工作副本被 svn 锁住时 `svn status` 仍 rc=0，只有目录 entry 带 `wc-locked="true"`（E15），不是报错，所以看这个属性，判 `WcLocked`：横幅显示 SVN 操作进行中，不跟进、不录制。分不出活锁与陈旧锁，不自动 cleanup。白名单外的目录被锁时看不到，那也不影响白名单。有钩子时由 Start-update 直接暂停 jkk 的刷新与跟进、Post-update 统一恢复（§6.13.1）
- **TortoiseSVN 钩子（§6.13）：** 注册后，Post-commit / Post-update 事件是即时且精确的信号；焦点 + wc.db 戳仍保留，兜住命令行 svn、IDE 插件与钩子失效的情形
- **以后：** 没有钩子、又真需要在 jkk 后台时也能发现（例如为了 C2 的通知），再补 notify 监听：在 `is_relevant`（`commands/watch.rs:117`）里对这个具体文件放行，其余 `.svn` 内容仍过滤；`wc.db-journal` 等 SQLite 旁路文件不放行；仓库外的工作副本根单独挂监听目标
- **交互：** 自动跟进（用户 2026-10-30 选定，Q1），执行条件、通知与开关见 §6.12。条件不满足时挂黄色横幅，给「跟进」按钮并写明没有自动执行的原因

### 6.7 「更新」（sync_in）从混合版本基线出发

| 步骤 | 混合版本基线下的情况 |
|---|---|
| 前置 | InSync（映射相等）直接放行；ExternallyAdvanced 走 A16 接管（`rebuild_baseline`，已能录混合版本），只有整体前进时 `catch_up_rev` 才是那个 revision，其余是预估的新 floor |
| S3 `checkout svn/base` | 磁盘变成各节点的 pristine；墓碑对应的文件本来就不在磁盘上 |
| S4 W-file 应为 0 修改 | 磁盘 == 各节点自己的 BASE，按构造为 0（`sync_in_from_mixed_baseline_checks_out_zero_w_modifications` 已验证） |
| S5 项目根 `svn update` | 混合版本照常归一（E10） |
| S7 录新基线 | 单值时命令与判据不变，归一后写回旧格式 `git-svn-id`；S5 之后仍不是单值时走 `record_mixed` |
| journal | `baseline_rev_before` 保留字段，存 floor；回退仍按基线 OID 做 CAS |

在 feature 分支上做根目录更新（U1 + U5，无论用 TortoiseSVN 还是 jkk「更新」）：磁盘上是 feature 的内容，SVN 会把同事的改动合进这些本地修改；跟进则是在 git 里把 feature 的 commit 重放到新基线上。两者的结果少数情况下不一致（与 W14 同类），条件 11 会挡住自动跟进，属于预期。界面上建议提示：更新前先切回 main（或任何没有未提交改动的分支）。

### 6.8 「交付」（sync_out）

- **第一期（已实现）：** 基线是混合版本时显式拒绝交付（`sync_out_mixed_baseline`），放在 `sync_out` 的最前面，提示「请先更新，或继续用 TortoiseSVN 提交后点跟进」。不要依赖 `remoteRev == baselineRev`（`svn_sync_out_check.rs:123`）碰巧拦住：它确实会拦，但给出的理由是错的；放在 `require_on_latest_baseline` 之后也不行，用户会先撞上 `sync_out_behind_baseline`
- **对日常工作流的影响（第五轮）：** U2 / U3 / U5 / U6 里「用 jkk 提交到 SVN」都要靠 C3。用户每天用 TortoiseSVN 提交，一周里基线几乎总是混合版本，第一期的交付基本用不上；现有交付提交后还会更新白名单目录（D6），这正是用户拒绝的。另外，交付送上去的是 commit，不是暂存区：U2 里只暂存、没 commit 的改动要先 commit 再交付
- **C3：不强制更新整个项目的交付**（用户 2026-10-30 确认要做，Q2）。思路：
  1. 用 E5 已证的按节点过期保护（`svn status -u` 的 `*`）取代 `remoteRev == baselineRev`：只要本次要交付的文件（及新增文件的父目录）在服务器上没有更新的版本，就允许交付
  2. 去掉 D6 的白名单 Update，交付之后不拉任何同事的改动
  3. D7a / D8 按「被提交的节点 → committedRev」更新映射，新基线照常是混合版本
  4. 交付清单必须跳过 `ignore-on-commit`（§6.11.2），否则会把本地补丁送上服务器
  5. 交付完成后当前分支的处理，与跟进的 F2–F4 是同一套（对象层重放、索引规则、其他分支）
- C3 必须单独出 spec：它动的是 D5 前后那段不可撤销的区间，主规格 §8.1.2、§8.2.1–§8.2.6、§8.6 的论证都要逐条重做，不和 C1 混在一起

### 6.9 提交图

不能简单地把混合版本基线挂到 `svn:<floor>` 上。`spliceHistory` 对同一个 revision 只认最新的那条基线（`src/engine/svnGraph.ts:108`），挂上去会顶掉 floor 节点原本的基线；之后按 `diffBase..baselineOid` 算 svn 节点的文件变化时（`svnGraph.ts:345`），会把用户自己后来的提交算进 r<floor> 里。

第一期的做法（已实现）：

- 混合版本基线不映射到任何 svn 节点。`graph.rs` 改用 `parse_message`，混合基线 `baseline_rev = None`，另给 `baseline_floor` / `baseline_newer` / `baseline_gone`，它就作为普通 git 节点画出来，父节点是上一条基线（已映射到 `svn:<floor>` 或更早），工作分支挂在它上面
- 节点上标注「svn/base：r<floor> + N 个文件在更新的 revision」，代替标题
- 没有父（`init_hybrid` 录的）或父基线的 revision 没拉到时，退回挂 `svn:<floor>`；`svn_log` 把 floor 补进锚点
- 被单值基线取代、没有 ref 与分支骑着的混合基线不再画，也不进 `deliveredFrom`
- `refs::collect` 过滤 `refs/mygitclient/`
- 用户自己的外部提交本来就作为 SVN 节点出现在主线上，不需要额外连线
- `svnGraph.ts` 的 `baselineMap`、`deliveredFrom`、`diffBase` 已逐个核对，见实现日志 Task 12

### 6.10 前端

- `describeSync` 新增 ExternallyAdvanced：黄色，「SVN 侧有 N 个文件有新内容」（只计内容真的变了的文件，§6.2），文件清单里标出作者（取 `svn status -v --xml` 的 `<commit author>`），让用户一眼看出是不是自己提交的。按钮只有「跟进」，不给「体检基线」：此时手动体检用的还是旧基线，必然把这些文件报成「基线与服务器不符」（`svn_init.rs:1535` 那份证据），而跟进本身就包含 R3b 体检
- `mixedRevision` 的文案只保留给「回退」类真正异常的情况
- 跟进完成后，体检弹框按分组结果展示，不再把已提交的文件列成「基线与服务器不符」
- 跟进完成后，在提示里列出 F3 的告警、F2 ① 换成 SVN 版本的路径、F5 ④ 移动过的分支和「落后基线」的分支
- 跟进带进来的改动按 revision 与作者列出，通知里如此，跟进录下的基线 commit 的说明里也如此（例如「跟进 r1245–r1250：同事 A、同事 B；自己的提交 r1248」）。这样在 git 历史里也能分清哪些是别人的改动（第六轮，U7）
- 「本地改动」按 SVN changelist 分组，见 §6.11.2
- 「我的 SVN 用户名」（`svnUser`，§6.11.1 的作者判断）：配置优先，否则 `LC_ALL=C` 跑 `svn auth`，按服务器 scheme + host + 端口匹配 realm，只取用户名。`file://`、没有缓存、同一服务器多个用户名、非 ASCII 用户名都取不到，一律当成「不是我」
- **C1a 现状：** 已做横幅、跟进报告弹框（要用户留意的节在前，每组截到 200 条）、用户名输入（跟进报告里，`uncertain` 非空或取不到用户时显示一行）、「落后基线」标记与切换确认框。没做：设置里的常驻用户名入口、体检弹框按分组展示、向导第 2 步的「混合版本」字样、changelist 分组。报告里 `dropped` / `rewritten` 只显示 OID 前 10 位

### 6.11 用户的实际用法带来的三项要求

2026-10-30 用户回答 Q0：平时用「看 diff」「存档（暂存区，以及 `ignore-on-commit` 之类的 changelist）」「创建分支」（§5.3）。前面各节默认用户不怎么用暂存区和分支，这里逐项修正。

#### 6.11.1 暂存区必须挺过跟进

用户拿暂存区当存档。原先的快路径要求「暂存区 == HEAD」，等于有暂存就走慢路径；而慢路径沿用 sync_in 的 stash apply，不带 `--index`（`svn_sync_stash.rs:161`），会丢掉暂存状态。结果就是每次 TortoiseSVN 提交后一跟进，存档就被清空，这不可接受。改为：

跟进不再看暂存区。分支上有没有自己的 commit 都一样：HEAD 从旧 tip 换成 NEW_TIP（§6.5 F2），新索引按下表逐个路径拼出来。表里的「旧」是旧 HEAD，「新」是 NEW_TIP；分支上没有 commit 时它们就是旧基线和新基线。

| 索引相对旧 HEAD | 新 HEAD 相对旧 HEAD | 再看什么 | 新索引 |
|---|---|---|---|
| 相同（没暂存） | 任意 | — | 取新 HEAD（新 HEAD 里没有就删掉） |
| 不同（已暂存） | 没变 | — | 保留暂存 |
| 不同 | 变了，且索引正好等于新 HEAD | — | 取新 HEAD（等价于保留） |
| 不同 | 变了，且索引不等于新 HEAD | 该文件最新提交的作者是我，且暂存早于快照：路径在 `diff-tree <base_oid> NEW` 里，且当前暂存 blob 等于失焦快照里的 blob | **丢弃暂存，取新 HEAD**。暂存的是中间版本，已被自己后来提交的版本取代 |
| 不同 | 同上 | 作者是我，但不满足上一行（blob 不等、快照里没有、路径不在 `diff-tree <base_oid> NEW` 里，或没有可信的快照） | 先后拿不准：保留暂存不动，不自动跟进（§6.12 条件 12），在横幅里列出这些文件交给用户确认 |
| 不同 | 同上 | 作者不是我 | 三方合并：`git merge-file`，base = 旧 HEAD、ours = 新 HEAD、theirs = 索引。干净 → 暂存合并结果；冲突、二进制两侧都改 → 取新 HEAD，列出「这几个文件的暂存内容没能保留」，并且不自动跟进（§6.12 条件 12） |

**为什么作者是我就可以直接丢弃（用户 2026-10-31 提出）。** TortoiseSVN 提交的是提交那一刻磁盘上的文件，而磁盘上的版本一定晚于之前的暂存，所以暂存的那份要么已包含在提交里，要么被后来的改动替换掉了；E5 又证明 SVN 按文件拒绝过期提交，提交成功说明那一刻的提交就是这个文件在服务器上的最新状态。丢弃暂存不会丢东西。不先试合并，是因为这种形态下三方合并几乎必然冲突（W15：只要暂存之后又改了同一处或相邻几行，就判冲突），第二轮修订里「自己提交的那种，合并结果通常就等于新 HEAD」的假设不成立。

**为什么还要看先后（第七轮改判据）。** 「作者是我 ⇒ 暂存的是中间版本」隐含了「暂存发生在提交之前」。反例：TortoiseSVN 提交了 B；跟进没有马上执行；用户接着改成 C 并暂存，又改成 D。这时作者仍是我，但 C 比 SVN 上的版本新，是真实的存档，丢弃就错了。

原设计用「暂存条目 mtime < T0（失焦时刻）」判断。W16 的推论被 W26 推翻：`add -p`、`--cacheinfo`、read-tree 取来的条目 mtime 全是 0，touch 后 status 会刷新，整文件 add 记的是文件自己的 mtime，**T0 判据作废**。改为失焦快照 `FollowMark`（§6.6），不依赖任何时钟：

- 暂存：该路径当前的暂存 blob 等于快照里的 blob ⇒ 这份暂存在外部提交之前就有了；不相等或快照里没有 ⇒ 拿不准，保留
- commit：commit 是快照 `head_oid` 的祖先（或就是它）⇒ 早于外部提交，F2 ① 换成 SVN 版本；否则正常重放
- **两条都只对 `diff-tree <base_oid> NEW` 里的路径成立。** 判定 InSync 不等于没有 SVN 提交：基线已发布、分支落后时写的快照比那批提交新，快照里的 commit 与暂存可能是提交之后的新工作。可替换的路径 = 作者是我 ∩ `after_mark`
- `base_oid` 缺失（旧格式）、读不到、不是 NEW 的祖先，或没有快照 ⇒ 一律拿不准：暂存保留，commit 正常重放

外部工具在 jkk 失焦前做的暂存同样被快照收进来，比「jkk 自己记暂存时刻」覆盖面更大。只会少一次自动丢弃，不会丢数据。代价：跟进只发布了基线之后再失焦，新快照的 `base_oid` 已是 NEW，再点跟进时 F2 ① 不再生效，走冲突出口，多一次人工判断。

**已知漏洞：另一台机器上的提交。** 用户在另一台机器上提交过同一个文件时，作者也是我，但这台机器上的暂存内容从没进过 SVN，丢弃是错的。这种情形应该很少，且旧索引已钉住（见下），能找回来。接受，spec 里写明。注册了 TortoiseSVN 钩子时，这个漏洞可以关掉，见 §6.13.3。

**有钩子事件时：** 作者 + 快照规则只是推断。TortoiseSVN 的 Post-commit 钩子能直接告诉 jkk「本机在什么时刻提交了哪些路径」，有事件的路径按 §6.13.3 判定，本节规则退为兜底。

**作者从哪来。** `svn status -v --xml` 里每个 entry 的 `<wc-status><commit author="…">` 就是该文件在其 BASE revision 上的最后提交者，与 §6.2 是同一次调用，不多发命令。「我」取 §6.10 的设置。

**最后一行（作者不是我）为什么不能直接「重置为新 HEAD」：** 变化若来自局部 update 拉下的同事改动，被重置掉的暂存内容从没进过 SVN，又只存在于旧暂存 blob 里，会丢（W13，独立 review 发现）。三方合并时暂存内容与同事改动合在一起保留下来。

**兜底：** 换入之前先把旧索引钉住（§6.5 F5 ②），`refs/mygitclient/follow/<opid>` 指向一个 commit，树里放原始 `index` blob 和 `blobs/<oid>`（相对 OLD 的全部已暂存 blob）。不用 write-tree：还原会丢 i-t-a、标志与 stat，冲突时根本写不出（W28）。必须挂 `blobs/`：索引对 git 是不透明的字节，它引用的暂存 blob 不可达，`gc --prune=now` 之后钉住的索引还在、内容没了。索引没有 reflog，journal 在 F6 就删了，没有这一步，任何被丢弃或重置的暂存都无处可找。保留最近 20 个（`KEEP_PINS`），报告里给出 `pinned_ref`。

**为什么不直接用 `git read-tree -m <旧> <新>`：** 不带 `-i` 时它会检查工作区，没暂存、直接在 TortoiseSVN 提交的文件（最常见的情形）会以 `not uptodate` 被拒（W8）。带 `-i` 能处理前三行（W12），但对后三行仍然整体拒绝（W6）。

**实现（`prepare_index_with`，`svn_follow_index.rs`）：** 持 IndexLock（§6.5 F5 ②）时，把真实索引复制进 `.git/index.lock`，以 `GIT_INDEX_FILE=<绝对路径>/.git/index.lock` 操作，这样没变的条目保留 stat 信息。否则从空索引起步，stat 全是 0，现场一万五千个文件的 `git status` 冷缓存要 53 秒，skip-worktree / intent-to-add 标志也会丢。真实索引在 rename 之前一个字节都不变：

1. 先把「已暂存 ∩ OLD→NEW 改过」的路径写成等于 NEW 的条目（`--index-info`），并记下 assume-unchanged / skip-worktree 的路径（第七轮修正：原写先写最终条目，最终条目与 OLD、NEW 都不同时 read-tree 整体拒绝，W25）；
2. 跑 `read-tree -i -m <旧> <新>` 处理其余路径；
3. 按表写最终条目（保留的暂存、三方合并结果、与 NEW 带进来的路径互为文件 / 目录时摘掉）；
4. 补回 assume-unchanged / skip-worktree 标志（`--index-info` 写过的条目两个都丢，只补「之前有、条目还在」的路径）；
5. `update-index --refresh`（`refresh_index_stat`），只写 stat，不碰磁盘。之后由 F5 ③ rename 成 `.git/index`。

几条边界：`merge-file --object-id` 的 rc 1–127 是冲突块数、255 是二进制，算 `LostToNew`，其余 rc（128 读不到对象、129 不认识选项）报错，不当成「合不了」。i-t-a 用 libgit2 读 `flags_extended` 认（`diff-index` 里它与暂存空文件同形）：作者是我时取 NEW，不算丢弃；作者不是我时 `LostToNew`。D/F 碰撞时 read-tree 退出 0 却静默吞掉一侧，所以预先判定并记 `LostToNew`（`collides_with`）。没有 `.git/index` 时报 `svn_follow_index_missing`。

- **验证：** W9 验证了前三行的五种情形；W13 验证了「作者不是我」的合并；W15、W26 与实现日志 Review 2 的探针 P2a–P2c 支撑「作者是我」两行的判据；分支上有 commit 的形态由 `tests/svn_follow.rs` 覆盖
- **前提：** 索引里没有冲突条目（stage 1–3）。「git 空闲」的前置检查已经覆盖 merge / rebase 中途
- **手动跟进**（§6.5，重放冲突或磁盘是旧内容时）：先 `git stash apply --index`（W7）；status 没变才退回不带 `--index`，并明确告知暂存状态没保住。退回之后 stash 与钉子都留着，点名暂存版本与磁盘不同的路径，给出取回办法（`git show <stash>^2:<文件>`、`git restore --source=<stash>^2 --staged -- <文件>`）
- 现有「更新」流程（sync_in S10）今天也会丢暂存状态，可以用同一手法一并修好。不进 C1 的话，记为后续项。C1a 先堵住数据丢失：有暂存版本与磁盘不同的路径时不 drop stash，结局同样是 `StageLost`（实现日志 Review 3 第 1 条）

#### 6.11.2 `ignore-on-commit` 等 changelist

用户的本地补丁放在 `ignore-on-commit` 里，靠 TortoiseSVN 提交时自动跳过。C-α 下这些文件永远不会进 SVN，所以跟进之后它们一直留在「本地改动」里。语义上是对的，但会和真正待提交的改动混在一起。

- **安全性：** E11 证明 changelist 成员关系挺得过 stash 与切分支；跟进不发 SVN 写命令。所以 C-α 不会破坏 changelist。「更新」里的 `svn update` 是否保留 changelist 是 svn 的原生行为，§11 补测
- **jkk 现状：** 全仓库没有读过 changelist。`svn status --xml` 把 changelist 成员放在单独的 `<changelist name="…">` 元素下，不在 `<target>` 里（2026-10-30 实测，`status -v` 时 target 下只剩目录和其他文件）。现有解析用 `descendants()` 遍历全部 `<entry>`（`svn_xml.rs:343` 等），所以这些文件没有被漏掉，只是不知道它们属于哪个 changelist。新增解析时要守住这一点：不能改成只遍历 `<target>`
- **要求（C1b）：** 「本地改动」按 changelist 分组，`ignore-on-commit` 组标注「TortoiseSVN 提交时会跳过」，默认折叠。changelist 名取 entry 父元素的 `name`，从已有的那次 `svn status --xml` 顺带读，不多发命令
- **要求（C3，在 jkk 里交付时）：** 必须同样跳过 `ignore-on-commit`，或把它们视同 Local-Only。否则在 jkk 里交付会把本地补丁送上服务器，而这正是 `.svnwip/ci.sh` 专门要绕开的坑（§5.1）
- **与 Local-Only（主规格 §6.7、D11）的关系（第五轮修订）：** 原写「两者并存时以 changelist 为准」，对 U6 不成立。`ignore-on-commit` 按文件，只能让 TortoiseSVN 默认不勾这个文件；U6 要的是同一个文件里按 hunk 区分。改为：
  - Local-Only commit 是按 hunk 的事实来源：注释 log 之类的 hunk 做成 Local-Only commit，它们不出现在「本地改动」里，跟进时照常重放保留，交付时被摘掉（`svn_deliver.rs` 已实现）
  - `ignore-on-commit` 是 TortoiseSVN 侧按文件的粗粒度保护，继续保留
  - 要把这类文件里的功能 hunk 提交上去：用 TortoiseSVN 时，走它自带的「提交后恢复」（提交框里右键文件 → Restore after commit，手工删掉不要的改动后提交，提交完自动恢复原文件；TortoiseSVN 手册 `tsvn-dug-commit.html`，经 Context7 查得）。用 jkk 交付时自动摘掉 Local-Only，不用手工删，但要等 C3（§6.8）
  - 两条路都由 F3（§6.5）与 Pre-commit 钩子（§6.13.4）按 hunk 检查本地补丁有没有被带上去

#### 6.11.3 分支

用户会建分支，所以 F5 ④ 不能只是「列出来提示一下」：

- **范围：** 包含旧 `svn/base` 的本地分支（`is-ancestor(旧基线, B)`）。基于更早基线的分支，本来就已落后，不在这次跟进里动，切换时按下面「落后基线」处理
- **跳过：** 在任何 worktree 里是 HEAD 的分支，以及配置了 upstream 的分支。SVN 模式本身不提供 remote 与 worktree（主规格 §1.2、§4.3），但外部 git 工具能加上它们，不能假设没有
- **没有自己 commit 的分支**（tip 本身就是一个基线 commit，能解析出基线 trailer）：跟进时一并 CAS 移到 NEW。它们不是当前分支，移动只改 ref，不碰磁盘，也不碰索引。**C1a 已实现**
- **有自己 commit 的分支：** 与当前分支同一套，在对象层把它的 commit 重放到 NEW 上（§6.5 F2，含「自己提交的路径取 SVN 版本」），不需要检出，不碰磁盘。几条分支共享的 commit 只重放一次（旧 → 新映射），否则 `commit_like` 每次取当前时间作为提交者时间（`svn_deliver.rs:270` 起），会得到 OID 不同的重复 commit。Local-Only 泄漏检查（F3）与条件 11 同样对它们做；先试 F2 的「已全部提交」捷径。第五轮修订把这一项从 C1b 挪回 C1a（§9）：U5 里每次在 main 上修完 bug 提交后 feature 都会落后，C1a 不做的话，每次切回 feature 都要走动磁盘的手动跟进
- **重放冲突、含 merge commit 且「已全部提交」捷径不适用、或命中 F3 / 条件 11 的分支：** 不动它，在分支列表上标「落后基线」。这不阻止当前分支的跟进，也不阻止自动跟进，否则一条有冲突的旧实验分支会让自动跟进永久失效。在 jkk 里切到它时，确认框给两个选项：
  - 「切换并手动跟进」（默认）：检出后对该分支走 §6.5 的手动跟进（带 stash 保护的 `rebase --empty=drop svn/base`，冲突交给用户解决）
  - 「仅切换」：附上 §6.5 F5 ④ 那段风险说明
- **新建分支：** 从当前 HEAD 建的，天然在最新基线上；从某个旧 commit 建的，同样标「落后基线」
- **「落后基线」的实现：** `svn_branches_behind` 走 libgit2，判据是「与 `svn/base` 有共同祖先，且分叉点不是它自己」，比 `head_behind_base` 宽：分叉点是交付 commit 的分支也标，但手动跟进对它报 `svn_follow_branch_unrelated`，只能「仅切换」
- **合并（第五轮，U5；C1a 未做）：** 设计是 SVN 模式下 jkk 的「合并」默认快进，不能快进时先 rebase 再快进，不产生 merge commit，因为跟进、「更新」与需要重放的交付都要求线性。但仓库里目前没有合并入口，这条没有落点，以后加入口时按此实现。外部工具产生的 merge commit，在内容全部提交到 SVN 之后由 F2 的「已全部提交」捷径收掉；还没提交完时，该分支不能跟进（`svn_follow_merge_in_range`）
- **外部 git 工具**（Rider / VS）切分支会绕过 jkk，挡不住，与主规格 §15 第 15 条同类。jkk 在 repo-changed 读到 HEAD 变了时跑第二档校验，发现 HEAD 落后基线就挂横幅，走 §6.5 的「分支跟进」入口。这样切过去（「仅切换」同理）时 git 已经把旧内容写回磁盘，分支跟进入口发现磁盘上是旧 HEAD 的内容就不走对象层，返回 NeedsManual（`staleDisk`），转手动跟进（§6.5 F2，实现日志 Review 3 第 2 条）

### 6.12 自动跟进

> C1a 没有实现本节（C2），只有「跟进」按钮。条件 11 没有实现；条件 9 里的目录压缩也没有。

用户 2026-10-30 选定自动跟进（Q1），而且会在 jkk 里做 git commit（Q3）。最初设想的「只有快路径自动、慢路径点按钮」在这个组合下几乎等于没有自动，所以 §6.5 把有 commit 的情形也改成了对象层重放：不写磁盘、不发 SVN 写命令。这是自动执行的前提。

**触发：** §6.6 的焦点 + wc.db 戳信号 → 第二档完整校验 → 判定为 ExternallyAdvanced（或命中「分支跟进」入口）→ 下表条件全部满足就直接执行 §6.5 F1–F6。

**条件**，任一不满足就不自动，改挂黄色横幅，给「跟进」按钮并写明是哪一条没满足：

| # | 条件 | 为什么 |
|---|---|---|
| 1 | 没有未收尾的 journal，同步锁空闲 | 不与「更新」「交付」或上一次跟进交错 |
| 2 | git 空闲：不在 merge / rebase / cherry-pick 中途，`.git/index.lock` 不存在；SVN 也空闲：工作副本没有被锁（§6.6） | 用户或外部工具正在操作时不插手 |
| 3 | HEAD 附着在非基线分支；需要重放时待重放区间线性 | 与「更新」同一道前置（`assert_linear`）；「已全部提交」捷径不需要线性（§6.5 F2） |
| 4 | 白名单内没有 SVN 冲突，没有 `~` | 冲突标记不能进基线（§6.3） |
| 5 | 「消失」的文件都已由服务器确认删除 | 否则可能是漏收（§6.2） |
| 6 | R3b 分组体检结果为 Verified | NotChecked（断网）也不自动：没验证过的基线不能在用户不知情时发布 |
| 7 | 当前分支的对象层重放无冲突 | 冲突需要人判断，走手动跟进（§6.5）。其他分支冲突只标「落后基线」，不阻止自动（§6.11.3） |
| 8 | 当前分支没有 Local-Only 泄漏（F3） | 本地补丁上了服务器，用户必须第一时间知道，不能被一次静默跟进带过去 |
| 9 | 映射条目没有超限（§6.1） | 超限说明一次带进来的改动异常多，交给人看 |
| 10 | git ≥ 2.43（F1 之前预检） | 对象层重放依赖 `merge-tree --merge-base`，拼索引依赖 `merge-file --object-id` |
| 11 | 当前分支上重放后保留下来的 commit，相对各自新父提交仍改动的路径，与本次外部变更（changed ∪ added ∪ removed）没有交集 | 有交集时重放即使干净也可能造出与磁盘不同的内容（W14），「本地改动」会出现反向改动。按 F2 ① 换路径、重放之后的改动算，所以「C1 改了 a 和 e、只有 a 被提交」这种常见情形不受影响：C1 重放后只剩 e。U7（每天局部更新）会让这条更常被触发：分支上的 commit 改过、同事也改了的文件就会命中；点「跟进」按钮仍可完成，属已知限制 |
| 12 | 拼索引时没有三方合并冲突，也没有「作者是我但先后拿不准」的路径（§6.11.1 表后两行） | 前者意味着某份暂存内容没能保留，后者意味着可能误丢一份比 SVN 更新的暂存，都必须让用户知道 |
| 13 | 用户没有撤销过针对同一份映射的跟进 | 撤销后判定又会变回 ExternallyAdvanced，不加这条会被立刻再次自动跟进 |

**通知：** 完成后弹一条不打断操作的提示，可展开详情：

- 跟进了哪些文件（标出作者，§6.10）；
- 丢弃了哪些变空的 commit，哪些 commit 里的哪些路径换成了 SVN 版本（F2 ①）；
- 哪些暂存被丢弃（作者是我）、哪些做了三方合并（§6.11.1）；
- 移动了哪些分支，哪些分支落后基线、为什么。

**开关：** 设置里可以关掉自动跟进，默认开。关掉后回到横幅按钮。

**撤销（可选）：** 跟进全程没碰磁盘。旧的基线与分支位置在 reflog 里，旧索引与暂存 blob 钉在 `refs/mygitclient/follow/<opid>`（§6.5 F5 ②）。在没有新的改动之前，可以在通知上给一个「撤销」：CAS 把分支和 `svn/base` 移回去，换回旧索引，并记下「用户撤销过这份映射」（条件 13）。做不做留给 spec 决定。

**为什么敢自动：** 全程不写磁盘、不发 SVN 写命令；每个 ref 都按旧值做 CAS；从读索引到换入索引一直持有 `index.lock`；冲突、验证不过或任何一条条件不满足就一个字节都不动。最坏情况只是 git 侧的指针被移动：分支与基线能从 reflog 找回，旧索引能从钉住的 ref 找回。

### 6.13 TortoiseSVN 钩子：精确模式

第四轮修订（2026-10-31，用户提出）。前面各节把 TortoiseSVN 当成黑盒，只能事后从 wc.db 与 `svn status` 推断它做了什么。用户提出给 TortoiseSVN 加一个与外部软件通讯的机制；查证后发现不需要改它的源码：TortoiseSVN 自带**客户端钩子脚本**（设置 → 钩子脚本，按工作副本路径注册）。

文档已确认（TortoiseSVN 手册 `tsvn-dug-settings.html`「Client Side Hook Scripts」，经 Context7 查得）：

- 钩子类型：Start-commit、Manual Pre-commit、Check-commit、Pre-commit、Post-commit、Start-update、Pre-update、Post-update、Pre-connect、Pre-lock、Post-lock
- 参数随钩子类型不同：`PATH`（临时文件，列出本次操作涉及的路径）、`DEPTH`、`MESSAGEFILE`、`REVISION`、`ERROR`、`CWD`、`RESULTPATH`（临时文件，列出操作实际碰过的路径）等
- 勾选「等待脚本结束」与「运行时隐藏脚本」时，脚本返回错误可以中止这次操作

**不用 `tsvn:precommithook` 等项目属性。** 它们是版本化属性，提交后对所有检出这个工程的同事生效。jkk 只改用户自己的本地环境。

**为什么不选「全程用 jkk、不用 TortoiseSVN」：** jkk 自己提交、自己更新时，检测与推断确实都可以去掉；但混合版本基线（§6.1–§6.4）照样要做，因为交付不更新整个项目（C3）之后基线仍是混合版本。而要替代 TortoiseSVN，jkk 得补齐勾选提交、`ignore-on-commit`、日志与 blame、冲突处理、externals，以及 Unity 二进制资源常用的 `svn:needs-lock` 加锁 / 解锁，等于重写一个 SVN 客户端。用户也表示难以避免使用 TortoiseSVN。所以「全程 jkk」只作为 C3 的远期形态，不作前提。

#### 6.13.1 用哪几个钩子

| 钩子 | jkk 拿来做什么 | 替掉或补上什么 |
|---|---|---|
| Pre-commit | 记下提交开始的本机时刻 t_commit；对照 Local-Only 路径清单拦截（§6.13.4） | 给 §6.13.3 一个准确的先后基准；R4 从提交后告警变为提交前拦截 |
| Post-commit | 补全这条提交事件：路径、`REVISION` | §6.11.1、§6.5 F2 ① 的作者 + 快照推断改为按事件判定（§6.13.3）；同时是即时的检测信号 |
| Post-update | 记一条更新事件：`RESULTPATH` 里的路径、`REVISION` | 跟进时确知这些路径的变化来自更新（同事的改动），直接走三方合并 |
| Start-update（可选） | 更新前把索引与分支位置钉住 | 更新出问题时有一个确定的回退点 |

#### 6.13.2 通讯方式：收件箱文件

- 钩子只做一件事：把参数与 `PATH` / `RESULTPATH` 文件的内容写成一条事件，放进 `.git/mygitclient/tsvn-inbox/<时间戳>-<序号>.json`，立即返回 0（Pre-commit 的拦截除外，见 §6.13.4）。不连接 jkk 进程，不读 git 仓库
- jkk 运行时立即处理（收件箱目录在文件监听里单独放行，或在下一次焦点检查时读取）；没在运行时，下次启动再处理。jkk 没开也绝不卡住提交
- 事件只是线索，真相仍是 §6.2 的映射比较：事件里的 revision 不大于基线映射里对应路径 revision 的，视为已处理并删除；收件箱的条目数与保留时长设上限
- 钩子本体的形态留给 spec：jkk 可执行文件带一个 CLI 子命令（要确认 Tauri 打包出的 GUI 子系统程序被 TortoiseSVN 调起时的行为、与单实例逻辑的交互），或随 jkk 分发一个极小的 exe。不用 PowerShell 脚本：启动慢，公司机器的 ExecutionPolicy 也可能禁止
- 多个 wc(P) 时每个工作副本根各注册一次；钩子按 `CWD` / `PATH` 找到对应的 jkk 仓库

#### 6.13.3 有事件时的暂存与 commit 判定

Pre-commit 与 Post-commit 合起来给出「本机在 t_commit 开始提交、提交成功的路径是 p」。t_commit 取提交**开始**的时刻：TortoiseSVN 读取的磁盘内容不早于这个时刻。第七轮修订：判定基准与 §6.11.1 一致，不用暂存条目 mtime（W26）与 git 的提交者时间，改看 t_commit 之前最近的失焦快照（`at` ≤ t_commit，下称 M）。对 §6.11.1 表的后三行与 §6.5 F2 ①：

| 情况 | 判定 |
|---|---|
| p 在本机的提交事件里，当前暂存 blob 等于 M 里的 blob | 暂存早于提交，是中间版本：丢弃，取新 HEAD |
| p 在本机的提交事件里，当前暂存 blob 不等于 M 里的（或 M 里没有） | 拿不准是不是提交之后才改的：原样保留（若暂存内容与提交的相同，早已落在表的第 3 行） |
| p 在本机的提交事件里，git commit 是 M 的 `head_oid` 的祖先（或就是它） | 该 commit 里 p 的改动换成 SVN 版本（F2 ①）；Local-Only commit 除外 |
| p 在本机的更新事件里 | 改动来自更新（同事），三方合并 |
| 钩子健康（§6.13.5），p 有变化却不在任何事件里，作者是我 | 来自另一台机器、命令行 svn 或 IDE 插件：按「先后拿不准」处理，保留暂存、不自动 |
| 钩子未注册或失效 | 退回 §6.11.1 的作者 + 快照规则 |

- M 是 jkk 自己写的，钩子不用读 git 仓库；`at` 与 t_commit 都用本机时钟。t_commit 之前没有 M 时按「拿不准」处理
- 钩子健康时，「作者是我、却不在本机事件里」不再按作者规则丢弃，§6.11.1 的「另一台机器」漏洞（R22）由此关掉，代价是命令行 / IDE 插件的提交不会自动跟进
- 只有 Post-commit、没有对应的 Pre-commit 记录时（例如 Pre-commit 钩子没注册上），t_commit 不可知，按「拿不准」处理

#### 6.13.4 提交前拦截 Local-Only 泄漏

- jkk 在 HEAD 变化时，把当前分支上 Local-Only commit 改过的路径，以及按 hunk 拆开的补丁，写进 `.git/mygitclient/local-only/`。钩子先拿 `PATH` 列表与路径清单求交集，没有交集直接放行，不读 git
- 有交集的文件再按 hunk 检查（第五轮修订，U6）：磁盘上这一刻的内容就是要提交的内容（用了「提交后恢复」时，也是用户删过本地改动之后的版本），逐个 hunk 判断 Local-Only 的改动是否还在里面（例如对单个 hunk 做 `git apply -R --check`，精度与上下文行的处理待实测，§11）。只剩功能 hunk 时放行。只按路径判断的话，U6 里每次正常提交功能 hunk 都会被拦
- 命中 → 钩子返回错误，TortoiseSVN 中止提交并显示错误内容：列出命中的文件、commit 与 hunk，提示取消勾选这些文件，或用「提交后恢复」删掉这些 hunk 再提交。jkk 设置里可以关掉拦截
- 钩子自身出错（清单读不到、超时）→ 放行（fail-open），由跟进时的 F3 事后兜底。拦截是附加的一道防线，不能因为它坏了就让用户提交不了
- Check-commit 在文档里与 Pre-commit 并列；它若拿得到勾选清单、返回错误时保留提交框，就比 Pre-commit 更合适（用户可以直接取消勾选再点提交）。两者的实际行为待实测（§11）；若 `PATH` 只给选择的根目录、拿不到勾选清单，拦截降级为 Post-commit 之后的告警

#### 6.13.5 注册与健康检查

- 由用户在 jkk 设置里明确开启。jkk 先列出将要写入的钩子（类型、工作副本路径、命令行），确认后写入 TortoiseSVN 的设置。存储位置（注册表或配置文件）与能否由程序写入待实测；不宜程序写入时，退回为给出步骤让用户在 TortoiseSVN 设置里手工添加
- 不覆盖用户已有的同类钩子：同一路径上已有同类型钩子时停下，提示用户
- 启动时与每次跟进前做健康检查：钩子仍在；命令路径仍指向当前的 jkk（便携版换了目录就会失效）；wc.db 显示有过提交、收件箱却没有对应事件时，判定钩子失效，降级到 §6.11.1 并提示
- 关闭开关时撤销注册
- 钩子是精确模式，不是前提：未注册或失效时，一切退回 §6.6 与 §6.11.1

#### 6.13.6 与自动跟进的关系

Post-commit / Post-update 事件是即时信号，jkk 在后台、不在焦点时也能马上跟进，而这原本是 C2 需要补 notify 才能做到的。所以有钩子时 C2 不必补 notify；notify 只留给没有钩子的环境。每周的根目录 Update 同样产生 Post-update 事件，整体前进是 ExternallyAdvanced 的特例，照常跟进。

### 6.14 按日常工作流逐项核对（第五轮）

对照 §1.5 的七个场景（U7 为第六轮补充）：

| # | 场景 | 怎么满足 | 所在分期 |
|---|---|---|---|
| U1 | 每周一根目录更新，其余时间不更新 | 一周内的日常提交让基线成为混合版本，由 §6.1–§6.4 记账，跟进不发 SVN 写命令；周一更新后映射归一、墓碑清空、trailer 回到旧格式；更新进行中跳过检测（§6.6）；在 feature 分支上更新的提示见 §6.7 | C1a |
| U2 | 暂存中间版、反复看 diff、最终版提交 | 用 TortoiseSVN 提交：暂存的中间版本按 §6.11.1（作者 + 失焦快照）或 §6.13.3（钩子事件）认定后丢弃，跟进后干净。用 jkk 提交：先 commit，再交付（§6.8） | TortoiseSVN：C1a，钩子精确判定在 C1h；jkk：C3 |
| U3 | 每轮一个 commit | 用 TortoiseSVN 提交：F2 ① 把这些 commit 里已提交的路径换成 SVN 版本，变空的 commit 丢弃，不留归档 ref（§10 Q7）。用 jkk 提交：交付把整段 commit 作为一次 SVN 提交 | 同 U2 |
| U4 | stash 修 bug 再回来 | 跟进不碰 stash；之后 apply 到新基线，只有修 bug 改了同一个文件时才出现真实冲突；changelist 不受影响（E11） | C1a |
| U5 | feature 分支 + main 上修 bug + 最后整体提交或 merge 回 main | main 上的 bugfix 提交后，feature 在对象层重放跟上（§6.11.3，已挪回 C1a）；冲突才标「落后基线」，切换时手动跟进。jkk 的合并默认快进 / rebase 未做（仓库里还没有合并入口）；外部工具产生的 merge commit 在内容全部提交后由「已全部提交」捷径收掉（§6.5 F2） | C1a；用 jkk 提交要 C3 |
| U6 | ignore changelist 里按 hunk 区分 | 本地补丁 hunk 做成 Local-Only commit；功能 hunk 用 TortoiseSVN「提交后恢复」或 jkk 交付提交；F3 与 Pre-commit 钩子按 hunk 检查泄漏（§6.5、§6.13.4、§6.11.2） | 检查：C1a / C1h；jkk 自动摘掉补丁：C3 |
| U7 | 子目录每天局部更新，要分清哪些是自己的改动 | 局部更新与自己的 TortoiseSVN 提交走同一个 ExternallyAdvanced → 跟进：同事的改动进基线，「本地改动」只剩自己的。按最后修改 revision 判定，只是被局部更新推高节点 revision 的文件不算变化，映射不膨胀（§6.2、E12）。暂存过的文件走三方合并（§6.11.1、W13）。局部更新产生 SVN 冲突时，先在 TortoiseSVN 里解决（条件 4）。通知与基线 commit 说明按 revision 和作者列出带进来的改动（§6.10）。分支上 commit 改过的文件被同事改了时，条件 11 会挡住自动跟进 | C1a；钩子事件让来源更确定：C1h |

结论：U1、U4、U7 在 C1a 完整满足（U7 依赖第六轮的「按最后修改 revision 判定」）；U2、U3 用 TortoiseSVN 提交时 C1a 满足，C1h 让判定更准；U5、U6 依赖第五轮的三处修订（按 hunk 检查泄漏、「已全部提交」捷径、其他分支重放挪回 C1a）。四个场景里「用 jkk 提交到 SVN」都要等 C3，因此 C3 排在 C2 之前（§9；用户 2026-11-01 确认，§10 Q8）。

## 7. 风险与安全论证

| # | 风险 | 处理 |
|---|---|---|
| R1 | 映射与字节不一致（基线撒谎） | 录制仍是 R2a–R2c；R3b 分组体检 + 墓碑核对，拿服务器证伪；R3c 复查映射，防录制期间又有提交 |
| R2 | 旧版 jkk 读到混合版本基线 | 不写 `git-svn-id`，旧版读不出 trailer。⚠️ 现在 `refreshSvnSync` 会吞掉错误（`repoStore.ts:482`），旧版表现为没有横幅，而不是报错。「更新」「交付」的前置会因读不出 trailer 而拒绝。旧版「重建基线」不读 trailer（`svn_init.rs:1348` 起），工作副本已经整体归一时它会成功，录出的是正确的单值基线，结果无害。旧版读到 flow=follow 的 journal 会报 `svn_journal_corrupt`，fail-closed。§11 逐个核实 |
| R3 | 「消失」的文件其实是漏收（`set-depth=exclude`），被当成删除录进基线 | §6.3：R3b 向服务器 HEAD 确认，还在就拒绝（`Excluded`），确认失败也拒绝（`Unconfirmed`）；整单元 diff 看不见被排除节点，另由 `svn info -R` 扫出来；墓碑在每次体检时再核对一次 |
| R4 | 外部提交带上了 Local-Only 补丁 | F3 按 hunk 告警，不只看 commit 是否变空；注册钩子后在提交前拦截（§6.13.4） |
| R5 | 手动跟进时 stash 撞名 / 冲突 | 只在对象层重放冲突时才走手动跟进；复用 sync_in 已有的 A17 与冲突出口，内容不丢（主规格 §7.3 S10） |
| R6 | 暂存内容丢失（用户拿暂存区当存档，§6.11.1） | 按规则拼索引保留暂存（W9）；作者是我且失焦快照证明暂存早于 SVN 提交才丢弃（W15、W26），拿不准时保留且不自动；作者不是我时三方合并（W13），合并冲突才取新 HEAD 并列出，且不自动跟进；换入前把原始索引与全部暂存 blob 钉在 `refs/mygitclient/follow/<opid>`，gc 后可恢复（W28）；手动跟进先 `stash apply --index`（W7），status 没变才退回并告知，退回后（以及「更新」里暂存与磁盘不同时）不删 stash、点名路径（Review 3） |
| R7 | 映射过大（局部 update 了大目录，或一周里带进来的改动很多） | 按最后修改 revision 判定，映射只因内容真的变了的文件增长（§6.2，第六轮）；超过 5000 条拒绝录制，目录压缩未实现（§6.1）；message 走 `-F`；体检命令分批 |
| R8 | 交付从混合版本基线出发 | C1 显式拒绝；C3 单独出 spec（§6.8） |
| R9 | Unity 以 `share=Read` 占用文件 | 跟进不写磁盘，不涉及；只有手动跟进与 sync_in 相同，已有 checkout 后核对 |
| R10 | 多个 wc(P)（externals） | 映射键用仓库相对路径；P4 保证同一仓库，revision 可比；体检按单元、再按 revision 双层循环；焦点检测对每个 wc(P) 的 wc.db 各读一次戳 |
| R11 | 跟进后切到仍停在旧基线上的分支，旧内容被写回磁盘，可能被用户经 TortoiseSVN 提交成回退 | F5 ④：所有分支都在对象层重放后一并移动（第五轮挪回 C1a）；冲突、不线性或命中检查的标「落后基线」，切换时默认「切换并手动跟进」（§6.11.3）；「仅切换」或外部工具切过去之后，跟进发现磁盘是旧内容就不走对象层，转手动跟进（§6.5 F2，Review 3） |
| R12 | 跟进中途崩溃 | 发布顺序固定为基线 → 拼索引 → 分支 → rename → 其他分支（§6.5）；journal 记断点，`resume_follow` 补做或往回走，每条路都能走到删 journal；不变量是不留「分支已移、索引是旧的、index.lock 不在」；「基线已前进、分支没跟上」由「分支跟进」入口接手 |
| R13 | 白名单内有 SVN 冲突时录制，把冲突标记收进基线 | 跟进前置检查拦住冲突（§6.5） |
| R14 | `ignore-on-commit` 里的本地补丁被 jkk 交付送上服务器 | C1 不从混合基线交付；C3 交付时必须跳过该 changelist（§6.11.2） |
| R15 | 以后改 XML 解析时只遍历 `<target>`，changelist 成员从 SVN 侧逻辑里静默消失 | §6.11.2 写明约束；补一条带 changelist 的 fixture 用例钉住 |
| R16 | 自动跟进在用户正操作时改写索引与分支 | §6.12 条件 1、2；准备阶段在锁外只建对象，发布阶段锁内复核；从读索引到换入索引一直持有 `index.lock`，外部的 add / commit / checkout 已有分支都插不进来，`checkout -b` 由两次 HEAD 核对兜住；ref 全部 CAS；完成后通知。代价：发布的那一瞬间，jkk 里的写操作会报「同步中」 |
| R17 | 对象层重放与 git rebase 的结果不一致 | 逐 commit 三方合并，语义等价 cherry-pick；W11 与 `git replay` 的结果逐树比对一致；复用 `svn_deliver.rs` 已经过 review 的实现，只加「丢弃变空」（判据是与当前父提交的树相同）与「自己提交的路径取 SVN 版本」 |
| R18 | 依赖实验性命令 | 不用 `git replay`（`EXPERIMENTAL!`，W10），只用稳定的 `merge-tree --write-tree` 与 `commit-tree` |
| R19 | 重放干净，却造出与磁盘不同的内容（W14） | F2 ① 先把「自己更晚提交的路径」换成 SVN 版本，消掉最常见的来源；剩下的由 §6.12 条件 11 拦住，不自动跟进，列出这些 commit 交给人看；§4.2 写明前提 |
| R20 | 几条分支共享的 commit 被各自重放成 OID 不同的重复 commit | 同一个原始 commit 只重放一次，记旧 → 新映射（§6.11.3） |
| R21 | 撤销之后立刻被再次自动跟进 | §6.12 条件 13 |
| R22 | 作者规则误丢暂存或 commit 里的改动：另一台机器上的提交（作者也是我），或「我」配置错 | 失焦快照（含 `base_oid` 路径范围）挡住「提交之后才暂存 / 提交」，Local-Only commit 不进 F2 ①；另一台机器的情形在没有钩子时接受（§6.11.1），钩子健康时关掉（§6.13.3）；旧索引与暂存 blob 钉住、原 commit 在 reflog，能找回；通知里列出所有被丢弃 / 换掉的路径；「我」取不到时一律当成「不是我」，退回三方合并 |
| R23 | 失焦快照失真：没写上、损坏、写于提交之后 | 没有快照或损坏 ⇒ 拿不准；前端四道闸（判定 InSync、无校验在跑、wc.db 戳没变、读戳成功）只会少写；`base_oid` 限定路径范围，不是 NEW 的祖先时拿不准。打开仓库后第一次失焦没有戳可比，提交在 jkk 前台完成的形状放得过去，下一次焦点之后就有了。代价只是少一次自动丢弃 |
| R24 | 钩子失效（jkk 换了目录、被用户在 TortoiseSVN 里删掉、事件丢失），jkk 却仍按「钩子健康」判定 | 启动时与每次跟进前做健康检查；wc.db 显示有提交而收件箱没有事件即判失效、降级并提示（§6.13.5）。失效方向是「本该有事件的路径被当成不在事件里」，按 §6.13.3 落进「拿不准」，不会误丢 |
| R25 | 钩子挡住了正常的提交，或拖慢 TortoiseSVN | 只有 Pre-commit 的 Local-Only 命中会返回错误，其余钩子一律立即返回 0；钩子自身出错时放行（fail-open）；钩子不读 git 仓库，只写收件箱文件、读一份预先算好的路径清单；设置里可以单独关掉拦截 |
| R26 | 钩子改了用户的 TortoiseSVN 配置，影响 jkk 之外的使用 | 用户明确开启才注册，写入前列出内容；不覆盖已有同类钩子；关闭时撤销；不用会提交进仓库的 `tsvn:*hook` 属性（§6.13） |
| R27 | Local-Only 检查按路径判断，U6 里正常提交功能 hunk 被误报，或被钩子拦下 | 改为按 hunk（§6.5 F3、§6.13.4） |
| R28 | 按 hunk 检查漏判：Local-Only hunk 附近有已提交的功能改动，上下文对不上 | F3 用零上下文 diff 做行号映射，重叠 hunk 里按 `+` 行逐条查（方向是多报）；B→L 与 B→NEW 独立 diff，内容重复时 hunk 滑动仍可能判成不重叠；钩子侧的精度待实测（§11）。钩子只是附加防线，F3 兜底 |
| R29 | merge 回 main 后，跟进、更新、交付全部因不线性被拒 | jkk 的合并默认快进 / rebase（C1a 未做，仓库还没有合并入口）；F2 的「已全部提交」捷径收掉内容已全部进 SVN 的分支（§6.5、§6.11.3） |
| R30 | 「已全部提交」捷径误判，把还有未提交内容的分支移到 NEW | 判据逐路径比较 blob 与 mode（含删除），区间里有 Local-Only 就不走捷径；旧分支位置在 reflog |
| R31 | 更新进行中触发检测与跟进，读到更新到一半的工作副本 | 工作副本被锁时跳过（§6.6、条件 2）；R1 本来就拒绝 update 中途 |
| R32 | 按最后修改 revision 判定后，基线里的标签比文件当前的节点 revision 旧 | 标签仍然是真的：c(f) ≤ R(f) 时 `f@R(f)` 的内容就是当前的 pristine（§4.1、E12）；体检按 R(f) 分组拿服务器比对，照样能证伪 |
| R33 | 局部更新把同事的改动当成「本地改动」，算到用户身上（U7） | 局部更新与提交走同一个跟进，同事的改动进基线；通知与基线 commit 说明标出来源 revision 与作者（§6.10） |
| R34 | `svn:keywords` / `svn:eol-style` 的文件只改关键字或换行时，磁盘 ≠ BASE，却被录进基线（E20） | C-α 之前就存在的盲区，C1a 不处理 |
| R35 | 离线录制时已提交的删除静默丢失：第一条 diff 就跑不成，还不知道有 D | 与改造前的离线行为相同，走 `NotChecked`，报告写 `verified: false`；联网后手动体检报 Lying。D 已知时一律 `Unconfirmed` 拒绝 |
| R36 | ④ 移其他分支在 rename 之后，index.lock 已放开，外部工具这时检出那条分支，它会在已检出状态下被移走 | 移动前重查 `worktree list`，窗口是一次 git 进程的时间；linked worktree 里 rebase 中的分支判不出 |
| R37 | 钉住的索引在 split index 下还原不了，sparse index 下 libgit2 读不了 | 默认都关闭；sparse index 时跟进报错退出，不丢东西。未实测 |

## 8. 影响面

| 层 | 模块 | 改动 |
|---|---|---|
| Rust engine | 新增 `svn_basemap.rs` | 新 trailer（含墓碑）的读写、`MAX_MAP_ENTRIES`；`svn_baseline.rs` 改 `-F` 写 message，`create_baseline_commit` / `publish_baseline` 收 `&BaseMap` |
| | 新增 `svn_nodes.rs` | 节点状态读取、`compute_map`、`dir_revs` |
| | `svn_sync.rs` | 文件级映射比较、ExternallyAdvanced、WcLocked、`head_behind_base`、`read_baseline_map`；`read_baseline_rev` 返回 floor |
| | `svn_init.rs` / `svn_preflight.rs` | `run_preflight_with`、`record_mixed`、R3c 比映射、`check_current_baseline` 读映射 |
| | `svn_health.rs`、新增 `svn_tombstone.rs` | `check_baseline_map`、`plan_diffs`、`HealthMap`（含 `Unconfirmed`、`Excluded`）；`server_head_rev`、`confirm_absent` |
| | `svn_server_set.rs` | `audit_server_set_map` 用墓碑剔除 |
| | `svn_log.rs` | 锚点补 floor |
| | `svn_sync_check.rs` / `svn_sync_in.rs` | 任何 ExternallyAdvanced 走 A16 接管；S7 混合走 `record_mixed`；手动跟进接入（`take_over`、`restore_step`、`SyncInOutcome::StageLost`） |
| | `svn_sync_out.rs` / `svn_sync_out_check.rs` | `sync_out_mixed_baseline`；Repository Root 改读映射 |
| | `svn_journal.rs` | `Flow::Follow`、`Flow::FollowManual`、`Phase::FollowPublishing`、`FollowSteps`、`stage_lost` |
| | 新增 `svn_follow.rs` | F1–F6 编排、「分支跟进」入口、崩溃恢复（`resume_follow`）、手动跟进（`manual_follow`）、`branches_behind` |
| | 新增 `svn_follow_index.rs` | 拼索引、钉住旧索引 |
| | 新增 `svn_follow_mark.rs` | 失焦快照、`my_svn_user`、wc.db 戳 |
| | 新增 `svn_replay.rs`、`svn_deliver.rs` | 对象层重放引擎（丢弃变空、按路径换 C^、共享 commit memo、`all_committed`），交付改为调用它 |
| | 新增 `svn_leak.rs` | 按 hunk 判定 Local-Only 泄漏（F3） |
| | `svn_xml.rs` | `NodeStatus`、`parse_node_status`（changelist 名、`<commit author>`、`copied`）、`parse_excluded_nodes` |
| | `svn_sync_stash.rs` | `restore_wip_keeping_index`（只有手动跟进用）；`staged_only_paths`（两者都用：暂存没保住时决定 stash 留不留，Review 3） |
| | `svn_config.rs` | `svnUser`、`set_svn_user` |
| | `graph.rs`、`refs.rs` | `parse_message`、`baseline_floor` / `baseline_newer` / `baseline_gone`；过滤 `refs/mygitclient/` |
| Rust commands | `commands/svn.rs`、`svn_sync.rs`、`lib.rs` | `svn_follow`、`svn_follow_continue`、`svn_follow_manual`、`svn_follow_mark`、`svn_wc_db_stamps`、`svn_branches_behind`、`svn_get_user`、`svn_set_user` |
| | `commands/watch.rs` | 过滤 `.git/mygitclient/` |
| 前端 | 新增 `src/engine/svnFollow.ts`、`FollowReportDialog.tsx` | 跟进结局、报告、确认框文案 |
| | `src/engine/svnSync.ts`、`svnInit.ts`、`svnJournal.ts`、`svnSyncIn.ts`、`types.ts` | 横幅模型、焦点检测纯逻辑、错误引导 |
| | `src/store/repoStore.ts`、`RepoTab.tsx`、`App.tsx`、`svnSyncFlow.ts`、`BaselineBanner.tsx`、`ConfirmDialog.tsx`、`BranchView.tsx` | 焦点 / 失焦、跟进入口、「落后基线」与「切换并手动跟进」 |
| | `src/engine/svnGraph.ts`、`CommitRow.tsx`、`styles.css` | 混合基线的接合、标注与收起（§6.9） |
| 测试 | `src-tauri/tests/svn_*.rs`、`tests/vitest/` | 见实现日志进度总账 |
| 未改（后续期） | 新增 `svn_tsvn_hook.rs`、钩子 CLI 入口、合并入口、设置页、「本地改动」按 changelist 分组 | C1h、C1b、C2 |

按 AGENTS.md 属于架构级改动：走 spec → plan，在独立 worktree 里施工，附 decisions 日志，合并前做独立 review。

## 9. 分期建议

第三轮修订：原 C1 拆成 C1a / C1b。原 C1 的范围与 Phase 2c 相当，而且还只是按钮版；先交付一个最小可用的版本，统计哪些条件经常挡住跟进，再决定 C1b 的优先级。

第四轮修订：插入 C1h（TortoiseSVN 钩子，§6.13），排在 C1b 之前。顺序为 C1a → C1h → C1b → C2 → C3。C1h 的投入不大，却同时把作者 + 快照推断换成事实、把 Local-Only 泄漏挡在提交之前，还让 C2 不必补 notify。C1a 的焦点检测与作者 + 快照规则仍要先做，它们是钩子未注册或失效时的兜底。

第五轮修订（按 §1.5 的日常工作流，§6.14）：「有自己 commit 的其他分支在对象层重放」从 C1b 挪回 C1a（U5）；C1a 新增「已全部提交」捷径与合并默认快进 / rebase（U5）、按 hunk 的 Local-Only 泄漏检查（U6）、更新进行中跳过检测（U1）。C1b 只剩 changelist 分组。另外建议 C3 排在 C2 之前：U2 / U3 / U5 / U6 里「用 jkk 提交到 SVN」都依赖 C3，而 C2 只是省掉一次点按钮。用户 2026-11-01 确认 C3 先（§10 Q8），C1a 之后的顺序为 C1h → C1b → C3 → C2；下表仍按编号列出。

第六轮修订（U7）：「按最后修改 revision 判定」放进 C1a。

第七轮修订（实现回写）：下表 C1a 一行按实际交付改写。计划内没做的：**合并默认快进 / rebase**（仓库没有合并入口）、**按目录压缩映射**。条件 11、体检弹框分组也没做。全部前端入口**没有在 `pnpm tauri dev` 真实窗口验证**，手测清单在实现日志 Task 11。

| 期 | 内容 | 对用户的价值 |
|---|---|---|
| C1a（已实现，未合并） | 混合版本基线（§6.1–§6.4，按最后修改 revision 判定前进，上限 5000 条）；跟进（§6.5，含对象层重放、「自己提交的路径取 SVN 版本」、「已全部提交」捷径、按规则拼索引含作者 + 失焦快照规则、按 hunk 的 Local-Only 泄漏检查、崩溃恢复）与「跟进」按钮；其他分支一并跟进（§6.11.3），冲突的标「落后基线」并提供「切换并手动跟进」；手动跟进；焦点 + wc.db 戳检测，`wc-locked` 时不跟进（§6.6）；§6.7、§6.9、§6.10 的横幅与报告；允许混合版本初始化（Q4）；交付遇到混合版本基线显式拒绝。**不含：** 合并默认快进、目录压缩 | TortoiseSVN 提交后切回 jkk，横幅立刻出现，点「跟进」即干净；暂存存档保住，中间版本的暂存自动让位给已提交的版本；feature 分支跟着 main 上的 bugfix 走，切回去不动磁盘；本地补丁 hunk 不会误报；每周更新照旧 |
| C1h | TortoiseSVN 钩子（§6.13）：Pre-commit / Post-commit / Post-update 收件箱事件；按事件判定暂存与 commit（§6.13.3）；提交前按 hunk 拦截 Local-Only（§6.13.4）；注册、撤销与健康检查 | 「暂存的是不是中间版本」不再靠猜，另一台机器的提交不会误丢暂存；本地补丁在 TortoiseSVN 提交框里就被拦下 |
| C1b | changelist 分组（§6.11.2） | `ignore-on-commit` 单独成组 |
| C2 | 自动跟进（§6.12）；增量录制；有钩子时由钩子事件即时触发（§6.13.6），没有钩子的环境才考虑补 notify 监听（§6.6） | 提交后切回 jkk 已经是干净的，不用点按钮 |
| C3 | 不强制更新整个项目的交付（§6.8），单独出 spec | 可以改在 jkk 里交付：只提交自己的文件，跳过 `ignore-on-commit`，不拉同事的改动 |

用户选的是自动跟进（Q1）。C2 排在 C1 之后，C1 先以按钮形式交付，是为了让跟进的结果先经人眼确认几次，再让它在后台无人值守地运行。几期之间跟进的逻辑完全相同，C2 只是加上触发、条件判断与通知。

## 10. 用户决策（2026-10-30 全部回答）

| # | 问题 | 选项 | 决定 |
|---|---|---|---|
| Q0 | 平时在 jkk 里实际用到哪些功能 | 只看 diff / 存档 / 回退；还用分支、stash、提交图；将来想在 jkk 里交付 | 看 diff、存档（暂存区与 changelist）、创建分支 → C-α（§5.3、§6.11） |
| Q1 | 外部提交后怎么跟进 | 横幅按钮 / 自动 | 自动（§6.12）。C1 先给按钮，C2 接上自动（§9） |
| Q2 | 在 jkk 里交付时是否也不更新 | 要 / 暂不要 | 要：交付不强制更新整个项目 → C3（§6.8） |
| Q3 | 平时会不会在 jkk 里做 git commit 或暂存 | 会 / 基本不 | 会（commit 与暂存都会）→ 有 commit 的分支也必须能不碰磁盘地跟进，于是改成对象层重放（§6.5 F2、W10 / W11） |
| Q4 | 初始化向导是否也允许在混合版本状态下建库 | 允许 / 维持单值要求 | 允许。初始化时没有上一份映射，floor 组里的每条 D 都要按 §6.2 向服务器确认后记成墓碑（§6.3） |
| Q5（2026-10-31） | TortoiseSVN 提交前，jkk 里暂存 / commit 的是不是最终版 | 总是最终版 / 不一定 | 不一定，两种都有。用户提出：文件的最新 revision 是自己提交的，jkk 里暂存的就是中间版本，直接丢弃（§6.11.1，加先后判断；原用 T0，第七轮改失焦快照） |
| Q6（2026-10-31） | 能否全程用 jkk，或让 TortoiseSVN 通知 jkk | 全程 jkk / 改 TortoiseSVN 源码 / 用 TortoiseSVN 自带钩子 | 用户提出两个思路。全程 jkk 不现实，只作为 C3 的远期形态；用 TortoiseSVN 自带的客户端钩子脚本，不改源码 → 新增 §6.13，分期为 C1h |
| Q7（2026-10-31） | U3 里内容已进 SVN、跟进时被丢弃的 commit，要不要挂归档 ref 留着回看 | 要 / 不要 | 不要。丢弃后只留在 reflog 里 |
| Q8（2026-11-01） | C3（jkk 里不更新整个项目的交付）是否排在 C2（自动跟进）之前 | C3 先 / C2 先 | C3 先（§9、§6.14） |
| Q9（2026-10-31） | 子目录局部更新之后，git 里要不要分清同事的改动与自己的改动 | 要 / 不要 | 要（U7）。局部更新走跟进，同事的改动进基线；判定改看最后修改 revision（§6.2） |

## 11. 进入正式 spec 前还要实测的点

第七轮：✅ 表示 C1a 已测完（编号见 §3.3），「部分」写明余下什么，其余未测的保留。

- ✅ sync_in 从混合版本基线出发，S3 之后 S4 的 W-file 修改数为 0：`sync_in_from_mixed_baseline_checks_out_zero_w_modifications`
- ✅ `svn info <url>/<path>@<rev>` 用于墓碑核对：W170000 + E200009，rc=1 不等于全部失败（E16）
- ✅ 分组体检的发法：不再分逐文件 / 整单元两种，命令形状见 §6.4（E18）。未测：真实白名单规模下的耗时
- ✅ `git commit-tree -F`：UTF-8 逐字节往返（W27）；message 始终写 UTF-8。未测：真实 GBK 工程
- ✅ jkk 发的只读 svn 命令不写 wc.db（E19）
- 未测（要真实窗口）：焦点事件在 Windows 上的实际时序（含最小化、切虚拟桌面、TortoiseSVN 弹框是否算 jkk 失焦），实现用的是 DOM `focus` / `blur`，手测清单在实现日志 Task 11
- 部分：常规情形磁盘与 BASE 一致，只改关键字或换行时有盲区（E20，R34）。未测：`svn commit` 改写关键字展开后跟进的表现
- 旧版 jkk 读不出 trailer 时，各入口（横幅、图、更新、重建、交付、体检、服务器侧核对、journal）的实际表现，确认全部 fail-closed 或结果无害
- ✅ 对象层重放：重命名、二进制、mode、非 ASCII 路径（W17）；「变空」判据（W18）；分支上有 commit 时拼索引（`tests/svn_follow.rs`）
- ✅ F2 ①：W10-R2、W14 两种形态结果树等于 NEW；同时改两路径时只留未提交的（W19）
- 部分：`svn auth` 的形状已测，要 `LC_ALL=C`（实现日志 Task 9）；暂存 mtime 判据被 W26 否决，已换失焦快照。未测：真实 TortoiseSVN 提交用的用户名与 `<commit author>` 一致；`svn+ssh` 的 realm 端口
- ✅ 发布顺序每个窗口的崩溃恢复：`resume_after_crash_between_each_step` 等，见 §6.5 与实现日志 Task 10、Review 2
- ✅ 三方合并：W22、`svn_follow_index` 的决策表用例；二进制两侧都改按冲突。未单独测：两侧换行不同（按推理整文件冲突，方向安全）
- ✅ 持 `.git/index.lock` 期间的行为：W23（`checkout -b` 等挡不住），改名时的占用：W24
- ✅ 「分支跟进」入口：判据已改用 merge-base 且要求是基线 commit（§6.5）。**结论：** 交付录的基线紧接着走这个入口时，只含更早基线的分支不移动，方向安全
- 不适用：条件 11 属 C2，C1a 没有实现
- ✅ 暂存区有新增、删除、mode 变化：W25 与 Task 8 的探针；`index.lock` 被占用：报 `svn_index_busy`
- ✅ `stash apply --index` 失败的真实形状：实现日志 Task 13 的 P1–P7
- ✅ changelist：update、合并、冲突、resolve、revert 不改成员；命令行 `svn ci` 移出成员（E21）
- ✅「切换并手动跟进」：切换失败就停，不替用户 stash；切换之后走手动跟进。真实窗口未验证
- TortoiseSVN 钩子（§6.13）：
  - Pre-commit / Check-commit / Post-commit 的 `PATH` 里，是提交框里勾选的文件，还是选择的根目录；Post-update 的 `RESULTPATH` 是否列出每个实际改动的文件
  - Check-commit 返回错误时是否保留提交框；Pre-commit 返回错误时 TortoiseSVN 怎么展示错误内容
  - 钩子设置的存储位置（注册表或配置文件）、能否由程序写入；同一路径同一类型能否挂多个钩子，与公司已有钩子（含 `tsvn:*hook` 属性）怎么共存
  - Tauri 打包出的 GUI 子系统程序作为钩子命令被调起时的行为（有无控制台闪现、退出码能否被 TortoiseSVN 拿到、与单实例逻辑的交互）
  - 钩子进程的耗时：在 Unity 大工程上提交时，钩子不能让提交明显变慢
  - 提交失败（E155011 过期、网络中断）时 Post-commit 是否仍被调用、`ERROR` 参数的形状；Pre-commit 有记录而 Post-commit 缺失时收件箱的清理
- 部分：F3 已测，相邻 / 重叠 hunk 的 12 个判定全对（W21），Review 2 补了同 hunk 三种漏报。未测：钩子侧单 hunk `git apply -R --check` 的上下文处理（W21 表明 `--unidiff-zero` 有假阳性，C1h 要换做法）、「提交后恢复」时 Pre-commit 看到的磁盘内容
- ✅「已全部提交」捷径：含 merge、删除、新增、mode、重命名（W20）；区间有 Local-Only 不走捷径
- 部分：status 不报错，只有目录带 `wc-locked`（E15）。未测：真实 TortoiseSVN 半小时更新期间的表现
- ✅ `<commit revision>` 在 `A`、`R`、copied 上的形状：E13、Review 1 第 4 条。未测：`svn update -r` 回退到 c(f) ≤ X < R(f) 按回退处理是否过严。目录压缩未实现，不适用
- U7 的真实规模：一周里每天局部更新几个模块目录后，映射的条目数与分组体检的 revision 组数；条件 11 被触发的频率
- （C2 补 notify 时）Windows notify 下 `.svn/wc.db` 的真实事件形状；TSVNCache 后台进程会不会频繁改写 wc.db；TortoiseSVN 一次提交里 wc.db 会被写几次、间隔多长，定出去抖的静默时长

## 12. 修订记录

### 2026-11-01 · 第七轮修订（实现回写）

C1a 实现完成（未合并）。把实现日志的「计划的错误」「偏离计划」、各 Task 与 Review 1 / 2 的修正回写到原章节，以代码为准。

| 严重度 | 问题 | 修订 |
|---|---|---|
| 高 | T0 判据（暂存条目 mtime）不成立，W16 的推论被 W26 推翻 | 改为失焦快照 `FollowMark`，判定路径限于 `diff-tree <base_oid> NEW`（§6.6、§6.11.1、§6.13.3、R6、R22、R23） |
| 高 | 拼索引顺序错：先写最终条目时 read-tree 整体拒绝（W25） | 先写成 NEW → read-tree → 写最终条目 → 补标志 → refresh（§6.11.1） |
| 高 | 发布顺序：改为 CAS 基线 → 拼索引进 index.lock → 核对 HEAD → CAS 分支 → 再核对 → rename → 其他分支；`checkout -b` 落在 CAS 之后时往前走 | §6.5 |
| 高 | 钉 write-tree 还原会丢标志，只钉索引 gc 后内容没了（W28） | 钉原始 index blob + `blobs/<oid>`（§6.5、§6.11.1） |
| 高 | F2 ① 不能换 Local-Only commit | 排除，判定改用快照祖先（§6.5） |
| 高 | Local-Only 判定：三方重放判不出（W21），映射位置比较有漏报 | 改写为实际算法（§6.5 F3） |
| 中 | floor 只看文件会发布假单值；映射按节点 revision 全记会膨胀 | F 含目录节点，c ≤ F 隐含 F，其余记 n（§6.1、§6.2） |
| 中 | 墓碑不能从旧映射推导；整单元 diff 看不见被排除节点；确认失败不能放行 | R3b 产生墓碑；`Excluded`、`Unconfirmed`（§6.2、§6.3） |
| 中 | `added` / `replaced` / copied 的 c(f) 不可用 | 不比 c(f)（§6.2） |
| 中 | 体检命令形状与设想不同 | `--ignore-properties`、分批、非 ASCII 回退、没有 `--targets`（§6.4） |
| 中 | 最低 git 版本 2.40 不够 | 2.43（§6.5、条件 10） |
| 中 | 手动跟进的实现形状 | `Flow::FollowManual`、前置条件、放回规则（§6.5） |
| 中 | 检测的实现 | 戳（mtime + size）、枚举缓存、`wc-locked`、失焦快照的写入条件（§6.6） |
| 低 | 目录压缩、合并默认快进没有实现 | 原文改写，不再声称已有（§6.1、§6.11.3、§9） |
| — | 其余 | §3 加 §3.3（E13–E21、W17–W28）；§7 加 R34–R37；§8 按实际文件改；§9 写准 C1a 范围；§11 标注结论；§6.7–§6.10、§6.12 同步。历史修订记录保留原文，其中的 T0 是当时的说法 |

### 2026-10-31 · 第六轮修订（补充场景 U7：子目录每天局部更新）

核实方式：开发机 SlikSvn 1.14.2，在 scratchpad 下建 `file://` 仓库与两份工作副本，复现「同事改一个文件、我只更新它所在的目录」（E12），跑完即删。

| 严重度 | 问题 | 修订 |
|---|---|---|
| 高 | 局部更新会推高整个目录下所有文件的节点 revision（E12）。按节点 revision 判定时，横幅把没变的文件也报成「前进」；映射逐行列出整个目录，很快超过上限，而超限的处理是「引导 Update 归一」，与 U7 冲突；每次局部更新还会录出一个树不变的基线 commit | 改看最后修改 revision：c(f) > R(f) 才算内容变了，否则映射保留 R(f) 不动（§6.2、§4.1、R32）；超限时先按目录压缩，删掉「引导 Update 归一」（§6.1、条件 9、R7） |
| 中 | 跟进后看不出带进来的改动来自谁 | 通知与基线 commit 说明按 revision 和作者列出（§6.10、R33） |
| 低 | U7 让条件 11 更常被触发 | 写明是已知限制，按钮仍可完成跟进（§6.12） |
| — | 场景与决策 | §1.5 加 U7，§6.14 加核对行，§10 加 Q9，§9、§11 同步 |

### 2026-10-31 · 第五轮修订（按用户的日常工作流逐项核对）

核实方式：用户补充六个日常场景（§1.5），逐项对照本文与现有代码。线性要求核对了 `svn_sync_check.rs:205`（`assert_linear`）与 `svn_deliver.rs:85`、`:99`（交付的 merge 拒绝）；Local-Only 已有实现核对了 `svn_deliver.rs` 的 D0.5；TortoiseSVN「提交后恢复」与 `ignore-on-commit` 的行为经 Context7 查官方手册 `tsvn-dug-commit.html`。

| 严重度 | 问题 | 修订 |
|---|---|---|
| 高 | U6：F3 与 Pre-commit 钩子按路径交集判断 Local-Only 泄漏，同一文件里正常提交功能 hunk 会被误报、被钩子拦下，自动跟进也会停下 | 改为按 hunk 判断（§6.5 F3、§6.13.4）；§6.11.2 不再「以 changelist 为准」，Local-Only commit 是按 hunk 的事实来源，功能 hunk 走 TortoiseSVN「提交后恢复」或 jkk 交付（R27、R28） |
| 高 | U5：merge 回 main 后再提交，跟进、「更新」、需要重放的交付都因不线性被拒，没有出口 | F2 加「已全部提交」捷径，不要求线性；SVN 模式下合并默认快进 / rebase（§6.5、§6.11.3、条件 3、R29、R30） |
| 高 | U5：其他分支的对象层重放被拆到 C1b，C1a 里每次在 main 上修完 bug，切回 feature 都要走动磁盘的手动跟进 | 挪回 C1a（§6.11.3、§9、R11） |
| 中 | U1：半小时的根目录更新期间，焦点检测可能在工作副本被锁、更新到一半时跑校验与跟进 | 工作副本被锁时跳过；有钩子时由 Start-update / Post-update 暂停与恢复（§6.6、§6.5 前置、条件 2、R31）；在 feature 分支上更新的提示（§6.7） |
| 中 | U2 / U3 / U5 / U6 里「用 jkk 提交到 SVN」第一期基本用不上 | §6.8 写明依赖 C3；建议 C3 排在 C2 之前，待用户确认（§9、§10 Q8） |
| — | U3：被丢弃的 commit 是否留归档 ref | 用户决定不留（§10 Q7） |
| — | 逐场景核对结果 | 新增 §1.5、§6.14；§8、§11 同步 |

### 2026-10-31 · 第四轮修订（用户提出 TortoiseSVN 通讯机制）

核实方式：TortoiseSVN 客户端钩子的类型、参数与「等待脚本结束时可中止操作」经 Context7 查官方手册（`tsvn-dug-settings.html`、`tsvn-dug-propertypage.html`）确认；钩子在真实 TortoiseSVN 下的行为未实测，列进 §11。

| 问题 | 修订 |
|---|---|
| 用户提出：全程用 jkk 是否更简单 | 检测与推断能去掉，但混合版本基线仍要做，而替代 TortoiseSVN 等于重写一个 SVN 客户端。只作为 C3 的远期形态（§6.13 开头、§10 Q6） |
| 用户提出：给 TortoiseSVN 加一个与外部软件通讯的机制 | 不改源码，用它自带的客户端钩子脚本；新增 §6.13：收件箱事件、按事件判定暂存与 commit、提交前拦截 Local-Only、注册与健康检查、与自动跟进的关系 |
| 作者 + T0 规则是推断，且有「另一台机器」漏洞（R22） | 有事件的路径按 §6.13.3 判定；钩子健康时，「作者是我却不在本机事件里」按「拿不准」处理，漏洞关掉 |
| Local-Only 泄漏只能事后发现（R4） | 注册钩子后在提交前拦截，F3 保留作兜底（§6.13.4） |
| C2 原本要补 notify 才能在后台发现提交 | 有钩子时由事件即时触发（§6.13.6） |
| — | §6.5 F3、§6.6、§6.11.1、§7（R4、R22，新增 R24–R26）、§8、§9（插入 C1h）、§10（Q6）、§11 同步 |

### 2026-10-31 · 第三轮修订（与用户讨论后）

核实方式：W15 在开发机 scratchpad 下用纯文件跑 `git merge-file`，W16 在 scratchpad 下的临时仓库里跑，均为 git 2.51.2.windows.1，跑完即删。文档引用的 `svn_sync.rs:109`、`svn_sync_check.rs:88`、`commands/watch.rs:117`、`svn_deliver.rs:155`、`svn_sync_stash.rs:161` 五处抽查，与当前 master（83d0e48）一致。

| 严重度 | 问题 | 修订 |
|---|---|---|
| 高 | 「暂存中间版本、继续改、TortoiseSVN 提交最终版」时，三方合并几乎必然冲突（W15），第二轮假设的「合并结果通常就等于新 HEAD」不成立；条件 12 在这种常见情形下总会命中，自动跟进基本不触发 | 按用户提出的规则改：文件最新提交的作者是我 ⇒ 暂存是中间版本，直接丢弃取新 HEAD；作者不是我才做三方合并（§6.11.1） |
| 高 | 作者规则隐含「暂存早于提交」；跟进被推迟时，提交之后才做的暂存会被误丢 | 加 T0 先后判断：暂存条目 mtime（W16）早于「最近一次确认 InSync 后的失焦时刻」才丢弃，否则保留且不自动（§6.11.1、§6.6、条件 12、R22、R23） |
| 中 | git commit 里的中间版本在重放时同样冲突（W10-R2），转手动跟进会动磁盘、触发 Unity reimport | F2 加「自己提交的路径取 SVN 版本」：按同一条作者 + T0 规则，把 commit 里这些路径换成父提交的条目再重放；按文件处理，不整个丢 commit（§6.5、条件 11、R19） |
| 中 | notify 监听 wc.db 带来三项未知数（事件形状、TSVNCache 频率、仓库外工作副本根） | 第一期改为窗口焦点 + wc.db mtime 比较；notify 推到 C2 需要时再补（§6.6、§11） |
| 中 | 原 C1 范围与 Phase 2c 相当，还只是按钮版 | 拆成 C1a（当前分支跟进的最小闭环）与 C1b（其他分支重放、changelist 分组）（§9、§6.11.2、§6.11.3） |
| 低 | floor 取最小值时，一个长期没更新的文件会拖低 floor | 超限报错列出拖低 floor 的文件（§6.1） |
| 低 | 映射的存放位置只有 trailer 一种 | 记下 git notes / 独立 ref 的备选，spec 再定（§6.1） |

### 2026-10-30（五）· 第二轮独立 review 后修订

核实方式：review 引用的代码位置逐个核对；H1、H2、L2 在临时 git 仓库里复测，结果与 review 一致（新增实测 W12–W14）。

| 严重度 | 问题 | 修订 |
|---|---|---|
| 高 | 拼索引时把「索引 ≠ 旧 ≠ 新」直接重置为新 HEAD；变化若来自局部 update 拉下的同事改动，暂存内容会丢，而且索引没有 reflog、journal 用完即删，找不回来（W13） | 改为逐路径三方合并，冲突才重置且不自动；换入前把旧索引钉在 `refs/mygitclient/follow/<opid>`（§6.11.1、§6.5 F5 ②、§6.12 条件 12） |
| 高 | 重放「干净」也可能造出与磁盘不同的内容，「本地改动」出现反向改动（W14） | §4.2 写明前提；§6.12 加条件 11；R19 |
| 中 | 「变空」写成「树与 onto 相同」，多个 commit 时判错 | 改为与当前父提交的树相同；原本就空的 commit 保留（§6.5 F2、W11） |
| 中 | 读索引在锁外，有 TOCTOU；②–④ 期间 IDE 的提交、检出能插进来 | IndexLock 从读索引一直持有到移完所有分支；锁内复核；跳过在任何 worktree 里检出的分支（§6.5、§6.11.3、R16） |
| 中 | 「InSync 但当前分支落后基线」没有入口；③ CAS 失败、放弃都没定义 | 新增「分支跟进」入口；定义 CAS 失败与放弃的处理（§6.5） |
| 中 | 其他分支冲突时，条件 7 让自动跟进永久失效；其他分支缺线性检查；共享 commit 重放出重复 OID；没跳过有 upstream / 被检出的分支 | 条件 7 只看当前分支；其他分支不线性也标「落后基线」；共享 commit 只重放一次；补跳过规则（§6.11.3、§6.12、R20） |
| 低 | 临时索引从空起步，丢 stat 信息与标志 | 改为复制真实索引、`read-tree -i -m`、`refresh_index_stat`（§6.11.1） |
| 低 | 否决 `read-tree -m` 的理由引错了实测 | 改为：不带 `-i` 被 W8 否决，带 `-i` 时最后一行仍被 W6 拒绝（W12） |
| 低 | 版本检查借用交付的事后报错 | 改为 F1 之前预检，用跟进自己的错误码（§6.5） |
| 低 | 联网体检期间一直持同步锁，jkk 里的写操作都会报忙 | 准备阶段（录制、体检、重放）移到锁外，发布阶段锁内复核（§6.5） |
| 低 | 撤销后会被立刻再次自动跟进 | §6.12 条件 13；R21 |

### 2026-10-30（四）· 用户回答 Q1–Q4

- Q1 自动跟进、Q3 会做 git commit：两者放在一起，「慢路径动磁盘、只能点按钮」等于没有自动。§6.5 改为对象层重放分支上自己的 commit（W10 / W11），所有分支的跟进都不写磁盘；冲突时才退回动磁盘的「手动跟进」。发布顺序改为基线 → 索引 → 分支
- 新增 §6.12 自动跟进：触发、前置条件、通知、开关、可选的撤销
- 不用 `git replay`（git 2.55 仍标 `EXPERIMENTAL!`），复用 `svn_deliver.rs` 已有的底层重放
- Q2 交付不强制更新整个项目 → C3 确认要做（§6.8 改写）；Q4 允许混合版本初始化（§6.3 R1）
- §6.11.1 的索引规则推广到「分支上有 commit」的形态；§6.11.3 有 commit 的分支也在对象层跟进
- §7 加 R16–R18，改 R5、R6、R8、R9、R11、R12；§8、§9、§10、§11 同步

### 2026-10-30（三）· 用户回答 Q0，选定 C-α

- 用户平时用「看 diff」「存档（暂存区、`ignore-on-commit` 等 changelist）」「创建分支」→ 选 C-α（§5.3）
- 新增 §6.11 三项要求：暂存区挺过跟进、changelist 分组与保护、分支跟进；§6.5 的 F2 / F4 相应改写（快路径不再要求暂存区为空，改用 `read-tree -m`；其他分支一并处理），原 F5 的分支提示并入 F4
- 新增实测 E11（changelist 挺过 stash / 切分支）、W5–W9，以及 `svn status --xml` 中 changelist 成员的位置。W8 否决了先写进文档的 `read-tree -m` 方案：没暂存、直接在 TortoiseSVN 提交的文件会让它以 `not uptodate` 拒绝；快路径改为按规则拼索引（W9 验证）
- §7 加 R14、R15，改 R6、R11；§8、§9、§10、§11 同步

### 2026-10-30（二）· 补充与 `.svnwip` 存档方案的对比

- 新增 §5.1（对比）、§5.2（候选 C-ε：存档模式）、§5.3（怎么选）；§5 表格加 C-ε 一行，C-α 的结论从「推荐」改为「有条件推荐」；§10 加 Q0
- §6.1 的 trailer 示例改用虚构路径，不在文档里放用户工程的真实文件名（AGENTS.md §11）

### 2026-10-30 · 独立 review 后修订

| 严重度 | 问题 | 修订 |
|---|---|---|
| 高 | 已提交删除的文件没有记账。第二次跟进时 floor 组仍报 D，被判成撒谎，跟进连续失败；初始化放开后直接失败 | 引入墓碑（§4、§6.1、§6.2、§6.4），并让服务器侧核对使用墓碑 |
| 中 | 跟进后切到旧分支，旧内容被写回磁盘 | F5 + R11（后来编号改为 F4，并在 §6.11.3 加强） |
| 中 | 快路径先移分支，窗口期是反向暂存 | 改为先换索引、后 CAS 移动分支（§6.5、R12） |
| 中 | `svn status --xml` 不给 kind，`*` 扩展名把目录算成 W-file；旧格式基线的映射键集合没有定义 | 键集合取基线树的文件列表（§6.2），顺带补上「只提交删除」的盲区（§1.4） |
| 中 | 区分不了「自己提交」与「局部 update」；缺 SVN 冲突前置检查 | 横幅改成中性措辞并标出作者（§6.10）；加冲突前置（§6.3、§6.5、R13） |
| 中 | 跟进前点「体检」仍会把已提交的文件报成撒谎 | 该状态下不给体检按钮（§6.10） |
| 中 | F3 只抓「整个变空」的 Local-Only commit | 改为按路径交集判断（§6.5） |
| 中 | 混合基线挂到 `svn:<floor>` 会顶掉原基线、算错文件变化 | 第一期不映射到 svn 节点（§6.9） |
| 中 | 影响面漏了服务器侧核对、log 补拉锚点、多处 trailer 读取、预览类型、journal、前端图与预览 | 补进 §8 |
| 低 | floor 定义为众数，正文却按最小值写；体检命令形式与现行代码不一致；`svn diff` 没有 `--targets`；旧版重建的表述错误；wc.db 监听的前提没写；实验编号与目标编号撞车 | floor 改为最小值；§6.4 命令形式与分批；§7 R2 改正；§6.6、§11 补前提；实验改编号 E / W |

review 指出 `svn_init.rs:1361` 应为 1365，核实后原引用无误（1361 行就是 `run_preflight` 调用），未改。review 补充的行号中有三处核实不符，已改正：两处单值授权注释在 `svn_health.rs:67`、`svn_pristine.rs:45`；前端类型文件是 `src/engine/types.ts`，不是 `src/types.ts`。

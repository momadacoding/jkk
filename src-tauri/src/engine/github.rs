//! 通过 `gh` CLI 读取当前仓库的 open Pull Request。
//!
//! 复用 gh 而不是自己调 GitHub REST API：token 的存取、SSO、代理、企业版
//! 主机名全由 gh 管着，我们一行凭据代码都不用写，也不用新增 HTTP 依赖。
//! 代价是没装 gh 的机器上看不到徽章——见 [`list_open_prs`] 的降级约定。

use crate::engine::proc::hidden_command;
use crate::error::{GitError, Result};
use git2::{BranchType, Oid, Repository};
use serde::{Deserialize, Serialize};
use std::process::Command;

/// 向 gh 索要的字段，和 [`PullRequest`] 一一对应。
const PR_FIELDS: &str = "number,headRefName,headRefOid,title,isDraft,state,url";

/// 每种状态各取多少条。
///
/// gh 每页 100 条，超出会自动翻页——翻页要多跑几次网络往返，而这条信息只是
/// 标签上的一个角标。超出的部分不显示徽章，这个取舍比让界面等上好几秒划算。
const PR_LIMIT: &str = "100";

/// 要查的状态，各查一次。
///
/// **不用 `--state all`。** all 会把「关闭但未合并」的 PR 也捞回来，而那些对
/// 「这条分支现在什么状态」没有信息量；更要命的是它们会占掉 100 条的额度——
/// 一个长期仓库里废弃 PR 可能比合并的还多，真正要显示的反而被挤出去。
/// 分开查，每种状态各有自己的 100 条额度。
const PR_STATES: [&str; 2] = ["open", "merged"];

/// 本地同名分支相对 PR 合并点（`headRefOid`）的位置，即 `git rev-list --left-right
/// --count <本地 tip>...<合并点>` 的两个数。
///
/// 只比 SHA 是否相等分不出方向：本地落后于合并点（别处又推了提交、本地没拉）和
/// 本地领先（合并后又提交）都是「不相等」，而前者内容全在 PR 里、后者删分支会丢东西。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct LocalRelation {
    /// 计算时本地分支的 tip。PR 列表只在打开仓库和 fetch/pull/push 后刷新，
    /// 本地分支却随时会动——前端只在它仍等于分支当前 tip 时才采信这个结果。
    pub tip: String,
    /// 本地分支有、合并点没有的提交数。
    pub ahead: usize,
    /// 合并点有、本地分支没有的提交数。
    pub behind: usize,
}

/// 一条 PR。除 `local_relation` 外，字段名对齐 `gh pr list --json` 的输出，
/// 也正好是前端要的形状。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub number: u64,
    /// PR 的来源分支名，不带 `refs/heads/` 前缀。前端拿它和本地分支精确匹配。
    pub head_ref_name: String,
    /// head 分支在**合并时**的提交 oid。
    ///
    /// 拿它和本地分支的 tip 比，判断 PR 合并之后分支有没有继续往前走——
    /// 走过的话不能画成「已全部合并」，那会诱导用户删掉还没合的提交。
    /// 方向（领先/落后/分叉）由 [`Self::local_relation`] 给出。
    ///
    /// `#[serde(default)]`：老版本 gh 可能不给这个字段，缺了就退回普通 merged，
    /// 不该让整条通道解析失败。
    #[serde(default)]
    pub head_ref_oid: String,
    pub title: String,
    pub is_draft: bool,
    /// gh 原样返回的 `OPEN` / `MERGED`（本模块不查 `CLOSED`，见 [`PR_STATES`]）。
    ///
    /// 保留成字符串而不是枚举：这一层只负责忠实搬运 gh 的输出，把未知取值映射
    /// 成枚举会在 gh 加新状态时直接解析失败，整条通道跟着哑掉。判定放前端。
    pub state: String,
    pub url: String,
    /// gh 不给这个字段，由 [`annotate_local_relations`] 在本地补算。
    ///
    /// 只对已合并、能找到同名本地分支、且合并点对象在本地的 PR 才有值；
    /// 其余一律 None，前端据此显示「无法判断」，不瞎猜方向。
    #[serde(default)]
    pub local_relation: Option<LocalRelation>,
}

/// 构造一个指向 `path` 的 gh 命令，并关掉所有交互与后台网络行为。
///
/// 三件事都是必须的：
/// - `hidden_command` —— 理由同 [`crate::engine::git_cli::git_command`]，装机版没有
///   console，否则每次刷新都会闪一个黑框。
/// - `GH_PROMPT_DISABLED=1` —— GUI 进程没有终端，gh 一旦转成交互式提问就会等 stdin，
///   子进程永久挂死并占住 Tauri 的线程池。
/// - `GH_NO_UPDATE_NOTIFIER=1` —— 否则 gh 会顺手去查一次自己的新版本，白白多一次网络往返。
fn gh_command(path: &str) -> Command {
    let mut cmd = hidden_command("gh");
    cmd.current_dir(path);
    cmd.env("GH_PROMPT_DISABLED", "1");
    cmd.env("GH_NO_UPDATE_NOTIFIER", "1");
    cmd.env("NO_COLOR", "1");
    cmd
}

/// 解析 `gh pr list --json` 的 stdout。
///
/// 单独拆出来是为了能脱离网络直接测：这一层的契约（字段名、驼峰映射、多余字段）
/// 才是真正容易错的地方。
pub fn parse_pr_list(json: &str) -> Result<Vec<PullRequest>> {
    // gh 带 --json 时总会打印 `[]`，但真拿到空 stdout 时，「没有 PR」比
    // 「解析崩了」更接近事实，也不值得为它弹一条错误
    if json.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(json)
        .map_err(|err| GitError::new("gh_json", format!("gh pr list 输出解析失败: {err}")))
}

/// 查一种状态的 PR。`None` 表示这次查不到（gh 起不来或非零退出），已记日志。
fn list_prs_in_state(path: &str, state: &str) -> Result<Option<Vec<PullRequest>>> {
    let output = match gh_command(path)
        .args(["pr", "list", "--state", state, "--limit", PR_LIMIT])
        .args(["--json", PR_FIELDS])
        .output()
    {
        Ok(output) => output,
        // 最常见的一种：这台机器上没装 gh
        Err(err) => {
            log::info!("gh 无法启动，分支上不显示 PR 徽章：{err}");
            return Ok(None);
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        log::info!(
            "gh pr list --state {state} 未成功，分支上不显示 PR 徽章：{}",
            stderr.trim()
        );
        return Ok(None);
    }

    Ok(Some(parse_pr_list(&String::from_utf8_lossy(
        &output.stdout,
    ))?))
}

/// 列出当前仓库里与分支相关的 PR：进行中的（open）和已合并的（merged）。
///
/// 合并过的也要，且和 open 一样重要：squash 合并之后原分支仍留在本地和远端，
/// 光看图分不出「这条分支还在做」还是「早就进主干、可以删了」——后者反而是
/// 更难从别处看出来的那个。
///
/// **失败一律降级成空列表，不返回 Err。** gh 没装、没登录、远端不是 GitHub、
/// 仓库根本没有远端——这些都不是「出错」，只是这台机器上没有 PR 信息可显示。
/// 让它们冒泡成 `GitError` 的话，界面上的错误横幅会被这条锦上添花的信息占住，
/// 真正的 git 错误反而看不见了。
///
/// 唯一会返回 Err 的情况是 gh 成功退出却吐出解析不了的 JSON——那说明我们对
/// 它的输出契约理解错了，属于本项目的 bug，应该报出来。
pub fn list_branch_prs(path: &str) -> Result<Vec<PullRequest>> {
    let mut all = Vec::new();
    for state in PR_STATES {
        match list_prs_in_state(path, state)? {
            Some(prs) => all.extend(prs),
            // 第一种状态就查不到，说明 gh 本身不可用（没装/没登录/不是 GitHub 仓库）。
            // 再为后面的状态各起一个注定失败的子进程没有意义，每次刷新都白付这份开销。
            None => break,
        }
    }
    // 成功也记一笔。这条通道的所有失败都是静默的（见上），没有这行日志的话，
    // 「徽章为什么没出来」在装机版上完全无从下手——分不清是没读到、还是读到了 0 条。
    log::info!("gh pr list 读到 {} 条 PR（open + merged）", all.len());
    annotate_local_relations(path, &mut all);
    Ok(all)
}

/// 给已合并的 PR 补上本地同名分支相对合并点的位置，见 [`LocalRelation`]。
///
/// 和 gh 那段一样**只降级不报错**：仓库打不开、分支不存在、合并点对象本地没有
/// （从没 fetch 过）、祖先计算失败（比如浅克隆走到了边界）都只是让这条 PR 拿不到
/// 结论，前端显示「无法判断」。为这种锦上添花的信息让 PR 通道整条失败不值得。
pub fn annotate_local_relations(path: &str, prs: &mut [PullRequest]) {
    if !prs.iter().any(|pr| pr.state == "MERGED") {
        return;
    }
    let repo = match Repository::open(path) {
        Ok(repo) => repo,
        Err(err) => {
            log::info!("打不开仓库，已合并 PR 不判断本地分支的领先落后：{err}");
            return;
        }
    };
    for pr in prs.iter_mut().filter(|pr| pr.state == "MERGED") {
        pr.local_relation = local_relation(&repo, &pr.head_ref_name, &pr.head_ref_oid);
    }
}

fn local_relation(repo: &Repository, branch: &str, head_ref_oid: &str) -> Option<LocalRelation> {
    // 空串（老版本 gh 不给 headRefOid）也在这里被挡掉：Oid::from_str 不接受空输入
    let merged_at = Oid::from_str(head_ref_oid).ok()?;
    let tip = repo
        .find_branch(branch, BranchType::Local)
        .ok()?
        .get()
        .target()?;
    // 先确认合并点对象在本地。缺对象时 graph_ahead_behind 也会报错，但显式查一次
    // 才能把「本地没有这个提交」和别的失败在语义上分开，不依赖它的错误码。
    repo.find_commit(merged_at).ok()?;
    let (ahead, behind) = repo.graph_ahead_behind(tip, merged_at).ok()?;
    Some(LocalRelation {
        tip: tip.to_string(),
        ahead,
        behind,
    })
}

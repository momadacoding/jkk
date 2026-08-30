//! 通过 `gh` CLI 读取当前仓库的 open Pull Request。
//!
//! 复用 gh 而不是自己调 GitHub REST API：token 的存取、SSO、代理、企业版
//! 主机名全由 gh 管着，我们一行凭据代码都不用写，也不用新增 HTTP 依赖。
//! 代价是没装 gh 的机器上看不到徽章——见 [`list_open_prs`] 的降级约定。

use crate::engine::proc::hidden_command;
use crate::error::{GitError, Result};
use serde::{Deserialize, Serialize};
use std::process::Command;

/// 向 gh 索要的字段，和 [`PullRequest`] 一一对应。
const PR_FIELDS: &str = "number,headRefName,title,isDraft,state,url";

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

/// 一条 PR。字段名对齐 `gh pr list --json` 的输出，也正好是前端要的形状。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub number: u64,
    /// PR 的来源分支名，不带 `refs/heads/` 前缀。前端拿它和本地分支精确匹配。
    pub head_ref_name: String,
    pub title: String,
    pub is_draft: bool,
    /// gh 原样返回的 `OPEN` / `MERGED`（本模块不查 `CLOSED`，见 [`PR_STATES`]）。
    ///
    /// 保留成字符串而不是枚举：这一层只负责忠实搬运 gh 的输出，把未知取值映射
    /// 成枚举会在 gh 加新状态时直接解析失败，整条通道跟着哑掉。判定放前端。
    pub state: String,
    pub url: String,
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
    Ok(all)
}

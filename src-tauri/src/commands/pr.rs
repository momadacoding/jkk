use crate::engine::github::{self, PullRequest};
use crate::error::Result;

/// 列出当前仓库里进行中和已合并的 PR，供侧边栏与 graph 在本地分支上打编号徽章。
///
/// 这是一条**独立于 graph 的低频通道**：它要走网络（实测一两秒），而分支列表由
/// 文件监听高频驱动，两者绑在一起会让每次文件改动都发一次网络请求。前端只在
/// 打开仓库和用户主动 fetch/pull/push 之后调用它。
///
/// gh 不可用时返回空列表而不是错误，理由见 [`github::list_branch_prs`]。
#[tauri::command(async)]
pub fn list_pull_requests(path: String) -> Result<Vec<PullRequest>> {
    github::list_branch_prs(&path)
}

use crate::error::Result;
use git2::{Repository, TreeWalkMode, TreeWalkResult};

/// 一次 tree 遍历最多返回多少个文件。
///
/// 界面把结果整棵铺出来，几万条路径足以让渲染卡死；超过就截断并置 `truncated`，
/// 让用户知道看到的不是全部，而不是对着一个假装完整的树。
const MAX_ENTRIES: usize = 20_000;

/// 单个文件最多往前端送多少字节的文本。
///
/// 压缩过的 lock 文件、生成的 bundle 动辄几十兆，整份塞进 DOM 会把界面卡死。
const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;

#[derive(serde::Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TreeListing {
    /// 相对仓库根的文件路径，不含目录本身。
    pub paths: Vec<String>,
    /// 是否因为超出上限被截断。
    pub truncated: bool,
}

/// 列出某个提交的完整文件树（只列文件，目录由前端从路径里推出来）。
pub fn list_tree(path: &str, rev: &str) -> Result<TreeListing> {
    let repo = Repository::open(path)?;
    let tree = repo.revparse_single(rev)?.peel_to_tree()?;

    let mut paths = Vec::new();
    let mut truncated = false;
    tree.walk(TreeWalkMode::PreOrder, |dir, entry| {
        if entry.kind() != Some(git2::ObjectType::Blob) {
            return TreeWalkResult::Ok;
        }
        if paths.len() >= MAX_ENTRIES {
            truncated = true;
            return TreeWalkResult::Abort;
        }
        // walk 给的 dir 已经带尾部斜杠（根目录是空串），直接拼即可
        paths.push(format!("{dir}{}", entry.name().unwrap_or_default()));
        TreeWalkResult::Ok
    })?;

    Ok(TreeListing { paths, truncated })
}

#[derive(serde::Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BlobContent {
    /// 文本内容；二进制文件为 None，此时前端不该尝试按行显示。
    pub text: Option<String>,
    pub binary: bool,
    /// blob 的完整字节数，即使内容被截断也是原始大小。
    pub size: u64,
    /// 是否因为超过 `MAX_TEXT_BYTES` 被截断。
    pub truncated: bool,
}

/// 读某个提交下某个文件的内容。
///
/// 走的是 tree 而不是工作区：看历史提交时工作区里的那份可能早就变了，甚至
/// 这个文件已经不存在。
pub fn read_blob(path: &str, rev: &str, file: &str) -> Result<BlobContent> {
    let repo = Repository::open(path)?;
    let tree = repo.revparse_single(rev)?.peel_to_tree()?;
    let blob = tree
        .get_path(std::path::Path::new(file))?
        .to_object(&repo)?
        .peel_to_blob()?;

    let size = blob.size() as u64;
    if blob.is_binary() {
        return Ok(BlobContent {
            text: None,
            binary: true,
            size,
            truncated: false,
        });
    }

    let bytes = blob.content();
    let truncated = bytes.len() > MAX_TEXT_BYTES;
    // 截断点退到最后一个换行，否则末尾会留半行，看着像文件本身就是坏的
    let cut = if truncated {
        bytes[..MAX_TEXT_BYTES]
            .iter()
            .rposition(|b| *b == b'\n')
            .map(|i| i + 1)
            .unwrap_or(MAX_TEXT_BYTES)
    } else {
        bytes.len()
    };

    Ok(BlobContent {
        text: Some(String::from_utf8_lossy(&bytes[..cut]).into_owned()),
        binary: false,
        size,
        truncated,
    })
}

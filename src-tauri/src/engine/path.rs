/// 归一化路径用于比较与索引。
///
/// git CLI 输出正斜杠，而 Tauri dialog / Windows 常量给的是反斜杠，
/// 裸字符串比较永远不相等。统一为正斜杠、去掉尾部斜杠，
/// 并在 Windows 上忽略大小写。
pub fn normalize(path: &str) -> String {
    let replaced = path.replace('\\', "/");
    let trimmed = replaced.trim_end_matches('/');
    if cfg!(windows) {
        trimmed.to_lowercase()
    } else {
        trimmed.to_string()
    }
}

//! SVN 各流程共用的小工具。原来在 `svn_sync_in`、`svn_sync_out`、`svn_follow`、`svn_lock`、
//! `svn_replay` 等模块里各抄一份（第 77 号 issue）。

/// Unix 秒。系统时钟早于 1970 时记 0：只用于记录与过期判断，不值得为此报错。
pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 报错与提示里显示的短 OID：前 10 位，不足 10 位原样返回。
///
/// 按字节截：OID 是 ASCII 十六进制。调用方传进来的也可能是别的文字（分支名之类）时
/// 不能用它——中文会截在字符中间而 panic。
pub(crate) fn short(oid: &str) -> &str {
    &oid[..oid.len().min(10)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_keeps_ten_hex_digits_and_leaves_shorter_input_alone() {
        assert_eq!(short("0123456789abcdef"), "0123456789");
        assert_eq!(short("abc"), "abc");
        assert_eq!(short(""), "");
    }
}

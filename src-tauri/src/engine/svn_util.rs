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

/// 流程里**开始** `git rebase` 时带上的配置，放在 `rebase` 子命令之前。交付侧的 `rebase --continue`
/// 也带；更新侧的 `rebase --continue` 原来就不带，`--abort` 两边都不带——统一它们会改命令行，
/// 不属第 77 号 issue 的纯重构，留着（见 `2026-11-23-svn-sync-dedup-batch2-decisions.md`）。
///
/// 用户配置不能改变 rebase 的语义（2c 独立 review C1，实测）：
/// - `rebase.autoStash=true` 会把工作区里的任何残留收起、重放、再放回，退出 0——
///   残留若是基线的字节，就悄悄撤掉了用户提交过的内容，而结局报 Done；
/// - `rebase.updateRefs=true` 会顺手移动指向被重放区间的其它分支。
///
/// 与 `--empty=drop` 同一个原则：不依赖默认值，也不依赖用户配置。
/// 原来更新、手动跟进、交付、交付的继续四处各写一遍，合成这一份（第 77 号 issue 第二批）。
pub(crate) const PINNED_REBASE_CONFIG: [&str; 4] = [
    "-c",
    "rebase.autoStash=false",
    "-c",
    "rebase.updateRefs=false",
];

/// `PINNED_REBASE_CONFIG` 后面接上 `rest`（`rest` 从 `rebase` 子命令开始，或先接别的 `-c`）。
pub(crate) fn pinned_rebase<'a>(rest: &[&'a str]) -> Vec<&'a str> {
    let mut args = PINNED_REBASE_CONFIG.to_vec();
    args.extend_from_slice(rest);
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_rebase_puts_the_config_before_the_rest() {
        assert_eq!(
            pinned_rebase(&["rebase", "--empty=drop", "abc"]),
            [
                "-c",
                "rebase.autoStash=false",
                "-c",
                "rebase.updateRefs=false",
                "rebase",
                "--empty=drop",
                "abc"
            ]
        );
    }

    #[test]
    fn short_keeps_ten_hex_digits_and_leaves_shorter_input_alone() {
        assert_eq!(short("0123456789abcdef"), "0123456789");
        assert_eq!(short("abc"), "abc");
        assert_eq!(short(""), "");
    }
}

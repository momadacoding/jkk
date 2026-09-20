//! §16 R2b 的取数入口：`svn cat -r BASE` 拿单个文件在服务器 rN 上的字节。
//!
//! 这些测试钉的是 spec §16.2 的四条硬约束，每一条都能独立把基线录坏：
//!
//! | 约束 | 违反的后果 |
//! |---|---|
//! | 只读，不碰磁盘 | 变成 `svn revert`，抹掉用户的本地修改 |
//! | 按字节收，不过 UTF-8 | GBK 源码被换成 U+FFFD，基线树被悄悄改坏且体检查不出来 |
//! | 用 `-r BASE` 不用 `-r N` | 重建从本地操作变成联网操作，离线直接不可用 |
//! | 未提交的 add 要能判别 | 分不出来就会把一个服务器上不存在的文件录进基线 |

#[path = "common/svn_fixtures.rs"]
mod svn_fixtures;

use app_lib::engine::svn_pristine::{cat_base, BaseContent};
use svn_fixtures::{svn_repo, svn_repo_dirty, svn_repo_with_gbk_source, DirtyKind};

fn bytes(c: BaseContent) -> Vec<u8> {
    match c {
        BaseContent::Bytes(b) => b,
        BaseContent::NotOnServer => panic!("期望取到服务器内容，实际报告服务器上没有"),
    }
}

#[test]
fn cat_base_returns_server_bytes_for_a_locally_modified_file() {
    let fx = svn_repo_dirty(DirtyKind::Modified);
    let disk_before = std::fs::read(fx.wc().join("Code/A.cs")).unwrap();

    let got = bytes(cat_base(&fx.wc_str(), "Code/A.cs").unwrap());

    assert_eq!(
        got, b"a\n",
        "取回的必须是服务器 r1 的内容，不是磁盘上改过的内容"
    );
    let disk_after = std::fs::read(fx.wc().join("Code/A.cs")).unwrap();
    assert_eq!(
        disk_before, disk_after,
        "cat 绝不能碰磁盘——碰了就等于 revert，会抹掉用户的本地修改"
    );
    assert_eq!(disk_after, b"changed\n", "用户的本地修改必须原样还在");
}

#[test]
fn cat_base_reaches_a_file_that_was_locally_deleted() {
    let fx = svn_repo_dirty(DirtyKind::Deleted);
    assert!(
        !fx.wc().join("Code/B.cs").exists(),
        "前提：这个文件已经被 svn delete 从磁盘上拿掉了"
    );

    let got = bytes(cat_base(&fx.wc_str(), "Code/B.cs").unwrap());

    assert_eq!(
        got, b"b\n",
        "本地删掉的文件仍然要能取回服务器版本——否则它会从基线里消失，\
         而基线的语义是「服务器上的样子」"
    );
}

#[test]
fn cat_base_reports_not_on_server_for_an_uncommitted_addition() {
    let fx = svn_repo_dirty(DirtyKind::Added);

    let got = cat_base(&fx.wc_str(), "Code/Added.cs").unwrap();

    assert_eq!(
        got,
        BaseContent::NotOnServer,
        "svn add 了没提交的文件在服务器 rN 上根本不存在，必须能和「取数失败」\
         区分开——分不开就只能二选一：要么把它录进基线（假基线），要么整个中止"
    );
}

#[test]
fn cat_base_preserves_non_utf8_bytes() {
    let fx = svn_repo_with_gbk_source();

    let got = bytes(cat_base(&fx.wc_str(), "Code/Gbk.cs").unwrap());

    let expected: Vec<u8> = vec![
        0x2F, 0x2F, 0x20, 0xD6, 0xD0, 0xCE, 0xC4, 0x0A, 0x63, 0x6C, 0x61, 0x73, 0x73, 0x20, 0x47,
        0x20, 0x7B, 0x7D, 0x0A,
    ];
    assert_eq!(
        got, expected,
        "GBK 字节必须逐字节原样返回。过一遍 String::from_utf8_lossy 就会变成 U+FFFD，\
         而那种损坏录进基线之后体检**查不出来**（体检比的是磁盘与服务器，不比基线树）"
    );
    // 直接断言「结果里没有 U+FFFD」是**错的**：这串字节本来就不是合法 UTF-8，
    // 对它做 lossy 转换必然产生替换字符，任何正确实现都过不去这条。
    // 要证的是「没走 lossy 那条路」，所以比的是**走过 lossy 之后会变成什么样**。
    let via_lossy = String::from_utf8_lossy(&got).into_owned().into_bytes();
    assert_ne!(
        got, via_lossy,
        "这个 fixture 必须是「过一遍 lossy 就会变形」的输入，否则这条测试证明不了任何事"
    );
    assert_eq!(
        got.len(),
        expected.len(),
        "长度都对不上说明中途被转换过——lossy 会把 2 个非法字节膨胀成 3 字节的 U+FFFD"
    );
}

#[test]
fn cat_base_works_without_the_server() {
    let fx = svn_repo_dirty(DirtyKind::Modified);
    // 把仓库目录移走，让 file:// URL 不可达——本地 fixture 上能做到的最接近断网的形态。
    let repo = fx.repo();
    let away = fx.path().join("repo_away");
    std::fs::rename(&repo, &away).unwrap();

    let got = cat_base(&fx.wc_str(), "Code/A.cs");

    std::fs::rename(&away, &repo).unwrap(); // 先还原，免得断言失败时 TempDir 清理出问题
    assert_eq!(
        bytes(got.unwrap()),
        b"a\n",
        "必须走 .svn/pristine，不联网。写成 `-r <数字>` 的话这里会是 E170013，\
         重建基线就从一个本地操作变成了联网操作（spec §16.2）"
    );
}

#[test]
fn cat_base_rejects_an_empty_path() {
    let fx = svn_repo();
    let err = cat_base(&fx.wc_str(), "").unwrap_err();
    assert_eq!(
        err.code, "svn_pristine_empty_path",
        "空路径必须显式拒绝，不能交给 svn 去解释成「当前目录」"
    );
}

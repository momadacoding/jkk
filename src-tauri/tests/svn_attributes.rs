//! `.git/info/attributes` 的生成与写入测试。
//!
//! **断言对象是 git 的实际裁决（`git check-attr`），不是生成出来的文本**
//! ——测文本只能证明「拼出了我以为的字符串」，证明不了 git 会不会照做，
//! 而这个模块存在的全部理由就是后者（spec §7.3.0）。同 `svn_exclude.rs` 的立场。

use app_lib::engine::svn_attributes::{
    has_eol_rule, restore_attributes, write_attributes, GENERATED_HEADER,
};
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn run_git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("git not found");
    assert!(
        out.status.success(),
        "git {args:?} 失败：{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// `git check-attr text -- <path>` 的裁决，形如 `Code/A.cs: text: unset`。
fn text_attr(dir: &Path, path: &str) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(["check-attr", "text", "--", path])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .rsplit(':')
        .next()
        .unwrap_or("")
        .trim()
        .to_string()
}

fn repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    run_git(dir.path(), &["init", "-b", "main"]);
    // 本机 core.autocrlf 可能是任何值；显式打开，测的才是「规则挡不挡得住它」
    run_git(dir.path(), &["config", "core.autocrlf", "true"]);
    fs::create_dir_all(dir.path().join("Code")).unwrap();
    fs::write(dir.path().join("Code/A.cs"), "a\n").unwrap();
    dir
}

#[test]
fn rule_turns_off_eol_conversion_for_every_path() {
    let dir = repo();
    assert_eq!(
        text_attr(dir.path(), "Code/A.cs"),
        "unspecified",
        "前提：没写规则时 text 属性未指定"
    );

    write_attributes(dir.path().to_str().unwrap()).unwrap();

    assert_eq!(
        text_attr(dir.path(), "Code/A.cs"),
        "unset",
        "写了 `* -text` 之后 git 必须把 text 判成 unset"
    );
}

#[test]
fn rule_beats_a_gitattributes_committed_into_the_working_tree() {
    // spec §7.3.0 取本方案而非 `core.autocrlf=false` 的**决定性理由**：
    // `$GIT_DIR/info/attributes` 的优先级压过工作树里的 `.gitattributes`。
    // 别人真把一个 `* text=auto` 提交进 SVN 时，config 那条路会被掀翻，这条不会。
    let dir = repo();
    write_attributes(dir.path().to_str().unwrap()).unwrap();
    fs::write(dir.path().join(".gitattributes"), "* text=auto\n").unwrap();

    assert_eq!(
        text_attr(dir.path(), "Code/A.cs"),
        "unset",
        "工作树里的 `* text=auto` 不该盖过 .git/info/attributes"
    );
}

#[test]
fn has_eol_rule_reports_whether_we_installed_it() {
    let dir = repo();
    let root = dir.path().to_str().unwrap();
    assert!(!has_eol_rule(root), "没写过就该报 false");

    write_attributes(root).unwrap();
    assert!(has_eol_rule(root), "写过就该报 true");
}

#[test]
fn a_users_own_attributes_file_is_backed_up_not_swallowed() {
    // `.git/info/attributes` 是 git 给用户自己用的地方。无条件整文件覆盖
    // 等于删掉用户手写的规则且不留痕迹。同 `write_exclude` 的立场。
    let dir = repo();
    let root = dir.path().to_str().unwrap();
    let info = dir.path().join(".git/info");
    fs::create_dir_all(&info).unwrap();
    fs::write(info.join("attributes"), "*.psd binary\n").unwrap();

    write_attributes(root).unwrap();

    assert_eq!(
        fs::read_to_string(info.join("attributes.bak")).unwrap(),
        "*.psd binary\n",
        "用户原有的规则必须备份成 attributes.bak"
    );
    assert!(fs::read_to_string(info.join("attributes"))
        .unwrap()
        .starts_with(GENERATED_HEADER));
}

#[test]
fn rewriting_our_own_file_does_not_create_a_second_backup() {
    // 备份已存在则不再覆盖——那说明我们早就接管过一次，attributes.bak 里
    // 才是用户真正的原件。第二次写不许把它糊成我们自己生成的内容。
    let dir = repo();
    let root = dir.path().to_str().unwrap();
    let info = dir.path().join(".git/info");
    fs::create_dir_all(&info).unwrap();
    fs::write(info.join("attributes"), "*.psd binary\n").unwrap();

    write_attributes(root).unwrap();
    write_attributes(root).unwrap();

    assert_eq!(
        fs::read_to_string(info.join("attributes.bak")).unwrap(),
        "*.psd binary\n",
        "第二次写把用户的原件覆盖掉了"
    );
}

#[test]
fn a_non_utf8_attributes_file_is_backed_up_byte_for_byte() {
    // 用户的 `.git/info/attributes` 完全可能不是 UTF-8（GBK 注释的中文工程常见）。
    // 判断要不要备份时过了一次 `from_utf8_lossy`，**写备份时绝不能用那个结果**
    // ——否则原件里的字节会被 U+FFFD 替换字符污染，而且无人发现。
    //
    // 同 spec §16.2 末尾那条「`svn cat` 必须按字节接收」是一类坑。
    let dir = repo();
    let root = dir.path().to_str().unwrap();
    let info = dir.path().join(".git/info");
    fs::create_dir_all(&info).unwrap();
    // GBK 的「中文」+ 一条真规则。这串字节在 UTF-8 下不合法。
    let raw: Vec<u8> = b"# \xd6\xd0\xce\xc4\n*.psd binary\n".to_vec();
    assert!(
        String::from_utf8(raw.clone()).is_err(),
        "前提不成立：这串字节本该不是合法 UTF-8"
    );
    fs::write(info.join("attributes"), &raw).unwrap();

    write_attributes(root).unwrap();

    assert_eq!(
        fs::read(info.join("attributes.bak")).unwrap(),
        raw,
        "备份必须逐字节等于原件，不许过 UTF-8 转换"
    );
}

#[test]
fn restore_puts_back_exactly_what_was_there_before() {
    // Task 4 的回滚要靠它：迁移时重建基线失败，仓库必须回到「规则没装」的
    // 原状，不能停在「规则装了但基线还是旧的」——那正是 spec §7.3.0 说的
    // 潜伏故障状态。
    let dir = repo();
    let root = dir.path().to_str().unwrap();
    let target = dir.path().join(".git/info/attributes");

    let prev = write_attributes(root).unwrap();
    assert!(prev.is_none(), "原本不存在时应返回 None");
    assert!(target.exists());

    restore_attributes(root, prev.as_deref()).unwrap();
    assert!(!target.exists(), "原本不存在，还原后就该不存在");
    assert!(!has_eol_rule(root));
}

#[test]
fn restore_brings_back_a_users_original_bytes() {
    let dir = repo();
    let root = dir.path().to_str().unwrap();
    let info = dir.path().join(".git/info");
    fs::create_dir_all(&info).unwrap();
    fs::write(info.join("attributes"), "*.psd binary\n").unwrap();

    let prev = write_attributes(root).unwrap();
    restore_attributes(root, prev.as_deref()).unwrap();

    assert_eq!(
        fs::read_to_string(info.join("attributes")).unwrap(),
        "*.psd binary\n",
        "还原必须逐字节回到用户原来的内容"
    );
}

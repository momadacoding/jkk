use crate::error::{GitError, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// 一条用户自定义命令。
///
/// 字段名全是单词，Tauri 的 snake_case → camelCase 转换在这里没有差异，
/// 但仍显式标注 `rename_all`，与 `RepoInfo` 等既有结构保持一致；
/// 将来加多词字段时，用户配置文件里的键名才不会突然改变风格。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomCommand {
    pub id: String,
    pub label: String,
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_cwd")]
    pub cwd: String,
}

fn default_cwd() -> String {
    "$WORKTREE".to_string()
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConfigFile {
    /// 目前恒为 1，只为将来迁移留位；解析时不校验其值。
    #[serde(default = "default_version")]
    version: u32,
    #[serde(default)]
    commands: Vec<CustomCommand>,
}

fn default_version() -> u32 {
    1
}

fn cmd(id: &str, label: &str, program: &str, args: &[&str]) -> CustomCommand {
    CustomCommand {
        id: id.to_string(),
        label: label.to_string(),
        program: program.to_string(),
        args: args.iter().map(|s| s.to_string()).collect(),
        cwd: default_cwd(),
    }
}

/// 当前平台的默认预设。
///
/// 这些程序在本机不一定装了——那是预期情况，点下去会报「找不到可执行文件」
/// 并指向配置文件。这里不做可用性探测。
pub fn default_commands() -> Vec<CustomCommand> {
    #[cfg(target_os = "windows")]
    {
        vec![
            cmd("vscode", "VS Code", "code", &["$WORKTREE"]),
            cmd("terminal", "Terminal", "wt", &["-d", "$WORKTREE"]),
            cmd("explorer", "Explorer", "explorer", &["$WORKTREE"]),
        ]
    }
    #[cfg(target_os = "macos")]
    {
        vec![
            cmd("vscode", "VS Code", "code", &["$WORKTREE"]),
            cmd(
                "terminal",
                "Terminal",
                "open",
                &["-a", "Terminal", "$WORKTREE"],
            ),
            cmd("finder", "Finder", "open", &["$WORKTREE"]),
        ]
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        vec![
            cmd("vscode", "VS Code", "code", &["$WORKTREE"]),
            // 大多数发行版把它指向用户选定的终端；它自己会在 cwd 里启动
            cmd("terminal", "Terminal", "x-terminal-emulator", &[]),
            cmd("files", "Files", "xdg-open", &["$WORKTREE"]),
        ]
    }
}

/// 读取配置；文件不存在时先写出默认预设再返回。
///
/// 解析失败**不写文件**：用户手写 JSON 漏个逗号是常事，
/// 拿默认预设去覆盖等于把他攒的命令全删了。
pub fn load_from(path: &Path) -> Result<Vec<CustomCommand>> {
    if !path.exists() {
        let defaults = default_commands();
        write_defaults(path, &defaults)?;
        return Ok(defaults);
    }
    let text = std::fs::read_to_string(path)?;
    let parsed: ConfigFile = serde_json::from_str(&text).map_err(|e| {
        GitError::new(
            "custom_config",
            format!("customCommands.json 解析失败：{e}"),
        )
    })?;
    Ok(parsed.commands)
}

fn write_defaults(path: &Path, commands: &[CustomCommand]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = ConfigFile {
        version: 1,
        commands: commands.to_vec(),
    };
    let text = serde_json::to_string_pretty(&file)
        .map_err(|e| GitError::new("custom_config", e.to_string()))?;
    std::fs::write(path, text)?;
    Ok(())
}

use crate::error::{GitError, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

const CODE: &str = "external_tools_config";

/// 一个外部 diff / merge 工具。
///
/// `diff_args` / `merge_args` 缺省表示这个工具不能做对应的事，
/// 菜单里就不出现它（例如 UnityYAMLMerge 只配 merge）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalTool {
    pub id: String,
    pub label: String,
    pub program: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff_args: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_args: Option<Vec<String>>,
    /// 哪些退出码算正常结束；缺省只认 0。
    ///
    /// BComp.exe 用退出码报比较结果（1 相同、13 有差异、14 有冲突……），
    /// 一律按「非 0 即故障」处理会在每次关掉 BC 时误报。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success_exit_codes: Option<Vec<i32>>,
    /// 已知故障码的说明，键是十进制退出码（JSON 的键只能是字符串）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code_messages: Option<BTreeMap<String, String>>,
}

/// 这个退出码对这个工具来说算不算正常结束。
pub fn is_success_exit(tool: &ExternalTool, code: i32) -> bool {
    match &tool.success_exit_codes {
        Some(codes) => codes.contains(&code),
        None => code == 0,
    }
}

/// 配置里给这个退出码写的说明。
pub fn exit_code_message(tool: &ExternalTool, code: i32) -> Option<String> {
    tool.exit_code_messages
        .as_ref()?
        .get(&code.to_string())
        .cloned()
}

/// 按文件名选工具的一条规则；没写的那一项回落到顶层默认值。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolRule {
    pub patterns: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalToolsConfig {
    /// 目前恒为 1，只为将来迁移留位；解析时不校验其值。
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub default_diff: Option<String>,
    #[serde(default)]
    pub default_merge: Option<String>,
    #[serde(default)]
    pub rules: Vec<ToolRule>,
    #[serde(default)]
    pub tools: Vec<ExternalTool>,
}

fn default_version() -> u32 {
    1
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// BC 不在 PATH 上，Windows 预设写本机实测的安装路径；不探测安装位置，
/// 装在别处的用户改配置文件即可（与 `custom_config.rs` 同样的取舍）。
fn bc_program() -> &'static str {
    if cfg!(target_os = "windows") {
        r"C:\Program Files\Beyond Compare 5\BComp.exe"
    } else {
        "bcomp"
    }
}

/// 默认预设：Beyond Compare 为默认，另带 VS Code。
///
/// UnityYAMLMerge 故意不放进来：它装在 Unity Hub 下按编辑器版本区分的目录里，
/// 写死的路径迟早会错。
pub fn default_config() -> ExternalToolsConfig {
    ExternalToolsConfig {
        version: 1,
        default_diff: Some("bc".to_string()),
        default_merge: Some("bc".to_string()),
        rules: Vec::new(),
        tools: vec![
            ExternalTool {
                id: "bc".to_string(),
                label: "Beyond Compare".to_string(),
                program: bc_program().to_string(),
                diff_args: Some(strings(&[
                    "$LOCAL",
                    "$REMOTE",
                    "/lefttitle=$LOCAL_TITLE",
                    "/righttitle=$REMOTE_TITLE",
                ])),
                merge_args: Some(strings(&[
                    "$LOCAL",
                    "$REMOTE",
                    "$BASE",
                    "$MERGED",
                    "/lefttitle=$LOCAL_TITLE",
                    "/righttitle=$REMOTE_TITLE",
                    "/centertitle=$BASE_TITLE",
                    "/outputtitle=$MERGED_TITLE",
                ])),
                // BC5 命令行文档：0 成功，1/2 相同，11/12/13 有差异，14 有冲突，都是正常的比较结果；
                // 100 起是故障。101（有冲突、合并结果没写出）对 merge 有意义，所以不算成功
                success_exit_codes: Some(vec![0, 1, 2, 11, 12, 13, 14]),
                exit_code_messages: Some(
                    [
                        (100, "未知错误"),
                        (101, "有冲突，合并结果没有写出"),
                        (102, "BComp.exe 无法等待 BCompare.exe 结束"),
                        (103, "BComp.exe 找不到 BCompare.exe"),
                        (104, "试用期已过"),
                        (105, "加载脚本文件出错"),
                        (106, "脚本语法错误"),
                        (107, "脚本无法加载文件或文件夹"),
                    ]
                    .into_iter()
                    .map(|(code, msg)| (code.to_string(), msg.to_string()))
                    .collect(),
                ),
            },
            ExternalTool {
                id: "vscode".to_string(),
                label: "VS Code".to_string(),
                program: "code".to_string(),
                diff_args: Some(strings(&["--wait", "--diff", "$LOCAL", "$REMOTE"])),
                // 参数顺序取自 VS Code 1.70 发行说明的合并示例
                merge_args: Some(strings(&[
                    "--wait", "--merge", "$REMOTE", "$LOCAL", "$BASE", "$MERGED",
                ])),
                success_exit_codes: None,
                exit_code_messages: None,
            },
        ],
    }
}

/// 读取配置；文件不存在时先写出默认预设再返回。
///
/// 解析或校验失败**不写文件**：拿默认预设去覆盖，等于把用户配好的工具全删了。
/// 报错一律带上路径，用户才知道去哪改。
pub fn load_from(path: &Path) -> Result<ExternalToolsConfig> {
    if !path.exists() {
        let defaults = default_config();
        write_defaults(path, &defaults)?;
        return Ok(defaults);
    }
    let text =
        std::fs::read_to_string(path).map_err(|e| config_error(path, format!("读取失败：{e}")))?;
    let cfg: ExternalToolsConfig =
        serde_json::from_str(&text).map_err(|e| config_error(path, format!("解析失败：{e}")))?;
    validate(&cfg).map_err(|msg| config_error(path, msg))?;
    Ok(cfg)
}

/// 按 id 找工具。规则和默认值在加载时已校验过，这里失败只可能是前端传错了 id。
pub fn find_tool<'a>(cfg: &'a ExternalToolsConfig, id: &str) -> Result<&'a ExternalTool> {
    cfg.tools
        .iter()
        .find(|t| t.id == id)
        .ok_or_else(|| GitError::new(CODE, format!("外部工具配置里没有 id 为「{id}」的工具")))
}

fn config_error(path: &Path, msg: String) -> GitError {
    GitError::new(CODE, format!("{}：{msg}", path.display()))
}

/// 引用了不存在的工具时直接报错，不悄悄回退到默认工具：
/// 回退会让场景文件落到 BC 的文本合并里，正是用户要避免的。
fn validate(cfg: &ExternalToolsConfig) -> std::result::Result<(), String> {
    let mut ids = HashSet::new();
    for tool in &cfg.tools {
        if !ids.insert(tool.id.as_str()) {
            return Err(format!("工具 id「{}」重复", tool.id));
        }
    }
    let check = |id: &Option<String>, place: &str| match id {
        Some(id) if !ids.contains(id.as_str()) => Err(format!("{place}引用了不存在的工具「{id}」")),
        _ => Ok(()),
    };
    check(&cfg.default_diff, "defaultDiff ")?;
    check(&cfg.default_merge, "defaultMerge ")?;
    for (i, rule) in cfg.rules.iter().enumerate() {
        check(&rule.diff, &format!("第 {} 条规则的 diff ", i + 1))?;
        check(&rule.merge, &format!("第 {} 条规则的 merge ", i + 1))?;
    }
    Ok(())
}

fn write_defaults(path: &Path, cfg: &ExternalToolsConfig) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(cfg).map_err(|e| GitError::new(CODE, e.to_string()))?;
    std::fs::write(path, text)?;
    Ok(())
}

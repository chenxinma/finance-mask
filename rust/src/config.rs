//! 配置单一来源 —— 内嵌仓库 `config/*.json` + 路径覆盖。
//! 移植自 scanner/patterns.py、scanner/column_matcher.py 的配置加载段。

use std::collections::HashMap;
use std::fmt;
use std::path::Path;

use serde::Deserialize;

use crate::models::{ColumnRule, DetectedType};

/// 内嵌默认正则规则（仓库 config/pattern_rules.json，include_str! 相对本文件）
pub const PATTERN_RULES_JSON: &str = include_str!("../../config/pattern_rules.json");
/// 内嵌默认列头规则（仓库 config/column_rules.json）
pub const COLUMN_RULES_JSON: &str = include_str!("../../config/column_rules.json");

/// 配置加载错误（错误信息携带文件路径/来源）
#[derive(Debug)]
pub enum ConfigError {
    /// 配置文件不存在（对应 Python FileNotFoundError）
    NotFound(String),
    /// 文件读取失败
    Read {
        path: String,
        source: std::io::Error,
    },
    /// JSON 解析失败或字段缺失/类型不符（对应 Python KeyError/ValueError）
    Json {
        origin: String,
        source: serde_json::Error,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::NotFound(path) => write!(f, "配置文件不存在: {path}"),
            ConfigError::Read { path, source } => write!(f, "配置文件读取失败: {path}: {source}"),
            ConfigError::Json { origin, source } => write!(f, "配置解析失败 ({origin}): {source}"),
        }
    }
}

impl std::error::Error for ConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ConfigError::Read { source, .. } => Some(source),
            ConfigError::Json { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// 未编译的规则规格（`PatternRule` 持有编译产物，编译在 patterns.rs 完成）
#[derive(Debug)]
pub struct PatternRuleSpec {
    pub name: String,
    pub detected_type: DetectedType,
    pub description: String,
    /// 优先级：0=字典（最高），1=正则（默认）
    pub priority: u8,
    /// 正则模式或字典词条（二选一）
    pub source: RuleSource,
}

/// 规则来源：正则模式 or 字典词条列表
#[derive(Debug)]
pub enum RuleSource {
    Regex(String),
    Dict(Vec<String>),
}

/// 类型映射，对应 patterns.py `_load_builtin_rules` 的 type_map；
/// 迭代顺序即规则加载顺序（amount→entity→person→account），
/// 位点扫描顺序依赖它，type_map 之外的类型整组跳过。
const PATTERN_TYPE_MAP: [(&str, DetectedType); 4] = [
    ("amount", DetectedType::Amount),
    ("entity", DetectedType::Entity),
    ("person", DetectedType::Person),
    ("account", DetectedType::Account),
];

/// pattern_rules.json 的条目 schema（name/pattern 必填，description 缺省 ""）
/// 新增 dict_path：指定 txt 字典文件路径（每行一个实体），匹配文本中包含任意一个实体即命中
#[derive(Deserialize)]
struct PatternRuleRaw {
    name: String,
    #[serde(default)]
    pattern: String,
    #[serde(default)]
    description: String,
    /// txt 字典文件路径（相对于配置文件目录），每行一个实体名称
    #[serde(default)]
    dict_path: String,
}

/// pattern_rules.json 的整体 schema（rules 缺省为空，对应 config.get("rules", {})）
#[derive(Deserialize)]
struct PatternRulesFile {
    #[serde(default)]
    rules: HashMap<String, Vec<PatternRuleRaw>>,
}

/// column_rules.json 的整体 schema（rules 缺省为空列表，对应 config.get("rules", [])）
#[derive(Deserialize)]
struct ColumnRulesFile {
    #[serde(default)]
    rules: Vec<ColumnRule>,
}

/// 内嵌默认正则规则（需要 config_dir 解析字典文件路径）
pub fn builtin_pattern_rules(config_dir: &Path) -> Result<Vec<PatternRuleSpec>, ConfigError> {
    parse_pattern_rules_with_base(
        PATTERN_RULES_JSON,
        "内嵌 config/pattern_rules.json",
        Some(config_dir),
    )
}

/// 从文件加载正则规则（路径覆盖内嵌默认）
pub fn pattern_rules_from_path(path: &Path) -> Result<Vec<PatternRuleSpec>, ConfigError> {
    let origin = path.display().to_string();
    let base_dir = path.parent().map(|p| p.to_path_buf());
    parse_pattern_rules_with_base(&read_file(path)?, &origin, base_dir.as_deref())
}

/// 内嵌默认列头规则
pub fn builtin_column_rules() -> Result<Vec<ColumnRule>, ConfigError> {
    parse_column_rules(COLUMN_RULES_JSON, "内嵌 config/column_rules.json")
}

/// 从文件加载列头规则（路径覆盖内嵌默认）
pub fn column_rules_from_path(path: &Path) -> Result<Vec<ColumnRule>, ConfigError> {
    let origin = path.display().to_string();
    parse_column_rules(&read_file(path)?, &origin)
}

fn read_file(path: &Path) -> Result<String, ConfigError> {
    if !path.exists() {
        return Err(ConfigError::NotFound(path.display().to_string()));
    }
    std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.display().to_string(),
        source,
    })
}

/// 读取文本文件（用于字典文件），origin 用于错误信息
fn read_file_text(path: &Path, origin: &str) -> Result<String, ConfigError> {
    if !path.exists() {
        return Err(ConfigError::NotFound(format!(
            "字典文件不存在: {} (配置: {origin})",
            path.display()
        )));
    }
    std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: format!("{} (配置: {origin})", path.display()),
        source,
    })
}

fn parse_pattern_rules_with_base(
    json: &str,
    origin: &str,
    base_dir: Option<&Path>,
) -> Result<Vec<PatternRuleSpec>, ConfigError> {
    let file: PatternRulesFile =
        serde_json::from_str(json).map_err(|source| ConfigError::Json {
            origin: origin.to_string(),
            source,
        })?;
    let mut specs = Vec::new();
    for (type_name, detected_type) in PATTERN_TYPE_MAP {
        // 未知类型键留在 HashMap 中不被读取，等价于 Python 的 continue
        let Some(rules) = file.rules.get(type_name) else {
            continue;
        };
        for raw in rules {
            let is_dict = !raw.dict_path.is_empty();
            let (source, description) = if !is_dict {
                (RuleSource::Regex(raw.pattern.clone()), raw.description.clone())
            } else {
                // 从 txt 字典文件加载词条（运行时读取）
                let dict_path = match base_dir {
                    Some(dir) => dir.join(&raw.dict_path),
                    None => Path::new(&raw.dict_path).to_path_buf(),
                };
                let content = read_file_text(&dict_path, &raw.dict_path)?;
                let words: Vec<String> = content
                    .lines()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect();
                if words.is_empty() {
                    return Err(ConfigError::NotFound(format!(
                        "字典文件为空: {}",
                        raw.dict_path
                    )));
                };
                let desc = if raw.description.is_empty() {
                    format!("字典实体({}词): {}", words.len(), raw.dict_path)
                } else {
                    raw.description.clone()
                };
                (RuleSource::Dict(words), desc)
            };
            specs.push(PatternRuleSpec {
                name: raw.name.clone(),
                source,
                detected_type: detected_type.clone(),
                description,
                // 字典规则优先级更高（0），正则规则默认（1）
                priority: if is_dict { 0 } else { 1 },
            });
        }
    }
    Ok(specs)
}

fn parse_column_rules(json: &str, origin: &str) -> Result<Vec<ColumnRule>, ConfigError> {
    let file: ColumnRulesFile = serde_json::from_str(json).map_err(|source| ConfigError::Json {
        origin: origin.to_string(),
        source,
    })?;
    Ok(file.rules)
}

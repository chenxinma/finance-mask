//! 全文扫描正则规则库 —— 移植自 src/finance_mask/scanner/patterns.py。
//! 用 fancy-regex（规则集含 `(?!%)` 负向前瞻，Python `re` 与其语法在此等价）。

use std::fmt;
use std::path::Path;

use fancy_regex::Regex;

use crate::config::{self, PatternRuleSpec};
use crate::models::DetectedType;

/// 单条正则规则（对应 patterns.py PatternRule dataclass）
#[derive(Debug)]
pub struct PatternRule {
    pub name: String,
    pub pattern: String,
    pub detected_type: DetectedType,
    pub description: String,
    compiled: Regex,
}

/// 正则规则库管理器（对应 patterns.py PatternRegistry）
#[derive(Debug)]
pub struct PatternRegistry {
    rules: Vec<PatternRule>,
}

/// 规则库构建错误（正则编译失败必须携带规则名）
#[derive(Debug)]
pub enum PatternError {
    /// 配置读取/解析失败
    Config(config::ConfigError),
    /// 正则编译失败
    Compile {
        rule_name: String,
        pattern: String,
        source: Box<fancy_regex::Error>,
    },
}

impl From<config::ConfigError> for PatternError {
    fn from(e: config::ConfigError) -> Self {
        PatternError::Config(e)
    }
}

impl fmt::Display for PatternError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PatternError::Config(e) => write!(f, "pattern 规则加载失败: {e}"),
            PatternError::Compile {
                rule_name,
                pattern,
                source,
            } => write!(
                f,
                "正则编译失败 (rule={rule_name}, pattern={pattern:?}): {source}"
            ),
        }
    }
}

impl std::error::Error for PatternError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            PatternError::Config(e) => Some(e),
            PatternError::Compile { source, .. } => Some(source),
        }
    }
}

impl PatternRegistry {
    /// 内嵌默认规则（include_str! 引仓库 config/pattern_rules.json）
    pub fn builtin() -> Result<Self, PatternError> {
        Self::from_specs(config::builtin_pattern_rules()?)
    }

    /// 从配置文件加载规则（路径覆盖内嵌默认）
    pub fn from_path(p: &Path) -> Result<Self, PatternError> {
        Self::from_specs(config::pattern_rules_from_path(p)?)
    }

    fn from_specs(specs: Vec<PatternRuleSpec>) -> Result<Self, PatternError> {
        let mut rules = Vec::with_capacity(specs.len());
        for spec in specs {
            let compiled = Regex::new(&spec.pattern).map_err(|source| PatternError::Compile {
                rule_name: spec.name.clone(),
                pattern: spec.pattern.clone(),
                source: Box::new(source),
            })?;
            rules.push(PatternRule {
                name: spec.name,
                pattern: spec.pattern,
                detected_type: spec.detected_type,
                description: spec.description,
                compiled,
            });
        }
        Ok(Self { rules })
    }

    pub fn rules(&self) -> &[PatternRule] {
        &self.rules
    }

    /// 对文本跑全部规则，返回命中的 (rule, matched_text)。
    /// 命中语义照抄 patterns.py `scan_text`：逐规则 find_iter（非重叠、从左到右），
    /// AMOUNT 命中若为纯数字（去掉 , . - 后全为数字）且去 , - 后长度 < 4 则跳过
    /// （可能是普通数字）。同一文本可被多条规则命中，全部保留。
    pub fn scan(&self, text: &str) -> Vec<(&PatternRule, String)> {
        let mut results = Vec::new();
        if text.is_empty() {
            return results;
        }
        for rule in &self.rules {
            for m in rule.compiled.find_iter(text) {
                let m = m.unwrap_or_else(|e| {
                    panic!(
                        "正则匹配失败 (rule={}, pattern={:?}): {e}",
                        rule.name, rule.pattern
                    )
                });
                let matched_text = m.as_str();
                if rule.detected_type == DetectedType::Amount {
                    // Python: matched.replace(",","").replace(".","").replace("-","").isdigit()
                    let stripped: String = matched_text
                        .chars()
                        .filter(|c| !matches!(c, ',' | '.' | '-'))
                        .collect();
                    let is_digits =
                        !stripped.is_empty() && stripped.chars().all(|c| c.is_ascii_digit());
                    if is_digits {
                        // Python: clean = matched.replace(",","").replace("-","")
                        let clean: String = matched_text
                            .chars()
                            .filter(|c| !matches!(c, ',' | '-'))
                            .collect();
                        if clean.chars().count() < 4 {
                            continue;
                        }
                    }
                }
                results.push((rule, matched_text.to_string()));
            }
        }
        results
    }
}

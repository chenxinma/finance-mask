//! 全文扫描规则库 —— 移植自 src/finance_mask/scanner/patterns.py。
//! 正则用 fancy-regex（规则集含 `(?!%)` 负向前瞻）；
//! 字典匹配用 aho-corasick，O(n) 多模式匹配，万级词条无性能退化。

use std::fmt;
use std::path::Path;

use aho_corasick::AhoCorasick;
use fancy_regex::Regex;

use crate::config::{self, PatternRuleSpec, RuleSource};
use crate::models::DetectedType;

/// 单条规则（对应 patterns.py PatternRule dataclass）
#[derive(Debug)]
pub struct PatternRule {
    pub name: String,
    pub detected_type: DetectedType,
    pub description: String,
    /// 优先级：0=字典（最高），1=正则（默认）
    pub priority: u8,
    compiled: CompiledRule,
}

#[derive(Debug)]
enum CompiledRule {
    #[allow(dead_code)]
    Regex { pattern: String, compiled: Regex },
    #[allow(dead_code)]
    Dict { words: Vec<String>, ac: AhoCorasick },
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
    /// 正则/字典编译失败
    Compile {
        rule_name: String,
        pattern: String,
        source: Box<dyn std::error::Error + Send + Sync>,
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
            PatternError::Compile { source, .. } => Some(source.as_ref()),
        }
    }
}

impl PatternRegistry {
    /// 内嵌默认规则（需要 config_dir 解析字典文件路径）
    pub fn builtin(config_dir: &Path) -> Result<Self, PatternError> {
        Self::from_specs(config::builtin_pattern_rules(config_dir)?)
    }

    /// 从配置文件加载规则（路径覆盖内嵌默认）
    pub fn from_path(p: &Path) -> Result<Self, PatternError> {
        Self::from_specs(config::pattern_rules_from_path(p)?)
    }

    fn from_specs(specs: Vec<PatternRuleSpec>) -> Result<Self, PatternError> {
        let mut rules = Vec::with_capacity(specs.len());
        for spec in specs {
            let compiled = match spec.source {
                RuleSource::Regex(pattern) => {
                    let re = Regex::new(&pattern).map_err(|source| PatternError::Compile {
                        rule_name: spec.name.clone(),
                        pattern: pattern.clone(),
                        source: Box::new(source),
                    })?;
                    CompiledRule::Regex {
                        pattern,
                        compiled: re,
                    }
                }
                RuleSource::Dict(words) => {
                    let ac = AhoCorasick::new(&words).map_err(|source| PatternError::Compile {
                        rule_name: spec.name.clone(),
                        pattern: format!("字典({}词)", words.len()),
                        source: Box::new(source),
                    })?;
                    CompiledRule::Dict { words, ac }
                }
            };
            rules.push(PatternRule {
                name: spec.name,
                detected_type: spec.detected_type,
                description: spec.description,
                priority: spec.priority,
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
    /// （可能是普通数字）。
    ///
    /// 优先级：字典规则(priority=0)先跑，正则规则(priority=1)后跑；
    /// 正则命中若与已命中的字典范围重叠则跳过（字典优先）。
    pub fn scan(&self, text: &str) -> Vec<(&PatternRule, String)> {
        let mut results = Vec::new();
        if text.is_empty() {
            return results;
        }
        // 按优先级排序（priority 越小越优先），保持同优先级内的原始顺序
        let mut indexed: Vec<(usize, &PatternRule)> = self.rules.iter().enumerate().collect();
        indexed.sort_by_key(|(_, r)| r.priority);

        // 记录高优先级规则已命中的字节范围
        let mut covered: Vec<(usize, usize)> = Vec::new();

        for (_, rule) in indexed {
            let matches: Vec<(usize, usize, String)> = match &rule.compiled {
                CompiledRule::Regex { compiled, .. } => {
                    compiled
                        .find_iter(text)
                        .filter_map(|m| {
                            let m = m.unwrap_or_else(|e| {
                                panic!("正则匹配失败 (rule={}): {e}", rule.name)
                            });
                            let matched = m.as_str();
                            if should_skip_amount(rule, matched) {
                                return None;
                            }
                            Some((m.start(), m.end(), matched.to_string()))
                        })
                        .collect()
                }
                CompiledRule::Dict { ac, .. } => {
                    ac.find_iter(text)
                        .filter_map(|m| {
                            let matched = &text[m.start()..m.end()];
                            if should_skip_amount(rule, matched) {
                                return None;
                            }
                            Some((m.start(), m.end(), matched.to_string()))
                        })
                        .collect()
                }
            };

            let is_high_priority = rule.priority == 0;
            for (start, end, matched) in matches {
                if !is_high_priority {
                    // 低优先级规则：检查是否与已覆盖范围重叠
                    if covered.iter().any(|&(s, e)| start < e && end > s) {
                        continue; // 重叠，跳过
                    }
                }
                covered.push((start, end));
                results.push((rule, matched));
            }
        }
        results
    }
}

/// amount 规则的短数字跳过逻辑
fn should_skip_amount(rule: &PatternRule, matched: &str) -> bool {
    if rule.detected_type != DetectedType::Amount {
        return false;
    }
    let stripped: String = matched
        .chars()
        .filter(|c| !matches!(c, ',' | '.' | '-'))
        .collect();
    let is_digits = !stripped.is_empty() && stripped.chars().all(|c| c.is_ascii_digit());
    if is_digits {
        let clean: String = matched.chars().filter(|c| !matches!(c, ',' | '-')).collect();
        if clean.chars().count() < 4 {
            return true;
        }
    }
    false
}

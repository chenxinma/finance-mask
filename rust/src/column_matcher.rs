//! 列头匹配引擎 —— 移植自 src/finance_mask/scanner/column_matcher.py（104 行）。
//!
//! 匹配行为以 Python 为准，仅两种方式：
//! - Exact：字符串相等
//! - Regex：`re.search` 语义（fancy-regex `is_match`）
//!
//! `MatchType::Position` 在 Python 侧从未实现匹配（仅加载不生效，无调用方），
//! 不参与匹配（2026-09-06 裁决）。

use crate::models::{ColumnRule, MatchType};

/// 列头匹配引擎（对应 column_matcher.py ColumnMatcher）
#[derive(Debug)]
pub struct ColumnMatcher {
    rules: Vec<ColumnRule>,
}

impl ColumnMatcher {
    /// 以给定规则构建；按 priority 升序稳定排序
    /// （数字越小优先级越高，同优先级保持传入顺序，对应 Python `sort(key=priority)`）
    pub fn new(rules: Vec<ColumnRule>) -> Self {
        let mut rules = rules;
        rules.sort_by_key(|r| r.priority);
        Self { rules }
    }

    /// 匹配列名，返回命中的 ColumnRule 或 None。
    ///
    /// 匹配顺序：精确匹配 > 正则匹配（两轮次序，priority 只在轮内生效）
    pub fn match_header(&self, header: &str) -> Option<&ColumnRule> {
        let header = header.trim();
        if header.is_empty() {
            return None;
        }

        // 第一轮：精确匹配
        for rule in &self.rules {
            if rule.match_type == MatchType::Exact && rule.pattern == header {
                return Some(rule);
            }
        }

        // 第二轮：正则匹配
        for rule in &self.rules {
            if rule.match_type == MatchType::Regex {
                let re = fancy_regex::Regex::new(&rule.pattern).unwrap_or_else(|e| {
                    panic!("列头规则正则编译失败 (pattern={:?}): {e}", rule.pattern)
                });
                match re.is_match(header) {
                    Ok(true) => return Some(rule),
                    Ok(false) => {}
                    // 对应 Python re.search 的运行时异常：崩溃而非静默漏配
                    Err(e) => panic!("列头规则正则匹配失败 (pattern={:?}): {e}", rule.pattern),
                }
            }
        }

        None
    }
}

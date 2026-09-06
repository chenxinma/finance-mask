//! 账号/合同号脱敏 + 名称脱敏实现
//!
//! 直接移植 `src/finance_mask/engine/account.py` AccountRedactor.mask_account
//! 和 `src/finance_mask/engine/name.py` NameRedactor。

use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Account redactor
// ---------------------------------------------------------------------------

/// 账号/合同号遮掩 —— 保留前 keep_prefix 位和后 keep_suffix 位，中间替换为 mask_char。
/// 原始分隔符（空格、`-`、`/`）在原始位置保留。
pub fn mask_account(value: &str, keep_prefix: usize, keep_suffix: usize, mask_char: char) -> String {
    let value = value.trim();
    if value.is_empty() {
        return String::new();
    }

    // 收集分隔符位置及干净字符
    let mut separators: Vec<(usize, char)> = Vec::new();
    let mut clean_chars: Vec<char> = Vec::new();
    for (i, ch) in value.chars().enumerate() {
        if ch == ' ' || ch == '-' || ch == '/' {
            separators.push((i, ch));
        } else {
            clean_chars.push(ch);
        }
    }

    let clean_len = clean_chars.len();

    // 长度不够遮掩 → 原样返回
    if clean_len <= keep_prefix + keep_suffix {
        return value.to_string();
    }

    // 遮掩核心部分
    let suffix_start = clean_len - keep_suffix;
    let mut masked: Vec<char> = Vec::with_capacity(clean_len);
    masked.extend_from_slice(&clean_chars[..keep_prefix]);
    for _ in 0..(clean_len - keep_prefix - keep_suffix) {
        masked.push(mask_char);
    }
    masked.extend_from_slice(&clean_chars[suffix_start..]);

    // 恢复分隔符
    for &(pos, sep) in &separators {
        if pos < masked.len() {
            masked.insert(pos, sep);
        } else {
            masked.push(sep);
        }
    }

    masked.into_iter().collect()
}

// ---------------------------------------------------------------------------
// Name redactor
// ---------------------------------------------------------------------------

/// 名称脱敏器（机构名 / 人名）
pub struct NameRedactor {
    entity_mapping: HashMap<String, String>,
    entity_counter: HashMap<String, usize>,
}

impl NameRedactor {
    pub fn new() -> Self {
        Self {
            entity_mapping: HashMap::new(),
            entity_counter: HashMap::new(),
        }
    }

    /// 机构名代号替换 —— 相同实体始终映射到同一别名。
    pub fn alias(&mut self, value: &str, prefix: &str) -> String {
        if value.is_empty() {
            return value.to_string();
        }

        if let Some(existing) = self.entity_mapping.get(value) {
            return existing.clone();
        }

        let counter = self.entity_counter.entry(prefix.to_string()).and_modify(|c| *c += 1).or_insert(1);
        let letter = number_to_letter(*counter);
        let result = format!("[{prefix}{letter}]");

        self.entity_mapping.insert(value.to_string(), result.clone());
        result
    }

    /// 姓名遮掩 —— 保留前 keep_first 个字符，其余替换为 `*`。
    pub fn mask_name(value: &str, keep_first: usize) -> String {
        let value = value.trim();
        if value.is_empty() {
            return String::new();
        }
        if value.chars().count() <= keep_first {
            return value.to_string();
        }

        let chars: Vec<char> = value.chars().collect();

        // 中文姓名检测：第一个字符在 CJK 统一汉字范围内
        if is_chinese(chars[0]) {
            let prefix: String = chars[..keep_first].iter().collect();
            let stars = "*".repeat(chars.len() - keep_first);
            return format!("{prefix}{stars}");
        }

        // 英文姓名
        let parts: Vec<&str> = value.split(' ').collect();
        if parts.len() > 1 {
            let first = parts[0];
            let first_prefix: String = first.chars().take(keep_first).collect();
            let mut result = format!("{first_prefix}*");
            for _ in &parts[1..] {
                result.push_str(" *");
            }
            return result;
        }

        // 单个单词
        let prefix: String = chars[..keep_first].iter().collect();
        let stars = "*".repeat(chars.len() - keep_first);
        format!("{prefix}{stars}")
    }

    /// 重置计数器和映射表（用于新的脱敏任务）
    pub fn reset(&mut self) {
        self.entity_mapping.clear();
        self.entity_counter.clear();
    }
}

fn is_chinese(ch: char) -> bool {
    matches!(ch, '\u{4E00}'..='\u{9FFF}')
}

/// 数字转字母编号：1→A, 2→B, …, 26→Z, 27→AA, …
fn number_to_letter(mut n: usize) -> String {
    let mut result = String::new();
    while n > 0 {
        let q = (n - 1) / 26;
        let r = (n - 1) % 26;
        result.insert(0, (b'A' + r as u8) as char);
        n = q;
    }
    result
}

// ===========================================================================
// Tests —— 移植自 tests/test_account.py + tests/test_name.py
// ===========================================================================
#[cfg(test)]
mod tests {
    use super::*;

    // ---- account tests ----

    #[test]
    fn mask_account_basic() {
        let result = mask_account("6222021234567890123", 3, 4, '*');
        assert_eq!(result, "622************0123");
    }

    #[test]
    fn mask_account_with_spaces() {
        let result = mask_account("6222 0212 3456 7890", 3, 4, '*');
        assert!(result.starts_with("622"));
        assert!(result.ends_with("7890"));
    }

    #[test]
    fn mask_account_short() {
        let result = mask_account("12345", 3, 4, '*');
        assert_eq!(result, "12345");
    }

    #[test]
    fn mask_account_contract() {
        let result = mask_account("HT-2024-001234", 3, 4, '*');
        // separators at positions 2, 7 (0-indexed in original "HT-2024-001234")
        assert!(result.starts_with("HT-"));
        assert!(result.ends_with("1234"));
    }

    #[test]
    fn mask_account_empty() {
        let result = mask_account("", 3, 4, '*');
        assert_eq!(result, "");
    }

    // ---- alias tests ----

    #[test]
    fn alias_basic() {
        let mut r = NameRedactor::new();
        assert_eq!(r.alias("阿里巴巴集团", "公司"), "[公司A]");
    }

    #[test]
    fn alias_multiple() {
        let mut r = NameRedactor::new();
        assert_eq!(r.alias("阿里巴巴集团", "公司"), "[公司A]");
        assert_eq!(r.alias("腾讯科技", "公司"), "[公司B]");
    }

    #[test]
    fn alias_same_entity_same_value() {
        let mut r = NameRedactor::new();
        assert_eq!(r.alias("阿里巴巴集团", "公司"), "[公司A]");
        assert_eq!(r.alias("腾讯公司", "公司"), "[公司B]");
        assert_eq!(r.alias("阿里巴巴集团", "公司"), "[公司A]");
    }

    #[test]
    fn alias_different_prefix() {
        let mut r = NameRedactor::new();
        assert_eq!(r.alias("中国银行", "银行"), "[银行A]");
    }

    // ---- number_to_letter ----

    #[test]
    fn number_to_letter_26_is_z() {
        assert_eq!(number_to_letter(26), "Z");
    }

    #[test]
    fn number_to_letter_27_is_aa() {
        assert_eq!(number_to_letter(27), "AA");
    }

    // ---- mask_name tests ----

    #[test]
    fn mask_name_chinese() {
        assert_eq!(NameRedactor::mask_name("张三", 1), "张*");
    }

    #[test]
    fn mask_name_long_chinese() {
        assert_eq!(NameRedactor::mask_name("欧阳修", 1), "欧**");
    }

    #[test]
    fn mask_name_english() {
        assert_eq!(NameRedactor::mask_name("John Smith", 1), "J* *");
    }

    #[test]
    fn mask_name_single_char() {
        assert_eq!(NameRedactor::mask_name("张", 1), "张");
    }

    #[test]
    fn mask_name_empty() {
        assert_eq!(NameRedactor::mask_name("", 1), "");
    }
}

//! 账号/合同号脱敏 + 名称脱敏 + 金额脱敏实现
//!
//! 直接移植 `src/finance_mask/engine/account.py` AccountRedactor.mask_account
//! `src/finance_mask/engine/name.py` NameRedactor
//! 和 `src/finance_mask/engine/amount.py` AmountRedactor。

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

// ---------------------------------------------------------------------------
// Amount redactor
// ---------------------------------------------------------------------------

/// Error type for amount redactor operations.
#[derive(Debug, Clone)]
pub struct AmountError(pub String);

impl std::fmt::Display for AmountError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "金额操作失败: {}", self.0)
    }
}

impl std::error::Error for AmountError {}

/// Deterministic PRNG (xorshift64). Used for reproducible perturbation.
struct SeededRng(u64);

impl SeededRng {
    fn new(seed: u64) -> Self {
        // Ensure non-zero state
        Self(seed.wrapping_add(1).max(1))
    }

    fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// Returns a value in [0, 1)
    fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / ((1u64 << 53) as f64)
    }
}

/// Chinese unit suffixes (longest first) with their numeric multipliers.
const CHINESE_UNITS: &[(&str, f64)] = &[
    ("\u{4e07}\u{4ebf}", 1e12),  // 万亿
    ("\u{4ebf}", 1e8),           // 亿
    ("\u{767e}\u{4e07}", 1e6),   // 百万
    ("\u{4e07}", 1e4),           // 万
    ("\u{5343}", 1e3),           // 千
];

/// Western unit conversion factors.
const UNIT_FACTORS: &[(&str, f64)] = &[
    ("thousand", 1000.0),
    ("million", 1_000_000.0),
    ("billion", 1_000_000_000.0),
];

/// Chinese labels for western units.
const UNIT_LABELS: &[(&str, &str)] = &[
    ("thousand", "\u{5343}"),      // 千
    ("million", "\u{767e}\u{4e07}"), // 百万
    ("billion", "\u{4ebf}"),       // 亿
];

/// Round `value` to `decimal_places` using round-half-up
/// (matches Python's `decimal.ROUND_HALF_UP`).
fn round_half_up(value: f64, decimal_places: usize) -> f64 {
    if !value.is_finite() {
        return value;
    }
    let factor = 10f64.powi(decimal_places as i32);
    let shifted = value * factor;
    if shifted >= 0.0 {
        (shifted + 0.5).floor() / factor
    } else {
        (shifted - 0.5).ceil() / factor
    }
}

/// Format a floating-point value with thousand separators and fixed decimal
/// places (equivalent to Python's `f"{value:,.Nf}"`).
fn format_amount(value: f64, decimal_places: usize) -> String {
    // Normalise -0.0 to 0.0 so we don't output "-0.00"
    let value = if value == 0.0 { 0.0 } else { value };

    let rounded = round_half_up(value, decimal_places);
    let formatted = format!("{:.prec$}", rounded, prec = decimal_places);

    // Split into integer and decimal parts
    let (int_part, dec_part) = match formatted.find('.') {
        Some(pos) => (&formatted[..pos], Some(&formatted[pos..])),
        None => (formatted.as_str(), None),
    };

    // Separate sign
    let (sign, digits) = if let Some(rest) = int_part.strip_prefix('-') {
        ("-", rest)
    } else {
        ("", int_part)
    };

    // Insert thousand separators (built right-to-left, then reversed)
    let formatted_int: String = {
        let mut buf = String::new();
        for (i, ch) in digits.chars().rev().enumerate() {
            if i > 0 && i % 3 == 0 {
                buf.push(',');
            }
            buf.push(ch);
        }
        buf.chars().rev().collect()
    };

    match dec_part {
        Some(dec) => format!("{}{}{}", sign, formatted_int, dec),
        None => format!("{}{}", sign, formatted_int),
    }
}

/// 金额脱敏器
pub struct AmountRedactor;

impl AmountRedactor {
    // ---------------------------------------------------------------------
    // 降低精度 (precision)
    // ---------------------------------------------------------------------

    /// 降低精度模式
    ///
    /// * If `value` carries a Chinese unit (万亿/亿/百万/万/千 + 元), the numeric
    ///   part is rounded to an integer and returned as `"{int}{unit}"`.
    /// * Otherwise the value is divided by the target unit factor, rounded to
    ///   `decimal_places`, and returned as `"{result}{label}元"`.
    pub fn precision(
        value: &str,
        unit: &str,
        decimal_places: usize,
    ) -> Result<String, AmountError> {
        let (num_part, original_unit) = Self::extract_original_unit(value);

        // Case 1: input already carries a Chinese unit → round to integer
        if !original_unit.is_empty() {
            let clean_num = num_part.replace([',', ' '], "");
            let numeric_value: f64 = clean_num
                .parse()
                .map_err(|_| AmountError(format!("无法解析为数值: {}", value)))?;
            let rounded = round_half_up(numeric_value, 0) as i64;
            return Ok(format!("{}{}", rounded, original_unit));
        }

        // Case 2: no Chinese unit → convert using target unit
        let numeric_value = Self::parse_number(value)?;

        // Zero is a special case in the Python source
        if numeric_value == 0.0 {
            let label = UNIT_LABELS
                .iter()
                .find(|(k, _)| *k == unit)
                .map(|(_, v)| *v)
                .unwrap_or("");
            return Ok(format!("0.{}{}元", "0".repeat(decimal_places), label));
        }

        let factor = UNIT_FACTORS
            .iter()
            .find(|(k, _)| *k == unit)
            .map(|(_, v)| *v)
            .ok_or_else(|| {
                AmountError(format!("不支持的单位: {}，支持的单位: thousand/million/billion", unit))
            })?;

        let converted = numeric_value / factor;
        let result = round_half_up(converted, decimal_places);
        let label = UNIT_LABELS
            .iter()
            .find(|(k, _)| *k == unit)
            .map(|(_, v)| *v)
            .unwrap_or("");

        Ok(format!("{:.prec$}{}元", result, label, prec = decimal_places))
    }

    // ---------------------------------------------------------------------
    // 随机扰动 (perturb)
    // ---------------------------------------------------------------------

    /// 随机扰动模式
    ///
    /// Parse the numeric value (may carry Chinese units), apply a deterministic
    /// random perturbation in `[-percentage%, +percentage%]`, and format the
    /// result with thousand separators and two decimal places.
    pub fn perturb(
        value: &str,
        percentage: f64,
        seed: u64,
    ) -> Result<String, AmountError> {
        let numeric_value = Self::parse_number(value)?;

        if !(0.0..=100.0).contains(&percentage) {
            return Err(AmountError(format!(
                "percentage 必须在 0-100 之间，当前值: {}",
                percentage
            )));
        }

        let mut rng = SeededRng::new(seed);
        let range_factor = percentage / 100.0;
        let random_factor = rng.next_f64() * 2.0 - 1.0; // [-1, 1)
        let perturbation = numeric_value * range_factor * random_factor;
        let result = numeric_value + perturbation;

        Ok(format_amount(result, 2))
    }

    // ---------------------------------------------------------------------
    // 遮掩 (mask)
    // ---------------------------------------------------------------------

    /// 遮掩模式
    ///
    /// Preserves the first digit of the integer part, replaces the rest with
    /// `*`, keeps thousand-separator and decimal-point positions.
    ///
    /// Example: `"12,345,678.90"` → `"1*,***,***.**"`
    pub fn mask(value: &str) -> String {
        let trimmed = value.trim();
        let is_negative = trimmed.starts_with('-');
        let stripped = if is_negative { &trimmed[1..] } else { trimmed };
        let clean = stripped.replace(',', "");

        let (int_raw, has_dec) = match clean.find('.') {
            Some(pos) => (&clean[..pos], true),
            None => (clean.as_str(), false),
        };

        // Mask integer part: keep first digit, replace rest with '*'
        let masked_int: String = if !int_raw.is_empty() {
            let mut s = String::with_capacity(int_raw.len() * 2);
            let chars: Vec<char> = int_raw.chars().collect();
            s.push(chars[0]);
            for _ in 1..chars.len() {
                s.push('*');
            }
            s
        } else {
            String::new()
        };

        // Re-insert thousand separators into masked integer
        let formatted_int: String = if masked_int.len() > 3 {
            let mut buf = String::new();
            for (i, ch) in masked_int.chars().rev().enumerate() {
                if i > 0 && i % 3 == 0 {
                    buf.push(',');
                }
                buf.push(ch);
            }
            buf.chars().rev().collect()
        } else {
            masked_int
        };

        // Assemble result
        let mut result = formatted_int;
        if has_dec {
            result.push_str(".**");
        }
        if is_negative {
            result = format!("-{}", result);
        }
        result
    }

    // ---------------------------------------------------------------------
    // 差分偏移 (differential shift)
    // ---------------------------------------------------------------------

    /// 差分偏移模式
    ///
    /// All values receive the same deterministic offset.  The offset range is
    /// chosen so that every result stays positive.
    pub fn differential_shift(values: &[f64], seed: u64) -> Vec<f64> {
        if values.is_empty() {
            return Vec::new();
        }

        let mut rng = SeededRng::new(seed);
        let min_val = values.iter().copied().fold(f64::INFINITY, f64::min);
        let avg_abs = values.iter().map(|v| v.abs()).sum::<f64>() / values.len() as f64;
        let max_shift = (0.1 * avg_abs).min(min_val * 0.9);
        let offset = max_shift * (rng.next_f64() * 2.0 - 1.0);

        values.iter().map(|v| v + offset).collect()
    }

    // ---------------------------------------------------------------------
    // 比例缩放 (proportional scale)
    // ---------------------------------------------------------------------

    /// 比例缩放模式
    ///
    /// All values receive the same deterministic scale factor in [0.9, 1.1).
    pub fn proportional_scale(values: &[f64], seed: u64) -> Vec<f64> {
        if values.is_empty() {
            return Vec::new();
        }

        let mut rng = SeededRng::new(seed);
        let scale = 0.9 + 0.2 * rng.next_f64(); // [0.9, 1.1)
        values.iter().map(|v| v * scale).collect()
    }

    // ---------------------------------------------------------------------
    // Helper: parse_number
    // ---------------------------------------------------------------------

    /// Parse a numeric string into `f64`.
    ///
    /// Supports thousand separators, negative values, Chinese unit suffixes
    /// (万亿/亿/百万/万/千), and trailing 元.
    pub fn parse_number(value: &str) -> Result<f64, AmountError> {
        if value.is_empty() {
            return Err(AmountError("空值无法解析".to_string()));
        }

        let clean = value.trim().replace([',', ' '], "");

        // Try Chinese unit suffixes (longest first)
        for &(unit_text, multiplier) in CHINESE_UNITS {
            if clean.contains(unit_text) {
                let num_part = clean.split(unit_text).next().unwrap_or("");
                let num_part = num_part.trim_end_matches('\u{5143}'); // trim 元
                let numeric: f64 = num_part
                    .parse()
                    .map_err(|_| AmountError(format!("无法解析为数值: {}", value)))?;
                return Ok(numeric * multiplier);
            }
        }

        // Strip trailing 元
        let clean = clean.trim_end_matches('\u{5143}'); // 元

        clean
            .parse::<f64>()
            .map_err(|_| AmountError(format!("无法解析为数值: {}", value)))
    }

    // ---------------------------------------------------------------------
    // Helper: extract_original_unit
    // ---------------------------------------------------------------------

    /// Split a value into its numeric part and unit.
    ///
    /// Examples:
    /// - `"22.12亿元"` → `("22.12", "亿元")`
    /// - `"100万"`   → `("100", "万")`
    /// - `"1234.56"` → `("1234.56", "")`
    pub fn extract_original_unit(value: &str) -> (String, String) {
        if value.is_empty() {
            return (value.to_string(), String::new());
        }

        let clean = value.trim().replace([',', ' '], "");

        // Check longest suffix first
        let patterns: &[&str] = &[
            "\u{4e07}\u{4ebf}",  // 万亿
            "\u{4ebf}",           // 亿
            "\u{767e}\u{4e07}",   // 百万
            "\u{4e07}",           // 万
            "\u{5343}",           // 千
        ];

        for unit_text in patterns {
            if clean.contains(unit_text) {
                let parts: Vec<&str> = clean.split(unit_text).collect();
                let num_part = parts[0];
                let remaining = if parts.len() > 1 { parts[1] } else { "" };
                let unit_part = if remaining.starts_with('\u{5143}') {
                    // 元 follows the unit
                    format!("{}\u{5143}", unit_text)
                } else {
                    unit_text.to_string()
                };
                return (num_part.to_string(), unit_part);
            }
        }

        // Check for trailing 元
        if clean.ends_with('\u{5143}') {
            let num_part = &clean[..clean.len() - '\u{5143}'.len_utf8()];
            return (num_part.to_string(), "\u{5143}".to_string());
        }

        (clean, String::new())
    }
}

// ===========================================================================
// Tests —— 移植自 tests/test_account.py + tests/test_name.py + tests/test_amount.py
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

    // ======================================================================
    // Amount redactor tests
    // ======================================================================

    // ---- parse_number ----

    #[test]
    fn amount_parse_number_with_unit() {
        let r = AmountRedactor::parse_number("1.5亿元").unwrap();
        assert!((r - 150_000_000.0).abs() < 1.0);

        let r = AmountRedactor::parse_number("100万元").unwrap();
        assert!((r - 1_000_000.0).abs() < 1.0);

        let r = AmountRedactor::parse_number("5000万元").unwrap();
        assert!((r - 50_000_000.0).abs() < 1.0);

        let r = AmountRedactor::parse_number("2.3万亿元").unwrap();
        assert!((r - 2_300_000_000_000.0).abs() < 1.0);

        let r = AmountRedactor::parse_number("500千元").unwrap();
        assert!((r - 500_000.0).abs() < 1.0);

        let r = AmountRedactor::parse_number("100百万").unwrap();
        assert!((r - 100_000_000.0).abs() < 1.0);
    }

    #[test]
    fn amount_parse_number_plain() {
        let r = AmountRedactor::parse_number("1234567.89").unwrap();
        assert!((r - 1_234_567.89).abs() < 0.01);

        let r = AmountRedactor::parse_number("1,234,567.89").unwrap();
        assert!((r - 1_234_567.89).abs() < 0.01);

        let r = AmountRedactor::parse_number("-500.00").unwrap();
        assert!((r - (-500.0)).abs() < 0.01);
    }

    #[test]
    fn amount_parse_number_empty() {
        assert!(AmountRedactor::parse_number("").is_err());
    }

    // ---- extract_original_unit ----

    #[test]
    fn amount_extract_original_unit() {
        let (n, u) = AmountRedactor::extract_original_unit("22.12亿元");
        assert_eq!(n, "22.12");
        assert_eq!(u, "亿元");

        let (n, u) = AmountRedactor::extract_original_unit("100万元");
        assert_eq!(n, "100");
        assert_eq!(u, "万元");  // 万+元, not 百万

        let (n, u) = AmountRedactor::extract_original_unit("100万");
        assert_eq!(n, "100");
        assert_eq!(u, "万");

        let (n, u) = AmountRedactor::extract_original_unit("100元");
        assert_eq!(n, "100");
        assert_eq!(u, "元");

        let (n, u) = AmountRedactor::extract_original_unit("2.3万亿元");
        assert_eq!(n, "2.3");
        assert_eq!(u, "万亿元");

        let (n, u) = AmountRedactor::extract_original_unit("1234567.89");
        assert_eq!(n, "1234567.89");
        assert!(u.is_empty());
    }

    // ---- precision ----

    #[test]
    fn amount_precision_with_unit() {
        assert_eq!(
            AmountRedactor::precision("22.12亿元", "million", 2).unwrap(),
            "22亿元"
        );
        assert_eq!(
            AmountRedactor::precision("22.55亿元", "million", 2).unwrap(),
            "23亿元"
        );
        assert_eq!(
            AmountRedactor::precision("100万元", "million", 2).unwrap(),
            "100万元"
        );
        assert_eq!(
            AmountRedactor::precision("2.3万亿元", "million", 2).unwrap(),
            "2万亿元"
        );
        assert_eq!(
            AmountRedactor::precision("-3.14亿元", "million", 2).unwrap(),
            "-3亿元"
        );
    }

    #[test]
    fn amount_precision_basic() {
        let result = AmountRedactor::precision("12345678.90", "million", 2).unwrap();
        assert!(result.contains("12.35"), "got: {}", result);
        assert!(result.contains("百万"), "got: {}", result);
    }

    #[test]
    fn amount_precision_with_commas() {
        let result = AmountRedactor::precision("1,234,567.89", "million", 2).unwrap();
        assert!(result.contains("1.23"), "got: {}", result);
    }

    #[test]
    fn amount_precision_negative() {
        let result = AmountRedactor::precision("-1234567.89", "million", 2).unwrap();
        assert!(result.contains('-'), "got: {}", result);
        assert!(result.contains("1.23"), "got: {}", result);
    }

    #[test]
    fn amount_precision_zero() {
        let result = AmountRedactor::precision("0", "million", 2).unwrap();
        assert!(result.contains("0.00"), "got: {}", result);
    }

    #[test]
    fn amount_precision_billion() {
        let result = AmountRedactor::precision("1234567890", "billion", 2).unwrap();
        assert!(result.contains("1.23"), "got: {}", result);
        assert!(result.contains('亿'), "got: {}", result);
    }

    #[test]
    fn amount_precision_invalid_unit() {
        assert!(AmountRedactor::precision("12345678.90", "trillion", 2).is_err());
    }

    // ---- perturb ----

    #[test]
    fn amount_perturb_basic() {
        let result = AmountRedactor::perturb("1000000", 5.0, 42).unwrap();
        let numeric: f64 = result.replace(',', "").parse().unwrap();
        assert!(
            numeric >= 950_000.0 && numeric <= 1_050_000.0,
            "got: {}", result
        );
    }

    #[test]
    fn amount_perturb_zero_percentage() {
        let result = AmountRedactor::perturb("1000000", 0.0, 42).unwrap();
        assert_eq!(result, "1,000,000.00");
    }

    #[test]
    fn amount_perturb_invalid_percentage() {
        assert!(AmountRedactor::perturb("1000000", 200.0, 42).is_err());
    }

    #[test]
    fn amount_perturb_with_unit() {
        let result = AmountRedactor::perturb("1.5亿元", 5.0, 42).unwrap();
        let numeric: f64 = result.replace(',', "").parse().unwrap();
        // 1.5亿 = 150,000,000 → ±5% = [142,500,000 .. 157,500,000]
        assert!(
            numeric >= 142_500_000.0 && numeric <= 157_500_000.0,
            "got: {}", result
        );
    }

    // ---- mask ----

    #[test]
    fn amount_mask_basic() {
        let result = AmountRedactor::mask("12345678.90");
        assert!(result.starts_with('1'), "got: {}", result);
        assert!(result.contains("**"), "got: {}", result);
    }

    #[test]
    fn amount_mask_negative() {
        let result = AmountRedactor::mask("-12345678.90");
        assert!(result.starts_with("-1"), "got: {}", result);
    }

    #[test]
    fn amount_mask_with_commas() {
        let result = AmountRedactor::mask("1,234,567.89");
        assert!(result.starts_with('1'), "got: {}", result);
        assert!(result.contains(','), "got: {}", result);
    }

    #[test]
    fn amount_mask_exact_format() {
        let result = AmountRedactor::mask("12,345,678.90");
        assert_eq!(result, "1*,***,***.**");
    }

    #[test]
    fn amount_mask_no_decimal() {
        let result = AmountRedactor::mask("12345678");
        assert!(result.starts_with('1'), "got: {}", result);
        assert!(!result.contains('.'), "got: {}", result);
    }

    #[test]
    fn amount_mask_small_number() {
        let result = AmountRedactor::mask("123");
        assert_eq!(result, "1**");
    }

    // ---- differential_shift ----

    #[test]
    fn amount_differential_shift_basic() {
        let values = vec![1_000_000.0, 2_000_000.0, 3_000_000.0];
        let result = AmountRedactor::differential_shift(&values, 42);
        assert_eq!(result.len(), 3);

        // All values should be positive
        for v in &result {
            assert!(*v > 0.0, "expected positive, got {}", v);
        }

        // Pairwise differences should be preserved
        let d01 = result[1] - result[0];
        let d12 = result[2] - result[1];
        let orig01 = values[1] - values[0];
        let orig12 = values[2] - values[1];
        assert!((d01 - orig01).abs() < 0.01, "d01 diff: {}", (d01 - orig01).abs());
        assert!((d12 - orig12).abs() < 0.01, "d12 diff: {}", (d12 - orig12).abs());
    }

    #[test]
    fn amount_differential_shift_deterministic() {
        let values = vec![500_000.0, 1_000_000.0];
        let r1 = AmountRedactor::differential_shift(&values, 99);
        let r2 = AmountRedactor::differential_shift(&values, 99);
        assert_eq!(r1, r2, "same seed should produce same result");
    }

    #[test]
    fn amount_differential_shift_empty() {
        let r = AmountRedactor::differential_shift(&[], 42);
        assert!(r.is_empty());
    }

    // ---- proportional_scale ----

    #[test]
    fn amount_proportional_scale_basic() {
        let values = vec![1_000_000.0, 2_000_000.0, 3_000_000.0];
        let result = AmountRedactor::proportional_scale(&values, 42);
        assert_eq!(result.len(), 3);

        // Ratios should be preserved
        let r0 = result[0] / values[0];
        let r1 = result[1] / values[1];
        let r2 = result[2] / values[2];
        assert!((r0 - r1).abs() < 1e-10, "r0 vs r1: {}", (r0 - r1).abs());
        assert!((r1 - r2).abs() < 1e-10, "r1 vs r2: {}", (r1 - r2).abs());

        // Scale should be in [0.9, 1.1]
        assert!(r0 >= 0.9 && r0 <= 1.1, "scale out of range: {}", r0);
    }

    #[test]
    fn amount_proportional_scale_deterministic() {
        let values = vec![500_000.0, 1_000_000.0];
        let r1 = AmountRedactor::proportional_scale(&values, 77);
        let r2 = AmountRedactor::proportional_scale(&values, 77);
        assert_eq!(r1, r2, "same seed should produce same result");
    }

    #[test]
    fn amount_proportional_scale_empty() {
        let r = AmountRedactor::proportional_scale(&[], 42);
        assert!(r.is_empty());
    }
}

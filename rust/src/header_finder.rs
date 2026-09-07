//! 表头行查找 —— 移植自 src/finance_mask/scanner/header_finder.py（562 行）。
//!
//! 与 Python 版的对应关系：
//! - `find_header_row`：前 20 行多维评分（唯一值率/低数字率/中文率/字符类型熵/
//!   下一行数据验证/非空数加分/稀疏扣分/纯中文短列名加分），选最佳行后向上/向下
//!   扩展形成多行表头。
//! - `get_vertical_value` / `build_header_name`：垂直合并单元格取主格值，多行表头
//!   名合并（清洗 `[\n /]+` → `_`）。
//! - pandas 的 `find_header_row_from_dataframe` 只被 Python 侧的 DataFrame 读取
//!   路径调用（`_read_data_with_multi_header`），而扫描器实际路径
//!   （`_scan_data_sheet` 等）只走 openpyxl 路径，故本模块不移植该函数。
//!
//! 数据视图：`rows` 为稠密网格（第 r 行 1-based = `rows[r-1]`，0-based 列索引），
//! 无值单元格为 `Data::Empty`（对应 openpyxl 的 None）。行数/列数必须按
//! openpyxl normal 模式的 `max_row`/`max_column` 语义构建（见 excel_scanner::SheetData）。

use std::collections::HashSet;

use calamine::Data;

/// header_finder.py `MAX_HEADER_ROWS`
pub const MAX_HEADER_ROWS: usize = 5;

/// 合并单元格区域（0-based 行/列，含端点），对应 openpyxl `CellRange`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MergeRect {
    pub start: (u32, u32),
    pub end: (u32, u32),
}

impl MergeRect {
    pub fn contains(&self, row: u32, col: u32) -> bool {
        row >= self.start.0 && row <= self.end.0 && col >= self.start.1 && col <= self.end.1
    }
}

// ---------------------------------------------------------------------------
// 单元格字符串化 —— 复刻 Python `str(cell.value)`（openpyxl 语义）
// ---------------------------------------------------------------------------

/// 单元格字符串化（Rust 原生格式）。
///
/// - 数值：整数值（无小数部分且 |v| < 1e16）输出整数串，否则 Rust 默认 f64 格式
/// - 日期：calamine 给出的 ymd_hms_milli 原样格式化（含 Excel 1900 闰年虚构日期，
///   不做 openpyxl 的修正——下游语义比对不依赖日期串逐字节对齐）
pub fn cell_to_string(cell: &Data) -> String {
    match cell {
        Data::Int(i) => i.to_string(),
        Data::Float(f) => {
            if *f == f.trunc() && f.abs() < 1e16 {
                format!("{}", *f as i64)
            } else {
                format!("{}", f)
            }
        }
        Data::String(s) => s.clone(),
        Data::Bool(b) => if *b { "True".into() } else { "False".into() },
        Data::DateTime(dt) => {
            let (y, mo, d, h, mi, s, milli) = dt.to_ymd_hms_milli();
            let micro = milli * 1000;
            if dt.as_f64() >= 0.0 && dt.as_f64() < 1.0 && milli < 1000 {
                // 纯时间值 [0,1)
                if micro == 0 { format!("{h:02}:{mi:02}:{s:02}") }
                else { format!("{h:02}:{mi:02}:{s:02}.{micro:06}") }
            } else if micro == 0 {
                format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}")
            } else {
                format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}.{micro:06}")
            }
        }
        Data::DateTimeIso(s) | Data::DurationIso(s) => s.clone(),
        Data::Error(e) => e.to_string(),
        Data::Empty => String::new(),
    }
}



// ---------------------------------------------------------------------------
// 行判定（is_annotation_row / is_data_row）
// ---------------------------------------------------------------------------

fn is_chinese_char(c: char) -> bool {
    ('\u{4e00}'..='\u{9fff}').contains(&c)
}

/// header_finder.py `is_annotation_row`。
/// `row_values` 为整行单元格字符串（None → 不含在列表中）。
pub fn is_annotation_row(row_values: &[String]) -> bool {
    let non_empty: Vec<&str> = row_values
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if non_empty.is_empty() {
        return false;
    }
    // 第一个非空值以注释关键字开头
    let first = non_empty[0];
    let annotation_prefixes = ["注释", "Notes", "Description", "注释："];
    if annotation_prefixes.iter().any(|p| first.starts_with(p)) {
        return true;
    }
    // 50%+ 非空值长度 < 5（排除含中文的值）
    let short_count = non_empty
        .iter()
        .filter(|v| !v.chars().any(is_chinese_char) && v.chars().count() < 5)
        .count();
    short_count as f64 / non_empty.len() as f64 >= 0.5
}

/// Python `re.match(r"^-?\d+(\.\d+)?$", v)`：注意 `$` 还可匹配末尾单个换行
/// （Python re 的 `$` 语义），且 `\d` 为 Unicode Nd（此处按 ASCII 数字，夹具无非 ASCII 数字）。
fn py_number_match(v: &str) -> bool {
    let t = match v.strip_suffix('\n') {
        Some(t) => t,
        None => v,
    };
    let b = t.as_bytes();
    let mut i = 0;
    if i < b.len() && b[i] == b'-' {
        i += 1;
    }
    let int_start = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    if i == int_start {
        return false; // 至少一位整数
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let frac_start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == frac_start {
            return false; // '.' 后至少一位
        }
    }
    i == b.len()
}

/// number_pattern + `len(val.replace("-","").replace(".","")) <= 15` 长度约束
fn is_number_like(v: &str) -> bool {
    py_number_match(v) && v.chars().filter(|c| *c != '-' && *c != '.').count() <= 15
}

/// header_finder.py `is_data_row`。
/// `row_values` 为整行单元格字符串（None → 不含在列表中，注意：不 strip）。
pub fn is_data_row(row_values: &[String]) -> bool {
    let non_empty = row_values;
    if non_empty.is_empty() {
        return false;
    }

    let mut chinese_count = 0usize;
    let mut total_chars = 0usize;
    for val in non_empty {
        for ch in val.chars() {
            total_chars += 1;
            if is_chinese_char(ch) {
                chinese_count += 1;
            }
        }
    }
    let chinese_ratio = if total_chars > 0 {
        chinese_count as f64 / total_chars as f64
    } else {
        0.0
    };

    // 数字比例 > 60%
    let number_count = non_empty.iter().filter(|v| is_number_like(v)).count();
    let number_ratio = number_count as f64 / non_empty.len() as f64;
    if number_ratio > 0.6 {
        return true;
    }

    // 唯一值比例 < 40%
    let unique_count = non_empty.iter().collect::<HashSet<_>>().len();
    let unique_ratio = unique_count as f64 / non_empty.len() as f64;
    if unique_ratio < 0.4 {
        if chinese_ratio > 0.5 && non_empty.len() > 10 {
            return false;
        }
        return true;
    }

    // 第一个值是数字（可能是 ID/序号）
    if non_empty.len() >= 3 && is_number_like(&non_empty[0]) {
        let first_val = &non_empty[0];
        let len = first_val.chars().count();
        if (1..=8).contains(&len) {
            return true;
        }
    }

    // 混合内容：数字 + 中文文本
    if number_count > 0 && chinese_count > 0 && non_empty.len() >= 3 {
        return true;
    }

    // 短值、高唯一性、字母数字 ID
    if non_empty.len() >= 3 && chinese_ratio > 0.0 {
        let avg_len = non_empty.iter().map(|v| v.chars().count()).sum::<usize>() as f64
            / non_empty.len() as f64;
        if avg_len < 15.0 && unique_ratio > 0.8 {
            let alpha_count = non_empty
                .iter()
                .filter(|v| {
                    // Python `^[A-Za-z0-9]+$` 的 $ 可匹配末尾单个换行
                    let t = v.strip_suffix('\n').unwrap_or(v);
                    !t.is_empty()
                        && t.chars().all(|c| c.is_ascii_alphanumeric())
                        && !is_number_like(v)
                })
                .count();
            if alpha_count > 0 {
                return true;
            }
        }
    }

    false
}

// ---------------------------------------------------------------------------
// 评分
// ---------------------------------------------------------------------------

/// header_finder.py `_score_unique_ratio`（权重 1.0）
fn score_unique_ratio(values: &[String]) -> f64 {
    let unique = values.iter().collect::<HashSet<_>>().len();
    unique as f64 / values.len() as f64
}

/// header_finder.py `_score_number_ratio`（权重 1.5）
fn score_number_ratio(values: &[String]) -> f64 {
    let number_count = values.iter().filter(|v| is_number_like(v)).count();
    (1.0 - number_count as f64 / values.len() as f64) * 1.5
}

/// header_finder.py `_score_chinese_ratio`（权重 2.0）
fn score_chinese_ratio(values: &[String]) -> f64 {
    let mut total = 0usize;
    let mut chinese = 0usize;
    for val in values {
        for ch in val.chars() {
            total += 1;
            if is_chinese_char(ch) {
                chinese += 1;
            }
        }
    }
    if total == 0 {
        return 0.0;
    }
    chinese as f64 / total as f64 * 2.0
}

/// header_finder.py `_score_content_diversity`（字符类型香农熵，权重 2.0）
fn score_content_diversity(values: &[String]) -> f64 {
    #[derive(PartialEq, Clone, Copy)]
    enum Ty {
        C,
        L,
        N,
        O,
    }
    fn char_type(c: char) -> Ty {
        if is_chinese_char(c) {
            Ty::C
        } else if c.is_ascii_alphabetic() {
            Ty::L
        } else if c.is_numeric() {
            Ty::N
        } else {
            Ty::O
        }
    }
    let mut counts = [0usize; 4];
    let mut total = 0usize;
    for val in values {
        for ch in val.chars() {
            let t = match char_type(ch) {
                Ty::C => 0,
                Ty::L => 1,
                Ty::N => 2,
                Ty::O => 3,
            };
            counts[t] += 1;
            total += 1;
        }
    }
    if total == 0 {
        return 0.0;
    }
    let mut entropy = 0.0f64;
    for c in counts {
        if c > 0 {
            let p = c as f64 / total as f64;
            entropy -= p * p.ln();
        }
    }
    // 4 种类型的最大熵是 ln(4) ≈ 1.386，归一到 [0,1] 后按权重 2.0 缩放
    entropy / (4.0f64).ln() * 2.0
}

/// header_finder.py `_score_data_validation`（下一行应多为数字，权重 1.0）。
/// `row_idx` 1-based；`rows` 同 find_header_row。
fn score_data_validation(rows: &[Vec<Data>], row_idx: usize) -> f64 {
    let max_row = rows.len();
    if row_idx >= max_row {
        return 0.0;
    }
    let next_values: Vec<String> = rows[row_idx]
        .iter()
        .filter(|c| !matches!(c, Data::Empty))
        .map(cell_to_string)
        .collect();
    if next_values.is_empty() {
        return 0.0;
    }
    let number_count = next_values.iter().filter(|v| is_number_like(v)).count();
    if number_count as f64 / next_values.len() as f64 > 0.5 {
        1.0
    } else {
        0.0
    }
}

/// header_finder.py `_calculate_row_score`。
/// `row` 为整行单元格（含 Empty），`row_idx` 1-based。
fn calculate_row_score(rows: &[Vec<Data>], row: &[Data], row_idx: usize) -> f64 {
    let non_empty_values: Vec<String> = row
        .iter()
        .filter(|c| !matches!(c, Data::Empty))
        .map(cell_to_string)
        .collect();
    if non_empty_values.is_empty() {
        return 0.0;
    }
    let non_empty_count = non_empty_values.len();

    let mut score = 0.0;
    score += score_unique_ratio(&non_empty_values);
    score += score_number_ratio(&non_empty_values);
    score += score_chinese_ratio(&non_empty_values);
    score += score_content_diversity(&non_empty_values);
    score += score_data_validation(rows, row_idx);

    if non_empty_count >= 5 {
        score += 0.5;
    }
    if non_empty_count >= 10 {
        score += 0.5;
    }
    if non_empty_count <= 2 {
        score *= 0.5;
    }

    // 典型表头行：全中文（Python 正则 `^[\u4e00-\u9fff\s/%()（）]+$`，
    // \s 按 Unicode 空白处理）且短
    let all_chinese = non_empty_values.iter().all(|v| {
        !v.is_empty()
            && v.chars().all(|c| {
                is_chinese_char(c)
                    || c.is_whitespace()
                    || matches!(c, '/' | '%' | '(' | ')' | '（' | '）')
            })
    });
    let avg_len = non_empty_values.iter().map(|v| v.chars().count()).sum::<usize>() as f64
        / non_empty_count as f64;
    if all_chinese && avg_len <= 10.0 {
        score += 1.0;
    }

    score
}

/// 取第 `row_1based` 行的非空字符串（`cell is not None`，不 strip）——
/// 对应 Python `iter_rows(..., values_only=True)` 后的 `[str(v) for v in row if v is not None]`
fn row_strings(rows: &[Vec<Data>], row_1based: usize) -> Vec<String> {
    rows.get(row_1based - 1)
        .map(|row| {
            row.iter()
                .filter(|c| !matches!(c, Data::Empty))
                .map(cell_to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// 行内非空（`v is not None and str(v).strip()`）数量
fn row_non_empty_count(rows: &[Vec<Data>], row_1based: usize) -> usize {
    rows.get(row_1based - 1)
        .map(|row| {
            row.iter()
                .filter(|c| {
                    !matches!(c, Data::Empty) && !cell_to_string(c).trim().is_empty()
                })
                .count()
        })
        .unwrap_or(0)
}

/// header_finder.py `find_header_row`。
///
/// `rows`：稠密网格（1-based 第 r 行 = `rows[r-1]`，缺失单元格 = `Data::Empty`），
/// 行数/列数须为 openpyxl normal 模式 max_row/max_column 语义。
///
/// 返回 `(start_row, end_row)`（0-based 含端点）；未找到返回 `None`（Python 的 (-1,-1)）。
pub fn find_header_row(rows: &[Vec<Data>]) -> Option<(u32, u32)> {
    let max_scan_rows = 20usize;
    let max_row = rows.len();

    struct RowInfo {
        row_0based: u32,
        score: f64,
        non_empty_count: usize,
        is_annotation: bool,
    }

    let mut row_info: Vec<RowInfo> = Vec::new();
    for row_idx in 1..=max_row.min(max_scan_rows) {
        let row = &rows[row_idx - 1];
        let values = row_strings(rows, row_idx);
        let score = calculate_row_score(rows, row, row_idx);
        if score > 0.0 {
            let non_empty_count = row
                .iter()
                .filter(|c| !matches!(c, Data::Empty) && !cell_to_string(c).trim().is_empty())
                .count();
            row_info.push(RowInfo {
                row_0based: (row_idx - 1) as u32,
                score,
                non_empty_count,
                is_annotation: is_annotation_row(&values),

            });
        }
    }

    if row_info.is_empty() {
        return None;
    }

    // 按分数降序、非空数降序（稳定排序保持原行序）
    row_info.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.non_empty_count.cmp(&a.non_empty_count))
    });

    // 全是注释行 → 未找到（Python 的 best_row_info is None → (-1,-1)）
    let best = row_info.iter().find(|info| !info.is_annotation)?;
    let best_row = best.row_0based;
    let best_non_empty_count = best.non_empty_count;

    // 向上扩展：捕获主表头上方的标题/元数据行
    let mut header_start = best_row;
    for offset in 1..MAX_HEADER_ROWS as u32 {
        let prev_row_idx = best_row as i64 - offset as i64;
        if prev_row_idx < 0 {
            break;
        }
        let prev_1based = (prev_row_idx + 1) as usize;
        let non_empty = row_non_empty_count(rows, prev_1based);
        if non_empty == 0 {
            break; // 空行停止
        }
        let prev_values = row_strings(rows, prev_1based);
        if is_data_row(&prev_values) {
            break;
        }
        // 非空数显著少于最佳行 → 停止（Python max(3, count*0.3)）
        let threshold = 3.0_f64.max(best_non_empty_count as f64 * 0.3);
        if (non_empty as f64) < threshold {
            break;
        }
        header_start = prev_row_idx as u32;
    }

    // 向下扩展：多行表头
    let mut header_rows = vec![header_start];
    let down_limit = (header_start as usize + MAX_HEADER_ROWS).min(max_row);
    let mut offset = header_start as usize + 1;
    while offset < down_limit {
        let row_1based = offset + 1;
        let values = row_strings(rows, row_1based);
        let total_cols = rows[offset].len();
        let non_empty_count = values.iter().filter(|v| !v.trim().is_empty()).count();

        if total_cols > 0 {
            let blanks_ratio = (total_cols - non_empty_count) as f64 / total_cols as f64;
            if blanks_ratio > 0.8 && non_empty_count < 5 {
                break;
            }
        }
        if is_data_row(&values) {
            break;
        }
        header_rows.push(offset as u32);
        offset += 1;
    }

    Some((*header_rows.first()?, *header_rows.last()?))
}

// ---------------------------------------------------------------------------
// 合并单元格取值 / 多行表头名
// ---------------------------------------------------------------------------

/// header_finder.py `get_vertical_value`：取单元格字符串，垂直合并取主格（左上）值。
/// `row`/`col` 0-based；无值返回 ""。
pub fn get_vertical_value(
    rows: &[Vec<Data>],
    merged: &[MergeRect],
    row: u32,
    col: u32,
) -> String {
    if let Some(cell) = rows.get(row as usize).and_then(|r| r.get(col as usize)) {
        if !matches!(cell, Data::Empty) {
            return cell_to_string(cell);
        }
    }
    // 合并区域（按文档顺序，第一个包含该坐标的区域胜出——与 openpyxl 迭代一致）
    for rect in merged {
        if rect.contains(row, col) {
            if let Some(master) = rows
                .get(rect.start.0 as usize)
                .and_then(|r| r.get(rect.start.1 as usize))
            {
                if !matches!(master, Data::Empty) {
                    return cell_to_string(master);
                }
            }
            return String::new();
        }
    }
    String::new()
}

/// header_finder.py `build_header_name`：多行表头名合并。
/// `parts` 为该列在表头各行的值（`get_vertical_value` 结果；`None` = 无值），
/// 非 `None` 且 `strip()` 后非空的值清洗 `[\n /]+` → `_` 后以 `_` 连接。
pub fn build_header_name(parts: &[Option<String>]) -> String {
    let mut cleaned_parts = Vec::new();
    for part in parts.iter().flatten() {
        let v = part.trim();
        if v.is_empty() {
            continue;
        }
        // re.sub(r"[\n /]+", "_", v)：[\n /] 字符的连续段替换为单个 _
        let mut out = String::with_capacity(v.len());
        let mut in_run = false;
        for ch in v.chars() {
            if ch == '\n' || ch == ' ' || ch == '/' {
                if !in_run {
                    out.push('_');
                    in_run = true;
                }
            } else {
                out.push(ch);
                in_run = false;
            }
        }
        cleaned_parts.push(out);
    }
    cleaned_parts.join("_")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(rows: Vec<Vec<&str>>) -> Vec<Vec<Data>> {
        rows.into_iter()
            .map(|r| {
                r.into_iter()
                    .map(|s| {
                        if s.is_empty() {
                            Data::Empty
                        } else {
                            Data::String(s.to_string())
                        }
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn rust_native_float_formatting() {
        // Rust 默认 f64 格式（不需要 Python str(float) 逐字节对齐）
        assert_eq!(cell_to_string(&Data::Float(120.0)), "120");
        assert_eq!(cell_to_string(&Data::Float(0.0)), "0");
        assert_eq!(cell_to_string(&Data::Float(12345678.9)), "12345678.9");
        assert_eq!(cell_to_string(&Data::Float(0.03)), "0.03");
        assert_eq!(cell_to_string(&Data::Int(42)), "42");
        assert_eq!(cell_to_string(&Data::Bool(true)), "True");
        assert_eq!(cell_to_string(&Data::Empty), "");
    }

    #[test]
    fn cell_to_string_integral_float_is_int_text() {
        // 夹具实测（.t2tmp/kinds.txt）：整数值的 <v> 原文一律无 '.'（openpyxl 写出
        // int），openpyxl 读回 int → str 无小数点；非整数值走 str(float)
        assert_eq!(cell_to_string(&Data::Float(120.0)), "120");
        assert_eq!(cell_to_string(&Data::Float(0.0)), "0");
        assert_eq!(cell_to_string(&Data::Float(-0.0)), "0");
        assert_eq!(cell_to_string(&Data::Float(19906031621.0)), "19906031621");
        assert_eq!(cell_to_string(&Data::Float(1234567890.12)), "1234567890.12");
        assert_eq!(cell_to_string(&Data::Float(0.03)), "0.03");
        assert_eq!(cell_to_string(&Data::Int(42)), "42");
        assert_eq!(cell_to_string(&Data::String("ab".into())), "ab");
        assert_eq!(cell_to_string(&Data::Bool(true)), "True");
        assert_eq!(cell_to_string(&Data::Empty), "");
    }

    #[test]
    fn py_number_match_python_dollar_semantics() {
        assert!(py_number_match("123"));
        assert!(py_number_match("-45.5"));
        assert!(py_number_match("123\n")); // Python $ 匹配末尾换行
        assert!(!py_number_match("123\n\n"));
        assert!(!py_number_match(" 123"));
        assert!(!py_number_match("123."));
        assert!(!py_number_match("12.34.56"));
        assert!(!py_number_match("abc"));
        assert!(!py_number_match("-.5"));
        assert!(!py_number_match(""));
    }

    #[test]
    fn calamine_datetime_formatting() {
        // calamine to_ymd_hms_milli 原样输出（含 Excel 1900 闰年虚构日期，
        // 不做 openpyxl 的修正——下游语义比对不依赖日期串逐字节对齐）
        use calamine::ExcelDateTime;
        let d = |v: f64| Data::DateTime(ExcelDateTime::new(v, calamine::ExcelDateTimeType::DateTime, false));
        assert_eq!(cell_to_string(&d(46076.0)), "2026-02-23 00:00:00");
        assert_eq!(cell_to_string(&d(0.0)), "00:00:00"); // 纯时间值
        assert_eq!(cell_to_string(&d(0.5)), "12:00:00");
        // Excel 1900 闰年：serial 60 = 虚构1900-02-29（calamine原样输出）
        assert_eq!(cell_to_string(&d(60.0)), "1900-02-29 00:00:00");
        assert_eq!(cell_to_string(&d(61.0)), "1900-03-01 00:00:00");
        assert_eq!(cell_to_string(&d(25569.0)), "1970-01-01 00:00:00");
    }

    #[test]
    fn build_header_name_joins_and_cleans() {
        // 对应 tests/test_multi_header.py::test_build_header_name 的语义
        let parts = [
            Some("单据信息".to_string()),
            Some("单据编号".to_string()),
        ];
        assert_eq!(build_header_name(&parts), "单据信息_单据编号");
        // 空 part 跳过
        assert_eq!(
            build_header_name(&[None, Some("单据日期".to_string())]),
            "单据日期"
        );
        // [\n /]+ → _
        assert_eq!(
            build_header_name(&[Some("本期 金额/占比".to_string())]),
            "本期_金额_占比"
        );
        // 全空白 part 跳过（strip 后为空）
        assert_eq!(build_header_name(&[Some("  ".to_string())]), "");
    }

    #[test]
    fn find_header_row_single_and_multi() {
        // 单行表头（对应 tests/test_excel_scanner_v2.py::test_find_header_row）
        let rows = grid(vec![
            vec!["项目", "本期金额", "上期金额", "同比变动"],
            vec!["营业收入", "12345678.9", "11111111.11", "11.1%"],
            vec!["营业成本", "8765432.1", "7777777.77", "12.7%"],
        ]);
        assert_eq!(find_header_row(&rows), Some((0, 0)));

        // 多行表头（对应 tests/test_multi_header.py 的仓库单据布局，压缩到无标题行）
        let rows = grid(vec![
            vec!["单据信息", "", "产品详情", "", "入库详情", ""],
            vec!["单据编号", "单据日期", "产品名称", "产品规格", "本次入库", "补充说明"],
            vec!["CK-A001", "2026-04-01", "纸箱", "50*40cm", "120", "日常补货"],
        ]);
        assert_eq!(find_header_row(&rows), Some((0, 1)));
    }

    #[test]
    fn find_header_row_rejects_data_only_and_empty() {
        // 全数字行：无表头。Python 实测：短非中文值占比 ≥50% → is_annotation_row
        // 为真，row_info 全为注释行 → best_row_info None → (-1,-1)
        let rows = grid(vec![
            vec!["1", "2", "3", "4"],
            vec!["5", "6", "7", "8"],
        ]);
        assert_eq!(find_header_row(&rows), None);

        // 空表 → None
        let empty: Vec<Vec<Data>> = vec![];
        assert_eq!(find_header_row(&empty), None);
    }

    #[test]
    fn find_header_row_extends_up_through_title_rows() {
        // 标题/元数据行在表头上方（对应仓库入库单：第2行标题、第6行日期、8-9 表头）
        let rows = grid(vec![
            vec!["", "", "", "", "", ""],
            vec!["仓库入库单", "", "", "", "", ""],
            vec!["", "", "", "", "", ""],
            vec!["", "", "", "", "", ""],
            vec!["", "", "", "", "", ""],
            vec!["登记日期：", "2026-04-23", "", "", "", ""],
            vec!["", "", "", "", "", ""],
            vec!["单据信息", "", "产品详情", "", "入库详情", ""],
            vec!["单据编号", "单据日期", "产品名称", "产品规格", "本次入库", "补充说明"],
            vec!["CK-A001", "2026-04-01", "纸箱", "50*40cm", "120", "日常补货"],
        ]);
        // 对应 test_multi_header.py::test_find_multi_header：表头 8-9 行（0-based 7-8）
        assert_eq!(find_header_row(&rows), Some((7, 8)));
    }

    #[test]
    fn get_vertical_value_uses_merge_master() {
        // A8:B8 合并，B8 无值（openpyxl normal 模式 MergedCell）→ 取 A8
        let rows = grid(vec![
            vec!["单据信息", "", "单据编号"],
            vec!["X", "Y", "Z"],
        ]);
        let merged = [MergeRect {
            start: (0, 0),
            end: (0, 1),
        }];
        assert_eq!(get_vertical_value(&rows, &merged, 0, 0), "单据信息");
        assert_eq!(get_vertical_value(&rows, &merged, 0, 1), "单据信息");
        assert_eq!(get_vertical_value(&rows, &merged, 0, 2), "单据编号");
        assert_eq!(get_vertical_value(&rows, &merged, 1, 1), "Y");
        assert_eq!(get_vertical_value(&rows, &[], 5, 5), "");
    }

    #[test]
    fn is_annotation_row_and_is_data_row_port() {
        assert!(is_annotation_row(&["注释：以下数据仅供参考".to_string()]));
        assert!(is_annotation_row(&[
            "Notes".to_string(),
            "abc".to_string(),
            "de".to_string()
        ]));
        assert!(!is_annotation_row(&["单据编号".to_string(), "单据日期".to_string()]));

        assert!(is_data_row(&[
            "1".to_string(),
            "2".to_string(),
            "3".to_string(),
            "4".to_string()
        ]));
        assert!(!is_data_row(&[
            "单据编号".to_string(),
            "单据日期".to_string(),
            "产品名称".to_string()
        ]));
    }
}

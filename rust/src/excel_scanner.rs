//! Excel 文件扫描器 —— 移植自 src/finance_mask/scanner/excel_scanner_v2.py（617 行）。
//!
//! 本模块负责：加载 xlsx（calamine）→ 分类（classify.rs）→ 行列数据 + 合并单元格 →
//! 列规则提取 / Form 扫描 / 全文扫描 / 位点生成。外部由 main.rs 的 generate 子命令编排。
//!
//! 数据视图：`rows` 为稠密网格（1-based 第 r 行 = `rows[r-1]`，0-based 列索引），
//! 无值单元格为 `Data::Empty`（对应 openpyxl 的 None）。

use std::collections::HashMap;
use std::io::BufReader;
use std::path::Path;

use calamine::{Data, Reader, Xlsx};
use quick_xml::events::Event;
use quick_xml::Reader as XmlReader;
use zip::ZipArchive;

use crate::classify::{ClassifiedSheet, SheetType, classify_excel_sheets};
use crate::column_matcher::ColumnMatcher;
use crate::header_finder::{MergeRect, build_header_name, find_header_row, get_vertical_value};
use crate::models::{ColumnRule, MatchType};
use crate::patterns::PatternRegistry;


// ---------------------------------------------------------------------------
// SheetData：单个工作表的解析结果
// ---------------------------------------------------------------------------

/// 单个工作表的解析结果
#[derive(Debug)]
pub struct SheetData {
    /// 工作表名
    pub name: String,
    /// 分类结果（Data/Form/Unknown）
    pub sheet_type: SheetType,
    /// 稠密网格（1-based 第 r 行 = `rows[r-1]`，0-based 列索引）
    pub rows: Vec<Vec<Data>>,
    /// 合并单元格区域
    pub merged: Vec<MergeRect>,
    /// 表头起始行（0-based，含）；None = 未找到
    pub header_start: Option<u32>,
    /// 表头结束行（0-based，含）；None = 未找到
    pub header_end: Option<u32>,
    /// 最大列数（0-based 最大列索引 + 1，即列数）
    pub max_col: usize,
}

// ---------------------------------------------------------------------------
// 工作簿解析：xlsx → Vec<SheetData>
// ---------------------------------------------------------------------------

/// 加载 xlsx 文件，返回所有可见工作表的 `SheetData` 列表。
///
/// 分类使用 crate 的 `classify_excel_sheets`（当前 lib.rs 密度+熵阈值逻辑），
/// 若分类失败则所有工作表默认为 Form（与 Python `classify_with_fallback` 一致）。
pub fn parse_workbook(path: &Path) -> Result<Vec<SheetData>, Box<dyn std::error::Error>> {
    let mut workbook: Xlsx<_> = calamine::open_workbook(path)?;

    // 分类（含失败回退：全部 Form）
    let classifications = classify_excel_sheets(
        path.to_str().ok_or("non-UTF-8 path")?,
    )
    .unwrap_or_else(|_| {
        // 回退：所有工作表默认 Form（与 Python LayoutViewClassifier.classify_with_fallback 一致）
        // 注意：calamine 的 worksheets() 不含不可见表，故无需过滤
        Vec::new()
    });
    let class_map: HashMap<&str, &ClassifiedSheet> = classifications
        .iter()
        .map(|c| (c.original.sheet_name.as_str(), c))
        .collect();

    // 合并单元格（zip + quick-xml 解析 sheetN.xml 的 <mergeCells>）
    let merged_map = parse_all_merged_cells(path)?;

    let mut result = Vec::new();
    for (sheet_name, range) in workbook.worksheets() {
        // 跳过不可见工作表
        let meta = workbook
            .sheets_metadata()
            .iter()
            .find(|m| m.name == sheet_name);
        if let Some(m) = meta {
            if m.visible != calamine::SheetVisible::Visible {
                continue;
            }
        }

        let sheet_type = class_map
            .get(sheet_name.as_str())
            .map(|c| c.sheet_type.clone())
            .unwrap_or(SheetType::Form); // 回退默认

        let (start_row, start_col, end_row, end_col) = match range.start() {
            Some((sr, sc)) => match range.end() {
                Some((er, ec)) => (sr, sc, er, ec),
                None => {
                    result.push(empty_sheet(&sheet_name, sheet_type));
                    continue;
                }
            },
            None => {
                result.push(empty_sheet(&sheet_name, sheet_type));
                continue;
            }
        };

        let num_rows = (end_row - start_row + 1) as usize;
        let num_cols = (end_col - start_col + 1) as usize;
        let mut rows = Vec::with_capacity(num_rows);

        for r in start_row..=end_row {
            let mut row = Vec::with_capacity(num_cols);
            for c in start_col..=end_col {
                row.push(
                    range
                        .get_value((r, c))
                        .cloned()
                        .unwrap_or(Data::Empty),
                );
            }
            rows.push(row);
        }

        let merged = merged_map
            .get(sheet_name.as_str())
            .cloned()
            .unwrap_or_default();

        result.push(SheetData {
            name: sheet_name,
            sheet_type,
            rows,
            merged,
            header_start: None,
            header_end: None,
            max_col: num_cols,
        });
    }

    // 查找表头行（Data 类型工作表）
    for sd in &mut result {
        if sd.sheet_type == SheetType::Data && !sd.rows.is_empty() {
            if let Some((start, end)) = find_header_row(&sd.rows) {
                sd.header_start = Some(start);
                sd.header_end = Some(end);
            }
        }
    }

    Ok(result)
}

fn empty_sheet(name: &str, sheet_type: SheetType) -> SheetData {
    SheetData {
        name: name.to_string(),
        sheet_type,
        rows: Vec::new(),
        merged: Vec::new(),
        header_start: None,
        header_end: None,
        max_col: 0,
    }
}

// ---------------------------------------------------------------------------
// 合并单元格提取（zip + quick-xml）
// ---------------------------------------------------------------------------

/// 从 xlsx 文件中提取所有工作表的合并单元格区域。
/// key = 工作表名，value = 该表的合并区域列表。
fn parse_all_merged_cells(
    path: &Path,
) -> Result<HashMap<String, Vec<MergeRect>>, Box<dyn std::error::Error>> {
    let file = std::fs::File::open(path)?;
    let reader = BufReader::new(file);
    let mut archive = ZipArchive::new(reader)?;

    let mut result = HashMap::new();

    // 收集 sheetN.xml 的路径
    let sheet_paths: Vec<String> = archive
        .file_names()
        .filter(|n| {
            n.starts_with("xl/worksheets/sheet") && n.ends_with(".xml")
        })
        .map(|n| n.to_string())
        .collect();

    // 从 workbook.xml 提取 rId → sheet name 映射
    let wb_xml = {
        let mut f = archive.by_name("xl/workbook.xml")?;
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut f, &mut buf)?;
        buf
    };
    let rels_xml = {
        let mut f = archive.by_name("xl/_rels/workbook.xml.rels")?;
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut f, &mut buf)?;
        buf
    };

    // 解析 workbook.xml 中 <sheet> 的 name 和 r:id
    let mut sheet_entries: Vec<(String, String)> = Vec::new(); // (name, rId)
    let mut reader = XmlReader::from_str(&wb_xml);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(ref e)) | Ok(Event::Start(ref e)) => {
                if e.name().as_ref() == b"sheet" {
                    let name = e
                        .attributes()
                        .find(|a| a.as_ref().map(|a| a.key.as_ref() == b"name").unwrap_or(false))
                        .and_then(|a| a.ok())
                        .and_then(|a| String::from_utf8(a.value.to_vec()).ok())
                        .unwrap_or_default();
                    let rid = e
                        .attributes()
                        .find(|a| {
                            a.as_ref()
                                .map(|a| {
                                    let k = a.key.as_ref();
                                    k == b"r:id" || k == b"{http://schemas.openxmlformats.org/officeDocument/2006/relationships}id"
                                })
                                .unwrap_or(false)
                        })
                        .and_then(|a| a.ok())
                        .and_then(|a| String::from_utf8(a.value.to_vec()).ok())
                        .unwrap_or_default();
                    if !name.is_empty() && !rid.is_empty() {
                        sheet_entries.push((name, rid));
                    }
                }
            }
            Ok(Event::Eof) => break,
            _ => {}
        }
        buf.clear();
    }

    // 解析 rels：rId → Target
    let mut rid_to_target: HashMap<String, String> = HashMap::new();
    let mut reader = XmlReader::from_str(&rels_xml);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(ref e)) => {
                if e.name().as_ref() == b"Relationship" {
                    let id = e
                        .attributes()
                        .find(|a| a.as_ref().map(|a| a.key.as_ref() == b"Id").unwrap_or(false))
                        .and_then(|a| a.ok())
                        .and_then(|a| String::from_utf8(a.value.to_vec()).ok())
                        .unwrap_or_default();
                    let target = e
                        .attributes()
                        .find(|a| a.as_ref().map(|a| a.key.as_ref() == b"Target").unwrap_or(false))
                        .and_then(|a| a.ok())
                        .and_then(|a| String::from_utf8(a.value.to_vec()).ok())
                        .unwrap_or_default();
                    if !id.is_empty() && !target.is_empty() {
                        rid_to_target.insert(id, target);
                    }
                }
            }
            Ok(Event::Eof) => break,
            _ => {}
        }
        buf.clear();
    }

    // 构建 sheet_name → sheet_path 映射
    let mut name_to_path: HashMap<String, String> = HashMap::new();
    for (name, rid) in &sheet_entries {
        if let Some(target) = rid_to_target.get(rid) {
            // target 是相对路径如 "worksheets/sheet1.xml"
            let full = format!("xl/{}", target.trim_start_matches('/'));
            name_to_path.insert(name.clone(), full);
        }
    }

    // 对每个 sheet_path 解析 mergeCells
    for sheet_path in &sheet_paths {
        // 找到对应的 sheet_name
        let sheet_name = name_to_path
            .iter()
            .find(|(_, p)| p.as_str() == sheet_path.as_str())
            .map(|(n, _)| n.clone());

        let sheet_name = match sheet_name {
            Some(n) => n,
            None => continue,
        };

        let xml = {
            let mut f = archive.by_name(sheet_path)?;
            let mut buf = String::new();
            std::io::Read::read_to_string(&mut f, &mut buf)?;
            buf
        };

        let merged = parse_sheet_merges(&xml);
        if !merged.is_empty() {
            result.insert(sheet_name, merged);
        }
    }

    Ok(result)
}

/// 解析单个 sheetN.xml 中的 <mergeCell ref="A1:B2"/> 元素。
fn parse_sheet_merges(xml: &str) -> Vec<MergeRect> {
    let mut merged = Vec::new();
    let mut reader = XmlReader::from_str(xml);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(ref e)) => {
                if e.name().as_ref() == b"mergeCell" {
                    if let Some(ref_attr) = e
                        .attributes()
                        .find(|a| a.as_ref().map(|a| a.key.as_ref() == b"ref").unwrap_or(false))
                        .and_then(|a| a.ok())
                    {
                        let ref_str = std::str::from_utf8(&ref_attr.value).unwrap_or("");
                        if let Some(rect) = parse_merge_ref(ref_str) {
                            merged.push(rect);
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            _ => {}
        }
        buf.clear();
    }
    merged
}

/// 解析 "A1:B2" → MergeRect（0-based 行/列）
fn parse_merge_ref(s: &str) -> Option<MergeRect> {
    let (a, b) = s.split_once(':')?;
    let (r1, c1) = parse_cell_ref(a)?;
    let (r2, c2) = parse_cell_ref(b)?;
    Some(MergeRect {
        start: (r1, c1),
        end: (r2, c2),
    })
}

/// 解析 "A1" → (0-based 行, 0-based 列)
fn parse_cell_ref(s: &str) -> Option<(u32, u32)> {
    let bytes = s.as_bytes();
    let mut col = 0u32;
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
        col = col * 26 + (bytes[i].to_ascii_uppercase() - b'A' + 1) as u32;
        i += 1;
    }
    col -= 1; // 0-based
    let row: u32 = s[i..].parse().ok()?;
    Some((row - 1, col)) // 0-based
}

// ---------------------------------------------------------------------------
// 列字母转换
// ---------------------------------------------------------------------------

/// 0-based 列索引 → Excel 列字母（0=A, 1=B, ..., 25=Z, 26=AA）
pub fn col_letter(mut col: usize) -> String {
    let mut s = String::new();
    col += 1;
    while col > 0 {
        let rem = (col - 1) % 26;
        s.insert(0, (b'A' + rem as u8) as char);
        col = (col - 1) / 26;
    }
    s
}

// ---------------------------------------------------------------------------
// 单元格坐标格式
// ---------------------------------------------------------------------------

/// (0-based 行, 0-based 列) → Excel 单元格坐标（如 "B5"）
pub fn cell_coord(row: u32, col: u32) -> String {
    format!("{}{}", col_letter(col as usize), row + 1)
}

// ---------------------------------------------------------------------------
// 辅助：行 → 非空字符串列表（用于 is_annotation_row / is_data_row）
// ---------------------------------------------------------------------------

/// 取一行的非空单元格字符串（cell_to_string 后 trim 非空）。
pub fn row_non_empty_strings(row: &[Data]) -> Vec<String> {
    row.iter()
        .filter(|c| !matches!(c, Data::Empty))
        .map(|c| crate::header_finder::cell_to_string(c))
        .filter(|s| !s.trim().is_empty())
        .collect()
}

// ---------------------------------------------------------------------------
// ExcelScanner：列规则扫描器（对应 Python ExcelScannerV2 的扫描逻辑）
// ---------------------------------------------------------------------------

pub struct ExcelScanner {
    matcher: ColumnMatcher,
    patterns: PatternRegistry,
}

impl ExcelScanner {
    pub fn new(matcher: ColumnMatcher, patterns: PatternRegistry) -> Self {
        Self { matcher, patterns }
    }

    /// 扫描 Data 类型工作表，返回匹配到的列规则。
    /// 对应 Python `_scan_data_sheet`。
    ///
    /// - `header_start == None` → 无表头，返回 `None`
    /// - 有表头但无列匹配 → 返回 `Some(vec![])`
    pub fn scan_data_sheet(&self, sheet: &SheetData) -> Option<Vec<ColumnRule>> {
        let header_start = sheet.header_start?;
        let header_end = sheet.header_end.unwrap_or(header_start);

        // 构建每列的表头名称（可能跨多行）
        let mut header_names: Vec<String> = Vec::with_capacity(sheet.max_col);
        for col in 0..sheet.max_col {
            let parts: Vec<Option<String>> = (header_start..=header_end)
                .map(|row| {
                    let val =
                        get_vertical_value(&sheet.rows, &sheet.merged, row, col as u32);
                    if val.trim().is_empty() {
                        None
                    } else {
                        Some(val)
                    }
                })
                .collect();
            header_names.push(build_header_name(&parts));
        }

        // 去重列名：首次保留原名，后续 _1, _2, …（与 Python 一致）
        let mut seen: HashMap<String, usize> = HashMap::new();
        let mut deduped: Vec<String> = Vec::with_capacity(header_names.len());
        for name in header_names {
            let count = seen.entry(name.clone()).or_insert(0);
            if *count == 0 {
                deduped.push(name);
            } else {
                deduped.push(format!("{}_{}", name, count));
            }
            *count += 1;
        }

        // 逐列匹配规则
        let mut column_rules: Vec<ColumnRule> = Vec::new();
        for col_name in &deduped {
            let col_name_str = col_name.trim();
            if col_name_str.is_empty() {
                continue;
            }
            if let Some(rule) = self.matcher.match_header(col_name_str) {
                column_rules.push(ColumnRule {
                    match_type: MatchType::Exact,
                    pattern: col_name.clone(),
                    action: rule.action.clone(),
                    params: rule.params.clone(),
                    detected_type: rule.detected_type.clone(),
                    priority: rule.priority,
                });
            }
        }

        Some(column_rules)
    }

    /// 收集所有 Data 类型工作表的列规则（`sheet.name → rules`）。
    /// 对应 Python `get_column_rules`。
    pub fn get_column_rules(
        &self,
        sheets: &[SheetData],
    ) -> HashMap<String, Vec<ColumnRule>> {
        let mut result = HashMap::new();
        for sheet in sheets {
            if sheet.sheet_type == SheetType::Data {
                if let Some(rules) = self.scan_data_sheet(sheet) {
                    if !rules.is_empty() {
                        result.insert(sheet.name.clone(), rules);
                    }
                }
            }
        }
        result
    }
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ActionType, DetectedType};

    #[test]
    fn col_letter_basic() {
        assert_eq!(col_letter(0), "A");
        assert_eq!(col_letter(25), "Z");
        assert_eq!(col_letter(26), "AA");
        assert_eq!(col_letter(701), "ZZ");
    }

    #[test]
    fn cell_coord_format() {
        assert_eq!(cell_coord(0, 0), "A1");
        assert_eq!(cell_coord(4, 1), "B5");
    }

    #[test]
    fn parse_cell_ref_standard() {
        assert_eq!(parse_cell_ref("A1"), Some((0, 0)));
        assert_eq!(parse_cell_ref("B5"), Some((4, 1)));
        assert_eq!(parse_cell_ref("AA10"), Some((9, 26)));
    }

    #[test]
    fn parse_merge_ref_range() {
        let r = parse_merge_ref("A8:B8").unwrap();
        assert_eq!(r.start, (7, 0));
        assert_eq!(r.end, (7, 1));
        assert!(r.contains(7, 0));
        assert!(r.contains(7, 1));
        assert!(!r.contains(7, 2));
    }

    #[test]
    fn parse_sheet_merges_from_xml() {
        let xml = r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
            <sheetData/>
            <mergeCells count="2">
                <mergeCell ref="A1:B1"/>
                <mergeCell ref="C3:D5"/>
            </mergeCells>
        </worksheet>"#;
        let merges = parse_sheet_merges(xml);
        assert_eq!(merges.len(), 2);
        assert_eq!(merges[0], MergeRect { start: (0, 0), end: (0, 1) });
        assert_eq!(merges[1], MergeRect { start: (2, 2), end: (4, 3) });
    }

    #[test]
    fn parse_workbook_classifies_sheets() {
        let crate_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let path = crate_dir.join("tests/fixtures/data1.xlsx");
        let sheets = parse_workbook(&path).unwrap();
        assert!(!sheets.is_empty());
        // data1.xlsx 应至少有一个 Data 类型工作表
        assert!(
            sheets.iter().any(|s| s.sheet_type == SheetType::Data),
            "data1 should have Data sheets: {:?}",
            sheets.iter().map(|s| (&s.name, &s.sheet_type)).collect::<Vec<_>>()
        );
    }

    // -----------------------------------------------------------------------
    // ExcelScanner 测试
    // -----------------------------------------------------------------------

    /// 构建简单的 Data SheetData（手动构造网格）
    fn make_data_sheet(
        name: &str,
        rows: Vec<Vec<Data>>,
        header_start: u32,
        header_end: u32,
    ) -> SheetData {
        let max_col = rows.iter().map(|r| r.len()).max().unwrap_or(0);
        SheetData {
            name: name.to_string(),
            sheet_type: SheetType::Data,
            rows,
            merged: vec![],
            header_start: Some(header_start),
            header_end: Some(header_end),
            max_col,
        }
    }

    fn make_scanner(rules: Vec<ColumnRule>) -> ExcelScanner {
        let matcher = ColumnMatcher::new(rules);
        let patterns = PatternRegistry::builtin().unwrap();
        ExcelScanner::new(matcher, patterns)
    }

    #[test]
    fn scan_data_sheet_basic() {
        // 一行表头 + 两行数据
        let sheet = make_data_sheet(
            "测试表",
            vec![
                vec![
                    Data::String("姓名".into()),
                    Data::String("金额".into()),
                    Data::String("备注".into()),
                ],
                vec![
                    Data::String("张三".into()),
                    Data::Float(1234.5),
                    Data::String("测试".into()),
                ],
                vec![
                    Data::String("李四".into()),
                    Data::Float(5678.9),
                    Data::String("备注2".into()),
                ],
            ],
            0,
            0,
        );

        let scanner = make_scanner(vec![ColumnRule {
            match_type: MatchType::Exact,
            pattern: "金额".to_string(),
            action: ActionType::Precision,
            params: None,
            detected_type: DetectedType::Amount,
            priority: 0,
        }]);

        let result = scanner.scan_data_sheet(&sheet);
        assert!(result.is_some());
        let rules = result.unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].pattern, "金额");
        assert_eq!(rules[0].match_type, MatchType::Exact);
        assert_eq!(rules[0].action, ActionType::Precision);
        assert_eq!(rules[0].detected_type, DetectedType::Amount);
    }

    #[test]
    fn scan_data_sheet_no_header_returns_none() {
        let sheet = SheetData {
            name: "空表".to_string(),
            sheet_type: SheetType::Data,
            rows: vec![vec![Data::Float(1.0), Data::Float(2.0)]],
            merged: vec![],
            header_start: None,
            header_end: None,
            max_col: 2,
        };

        let scanner = make_scanner(vec![]);
        assert!(scanner.scan_data_sheet(&sheet).is_none());
    }

    #[test]
    fn scan_data_sheet_no_match_returns_empty() {
        let sheet = make_data_sheet(
            "测试表",
            vec![
                vec![Data::String("未知列".into()), Data::String("其他列".into())],
                vec![Data::Float(1.0), Data::Float(2.0)],
            ],
            0,
            0,
        );

        // 没有匹配规则
        let scanner = make_scanner(vec![ColumnRule {
            match_type: MatchType::Exact,
            pattern: "金额".to_string(),
            action: ActionType::Precision,
            params: None,
            detected_type: DetectedType::Amount,
            priority: 0,
        }]);

        let result = scanner.scan_data_sheet(&sheet).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn scan_data_sheet_dedup_second_gets_suffix() {
        // 两列同名 "金额"，第二列应变为 "金额_1"
        let sheet = make_data_sheet(
            "测试表",
            vec![
                vec![Data::String("金额".into()), Data::String("金额".into())],
                vec![Data::Float(100.0), Data::Float(200.0)],
            ],
            0,
            0,
        );

        // 两条规则分别匹配 "金额" 和 "金额_1"
        let scanner = make_scanner(vec![
            ColumnRule {
                match_type: MatchType::Exact,
                pattern: "金额".to_string(),
                action: ActionType::Precision,
                params: None,
                detected_type: DetectedType::Amount,
                priority: 0,
            },
            ColumnRule {
                match_type: MatchType::Exact,
                pattern: "金额_1".to_string(),
                action: ActionType::Precision,
                params: None,
                detected_type: DetectedType::Amount,
                priority: 0,
            },
        ]);

        let result = scanner.scan_data_sheet(&sheet).unwrap();
        assert_eq!(result.len(), 2, "should match both deduped names");
        assert_eq!(result[0].pattern, "金额");
        assert_eq!(result[1].pattern, "金额_1");
    }

    #[test]
    fn scan_data_sheet_dedup_three_same_names() {
        // 三列同名 "编号"，应变为 "编号", "编号_1", "编号_2"
        let sheet = make_data_sheet(
            "三列同名",
            vec![
                vec![
                    Data::String("编号".into()),
                    Data::String("编号".into()),
                    Data::String("编号".into()),
                ],
                vec![Data::Float(1.0), Data::Float(2.0), Data::Float(3.0)],
            ],
            0,
            0,
        );

        let scanner = make_scanner(vec![
            ColumnRule {
                match_type: MatchType::Exact,
                pattern: "编号".to_string(),
                action: ActionType::Mask,
                params: None,
                detected_type: DetectedType::Account,
                priority: 0,
            },
            ColumnRule {
                match_type: MatchType::Exact,
                pattern: "编号_1".to_string(),
                action: ActionType::Mask,
                params: None,
                detected_type: DetectedType::Account,
                priority: 0,
            },
            ColumnRule {
                match_type: MatchType::Exact,
                pattern: "编号_2".to_string(),
                action: ActionType::Mask,
                params: None,
                detected_type: DetectedType::Account,
                priority: 0,
            },
        ]);

        let result = scanner.scan_data_sheet(&sheet).unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].pattern, "编号");
        assert_eq!(result[1].pattern, "编号_1");
        assert_eq!(result[2].pattern, "编号_2");
    }

    #[test]
    fn get_column_rules_from_workbook() {
        let crate_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let path = crate_dir.join("tests/fixtures/data1.xlsx");
        let sheets = parse_workbook(&path).unwrap();

        // 空匹配器 → 所有表都无匹配
        let scanner_empty = make_scanner(vec![]);
        let result = scanner_empty.get_column_rules(&sheets);
        assert!(result.is_empty(), "empty matcher should match nothing");

        // 用内置列规则加载的匹配器（config 内嵌列规则可能匹配 data1 的表头）
        let builtin_rules =
            crate::config::builtin_column_rules().unwrap();
        let scanner = make_scanner(builtin_rules);
        let result = scanner.get_column_rules(&sheets);
        // data1 有 Data 表 → map 可能非空也可能为空（取决于表头内容）
        // 至少不 panic
        let _ = result;
    }
}

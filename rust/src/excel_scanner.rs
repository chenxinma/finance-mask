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
use crate::header_finder::{MergeRect, find_header_row};


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
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

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
}

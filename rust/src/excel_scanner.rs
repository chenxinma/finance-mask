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
use crate::header_finder::{MergeRect, build_header_name, cell_to_string, find_header_row, get_vertical_value};
use crate::models::{ActionType, ColumnRule, DetectedType, DiscoveredBy, Location, MatchType, Site, SiteType};
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

        // 网格从 (0,0) 开始构建（对齐 openpyxl 绝对坐标语义）：
        // used range 之外的行/列填 Empty。这样 rows[0] 恒对应 Excel 第 1 行，
        // cell_coord(row, col) 生成的坐标与 XML 里的 r="A1" 一致。
        let num_rows = (end_row + 1) as usize;
        let num_cols = (end_col + 1) as usize;
        let mut rows = Vec::with_capacity(num_rows);

        for r in 0..=end_row {
            let mut row = Vec::with_capacity(num_cols);
            for c in 0..=end_col {
                let cell = if r >= start_row && c >= start_col {
                    range.get_value((r, c)).cloned().unwrap_or(Data::Empty)
                } else {
                    Data::Empty
                };
                row.push(cell);
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

    /// 扫描 Form 类型工作表，返回匹配到的位点。
    /// 对应 Python `_scan_form_sheet`。
    ///
    /// 按单元格逐行逐列扫描：
    /// 1. 前 5 行中首次匹配列规则的单元格记录为表头列
    /// 2. 后续行中，已知列做列规则匹配，其余做全文正则扫描
    pub fn scan_form_sheet(&self, sheet: &SheetData) -> Vec<Site> {
        let mut sites = Vec::new();
        let mut header_row: HashMap<usize, String> = HashMap::new(); // 0-based col_idx → col_name
        let mut header_found = false;

        for (r_idx, row) in sheet.rows.iter().enumerate() {
            let row_1based = (r_idx + 1) as u32; // 1-based for cell_coord / Location.cell
            for (c_idx, cell) in row.iter().enumerate() {
                if matches!(cell, Data::Empty) {
                    continue;
                }
                let cell_str = crate::header_finder::cell_to_string(cell);
                let cell_str = cell_str.trim();
                if cell_str.is_empty() {
                    continue;
                }
                let coord = cell_coord(r_idx as u32, c_idx as u32); // 0-based args, 1-based output

                // --- 表头探测（前 5 行，首次命中即停） ---
                if row_1based <= 5 && !header_found {
                    if let Some(_rule) = self.matcher.match_header(cell_str) {
                        header_row.insert(c_idx, cell_str.to_string());
                        header_found = true;
                        continue; // 表头单元格本身不产生 Site
                    }
                }

                // --- 列规则匹配 ---
                if let Some(col_name) = header_row.get(&c_idx) {
                    if let Some(rule) = self.matcher.match_header(col_name) {
                        let action = rule.action.clone();
                        let params = rule.params.clone();
                        sites.push(Site {
                            site_id: format!("sheet_{}_{}", sheet.name, coord),
                            location: Location {
                                site_type: SiteType::Excel,
                                sheet: Some(sheet.name.clone()),
                                cell: Some(coord),
                                column: Some(col_name.clone()),
                                ..Location::default()
                            },
                            original_value: cell_str.to_string(),
                            detected_type: rule.detected_type.clone(),
                            discovered_by: DiscoveredBy::ColumnRule,
                            enabled: true,
                            action,
                            params,
                            redacted_value: None,
                        });
                        continue;
                    }
                }

                // --- 全文正则扫描（兜底） ---
                let hits = self.patterns.scan(cell_str);
                for (rule, matched_text) in hits {
                    let (action, params) = get_default_action(&rule.detected_type);
                    sites.push(Site {
                        site_id: format!("sheet_{}_{}", sheet.name, coord),
                        location: Location {
                            site_type: SiteType::Excel,
                            sheet: Some(sheet.name.clone()),
                            cell: Some(coord.clone()),
                            ..Location::default()
                        },
                        original_value: matched_text,
                        detected_type: rule.detected_type.clone(),
                        discovered_by: DiscoveredBy::FulltextScan,
                        enabled: true,
                        action,
                        params,
                        redacted_value: None,
                    });
                }
            }
        }

        sites
    }

    /// 扫描表头之前和数据之后的文本内容。
    /// 对应 Python `_scan_extra_text`。
    ///
    /// 对于 Data 类型表格，表头之前（如公司名称、表格标题）和
    /// 数据之后（如注释信息）的文本也需要进行敏感信息识别。
    pub fn scan_extra_text(
        &self,
        sheet: &SheetData,
        _column_rules: &[ColumnRule],
    ) -> Vec<Site> {
        let mut sites = Vec::new();

        // 无表头 → 无法确定 pre/post 区域
        let header_start = match sheet.header_start {
            Some(h) => h as usize,
            None => return sites,
        };
        let header_end = sheet.header_end.unwrap_or(header_start as u32) as usize;

        // --- 表头之前的行（rows 0..header_start） ---
        for r in 0..header_start {
            let row = match sheet.rows.get(r) {
                Some(row) => row,
                None => continue,
            };
            for (c, cell) in row.iter().enumerate() {
                let s = cell_to_string(cell);
                let s = s.trim();
                if s.is_empty() {
                    continue;
                }
                let coord = cell_coord(r as u32, c as u32);
                for (rule, matched_text) in self.patterns.scan(s) {
                    let (action, params) = get_default_action(&rule.detected_type);
                    sites.push(Site {
                        site_id: format!("sheet_{}_{}", sheet.name, coord),
                        location: Location {
                            site_type: SiteType::Excel,
                            sheet: Some(sheet.name.clone()),
                            cell: Some(coord.clone()),
                            ..Location::default()
                        },
                        original_value: matched_text,
                        detected_type: rule.detected_type.clone(),
                        discovered_by: DiscoveredBy::FulltextScan,
                        enabled: true,
                        action,
                        params,
                        redacted_value: None,
                    });
                }
            }
        }

        // --- 数据之后的行（rows header_end+1..sheet.rows.len()） ---
        for r in (header_end + 1)..sheet.rows.len() {
            let row = match sheet.rows.get(r) {
                Some(row) => row,
                None => continue,
            };

            // 检查 B 列（index 1）是否有值 → 有值说明是数据行，跳过
            let b_has_value = row
                .get(1)
                .map(|c| !matches!(c, Data::Empty))
                .unwrap_or(false);
            if b_has_value {
                continue;
            }

            // B 列为空 → 扫描 A 列（index 0）
            if let Some(cell) = row.first() {
                let s = cell_to_string(cell);
                let s = s.trim();
                if s.is_empty() {
                    continue;
                }
                let coord = cell_coord(r as u32, 0);
                for (rule, matched_text) in self.patterns.scan(s) {
                    let (action, params) = get_default_action(&rule.detected_type);
                    sites.push(Site {
                        site_id: format!("sheet_{}_{}", sheet.name, coord),
                        location: Location {
                            site_type: SiteType::Excel,
                            sheet: Some(sheet.name.clone()),
                            cell: Some(coord.clone()),
                            ..Location::default()
                        },
                        original_value: matched_text,
                        detected_type: rule.detected_type.clone(),
                        discovered_by: DiscoveredBy::FulltextScan,
                        enabled: true,
                        action,
                        params,
                        redacted_value: None,
                    });
                }
            }
        }

        sites
    }

    /// 扫描整个工作簿，返回所有匹配到的位点。
    /// 对应 Python `scan`。
    pub fn scan(&self, sheets: &[SheetData]) -> Vec<Site> {
        let mut sites = Vec::new();

        for sheet in sheets {
            if sheet.sheet_type == SheetType::Data {
                if let Some(rules) = self.scan_data_sheet(sheet) {
                    if !rules.is_empty() {
                        // 有列规则：仅采集 extra text 位点
                        // （_apply_column_rules 在后续任务实现）
                        let extra = self.scan_extra_text(sheet, &rules);
                        sites.extend(extra);
                    } else {
                        // 无列规则匹配 → 回退到 Form 扫描
                        sites.extend(self.scan_form_sheet(sheet));
                    }
                } else {
                    // 无表头 → 回退到 Form 扫描
                    sites.extend(self.scan_form_sheet(sheet));
                }
            } else {
                // Form / Unknown 类型
                sites.extend(self.scan_form_sheet(sheet));
            }
        }

        sites
    }
}

/// 敏感类型 → 默认脱敏动作（对应 Python `_get_default_action`）
pub fn get_default_action(detected_type: &DetectedType) -> (ActionType, Option<serde_json::Value>) {
    use serde_json::json;
    match detected_type {
        DetectedType::Amount => (
            ActionType::Precision,
            Some(json!({"unit": "million", "decimal_places": 2})),
        ),
        DetectedType::Entity => (
            ActionType::Alias,
            Some(json!({"prefix": "公司"})),
        ),
        DetectedType::Person => (ActionType::MaskName, None),
        DetectedType::Account => (
            ActionType::MaskAccount,
            Some(json!({"keep_prefix": 3, "keep_suffix": 4})),
        ),
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

    // -------------------------------------------------------------------
    // scan_form_sheet 测试
    // -------------------------------------------------------------------

    /// 构建 Form 类型 SheetData（手动构造网格）
    fn make_form_sheet(name: &str, rows: Vec<Vec<Data>>) -> SheetData {
        let max_col = rows.iter().map(|r| r.len()).max().unwrap_or(0);
        SheetData {
            name: name.to_string(),
            sheet_type: SheetType::Form,
            rows,
            merged: vec![],
            header_start: None,
            header_end: None,
            max_col,
        }
    }

    #[test]
    fn scan_form_sheet_basic() {
        // key-value 表单布局：row0=公司名称+阿里巴巴，row1=金额+1,234,567.89 元
        let sheet = make_form_sheet(
            "表单",
            vec![
                vec![
                    Data::String("公司名称".into()),
                    Data::String("阿里巴巴集团".into()),
                ],
                vec![
                    Data::String("金额".into()),
                    Data::String("1,234,567.89 元".into()),
                ],
            ],
        );

        // 空匹配器（无列规则）→ 所有命中来自全文扫描
        let scanner = make_scanner(vec![]);
        let sites = scanner.scan_form_sheet(&sheet);

        // 金额单元格应被全文扫描命中为 Amount
        let amount_site = sites.iter().find(|s| s.detected_type == DetectedType::Amount);
        assert!(
            amount_site.is_some(),
            "should detect amount in '1,234,567.89 元', got: {:?}",
            sites
        );
        let site = amount_site.unwrap();
        assert_eq!(site.discovered_by, DiscoveredBy::FulltextScan);
        assert_eq!(site.location.cell.as_deref(), Some("B2"));
        assert_eq!(site.location.sheet.as_deref(), Some("表单"));
    }

    #[test]
    fn scan_form_sheet_header_detection() {
        // 表头在 row0 col0：“公司名称” 匹配列规则
        // row1 col0 的值 “百度在线” 应走 ColumnRule 路径
        let sheet = make_form_sheet(
            "公司表",
            vec![
                vec![
                    Data::String("公司名称".into()),
                    Data::String("备注".into()),
                ],
                vec![
                    Data::String("百度在线网络技术有限公司".into()),
                    Data::String("普通备注".into()),
                ],
            ],
        );

        // 构建匹配 “公司名称” 的列规则
        let scanner = make_scanner(vec![ColumnRule {
            match_type: MatchType::Exact,
            pattern: "公司名称".to_string(),
            action: ActionType::Alias,
            params: Some(serde_json::json!({"prefix": "公司"})),
            detected_type: DetectedType::Entity,
            priority: 0,
        }]);

        let sites = scanner.scan_form_sheet(&sheet);

        // row1 col0 应产生 ColumnRule 位点（detected_type=Entity）
        let entity_site = sites
            .iter()
            .find(|s| s.discovered_by == DiscoveredBy::ColumnRule && s.detected_type == DetectedType::Entity);
        assert!(
            entity_site.is_some(),
            "should have ColumnRule entity site, got: {:?}",
            sites
        );
        let site = entity_site.unwrap();
        assert_eq!(site.location.cell.as_deref(), Some("A2"));
        assert_eq!(site.location.column.as_deref(), Some("公司名称"));
        assert_eq!(site.action, ActionType::Alias);
    }

    // -------------------------------------------------------------------
    // scan_extra_text 测试
    // -------------------------------------------------------------------

    #[test]
    fn scan_extra_text_finds_pattern_above_header() {
        // 构造一个 Data 表：row0=标题 "天齐锂业股份有限公司"，row1=表头，row2=数据
        // scan_extra_text 应在 row0 发现实体模式
        let sheet = make_data_sheet(
            "测试表",
            vec![
                // row 0: 标题行（header 之前）
                vec![Data::String("天齐锂业股份有限公司".into())],
                // row 1: 表头
                vec![
                    Data::String("公司名称".into()),
                    Data::String("金额".into()),
                ],
                // row 2: 数据行（B 列有值 → 跳过）
                vec![
                    Data::String("百度在线".into()),
                    Data::Float(1234.5),
                ],
            ],
            1, // header_start
            1, // header_end
        );

        let scanner = make_scanner(vec![]);
        let sites = scanner.scan_extra_text(&sheet, &[]);

        // 应在 A1（row0 col0）发现 Entity 位点
        assert_eq!(sites.len(), 1, "expected 1 site from title row, got: {:?}", sites);
        let site = &sites[0];
        assert_eq!(site.detected_type, DetectedType::Entity);
        assert_eq!(site.discovered_by, DiscoveredBy::FulltextScan);
        assert_eq!(site.location.cell.as_deref(), Some("A1"));
        assert_eq!(site.location.sheet.as_deref(), Some("测试表"));
        assert!(site.location.column.is_none(), "fulltext scan should have no column");
    }
}

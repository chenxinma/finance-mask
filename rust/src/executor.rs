//! 脱敏执行器 —— 移植自 src/finance_mask/engine/executor.py。
//!
//! 编排：加载策略 → Excel（列规则 + 位点）或 PPT（位点替换）→ xmlsurgeon/ppt_writer
//! 外科手术写回 → 水印嵌入 → 审计日志导出。

use std::collections::HashMap;
use std::path::Path;

use crate::audit::{self, AuditLogger};
use crate::engine::{mask_account, AmountRedactor, NameRedactor};
use crate::excel_scanner;
use crate::header_finder::{build_header_name, get_vertical_value};
use crate::models::*;
use crate::ppt_reader;
use crate::ppt_writer::PptxEditor;
use crate::watermark::Watermark;
use crate::xmlsurgeon::XmlSurgeon;

#[derive(Debug)]
pub enum ExecError {
    Surgeon(crate::xmlsurgeon::SurgeonError),
    PptReader(crate::ppt_reader::PptError),
    PptWriter(crate::ppt_writer::PptError),
    Audit(crate::audit::AuditError),
    Io(std::io::Error),
    /// 不支持的脱敏动作 / 缺参数等业务错误
    Redact(String),
}

impl std::fmt::Display for ExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecError::Surgeon(e) => write!(f, "xlsx 写回失败: {e}"),
            ExecError::PptReader(e) => write!(f, "pptx 解析失败: {e}"),
            ExecError::PptWriter(e) => write!(f, "pptx 写回失败: {e}"),
            ExecError::Audit(e) => write!(f, "审计导出失败: {e}"),
            ExecError::Io(e) => write!(f, "IO 错误: {e}"),
            ExecError::Redact(msg) => write!(f, "脱敏失败: {msg}"),
        }
    }
}

impl std::error::Error for ExecError {}

impl From<std::io::Error> for ExecError {
    fn from(e: std::io::Error) -> Self {
        ExecError::Io(e)
    }
}

#[derive(Debug, Default)]
pub struct ExecutionReport {
    pub success: bool,
    pub processed: usize,
    pub skipped: usize,
    pub errors: usize,
    pub dry_run: bool,
}

/// 脱敏执行器。一次 execute 调用处理一个文件；alias 映射在一次执行内保持一致。
pub struct Executor {
    strategy: Strategy,
    operator: String,
}

impl Executor {
    pub fn new(strategy: Strategy, operator: &str) -> Self {
        Self {
            strategy,
            operator: operator.to_string(),
        }
    }

    /// 执行脱敏。按扩展名分派（.xlsx → excel，.pptx → ppt）。
    pub fn execute(
        &mut self,
        input: &Path,
        output: &Path,
        dry_run: bool,
    ) -> Result<ExecutionReport, ExecError> {
        match input.extension().and_then(|e| e.to_str()) {
            Some(ext) if ext.eq_ignore_ascii_case("xlsx") => {
                self.execute_excel(input, output, dry_run)
            }
            Some(ext) if ext.eq_ignore_ascii_case("pptx") => {
                self.execute_ppt(input, output, dry_run)
            }
            _ => Err(ExecError::Redact(format!(
                "不支持的文件格式: {}",
                input.display()
            ))),
        }
    }

    // ------------------------------------------------------------------
    // Excel
    // ------------------------------------------------------------------

    fn execute_excel(
        &mut self,
        input: &Path,
        output: &Path,
        dry_run: bool,
    ) -> Result<ExecutionReport, ExecError> {
        let sheets = excel_scanner::parse_workbook(input)
            .map_err(|e| ExecError::Redact(format!("解析 xlsx 失败: {e}")))?;

        let source_file = input
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let output_file = output
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();

        let mut audit = AuditLogger::new(source_file, output_file, &self.operator);
        let mut name_redactor = NameRedactor::new();
        let mut report = ExecutionReport {
            success: true,
            dry_run,
            ..Default::default()
        };

        // 已执行写回的 cell（sheet, cell_ref) → redacted，供位点路径避免重复
        let mut written: HashMap<(String, String), String> = HashMap::new();

        let mut surgeon = if dry_run {
            None
        } else {
            Some(XmlSurgeon::open(input).map_err(ExecError::Surgeon)?)
        };

        // 1. 列规则（Data 表）
        if let Some(ref rules) = self.strategy.column_rules {
            if !rules.is_empty() {
                self.execute_column_rules_excel(
                    &sheets,
                    rules,
                    &mut audit,
                    &mut name_redactor,
                    &mut report,
                    &mut written,
                    surgeon.as_mut(),
                    dry_run,
                );
            }
        }

        // 2. 单元格位点（Form 表）
        let excel_sites: Vec<&Site> = self
            .strategy
            .sites
            .iter()
            .filter(|s| s.location.site_type == SiteType::Excel)
            .collect();
        for site in excel_sites {
            if !site.enabled {
                report.skipped += 1;
                continue;
            }
            let Some(ref sheet_name) = site.location.sheet else {
                continue;
            };
            let Some(ref cell_ref) = site.location.cell else {
                continue;
            };
            let key = (sheet_name.clone(), cell_ref.clone());
            if written.contains_key(&key) {
                // 列规则已处理该 cell
                continue;
            }
            let original = &site.original_value;
            match redact_value(
                &mut name_redactor,
                original,
                &site.action,
                site.params.as_ref(),
            ) {
                Ok(redacted) => {
                    audit.log_change(
                        &site.site_id,
                        location_json(&site.location),
                        original,
                        &redacted,
                        &action_str(&site.action),
                    );
                    if let Some(ref mut s) = surgeon {
                        // 尝试保留数值类型（对齐 Python executor 行为）
                        let write_value = if site.action == ActionType::Precision || site.action == ActionType::Perturb || site.action == ActionType::Mask {
                            try_numeric_value(&redacted)
                        } else {
                            redacted.clone()
                        };
                        if let Err(e) =
                            s.set_cell_text(sheet_name, cell_ref, &write_value)
                        {
                            report.errors += 1;
                            audit.log_error(
                                &site.site_id,
                                &format!("写回失败: {e}"),
                            );
                            continue;
                        }
                    }
                    written.insert(key, redacted);
                    report.processed += 1;
                }
                Err(e) => {
                    report.errors += 1;
                    audit.log_error(&site.site_id, &e.to_string());
                }
            }
        }

        // 3. 保存 + 水印 + 审计
        if !dry_run {
            if let Some(surgeon) = surgeon {
                surgeon.save(output).map_err(ExecError::Surgeon)?;
                embed_watermark_xlsx(output, &self.operator)
                    .map_err(ExecError::Surgeon)?;
            }
            let output_hash = audit::compute_file_hash(output)
                .map_err(ExecError::Audit)?;
            export_audit_log(&audit, input, output, Some(&output_hash))?;
        }

        Ok(report)
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_column_rules_excel(
        &self,
        sheets: &[excel_scanner::SheetData],
        rules: &[ColumnRule],
        audit: &mut AuditLogger,
        name_redactor: &mut NameRedactor,
        report: &mut ExecutionReport,
        written: &mut HashMap<(String, String), String>,
        mut surgeon: Option<&mut XmlSurgeon>,
        dry_run: bool,
    ) {
        for sheet in sheets {
            let Some(header_start) = sheet.header_start else {
                continue;
            };
            let Some(header_end) = sheet.header_end else {
                continue;
            };

            // 列名 → 列索引（重复列名加 _1/_2 后缀，同 scan_data_sheet）
            let mut col_name_to_idx: HashMap<String, u32> = HashMap::new();
            for col_idx in 0..sheet.max_col as u32 {
                let mut parts = Vec::new();
                for row in header_start..=header_end {
                    let v = get_vertical_value(&sheet.rows, &sheet.merged, row, col_idx);
                    if v.trim().is_empty() {
                        parts.push(None);
                    } else {
                        parts.push(Some(v));
                    }
                }
                let col_name = build_header_name(&parts);
                if col_name.is_empty() {
                    continue;
                }
                if col_name_to_idx.contains_key(&col_name) {
                    let mut counter = 1;
                    loop {
                        let new_name = format!("{col_name}_{counter}");
                        if !col_name_to_idx.contains_key(&new_name) {
                            col_name_to_idx.insert(new_name, col_idx);
                            break;
                        }
                        counter += 1;
                    }
                } else {
                    col_name_to_idx.insert(col_name, col_idx);
                }
            }

            // 应用每条列规则
            for rule in rules {
                let Some(&col_idx) = col_name_to_idx.get(&rule.pattern) else {
                    continue;
                };
                // 数据行：header_end+1 .. rows.len()
                for row_idx in (header_end + 1) as usize..sheet.rows.len() {
                    let Some(cell) = sheet.rows[row_idx].get(col_idx as usize)
                    else {
                        continue;
                    };
                    if matches!(cell, calamine::Data::Empty) {
                        continue;
                    }
                    let original =
                        crate::header_finder::cell_to_string(cell).trim().to_string();
                    if original.is_empty() {
                        continue;
                    }
                    let cell_ref = excel_scanner::cell_coord(row_idx as u32, col_idx);
                    match redact_value(
                        name_redactor,
                        &original,
                        &rule.action,
                        rule.params.as_ref(),
                    ) {
                        Ok(redacted) => {
                            // I6: site_id format matches Python: {sheet}_{cell}
                            let site_id =
                                format!("{}_{}", sheet.name, cell_ref);
                            audit.log_change(
                                &site_id,
                                serde_json::json!({
                                    "type": "excel",
                                    "sheet": sheet.name,
                                    "cell": cell_ref,
                                    "column": rule.pattern,
                                }),
                                &original,
                                &redacted,
                                &action_str(&rule.action),
                            );
                            if !dry_run {
                                if let Some(ref mut s) = surgeon {
                                    // 尝试保留数值类型（对齐 Python executor 行为）
                                    let write_value = if rule.action == ActionType::Precision || rule.action == ActionType::Perturb || rule.action == ActionType::Mask {
                                        try_numeric_value(&redacted)
                                    } else {
                                        redacted.clone()
                                    };
                                    // I7: don't swallow write-back errors
                                    match s.set_cell_text(&sheet.name, &cell_ref, &write_value) {
                                        Ok(()) => {
                                            written.insert(
                                                (sheet.name.clone(), cell_ref),
                                                redacted,
                                            );
                                        }
                                        Err(e) => {
                                            report.errors += 1;
                                            audit.log_error(
                                                &site_id,
                                                &format!("写回失败: {e}"),
                                            );
                                        }
                                    }
                                }
                            }
                            report.processed += 1;
                        }
                        Err(e) => {
                            report.errors += 1;
                            audit.log_error(
                                &format!("{}_{}", sheet.name, cell_ref),
                                &e.to_string(),
                            );
                        }
                    }
                }
            }
        }
    }

    // ------------------------------------------------------------------
    // PPT
    // ------------------------------------------------------------------

    fn execute_ppt(
        &mut self,
        input: &Path,
        output: &Path,
        dry_run: bool,
    ) -> Result<ExecutionReport, ExecError> {
        let slides = ppt_reader::parse_pptx(input).map_err(ExecError::PptReader)?;

        let source_file = input
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let output_file = output
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();

        let mut audit = AuditLogger::new(source_file, output_file, &self.operator);
        let mut name_redactor = NameRedactor::new();
        let mut report = ExecutionReport {
            success: true,
            dry_run,
            ..Default::default()
        };

        let mut editor = if dry_run {
            None
        } else {
            Some(PptxEditor::open(input).map_err(ExecError::PptWriter)?)
        };

        for site in &self.strategy.sites {
            if site.location.site_type != SiteType::Ppt {
                continue;
            }
            if !site.enabled {
                report.skipped += 1;
                continue;
            }
            let Some(slide_idx) = site.location.slide else {
                continue;
            };
            // 定位该位点的当前文本（优先从解析数据读取，回退 original_value）
            let current = current_ppt_text(&slides, slide_idx as usize, site)
                .unwrap_or_else(|| site.original_value.clone());

            match redact_value(
                &mut name_redactor,
                &current,
                &site.action,
                site.params.as_ref(),
            ) {
                Ok(redacted) => {
                    audit.log_change(
                        &site.site_id,
                        location_json(&site.location),
                        &current,
                        &redacted,
                        &action_str(&site.action),
                    );
                    if let Some(ref mut ed) = editor {
                        ed.replace_text(slide_idx as usize, &current, &redacted)
                            .map_err(ExecError::PptWriter)?;
                    }
                    report.processed += 1;
                }
                Err(e) => {
                    report.errors += 1;
                    audit.log_error(&site.site_id, &e.to_string());
                }
            }
        }

        if !dry_run {
            if let Some(editor) = editor {
                editor.save(output).map_err(ExecError::PptWriter)?;
            }
            // I1: embed watermark into pptx
            embed_watermark_pptx(output, &self.operator)
                .map_err(ExecError::PptWriter)?;
            let output_hash = audit::compute_file_hash(output)
                .map_err(ExecError::Audit)?;
            export_audit_log(&audit, input, output, Some(&output_hash))?;
        }

        Ok(report)
    }
}

// ---------------------------------------------------------------------------
// 脱敏值分发（对应 Python _redact_value）
// ---------------------------------------------------------------------------

pub fn redact_value(
    name_redactor: &mut NameRedactor,
    value: &str,
    action: &ActionType,
    params: Option<&serde_json::Value>,
) -> Result<String, ExecError> {
    let p = |key: &str| -> Option<&serde_json::Value> {
        params.and_then(|p| p.get(key))
    };
    let p_str = |key: &str, default: &str| -> String {
        p(key)
            .and_then(|v| v.as_str())
            .unwrap_or(default)
            .to_string()
    };
    let p_f64 = |key: &str, default: f64| -> f64 {
        p(key).and_then(|v| v.as_f64()).unwrap_or(default)
    };
    let p_usize = |key: &str, default: usize| -> usize {
        p(key).and_then(|v| v.as_u64()).map(|v| v as usize).unwrap_or(default)
    };

    match action {
        ActionType::Precision => {
            let unit = p_str("unit", "million");
            let dp = p_usize("decimal_places", 2);
            AmountRedactor::precision(value, &unit, dp)
                .map_err(|e| ExecError::Redact(e.to_string()))
        }
        ActionType::Perturb => {
            let percentage = p_f64("percentage", 5.0);
            // seed：确定性扰动（简单哈希 value）
            let seed = simple_seed(value);
            AmountRedactor::perturb(value, percentage, seed)
                .map_err(|e| ExecError::Redact(e.to_string()))
        }
        ActionType::Mask => Ok(AmountRedactor::mask(value)),
        ActionType::Alias => {
            let prefix = p_str("prefix", "公司");
            Ok(name_redactor.alias(value, &prefix))
        }
        ActionType::MaskName => {
            let keep_first = p_usize("keep_first", 1);
            Ok(NameRedactor::mask_name(value, keep_first))
        }
        ActionType::MaskAccount => {
            let keep_prefix = p_usize("keep_prefix", 3);
            let keep_suffix = p_usize("keep_suffix", 4);
            Ok(mask_account(value, keep_prefix, keep_suffix, '*'))
        }
        ActionType::DifferentialShift => {
            Err(ExecError::Redact(
                "差分偏移模式需要预计算的 params.shift 参数".into(),
            ))
        }
        ActionType::ProportionalScale => {
            Err(ExecError::Redact(
                "比例缩放模式需要预计算的 params.scale 参数".into(),
            ))
        }
    }
}

/// 尝试将脱敏值转为数值字符串（对齐 Python executor 的数值保留逻辑）。
///
/// Python 在写入 Excel 前会 strip 单位后缀再 parse float：
/// ```python
/// cell.value = float(redacted_value.replace(",", "").replace("元", "").replace("百万", "").replace("亿", "").replace("千", ""))
/// ```
/// 成功则返回纯数值字符串（如 "22"），失败则返回原值。
fn try_numeric_value(redacted: &str) -> String {
    let cleaned = redacted
        .replace(',', "")
        .replace("元", "")
        .replace("百万", "")
        .replace("亿", "")
        .replace("千", "")
        .replace(' ', "");
    if let Ok(n) = cleaned.parse::<f64>() {
        // 保持整数不带小数点，浮点保留原样
        if n.fract() == 0.0 && !cleaned.contains('.') {
            format!("{}", n as i64)
        } else {
            format!("{}", n)
        }
    } else {
        redacted.to_string()
    }
}

/// 简单确定性 seed（value 的 FNV-1a 哈希）。
///
/// **有意偏差 vs Python**：Python 的 perturb 使用 `random.Random(value)` 生成
/// 随机偏移，每次运行结果不同。Rust 使用 FNV-1a 哈希保证确定性——同一输入
/// 永远产生同一扰动值，便于可重现构建和测试。审计日志中 perturb 行的
/// `redacted` 值不可能与 Python 的随机结果匹配，这是设计决策而非 bug。
fn simple_seed(s: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

// ---------------------------------------------------------------------------
// 辅助
// ---------------------------------------------------------------------------

fn action_str(a: &ActionType) -> String {
    match a {
        ActionType::Precision => "precision",
        ActionType::Perturb => "perturb",
        ActionType::Mask => "mask",
        ActionType::Alias => "alias",
        ActionType::MaskName => "mask_name",
        ActionType::MaskAccount => "mask_account",
        ActionType::DifferentialShift => "differential_shift",
        ActionType::ProportionalScale => "proportional_scale",
    }
    .to_string()
}

/// Location → JSON（省略 None 字段，对齐 Python model_dump(exclude_none=True)）
fn location_json(loc: &Location) -> serde_json::Value {
    let mut m = serde_json::Map::new();
    let t = match loc.site_type {
        SiteType::Excel => "excel",
        SiteType::Ppt => "ppt",
    };
    m.insert("type".into(), t.into());
    if let Some(ref v) = loc.sheet {
        m.insert("sheet".into(), v.clone().into());
    }
    if let Some(ref v) = loc.cell {
        m.insert("cell".into(), v.clone().into());
    }
    if let Some(ref v) = loc.column {
        m.insert("column".into(), v.clone().into());
    }
    if let Some(v) = loc.slide {
        m.insert("slide".into(), v.into());
    }
    if let Some(ref v) = loc.shape_id {
        m.insert("shape_id".into(), v.clone().into());
    }
    if let Some(ref v) = loc.table_location {
        m.insert("table_location".into(), v.clone().into());
    }
    serde_json::Value::Object(m)
}

/// 从解析的 PPT 数据读取位点的当前文本。
/// 备注位点 → notes 段落；表格位点 → table_rows[r][c]；文本框 → 段落查找。
fn current_ppt_text(
    slides: &[ppt_reader::PptSlide],
    slide_idx: usize,
    site: &Site,
) -> Option<String> {
    let slide = slides.iter().find(|s| s.slide_idx == slide_idx)?;

    // 备注位点
    if site.location.shape_id.as_deref() == Some("notes") {
        return slide
            .notes
            .iter()
            .find(|n| n.contains(&site.original_value))
            .cloned();
    }

    // 表格位点（R2C3）
    if let Some(ref tl) = site.location.table_location {
        let (row, col) = parse_table_location(tl)?;
        for shape in &slide.shapes {
            if shape.kind != ppt_reader::ShapeKind::Table {
                continue;
            }
            if let Some(cell) = shape
                .table_rows
                .get(row.checked_sub(1)?)
                .and_then(|r| r.get(col.checked_sub(1)?))
            {
                if cell.contains(&site.original_value) {
                    return Some(cell.clone());
                }
            }
        }
        return None;
    }

    // 文本框：查找包含 original_value 的段落
    for shape in &slide.shapes {
        if shape.kind != ppt_reader::ShapeKind::TextBox {
            continue;
        }
        for para in &shape.paragraphs {
            if para.contains(&site.original_value) {
                return Some(para.clone());
            }
        }
    }
    None
}

/// "R2C3" → (2, 3)（1-based）
fn parse_table_location(tl: &str) -> Option<(usize, usize)> {
    let rest = tl.strip_prefix('R')?;
    let (r, c) = rest.split_once('C')?;
    Some((r.parse().ok()?, c.parse().ok()?))
}

/// 导出审计日志：{input_stem}_日志.json（与 Python get_log_filename 一致）
fn export_audit_log(
    audit: &AuditLogger,
    input: &Path,
    output: &Path,
    file_hash: Option<&str>,
) -> Result<(), ExecError> {
    let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("output");
    let log_path = output.with_file_name(format!("{stem}_日志.json"));
    audit::export(audit, &log_path, file_hash).map_err(ExecError::Audit)?;
    Ok(())
}

/// Excel 水印嵌入：每个 sheet 前 10 行的首个非空单元格末尾追加零宽字符。
/// 对应 Python WatermarkEncoder.embed_to_excel（先 save 再嵌入，二次写回）。
fn embed_watermark_xlsx(
    output: &Path,
    operator: &str,
) -> Result<(), crate::xmlsurgeon::SurgeonError> {
    let payload = format!(
        "{}|{}|{}",
        operator,
        chrono::Local::now().format("%Y-%m-%dT%H:%M:%S"),
        &audit::compute_file_hash(output)
            .map(|h| h[..16.min(h.len())].to_string())
            .unwrap_or_default()
    );
    let watermark = Watermark::encode(&payload);

    let sheets = excel_scanner::parse_workbook(output)
        .map_err(|e| crate::xmlsurgeon::SurgeonError::EntryNotFound(format!(
            "水印解析失败: {e}"
        )))?;
    let mut surgeon = XmlSurgeon::open(output)?;
    for sheet in &sheets {
        // 前 10 行首个非空 cell
        'outer: for row in sheet.rows.iter().take(10) {
            for (col_idx, cell) in row.iter().enumerate() {
                if matches!(cell, calamine::Data::Empty) {
                    continue;
                }
                let text = crate::header_finder::cell_to_string(cell).trim().to_string();
                if text.is_empty() {
                    continue;
                }
                // 找到首个非空 cell：追加水印
                let row_idx = sheet
                    .rows
                    .iter()
                    .position(|r| std::ptr::eq(r, row))
                    .unwrap_or(0);
                let cell_ref = excel_scanner::cell_coord(row_idx as u32, col_idx as u32);
                let marked = format!("{text}{watermark}");
                surgeon.set_cell_text(&sheet.name, &cell_ref, &marked)?;
                break 'outer;
            }
        }
    }
    surgeon.save(output)?;
    Ok(())
}

/// PPTX 水印嵌入：遍历每个 slide，找到第一个 <a:t> 文本追加零宽字符。
/// 对应 Python WatermarkEncoder.embed_to_ppt。
fn embed_watermark_pptx(
    output: &Path,
    operator: &str,
) -> Result<(), crate::ppt_writer::PptError> {
    use std::io::{Read, Write};
    use zip::read::ZipArchive;
    use zip::write::{SimpleFileOptions, ZipWriter};

    let payload = format!(
        "{}|{}|{}",
        operator,
        chrono::Local::now().format("%Y-%m-%dT%H:%M:%S"),
        &audit::compute_file_hash(output)
            .map(|h| h[..16.min(h.len())].to_string())
            .unwrap_or_default()
    );
    let watermark = Watermark::encode(&payload);
    if watermark.is_empty() {
        return Ok(());
    }

    // Read all entries from the pptx zip.
    // IMPORTANT: drop archive BEFORE File::create on Windows — the file handle
    // must be released so truncation doesn't corrupt buffered reads.
    let entries: Vec<(String, Vec<u8>)> = {
        let file = std::fs::File::open(output)?;
        let mut archive = ZipArchive::new(std::io::BufReader::new(file))?;
        let mut entries = Vec::new();
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)?;
            let name = entry.name().to_string();
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            entries.push((name, buf));
        }
        entries
    };

    // For each slide XML, find first <a:t> (with or without attributes) and append watermark
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(6));

    let out_file = std::fs::File::create(output)?;
    let mut zip = ZipWriter::new(std::io::BufWriter::new(out_file));

    let mut watermarked = false;
    for (name, content) in &entries {
        if name.starts_with("ppt/slides/slide") && name.ends_with(".xml") && !watermarked {
            let xml = String::from_utf8_lossy(content);
            // Look for <a:t> or <a:t ...> (with attributes like xml:space="preserve")
            let needle = find_a_t_open(&xml);
            if let Some((_idx, tag_end)) = needle {
                let text_start = tag_end;
                if let Some(end_idx) = xml[text_start..].find("</a:t>") {
                    let original_text = &xml[text_start..text_start + end_idx];
                    let new_text = format!("{}{}", original_text, &watermark);
                    let new_xml = format!("{}{}{}",
                        &xml[..text_start],
                        quick_xml::escape::escape(&new_text),
                        &xml[text_start + end_idx..]
                    );
                    zip.start_file(name, options)?;
                    zip.write_all(new_xml.as_bytes())?;
                    watermarked = true;
                    continue;
                }
            }
        }
        zip.start_file(name, options)?;
        zip.write_all(content)?;
    }

    zip.finish()?;
    Ok(())
}

/// Find an `<a:t>` opening tag (with or without attributes) in the XML string.
/// Returns (start_of_tag, position_after_closing_>).
fn find_a_t_open(xml: &str) -> Option<(usize, usize)> {
    let bytes = xml.as_bytes();
    let mut search_from = 0;
    loop {
        let rel = xml[search_from..].find("<a:t")?;
        let start = search_from + rel;
        let after = start + 4;
        if after >= bytes.len() {
            return None;
        }
        let next = bytes[after];
        // Must be followed by >, /, or whitespace to be <a:t>, not <a:tbl> etc.
        let is_a_t = next == b'>' || next == b'/' || next == b' ' || next == b'\t' || next == b'\r' || next == b'\n';
        if is_a_t {
            // Find the closing >
            if let Some(gt) = xml[start..].find('>') {
                let tag_end = start + gt + 1;
                // Skip self-closing <a:t/> (no text content)
                if xml[start..tag_end].ends_with("/>") {
                    search_from = tag_end;
                    continue;
                }
                return Some((start, tag_end));
            }
        }
        search_from = after;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::NameRedactor;

    #[test]
    fn redact_value_dispatches_all_simple_actions() {
        let mut nr = NameRedactor::new();
        // mask
        assert_eq!(
            redact_value(&mut nr, "12,345.00", &ActionType::Mask, None).unwrap(),
            "1*,***.**"
        );
        // mask_name
        assert_eq!(
            redact_value(&mut nr, "张三丰", &ActionType::MaskName, None).unwrap(),
            "张**"
        );
        // mask_account
        assert_eq!(
            redact_value(
                &mut nr,
                "6222021234567890",
                &ActionType::MaskAccount,
                Some(&serde_json::json!({"keep_prefix": 3, "keep_suffix": 4}))
            )
            .unwrap(),
            "622*********7890"
        );
        // alias（状态一致性）
        let a1 = redact_value(
            &mut nr,
            "天齐锂业股份有限公司",
            &ActionType::Alias,
            Some(&serde_json::json!({"prefix": "公司"})),
        )
        .unwrap();
        let a2 = redact_value(
            &mut nr,
            "天齐锂业股份有限公司",
            &ActionType::Alias,
            Some(&serde_json::json!({"prefix": "公司"})),
        )
        .unwrap();
        assert_eq!(a1, a2, "同一实体应映射同一别名");
        // precision
        let p = redact_value(
            &mut nr,
            "22.12亿元",
            &ActionType::Precision,
            None,
        )
        .unwrap();
        assert_eq!(p, "22亿元");
        // differential_shift / proportional_scale 无预计算参数 → 错误
        assert!(redact_value(&mut nr, "100", &ActionType::DifferentialShift, None).is_err());
        assert!(redact_value(&mut nr, "100", &ActionType::ProportionalScale, None).is_err());
    }

    #[test]
    fn parse_table_location_r2c3() {
        assert_eq!(parse_table_location("R2C3"), Some((2, 3)));
        assert_eq!(parse_table_location("R10C12"), Some((10, 12)));
        assert_eq!(parse_table_location("bad"), None);
    }

    #[test]
    fn excel_execute_roundtrip_on_fixture() {
        // 用 data1.xlsx 真实解析构建策略：一条列规则（data1 增员表的某个表头列）
        let input = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/data1.xlsx");
        let sheets = excel_scanner::parse_workbook(&input).unwrap();
        assert!(!sheets.is_empty());
        let sheet = &sheets[0];

        // 构建策略：对第一列做 mask（pattern 用第一列表头名——从表头行拿）
        let header_start = sheet.header_start.unwrap_or(0);
        let header_end = sheet.header_end.unwrap_or(header_start);
        let mut first_header = String::new();
        for col_idx in 0..sheet.max_col as u32 {
            let mut parts = Vec::new();
            for row in header_start..=header_end {
                let v = get_vertical_value(&sheet.rows, &sheet.merged, row, col_idx);
                parts.push(if v.trim().is_empty() { None } else { Some(v) });
            }
            let name = build_header_name(&parts);
            if !name.is_empty() {
                first_header = name;
                break;
            }
        }
        assert!(!first_header.is_empty(), "fixture 应有表头");

        let strategy = Strategy {
            metadata: crate::models::Metadata::default(),
            column_rules: Some(vec![ColumnRule {
                match_type: MatchType::Exact,
                pattern: first_header,
                action: ActionType::Mask,
                params: None,
                detected_type: DetectedType::Amount,
                priority: 0,
            }]),
            sites: vec![],
            global_params: None,
        };

        let tmp = tempfile::NamedTempFile::with_suffix(".xlsx").unwrap();
        let mut exec = Executor::new(strategy, "tester");
        let report = exec.execute(&input, tmp.path(), false).unwrap();
        assert!(report.success);
        assert!(report.processed > 0, "列规则应处理至少一个数据 cell");

        // 审计日志存在（命名基于输入文件 stem，与 Python get_log_filename 一致）
        let input_stem = input.file_stem().unwrap().to_str().unwrap();
        let log_path = tmp.path().with_file_name(format!("{input_stem}_日志.json"));
        assert!(log_path.exists(), "审计日志应写入: {}", log_path.display());
        let log: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&log_path).unwrap())
                .unwrap();
        assert!(log["total_changes"].as_u64().unwrap() > 0);

        // 输出文件可被 calamine 重新打开
        let reparsed = excel_scanner::parse_workbook(tmp.path()).unwrap();
        assert!(!reparsed.is_empty());
    }
}

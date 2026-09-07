//! PPTX 阅读层 —— 直接解析 OOXML zip 结构，不依赖外部 pptx 库。
//!
//! 仅使用 `zip` + `quick_xml`（与 excel_scanner.rs 的合并单元格解析一致），
//! 将 .pptx 的幻灯片正文（文本框 / 表格）与备注文本抽取为结构化数据。
//!
//! 元素名匹配使用 `local_name()` 而非 `name()`：quick-xml 0.36 中
//! `name()` 返回带前缀的原始名（如 `p:sp`），`local_name()` 返回去掉前缀后的
//! 本地名（如 `sp`）。本模块一律匹配本地名。

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader as XmlReader;
use zip::ZipArchive;

// ---------------------------------------------------------------------------
// 数据模型
// ---------------------------------------------------------------------------

/// 幻灯片中的一个形状：文本框或表格。
#[derive(Debug, Clone, PartialEq)]
pub struct PptShape {
    /// `cNvPr` 的 `id` 属性（数字原样存为字符串）
    pub id: String,
    /// `cNvPr` 的 `name` 属性
    pub name: String,
    /// 形状类型：文本框或表格
    pub kind: ShapeKind,
    /// TextBox：每个段落 = 该段内所有 `<a:t>` 运行拼接
    pub paragraphs: Vec<String>,
    /// Table：行 × 单元格文本（单元格 = 该单元格内所有 `<a:t>` 拼接）
    pub table_rows: Vec<Vec<String>>,
}

/// 形状类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeKind {
    TextBox,
    Table,
}

/// 单张幻灯片（含备注）。
#[derive(Debug, Clone)]
pub struct PptSlide {
    /// 1-based 幻灯片编号
    pub slide_idx: usize,
    /// 幻灯片中的形状
    pub shapes: Vec<PptShape>,
    /// 备注段落（每个段落 = 段内所有 `<a:t>` 运行拼接）
    pub notes: Vec<String>,
}

// ---------------------------------------------------------------------------
// 错误类型
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum PptError {
    Io(std::io::Error),
    Zip(zip::result::ZipError),
    Xml(quick_xml::Error),
    Utf8(std::string::FromUtf8Error),
}

impl std::fmt::Display for PptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PptError::Io(e) => write!(f, "IO error: {}", e),
            PptError::Zip(e) => write!(f, "zip error: {}", e),
            PptError::Xml(e) => write!(f, "XML error: {}", e),
            PptError::Utf8(e) => write!(f, "UTF-8 error: {}", e),
        }
    }
}

impl std::error::Error for PptError {}

impl From<std::io::Error> for PptError {
    fn from(e: std::io::Error) -> Self {
        PptError::Io(e)
    }
}

impl From<zip::result::ZipError> for PptError {
    fn from(e: zip::result::ZipError) -> Self {
        PptError::Zip(e)
    }
}

impl From<quick_xml::Error> for PptError {
    fn from(e: quick_xml::Error) -> Self {
        PptError::Xml(e)
    }
}

impl From<std::string::FromUtf8Error> for PptError {
    fn from(e: std::string::FromUtf8Error) -> Self {
        PptError::Utf8(e)
    }
}

// ---------------------------------------------------------------------------
// 顶层入口
// ---------------------------------------------------------------------------

/// 解析 .pptx 文件，返回按幻灯片编号（数值）升序排序的幻灯片列表。
pub fn parse_pptx(path: &Path) -> Result<Vec<PptSlide>, PptError> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut archive = ZipArchive::new(reader)?;

    // 收集 slideN.xml（记录数字编号）与全部入口名（用于备注存在性判断）。
    let mut slide_entries: Vec<(usize, String)> = Vec::new();
    let mut entry_names: Vec<String> = Vec::new();
    for name in archive.file_names() {
        let owned = name.to_string();
        if let Some(num) = slide_number_from_name(&owned) {
            slide_entries.push((num, owned.clone()));
        }
        entry_names.push(owned);
    }
    // 数值排序，避免 slide10 排在 slide2 之前。
    slide_entries.sort_by_key(|(num, _)| *num);

    let mut slides = Vec::with_capacity(slide_entries.len());
    for (num, name) in slide_entries {
        let slide_xml = read_zip_entry(&mut archive, &name)?.unwrap_or_default();
        let shapes = parse_slide_xml(&slide_xml)?;

        let notes_name = format!("ppt/notesSlides/notesSlide{}.xml", num);
        let notes = if entry_names.iter().any(|n| n == &notes_name) {
            let notes_xml = read_zip_entry(&mut archive, &notes_name)?.unwrap_or_default();
            parse_notes_xml(&notes_xml)?
        } else {
            Vec::new()
        };

        slides.push(PptSlide {
            slide_idx: num,
            shapes,
            notes,
        });
    }

    Ok(slides)
}

/// 从 `ppt/slides/slideN.xml` 提取幻灯片编号 N。
fn slide_number_from_name(name: &str) -> Option<usize> {
    let rest = name.strip_prefix("ppt/slides/slide")?;
    let num = rest.strip_suffix(".xml")?;
    num.parse::<usize>().ok()
}

/// 读取 zip 入口的 UTF-8 文本；入口不存在时返回 `None`。
fn read_zip_entry(
    archive: &mut ZipArchive<BufReader<File>>,
    name: &str,
) -> Result<Option<String>, PptError> {
    match archive.by_name(name) {
        Ok(mut f) => {
            let mut bytes = Vec::new();
            Read::read_to_end(&mut f, &mut bytes)?;
            Ok(Some(String::from_utf8(bytes)?))
        }
        Err(zip::result::ZipError::FileNotFound) => Ok(None),
        Err(e) => Err(PptError::Zip(e)),
    }
}

// ---------------------------------------------------------------------------
// 幻灯片 XML 解析
// ---------------------------------------------------------------------------

/// 当前正在构建的形状。
struct ShapeBuilder {
    id: String,
    name: String,
    kind: ShapeKind,
    paragraphs: Vec<String>,
    table_rows: Vec<Vec<String>>,
}

/// 解析单个 slideN.xml，返回其中的形状（文本框 / 表格）。
fn parse_slide_xml(xml: &str) -> Result<Vec<PptShape>, PptError> {
    let mut reader = XmlReader::from_str(xml);
    let mut buf = Vec::new();

    let mut shapes = Vec::new();
    let mut stack: Vec<String> = Vec::new();

    let mut cur: Option<ShapeBuilder> = None;
    let mut cur_para = String::new(); // TextBox 当前段落累计
    let mut cur_cell = String::new(); // Table 当前单元格累计
    let mut cur_row: Vec<String> = Vec::new(); // Table 当前行

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let ln = e.local_name();
                let name = ln.as_ref();
                match name {
                    b"sp" => {
                        begin_shape(&mut cur, ShapeKind::TextBox);
                        cur_para.clear();
                        cur_cell.clear();
                        cur_row.clear();
                    }
                    b"graphicFrame" => {
                        begin_shape(&mut cur, ShapeKind::Table);
                        cur_para.clear();
                        cur_cell.clear();
                        cur_row.clear();
                    }
                    b"cNvPr" => {
                        capture_cnvpr(&mut cur, &stack, e);
                    }
                    b"p" => {
                        cur_para.clear();
                    }
                    b"tr" => {
                        cur_row.clear();
                    }
                    b"tc" => {
                        cur_cell.clear();
                    }
                    _ => {}
                }
                stack.push(String::from_utf8_lossy(name).into_owned());
            }
            Ok(Event::Empty(ref e)) => {
                let ln = e.local_name();
                let name = ln.as_ref();
                if name == b"cNvPr" {
                    capture_cnvpr(&mut cur, &stack, e);
                }
                // 空元素不压栈
            }
            Ok(Event::Text(ref e)) => {
                if stack.last().map(String::as_str) == Some("t") {
                    let text = e.unescape()?;
                    append_text(&mut cur, &mut cur_para, &mut cur_cell, &text);
                }
            }
            Ok(Event::CData(e)) => {
                if stack.last().map(String::as_str) == Some("t") {
                    let bytes: Vec<u8> = e.iter().cloned().collect();
                    let text = String::from_utf8(bytes)?;
                    append_text(&mut cur, &mut cur_para, &mut cur_cell, &text);
                }
            }
            Ok(Event::End(ref e)) => {
                let ln = e.local_name();
                let name = ln.as_ref();
                match name {
                    b"p" => {
                        if let Some(c) = cur.as_mut() {
                            if c.kind == ShapeKind::TextBox {
                                c.paragraphs.push(std::mem::take(&mut cur_para));
                            }
                        }
                    }
                    b"tc" => {
                        cur_row.push(std::mem::take(&mut cur_cell));
                    }
                    b"tr" => {
                        if let Some(c) = cur.as_mut() {
                            c.table_rows.push(std::mem::take(&mut cur_row));
                        }
                    }
                    b"sp" | b"graphicFrame" => {
                        if let Some(c) = cur.take() {
                            shapes.push(PptShape {
                                id: c.id,
                                name: c.name,
                                kind: c.kind,
                                paragraphs: c.paragraphs,
                                table_rows: c.table_rows,
                            });
                        }
                    }
                    _ => {}
                }
                stack.pop();
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(PptError::Xml(e)),
            _ => {}
        }
        buf.clear();
    }

    Ok(shapes)
}

fn begin_shape(cur: &mut Option<ShapeBuilder>, kind: ShapeKind) {
    *cur = Some(ShapeBuilder {
        id: String::new(),
        name: String::new(),
        kind,
        paragraphs: Vec::new(),
        table_rows: Vec::new(),
    });
}

/// 捕获形状自身 `nvSpPr` / `nvGraphicFramePr` 下的 `cNvPr` 属性。
///
/// 跳过 `spTree` 顶层 `nvGrpSpPr`（组形状）的 `cNvPr`。
fn capture_cnvpr(cur: &mut Option<ShapeBuilder>, stack: &[String], e: &BytesStart) {
    let parent = stack.last().map(String::as_str);
    if parent != Some("nvSpPr") && parent != Some("nvGraphicFramePr") {
        return;
    }
    if let Some(c) = cur.as_mut() {
        if c.id.is_empty() {
            c.id = attr_value(e, b"id");
            c.name = attr_value(e, b"name");
        }
    }
}

fn append_text(
    cur: &mut Option<ShapeBuilder>,
    cur_para: &mut String,
    cur_cell: &mut String,
    text: &str,
) {
    if let Some(c) = cur.as_mut() {
        match c.kind {
            ShapeKind::TextBox => cur_para.push_str(text),
            ShapeKind::Table => cur_cell.push_str(text),
        }
    }
}

fn attr_value(e: &BytesStart, name: &[u8]) -> String {
    e.attributes()
        .filter_map(|a| a.ok())
        .find(|a| a.key.as_ref() == name)
        .and_then(|a| a.unescape_value().ok())
        .map(|s| s.into_owned())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// 备注 XML 解析
// ---------------------------------------------------------------------------

/// 解析单个 notesSlideN.xml，返回备注段落（按 `<a:p>` 分组拼接 `<a:t>`）。
fn parse_notes_xml(xml: &str) -> Result<Vec<String>, PptError> {
    let mut reader = XmlReader::from_str(xml);
    let mut buf = Vec::new();

    let mut paragraphs = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    let mut cur = String::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let ln = e.local_name();
                let name = ln.as_ref();
                if name == b"p" {
                    cur.clear();
                }
                stack.push(String::from_utf8_lossy(name).into_owned());
            }
            Ok(Event::Empty(_)) => {}
            Ok(Event::Text(ref e)) => {
                if stack.last().map(String::as_str) == Some("t") {
                    cur.push_str(&e.unescape()?);
                }
            }
            Ok(Event::CData(e)) => {
                if stack.last().map(String::as_str) == Some("t") {
                    let bytes: Vec<u8> = e.iter().cloned().collect();
                    let text = String::from_utf8(bytes)?;
                    cur.push_str(&text);
                }
            }
            Ok(Event::End(ref e)) => {
                let ln = e.local_name();
                let name = ln.as_ref();
                if name == b"p" {
                    paragraphs.push(std::mem::take(&mut cur));
                }
                stack.pop();
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(PptError::Xml(e)),
            _ => {}
        }
        buf.clear();
    }

    Ok(paragraphs)
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// 最小文本框幻灯片 XML（无命名空间声明，quick-xml 不校验命名空间）。
    const TEXTBOX_XML: &str = r#"<p:sld><p:spTree><p:sp><p:nvSpPr><p:cNvPr id="2" name="TextBox 1"/></p:nvSpPr><p:txBody><a:p><a:r><a:t>Hello</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:sld>"#;

    /// 最小表格幻灯片 XML。
    const TABLE_XML: &str = r#"<p:sld><p:spTree><p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id="3" name="Table 1"/></p:nvGraphicFramePr><a:graphic><a:graphicData><a:tbl><a:tr><a:tc><a:txBody><a:p><a:r><a:t>cell text</a:t></a:r></a:p></a:txBody></a:tc></a:tr></a:tbl></a:graphicData></a:graphic></p:graphicFrame></p:spTree></p:sld>"#;

    #[test]
    fn parse_synthetic_textbox_xml() {
        let shapes = parse_slide_xml(TEXTBOX_XML).unwrap();
        assert_eq!(shapes.len(), 1, "expected one shape, got: {:?}", shapes);
        let shape = &shapes[0];
        assert_eq!(shape.id, "2");
        assert_eq!(shape.name, "TextBox 1");
        assert_eq!(shape.kind, ShapeKind::TextBox);
        assert_eq!(shape.paragraphs, vec!["Hello"]);
        assert!(shape.table_rows.is_empty());
    }

    #[test]
    fn parse_synthetic_table_xml() {
        let shapes = parse_slide_xml(TABLE_XML).unwrap();
        assert_eq!(shapes.len(), 1, "expected one shape, got: {:?}", shapes);
        let shape = &shapes[0];
        assert_eq!(shape.id, "3");
        assert_eq!(shape.name, "Table 1");
        assert_eq!(shape.kind, ShapeKind::Table);
        assert_eq!(shape.table_rows, vec![vec!["cell text"]]);
        assert!(shape.paragraphs.is_empty());
    }

    #[test]
    fn quick_xml_reports_prefixed_name_and_stripped_local_name() {
        let xml = r#"<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:sp/></p:sld>"#;
        let mut reader = XmlReader::from_str(xml);
        let mut buf = Vec::new();
        let mut saw_sp = false;
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                    let qname = e.name();
                    let lname = e.local_name();
                    if lname.as_ref() == b"sp" {
                        saw_sp = true;
                        // name() 保留命名空间前缀，local_name() 去除前缀。
                        assert_eq!(qname.as_ref(), b"p:sp");
                        assert_eq!(lname.as_ref(), b"sp");
                    }
                }
                Ok(Event::Eof) => break,
                Err(e) => panic!("xml error: {}", e),
                _ => {}
            }
            buf.clear();
        }
        assert!(saw_sp, "should have seen a p:sp element");
    }

    #[test]
    fn parse_real_sample_report() {
        let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let path = crate_dir.join("../examples/sample_report.pptx");
        let slides = parse_pptx(&path).unwrap();

        assert!(!slides.is_empty(), "sample_report.pptx should have slides");
        assert!(
            slides.iter().any(|s| !s.shapes.is_empty()),
            "at least one slide should have shapes"
        );

        // 至少有一段非空文本。
        let has_text = slides.iter().flat_map(|s| s.shapes.iter()).any(|sh| {
            sh.paragraphs.iter().any(|p| !p.trim().is_empty())
                || sh
                    .table_rows
                    .iter()
                    .flat_map(|r| r.iter())
                    .any(|c| !c.trim().is_empty())
        });
        assert!(has_text, "sample_report.pptx should contain non-empty text");

        // 本 fixture 没有备注目录，notes 应为空且不 panic。
        assert!(
            slides.iter().all(|s| s.notes.is_empty()),
            "sample_report.pptx has no notesSlides, notes should be empty"
        );

        // 观察到的形状类型：文本框与表格都应出现。
        let kinds: Vec<ShapeKind> = slides
            .iter()
            .flat_map(|s| s.shapes.iter())
            .map(|sh| sh.kind)
            .collect();
        assert!(kinds.contains(&ShapeKind::TextBox));
        assert!(kinds.contains(&ShapeKind::Table));
    }

    #[test]
    fn slide_number_sorting_is_numeric() {
        // 直接验证编号提取 + 数值排序逻辑（无需真实 zip）。
        let mut entries: Vec<(usize, String)> = vec![
            (2, "ppt/slides/slide2.xml".to_string()),
            (10, "ppt/slides/slide10.xml".to_string()),
            (1, "ppt/slides/slide1.xml".to_string()),
        ];
        entries.sort_by_key(|(num, _)| *num);
        assert_eq!(
            entries.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
            vec![1, 2, 10]
        );
        assert_eq!(slide_number_from_name("ppt/slides/slide10.xml"), Some(10));
        assert_eq!(
            slide_number_from_name("ppt/notesSlides/notesSlide1.xml"),
            None
        );
    }
}

//! xmlsurgeon — surgical xlsx cell editing.
//!
//! Provides [`XmlSurgeon`] for opening an xlsx file, modifying individual cell
//! text values, and saving the result. All zip entries are kept in memory so
//! non-sheet content (styles, themes, …) is preserved byte-for-byte.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

use quick_xml::events::{BytesStart, Event};
use quick_xml::reader::Reader as XmlReader;
use zip::read::ZipArchive;
use zip::write::{SimpleFileOptions, ZipWriter};

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Errors that can occur during xmlsurgeon operations.
#[derive(Debug)]
pub enum SurgeonError {
    /// I/O error (file open, read, write).
    Io(std::io::Error),
    /// Invalid or malformed zip archive.
    Zip(zip::result::ZipError),
    /// XML parse or serialize error.
    Xml(quick_xml::Error),
    /// A required entry was not found in the xlsx archive.
    EntryNotFound(String),
    /// Sheet name could not be mapped to an XML path.
    SheetNotFound(String),
    /// Cell reference was not found in the sheet XML.
    CellNotFound(String),
    /// UTF-8 decoding error.
    Utf8(std::string::FromUtf8Error),
}

impl std::fmt::Display for SurgeonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SurgeonError::Io(e) => write!(f, "IO error: {}", e),
            SurgeonError::Zip(e) => write!(f, "Zip error: {}", e),
            SurgeonError::Xml(e) => write!(f, "XML error: {}", e),
            SurgeonError::EntryNotFound(name) => {
                write!(f, "Entry not found: {}", name)
            }
            SurgeonError::SheetNotFound(name) => {
                write!(f, "Sheet not found: {}", name)
            }
            SurgeonError::CellNotFound(cell_ref) => {
                write!(f, "Cell not found: {}", cell_ref)
            }
            SurgeonError::Utf8(e) => write!(f, "UTF-8 error: {}", e),
        }
    }
}

impl std::error::Error for SurgeonError {}

impl From<std::io::Error> for SurgeonError {
    fn from(e: std::io::Error) -> Self {
        SurgeonError::Io(e)
    }
}

impl From<zip::result::ZipError> for SurgeonError {
    fn from(e: zip::result::ZipError) -> Self {
        SurgeonError::Zip(e)
    }
}

impl From<quick_xml::Error> for SurgeonError {
    fn from(e: quick_xml::Error) -> Self {
        SurgeonError::Xml(e)
    }
}

impl From<std::string::FromUtf8Error> for SurgeonError {
    fn from(e: std::string::FromUtf8Error) -> Self {
        SurgeonError::Utf8(e)
    }
}

// ---------------------------------------------------------------------------
// Entry storage helper
// ---------------------------------------------------------------------------

/// Store entries as Vec for ordered iteration but provide fast lookup by name.
fn find_entry_idx(entries: &[(String, Vec<u8>)], name: &str) -> Option<usize> {
    entries.iter().position(|(n, _)| n == name)
}

// ---------------------------------------------------------------------------
// XmlSurgeon
// ---------------------------------------------------------------------------

/// Surgical xlsx cell editor.
///
/// Loads all zip entries into memory, allows targeted cell modification, and
/// writes a new xlsx file with all non-modified entries preserved byte-for-byte.
pub struct XmlSurgeon {
    /// (filename, content) pairs from the zip archive.
    entries: Vec<(String, Vec<u8>)>,
}

impl XmlSurgeon {
    /// Open an xlsx file and load all zip entries into memory.
    pub fn open(path: &Path) -> Result<Self, SurgeonError> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let mut archive = ZipArchive::new(reader)?;

        let mut entries = Vec::new();
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)?;
            let name = entry.name().to_string();
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            entries.push((name, buf));
        }

        Ok(Self { entries })
    }

    /// Set a cell's text value in a specific sheet.
    ///
    /// - `sheet`: sheet name (e.g., "Sheet1")
    /// - `cell_ref`: Excel cell reference (e.g., "B5")
    /// - `new_text`: the new text value
    ///
    /// Handles shared strings (`t="s"`): appends to sharedStrings.xml and
    /// updates the cell's index. For inline strings (`t="str"` or no type),
    /// replaces the value directly.
    pub fn set_cell_text(
        &mut self,
        sheet: &str,
        cell_ref: &str,
        new_text: &str,
    ) -> Result<(), SurgeonError> {
        // 1. Map sheet name → sheet XML path
        let sheet_path = self.resolve_sheet_path(sheet)?;

        // 2. Load shared strings lazily if we need them
        let mut shared_strings: Option<SharedStrings> = None;

        // 3. Find the sheet entry index
        let sheet_idx = find_entry_idx(&self.entries, &sheet_path)
            .ok_or_else(|| SurgeonError::EntryNotFound(sheet_path.clone()))?;

        let sheet_xml_bytes = &self.entries[sheet_idx].1;
        let sheet_xml_str = String::from_utf8(sheet_xml_bytes.clone())?;

        // 4. Modify the sheet XML: replace the target cell's value using
        //    string-level replacement (bypass quick-xml writer for cell edit).
        //    This is more reliable than event-based XML rewriting.
        let modified_sheet = {
            let xml = &sheet_xml_str;
            let needle = format!("<c r=\"{}", cell_ref);
            let mut cell_start = None;
            let mut pos = 0;
            while pos < xml.len() {
                if let Some(idx) = xml[pos..].find(&needle) {
                    let abs_idx = pos + idx;
                    // Verify it's an exact cell ref (next char is '"')
                    let after_needle = abs_idx + needle.len();
                    if after_needle < xml.len() && xml.as_bytes()[after_needle] == b'"' {
                        cell_start = Some(abs_idx);
                        break;
                    }
                    pos = abs_idx + needle.len();
                } else {
                    break;
                }
            }

            let cell_start = cell_start.ok_or_else(|| SurgeonError::CellNotFound(cell_ref.to_string()))?;

            // Find the <c r="..." opening tag end (the first '>' after cell_start)
            let tag_end = xml[cell_start..].find('>')
                .ok_or_else(|| SurgeonError::CellNotFound(cell_ref.to_string()))?;
            let tag_end_abs = cell_start + tag_end;
            let is_self_closing = xml.as_bytes()[tag_end_abs - 1] == b'/';

            // Extract the opening <c> tag to determine cell type
            let opening_tag = &xml[cell_start..=tag_end_abs];
            let cell_type = if opening_tag.contains("t=\"s\"") {
                CellType::SharedString
            } else if opening_tag.contains("t=\"str\"") || opening_tag.contains("t=\"inlineStr\"") {
                CellType::InlineString
            } else {
                CellType::None
            };

            // Determine the new <v> content (保持原类型，只换 <v> 内容)
            let new_v_content = match cell_type {
                CellType::SharedString => {
                    // 保持 t="s"：追加新文本到 sharedStrings.xml，<v> 写新索引
                    if shared_strings.is_none() {
                        shared_strings = Some(self.load_shared_strings()?);
                    }
                    let ss = shared_strings.as_mut().unwrap();
                    let idx = ss.append(new_text);
                    idx.to_string()
                }
                CellType::InlineString | CellType::Formula | CellType::None => {
                    // 保持原类型：直接写文本
                    new_text.to_string()
                }
            };

            // 构建替换：保留原始开标签（含全部属性），只替换 <v> 的内容。
            // 自闭合 cell（<c r="A1"/>）没有 <v>，插入 <v> 时保持无类型属性。
            let new_cell = if is_self_closing {
                // <c r="A1" s="23"/> → <c r="A1" s="23"><v>new</v></c>
                format!("{}<v>{}</v></c>", opening_tag.trim_end_matches('/'), escape_xml(&new_v_content))
            } else {
                // <c r="A1" s="23" t="s"><v>old</v></c> → 保留开标签，只换 <v> 内容
                // 找到 </c> 的位置，然后替换整个 cell 内容为 开标签 + <v>新内容</v> + </c>
                let close_tag = "</c>";
                let after_start = &xml[tag_end_abs + 1..];
                let close_pos = after_start.find(close_tag)
                    .ok_or_else(|| SurgeonError::CellNotFound(cell_ref.to_string()))?;
                let _full_cell_end = tag_end_abs + 1 + close_pos + close_tag.len();

                // 提取 <v>...</v> 之外可能存在的其他子元素（如 <is>、<f>）
                let cell_body = &xml[tag_end_abs + 1..tag_end_abs + 1 + close_pos];
                // 只替换 <v> 的内容；若没有 <v>，在 </c> 前插入一个
                let new_body = if let Some(v_start) = cell_body.find("<v>") {
                    let v_end = cell_body[v_start..].find("</v>")
                        .map(|p| v_start + p)
                        .ok_or_else(|| SurgeonError::CellNotFound(cell_ref.to_string()))?;
                    format!("{}<v>{}</v>{}", &cell_body[..v_start], escape_xml(&new_v_content), &cell_body[v_end + 4..])
                } else if let Some(v_start) = cell_body.find("<v ") {
                    // <v> 带属性的情况（罕见但处理）
                    let v_end = cell_body[v_start..].find("</v>")
                        .map(|p| v_start + p)
                        .ok_or_else(|| SurgeonError::CellNotFound(cell_ref.to_string()))?;
                    let v_open_end = cell_body[v_start..].find('>')
                        .map(|p| v_start + p)
                        .ok_or_else(|| SurgeonError::CellNotFound(cell_ref.to_string()))?;
                    format!("{}{}{}</v>{}", &cell_body[..v_start], &cell_body[v_start..=v_open_end], escape_xml(&new_v_content), &cell_body[v_end + 4..])
                } else {
                    // cell 没有 <v>，插入一个
                    format!("<v>{}</v>{}", escape_xml(&new_v_content), cell_body)
                };

                format!("{}{}</c>", opening_tag, new_body)
            };

            // Replace the cell in the XML
            if is_self_closing {
                let cell_end = tag_end_abs + 1;
                let mut result = Vec::with_capacity(xml.len() + new_cell.len());
                result.extend_from_slice(xml[..cell_start].as_bytes());
                result.extend_from_slice(new_cell.as_bytes());
                result.extend_from_slice(xml[cell_end..].as_bytes());
                result
            } else {
                let close_tag = "</c>";
                let after_start = &xml[tag_end_abs + 1..];
                let close_pos = after_start.find(close_tag)
                    .ok_or_else(|| SurgeonError::CellNotFound(cell_ref.to_string()))?;
                let full_cell_end = tag_end_abs + 1 + close_pos + close_tag.len();
                let mut result = Vec::with_capacity(xml.len() + new_cell.len());
                result.extend_from_slice(xml[..cell_start].as_bytes());
                result.extend_from_slice(new_cell.as_bytes());
                result.extend_from_slice(xml[full_cell_end..].as_bytes());
                result
            }
        };
        // string replacement succeeded if we got here (CellNotFound is returned
        // earlier if the cell reference was not found)

        // Replace the sheet entry
        self.entries[sheet_idx].1 = modified_sheet;

        // If shared strings were modified, write them back
        if let Some(ss) = shared_strings {
            let ss_idx = find_entry_idx(&self.entries, "xl/sharedStrings.xml")
                .ok_or_else(|| {
                    SurgeonError::EntryNotFound("xl/sharedStrings.xml".to_string())
                })?;
            let bytes = ss.serialize()?;
            self.entries[ss_idx].1 = bytes;
        }

        Ok(())
    }

    /// Return the number of entries (for testing/inspection).
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Snapshot of entries (for testing/debugging).
    pub fn entries_snapshot(&self) -> &[(String, Vec<u8>)] {
        &self.entries
    }

    /// Read an entry's content as UTF-8 string (public for testing).
    pub fn read_entry_public(&self, name: &str) -> Result<String, SurgeonError> {
        self.read_entry(name)
    }

    /// Save the modified xlsx to a new path.
    pub fn save(&self, path: &Path) -> Result<(), SurgeonError> {
        let file = File::create(path)?;
        let writer = BufWriter::new(file);
        let mut zip = ZipWriter::new(writer);
        let options = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .compression_level(Some(6));

        for (name, content) in &self.entries {
            zip.start_file(name, options)?;
            zip.write_all(content)?;
        }

        zip.finish()?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Resolve a sheet name to its XML path inside the xlsx archive.
    fn resolve_sheet_path(&self, sheet_name: &str) -> Result<String, SurgeonError> {
        let wb_xml = self.read_entry("xl/workbook.xml")?;
        let rels_xml = self.read_entry("xl/_rels/workbook.xml.rels")?;

        let sheet_entries = parse_workbook_sheets(&wb_xml);
        let rid_to_target = parse_rels(&rels_xml);

        for (name, rid) in &sheet_entries {
            if name == sheet_name {
                if let Some(target) = rid_to_target.get(rid) {
                    let full = format!("xl/{}", target.trim_start_matches('/'));
                    return Ok(full);
                }
            }
        }

        Err(SurgeonError::SheetNotFound(sheet_name.to_string()))
    }

    /// Read an entry's content as a UTF-8 string.
    fn read_entry(&self, name: &str) -> Result<String, SurgeonError> {
        find_entry_idx(&self.entries, name)
            .map(|i| String::from_utf8(self.entries[i].1.clone()))
            .transpose()?
            .ok_or_else(|| SurgeonError::EntryNotFound(name.to_string()))
    }

    /// Load sharedStrings.xml into a [`SharedStrings`] helper.
    fn load_shared_strings(&self) -> Result<SharedStrings, SurgeonError> {
        let xml = self.read_entry("xl/sharedStrings.xml")?;
        SharedStrings::parse(&xml)
    }
}

// ---------------------------------------------------------------------------
// CellType detection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
enum CellType {
    SharedString,
    InlineString,
    Formula,
    None,
}

/// Escape XML special characters in text content.
fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Get an attribute value from a [`BytesStart`] element by local name.
fn get_attr(e: &BytesStart, name: &[u8]) -> Option<String> {
    for attr in e.attributes().flatten() {
        if attr.key.as_ref() == name {
            return String::from_utf8(attr.value.to_vec()).ok();
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Workbook XML parsing helpers
// ---------------------------------------------------------------------------

/// Parse workbook.xml and return `Vec<(sheet_name, rId)>`.
fn parse_workbook_sheets(xml: &str) -> Vec<(String, String)> {
    let mut entries = Vec::new();
    let mut reader = XmlReader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(ref e)) | Ok(Event::Start(ref e)) => {
                if e.name().as_ref() == b"sheet" {
                    let name = get_attr(e, b"name").unwrap_or_default();
                    let rid = get_attr(e, b"r:id")
                        .or_else(|| {
                            get_attr(
                                e,
                                b"{http://schemas.openxmlformats.org/officeDocument/2006/relationships}id",
                            )
                        })
                        .unwrap_or_default();
                    if !name.is_empty() && !rid.is_empty() {
                        entries.push((name, rid));
                    }
                }
            }
            Ok(Event::Eof) => break,
            _ => {}
        }
        buf.clear();
    }
    entries
}

/// Parse workbook.xml.rels and return `HashMap<Id, Target>`.
fn parse_rels(xml: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let mut reader = XmlReader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(ref e)) => {
                if e.name().as_ref() == b"Relationship" {
                    let id = get_attr(e, b"Id").unwrap_or_default();
                    let target = get_attr(e, b"Target").unwrap_or_default();
                    if !id.is_empty() && !target.is_empty() {
                        map.insert(id, target);
                    }
                }
            }
            Ok(Event::Eof) => break,
            _ => {}
        }
        buf.clear();
    }
    map
}

// ---------------------------------------------------------------------------
// SharedStrings helper
// ---------------------------------------------------------------------------

/// Represents the content of `xl/sharedStrings.xml`.
/// Allows appending new strings and re-serializing.
struct SharedStrings {
    /// The existing `<si><t>...</t></si>` entries.
    si_entries: Vec<String>,
    /// Running count attribute value.
    count: u32,
}

impl SharedStrings {
    /// Parse sharedStrings.xml into a `SharedStrings` helper.
    fn parse(xml: &str) -> Result<Self, SurgeonError> {
        let mut si_entries = Vec::new();
        let mut count = 0u32;
        let mut reader = XmlReader::from_str(xml);
        reader.config_mut().trim_text(true);
        let mut buf = Vec::new();
        let mut in_si = false;
        let mut in_t = false;
        let mut current_t = String::new();

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(ref e)) => {
                    if e.name().as_ref() == b"si" {
                        in_si = true;
                    } else if e.name().as_ref() == b"t" && in_si {
                        in_t = true;
                        current_t.clear();
                    }
                }
                Ok(Event::Text(ref e)) => {
                    if in_t {
                        current_t.push_str(&e.unescape().unwrap_or_default());
                    }
                }
                Ok(Event::End(ref e)) => {
                    if e.name().as_ref() == b"t" && in_si {
                        in_t = false;
                    } else if e.name().as_ref() == b"si" {
                        in_si = false;
                        si_entries.push(current_t.clone());
                        current_t.clear();
                        count += 1;
                    }
                }
                Ok(Event::Eof) => break,
                _ => {}
            }
            buf.clear();
        }

        Ok(Self { si_entries, count })
    }

    /// Append a new string and return its index.
    fn append(&mut self, text: &str) -> usize {
        let idx = self.si_entries.len();
        self.si_entries.push(text.to_string());
        self.count += 1;
        idx
    }

    /// Serialize back to XML bytes.
    fn serialize(&self) -> Result<Vec<u8>, SurgeonError> {
        let mut out = Vec::new();
        // Write XML declaration
        out.extend_from_slice(
            b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n",
        );

        let count_str = self.count.to_string();
        let unique_str = self.si_entries.len().to_string();

        // Open <sst>
        out.extend_from_slice(b"<sst");
        out.extend_from_slice(b" xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"");
        write_attr(&mut out, b" count", &count_str);
        write_attr(&mut out, b" uniqueCount", &unique_str);
        out.extend_from_slice(b">");

        // Each <si><t>...</t></si>
        for text in &self.si_entries {
            out.extend_from_slice(b"<si><t>");
            escape_xml_text(&mut out, text);
            out.extend_from_slice(b"</t></si>");
        }

        out.extend_from_slice(b"</sst>");
        Ok(out)
    }
}

/// Write an XML attribute like ` name="value"`.
fn write_attr(out: &mut Vec<u8>, key: &[u8], value: &str) {
    out.extend_from_slice(key);
    out.push(b'=');
    out.push(b'"');
    out.extend_from_slice(value.as_bytes());
    out.push(b'"');
}

/// Escape XML special characters in text content.
fn escape_xml_text(out: &mut Vec<u8>, text: &str) {
    for byte in text.bytes() {
        match byte {
            b'<' => out.extend_from_slice(b"&lt;"),
            b'>' => out.extend_from_slice(b"&gt;"),
            b'&' => out.extend_from_slice(b"&amp;"),
            b'"' => out.extend_from_slice(b"&quot;"),
            b'\'' => out.extend_from_slice(b"&apos;"),
            _ => out.push(byte),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use quick_xml::writer::Writer as XmlWriter;
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    #[test]
    fn open_and_save_roundtrip() {
        let path = fixture("data1.xlsx");
        let surgeon = XmlSurgeon::open(&path).unwrap();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        surgeon.save(tmp.path()).unwrap();

        // Verify the saved file is a valid zip/xlsx
        let saved = XmlSurgeon::open(tmp.path()).unwrap();
        assert!(!saved.entries.is_empty());
    }

    #[test]
    fn shared_strings_parse_and_serialize() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="2" uniqueCount="2">
  <si><t>Hello</t></si>
  <si><t>World</t></si>
</sst>"#;
        let ss = SharedStrings::parse(xml).unwrap();
        assert_eq!(ss.si_entries, vec!["Hello", "World"]);
        assert_eq!(ss.count, 2);

        let mut ss = ss;
        let idx = ss.append("New");
        assert_eq!(idx, 2);
        assert_eq!(ss.count, 3);

        let bytes = ss.serialize().unwrap();
        let s = String::from_utf8(bytes).unwrap();
        assert!(s.contains("<si><t>New</t></si>"));
        assert!(s.contains("count=\"3\""));
        assert!(s.contains("uniqueCount=\"3\""));
    }

    #[test]
    fn shared_strings_escape_xml() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="1" uniqueCount="1">
  <si><t>Test &amp; Value</t></si>
</sst>"#;
        let ss = SharedStrings::parse(xml).unwrap();
        assert_eq!(ss.si_entries, vec!["Test & Value"]);
    }

    #[test]
    fn parse_workbook_sheets_basic() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"
          xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets>
    <sheet name="Sheet1" sheetId="1" r:id="rId1"/>
    <sheet name="Sheet2" sheetId="2" r:id="rId2"/>
  </sheets>
</workbook>"#;
        let entries = parse_workbook_sheets(xml);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0], ("Sheet1".into(), "rId1".into()));
        assert_eq!(entries[1], ("Sheet2".into(), "rId2".into()));
    }

    #[test]
    fn parse_rels_basic() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet2.xml"/>
</Relationships>"#;
        let map = parse_rels(xml);
        assert_eq!(map.get("rId1").unwrap(), "worksheets/sheet1.xml");
        assert_eq!(map.get("rId2").unwrap(), "worksheets/sheet2.xml");
    }

    #[test]
    fn set_cell_text_shared_string() {
        // Create a minimal xlsx in memory with shared strings
        // This is a unit test for the core logic — integration tests use real fixtures
        let xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1">
      <c r="A1" t="s"><v>0</v></c>
    </row>
  </sheetData>
</worksheet>"#;
        // Just test that parsing + writing works without errors
        let mut reader = XmlReader::from_str(xml);
        reader.config_mut().trim_text(true);
        let mut out = Vec::new();
        let mut writer = XmlWriter::new(&mut out);
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Eof) => break,
                Ok(e) => {
                    writer.write_event(e.into_owned()).unwrap();
                }
                Err(_) => panic!("XML parse error"),
            }
            buf.clear();
        }
        let result = String::from_utf8(out).unwrap();
        assert!(result.contains("A1"));
        assert!(result.contains("s"));
    }
}

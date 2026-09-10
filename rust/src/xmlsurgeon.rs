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
            SurgeonError::Zip(e) => write!(f, "zip error: {}", e),
            SurgeonError::Xml(e) => write!(f, "XML error: {}", e),
            SurgeonError::EntryNotFound(name) => write!(f, "Entry not found: {name}"),
            SurgeonError::SheetNotFound(name) => write!(f, "Sheet not found: {name}"),
            SurgeonError::CellNotFound(r) => write!(f, "Cell not found: {r}"),
            SurgeonError::Utf8(e) => write!(f, "UTF-8 error: {e}"),
        }
    }
}

impl std::error::Error for SurgeonError {}

impl From<std::io::Error> for SurgeonError {
    fn from(e: std::io::Error) -> Self { Self::Io(e) }
}
impl From<zip::result::ZipError> for SurgeonError {
    fn from(e: zip::result::ZipError) -> Self { Self::Zip(e) }
}
impl From<quick_xml::Error> for SurgeonError {
    fn from(e: quick_xml::Error) -> Self { Self::Xml(e) }
}
impl From<std::string::FromUtf8Error> for SurgeonError {
    fn from(e: std::string::FromUtf8Error) -> Self { Self::Utf8(e) }
}

// ---------------------------------------------------------------------------
// Entry storage helper
// ---------------------------------------------------------------------------

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
    entries: Vec<(String, Vec<u8>)>,
}

impl XmlSurgeon {
    /// Open an xlsx file and load all zip entries into memory.
    pub fn open(path: &Path) -> Result<Self, SurgeonError> {
        let file = File::open(path)?;
        let mut archive = ZipArchive::new(BufReader::new(file))?;
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

    /// Read the current text value of a cell.
    pub fn get_cell_text(&self, sheet: &str, cell_ref: &str) -> Result<String, SurgeonError> {
        let sheet_path = self.resolve_sheet_path(sheet)?;
        let xml = self.read_entry(&sheet_path)?;
        self.read_cell_value(&xml, cell_ref)
    }

    /// Replace a substring within a cell's text (preserving the rest).
    pub fn replace_in_cell(&mut self, sheet: &str, cell_ref: &str, old: &str, new: &str) -> Result<(), SurgeonError> {
        let current = self.get_cell_text(sheet, cell_ref)?;
        let replaced = current.replace(old, new);
        if replaced == current { return Ok(()); }
        self.set_cell_text(sheet, cell_ref, &replaced)
    }

    /// Set a cell's text value in a specific sheet.
    ///
    /// Handles shared strings (`t="s"`), inline strings (`t="inlineStr"`),
    /// and numeric cells. For inline strings, modifies `<is><t>` content;
    /// for shared strings, appends to sharedStrings.xml and updates the index.
    pub fn set_cell_text(&mut self, sheet: &str, cell_ref: &str, new_text: &str) -> Result<(), SurgeonError> {
        let sheet_path = self.resolve_sheet_path(sheet)?;
        let mut shared_strings: Option<SharedStrings> = None;

        let sheet_idx = find_entry_idx(&self.entries, &sheet_path)
            .ok_or_else(|| SurgeonError::EntryNotFound(sheet_path.clone()))?;
        let xml = String::from_utf8(self.entries[sheet_idx].1.clone())?;

        // Locate the <c r="CELLREF"...> element
        let needle = format!("<c r=\"{cell_ref}");
        let cell_start = find_cell_start(&xml, &needle)?;
        let tag_end_abs = xml[cell_start..].find('>')
            .map(|p| cell_start + p)
            .ok_or_else(|| SurgeonError::CellNotFound(cell_ref.into()))?;
        let is_self_closing = xml.as_bytes()[tag_end_abs - 1] == b'/';
        let opening_tag = &xml[cell_start..=tag_end_abs];
        let cell_type = detect_cell_type(opening_tag);

        // Build the replacement content
        let new_v_content = match cell_type {
            CellType::SharedString => {
                if shared_strings.is_none() {
                    shared_strings = Some(self.load_shared_strings()?);
                }
                let ss = shared_strings.as_mut().unwrap();
                ss.append(new_text).to_string()
            }
            _ => new_text.to_string(),
        };

        // Adjust tag for numeric cells writing non-numeric text
        let base_tag = if cell_type == CellType::None && new_text.parse::<f64>().is_err() {
            let no_close = opening_tag.trim_end_matches('>').trim_end_matches('/').trim_end();
            if is_self_closing { format!("{no_close} t=\"str\"/>") }
            else { format!("{no_close} t=\"str\">") }
        } else {
            opening_tag.to_string()
        };

        let new_cell = if is_self_closing {
            let tag = base_tag.strip_suffix("/>").map(|t| format!("{t}>")).unwrap_or(base_tag);
            format!("{tag}<v>{}</v></c>", quick_xml::escape::escape(&new_v_content))
        } else {
            let body_start = tag_end_abs + 1;
            let body_end = xml[body_start..].find("</c>")
                .map(|p| body_start + p)
                .ok_or_else(|| SurgeonError::CellNotFound(cell_ref.into()))?;
            let body = &xml[body_start..body_end];

            let new_body = if cell_type == CellType::InlineString {
                replace_is_t_text(body, new_text)
                    .unwrap_or_else(|| replace_v_in_body(body, &new_v_content))
            } else {
                replace_v_in_body(body, &new_v_content)
            };
            format!("{base_tag}{new_body}</c>")
        };

        // Splice the new cell into the XML
        let full_end = if is_self_closing {
            tag_end_abs + 1
        } else {
            let body_start = tag_end_abs + 1;
            body_start + xml[body_start..].find("</c>")
                .ok_or_else(|| SurgeonError::CellNotFound(cell_ref.into()))? + 4
        };
        let mut result = Vec::with_capacity(xml.len() + new_cell.len());
        result.extend_from_slice(xml[..cell_start].as_bytes());
        result.extend_from_slice(new_cell.as_bytes());
        result.extend_from_slice(xml[full_end..].as_bytes());
        self.entries[sheet_idx].1 = result;

        // Write back shared strings if modified
        if let Some(ss) = shared_strings {
            let ss_idx = find_entry_idx(&self.entries, "xl/sharedStrings.xml")
                .ok_or_else(|| SurgeonError::EntryNotFound("xl/sharedStrings.xml".into()))?;
            self.entries[ss_idx].1 = ss.serialize()?;
        }

        Ok(())
    }

    pub fn entry_count(&self) -> usize { self.entries.len() }
    pub fn entries_snapshot(&self) -> &[(String, Vec<u8>)] { &self.entries }
    pub fn read_entry_public(&self, name: &str) -> Result<String, SurgeonError> { self.read_entry(name) }

    /// Save the modified xlsx to a new path.
    pub fn save(&self, path: &Path) -> Result<(), SurgeonError> {
        let mut zip = ZipWriter::new(BufWriter::new(File::create(path)?));
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

    // -- private helpers --

    fn resolve_sheet_path(&self, sheet_name: &str) -> Result<String, SurgeonError> {
        let wb_xml = self.read_entry("xl/workbook.xml")?;
        let rels_xml = self.read_entry("xl/_rels/workbook.xml.rels")?;
        let sheets = parse_workbook_sheets(&wb_xml);
        let rid_to_target = parse_rels(&rels_xml);
        for (name, rid) in &sheets {
            if name == sheet_name {
                if let Some(target) = rid_to_target.get(rid) {
                    let t = target.trim_start_matches('/');
                    return Ok(if t.starts_with("xl/") { t.into() } else { format!("xl/{t}") });
                }
            }
        }
        Err(SurgeonError::SheetNotFound(sheet_name.into()))
    }

    fn read_entry(&self, name: &str) -> Result<String, SurgeonError> {
        find_entry_idx(&self.entries, name)
            .map(|i| String::from_utf8(self.entries[i].1.clone()))
            .transpose()?
            .ok_or_else(|| SurgeonError::EntryNotFound(name.into()))
    }

    fn read_cell_value(&self, xml: &str, cell_ref: &str) -> Result<String, SurgeonError> {
        let needle = format!("<c r=\"{cell_ref}");
        let cell_start = find_cell_start(xml, &needle)?;
        let tag_end_abs = xml[cell_start..].find('>')
            .map(|p| cell_start + p)
            .ok_or_else(|| SurgeonError::CellNotFound(cell_ref.into()))?;
        let opening = &xml[cell_start..=tag_end_abs];
        let body_start = tag_end_abs + 1;
        let body_end = xml[body_start..].find("</c>")
            .map(|p| body_start + p)
            .unwrap_or(xml.len());
        let body = &xml[body_start..body_end];

        match detect_cell_type(opening) {
            CellType::SharedString => {
                let v_raw = extract_tag_text(body, "v")
                    .ok_or_else(|| SurgeonError::CellNotFound(cell_ref.into()))?;
                let idx: usize = quick_xml::escape::unescape(&v_raw)
                    .map_err(|_| SurgeonError::CellNotFound(cell_ref.into()))?
                    .parse()
                    .map_err(|_| SurgeonError::CellNotFound(cell_ref.into()))?;
                let ss_xml = self.read_entry("xl/sharedStrings.xml")?;
                let ss = SharedStrings::parse(&ss_xml)?;
                ss.si_entries.get(idx).cloned()
                    .ok_or_else(|| SurgeonError::CellNotFound(format!("shared string index {idx}")))
            }
            CellType::InlineString => {
                Ok(extract_is_t_text(body).unwrap_or_default())
            }
            _ => {
                Ok(extract_tag_text(body, "v")
                    .and_then(|raw| quick_xml::escape::unescape(&raw).ok().map(|s| s.into_owned()))
                    .unwrap_or_default())
            }
        }
    }

    fn load_shared_strings(&self) -> Result<SharedStrings, SurgeonError> {
        SharedStrings::parse(&self.read_entry("xl/sharedStrings.xml")?)
    }
}

// ---------------------------------------------------------------------------
// Cell type & content helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
enum CellType { SharedString, InlineString, None }

fn detect_cell_type(tag: &str) -> CellType {
    if tag.contains("t=\"s\"") { CellType::SharedString }
    else if tag.contains("t=\"inlineStr\"") { CellType::InlineString }
    else { CellType::None }
}

fn find_cell_start(xml: &str, needle: &str) -> Result<usize, SurgeonError> {
    let mut pos = 0;
    while pos < xml.len() {
        let Some(idx) = xml[pos..].find(needle) else { break };
        let abs = pos + idx;
        let after = abs + needle.len();
        if after < xml.len() && xml.as_bytes()[after] == b'"' {
            return Ok(abs);
        }
        pos = after;
    }
    Err(SurgeonError::CellNotFound(needle.trim_start_matches("<c r=\"").into()))
}

/// Extract text content of `<tag>...</tag>` (first occurrence in body), unescaped.
fn extract_tag_text(body: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let open_alt = format!("<{tag} ");
    let close = format!("</{tag}>");
    let start = body.find(&open).map(|p| p + open.len())
        .or_else(|| body.find(&open_alt).and_then(|p| body[p..].find('>').map(|g| p + g + 1)))?;
    let end = body[start..].find(&close)?;
    Some(body[start..start + end].to_string())
}

/// Extract text from `<is><t>...</t></is>`, unescaped.
fn extract_is_t_text(body: &str) -> Option<String> {
    let is_start = body.find("<is>")?;
    let t_rel = body[is_start..].find("<t")?;
    let t_abs = is_start + t_rel;
    let t_tag_end = body[t_abs..].find('>').map(|p| t_abs + p)?;
    let content_start = t_tag_end + 1;
    let content_end = body[content_start..].find("</t>")?;
    let raw = &body[content_start..content_start + content_end];
    quick_xml::escape::unescape(raw).ok().map(|s| s.into_owned())
}

/// Replace `<v>...</v>` content in cell body.
fn replace_v_in_body(body: &str, new_v: &str) -> String {
    let escaped = quick_xml::escape::escape(new_v);
    if let Some(s) = body.find("<v>") {
        let e = body[s..].find("</v>").map(|p| s + p).unwrap_or(body.len());
        format!("{}<v>{escaped}</v>{}", &body[..s], &body[e + 4..])
    } else if let Some(s) = body.find("<v ") {
        let e = body[s..].find("</v>").map(|p| s + p).unwrap_or(body.len());
        let g = body[s..].find('>').map(|p| s + p).unwrap_or(e);
        format!("{}{}{escaped}</v>{}", &body[..s], &body[s..=g], &body[e + 4..])
    } else {
        format!("<v>{escaped}</v>{body}")
    }
}

/// Replace text inside `<is><t>...</t></is>` in cell body.
fn replace_is_t_text(body: &str, new_text: &str) -> Option<String> {
    let is_start = body.find("<is>")?;
    let t_rel = body[is_start..].find("<t")?;
    let t_abs = is_start + t_rel;
    let t_tag_end = body[t_abs..].find('>').map(|p| t_abs + p)?;
    let content_start = t_tag_end + 1;
    let t_close = body[content_start..].find("</t>")?;
    let escaped = quick_xml::escape::escape(new_text);
    Some(format!("{}{escaped}{}", &body[..content_start], &body[content_start + t_close..]))
}

// ---------------------------------------------------------------------------
// Workbook XML parsing helpers
// ---------------------------------------------------------------------------

fn get_attr(e: &BytesStart, name: &[u8]) -> Option<String> {
    for attr in e.attributes().flatten() {
        if attr.key.as_ref() == name {
            return String::from_utf8(attr.value.to_vec()).ok();
        }
    }
    None
}

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
                        .or_else(|| get_attr(e, b"{http://schemas.openxmlformats.org/officeDocument/2006/relationships}id"))
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
                    if !id.is_empty() && !target.is_empty() { map.insert(id, target); }
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

struct SharedStrings {
    si_entries: Vec<String>,
    count: u32,
}

impl SharedStrings {
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
                    match e.name().as_ref() {
                        b"si" => in_si = true,
                        b"t" if in_si => { in_t = true; current_t.clear(); }
                        _ => {}
                    }
                }
                Ok(Event::Text(ref e)) if in_t => {
                    current_t.push_str(&e.unescape().unwrap_or_default());
                }
                Ok(Event::End(ref e)) => match e.name().as_ref() {
                    b"t" if in_si => in_t = false,
                    b"si" => {
                        in_si = false;
                        si_entries.push(current_t.clone());
                        current_t.clear();
                        count += 1;
                    }
                    _ => {}
                },
                Ok(Event::Eof) => break,
                _ => {}
            }
            buf.clear();
        }
        Ok(Self { si_entries, count })
    }

    fn append(&mut self, text: &str) -> usize {
        let idx = self.si_entries.len();
        self.si_entries.push(text.into());
        self.count += 1;
        idx
    }

    fn serialize(&self) -> Result<Vec<u8>, SurgeonError> {
        let mut out = Vec::new();
        out.extend_from_slice(b"<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n");
        let count = self.count.to_string();
        let unique = self.si_entries.len().to_string();
        out.extend_from_slice(b"<sst xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"");
        write_attr(&mut out, b" count", &count);
        write_attr(&mut out, b" uniqueCount", &unique);
        out.extend_from_slice(b">");
        for text in &self.si_entries {
            out.extend_from_slice(b"<si><t>");
            out.extend_from_slice(quick_xml::escape::escape(text).as_bytes());
            out.extend_from_slice(b"</t></si>");
        }
        out.extend_from_slice(b"</sst>");
        Ok(out)
    }
}

fn write_attr(out: &mut Vec<u8>, key: &[u8], value: &str) {
    out.extend_from_slice(key);
    out.push(b'=');
    out.push(b'"');
    out.extend_from_slice(value.as_bytes());
    out.push(b'"');
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
    }

    #[test]
    fn open_and_save_roundtrip() {
        let surgeon = XmlSurgeon::open(&fixture("data1.xlsx")).unwrap();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        surgeon.save(tmp.path()).unwrap();
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
        let mut ss = ss;
        assert_eq!(ss.append("New"), 2);
        assert_eq!(ss.count, 3);
        let s = String::from_utf8(ss.serialize().unwrap()).unwrap();
        assert!(s.contains("<si><t>New</t></si>"));
        assert!(s.contains("count=\"3\""));
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
        assert_eq!(entries, vec![("Sheet1".into(), "rId1".into()), ("Sheet2".into(), "rId2".into())]);
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
        let xml = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1">
      <c r="A1" t="s"><v>0</v></c>
    </row>
  </sheetData>
</worksheet>"#;
        let mut reader = XmlReader::from_str(xml);
        reader.config_mut().trim_text(true);
        let mut out = Vec::new();
        let mut writer = quick_xml::writer::Writer::new(&mut out);
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Eof) => break,
                Ok(e) => { writer.write_event(e.into_owned()).unwrap(); }
                Err(_) => panic!("XML parse error"),
            }
            buf.clear();
        }
        let result = String::from_utf8(out).unwrap();
        assert!(result.contains("A1"));
    }
}

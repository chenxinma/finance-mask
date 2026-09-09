//! ppt_writer — surgical pptx text write-back.
//!
//! Provides [`PptxEditor`] for opening a .pptx, replacing text inside `<a:t>`
//! elements of a slide's XML (and its notes XML), and saving the result. Every
//! other byte of every other entry is preserved exactly, mirroring the zip
//! open/save pattern of [`crate::xmlsurgeon::XmlSurgeon`].

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

use zip::read::ZipArchive;
use zip::write::{SimpleFileOptions, ZipWriter};

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// Errors that can occur during pptx write-back operations.
#[derive(Debug)]
pub enum PptError {
    /// I/O error (file open, read, write).
    Io(std::io::Error),
    /// Invalid or malformed zip archive.
    Zip(zip::result::ZipError),
    /// XML parse or serialize error.
    Xml(quick_xml::Error),
    /// UTF-8 decoding error.
    Utf8(std::string::FromUtf8Error),
    /// The slide XML entry for a 1-based slide index was not found.
    SlideNotFound(usize),
}

impl std::fmt::Display for PptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PptError::Io(e) => write!(f, "IO error: {}", e),
            PptError::Zip(e) => write!(f, "zip error: {}", e),
            PptError::Xml(e) => write!(f, "XML error: {}", e),
            PptError::Utf8(e) => write!(f, "UTF-8 error: {}", e),
            PptError::SlideNotFound(idx) => {
                write!(f, "slide XML entry not found: ppt/slides/slide{}.xml", idx)
            }
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
// Entry storage helper
// ---------------------------------------------------------------------------

/// Store entries as `Vec` for ordered iteration but provide lookup by name.
fn find_entry_idx(entries: &[(String, Vec<u8>)], name: &str) -> Option<usize> {
    entries.iter().position(|(n, _)| n == name)
}

// ---------------------------------------------------------------------------
// PptxEditor
// ---------------------------------------------------------------------------

/// Surgical pptx text editor.
///
/// Loads all zip entries into memory, allows targeted replacement of text
/// inside `<a:t>` elements, and writes a new pptx file with all non-modified
/// entries preserved byte-for-byte.
pub struct PptxEditor {
    /// (filename, content) pairs from the zip archive.
    entries: Vec<(String, Vec<u8>)>,
}

impl PptxEditor {
    /// Open a .pptx file and load all zip entries into memory.
    pub fn open(path: &Path) -> Result<Self, PptError> {
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

    /// Replace all occurrences of `old` with `new` inside `<a:t>...</a:t>`
    /// elements of slide `slide_idx` (1-based).
    ///
    /// Uses quick-xml event traversal: only Text/CData nodes inside `<a:t>`
    /// elements are modified; all other markup is copied verbatim.
    ///
    /// Both the slide body (`ppt/slides/slide{idx}.xml`) and, when present,
    /// its notes (`ppt/notesSlides/notesSlide{idx}.xml`) are updated. Returns
    /// [`PptError::SlideNotFound`] if the slide XML entry does not exist.
    /// A missing notes entry is not an error (slides without notes are normal).
    pub fn replace_text(&mut self, slide_idx: usize, old: &str, new: &str) -> Result<(), PptError> {
        let slide_name = format!("ppt/slides/slide{}.xml", slide_idx);
        let slide_entry =
            find_entry_idx(&self.entries, &slide_name).ok_or(PptError::SlideNotFound(slide_idx))?;
        let slide_xml = String::from_utf8(self.entries[slide_entry].1.clone())?;
        let result = replace_in_xml_events(&slide_xml, old, new);
        self.entries[slide_entry].1 = result.into_bytes();

        let notes_name = format!("ppt/notesSlides/notesSlide{}.xml", slide_idx);
        if let Some(notes_entry) = find_entry_idx(&self.entries, &notes_name) {
            let notes_xml = String::from_utf8(self.entries[notes_entry].1.clone())?;
            let result = replace_in_xml_events(&notes_xml, old, new);
            self.entries[notes_entry].1 = result.into_bytes();
        }

        Ok(())
    }

    /// Save all entries back to a new .pptx file.
    pub fn save(&self, path: &Path) -> Result<(), PptError> {
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
}

// ---------------------------------------------------------------------------
// Replacement core (event-based)
// ---------------------------------------------------------------------------

use quick_xml::events::{BytesCData, BytesEnd, BytesStart, BytesText, Event};
use quick_xml::reader::Reader;
use quick_xml::writer::Writer;

/// Replace `old` with `new` in text content inside `<a:t>` elements.
///
/// Uses quick-xml event traversal: reads every event, modifies Text/CData
/// nodes that sit inside an `<a:t>` element, and writes everything else
/// verbatim. This preserves XML structure perfectly — no string-level
/// truncation, no risk of corrupting surrounding markup.
///
/// For cross-run text (split across multiple `<a:t>` inside one `<a:p>`),
/// the paragraph is re-serialized with a single merged `<a:r><a:t>` run.
fn replace_in_xml_events(xml: &str, old: &str, new: &str) -> String {
    if old.is_empty() {
        return xml.to_string();
    }

    // Phase 1: single-run replacement via event stream.
    let result = replace_single_run(xml, old, new);
    if result != xml {
        return result;
    }

    // Phase 2: cross-run fallback — text split across multiple <a:t> runs.
    replace_cross_run_events(xml, old, new)
}

/// Single-run replacement: walk events, replace text inside each `<a:t>`.
fn replace_single_run(xml: &str, old: &str, new: &str) -> String {
    let mut reader = Reader::from_str(xml);
    let mut writer = Writer::new(Vec::with_capacity(xml.len()));
    let mut buf = Vec::new();
    // Depth counter: >0 means we are inside an <a:t> element.
    let mut in_a_t_depth: u32 = 0;
    let mut found = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                if is_a_t_tag(e) {
                    in_a_t_depth += 1;
                }
                writer.write_event(Event::Start(e.clone())).ok();
            }
            Ok(Event::End(ref e)) => {
                if is_a_t_end(e) && in_a_t_depth > 0 {
                    in_a_t_depth -= 1;
                }
                writer.write_event(Event::End(e.clone())).ok();
            }
            Ok(Event::Empty(ref e)) => {
                // Self-closing <a:t/> — nothing to replace.
                writer.write_event(Event::Empty(e.clone())).ok();
            }
            Ok(Event::Text(ref e)) if in_a_t_depth > 0 => {
                let text = e.unescape().unwrap_or_default();
                if text.contains(old) {
                    let replaced = text.replace(old, new);
                    writer.write_event(Event::Text(BytesText::new(&replaced))).ok();
                    found = true;
                } else {
                    writer.write_event(Event::Text(e.clone())).ok();
                }
            }
            Ok(Event::CData(ref e)) if in_a_t_depth > 0 => {
                let text = String::from_utf8_lossy(e.as_ref()).to_string();
                if text.contains(old) {
                    let replaced = text.replace(old, new);
                    writer.write_event(Event::CData(BytesCData::new(&replaced))).ok();
                    found = true;
                } else {
                    writer.write_event(Event::CData(e.clone())).ok();
                }
            }
            Ok(Event::Eof) => break,
            Ok(e) => {
                writer.write_event(e.into_owned()).ok();
            }
            Err(_) => break,
        }
        buf.clear();
    }

    if found {
        String::from_utf8(writer.into_inner()).unwrap_or_else(|_| xml.to_string())
    } else {
        xml.to_string()
    }
}

/// Cross-run replacement: when `old` spans multiple `<a:t>` runs inside a
/// `<a:p>` paragraph, concatenate all `<a:t>` text, check for match, and
/// if found, merge runs into a single `<a:r><a:t>` with the replacement.
///
/// Only replaces the FIRST occurrence.
fn replace_cross_run_events(xml: &str, old: &str, new: &str) -> String {
    let mut reader = Reader::from_str(xml);
    let mut writer = Writer::new(Vec::with_capacity(xml.len()));
    let mut buf = Vec::new();

    // State for collecting a paragraph's <a:t> text.
    let mut in_para = false;
    let mut para_events: Vec<Event> = Vec::new();
    let mut para_texts: Vec<String> = Vec::new();
    let mut current_text = String::new();
    let mut in_a_t_depth: u32 = 0;
    let mut replaced = false;

    loop {
        let event = reader.read_event_into(&mut buf);
        // Clone/own the event immediately so we can clear buf.
        let owned: Event = match event {
            Ok(e) => e.into_owned(),
            Err(_) => break,
        };
        buf.clear();

        match &owned {
            Event::Start(e) => {
                let ln = e.local_name();
                if ln.as_ref() == b"p" && !replaced {
                    in_para = true;
                    para_events.clear();
                    para_texts.clear();
                }
                if is_a_t_tag(e) {
                    in_a_t_depth += 1;
                    current_text.clear();
                }
                if in_para {
                    para_events.push(owned.clone());
                } else {
                    writer.write_event(owned.clone()).ok();
                }
            }
            Event::End(e) => {
                if is_a_t_end(e) && in_a_t_depth > 0 {
                    in_a_t_depth -= 1;
                    if in_para {
                        para_texts.push(current_text.clone());
                    }
                }
                let ln = e.local_name();
                if ln.as_ref() == b"p" && in_para {
                    let concat = para_texts.join("");
                    if !replaced && concat.contains(old) {
                        let r = concat.replacen(old, new, 1);
                        let escaped = escape_xml_text(&r);
                        writer.write_event(Event::Start(BytesStart::new("a:p"))).ok();
                        writer.write_event(Event::Start(BytesStart::new("a:r"))).ok();
                        writer.write_event(Event::Start(BytesStart::new("a:t"))).ok();
                        writer.write_event(Event::Text(BytesText::new(&escaped))).ok();
                        writer.write_event(Event::End(BytesEnd::new("a:t"))).ok();
                        writer.write_event(Event::End(BytesEnd::new("a:r"))).ok();
                        writer.write_event(Event::End(BytesEnd::new("a:p"))).ok();
                        replaced = true;
                    } else {
                        for ev in para_events.drain(..) {
                            writer.write_event(ev).ok();
                        }
                        writer.write_event(owned.clone()).ok();
                    }
                    in_para = false;
                    para_events.clear();
                    para_texts.clear();
                } else if in_para {
                    para_events.push(owned.clone());
                } else {
                    writer.write_event(owned.clone()).ok();
                }
            }
            Event::Empty(e) => {
                if is_a_t_tag(e) {
                    // Self-closing <a:t/> — nothing to replace.
                }
                if in_para {
                    para_events.push(owned.clone());
                } else {
                    writer.write_event(owned.clone()).ok();
                }
            }
            Event::Text(e) if in_a_t_depth > 0 => {
                let text = e.unescape().unwrap_or_default().to_string();
                current_text.push_str(&text);
                if in_para {
                    para_events.push(owned.clone());
                } else {
                    writer.write_event(owned.clone()).ok();
                }
            }
            Event::CData(e) if in_a_t_depth > 0 => {
                let text = String::from_utf8_lossy(e.as_ref()).to_string();
                current_text.push_str(&text);
                if in_para {
                    para_events.push(owned.clone());
                } else {
                    writer.write_event(owned.clone()).ok();
                }
            }
            Event::Eof => break,
            _ => {
                if in_para {
                    para_events.push(owned.clone());
                } else {
                    writer.write_event(owned.clone()).ok();
                }
            }
        }
    }

    if replaced {
        String::from_utf8(writer.into_inner()).unwrap_or_else(|_| xml.to_string())
    } else {
        xml.to_string()
    }
}

/// Check if a start element is `<a:t>` (not `<a:tbl>`, `<a:tc>`, etc.).
fn is_a_t_tag(e: &BytesStart) -> bool {
    let ln = e.local_name();
    ln.as_ref() == b"t" && e.name().as_ref().starts_with(b"a:")
}

/// Check if an end element is `</a:t>`.
fn is_a_t_end(e: &BytesEnd) -> bool {
    let ln = e.local_name();
    ln.as_ref() == b"t" && e.name().as_ref().starts_with(b"a:")
}

/// Minimal XML text escaping for values written into <a:t>.
fn escape_xml_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn replace_text_in_slide_xml() {
        // Plain text content.
        assert_eq!(
            replace_in_xml_events("<a:t>Hello World</a:t>", "World", "Finance"),
            "<a:t>Hello Finance</a:t>"
        );

        // Opening tag with an attribute is preserved verbatim.
        assert_eq!(
            replace_in_xml_events(r#"<a:t xml:space="preserve">abc</a:t>"#, "abc", "xyz"),
            r#"<a:t xml:space="preserve">xyz</a:t>"#
        );

        // Text inside <a:t> is replaced, but attribute text is untouched.
        assert_eq!(
            replace_in_xml_events(
                r#"<a:p><a:t>Hello</a:t></a:p><p:sp name="Hello"/>"#,
                "Hello",
                "Bye"
            ),
            r#"<a:p><a:t>Bye</a:t></a:p><p:sp name="Hello"/>"#
        );

        // Element names starting with "<a:t" (e.g. <a:txBody>) are not mistaken
        // for <a:t>, so the real <a:t> close tag survives intact.
        assert_eq!(
            replace_in_xml_events(
                "<a:txBody><a:r><a:t>Hello</a:t></a:r></a:txBody>",
                "Hello",
                "Bye"
            ),
            "<a:txBody><a:r><a:t>Bye</a:t></a:r></a:txBody>"
        );
    }

    #[test]
    fn replace_text_missing_slide_errors() {
        let mut editor = PptxEditor {
            entries: vec![(
                "ppt/slides/slide2.xml".to_string(),
                b"<a:t>x</a:t>".to_vec(),
            )],
        };
        match editor.replace_text(1, "x", "y") {
            Err(PptError::SlideNotFound(1)) => {}
            other => panic!("expected SlideNotFound(1), got {:?}", other),
        }
    }

    /// Collect all text in a slide (paragraphs + table cells + notes).
    fn slide_text(slide: &crate::ppt_reader::PptSlide) -> String {
        let mut text = String::new();
        for shape in &slide.shapes {
            for p in &shape.paragraphs {
                text.push_str(p);
                text.push('\n');
            }
            for row in &shape.table_rows {
                for cell in row {
                    text.push_str(cell);
                    text.push('\n');
                }
            }
        }
        for note in &slide.notes {
            text.push_str(note);
            text.push('\n');
        }
        text
    }

    #[test]
    fn replace_cross_run_basic() {
        // Text split across two <a:r> runs
        let xml = r#"<a:txBody><a:p><a:r><a:t>天齐锂业</a:t></a:r><a:r><a:t>股份有限公司</a:t></a:r></a:p></a:txBody>"#;
        let result = replace_in_xml_events(xml, "天齐锂业股份有限公司", "[公司A]");
        assert!(result.contains("[公司A]"), "cross-run should replace: {}", result);
        assert!(!result.contains("天齐锂业"), "original should be gone: {}", result);
    }

    #[test]
    fn replace_cross_run_no_match() {
        let xml = r#"<a:txBody><a:p><a:r><a:t>Hello</a:t></a:r></a:p></a:txBody>"#;
        let result = replace_in_xml_events(xml, "Missing", "X");
        assert_eq!(result, xml, "no match should return unchanged");
    }

    #[test]
    fn replace_cross_run_single_run_still_works() {
        // Even in cross-run mode, single-run text should still be found
        let xml = r#"<a:txBody><a:p><a:r><a:t>Hello World</a:t></a:r></a:p></a:txBody>"#;
        let result = replace_in_xml_events(xml, "Hello", "Bye");
        assert!(result.contains("Bye World"), "single run via cross-run: {}", result);
    }

    #[test]
    fn real_pptx_roundtrip() {
        let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let src = crate_dir.join("../examples/sample_report.pptx");

        let slides = crate::ppt_reader::parse_pptx(&src).unwrap();
        let slide1 = slides
            .iter()
            .find(|s| s.slide_idx == 1)
            .expect("sample_report.pptx has slide 1");

        // Pick a distinctive, non-empty paragraph as the replacement target.
        let needle = slide1
            .shapes
            .iter()
            .flat_map(|s| s.paragraphs.iter())
            .find(|p| !p.trim().is_empty())
            .expect("slide 1 has a non-empty paragraph")
            .clone();
        assert!(!needle.is_empty());
        assert!(slide_text(slide1).contains(&needle));

        let mut editor = PptxEditor::open(&src).unwrap();
        editor.replace_text(1, &needle, "REDACTED_TEST").unwrap();

        let tmp = tempfile::NamedTempFile::new().unwrap();
        editor.save(tmp.path()).unwrap();

        let reparsed = crate::ppt_reader::parse_pptx(tmp.path()).unwrap();
        let reparsed_slide1 = reparsed
            .iter()
            .find(|s| s.slide_idx == 1)
            .expect("re-parsed file has slide 1");
        let reparsed_text = slide_text(reparsed_slide1);

        assert!(reparsed_text.contains("REDACTED_TEST"));
        assert!(!reparsed_text.contains(&needle));
    }
}

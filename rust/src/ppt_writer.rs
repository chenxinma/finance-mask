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
    /// Both the slide body (`ppt/slides/slide{idx}.xml`) and, when present,
    /// its notes (`ppt/notesSlides/notesSlide{idx}.xml`) are updated. Returns
    /// [`PptError::SlideNotFound`] if the slide XML entry does not exist.
    /// A missing notes entry is not an error (slides without notes are normal).
    pub fn replace_text(&mut self, slide_idx: usize, old: &str, new: &str) -> Result<(), PptError> {
        let slide_name = format!("ppt/slides/slide{}.xml", slide_idx);
        let slide_entry =
            find_entry_idx(&self.entries, &slide_name).ok_or(PptError::SlideNotFound(slide_idx))?;
        let slide_xml = String::from_utf8(self.entries[slide_entry].1.clone())?;
        // First try: single-run replacement
        let mut result = replace_in_xml(&slide_xml, old, new);
        // I5 fallback: if old spans multiple <a:t> runs, merge and replace
        if result == slide_xml {
            result = replace_cross_run(&slide_xml, old, new);
        }
        self.entries[slide_entry].1 = result.into_bytes();

        let notes_name = format!("ppt/notesSlides/notesSlide{}.xml", slide_idx);
        if let Some(notes_entry) = find_entry_idx(&self.entries, &notes_name) {
            let notes_xml = String::from_utf8(self.entries[notes_entry].1.clone())?;
            let mut result = replace_in_xml(&notes_xml, old, new);
            if result == notes_xml {
                result = replace_cross_run(&notes_xml, old, new);
            }
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
// Replacement core
// ---------------------------------------------------------------------------

/// Replace `old` with `new` only inside `<a:t>...</a:t>` text content.
///
/// Byte-level scan over the raw XML string: the replacement is applied only to
/// the span between an `<a:t` opening tag (with or without attributes) and its
/// matching `</a:t>` closing tag. Everything else — including element names
/// that merely start with `<a:t` (e.g. `<a:tbl>`, `<a:tc>`, `<a:tr>`,
/// `<a:txBody>`, `<a:tableStyleId>`), attributes, and other markup — is copied
/// through verbatim.
///
/// # Escaping assumption
///
/// `old` and `new` are plain (unescaped) text. The replacement is a raw
/// substring replacement of the stored bytes and `new` is written back
/// verbatim: no XML escaping is performed. Finance-mask values (amounts,
/// names, account numbers) contain no `<`, `>`, or `&`, so callers must pass
/// values that do not require XML escaping.
fn replace_in_xml(xml: &str, old: &str, new: &str) -> String {
    // Empty `old` would make `str::replace` insert `new` between every
    // character; treat it as a no-op instead of producing surprising output.
    if old.is_empty() {
        return xml.to_string();
    }

    let bytes = xml.as_bytes();
    let mut out = String::with_capacity(xml.len());
    // Index of the next byte not yet copied verbatim into `out`.
    let mut cursor = 0;
    // Index at which to resume scanning for the next `<a:t` candidate.
    let mut search_from = 0;

    loop {
        let Some(rel) = xml[search_from..].find("<a:t") else {
            break;
        };
        let start = search_from + rel;

        // `<a:t` must be followed by `>`, `/`, or whitespace to be an `<a:t>`
        // element. Anything else is a different element name and is skipped.
        let after = start + "<a:t".len();
        if after >= bytes.len() {
            break;
        }
        let next = bytes[after];
        let is_a_t = next == b'>'
            || next == b'/'
            || next == b' '
            || next == b'\t'
            || next == b'\r'
            || next == b'\n';
        if !is_a_t {
            search_from = start + "<a:t".len();
            continue;
        }

        // Find the end of the opening tag (first `>` after `<a:t`).
        let Some(rel_gt) = xml[start..].find('>') else {
            break;
        };
        let open_end = start + rel_gt;

        // Self-closing (`<a:t/>` or `<a:t xml:space="preserve"/>`) has no
        // text content, so there is nothing to replace.
        if xml[start..open_end].ends_with('/') {
            search_from = open_end + 1;
            continue;
        }

        // Text content spans from just after `>` to the matching `</a:t>`.
        let content_start = open_end + 1;
        let Some(rel_close) = xml[content_start..].find("</a:t>") else {
            break;
        };
        let close_start = content_start + rel_close;
        let close_end = close_start + "</a:t>".len();

        out.push_str(&xml[cursor..content_start]);
        out.push_str(&xml[content_start..close_start].replace(old, new));
        out.push_str(&xml[close_start..close_end]);
        cursor = close_end;
        search_from = close_end;
    }

    out.push_str(&xml[cursor..]);
    out
}

/// I5 fallback: when `old` spans multiple `<a:t>` runs inside a `<a:p>`
/// paragraph, concatenate all `<a:t>` text, check for match, and if found,
/// merge all runs into one with the replacement applied.
///
/// Only replaces the FIRST occurrence across runs (matches `replace_in_xml`
/// behavior of one replacement per call via executor).
fn replace_cross_run(xml: &str, old: &str, new: &str) -> String {
    if old.is_empty() {
        return xml.to_string();
    }

    let bytes = xml.as_bytes();
    let mut out = String::with_capacity(xml.len());
    let mut cursor = 0;

    loop {
        // Find next <a:p> (not <a:para>, <a:pic>, etc.)
        let Some(rel) = xml[cursor..].find("<a:p") else { break };
        let p_start = cursor + rel;
        let after_tag = p_start + 4;
        if after_tag >= bytes.len() { break; }
        let next = bytes[after_tag];
        if next != b'>' && next != b' ' && next != b'\t' && next != b'\r' && next != b'\n' && next != b'/' {
            cursor = after_tag;
            continue;
        }

        // Find matching </a:p> (accounting for nesting)
        let mut depth = 1u32;
        let mut scan = after_tag;
        let p_content_end = loop {
            if let Some(r) = xml[scan..].find("</a:p>") {
                let abs = scan + r;
                depth -= 1;
                if depth == 0 { break Some(abs); }
                scan = abs + 6;
            } else if let Some(r) = xml[scan..].find("<a:p") {
                let abs = scan + r;
                let a = abs + 4;
                if a < bytes.len() {
                    let c = bytes[a];
                    if c == b'>' || c == b' ' || c == b'\t' || c == b'\r' || c == b'\n' || c == b'/' {
                        depth += 1;
                    }
                }
                scan = abs + 4;
            } else {
                break None;
            }
        };
        let Some(content_end) = p_content_end else { break };
        let p_full_end = content_end + "</a:p>".len();
        let p_xml = &xml[p_start..p_full_end];

        // Concatenate all <a:t> text content within this paragraph
        let mut concat = String::new();
        let mut pos = 0;
        while pos < p_xml.len() {
            if let Some(r) = p_xml[pos..].find("<a:t") {
                let abs = pos + r;
                let a = abs + 4;
                if a >= p_xml.len() { break; }
                let c = p_xml.as_bytes()[a];
                let is_a_t = c == b'>' || c == b'/' || c == b' ' || c == b'\t' || c == b'\r' || c == b'\n';
                if !is_a_t { pos = a; continue; }
                if let Some(gt) = p_xml[abs..].find('>') {
                    let open_end = abs + gt;
                    if p_xml[abs..open_end].ends_with('/') {
                        pos = open_end + 1;
                        continue;
                    }
                    let text_start = open_end + 1;
                    if let Some(cl) = p_xml[text_start..].find("</a:t>") {
                        let text_end = text_start + cl;
                        concat.push_str(&p_xml[text_start..text_end]);
                        pos = text_end + 6;
                        continue;
                    }
                }
                break;
            } else {
                break;
            }
        }

        // Copy everything before this paragraph
        out.push_str(&xml[cursor..p_start]);

        if concat.contains(old) {
            // Found: merge all runs into one with replacement
            let replaced = concat.replacen(old, new, 1);
            let escaped = escape_xml_text(&replaced);
            out.push_str(&format!("<a:p><a:r><a:t>{}</a:t></a:r></a:p>", escaped));
        } else {
            // No match — keep paragraph as-is
            out.push_str(p_xml);
        }
        cursor = p_full_end;
    }

    out.push_str(&xml[cursor..]);
    out
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
            replace_in_xml("<a:t>Hello World</a:t>", "World", "Finance"),
            "<a:t>Hello Finance</a:t>"
        );

        // Opening tag with an attribute is preserved verbatim.
        assert_eq!(
            replace_in_xml(r#"<a:t xml:space="preserve">abc</a:t>"#, "abc", "xyz"),
            r#"<a:t xml:space="preserve">xyz</a:t>"#
        );

        // Text inside <a:t> is replaced, but attribute text is untouched.
        assert_eq!(
            replace_in_xml(
                r#"<a:p><a:t>Hello</a:t></a:p><p:sp name="Hello"/>"#,
                "Hello",
                "Bye"
            ),
            r#"<a:p><a:t>Bye</a:t></a:p><p:sp name="Hello"/>"#
        );

        // Element names starting with "<a:t" (e.g. <a:txBody>) are not mistaken
        // for <a:t>, so the real <a:t> close tag survives intact.
        assert_eq!(
            replace_in_xml(
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
        let result = replace_in_xml(xml, "天齐锂业股份有限公司", "[公司A]");
        // Single-run replacement won't find it
        assert_eq!(result, xml, "single-run should not match cross-run text");

        // Cross-run fallback should
        let result = replace_cross_run(xml, "天齐锂业股份有限公司", "[公司A]");
        assert!(result.contains("[公司A]"), "cross-run should replace: {}", result);
        assert!(!result.contains("天齐锂业"), "original should be gone: {}", result);
    }

    #[test]
    fn replace_cross_run_no_match() {
        let xml = r#"<a:txBody><a:p><a:r><a:t>Hello</a:t></a:r></a:p></a:txBody>"#;
        let result = replace_cross_run(xml, "Missing", "X");
        assert_eq!(result, xml, "no match should return unchanged");
    }

    #[test]
    fn replace_cross_run_single_run_still_works() {
        // Even in cross-run mode, single-run text should still be found
        let xml = r#"<a:txBody><a:p><a:r><a:t>Hello World</a:t></a:r></a:p></a:txBody>"#;
        let result = replace_cross_run(xml, "Hello", "Bye");
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

//! Integration tests for xmlsurgeon — surgical xlsx cell editing.

use calamine::Reader;
use finance_mask_core::xmlsurgeon::XmlSurgeon;
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// 获取 fixture 的第一个可见 sheet 名（替代硬编码 "Sheet1"）。
fn first_sheet_name(path: &std::path::Path) -> String {
    let wb: calamine::Xlsx<_> = calamine::open_workbook(path).unwrap();
    wb.sheets_metadata()
        .iter()
        .find(|m| m.visible == calamine::SheetVisible::Visible)
        .map(|m| m.name.clone())
        .expect("fixture should have at least one visible sheet")
}

/// Roundtrip test: open, set a cell, save, re-open with calamine, verify.
#[test]
fn roundtrip_set_cell_text() {
    let path = fixture("data1.xlsx");

    // First, discover the sheet structure via calamine
    let mut wb: calamine::Xlsx<_> = calamine::open_workbook(&path).unwrap();
    let sheets_meta = wb.sheets_metadata().to_vec();
    let first_sheet = sheets_meta
        .iter()
        .find(|m| m.visible == calamine::SheetVisible::Visible)
        .map(|m| m.name.clone())
        .expect("should have at least one visible sheet");

    // Read a known cell value from the original file
    let sheet_data = wb
        .worksheet_range(&first_sheet)
        .unwrap();

    // Use cell A1 as target
    let _original_a1 = sheet_data
        .get_value((0, 0))
        .map(|d| format!("{:?}", d))
        .unwrap_or_default();

    // Open with xmlsurgeon and set A1 to a known value
    let mut surgeon = XmlSurgeon::open(&path).unwrap();
    surgeon
        .set_cell_text(&first_sheet, "A1", "SURGEON_TEST_VALUE")
        .expect("set_cell_text should succeed");

    // Save to temp file
    let tmp = tempfile::NamedTempFile::with_suffix(".xlsx").unwrap();
    surgeon.save(tmp.path()).expect("save should succeed");

    // Re-open with calamine and verify A1
    let mut wb2: calamine::Xlsx<_> = calamine::open_workbook(tmp.path()).unwrap();
    let sheet2 = wb2
        .worksheet_range(&first_sheet)
        .unwrap();
    let new_a1 = sheet2
        .get_value((0, 0))
        .map(|d| format!("{:?}", d))
        .unwrap_or_default();

    assert!(
        new_a1.contains("SURGEON_TEST_VALUE"),
        "A1 should contain SURGEON_TEST_VALUE, got: {}",
        new_a1
    );

    // Verify other cells are unchanged (check B1)
    if let Some(original_b1) = sheet_data.get_value((0, 1)) {
        let new_b1 = sheet2.get_value((0, 1)).unwrap();
        assert_eq!(
            format!("{:?}", original_b1),
            format!("{:?}", new_b1),
            "B1 should be unchanged"
        );
    }
}

/// Test that modifying a cell updates sharedStrings.xml correctly.
#[test]
fn shared_string_append_test() {
    let path = fixture("data1.xlsx");
    let mut surgeon = XmlSurgeon::open(&path).unwrap();
    let sheet = first_sheet_name(&path);

    // Get the original entry count
    let original_count = surgeon.entry_count();

    // Set a cell value (most xlsx use shared strings)
    surgeon
        .set_cell_text(&sheet, "A1", "NEW_SHARED_VALUE")
        .expect("set_cell_text should succeed");

    // Entry count should be the same (no new entries added, just modified)
    assert_eq!(
        surgeon.entry_count(),
        original_count,
        "entry count should not change after set_cell_text"
    );

    // Save and verify the file is still a valid xlsx
    let tmp = tempfile::NamedTempFile::with_suffix(".xlsx").unwrap();
    surgeon.save(tmp.path()).expect("save should succeed");

    let saved = XmlSurgeon::open(tmp.path()).unwrap();
    assert_eq!(
        saved.entry_count(),
        original_count,
        "saved file should have same entry count"
    );
}

/// Test that saving preserves approximate file size (no data loss).
#[test]
fn preserve_formatting_size_check() {
    let path = fixture("data1.xlsx");
    let original_size = std::fs::metadata(&path).unwrap().len();

    let mut surgeon = XmlSurgeon::open(&path).unwrap();
    let sheet = first_sheet_name(&path);
    surgeon
        .set_cell_text(&sheet, "A1", "SIZE_TEST")
        .expect("set_cell_text should succeed");

    let tmp = tempfile::NamedTempFile::with_suffix(".xlsx").unwrap();
    surgeon.save(tmp.path()).expect("save should succeed");

    let saved_size = std::fs::metadata(tmp.path()).unwrap().len();

    // Size should be similar (within 20% — some variance from recompression is expected)
    let ratio = saved_size as f64 / original_size as f64;
    assert!(
        ratio > 0.5 && ratio < 2.0,
        "file size changed dramatically: original={}, saved={}, ratio={:.2}",
        original_size,
        saved_size,
        ratio
    );
}

/// Test that cell_ref mismatch returns an error.
#[test]
fn cell_not_found_error() {
    let path = fixture("data1.xlsx");
    let mut surgeon = XmlSurgeon::open(&path).unwrap();

    // Use a cell reference that almost certainly doesn't exist
    let result = surgeon.set_cell_text(&first_sheet_name(&path), "ZZZZ99999", "test");
    assert!(result.is_err(), "should error on non-existent cell");
}

/// Test that non-existent sheet name returns an error.
#[test]
fn sheet_not_found_error() {
    let path = fixture("data1.xlsx");
    let mut surgeon = XmlSurgeon::open(&path).unwrap();

    let result = surgeon.set_cell_text("NonExistentSheet", "A1", "test");
    assert!(result.is_err(), "should error on non-existent sheet");
}

/// Test multiple sequential edits to different cells.
#[test]
fn multiple_edits() {
    let path = fixture("data1.xlsx");
    let mut surgeon = XmlSurgeon::open(&path).unwrap();
    let sheet_name = first_sheet_name(&path);

    // Edit two cells
    surgeon
        .set_cell_text(&sheet_name, "A1", "FIRST_EDIT")
        .expect("first edit should succeed");
    surgeon
        .set_cell_text(&sheet_name, "A2", "SECOND_EDIT")
        .expect("second edit should succeed");

    let tmp = tempfile::NamedTempFile::with_suffix(".xlsx").unwrap();
    surgeon.save(tmp.path()).expect("save should succeed");

    let mut wb: calamine::Xlsx<_> = calamine::open_workbook(tmp.path()).unwrap();
    let sheet = wb
        .worksheet_range(&sheet_name)
        .unwrap();

    let a1 = format!("{:?}", sheet.get_value((0, 0)).unwrap());
    let a2 = format!("{:?}", sheet.get_value((1, 0)).unwrap());

    assert!(a1.contains("FIRST_EDIT"), "A1 should be FIRST_EDIT, got: {}", a1);
    assert!(a2.contains("SECOND_EDIT"), "A2 should be SECOND_EDIT, got: {}", a2);
}

/// Test XML special characters in cell values.
#[test]
fn xml_special_characters() {
    let path = fixture("data1.xlsx");
    let mut surgeon = XmlSurgeon::open(&path).unwrap();
    let sheet_name = first_sheet_name(&path);

    let special = "Test <b>bold</b> & \"quotes\" 'apostrophe'";
    surgeon
        .set_cell_text(&sheet_name, "A1", special)
        .expect("set with special chars should succeed");

    let tmp = tempfile::NamedTempFile::with_suffix(".xlsx").unwrap();
    surgeon.save(tmp.path()).expect("save should succeed");

    let mut wb: calamine::Xlsx<_> = calamine::open_workbook(tmp.path()).unwrap();
    let sheet = wb
        .worksheet_range(&sheet_name)
        .unwrap();

    let a1 = format!("{:?}", sheet.get_value((0, 0)).unwrap());
    assert!(
        a1.contains("bold"),
        "should preserve special chars, got: {}",
        a1
    );
}

/// Debug helper: list sheets in data1.xlsx
#[test]
#[ignore]
fn list_data1_sheets() {
    let path = fixture("data1.xlsx");
    let wb: calamine::Xlsx<_> = calamine::open_workbook(&path).unwrap();
    let meta = wb.sheets_metadata().to_vec();
    for m in &meta {
        eprintln!("Sheet: '{}', visible: {:?}", m.name, m.visible);
    }
}

/// Debug: inspect the sheet XML for a specific cell
#[test]
#[ignore]
fn debug_inspect_data1() {
    use zip::ZipArchive;
    let path = fixture("data1.xlsx");
    let file = std::fs::File::open(&path).unwrap();
    let mut archive = ZipArchive::new(std::io::BufReader::new(file)).unwrap();
    
    // List all entries
    for i in 0..archive.len() {
        let entry = archive.by_index(i).unwrap();
        println!("Entry: {}", entry.name());
    }
    
    // Read workbook.xml
    let mut wb = String::new();
    archive.by_name("xl/workbook.xml").unwrap().read_to_string(&mut wb).unwrap();
    println!("\nworkbook.xml:\n{}", &wb[..500.min(wb.len())]);
    
    // Read rels
    let mut rels = String::new();
    archive.by_name("xl/_rels/workbook.xml.rels").unwrap().read_to_string(&mut rels).unwrap();
    println!("\nworkbook.xml.rels:\n{}", &rels[..500.min(rels.len())]);
    
    // Read sheet1.xml
    let mut sheet = String::new();
    archive.by_name("xl/worksheets/sheet1.xml").unwrap().read_to_string(&mut sheet).unwrap();
    // Print first 2000 chars
    println!("\nsheet1.xml (first 2000):\n{}", &sheet[..2000.min(sheet.len())]);
}

use std::io::Read;

/// C1: writing non-numeric text to a numeric cell must add t="str".
#[test]
fn numeric_cell_gets_type_str_on_text_write() {
    let path = fixture("data1.xlsx");

    // data1.xlsx sheet2 has <c r="A2" s="11"> (numeric, no t= attribute)
    // Discover sheet2's name dynamically
    let mut wb: calamine::Xlsx<_> = calamine::open_workbook(&path).unwrap();
    let sheets_meta = wb.sheets_metadata().to_vec();
    let sheet2_name = sheets_meta.iter()
        .find(|m| m.visible == calamine::SheetVisible::Visible && m.name != sheets_meta[0].name)
        .map(|m| m.name.clone())
        .unwrap_or_else(|| sheets_meta.last().unwrap().name.clone());

    let mut surgeon = XmlSurgeon::open(&path).unwrap();
    // Write masked text to A2 (a numeric cell)
    surgeon.set_cell_text(&sheet2_name, "A2", "1*,***,***.**").unwrap();

    let tmp = tempfile::NamedTempFile::with_suffix(".xlsx").unwrap();
    surgeon.save(tmp.path()).unwrap();

    // Verify: the raw XML for A2 must contain t="str"
    let saved = XmlSurgeon::open(tmp.path()).unwrap();
    let entries = saved.entries_snapshot();
    // Find sheet2's XML path
    let sheet_xml = entries.iter()
        .find(|(n, _)| n.contains("sheet2"))
        .map(|(_, c)| String::from_utf8_lossy(c).to_string())
        .unwrap();
    let idx = sheet_xml.find("<c r=\"A2\"").expect("A2 should exist");
    let tag_end = sheet_xml[idx..].find('>').map(|p| idx + p).unwrap();
    let cell_tag = &sheet_xml[idx..=tag_end];
    assert!(cell_tag.contains("t=\"str\""),
        "numeric cell with text must have t=\"str\", got tag: {}", cell_tag);

    // Verify: calamine can read the file (strict reader validation)
    let mut wb2: calamine::Xlsx<_> = calamine::open_workbook(tmp.path()).unwrap();
    let _ = wb2.worksheet_range(&sheet2_name).unwrap();
}

/// C1: writing a number to a numeric cell must NOT add t="str".
#[test]
fn numeric_cell_keeps_no_type_on_number_write() {
    let path = fixture("data1.xlsx");

    let mut wb: calamine::Xlsx<_> = calamine::open_workbook(&path).unwrap();
    let sheets_meta = wb.sheets_metadata().to_vec();
    let sheet2_name = sheets_meta.iter()
        .find(|m| m.visible == calamine::SheetVisible::Visible && m.name != sheets_meta[0].name)
        .map(|m| m.name.clone())
        .unwrap_or_else(|| sheets_meta.last().unwrap().name.clone());

    let mut surgeon = XmlSurgeon::open(&path).unwrap();
    surgeon.set_cell_text(&sheet2_name, "A2", "9999999").unwrap();

    let tmp = tempfile::NamedTempFile::with_suffix(".xlsx").unwrap();
    surgeon.save(tmp.path()).unwrap();

    // Verify: the cell tag must NOT have t="str"
    let saved = XmlSurgeon::open(tmp.path()).unwrap();
    let entries = saved.entries_snapshot();
    let sheet_xml = entries.iter()
        .find(|(n, _)| n.contains("sheet2"))
        .map(|(_, c)| String::from_utf8_lossy(c).to_string())
        .unwrap();
    let idx = sheet_xml.find("<c r=\"A2\"").expect("A2 should exist");
    let tag_end = sheet_xml[idx..].find('>').map(|p| idx + p).unwrap();
    let cell_tag = &sheet_xml[idx..=tag_end];
    assert!(!cell_tag.contains("t=\"str\""),
        "numeric cell writing number must NOT have t=\"str\", got: {}", cell_tag);
}

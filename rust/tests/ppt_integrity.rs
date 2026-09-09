// rust/tests/ppt_integrity.rs
// 集成测试：验证 PPT 脱敏后的文件有效性

use finance_mask_core::ppt_reader;
use finance_mask_core::ppt_writer::PptxEditor;
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../examples")
        .join(name)
}

/// 获取 Python 解释器路径
fn python_exe() -> PathBuf {
    let venv_python = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../.venv/Scripts/python.exe");
    if venv_python.exists() {
        return venv_python;
    }
    PathBuf::from("python")
}

/// 验证 pptx 文件是否有效（可被 python-pptx 读取）
fn verify_pptx_with_python(path: &std::path::Path) -> bool {
    let python = python_exe();
    let script = r#"
import sys
try:
    from pptx import Presentation
    prs = Presentation(sys.argv[1])
    for slide in prs.slides:
        for shape in slide.shapes:
            if shape.has_text_frame:
                _ = shape.text_frame.text
            if shape.has_table:
                for row in shape.table.rows:
                    for cell in row.cells:
                        _ = cell.text
    print("VALID")
except Exception as e:
    print(f"INVALID: {e}", file=sys.stderr)
    sys.exit(1)
"#;
    let output = std::process::Command::new(&python)
        .arg("-c")
        .arg(script)
        .arg(path)
        .output()
        .expect("Failed to execute Python");

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout.trim() == "VALID"
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        eprintln!("Python verification failed: {}", stderr);
        false
    }
}

#[test]
fn pptx_roundtrip_preserves_validity() {
    let input = fixture("sample_report.pptx");
    let editor = PptxEditor::open(&input).expect("Failed to open pptx");

    let tmp = tempfile::NamedTempFile::with_suffix(".pptx").unwrap();
    editor.save(tmp.path()).expect("Failed to save pptx");

    assert!(
        verify_pptx_with_python(tmp.path()),
        "Roundtrip pptx file is not valid according to python-pptx"
    );
}

#[test]
fn pptx_replace_text_preserves_validity() {
    let input = fixture("sample_report.pptx");
    let slides = ppt_reader::parse_pptx(&input).expect("Failed to parse pptx");

    // Find a text to replace
    let slide = slides.iter().find(|s| s.slide_idx == 2).unwrap();
    let shape = slide.shapes.iter().find(|s| !s.paragraphs.is_empty()).unwrap();
    let original_text = &shape.paragraphs[0];

    let mut editor = PptxEditor::open(&input).expect("Failed to open pptx");
    editor.replace_text(2, original_text, "REPLACED_TEXT").unwrap();

    let tmp = tempfile::NamedTempFile::with_suffix(".pptx").unwrap();
    editor.save(tmp.path()).expect("Failed to save pptx");

    assert!(
        verify_pptx_with_python(tmp.path()),
        "PPTX with replaced text is not valid"
    );

    // Verify the replacement worked
    let reparsed = ppt_reader::parse_pptx(tmp.path()).unwrap();
    let reparsed_slide = reparsed.iter().find(|s| s.slide_idx == 2).unwrap();
    let all_text: String = reparsed_slide
        .shapes
        .iter()
        .flat_map(|s| s.paragraphs.iter())
        .chain(reparsed_slide.notes.iter())
        .cloned()
        .collect::<Vec<String>>()
        .join(" ");
    assert!(
        all_text.contains("REPLACED_TEXT"),
        "Replacement text should be present"
    );
}

#[test]
fn pptx_replace_cross_run_text() {
    // Test replacing text that spans multiple <a:t> runs
    let input = fixture("sample_report.pptx");
    let mut editor = PptxEditor::open(&input).expect("Failed to open pptx");

    // Use a longer text that might span runs
    editor.replace_text(2, "2024年度", "[年度]").unwrap();

    let tmp = tempfile::NamedTempFile::with_suffix(".pptx").unwrap();
    editor.save(tmp.path()).expect("Failed to save pptx");

    assert!(
        verify_pptx_with_python(tmp.path()),
        "PPTX with cross-run replacement is not valid"
    );
}

#[test]
fn pptx_replace_in_table_preserves_validity() {
    let input = fixture("sample_report.pptx");
    let slides = ppt_reader::parse_pptx(&input).expect("Failed to parse pptx");

    // Find a table cell to replace
    let slide3 = slides.iter().find(|s| s.slide_idx == 3).unwrap();
    let table_shape = slide3
        .shapes
        .iter()
        .find(|s| s.kind == ppt_reader::ShapeKind::Table)
        .expect("Slide 3 should have a table");

    if let Some(cell_text) = table_shape
        .table_rows
        .get(1)
        .and_then(|row| row.get(1))
    {
        if !cell_text.is_empty() {
            let mut editor = PptxEditor::open(&input).expect("Failed to open pptx");
            editor.replace_text(3, cell_text, "[REDACTED]").unwrap();

            let tmp = tempfile::NamedTempFile::with_suffix(".pptx").unwrap();
            editor.save(tmp.path()).expect("Failed to save pptx");

            assert!(
                verify_pptx_with_python(tmp.path()),
                "PPTX with table cell replacement is not valid"
            );
        }
    }
}

#[test]
fn pptx_replace_nonexistent_text_is_noop() {
    let input = fixture("sample_report.pptx");
    let mut editor = PptxEditor::open(&input).expect("Failed to open pptx");

    // Try to replace text that doesn't exist - should not error
    editor.replace_text(1, "NONEXISTENT_TEXT_12345", "[X]").unwrap();

    let tmp = tempfile::NamedTempFile::with_suffix(".pptx").unwrap();
    editor.save(tmp.path()).expect("Failed to save pptx");

    assert!(
        verify_pptx_with_python(tmp.path()),
        "PPTX should still be valid after no-op replacement"
    );
}

// 测试 PPT 水印嵌入后的文件有效性

use finance_mask_core::watermark::Watermark;
use std::io::Write;
use std::path::PathBuf;
use zip::read::ZipArchive;
use zip::write::{SimpleFileOptions, ZipWriter};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../examples")
        .join(name)
}

fn python_exe() -> PathBuf {
    let venv_python =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.venv/Scripts/python.exe");
    if venv_python.exists() {
        return venv_python;
    }
    PathBuf::from("python")
}

fn verify_pptx(path: &std::path::Path) -> Result<(), String> {
    let python = python_exe();
    let script = r#"
import sys
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
print("OK")
"#;
    let output = std::process::Command::new(&python)
        .arg("-c")
        .arg(script)
        .arg(path)
        .output()
        .expect("Failed to execute Python");

    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).to_string())
    }
}

/// 模拟 embed_watermark_pptx 的逻辑，但不做实际水印嵌入，
/// 只测试 zip 重写是否破坏文件结构。
#[test]
fn pptx_zip_roundtrip_preserves_validity() {
    let input = fixture("sample_report.pptx");
    let tmp = tempfile::NamedTempFile::with_suffix(".pptx").unwrap();

    // 读取所有 entries
    let file = std::fs::File::open(&input).unwrap();
    let mut archive = ZipArchive::new(std::io::BufReader::new(file)).unwrap();
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).unwrap();
        let name = entry.name().to_string();
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut buf).unwrap();
        entries.push((name, buf));
    }

    // 原样写回（不修改任何内容）
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(6));
    let out_file = std::fs::File::create(tmp.path()).unwrap();
    let mut zip = ZipWriter::new(std::io::BufWriter::new(out_file));
    for (name, content) in &entries {
        zip.start_file(name, options).unwrap();
        zip.write_all(content).unwrap();
    }
    zip.finish().unwrap();

    // 验证
    let result = verify_pptx(tmp.path());
    assert!(result.is_ok(), "Zip roundtrip should preserve validity: {}", result.unwrap_err());
}

/// 测试水印嵌入后的文件有效性
#[test]
fn pptx_watermark_embed_preserves_validity() {
    let input = fixture("sample_report.pptx");
    let tmp = tempfile::NamedTempFile::with_suffix(".pptx").unwrap();

    // 先复制文件
    std::fs::copy(&input, tmp.path()).unwrap();

    // 嵌入水印
    let payload = "test_user|2024-01-01T00:00:00|abc123";
    let watermark = Watermark::encode(payload);
    assert!(!watermark.is_empty(), "watermark should not be empty");

    // 读取所有 entries
    let file = std::fs::File::open(tmp.path()).unwrap();
    let mut archive = ZipArchive::new(std::io::BufReader::new(file)).unwrap();
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).unwrap();
        let name = entry.name().to_string();
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut buf).unwrap();
        entries.push((name, buf));
    }

    // 修改第一个 slide 的第一个 <a:t>
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(6));
    let out_file = std::fs::File::create(tmp.path()).unwrap();
    let mut zip = ZipWriter::new(std::io::BufWriter::new(out_file));

    let mut watermarked = false;
    for (name, content) in &entries {
        if name.starts_with("ppt/slides/slide") && name.ends_with(".xml") && !watermarked {
            let xml = String::from_utf8_lossy(content);
            if let Some(idx) = xml.find("<a:t>") {
                let text_start = idx + 5;
                if let Some(end_idx) = xml[text_start..].find("</a:t>") {
                    let original_text = &xml[text_start..text_start + end_idx];
                    let new_text = format!("{}{}", original_text, &watermark);
                    let new_xml = format!(
                        "{}{}{}",
                        &xml[..text_start],
                        quick_xml::escape::escape(&new_text),
                        &xml[text_start + end_idx..]
                    );
                    zip.start_file(name, options).unwrap();
                    zip.write_all(new_xml.as_bytes()).unwrap();
                    watermarked = true;
                    continue;
                }
            }
        }
        zip.start_file(name, options).unwrap();
        zip.write_all(content).unwrap();
    }
    zip.finish().unwrap();

    assert!(watermarked, "should have found a <a:t> to watermark");

    // 验证
    let result = verify_pptx(tmp.path());
    assert!(result.is_ok(), "Watermarked pptx should be valid: {}", result.unwrap_err());
}

/// 测试 <a:t xml:space="preserve"> 形式（带属性的 <a:t>）
#[test]
fn pptx_watermark_with_attributed_t_element() {
    let input = fixture("sample_report.pptx");
    let tmp = tempfile::NamedTempFile::with_suffix(".pptx").unwrap();

    // 读取所有 entries
    let file = std::fs::File::open(&input).unwrap();
    let mut archive = ZipArchive::new(std::io::BufReader::new(file)).unwrap();
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).unwrap();
        let name = entry.name().to_string();
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut entry, &mut buf).unwrap();
        entries.push((name, buf));
    }

    // 将所有 <a:t> 替换为 <a:t xml:space="preserve">，然后嵌入水印
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(Some(6));
    let out_file = std::fs::File::create(tmp.path()).unwrap();
    let mut zip = ZipWriter::new(std::io::BufWriter::new(out_file));

    let watermark = Watermark::encode("test|2024-01-01|hash");
    let mut watermarked = false;

    for (name, content) in &entries {
        if name.starts_with("ppt/slides/slide") && name.ends_with(".xml") && !watermarked {
            let xml = String::from_utf8_lossy(content);
            // 先将 <a:t> 替换为 <a:t xml:space="preserve">
            let xml = xml.replace("<a:t>", r#"<a:t xml:space="preserve">"#);
            // 然后找带属性的 <a:t
            let needle = r#"<a:t xml:space="preserve">"#;
            if let Some(idx) = xml.find(needle) {
                let text_start = idx + needle.len();
                if let Some(end_idx) = xml[text_start..].find("</a:t>") {
                    let original_text = &xml[text_start..text_start + end_idx];
                    let new_text = format!("{}{}", original_text, &watermark);
                    let new_xml = format!(
                        "{}{}{}",
                        &xml[..text_start],
                        quick_xml::escape::escape(&new_text),
                        &xml[text_start + end_idx..]
                    );
                    zip.start_file(name, options).unwrap();
                    zip.write_all(new_xml.as_bytes()).unwrap();
                    watermarked = true;
                    continue;
                }
            }
        }
        zip.start_file(name, options).unwrap();
        zip.write_all(content).unwrap();
    }
    zip.finish().unwrap();

    assert!(watermarked, "should have found a <a:t> to watermark");

    // 验证
    let result = verify_pptx(tmp.path());
    assert!(
        result.is_ok(),
        "Pptx with attributed <a:t> and watermark should be valid: {}",
        result.unwrap_err()
    );
}

// 调试：找出 PPT 脱敏后哪个 zip entry 被破坏

use std::path::Path;

use finance_mask_core::column_matcher::ColumnMatcher;
use finance_mask_core::executor::Executor;
use finance_mask_core::models::*;
use finance_mask_core::patterns::PatternRegistry;
use finance_mask_core::ppt_reader;
use finance_mask_core::ppt_scanner::PptScanner;
use finance_mask_core::ppt_writer::PptxEditor;
use std::io::Read;

fn config_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("config").leak()
}
use std::path::PathBuf;
use zip::read::ZipArchive;

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

/// 检查 zip 中所有 XML entry 是否以 '<' 开头
fn check_zip_entries(path: &std::path::Path) -> Vec<String> {
    let file = std::fs::File::open(path).unwrap();
    let mut archive = ZipArchive::new(std::io::BufReader::new(file)).unwrap();
    let mut bad = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).unwrap();
        let name = entry.name().to_string();
        if name.ends_with(".xml") || name.ends_with(".rels") {
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf).unwrap();
            if buf.is_empty() {
                bad.push(format!("{}: EMPTY", name));
            } else if !buf.starts_with(b"<") {
                bad.push(format!(
                    "{}: first 20 bytes = {:?}",
                    name,
                    String::from_utf8_lossy(&buf[..20.min(buf.len())])
                ));
            }
        }
    }
    bad
}

#[test]
fn debug_tianqi_pptx_step_by_step() {
    let input = fixture("天齐锂业2026年半年度财务报告.pptx");
    if !input.exists() {
        eprintln!("skipping: fixture not found");
        return;
    }

    // Step 1: 只做 PptxEditor.open + save（不做任何替换）
    {
        let editor = PptxEditor::open(&input).unwrap();
        let tmp = tempfile::NamedTempFile::with_suffix(".pptx").unwrap();
        editor.save(tmp.path()).unwrap();
        let bad = check_zip_entries(tmp.path());
        assert!(bad.is_empty(), "Step 1 (open+save) broke entries: {:?}", bad);
        assert!(verify_pptx(tmp.path()).is_ok(), "Step 1 python-pptx failed");
        eprintln!("Step 1 OK: open+save preserves validity");
    }

    // Step 2: 解析 + 扫描（不做替换）
    let slides = ppt_reader::parse_pptx(&input).unwrap();
    let scanner = PptScanner::new(
        ColumnMatcher::new(vec![]),
        PatternRegistry::builtin(config_dir()).unwrap(),
    );
    let sites = scanner.scan(&slides);
    eprintln!("Step 2: found {} sites", sites.len());

    // Step 3: 只做第一个 site 的 replace_text
    if let Some(site) = sites.first() {
        let slide_idx = site.location.slide.unwrap_or(1) as usize;
        let original = &site.original_value;
        let redacted = "[REDACTED]";

        let mut editor = PptxEditor::open(&input).unwrap();
        editor.replace_text(slide_idx, original, redacted).unwrap();
        let tmp = tempfile::NamedTempFile::with_suffix(".pptx").unwrap();
        editor.save(tmp.path()).unwrap();

        let bad = check_zip_entries(tmp.path());
        if !bad.is_empty() {
            eprintln!("Step 3 BROKE entries: {:?}", bad);
        }
        match verify_pptx(tmp.path()) {
            Ok(()) => eprintln!("Step 3 OK: single replace preserves validity"),
            Err(e) => eprintln!("Step 3 FAILED: {}", e),
        }
    }

    // Step 4: 完整执行（含水印）
    {
        let strategy = Strategy {
            metadata: Metadata::default(),
            column_rules: None,
            sites: sites.clone(),
            global_params: None,
        };
        let tmp = tempfile::NamedTempFile::with_suffix(".pptx").unwrap();
        let mut exec = Executor::new(strategy, "test_user");
        let report = exec.execute(&input, tmp.path(), false).unwrap();
        eprintln!("Step 4: processed={}, errors={}", report.processed, report.errors);

        let bad = check_zip_entries(tmp.path());
        if !bad.is_empty() {
            eprintln!("Step 4 BROKE entries: {:?}", bad);
            for b in &bad {
                eprintln!("  {}", b);
            }
        }
        match verify_pptx(tmp.path()) {
            Ok(()) => eprintln!("Step 4 OK: full execution preserves validity"),
            Err(e) => {
                eprintln!("Step 4 FAILED: {}", e);
                panic!("Full execution produced invalid pptx");
            }
        }
    }
}

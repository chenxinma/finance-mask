// 端到端 PPT 脱敏测试：扫描 → 策略 → 脱敏 → python-pptx 验证

use std::path::Path;

use finance_mask_core::column_matcher::ColumnMatcher;
use finance_mask_core::executor::Executor;
use finance_mask_core::models::*;
use finance_mask_core::patterns::PatternRegistry;
use finance_mask_core::ppt_reader;
use finance_mask_core::ppt_scanner::PptScanner;
use std::path::PathBuf;

fn config_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("config").leak()
}

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

/// 用 python-pptx 打开文件，验证结构完整性
fn verify_pptx(path: &std::path::Path) -> Result<String, String> {
    let python = python_exe();
    let script = r#"
import sys
try:
    from pptx import Presentation
    prs = Presentation(sys.argv[1])
    count = 0
    for slide in prs.slides:
        for shape in slide.shapes:
            if shape.has_text_frame:
                _ = shape.text_frame.text
                count += 1
            if shape.has_table:
                for row in shape.table.rows:
                    for cell in row.cells:
                        _ = cell.text
                        count += 1
    print(f"VALID:{count}")
except Exception as e:
    print(f"INVALID:{e}", file=sys.stderr)
    sys.exit(1)
"#;
    let output = std::process::Command::new(&python)
        .arg("-c")
        .arg(script)
        .arg(path)
        .output()
        .expect("Failed to execute Python");

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if output.status.success() && stdout.starts_with("VALID") {
        Ok(stdout)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        Err(format!("stdout: {}\nstderr: {}", stdout, stderr))
    }
}

#[test]
fn ppt_e2e_scan_redact_verify() {
    let input = fixture("sample_report.pptx");

    // 1. 解析
    let slides = ppt_reader::parse_pptx(&input).expect("Failed to parse pptx");
    assert!(!slides.is_empty(), "pptx should have slides");

    // 2. 扫描位点（无列头规则，仅全文扫描）
    let scanner = PptScanner::new(
        ColumnMatcher::new(vec![]),
        PatternRegistry::builtin(config_dir()).unwrap(),
    );
    let sites = scanner.scan(&slides);
    assert!(!sites.is_empty(), "should find sensitive sites");

    // 3. 构建策略
    let strategy = Strategy {
        metadata: Metadata::default(),
        column_rules: None,
        sites: sites.clone(),
        global_params: None,
    };

    // 4. 执行脱敏
    let tmp = tempfile::NamedTempFile::with_suffix(".pptx").unwrap();
    let mut exec = Executor::new(strategy, "test_user");
    let report = exec
        .execute(&input, tmp.path(), false)
        .expect("Execute failed");
    assert!(report.success, "execute should succeed");
    assert!(report.processed > 0, "should process at least one site");

    // 5. 用 python-pptx 验证文件有效性
    let result = verify_pptx(tmp.path());
    assert!(
        result.is_ok(),
        "Modified pptx should be valid: {}",
        result.unwrap_err()
    );
}

#[test]
fn ppt_e2e_dry_run() {
    let input = fixture("sample_report.pptx");
    let slides = ppt_reader::parse_pptx(&input).unwrap();
    let scanner = PptScanner::new(
        ColumnMatcher::new(vec![]),
        PatternRegistry::builtin(config_dir()).unwrap(),
    );
    let sites = scanner.scan(&slides);

    let strategy = Strategy {
        metadata: Metadata::default(),
        column_rules: None,
        sites,
        global_params: None,
    };

    let tmp = tempfile::NamedTempFile::with_suffix(".pptx").unwrap();
    let mut exec = Executor::new(strategy, "test_user");
    let report = exec.execute(&input, tmp.path(), true).unwrap();
    assert!(report.dry_run);
    // dry_run 不应创建文件
    assert!(!tmp.path().exists(), "dry_run should not create output file");
}

#[test]
fn ppt_e2e_tianqi_report() {
    let input = fixture("天齐锂业2026年半年度财务报告.pptx");
    if !input.exists() {
        eprintln!("skipping: fixture not found");
        return;
    }

    let slides = ppt_reader::parse_pptx(&input).expect("Failed to parse pptx");
    let scanner = PptScanner::new(
        ColumnMatcher::new(vec![]),
        PatternRegistry::builtin(config_dir()).unwrap(),
    );
    let sites = scanner.scan(&slides);
    if sites.is_empty() {
        return;
    }

    let strategy = Strategy {
        metadata: Metadata::default(),
        column_rules: None,
        sites,
        global_params: None,
    };

    let tmp = tempfile::NamedTempFile::with_suffix(".pptx").unwrap();
    let mut exec = Executor::new(strategy, "test_user");
    let report = exec.execute(&input, tmp.path(), false).expect("Execute failed");
    assert!(report.success);

    let result = verify_pptx(tmp.path());
    assert!(
        result.is_ok(),
        "Modified 天齐锂业 pptx should be valid: {}",
        result.unwrap_err()
    );
}

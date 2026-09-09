// rust/tests/xlsx_integrity.rs
// 最小测试用例：验证 xmlsurgeon 修改后的 xlsx 文件可被 pandas 正常读取

use calamine::Reader;
use finance_mask_core::xmlsurgeon::XmlSurgeon;
use std::path::{Path, PathBuf};
use std::process::Command;

/// 获取 fixture 文件的路径
fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// 获取 Python 解释器路径（优先使用 .venv）
fn python_exe() -> PathBuf {
    // 尝试使用 .venv 中的 Python
    let venv_python = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../.venv/Scripts/python.exe");
    if venv_python.exists() {
        return venv_python;
    }
    // 回退到系统 Python
    PathBuf::from("python")
}

/// 验证 xlsx 文件是否有效（可被 pandas 读取）
fn verify_xlsx_with_python(path: &Path) -> bool {
    let python = python_exe();
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/verify_xlsx.py");
    
    let output = Command::new(&python)
        .arg(&script)
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
fn modify_and_verify_xlsx_integrity() {
    // 1. 打开 fixture 文件
    let input_path = fixture("data1.xlsx");
    let mut surgeon = XmlSurgeon::open(&input_path).expect("Failed to open xlsx");
    
    // 2. 修改一个单元格（假设第一个工作表是 "增员"，A1 是 "编号"）
    //    注意：我们需要知道工作表名和单元格引用
    //    使用 "增员" 工作表，B2 单元格（原值可能是 "南正学"）
    let sheet_name = "增员";
    let cell_ref = "B2";
    let new_value = "测试修改";
    
    surgeon.set_cell_text(sheet_name, cell_ref, new_value)
        .expect("Failed to modify cell");
    
    // 3. 保存到临时文件
    let tmp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let output_path = tmp_dir.path().join("modified.xlsx");
    surgeon.save(&output_path).expect("Failed to save xlsx");
    
    // 4. 验证文件有效性
    assert!(
        verify_xlsx_with_python(&output_path),
        "Modified xlsx file is not valid according to pandas"
    );
    
    // 5. 额外验证：修改后的文件可以被 XmlSurgeon 重新打开
    let reopened = XmlSurgeon::open(&output_path).expect("Failed to reopen modified xlsx");
    assert!(reopened.entry_count() > 0, "Reopened xlsx has no entries");
}

#[test]
fn roundtrip_without_modification_preserves_validity() {
    // 只打开并保存，不修改，检查文件是否仍然有效
    let input_path = fixture("data1.xlsx");
    let surgeon = XmlSurgeon::open(&input_path).expect("Failed to open xlsx");
    
    let tmp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let output_path = tmp_dir.path().join("roundtrip.xlsx");
    surgeon.save(&output_path).expect("Failed to save xlsx");
    
    assert!(
        verify_xlsx_with_python(&output_path),
        "Roundtrip xlsx file is not valid according to pandas"
    );
}

#[test]
fn modify_shared_string_cell() {
    // 修改共享字符串单元格（A2 是共享字符串 "20034692"）
    let input_path = fixture("data1.xlsx");
    let mut surgeon = XmlSurgeon::open(&input_path).expect("Failed to open xlsx");
    
    // 修改 A2 为新的文本值
    surgeon.set_cell_text("增员", "A2", "新编号123")
        .expect("Failed to modify shared string cell");
    
    let tmp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let output_path = tmp_dir.path().join("modified_shared.xlsx");
    surgeon.save(&output_path).expect("Failed to save xlsx");
    
    assert!(
        verify_xlsx_with_python(&output_path),
        "Modified shared string cell produced invalid xlsx"
    );
}

#[test]
fn modify_numeric_cell_to_text() {
    // 修改数字单元格为文本（F2 是数字 17100）
    let input_path = fixture("data1.xlsx");
    let mut surgeon = XmlSurgeon::open(&input_path).expect("Failed to open xlsx");
    
    // 修改 F2 为文本值
    surgeon.set_cell_text("增员", "F2", "文本值")
        .expect("Failed to modify numeric cell");
    
    let tmp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let output_path = tmp_dir.path().join("modified_numeric.xlsx");
    surgeon.save(&output_path).expect("Failed to save xlsx");
    
    assert!(
        verify_xlsx_with_python(&output_path),
        "Modified numeric cell to text produced invalid xlsx"
    );
}

#[test]
fn modify_date_cell() {
    // 修改日期单元格（C2 是日期）
    let input_path = fixture("data1.xlsx");
    let mut surgeon = XmlSurgeon::open(&input_path).expect("Failed to open xlsx");
    
    // 修改 C2 为新的日期文本
    surgeon.set_cell_text("增员", "C2", "2023-01-01")
        .expect("Failed to modify date cell");
    
    let tmp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let output_path = tmp_dir.path().join("modified_date.xlsx");
    surgeon.save(&output_path).expect("Failed to save xlsx");
    
    assert!(
        verify_xlsx_with_python(&output_path),
        "Modified date cell produced invalid xlsx"
    );
}

#[test]
fn numeric_value_preservation() {
    // 验证数值保留逻辑：写入 "22" 到数字单元格应保持数值类型
    let input_path = fixture("data1.xlsx");
    let mut surgeon = XmlSurgeon::open(&input_path).expect("Failed to open xlsx");
    
    // F2 是数字单元格，写入纯数字
    surgeon.set_cell_text("增员", "F2", "99999")
        .expect("Failed to modify numeric cell");
    
    let tmp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let output_path = tmp_dir.path().join("numeric_preserved.xlsx");
    surgeon.save(&output_path).expect("Failed to save xlsx");
    
    // 验证文件有效
    assert!(
        verify_xlsx_with_python(&output_path),
        "Numeric preservation produced invalid xlsx"
    );
    
    // 验证值是数值类型（通过 calamine 检查）
    let mut wb: calamine::Xlsx<_> = calamine::open_workbook(&output_path).unwrap();
    let sheet = wb.worksheet_range("增员").unwrap();
    let f2 = sheet.get_value((1, 5)).unwrap(); // F2 = row 1, col 5 (0-based)
    match f2 {
        calamine::Data::Float(n) => assert_eq!(*n, 99999.0, "F2 should be numeric"),
        other => panic!("F2 should be Float, got: {:?}", other),
    }
}

#[test]
fn modify_empty_cell() {
    // 修改空单元格（假设 K2 是空的）
    let input_path = fixture("data1.xlsx");
    let mut surgeon = XmlSurgeon::open(&input_path).expect("Failed to open xlsx");
    
    // 修改 K2 为文本值
    surgeon.set_cell_text("增员", "K2", "新值")
        .expect("Failed to modify empty cell");
    
    let tmp_dir = tempfile::tempdir().expect("Failed to create temp dir");
    let output_path = tmp_dir.path().join("modified_empty.xlsx");
    surgeon.save(&output_path).expect("Failed to save xlsx");
    
    assert!(
        verify_xlsx_with_python(&output_path),
        "Modified empty cell produced invalid xlsx"
    );
}
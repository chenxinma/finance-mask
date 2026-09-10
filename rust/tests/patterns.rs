// rust/tests/patterns.rs —— 移植 Python scanner/patterns.py 的加载与扫描行为
use std::path::Path;

use finance_mask_core::patterns::PatternRegistry;

fn config_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("config").leak()
}

#[test]
fn builtin_rules_load_and_scan_amount() {
    let reg = PatternRegistry::builtin(config_dir()).unwrap();
    assert!(!reg.rules().is_empty());
    let hits = reg.scan("本项目总投资 1,234,567.89 元");
    assert!(
        hits.iter().any(|(_, m)| m.contains("1,234,567.89")),
        "amount pattern must hit, got: {:?}",
        hits.iter().map(|(r, _)| r.name.clone()).collect::<Vec<_>>()
    );
}

#[test]
fn negative_lookahead_percent_not_matched() {
    let reg = PatternRegistry::builtin(config_dir()).unwrap();
    // "12,345.00%" 是百分比，不应命中金额规则（(?!%) 前瞻语义）
    let hits = reg.scan("增长率 12,345.00% 完成");
    assert!(!hits.iter().any(|(_, m)| m.contains("12,345.00")));
}

#[test]
fn builtin_registry_loads_nine_rules_in_type_map_order() {
    let reg = PatternRegistry::builtin(config_dir()).unwrap();
    let names: Vec<&str> = reg.rules().iter().map(|r| r.name.as_str()).collect();
    // 加载顺序 = patterns.py type_map 顺序（amount→entity→person→account），
    // site_id 生成顺序依赖该次序，差分测试靠它对齐
    assert_eq!(
        names,
        vec![
            "amount_with_unit",
            "amount_with_commas",
            "amount_with_commas_int",
            "amount_no_commas",
            "amount_no_commas_int",
            "entity_chinese",
            "entity_english",
            "bank_account",
            "contract_no",
        ]
    );
}

#[test]
fn builtin_scan_hits_match_python_oracle() {
    // 命中清单与 Python oracle（patterns.py scan_text）逐条核对过（2026-09-06）：
    // 含规则次序、命中次序、命中串完全一致
    let reg = PatternRegistry::builtin(config_dir()).unwrap();
    assert!(reg.scan("").is_empty()); // 空文本直接返回（对应 scan_text 的 if not text）

    let hits = reg.scan("本项目总投资 1,234,567.89 元");
    let hits: Vec<(&str, &str)> = hits
        .iter()
        .map(|(r, m)| (r.name.as_str(), m.as_str()))
        .collect();
    assert_eq!(
        hits,
        vec![
            ("amount_with_commas", "1,234,567.89"),
            ("amount_with_commas_int", "1,234,567"),
        ]
    );

    let hits = reg.scan("客户：天齐锂业股份有限公司；账号 6222 0202 0000 1234 567");
    let hits: Vec<(&str, &str)> = hits
        .iter()
        .map(|(r, m)| (r.name.as_str(), m.as_str()))
        .collect();
    assert_eq!(
        hits,
        vec![
            ("entity_chinese", "天齐锂业股份有限公司"),
            ("bank_account", "6222 0202 0000 1234"),
        ]
    );

    // 百分比不命中（(?!%) 前瞻）
    assert!(reg.scan("增长率 12,345.00% 完成").is_empty());
}

#[test]
fn from_path_overrides_builtin_rules() {
    // 路径覆盖 + type_map 之外的类型整组跳过（对应 patterns.py type_map.get → None → continue）
    let json = r#"{"rules": {
        "amount": [{"name": "small_num", "pattern": "\\d{1,4}", "description": "短数字"}],
        "unknown_type": [{"name": "ignored", "pattern": ".*"}]
    }}"#;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pattern_rules.json");
    std::fs::write(&path, json).unwrap();
    let reg = PatternRegistry::from_path(&path).unwrap();
    assert_eq!(reg.rules().len(), 1);
    assert_eq!(reg.rules()[0].name, "small_num");
    // 短数字过滤（scan_text 的 AMOUNT 分支）：纯数字命中去 , - 后长度 < 4 被丢弃
    let hits = reg.scan("编号 123 与 1234");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].1, "1234");
}

#[test]
fn from_path_missing_file_reports_path() {
    let err = PatternRegistry::from_path(Path::new("no/such/pattern_rules.json")).unwrap_err();
    // 对应 Python FileNotFoundError: 配置文件不存在: <path>
    assert!(
        err.to_string().contains("no/such/pattern_rules.json"),
        "err: {err}"
    );
}

#[test]
fn regex_compile_error_carries_rule_name() {
    let json = r#"{"rules": {"amount": [{"name": "bad_rule", "pattern": "(?!unclosed"}]}}"#;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pattern_rules.json");
    std::fs::write(&path, json).unwrap();
    let err = PatternRegistry::from_path(&path).unwrap_err();
    assert!(err.to_string().contains("bad_rule"), "err: {err}");
}

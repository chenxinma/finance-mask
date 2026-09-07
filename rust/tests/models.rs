// rust/tests/models.rs —— 移植 Python models/site.py、models/strategy.py、scanner/column_matcher.py 的行为断言
use finance_mask_core::column_matcher::ColumnMatcher;
use finance_mask_core::config;
use finance_mask_core::models::*;

/// 构造仅匹配字段相关的最小列头规则（detected_type/action 不参与匹配）
fn rule(match_type: MatchType, pattern: &str, priority: i32) -> ColumnRule {
    ColumnRule {
        match_type,
        pattern: pattern.to_string(),
        action: ActionType::Mask,
        params: None,
        detected_type: DetectedType::Amount,
        priority,
    }
}

/// 命中规则的脱敏动作（供断言用，避免从共享引用中移动字段）
fn hit_action(m: &ColumnMatcher, header: &str) -> Option<ActionType> {
    m.match_header(header).map(|r| r.action.clone())
}

// ---------- Location 校验（对应 site.py model_validator） ----------

#[test]
fn location_excel_requires_sheet_and_cell() {
    let mut loc = Location {
        site_type: SiteType::Excel,
        ..Default::default()
    };
    assert!(validate_location(&loc).is_err());
    loc.sheet = Some("Sheet1".into());
    loc.cell = Some("B5".into());
    assert!(validate_location(&loc).is_ok());
}

#[test]
fn location_ppt_requires_slide_and_shape_id() {
    let mut loc = Location {
        site_type: SiteType::Ppt,
        ..Default::default()
    };
    assert!(validate_location(&loc).is_err());
    loc.slide = Some(1);
    assert!(validate_location(&loc).is_err());
    loc.shape_id = Some("TextBox 3".into());
    assert!(validate_location(&loc).is_ok());
}

// ---------- Strategy YAML 读写闭环（人审产物） ----------

#[test]
fn strategy_yaml_roundtrip_keeps_field_names() {
    let s = r#"
metadata: {version: "1.0", source_file: a.xlsx, generated_at: "2026-09-05T00:00:00", total_sites: 1}
sites:
  - site_id: s1
    location: {type: excel, sheet: S1, cell: A1}
    original_value: "22.12亿元"
    detected_type: amount
    discovered_by: fulltext_scan
    enabled: true
    action: precision
"#;
    let strat: Strategy = serde_yaml::from_str(s).unwrap();
    assert_eq!(strat.sites[0].location.cell.as_deref(), Some("A1"));
    // 回写后仍可解析（人审 YAML 的读写闭环）
    let out = serde_yaml::to_string(&strat).unwrap();
    let strat2: Strategy = serde_yaml::from_str(&out).unwrap();
    assert_eq!(strat, strat2);
}

#[test]
fn strategy_yaml_defaults_version_and_keeps_position_rule() {
    // version 缺省 "1.0"；match_type: position 是 Python 侧死代码，
    // 但规则可携带它，必须能反序列化（2026-09-06 裁决：仅保留 serde 兼容）
    let s = r#"
metadata: {source_file: a.xlsx, generated_at: "2026-09-05T00:00:00", total_sites: 0}
column_rules:
  - match_type: position
    pattern: "3"
    action: mask
    detected_type: amount
    priority: 5
sites: []
"#;
    let strat: Strategy = serde_yaml::from_str(s).unwrap();
    assert_eq!(strat.metadata.version, "1.0");
    let rules = strat.column_rules.as_ref().unwrap();
    assert_eq!(rules[0].match_type, MatchType::Position);
    assert_eq!(rules[0].priority, 5);
    assert_eq!(rules[0].params, None);
    let out = serde_yaml::to_string(&strat).unwrap();
    let strat2: Strategy = serde_yaml::from_str(&out).unwrap();
    assert_eq!(strat, strat2);
}

// ---------- ColumnMatcher（对应 column_matcher.py match()） ----------

#[test]
fn column_matcher_exact_match_strips_header_whitespace() {
    let m = ColumnMatcher::new(vec![rule(MatchType::Exact, "营业收入", 0)]);
    assert_eq!(
        m.match_header("  营业收入 \t").map(|r| r.pattern.as_str()),
        Some("营业收入")
    );
    assert_eq!(m.match_header("营业 收入"), None);
}

#[test]
fn column_matcher_exact_round_precedes_regex() {
    // Python match() 两轮次序：先扫全部 exact，再扫 regex（priority 只在轮内排序）
    let m = ColumnMatcher::new(vec![
        rule(MatchType::Regex, ".*收入.*", 0),
        rule(MatchType::Exact, "营业收入", 9),
    ]);
    assert_eq!(
        m.match_header("营业收入").map(|r| r.pattern.as_str()),
        Some("营业收入")
    );
}

#[test]
fn column_matcher_regex_match_like_python_search() {
    // re.search 语义：模式命中表头任意位置即算匹配
    let m = ColumnMatcher::new(vec![rule(MatchType::Regex, ".+年(上|下)半年", 0)]);
    assert!(m.match_header("2023年上半年").is_some());
    assert!(m.match_header("2023年全年").is_none());
}

#[test]
fn column_matcher_lower_priority_number_wins_within_round() {
    // priority 数字越小优先级越高（Python sort(key=priority) 稳定升序）
    let m = ColumnMatcher::new(vec![
        rule(MatchType::Regex, "金.*", 2),
        rule(MatchType::Regex, ".*额", 1),
    ]);
    assert_eq!(
        m.match_header("合同金额").map(|r| r.pattern.as_str()),
        Some(".*额")
    );
}

#[test]
fn column_matcher_empty_or_blank_header_returns_none() {
    let m = ColumnMatcher::new(vec![rule(MatchType::Exact, "营业收入", 0)]);
    assert!(m.match_header("").is_none());
    assert!(m.match_header("   ").is_none());
}

#[test]
fn column_matcher_position_rules_never_match() {
    // Python 从未实现位置匹配（仅加载不生效，无调用方）；
    // Rust 不超前实现 —— position 规则不参与匹配（2026-09-06 裁决）
    let m = ColumnMatcher::new(vec![rule(MatchType::Position, "营业收入", 0)]);
    assert!(m.match_header("营业收入").is_none());
}

// ---------- 内嵌列头规则配置（config/column_rules.json 单一真源） ----------

#[test]
fn builtin_column_rules_load_and_match() {
    let rules = config::builtin_column_rules().unwrap();
    assert_eq!(rules.len(), 21); // 规则数与 config/column_rules.json 同步维护
    let m = ColumnMatcher::new(rules);
    // 精确规则
    assert_eq!(hit_action(&m, "营业收入"), Some(ActionType::Precision));
    assert_eq!(hit_action(&m, "客户名称"), Some(ActionType::Alias));
    assert_eq!(hit_action(&m, "联系人"), Some(ActionType::MaskName));
    assert_eq!(hit_action(&m, "账号"), Some(ActionType::MaskAccount));
    // 正则规则（.+年(上|下)半年，priority 1）
    assert_eq!(hit_action(&m, "2024年上半年"), Some(ActionType::Perturb));
    // 正则兜底（.*金额.*，priority 2）
    assert_eq!(hit_action(&m, "本期金额"), Some(ActionType::Precision));
    // 未命中
    assert!(m.match_header("序号").is_none());
}

#[test]
fn column_rules_from_path_overrides_builtin() {
    // 路径覆盖：自定义 JSON 替换内嵌默认（含 params:null → None 的 schema 分支）
    let json = r#"{"rules": [
        {"match_type": "exact", "pattern": "资本公积", "action": "mask",
         "params": null, "detected_type": "amount", "priority": 0}
    ]}"#;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("column_rules.json");
    std::fs::write(&path, json).unwrap();
    let rules = config::column_rules_from_path(&path).unwrap();
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].pattern, "资本公积");
    assert_eq!(rules[0].params, None);
    let m = ColumnMatcher::new(rules);
    assert_eq!(hit_action(&m, "资本公积"), Some(ActionType::Mask));
}

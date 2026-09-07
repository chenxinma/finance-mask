//! 策略 YAML 读写 —— 移植自 src/finance_mask/utils/yaml_io.py。
//!
//! - `load_strategy_yaml`：serde_yaml 反序列化（models 的 serde(default) 提供
//!   Python load_strategy 的字段默认值）
//! - `export_strategy_yaml`：手工构建 serde_yaml::Mapping（YamlValue::String
//!   保证引号语义）+ original_value 行尾注释（人审可读性）

use std::path::Path;

use serde_yaml::Value as YamlValue;

use crate::models::{ColumnRule, Site, Strategy};

#[derive(Debug)]
pub enum YamlError {
    Io(std::io::Error),
    Yaml(serde_yaml::Error),
    Validation(String),
}

impl std::fmt::Display for YamlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            YamlError::Io(e) => write!(f, "IO error: {e}"),
            YamlError::Yaml(e) => write!(f, "YAML error: {e}"),
            YamlError::Validation(msg) => write!(f, "校验失败: {msg}"),
        }
    }
}

impl std::error::Error for YamlError {}

impl From<std::io::Error> for YamlError {
    fn from(e: std::io::Error) -> Self {
        YamlError::Io(e)
    }
}

impl From<serde_yaml::Error> for YamlError {
    fn from(e: serde_yaml::Error) -> Self {
        YamlError::Yaml(e)
    }
}

/// 加载 YAML 策略文件。
pub fn load_strategy_yaml(path: &Path) -> Result<Strategy, YamlError> {
    let s = std::fs::read_to_string(path)?;
    let strategy: Strategy = serde_yaml::from_str(&s)?;
    // I8: validate all site locations
    for site in &strategy.sites {
        crate::models::validate_location(&site.location)
            .map_err(|e| YamlError::Validation(format!("位点 {}: {}", site.site_id, e)))?;
    }
    Ok(strategy)
}

/// 导出策略为 YAML 文件（含 original_value 行尾注释）。
///
/// 字段顺序与 Python export_strategy 一致：metadata → column_rules（有才写，
/// 不含 priority）→ sites。Location 省略 falsy 字段（None/空串/slide=0）。
pub fn export_strategy_yaml(
    sites: &[Site],
    source_file: &str,
    source_hash: Option<&str>,
    column_rules: &[ColumnRule],
    output_path: &Path,
) -> Result<(), YamlError> {
    let yaml = build_yaml_string(sites, source_file, source_hash, column_rules)?;
    std::fs::write(output_path, yaml)?;
    Ok(())
}

/// 构建 YAML 字符串（export_strategy_yaml 的核心，便于测试）。
pub fn build_yaml_string(
    sites: &[Site],
    source_file: &str,
    source_hash: Option<&str>,
    column_rules: &[ColumnRule],
) -> Result<String, YamlError> {
    let mut root = serde_yaml::Mapping::new();
    let yk = |s: &str| YamlValue::String(s.into());

    // metadata
    let mut metadata = serde_yaml::Mapping::new();
    metadata.insert(yk("version"), YamlValue::String("1.0".into()));
    metadata.insert(yk("source_file"), YamlValue::String(source_file.into()));
    if let Some(hash) = source_hash {
        metadata.insert(yk("source_hash"), YamlValue::String(hash.into()));
    }
    metadata.insert(
        yk("generated_at"),
        YamlValue::String(
            chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.6f").to_string(),
        ),
    );
    metadata.insert(yk("total_sites"), YamlValue::Number(sites.len().into()));
    root.insert(yk("metadata"), YamlValue::Mapping(metadata));

    // column_rules（不含 priority）
    if !column_rules.is_empty() {
        let mut rules_seq = Vec::new();
        for rule in column_rules {
            let rule_val = serde_yaml::to_value(rule)?;
            if let YamlValue::Mapping(mut m) = rule_val {
                m.remove(&yk("priority"));
                rules_seq.push(YamlValue::Mapping(m));
            }
        }
        root.insert(yk("column_rules"), YamlValue::Sequence(rules_seq));
    }

    // sites（Location falsy 字段省略）
    let mut sites_seq = Vec::new();
    for site in sites {
        sites_seq.push(site_to_yaml(site)?);
    }
    root.insert(yk("sites"), YamlValue::Sequence(sites_seq));

    let mut yaml = serde_yaml::to_string(&YamlValue::Mapping(root))?;

    // eol 注释后处理：为每个 site 的 original_value 行追加
    // `  # {detected_type}: {original_value 前 30 字符}`
    add_eol_comments(&mut yaml, sites);

    Ok(yaml)
}

/// Site → YAML Mapping（字段顺序对齐 Python；Location 省略 falsy 字段）。
fn site_to_yaml(site: &Site) -> Result<YamlValue, YamlError> {
    let yk = |s: &str| YamlValue::String(s.into());
    let mut m = serde_yaml::Mapping::new();

    m.insert(yk("site_id"), YamlValue::String(site.site_id.clone()));

    // location（falsy 省略：None / 空串 / slide==0，对应 Python `if x:`）
    let mut loc = serde_yaml::Mapping::new();
    let site_type_str = match site.location.site_type {
        crate::models::SiteType::Excel => "excel",
        crate::models::SiteType::Ppt => "ppt",
    };
    loc.insert(yk("type"), YamlValue::String(site_type_str.into()));
    if let Some(ref v) = site.location.sheet {
        if !v.is_empty() {
            loc.insert(yk("sheet"), YamlValue::String(v.clone()));
        }
    }
    if let Some(ref v) = site.location.cell {
        if !v.is_empty() {
            loc.insert(yk("cell"), YamlValue::String(v.clone()));
        }
    }
    if let Some(ref v) = site.location.column {
        if !v.is_empty() {
            loc.insert(yk("column"), YamlValue::String(v.clone()));
        }
    }
    if let Some(v) = site.location.slide {
        if v != 0 {
            loc.insert(yk("slide"), YamlValue::Number(v.into()));
        }
    }
    if let Some(ref v) = site.location.shape_id {
        if !v.is_empty() {
            loc.insert(yk("shape_id"), YamlValue::String(v.clone()));
        }
    }
    if let Some(ref v) = site.location.table_location {
        if !v.is_empty() {
            loc.insert(yk("table_location"), YamlValue::String(v.clone()));
        }
    }
    m.insert(yk("location"), YamlValue::Mapping(loc));

    m.insert(
        yk("original_value"),
        YamlValue::String(site.original_value.clone()),
    );
    m.insert(yk("detected_type"), enum_str(&site.detected_type));
    m.insert(yk("discovered_by"), enum_str(&site.discovered_by));
    m.insert(yk("enabled"), YamlValue::Bool(site.enabled));
    m.insert(yk("action"), enum_str(&site.action));
    // params 仅真值（非 None 且非空对象）
    if let Some(ref p) = site.params {
        if !is_empty_json(p) {
            m.insert(yk("params"), json_to_yaml(p));
        }
    }

    Ok(YamlValue::Mapping(m))
}

/// serde_json::Value → serde_yaml::Value
fn json_to_yaml(v: &serde_json::Value) -> YamlValue {
    match v {
        serde_json::Value::Null => YamlValue::Null,
        serde_json::Value::Bool(b) => YamlValue::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                YamlValue::Number(i.into())
            } else if let Some(f) = n.as_f64() {
                YamlValue::Number(f.into())
            } else {
                YamlValue::Null
            }
        }
        serde_json::Value::String(s) => YamlValue::String(s.clone()),
        serde_json::Value::Array(arr) => {
            YamlValue::Sequence(arr.iter().map(json_to_yaml).collect())
        }
        serde_json::Value::Object(map) => {
            let mut m = serde_yaml::Mapping::new();
            for (k, v) in map {
                m.insert(YamlValue::String(k.clone()), json_to_yaml(v));
            }
            YamlValue::Mapping(m)
        }
    }
}

fn is_empty_json(v: &serde_json::Value) -> bool {
    match v {
        serde_json::Value::Null => true,
        serde_json::Value::Object(m) => m.is_empty(),
        serde_json::Value::Array(a) => a.is_empty(),
        _ => false,
    }
}

/// 行尾注释后处理：在 original_value 行追加注释。
/// ruamel eol 格式：`original_value: xxx  # detected_type: 前30字符`
fn add_eol_comments(yaml: &mut String, sites: &[Site]) {
    // 每个 site 一行 original_value；按顺序逐个处理
    // （serde_yaml 输出的缩进是 2 空格，site 在 seq 中：
    //   "- site_id: ...\n    original_value: xxx\n"）
    let mut lines: Vec<String> = yaml.lines().map(|l| l.to_string()).collect();
    let mut site_idx = 0;
    for line in lines.iter_mut() {
        if site_idx >= sites.len() {
            break;
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with("original_value:") {
            let site = &sites[site_idx];
            let dt = match site.detected_type {
                crate::models::DetectedType::Amount => "amount",
                crate::models::DetectedType::Entity => "entity",
                crate::models::DetectedType::Person => "person",
                crate::models::DetectedType::Account => "account",
            };
            let preview: String =
                site.original_value.chars().take(30).collect();
            line.push_str(&format!("  # {dt}: {preview}"));
            site_idx += 1;
        }
    }
    *yaml = lines.join("\n");
    if !yaml.ends_with('\n') {
        yaml.push('\n');
    }
}

/// DetectedType / DiscoveredBy / ActionType → snake_case 字符串
fn enum_str<T: serde::Serialize>(v: &T) -> YamlValue {
    serde_yaml::to_value(v).unwrap_or(YamlValue::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::*;

    fn make_site(site_id: &str, value: &str, dt: DetectedType) -> Site {
        Site {
            site_id: site_id.into(),
            location: Location {
                site_type: SiteType::Excel,
                sheet: Some("Sheet1".into()),
                cell: Some("A2".into()),
                ..Default::default()
            },
            original_value: value.into(),
            detected_type: dt,
            discovered_by: DiscoveredBy::FulltextScan,
            enabled: true,
            action: ActionType::Alias,
            params: Some(serde_json::json!({"prefix": "公司"})),
            redacted_value: None,
        }
    }

    #[test]
    fn export_load_roundtrip() {
        let sites = vec![
            make_site("s1", "天齐锂业股份有限公司", DetectedType::Entity),
            make_site("s2", "1,234,567.89", DetectedType::Amount),
        ];
        let tmp = tempfile::NamedTempFile::with_suffix(".yaml").unwrap();
        export_strategy_yaml(&sites, "test.xlsx", Some("abc123"), &[], tmp.path()).unwrap();

        let loaded = load_strategy_yaml(tmp.path()).unwrap();
        assert_eq!(loaded.sites.len(), 2);
        assert_eq!(loaded.sites[0].site_id, "s1");
        assert_eq!(loaded.sites[0].action, ActionType::Alias);
        assert_eq!(
            loaded.sites[0].params,
            Some(serde_json::json!({"prefix": "公司"}))
        );
        assert_eq!(loaded.metadata.version, "1.0");
        assert_eq!(loaded.metadata.total_sites, 2);
    }

    #[test]
    fn load_missing_fields_gets_defaults() {
        let yaml = r#"
sites:
- site_id: s1
  location: {type: excel, sheet: S1, cell: A1}
  original_value: test
"#;
        let tmp = tempfile::NamedTempFile::with_suffix(".yaml").unwrap();
        std::fs::write(tmp.path(), yaml).unwrap();
        let loaded = load_strategy_yaml(tmp.path()).unwrap();
        assert_eq!(loaded.sites[0].site_id, "s1");
        assert!(loaded.sites[0].enabled); // default true
        assert_eq!(loaded.sites[0].action, ActionType::Mask); // default mask
        assert_eq!(
            loaded.sites[0].detected_type,
            DetectedType::Amount
        ); // default amount
        assert_eq!(
            loaded.sites[0].discovered_by,
            DiscoveredBy::FulltextScan
        ); // default fulltext_scan
        assert_eq!(loaded.sites[0].params, None);
    }

    #[test]
    fn export_has_eol_comment_and_priority_omitted() {
        let rule = ColumnRule {
            match_type: MatchType::Exact,
            pattern: "联系人".into(),
            action: ActionType::MaskName,
            params: None,
            detected_type: DetectedType::Person,
            priority: 5,
        };
        let sites = vec![make_site("s1", "张三丰", DetectedType::Person)];
        let yaml =
            build_yaml_string(&sites, "t.xlsx", None, &[rule]).unwrap();

        // eol 注释存在
        assert!(yaml.contains("original_value: 张三丰  # person: 张三丰"), "{yaml}");
        // priority 不导出
        assert!(!yaml.contains("priority"), "{yaml}");
        // metadata 顺序 + 引号
        assert!(yaml.contains("version: '1.0'"), "{yaml}");
        // falsy location 字段（column 无值）省略
        assert!(!yaml.contains("column:"), "{yaml}");
    }

    #[test]
    fn falsy_location_fields_omitted() {
        let mut site = make_site("s1", "v", DetectedType::Entity);
        site.location.sheet = Some(String::new()); // 空串 → falsy → 省略
        site.location.slide = Some(0); // 0 → falsy → 省略
        site.location.column = None;
        let yaml = build_yaml_string(&[site], "t.xlsx", None, &[]).unwrap();
        assert!(!yaml.contains("sheet:"), "{yaml}");
        assert!(!yaml.contains("slide:"), "{yaml}");
        assert!(!yaml.contains("column:"), "{yaml}");
        // type 仍然输出
        assert!(yaml.contains("type: excel"), "{yaml}");
    }

    #[test]
    fn python_golden_yaml_loads() {
        // Python 导出的真实策略文件能被 Rust 读回
        let golden = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../examples/策略.yaml");
        if !golden.exists() {
            eprintln!("skip: golden {} not found", golden.display());
            return;
        }
        let loaded = load_strategy_yaml(&golden).unwrap();
        assert!(!loaded.metadata.version.is_empty());
    }

    #[test]
    fn load_invalid_location_fails() {
        // Excel site missing sheet
        let yaml = r#"
sites:
- site_id: s1
  location: {type: excel, cell: A1}
  original_value: test
"#;
        let tmp = tempfile::NamedTempFile::with_suffix(".yaml").unwrap();
        std::fs::write(tmp.path(), yaml).unwrap();
        let result = load_strategy_yaml(tmp.path());
        assert!(result.is_err(), "should fail for missing sheet");
        let err = result.unwrap_err().to_string();
        assert!(err.contains("sheet"), "error should mention sheet: {err}");
    }
}

//! 核心数据模型 —— 与 Python pydantic 模型一一对应
//! 移植自 src/finance_mask/models/site.py、models/strategy.py

use serde::{Deserialize, Serialize};

/// 文件类型（site.py SiteType）。
/// `Default` 为 Location 的 derive 所需（默认 Excel），非 Python 侧语义。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SiteType {
    #[default]
    Excel,
    Ppt,
}

/// 敏感数据类型（site.py DetectedType）
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DetectedType {
    Amount,
    Entity,
    Person,
    Account,
}

/// 脱敏动作类型（site.py ActionType）
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ActionType {
    Precision,
    Perturb,
    Mask,
    Alias,
    MaskName,
    MaskAccount,
    DifferentialShift,
    ProportionalScale,
}

/// 发现方式（site.py DiscoveredBy）
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveredBy {
    ColumnRule,
    FulltextScan,
}

/// 列头匹配方式（strategy.py MatchType）
///
/// `Position` 在 Python 侧是死代码（仅加载不生效，无调用方），
/// 此处仅为 serde/YAML 兼容保留，不参与匹配（2026-09-06 裁决）。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MatchType {
    Exact,
    Regex,
    Position,
}

/// 位点位置定位（site.py Location）
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Location {
    #[serde(rename = "type")]
    pub site_type: SiteType,
    pub sheet: Option<String>,
    pub cell: Option<String>, // "B5"
    pub column: Option<String>,
    pub slide: Option<u32>,
    pub shape_id: Option<String>,
    pub table_location: Option<String>, // "R2C3"
}

/// 敏感信息位点（site.py Site）
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Site {
    pub site_id: String,
    pub location: Location,
    pub original_value: String,
    pub detected_type: DetectedType,
    pub discovered_by: DiscoveredBy,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub action: ActionType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redacted_value: Option<String>,
}

fn default_true() -> bool {
    true
}

/// 列头规则（strategy.py ColumnRule）
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ColumnRule {
    pub match_type: MatchType,
    pub pattern: String,
    pub action: ActionType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
    pub detected_type: DetectedType,
    #[serde(default)]
    pub priority: i32,
}

/// 策略文件元数据（strategy.py Metadata）
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Metadata {
    #[serde(default = "default_version")]
    pub version: String,
    pub source_file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,
    pub generated_at: String, // ISO 8601
    pub total_sites: usize,
}

fn default_version() -> String {
    "1.0".into()
}

/// 脱敏策略（strategy.py Strategy）
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Strategy {
    pub metadata: Metadata,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column_rules: Option<Vec<ColumnRule>>,
    pub sites: Vec<Site>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global_params: Option<serde_json::Value>,
}

/// Location 的 type 与定位字段校验（对应 pydantic model_validator）
pub fn validate_location(loc: &Location) -> Result<(), String> {
    match loc.site_type {
        SiteType::Excel => {
            if loc.sheet.is_none() || loc.cell.is_none() {
                Err("type=excel 时必须包含 sheet 和 cell 字段".into())
            } else {
                Ok(())
            }
        }
        SiteType::Ppt => {
            if loc.slide.is_none() || loc.shape_id.is_none() {
                Err("type=ppt 时必须包含 slide 和 shape_id 字段".into())
            } else {
                Ok(())
            }
        }
    }
}

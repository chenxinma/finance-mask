//! PPT 扫描器 —— 移植自 src/finance_mask/scanner/ppt_scanner.py。
//!
//! 本任务（5-b）覆盖文本框与备注扫描；表格扫描（`_scan_table`）在后续任务 5-c 实现。
//! 输入是 `crate::ppt_reader` 解析出的结构化 `Vec<PptSlide>`，而非原始 .pptx 路径。

use fancy_regex::Regex;

use crate::column_matcher::ColumnMatcher;
use crate::excel_scanner::get_default_action;
use crate::models::{ActionType, DetectedType, DiscoveredBy, Location, Site, SiteType};
use crate::patterns::PatternRegistry;
use crate::ppt_reader::{PptSlide, ShapeKind};

/// PPT 文件扫描器（对应 Python PPTScanner）。
pub struct PptScanner {
    matcher: ColumnMatcher,
    patterns: PatternRegistry,
}

impl PptScanner {
    pub fn new(matcher: ColumnMatcher, patterns: PatternRegistry) -> Self {
        Self { matcher, patterns }
    }

    /// 扫描解析出的幻灯片，返回所有检测到的位点。
    ///
    /// 对应 Python `scan` + `_scan_slide` + `_scan_text_frame` + `_scan_notes`
    /// （本任务只覆盖 TextBox 与 notes；Table 在 5-c）。
    pub fn scan(&self, slides: &[PptSlide]) -> Vec<Site> {
        let mut sites = Vec::new();

        for slide in slides {
            let slide_idx = slide.slide_idx as u32;

            // --- 文本框：全文正则扫描 ---
            for shape in &slide.shapes {
                if shape.kind != ShapeKind::TextBox {
                    continue;
                }
                for paragraph in &shape.paragraphs {
                    let full_text = paragraph.trim();
                    if full_text.is_empty() {
                        continue;
                    }

                    for (rule, matched_text) in self.patterns.scan(full_text) {
                        let (action, params) = get_default_action(&rule.detected_type);
                        let site_id = format!("slide_{}_{}", slide.slide_idx, shape.id);
                        sites.push(make_site(
                            site_id,
                            slide_idx,
                            shape.name.clone(),
                            None,
                            matched_text,
                            rule.detected_type.clone(),
                            DiscoveredBy::FulltextScan,
                            action,
                            params,
                        ));
                    }
                }
            }

            // --- 备注：短文本列头规则，否则全文扫描 ---
            for paragraph in &slide.notes {
                let full_text = paragraph.trim();
                if full_text.is_empty() {
                    continue;
                }

                let site_id = format!("notes_{}", slide.slide_idx);
                let location_slide = slide_idx;
                let shape_id = "notes".to_string();
                let table_location = Some("notes".to_string());

                // 短文本才尝试列头匹配（Python `len(full_text) <= 20`）
                if full_text.chars().count() <= 20 {
                    if let Some(rule) = self.matcher.match_header(full_text) {
                        if let Some(value) =
                            extract_value_from_text(full_text, &rule.detected_type)
                        {
                            // 与 Task 3 一致：仅当 action == Mask 且无 params 时回退默认动作
                            let (action, params) =
                                if rule.action != ActionType::Mask || rule.params.is_some() {
                                    (rule.action.clone(), rule.params.clone())
                                } else {
                                    get_default_action(&rule.detected_type)
                                };

                            sites.push(make_site(
                                site_id.clone(),
                                location_slide,
                                shape_id.clone(),
                                table_location.clone(),
                                value,
                                rule.detected_type.clone(),
                                DiscoveredBy::ColumnRule,
                                action,
                                params,
                            ));
                        }
                        // Python：命中列头规则后 `continue`，无论是否提取到值
                        continue;
                    }
                }

                for (rule, matched_text) in self.patterns.scan(full_text) {
                    let (action, params) = get_default_action(&rule.detected_type);
                    sites.push(make_site(
                        site_id.clone(),
                        location_slide,
                        shape_id.clone(),
                        table_location.clone(),
                        matched_text,
                        rule.detected_type.clone(),
                        DiscoveredBy::FulltextScan,
                        action,
                        params,
                    ));
                }
            }
        }

        sites
    }
}

/// 从短文本中按 detected_type 提取值（对应 Python `_extract_value_from_text`）。
///
/// Python 版本接收 `rule`，但只使用 `rule.detected_type`；此处直接接收该类型。
/// 无匹配时返回原文（与 Python 的默认 `return text` 一致）。
pub fn extract_value_from_text(text: &str, detected_type: &DetectedType) -> Option<String> {
    if text.is_empty() {
        return None;
    }

    match detected_type {
        DetectedType::Amount => {
            // 先带单位，再纯数字
            for pattern in [
                r"(-?\d{1,3}(?:,\d{3})*(?:\.\d+)?\s*(?:万亿|亿|百万|万|千)?元?)",
                r"(-?\d+(?:,\d{3})*(?:\.\d+)?)",
            ] {
                if let Some(value) = regex_first_group(pattern, text) {
                    return Some(value);
                }
            }
            Some(text.to_string())
        }
        DetectedType::Entity | DetectedType::Person => Some(text.to_string()),
        DetectedType::Account => {
            for pattern in [
                r"(\d{4}[\s-]?\d{4}[\s-]?\d{4}[\s-]?\d{4,})",
                r"([A-Za-z]{2,4}[-/]?\d{4}[-/]?\d{4,})",
            ] {
                if let Some(value) = regex_first_group(pattern, text) {
                    return Some(value);
                }
            }
            Some(text.to_string())
        }
    }
}

/// 对 `text` 执行正则，返回第一个捕获组（trim 后）。
fn regex_first_group(pattern: &str, text: &str) -> Option<String> {
    let re = Regex::new(pattern).ok()?;
    match re.captures(text) {
        Ok(Some(caps)) => caps.get(1).map(|m| m.as_str().trim().to_string()),
        _ => None,
    }
}

/// 构造一个 PPT 位点（type=Ppt，slide/shape_id 定位）。
#[allow(clippy::too_many_arguments)]
fn make_site(
    site_id: String,
    slide_idx: u32,
    shape_id: String,
    table_location: Option<String>,
    value: String,
    detected_type: DetectedType,
    discovered_by: DiscoveredBy,
    action: ActionType,
    params: Option<serde_json::Value>,
) -> Site {
    Site {
        site_id,
        location: Location {
            site_type: SiteType::Ppt,
            slide: Some(slide_idx),
            shape_id: Some(shape_id),
            table_location,
            ..Location::default()
        },
        original_value: value,
        detected_type,
        discovered_by,
        enabled: true,
        action,
        params,
        redacted_value: None,
    }
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ColumnRule, MatchType};
    use crate::ppt_reader::PptShape;

    fn make_scanner(rules: Vec<ColumnRule>) -> PptScanner {
        PptScanner::new(
            ColumnMatcher::new(rules),
            PatternRegistry::builtin().unwrap(),
        )
    }

    fn make_slide(slide_idx: usize, shapes: Vec<PptShape>, notes: Vec<String>) -> PptSlide {
        PptSlide {
            slide_idx,
            shapes,
            notes,
        }
    }

    fn make_textbox(id: &str, name: &str, paragraphs: Vec<&str>) -> PptShape {
        PptShape {
            id: id.to_string(),
            name: name.to_string(),
            kind: ShapeKind::TextBox,
            paragraphs: paragraphs.into_iter().map(String::from).collect(),
            table_rows: vec![],
        }
    }

    #[test]
    fn textbox_scan_finds_amount() {
        let slide = make_slide(
            1,
            vec![make_textbox(
                "2",
                "TextBox 1",
                vec!["本项目总投资 1,234,567.89 元"],
            )],
            vec![],
        );

        let scanner = make_scanner(vec![]);
        let sites = scanner.scan(&[slide]);

        let amount = sites
            .iter()
            .find(|s| {
                s.detected_type == DetectedType::Amount
                    && s.original_value.contains("1,234,567.89")
            });
        assert!(
            amount.is_some(),
            "should detect amount in textbox, got: {:?}",
            sites
        );

        let site = amount.unwrap();
        assert_eq!(site.discovered_by, DiscoveredBy::FulltextScan);
        assert_eq!(site.location.site_type, SiteType::Ppt);
        assert_eq!(site.location.slide, Some(1));
        assert_eq!(site.location.shape_id.as_deref(), Some("TextBox 1"));
        assert_eq!(site.location.table_location, None);
        assert_eq!(site.site_id, "slide_1_2");
    }

    #[test]
    fn notes_scan_short_text_column_rule() {
        // 备注短文本 + 列头规则（Regex 规则 "联系人" 命中 "联系人 张三"，
        // 与 Python 正则匹配使用 re.search 语义一致）。
        let slide = make_slide(
            1,
            vec![],
            vec!["联系人 张三".to_string()],
        );

        let scanner = make_scanner(vec![ColumnRule {
            match_type: MatchType::Regex,
            pattern: "联系人".to_string(),
            action: ActionType::MaskName,
            params: None,
            detected_type: DetectedType::Person,
            priority: 0,
        }]);

        let sites = scanner.scan(&[slide]);

        let column_site = sites
            .iter()
            .find(|s| s.discovered_by == DiscoveredBy::ColumnRule);
        assert!(
            column_site.is_some(),
            "should have a ColumnRule site from short notes, got: {:?}",
            sites
        );

        let site = column_site.unwrap();
        assert_eq!(site.detected_type, DetectedType::Person);
        assert_eq!(site.original_value, "联系人 张三");
        assert_eq!(site.action, ActionType::MaskName);
        assert_eq!(site.location.site_type, SiteType::Ppt);
        assert_eq!(site.location.slide, Some(1));
        assert_eq!(site.location.shape_id.as_deref(), Some("notes"));
        assert_eq!(site.location.table_location.as_deref(), Some("notes"));
        assert_eq!(site.site_id, "notes_1");
    }

    #[test]
    fn notes_long_text_fulltext() {
        let slide = make_slide(
            1,
            vec![],
            vec!["这是一段很长很长的备注文本超过二十个字符用来测试全文扫描路径".to_string()],
        );

        let scanner = make_scanner(vec![]);
        let sites = scanner.scan(&[slide]);

        // 长文本不进入列头匹配路径；当前内置规则无命中则为空。
        // 若有命中，discovered_by 必须是 FulltextScan。
        assert!(
            sites
                .iter()
                .all(|s| s.discovered_by == DiscoveredBy::FulltextScan),
            "long notes should only produce FulltextScan sites (or none), got: {:?}",
            sites
        );
    }

    #[test]
    fn empty_slide_returns_empty() {
        let slide = make_slide(1, vec![], vec![]);
        let scanner = make_scanner(vec![]);
        let sites = scanner.scan(&[slide]);
        assert!(sites.is_empty());
    }
}

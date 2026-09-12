//! rust/src/pipeline.rs —— CLI / UI 共享编排层。
//!
//! 从 main.rs 原样下沉：扫描/生成策略/执行脱敏的编排逻辑，行为逐行一致。
//! 差异仅两点：
//! - verbose 打印改为返回结构化报告（打印/展示由调用方负责，输出文本不变）
//! - 错误类型收敛为 String（编排层，底层模块仍保留类型化错误）

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::column_matcher::ColumnMatcher;
use crate::config;
use crate::excel_scanner::{parse_workbook, ExcelScanner};
use crate::executor::Executor;
use crate::models::{ColumnRule, DiscoveredBy, Site, Strategy};
use crate::patterns::PatternRegistry;
use crate::yaml_io;

pub type Result<T> = std::result::Result<T, String>;

/// generate 结果摘要（CLI 据此还原 verbose 输出，UI 直接序列化展示）
#[derive(Serialize, Debug)]
pub struct GenerateReport {
    /// 成功扫描的文件数
    pub scanned: usize,
    /// 因格式不支持被跳过的文件
    pub skipped_files: Vec<String>,
    pub total_sites: usize,
    pub total_column_rules: usize,
    /// 导出的策略文件路径
    pub output: String,
}

/// redact 单文件结果
#[derive(Serialize, Debug)]
pub struct FileReport {
    pub input: String,
    /// 输出文件路径（dry-run 时为拟输出路径）
    pub output: String,
    /// ok / skipped_exists / failed
    pub status: String,
    pub processed: usize,
    pub skipped: usize,
    pub errors: usize,
    /// status=failed 时的错误信息
    pub message: Option<String>,
}

/// redact 结果摘要
#[derive(Serialize, Debug, Default)]
pub struct RedactReport {
    pub files: Vec<FileReport>,
    pub total_processed: usize,
    pub total_errors: usize,
}

/// redact 入参（借用形式，避免 8 参数函数）
pub struct RedactOptions<'a> {
    pub input: &'a Path,
    pub strategy_path: Option<&'a Path>,
    pub default_policy: bool,
    pub output_dir: &'a Path,
    pub operator: &'a str,
    pub force: bool,
    pub dry_run: bool,
    /// 字典文件解析目录（CLI 传 exe 同级 config/，UI 传仓库 config/）
    pub config_dir: &'a Path,
}

/// 递归收集 xlsx/pptx 文件（单文件直接返回）
pub fn walk_files(input: &Path) -> Vec<PathBuf> {
    if input.is_file() {
        return vec![input.to_path_buf()];
    }
    let mut result = Vec::new();
    collect_files(input, &mut result);
    result
}

fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_files(&path, out);
            } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                let ext = ext.to_ascii_lowercase();
                if ext == "xlsx" || ext == "pptx" {
                    out.push(path);
                }
            }
        }
    }
}

/// 扫描单个文件。generate 与 default-policy 共用。
/// Ok(None) = 扩展名不支持（调用方跳过）；Err = 配置/解析失败（调用方中止，与 CLI 原行为一致）。
fn scan_file(
    file_path: &Path,
    config_dir: &Path,
) -> Result<Option<(Vec<Site>, Vec<ColumnRule>)>> {
    let ext = file_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let matcher = ColumnMatcher::new(config::builtin_column_rules().map_err(|e| e.to_string())?);
    let patterns =
        PatternRegistry::builtin(config_dir).map_err(|e| e.to_string())?;

    if ext == "xlsx" {
        let sheets = parse_workbook(file_path).map_err(|e| e.to_string())?;
        let scanner = ExcelScanner::new(matcher, patterns);
        let sites = scanner.scan(&sheets);
        let rules: Vec<ColumnRule> = scanner
            .get_column_rules(&sheets)
            .values()
            .flatten()
            .cloned()
            .collect();
        Ok(Some((sites, rules)))
    } else if ext == "pptx" {
        let slides = crate::ppt_reader::parse_pptx(file_path).map_err(|e| e.to_string())?;
        let scanner = crate::ppt_scanner::PptScanner::new(matcher, patterns);
        let sites = scanner.scan(&slides);
        Ok(Some((sites, vec![])))
    } else {
        Ok(None)
    }
}

/// 内置默认策略（对应 Python main.py _generate_default_strategy：扫描 → 全部位点启用）
pub fn generate_default_strategy(input: &Path, config_dir: &Path) -> Result<Strategy> {
    let (sites, column_rules) = scan_file(input, config_dir)?
        .ok_or_else(|| {
            format!(
                "不支持的文件格式: .{}",
                input
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase()
            )
        })?;
    let sites = deduplicate_sites(sites);
    let source_hash = sha256_file(input)?;

    Ok(Strategy {
        metadata: crate::models::Metadata {
            version: "1.0".into(),
            source_file: input
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .into(),
            source_hash: Some(source_hash),
            generated_at: chrono::Local::now()
                .format("%Y-%m-%dT%H:%M:%S%.6f")
                .to_string(),
            total_sites: sites.len(),
        },
        column_rules: if column_rules.is_empty() {
            None
        } else {
            Some(column_rules)
        },
        sites,
        global_params: None,
    })
}

/// 扫描输入（文件或文件夹）→ 导出策略 YAML
pub fn generate(input: &Path, output: &Path, config_dir: &Path) -> Result<GenerateReport> {
    let files = walk_files(input);
    if files.is_empty() {
        return Err(format!("未找到 xlsx/pptx 文件: {}", input.display()));
    }

    let mut all_sites: Vec<Site> = Vec::new();
    let mut all_column_rules: Vec<ColumnRule> = Vec::new();
    let mut scanned = 0usize;
    let mut skipped_files: Vec<String> = Vec::new();

    for file_path in &files {
        match scan_file(file_path, config_dir)? {
            Some((sites, column_rules)) => {
                scanned += 1;
                all_sites.extend(sites);
                all_column_rules.extend(column_rules);
            }
            None => {
                skipped_files.push(file_path.display().to_string());
            }
        }
    }

    // Dedup column_rules by pattern (matches Python: seen_patterns set)
    {
        let mut seen = std::collections::HashSet::new();
        let mut unique = Vec::new();
        for rule in all_column_rules {
            if seen.insert(rule.pattern.clone()) {
                unique.push(rule);
            }
        }
        all_column_rules = unique;
    }

    // Deduplicate sites
    let all_sites = deduplicate_sites(all_sites);

    // Export YAML
    let source_file = input
        .file_name()
        .unwrap_or_default()
        .to_str()
        .unwrap_or("")
        .to_string();
    let source_hash = sha256_file(input)?;

    yaml_io::export_strategy_yaml(
        &all_sites,
        &source_file,
        Some(&source_hash),
        &all_column_rules,
        output,
    )
    .map_err(|e| e.to_string())?;

    Ok(GenerateReport {
        scanned,
        skipped_files,
        total_sites: all_sites.len(),
        total_column_rules: all_column_rules.len(),
        output: output.display().to_string(),
    })
}

/// 执行脱敏（文件或文件夹批量）
pub fn redact(opts: &RedactOptions) -> Result<RedactReport> {
    let files = walk_files(opts.input);
    let mut report = RedactReport::default();
    if files.is_empty() {
        return Ok(report);
    }

    if !opts.dry_run {
        std::fs::create_dir_all(opts.output_dir).map_err(|e| e.to_string())?;
    }

    for file_path in &files {
        // 输出文件名：{stem}_脱敏{ext}
        let stem = file_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("output");
        let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let output_file = opts.output_dir.join(format!("{stem}_脱敏.{ext}"));

        if !opts.dry_run && output_file.exists() && !opts.force {
            report.files.push(FileReport {
                input: file_path.display().to_string(),
                output: output_file.display().to_string(),
                status: "skipped_exists".into(),
                processed: 0,
                skipped: 0,
                errors: 0,
                message: None,
            });
            continue;
        }

        // 加载策略
        let strategy: Strategy = if let Some(sp) = opts.strategy_path {
            yaml_io::load_strategy_yaml(sp).map_err(|e| e.to_string())?
        } else if opts.default_policy {
            generate_default_strategy(file_path, opts.config_dir)?
        } else {
            return Err("需要 --strategy 或 --default-policy".into());
        };

        let mut executor = Executor::new(strategy, opts.operator);
        match executor.execute(file_path, &output_file, opts.dry_run) {
            Ok(r) => {
                report.total_processed += r.processed;
                report.total_errors += r.errors;
                report.files.push(FileReport {
                    input: file_path.display().to_string(),
                    output: output_file.display().to_string(),
                    status: "ok".into(),
                    processed: r.processed,
                    skipped: r.skipped,
                    errors: r.errors,
                    message: None,
                });
            }
            Err(e) => {
                report.total_errors += 1;
                report.files.push(FileReport {
                    input: file_path.display().to_string(),
                    output: output_file.display().to_string(),
                    status: "failed".into(),
                    processed: 0,
                    skipped: 0,
                    errors: 1,
                    message: Some(e.to_string()),
                });
            }
        }
    }

    Ok(report)
}

/// 计算文件的 SHA-256 哈希值（hex 小写）
fn sha256_file(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(|e| e.to_string())?;
    Ok(format!("{:x}", hasher.finalize()))
}

/// 去重位点：同一 location + original_value 的位点合并，敏感类型取并集。
/// 对应 Python `main.py _deduplicate_sites`。
fn deduplicate_sites(sites: Vec<Site>) -> Vec<Site> {
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut result: Vec<Site> = Vec::with_capacity(sites.len());

    for site in sites {
        let key = format!("{}:{}", site.location, site.original_value);
        if let Some(&idx) = seen.get(&key) {
            let existing = &result[idx];
            if site.detected_type != existing.detected_type
                && site.discovered_by == DiscoveredBy::ColumnRule
            {
                result[idx] = site; // replace in place (preserves insertion order)
            }
        } else {
            seen.insert(key, result.len());
            result.push(site);
        }
    }

    result
}

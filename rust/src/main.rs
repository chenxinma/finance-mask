// rust/src/main.rs —— CLI 入口，generate 子命令
use std::collections::HashMap;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use serde_yaml::Value as YamlValue;

use finance_mask_core::column_matcher::ColumnMatcher;
use finance_mask_core::config;
use finance_mask_core::excel_scanner::{ExcelScanner, parse_workbook};
use finance_mask_core::models::{ColumnRule, DiscoveredBy, Site};
use finance_mask_core::patterns::PatternRegistry;

#[derive(Parser)]
#[command(name = "finance-mask", version = finance_mask_core::VERSION)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// 生成脱敏策略文件（扫描 Excel/PPT → YAML）
    Generate {
        /// 输入文件路径（.xlsx / .pptx）
        #[arg(short, long)]
        input: PathBuf,
        /// 输出策略文件路径（.yaml）
        #[arg(short, long)]
        output: PathBuf,
        /// 显示详细日志
        #[arg(short, long, default_value_t = false)]
        verbose: bool,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Generate {
            input,
            output,
            verbose,
        }) => {
            if let Err(e) = run_generate(&input, &output, verbose) {
                eprintln!("错误: {e}");
                std::process::exit(1);
            }
        }
        None => {
            println!("finance-mask (rust) {}", finance_mask_core::VERSION);
        }
    }
}

fn run_generate(
    input: &PathBuf,
    output: &PathBuf,
    verbose: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if verbose {
        eprintln!("开始扫描文件: {}", input.display());
    }

    // Step 1: Parse + scan
    let sheets = parse_workbook(input)?;
    let matcher = ColumnMatcher::new(config::builtin_column_rules()?);
    let patterns = PatternRegistry::builtin()?;
    let scanner = ExcelScanner::new(matcher, patterns);

    let sites = scanner.scan(&sheets);

    // Collect column_rules across sheets
    let column_rules_map = scanner.get_column_rules(&sheets);
    let mut all_column_rules: Vec<ColumnRule> = column_rules_map
        .values()
        .flatten()
        .cloned()
        .collect();

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

    // Step 2: Deduplicate sites
    let sites = deduplicate_sites(sites);

    if verbose {
        eprintln!(
            "扫描完成: {} 个位点, {} 条列规则",
            sites.len(),
            all_column_rules.len()
        );
    }

    // Step 3: Build YAML value manually (skip priority in column_rules,
    // matching Python export_strategy which never writes priority)
    let source_file = input
        .file_name()
        .unwrap_or_default()
        .to_str()
        .unwrap_or("")
        .to_string();

    let source_hash = sha256_file(input)?;

    let mut root = serde_yaml::Mapping::new();

    // metadata
    let mut metadata = serde_yaml::Mapping::new();
    metadata.insert(
        YamlValue::String("version".into()),
        YamlValue::String("1.0".into()),
    );
    metadata.insert(
        YamlValue::String("source_file".into()),
        YamlValue::String(source_file),
    );
    metadata.insert(
        YamlValue::String("source_hash".into()),
        YamlValue::String(source_hash),
    );
    metadata.insert(
        YamlValue::String("generated_at".into()),
        YamlValue::String(
            chrono::Local::now()
                .format("%Y-%m-%dT%H:%M:%S%.f")
                .to_string(),
        ),
    );
    metadata.insert(
        YamlValue::String("total_sites".into()),
        YamlValue::Number(sites.len().into()),
    );
    root.insert(
        YamlValue::String("metadata".into()),
        YamlValue::Mapping(metadata),
    );

    // column_rules (skip priority, matching Python export)
    if !all_column_rules.is_empty() {
        let mut rules_seq = Vec::new();
        for rule in &all_column_rules {
            let rule_val = serde_yaml::to_value(rule)?;
            // Remove "priority" key from the mapping
            if let YamlValue::Mapping(mut m) = rule_val {
                m.remove(&YamlValue::String("priority".into()));
                rules_seq.push(YamlValue::Mapping(m));
            } else {
                rules_seq.push(rule_val);
            }
        }
        root.insert(
            YamlValue::String("column_rules".into()),
            YamlValue::Sequence(rules_seq),
        );
    }

    // sites
    let sites_yaml = serde_yaml::to_value(&sites)?;
    root.insert(YamlValue::String("sites".into()), sites_yaml);

    // Step 4: Write YAML
    let yaml = serde_yaml::to_string(&YamlValue::Mapping(root))?;
    std::fs::write(output, yaml)?;

    if verbose {
        eprintln!("策略文件已导出: {}", output.display());
    }

    Ok(())
}

/// 计算文件的 SHA-256 哈希值（hex 小写）
fn sha256_file(path: &PathBuf) -> Result<String, Box<dyn std::error::Error>> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

/// 去重位点：同一 location + original_value 的位点合并，敏感类型取并集。
/// 对应 Python `main.py _deduplicate_sites`。
fn deduplicate_sites(sites: Vec<Site>) -> Vec<Site> {
    let mut seen: HashMap<String, usize> = HashMap::new(); // key → index in result
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

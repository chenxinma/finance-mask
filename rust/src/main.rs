// rust/src/main.rs —— CLI 入口：generate + redact 子命令
use std::path::PathBuf;

use clap::{Parser, Subcommand};

use finance_mask_core::column_matcher::ColumnMatcher;
use finance_mask_core::config;
use finance_mask_core::excel_scanner::{ExcelScanner, parse_workbook};
use finance_mask_core::executor::Executor;
use finance_mask_core::models::{ColumnRule, DiscoveredBy, Site, Strategy};
use finance_mask_core::patterns::PatternRegistry;
use finance_mask_core::yaml_io;

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
    /// 执行脱敏（按策略文件或默认策略）
    Redact {
        /// 输入文件或文件夹路径
        #[arg(short, long)]
        input: PathBuf,
        /// 策略文件路径
        #[arg(short, long)]
        strategy: Option<PathBuf>,
        /// 使用内置默认策略
        #[arg(long, default_value_t = false)]
        default_policy: bool,
        /// 输出目录
        #[arg(short, long)]
        output: PathBuf,
        /// 操作人ID（默认取系统用户名）
        #[arg(long)]
        operator: Option<String>,
        /// 显示详细日志
        #[arg(short, long, default_value_t = false)]
        verbose: bool,
        /// 覆盖已存在的输出文件
        #[arg(short, long, default_value_t = false)]
        force: bool,
        /// 仅预览拟修改位点，不实际执行
        #[arg(long, default_value_t = false)]
        dry_run: bool,
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
        Some(Commands::Redact {
            input,
            strategy,
            default_policy,
            output,
            operator,
            verbose,
            force,
            dry_run,
        }) => {
            let operator = operator.unwrap_or_else(whoami);
            if let Err(e) = run_redact(
                &input,
                strategy.as_deref(),
                default_policy,
                &output,
                &operator,
                verbose,
                force,
                dry_run,
            ) {
                eprintln!("错误: {e}");
                std::process::exit(1);
            }
        }
        None => {
            println!("finance-mask (rust) {}", finance_mask_core::VERSION);
        }
    }
}

fn whoami() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "unknown".into())
}

/// 递归收集 xlsx/pptx 文件（单文件直接返回）
fn walk_files(input: &std::path::Path) -> Vec<PathBuf> {
    if input.is_file() {
        return vec![input.to_path_buf()];
    }
    let mut result = Vec::new();
    collect_files(input, &mut result);
    result
}

fn collect_files(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
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

fn run_redact(
    input: &std::path::Path,
    strategy_path: Option<&std::path::Path>,
    default_policy: bool,
    output_dir: &std::path::Path,
    operator: &str,
    verbose: bool,
    force: bool,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let files = walk_files(input);
    if files.is_empty() {
        eprintln!("未找到 xlsx/pptx 文件: {}", input.display());
        return Ok(());
    }

    if !dry_run {
        std::fs::create_dir_all(output_dir)?;
    }

    if verbose {
        eprintln!("开始脱敏处理: {} 个文件", files.len());
    }

    let mut total_processed = 0usize;
    let mut total_errors = 0usize;

    for file_path in &files {
        // 输出文件名：{stem}_脱敏{ext}
        let stem = file_path.file_stem().and_then(|s| s.to_str()).unwrap_or("output");
        let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");
        let output_file = output_dir.join(format!("{stem}_脱敏.{ext}"));

        if !dry_run && output_file.exists() && !force {
            eprintln!("跳过（已存在，用 -f 覆盖）: {}", output_file.display());
            continue;
        }

        // 加载策略
        let strategy: Strategy = if let Some(sp) = strategy_path {
            yaml_io::load_strategy_yaml(sp)?
        } else if default_policy {
            generate_default_strategy(file_path)?
        } else {
            eprintln!("错误: 需要 --strategy 或 --default-policy");
            return Err("缺少策略".into());
        };

        let mut executor = Executor::new(strategy, operator);
        match executor.execute(file_path, &output_file, dry_run) {
            Ok(report) => {
                total_processed += report.processed;
                total_errors += report.errors;
                if verbose {
                    eprintln!(
                        "  {}: 处理 {} / 跳过 {} / 错误 {}",
                        file_path.display(),
                        report.processed,
                        report.skipped,
                        report.errors
                    );
                }
            }
            Err(e) => {
                eprintln!("文件处理失败 {}: {e}", file_path.display());
                total_errors += 1;
            }
        }
    }

    if verbose {
        eprintln!("脱敏完成: 处理 {total_processed} 个位点, {total_errors} 个错误");
    }
    Ok(())
}

/// 内置默认策略（对应 Python main.py _generate_default_strategy：扫描 → 全部位点启用）
fn generate_default_strategy(
    input: &std::path::Path,
) -> Result<Strategy, Box<dyn std::error::Error>> {
    let ext = input
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let matcher = ColumnMatcher::new(config::builtin_column_rules()?);
    let patterns = PatternRegistry::builtin()?;

    let (sites, column_rules) = if ext == "xlsx" {
        let sheets = parse_workbook(input)?;
        let scanner = ExcelScanner::new(matcher, patterns);
        let sites = scanner.scan(&sheets);
        let rules: Vec<ColumnRule> = scanner
            .get_column_rules(&sheets)
            .values()
            .flatten()
            .cloned()
            .collect();
        (sites, rules)
    } else if ext == "pptx" {
        let slides = finance_mask_core::ppt_reader::parse_pptx(input)?;
        let scanner = finance_mask_core::ppt_scanner::PptScanner::new(matcher, patterns);
        let sites = scanner.scan(&slides);
        (sites, vec![])
    } else {
        return Err(format!("不支持的文件格式: .{ext}").into());
    };

    let sites = deduplicate_sites(sites);
    let source_hash = sha256_file(input)?;

    Ok(Strategy {
        metadata: finance_mask_core::models::Metadata {
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

fn run_generate(
    input: &PathBuf,
    output: &PathBuf,
    verbose: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if verbose {
        eprintln!("开始扫描文件: {}", input.display());
    }

    let files = walk_files(input);
    if files.is_empty() {
        return Err(format!("未找到 xlsx/pptx 文件: {}", input.display()).into());
    }

    let mut all_sites: Vec<Site> = Vec::new();
    let mut all_column_rules: Vec<ColumnRule> = Vec::new();

    for file_path in &files {
        let ext = file_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        let matcher = ColumnMatcher::new(config::builtin_column_rules()?);
        let patterns = PatternRegistry::builtin()?;

        let (sites, column_rules) = if ext == "xlsx" {
            let sheets = parse_workbook(file_path)?;
            let scanner = ExcelScanner::new(matcher, patterns);
            let sites = scanner.scan(&sheets);
            let rules: Vec<ColumnRule> = scanner
                .get_column_rules(&sheets)
                .values()
                .flatten()
                .cloned()
                .collect();
            (sites, rules)
        } else if ext == "pptx" {
            let slides = finance_mask_core::ppt_reader::parse_pptx(file_path)?;
            let scanner = finance_mask_core::ppt_scanner::PptScanner::new(matcher, patterns);
            let sites = scanner.scan(&slides);
            (sites, vec![])
        } else {
            if verbose {
                eprintln!("跳过不支持的文件: {}", file_path.display());
            }
            continue;
        };

        all_sites.extend(sites);
        all_column_rules.extend(column_rules);
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

    if verbose {
        eprintln!(
            "扫描完成: {} 个位点, {} 条列规则",
            all_sites.len(),
            all_column_rules.len()
        );
    }

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
    )?;

    if verbose {
        eprintln!("策略文件已导出: {}", output.display());
    }

    Ok(())
}

/// 计算文件的 SHA-256 哈希值（hex 小写）
fn sha256_file(path: &std::path::Path) -> Result<String, Box<dyn std::error::Error>> {
    use sha2::{Digest, Sha256};
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

/// 去重位点：同一 location + original_value 的位点合并，敏感类型取并集。
/// 对应 Python `main.py _deduplicate_sites`。
fn deduplicate_sites(sites: Vec<Site>) -> Vec<Site> {
    let mut seen: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
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

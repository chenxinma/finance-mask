// rust/src/main.rs —— CLI 入口：generate + redact 子命令。
// 编排逻辑已下沉至 lib::pipeline（CLI 与 Tauri UI 共用），本文件只负责
// clap 解析与 verbose/stderr 输出（文本与下沉前逐行一致）。
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

use finance_mask_core::pipeline;

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

/// 获取 exe 所在目录下的 config 目录
fn exe_config_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let exe = std::env::current_exe()?;
    Ok(exe.parent().unwrap_or(Path::new(".")).join("config"))
}

fn run_generate(
    input: &PathBuf,
    output: &PathBuf,
    verbose: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if verbose {
        eprintln!("开始扫描文件: {}", input.display());
    }

    let config_dir = exe_config_dir()?;
    let report = pipeline::generate(input, output, &config_dir)?;

    if verbose {
        for f in &report.skipped_files {
            eprintln!("跳过不支持的文件: {f}");
        }
        eprintln!(
            "扫描完成: {} 个位点, {} 条列规则",
            report.total_sites, report.total_column_rules
        );
        eprintln!("策略文件已导出: {}", output.display());
    }

    Ok(())
}

fn run_redact(
    input: &Path,
    strategy_path: Option<&Path>,
    default_policy: bool,
    output_dir: &Path,
    operator: &str,
    verbose: bool,
    force: bool,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let files = pipeline::walk_files(input);
    if files.is_empty() {
        eprintln!("未找到 xlsx/pptx 文件: {}", input.display());
        return Ok(());
    }

    if verbose {
        eprintln!("开始脱敏处理: {} 个文件", files.len());
    }

    let config_dir = exe_config_dir()?;
    let report = pipeline::redact(&pipeline::RedactOptions {
        input,
        strategy_path,
        default_policy,
        output_dir,
        operator,
        force,
        dry_run,
        config_dir: &config_dir,
    })?;

    for f in &report.files {
        match f.status.as_str() {
            "skipped_exists" => eprintln!("跳过（已存在，用 -f 覆盖）: {}", f.output),
            "failed" => eprintln!(
                "文件处理失败 {}: {}",
                f.input,
                f.message.as_deref().unwrap_or("")
            ),
            "ok" if verbose => eprintln!(
                "  {}: 处理 {} / 跳过 {} / 错误 {}",
                f.input, f.processed, f.skipped, f.errors
            ),
            _ => {}
        }
    }

    if verbose {
        eprintln!(
            "脱敏完成: 处理 {} 个位点, {} 个错误",
            report.total_processed, report.total_errors
        );
    }
    Ok(())
}

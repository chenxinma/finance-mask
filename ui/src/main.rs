// ui/src/main.rs —— Tauri 后端：finance_mask_core lib 的薄客户端。
// 文件选择用 tauri-plugin-dialog（Rust 侧，前端零插件依赖）；
// 所有文件 IO 在本进程完成，前端只收 JSON。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::{Path, PathBuf};

use finance_mask_core::models::Strategy;
use finance_mask_core::{pipeline, yaml_io};
use tauri_plugin_dialog::DialogExt;

/// 字典/配置目录：exe 同级 config/（部署形态）优先，
/// 回退编译期仓库 config/（开发形态，cargo run 直接可用）。
fn config_dir() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        let d = exe.parent().unwrap_or(Path::new(".")).join("config");
        if d.join("entity_dict.txt").exists() {
            return d;
        }
    }
    // ponytail: 编译期路径兜底，正式分发时改为 bundle resources
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config")
}

fn whoami() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "unknown".into())
}

/// 打开文件选择对话框。kind: input=xlsx/pptx, yaml=策略文件
#[tauri::command]
async fn pick_file(app: tauri::AppHandle, kind: String) -> Result<Option<String>, String> {
    let fb = app.dialog().file();
    let fb = match kind.as_str() {
        "input" => fb.add_filter("Office 文件", &["xlsx", "pptx"]),
        "yaml" => fb.add_filter("策略文件", &["yaml", "yml"]),
        _ => fb,
    };
    Ok(fb.blocking_pick_file().map(|p| p.simplified().to_string()))
}

#[tauri::command]
async fn pick_dir(app: tauri::AppHandle) -> Result<Option<String>, String> {
    Ok(app
        .dialog()
        .file()
        .blocking_pick_folder()
        .map(|p| p.simplified().to_string()))
}

/// 保存对话框（选择策略 YAML 输出路径）
#[tauri::command]
async fn save_file(
    app: tauri::AppHandle,
    default_name: String,
) -> Result<Option<String>, String> {
    Ok(app
        .dialog()
        .file()
        .add_filter("策略文件", &["yaml", "yml"])
        .set_file_name(default_name)
        .blocking_save_file()
        .map(|p| p.simplified().to_string()))
}

#[tauri::command]
fn generate(input: String, output: String) -> Result<pipeline::GenerateReport, String> {
    pipeline::generate(Path::new(&input), Path::new(&output), &config_dir())
}

#[tauri::command]
fn load_strategy(path: String) -> Result<Strategy, String> {
    yaml_io::load_strategy_yaml(Path::new(&path)).map_err(|e| e.to_string())
}

/// 保存策略：与 CLI 同一导出通道（build_yaml_string，含行尾注释）
#[tauri::command]
fn save_strategy(path: String, strategy: Strategy) -> Result<(), String> {
    export(&strategy, Path::new(&path))
}

/// YAML 预览（不落盘）
#[tauri::command]
fn preview_yaml(strategy: Strategy) -> Result<String, String> {
    yaml_io::build_yaml_string(
        &strategy.sites,
        &strategy.metadata.source_file,
        strategy.metadata.source_hash.as_deref(),
        strategy.column_rules.as_deref().unwrap_or(&[]),
    )
    .map_err(|e| e.to_string())
}

fn export(strategy: &Strategy, path: &Path) -> Result<(), String> {
    yaml_io::export_strategy_yaml(
        &strategy.sites,
        &strategy.metadata.source_file,
        strategy.metadata.source_hash.as_deref(),
        strategy.column_rules.as_deref().unwrap_or(&[]),
        path,
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn redact(
    input: String,
    strategy_path: Option<String>,
    default_policy: bool,
    output_dir: String,
    operator: Option<String>,
    force: bool,
    dry_run: bool,
) -> Result<pipeline::RedactReport, String> {
    let operator = operator.filter(|s| !s.trim().is_empty()).unwrap_or_else(whoami);
    pipeline::redact(&pipeline::RedactOptions {
        input: Path::new(&input),
        strategy_path: strategy_path.as_deref().map(Path::new),
        default_policy,
        output_dir: Path::new(&output_dir),
        operator: &operator,
        force,
        dry_run,
        config_dir: &config_dir(),
    })
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            pick_file,
            pick_dir,
            save_file,
            generate,
            load_strategy,
            save_strategy,
            preview_yaml,
            redact
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

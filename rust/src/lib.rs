// rust/src/lib.rs —— 模块声明随 Phase 逐步填充
pub mod audit;
pub mod classify;
pub mod engine;
pub mod column_matcher;
pub mod config;
pub mod excel_scanner;
pub mod header_finder;
pub mod models;
pub mod patterns;
pub mod watermark;
pub mod xmlsurgeon;

pub const VERSION: &str = "0.1.0";

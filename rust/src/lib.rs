// rust/src/lib.rs —— 模块声明随 Phase 逐步填充
pub mod audit;
pub mod classify;
pub mod engine;
pub mod column_matcher;
pub mod config;
pub mod excel_scanner;
pub mod executor;
pub mod header_finder;
pub mod models;
pub mod patterns;
pub mod ppt_reader;
pub mod ppt_scanner;
pub mod ppt_writer;
pub mod watermark;
pub mod yaml_io;
pub mod xmlsurgeon;

pub const VERSION: &str = "0.1.0";

//! 审计日志生成与导出
//! 移植自 src/finance_mask/audit/logger.py —— ChangeRecord / AuditLogger / AuditLogExporter

use std::fmt;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// 单个位点的修改记录（对应 Python ChangeRecord.to_dict()）
#[derive(Debug, Clone, Serialize)]
pub struct ChangeRecord {
    pub site_id: String,
    pub location: serde_json::Value, // Location 序列化后仅保留非 None 字段
    pub original: String,
    pub redacted: String,
    pub action: String, // ActionType 的 snake_case 字符串
}

/// 审计日志错误
#[derive(Debug)]
pub enum AuditError {
    Io {
        path: String,
        source: std::io::Error,
    },
    Json {
        source: serde_json::Error,
    },
}

impl fmt::Display for AuditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuditError::Io { path, source } => write!(f, "文件读写失败: {path}: {source}"),
            AuditError::Json { source } => write!(f, "JSON 序列化失败: {source}"),
        }
    }
}

impl std::error::Error for AuditError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AuditError::Io { source, .. } => Some(source),
            AuditError::Json { source } => Some(source),
        }
    }
}

/// 审计日志记录器（对应 Python AuditLogger）
#[derive(Debug)]
pub struct AuditLogger {
    pub source_file: String,
    pub source_path: String,
    pub output_file: String,
    pub operator: String,
    pub timestamp: String, // ISO 8601
    pub changes: Vec<ChangeRecord>,
    pub errors: Vec<serde_json::Value>,
}

impl AuditLogger {
    pub fn new(source_file: &str, output_file: &str, operator: &str) -> Self {
        AuditLogger {
            source_file: source_file.to_string(),
            source_path: source_file.to_string(),
            output_file: output_file.to_string(),
            operator: operator.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            changes: Vec::new(),
            errors: Vec::new(),
        }
    }

    /// 记录一个位点的修改。`location` 由调用方从 models::Location 构建
    /// （仅保留非 None 字段，等价于 Python model_dump(exclude_none=True)）。
    pub fn log_change(
        &mut self,
        site_id: &str,
        location: serde_json::Value,
        original: &str,
        redacted: &str,
        action: &str,
    ) {
        self.changes.push(ChangeRecord {
            site_id: site_id.to_string(),
            location,
            original: original.to_string(),
            redacted: redacted.to_string(),
            action: action.to_string(),
        });
    }

    /// 记录一个错误（对应 Python log_error，键序 site_id/error/timestamp）
    pub fn log_error(&mut self, site_id: &str, error: &str) {
        self.errors.push(serde_json::json!({
            "site_id": site_id,
            "error": error,
            "timestamp": chrono::Utc::now().to_rfc3339(),
        }));
    }

    pub fn total_changes(&self) -> usize {
        self.changes.len()
    }

    pub fn total_errors(&self) -> usize {
        self.errors.len()
    }
}

/// 导出审计日志为 JSON 文件（对应 Python AuditLogExporter.export）
///
/// 顶层键序与 Python dict 插入顺序一致（file → source_path → output_file →
/// operator → timestamp → file_hash → total_changes → total_errors →
/// changes → errors），依赖 serde_json 的 preserve_order 特性保证。
pub fn export(
    logger: &AuditLogger,
    output_path: &Path,
    file_hash: Option<&str>,
) -> Result<PathBuf, AuditError> {
    let log_data = serde_json::json!({
        "file": logger.source_file,
        "source_path": logger.source_path,
        "output_file": logger.output_file,
        "operator": logger.operator,
        "timestamp": logger.timestamp,
        "file_hash": file_hash,
        "total_changes": logger.total_changes(),
        "total_errors": logger.total_errors(),
        "changes": logger.changes,
        "errors": logger.errors,
    });

    let json = serde_json::to_string_pretty(&log_data).map_err(|source| AuditError::Json {
        source,
    })?;
    std::fs::write(output_path, json).map_err(|source| AuditError::Io {
        path: output_path.display().to_string(),
        source,
    })?;
    Ok(output_path.to_path_buf())
}

/// 计算文件的 SHA-256 十六进制摘要（对应 Python AuditLogExporter.compute_file_hash）
pub fn compute_file_hash(path: &Path) -> Result<String, AuditError> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let mut file = std::fs::File::open(path).map_err(|source| AuditError::Io {
        path: path.display().to_string(),
        source,
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 4096];
    loop {
        let n = file.read(&mut buffer).map_err(|source| AuditError::Io {
            path: path.display().to_string(),
            source,
        })?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOP_LEVEL_KEYS: [&str; 10] = [
        "file",
        "source_path",
        "output_file",
        "operator",
        "timestamp",
        "file_hash",
        "total_changes",
        "total_errors",
        "changes",
        "errors",
    ];

    fn sample_location() -> serde_json::Value {
        serde_json::json!({
            "type": "excel",
            "sheet": "Sheet1",
            "cell": "A10",
            "column": "单据信息_单据编号",
        })
    }

    #[test]
    fn change_record_roundtrip() {
        let record = ChangeRecord {
            site_id: "Sheet1_A10".into(),
            location: sample_location(),
            original: "CK-A001".into(),
            redacted: "CK-**01".into(),
            action: "mask_account".into(),
        };
        let value = serde_json::to_value(&record).unwrap();
        let keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(|s| s.as_str())
            .collect();
        assert_eq!(
            keys,
            vec!["site_id", "location", "original", "redacted", "action"]
        );
    }

    #[test]
    fn audit_logger_records_changes_and_errors() {
        let mut logger = AuditLogger::new("仓库入库1.xlsx", "仓库入库1_脱敏.xlsx", "ma");
        logger.log_change("Sheet1_A10", sample_location(), "CK-A001", "CK-**01", "mask_account");
        logger.log_change("Sheet1_A11", sample_location(), "CK-A002", "CK-**02", "mask_account");
        logger.log_error("Sheet1_E10", "解析失败");
        assert_eq!(logger.total_changes(), 2);
        assert_eq!(logger.total_errors(), 1);
    }

    #[test]
    fn export_matches_python_format() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("审计日志.json");
        let mut logger = AuditLogger::new("仓库入库1.xlsx", "仓库入库1_脱敏.xlsx", "ma");
        logger.log_change("Sheet1_A10", sample_location(), "CK-A001", "CK-**01", "mask_account");

        export(&logger, &out, Some("deadbeef")).unwrap();
        let text = std::fs::read_to_string(&out).unwrap();

        // 顶层键序与 Python dict 顺序一致（在原始字符串中按序出现）
        let mut last = 0usize;
        for key in TOP_LEVEL_KEYS {
            let needle = format!("\"{key}\"");
            let pos = text.find(&needle).unwrap_or_else(|| panic!("缺少顶层键: {key}"));
            assert!(pos >= last, "顶层键 {key} 顺序错误");
            last = pos;
        }

        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        let change = &value["changes"][0];
        let change_keys: Vec<&str> = change
            .as_object()
            .unwrap()
            .keys()
            .map(|s| s.as_str())
            .collect();
        assert_eq!(
            change_keys,
            vec!["site_id", "location", "original", "redacted", "action"]
        );
        assert_eq!(value["file_hash"], "deadbeef");
    }

    #[test]
    fn compute_file_hash_sha256() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hash.bin");
        std::fs::write(&path, b"hello world").unwrap();
        let hash = compute_file_hash(&path).unwrap();
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn golden_sample_top_level_keys_match() {
        let golden = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../examples/仓库入库1_日志.json"
        );
        let text = std::fs::read_to_string(golden).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        let keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(|s| s.as_str())
            .collect();
        assert_eq!(keys, TOP_LEVEL_KEYS.to_vec());
    }
}

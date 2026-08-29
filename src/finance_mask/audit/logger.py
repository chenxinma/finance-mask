"""审计日志生成与导出"""
from __future__ import annotations

import hashlib
import json
import logging
from datetime import datetime
from pathlib import Path
from typing import Optional

from ..models.site import Site

logger = logging.getLogger(__name__)


class ChangeRecord:
    """单个位点的修改记录"""

    def __init__(
        self,
        site_id: str,
        location: dict,
        original: str,
        redacted: str,
        action: str,
    ):
        self.site_id = site_id
        self.location = location
        self.original = original
        self.redacted = redacted
        self.action = action

    def to_dict(self) -> dict:
        return {
            "site_id": self.site_id,
            "location": self.location,
            "original": self.original,
            "redacted": self.redacted,
            "action": self.action,
        }


class AuditLogger:
    """审计日志记录器"""

    def __init__(
        self,
        source_file: str,
        output_file: str,
        operator: str,
        source_path: Optional[str] = None,
    ):
        self.source_file = source_file
        self.source_path = source_path or source_file
        self.output_file = output_file
        self.operator = operator
        self.timestamp = datetime.now().isoformat()
        self.changes: list[ChangeRecord] = []
        self.errors: list[dict] = []

    def log_change(
        self,
        site: Site,
        original_value: str,
        redacted_value: str,
        action: str,
    ) -> None:
        """记录一个位点的修改"""
        change = ChangeRecord(
            site_id=site.site_id,
            location=site.location.model_dump(exclude_none=True),
            original=original_value,
            redacted=redacted_value,
            action=action,
        )
        self.changes.append(change)

    def log_error(self, site_id: str, error: str) -> None:
        """记录一个错误"""
        self.errors.append({
            "site_id": site_id,
            "error": error,
            "timestamp": datetime.now().isoformat(),
        })

    def get_total_changes(self) -> int:
        """获取总修改数"""
        return len(self.changes)

    def get_total_errors(self) -> int:
        """获取总错误数"""
        return len(self.errors)


class AuditLogExporter:
    """审计日志导出器"""

    @staticmethod
    def export(
        logger: AuditLogger,
        output_path: Path,
        file_hash: Optional[str] = None,
    ) -> Path:
        """
        导出审计日志为 JSON 文件
        
        Args:
            logger: 审计日志记录器
            output_path: 输出文件路径
            file_hash: 脱敏后文件的 SHA-256 哈希值
            
        Returns:
            日志文件路径
        """
        log_data = {
            "file": logger.source_file,
            "source_path": logger.source_path,
            "output_file": logger.output_file,
            "operator": logger.operator,
            "timestamp": logger.timestamp,
            "file_hash": file_hash,
            "total_changes": logger.get_total_changes(),
            "total_errors": logger.get_total_errors(),
            "changes": [change.to_dict() for change in logger.changes],
            "errors": logger.errors,
        }

        # 写入 JSON 文件
        with open(output_path, "w", encoding="utf-8") as f:
            json.dump(log_data, f, ensure_ascii=False, indent=2)

        # 使用 logging 模块记录日志
        import logging
        logging.getLogger(__name__).info(f"审计日志已导出: {output_path}")
        return output_path

    @staticmethod
    def compute_file_hash(filepath: Path) -> str:
        """计算文件的 SHA-256 哈希值"""
        sha256_hash = hashlib.sha256()
        with open(filepath, "rb") as f:
            for byte_block in iter(lambda: f.read(4096), b""):
                sha256_hash.update(byte_block)
        return sha256_hash.hexdigest()

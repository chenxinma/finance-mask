"""名称脱敏实现"""
from __future__ import annotations

import re
from typing import Optional


class NameRedactor:
    """名称脱敏器（机构名/人名）"""

    # 机构名计数器
    _entity_counter: dict[str, int] = {}
    # 实体名映射表：确保相同实体名始终映射到同一个别名
    _entity_mapping: dict[str, str] = {}

    @classmethod
    def alias(
        cls,
        value: str,
        prefix: str = "公司",
        mapping: Optional[dict[str, str]] = None,
    ) -> str:
        """
        机构名代号替换
        
        Args:
            value: 原始机构名
            prefix: 代号前缀（如 "公司"、"机构"）
            mapping: 自定义映射表（可选）
            
        Returns:
            代号替换后的字符串
        """
        if not value:
            return value

        # 使用自定义映射
        if mapping and value in mapping:
            return mapping[value]

        # 检查是否已在全局映射表中
        if value in cls._entity_mapping:
            return cls._entity_mapping[value]

        # 自动生成代号
        if prefix not in cls._entity_counter:
            cls._entity_counter[prefix] = 0

        cls._entity_counter[prefix] += 1
        counter = cls._entity_counter[prefix]

        # 使用字母编号：A, B, ..., Z, AA, AB, ...
        letter = cls._number_to_letter(counter)

        result = f"[{prefix}{letter}]"
        
        # 存入全局映射表
        cls._entity_mapping[value] = result
        
        return result

    @classmethod
    def mask_name(
        cls,
        value: str,
        keep_first: int = 1,
    ) -> str:
        """
        姓名遮掩
        
        保留姓氏，其余替换为 *
        
        Args:
            value: 原始姓名
            keep_first: 保留前几个字（默认保留姓氏）
            
        Returns:
            遮掩后的姓名
        """
        if not value:
            return value

        value = value.strip()
        if len(value) <= keep_first:
            return value

        # 处理中英文混合姓名
        # 中文姓名
        if re.match(r"[\u4e00-\u9fa5]+", value):
            return value[:keep_first] + "*" * (len(value) - keep_first)

        # 英文姓名（保留首字母）
        parts = value.split()
        if len(parts) > 1:
            # First Last 格式
            masked_parts = [parts[0][:keep_first] + "*"] + ["*" for _ in parts[1:]]
            return " ".join(masked_parts)

        # 单个单词
        return value[:keep_first] + "*" * (len(value) - keep_first)

    @classmethod
    def reset_counter(cls) -> None:
        """重置计数器和映射表（用于新的脱敏任务）"""
        cls._entity_counter.clear()
        cls._entity_mapping.clear()

    @classmethod
    def get_entity_mapping(cls) -> dict[str, str]:
        """获取当前实体映射表（用于调试或导出）"""
        return cls._entity_mapping.copy()

    @classmethod
    def _number_to_letter(cls, n: int) -> str:
        """将数字转换为字母编号（1=A, 2=B, ..., 26=Z, 27=AA, ...）"""
        result = ""
        while n > 0:
            n, remainder = divmod(n - 1, 26)
            result = chr(65 + remainder) + result
        return result

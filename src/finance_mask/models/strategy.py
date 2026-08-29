"""策略数据模型定义"""
from __future__ import annotations

from enum import Enum
from typing import Optional

from pydantic import BaseModel, Field

from .site import DetectedType, ActionType, Site


class MatchType(str, Enum):
    """列头匹配方式"""
    EXACT = "exact"        # 精确匹配
    REGEX = "regex"        # 正则匹配
    POSITION = "position"  # 位置匹配（第N列）


class ColumnRule(BaseModel):
    """列头规则"""
    match_type: MatchType = Field(description="匹配方式")
    pattern: str = Field(description="匹配模式（精确值或正则表达式）")
    action: ActionType = Field(description="脱敏动作标识")
    params: Optional[dict] = Field(None, description="脱敏参数")
    detected_type: DetectedType = Field(description="敏感类型")
    priority: int = Field(0, description="优先级（数字越小优先级越高）")


class Metadata(BaseModel):
    """策略文件元数据"""
    version: str = Field("1.0", description="策略文件格式版本")
    source_file: str = Field(description="原始文件名")
    source_hash: Optional[str] = Field(None, description="原始文件 SHA-256")
    generated_at: str = Field(description="策略生成时间（ISO 8601）")
    total_sites: int = Field(description="位点总数")


class Strategy(BaseModel):
    """脱敏策略"""
    metadata: Metadata = Field(description="文件元数据")
    column_rules: Optional[list[ColumnRule]] = Field(None, description="全局列头规则")
    sites: list[Site] = Field(description="所有位点列表")
    global_params: Optional[dict] = Field(None, description="全局参数（各位点可继承或覆盖）")

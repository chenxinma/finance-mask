"""位点数据模型定义"""
from __future__ import annotations

from enum import Enum
from typing import Optional

from pydantic import BaseModel, Field, model_validator


class SiteType(str, Enum):
    """文件类型"""
    EXCEL = "excel"
    PPT = "ppt"


class DetectedType(str, Enum):
    """敏感数据类型"""
    AMOUNT = "amount"      # 金额
    ENTITY = "entity"      # 机构/公司
    PERSON = "person"      # 人员
    ACCOUNT = "account"    # 账号/合同号


class ActionType(str, Enum):
    """脱敏动作类型"""
    PRECISION = "precision"    # 降低精度
    PERTURB = "perturb"        # 随机扰动
    MASK = "mask"              # 遮掩
    ALIAS = "alias"            # 代号替换（机构名）
    MASK_NAME = "mask_name"    # 姓名遮掩
    MASK_ACCOUNT = "mask_account"  # 账号遮掩
    DIFFERENTIAL_SHIFT = "differential_shift"  # 差分偏移（保持差值）
    PROPORTIONAL_SCALE = "proportional_scale"  # 比例缩放（保持比例）


class DiscoveredBy(str, Enum):
    """发现方式"""
    COLUMN_RULE = "column_rule"      # 列头定位
    FULLTEXT_SCAN = "fulltext_scan"  # 全文扫描


class Location(BaseModel):
    """位点位置定位"""
    type: SiteType = Field(description="文件类型：excel 或 ppt")
    sheet: Optional[str] = Field(None, description="Excel 工作表名")
    cell: Optional[str] = Field(None, description="Excel 单元格坐标（如 B5）")
    column: Optional[str] = Field(None, description="列名（表头文本）")
    slide: Optional[int] = Field(None, description="PPT 幻灯片编号（从1开始）")
    shape_id: Optional[str] = Field(None, description="PPT 形状ID或名称")
    table_location: Optional[str] = Field(None, description="PPT 表格中的行列位置（如 R2C3）")

    @model_validator(mode="after")
    def validate_location(self) -> "Location":
        """校验 type 与对应的定位字段必须匹配"""
        if self.type == SiteType.EXCEL:
            if not self.sheet or not self.cell:
                raise ValueError("type=excel 时必须包含 sheet 和 cell 字段")
        elif self.type == SiteType.PPT:
            if not self.slide or not self.shape_id:
                raise ValueError("type=ppt 时必须包含 slide 和 shape_id 字段")
        return self

    def __str__(self) -> str:
        if self.type == SiteType.EXCEL:
            return f"[Excel] {self.sheet}!{self.cell}"
        else:
            base = f"[PPT] 幻灯片{self.slide} 形状{self.shape_id}"
            if self.table_location:
                base += f" {self.table_location}"
            return base


class Site(BaseModel):
    """敏感信息位点"""
    site_id: str = Field(description="全局唯一标识符")
    location: Location = Field(description="文件内精确定位")
    original_value: str = Field(description="脱敏前原始值")
    detected_type: DetectedType = Field(description="敏感类型")
    discovered_by: DiscoveredBy = Field(description="发现方式")
    enabled: bool = Field(True, description="是否执行脱敏")
    action: ActionType = Field(description="脱敏动作标识")
    params: Optional[dict] = Field(None, description="脱敏参数")
    redacted_value: Optional[str] = Field(None, description="脱敏后的值（执行阶段填充）")

    def to_yaml_dict(self) -> dict:
        """转换为 YAML 可序列化的字典"""
        data = {
            "site_id": self.site_id,
            "location": self.location.model_dump(exclude_none=True),
            "original_value": self.original_value,
            "detected_type": self.detected_type.value,
            "discovered_by": self.discovered_by.value,
            "enabled": self.enabled,
            "action": self.action.value,
        }
        if self.params:
            data["params"] = self.params
        return data

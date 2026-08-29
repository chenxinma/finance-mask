"""PPT 文件扫描实现"""
from __future__ import annotations

import logging
from pathlib import Path
from typing import Optional

from pptx import Presentation
from pptx.util import Inches

from ..models.site import (
    Site,
    Location,
    SiteType,
    DetectedType,
    ActionType,
    DiscoveredBy,
)
from .base import BaseScanner
from .column_matcher import ColumnMatcher
from .patterns import PatternRegistry

logger = logging.getLogger(__name__)


class PPTScanner(BaseScanner):
    """PPT 文件扫描器"""

    def __init__(
        self,
        column_matcher: Optional[ColumnMatcher] = None,
        pattern_registry: Optional[PatternRegistry] = None,
    ):
        self.column_matcher = column_matcher or ColumnMatcher()
        self.pattern_registry = pattern_registry or PatternRegistry()

    def scan(self, filepath: Path) -> list[Site]:
        """扫描 PPT 文件"""
        sites: list[Site] = []

        try:
            prs = Presentation(str(filepath))
        except Exception as e:
            logger.error(f"无法打开 PPT 文件 {filepath}: {e}")
            return sites

        for slide_idx, slide in enumerate(prs.slides, start=1):
            slide_sites = self._scan_slide(filepath, slide_idx, slide)
            sites.extend(slide_sites)

        return sites

    def _scan_slide(self, filepath: Path, slide_idx: int, slide) -> list[Site]:
        """扫描单张幻灯片"""
        sites: list[Site] = []

        for shape in slide.shapes:
            shape_id = shape.shape_id
            shape_name = shape.name

            # 扫描表格
            if shape.has_table:
                table_sites = self._scan_table(
                    slide_idx, shape_id, shape_name, shape.table
                )
                sites.extend(table_sites)

            # 扫描文本框
            if shape.has_text_frame:
                text_sites = self._scan_text_frame(
                    slide_idx, shape_id, shape_name, shape.text_frame
                )
                sites.extend(text_sites)

            # 扫描图表（如果有）
            if shape.has_chart:
                chart_sites = self._scan_chart(
                    slide_idx, shape_id, shape_name, shape.chart
                )
                sites.extend(chart_sites)

        return sites

    def _scan_table(
        self, slide_idx: int, shape_id: int, shape_name: str, table
    ) -> list[Site]:
        """扫描表格"""
        sites: list[Site] = []
        header_row_idx = 0  # 假设第一行是表头
        header_map: dict[int, str] = {}

        # 识别表头
        if table.rows:
            header_row = table.rows[header_row_idx]
            for col_idx, cell in enumerate(header_row.cells):
                header_text = cell.text.strip()
                if header_text:
                    rule = self.column_matcher.match(header_text)
                    if rule:
                        header_map[col_idx] = header_text

        # 扫描数据行
        for row_idx, row in enumerate(table.rows):
            if row_idx == header_row_idx:
                continue  # 跳过表头行

            for col_idx, cell in enumerate(row.cells):
                cell_text = cell.text.strip()
                if not cell_text:
                    continue

                # 列头定位扫描
                if col_idx in header_map:
                    header_name = header_map[col_idx]
                    rule = self.column_matcher.match(header_name)
                    if rule:
                        site = self._create_site_from_column(
                            slide_idx=slide_idx,
                            shape_id=shape_id,
                            shape_name=shape_name,
                            table_location=f"R{row_idx + 1}C{col_idx + 1}",
                            value=cell_text,
                            rule=rule,
                        )
                        if site:
                            sites.append(site)
                            continue

                # 全文正则扫描（兜底）
                pattern_results = self.pattern_registry.scan_text(cell_text)
                for matched_value, detected_type, rule_name in pattern_results:
                    site = self._create_site_from_pattern(
                        slide_idx=slide_idx,
                        shape_id=shape_id,
                        shape_name=shape_name,
                        value=matched_value,
                        detected_type=detected_type,
                    )
                    if site:
                        sites.append(site)

        return sites

    def _scan_text_frame(
        self, slide_idx: int, shape_id: int, shape_name: str, text_frame
    ) -> list[Site]:
        """扫描文本框"""
        sites: list[Site] = []

        for paragraph in text_frame.paragraphs:
            full_text = paragraph.text.strip()
            if not full_text:
                continue

            # 全文正则扫描
            pattern_results = self.pattern_registry.scan_text(full_text)
            for matched_value, detected_type, rule_name in pattern_results:
                site = self._create_site_from_pattern(
                    slide_idx=slide_idx,
                    shape_id=shape_id,
                    shape_name=shape_name,
                    value=matched_value,
                    detected_type=detected_type,
                )
                if site:
                    sites.append(site)

        return sites

    def _scan_chart(
        self, slide_idx: int, shape_id: int, shape_name: str, chart
    ) -> list[Site]:
        """扫描图表（数据标签等）"""
        sites: list[Site] = []

        # 图表数据通常在图表对象中，这里扫描可见的标签文本
        # 注意：完整的图表数据扫描需要解析 PPTX 内部的 embeddings
        try:
            if chart.has_legend:
                for legend_entry in chart.legend.legend_entries:
                    # 图例文本通常不是敏感数据，跳过
                    pass

            # 扫描图表标题
            if chart.has_title:
                title_text = chart.chart_title.text_frame.text
                pattern_results = self.pattern_registry.scan_text(title_text)
                for matched_value, detected_type, rule_name in pattern_results:
                    site = self._create_site_from_pattern(
                        slide_idx=slide_idx,
                        shape_id=shape_id,
                        shape_name=shape_name,
                        value=matched_value,
                        detected_type=detected_type,
                    )
                    if site:
                        sites.append(site)
        except Exception as e:
            logger.warning(f"扫描图表时出错: {e}")

        return sites

    def _create_site_from_column(
        self,
        slide_idx: int,
        shape_id: int,
        shape_name: str,
        table_location: str,
        value: str,
        rule,
    ) -> Optional[Site]:
        """从列头规则创建位点"""
        if rule.action:
            action = rule.action
            params = rule.params
        else:
            action, params = self._get_default_action(rule.detected_type)

        location = Location(
            type=SiteType.PPT,
            slide=slide_idx,
            shape_id=shape_name,
            table_location=table_location,
        )

        site_id = self._generate_site_id("slide", str(slide_idx), str(shape_id), table_location)

        return Site(
            site_id=site_id,
            location=location,
            original_value=value,
            detected_type=rule.detected_type,
            discovered_by=DiscoveredBy.COLUMN_RULE,
            enabled=True,
            action=action,
            params=params,
        )

    def _create_site_from_pattern(
        self,
        slide_idx: int,
        shape_id: int,
        shape_name: str,
        value: str,
        detected_type: DetectedType,
    ) -> Optional[Site]:
        """从正则规则创建位点"""
        action, params = self._get_default_action(detected_type)

        location = Location(
            type=SiteType.PPT,
            slide=slide_idx,
            shape_id=shape_name,
        )

        site_id = self._generate_site_id("slide", str(slide_idx), str(shape_id))

        return Site(
            site_id=site_id,
            location=location,
            original_value=value,
            detected_type=detected_type,
            discovered_by=DiscoveredBy.FULLTEXT_SCAN,
            enabled=True,
            action=action,
            params=params,
        )

    def _get_default_action(self, detected_type: DetectedType) -> tuple[ActionType, dict]:
        """根据敏感类型获取默认脱敏动作"""
        if detected_type == DetectedType.AMOUNT:
            return ActionType.PRECISION, {"unit": "million", "decimal_places": 2}
        elif detected_type == DetectedType.ENTITY:
            return ActionType.ALIAS, {"prefix": "公司"}
        elif detected_type == DetectedType.PERSON:
            return ActionType.MASK_NAME, {}
        elif detected_type == DetectedType.ACCOUNT:
            return ActionType.MASK_ACCOUNT, {"keep_prefix": 3, "keep_suffix": 4}
        else:
            return ActionType.MASK, {}

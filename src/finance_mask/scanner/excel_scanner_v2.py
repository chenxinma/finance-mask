"""
Excel 文件扫描实现 - 支持 Data 和 Form 两种表格类型

使用 Rust 库识别表格类型：
- Data 类型：表头+数据的一维表格，使用 DataFrame 方式按列定义脱敏策略
- Form 类型：key-value 排版的表单数据，按当前实现处理
"""
from __future__ import annotations

import logging
from pathlib import Path
from typing import Optional

import pandas as pd
from openpyxl import load_workbook
from openpyxl.utils import get_column_letter

from ..models.site import (
    Site,
    Location,
    SiteType,
    DetectedType,
    ActionType,
    DiscoveredBy,
)
from ..models.strategy import ColumnRule, MatchType
from .base import BaseScanner
from .column_matcher import ColumnMatcher
from .patterns import PatternRegistry
from .layout_view import LayoutViewClassifier, SheetType
from .header_finder import find_header_row, build_header_name, find_header_row_from_dataframe
from typing import Union

logger = logging.getLogger(__name__)


class ExcelScannerV2(BaseScanner):
    """Excel 文件扫描器 V2 - 支持 Data 和 Form 两种表格类型"""

    def __init__(
        self,
        column_matcher: Optional[ColumnMatcher] = None,
        pattern_registry: Optional[PatternRegistry] = None,
        layout_classifier: Optional[LayoutViewClassifier] = None,
    ):
        self.column_matcher = column_matcher or ColumnMatcher()
        self.pattern_registry = pattern_registry or PatternRegistry()
        self.layout_classifier = layout_classifier or LayoutViewClassifier()

    def scan(self, filepath: Path) -> list[Site]:
        """扫描 Excel 文件"""
        sites: list[Site] = []
        
        # 获取工作表分类
        classifications = self.layout_classifier.classify_with_fallback(filepath)
        
        # 按分类结果处理每个工作表
        for classification in classifications:
            sheet_name = classification.sheet_name
            sheet_type = classification.sheet_type
            
            logger.info(f"工作表 '{sheet_name}' 类型: {sheet_type} (置信度: {classification.confidence:.2f})")
            
            try:
                if sheet_type == SheetType.DATA:
                    # Data 类型：获取列规则，然后应用到整个列
                    column_rules = self._scan_data_sheet(filepath, sheet_name)
                    if column_rules:
                        # 应用列规则到整个列
                        sheet_sites = self._apply_column_rules(filepath, sheet_name, column_rules)
                        # 扫描表头之前和数据之后的文本内容
                        extra_sites = self._scan_extra_text(filepath, sheet_name, column_rules)
                        sheet_sites.extend(extra_sites)
                    else:
                        # 如果没有列规则，回退到 Form 扫描
                        sheet_sites = self._scan_form_sheet(filepath, sheet_name)
                else:
                    # Form 类型：按当前实现处理（逐单元格扫描）
                    sheet_sites = self._scan_form_sheet(filepath, sheet_name)
                
                sites.extend(sheet_sites)
                logger.info(f"工作表 '{sheet_name}' 扫描完成，发现 {len(sheet_sites)} 个敏感位点")
                
            except Exception as e:
                logger.error(f"扫描工作表 '{sheet_name}' 失败: {e}")
                # 回退到 Form 类型处理
                try:
                    sheet_sites = self._scan_form_sheet(filepath, sheet_name)
                    sites.extend(sheet_sites)
                    logger.info(f"工作表 '{sheet_name}' 回退到 Form 扫描完成，发现 {len(sheet_sites)} 个敏感位点")
                except Exception as fallback_error:
                    logger.error(f"工作表 '{sheet_name}' 回退扫描也失败: {fallback_error}")
        
        return sites

    def get_column_rules(self, filepath: Path) -> dict[str, list[ColumnRule]]:
        """
        获取列级别的脱敏规则（用于 Data 类型表格）
        
        Args:
            filepath: Excel 文件路径
            
        Returns:
            字典，键为工作表名，值为该工作表的列规则列表
        """
        result: dict[str, list[ColumnRule]] = {}
        
        # 获取工作表分类
        classifications = self.layout_classifier.classify_with_fallback(filepath)
        
        # 按分类结果处理每个工作表
        for classification in classifications:
            sheet_name = classification.sheet_name
            sheet_type = classification.sheet_type
            
            if sheet_type == SheetType.DATA:
                try:
                    column_rules = self._scan_data_sheet(filepath, sheet_name)
                    if column_rules:
                        result[sheet_name] = column_rules
                except Exception as e:
                    logger.error(f"获取工作表 '{sheet_name}' 列规则失败: {e}")
        
        return result

    def scan_non_data_cells(self, filepath: Path) -> list[Site]:
        """
        扫描 Excel 文件中 Data 类型表格的非数据区域（表头之前、数据之后的文本）
        
        用于 generate 命令：当 Data 类型表格已有列规则时，
        额外扫描表头之前和数据之后的文本内容（如公司名称、注释等）。
        
        Args:
            filepath: Excel 文件路径
            
        Returns:
            位点列表
        """
        sites: list[Site] = []
        
        classifications = self.layout_classifier.classify_with_fallback(filepath)
        
        for classification in classifications:
            sheet_name = classification.sheet_name
            sheet_type = classification.sheet_type
            
            if sheet_type != SheetType.DATA:
                continue
            
            # 检查是否有列规则
            column_rules = self._scan_data_sheet(filepath, sheet_name)
            if not column_rules:
                continue
            
            extra_sites = self._scan_extra_text(filepath, sheet_name, column_rules)
            sites.extend(extra_sites)
            logger.info(f"工作表 '{sheet_name}' 非数据区域扫描完成，发现 {len(extra_sites)} 个敏感位点")
        
        return sites

    def _scan_data_sheet(self, filepath: Path, sheet_name: str) -> list[ColumnRule]:
        """
        扫描 Data 类型的工作表
        
        使用 DataFrame 方式按列定义脱敏策略
        支持多行表头合并
        
        Returns:
            列头规则列表（按列定义，不按单元格）
        """
        column_rules: list[ColumnRule] = []
        
        # 加载工作簿用于查找表头行
        wb = load_workbook(str(filepath), data_only=True)
        ws = wb[sheet_name]
        
        # 查找表头行范围（支持多行表头）
        header_start, header_end = find_header_row(ws, candidates=None)
        
        if header_start == -1:
            logger.warning(f"工作表 '{sheet_name}' 未找到表头行，回退到 Form 扫描")
            wb.close()
            # 对于 Form 类型，返回空列表，让调用者使用 Form 扫描
            return []
        
        logger.info(f"Data 类型工作表 '{sheet_name}'，表头行: {header_start + 1}-{header_end + 1}")
        
        # 构建合并后的表头名称
        max_col = ws.max_column or 0
        header_names: list[str] = []
        for col_idx in range(max_col):
            header_names.append(build_header_name(ws, col_idx, header_start, header_end))
        wb.close()
        
        if not header_names:
            logger.warning(f"工作表 '{sheet_name}' 表头为空")
            return []
        
        # 去重列名
        seen: dict[str, int] = {}
        deduped_header: list[str] = []
        for name in header_names:
            if name in seen:
                seen[name] += 1
                deduped_header.append(f"{name}_{seen[name]}")
            else:
                seen[name] = 0
                deduped_header.append(name)
        
        logger.info(f"Data 类型工作表 '{sheet_name}'，列: {deduped_header}")
        
        # 按列定义脱敏策略
        for col_idx, col_name in enumerate(deduped_header):
            col_name_str = str(col_name).strip()
            if not col_name_str:
                continue
            
            # 匹配列头规则
            rule = self.column_matcher.match(col_name_str)
            
            if rule:
                # 创建列级别的规则
                column_rule = ColumnRule(
                    match_type=MatchType.EXACT,
                    pattern=col_name_str,
                    action=rule.action,
                    params=rule.params,
                    detected_type=rule.detected_type,
                    priority=rule.priority,
                )
                column_rules.append(column_rule)
                logger.debug(f"列 '{col_name_str}' 匹配规则: {rule.action.value}")
            else:
                logger.debug(f"列 '{col_name_str}' 未匹配到规则")
        
        return column_rules

    def _apply_column_rules(
        self, filepath: Path, sheet_name: str, column_rules: list[ColumnRule]
    ) -> list[Site]:
        """
        将列规则应用到整个列，生成位点列表
        
        Args:
            filepath: Excel 文件路径
            sheet_name: 工作表名称
            column_rules: 列规则列表
            
        Returns:
            位点列表
        """
        sites: list[Site] = []
        
        # 加载工作簿
        wb = load_workbook(str(filepath), data_only=True)
        ws = wb[sheet_name]
        
        # 查找表头行范围
        header_start, header_end = find_header_row(ws, candidates=None)
        
        if header_start == -1:
            wb.close()
            return sites
        
        # 构建列名到列索引的映射
        max_col = ws.max_column or 0
        col_name_to_idx: dict[str, int] = {}
        for col_idx in range(max_col):
            col_name = build_header_name(ws, col_idx, header_start, header_end)
            if col_name:
                # 处理重复列名
                if col_name in col_name_to_idx:
                    # 添加数字后缀
                    counter = 1
                    new_name = f"{col_name}_{counter}"
                    while new_name in col_name_to_idx:
                        counter += 1
                        new_name = f"{col_name}_{counter}"
                    col_name = new_name
                col_name_to_idx[col_name] = col_idx
        
        # 应用列规则
        for rule in column_rules:
            col_name = rule.pattern
            if col_name not in col_name_to_idx:
                continue
            
            col_idx = col_name_to_idx[col_name]
            
            # 扫描该列的所有数据行
            for row_idx in range(header_end + 1, ws.max_row or 0):
                cell = ws.cell(row=row_idx + 1, column=col_idx + 1)
                if cell.value is None:
                    continue
                
                value_str = str(cell.value).strip()
                if not value_str:
                    continue
                
                # 创建位点
                cell_coord = f"{get_column_letter(col_idx + 1)}{row_idx + 1}"
                location = Location(
                    type=SiteType.EXCEL,
                    sheet=sheet_name,
                    cell=cell_coord,
                    column=col_name,
                )
                site_id = self._generate_site_id("sheet", sheet_name, cell_coord)
                
                site = Site(
                    site_id=site_id,
                    location=location,
                    original_value=value_str,
                    detected_type=rule.detected_type,
                    discovered_by=DiscoveredBy.COLUMN_RULE,
                    enabled=True,
                    action=rule.action,
                    params=rule.params,
                )
                sites.append(site)
        
        wb.close()
        return sites

    def _scan_extra_text(
        self,
        filepath: Path,
        sheet_name: str,
        column_rules: list[ColumnRule],
    ) -> list[Site]:
        """
        扫描表头之前和数据之后的文本内容
        
        对于 Data 类型表格，表头之前（如公司名称、表格标题）和
        数据之后（如注释信息）的文本也需要进行敏感信息识别。
        
        Args:
            filepath: Excel 文件路径
            sheet_name: 工作表名称
            column_rules: 列规则列表（用于判断表头行位置）
            
        Returns:
            位点列表
        """
        sites: list[Site] = []
        
        try:
            wb = load_workbook(str(filepath), data_only=True)
            ws = wb[sheet_name]
            
            # 查找表头行范围
            header_start, header_end = find_header_row(ws, candidates=None)
            
            if header_start == -1:
                wb.close()
                return sites
            
            # 扫描表头之前的行（行 1 到 header_start）
            for row_idx in range(1, header_start + 1):
                for col_idx in range(1, (ws.max_column or 0) + 1):
                    cell = ws.cell(row=row_idx, column=col_idx)
                    if cell.value is None:
                        continue
                    
                    cell_str = str(cell.value).strip()
                    if not cell_str:
                        continue
                    
                    # 全文正则扫描
                    pattern_results = self.pattern_registry.scan_text(cell_str)
                    for matched_value, detected_type, rule_name in pattern_results:
                        cell_coord = f"{get_column_letter(col_idx)}{row_idx}"
                        site = self._create_site_from_pattern(
                            sheet_name=sheet_name,
                            cell_coord=cell_coord,
                            value=matched_value,
                            detected_type=detected_type,
                        )
                        if site:
                            sites.append(site)
            
            # 扫描数据之后的行（从 ws.max_row 开始向下，但通常从 header_end + 数据行数后开始）
            # 这里我们扫描所有表头之后的非数据行
            # 简化处理：扫描表头之后的所有行，但只处理 A 列（通常是注释列）
            max_row = ws.max_row or 0
            for row_idx in range(header_end + 1, max_row + 1):
                # 检查这行是否在数据区域（通过检查 B 列是否有数值）
                data_cell = ws.cell(row=row_idx, column=2)
                if data_cell.value is not None:
                    # 这是数据行，跳过（由 _apply_column_rules 处理）
                    continue
                
                # 这是非数据行（可能是注释行），扫描 A 列
                cell = ws.cell(row=row_idx, column=1)
                if cell.value is None:
                    continue
                
                cell_str = str(cell.value).strip()
                if not cell_str:
                    continue
                
                # 全文正则扫描
                pattern_results = self.pattern_registry.scan_text(cell_str)
                for matched_value, detected_type, rule_name in pattern_results:
                    cell_coord = f"A{row_idx}"
                    site = self._create_site_from_pattern(
                        sheet_name=sheet_name,
                        cell_coord=cell_coord,
                        value=matched_value,
                        detected_type=detected_type,
                    )
                    if site:
                        sites.append(site)
            
            wb.close()
            
        except Exception as e:
            logger.error(f"扫描工作表 '{sheet_name}' 额外文本失败: {e}")
        
        return sites

    def _read_data_with_multi_header(
        self, ws, header_start: int, header_end: int
    ) -> pd.DataFrame:
        """
        读取支持多行表头的 DataFrame
        
        Args:
            ws: openpyxl 工作表对象
            header_start: 表头起始行（0-based）
            header_end: 表头结束行（0-based）
            
        Returns:
            DataFrame
        """
        # 构建表头名称
        max_col = ws.max_column or 0
        header: list[str] = []
        for col_idx in range(max_col):
            header.append(build_header_name(ws, col_idx, header_start, header_end))
        
        # 去重列名，添加数字后缀
        seen: dict[str, int] = {}
        deduped_header: list[str] = []
        for name in header:
            if name in seen:
                seen[name] += 1
                deduped_header.append(f"{name}_{seen[name]}")
            else:
                seen[name] = 0
                deduped_header.append(name)
        
        # 收集数据行（从 header_end + 1 开始）
        data = []
        for idx, row in enumerate(ws.iter_rows(values_only=True)):
            if idx > header_end:
                data.append(row)
        
        if not deduped_header:
            return pd.DataFrame()
        
        df = pd.DataFrame(data, columns=deduped_header)
        
        # 将 None 转换为空字符串
        for col in df.columns:
            df[col] = df[col].astype(str).replace("None", "")
        
        return df

    def _scan_form_sheet(self, filepath: Path, sheet_name: str) -> list[Site]:
        """
        扫描 Form 类型的工作表
        
        按当前实现处理：逐单元格扫描
        """
        sites: list[Site] = []
        
        try:
            wb = load_workbook(str(filepath), read_only=True, data_only=True)
            ws = wb[sheet_name]
            
            header_row: dict[int, str] = {}  # 列索引 -> 列名
            header_found = False
            
            for row_idx, row in enumerate(ws.iter_rows(min_row=1), start=1):
                for col_idx, cell in enumerate(row, start=1):
                    cell_value = cell.value
                    if cell_value is None:
                        continue
                    
                    cell_str = str(cell_value).strip()
                    if not cell_str:
                        continue
                    
                    cell_coord = f"{get_column_letter(col_idx)}{row_idx}"
                    
                    # 尝试识别表头行（前5行中查找）
                    if row_idx <= 5 and not header_found:
                        rule = self.column_matcher.match(cell_str)
                        if rule:
                            header_row[col_idx] = cell_str
                            header_found = True
                            continue
                    
                    # 列头定位扫描
                    if col_idx in header_row:
                        header_name = header_row[col_idx]
                        rule = self.column_matcher.match(header_name)
                        if rule:
                            site = self._create_site_from_column(
                                sheet_name=sheet_name,
                                cell_coord=cell_coord,
                                header_name=header_name,
                                value=cell_str,
                                rule=rule,
                            )
                            if site:
                                sites.append(site)
                                continue
                    
                    # 全文正则扫描（兜底）
                    pattern_results = self.pattern_registry.scan_text(cell_str)
                    for matched_value, detected_type, rule_name in pattern_results:
                        site = self._create_site_from_pattern(
                            sheet_name=sheet_name,
                            cell_coord=cell_coord,
                            value=matched_value,
                            detected_type=detected_type,
                        )
                        if site:
                            sites.append(site)
            
            wb.close()
            
        except Exception as e:
            logger.error(f"Form 扫描工作表 '{sheet_name}' 失败: {e}")
            raise
        
        return sites

    def _create_site_from_column(
        self,
        sheet_name: str,
        cell_coord: str,
        header_name: str,
        value: str,
        rule,
    ) -> Optional[Site]:
        """从列头规则创建位点"""
        # 根据规则类型确定默认动作
        if rule.action:
            action = rule.action
            params = rule.params
        else:
            action, params = self._get_default_action(rule.detected_type)
        
        location = Location(
            type=SiteType.EXCEL,
            sheet=sheet_name,
            cell=cell_coord,
            column=header_name,
        )
        
        site_id = self._generate_site_id("sheet", sheet_name, cell_coord)
        
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
        sheet_name: str,
        cell_coord: str,
        value: str,
        detected_type: DetectedType,
    ) -> Optional[Site]:
        """从正则规则创建位点"""
        action, params = self._get_default_action(detected_type)
        
        location = Location(
            type=SiteType.EXCEL,
            sheet=sheet_name,
            cell=cell_coord,
        )
        
        site_id = self._generate_site_id("sheet", sheet_name, cell_coord)
        
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

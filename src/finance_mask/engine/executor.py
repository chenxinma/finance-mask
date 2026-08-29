"""策略执行引擎"""
from __future__ import annotations

import logging
from pathlib import Path
from typing import Optional

from openpyxl import load_workbook
from pptx import Presentation

from ..models.site import Site, SiteType, ActionType, DetectedType
from ..models.strategy import Strategy
from ..audit.logger import AuditLogger
from ..watermark.encoder import WatermarkEncoder
from .amount import AmountRedactor
from .name import NameRedactor
from .account import AccountRedactor

logger = logging.getLogger(__name__)


class Executor:
    """策略执行引擎"""

    def __init__(self, strategy: Strategy, operator: str = "unknown"):
        self.strategy = strategy
        self.operator = operator
        self.audit_logger: Optional[AuditLogger] = None

    def execute(
        self,
        input_path: Path,
        output_path: Path,
        dry_run: bool = False,
    ) -> dict:
        """
        执行脱敏
        
        Args:
            input_path: 输入文件路径
            output_path: 输出文件路径
            dry_run: 是否仅预览不实际执行
            
        Returns:
            执行结果字典
        """
        # 初始化审计日志
        self.audit_logger = AuditLogger(
            source_file=input_path.name,
            output_file=output_path.name,
            operator=self.operator,
            source_path=str(input_path),
        )

        results = {
            "success": True,
            "total_sites": len(self.strategy.sites),
            "processed": 0,
            "skipped": 0,
            "errors": 0,
            "dry_run": dry_run,
        }

        # 按文件类型分组处理
        if input_path.suffix.lower() == ".xlsx":
            self._execute_excel(input_path, output_path, dry_run, results)
        elif input_path.suffix.lower() == ".pptx":
            self._execute_ppt(input_path, output_path, dry_run, results)
        else:
            results["success"] = False
            results["error"] = f"不支持的文件格式: {input_path.suffix}"
            return results

        return results

    def _execute_excel(
        self,
        input_path: Path,
        output_path: Path,
        dry_run: bool,
        results: dict,
    ) -> None:
        """执行 Excel 脱敏"""
        try:
            wb = load_workbook(str(input_path))
        except Exception as e:
            results["success"] = False
            results["error"] = f"无法打开 Excel 文件: {e}"
            return

        # 处理列规则（Data 类型表格）
        if self.strategy.column_rules:
            self._execute_column_rules(wb, dry_run, results)
        
        # 处理单元格位点（Form 类型表格）
        excel_sites = [s for s in self.strategy.sites if s.location.type == SiteType.EXCEL]
        if excel_sites:
            self._execute_cell_sites(wb, excel_sites, dry_run, results)

        # 保存文件（非 dry_run 模式）
        if not dry_run:
            try:
                wb.save(str(output_path))
                logger.info(f"Excel 文件已保存: {output_path}")
            except Exception as e:
                results["success"] = False
                results["error"] = f"保存 Excel 文件失败: {e}"
            finally:
                wb.close()

    def _execute_column_rules(
        self,
        wb,
        dry_run: bool,
        results: dict,
    ) -> None:
        """执行列规则（Data 类型表格）"""
        from ..scanner.header_finder import find_header_row, build_header_name
        from openpyxl.utils import get_column_letter
        
        for sheet_name in wb.sheetnames:
            ws = wb[sheet_name]
            
            # 查找表头行
            header_start, header_end = find_header_row(ws)
            if header_start == -1:
                continue
            
            # 构建列名到列索引的映射
            max_col = ws.max_column or 0
            col_name_to_idx: dict[str, int] = {}
            for col_idx in range(max_col):
                col_name = build_header_name(ws, col_idx, header_start, header_end)
                if col_name:
                    # 处理重复列名
                    if col_name in col_name_to_idx:
                        counter = 1
                        new_name = f"{col_name}_{counter}"
                        while new_name in col_name_to_idx:
                            counter += 1
                            new_name = f"{col_name}_{counter}"
                        col_name = new_name
                    col_name_to_idx[col_name] = col_idx
            
            # 应用列规则
            for rule in self.strategy.column_rules:
                col_name = rule.pattern
                if col_name not in col_name_to_idx:
                    continue
                
                col_idx = col_name_to_idx[col_name]
                
                # 扫描该列的所有数据行
                for row_idx in range(header_end + 1, ws.max_row or 0):
                    cell = ws.cell(row=row_idx + 1, column=col_idx + 1)
                    if cell.value is None:
                        continue
                    
                    original_value = str(cell.value).strip()
                    if not original_value:
                        continue
                    
                    try:
                        # 执行脱敏
                        redacted_value = self._redact_value(
                            original_value, rule.action, rule.params, rule.detected_type
                        )
                        
                        # 记录审计日志（使用临时 Site 对象）
                        from ..models.site import Location, SiteType, DiscoveredBy
                        temp_site = Site(
                            site_id=f"{sheet_name}_{get_column_letter(col_idx + 1)}{row_idx + 1}",
                            location=Location(
                                type=SiteType.EXCEL,
                                sheet=sheet_name,
                                cell=f"{get_column_letter(col_idx + 1)}{row_idx + 1}",
                                column=col_name,
                            ),
                            original_value=original_value,
                            detected_type=rule.detected_type,
                            discovered_by=DiscoveredBy.COLUMN_RULE,
                            enabled=True,
                            action=rule.action,
                            params=rule.params,
                        )
                        
                        self.audit_logger.log_change(
                            site=temp_site,
                            original_value=original_value,
                            redacted_value=redacted_value,
                            action=rule.action.value,
                        )
                        
                        # 写入文件（非 dry_run 模式）
                        if not dry_run:
                            # 尝试保持数值类型
                            try:
                                cell.value = float(redacted_value.replace(",", "").replace("元", "").replace("百万", "").replace("亿", "").replace("千", ""))
                            except (ValueError, TypeError):
                                cell.value = redacted_value
                        
                        results["processed"] += 1
                        
                    except Exception as e:
                        logger.error(f"处理单元格 {sheet_name}!{get_column_letter(col_idx + 1)}{row_idx + 1} 失败: {e}")
                        results["errors"] += 1

    def _execute_cell_sites(
        self,
        wb,
        excel_sites: list,
        dry_run: bool,
        results: dict,
    ) -> None:
        """执行单元格位点（Form 类型表格）"""
        for site in excel_sites:
            if not site.enabled:
                results["skipped"] += 1
                continue

            try:
                # 定位单元格
                ws = wb[site.location.sheet]
                cell = ws[site.location.cell]
                original_value = str(cell.value) if cell.value is not None else ""

                # 执行脱敏
                redacted_value = self._redact_value(
                    original_value, site.action, site.params, site.detected_type
                )

                # 记录审计日志
                self.audit_logger.log_change(
                    site=site,
                    original_value=original_value,
                    redacted_value=redacted_value,
                    action=site.action.value,
                )

                # 写入文件（非 dry_run 模式）
                if not dry_run:
                    # 尝试保持数值类型
                    try:
                        cell.value = float(redacted_value.replace(",", "").replace("元", "").replace("百万", "").replace("亿", "").replace("千", ""))
                    except (ValueError, TypeError):
                        cell.value = redacted_value

                results["processed"] += 1

            except Exception as e:
                logger.error(f"处理位点 {site.site_id} 失败: {e}")
                self.audit_logger.log_error(site.site_id, str(e))
                results["errors"] += 1

    def _execute_ppt(
        self,
        input_path: Path,
        output_path: Path,
        dry_run: bool,
        results: dict,
    ) -> None:
        """执行 PPT 脱敏"""
        try:
            prs = Presentation(str(input_path))
        except Exception as e:
            results["success"] = False
            results["error"] = f"无法打开 PPT 文件: {e}"
            return

        # 获取 PPT 类型的位点
        ppt_sites = [s for s in self.strategy.sites if s.location.type == SiteType.PPT]

        for site in ppt_sites:
            if not site.enabled:
                results["skipped"] += 1
                continue

            try:
                # 定位形状
                slide = prs.slides[site.location.slide - 1]  # slide 从 1 开始
                shape = None
                for s in slide.shapes:
                    if s.name == site.location.shape_id or str(s.shape_id) == site.location.shape_id:
                        shape = s
                        break

                if shape is None:
                    raise ValueError(f"未找到形状: {site.location.shape_id}")

                # 处理表格
                if shape.has_table and site.location.table_location:
                    self._redact_table_cell(shape.table, site, dry_run, results)
                # 处理文本框
                elif shape.has_text_frame:
                    self._redact_text_frame(shape.text_frame, site, dry_run, results)

                results["processed"] += 1

            except Exception as e:
                logger.error(f"处理位点 {site.site_id} 失败: {e}")
                self.audit_logger.log_error(site.site_id, str(e))
                results["errors"] += 1

        # 保存文件（非 dry_run 模式）
        if not dry_run:
            try:
                prs.save(str(output_path))
                logger.info(f"PPT 文件已保存: {output_path}")
            except Exception as e:
                results["success"] = False
                results["error"] = f"保存 PPT 文件失败: {e}"

    def _redact_table_cell(self, table, site: Site, dry_run: bool, results: dict) -> None:
        """脱敏表格单元格"""
        # 解析 table_location (R2C3 格式)
        location = site.location.table_location
        if not location:
            return

        try:
            row_str, col_str = location.replace("R", "").split("C")
            row_idx = int(row_str) - 1  # 转为 0-based
            col_idx = int(col_str) - 1
        except (ValueError, IndexError):
            raise ValueError(f"无效的表格位置: {location}")

        if row_idx >= len(table.rows) or col_idx >= len(table.columns):
            raise ValueError(f"表格位置越界: {location}")

        cell = table.rows[row_idx].cells[col_idx]
        original_value = cell.text.strip()

        # 执行脱敏
        redacted_value = self._redact_value(
            original_value, site.action, site.params, site.detected_type
        )

        # 记录审计日志
        self.audit_logger.log_change(
            site=site,
            original_value=original_value,
            redacted_value=redacted_value,
            action=site.action.value,
        )

        # 写入（非 dry_run 模式）
        if not dry_run:
            # 清空单元格并写入新值
            for paragraph in cell.text_frame.paragraphs:
                for run in paragraph.runs:
                    run.text = ""
                if paragraph.runs:
                    paragraph.runs[0].text = redacted_value
                else:
                    paragraph.text = redacted_value

    def _redact_text_frame(self, text_frame, site: Site, dry_run: bool, results: dict) -> None:
        """脱敏文本框"""
        for paragraph in text_frame.paragraphs:
            text = paragraph.text
            if site.original_value in text:
                # 执行脱敏
                redacted_value = self._redact_value(
                    site.original_value, site.action, site.params, site.detected_type
                )

                # 记录审计日志
                self.audit_logger.log_change(
                    site=site,
                    original_value=site.original_value,
                    redacted_value=redacted_value,
                    action=site.action.value,
                )

                # 替换文本（非 dry_run 模式）
                if not dry_run:
                    new_text = text.replace(site.original_value, redacted_value)
                    # 尝试在 run 中替换
                    for run in paragraph.runs:
                        if site.original_value in run.text:
                            run.text = run.text.replace(site.original_value, redacted_value)
                            break
                    else:
                        # 如果没在任何 run 中找到，直接设置段落文本
                        paragraph.text = new_text

                break

    def _redact_value(
        self,
        value: str,
        action: ActionType,
        params: Optional[dict],
        detected_type: DetectedType,
    ) -> str:
        """执行单个值的脱敏"""
        from decimal import Decimal
        
        params = params or {}

        if action == ActionType.PRECISION:
            return AmountRedactor.precision(
                value,
                unit=params.get("unit", "million"),
                decimal_places=params.get("decimal_places", 2),
            )
        elif action == ActionType.PERTURB:
            return AmountRedactor.perturb(
                value,
                percentage=params.get("percentage", 5.0),
            )
        elif action == ActionType.MASK:
            return AmountRedactor.mask(value)
        elif action == ActionType.ALIAS:
            return NameRedactor.alias(
                value,
                prefix=params.get("prefix", "公司"),
            )
        elif action == ActionType.MASK_NAME:
            return NameRedactor.mask_name(
                value,
                keep_first=params.get("keep_first", 1),
            )
        elif action == ActionType.MASK_ACCOUNT:
            return AccountRedactor.mask_account(
                value,
                keep_prefix=params.get("keep_prefix", 3),
                keep_suffix=params.get("keep_suffix", 4),
            )
        elif action == ActionType.DIFFERENTIAL_SHIFT:
            # 差分偏移：需要预计算的偏移量
            shift = params.get("shift")
            if shift is None:
                raise ValueError("差分偏移模式需要 params.shift 参数")
            return AmountRedactor.differential_shift(
                value,
                shift=Decimal(str(shift)),
            )
        elif action == ActionType.PROPORTIONAL_SCALE:
            # 比例缩放：需要预计算的缩放因子
            scale = params.get("scale")
            if scale is None:
                raise ValueError("比例缩放模式需要 params.scale 参数")
            numeric_value = AmountRedactor._parse_number(value)
            result = numeric_value * Decimal(str(scale))
            return f"{result.quantize(Decimal('0.01'), rounding=ROUND_HALF_UP):,.2f}"
        else:
            raise ValueError(f"不支持的脱敏动作: {action}")

    def precompute_differential_params(
        self,
        sites: list[Site],
        mode: str = "shift",
        percentage: float = 10.0,
    ) -> dict:
        """
        预计算差分参数
        
        对于差分偏移和比例缩放模式，需要先计算统一的偏移量/缩放因子，
        然后应用到所有相关位点。
        
        Args:
            sites: 需要差分处理的位点列表
            mode: 模式 "shift" 或 "scale"
            percentage: 偏移/缩放百分比
            
        Returns:
            包含偏移量或缩放因子的字典
        """
        values = [site.original_value for site in sites]
        
        if mode == "shift":
            results, shift = AmountRedactor.generate_differential_sequence(
                values,
                shift_range=None,
            )
            return {"shift": str(shift), "mode": "shift"}
        elif mode == "scale":
            results, scale = AmountRedactor.generate_proportional_differential_sequence(
                values,
                percentage=percentage,
            )
            return {"scale": str(scale), "mode": "scale"}
        else:
            raise ValueError(f"不支持的差分模式: {mode}")

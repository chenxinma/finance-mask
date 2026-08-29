"""集成测试"""
import pytest
import tempfile
from pathlib import Path

from openpyxl import Workbook

from src.finance_mask.scanner import ExcelScanner
from src.finance_mask.engine import Executor
from src.finance_mask.models.strategy import Strategy, Metadata
from src.finance_mask.models.site import SiteType
from src.finance_mask.utils import load_strategy, export_strategy
from src.finance_mask.watermark import WatermarkEncoder, WatermarkDecoder
from src.finance_mask.audit import AuditLogExporter


class TestIntegration:
    """集成测试"""

    @pytest.fixture
    def sample_excel(self, tmp_path):
        """创建示例 Excel 文件"""
        wb = Workbook()
        ws = wb.active
        ws.title = "利润表"

        # 表头
        ws["A1"] = "项目"
        ws["B1"] = "本期金额"
        ws["C1"] = "上期金额"

        # 数据
        ws["A2"] = "营业收入"
        ws["B2"] = 12345678.90
        ws["C2"] = 11111111.11

        ws["A3"] = "净利润"
        ws["B3"] = 8765432.10
        ws["C3"] = 7777777.77

        ws["A4"] = "客户名称"
        ws["B4"] = "阿里巴巴集团"

        ws["A5"] = "联系人"
        ws["B5"] = "张三"

        filepath = tmp_path / "test_report.xlsx"
        wb.save(str(filepath))
        return filepath

    @pytest.fixture
    def sample_pptx(self, tmp_path):
        """创建示例 PPT 文件"""
        from pptx import Presentation
        from pptx.util import Inches, Pt

        prs = Presentation()
        slide = prs.slides.add_slide(prs.slide_layouts[5])  # 空白布局

        # 添加文本框
        left = Inches(1)
        top = Inches(1)
        width = Inches(8)
        height = Inches(1)

        txBox = slide.shapes.add_textbox(left, top, width, height)
        tf = txBox.text_frame
        p = tf.paragraphs[0]
        p.text = "2024年度营业收入为 1,234,567,890.12 元，净利润为 876,543,210.00 元"

        # 添加表格
        rows = 3
        cols = 3
        left = Inches(1)
        top = Inches(3)
        width = Inches(8)
        height = Inches(2)

        table_shape = slide.shapes.add_table(rows, cols, left, top, width, height)
        table = table_shape.table

        # 表头
        table.cell(0, 0).text = "项目"
        table.cell(0, 1).text = "金额"
        table.cell(0, 2).text = "备注"

        # 数据
        table.cell(1, 0).text = "营业收入"
        table.cell(1, 1).text = "1,234,567,890.12"
        table.cell(1, 2).text = "同比增长10%"

        table.cell(2, 0).text = "净利润"
        table.cell(2, 1).text = "876,543,210.00"
        table.cell(2, 2).text = "同比增长15%"

        filepath = tmp_path / "test_report.pptx"
        prs.save(str(filepath))
        return filepath

    def test_excel_scan_and_redact(self, sample_excel, tmp_path):
        """测试 Excel 扫描和脱敏完整流程"""
        # 1. 扫描
        scanner = ExcelScanner()
        sites = scanner.scan(sample_excel)
        assert len(sites) > 0
        print(f"发现 {len(sites)} 个敏感位点")

        # 2. 导出策略
        strategy_path = export_strategy(
            sites=sites,
            source_file=sample_excel.name,
            output_path=tmp_path / "策略.yaml",
        )
        assert strategy_path.exists()

        # 3. 加载策略
        strategy = load_strategy(strategy_path)
        assert len(strategy.sites) > 0

        # 4. 执行脱敏
        output_path = tmp_path / "test_report_脱敏.xlsx"
        executor = Executor(strategy, operator="test_user")
        result = executor.execute(
            input_path=sample_excel,
            output_path=output_path,
        )
        assert result["success"] is True
        assert result["processed"] > 0

        # 5. 导出审计日志
        log_path = tmp_path / "test_report_日志.json"
        output_hash = AuditLogExporter.compute_file_hash(output_path)
        AuditLogExporter.export(
            logger=executor.audit_logger,
            output_path=log_path,
            file_hash=output_hash,
        )
        assert log_path.exists()

    def test_pptx_scan_and_redact(self, sample_pptx, tmp_path):
        """测试 PPT 扫描和脱敏完整流程"""
        # 1. 扫描
        from src.finance_mask.scanner import PPTScanner
        scanner = PPTScanner()
        sites = scanner.scan(sample_pptx)
        assert len(sites) > 0
        print(f"发现 {len(sites)} 个敏感位点")

        # 2. 导出策略
        strategy_path = export_strategy(
            sites=sites,
            source_file=sample_pptx.name,
            output_path=tmp_path / "策略.yaml",
        )
        assert strategy_path.exists()

        # 3. 加载策略
        strategy = load_strategy(strategy_path)
        assert len(strategy.sites) > 0

        # 4. 执行脱敏
        output_path = tmp_path / "test_report_脱敏.pptx"
        executor = Executor(strategy, operator="test_user")
        result = executor.execute(
            input_path=sample_pptx,
            output_path=output_path,
        )
        assert result["success"] is True
        assert result["processed"] > 0

    def test_watermark_roundtrip(self, sample_excel, tmp_path):
        """测试水印嵌入和解码"""
        # 1. 准备脱敏文件
        scanner = ExcelScanner()
        sites = scanner.scan(sample_excel)
        strategy_path = export_strategy(
            sites=sites,
            source_file=sample_excel.name,
            output_path=tmp_path / "策略.yaml",
        )
        strategy = load_strategy(strategy_path)
        output_path = tmp_path / "test_report_脱敏.xlsx"
        executor = Executor(strategy, operator="test_user")
        executor.execute(input_path=sample_excel, output_path=output_path)

        # 2. 嵌入水印
        payload = WatermarkEncoder.create_payload(
            operator="test_user",
            timestamp="2024-01-01T00:00:00",
            file_hash="abc123",
        )
        success = WatermarkEncoder.embed_to_excel(output_path, payload)
        assert success is True

        # 3. 解码水印
        payloads = WatermarkDecoder.extract_from_excel(str(output_path))
        assert len(payloads) > 0
        assert "test_user" in payloads[0]

    def test_dry_run_mode(self, sample_excel, tmp_path):
        """测试预览模式"""
        scanner = ExcelScanner()
        sites = scanner.scan(sample_excel)
        strategy_path = export_strategy(
            sites=sites,
            source_file=sample_excel.name,
            output_path=tmp_path / "策略.yaml",
        )
        strategy = load_strategy(strategy_path)
        output_path = tmp_path / "test_report_脱敏.xlsx"
        executor = Executor(strategy, operator="test_user")
        result = executor.execute(
            input_path=sample_excel,
            output_path=output_path,
            dry_run=True,
        )
        assert result["success"] is True
        assert result["dry_run"] is True
        # dry_run 模式下文件不应被创建
        assert not output_path.exists()


if __name__ == "__main__":
    pytest.main([__file__, "-v"])

"""零宽字符水印编码与嵌入"""
from __future__ import annotations

import logging
from datetime import datetime
from pathlib import Path
from typing import Optional

logger = logging.getLogger(__name__)


class WatermarkEncoder:
    """零宽字符水印编码器"""

    # 零宽字符映射
    ZERO_WIDTH_CHARS = {
        "0": "\u200B",  # 零宽空格
        "1": "\u200C",  # 零宽非连接符
    }

    # 最小嵌入文本长度（避免破坏短文本排版）
    MIN_EMBED_TEXT_LENGTH = 20

    @classmethod
    def encode(cls, payload: str) -> str:
        """
        将 payload 编码为零宽字符序列
        
        Args:
            payload: 要编码的字符串
            
        Returns:
            零宽字符序列
        """
        # 转换为 UTF-8 字节
        payload_bytes = payload.encode("utf-8")

        # 转换为二进制字符串
        binary = "".join(format(byte, "08b") for byte in payload_bytes)

        # 转换为零宽字符
        watermark = "".join(cls.ZERO_WIDTH_CHARS[bit] for bit in binary)

        return watermark

    @classmethod
    def decode(cls, text: str) -> str:
        """
        从文本中提取并解码零宽字符水印
        
        Args:
            text: 包含零宽字符的文本
            
        Returns:
            解码后的 payload
        """
        return WatermarkDecoder.decode(text)

    @classmethod
    def embed_to_excel(
        cls,
        filepath: Path,
        payload: str,
        output_path: Optional[Path] = None,
    ) -> bool:
        """
        在 Excel 文件中嵌入水印
        
        策略：每个工作表的首个非空单元格末尾嵌入
        
        Args:
            filepath: Excel 文件路径
            payload: 水印内容
            output_path: 输出路径（None 则覆盖原文件）
            
        Returns:
            是否成功
        """
        try:
            from openpyxl import load_workbook

            watermark = cls.encode(payload)
            wb = load_workbook(str(filepath))

            embed_count = 0
            for sheet_name in wb.sheetnames:
                ws = wb[sheet_name]
                # 找到首个非空单元格
                for row in ws.iter_rows(min_row=1, max_row=10):
                    for cell in row:
                        if cell.value is not None:
                            cell_str = str(cell.value).strip()
                            if cell_str:
                                # 放宽长度限制，允许较短文本也嵌入水印
                                cell.value = cell_str + watermark
                                embed_count += 1
                                break
                    if embed_count > 0:
                        break

            if embed_count > 0:
                save_path = output_path or filepath
                wb.save(str(save_path))
                logger.info(f"水印嵌入成功，共 {embed_count} 个嵌入点")
                return True
            else:
                logger.warning("未找到合适的嵌入点")
                return False

        except Exception as e:
            logger.error(f"Excel 水印嵌入失败: {e}")
            return False

    @classmethod
    def embed_to_ppt(
        cls,
        filepath: Path,
        payload: str,
        output_path: Optional[Path] = None,
    ) -> bool:
        """
        在 PPT 文件中嵌入水印
        
        策略：所有文本框末尾嵌入
        
        Args:
            filepath: PPT 文件路径
            payload: 水印内容
            output_path: 输出路径（None 则覆盖原文件）
            
        Returns:
            是否成功
        """
        try:
            from pptx import Presentation

            watermark = cls.encode(payload)
            prs = Presentation(str(filepath))

            embed_count = 0
            for slide in prs.slides:
                for shape in slide.shapes:
                    if shape.has_text_frame:
                        for paragraph in shape.text_frame.paragraphs:
                            if paragraph.text.strip():
                                text = paragraph.text
                                # 检查文本长度
                                if len(text) >= cls.MIN_EMBED_TEXT_LENGTH:
                                    # 在段落末尾嵌入水印
                                    if paragraph.runs:
                                        paragraph.runs[-1].text += watermark
                                    else:
                                        paragraph.text += watermark
                                    embed_count += 1

            if embed_count > 0:
                save_path = output_path or filepath
                prs.save(str(save_path))
                logger.info(f"水印嵌入成功，共 {embed_count} 个嵌入点")
                return True
            else:
                logger.warning("未找到合适的嵌入点")
                return False

        except Exception as e:
            logger.error(f"PPT 水印嵌入失败: {e}")
            return False

    @classmethod
    def create_payload(
        cls,
        operator: str,
        timestamp: Optional[str] = None,
        file_hash: Optional[str] = None,
    ) -> str:
        """
        创建水印 payload
        
        Args:
            operator: 操作人ID
            timestamp: 时间戳（ISO 8601 格式）
            file_hash: 文件哈希值
            
        Returns:
            格式化的 payload 字符串
        """
        if timestamp is None:
            timestamp = datetime.now().isoformat()

        parts = [operator, timestamp]
        if file_hash:
            parts.append(file_hash[:16])  # 只使用哈希前16位

        return "|".join(parts)

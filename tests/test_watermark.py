"""水印编解码测试"""
import pytest

from src.finance_mask.watermark.encoder import WatermarkEncoder
from src.finance_mask.watermark.decoder import WatermarkDecoder


class TestWatermark:
    """水印编解码测试"""

    def test_encode_decode_roundtrip(self):
        """测试编码-解码往返"""
        payload = "test_user|2024-01-01T00:00:00|abc123"
        encoded = WatermarkEncoder.encode(payload)
        decoded = WatermarkDecoder.decode(encoded)
        assert decoded == payload

    def test_encode_chinese(self):
        """测试中文编码"""
        payload = "张三|2024-01-01|测试"
        encoded = WatermarkEncoder.encode(payload)
        decoded = WatermarkDecoder.decode(encoded)
        assert decoded == payload

    def test_encode_empty(self):
        """测试空字符串编码"""
        payload = ""
        encoded = WatermarkEncoder.encode(payload)
        # 空字符串编码为空字符串，解码应抛出异常
        assert encoded == ""
        with pytest.raises(ValueError):
            WatermarkDecoder.decode(encoded)

    def test_has_watermark(self):
        """测试水印检测"""
        payload = "test"
        encoded = WatermarkEncoder.encode(payload)
        text = f"Hello{encoded}World"
        assert WatermarkDecoder.has_watermark(text) is True

    def test_no_watermark(self):
        """测试无水印检测"""
        text = "Hello World"
        assert WatermarkDecoder.has_watermark(text) is False

    def test_create_payload(self):
        """测试 payload 创建"""
        payload = WatermarkEncoder.create_payload(
            operator="test_user",
            timestamp="2024-01-01T00:00:00",
            file_hash="abc123def456"
        )
        assert "test_user" in payload
        assert "2024-01-01" in payload
        assert "abc123def" in payload  # 哈希前16位

    def test_encode_length(self):
        """测试编码长度（验证零宽字符数量）"""
        payload = "test"  # 4 字节 = 32 bit
        encoded = WatermarkEncoder.encode(payload)
        # 每个 bit 对应一个零宽字符
        assert len(encoded) == 32

    def test_decode_invalid_text(self):
        """测试无效文本解码"""
        with pytest.raises(ValueError):
            WatermarkDecoder.decode("Hello World")  # 无零宽字符

    def test_embed_text_length_check(self):
        """测试嵌入文本长度检查"""
        # 短文本不应嵌入
        assert WatermarkEncoder.MIN_EMBED_TEXT_LENGTH == 20


if __name__ == "__main__":
    pytest.main([__file__, "-v"])

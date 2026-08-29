"""名称脱敏测试"""
import pytest

from src.finance_mask.engine.name import NameRedactor


class TestNameRedactor:
    """名称脱敏器测试"""

    def setup_method(self):
        """每个测试前重置计数器"""
        NameRedactor.reset_counter()

    def test_alias_basic(self):
        """测试代号替换基本功能"""
        result = NameRedactor.alias("阿里巴巴集团", prefix="公司")
        assert result == "[公司A]"

    def test_alias_multiple(self):
        """测试多个机构名代号"""
        result1 = NameRedactor.alias("阿里巴巴集团", prefix="公司")
        result2 = NameRedactor.alias("腾讯科技", prefix="公司")
        assert result1 == "[公司A]"
        assert result2 == "[公司B]"

    def test_alias_with_mapping(self):
        """测试自定义映射"""
        mapping = {"阿里巴巴集团": "[电商巨头]"}
        result = NameRedactor.alias("阿里巴巴集团", mapping=mapping)
        assert result == "[电商巨头]"

    def test_alias_different_prefix(self):
        """测试不同前缀"""
        result = NameRedactor.alias("中国银行", prefix="银行")
        assert result == "[银行A]"

    def test_mask_name_chinese(self):
        """测试中文姓名遮掩"""
        result = NameRedactor.mask_name("张三")
        assert result == "张*"

    def test_mask_name_long(self):
        """测试长姓名遮掩"""
        result = NameRedactor.mask_name("欧阳修")
        assert result == "欧**"

    def test_mask_name_english(self):
        """测试英文姓名遮掩"""
        result = NameRedactor.mask_name("John Smith")
        assert result == "J* *"

    def test_mask_name_single_char(self):
        """测试单字符姓名"""
        result = NameRedactor.mask_name("张")
        assert result == "张"

    def test_mask_name_empty(self):
        """测试空姓名"""
        result = NameRedactor.mask_name("")
        assert result == ""


if __name__ == "__main__":
    pytest.main([__file__, "-v"])

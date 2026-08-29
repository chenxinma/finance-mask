"""账号脱敏测试"""
import pytest

from src.finance_mask.engine.account import AccountRedactor


class TestAccountRedactor:
    """账号脱敏器测试"""

    def test_mask_account_basic(self):
        """测试基本账号遮掩"""
        result = AccountRedactor.mask_account("6222021234567890123", keep_prefix=3, keep_suffix=4)
        assert result == "622************0123"

    def test_mask_account_with_spaces(self):
        """测试带空格的账号"""
        result = AccountRedactor.mask_account("6222 0212 3456 7890", keep_prefix=3, keep_suffix=4)
        assert result.startswith("622")
        assert result.endswith("7890")

    def test_mask_account_short(self):
        """测试短账号"""
        result = AccountRedactor.mask_account("12345", keep_prefix=3, keep_suffix=4)
        assert result == "12345"  # 太短不遮掩

    def test_mask_account_contract(self):
        """测试合同编号"""
        result = AccountRedactor.mask_account("HT-2024-001234", keep_prefix=3, keep_suffix=4)
        assert result.startswith("HT-")
        assert result.endswith("1234")

    def test_mask_account_empty(self):
        """测试空账号"""
        result = AccountRedactor.mask_account("")
        assert result == ""


if __name__ == "__main__":
    pytest.main([__file__, "-v"])

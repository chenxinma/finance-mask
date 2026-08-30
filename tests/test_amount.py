"""金额脱敏测试"""
import pytest
from decimal import Decimal

from src.finance_mask.engine.amount import AmountRedactor


class TestAmountRedactor:
    """金额脱敏器测试"""

    def test_parse_number_with_unit(self):
        """测试带中文单位的数值解析"""
        assert AmountRedactor._parse_number("1.5亿元") == Decimal("150000000")
        assert AmountRedactor._parse_number("100万元") == Decimal("1000000")
        assert AmountRedactor._parse_number("5000万元") == Decimal("50000000")
        assert AmountRedactor._parse_number("2.3万亿元") == Decimal("2300000000000")
        assert AmountRedactor._parse_number("500千元") == Decimal("500000")
        assert AmountRedactor._parse_number("100百万") == Decimal("100000000")

    def test_precision_with_unit(self):
        """测试带单位金额的精度降低"""
        # 保持原始单位，降低精度到整数
        assert AmountRedactor.precision("22.12亿元") == "22亿元"
        assert AmountRedactor.precision("22.55亿元") == "23亿元"
        assert AmountRedactor.precision("100万元") == "100万元"
        assert AmountRedactor.precision("2.3万亿元") == "2万亿元"
        assert AmountRedactor.precision("-3.14亿元") == "-3亿元"

    def test_precision_basic(self):
        """测试降低精度基本功能"""
        result = AmountRedactor.precision("12345678.90", unit="million", decimal_places=2)
        assert "12.35" in result
        assert "百万" in result

    def test_precision_with_commas(self):
        """测试带千分位的金额"""
        result = AmountRedactor.precision("1,234,567.89", unit="million", decimal_places=2)
        assert "1.23" in result

    def test_precision_negative(self):
        """测试负数金额"""
        result = AmountRedactor.precision("-1234567.89", unit="million", decimal_places=2)
        assert "-" in result
        assert "1.23" in result

    def test_precision_zero(self):
        """测试零值"""
        result = AmountRedactor.precision("0", unit="million", decimal_places=2)
        assert "0.00" in result

    def test_precision_billion(self):
        """测试亿单位"""
        result = AmountRedactor.precision("1234567890", unit="billion", decimal_places=2)
        assert "1.23" in result
        assert "亿" in result

    def test_precision_invalid_unit(self):
        """测试无效单位"""
        with pytest.raises(ValueError):
            AmountRedactor.precision("12345678.90", unit="trillion")

    def test_perturb_basic(self):
        """测试随机扰动基本功能"""
        result = AmountRedactor.perturb("1000000", percentage=5.0, seed=42)
        # 结果应该在 950000-1050000 之间
        numeric = float(result.replace(",", ""))
        assert 950000 <= numeric <= 1050000

    def test_perturb_zero_percentage(self):
        """测试零扰动百分比"""
        result = AmountRedactor.perturb("1000000", percentage=0.0, seed=42)
        assert result == "1,000,000.00"

    def test_perturb_invalid_percentage(self):
        """测试无效扰动百分比"""
        with pytest.raises(ValueError):
            AmountRedactor.perturb("1000000", percentage=200.0)

    def test_mask_basic(self):
        """测试遮掩基本功能"""
        result = AmountRedactor.mask("12345678.90")
        assert result.startswith("1")
        assert "**" in result

    def test_mask_negative(self):
        """测试负数遮掩"""
        result = AmountRedactor.mask("-12345678.90")
        assert result.startswith("-1")

    def test_mask_with_commas(self):
        """测试带千分位的遮掩"""
        result = AmountRedactor.mask("1,234,567.89")
        assert result.startswith("1")
        assert "," in result

    def test_generate_perturbation_sequence(self):
        """测试总量守恒扰动序列"""
        import random
        random.seed(42)
        values = ["1000000", "2000000", "3000000"]
        results = AmountRedactor.generate_perturbation_sequence(values, percentage=5.0)

        # 验证结果数量
        assert len(results) == 3

        # 验证总量守恒（近似）
        original_total = sum(float(v.replace(",", "")) for v in values)
        result_total = sum(float(r.replace(",", "")) for r in results)
        assert abs(original_total - result_total) < original_total * 0.01  # 允许1%误差

    def test_differential_shift_basic(self):
        """测试差分偏移基本功能"""
        from decimal import Decimal
        shift = Decimal("100000")  # 偏移10万
        result = AmountRedactor.differential_shift("1000000", shift)
        assert result == "1,100,000.00"

    def test_differential_shift_negative(self):
        """测试负数差分偏移"""
        from decimal import Decimal
        shift = Decimal("-50000")  # 偏移-5万
        result = AmountRedactor.differential_shift("1000000", shift)
        assert result == "950,000.00"

    def test_differential_shift_with_commas(self):
        """测试带千分位的差分偏移"""
        from decimal import Decimal
        shift = Decimal("10000")
        result = AmountRedactor.differential_shift("1,234,567.89", shift)
        assert result == "1,244,567.89"

    def test_generate_differential_sequence(self):
        """测试差分序列生成"""
        import random
        random.seed(42)
        values = ["1000000", "2000000", "3000000"]
        results, shift = AmountRedactor.generate_differential_sequence(values)

        # 验证结果数量
        assert len(results) == 3

        # 验证差值保持不变
        original_diff1 = float(values[1].replace(",", "")) - float(values[0].replace(",", ""))
        result_diff1 = float(results[1].replace(",", "")) - float(results[0].replace(",", ""))
        assert abs(original_diff1 - result_diff1) < 0.01  # 差值应完全相同

        original_diff2 = float(values[2].replace(",", "")) - float(values[1].replace(",", ""))
        result_diff2 = float(results[2].replace(",", "")) - float(results[1].replace(",", ""))
        assert abs(original_diff2 - result_diff2) < 0.01

    def test_generate_differential_sequence_with_range(self):
        """测试指定范围的差分序列"""
        import random
        random.seed(42)
        values = ["1000000", "2000000"]
        results, shift = AmountRedactor.generate_differential_sequence(
            values,
            shift_range=(100000, 200000),  # 偏移10-20万
        )

        # 验证偏移量在范围内
        assert Decimal("100000") <= shift <= Decimal("200000")

        # 验证所有值都应用了相同的偏移
        shift1 = float(results[0].replace(",", "")) - float(values[0].replace(",", ""))
        shift2 = float(results[1].replace(",", "")) - float(values[1].replace(",", ""))
        assert abs(shift1 - shift2) < 0.01

    def test_generate_proportional_differential_sequence(self):
        """测试比例差分序列生成"""
        import random
        random.seed(42)
        values = ["1000000", "2000000", "3000000"]
        results, scale = AmountRedactor.generate_proportional_differential_sequence(
            values,
            percentage=10.0,
        )

        # 验证结果数量
        assert len(results) == 3

        # 验证缩放因子在范围内
        from decimal import Decimal
        assert Decimal("0.9") <= scale <= Decimal("1.1")

        # 验证所有值都应用了相同的比例
        ratio1 = float(results[0].replace(",", "")) / float(values[0].replace(",", ""))
        ratio2 = float(results[1].replace(",", "")) / float(values[1].replace(",", ""))
        ratio3 = float(results[2].replace(",", "")) / float(values[2].replace(",", ""))
        assert abs(ratio1 - ratio2) < 0.001
        assert abs(ratio2 - ratio3) < 0.001

    def test_generate_differential_sequence_preserves_ratios(self):
        """测试差分序列保持比例关系"""
        import random
        random.seed(42)
        values = ["1000000", "2000000"]
        results, scale = AmountRedactor.generate_proportional_differential_sequence(
            values,
            percentage=5.0,
        )

        # 原始比例
        original_ratio = float(values[1].replace(",", "")) / float(values[0].replace(",", ""))
        # 结果比例
        result_ratio = float(results[1].replace(",", "")) / float(results[0].replace(",", ""))

        # 比例应保持不变
        assert abs(original_ratio - result_ratio) < 0.001


if __name__ == "__main__":
    pytest.main([__file__, "-v"])

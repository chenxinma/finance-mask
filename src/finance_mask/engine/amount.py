"""金额脱敏实现"""
from __future__ import annotations

import random
from decimal import Decimal, ROUND_HALF_UP
from typing import Optional


class AmountRedactor:
    """金额脱敏器"""

    # 单位映射
    UNIT_FACTORS = {
        "thousand": Decimal("1000"),
        "million": Decimal("1000000"),
        "billion": Decimal("1000000000"),
    }

    UNIT_LABELS = {
        "thousand": "千",
        "million": "百万",
        "billion": "亿",
    }

    @classmethod
    def precision(
        cls,
        value: str,
        unit: str = "million",
        decimal_places: int = 2,
    ) -> str:
        """
        降低精度模式
        
        Args:
            value: 原始值（支持千分位格式、负数、带中文单位的金额）
            unit: 目标单位 (thousand/million/billion)，当输入带单位时会被忽略
            decimal_places: 保留小数位数
            
        Returns:
            格式化后的字符串，保持原始单位
        """
        try:
            # 提取原始数值和单位
            num_part, original_unit = cls._extract_original_unit(value)
            
            # 如果输入带单位，解析数值部分（不乘以单位因子）
            if original_unit:
                # 解析纯数值部分
                clean_num = num_part.replace(",", "").replace(" ", "")
                numeric_value = Decimal(clean_num)
                # 降低精度：四舍五入到整数
                result = numeric_value.quantize(Decimal("1"), rounding=ROUND_HALF_UP)
                return f"{int(result)}{original_unit}"
            
            # 不带单位的情况，使用原有逻辑
            numeric_value = cls._parse_number(value)

            # 处理零值
            if numeric_value == 0:
                return f"0.{'0' * decimal_places}{cls.UNIT_LABELS.get(unit, '')}元"

            # 获取单位因子
            if unit not in cls.UNIT_FACTORS:
                raise ValueError(f"不支持的单位: {unit}，支持的单位: {list(cls.UNIT_FACTORS.keys())}")

            factor = cls.UNIT_FACTORS[unit]
            converted = numeric_value / factor

            # 四舍五入
            quantize_str = "0." + "0" * decimal_places
            result = converted.quantize(Decimal(quantize_str), rounding=ROUND_HALF_UP)

            # 格式化输出
            label = cls.UNIT_LABELS.get(unit, "")
            return f"{result}{label}元"

        except Exception as e:
            raise ValueError(f"金额降低精度失败: {value}, 错误: {e}")

    @classmethod
    def perturb(
        cls,
        value: str,
        percentage: float = 5.0,
        seed: Optional[int] = None,
    ) -> str:
        """
        随机扰动模式
        
        Args:
            value: 原始值
            percentage: 扰动百分比（±X%）
            seed: 随机种子（用于可复现）
            
        Returns:
            扰动后的值
        """
        try:
            numeric_value = cls._parse_number(value)

            # 输入校验
            if percentage < 0 or percentage > 100:
                raise ValueError(f"percentage 必须在 0-100 之间，当前值: {percentage}")

            if seed is not None:
                random.seed(seed)

            # 计算扰动范围
            range_factor = Decimal(str(percentage)) / Decimal("100")
            perturbation = numeric_value * range_factor * Decimal(str(random.uniform(-1, 1)))

            result = numeric_value + perturbation

            # 格式化输出（保留两位小数）
            return f"{result.quantize(Decimal('0.01'), rounding=ROUND_HALF_UP):,.2f}"

        except Exception as e:
            raise ValueError(f"金额随机扰动失败: {value}, 错误: {e}")

    @classmethod
    def mask(cls, value: str) -> str:
        """
        遮掩模式
        
        仅保留首位数字，其余替换为 *
        
        Args:
            value: 原始值
            
        Returns:
            遮掩后的字符串
        """
        try:
            # 保留原始格式信息
            is_negative = value.strip().startswith("-")
            clean_value = value.strip().lstrip("-").replace(",", "")

            # 分离整数和小数部分
            if "." in clean_value:
                int_part, dec_part = clean_value.split(".")
            else:
                int_part = clean_value
                dec_part = ""

            # 遮掩整数部分（保留首位）
            if int_part:
                masked_int = int_part[0] + "*" * (len(int_part) - 1)
            else:
                masked_int = ""

            # 格式化（添加千分位）
            if len(masked_int) > 3:
                # 从右向左每3位添加逗号
                formatted = ""
                for i, char in enumerate(reversed(masked_int)):
                    if i > 0 and i % 3 == 0:
                        formatted = "," + formatted
                    formatted = char + formatted
                masked_int = formatted

            # 遮掩小数部分
            masked_dec = "**" if dec_part else ""

            # 组合结果
            result = masked_int
            if masked_dec:
                result += "." + masked_dec

            if is_negative:
                result = "-" + result

            return result

        except Exception as e:
            raise ValueError(f"金额遮掩失败: {value}, 错误: {e}")

    @classmethod
    def _parse_number(cls, value: str) -> Decimal:
        """解析数值字符串为 Decimal，支持带中文单位的金额"""
        if not value:
            raise ValueError("空值无法解析")

        # 移除千分位逗号和空格
        clean = str(value).strip().replace(",", "").replace(" ", "")

        # 中文单位到数值的映射
        unit_multipliers = {
            "万亿": Decimal("1000000000000"),
            "亿": Decimal("100000000"),
            "百万": Decimal("1000000"),
            "万": Decimal("10000"),
            "千": Decimal("1000"),
        }

        # 尝试匹配带单位的数值
        # 例如: "1.5亿元" -> 数值=1.5, 单位=亿
        for unit_text, multiplier in unit_multipliers.items():
            if unit_text in clean:
                # 提取数值部分
                num_part = clean.split(unit_text)[0]
                # 移除可能的 "元" 后缀
                num_part = num_part.rstrip("元")
                try:
                    return Decimal(num_part) * multiplier
                except Exception:
                    raise ValueError(f"无法解析为数值: {value}")

        # 移除 "元" 后缀（如果有的话）
        if clean.endswith("元"):
            clean = clean[:-1]

        try:
            return Decimal(clean)
        except Exception:
            raise ValueError(f"无法解析为数值: {value}")

    @classmethod
    def _extract_original_unit(cls, value: str) -> tuple[str, str]:
        """
        提取原始数值和单位
        
        Args:
            value: 原始值（如 "22.12亿元"）
            
        Returns:
            (数值部分, 单位部分) 元组，如 ("22.12", "亿元")
        """
        if not value:
            return (value, "")

        clean = str(value).strip().replace(",", "").replace(" ", "")

        # 中文单位列表（按长度从长到短匹配）
        unit_patterns = ["万亿", "亿", "百万", "万", "千"]

        for unit_text in unit_patterns:
            if unit_text in clean:
                num_part = clean.split(unit_text)[0]
                # 处理可能的 "元" 后缀
                unit_part = unit_text
                remaining = clean.split(unit_text)[1]
                if remaining.startswith("元"):
                    unit_part = unit_text + "元"
                return (num_part, unit_part)

        # 检查是否有 "元" 后缀
        if clean.endswith("元"):
            return (clean[:-1], "元")

        return (clean, "")

    @classmethod
    def generate_perturbation_sequence(
        cls,
        values: list[str],
        percentage: float = 5.0,
    ) -> list[str]:
        """
        生成总量守恒的扰动序列
        
        对于一组金额，生成扰动后保持总和不变的序列
        
        Args:
            values: 原始值列表
            percentage: 扰动百分比
            
        Returns:
            扰动后的值列表
        """
        if not values:
            return []

        # 解析所有值
        numeric_values = [cls._parse_number(v) for v in values]
        total = sum(numeric_values)

        # 计算每个值的扰动范围
        range_factor = Decimal(str(percentage)) / Decimal("100")

        # 生成 N-1 个随机扰动
        perturbations = []
        for i in range(len(values) - 1):
            max_perturbation = numeric_values[i] * range_factor
            perturbation = max_perturbation * Decimal(str(random.uniform(-1, 1)))
            perturbations.append(perturbation)

        # 第 N 个扰动 = -(前 N-1 个之和)，保证总和为 0
        last_perturbation = -sum(perturbations)

        # 限幅处理：确保最后一个扰动在允许范围内
        max_last = numeric_values[-1] * range_factor
        if abs(last_perturbation) > abs(max_last):
            # 重新采样
            return cls.generate_perturbation_sequence(values, percentage)

        perturbations.append(last_perturbation)

        # 应用扰动
        results = []
        for i, (original, perturbation) in enumerate(zip(numeric_values, perturbations)):
            result = original + perturbation
            formatted = f"{result.quantize(Decimal('0.01'), rounding=ROUND_HALF_UP):,.2f}"
            results.append(formatted)

        return results

    @classmethod
    def differential_shift(
        cls,
        value: str,
        shift: Decimal,
    ) -> str:
        """
        差分偏移模式（单个值）
        
        对单个金额加上固定的偏移量，用于差分计算
        
        Args:
            value: 原始值
            shift: 偏移量（可正可负）
            
        Returns:
            偏移后的值
        """
        try:
            numeric_value = cls._parse_number(value)
            result = numeric_value + shift
            return f"{result.quantize(Decimal('0.01'), rounding=ROUND_HALF_UP):,.2f}"
        except Exception as e:
            raise ValueError(f"金额差分偏移失败: {value}, 错误: {e}")

    @classmethod
    def generate_differential_sequence(
        cls,
        values: list[str],
        shift_range: Optional[tuple[float, float]] = None,
        seed: Optional[int] = None,
    ) -> tuple[list[str], Decimal]:
        """
        生成差分扰动序列
        
        基于一个随机偏移量，对所有金额进行相同的偏移，
        保持金额之间的差值不变。
        
        适用场景：
        - 资产负债表（资产=负债+所有者权益，差值关系重要）
        - 收入支出对比（需要保持收支差额）
        - 多期对比数据（需要保持增长/下降趋势）
        
        Args:
            values: 原始值列表
            shift_range: 偏移量范围 (min, max)，默认为所有金额平均值的 ±10%
            seed: 随动种子（用于可复现）
            
        Returns:
            (偏移后的值列表, 使用的偏移量)
        """
        if not values:
            return [], Decimal("0")

        # 解析所有值
        numeric_values = [cls._parse_number(v) for v in values]

        if seed is not None:
            random.seed(seed)

        # 计算偏移量范围
        if shift_range is None:
            # 默认：平均值的 ±10%
            avg_value = sum(abs(v) for v in numeric_values) / len(numeric_values)
            max_shift = avg_value * Decimal("0.10")
            shift_min = -max_shift
            shift_max = max_shift
        else:
            shift_min = Decimal(str(shift_range[0]))
            shift_max = Decimal(str(shift_range[1]))

        # 生成随机偏移量
        shift = shift_min + (shift_max - shift_min) * Decimal(str(random.random()))

        # 对所有金额应用相同的偏移
        results = []
        for original in numeric_values:
            result = original + shift
            formatted = f"{result.quantize(Decimal('0.01'), rounding=ROUND_HALF_UP):,.2f}"
            results.append(formatted)

        return results, shift

    @classmethod
    def generate_proportional_differential_sequence(
        cls,
        values: list[str],
        percentage: float = 10.0,
        seed: Optional[int] = None,
    ) -> tuple[list[str], Decimal]:
        """
        生成比例差分扰动序列
        
        基于一个随机比例因子，对所有金额进行同比例缩放，
        保持金额之间的比例关系和差值比例不变。
        
        适用场景：
        - 需要保持金额间比例关系的场景
        - 比率指标（如毛利率、净利率）相关的金额
        
        Args:
            values: 原始值列表
            percentage: 缩放百分比范围（±X%）
            seed: 随机种子（用于可复现）
            
        Returns:
            (缩放后的值列表, 使用的缩放因子)
        """
        if not values:
            return [], Decimal("1")

        # 解析所有值
        numeric_values = [cls._parse_number(v) for v in values]

        if seed is not None:
            random.seed(seed)

        # 计算缩放因子范围
        range_factor = Decimal(str(percentage)) / Decimal("100")
        scale_min = Decimal("1") - range_factor
        scale_max = Decimal("1") + range_factor

        # 生成随机缩放因子
        scale = scale_min + (scale_max - scale_min) * Decimal(str(random.random()))

        # 对所有金额应用相同的缩放
        results = []
        for original in numeric_values:
            result = original * scale
            formatted = f"{result.quantize(Decimal('0.01'), rounding=ROUND_HALF_UP):,.2f}"
            results.append(formatted)

        return results, scale

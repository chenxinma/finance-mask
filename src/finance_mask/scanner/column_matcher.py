"""列头匹配引擎"""
from __future__ import annotations

import re
from typing import Optional

from ..models.strategy import ColumnRule, MatchType
from ..models.site import DetectedType, ActionType


# 内置列头规则
BUILTIN_COLUMN_RULES = [
    # 金额相关列
    ColumnRule(
        match_type=MatchType.EXACT,
        pattern="营业收入",
        action=ActionType.PRECISION,
        params={"unit": "million", "decimal_places": 2},
        detected_type=DetectedType.AMOUNT,
        priority=1
    ),
    ColumnRule(
        match_type=MatchType.EXACT,
        pattern="营业总收入",
        action=ActionType.PRECISION,
        params={"unit": "million", "decimal_places": 2},
        detected_type=DetectedType.AMOUNT,
        priority=1
    ),
    ColumnRule(
        match_type=MatchType.EXACT,
        pattern="净利润",
        action=ActionType.PRECISION,
        params={"unit": "million", "decimal_places": 2},
        detected_type=DetectedType.AMOUNT,
        priority=1
    ),
    ColumnRule(
        match_type=MatchType.EXACT,
        pattern="归母净利润",
        action=ActionType.PRECISION,
        params={"unit": "million", "decimal_places": 2},
        detected_type=DetectedType.AMOUNT,
        priority=1
    ),
    ColumnRule(
        match_type=MatchType.EXACT,
        pattern="总资产",
        action=ActionType.PRECISION,
        params={"unit": "billion", "decimal_places": 2},
        detected_type=DetectedType.AMOUNT,
        priority=1
    ),
    ColumnRule(
        match_type=MatchType.EXACT,
        pattern="总负债",
        action=ActionType.PRECISION,
        params={"unit": "billion", "decimal_places": 2},
        detected_type=DetectedType.AMOUNT,
        priority=1
    ),
    ColumnRule(
        match_type=MatchType.EXACT,
        pattern="净资产",
        action=ActionType.PRECISION,
        params={"unit": "billion", "decimal_places": 2},
        detected_type=DetectedType.AMOUNT,
        priority=1
    ),
    ColumnRule(
        match_type=MatchType.EXACT,
        pattern="合同金额",
        action=ActionType.PERTURB,
        params={"percentage": 5},
        detected_type=DetectedType.AMOUNT,
        priority=1
    ),
    ColumnRule(
        match_type=MatchType.EXACT,
        pattern="合同总额",
        action=ActionType.PERTURB,
        params={"percentage": 5},
        detected_type=DetectedType.AMOUNT,
        priority=1
    ),
    ColumnRule(
        match_type=MatchType.EXACT,
        pattern="现金流",
        action=ActionType.PRECISION,
        params={"unit": "million", "decimal_places": 2},
        detected_type=DetectedType.AMOUNT,
        priority=1
    ),
    ColumnRule(
        match_type=MatchType.EXACT,
        pattern="毛利率",
        action=ActionType.MASK,
        detected_type=DetectedType.AMOUNT,
        priority=1
    ),
    # 人员相关列
    ColumnRule(
        match_type=MatchType.EXACT,
        pattern="客户名称",
        action=ActionType.ALIAS,
        params={"prefix": "客户"},
        detected_type=DetectedType.ENTITY,
        priority=1
    ),
    ColumnRule(
        match_type=MatchType.EXACT,
        pattern="联系人",
        action=ActionType.MASK_NAME,
        detected_type=DetectedType.PERSON,
        priority=1
    ),
    # 账号相关列
    ColumnRule(
        match_type=MatchType.EXACT,
        pattern="账号",
        action=ActionType.MASK_ACCOUNT,
        params={"keep_prefix": 3, "keep_suffix": 4},
        detected_type=DetectedType.ACCOUNT,
        priority=1
    ),
    ColumnRule(
        match_type=MatchType.EXACT,
        pattern="合同编号",
        action=ActionType.MASK_ACCOUNT,
        params={"keep_prefix": 3, "keep_suffix": 4},
        detected_type=DetectedType.ACCOUNT,
        priority=1
    ),
    # 正则匹配规则
    ColumnRule(
        match_type=MatchType.REGEX,
        pattern=r".*金额.*",
        action=ActionType.PRECISION,
        params={"unit": "million", "decimal_places": 2},
        detected_type=DetectedType.AMOUNT,
        priority=2
    ),
    ColumnRule(
        match_type=MatchType.REGEX,
        pattern=r".*收入.*",
        action=ActionType.PRECISION,
        params={"unit": "million", "decimal_places": 2},
        detected_type=DetectedType.AMOUNT,
        priority=2
    ),
    ColumnRule(
        match_type=MatchType.REGEX,
        pattern=r".*利润.*",
        action=ActionType.PRECISION,
        params={"unit": "million", "decimal_places": 2},
        detected_type=DetectedType.AMOUNT,
        priority=2
    ),
    # 多行表头合并后的列名规则
    ColumnRule(
        match_type=MatchType.REGEX,
        pattern=r".*单据编号.*",
        action=ActionType.MASK_ACCOUNT,
        params={"keep_prefix": 3, "keep_suffix": 4},
        detected_type=DetectedType.ACCOUNT,
        priority=2
    ),
    ColumnRule(
        match_type=MatchType.REGEX,
        pattern=r".*本次入库.*",
        action=ActionType.PRECISION,
        params={"unit": "thousand", "decimal_places": 0},
        detected_type=DetectedType.AMOUNT,
        priority=2
    ),
]


class ColumnMatcher:
    """列头匹配引擎"""

    def __init__(self, custom_rules: Optional[list[ColumnRule]] = None):
        self._rules: list[ColumnRule] = BUILTIN_COLUMN_RULES.copy()
        if custom_rules:
            self._rules.extend(custom_rules)
        # 按优先级排序
        self._rules.sort(key=lambda r: r.priority)

    def match(self, header: str) -> Optional[ColumnRule]:
        """
        匹配列名，返回匹配的 ColumnRule 或 None
        
        匹配顺序：精确匹配 > 正则匹配
        """
        if not header or not isinstance(header, str):
            return None

        header = header.strip()
        if not header:
            return None

        # 第一轮：精确匹配
        for rule in self._rules:
            if rule.match_type == MatchType.EXACT:
                if rule.pattern == header:
                    return rule

        # 第二轮：正则匹配
        for rule in self._rules:
            if rule.match_type == MatchType.REGEX:
                if re.search(rule.pattern, header) is not None:
                    return rule

        return None

    def add_rule(self, rule: ColumnRule) -> None:
        """添加自定义规则"""
        self._rules.append(rule)
        self._rules.sort(key=lambda r: r.priority)

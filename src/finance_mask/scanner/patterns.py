"""正则规则库管理"""
from __future__ import annotations

import re
from dataclasses import dataclass, field
from typing import Optional

from ..models.site import DetectedType


@dataclass
class PatternRule:
    """单条正则规则"""
    name: str
    pattern: str
    detected_type: DetectedType
    description: str = ""
    compiled: re.Pattern = field(init=False, repr=False)

    def __post_init__(self):
        self.compiled = re.compile(self.pattern)


class PatternRegistry:
    """正则规则库管理器"""

    def __init__(self):
        self._rules: list[PatternRule] = []
        self._load_builtin_rules()

    def _load_builtin_rules(self) -> None:
        """加载内置规则"""
        # 金额正则 - 多模式组合，覆盖千分位/无千分位/负数/整数/小数
        amount_patterns = [
            # 千分位格式：1,234,567.89 或 -1,234,567.89
            PatternRule(
                name="amount_with_commas",
                pattern=r"-?\d{1,3}(,\d{3})+\.\d{2}",
                detected_type=DetectedType.AMOUNT,
                description="千分位金额（带小数）"
            ),
            # 千分位格式整数：1,234,567
            PatternRule(
                name="amount_with_commas_int",
                pattern=r"-?\d{1,3}(,\d{3})+",
                detected_type=DetectedType.AMOUNT,
                description="千分位金额（整数）"
            ),
            # 无千分位：12345678.90 或 -12345678.90
            PatternRule(
                name="amount_no_commas",
                pattern=r"-?\d{4,}\.\d{2}",
                detected_type=DetectedType.AMOUNT,
                description="无千分位金额（带小数）"
            ),
            # 无千分位整数：12345678
            PatternRule(
                name="amount_no_commas_int",
                pattern=r"-?\d{5,}",
                detected_type=DetectedType.AMOUNT,
                description="无千分位金额（整数，5位以上）"
            ),
        ]

        # 机构名正则 - 放宽至 2-20 个汉字，支持英文/数字前缀
        entity_patterns = [
            PatternRule(
                name="entity_chinese",
                pattern=r"[\u4e00-\u9fa5]{2,20}(公司|集团|银行|证券|基金|保险|信托|投资|控股|科技|实业|贸易)",
                detected_type=DetectedType.ENTITY,
                description="中文机构名"
            ),
            PatternRule(
                name="entity_with_prefix",
                pattern=r"[A-Za-z0-9\u4e00-\u9fa5]{1,30}(公司|集团|银行|证券|基金|有限公司)",
                detected_type=DetectedType.ENTITY,
                description="含英文/数字前缀的机构名"
            ),
        ]

        # 账号/合同号正则
        account_patterns = [
            PatternRule(
                name="bank_account",
                pattern=r"\d{4}[\s-]?\d{4}[\s-]?\d{4}[\s-]?\d{4,}",
                detected_type=DetectedType.ACCOUNT,
                description="银行卡号（16-19位）"
            ),
            PatternRule(
                name="contract_no",
                pattern=r"[A-Za-z]{2,4}[-/]?\d{4}[-/]?\d{4,}",
                detected_type=DetectedType.ACCOUNT,
                description="合同编号"
            ),
        ]

        self._rules.extend(amount_patterns)
        self._rules.extend(entity_patterns)
        self._rules.extend(account_patterns)

    def add_rule(self, rule: PatternRule) -> None:
        """添加自定义规则"""
        self._rules.append(rule)

    def get_rules(self, detected_type: Optional[DetectedType] = None) -> list[PatternRule]:
        """获取规则列表，可按类型过滤"""
        if detected_type is None:
            return self._rules.copy()
        return [r for r in self._rules if r.detected_type == detected_type]

    def scan_text(self, text: str) -> list[tuple[str, DetectedType, str]]:
        """扫描文本，返回 (匹配值, 敏感类型, 规则名) 列表"""
        results = []
        if not text or not isinstance(text, str):
            return results

        for rule in self._rules:
            for match in rule.compiled.finditer(text):
                matched_text = match.group()
                # 过滤掉纯数字且长度小于5的（可能是普通数字）
                if rule.detected_type == DetectedType.AMOUNT and matched_text.replace(",", "").replace(".", "").replace("-", "").isdigit():
                    clean = matched_text.replace(",", "").replace("-", "")
                    if len(clean) < 4:
                        continue
                results.append((matched_text, rule.detected_type, rule.name))

        return results

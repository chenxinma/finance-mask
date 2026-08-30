"""列头匹配引擎"""
from __future__ import annotations

import json
import re
from pathlib import Path
from typing import Optional

from ..models.strategy import ColumnRule, MatchType
from ..models.site import DetectedType, ActionType

# 默认配置文件路径
DEFAULT_CONFIG_PATH = Path(__file__).parent.parent.parent.parent / "config" / "column_rules.json"


def _load_rules_from_config(config_path: Path) -> list[ColumnRule]:
    """从配置文件加载列头规则"""
    if not config_path.exists():
        raise FileNotFoundError(f"配置文件不存在: {config_path}")
    
    with open(config_path, 'r', encoding='utf-8') as f:
        config = json.load(f)
    
    # 类型映射
    match_type_map = {
        "exact": MatchType.EXACT,
        "regex": MatchType.REGEX,
        "position": MatchType.POSITION,
    }
    
    action_type_map = {
        "precision": ActionType.PRECISION,
        "perturb": ActionType.PERTURB,
        "mask": ActionType.MASK,
        "alias": ActionType.ALIAS,
        "mask_name": ActionType.MASK_NAME,
        "mask_account": ActionType.MASK_ACCOUNT,
        "differential_shift": ActionType.DIFFERENTIAL_SHIFT,
        "proportional_scale": ActionType.PROPORTIONAL_SCALE,
    }
    
    detected_type_map = {
        "amount": DetectedType.AMOUNT,
        "entity": DetectedType.ENTITY,
        "person": DetectedType.PERSON,
        "account": DetectedType.ACCOUNT,
    }
    
    rules = []
    for rule_data in config.get("rules", []):
        rule = ColumnRule(
            match_type=match_type_map[rule_data["match_type"]],
            pattern=rule_data["pattern"],
            action=action_type_map[rule_data["action"]],
            params=rule_data.get("params"),
            detected_type=detected_type_map[rule_data["detected_type"]],
            priority=rule_data.get("priority", 0),
        )
        rules.append(rule)
    
    return rules


class ColumnMatcher:
    """列头匹配引擎"""

    def __init__(self, custom_rules: Optional[list[ColumnRule]] = None, config_path: Optional[Path] = None):
        self._rules: list[ColumnRule] = _load_rules_from_config(config_path or DEFAULT_CONFIG_PATH)
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

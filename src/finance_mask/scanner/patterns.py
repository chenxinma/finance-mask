"""正则规则库管理"""
from __future__ import annotations

import json
import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional

from ..models.site import DetectedType

# 默认配置文件路径
DEFAULT_CONFIG_PATH = Path(__file__).parent.parent.parent.parent / "config" / "pattern_rules.json"


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

    def __init__(self, config_path: Optional[Path] = None):
        self._rules: list[PatternRule] = []
        self._config_path = config_path
        self._load_builtin_rules()

    def _load_builtin_rules(self) -> None:
        """从配置文件加载规则"""
        config_path = self._config_path or DEFAULT_CONFIG_PATH
        
        if not config_path.exists():
            raise FileNotFoundError(f"配置文件不存在: {config_path}")
        
        with open(config_path, 'r', encoding='utf-8') as f:
            config = json.load(f)
        
        # 类型映射
        type_map = {
            "amount": DetectedType.AMOUNT,
            "entity": DetectedType.ENTITY,
            "person": DetectedType.PERSON,
            "account": DetectedType.ACCOUNT,
        }
        
        # 加载各类型规则
        for type_name, rules in config.get("rules", {}).items():
            detected_type = type_map.get(type_name)
            if not detected_type:
                continue
            
            for rule_data in rules:
                rule = PatternRule(
                    name=rule_data["name"],
                    pattern=rule_data["pattern"],
                    detected_type=detected_type,
                    description=rule_data.get("description", ""),
                )
                self._rules.append(rule)

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

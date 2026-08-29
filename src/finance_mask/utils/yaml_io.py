"""YAML 策略读写（保留注释/格式）"""
from __future__ import annotations

import logging
from pathlib import Path
from typing import Optional

from ruamel.yaml import YAML
from ruamel.yaml.comments import CommentedMap, CommentedSeq

from ..models.site import Site, Location, SiteType, DetectedType, ActionType, DiscoveredBy
from ..models.strategy import Strategy, Metadata, ColumnRule, MatchType

logger = logging.getLogger(__name__)


def load_strategy(filepath: Path) -> Strategy:
    """
    加载 YAML 策略文件
    
    Args:
        filepath: 策略文件路径
        
    Returns:
        Strategy 对象
    """
    yaml = YAML()
    yaml.preserve_quotes = True

    try:
        with open(filepath, "r", encoding="utf-8") as f:
            data = yaml.load(f)
    except Exception as e:
        raise ValueError(f"YAML 文件加载失败: {e}")

    # 解析 metadata
    metadata_dict = data.get("metadata", {})
    metadata = Metadata(
        version=metadata_dict.get("version", "1.0"),
        source_file=metadata_dict.get("source_file", ""),
        source_hash=metadata_dict.get("source_hash"),
        generated_at=metadata_dict.get("generated_at", ""),
        total_sites=metadata_dict.get("total_sites", 0),
    )

    # 解析 column_rules
    column_rules = []
    for rule_dict in data.get("column_rules", []):
        rule = ColumnRule(
            match_type=MatchType(rule_dict.get("match_type", "exact")),
            pattern=rule_dict.get("pattern", ""),
            action=ActionType(rule_dict.get("action", "mask")),
            params=rule_dict.get("params"),
            detected_type=DetectedType(rule_dict.get("detected_type", "amount")),
            priority=rule_dict.get("priority", 0),
        )
        column_rules.append(rule)

    # 解析 sites
    sites = []
    for site_dict in data.get("sites", []):
        location_dict = site_dict.get("location", {})
        location = Location(
            type=SiteType(location_dict.get("type", "excel")),
            sheet=location_dict.get("sheet"),
            cell=location_dict.get("cell"),
            column=location_dict.get("column"),
            slide=location_dict.get("slide"),
            shape_id=location_dict.get("shape_id"),
            table_location=location_dict.get("table_location"),
        )
        site = Site(
            site_id=site_dict.get("site_id", ""),
            location=location,
            original_value=site_dict.get("original_value", ""),
            detected_type=DetectedType(site_dict.get("detected_type", "amount")),
            discovered_by=DiscoveredBy(site_dict.get("discovered_by", "fulltext_scan")),
            enabled=site_dict.get("enabled", True),
            action=ActionType(site_dict.get("action", "mask")),
            params=site_dict.get("params"),
        )
        sites.append(site)

    return Strategy(
        metadata=metadata,
        column_rules=column_rules if column_rules else None,
        sites=sites,
        global_params=data.get("global_params"),
    )


def export_strategy(
    sites: list[Site],
    source_file: str,
    source_hash: Optional[str] = None,
    column_rules: Optional[list[ColumnRule]] = None,
    output_path: Optional[Path] = None,
) -> Path:
    """
    导出 Site 列表为 YAML 文件
    
    Args:
        sites: 位点列表
        source_file: 源文件名
        source_hash: 源文件哈希
        column_rules: 列头规则
        output_path: 输出路径
        
    Returns:
        YAML 文件路径
    """
    from datetime import datetime

    yaml = YAML()
    yaml.default_flow_style = False
    yaml.allow_unicode = True

    # 构建数据结构
    data = CommentedMap()

    # metadata
    metadata = CommentedMap()
    metadata["version"] = "1.0"
    metadata["source_file"] = source_file
    if source_hash:
        metadata["source_hash"] = source_hash
    metadata["generated_at"] = datetime.now().isoformat()
    metadata["total_sites"] = len(sites)
    data["metadata"] = metadata

    # column_rules
    if column_rules:
        rules_seq = CommentedSeq()
        for rule in column_rules:
            rule_map = CommentedMap()
            rule_map["match_type"] = rule.match_type.value
            rule_map["pattern"] = rule.pattern
            rule_map["action"] = rule.action.value
            if rule.params:
                rule_map["params"] = rule.params
            rule_map["detected_type"] = rule.detected_type.value
            rules_seq.append(rule_map)
        data["column_rules"] = rules_seq

    # sites
    sites_seq = CommentedSeq()
    for site in sites:
        site_map = CommentedMap()
        site_map["site_id"] = site.site_id

        # location
        location_map = CommentedMap()
        location_map["type"] = site.location.type.value
        if site.location.sheet:
            location_map["sheet"] = site.location.sheet
        if site.location.cell:
            location_map["cell"] = site.location.cell
        if site.location.column:
            location_map["column"] = site.location.column
        if site.location.slide:
            location_map["slide"] = site.location.slide
        if site.location.shape_id:
            location_map["shape_id"] = site.location.shape_id
        if site.location.table_location:
            location_map["table_location"] = site.location.table_location
        site_map["location"] = location_map

        site_map["original_value"] = site.original_value
        site_map["detected_type"] = site.detected_type.value
        site_map["discovered_by"] = site.discovered_by.value
        site_map["enabled"] = site.enabled
        site_map["action"] = site.action.value
        if site.params:
            site_map["params"] = site.params

        # 添加注释
        site_map.yaml_add_eol_comment(
            f"# {site.detected_type.value}: {site.original_value[:30]}",
            "original_value",
        )

        sites_seq.append(site_map)

    data["sites"] = sites_seq

    # 写入文件
    if output_path is None:
        output_path = Path(f"{source_file}_策略.yaml")

    with open(output_path, "w", encoding="utf-8") as f:
        yaml.dump(data, f)

    logger.info(f"策略文件已导出: {output_path}")
    return output_path

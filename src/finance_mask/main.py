"""CLI 入口"""
from __future__ import annotations

import logging
import sys
from pathlib import Path
from typing import Optional

import click
from rich.console import Console
from rich.logging import RichHandler
from rich.progress import Progress, SpinnerColumn, TextColumn, BarColumn, TaskProgressColumn
from rich.table import Table

from .models.site import Site
from .models.strategy import Strategy, Metadata, ColumnRule
from .scanner import ExcelScannerV2, PPTScanner
from .engine import Executor
from .watermark import WatermarkEncoder
from .audit import AuditLogExporter
from .utils import load_strategy, export_strategy, walk_files, file_hash
from .utils.file_utils import ensure_output_dir, get_output_filename, get_log_filename

console = Console()

# 配置日志
logging.basicConfig(
    level=logging.INFO,
    format="%(message)s",
    handlers=[RichHandler(console=console, rich_tracebacks=True)],
)
logger = logging.getLogger(__name__)


@click.group()
@click.version_option(version="0.1.0", prog_name="finance-mask")
def cli():
    """财务文件智能脱敏工具 - 自动扫描、脱敏、水印嵌入"""
    pass


@cli.command()
@click.option("--input", "-i", required=True, type=click.Path(exists=True), help="输入文件或文件夹路径")
@click.option("--output", "-o", required=True, type=click.Path(), help="输出的策略文件路径（.yaml）")
@click.option("--verbose", "-v", is_flag=True, help="显示详细日志")
def generate(input: str, output: str, verbose: bool):
    """生成脱敏策略文件"""
    if verbose:
        logging.getLogger().setLevel(logging.DEBUG)

    input_path = Path(input)
    output_path = Path(output)

    console.print(f"[bold blue]开始扫描文件: {input_path}[/bold blue]")

    # 初始化扫描器（使用 V2 版本支持 Data/Form 类型识别）
    excel_scanner = ExcelScannerV2()
    ppt_scanner = PPTScanner()

    all_sites: list[Site] = []
    all_column_rules: list[ColumnRule] = []
    files_processed = 0

    with Progress(
        SpinnerColumn(),
        TextColumn("[progress.description]{task.description}"),
        BarColumn(),
        TaskProgressColumn(),
        console=console,
    ) as progress:
        # 收集所有文件
        files = list(walk_files(input_path))
        task = progress.add_task("扫描文件...", total=len(files))

        for file_path in files:
            progress.update(task, description=f"扫描: {file_path.name}")

            try:
                if file_path.suffix.lower() == ".xlsx":
                    # 对于 Excel 文件，获取列级别规则
                    column_rules_map = excel_scanner.get_column_rules(file_path)
                    for sheet_name, rules in column_rules_map.items():
                        all_column_rules.extend(rules)
                    
                    # 只有当没有列规则时才获取位点（用于 Form 类型表格）
                    if not column_rules_map:
                        sites = excel_scanner.scan(file_path)
                        all_sites.extend(sites)
                elif file_path.suffix.lower() == ".pptx":
                    sites = ppt_scanner.scan(file_path)
                    all_sites.extend(sites)
                else:
                    continue

                files_processed += 1
                logger.info(f"文件 {file_path.name}: 列规则 {len(all_column_rules)} 个, 位点 {len(all_sites)} 个")

            except Exception as e:
                logger.error(f"扫描文件 {file_path.name} 失败: {e}")

            progress.advance(task)

    # 去重（合并敏感类型取并集）
    all_sites = _deduplicate_sites(all_sites)
    
    # 去重列规则
    unique_column_rules = []
    seen_patterns = set()
    for rule in all_column_rules:
        if rule.pattern not in seen_patterns:
            unique_column_rules.append(rule)
            seen_patterns.add(rule.pattern)

    # 计算源文件哈希
    source_hash_value = None
    if input_path.is_file():
        source_hash_value = file_hash(input_path)

    # 导出策略文件
    strategy_path = export_strategy(
        sites=all_sites,
        source_file=input_path.name,
        source_hash=source_hash_value,
        output_path=output_path,
        column_rules=unique_column_rules if unique_column_rules else None,
    )

    # 显示摘要
    console.print("\n[bold green]扫描完成！[/bold green]")
    _print_scan_summary(all_sites, files_processed, strategy_path, unique_column_rules)


@cli.command()
@click.option("--input", "-i", required=True, type=click.Path(exists=True), help="输入文件或文件夹路径")
@click.option("--strategy", "-s", type=click.Path(exists=True), help="策略文件路径")
@click.option("--default-policy", is_flag=True, help="使用内置默认策略")
@click.option("--output", "-o", required=True, type=click.Path(), help="输出目录")
@click.option("--operator", default=None, help="操作人ID（默认取系统用户名）")
@click.option("--verbose", "-v", is_flag=True, help="显示详细日志")
@click.option("--force", "-f", is_flag=True, help="覆盖已存在的输出文件")
@click.option("--dry-run", is_flag=True, help="仅预览拟修改位点，不实际执行")
def redact(
    input: str,
    strategy: Optional[str],
    default_policy: bool,
    output: str,
    operator: Optional[str],
    verbose: bool,
    force: bool,
    dry_run: bool,
):
    """执行脱敏"""
    if verbose:
        logging.getLogger().setLevel(logging.DEBUG)

    # 参数校验
    if not strategy and not default_policy:
        console.print("[bold red]错误: 必须指定 --strategy 或 --default-policy[/bold red]")
        sys.exit(1)

    input_path = Path(input)
    output_dir = Path(output)
    ensure_output_dir(output_dir)

    # 获取操作人
    if operator is None:
        import getpass
        operator = getpass.getuser()

    console.print(f"[bold blue]开始脱敏处理: {input_path}[/bold blue]")

    # 加载策略
    if strategy:
        strategy_path = Path(strategy)
        strategy_obj = load_strategy(strategy_path)
        console.print(f"已加载策略文件: {strategy_path}")
    else:
        # 使用默认策略：先扫描再生成策略
        console.print("[yellow]使用默认策略模式：先扫描文件...[/yellow]")
        strategy_obj = _generate_default_strategy(input_path)

    # 处理文件
    files = list(walk_files(input_path))
    total_files = len(files)
    success_count = 0
    fail_count = 0
    total_changes = 0

    with Progress(
        SpinnerColumn(),
        TextColumn("[progress.description]{task.description}"),
        BarColumn(),
        TaskProgressColumn(),
        console=console,
    ) as progress:
        task = progress.add_task("脱敏处理...", total=total_files)

        for file_path in files:
            progress.update(task, description=f"处理: {file_path.name}")

            try:
                # 生成输出路径
                output_file = get_output_filename(file_path, output_dir)
                log_file = get_log_filename(file_path, output_dir)

                # 检查文件是否已存在
                if output_file.exists() and not force:
                    console.print(f"[yellow]跳过: {output_file.name} 已存在（使用 --force 覆盖）[/yellow]")
                    continue

                # 执行脱敏
                executor = Executor(strategy_obj, operator=operator)
                result = executor.execute(
                    input_path=file_path,
                    output_path=output_file,
                    dry_run=dry_run,
                )

                if result["success"]:
                    # 导出审计日志
                    if not dry_run:
                        output_hash = file_hash(output_file)
                        AuditLogExporter.export(
                            logger=executor.audit_logger,
                            output_path=log_file,
                            file_hash=output_hash,
                        )

                        # 嵌入水印
                        payload = WatermarkEncoder.create_payload(
                            operator=operator,
                            file_hash=output_hash,
                        )
                        if file_path.suffix.lower() == ".xlsx":
                            WatermarkEncoder.embed_to_excel(output_file, payload)
                        elif file_path.suffix.lower() == ".pptx":
                            WatermarkEncoder.embed_to_ppt(output_file, payload)

                    success_count += 1
                    total_changes += result["processed"]
                    logger.info(f"文件 {file_path.name}: 处理 {result['processed']} 个位点")
                else:
                    fail_count += 1
                    logger.error(f"文件 {file_path.name} 处理失败: {result.get('error', '未知错误')}")

            except Exception as e:
                fail_count += 1
                logger.error(f"文件 {file_path.name} 处理异常: {e}")

            progress.advance(task)

    # 显示摘要
    console.print("\n[bold green]脱敏处理完成！[/bold green]")
    _print_redact_summary(success_count, fail_count, total_changes, dry_run)


def _deduplicate_sites(sites: list[Site]) -> list[Site]:
    """去重：同一位置的位点合并敏感类型取并集"""
    site_map: dict[str, Site] = {}

    for site in sites:
        # 使用 location 作为去重键
        location_key = str(site.location)

        if location_key in site_map:
            existing = site_map[location_key]
            # 如果新位点的敏感类型与现有不同，保留原始值但更新类型
            if site.detected_type != existing.detected_type:
                # 优先保留列头定位的结果
                if site.discovered_by.value == "column_rule":
                    site_map[location_key] = site
        else:
            site_map[location_key] = site

    return list(site_map.values())


def _generate_default_strategy(input_path: Path) -> Strategy:
    """生成默认策略"""
    from datetime import datetime

    # 使用 V2 版本支持 Data/Form 类型识别
    excel_scanner = ExcelScannerV2()
    ppt_scanner = PPTScanner()

    all_sites: list[Site] = []

    for file_path in walk_files(input_path):
        try:
            if file_path.suffix.lower() == ".xlsx":
                sites = excel_scanner.scan(file_path)
            elif file_path.suffix.lower() == ".pptx":
                sites = ppt_scanner.scan(file_path)
            else:
                continue
            all_sites.extend(sites)
        except Exception as e:
            logger.error(f"扫描文件 {file_path.name} 失败: {e}")

    all_sites = _deduplicate_sites(all_sites)

    metadata = Metadata(
        version="1.0",
        source_file=input_path.name,
        generated_at=datetime.now().isoformat(),
        total_sites=len(all_sites),
    )

    return Strategy(
        metadata=metadata,
        sites=all_sites,
    )


def _print_scan_summary(
    sites: list[Site],
    files_processed: int,
    strategy_path: Path,
    column_rules: list[ColumnRule] = None,
) -> None:
    """打印扫描摘要"""
    table = Table(title="扫描结果摘要")
    table.add_column("指标", style="cyan")
    table.add_column("数值", style="green")

    table.add_row("处理文件数", str(files_processed))
    table.add_row("发现位点总数", str(len(sites)))

    # 按类型统计
    type_counts = {}
    for site in sites:
        type_name = site.detected_type.value
        type_counts[type_name] = type_counts.get(type_name, 0) + 1

    for type_name, count in type_counts.items():
        table.add_row(f"  - {type_name}", str(count))

    # 按发现方式统计
    column_rule_count = sum(1 for s in sites if s.discovered_by.value == "column_rule")
    fulltext_count = sum(1 for s in sites if s.discovered_by.value == "fulltext_scan")
    table.add_row("列头定位发现", str(column_rule_count))
    table.add_row("全文扫描发现", str(fulltext_count))

    # 显示列规则
    if column_rules:
        table.add_row("列级别规则", str(len(column_rules)))
        for rule in column_rules[:3]:  # 只显示前3个
            table.add_row(f"  - {rule.pattern}", f"{rule.action.value} ({rule.detected_type.value})")
        if len(column_rules) > 3:
            table.add_row("  ...", f"还有 {len(column_rules) - 3} 个规则")

    table.add_row("策略文件", str(strategy_path))

    console.print(table)
    console.print("\n[yellow]提示: 请审核策略文件，调整 enabled 和 action 后执行脱敏[/yellow]")


def _print_redact_summary(
    success_count: int,
    fail_count: int,
    total_changes: int,
    dry_run: bool,
) -> None:
    """打印脱敏摘要"""
    table = Table(title="脱敏结果摘要")
    table.add_column("指标", style="cyan")
    table.add_column("数值", style="green")

    if dry_run:
        table.add_row("模式", "[yellow]预览模式（dry-run）[/yellow]")

    table.add_row("成功文件数", str(success_count))
    table.add_row("失败文件数", str(fail_count))
    table.add_row("总修改位点数", str(total_changes))

    console.print(table)

    if dry_run:
        console.print("\n[yellow]提示: 当前为预览模式，未实际修改文件。去掉 --dry-run 参数执行实际脱敏[/yellow]")


def main():
    """主入口"""
    cli()


if __name__ == "__main__":
    main()

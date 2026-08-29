"""独立的水印解码工具"""
from __future__ import annotations

import sys
from pathlib import Path

import click
from rich.console import Console
from rich.table import Table

from .watermark.decoder import WatermarkDecoder

console = Console()


@click.command()
@click.argument("filepath", type=click.Path(exists=True))
def decode_watermark(filepath: str):
    """从脱敏文件中提取和验证水印"""
    file_path = Path(filepath)

    console.print(f"[bold blue]正在分析文件: {file_path}[/bold blue]")

    try:
        if file_path.suffix.lower() == ".xlsx":
            payloads = WatermarkDecoder.extract_from_excel(str(file_path))
        elif file_path.suffix.lower() == ".pptx":
            payloads = WatermarkDecoder.extract_from_ppt(str(file_path))
        else:
            console.print(f"[bold red]不支持的文件格式: {file_path.suffix}[/bold red]")
            sys.exit(1)

        if not payloads:
            console.print("[yellow]未找到水印信息[/yellow]")
            sys.exit(0)

        # 显示解码结果
        table = Table(title="水印解码结果")
        table.add_column("序号", style="cyan")
        table.add_column("操作人", style="green")
        table.add_column("时间戳", style="green")
        table.add_column("文件哈希（前16位）", style="green")

        for i, payload in enumerate(payloads, 1):
            parts = payload.split("|")
            operator = parts[0] if len(parts) > 0 else "未知"
            timestamp = parts[1] if len(parts) > 1 else "未知"
            hash_prefix = parts[2] if len(parts) > 2 else "无"

            table.add_row(str(i), operator, timestamp, hash_prefix)

        console.print(table)

    except Exception as e:
        console.print(f"[bold red]水印解码失败: {e}[/bold red]")
        sys.exit(1)


if __name__ == "__main__":
    decode_watermark()

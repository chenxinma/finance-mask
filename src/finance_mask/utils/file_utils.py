"""文件遍历、路径处理、文件哈希"""
from __future__ import annotations

import hashlib
import logging
from pathlib import Path
from typing import Generator

logger = logging.getLogger(__name__)

# 支持的文件扩展名
SUPPORTED_EXTENSIONS = {".xlsx", ".pptx"}


def walk_files(input_path: Path) -> Generator[Path, None, None]:
    """
    递归遍历文件夹，返回所有匹配扩展名的文件
    
    Args:
        input_path: 输入路径（文件或文件夹）
        
    Yields:
        文件路径
    """
    if input_path.is_file():
        if input_path.suffix.lower() in SUPPORTED_EXTENSIONS:
            yield input_path
        else:
            logger.warning(f"不支持的文件格式: {input_path}")
    elif input_path.is_dir():
        for file_path in input_path.rglob("*"):
            if file_path.is_file() and file_path.suffix.lower() in SUPPORTED_EXTENSIONS:
                yield file_path
    else:
        logger.error(f"路径不存在: {input_path}")


def file_hash(filepath: Path) -> str:
    """
    计算文件的 SHA-256 哈希值
    
    Args:
        filepath: 文件路径
        
    Returns:
        十六进制哈希字符串
    """
    sha256_hash = hashlib.sha256()
    with open(filepath, "rb") as f:
        for byte_block in iter(lambda: f.read(4096), b""):
            sha256_hash.update(byte_block)
    return sha256_hash.hexdigest()


def ensure_output_dir(output_path: Path) -> Path:
    """
    确保输出目录存在
    
    Args:
        output_path: 输出路径
        
    Returns:
        输出路径
    """
    output_path.mkdir(parents=True, exist_ok=True)
    return output_path


def get_output_filename(
    input_path: Path,
    output_dir: Path,
    suffix: str = "_脱敏",
) -> Path:
    """
    生成输出文件名
    
    Args:
        input_path: 输入文件路径
        output_dir: 输出目录
        suffix: 文件名后缀
        
    Returns:
        输出文件路径
    """
    stem = input_path.stem
    ext = input_path.suffix
    output_name = f"{stem}{suffix}{ext}"
    return output_dir / output_name


def get_log_filename(
    input_path: Path,
    output_dir: Path,
) -> Path:
    """
    生成日志文件名
    
    Args:
        input_path: 输入文件路径
        output_dir: 输出目录
        
    Returns:
        日志文件路径
    """
    stem = input_path.stem
    log_name = f"{stem}_日志.json"
    return output_dir / log_name

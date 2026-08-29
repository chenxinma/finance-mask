"""扫描器抽象基类"""
from __future__ import annotations

from abc import ABC, abstractmethod
from pathlib import Path

from ..models.site import Site


class BaseScanner(ABC):
    """扫描器抽象基类"""

    @abstractmethod
    def scan(self, filepath: Path) -> list[Site]:
        """
        扫描文件中的敏感内容
        
        Args:
            filepath: 文件路径
            
        Returns:
            位点列表
        """
        pass

    def _generate_site_id(self, prefix: str, *parts: str) -> str:
        """生成位点唯一标识符"""
        return f"{prefix}_{'_'.join(str(p) for p in parts)}"

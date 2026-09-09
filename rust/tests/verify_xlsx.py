#!/usr/bin/env python3
"""
验证 xlsx 文件是否可被 pandas (openpyxl) 正常读取。
用法: python verify_xlsx.py <file.xlsx>
输出: VALID / INVALID + 错误信息
"""
import sys
import pandas as pd

def verify(path: str) -> bool:
    try:
        # 尝试读取所有工作表
        xls = pd.ExcelFile(path)
        for sheet in xls.sheet_names:
            df = pd.read_excel(xls, sheet_name=sheet)
            # 如果读取成功，文件基本有效
        return True
    except Exception as e:
        print(f"INVALID: {e}", file=sys.stderr)
        return False

if __name__ == "__main__":
    if len(sys.argv) != 2:
        print("Usage: python verify_xlsx.py <file.xlsx>", file=sys.stderr)
        sys.exit(1)
    path = sys.argv[1]
    if verify(path):
        print("VALID")
        sys.exit(0)
    else:
        sys.exit(1)
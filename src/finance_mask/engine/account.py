"""账号/合同号脱敏实现"""
from __future__ import annotations


class AccountRedactor:
    """账号/合同号脱敏器"""

    @classmethod
    def mask_account(
        cls,
        value: str,
        keep_prefix: int = 3,
        keep_suffix: int = 4,
        mask_char: str = "*",
    ) -> str:
        """
        账号遮掩
        
        保留前N位和后M位，中间替换为 *
        
        Args:
            value: 原始账号/合同号
            keep_prefix: 保留前几位
            keep_suffix: 保留后几位
            mask_char: 遮掩字符
            
        Returns:
            遮掩后的字符串
        """
        if not value:
            return value

        value = value.strip()
        
        # 保留原始分隔符
        separators = []
        clean_chars = []
        for i, char in enumerate(value):
            if char in " -/":
                separators.append((i, char))
            else:
                clean_chars.append(char)

        clean = "".join(clean_chars)

        # 如果长度不够遮掩，直接返回
        if len(clean) <= keep_prefix + keep_suffix:
            return value

        # 遮掩
        prefix = clean[:keep_prefix]
        suffix = clean[-keep_suffix:] if keep_suffix > 0 else ""
        middle_len = len(clean) - keep_prefix - keep_suffix
        masked = prefix + mask_char * middle_len + suffix

        # 恢复分隔符位置
        result = list(masked)
        for pos, sep in separators:
            if pos < len(result):
                result.insert(pos, sep)
        
        return "".join(result)

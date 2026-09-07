//! 零宽字符水印编解码与嵌入

/// 水印编解码器
pub struct Watermark;

/// 最小嵌入文本长度
const MIN_EMBED_TEXT_LENGTH: usize = 20;

/// 零宽字符映射
const ZERO_WIDTH_SPACE: char = '\u{200B}';  // 零宽空格 -> '0'
const ZERO_WIDTH_NON_JOINER: char = '\u{200C}'; // 零宽非连接符 -> '1'

impl Watermark {
    /// 将 payload 字符串编码为零宽字符序列
    ///
    /// 1. 将 payload 转换为 UTF-8 字节
    /// 2. 将每个字节转换为 8 位二进制字符串
    /// 3. 映射: '0' -> '\u{200B}', '1' -> '\u{200C}'
    /// 4. 拼接
    pub fn encode(payload: &str) -> String {
        let mut result = String::new();
        
        for byte in payload.bytes() {
            // 将每个字节转换为 8 位二进制表示
            for i in (0..8).rev() {
                let bit = (byte >> i) & 1;
                if bit == 0 {
                    result.push(ZERO_WIDTH_SPACE);
                } else {
                    result.push(ZERO_WIDTH_NON_JOINER);
                }
            }
        }
        
        result
    }
    
    /// 从文本中提取零宽字符并解码为 payload 字符串
    ///
    /// 1. 提取所有 '\u{200B}' 和 '\u{200C}' 字符
    /// 2. 映射: '\u{200B}' -> '0', '\u{200C}' -> '1'
    /// 3. 分割为 8 位块
    /// 4. 将每个块转换为字节
    /// 5. 将字节转换为 UTF-8 字符串
    /// 6. 成功返回 Some(payload)，无水印返回 None
    pub fn decode(text: &str) -> Option<String> {
        // 提取所有零宽字符并转换为二进制字符串
        let mut binary = String::new();
        
        for c in text.chars() {
            match c {
                ZERO_WIDTH_SPACE => binary.push('0'),
                ZERO_WIDTH_NON_JOINER => binary.push('1'),
                _ => {}
            }
        }
        
        // 检查是否有零宽字符
        if binary.is_empty() {
            return None;
        }
        
        // 截断到 8 的倍数
        let len = binary.len();
        let truncated_len = len - (len % 8);
        if truncated_len == 0 {
            return None;
        }
        let binary = &binary[..truncated_len];
        
        // 转换为字节
        let mut bytes = Vec::new();
        for chunk in binary.as_bytes().chunks(8) {
            if chunk.len() == 8 {
                let byte_str = std::str::from_utf8(chunk).ok()?;
                let byte = u8::from_str_radix(byte_str, 2).ok()?;
                bytes.push(byte);
            }
        }
        
        // UTF-8 解码
        String::from_utf8(bytes).ok()
    }
    
    /// 将水印嵌入文本末尾（附加零宽字符）
    ///
    /// 1. 如果文本长度 < MIN_EMBED_TEXT_LENGTH (20) -> 返回原始文本
    /// 2. 否则: text + encode(payload)
    pub fn embed(text: &str, payload: &str) -> String {
        if text.len() < MIN_EMBED_TEXT_LENGTH {
            return text.to_string();
        }
        
        let watermark = Self::encode(payload);
        format!("{}{}", text, watermark)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_roundtrip() {
        let payload = "operator=ma;ts=2026-09-05;sha=abc123";
        let encoded = Watermark::encode(payload);
        
        // 验证编码结果只包含零宽字符
        for c in encoded.chars() {
            assert!(
                c == ZERO_WIDTH_SPACE || c == ZERO_WIDTH_NON_JOINER,
                "编码结果包含非零宽字符: {}",
                c
            );
        }
        
        // 验证解码结果
        let decoded = Watermark::decode(&encoded).expect("解码应该成功");
        assert_eq!(decoded, payload);
    }

    #[test]
    fn test_embed_and_decode() {
        let text = "这是一段足够长的文本用来承载水印xxxxxxxx";
        let payload = "test_payload";
        let marked = Watermark::embed(text, payload);
        
        // 验证原始文本被保留
        assert!(marked.starts_with(text), "原始文本应该被保留");
        
        // 验证水印可以被解码
        let decoded = Watermark::decode(&marked).expect("解码应该成功");
        assert_eq!(decoded, payload);
    }

    #[test]
    fn test_short_text_not_embedded() {
        let text = "短文本";
        let payload = "payload";
        let marked = Watermark::embed(text, payload);
        
        // 验证短文本没有被嵌入水印
        assert_eq!(marked, text, "短文本不应该被嵌入水印");
        
        // 验证没有零宽字符
        let has_zero_width = marked.chars().any(|c| c == ZERO_WIDTH_SPACE || c == ZERO_WIDTH_NON_JOINER);
        assert!(!has_zero_width, "短文本不应该包含零宽字符");
    }

    #[test]
    fn test_encode_chinese() {
        let payload = "张三|2024-01-01|测试";
        let encoded = Watermark::encode(payload);
        let decoded = Watermark::decode(&encoded).expect("解码应该成功");
        assert_eq!(decoded, payload);
    }

    #[test]
    fn test_encode_empty() {
        let payload = "";
        let encoded = Watermark::encode(payload);
        assert!(encoded.is_empty(), "空字符串编码应该为空字符串");
        
        // 空字符串解码应该返回 None
        let decoded = Watermark::decode(&encoded);
        assert!(decoded.is_none(), "空字符串解码应该返回 None");
    }

    #[test]
    fn test_decode_no_watermark() {
        let text = "Hello World";
        let decoded = Watermark::decode(text);
        assert!(decoded.is_none(), "无水印文本应该返回 None");
    }

    #[test]
    fn test_encode_length() {
        let payload = "test";  // 4 字节 = 32 位
        let encoded = Watermark::encode(payload);
        // 每个位对应一个零宽字符
        assert_eq!(encoded.chars().count(), 32, "4字节应该编码为32个零宽字符");
    }

    #[test]
    fn test_embed_minimum_length() {
        // 测试最小长度边界
        let text_19 = "a".repeat(19);  // 19 字符，小于 MIN_EMBED_TEXT_LENGTH
        let text_20 = "a".repeat(20);  // 20 字符，等于 MIN_EMBED_TEXT_LENGTH
        let payload = "test";
        
        let marked_19 = Watermark::embed(&text_19, payload);
        assert_eq!(marked_19, text_19, "19字符文本不应该被嵌入");
        
        let marked_20 = Watermark::embed(&text_20, payload);
        assert!(marked_20.len() > text_20.len(), "20字符文本应该被嵌入");
    }

    #[test]
    fn test_encode_decode_binary_mapping() {
        // 测试特定字节的二进制映射
        let payload = "A";  // ASCII 65 = 01000001
        let encoded = Watermark::encode(payload);
        
        // 验证编码长度（1字节 = 8位）
        assert_eq!(encoded.chars().count(), 8);
        
        // 验证解码
        let decoded = Watermark::decode(&encoded).expect("解码应该成功");
        assert_eq!(decoded, "A");
    }

    #[test]
    fn test_decode_partial_binary() {
        // 测试不完整的二进制（不是8的倍数）
        let mut binary = String::new();
        binary.push(ZERO_WIDTH_SPACE);  // '0'
        binary.push(ZERO_WIDTH_NON_JOINER);  // '1'
        binary.push(ZERO_WIDTH_SPACE);  // '0'
        binary.push(ZERO_WIDTH_NON_JOINER);  // '1'
        // 只有4位，不是8的倍数
        
        let decoded = Watermark::decode(&binary);
        assert!(decoded.is_none(), "不完整的二进制应该返回 None");
    }
}
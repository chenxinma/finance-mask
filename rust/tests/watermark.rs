//! watermark 集成测试 —— 对应 task-4-a 的测试要求。

use finance_mask_core::watermark::Watermark;

#[test]
fn test_encode_decode_roundtrip() {
    let payload = "operator=ma;ts=2026-09-05;sha=abc123";
    let encoded = Watermark::encode(payload);
    
    // 验证编码结果只包含零宽字符
    for c in encoded.chars() {
        assert!(
            c == '\u{200B}' || c == '\u{200C}',
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
    let has_zero_width = marked.chars().any(|c| c == '\u{200B}' || c == '\u{200C}');
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
    binary.push('\u{200B}');  // '0'
    binary.push('\u{200C}');  // '1'
    binary.push('\u{200B}');  // '0'
    binary.push('\u{200C}');  // '1'
    // 只有4位，不是8的倍数
    
    let decoded = Watermark::decode(&binary);
    assert!(decoded.is_none(), "不完整的二进制应该返回 None");
}
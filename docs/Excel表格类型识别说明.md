# Excel 表格类型识别说明

## 概述

本功能使用 Rust 动态库自动识别 Excel 工作表的类型，并根据不同类型采用不同的扫描策略：

- **Data 类型**：表头+数据的一维表格，使用 DataFrame 方式按列定义脱敏策略
- **Form 类型**：key-value 排版的表单数据，按逐单元格方式扫描

## 技术架构

```
┌─────────────────────────────────────────────────────────────┐
│                    ExcelScannerV2                           │
├─────────────────────────────────────────────────────────────┤
│  1. LayoutViewClassifier (Rust 库)                          │
│     - 识别工作表类型: Data / Form                            │
│     - 返回置信度分数                                        │
├─────────────────────────────────────────────────────────────┤
│  2. Data 类型处理流程                                       │
│     - find_header_row(): 查找表头行                         │
│     - pd.read_excel(): 读取 DataFrame                       │
│     - ColumnMatcher: 按列匹配脱敏规则                        │
├─────────────────────────────────────────────────────────────┤
│  3. Form 类型处理流程                                       │
│     - 逐单元格扫描                                         │
│     - 正则表达式匹配                                        │
│     - 全文扫描兜底                                         │
└─────────────────────────────────────────────────────────────┘
```

## 工作表类型说明

### Data 类型（一维表格）

**特征**：
- 第一行是表头（列名）
- 后续行是数据
- 每列数据类型一致
- 适合用 DataFrame 处理

**示例**：
```
| 项目     | 本期金额      | 上期金额      | 同比变动 |
|----------|---------------|---------------|----------|
| 营业收入 | 12,345,678.90 | 11,111,111.11 | 11.1%    |
| 营业成本 | 8,765,432.10  | 7,777,777.77  | 12.7%    |
| 毛利润   | 3,580,246.80  | 3,333,333.34  | 7.4%     |
```

**扫描策略**：
1. 使用 `find_header_row` 算法查找表头行
2. 使用 pandas 读取 DataFrame
3. 按列名匹配脱敏规则
4. 对匹配的列，批量处理所有数据行

### Form 类型（表单数据）

**特征**：
- key-value 排版
- 每行是一个独立的数据项
- 通常用于填写表单、信息登记等

**示例**：
```
| 公司名称     | 阿里巴巴集团           |
|--------------|------------------------|
| 法定代表人   | 张三                   |
| 注册资本     | 1000000万元            |
| 营业收入     | 1234567890.12元        |
```

**扫描策略**：
1. 逐单元格扫描
2. 尝试识别 key 作为列名
3. 对 value 进行正则匹配
4. 全文扫描兜底

## 使用方法

### 1. 命令行使用

```bash
# 生成策略（自动识别表格类型）
python -m finance_mask generate -i report.xlsx -o strategy.yaml

# 执行脱敏
python -m finance_mask redact -i report.xlsx -s strategy.yaml -o output/
```

### 2. Python API 使用

```python
from finance_mask.scanner import ExcelScannerV2

# 创建扫描器
scanner = ExcelScannerV2()

# 扫描文件
sites = scanner.scan("report.xlsx")

# 查看结果
for site in sites:
    print(f"{site.site_id}: {site.original_value} ({site.detected_type})")
```

### 3. 单独使用分类器

```python
from finance_mask.scanner import LayoutViewClassifier

# 创建分类器
classifier = LayoutViewClassifier()

# 分类工作表
classifications = classifier.classify_with_fallback("report.xlsx")

for c in classifications:
    print(f"{c.sheet_name}: {c.sheet_type} (置信度: {c.confidence:.2f})")
```

### 4. 使用表头查找算法

```python
from openpyxl import load_workbook
from finance_mask.scanner import find_header_row

# 加载工作簿
wb = load_workbook("report.xlsx")
ws = wb.active

# 查找表头行
header_row = find_header_row(ws, max_scan_rows=10)
print(f"表头行索引: {header_row}")

# 使用 pandas 读取
import pandas as pd
df = pd.read_excel("report.xlsx", header=header_row)
print(df.columns.tolist())
```

## 表头查找算法

`find_header_row` 函数使用多特征评分算法查找表头行，支持多行表头。

### 主要改进

1. **支持多行表头**：返回 `(start_row, end_row)` 而不是单行
2. **注释行识别**：自动识别并跳过注释/说明行
3. **数据行识别**：更准确地识别数据行，避免误判
4. **Shannon 熵评分**：使用信息熵计算内容多样性
5. **合并单元格支持**：正确处理合并单元格
6. **多行表头名称合并**：自动合并多行表头名称

### 评分规则

| 规则 | 特征 | 权重 | 说明 |
|------|------|------|------|
| 规则1 | 空值比例 | - | 空值超过 50% 直接排除 |
| 规则2 | 唯一值比例 | 1.0 | 表头通常唯一值多 |
| 规则3 | 数字占比 | 1.5 | 表头数字少，数据行数字多 |
| 规则4 | 中文字符占比 | 2.0 | 表头通常包含中文 |
| 规则5 | 下方行验证 | 1.0 | 下一行是数据行（数字占比>50%） |
| 规则6 | 关键字匹配 | 3.0 | 匹配候选关键字 |
| 规则7 | 内容多样性 | 2.0 | Shannon 熵计算字符类型多样性 |

### 使用方法

```python
from finance_mask.scanner import find_header_row, build_header_row

# 查找表头行范围（支持多行表头）
header_start, header_end = find_header_row(ws, candidates=None)

# 对于多行表头，合并表头名称
for col_idx in range(ws.max_column):
    combined_name = build_header_name(ws, col_idx, header_start, header_end)
    print(f"列 {col_idx}: {combined_name}")
```

### 使用候选关键字

```python
from finance_mask.scanner import find_header_row

# 指定候选关键字，提高表头识别准确率
header_candidates = ["营业收入", "净利润", "资产", "负债"]
header_start, header_end = find_header_row(ws, candidates=header_candidates)
```

## Rust 动态库

### 库文件位置

- Windows: `lib/layout_view.dll`
- Linux: `lib/liblayout_view.so`

### 回退机制

如果 Rust 库不可用，系统会自动回退到默认分类（所有工作表视为 Form 类型）：

```python
from finance_mask.scanner import LayoutViewClassifier

classifier = LayoutViewClassifier()

# 尝试使用 Rust 库分类，失败则使用默认分类
classifications = classifier.classify_with_fallback("report.xlsx")
```

## 性能对比

| 工作表类型 | 扫描方式 | 适用场景 | 性能 |
|------------|----------|----------|------|
| Data | DataFrame 批量处理 | 一维表格、报表 | 快（批量处理） |
| Form | 逐单元格扫描 | 表单、登记表 | 中等（逐个处理） |

## 示例文件

- `examples/layout_view_demo.py`：表格类型识别演示
- `examples/data_report.xlsx`：Data 类型示例
- `examples/form_report.xlsx`：Form 类型示例

## 测试

```bash
# 运行表格类型识别测试
uv run pytest tests/test_excel_scanner_v2.py -v

# 运行所有测试
uv run pytest tests/ -v
```

## 注意事项

1. **Rust 库依赖**：需要 Rust 动态库文件存在于 `lib/` 目录
2. **回退机制**：如果 Rust 库不可用，会回退到 Form 类型扫描
3. **表头查找**：对于复杂表格，可能需要手动指定 `header_candidates`
4. **置信度**：分类结果包含置信度分数，低置信度时建议人工确认

## 后续优化

1. 支持更复杂的表格结构识别（如多级表头、合并单元格）
2. 支持自定义分类规则
3. 优化 Form 类型的 key-value 识别算法
4. 支持更多文件格式（如 .xls、.csv）

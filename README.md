# 财务文件智能脱敏工具

一款纯本地运行的财务文件智能脱敏工具（Python CLI），自动扫描 XLSX / PPTX 中的敏感数据，执行可配置的脱敏策略，并嵌入隐形水印实现分发溯源。

## 功能特性

### 扫描识别

- **双表类型自动识别**：通过 Rust 动态库自动区分 Data 型（表头+数据的 DataFrame）和 Form 型（key-value 表单）表格，采用不同扫描策略
- **列头定位扫描**：匹配列头名称（精确/正则），定位整列数据行；支持多行合并表头
- **全文正则扫描**：兜底扫描全部单元格文本，识别金额、机构名、银行卡号、合同号等
- **非数据区域扫描**：Data 型表格中表头之前（公司名称、表格标题）和数据之后（注释信息）的文本也会被识别和脱敏
- **PPT 备注扫描**：自动扫描幻灯片备注中的敏感内容

### 脱敏模式

| 类型 | 模式 | 说明 | 示例 |
|------|------|------|------|
| **金额** | `precision` | 降低精度，保留原始单位 | `22.12亿元` → `22亿元` |
| | `perturb` | 随机扰动 ±X% | `12,345,678.90` → `12,234,567.89` |
| | `mask` | 首位保留，其余遮掩 | `12,345,678.90` → `1*,***,***.**` |
| | `differential_shift` | 差分偏移，保持金额间差值关系 | 所有金额加同一偏移量 |
| | `proportional_scale` | 比例缩放，保持金额间比例关系 | 所有金额乘同一缩放因子 |
| **名称** | `alias` | 代号替换，相同实体始终映射同一别名 | `天齐锂业股份有限公司` → `[公司A]` |
| | `mask_name` | 姓名遮掩 | `张三` → `张*` |
| **账号** | `mask_account` | 账号遮掩，保留前后几位 | `6222021234567890` → `622***********0123` |

### 工作流

- **策略可审**：生成 YAML 策略文件，人工审核 `enabled`、`action`、`params` 后再执行
- **审计日志**：每次执行自动记录原始值→脱敏值的 JSON 日志
- **隐形水印**：在脱敏文件中嵌入零宽字符水印（操作人+时间戳+文件哈希），支持事后溯源
- **批量处理**：支持文件夹递归扫描，单文件失败不影响其他文件

## 安装

```bash
git clone https://github.com/chenxinma/finance-mask.git
cd finance-mask

# 使用 uv（推荐）
uv sync

# 或 pip
pip install -e .
```

> 依赖 Python ≥ 3.13。Rust 动态库 `layout_view.dll` / `liblayout_view.so` 用于表格类型自动识别，已包含在 `lib/` 目录。

## 快速开始

### 1. 生成脱敏策略

```bash
finance-mask generate -i 财务报告.xlsx -o 策略.yaml
```

扫描文件后生成 YAML 策略文件，包含发现的列规则和敏感位点：

```yaml
# 列级别规则（Data 型表格，整列脱敏）
column_rules:
- match_type: exact
  pattern: 2026年上半年
  action: perturb
  params: { percentage: 10 }
  detected_type: amount

# 单元格位点（全文扫描 / 非数据区域）
sites:
- site_id: sheet_利润表_A1
  location: { type: excel, sheet: 利润表, cell: A1 }
  original_value: 天齐锂业股份有限公司
  detected_type: entity
  enabled: true
  action: alias
  params: { prefix: 公司 }
```

### 2. 审核策略

打开 `策略.yaml`，按需调整：

- `enabled: true/false` — 是否对该位点执行脱敏
- `action` — 脱敏方式（见上方模式表）
- `params` — 脱敏参数（如 `percentage`、`prefix`、`shift`）

### 3. 执行脱敏

```bash
finance-mask redact -i 财务报告.xlsx -s 策略.yaml -o ./输出/ --operator zhangsan
```

输出目录中生成：
- `财务报告_脱敏.xlsx` — 脱敏后的文件（已嵌入水印）
- `财务报告_日志.json` — 审计日志

### 4. 快速脱敏（跳过审核）

```bash
finance-mask redact -i 财务报告.xlsx --default-policy -o ./输出/
```

### 5. 预览模式

```bash
finance-mask redact -i 财务报告.xlsx -s 策略.yaml -o ./输出/ --dry-run
```

### 6. 验证水印

```bash
python -m finance_mask.decode_watermark ./输出/财务报告_脱敏.xlsx
```

## 命令参考

### generate

```bash
finance-mask generate -i <输入> -o <策略.yaml> [-v]
```

| 参数 | 简写 | 必填 | 说明 |
|------|------|------|------|
| `--input` | `-i` | 是 | 输入文件或文件夹路径 |
| `--output` | `-o` | 是 | 输出策略文件路径 |
| `--verbose` | `-v` | 否 | 显示详细日志 |

### redact

```bash
finance-mask redact -i <输入> [-s <策略.yaml> | --default-policy] -o <输出目录> [--operator <ID>] [-f] [-v] [--dry-run]
```

| 参数 | 简写 | 必填 | 说明 |
|------|------|------|------|
| `--input` | `-i` | 是 | 输入文件或文件夹路径 |
| `--strategy` | `-s` | 条件 | 策略文件路径 |
| `--default-policy` | | 条件 | 使用内置默认策略（与 `-s` 二选一） |
| `--output` | `-o` | 是 | 输出目录 |
| `--operator` | | 否 | 操作人 ID（默认取系统用户名） |
| `--force` | `-f` | 否 | 覆盖已存在的输出文件 |
| `--verbose` | `-v` | 否 | 显示详细日志 |
| `--dry-run` | | 否 | 仅预览拟修改位点，不实际执行 |

## 配置文件

### config/column_rules.json — 列头规则

用于表头定位，当单元格所在列的列头匹配规则时，整列数据按该规则脱敏。

```json
{
  "match_type": "exact",       // exact 精确匹配 / regex 正则匹配
  "pattern": "营业收入",        // 列头名称或正则
  "action": "precision",       // 脱敏动作
  "params": { "unit": "million", "decimal_places": 2 },
  "detected_type": "amount",   // amount / entity / person / account
  "priority": 1
}
```

### config/pattern_rules.json — 正则规则

用于全文正则扫描，按敏感类型分组配置：

```json
{
  "rules": {
    "amount":   [{ "name": "...", "pattern": "...", "description": "..." }],
    "entity":   [{ "name": "...", "pattern": "...", "description": "..." }],
    "person":   [],
    "account":  [{ "name": "...", "pattern": "...", "description": "..." }]
  }
}
```

## 差分脱敏策略

对于需要保持数据间数学关系的场景（如资产负债表 `资产 = 负债 + 权益`），可手动编写差分策略：

```yaml
sites:
  - site_id: sheet_资产负债表_B2
    action: differential_shift    # 差分偏移
    params: { shift: "1139426.80" }  # 所有相关单元格使用相同偏移量

  - site_id: sheet_利润表_B2
    action: proportional_scale    # 比例缩放
    params: { scale: "1.0418" }   # 所有相关单元格使用相同缩放因子
```

详见 `examples/差分策略示例.yaml`。

## 项目结构

```
finance_mask/
├── src/finance_mask/
│   ├── __main__.py              # CLI 入口
│   ├── main.py                  # Click 命令定义
│   ├── models/                  # 数据模型
│   │   ├── site.py              #   位点模型（Location, Site）
│   │   └── strategy.py          #   策略模型（Strategy, ColumnRule）
│   ├── scanner/                 # 扫描器
│   │   ├── excel_scanner_v2.py  #   Excel V2（Data/Form 双模式）
│   │   ├── ppt_scanner.py       #   PPT 扫描（含备注）
│   │   ├── layout_view.py       #   表格类型识别（Rust FFI）
│   │   ├── header_finder.py     #   表头行定位（支持多行合并）
│   │   ├── column_matcher.py    #   列头匹配器
│   │   └── patterns.py          #   正则规则库
│   ├── engine/                  # 脱敏引擎
│   │   ├── amount.py            #   金额（precision/perturb/mask/differential）
│   │   ├── name.py              #   名称（alias/mask_name）
│   │   ├── account.py           #   账号（mask_account）
│   │   └── executor.py          #   策略执行器
│   ├── watermark/               # 水印模块
│   │   ├── encoder.py           #   零宽字符编码与嵌入
│   │   └── decoder.py           #   解码与提取
│   ├── audit/                   # 审计模块
│   │   └── logger.py            #   审计日志记录与导出
│   └── utils/
│       ├── yaml_io.py           #   策略文件读写
│       └── file_utils.py        #   文件遍历、哈希等工具
├── config/
│   ├── column_rules.json        # 列头规则配置
│   └── pattern_rules.json       # 正则规则配置
├── lib/
│   └── layout_view.dll          # Rust 表格类型识别动态库
├── examples/
│   └── ...                      # 示例文件和策略
└── tests/
    └── ...                      # 单元测试
```

## 许可证

MIT License

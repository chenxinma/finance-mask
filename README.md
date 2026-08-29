# 财务文件智能脱敏工具

一款纯本地运行的财务文件智能脱敏工具（Python CLI），用于自动扫描、脱敏和水印嵌入财务文件。

## 功能特性

- **智能发现**：自动扫描 PPTX/XLSX 文件中的所有潜在敏感内容
- **双引擎扫描**：列头定位 + 全文正则，确保全面覆盖
- **策略可审**：生成可人工审核和编辑的 YAML 策略文件
- **一键执行**：按策略自动执行脱敏，保留原格式
- **可溯源**：在脱敏文件中嵌入隐形水印，实现分发文件溯源
- **批量处理**：支持文件夹批量处理，单文件失败不影响其他文件

## 安装

```bash
# 克隆项目
git clone <repository-url>
cd finance-mask

# 安装依赖
uv sync

# 或使用 pip
pip install -e .
```

## 快速开始

### 1. 生成脱敏策略

```bash
python -m finance_mask generate -i 2024年度财务报告.pptx -o 策略.yaml
```

### 2. 审核策略文件

打开生成的 `策略.yaml`，检查并调整：

- `enabled: true/false` - 是否执行脱敏
- `action` - 脱敏方式（precision/perturb/mask/alias/mask_name/mask_account）
- `params` - 脱敏参数

### 3. 执行脱敏

```bash
python -m finance_mask redact -i 2024年度财务报告.pptx -s 策略.yaml -o ./输出/ --operator finance_zhang
```

### 4. 验证水印

```bash
python -m finance_mask.decode_watermark ./输出/2024年度财务报告_脱敏.pptx
```

## 支持的脱敏模式

### 金额脱敏

| 模式                                  | 说明   | 示例                                 |
| ----------------------------------- | ---- | ---------------------------------- |
| precision                           | 降低精度 | 12,345,678.90 → 12.35百万元           |
| perturb | 随机扰动 | 12,345,678.90 → 12,234,567.89（±5%） |
| mask                                | 遮掩   | 12,345,678.90 → 1*,***,***.**      |

### 名称脱敏

| 模式        | 说明   | 示例             |
| --------- | ---- | -------------- |
| alias     | 代号替换 | 阿里巴巴集团 → [公司A] |
| mask_name | 姓名遮掩 | 张三 → 张*        |

### 账号脱敏

| 模式           | 说明   | 示例                                       |
| ------------ | ---- | ---------------------------------------- |
| mask_account | 账号遮掩 | 6222021234567890123 → 622***********0123 |

## 命令详解

### generate 命令

```bash
python -m finance_mask generate -i <输入> -o <策略文件> [-v]
```

| 参数        | 简写  | 必填  | 说明               |
| --------- | --- | --- | ---------------- |
| --input   | -i  | 是   | 输入文件或文件夹路径       |
| --output  | -o  | 是   | 输出的策略文件路径（.yaml） |
| --verbose | -v  | 否   | 显示详细日志           |

### redact 命令

```bash
python -m finance_mask redact -i <输入> [-s <策略文件> | --default-policy] -o <输出目录> [--operator <操作人>] [-v] [-f] [--dry-run]
```

| 参数               | 简写  | 必填  | 说明         |
| ---------------- | --- | --- | ---------- |
| --input          | -i  | 是   | 输入文件或文件夹路径 |
| --strategy       | -s  | 条件  | 策略文件路径     |
| --default-policy | -   | 条件  | 使用内置默认策略   |
| --output         | -o  | 是   | 输出目录       |
| --operator       | -   | 否   | 操作人ID      |
| --verbose        | -v  | 否   | 显示详细日志     |
| --force          | -f  | 否   | 覆盖已存在的输出文件 |
| --dry-run        | -   | 否   | 仅预览不实际执行   |

## 项目结构

```
finance_mask/
├── __init__.py
├── main.py              # CLI 入口
├── models/              # 数据模型
│   ├── site.py          # 位点模型
│   └── strategy.py      # 策略模型
├── scanner/             # 扫描器
│   ├── base.py          # 抽象基类
│   ├── excel_scanner.py # Excel 扫描
│   ├── ppt_scanner.py   # PPT 扫描
│   ├── column_matcher.py# 列头匹配
│   └── patterns.py      # 正则规则库
├── engine/              # 脱敏引擎
│   ├── amount.py        # 金额脱敏
│   ├── name.py          # 名称脱敏
│   ├── account.py       # 账号脱敏
│   └── executor.py      # 策略执行
├── watermark/           # 水印模块
│   ├── encoder.py       # 编码嵌入
│   └── decoder.py       # 解码提取
├── audit/               # 审计模块
│   └── logger.py        # 日志记录
├── utils/               # 工具函数
│   ├── yaml_io.py       # YAML 读写
│   └── file_utils.py    # 文件工具
└── decode_watermark.py  # 水印解码工具
```

## 依赖

- openpyxl >= 3.1.0
- python-pptx >= 1.0.0
- click >= 8.1.0
- rich >= 13.0.0
- pydantic >= 2.0.0
- ruamel.yaml >= 0.18.0

## 许可证

MIT License

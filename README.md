# 财务文件智能脱敏工具

一款纯本地运行的财务文件智能脱敏工具（Rust CLI + Tauri 桌面 UI），自动扫描 XLSX / PPTX 中的敏感数据，执行可配置的脱敏策略，并嵌入隐形水印实现分发溯源。

## 功能特性

### 扫描识别

- **双表类型自动识别**：内置分类器自动区分 Data 型（表头+数据的 DataFrame）和 Form 型（key-value 表单）表格，采用不同扫描策略
- **列头定位扫描**：匹配列头名称（精确/正则），定位整列数据行；支持多行合并表头
- **全文正则扫描**：兜底扫描全部单元格文本，识别金额、机构名、银行卡号、合同号等
- **字典实体匹配**：支持从 txt 字典文件加载实体名称（每行一个），使用 Aho-Corasick 算法 O(n) 高效匹配，适合万级词条场景；字典匹配优先级高于正则匹配
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
cd finance-mask/rust

cargo build --release
```

> 依赖 Rust 工具链（edition 2021）。二进制产物：`rust/target/release/finance-mask`（Windows 为 `finance-mask.exe`），下文示例假设其已在 PATH 中，否则用完整路径调用。
>
> 正则/列头规则在编译期内嵌进二进制；实体字典 `entity_dict.txt` 在运行时从可执行文件同级的 `config/` 目录读取，部署时需将仓库 `config/` 拷贝到二进制旁。

### 桌面 UI（Tauri）

```bash
cd ui
cargo run
```

三步向导（9:16 竖屏，面向无 IT 背景财务用户）：① 选择文件（单个 xlsx/pptx，扫描 → 生成策略）② 审核策略（卡片式位点/列规则，全文搜索定位，处理方式按类型分组中文下拉 + 参数表单，支持 YAML 预览）③ 执行脱敏（默认 dry-run 预览，单文件结果卡）。前端为纯静态文件，无 CDN/npm 依赖，适配离线内网环境；UI 设计规范见 `docs/frontend-design.md`；开发模式下字典自动回退读取仓库 `config/`。

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

## 命令参考

### generate

```bash
finance-mask generate -i <输入> -o <策略.yaml> [-v]
```

| 参数 | 简写 | 必填 | 说明 |
|------|------|------|------|
| `--input` | `-i` | 是 | 输入文件路径（.xlsx / .pptx） |
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
    "entity":   [
      { "name": "entity_chinese", "pattern": ".+(公司|集团|银行|证券|基金|有限公司)", "description": "中文机构名" },
      { "name": "entity_dict", "dict_path": "entity_dict.txt", "description": "字典实体" },
      { "name": "entity_english", "pattern": "[A-Z][A-Za-z0-9]{1,14}(公司|集团|银行|证券|基金|有限公司)", "description": "英文机构名" }
    ],
    "person":   [],
    "account":  [{ "name": "...", "pattern": "...", "description": "..." }]
  }
}
```

**字典规则**：当 `dict_path` 非空时，从指定 txt 文件加载实体词条（每行一个），使用 Aho-Corasick 算法匹配。字典匹配优先级高于正则匹配——若同一文本位置被字典命中，正则规则将跳过该区域。

### config/entity_dict.txt — 实体字典

每行一个实体名称，用于字典匹配：

```
天齐锂业
格林布什
措拉
奎纳
四川措拉
```

> 列头/正则规则在编译期内嵌，修改 `column_rules.json` / `pattern_rules.json` 后需 `cargo build --release` 重新编译；字典 `entity_dict.txt` 在运行时读取，修改后无需重新编译。

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

详见 `docs/差分脱敏模式说明.md`。

## 项目结构

```
finance-mask/
├── rust/                          # Rust 核心（finance-mask-core 库 + finance-mask CLI）
│   ├── Cargo.toml
│   ├── src/
│   │   ├── main.rs                # CLI 入口：generate / redact 子命令
│   │   ├── lib.rs                 # 模块声明
│   │   ├── models.rs              # 核心数据模型（Site / Strategy / ColumnRule）
│   │   ├── config.rs              # 配置单一来源（内嵌 config/*.json + 路径覆盖）
│   │   ├── classify.rs            # Excel 工作表 Data/Form 分类
│   │   ├── header_finder.rs       # 表头行定位（多维评分 + 多行合并表头）
│   │   ├── column_matcher.rs      # 列头匹配（exact / regex）
│   │   ├── patterns.rs            # 全文扫描规则库（fancy-regex + Aho-Corasick 字典）
│   │   ├── excel_scanner.rs       # Excel 扫描器（列规则 / Form / 全文 / 位点生成）
│   │   ├── ppt_reader.rs          # PPTX 阅读层（zip + quick-xml 解析 OOXML）
│   │   ├── ppt_scanner.rs         # PPT 扫描器（文本框 / 表格 / 备注）
│   │   ├── engine.rs              # 脱敏实现（金额 / 名称 / 账号）
│   │   ├── executor.rs            # 脱敏执行器（策略 → 写回 → 水印 → 审计日志）
│   │   ├── xmlsurgeon.rs          # xlsx 外科手术式写回（非目标内容逐字节保留）
│   │   ├── ppt_writer.rs          # pptx 外科手术式写回
│   │   ├── watermark.rs           # 零宽字符水印编解码与嵌入
│   │   ├── audit.rs               # 审计日志生成与导出
│   │   ├── yaml_io.rs             # 策略 YAML 读写
│   │   └── pipeline.rs            # CLI/UI 共享编排层（generate/redact）
│   └── tests/                     # 集成测试与 fixtures
├── ui/                            # Tauri 2 桌面 UI（path 依赖 rust/ core）
│   ├── src/main.rs                # Tauri 命令：文件选择/生成/编辑/执行
│   ├── frontend/                  # 纯静态前端（index.html/app.js/style.css，无构建链）
│   └── tauri.conf.json
├── config/
│   ├── column_rules.json          # 列头规则配置
│   ├── pattern_rules.json         # 正则规则配置
│   └── entity_dict.txt            # 实体字典（每行一个实体）
├── docs/                          # 设计文档、实施计划与用户手册
├── examples/                      # 示例文件与策略
└── python-core.tar.gz             # 已归档的 Python 版核心
```

## 许可证

MIT License

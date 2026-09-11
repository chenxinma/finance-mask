# Rust 核心引擎重写实施计划（Phase 1：core + CLI）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 Python 版 finance-mask 的扫描/脱敏/执行核心重写为 Rust（单一 crate：lib + CLI bin），以 Python 实现为行为基准，达到策略生成与执行结果的完全对等，为后续 UI 壳提供封闭核心。

**Architecture:** 单 crate `finance-mask-core`（库 + `finance-mask` 二进制），位于本仓库 `rust/` 目录。输入 JSON 配置（pattern_rules / column_rules），输出 YAML 策略（人审产物），执行脱敏并产出审计日志。xlsx/pptx 的读用 calamine，写采用"zip + quick-xml 外科手术式"只改目标单元格/文本节点、其余字节原样保留，保真度高于 openpyxl。layout-view（D:\work\rust\layout-view）源码吸收为 `classify` 模块，删除 C FFI 层。

**Tech Stack:** Rust 2021 / calamine 0.32 / quick-xml / zip / fancy-regex（patterns 含 `(?!%)` 负向前瞻，`regex` crate 不支持）/ serde + serde_json + serde_yaml / clap 4 / sha2 / chrono / comfy-table。

**Spec:** 本计划自带设计决策（2026-09 评估对话）；Python 实现（`src/finance_mask/`）与 `docs/设计评审.md` 为行为规格来源。UI 壳（Tauri / 本地 web / 内网部署）待定，另立计划，不在本计划范围。

## 全局约束

- **行为基准**：Python 版是 oracle。策略 YAML 的**语义内容**（解析后的 Strategy 结构）必须一致；注释与排版允许不同。
- **差分测试是主验收手段**：同一 fixture，Python `generate` 与 Rust `generate` 的输出解析后逐字段比对。
- **CLI 兼容**：命令名 `generate` / `redact` 及参数（`-i/-o/-s/--default-policy/--operator/-f/--dry-run/-v`）保持不变。
- **YAML 策略格式**：字段名与层级与 Python 版一致（见 Task 1 的类型定义），人工审核流程不断裂。
- **Windows 优先**（现有 dll 为 Windows），路径处理用 `Path` 不拼字符串。
- **配置单一来源**：`config/pattern_rules.json`、`config/column_rules.json` 仍为唯一真源，Rust 用 `include_str!` 内嵌 + `--config` 路径覆盖。
- **禁止占位**：每个 Task 的测试先行，测试从 Python 测试套件移植。
- 提交粒度：每个 Task 至少一次 commit，信息用 `feat(rust): ...` / `test(rust): ...`。

---

## 工作量与阶段总览

| Phase | 内容 | 风险 | 预估 |
|---|---|---|---|
| 0 | 工程初始化 + 差分测试基线 | 低 | 0.5 天 |
| 1 | 核心类型 + 规则层（models/patterns/column_matcher） | 低 | 2-3 天 |
| 2 | Excel 扫描（吸收 layout-view + excel_scanner_v2 + header_finder） | **高** | 5-8 天 |
| 3 | 脱敏动作引擎（amount/name/account） | 中 | 3-4 天 |
| 4 | 文件回写层（xlsx 外科手术 + 水印 + 审计） | **高** | 4-6 天 |
| 5 | pptx 支持（扫描 + 回写，复用 Phase 4 基础设施） | 中 | 3-5 天 |
| 6 | 执行器 + 策略 YAML IO + CLI + 端到端对等 | 中 | 3-5 天 |
| 7 | UI 壳 | 待定 | 另立计划 |

关键依赖顺序：0 → 1 → 2 → 3 → 4 → 5 → 6。Phase 3 只依赖 Phase 1，可与 Phase 2 并行。

---

## Task 0（Phase 0）：工程初始化 + 差分测试基线

**Files:**
- Create: `rust/Cargo.toml`、`rust/src/lib.rs`、`rust/src/main.rs`、`rust/tests/common/mod.rs`
- Create: `scripts/diff_test.py`（差分测试脚本，调用双端 CLI）
- Test: `rust/tests/smoke.rs`

**Interfaces:**
- Produces: crate `finance_mask_core`（lib）+ bin `finance-mask`；差分脚本 `python scripts/diff_test.py <fixture.xlsx>`，退出码 0 = 语义一致。

- [ ] **Step 0.1: 建 workspace**

```toml
# rust/Cargo.toml
[package]
name = "finance-mask-core"
version = "0.1.0"
edition = "2021"

[lib]
name = "finance_mask_core"
path = "src/lib.rs"

[[bin]]
name = "finance-mask"
path = "src/main.rs"

[dependencies]
calamine = "0.32"
quick-xml = "0.36"
zip = "2"
fancy-regex = "0.14"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"
clap = { version = "4", features = ["derive"] }
sha2 = "0.10"
chrono = "0.4"
comfy-table = "7"

[dev-dependencies]
tempfile = "3"
```

```rust
// rust/src/lib.rs —— 初始只有模块声明骨架，随 Phase 逐步填充
pub mod models;      // Phase 1
```

```rust
// rust/src/main.rs —— clap 骨架，Phase 6 补全子命令
fn main() { println!("finance-mask (rust) 0.1.0"); }
```

- [ ] **Step 0.2: 冒烟测试**

```rust
// rust/tests/smoke.rs
#[test]
fn crate_builds_and_version_constant_exists() {
    assert_eq!(finance_mask_core::VERSION, "0.1.0");
}
```

Run: `cargo test --manifest-path rust/Cargo.toml`
Expected: PASS（lib.rs 加 `pub const VERSION: &str = "0.1.0";`）

- [ ] **Step 0.3: 差分测试脚本（先立基线，后续 Phase 逐步启用断言）**

```python
#!/usr/bin/env python3
"""差分测试：Python oracle vs Rust 实现。
用法: python scripts/diff_test.py <input.xlsx> [--redact]
语义比对策略 YAML（解析后比对，忽略注释/排版差异）。
"""
import subprocess, sys, tempfile, os, yaml, pathlib

PY = ["uv", "run", "finance-mask"]
RS = ["cargo", "run", "--quiet", "--manifest-path", "rust/Cargo.toml", "--"]

def gen(cmds, input, out):
    subprocess.run(cmds + ["generate", "-i", input, "-o", out], check=True,
                   capture_output=True)

VOLATILE = {"generated_at"}  # 时间戳每次运行必不同，比对前归一化

def semantic(path):
    d = yaml.safe_load(pathlib.Path(path).read_text(encoding="utf-8"))
    md = d.get("metadata") if isinstance(d, dict) else None
    if isinstance(md, dict):
        for k in VOLATILE:
            md[k] = "<TS>"   # 差分只比语义，时间戳归一化
    return d

def main():
    input = sys.argv[1]
    with tempfile.TemporaryDirectory() as d:
        py_out, rs_out = os.path.join(d, "py.yaml"), os.path.join(d, "rs.yaml")
        gen(PY, input, py_out)
        gen(RS, input, rs_out)
        py, rs = semantic(py_out), semantic(rs_out)
        if py == rs:
            print(f"DIFF-OK: {input} (sites={len(py.get('sites', []))})")
            return 0
        print(f"DIFF-FAIL: {input}")
        # 打印首个差异路径
        for k in set(py) | set(rs):
            if py.get(k) != rs.get(k):
                print(f"  field '{k}' differs")
        return 1

if __name__ == "__main__":
    sys.exit(main())
```

Run: `python scripts/diff_test.py examples/仓库入库1.xlsx`
Expected: 本阶段 Rust 尚无 generate 命令，脚本应报告 Rust 侧失败——这是**预期失败**，作为后续 Phase 的验收入口。

- [ ] **Step 0.4: Commit**

```bash
git add rust/ scripts/diff_test.py
git commit -m "feat(rust): scaffold core crate + differential test harness"
```

**出口标准**：`cargo test` 绿；diff_test.py 可运行并正确报告当前失败。

---

## Task 1（Phase 1）：核心类型 + 规则层

**Files:**
- Create: `rust/src/models.rs`（Site/Location/Strategy 全部类型）
- Create: `rust/src/patterns.rs`（PatternRule/PatternRegistry）
- Create: `rust/src/column_matcher.rs`
- Create: `rust/src/config.rs`（内嵌默认配置 + 路径覆盖）
- Test: `rust/tests/models.rs`、`rust/tests/patterns.rs`
- Port from: `src/finance_mask/models/site.py`、`models/strategy.py`、`scanner/patterns.py`、`scanner/column_matcher.py`

**Interfaces:**
- Consumes: `config/pattern_rules.json`、`config/column_rules.json`（仓库既有）
- Produces:

```rust
// models.rs —— 与 Python pydantic 模型一一对应
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SiteType { Excel, Ppt }

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DetectedType { Amount, Entity, Person, Account }

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ActionType {
    Precision, Perturb, Mask, Alias, MaskName, MaskAccount,
    DifferentialShift, ProportionalScale,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveredBy { ColumnRule, FulltextScan }

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MatchType { Exact, Regex, Position }

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Default)]
pub struct Location {
    #[serde(rename = "type")]
    pub site_type: SiteType,
    pub sheet: Option<String>,
    pub cell: Option<String>,       // "B5"
    pub column: Option<String>,
    pub slide: Option<u32>,
    pub shape_id: Option<String>,
    pub table_location: Option<String>, // "R2C3"
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Site {
    pub site_id: String,
    pub location: Location,
    pub original_value: String,
    pub detected_type: DetectedType,
    pub discovered_by: DiscoveredBy,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub action: ActionType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redacted_value: Option<String>,
}
fn default_true() -> bool { true }

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct ColumnRule {
    pub match_type: MatchType,
    pub pattern: String,
    pub action: ActionType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
    pub detected_type: DetectedType,
    #[serde(default)]
    pub priority: i32,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Metadata {
    #[serde(default = "default_version")]
    pub version: String,
    pub source_file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,
    pub generated_at: String,   // ISO 8601
    pub total_sites: usize,
}
fn default_version() -> String { "1.0".into() }

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Strategy {
    pub metadata: Metadata,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column_rules: Option<Vec<ColumnRule>>,
    pub sites: Vec<Site>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub global_params: Option<serde_json::Value>,
}

/// Location 的 type 与定位字段校验（对应 pydantic model_validator）
pub fn validate_location(loc: &Location) -> Result<(), String> {
    match loc.site_type {
        SiteType::Excel => {
            if loc.sheet.is_none() || loc.cell.is_none() {
                Err("type=excel 时必须包含 sheet 和 cell 字段".into())
            } else { Ok(()) }
        }
        SiteType::Ppt => {
            if loc.slide.is_none() || loc.shape_id.is_none() {
                Err("type=ppt 时必须包含 slide 和 shape_id 字段".into())
            } else { Ok(()) }
        }
    }
}
```

```rust
// patterns.rs 关键接口
pub struct PatternRule {
    pub name: String,
    pub pattern: String,
    pub detected_type: DetectedType,
    pub description: String,
    compiled: fancy_regex::Regex,   // 注意：fancy-regex，因含 (?!%) 前瞻
}

pub struct PatternRegistry { rules: Vec<PatternRule> }

impl PatternRegistry {
    /// 内嵌默认规则（include_str! 引仓库 config/pattern_rules.json）
    pub fn builtin() -> Result<Self, PatternError>;
    pub fn from_path(p: &Path) -> Result<Self, PatternError>;
    pub fn rules(&self) -> &[PatternRule];
    /// 对文本跑全部规则，返回命中的 (rule, matched_text)
    pub fn scan(&self, text: &str) -> Vec<(&PatternRule, String)>;
}
```

```rust
// column_matcher.rs 关键接口（对应 column_matcher.py，104 行）
pub struct ColumnMatcher { rules: Vec<ColumnRule> }
impl ColumnMatcher {
    pub fn new(rules: Vec<ColumnRule>) -> Self;
    /// 表头文本 -> 命中的最高优先级规则
    pub fn match_header(&self, header: &str) -> Option<&ColumnRule>;
}
```

- [ ] **Step 1.1: 移植测试先行** —— 从 `tests/` 中挑 models/patterns 相关断言写成 `rust/tests/models.rs`：

```rust
use finance_mask_core::models::*;

#[test]
fn location_excel_requires_sheet_and_cell() {
    let mut loc = Location { site_type: SiteType::Excel, ..Default::default() };
    assert!(validate_location(&loc).is_err());
    loc.sheet = Some("Sheet1".into());
    loc.cell = Some("B5".into());
    assert!(validate_location(&loc).is_ok());
}

#[test]
fn strategy_yaml_roundtrip_keeps_field_names() {
    let s = r#"
metadata: {version: "1.0", source_file: a.xlsx, generated_at: "2026-09-05T00:00:00", total_sites: 1}
sites:
  - site_id: s1
    location: {type: excel, sheet: S1, cell: A1}
    original_value: "22.12亿元"
    detected_type: amount
    discovered_by: fulltext_scan
    enabled: true
    action: precision
"#;
    let strat: Strategy = serde_yaml::from_str(s).unwrap();
    assert_eq!(strat.sites[0].location.cell.as_deref(), Some("A1"));
    // 回写后仍可解析（人审 YAML 的读写闭环）
    let out = serde_yaml::to_string(&strat).unwrap();
    let strat2: Strategy = serde_yaml::from_str(&out).unwrap();
    assert_eq!(strat, strat2);
}
```

- [ ] **Step 1.2: 运行确认失败**（模块未建）
Run: `cargo test --manifest-path rust/Cargo.toml`
Expected: 编译错误（models 未定义）

- [ ] **Step 1.3: 实现 models.rs / patterns.rs / column_matcher.rs / config.rs**
移植要点：
- patterns.json 的 9 条规则中含 `(?!%)`，全部用 `fancy_regex::Regex::new`，编译失败时报规则名（Python `re` 与 fancy-regex 语法在此规则集上等价，无需改写）。
- `config.rs`：`include_str!("../../config/pattern_rules.json")` / `column_rules.json` 内嵌，`from_path` 支持覆盖；JSON schema 与 Python 侧 `type_map` 一致（amount/entity/person/account）。
- column_matcher 的匹配行为以 Python 为准（仅两种）：Exact（字符串相等）、Regex（fancy-regex is_match）。MatchType::Position 枚举值保留（serde 反序列化兼容，规则可携带 position）但不参与匹配——Python 从未实现位置匹配（仅加载不生效，无调用方），Rust 不超前实现（2026-09-06 裁决）。

- [ ] **Step 1.4: patterns 测试**

```rust
// rust/tests/patterns.rs
use finance_mask_core::patterns::PatternRegistry;

#[test]
fn builtin_rules_load_and_scan_amount() {
    let reg = PatternRegistry::builtin().unwrap();
    assert!(!reg.rules().is_empty());
    let hits = reg.scan("本项目总投资 1,234,567.89 元");
    assert!(hits.iter().any(|(r, m)| m.contains("1,234,567.89")),
        "amount pattern must hit, got: {:?}", hits.iter().map(|(r,_)| r.name.clone()).collect::<Vec<_>>());
}

#[test]
fn negative_lookahead_percent_not_matched() {
    let reg = PatternRegistry::builtin().unwrap();
    // "12,345.00%" 是百分比，不应命中金额规则（(?!%) 前瞻语义）
    let hits = reg.scan("增长率 12,345.00% 完成");
    assert!(!hits.iter().any(|(_, m)| m.contains("12,345.00")));
}
```

- [ ] **Step 1.5: 运行测试至绿**
Run: `cargo test --manifest-path rust/Cargo.toml`
Expected: PASS

- [ ] **Step 1.6: Commit**

```bash
git add rust/
git commit -m "feat(rust): core types, pattern registry, column matcher"
```

**出口标准**：类型/规则/匹配器单测绿；`Strategy` YAML 读写闭环测试通过。

---

## Task 2（Phase 2）：Excel 扫描器（吸收 layout-view）

**Files:**
- Create: `rust/src/classify.rs`（吸收 layout-view 源码）
- Create: `rust/src/header_finder.rs`
- Create: `rust/src/excel_scanner.rs`
- Test: `rust/tests/classify.rs`、`rust/tests/excel_scan.rs`
- Port from: `D:\work\rust\layout-view\src\lib.rs`、`src/finance_mask/scanner/excel_scanner_v2.py`(617行)、`scanner/header_finder.py`(562行)

**Interfaces:**
- Consumes: Task 1 的 `PatternRegistry` / `ColumnMatcher` / `Site`
- Produces:

```rust
// classify.rs —— layout-view 直接吸收：复制 lib.rs，删除底部 C FFI 段
// （classify_excel_sheets_c / free_c_string），其余 pub 保持
pub struct SheetDataDensity { /* 字段照抄 layout-view */ }
pub enum SheetType { Data, Form, Unknown }
pub struct ClassifiedSheet { pub original: SheetDataDensity, pub sheet_type: SheetType, pub classification_reason: String }
pub fn classify_excel_sheets(xlsx_path: &str) -> Result<Vec<ClassifiedSheet>, Box<dyn std::error::Error>>;

// header_finder.rs —— 对应 header_finder.py
pub struct HeaderInfo { pub row: u32, pub headers: Vec<String> }
pub fn find_header_row(rows: &[Vec<calamine::Data>]) -> Option<HeaderInfo>;  // 保留类型信息（数值率检测需要）
pub fn build_header_name(parts: &[Option<String>]) -> String; // 多行合并表头

// excel_scanner.rs —— 对应 excel_scanner_v2.py 的 ExcelScannerV2
pub struct ExcelScanner { patterns: PatternRegistry, matcher: ColumnMatcher }
impl ExcelScanner {
    pub fn new(patterns: PatternRegistry, matcher: ColumnMatcher) -> Self;
    /// 扫描单个 xlsx，产出位点（列规则定位 + 全文正则兜底 + 非数据区域）
    pub fn scan(&self, path: &Path) -> Result<Vec<Site>, ScanError>;
}
```

**移植要点（风险最高的一段，先读透再动手）：**
1. `classify.rs`：layout-view 已用 calamine 0.32，代码可直接搬（含 lazy_static 正则、香农熵混合度、density>0.46 / mix>0.35 阈值）。layout-view 的 `files/*.xlsx`（data1-6, form1-3）是现成分类测试夹具，复制到 `rust/tests/fixtures/`。
2. `header_finder.py` 中 pandas 部分（`find_header_row_from_dataframe`，建 DataFrame 判空列/类型分布找表头行）改为直接在 `Vec<Vec<calamine::Data>>` 上实现同一算法：**不引入 polars**，逐行计算"非空率/数值率/文本相似度"，与 Python 版阈值逐一对齐。
3. `excel_scanner_v2.py` 的扫描次序必须保持：先 classify（Data 走列头定位+非数据区域扫描，Form 走全单元格扫描）→ 表头定位 → 列规则匹配 → 全文正则兜底。**site_id 生成顺序 = 扫描顺序**（sheet 顺序、行序、列序），差分测试靠它对齐。
4. openpyxl 的 `get_column_letter` 坐标换算：自写 `fn col_letter(idx: u32) -> String`（A..Z, AA..），注意 Python 1-based、calamine 0-based。

- [ ] **Step 2.1: classify 吸收 + 测试**

```rust
// rust/tests/classify.rs
use finance_mask_core::classify::{classify_excel_sheets, SheetType};

#[test]
fn layout_view_fixtures_classify_as_expected() {
    for f in ["data1", "data3", "data6"] {
        let r = classify_excel_sheets(&format!("tests/fixtures/{f}.xlsx")).unwrap();
        assert!(r.iter().any(|s| s.sheet_type == SheetType::Data), "{f}");
    }
    for f in ["form1", "form2", "form3"] {
        let r = classify_excel_sheets(&format!("tests/fixtures/{f}.xlsx")).unwrap();
        assert!(r.iter().all(|s| s.sheet_type != SheetType::Data), "{f}");
    }
}
```

Run: `cargo test classify` → 绿后 commit `feat(rust): absorb layout-view as classify module`

- [ ] **Step 2.2: header_finder 移植（TDD）** —— 先把 `tests/test_multi_header.py`(142行) 与 `tests/test_excel_scanner_v2.py`(251行) 中表头相关用例译成 Rust 测试，覆盖：单行表头、多行合并表头、空行跳过、无表头（返回 None）。

- [ ] **Step 2.3: excel_scanner 移植** —— 实现 `scan()`；site_id 规则照抄 Python（格式为 `site_{n}` 递增或含 sheet 前缀，**以 Python 实现为准**，动手前先读 `excel_scanner_v2.py` 的 `_create_site*` 系列）。

- [ ] **Step 2.4: 差分验收**

```bash
python scripts/diff_test.py examples/仓库入库1.xlsx
python scripts/diff_test.py examples/sample_report.xlsx
# 多行表头夹具：用 examples/multi_header_demo.py 的 create_multi_header_workbook 逻辑
# 生成 multi_header.xlsx / single_header.xlsx，存 rust/tests/fixtures/ 并提交，同样跑差分
```

Expected: `DIFF-OK`（generate 尚未接 CLI 时，可临时在 main.rs 加 hidden 子命令调 scan→序列化）

- [ ] **Step 2.5: Commit**

```bash
git add rust/
git commit -m "feat(rust): excel scanner with header finding, parity with python generate"
```

**出口标准**：全部 fixture 差分通过（sites 数量、每个 site 的字段语义一致）。

---

## Task 3（Phase 3）：脱敏动作引擎

**Files:**
- Create: `rust/src/engine/mod.rs`、`rust/src/engine/amount.rs`、`rust/src/engine/name.rs`、`rust/src/engine/account.rs`
- Test: `rust/tests/engine_amount.rs`、`engine_name.rs`、`engine_account.rs`
- Port from: `engine/amount.py`(440行)、`engine/name.py`(120行)、`engine/account.py`(62行)；测试来自 `tests/test_amount.py`(213行)、`test_name.py`(77行)、`test_account.py`(62行)

**Interfaces:**
- Consumes: Task 1 的 `ActionType` / `params: serde_json::Value`
- Produces:

```rust
// engine/mod.rs
pub trait Redactor {
    /// 对单个值执行脱敏。params 来自 Site.params / ColumnRule.params。
    fn redact(&mut self, value: &str, action: &ActionType, params: &serde_json::Value)
        -> Result<String, RedactError>;
}

pub struct AmountRedactor { /* 千分位/单位解析状态 */ }
impl AmountRedactor { pub fn new() -> Self { Self::default() } }
pub struct NameRedactor { alias_map: std::collections::HashMap<String, String> } // 实体→别名，保一致性
pub struct AccountRedactor;

/// 差分/比例模式需要跨位点状态（同文件所有金额共享 offset/scale）
pub struct AmountGroupState { pub offset: Option<f64>, pub scale: Option<f64> }
```

**移植要点：**
1. **金额解析**是难点：千分位（`1,234,567.89`）、中文单位（`万亿|亿|百万|万|千` + 可选 `元`）、负号。Python 版的解析正则与单位换算表照抄；输出必须**保留原格式**（千分位/单位还原），`22.12亿元 → 22亿元`（precision）、`12,345,678.90 → 1*,***,***.**`（mask）。
2. **differential_shift / proportional_scale**：偏移量/缩放因子对整个文件只抽一次（首个金额位点时确定，之后复用）——对应 Python 的实现细节，动手前读 `amount.py` 中状态初始化逻辑，Rust 用 `AmountGroupState` 持有。
3. **alias 一致性**：`NameRedactor` 内 HashMap 保证"相同实体→同一别名"（`[公司A]`），分配顺序 = 首次出现顺序 = 扫描顺序，依赖 Task 2 的 site 顺序保证。
4. `mask_name`：`张三 → 张*`（首字保留）；`mask_account`：保留前 N 后 M 位，params 里 `keep_prefix` / `keep_suffix`。

- [ ] **Step 3.1: 移植测试先行** —— `test_amount.py` 的用例全量翻译（precision/perturb/mask/differential_shift/proportional_scale 五类，含中文单位与千分位边界），例如：

```rust
// rust/tests/engine_amount.rs
use finance_mask_core::engine::{AmountRedactor, Redactor};
use finance_mask_core::models::ActionType;
use serde_json::json;

#[test]
fn precision_keeps_unit() {
    let mut r = AmountRedactor::new();
    let out = r.redact("22.12亿元", &ActionType::Precision, &json!({"digits": 0})).unwrap();
    assert_eq!(out, "22亿元");
}

#[test]
fn mask_keeps_first_digit_and_separators() {
    let mut r = AmountRedactor::new();
    let out = r.redact("12,345,678.90", &ActionType::Mask, &json!({})).unwrap();
    assert_eq!(out, "1*,***,***.**");
}
```

（perturb 的随机性测试：断言结果仍在 ±X% 区间且格式合法，不断言具体值——与 Python 测试同策略）

- [ ] **Step 3.2: 运行确认失败 → 实现三个 redactor → 运行至绿**
Run: `cargo test engine_`
Expected: PASS（test_amount/test_name/test_account 全部语义等价翻译）

- [ ] **Step 3.3: Commit**

```bash
git add rust/
git commit -m "feat(rust): redaction engine (amount/name/account) with ported tests"
```

**出口标准**：Python 测试套件中 amount/name/account 的全部断言在 Rust 侧通过。

---

## Task 4（Phase 4）：文件回写层（xlsx 外科手术 + 水印 + 审计）

**Files:**
- Create: `rust/src/xmlsurgeon.rs`（zip + quick-xml 核心基础设施）
- Create: `rust/src/watermark.rs`
- Create: `rust/src/audit.rs`
- Test: `rust/tests/xmlsurgeon.rs`、`watermark.rs`
- Port from: `watermark/encoder.py`(194行)、`watermark/decoder.py`(154行)、`audit/logger.py`(144行)

**Interfaces:**
- Produces:

```rust
// xmlsurgeon.rs —— xlsx/pptx 共用（本质都是 zip+XML）
pub struct XmlSurgeon { /* zip 读入内存的 entries: Vec<(name, Vec<u8>)> */ }

impl XmlSurgeon {
    pub fn open(path: &Path) -> Result<Self, SurgeonError>;
    /// xlsx: 改某 sheet 某单元格的值（处理 sharedStrings 追加）
    pub fn set_cell_text(&mut self, sheet: &str, cell_ref: &str, new_text: &str) -> Result<(), SurgeonError>;
    /// pptx: 改某 slide 某 shape 文本（保留 run 结构，见 Task 5）
    pub fn set_run_text(&mut self, part: &str, shape_id: &str, run_idx: usize, new_text: &str) -> Result<(), SurgeonError>;
    pub fn save(&self, path: &Path) -> Result<(), SurgeonError>;
}

// watermark.rs
pub struct Watermark;
impl Watermark {
    pub fn encode(payload: &str) -> String;     // UTF-8 → 二进制 → \u{200B}/\u{200C}
    pub fn decode(text: &str) -> Option<String>;
    /// 嵌入规则照抄 Python：MIN_EMBED_TEXT_LENGTH=20，嵌入首段文本尾部
    pub fn embed(text: &str, payload: &str) -> String;
}

// audit.rs —— JSON 行审计日志
pub struct AuditLogger { out: std::fs::File }
pub struct AuditEntry { /* site_id, original, redacted, action, ts, operator */ }
```

**移植要点（本计划的核心新技术决策）：**
1. **外科手术式回写**：解压 xlsx 到内存 entries；只解析目标 `xl/worksheets/sheetN.xml`，quick-xml 事件流中定位 `<c r="B5" ...>`，其余 entry 字节原样复制。**保真度高于 openpyxl**（后者重写整个包，丢 VBA/部分图表细节）。
2. **sharedStrings 处理**（关键正确性点）：单元格 `<c r="B5" t="s"><v>37</v></c>` 指向共享字符串表。回写新文本时：向 `xl/sharedStrings.xml` 追加 `<si><t>新文本</t></si>`，更新 `count/uniqueCount` 属性，单元格 `<v>` 指向新索引。若原 cell 是 `t="str"`（公式串）或 inline，直接替换 `<v>`/`<is><t>` 文本。数字格式的金额改文本会改单元格类型——对照 Python openpyxl 赋值行为（值变字符串后 `t` 置 `str`…… 以 Python 实际产出为基准，差分比对脱敏输出文件的单格值）。
3. **水印**：payload = `操作人+时间戳+文件哈希`（sha2 计算），零宽字符映射 `0→\u{200B}`、`1→\u{200C}`；嵌入位置与 Python `embed_to_excel` 相同（首个满足最小长度的文本格，追加于尾部）。
4. **审计日志**：与 `examples/仓库入库1_日志.json` 格式逐字段对齐（它是现成黄金样本）。

- [ ] **Step 4.1: watermark 测试先行**（从 `tests/test_watermark.py` 移植）：

```rust
// rust/tests/watermark.rs
use finance_mask_core::watermark::Watermark;

#[test]
fn roundtrip() {
    let payload = "operator=ma;ts=2026-09-05;sha=abc123";
    let marked = Watermark::embed("这是一段足够长的文本用来承载水印xxxx", payload);
    assert_eq!(Watermark::decode(&marked).as_deref(), Some(payload));
}

#[test]
fn short_text_not_embedded() {
    // MIN_EMBED_TEXT_LENGTH=20：短文本原样返回
    assert_eq!(Watermark::embed("短文本", "payload"), "短文本");
}

#[test]
fn zero_width_invisible() {
    let w = Watermark::encode("A");
    assert!(w.chars().all(|c| c == '\u{200B}' || c == '\u{200C}'));
    assert_eq!(w.chars().count(), 8); // 'A' = 1 字节 = 8 bit
}
```

- [ ] **Step 4.2: 实现 watermark.rs 至绿，commit `feat(rust): zero-width watermark`**

- [ ] **Step 4.3: xmlsurgeon 测试**：

```rust
// rust/tests/xmlsurgeon.rs —— 往返保真测试（核心资产）
#[test]
fn only_target_cell_changes() {
    let mut s = XmlSurgeon::open(std::path::Path::new("tests/fixtures/data1.xlsx")).unwrap();
    s.set_cell_text("Sheet1", "B2", "REDACTED").unwrap();
    let out = tempfile::NamedTempFile::new().unwrap();
    s.save(out.path()).unwrap();
    // 断言：解压两文件，除 sheetN.xml 的 B2 值与 sharedStrings.xml 外全部字节一致
    assert!(only_entries_differ("tests/fixtures/data1.xlsx", out.path(),
        &["xl/worksheets/sheet1.xml", "xl/sharedStrings.xml"]));
    // 断言：B2 读回为 REDACTED（用 calamine 验证）
}
```

- [ ] **Step 4.4: 实现 xmlsurgeon.rs（含 sharedStrings 追加逻辑）至绿，commit `feat(rust): surgical xlsx write-back`**

- [ ] **Step 4.5: audit.rs** —— 序列化字段与 `仓库入库1_日志.json` 对齐，单测：写两条 entry → 解析回 → 字段一致。commit。

**出口标准**：保真往返测试绿（除目标 entry 外字节一致）；水印 roundtrip 绿；审计格式与黄金样本对齐。

---

## Task 5（Phase 5）：pptx 支持

**Files:**
- Create: `rust/src/ppt_scanner.rs`
- Modify: `rust/src/xmlsurgeon.rs`（如需补充 run 定位）
- Test: `rust/tests/ppt_scan.rs`
- Port from: `scanner/ppt_scanner.py`(460行)；fixture: `examples/sample_report.pptx`

**Interfaces:**
- Consumes: Task 2 的扫描框架、Task 4 的 `XmlSurgeon::set_run_text`
- Produces: `pub struct PptScanner; impl PptScanner { pub fn scan(&self, path: &Path) -> Result<Vec<Site>, ScanError>; }`

**移植要点：**
1. pptx 解剖：`ppt/slides/slideN.xml` 的 `<p:sp>`（shape，`<p:nvSpPr><p:cNvPr id=.. name=..>`）内 `<a:p>`(段落)/`<a:r>`(run)/`<a:t>`(文本)；备注在 `ppt/notesSlides/notesSlideN.xml`；图表标题在 chart XML。用 quick-xml 事件流遍历，**不建 DOM**。
2. 扫描面照抄 Python：shape 文本框、表格（`<a:tbl>` 的 RxCy 定位）、图表标题、备注。命中文本→Site（`SiteType::Ppt`，shape_id 用 `<p:cNvPr id>`，与 python-pptx 的 shape_id 同源）。
3. 回写：命中 run 整段替换 or 保留 run 前缀后缀——**以 Python `_execute_ppt` 的实际行为为准**（动手前精读），通常是对 run.text 赋值，即替换整个 `<a:t>` 文本。
4. python-pptx 的 `prs.save` 会重写整个包；Rust 侧外科手术式只动 slide/notes XML——输出与 Python 版**不要求字节一致**，验收标准是：两边脱敏输出用各自库重新读取，**可见文本逐项一致** + 水印可解出。

- [ ] **Step 5.1: 移植 `ppt_scanner` 扫描部分，差分 `generate` 于 `examples/sample_report.pptx`（sites 语义一致）**
- [ ] **Step 5.2: 回写路径接入 `set_run_text`，端到端：scan → redact → save → 重新 scan 校验值已替换且水印可解**
- [ ] **Step 5.3: Commit** `feat(rust): pptx scan and surgical write-back`

**出口标准**：sample_report.pptx 差分通过；脱敏输出重读校验通过。

---

## Task 6（Phase 6）：执行器 + 策略 YAML IO + CLI

**Files:**
- Create: `rust/src/yaml_io.rs`、`rust/src/executor.rs`；补全 `rust/src/main.rs`
- Test: `rust/tests/integration.rs`
- Port from: `utils/yaml_io.py`(194行)、`engine/executor.py`(539行)、`main.py`(393行)、`utils/file_utils.py`(107行)

**Interfaces:**
- Consumes: 全部前序 Task
- Produces:

```rust
// yaml_io.rs
/// 写：手工格式化生成（带注释模板，字段层级与 Python 输出一致）
pub fn export_strategy_yaml(s: &Strategy, path: &Path) -> Result<(), YamlError>;
/// 读：serde_yaml 解析（人审修改后回读）
pub fn load_strategy_yaml(path: &Path) -> Result<Strategy, YamlError>;

// executor.rs
pub struct Executor { strategy: Strategy, operator: String }
pub struct ExecutionReport { pub success: bool, pub processed: usize, pub skipped: usize, pub errors: usize, pub dry_run: bool }
impl Executor {
    pub fn execute(&self, input: &Path, output: &Path, dry_run: bool) -> Result<ExecutionReport, ExecError>;
    pub fn execute_batch(&self, input_dir: &Path, output_dir: &Path, dry_run: bool) -> Vec<(Path, Result<ExecutionReport, ExecError>)>;
}

// main.rs —— clap 派生，命令面与 Python 完全一致
// finance-mask generate -i X -o 策略.yaml [-v]
// finance-mask redact -i X -o OUTDIR [-s 策略.yaml | --default-policy] [--operator ID] [-f] [--dry-run] [-v]
```

**移植要点：**
1. **YAML 写**：serde_yaml 不产注释。`export_strategy_yaml` 手工拼字符串（每个 site 块后加 eol 注释 `# [Excel] Sheet1!B5` 之类，格式模仿 `yaml_io.py` 的 `yaml_add_eol_comment` 输出）；**读**用 serde_yaml。读写的字段名即 Task 1 类型定义——这就是人审格式的契约。
2. **执行流**（`executor.py`）：列规则（Data 表）→ 位点（Form 表）→ dry_run 不落盘 → 水印 → 审计 → 批量模式逐文件隔离失败。
3. **默认策略**（`--default-policy`，`main.py:283 _generate_default_strategy`）：内置列规则表照抄。
4. CLI 输出：rich 表 → comfy-table；`-v` 日志 → `eprintln!`（或 `env_logger`，**不引重框架**）。
5. `file_utils.py` 的目录递归/输出命名规则（`X_脱敏.xlsx`）照抄。

- [ ] **Step 6.1: yaml_io 测试**：Python 生成的 `examples/策略.yaml` 用 Rust 读回 → Strategy 语义一致；Rust 导出 → 再读回 → 一致（人审闭环）。
- [ ] **Step 6.2: executor + CLI 移植**
- [ ] **Step 6.3: 端到端对等验收**：

```bash
# 双端全流程差分：生成→执行→比对
python scripts/diff_test.py examples/仓库入库1.xlsx
cargo run --manifest-path rust/Cargo.toml -- redact -i examples/仓库入库1.xlsx \
    -s 策略.yaml -o /tmp/rs_out --operator tester
uv run finance-mask redact -i examples/仓库入库1.xlsx \
    -s 策略.yaml -o /tmp/py_out --operator tester
# 比对：脱敏输出用 calamine/openpyxl 各自重读，单元格值逐项一致；
#       审计日志语义一致；水印 payload 同 operator 可解。
```

- [ ] **Step 6.4: 全量回归 + 收尾 commit**

```bash
cargo test --manifest-path rust/Cargo.toml   # 全绿
git add rust/ scripts/
git commit -m "feat(rust): executor, strategy yaml io, cli — full parity with python"
```

**出口标准**：examples/ 全部夹具端到端对等（generate 语义一致、redact 输出重读一致、审计与水印对齐）；`cargo test` 全绿。

---

## Task 7（Phase 7，另行计划）：UI 壳

**状态：已启动（2026-09-10），Tauri 2 骨架已落地 `ui/`。** 启动条件：Phase 0-6 完成。接口事实：UI 是 `finance_mask_core` lib 的薄客户端（Tauri 后端直接 path-depend core crate，编排逻辑已下沉至 `lib::pipeline`），策略 YAML 是唯一人审产物，UI 的规则配置/策略修订界面是它的图形化编辑器。

**已决策：** UI 形态 = **a) Tauri 桌面应用**（Rust 后端 + web 前端，单 exe 分发，与 core 同 workspace）。
**待确认（启动 UI 计划前）：** 使用环境是否隔离内网/涉密（影响 CDN、自动更新、webview 离线打包）。

UI 计划在上述问题回答后另立文档（`docs/UI实施计划.md`）。

---

## 风险登记册

| 风险 | 影响 | 缓解 |
|---|---|---|
| header_finder 移植走样（562 行，边角案例多） | 差分失败 | test_multi_header/test_excel_scanner_v2 全量移植先行；逐 fixture 调试 |
| sharedStrings 追加逻辑出错 | 回写文件损坏 | 保真往返测试（Task 4.3）+ calamine 读回验证 |
| Python 版隐式行为未被发现（如 site_id 格式、扫描顺序） | 对等缺口 | 差分测试以 Python 真实输出为准绳；动手前精读对应 Python 段 |
| fancy-regex 与 Python re 语义差异 | 误报/漏报 | patterns 仅 9 条，逐条用 Python 实测样例双向验证 |
| 差分/比例模式的跨位点状态 | 金额关系破坏 | AmountGroupState 显式建模，测试断言差值/比例保持 |
| ruamel 注释格式不可复刻 | 人审体验降级 | 仅语义兼容是硬约束；注释格式尽力模仿，允许差异 |

## 完成定义（Phase 1-6 整体）

1. `cargo test` 全绿（含移植的全部测试）
2. examples/ 与 layout-view/files/ 全部夹具差分通过
3. `finance-mask redact` 端到端输出与 Python 版重读一致
4. 本计划 UI 之外的 Python 依赖（Python 运行时、dll 分发）可从发布物中移除——Rust 单二进制 + 内嵌配置

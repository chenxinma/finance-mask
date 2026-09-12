# 前端 UI 设计文档 —— 竖屏向导式重构

> 分支：`feat/ui-wizard-redesign`
> 视觉规范来源：`docs/design.md`（Origami Geométrico）
> 目标用户：**无 IT 背景的一般财务人员**
> 涉及文件：`ui/frontend/index.html`、`ui/frontend/style.css`、`ui/frontend/app.js`、`ui/tauri.conf.json`
> 后端约束：**不改 Rust 代码**，仅复用现有 invoke 命令（`pick_file` / `pick_dir` / `save_file` / `generate` / `load_strategy` / `save_strategy` / `preview_yaml` / `redact`）

---

## 1. 需求（本次同步修改的 5 项交互）

| # | 需求 | 落地方式 |
|---|------|----------|
| 1 | 9:16 竖屏布局 | 窗口 540×960（min 432×768），CSS 全部按窄屏纵向排版 |
| 2 | 3 步向导式交互 | 顶部步骤条 + 单一活动步骤 + 底部固定「上一步/下一步」导航 |
| 3 | 修订策略时全文搜索定位点位 | 步骤② 顶部粘性搜索框，对位点/列规则全文匹配过滤 + `<mark>` 高亮 + 计数 |
| 4 | 策略按类型分组、中文显示、参数用表单 | 动作下拉按 `detected_type` 过滤并显示中文名；每种动作渲染专属参数表单（见 §6） |
| 5 | 单文件处理，去掉批量/文件夹 | 全流程只持有一个输入文件；移除「选文件夹」「批量报告表格」 |

## 2. 视觉规范（映射自 docs/design.md）

### 颜色

| 用途 | 色值 |
|------|------|
| 页面/卡片背景 Paper White | `#FAFAFA` |
| 主文字 Ink Black（禁用纯黑） | `#1A1A1A` |
| 主强调 / CTA Accent Coral | `#FF6B6B`（hover 加深 8%：`#f95252`） |
| 次级文字 / 边框 Fold Shadow | `#B0B0B0` |
| 次级文字 Steel Grey | `#4A4A4A` |
| 成功 / 完成态 Sage Paper | `#A8D5BA` |
| 信息 / 装饰 Sky Fold | `#87CEEB` |
| 警告 / 装饰 Warm Crease | `#F0C987` |
| 失败态（语义红） | `#D64545` |

### 字体（离线环境，禁 CDN/网络字体，仅声明 + 系统回退）

```css
--font-ui: Poppins, "Segoe UI", "Microsoft YaHei", system-ui, sans-serif;
--font-mono: "JetBrains Mono", Consolas, monospace;  /* 位点值、site_id */
```

层级：H1 1.5rem/700 · H2 1.125rem/700 · 正文 1rem/1.6 · 标签/说明 0.875rem/500。

### 形状与深度

- **圆角 0px**：按钮、卡片、输入框全部直角（折纸风格）。
- 卡片：`#FAFAFA` 面 + `1px solid #B0B0B0` 描边 + 纸折阴影 `box-shadow: 2px 2px 0 rgba(26,26,26,.08)`。
- 装饰：头部/步骤条用 `clip-path: polygon(...)` 切角或折角三角形；背景可加一层内联 SVG data-URI 的浅色 tessellation 纹理（禁外链图片）。
- 悬停：颜色微移 + 阴影变化，200ms；按钮 active 时 `translateY(1px)`。

### 动效（只动 transform / opacity）

- 步骤切换：淡入 200ms。
- 卡片列表入场：fade + translateY(16px→0)，420ms ease-out，逐项 stagger 80ms。
- 加载态：shimmer 骨架条（禁转圈 spinner）。

### Do / Don't（继承 design.md）

- 禁 emoji（现有 `✓ ⊘ ✗` 改为「成功/跳过/失败」文字徽章 + 颜色）。
- 禁纯黑 `#000`；用 `100dvh` 不用 `100vh`。
- z-index 契约：sticky 搜索框/步骤条 100 · 遮罩 200 · 弹层（YAML 预览）300 · toast 500。
- 文案面向财务人员：说「脱敏」「位点」可以，但按钮/提示一律大白话（如「仅预览，不修改文件」），禁 AI 套话。

## 3. 窗口与整体布局（9:16）

`ui/tauri.conf.json`：

```json
"windows": [{ "title": "财务文件智能脱敏", "width": 540, "height": 960,
              "minWidth": 432, "minHeight": 768, "resizable": true }]
```

页面骨架（flex column，`min-height: 100dvh`）：

```
┌────────────────────────────┐
│ header：产品名 + 折角装饰     │  固定
│ stepper：①─②─③ 步骤条       │  sticky, z=100
├────────────────────────────┤
│ main：当前步骤内容（纵向滚动） │  flex:1, overflow-y:auto
├────────────────────────────┤
│ footer：[上一步] 状态文本 [下一步/主操作] │  固定底栏
└────────────────────────────┘
```

- 步骤条：3 个节点（数字 + 标签「选择文件 / 审核策略 / 执行脱敏」），连线；当前步 coral 填充，已完成步 sage 底 + ink 数字，未到步灰描边。点击已完成步骤可回跳；未到达步骤不可点。
- 底栏主按钮随步骤变化：①「扫描并生成策略」②「保存并下一步」③「执行脱敏」。步骤①无「上一步」。
- 状态文本替代原 footer status，错误用 coral 红字，成功用 sage 徽章。

## 4. 步骤① 选择文件

单列纵向表单（label 在上，输入在下，符合 design.md Inputs 规范）：

1. **待脱敏文件**（必填）：只读 input + 「浏览…」按钮 → `pick_file(kind:'input')`，仅 .xlsx/.pptx，**单文件**。
2. **策略保存位置**（必填）：只读 input + 「浏览…」→ `save_file`，选文件后自动带默认名 `原文件名_策略.yaml`。
3. 主操作「扫描并生成策略」→ `generate`；按钮进入 shimmer/禁用态。
4. 完成后显示摘要卡（折纸风卡片）：`发现 N 个敏感位点、M 条列规则`，跳过文件列表（如有），策略路径。1s 后或点「下一步」进入步骤②并自动 `load_strategy`。
5. 失败：摘要卡变失败态（红描边），显示错误信息，停留本步。

## 5. 步骤② 审核策略

### 5.1 搜索框（需求 3）

- main 顶部 **sticky**（z=100），全宽 input，placeholder「搜索位点 / 原始值 / 位置…」。
- 纯前端过滤：对 `site_id`、`original_value`、位置文本（sheet/cell/slide/shape）以及列规则 `pattern` 做不区分大小写的子串匹配；命中项内 `<mark>` 高亮（Warm Crease 底）。
- 右侧实时计数「N / 总数」；无结果显示空态（图标 + 「未找到匹配位点」+「清空搜索」按钮）。
- 清空即恢复全量。过滤只影响显示，**不影响 collect()/保存的数据**。

### 5.2 列表形态：卡片，不用宽表格

竖屏放不下 7 列表格，位点与列规则一律渲染为纵向堆叠卡片（入场 stagger 动画）：

**位点卡片**：
```
┌──────────────────────────────┐
│ [启用开关]  sheet_利润表_A1     │  site_id 用 mono
│ 位置：[Excel] 利润表!A1        │
│ 原始值：22.12亿元   类型：金额  │  原始值 mono、超长省略+title；类型为中文徽章
│ 处理方式：[下拉：中文名，按类型分组] │
│ 参数表单（随动作切换，见 §6）    │
└──────────────────────────────┘
```

**列规则卡片**：`匹配方式(中文) + pattern(mono) + 类型徽章 + 处理方式下拉 + 参数表单`。

区块标题：「列规则 (M)」「敏感位点 (N)」，计数随过滤联动。

### 5.3 元信息与次级操作

- 顶部 meta 一行：来源文件 / 生成时间。
- 「YAML 预览」→ `preview_yaml`，以全屏弹层（z=300，直角卡片，mono 滚动区）展示，带「关闭」；预览前先 collect()，JSON 非法则状态栏报错。
- 「打开已有策略…」→ `pick_file(kind:'yaml')` + `load_strategy`，允许中途换策略文件。
- 底栏主操作「保存并下一步」→ collect() 校验 → `save_strategy` → 进入步骤③。校验失败定位到出错卡片并红描边。

## 6. 处理方式（动作）选择与参数表单（需求 4）

### 6.1 下拉：按 detected_type 分组 + 中文

`<select>` 内用 `<optgroup>`，**推荐组在前**；若卡片当前 action 不属于该类型（旧策略文件），追加进列表保证不丢值：

| detected_type | 中文徽章 | 可选动作（中文名 → 值） |
|---|---|---|
| amount 金额 | 金额 | 降低精度→`precision` · 随机扰动→`perturb` · 金额遮掩→`mask` · 差分偏移→`differential_shift` · 比例缩放→`proportional_scale` |
| entity 机构名 | 机构名 | 代号替换→`alias` |
| person 人名 | 人名 | 姓名遮掩→`mask_name` |
| account 账号 | 账号 | 账号遮掩→`mask_account` |

下拉项文案带一句人话说明，如「随机扰动（±X% 内浮动）」。

### 6.2 参数表单（与 rust/src/executor.rs 实参一一对应）

动作切换时**就地重渲染**参数区；数字用 `<input type="number">`，无需任何校验库：

| 动作 | 字段（label → params key） | 控件与默认 |
|---|---|---|
| precision | 换算单位 → `unit` | select：千`thousand`/百万`million`/亿`billion`，默认 million；小数位数 → `decimal_places`，number，默认 2 |
| perturb | 扰动幅度(%) → `percentage` | number，默认 5 |
| mask | — | 显示提示「无需参数：保留首位，其余以 * 遮掩」 |
| differential_shift | 偏移量 → `shift` | number，**必填**（引擎要求预计算值），placeholder 如 `1000` |
| proportional_scale | 缩放系数 → `scale` | number，step 0.01，**必填**，placeholder 如 `1.05` |
| alias | 代号前缀 → `prefix` | text，默认 `公司`（效果：`[公司A]`） |
| mask_name | 保留前几个字 → `keep_first` | number，默认 1（效果：`张三→张*`） |
| mask_account | 保留前几位 → `keep_prefix`（默认 3）；保留后几位 → `keep_suffix`（默认 4） | number ×2 |

- 每个动作附一行效果示例（灰色小字），如「示例：`22.12亿元 → 22亿元`」。
- collect() 规则改为**从表单字段组装 params 对象**（替代原手填 JSON input）；必填项为空时抛错并聚焦该卡片。无参数动作 `params` 置 `null`。

## 7. 步骤③ 执行脱敏

1. 只读摘要卡：输入文件（来自步骤①）、策略文件（来自步骤②）、可点「返回修改」。
2. 表单：
   - **输出目录**（必填）：`pick_dir`；默认建议 = 输入文件同目录（自动填入，可改）。
   - **操作人**：text，placeholder「默认取系统用户名（写入水印）」。
   - 复选：**仅预览，不修改文件**（dry-run，默认勾选，降低误操作焦虑）· **输出已存在时覆盖**（force）。
3. 「执行脱敏」→ `redact`（`input` 传单个文件路径，`strategyPath` 传步骤②保存路径，`defaultPolicy:false`）。
4. 结果卡（单文件，非表格）：状态徽章（成功 sage / 跳过 灰 / 失败 红）+ 处理 N 处 / 跳过 N / 错误 N + 输出文件路径（可复制）+ 提示「审计日志 *_日志.json 位于输出目录，水印已嵌入」。失败显示 message。
5. 底栏「重新开始」→ 清空全部状态回步骤①。

> 原「使用默认策略（跳过审核）」入口在向导流中移除（`redact` 调用固定 `defaultPolicy:false`），CLI 仍保留该能力。

## 8. 开发任务拆分（subagent 执行）

| 任务 | 内容 | 产物 |
|---|---|---|
| T1 | 窗口尺寸 + 页面骨架：tauri.conf.json 540×960；index.html 重写为 header/stepper/3×step/footer 结构 | index.html, tauri.conf.json |
| T2 | 视觉系统：design.md 色板变量、0 直角、卡片/表单/徽章/按钮/搜索框/弹层样式、动画与骨架屏 | style.css |
| T3 | 步骤①逻辑：文件选择、默认策略名、generate、摘要卡、自动进② | app.js |
| T4 | 步骤②逻辑：卡片渲染、搜索过滤+高亮、分组动作下拉、参数表单联动、collect/保存/YAML 弹层 | app.js |
| T5 | 步骤③逻辑：单文件执行、结果卡、重新开始；移除文件夹/批量代码 | app.js |
| T6 | 验证：`node --check ui/frontend/app.js`；`cd ui && cargo check`；过 §9 清单 | — |

T1–T5 同一 worker 顺序完成（单写者，避免并行冲突），完成后 reviewer 对照本文档审查。

## 9. 验收清单

- [ ] 窗口默认 540×960，min 432×768，缩窄无横向滚动条
- [ ] 三步骤条可导航；未到达步骤不可点；已完成可回跳且状态保留
- [ ] 全流程只能选择一个 .xlsx/.pptx；界面无「文件夹」入口
- [ ] 步骤② 搜索：命中过滤 + 高亮 + 计数；清空恢复；过滤不影响保存数据
- [ ] 动作下拉按类型分组、全中文；切换动作参数表单即时变化
- [ ] `differential_shift`/`proportional_scale` 必填校验生效
- [ ] 保存后重新打开 YAML，action/params 与界面一致（含中文值不乱码）
- [ ] 步骤③ dry-run 默认勾选；结果卡显示状态/计数/输出路径
- [ ] 无 emoji、无 CDN/外链、无 `100vh`、无纯黑；直角 + 折纸装饰可见
- [ ] `node --check` 与 `cargo check` 通过

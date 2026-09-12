// ui/frontend/app.js —— 竖屏三步向导，纯本地前端，无框架无 CDN。
// invoke 走 window.__TAURI__（tauri.conf.json withGlobalTauri=true）。
// 规格：docs/frontend-design.md
const { invoke } = window.__TAURI__.core;
const $ = (s) => document.querySelector(s);

// ---- 状态（全流程只持有一个输入文件） ----
function newState() {
  return {
    step: 1, maxStep: 1,
    inputPath: null,      // 步骤① 选中的唯一输入文件
    strategyPath: null,   // 策略 YAML 路径
    strategy: null,       // 编辑中的 Strategy（与后端 serde JSON 同构）
    generated: false,     // 步骤① 扫描成功
  };
}
let S = newState();

// ---- 动作/类型元数据（key 与 rust/src/executor.rs 一一对应） ----
const TYPE_LABELS = { amount: '金额', entity: '机构名', person: '人名', account: '账号' };
const TYPE_ACTIONS = {
  amount: ['precision', 'perturb', 'mask', 'differential_shift', 'proportional_scale'],
  entity: ['alias'],
  person: ['mask_name'],
  account: ['mask_account'],
};
const ACTION_LABEL = {
  precision: '降低精度（按单位取整）',
  perturb: '随机扰动（±X% 内浮动）',
  mask: '金额遮掩（保留首位）',
  differential_shift: '差分偏移（统一加减偏移量）',
  proportional_scale: '比例缩放（统一乘系数）',
  alias: '代号替换（如 [公司A]）',
  mask_name: '姓名遮掩（如 张三→张*）',
  mask_account: '账号遮掩（保留前后几位）',
};
const ACTION_EXAMPLE = {
  precision: '示例：22.12亿元 → 22亿元',
  perturb: '示例：12,345,678 → 在 ±5% 内浮动',
  mask: '示例：12,345,678.90 → 1*,***,***.**，无需参数',
  differential_shift: '示例：所有金额统一 +1000',
  proportional_scale: '示例：所有金额统一 ×1.05',
  alias: '示例：天齐锂业股份有限公司 → [公司A]',
  mask_name: '示例：张三 → 张*',
  mask_account: '示例：6222021234567890 → 622***********7890',
};
// 每种动作的参数表单；def 与 executor.rs 默认值一致
const PARAM_FORMS = {
  precision: [
    { k: 'unit', label: '换算单位', type: 'select', def: 'million', options: [['thousand', '千'], ['million', '百万'], ['billion', '亿']] },
    { k: 'decimal_places', label: '小数位数', type: 'number', def: 2 },
  ],
  perturb: [{ k: 'percentage', label: '扰动幅度 (%)', type: 'number', def: 5 }],
  mask: [],
  differential_shift: [{ k: 'shift', label: '偏移量', type: 'number', required: true, ph: '如 1000' }],
  proportional_scale: [{ k: 'scale', label: '缩放系数', type: 'number', step: '0.01', required: true, ph: '如 1.05' }],
  alias: [{ k: 'prefix', label: '代号前缀', type: 'text', def: '公司' }],
  mask_name: [{ k: 'keep_first', label: '保留前几个字', type: 'number', def: 1 }],
  mask_account: [
    { k: 'keep_prefix', label: '保留前几位', type: 'number', def: 3 },
    { k: 'keep_suffix', label: '保留后几位', type: 'number', def: 4 },
  ],
};

// ---- 工具 ----
function esc(s) {
  return String(s ?? '').replace(/[&<>"']/g,
    (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
}
function markHtml(text, q) {
  const t = String(text ?? '');
  if (!q) return esc(t);
  const lower = t.toLowerCase();
  let out = '', i = 0;
  for (;;) {
    const idx = lower.indexOf(q, i);
    if (idx < 0) { out += esc(t.slice(i)); break; }
    out += esc(t.slice(i, idx)) + '<mark>' + esc(t.slice(idx, idx + q.length)) + '</mark>';
    i = idx + q.length;
  }
  return out;
}
function setStatus(msg, kind) {
  const el = $('#status');
  el.textContent = msg;
  el.className = kind ?? '';
}
function dirname(p) { return String(p).replace(/[\\/][^\\/]*$/, ''); }
function locText(l) {
  return l.type === 'ppt'
    ? `[PPT] 幻灯片${l.slide ?? 0} 形状${l.shape_id ?? ''}${l.table_location ? ' ' + l.table_location : ''}`
    : `[Excel] ${l.sheet ?? ''}!${l.cell ?? ''}`;
}

// ---- 向导导航 ----
function gotoStep(n) {
  S.step = n;
  S.maxStep = Math.max(S.maxStep, n);
  document.querySelectorAll('.panel').forEach((p) => p.classList.toggle('active', p.id === 'panel' + n));
  document.querySelectorAll('.stepper .step').forEach((b) => {
    const i = +b.dataset.step;
    b.classList.toggle('active', i === n);
    b.classList.toggle('done', i < n || (i <= S.maxStep && i !== n));
  });
  $('#nav-back').classList.toggle('hidden', n === 1);
  $('#nav-restart').classList.toggle('hidden', n !== 3);
  $('#nav-next').textContent =
    n === 1 ? (S.generated ? '审核策略 →' : '扫描并生成策略')
    : n === 2 ? '保存并下一步'
    : '执行脱敏';
  $('#main').scrollTop = 0;
}
$('#stepper').onclick = (e) => {
  const b = e.target.closest('.step');
  if (b && +b.dataset.step <= S.maxStep) gotoStep(+b.dataset.step);
};
$('#nav-back').onclick = () => gotoStep(S.step - 1);
$('#nav-restart').onclick = () => {
  S = newState();
  ['#gen-input', '#gen-output', '#search', '#run-out', '#run-operator'].forEach((s) => { $(s).value = ''; });
  $('#run-dry').checked = true;
  $('#run-force').checked = false;
  $('#gen-summary').classList.add('hidden');
  $('#run-report').classList.add('hidden');
  gotoStep(1);
  setStatus('就绪');
};
$('#nav-next').onclick = () => {
  if (S.step === 1) { if (S.generated) advanceToStep2(); else doGenerate(); }
  else if (S.step === 2) doSaveNext();
  else doRedact();
};

// ---- 实体词典编辑（config/entity_dict.txt，每行一个实体名） ----
function dictCount() {
  const n = $('#dict-text').value.split('\n').map((l) => l.trim()).filter(Boolean).length;
  $('#dict-count').textContent = n ? `${n} 个实体名` : '';
}
$('#dict-edit').onclick = async () => {
  try {
    $('#dict-text').value = await invoke('dict_load');
    dictCount();
    $('#dict-modal').classList.remove('hidden');
    setStatus('词典修改后，下次扫描生效', '');
  } catch (e) {
    setStatus('词典读取失败: ' + e, 'err');
  }
};
$('#dict-text').oninput = dictCount;
$('#dict-save').onclick = async () => {
  try {
    const n = await invoke('dict_save', { content: $('#dict-text').value });
    setStatus(`词典已保存：${n} 个实体名`, 'ok');
    $('#dict-modal').classList.add('hidden');
  } catch (e) {
    setStatus('词典保存失败: ' + e, 'err');
  }
};
$('#dict-close').onclick = () => $('#dict-modal').classList.add('hidden');
$('#dict-modal').onclick = (e) => { if (e.target === $('#dict-modal')) $('#dict-modal').classList.add('hidden'); };

// ---- ① 选择文件（单文件） ----
$('#gen-pick').onclick = async () => {
  const p = await invoke('pick_file', { kind: 'input' });
  if (p) {
    $('#gen-input').value = p;
    S.inputPath = p;
    S.generated = false;
    $('#gen-summary').classList.add('hidden');
    if (!$('#gen-output').value) {
      $('#gen-output').value = p.replace(/\.(xlsx|pptx)$/i, '') + '_策略.yaml';
    }
  }
};
$('#gen-save-pick').onclick = async () => {
  const base = ($('#gen-input').value.split(/[\\/]/).pop() || '策略').replace(/\.(xlsx|pptx)$/i, '');
  const p = await invoke('save_file', { defaultName: base + '_策略.yaml' });
  if (p) $('#gen-output').value = p;
};

async function doGenerate() {
  const input = $('#gen-input').value;
  const output = $('#gen-output').value;
  if (!input || !output) return setStatus('请先选择待脱敏文件与策略保存位置', 'err');
  const btn = $('#nav-next');
  btn.disabled = true; btn.classList.add('loading');
  setStatus('扫描中…');
  const sum = $('#gen-summary');
  try {
    const r = await invoke('generate', { input, output });
    S.strategyPath = r.output;
    S.generated = true;
    sum.className = 'card summary-card ok';
    sum.innerHTML =
      `<strong>扫描完成</strong>` +
      `<div class="result-stats">发现 ${r.total_sites} 个敏感位点、${r.total_column_rules} 条列规则</div>` +
      (r.skipped_files.length ? `<div class="hint">跳过不支持的文件：${esc(r.skipped_files.join(', '))}</div>` : '') +
      `<div class="result-out">策略已导出：${esc(r.output)}</div>`;
    sum.classList.remove('hidden');
    setStatus('生成完成，即将进入人工审核…', 'ok');
    $('#nav-next').textContent = '审核策略 →';
    setTimeout(() => { if (S.generated && S.step === 1) advanceToStep2(); }, 900);
  } catch (e) {
    sum.className = 'card summary-card fail';
    sum.innerHTML = `<strong>生成失败</strong><div class="result-out">${esc(e)}</div>`;
    sum.classList.remove('hidden');
    setStatus('生成失败', 'err');
  } finally {
    btn.disabled = false; btn.classList.remove('loading');
  }
}

let advancing = false;
async function advanceToStep2() {
  if (advancing || !S.strategyPath) return;
  advancing = true;
  try { await loadStrategy(S.strategyPath); gotoStep(2); }
  finally { advancing = false; }
}

// ---- ② 审核策略 ----
$('#edit-pick').onclick = async () => {
  const p = await invoke('pick_file', { kind: 'yaml' });
  if (p) await loadStrategy(p);
};

async function loadStrategy(path) {
  try {
    S.strategy = await invoke('load_strategy', { path });
    S.strategyPath = path;
    S.generated = true;
    $('#search').value = '';
    renderStrategy();
    setStatus(`已加载策略：${S.strategy.sites.length} 个位点`, 'ok');
  } catch (e) {
    setStatus('加载失败: ' + e, 'err');
    throw e;
  }
}

function actionSelectHtml(item) {
  const list = TYPE_ACTIONS[item.detected_type] ?? [];
  const extra = list.includes(item.action) ? [] : [item.action];
  const opt = (v) => `<option value="${v}"${v === item.action ? ' selected' : ''}>${ACTION_LABEL[v] ?? v}</option>`;
  return `<select data-f="action">` +
    (extra.length ? `<optgroup label="当前值">${extra.map(opt).join('')}</optgroup>` : '') +
    `<optgroup label="${TYPE_LABELS[item.detected_type] ?? item.detected_type}类适用">${list.map(opt).join('')}</optgroup>` +
    `</select>`;
}

function paramsHtml(item) {
  const fields = PARAM_FORMS[item.action] ?? [];
  const body = fields.map((f) => {
    const cur = (item.params && item.params[f.k] != null) ? item.params[f.k] : (f.def ?? '');
    const input = f.type === 'select'
      ? `<select data-p="${f.k}">${f.options.map(([v, l]) => `<option value="${v}"${String(cur) === v ? ' selected' : ''}>${l}</option>`).join('')}</select>`
      : `<input type="${f.type}" data-p="${f.k}" value="${esc(cur)}"${f.step ? ` step="${f.step}"` : ''}${f.ph ? ` placeholder="${f.ph}"` : ''}>`;
    return `<div class="p-field"><label>${f.label}${f.required ? ' *' : ''}</label>${input}</div>`;
  }).join('');
  return body + `<p class="example">${ACTION_EXAMPLE[item.action] ?? ''}</p>`;
}

function cardHtml(item, attr, i) {
  const isSite = attr === 'data-site';
  const type = item.detected_type;
  const searchable = isSite
    ? [item.site_id, item.original_value, locText(item.location), item.location.sheet, item.location.cell, item.location.slide, item.location.shape_id, item.location.table_location]
    : [item.pattern, item.match_type];
  return `<div class="card" ${attr}="${i}" data-search="${esc(searchable.join(' ').toLowerCase())}">` +
    `<div class="card-top">` +
      (isSite
        ? `<label class="switch"><input type="checkbox" data-f="enabled" ${item.enabled ? 'checked' : ''}>启用</label>`
        : `<span class="switch">匹配方式：${item.match_type === 'regex' ? '正则' : '精确'}</span>`) +
      `<span class="sid mono" data-hl>${esc(isSite ? item.site_id : item.pattern)}</span>` +
    `</div>` +
    (isSite
      ? `<div class="loc" data-hl>${esc(locText(item.location))}</div>
         <div class="val-row"><span class="val mono" data-hl title="${esc(item.original_value)}">${esc(item.original_value)}</span>
         <span class="badge badge-${esc(type)}">${TYPE_LABELS[type] ?? esc(type)}</span></div>`
      : `<div class="val-row"><span class="badge badge-${esc(type)}">${TYPE_LABELS[type] ?? esc(type)}</span></div>`) +
    `<div class="act-row"><label>处理方式</label>${actionSelectHtml(item)}</div>` +
    `<div class="params">${paramsHtml(item)}</div>` +
  `</div>`;
}

function renderStrategy() {
  const st = S.strategy;
  const m = st.metadata;
  $('#edit-meta').textContent = `来源：${m.source_file}｜生成时间：${m.generated_at}`;

  const rules = st.column_rules ?? [];
  $('#rules-count').textContent = `(${rules.length})`;
  $('#rules-list').innerHTML = rules.length
    ? rules.map((r, i) => cardHtml(r, 'data-rule', i)).join('')
    : '<p class="hint">（无列规则）</p>';

  $('#sites-count').textContent = `(${st.sites.length})`;
  $('#sites-list').innerHTML = st.sites.length
    ? st.sites.map((s, i) => cardHtml(s, 'data-site', i)).join('')
    : '<p class="hint">（无敏感位点）</p>';

  document.querySelectorAll('.card').forEach((c, i) => {
    c.style.animationDelay = Math.min(i, 8) * 80 + 'ms';
    c.querySelectorAll('[data-hl]').forEach((el) => { el.dataset.text = el.textContent; });
  });
  applySearch();
}

function itemOf(card) {
  return card.dataset.site != null
    ? S.strategy.sites[+card.dataset.site]
    : (S.strategy.column_rules ?? [])[+card.dataset.rule];
}

// 搜索：纯前端过滤，不影响保存数据（需求 3）
function applySearch() {
  const q = $('#search').value.trim().toLowerCase();
  let shown = 0, total = 0;
  document.querySelectorAll('.card[data-site], .card[data-rule]').forEach((card) => {
    total++;
    const hit = !q || card.dataset.search.includes(q);
    card.classList.toggle('hidden', !hit);
    if (hit) shown++;
    card.querySelectorAll('[data-hl]').forEach((el) => { el.innerHTML = markHtml(el.dataset.text, q); });
  });
  $('#search-count').textContent = q ? `匹配 ${shown} / 共 ${total}` : '';
  $('#empty-search').classList.toggle('hidden', !(q && shown === 0));
}
$('#search').oninput = applySearch;
$('#search-clear').onclick = () => { $('#search').value = ''; applySearch(); };

// 动作切换：就地重渲染参数表单（切换后参数恢复默认值）
$('#main').addEventListener('change', (e) => {
  const sel = e.target.closest('select[data-f="action"]');
  if (sel) {
    const card = sel.closest('.card');
    const item = itemOf(card);
    if (!item) return;
    item.action = sel.value;
    item.params = null;
    card.querySelector('.params').innerHTML = paramsHtml(item);
    card.classList.remove('error');
    return;
  }
  const ck = e.target.closest('input[data-f="enabled"]');
  if (ck) {
    const item = itemOf(ck.closest('.card'));
    if (item) item.enabled = ck.checked;
  }
});

// collect：从表单字段组装 params（替代手填 JSON）；必填缺失/非法时抛错并定位卡片
function collect() {
  document.querySelectorAll('.card[data-site], .card[data-rule]').forEach((card) => {
    const item = itemOf(card);
    if (!item) return;
    const defs = PARAM_FORMS[item.action] ?? [];
    if (!defs.length) { item.params = null; card.classList.remove('error'); return; }
    const obj = {};
    for (const f of defs) {
      const el = card.querySelector(`[data-p="${f.k}"]`);
      const raw = el ? String(el.value).trim() : '';
      if (f.required && raw === '') throw { card, msg: `「${ACTION_LABEL[item.action]}」需要填写${f.label}` };
      if (raw === '') { obj[f.k] = f.def ?? null; continue; }
      if (f.type === 'number') {
        const n = Number(raw);
        if (Number.isNaN(n)) throw { card, msg: `${f.label}必须是数字：${raw}` };
        obj[f.k] = n;
      } else {
        obj[f.k] = raw;
      }
    }
    item.params = obj;
    card.classList.remove('error');
  });
}
function collectOrFail() {
  try { collect(); return true; }
  catch (e) {
    if (e && e.card) {
      e.card.classList.remove('hidden');
      e.card.classList.add('error');
      e.card.scrollIntoView({ block: 'center' });
      setStatus(e.msg, 'err');
    } else {
      setStatus('参数校验失败: ' + (e.message ?? e), 'err');
    }
    return false;
  }
}

async function doSaveNext() {
  if (!S.strategy || !S.strategyPath) return setStatus('请先在步骤①生成策略，或打开已有策略文件', 'err');
  if (!S.inputPath) return setStatus('缺少输入文件，请回到步骤①选择', 'err');
  if (!collectOrFail()) return;
  try {
    await invoke('save_strategy', { path: S.strategyPath, strategy: S.strategy });
    setStatus('策略已保存', 'ok');
    fillStep3();
    gotoStep(3);
  } catch (e) {
    setStatus('保存失败: ' + e, 'err');
  }
}

// YAML 预览弹层
$('#edit-yaml-btn').onclick = async () => {
  if (!S.strategy) return setStatus('请先加载策略', 'err');
  if (!collectOrFail()) return;
  try {
    $('#modal-body').textContent = await invoke('preview_yaml', { strategy: S.strategy });
    $('#modal').classList.remove('hidden');
  } catch (e) {
    setStatus('预览失败: ' + e, 'err');
  }
};
$('#modal-close').onclick = () => $('#modal').classList.add('hidden');
$('#modal').onclick = (e) => { if (e.target === $('#modal')) $('#modal').classList.add('hidden'); };

// ---- ③ 执行脱敏（单文件） ----
function fillStep3() {
  $('#run-input-show').textContent = S.inputPath ?? '（未选择）';
  $('#run-strategy-show').textContent = S.strategyPath ?? '（未选择）';
  if (!$('#run-out').value && S.inputPath) $('#run-out').value = dirname(S.inputPath);
}
$('#run-back-edit').onclick = () => gotoStep(2);
$('#run-pick-out').onclick = async () => {
  const p = await invoke('pick_dir');
  if (p) $('#run-out').value = p;
};

async function doRedact() {
  const outputDir = $('#run-out').value;
  if (!S.inputPath || !S.strategyPath) return setStatus('缺少输入文件或策略，请返回前面步骤', 'err');
  if (!outputDir) return setStatus('请选择输出目录', 'err');
  const dryRun = $('#run-dry').checked;
  const btn = $('#nav-next');
  btn.disabled = true; btn.classList.add('loading');
  setStatus(dryRun ? '预览中…' : '执行中…');
  try {
    const r = await invoke('redact', {
      input: S.inputPath,
      strategyPath: S.strategyPath,
      defaultPolicy: false,
      outputDir,
      operator: $('#run-operator').value.trim() || null,
      force: $('#run-force').checked,
      dryRun,
    });
    renderResult(r, dryRun);
    setStatus(`完成：处理 ${r.total_processed} 处，${r.total_errors} 个错误`, r.total_errors ? 'err' : 'ok');
  } catch (e) {
    $('#run-report').className = 'card summary-card fail';
    $('#run-report').innerHTML = `<strong>执行失败</strong><div class="result-out">${esc(e)}</div>`;
    $('#run-report').classList.remove('hidden');
    setStatus('执行失败', 'err');
  } finally {
    btn.disabled = false; btn.classList.remove('loading');
  }
}

function renderResult(r, dryRun) {
  const box = $('#run-report');
  const f = (r.files ?? [])[0];
  if (!f) {
    box.className = 'card summary-card fail';
    box.innerHTML = '<strong>未处理任何文件</strong>';
    box.classList.remove('hidden');
    return;
  }
  const meta = { ok: ['ok', '成功'], skipped_exists: ['skipped', '跳过（输出已存在）'], failed: ['fail', '失败'] };
  const [cls, label] = meta[f.status] ?? ['', f.status];
  box.className = `card summary-card ${cls === 'fail' ? 'fail' : 'ok'}`;
  box.innerHTML =
    `<span class="result-badge ${cls}">${esc(label)}</span>` +
    `<div class="result-stats">处理 ${f.processed} 处 · 跳过 ${f.skipped} · 错误 ${f.errors}</div>` +
    `<div class="result-out">${esc(f.status === 'failed' ? (f.message ?? '') : f.output)}</div>` +
    (dryRun
      ? '<p class="hint">预览模式：未实际修改文件。确认无误后取消勾选「仅预览」再执行一次。</p>'
      : '<p class="hint">脱敏文件与审计日志（*_日志.json）位于输出目录，水印已嵌入。</p>');
  box.classList.remove('hidden');
}

gotoStep(1);
setStatus('就绪');

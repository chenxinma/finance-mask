// ui/frontend/app.js —— 纯本地前端，无框架无 CDN。
// invoke 走 window.__TAURI__（tauri.conf.json withGlobalTauri=true）。
const { invoke } = window.__TAURI__.core;
const $ = (s) => document.querySelector(s);
const status = (msg) => { $('#status').textContent = msg; };

const ACTIONS = ['precision', 'perturb', 'mask', 'alias', 'mask_name',
  'mask_account', 'differential_shift', 'proportional_scale'];

let strategy = null;  // 当前编辑中的 Strategy（与后端 serde JSON 同构）
let editPath = null;

function esc(s) {
  return String(s ?? '').replace(/[&<>"']/g,
    (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
}
function opts(list, sel) {
  return list.map((v) => `<option value="${v}"${v === sel ? ' selected' : ''}>${v}</option>`).join('');
}

// ---- 页签切换 ----
document.querySelectorAll('.tabs button').forEach((b) => {
  b.onclick = () => {
    document.querySelectorAll('.tabs button').forEach((x) => x.classList.toggle('active', x === b));
    document.querySelectorAll('.tab').forEach((t) => t.classList.toggle('active', t.id === 'tab-' + b.dataset.tab));
  };
});
const switchTab = (name) => document.querySelector(`.tabs button[data-tab="${name}"]`).click();

// ---- ① 生成策略 ----
$('#gen-pick').onclick = async () => {
  const p = await invoke('pick_file', { kind: 'input' });
  if (p) {
    $('#gen-input').value = p;
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

$('#gen-run').onclick = async () => {
  const input = $('#gen-input').value;
  const output = $('#gen-output').value;
  if (!input || !output) return status('请先选择输入文件与策略保存路径');
  status('扫描中…');
  $('#gen-run').disabled = true;
  try {
    const r = await invoke('generate', { input, output });
    const rep = $('#gen-report');
    rep.textContent =
      `扫描 ${r.scanned} 个文件：${r.total_sites} 个位点 / ${r.total_column_rules} 条列规则` +
      (r.skipped_files.length ? `\n跳过不支持的文件: ${r.skipped_files.join(', ')}` : '') +
      `\n策略已导出: ${r.output}`;
    rep.classList.remove('hidden');
    status('生成完成，进入人工审核');
    editPath = r.output;
    await loadStrategy(r.output);
    switchTab('edit');
  } catch (e) {
    const rep = $('#gen-report');
    rep.textContent = '生成失败: ' + e;
    rep.classList.remove('hidden');
    status('生成失败');
  } finally {
    $('#gen-run').disabled = false;
  }
};

// ---- ② 编辑策略 ----
$('#edit-pick').onclick = async () => {
  const p = await invoke('pick_file', { kind: 'yaml' });
  if (p) { editPath = p; await loadStrategy(p); }
};

async function loadStrategy(path) {
  try {
    strategy = await invoke('load_strategy', { path });
    $('#edit-path').value = path;
    renderStrategy();
    status(`已加载 ${strategy.sites.length} 个位点`);
  } catch (e) {
    status('加载失败: ' + e);
  }
}

function locText(l) {
  return l.type === 'ppt'
    ? `[PPT] 幻灯片${l.slide ?? 0} 形状${l.shape_id ?? ''}${l.table_location ? ' ' + l.table_location : ''}`
    : `[Excel] ${l.sheet ?? ''}!${l.cell ?? ''}`;
}

function renderStrategy() {
  const m = strategy.metadata;
  $('#edit-meta').textContent =
    `来源: ${m.source_file}｜生成时间: ${m.generated_at}｜位点总数: ${strategy.sites.length}`;

  const rules = strategy.column_rules ?? [];
  $('#rules-count').textContent = `(${rules.length})`;
  $('#rules-table').innerHTML = rules.length
    ? '<tr><th>匹配</th><th>模式</th><th>动作</th><th>params (JSON)</th><th>类型</th></tr>' +
      rules.map((r, i) => `<tr>
        <td>${esc(r.match_type)}</td>
        <td class="mono">${esc(r.pattern)}</td>
        <td><select data-rule="${i}" data-f="action">${opts(ACTIONS, r.action)}</select></td>
        <td><input class="params" data-rule="${i}" value="${esc(JSON.stringify(r.params ?? {}))}"></td>
        <td>${esc(r.detected_type)}</td></tr>`).join('')
    : '<tr><td class="hint">（无列规则）</td></tr>';

  $('#sites-count').textContent = `(${strategy.sites.length})`;
  $('#sites-table').innerHTML =
    '<tr><th>启用</th><th>site_id</th><th>位置</th><th>原始值</th><th>类型</th><th>动作</th><th>params (JSON)</th></tr>' +
    strategy.sites.map((s, i) => `<tr>
      <td><input type="checkbox" data-site="${i}" data-f="enabled" ${s.enabled ? 'checked' : ''}></td>
      <td class="mono">${esc(s.site_id)}</td>
      <td>${esc(locText(s.location))}</td>
      <td class="val" title="${esc(s.original_value)}">${esc(s.original_value)}</td>
      <td>${esc(s.detected_type)}</td>
      <td><select data-site="${i}" data-f="action">${opts(ACTIONS, s.action)}</select></td>
      <td><input class="params" data-site="${i}" value="${esc(JSON.stringify(s.params ?? {}))}"></td></tr>`).join('');

  $('#edit-preview').classList.add('hidden');
}

// 把表格编辑状态收回到 strategy 对象；params JSON 非法时抛错
function collect() {
  document.querySelectorAll('input[data-f="enabled"]').forEach((el) => {
    strategy.sites[+el.dataset.site].enabled = el.checked;
  });
  document.querySelectorAll('select[data-f="action"]').forEach((el) => {
    const t = el.dataset.site != null
      ? strategy.sites[+el.dataset.site]
      : (strategy.column_rules ?? [])[+el.dataset.rule];
    if (t) t.action = el.value;
  });
  document.querySelectorAll('input.params').forEach((el) => {
    const t = el.dataset.site != null
      ? strategy.sites[+el.dataset.site]
      : (strategy.column_rules ?? [])[+el.dataset.rule];
    if (!t) return;
    const raw = el.value.trim();
    t.params = raw ? JSON.parse(raw) : null;
  });
}

$('#edit-save').onclick = async () => {
  if (!strategy || !editPath) return status('请先打开策略文件');
  try { collect(); } catch (e) { return status('params JSON 非法: ' + e.message); }
  try {
    await invoke('save_strategy', { path: editPath, strategy });
    status(`已保存: ${editPath}`);
  } catch (e) {
    status('保存失败: ' + e);
  }
};

$('#edit-yaml-btn').onclick = async () => {
  if (!strategy) return status('请先打开策略文件');
  try { collect(); } catch (e) { return status('params JSON 非法: ' + e.message); }
  try {
    const yaml = await invoke('preview_yaml', { strategy });
    const pre = $('#edit-preview');
    pre.textContent = yaml;
    pre.classList.remove('hidden');
    status('YAML 预览（保存后的实际内容）');
  } catch (e) {
    status('预览失败: ' + e);
  }
};

// ---- ③ 执行脱敏 ----
$('#run-pick-input').onclick = async () => {
  const p = await invoke('pick_file', { kind: 'input' });
  if (p) $('#run-input').value = p;
};
$('#run-pick-dir').onclick = async () => {
  const p = await invoke('pick_dir');
  if (p) $('#run-input').value = p;
};
$('#run-pick-strategy').onclick = async () => {
  const p = await invoke('pick_file', { kind: 'yaml' });
  if (p) $('#run-strategy').value = p;
};
$('#run-pick-out').onclick = async () => {
  const p = await invoke('pick_dir');
  if (p) $('#run-out').value = p;
};
$('#run-default').onchange = (e) => {
  $('#run-strategy').disabled = e.target.checked;
  $('#run-pick-strategy').disabled = e.target.checked;
};

$('#run-run').onclick = async () => {
  const input = $('#run-input').value;
  const outputDir = $('#run-out').value;
  if (!input || !outputDir) return status('请选择输入与输出目录');
  const defaultPolicy = $('#run-default').checked;
  const strategyPath = $('#run-strategy').value;
  if (!defaultPolicy && !strategyPath) return status('请选择策略文件，或勾选默认策略');

  status(defaultPolicy ? '执行中（默认策略）…' : '执行中…');
  $('#run-run').disabled = true;
  try {
    const r = await invoke('redact', {
      input,
      strategyPath: defaultPolicy ? null : strategyPath,
      defaultPolicy,
      outputDir,
      operator: $('#run-operator').value.trim() || null,
      force: $('#run-force').checked,
      dryRun: $('#run-dry').checked,
    });
    renderReport(r);
    status(`完成：处理 ${r.total_processed} 个位点，${r.total_errors} 个错误`);
  } catch (e) {
    $('#run-report').innerHTML = `<pre class="report">执行失败: ${esc(e)}</pre>`;
    status('执行失败');
  } finally {
    $('#run-run').disabled = false;
  }
};

function renderReport(r) {
  if (!r.files.length) {
    $('#run-report').innerHTML = '<pre class="report">未找到 xlsx/pptx 文件</pre>';
    return;
  }
  const badge = { ok: '✓ 成功', skipped_exists: '⊘ 跳过（已存在）', failed: '✗ 失败' };
  $('#run-report').innerHTML =
    '<div class="table-wrap"><table><tr><th>文件</th><th>状态</th><th>处理</th><th>跳过</th><th>错误</th><th>输出 / 信息</th></tr>' +
    r.files.map((f) => `<tr class="${esc(f.status)}">
      <td class="val" title="${esc(f.input)}">${esc(f.input)}</td>
      <td>${badge[f.status] ?? esc(f.status)}</td>
      <td>${f.processed}</td><td>${f.skipped}</td><td>${f.errors}</td>
      <td class="val" title="${esc(f.status === 'failed' ? (f.message ?? '') : f.output)}">${esc(f.status === 'failed' ? (f.message ?? '') : f.output)}</td></tr>`).join('') +
    '</table></div>' +
    ($('#run-dry').checked
      ? '<p class="hint">预览模式：未实际修改文件。</p>'
      : '<p class="hint">审计日志与脱敏文件位于输出目录（*_日志.json / *_脱敏.*），水印已嵌入。</p>');
}

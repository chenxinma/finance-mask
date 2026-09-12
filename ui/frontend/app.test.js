// app.js 冒烟自检（node ui/frontend/app.test.js）：
// 用最小 DOM/TAURI 桩加载 app.js，校验纯函数逻辑（搜索高亮/路径/动作分组/参数表单）。
const assert = require('assert');
const vm = require('vm');
const fs = require('fs');
const path = require('path');

function el() {
  return {
    classList: { add() {}, remove() {}, toggle() {} },
    style: {}, dataset: {},
    addEventListener() {},
    querySelectorAll: () => [], querySelector: () => null,
    textContent: '', innerHTML: '', value: '',
  };
}
globalThis.window = { __TAURI__: { core: { invoke: async () => { throw new Error('no tauri in test'); } } } };
globalThis.document = { querySelector: () => el(), querySelectorAll: () => [] };
vm.runInThisContext(fs.readFileSync(path.join(__dirname, 'app.js'), 'utf8'), { filename: 'app.js' });

// 搜索高亮：不区分大小写，转义安全
assert.strictEqual(markHtml('利润表!A1', 'a1'), '利润表!<mark>A1</mark>');
assert.strictEqual(markHtml('<b>x</b>', 'b'), '&lt;<mark>b</mark>&gt;x&lt;/<mark>b</mark>&gt;');
assert.strictEqual(markHtml('abc', ''), 'abc');

// 输出目录默认值 = 输入文件同目录
assert.strictEqual(dirname('C:\\reports\\财务.xlsx'), 'C:\\reports');
assert.strictEqual(dirname('/data/f.pptx'), '/data');

// 动作下拉：按类型分组；类型外当前值保留不丢
const sel = actionSelectHtml({ detected_type: 'amount', action: 'perturb' });
assert.ok(sel.includes('金额类适用') && sel.includes('value="perturb" selected'));
assert.ok(!sel.includes('alias'));
const sel2 = actionSelectHtml({ detected_type: 'person', action: 'perturb' });
assert.ok(sel2.includes('当前值') && sel2.includes('value="perturb" selected') && sel2.includes('mask_name'));

// 参数表单：现有值回填、必填标记、无参数动作
assert.ok(paramsHtml({ action: 'perturb', params: { percentage: 10 } }).includes('value="10"'));
assert.ok(paramsHtml({ action: 'perturb', params: null }).includes('value="5"')); // executor 默认
assert.ok(paramsHtml({ action: 'differential_shift', params: null }).includes('偏移量 *'));
assert.ok(!paramsHtml({ action: 'mask', params: null }).includes('data-p='));
assert.ok(paramsHtml({ action: 'precision', params: null }).includes('value="million" selected'));

// 卡片搜索索引含 site_id / 原始值 / 位置（小写）
const card = cardHtml({
  site_id: 'sheet_利润表_B2', location: { type: 'excel', sheet: '利润表', cell: 'B2' },
  original_value: 'ABC 123', detected_type: 'amount', enabled: true, action: 'mask', params: null,
}, 'data-site', 0);
assert.ok(card.includes('data-search="sheet_利润表_b2 abc 123 [excel] 利润表!b2 利润表 b2'));

// 类型-动作映射完整
for (const acts of Object.values(TYPE_ACTIONS)) {
  for (const a of acts) assert.ok(ACTION_LABEL[a] && PARAM_FORMS[a] !== undefined, a);
}

console.log('app.test.js: all checks passed');

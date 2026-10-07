import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { normalizeInventory, analyzeSelection, recommendTargets, buildPlan, renderPlanMarkdown } from '../src/domain.mjs';
import * as domain from '../src/domain.mjs';

const asset = (id, kind, dependsOn = [], extra = {}) => ({ id, kind, name: id.split(':')[1], dependsOn, status: 'supported', issues: [], ...extra });
const inventory = (assets, extra = {}) => ({ schemaVersion: 1, source: { name: '業務.accdb', kind: 'inventory' }, assets, relations: [], limitations: [], ...extra });
const solo = { users: 'solo', concurrentEditing: false, location: 'device', offlineRequired: true, permissions: 'same', coexistence: 'replace-scope' };
const team = { users: 'team', concurrentEditing: true, location: 'remote', offlineRequired: false, permissions: 'roles', coexistence: 'keep-source' };

test('利用形態は日本語で説明し、JSONの識別子と真偽値は保持する', async () => {
  const demo = JSON.parse(await readFile(new URL('../samples/demo.inventory.json', import.meta.url), 'utf8'));
  const plan = buildPlan(demo, { selectedIds: ['form:受注入力'], usage: team, targetId: 'web' });
  const requirement = plan.requirements.find(item => item.title === '利用形態と移行先の制約を確認する');
  assert.ok(requirement.evidence.includes('利用人数: 複数人'));
  assert.ok(requirement.evidence.includes('同時編集: あり'));
  assert.ok(requirement.evidence.includes('利用場所: 社外・遠隔'));
  assert.equal(plan.usage.users, 'team');
  assert.equal(plan.usage.concurrentEditing, true);
  const text = renderPlanMarkdown(plan);
  assert.match(text, /利用人数: 複数人/);
  assert.match(text, /オフライン利用: 不要/);
  assert.match(text, /合成サンプル/);
  assert.ok(!text.includes('利用人数: team'));
});
const shared = () => inventory([
  asset('form:受注', 'form', ['query:受注']), asset('query:受注', 'query', ['table:受注']),
  asset('table:受注', 'table'), asset('table:顧客', 'table'),
  asset('form:顧客', 'form', ['table:顧客']), asset('report:請求', 'report', ['table:受注']),
  asset('page:在庫', 'page', ['table:在庫']), asset('table:在庫', 'table')
], { relations: [{ from: 'table:受注', to: 'table:顧客', fields: [{ from: '顧客ID', to: 'ID' }], enforced: true }] });

test('選択の推移閉包と共有DBの未選択機能への影響を区別する', () => {
  const result = analyzeSelection(normalizeInventory(shared()), ['form:受注']);
  assert.deepEqual(result.selectedIds, ['form:受注']);
  assert.deepEqual(new Set(result.dependencyIds), new Set(['query:受注', 'table:受注', 'table:顧客']));
  assert.deepEqual(new Set(result.impactedIds), new Set(['form:顧客', 'report:請求']));
  assert.deepEqual(new Set(result.sharedIds), new Set(['table:受注', 'table:顧客']));
  assert.ok(result.evidence.some(value => value.includes('form:受注')));
  assert.equal(result.selectedIds.includes('form:顧客'), false);
});

test('選択フォームへの直接参照も未選択機能の影響候補に含める', () => {
  const input = normalizeInventory(inventory([
    asset('form:Child', 'form', []),
    asset('form:Parent', 'form', ['form:Child']),
    asset('query:Mid', 'query', ['form:Child']),
    asset('form:Indirect', 'form', ['query:Mid'])
  ]));
  const result = analyzeSelection(input, ['form:Child']);
  assert.deepEqual(result.selectedIds, ['form:Child']);
  assert.deepEqual(result.dependencyIds, []);
  assert.deepEqual(new Set(result.impactedIds), new Set(['form:Parent', 'form:Indirect']));
  assert.deepEqual(new Set(result.sharedIds), new Set(['form:Child']));
  const plan = buildPlan(input, { selectedIds: ['form:Child'], usage: solo });
  assert.ok(plan.requirements.some(item => item.title === '未選択機能への影響を確認する'));
  assert.deepEqual(plan.analysis.selectedIds, ['form:Child']);
});

test('循環、壊れた参照、動的参照、非対応を黙って成功にしない', () => {
  const input = inventory([
    asset('form:受注', 'form', ['query:A', 'page:旧画面']),
    asset('query:A', 'query', ['query:B', 'table:未取得'], { status: 'partial', issues: [{ code: 'DYNAMIC_REFERENCE', message: '実行時のテーブル名を確認' }] }),
    asset('query:B', 'query', ['query:A']), asset('page:旧画面', 'page', [], { status: 'unsupported' })
  ]);
  const result = analyzeSelection(normalizeInventory(input), ['form:受注']);
  assert.ok(result.unresolved.some(item => item.code === 'MISSING_REFERENCE' && item.referenceId === 'table:未取得'));
  assert.ok(result.unresolved.some(item => item.code === 'CYCLE'));
  assert.ok(result.unresolved.some(item => item.code === 'DYNAMIC_REFERENCE'));
  assert.ok(result.blockers.some(item => item.assetId === 'page:旧画面'));
  assert.deepEqual(result.selectedIds, ['form:受注']);
});

test('版、重複ID、不正な型、参照形、選択の誤りを拒否する', () => {
  for (const input of [null, [], inventory([], { schemaVersion: 2 }), inventory([asset('form:A', 'form'), asset('form:A', 'form')]), inventory([asset('form:A', 'wrong')]), inventory([asset('form:A', 'form', 'table:A')]), inventory([asset('table:A', 'table', [], { rowCount: -1 })]), inventory([asset('table:A', 'table', [], { fields: [{ name: 'ID', required: 'yes' }] })])]) {
    assert.throws(() => normalizeInventory(input));
  }
  const input = normalizeInventory(shared());
  assert.throws(() => analyzeSelection(input, []));
  assert.throws(() => analyzeSelection(input, ['form:存在しない']));
  assert.throws(() => analyzeSelection(input, ['table:受注']));
  assert.throws(() => buildPlan(input, { selectedIds: ['form:受注'], usage: { users: 12 } }));
  assert.throws(() => buildPlan(input, { selectedIds: ['form:受注'], usage: { concurrentEditing: 'true' } }));
  assert.throws(() => buildPlan(input, { selectedIds: ['form:受注'], targetId: 'unknown' }));
});

test('メタデータの許可フィールドだけを返し原本パスと秘密を残さない', () => {
  const input = inventory([asset('form:A', 'form', [], { rawVba: 'TOP_SECRET', connectionString: 'Password=TOP_SECRET', sql: 'TOP_SECRET', issues: [{ code: 'EXTERNAL', message: '接続 Password=TOP_SECRET; Server=secret-host; C:\\private\\data.accdb' }] })], { source: { name: 'C:\\private\\data.accdb', kind: 'access', originalPath: 'C:\\private\\data.accdb', password: 'TOP_SECRET' }, secrets: 'TOP_SECRET' });
  const normalized = normalizeInventory(input);
  const output = JSON.stringify(normalized);
  assert.equal(normalized.source.name, 'data.accdb');
  assert.doesNotMatch(output, /TOP_SECRET|private|secret-host|rawVba|connectionString|originalPath/);
  assert.ok(normalized.assets[0].issues[0].message.includes('接続情報を除去'));
  assert.equal(input.source.name, 'C:\\private\\data.accdb');
});

test('引用符を二重化した接続秘密も値全体を除去する', () => {
  const messages = ['PWD="prefix""SYNTHETIC_SECRET";', "PWD='prefix''SYNTHETIC_SECRET';", 'PWD="prefix"SYNTHETIC_SECRET;', 'Password="a;b"SYNTHETIC_SECRET'];
  for (const message of messages) {
    const normalized = normalizeInventory(inventory([asset('form:A', 'form', [], { issues: [{ code: 'CONNECTION', message }] })]));
    assert.ok(normalized.assets[0].issues[0].message.includes('接続情報を除去'), message);
    assert.doesNotMatch(normalized.assets[0].issues[0].message, /SYNTHETIC_SECRET/, message);
  }
  assert.throws(() => normalizeInventory(inventory([asset('form:PWD="a""b"', 'form')])));
  const plan = buildPlan(normalizeInventory(inventory([asset('form:A', 'form')])), { selectedIds: ['form:A'], usage: solo, notes: '控え: PWD="prefix""SYNTHETIC_SECRET";' });
  assert.doesNotMatch(JSON.stringify(plan) + renderPlanMarkdown(plan), /SYNTHETIC_SECRET/);
});

test('各利用形態の候補と制約を理由付きで提示する', () => {
  const input = normalizeInventory(shared());
  const analysis = analyzeSelection(input, ['form:受注']);
  const byId = usage => Object.fromEntries(recommendTargets(input, usage, analysis).map(item => [item.id, item]));
  assert.equal(byId(solo).excel.fit, 'recommended');
  assert.equal(byId(solo)['sheets-gas'].fit, 'not-recommended');
  assert.equal(byId(team).web.fit, 'recommended');
  assert.equal(byId(team).excel.fit, 'not-recommended');
  assert.equal(byId({ ...team, permissions: 'same' })['sheets-gas'].fit, 'recommended');
  assert.equal(byId({ ...team, offlineRequired: true }).web.fit, 'conditional');
  assert.equal(byId({ ...solo, users: 'team', concurrentEditing: true, location: 'lan', offlineRequired: false }).web.fit, 'recommended');
  assert.ok(byId({ ...team, coexistence: 'shared-store' }).web.requirements.some(text => /共有|同期/.test(text)));
  for (const item of recommendTargets(input, {}, analysis)) {
    assert.equal(item.fit, 'conditional');
    assert.ok(item.unknowns.length > 0);
    assert.ok(item.reasons.length > 0);
  }
});

test('合成サンプルから根拠・未承認要件・移行と検証計画・引き継ぎを生成する', async () => {
  const input = normalizeInventory(JSON.parse(await readFile(new URL('../samples/demo.inventory.json', import.meta.url), 'utf8')));
  assert.equal(input.source.kind, 'synthetic');
  const form = input.assets.find(item => item.kind === 'form');
  const plan = buildPlan(input, { selectedIds: [form.id], usage: team, targetId: 'web', notes: '段階的に確認する' });
  assert.equal(plan.version, 1);
  assert.ok(['draft', 'review-required'].includes(plan.status));
  assert.ok(plan.requirements.length > 0);
  assert.ok(plan.requirements.every(item => item.confirmed === false && item.evidence.length && item.verification));
  assert.ok(plan.migrationSteps.length > 0 && plan.validationSteps.length > 0);
  assert.ok(plan.risks.some(value => /未選択|共有/.test(value)));
  assert.ok(plan.notices.some(value => /合成/.test(value)));
  assert.ok(plan.hand_over);
  const markdown = renderPlanMarkdown(plan);
  assert.match(markdown, /合成/);
  assert.match(markdown, /未承認|未確認/);
  assert.match(markdown, /根拠/);
  assert.match(markdown, /検証/);
  const prohibitedTerm = ['hand', 'off'].join('');
  assert.doesNotMatch(markdown, new RegExp(prohibitedTerm));
});

test('未確認の利用形態と全体の解析制限を計画に保持する', () => {
  const input = normalizeInventory(shared());
  input.limitations.push({ code: 'PROTECTED', message: '保護されたオブジェクトが未取得' });
  const plan = buildPlan(input, { selectedIds: ['form:受注'], usage: {}, notes: '<script>alert(1)</script> Password=TOP_SECRET;' });
  assert.equal(plan.status, 'review-required');
  assert.ok(plan.unresolved.some(item => item.code === 'PROTECTED'));
  assert.ok(plan.unresolved.some(item => item.code === 'USAGE_UNKNOWN'));
  const markdown = renderPlanMarkdown(plan);
  assert.doesNotMatch(markdown, /<script>|TOP_SECRET/);
  assert.ok(plan.requirements.every(item => item.confirmed === false));
});

test('省略と型の異なるnullを区別して誤った取得状態を受け付けない', () => {
  for (const extra of [{ dependsOn: null }, { issues: null }, { status: null }]) {
    assert.throws(() => normalizeInventory(inventory([asset('form:A', 'form', [], extra)])));
  }
  assert.throws(() => normalizeInventory(inventory([], { relations: null })));
  assert.throws(() => normalizeInventory(inventory([], { limitations: null })));
  const input = inventory([{ id: 'form:A', name: 'A', kind: 'form' }]);
  const normalized = normalizeInventory(input);
  assert.equal(normalized.assets[0].status, 'partial');
  assert.ok(analyzeSelection(normalized, ['form:A']).unresolved.some(item => item.code === 'PARTIAL'));
});

test('外部テーブルを読み込んだ件数として表示しない', () => {
  const input = normalizeInventory(inventory([
    asset('form:A', 'form', ['table:外部']),
    asset('table:外部', 'table', [], { isLinked: true, rowCount: 999, fields: [{ name: 'ID', dataType: 'Long' }] })
  ]));
  assert.equal(input.assets[1].rowCount, undefined);
  assert.ok(analyzeSelection(input, ['form:A']).unresolved.some(item => item.code === 'EXTERNAL_REFERENCE'));
  assert.match(renderPlanMarkdown(buildPlan(input, { selectedIds: ['form:A'], usage: solo })), /未計数/);
});

test('未取得資産に紐付く解析制限を消さない', () => {
  const input = normalizeInventory(inventory([asset('form:A', 'form')], { limitations: [{ code: 'PROTECTED', assetId: 'module:未取得', message: '保護された定義は未取得です' }] }));
  assert.ok(analyzeSelection(input, ['form:A']).unresolved.some(item => item.assetId === 'module:未取得'));
});

test('深い依存でも再帰エラーを起こさず循環を報告する', () => {
  const assets = [asset('form:A', 'form', ['query:0'])];
  for (let index = 0; index < 2000; index++) assets.push(asset(`query:${index}`, 'query', [`query:${index === 1999 ? 0 : index + 1}`]));
  const result = analyzeSelection(normalizeInventory(inventory(assets)), ['form:A']);
  assert.equal(result.dependencyIds.length, 2000);
  assert.equal(result.unresolved.filter(item => item.code === 'CYCLE').length, 1);
});

const splitDatabase = () => {
  const frontend = inventory([
    asset('form:受注入力', 'form', ['table:受注リンク']),
    asset('report:請求書', 'report', ['table:受注リンク']),
    asset('table:受注リンク', 'table', [], { isLinked: true, linkedDatabaseName: 'backend.accdb', linkedTableName: '受注' })
  ], { source: { name: 'frontend.mdb', kind: 'synthetic' } });
  const backend = inventory([
    asset('table:受注', 'table', [], { local: true, fields: [{ name: '受注ID', dataType: 'Long', required: true, isPrimaryKey: true }, { name: '顧客ID', dataType: 'Long', required: true, isPrimaryKey: false }] }),
    asset('table:顧客', 'table', [], { local: true, fields: [{ name: '顧客ID', dataType: 'Long', required: true, isPrimaryKey: true }] }),
    asset('form:顧客管理', 'form', ['table:顧客'])
  ], { source: { name: 'backend.accdb', kind: 'synthetic' }, relations: [{ from: 'table:受注', to: 'table:顧客', fields: [{ from: '顧客ID', to: '顧客ID' }], enforced: true }] });
  return [frontend, backend];
};

test('分割DBのaliasを明示リンクだけで対応させPK/FK/影響を維持する', () => {
  const materials = splitDatabase();
  const originals = JSON.stringify(materials);
  const merged = domain.mergeInventories(materials);
  assert.equal(merged.source.kind, 'inventory');
  assert.equal(merged.source.name, 'frontend.mdb + backend.accdb');
  const form = merged.assets.find(item => item.name === '受注入力');
  const alias = merged.assets.find(item => item.name === '受注リンク');
  const orders = merged.assets.find(item => item.name === '受注');
  const customer = merged.assets.find(item => item.name === '顧客');
  const customerForm = merged.assets.find(item => item.name === '顧客管理');
  assert.equal(alias.linkedDatabaseName, 'backend.accdb');
  assert.equal(alias.linkedTableName, '受注');
  assert.equal(alias.fields.find(field => field.name === '受注ID').isPrimaryKey, true);
  assert.ok(alias.dependsOn.includes(orders.id));
  assert.ok(merged.relations.some(item => item.from === orders.id && item.to === customer.id && item.fields[0].from === '顧客ID'));
  const analysis = analyzeSelection(merged, [form.id]);
  assert.ok(analysis.dependencyIds.includes(orders.id));
  assert.ok(analysis.dependencyIds.includes(customer.id));
  assert.ok(analysis.impactedIds.includes(customerForm.id));
  assert.ok(analysis.impactedIds.includes(merged.assets.find(item => item.kind === 'report').id));
  assert.deepEqual(analysis.selectedIds, [form.id]);
  assert.match(customerForm.caption, /backend\.accdb/);
  assert.equal(JSON.stringify(materials), originals);
  const plan = buildPlan(merged, { selectedIds: [form.id], usage: solo });
  assert.ok(plan.notices.some(item => item.includes('確認していません')));
});

test('入力順逆でもリンクが解決し最初の素材がprimaryになる', () => {
  const [frontend, backend] = splitDatabase();
  const merged = domain.mergeInventories([backend, frontend]);
  assert.equal(merged.source.name, 'backend.accdb + frontend.mdb');
  assert.match(merged.assets[0].caption, /backend\.accdb/);
  const form = merged.assets.find(item => item.name === '受注入力');
  const analysis = analyzeSelection(merged, [form.id]);
  assert.ok(analysis.dependencyIds.includes(merged.assets.find(item => item.name === '顧客').id));
});

test('同名別DBのテーブルを混同せず明示したbackendだけへリンクする', () => {
  const [frontend, backend] = splitDatabase();
  const other = inventory([asset('table:受注', 'table', [], { local: true, fields: [{ name: '別DB専用', dataType: 'Text' }] })], { source: { name: 'other.accdb', kind: 'synthetic' } });
  const merged = domain.mergeInventories([frontend, backend, other]);
  const orders = merged.assets.filter(item => item.name === '受注');
  assert.equal(orders.length, 2);
  assert.notEqual(orders[0].id, orders[1].id);
  const alias = merged.assets.find(item => item.name === '受注リンク');
  assert.ok(alias.fields.some(item => item.name === '受注ID'));
  assert.equal(alias.fields.some(item => item.name === '別DB専用'), false);
  assert.ok(merged.limitations.some(item => item.code === 'TABLE_NAME_COLLISION'));
});

test('未提供backend、重複DB名、重複table名、リンク名不足を未解決に残す', () => {
  const [frontend, backend] = splitDatabase();
  const scenarios = [
    [[frontend], 'LINK_BACKEND_MISSING'],
    [[frontend, backend, structuredClone(backend)], 'LINK_AMBIGUOUS'],
    [[frontend, inventory([], { source: backend.source })], 'LINK_TABLE_MISSING'],
    [[frontend, inventory([...backend.assets, asset('table:受注別ID', 'table', [], { name: '受注', local: true })], { source: backend.source })], 'LINK_AMBIGUOUS']
  ];
  const missing = structuredClone(frontend);
  delete missing.assets[2].linkedTableName;
  scenarios.push([[missing, backend], 'LINK_METADATA_MISSING']);
  for (const [materials, code] of scenarios) {
    const merged = domain.mergeInventories(materials);
    const alias = merged.assets.find(item => item.name === '受注リンク');
    assert.equal(alias.dependsOn.length, 0);
    assert.ok(merged.limitations.some(item => item.code === code && item.assetId === alias.id), code);
    const form = merged.assets.find(item => item.name === '受注入力');
    assert.ok(analyzeSelection(merged, [form.id]).unresolved.some(item => item.code === code), code);
  }
});

test('安全なリンクmetadataを保持しパス・型・空値は拒否する', () => {
  const good = normalizeInventory(splitDatabase()[0]);
  assert.equal(good.assets[2].linkedDatabaseName, 'backend.accdb');
  assert.equal(good.assets[2].linkedTableName, '受注');
  for (const [key, value] of [['linkedDatabaseName', 'C:\\private\\backend.accdb'], ['linkedDatabaseName', '/private/backend.accdb'], ['linkedDatabaseName', '../backend.accdb'], ['linkedDatabaseName', ''], ['linkedDatabaseName', 2], ['linkedTableName', ''], ['linkedTableName', false]]) {
    const input = splitDatabase()[0];
    input.assets[2][key] = value;
    assert.throws(() => normalizeInventory(input));
  }
  assert.throws(() => domain.mergeInventories([]));
  assert.throws(() => domain.mergeInventories(null));
});

test('共有DBで共存未定なら固有の未解決事項としてレビューを要求する', () => {
  const input = normalizeInventory(shared());
  const plan = buildPlan(input, { selectedIds: ['form:受注'], usage: { ...solo, coexistence: 'undecided' }, targetId: 'excel' });
  assert.equal(plan.status, 'review-required');
  assert.ok(plan.unresolved.some(item => item.code === 'COEXISTENCE_UNDECIDED'));
  assert.equal(plan.requirements.some(item => item.confirmed === true), false);
});

test('report-root: 帳票を移行範囲の起点として選べ、他の資産は自動選択しない', () => {
  const input = normalizeInventory(inventory([
    asset('report:請求', 'report', ['query:請求', 'macro:印刷'], { status: 'partial', issues: [{ code: 'STATIC_DEFINITION_ONLY', message: '保存定義のみ' }] }),
    asset('query:請求', 'query', ['table:受注']),
    asset('table:受注', 'table'),
    asset('macro:印刷', 'macro', [], { status: 'unsupported' }),
    asset('form:受注', 'form', ['table:受注']),
    asset('page:旧画面', 'page', ['table:受注'], { status: 'unsupported' }),
    asset('report:月次', 'report', ['table:顧客']),
    asset('form:顧客', 'form', ['table:顧客']),
    asset('table:顧客', 'table'),
    asset('module:共通', 'module'),
    asset('external:外部', 'external')
  ]));
  const result = analyzeSelection(input, ['report:請求']);
  assert.deepEqual(result.selectedIds, ['report:請求']);
  assert.deepEqual(new Set(result.dependencyIds), new Set(['query:請求', 'table:受注', 'macro:印刷']));
  assert.deepEqual(new Set(result.impactedIds), new Set(['form:受注', 'page:旧画面']));
  assert.deepEqual(new Set(result.sharedIds), new Set(['table:受注']));
  for (const id of ['form:受注', 'page:旧画面', 'report:月次', 'form:顧客', 'macro:印刷', 'module:共通', 'external:外部']) {
    assert.equal(result.selectedIds.includes(id), false, id);
  }
  assert.equal(result.impactedIds.includes('report:月次'), false);
  assert.equal(result.impactedIds.includes('form:顧客'), false);
  assert.ok(result.blockers.some(item => item.assetId === 'macro:印刷' && item.code === 'UNSUPPORTED'));
  assert.equal(result.blockers.some(item => item.assetId === 'page:旧画面'), false);
  assert.ok(result.unresolved.some(item => item.assetId === 'report:請求' && item.code === 'PARTIAL'));
  assert.ok(result.unresolved.some(item => item.code === 'STATIC_DEFINITION_ONLY'));
  assert.ok(result.evidence.some(value => value.includes('report:請求') && value.includes('自動追加していません')));
});

test('report-root: 選択起点はフォーム、ページ、帳票だけを受け付ける', () => {
  const input = normalizeInventory(inventory([
    asset('form:受注', 'form', ['table:受注']),
    asset('page:旧画面', 'page', [], { status: 'unsupported' }),
    asset('report:請求', 'report', ['table:受注']),
    asset('table:受注', 'table'),
    asset('query:受注', 'query', ['table:受注']),
    asset('macro:印刷', 'macro'),
    asset('module:共通', 'module'),
    asset('external:外部', 'external')
  ]));
  assert.deepEqual(analyzeSelection(input, ['form:受注']).selectedIds, ['form:受注']);
  const page = analyzeSelection(input, ['page:旧画面']);
  assert.deepEqual(page.selectedIds, ['page:旧画面']);
  assert.ok(page.blockers.some(item => item.assetId === 'page:旧画面' && item.code === 'UNSUPPORTED'));
  assert.equal(page.selectedIds.includes('form:受注'), false);
  assert.equal(page.selectedIds.includes('report:請求'), false);
  for (const id of ['table:受注', 'query:受注', 'macro:印刷', 'module:共通', 'external:外部']) {
    assert.throws(() => analyzeSelection(input, [id]), /帳票/, id);
  }
  assert.throws(() => analyzeSelection(input, []), /帳票/);
});

test('report-root: VBA要約は保持しmoduleTypeはstandardかclassだけ', () => {
  const input = inventory([
    asset('form:受注', 'form', [], { hasCodeModule: true, moduleType: 'class', procedureCount: 2, procedureNames: ['Form_Load', '請求計算'] }),
    asset('report:請求', 'report', [], { hasCodeModule: true, moduleType: 'class', procedureCount: 1, procedureNames: ['Report_Open'] }),
    asset('module:標準', 'module', [], { hasCodeModule: true, moduleType: 'standard', procedureCount: 1, procedureNames: ['TaxRate'] }),
    asset('module:クラス', 'module', [], { hasCodeModule: true, moduleType: 'class', procedureCount: 0, procedureNames: [] }),
    asset('module:名前のみ', 'module', [], { procedureNames: ['OnlyName'] }),
    asset('report:件数のみ', 'report', [], { hasCodeModule: true, moduleType: 'class', procedureCount: 3 }),
    asset('form:画面のみ', 'form', [], { hasCodeModule: false })
  ]);
  const normalized = normalizeInventory(input);
  const form = normalized.assets.find(item => item.id === 'form:受注');
  assert.equal(form.kind, 'form');
  assert.equal(form.hasCodeModule, true);
  assert.equal(form.moduleType, 'class');
  assert.equal(form.procedureCount, 2);
  assert.deepEqual(form.procedureNames, ['Form_Load', '請求計算']);
  const report = normalized.assets.find(item => item.id === 'report:請求');
  assert.equal(report.kind, 'report');
  assert.equal(report.moduleType, 'class');
  assert.deepEqual(report.procedureNames, ['Report_Open']);
  assert.equal(normalized.assets.find(item => item.id === 'module:標準').moduleType, 'standard');
  const classModule = normalized.assets.find(item => item.id === 'module:クラス');
  assert.equal(classModule.moduleType, 'class');
  assert.deepEqual(classModule.procedureNames, []);
  assert.equal(classModule.procedureCount, 0);
  const namesOnly = normalized.assets.find(item => item.id === 'module:名前のみ');
  assert.deepEqual(namesOnly.procedureNames, ['OnlyName']);
  assert.equal(namesOnly.procedureCount, undefined);
  assert.equal(namesOnly.moduleType, undefined);
  const countOnly = normalized.assets.find(item => item.id === 'report:件数のみ');
  assert.equal(countOnly.kind, 'report');
  assert.equal(countOnly.moduleType, 'class');
  assert.equal(countOnly.procedureCount, 3);
  assert.equal(countOnly.procedureNames, undefined);
  const bare = normalized.assets.find(item => item.id === 'form:画面のみ');
  assert.equal(bare.hasCodeModule, false);
  assert.equal(bare.moduleType, undefined);
  assert.equal(bare.procedureNames, undefined);
  assert.equal(JSON.stringify(normalized).includes('Debug.Print'), false);
  const merged = domain.mergeInventories([input]);
  const mergedForm = merged.assets.find(item => item.name === '受注');
  assert.equal(mergedForm.kind, 'form');
  assert.equal(mergedForm.hasCodeModule, true);
  assert.equal(mergedForm.moduleType, 'class');
  assert.deepEqual(mergedForm.procedureNames, ['Form_Load', '請求計算']);
});

test('report-root: 不正なmoduleTypeとソース断片は拒否する', () => {
  const base = (extra, kind = 'form', id = 'form:A') => inventory([asset(id, kind, [], extra)]);
  for (const extra of [
    { moduleType: 'form' },
    { moduleType: 'report' },
    { hasCodeModule: true, moduleType: 'standard' },
    { hasCodeModule: 'true' },
    { procedureCount: -1 },
    { procedureCount: 1.5 },
    { procedureCount: '1' },
    { procedureNames: 'Form_Load' },
    { procedureNames: ['Public Sub Form_Load()'] },
    { procedureNames: ['Sub Form_Load()\n  Debug.Print 1\nEnd Sub'] },
    { procedureNames: ['Form Load'] },
    { procedureNames: ['1Load'] },
    { procedureNames: ['PWD="secret"'] },
    { procedureNames: ['C:\\private\\mod.bas'] },
    { procedureNames: ['Form_Load', 'Form_Load'] },
    { procedureCount: 1, procedureNames: ['Form_Load', 'Extra'] },
    { hasCodeModule: false, moduleType: 'class' },
    { hasCodeModule: false, procedureNames: ['Form_Load'] },
    { hasCodeModule: false, procedureCount: 1 }
  ]) {
    assert.throws(() => normalizeInventory(base(extra)), JSON.stringify(extra));
  }
  assert.throws(() => normalizeInventory(base({ moduleType: 'form' }, 'report', 'report:A')));
  assert.throws(() => normalizeInventory(base({ moduleType: 'standard' }, 'report', 'report:A')));
  assert.throws(() => normalizeInventory(base({ hasCodeModule: true, moduleType: 'form' }, 'module', 'module:A')));
  assert.throws(() => normalizeInventory(inventory([asset('table:A', 'table', [], { hasCodeModule: false })])));
  assert.throws(() => normalizeInventory(inventory([asset('query:A', 'query', [], { procedureNames: ['Run'] })])));
  assert.throws(() => normalizeInventory(inventory([asset('page:A', 'page', [], { moduleType: 'class' })])));
  assert.throws(() => normalizeInventory(inventory([asset('macro:A', 'macro', [], { procedureCount: 0 })])));
  assert.throws(() => normalizeInventory(inventory([asset('external:A', 'external', [], { hasCodeModule: false })])));
});

test('report-root: プロシージャ名と件数の上限を検証する', () => {
  const names = count => Array.from({ length: count }, (_, index) => `Proc${index}`);
  const ok = normalizeInventory(inventory([asset('module:大', 'module', [], { hasCodeModule: true, moduleType: 'standard', procedureCount: 5000, procedureNames: names(1000) })]));
  assert.equal(ok.assets[0].procedureNames.length, 1000);
  assert.equal(ok.assets[0].procedureCount, 5000);
  const maxLength = normalizeInventory(inventory([asset('module:境界', 'module', [], { hasCodeModule: true, moduleType: 'standard', procedureCount: 1, procedureNames: ['a'.repeat(255)] })]));
  assert.equal(maxLength.assets[0].procedureNames[0].length, 255);
  for (const extra of [
    { hasCodeModule: true, moduleType: 'standard', procedureCount: 5000, procedureNames: names(1001) },
    { hasCodeModule: true, moduleType: 'standard', procedureCount: 100001 },
    { hasCodeModule: true, moduleType: 'standard', procedureNames: ['a'.repeat(256)] }
  ]) assert.throws(() => normalizeInventory(inventory([asset('module:M', 'module', [], extra)])), JSON.stringify(extra).slice(0, 60));
});

test('report-root: Data Access Pagesの取得失敗はページ0件にしない', () => {
  const input = normalizeInventory(inventory([
    asset('form:受注', 'form'),
    asset('report:請求', 'report')
  ], { limitations: [{ code: 'DATA_ACCESS_PAGES_UNAVAILABLE', message: 'Access Data Access Pages の一覧を取得できませんでした。不在と取得失敗を区別できないため、ページがないとは判断しません。' }] }));
  assert.equal(input.assets.some(item => item.kind === 'page'), false);
  assert.equal(input.limitations.filter(item => item.code === 'DATA_ACCESS_PAGES_UNAVAILABLE').length, 1);
  assert.match(input.limitations[0].message, /ページがないとは判断しません/);
  const result = analyzeSelection(input, ['report:請求']);
  assert.deepEqual(result.selectedIds, ['report:請求']);
  assert.ok(result.unresolved.some(item => item.code === 'DATA_ACCESS_PAGES_UNAVAILABLE'));
  assert.equal(result.selectedIds.includes('form:受注'), false);
});

test('report-root: 選択画面の文言は帳票とData Access Pagesを示す', async () => {
  const source = await readFile(new URL('../public/app.mjs', import.meta.url), 'utf8');
  assert.match(source, /asset\.kind === 'form' \|\| asset\.kind === 'page' \|\| asset\.kind === 'report'/);
  assert.match(source, /\['report', '帳票'\]/);
  assert.match(source, /フォーム・ページ・帳票を選択/);
  assert.match(source, /Access Data Access Pages（レガシー）/);
  assert.match(source, /DATA_ACCESS_PAGES_UNAVAILABLE/);
  assert.match(source, /ページがないとは判断しません/);
  assert.match(source, /取得できても非対応のままです/);
});

test('計画に業務フロー候補とER図のMermaidを含める', () => {
  const input = inventory([
    asset('form:受注', 'form', ['table:受注']), asset('form:顧客', 'form', ['table:受注']),
    asset('table:受注', 'table', [], { fields: [{ name: '受注番号', dataType: 'Long', isPrimaryKey: true }, { name: '顧客"ID', dataType: '長整数' }] }),
    asset('table:顧客', 'table', [], { fields: [{ name: '顧客ID', dataType: 'Long', isPrimaryKey: true }] }),
    asset('table:無関係', 'table')
  ], { relations: [{ from: 'table:受注', to: 'table:顧客', fields: [{ from: '顧客ID', to: '顧客ID' }] }] });
  const plan = buildPlan(input, { selectedIds: ['form:受注'], usage: solo });
  assert.match(plan.diagrams.flow, /^flowchart LR/);
  assert.match(plan.diagrams.flow, /form: 受注/);
  assert.match(plan.diagrams.flow, /form: 顧客/, '共有資産に依存する未選択画面も点線で示す');
  assert.match(plan.diagrams.er, /^erDiagram/);
  assert.match(plan.diagrams.er, /\|\|--o\{/);
  assert.match(plan.diagrams.er, /顧客#quot;ID/);
  assert.doesNotMatch(plan.diagrams.er, /無関係/);
  const text = renderPlanMarkdown(plan);
  assert.match(text, /```mermaid\nflowchart LR/);
  assert.match(text, /```mermaid\nerDiagram/);
});

test('影響候補の未選択画面は中間資産を経由する経路も図に示す', async () => {
  const demo = JSON.parse(await readFile(new URL('../samples/demo.inventory.json', import.meta.url), 'utf8'));
  const { flow } = buildPlan(demo, { selectedIds: ['form:受注入力'], usage: solo }).diagrams;
  const id = name => flow.match(new RegExp(String.raw`(n\d+)\[\(?"[a-z]+: ${name}"`))[1];
  const lines = flow.split('\n').map(line => line.trim());
  assert.ok(lines.includes(`${id('請求書')} -.-> ${id('請求対象')}`));
  assert.ok(lines.includes(`${id('請求対象')} -.->|影響候補| ${id('受注')}`));
});

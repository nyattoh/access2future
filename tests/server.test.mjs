import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createAppServer } from '../src/server.mjs';
import { setTimeout as delay } from 'node:timers/promises';
import { randomUUID } from 'node:crypto';

const inventory = {
  schemaVersion: 1,
  source: { name: '合成サンプル.accdb', kind: 'synthetic' },
  assets: [
    { id: 'table:orders', kind: 'table', name: 'orders', status: 'supported', dependsOn: [], issues: [] },
    { id: 'form:orders', kind: 'form', name: '受注入力', status: 'supported', dependsOn: ['table:orders'], issues: [] },
    { id: 'form:billing', kind: 'form', name: '請求処理', status: 'supported', dependsOn: ['table:orders'], issues: [] },
  ],
  relations: [], limitations: [],
};
const usage = { users: 'solo', concurrentEditing: false, location: 'device', offlineRequired: true, permissions: 'same', coexistence: 'undecided' };

async function withServer(run, overrides = {}) {
  const tempDir = await mkdtemp(join(tmpdir(), 'access2future-http-test-'));
  const server = createAppServer({
    tempDir,
    demoInventory: inventory,
    capabilities: async () => ({ available: true }),
    importer: async () => inventory,
    ...overrides,
  });
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
  const base = `http://127.0.0.1:${server.address().port}`;
  try { await run(base, tempDir); }
  finally {
    await new Promise((resolve) => { server.close(resolve); server.closeAllConnections(); });
    await rm(tempDir, { recursive: true, force: true });
  }
}

test('ローカルのヘルスと合成デモを、実解析と区別して返す', async () => {
  await withServer(async (base) => {
    const health = await fetch(`${base}/api/health`);
    assert.equal(health.status, 200);
    const state = await health.json();
    assert.equal(state.access.available, true);
    assert.equal(health.headers.get('access-control-allow-origin'), null);
    assert.match(health.headers.get('cache-control'), /no-store/);
    const demo = await (await fetch(`${base}/api/demo`)).json();
    assert.equal(demo.source.kind, 'synthetic');
  });
});

test('環境確認を共有し、画面を開くたびに新しいAccessを起動しない', async () => {
  let checks = 0;
  await withServer(async (base) => {
    const responses = await Promise.all([fetch(`${base}/api/health`), fetch(`${base}/api/health`)]);
    assert.ok(responses.every((response) => response.status === 200));
    await fetch(`${base}/api/health`);
    assert.equal(checks, 1);
  }, { capabilities: async () => { checks++; await new Promise((resolve) => setTimeout(resolve, 15)); return { available: true }; } });
});

test('不正な版のInventoryと壊れたJSONを拒否する', async () => {
  await withServer(async (base) => {
    for (const body of ['{', JSON.stringify({ ...inventory, schemaVersion: 99 })]) {
      const response = await fetch(`${base}/api/inventory`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body });
      assert.equal(response.status, 400);
      assert.equal(typeof (await response.json()).error.message, 'string');
    }
  });
});

test('JSONの自己申告を、実Access解析の実績として表示しない', async () => {
  await withServer(async (base) => {
    const response = await fetch(`${base}/api/inventory`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ ...inventory, source: { name: 'claimed.accdb', kind: 'access' } }) });
    assert.equal(response.status, 200);
    assert.equal((await response.json()).source.kind, 'inventory');
  });
});

test('利用者が取り込んだフロント・バックの構造資料だけでリンクを解決する', async () => {
  await withServer(async (base) => {
    const frontend = {
      schemaVersion: 1, source: { name: 'frontend.mdb', kind: 'inventory' },
      assets: [
        { id: 'form:entry', name: '入力', kind: 'form', status: 'supported', dependsOn: ['table:alias'], issues: [] },
        { id: 'table:alias', name: 'alias', kind: 'table', status: 'partial', dependsOn: [], issues: [], linked: true, linkedDatabaseName: 'backend.accdb', linkedTableName: 'records' },
      ], relations: [], limitations: [],
    };
    const backend = {
      schemaVersion: 1, source: { name: 'backend.accdb', kind: 'inventory' },
      assets: [{ id: 'table:records', name: 'records', kind: 'table', status: 'supported', dependsOn: [], issues: [], local: true, fields: [{ name: 'ID', dataType: 'Long', required: true, isPrimaryKey: true }] }],
      relations: [], limitations: [],
    };
    const response = await fetch(`${base}/api/merge`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ inventories: [frontend, backend] }) });
    assert.equal(response.status, 200);
    const merged = await response.json();
    assert.equal(merged.source.kind, 'inventory');
    assert.equal(merged.assets.length, 3);
    const alias = merged.assets.find((asset) => asset.name === 'alias');
    assert.ok(alias.dependsOn.some((id) => id.includes('records')));
    assert.equal(alias.fields[0].isPrimaryKey, true);
    assert.ok(!JSON.stringify(merged).includes('C:/'));
  });
});

test('選択と利用形態から、共有資産への影響と未確認事項を含む計画を返す', async () => {
  await withServer(async (base) => {
    const response = await fetch(`${base}/api/plan`, {
      method: 'POST', headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ inventory, selectedIds: ['form:orders'], usage, targetId: 'web', notes: '業務担当者による確認が必要' }),
    });
    assert.equal(response.status, 200);
    const { plan, markdown } = await response.json();
    assert.deepEqual(plan.analysis.selectedIds, ['form:orders']);
    assert.ok(plan.analysis.impactedIds.includes('form:billing'));
    assert.equal(plan.status, 'review-required');
    assert.equal(typeof markdown, 'string');
    assert.ok(markdown.length > 100);
    assert.ok(plan.requirements.every((item) => item.confirmed !== true));
    assert.equal(plan.source.kind, 'synthetic');
  });
});

test('利用者の選択が空の計画を受入れない', async () => {
  await withServer(async (base) => {
    const response = await fetch(`${base}/api/plan`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ inventory, selectedIds: [], usage, targetId: 'web' }) });
    assert.equal(response.status, 400);
    assert.match((await response.json()).error.message, /フォーム・ページ・帳票/);
  });
});

test('別サイトからのAPI要求と任意パスの指定を拒否する', async () => {
  await withServer(async (base) => {
    const external = await fetch(`${base}/api/demo`, { headers: { Origin: 'https://unrelated.invalid' } });
    assert.equal(external.status, 403);
    const arbitrary = await fetch(`${base}/api/import`, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ path: 'C:/private/data.accdb' }) });
    assert.equal(arbitrary.status, 415);
    const traversal = await fetch(`${base}/%2e%2e%2fpackage.json`);
    assert.equal(traversal.status, 400);
  });
});

test('Access拡張子に偽装したデータは、COMを呼ぶ前に拒否する', async () => {
  let calls = 0;
  await withServer(async (base) => {
    const response = await fetch(`${base}/api/import`, { method: 'POST', headers: { 'Content-Type': 'application/octet-stream', 'X-File-Name': encodeURIComponent('偽のファイル.accdb') }, body: 'not an Access file' });
    assert.equal(response.status, 400);
    assert.equal(calls, 0);
  }, { importer: async () => { calls++; return inventory; } });
});

test('アップロードをランダムな一時パスに分離し、処理後に削除する', async () => {
  let observedPath;
  await withServer(async (base, tempDir) => {
    const binary = Buffer.alloc(512);
    binary.write('Standard Jet DB', 4, 'ascii');
    const response = await fetch(`${base}/api/import`, { method: 'POST', headers: { 'Content-Type': 'application/octet-stream', 'X-File-Name': encodeURIComponent('業務資料.mdb') }, body: binary });
    assert.equal(response.status, 200);
    const imported = await response.json();
    assert.equal(imported.source.name, '業務資料.mdb');
    assert.ok(observedPath.startsWith(tempDir));
    assert.ok(!observedPath.includes('業務資料'));
    assert.deepEqual(await readdir(tempDir), []);
  }, { importer: async (path) => { observedPath = path; return inventory; } });
});

test('サイズ上限を超える入力を解析しない', async () => {
  await withServer(async (base) => {
    const response = await fetch(`${base}/api/import`, { method: 'POST', headers: { 'Content-Type': 'application/octet-stream', 'X-File-Name': 'large.mdb' }, body: Buffer.alloc(100) });
    assert.equal(response.status, 413);
  }, { maxFileBytes: 64 });
});

test('内部のパスや秘密を処理エラーの応答に含めない', async () => {
  await withServer(async (base, tempDir) => {
    const binary = Buffer.alloc(512);
    binary.write('Standard ACE DB', 4, 'ascii');
    const response = await fetch(`${base}/api/import`, { method: 'POST', headers: { 'Content-Type': 'application/octet-stream', 'X-File-Name': 'sample.accdb' }, body: binary });
    assert.equal(response.status, 500);
    const text = await response.text();
    assert.ok(!text.includes('DO_NOT_EXPOSE'));
    assert.ok(!text.includes('C:/private'));
    assert.deepEqual(await readdir(tempDir), []);
  }, { importer: async () => { throw new Error('C:/private DO_NOT_EXPOSE'); } });
});

test('解析の中止はworker停止とファイル削除を待ち、次の解析を受け付ける', async () => {
  let began;
  const started = new Promise(resolve => { began = resolve; });
  let calls = 0;
  let workerStopped = false;
  await withServer(async (base, tempDir) => {
    const binary = Buffer.alloc(512); binary.write('Standard ACE DB', 4, 'ascii');
    const init = { method: 'POST', headers: { 'Content-Type': 'application/octet-stream', 'X-File-Name': 'dummy.accdb' }, body: binary };
    const first = fetch(`${base}/api/import`, init).catch(error => ({ status: 0, error }));
    await started;
    const busy = await fetch(`${base}/api/import`, init); assert.equal(busy.status, 409);
    const cancelled = await fetch(`${base}/api/import/cancel`, { method: 'POST' });
    assert.equal(cancelled.status, 200);
    assert.deepEqual(await cancelled.json(), { cancelled: true });
    assert.equal(workerStopped, true);
    assert.deepEqual(await readdir(tempDir), []);
    assert.equal((await first).status, 499);
    const second = await fetch(`${base}/api/import`, init); assert.equal(second.status, 200);
    const idle = await fetch(`${base}/api/import/cancel`, { method: 'POST' });
    assert.deepEqual(await idle.json(), { cancelled: false });
  }, { importer: async (_path, options) => {
    if (++calls > 1) return inventory;
    began();
    await new Promise(resolve => options.signal?.addEventListener('abort', resolve, { once: true }));
    await delay(30); workerStopped = true;
    throw Object.assign(new Error('cancelled'), { code: 'ACCESS_CANCELLED' });
  } });
});

test('画面が閉じた解析も中止し、処理枠とファイルを解放する', async () => {
  let began;
  const started = new Promise(resolve => { began = resolve; });
  let stopped;
  const halted = new Promise(resolve => { stopped = resolve; });
  await withServer(async (base, tempDir) => {
    const binary = Buffer.alloc(512); binary.write('Standard ACE DB', 4, 'ascii');
    const controller = new AbortController();
    const response = fetch(`${base}/api/import`, { method: 'POST', headers: { 'Content-Type': 'application/octet-stream', 'X-File-Name': 'dummy.accdb' }, body: binary, signal: controller.signal }).catch(error => error.name);
    await started; controller.abort();
    assert.equal(await response, 'AbortError');
    await Promise.race([halted, delay(1000).then(() => { throw new Error('Disconnected import was not cancelled'); })]);
    const cleanup = await fetch(`${base}/api/import/cancel`, { method: 'POST' }); assert.equal(cleanup.status, 200);
    assert.deepEqual(await readdir(tempDir), []);
  }, { importer: async (_path, options) => {
    began();
    await new Promise(resolve => options.signal?.addEventListener('abort', resolve, { once: true }));
    stopped(); throw Object.assign(new Error('cancelled'), { code: 'ACCESS_CANCELLED' });
  } });
});

test('解析IDに対応した実測進捗だけを返し、逆行と秘密情報を除外する', async () => {
  const id = randomUUID();
  let advance;
  let finish;
  let began;
  const started = new Promise(resolve => { began = resolve; });
  const finished = new Promise(resolve => { finish = resolve; });
  await withServer(async base => {
    const get = async queryId => (await fetch(`${base}/api/import/progress?requestId=${queryId}`)).json();
    assert.equal((await get(id)).state, 'waiting');
    const binary = Buffer.alloc(512); binary.write('Standard ACE DB', 4, 'ascii');
    const result = fetch(`${base}/api/import`, { method: 'POST', headers: { 'Content-Type': 'application/octet-stream', 'X-File-Name': 'dummy.accdb', 'X-Import-Id': id }, body: binary }).catch(error => ({ status: 0, error }));
    await started;
    try {
      const measured = await get(id);
      assert.deepEqual(measured, { requestId: id, state: 'running', phase: 'analysing', completed: 2, total: 10 });
      advance({ phase: 'analysing', completed: 1, total: 10 });
      advance({ phase: 'analysing', completed: 9, total: 8 });
      advance({ phase: 'analysing', completed: 3, total: 11 });
      assert.deepEqual(await get(id), measured);
      advance({ phase: 'analysing', completed: 6, total: 10, secret: 'DO_NOT_EXPOSE' });
      const updated = await get(id); assert.equal(updated.completed, 6); assert.ok(!JSON.stringify(updated).includes('DO_NOT_EXPOSE'));
      assert.equal((await get(randomUUID())).state, 'waiting');
      advance({ phase: 'finishing', completed: 10, total: 10 });
    } finally { finish(); }
    assert.equal((await result).status, 200);
    const done = await get(id); assert.equal(done.state, 'completed'); assert.equal(done.completed, 10); assert.equal(done.total, 10);
    const invalid = await fetch(`${base}/api/import/progress?requestId=../private`); assert.equal(invalid.status, 400);
  }, { importer: async (_path, options) => {
    advance = options.onProgress;
    advance?.({ phase: 'analysing', completed: 2, total: 10, password: 'DO_NOT_EXPOSE' });
    began(); await finished; return inventory;
  } });
});

test('取消し後の進捗は完了とせず、遅いcallbackを捨てる', async () => {
  const id = randomUUID();
  let advance;
  let began;
  const started = new Promise(resolve => { began = resolve; });
  await withServer(async base => {
    const binary = Buffer.alloc(512); binary.write('Standard ACE DB', 4, 'ascii');
    const result = fetch(`${base}/api/import`, { method: 'POST', headers: { 'Content-Type': 'application/octet-stream', 'X-File-Name': 'dummy.accdb', 'X-Import-Id': id }, body: binary }).catch(error => ({ status: 0, error }));
    await started;
    await fetch(`${base}/api/import/cancel`, { method: 'POST' });
    assert.equal((await result).status, 499);
    advance?.({ phase: 'finishing', completed: 10, total: 10 });
    const response = await fetch(`${base}/api/import/progress?requestId=${id}`);
    const snapshot = await response.json(); assert.equal(snapshot.state, 'cancelled'); assert.equal(snapshot.completed, 2);
  }, { importer: async (_path, options) => {
    advance = options.onProgress;
    advance?.({ phase: 'analysing', completed: 2, total: 10 });
    began(); await new Promise(resolve => options.signal.addEventListener('abort', resolve, { once: true }));
    throw Object.assign(new Error('Cancelled'), { code: 'ACCESS_CANCELLED' });
  } });
});

test('解析上限15分をimporterへ渡し、時間切れでも実測件数をfailedで保持する', async () => {
  const id = randomUUID();
  let received;
  await withServer(async base => {
    const binary = Buffer.alloc(512); binary.write('Standard ACE DB', 4, 'ascii');
    const response = await fetch(`${base}/api/import`, { method: 'POST', headers: { 'Content-Type': 'application/octet-stream', 'X-File-Name': 'dummy.mdb', 'X-Import-Id': id }, body: binary });
    assert.equal(received, 900_000);
    assert.equal(response.status, 504);
    const { error } = await response.json();
    assert.equal(error.code, 'ACCESS_TIMEOUT'); assert.match(error.message, /15分/); assert.doesNotMatch(error.message, /180|3分/);
    const snapshot = await (await fetch(`${base}/api/import/progress?requestId=${id}`)).json();
    assert.deepEqual(snapshot, { requestId: id, state: 'failed', phase: 'analysing', completed: 74, total: 128 });
    assert.equal((await (await fetch(`${base}/api/import/progress?requestId=${randomUUID()}`)).json()).state, 'waiting');
  }, { importer: async (_path, options) => {
    received = options.timeoutMs;
    options.onProgress({ phase: 'analysing', completed: 74, total: 128 });
    throw Object.assign(new Error('timeout'), { code: 'ACCESS_TIMEOUT' });
  } });
});

test('総数が未確定のまま失敗したときは、件数を作らずfailedで返す', async () => {
  const id = randomUUID();
  await withServer(async base => {
    const binary = Buffer.alloc(512); binary.write('Standard ACE DB', 4, 'ascii');
    const response = await fetch(`${base}/api/import`, { method: 'POST', headers: { 'Content-Type': 'application/octet-stream', 'X-File-Name': 'dummy.mdb', 'X-Import-Id': id }, body: binary });
    assert.equal(response.status, 504);
    assert.deepEqual(await (await fetch(`${base}/api/import/progress?requestId=${id}`)).json(), { requestId: id, state: 'failed', phase: 'enumerating', completed: 0, total: null });
  }, { importer: async (_path, options) => {
    options.onProgress({ phase: 'enumerating', completed: 0, total: null });
    throw Object.assign(new Error('timeout'), { code: 'ACCESS_TIMEOUT' });
  } });
});

import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile, readFile, stat, chmod, rm, rename } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { importAccess, getAccessCapabilities } from '../src/access-import.mjs';
import { setTimeout as delay } from 'node:timers/promises';

async function fixture(fn) {
  const dir = await mkdtemp(join(tmpdir(), 'access-import-test-'));
  const path = join(dir, 'synthetic.accdb');
  const bytes = Buffer.alloc(256); bytes.write('Standard ACE DB', 4, 'ascii');
  await writeFile(path, bytes);
  try { await fn(path); } finally { await rm(dir, { recursive: true, force: true }); }
}
const inventory = () => ({schemaVersion:1,source:{name:'synthetic.accdb',kind:'access'},assets:[{id:'form:Orders',kind:'form',name:'Orders',dependsOn:['table:Customers'],status:'partial',issues:[{code:'DYNAMIC_REFERENCE',message:'Needs review'}]}],relations:[],limitations:[{code:'DYNAMIC_REFERENCE',message:'Needs review',assetId:'form:Orders'}]});

test('signature and extension are rejected before spawning a process', async () => {
  await fixture(async path => {
    await writeFile(path, 'not an Access database');
    await assert.rejects(importAccess(path, { runner: () => assert.fail('must not spawn') }), {code:'INVALID_ACCESS_FILE'});
    await assert.rejects(importAccess(path+'.txt'), {code:'INVALID_ACCESS_FILE'});
  });
});
test('partial extraction preserves unresolved dependencies and adds a fingerprint', async () => {
  await fixture(async path => {
    const result = await importAccess(path, {runner:async (command,args,options) => {
      assert.ok(Array.isArray(args)); assert.equal(options.shell,false); assert.ok(options.timeout > 0);
      return {stdout:JSON.stringify(inventory())};
    }});
    assert.equal(result.source.kind,'access'); assert.match(result.source.fingerprint,/^[a-f0-9]{64}$/);
    assert.equal(result.assets[0].status,'partial'); assert.deepEqual(result.assets[0].dependsOn,['table:Customers']);
    assert.equal(result.limitations[0].code,'DYNAMIC_REFERENCE');
  });
});
test('VBAを持たないフォームの空の手続き要約を受け入れる', async () => {
  await fixture(async path => {
    const data = inventory();
    Object.assign(data.assets[0], { hasCodeModule: false, procedureNames: [], procedureCount: 0 });
    const result = await importAccess(path, { runner: async () => ({ stdout: JSON.stringify(data) }) });
    assert.equal(result.assets[0].hasCodeModule, false);
    assert.deepEqual(result.assets[0].procedureNames, []);
    assert.equal(result.assets[0].procedureCount, 0);
  });
});
test('native worker receives its own readonly copy; caller bytes remain unchanged', async () => {
  await fixture(async path => {
    const before=await readFile(path);
    await importAccess(path,{runner:async (_command,args) => {
      const staged=args[args.indexOf('-SourcePath')+1]; assert.notEqual(staged,path);
      assert.equal((await stat(staged)).mode & 0o222,0);
      // Simulate the source mutation observed in Access without touching caller input.
      await chmod(staged,0o666); await writeFile(staged,'changed worker copy');
      return {stdout:JSON.stringify(inventory())};
    }});
    assert.deepEqual(await readFile(path),before);
  });
});
test('解析予算の既定は15分(900000ms)、明示指定はそれに優先する', async () => {
  await fixture(async path => {
    const budgets = [];
    await importAccess(path, { runner: async (_command, _args, options) => {
      budgets.push(options.timeout); // 子のPowerShellへ渡る実効タイムアウト。
      return { stdout: JSON.stringify(inventory()) };
    } });
    await importAccess(path, { timeoutMs: 12_000, runner: async (_command, _args, options) => {
      budgets.push(options.timeout);
      return { stdout: JSON.stringify(inventory()) };
    } });
    assert.deepEqual(budgets, [900_000, 12_000]);
  });
});
test('worker failures and timeout never expose paths, SQL or credentials', async () => {
  await fixture(async path => {
    for (const [failure, code] of [[{code:'ETIMEDOUT',message:path+' password=secret'},'ACCESS_TIMEOUT'],[{code:'ENOENT',message:path},'ACCESS_UNAVAILABLE'],[{code:1,stderr:'password=secret '+path},'ACCESS_EXTRACTION_FAILED']]) {
      await assert.rejects(importAccess(path,{runner:async()=>{throw failure;}}), err=>err.code===code&&!err.message.includes(path)&&!err.message.includes('secret'));
    }
  });
});
test('malformed worker JSON is rejected', async () => {
  await fixture(async path => {
    await assert.rejects(importAccess(path,{runner:async()=>({stdout:'not json'})}),{code:'ACCESS_EXTRACTION_FAILED'});
    await assert.rejects(importAccess(path,{runner:async()=>({stdout:'{"schemaVersion":2}'})}),{code:'ACCESS_EXTRACTION_FAILED'});
  });
});
test('worker output uses the shared safety allowlist', async () => {
  await fixture(async path => {
    const data=inventory(); data.rawVBA='private'; data.assets[0].rawVBA='private'; data.assets[0].caption='Password=secret; C:\\private\\source.mdb';
    const result=await importAccess(path,{runner:async()=>({stdout:JSON.stringify(data)})});
    const json=JSON.stringify(result); assert.ok(!json.includes('rawVBA')); assert.ok(!json.includes('secret')); assert.ok(!json.includes('private'));
  });
});
test('capability failures are unavailable and do not leak native diagnostics', async () => {
  assert.deepEqual(await getAccessCapabilities({runner:async()=>{throw new Error('private path');}}),{available:false,reason:'Microsoft Access COM を利用できません。'});
  assert.deepEqual(await getAccessCapabilities({runner:async()=>({stdout:'{"available":true}'})}),{available:true});
});

test('中止された解析は起動せず、進行中のworkerにも中止を伝える', async () => {
  await fixture(async path => {
    const before = await readFile(path);
    const alreadyCancelled = new AbortController();
    alreadyCancelled.abort();
    await assert.rejects(importAccess(path, { signal: alreadyCancelled.signal, runner: () => assert.fail('cancelled import must not launch') }), { code: 'ACCESS_CANCELLED' });
    const controller = new AbortController();
    let staged;
    await assert.rejects(importAccess(path, { signal: controller.signal, runner: async (_command, args, options) => {
      staged = args[args.indexOf('-WorkDirectory') + 1];
      assert.ok(options.signal instanceof AbortSignal);
      await writeFile(join(staged, 'access-owner.json'), JSON.stringify({ pid: 2147483647, startedAtTicks: '0' }));
      const cancelled = new Promise((_resolve, reject) => options.signal.addEventListener('abort', () => reject(Object.assign(new Error('private diagnostics'), { name: 'AbortError', code: 'ABORT_ERR' })), { once: true }));
      controller.abort();
      return cancelled;
    } }), error => error.code === 'ACCESS_CANCELLED' && !error.message.includes('private'));
    await assert.rejects(stat(staged), { code: 'ENOENT' });
    assert.deepEqual(await readFile(path), before);
  });
});

test('onProgress は実測 snapshot を 0→途中→完了 で渡し、callback 例外を伝播しない', async () => {
  await fixture(async path => {
    const snapshots = [];
    const result = await importAccess(path, { runner: async (_command, args) => {
      const work = args[args.indexOf('-WorkDirectory') + 1];
      const progress = join(work, 'progress.json');
      // 原子置換で書き、読み手が torn JSON を観測しない（PowerShell と同じ方式）。
      const write = async snapshot => { await writeFile(progress + '.tmp', JSON.stringify(snapshot)); await rename(progress + '.tmp', progress); };
      await write({ phase: 'preparing', completed: 0, total: null });
      await delay(60);
      await write({ phase: 'enumerating', completed: 0, total: null });
      await delay(60);
      await write({ phase: 'analysing', completed: 0, total: 5 });
      await delay(300);
      await write({ phase: 'analysing', completed: 3, total: 5 });
      await delay(300);
      await write({ phase: 'finishing', completed: 5, total: 5 });
      await delay(120);
      return { stdout: JSON.stringify(inventory()) };
    }, onProgress: snapshot => { snapshots.push(snapshot); throw new Error('callback failure'); } });
    assert.equal(result.source.kind, 'access');
    assert.deepEqual(snapshots[0], { phase: 'preparing', completed: 0, total: null });
    assert.ok(snapshots.some(s => s.phase === 'analysing' && s.completed > 0 && s.completed < s.total), JSON.stringify(snapshots));
    assert.deepEqual(snapshots.at(-1), { phase: 'finishing', completed: 5, total: 5 });
    for (const snapshot of snapshots) {
      assert.deepEqual(Object.keys(snapshot).sort(), ['completed', 'phase', 'total']);
      assert.ok(Number.isSafeInteger(snapshot.completed) && snapshot.completed >= 0);
      assert.ok(snapshot.total === null || (Number.isSafeInteger(snapshot.total) && snapshot.total >= 0));
      if (snapshot.total !== null) assert.ok(snapshot.completed <= snapshot.total);
    }
    for (let index = 1; index < snapshots.length; index++) assert.ok(snapshots[index].completed >= snapshots[index - 1].completed);
  });
});
test('progress.json の不完全JSON・不正値・逆行は無視され、余剰字段は除去される', async () => {
  await fixture(async path => {
    const snapshots = [];
    await importAccess(path, { runner: async (_command, args) => {
      const progress = join(args[args.indexOf('-WorkDirectory') + 1], 'progress.json');
      const write = async text => { await writeFile(progress + '.tmp', text); await rename(progress + '.tmp', progress); await delay(220); };
      await write('{"phase":"analysing","completed":2,"total":5}');
      await write('{"phase":"analysing","completed":3,"total":5,"name":"SecretTable","sql":"SELECT password","path":"C:\\\\private"}');
      await write('{"phase":"analysing","comple');
      await write('{"phase":"hacking","completed":4,"total":5}');
      await write('{"phase":"analysing","completed":-1,"total":5}');
      await write('{"phase":"analysing","completed":9,"total":5}');
      await write('{"phase":"analysing","completed":"4","total":5}');
      await write('{"phase":"analysing","completed":4,"total":-2}');
      await write('{"phase":"analysing","completed":4,"total":6}');
      await write('{"phase":"enumerating","completed":3,"total":null}');
      await write('{"phase":"analysing","completed":1,"total":5}');
      await write('{"phase":"finishing","completed":5,"total":5}');
      return { stdout: JSON.stringify(inventory()) };
    }, onProgress: snapshot => snapshots.push(snapshot) });
    assert.deepEqual(snapshots, [
      { phase: 'preparing', completed: 0, total: null },
      { phase: 'analysing', completed: 2, total: 5 },
      { phase: 'analysing', completed: 3, total: 5 },
      { phase: 'finishing', completed: 5, total: 5 }
    ]);
    assert.ok(!JSON.stringify(snapshots).includes('SecretTable'));
  });
});
test('取消後は callback が停止し、progress.json も WorkDirectory ごと削除される', async () => {
  await fixture(async path => {
    const controller = new AbortController();
    const snapshots = [];
    let staged;
    await assert.rejects(importAccess(path, { signal: controller.signal, onProgress: snapshot => snapshots.push(snapshot), runner: async (_command, args, options) => {
      staged = args[args.indexOf('-WorkDirectory') + 1];
      await writeFile(join(staged, 'access-owner.json'), JSON.stringify({ pid: 2147483647, startedAtTicks: '0' }));
      await writeFile(join(staged, 'progress.json'), JSON.stringify({ phase: 'analysing', completed: 1, total: 4 }));
      const halted = new Promise((_resolve, reject) => options.signal.addEventListener('abort', () => reject(Object.assign(new Error('cancelled'), { name: 'AbortError', code: 'ABORT_ERR' })), { once: true }));
      controller.abort();
      return halted;
    } }), { code: 'ACCESS_CANCELLED' });
    assert.ok(snapshots.length > 0, '取消前に少なくとも初期snapshotが届いていること');
    const count = snapshots.length;
    await delay(400);
    assert.equal(snapshots.length, count);
    await assert.rejects(stat(join(staged, 'progress.json')), { code: 'ENOENT' });
  });
});
test('起動直後の中止は、所有PIDの完成した記録より先にPowerShellを終了しない', async () => {
  await fixture(async path => {
    const controller = new AbortController();
    await assert.rejects(importAccess(path, { signal: controller.signal, runner: async (_command, args, options) => {
      const directory = args[args.indexOf('-WorkDirectory') + 1];
      const ownerFile = join(directory, 'access-owner.json');
      const halted = new Promise((_resolve, reject) => options.signal.addEventListener('abort', () => reject(Object.assign(new Error('cancelled'), { code: 'ABORT_ERR' })), { once: true }));
      controller.abort();
      await delay(60);
      assert.equal(await readFile(join(directory, 'cancel-requested'), 'utf8'), '');
      assert.equal(options.signal.aborted, false);
      await writeFile(ownerFile, '{"pid":');
      await delay(60);
      assert.equal(options.signal.aborted, false);
      await writeFile(ownerFile, JSON.stringify({ pid: 2147483647, startedAtTicks: '0' }));
      return halted;
    } }), { code: 'ACCESS_CANCELLED' });
  });
});

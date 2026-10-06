import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { readFile, writeFile, chmod, mkdtemp, rm } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { basename, extname, join } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';
import { normalizeInventory } from './domain.mjs';

const execute = promisify(execFile);
const run = (...args) => {
  const pending = execute(...args);
  const closed = new Promise(resolve => pending.child.once('close', resolve));
  return pending.finally(() => closed);
};
const script = fileURLToPath(new URL('../scripts/export-access.ps1', import.meta.url));
function failure(code, message) { return Object.assign(new Error(message), { code }); }

const progressPhases = new Set(['preparing', 'enumerating', 'analysing', 'finishing']);
// 進捗は段階と件数だけを渡し、名前・SQL・パスを含めない。
function parseProgress(raw, previous) {
  let data;
  try { data = JSON.parse(raw); } catch { return null; }
  if (!data || typeof data !== 'object' || Array.isArray(data)) return null;
  if (!progressPhases.has(data.phase)) return null;
  if (!Number.isSafeInteger(data.completed) || data.completed < 0) return null;
  if (data.total !== null && (!Number.isSafeInteger(data.total) || data.total < 0)) return null;
  if (data.total !== null && data.completed > data.total) return null;
  if (data.total === null && data.completed !== 0) return null;
  const snapshot = { phase: data.phase, completed: data.completed, total: data.total };
  if (previous && (snapshot.completed < previous.completed || (previous.total !== null && snapshot.total !== previous.total) || [...progressPhases].indexOf(snapshot.phase) < [...progressPhases].indexOf(previous.phase))) return null;
  return snapshot;
}

// Verify the worker's COM process identity before forced cleanup after a timeout.
async function stopOwnedAccess(directory) {
  try {
    await run('powershell.exe', ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', script, '-CleanupOwned', '-WorkDirectory', directory], { shell: false, windowsHide: true, timeout: 10000 });
  } catch { /* Already exited, or COM was never created. */ }
}
export async function importAccess(filePath, options = {}) {
  if (options.signal?.aborted) throw failure('ACCESS_CANCELLED', 'Access 解析を中止しました。');
  if (!['.mdb', '.accdb'].includes(extname(filePath).toLowerCase())) throw failure('INVALID_ACCESS_FILE', 'Access ファイルの形式が不正です。');
  const bytes = await readFile(filePath).catch(() => { throw failure('INVALID_ACCESS_FILE', 'Access ファイルを読み取れません。'); });
  const signature = bytes.subarray(4, 20).toString('ascii');
  if (bytes.length < 128 || !/^Standard (Jet|ACE) DB\x00/.test(signature)) throw failure('INVALID_ACCESS_FILE', 'Access ファイルの署名が不正です。');
  const directory = await mkdtemp(join(tmpdir(), 'access2future-'));
  let interrupted = false;
  const workerController = new AbortController();
  let workerFinished = false;
  let cancellation;
  const cancel = () => {
    interrupted = true;
    cancellation = (async () => {
      await writeFile(join(directory, 'cancel-requested'), '');
      // Keep PowerShell alive until COM ownership is recorded or it exits safely.
      while (!workerFinished) {
        try {
          const owner = JSON.parse((await readFile(join(directory, 'access-owner.json'), 'utf8')).replace(/^\uFEFF/, ''));
          if (Number.isInteger(owner.pid) && owner.pid > 0 && /^\d+$/.test(owner.startedAtTicks)) { workerController.abort(); return; }
        } catch { /* Ownership can still be being written. */ }
        await delay(25);
      }
    })().catch(() => { /* Worker completion/timeout still owns final cleanup. */ });
  };
  // 実測進捗: worker が WorkDirectory の progress.json へ原子置換で書く snapshot を 150ms で読む。
  // 宣言は try の外。finally からも参照し、onProgress なしでも通常経路を維持する。
  const onProgress = typeof options.onProgress === 'function' ? options.onProgress : null;
  const progressPath = join(directory, 'progress.json');
  let lastProgress = null;
  let polling = true;
  const emitProgress = snapshot => {
    if (!onProgress || !snapshot || !polling || options.signal?.aborted) return;
    if (lastProgress && lastProgress.phase === snapshot.phase && lastProgress.completed === snapshot.completed && lastProgress.total === snapshot.total) return;
    lastProgress = snapshot;
    try { onProgress({ phase: snapshot.phase, completed: snapshot.completed, total: snapshot.total }); } catch { /* callback の失敗は解析を中止しない。 */ }
  };
  const readProgress = async () => { try { emitProgress(parseProgress((await readFile(progressPath, 'utf8')).replace(/^\uFEFF/, ''), lastProgress)); } catch { /* 未作成・破損・読取中の置換は無視。 */ } };
  const progressPoller = onProgress ? (async () => {
    emitProgress({ phase: 'preparing', completed: 0, total: null });
    while (polling) { await readProgress(); await delay(150); }
  })() : null;
  try {
    const stagedPath=join(directory,'readonly-source'+extname(filePath).toLowerCase());
    await writeFile(stagedPath,bytes); await chmod(stagedPath,0o444);
    options.signal?.addEventListener('abort', cancel, { once: true });
    if (options.signal?.aborted) cancel();
    const { stdout } = await (options.runner ?? run)('powershell.exe', ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', script, '-SourcePath', stagedPath, '-WorkDirectory', directory], { shell: false, windowsHide: true, signal: workerController.signal, timeout: options.timeoutMs ?? options.timeout ?? 900_000, maxBuffer: 16 * 1024 * 1024, encoding: 'utf8' });
    if (options.signal?.aborted) { interrupted = true; throw failure('ACCESS_CANCELLED', 'Access 解析を中止しました。'); }
    const result = JSON.parse(stdout.replace(/^\uFEFF/, '').trim());
    if (result.error) throw failure(result.error.code === 'ACCESS_UNAVAILABLE' ? 'ACCESS_UNAVAILABLE' : 'ACCESS_EXTRACTION_FAILED', 'Access のメタデータを取得できませんでした。');
    if (result.schemaVersion !== 1 || !Array.isArray(result.assets) || !Array.isArray(result.relations) || !Array.isArray(result.limitations)) throw new Error('Invalid inventory');
    result.source = { name: basename(filePath), kind: 'access', ...(result.source?.accessVersion ? { accessVersion: String(result.source.accessVersion) } : {}), analysedAt: new Date().toISOString(), fingerprint: createHash('sha256').update(bytes).digest('hex') };
    return normalizeInventory(result);
  } catch (error) {
    if (options.signal?.aborted || error.code === 'ABORT_ERR' || error.code === 'ACCESS_CANCELLED') { interrupted = true; throw failure('ACCESS_CANCELLED', 'Access 解析を中止しました。'); }
    if (error.code === 'ACCESS_UNAVAILABLE' || error.code === 'ENOENT') throw failure('ACCESS_UNAVAILABLE', 'Microsoft Access COM を利用できません。');
    if (error.killed || error.code === 'ETIMEDOUT') { interrupted = true; throw failure('ACCESS_TIMEOUT', 'Access 解析が制限時間を超えました。'); }
    throw failure('ACCESS_EXTRACTION_FAILED', 'Access のメタデータを取得できませんでした。保護・暗号化・破損の可能性があります。');
  } finally {
    // worker 終了後にも最終 snapshot を一度読み、速いDBの完了を取り逃がさない。その後 poll と callback を停止する。
    if (progressPoller) { await readProgress(); polling = false; await progressPoller; }
    workerFinished = true;
    options.signal?.removeEventListener('abort', cancel);
    await cancellation;
    if (interrupted) await stopOwnedAccess(directory);
    await rm(directory, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 });
  }
}
export async function getAccessCapabilities(options = {}) {
  const directory = await mkdtemp(join(tmpdir(), 'access2future-capability-'));
  let timedOut = false;
  try {
    const { stdout } = await (options.runner ?? run)('powershell.exe', ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', script, '-Capabilities', '-WorkDirectory', directory], { shell: false, windowsHide: true, timeout: options.timeoutMs ?? options.timeout ?? 15000, maxBuffer: 1024 * 1024, encoding: 'utf8' });
    if (JSON.parse(stdout.trim()).available === true) return { available: true };
  } catch (error) { timedOut = Boolean(error.killed || error.code === 'ETIMEDOUT'); }
  finally { if (timedOut) await stopOwnedAccess(directory); await rm(directory, { recursive: true, force: true, maxRetries: 5, retryDelay: 200 }); }
  return { available: false, reason: 'Microsoft Access COM を利用できません。' };
}

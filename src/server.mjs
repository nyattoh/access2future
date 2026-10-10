import { createServer } from 'node:http';
import { readFileSync } from 'node:fs';
import { readFile, writeFile, mkdir, unlink } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { dirname, resolve, extname, sep } from 'node:path';
import { randomUUID } from 'node:crypto';
import { normalizeInventory, mergeInventories, analyzeSelection, recommendTargets, buildPlan, renderPlanMarkdown } from './domain.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const appVersion = JSON.parse(readFileSync(resolve(root, 'package.json'), 'utf8')).version;
const publicRoot = resolve(root, 'public');
const mimeTypes = { '.html': 'text/html; charset=utf-8', '.mjs': 'text/javascript; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.css': 'text/css; charset=utf-8', '.svg': 'image/svg+xml', '.png': 'image/png', '.ico': 'image/x-icon', '.txt': 'text/plain; charset=utf-8' };

class HttpError extends Error {
  constructor(status, code, message) { super(message); this.status = status; this.code = code; }
}

function reply(response, status, data) {
  response.writeHead(status, { 'Content-Type': 'application/json; charset=utf-8' });
  response.end(JSON.stringify(data));
}

async function readBody(request, maximum) {
  const declared = Number(request.headers['content-length']);
  if (Number.isFinite(declared) && declared > maximum) {
    request.resume();
    throw new HttpError(413, 'INPUT_TOO_LARGE', 'ファイルまたは入力データがサイズ上限を超えています。');
  }
  const chunks = [];
  let size = 0;
  for await (const chunk of request) {
    size += chunk.length;
    if (size > maximum) {
      request.resume();
      throw new HttpError(413, 'INPUT_TOO_LARGE', 'ファイルまたは入力データがサイズ上限を超えています。');
    }
    chunks.push(chunk);
  }
  return Buffer.concat(chunks);
}

async function readJson(request, maximum) {
  if (!String(request.headers['content-type'] ?? '').startsWith('application/json')) {
    throw new HttpError(415, 'EXPECTED_JSON', 'JSON形式の入力を使用してください。');
  }
  const buffer = await readBody(request, maximum);
  try { return JSON.parse(buffer.toString('utf8')); }
  catch { throw new HttpError(400, 'INVALID_JSON', 'JSONファイルを読み取れません。形式を確認してください。'); }
}

function checkedInventory(input) {
  try { return normalizeInventory(input); }
  catch { throw new HttpError(400, 'INVALID_INVENTORY', '解析資料の版、資産ID、項目の型を確認してください。'); }
}

function accessFileName(header) {
  let name;
  try { name = decodeURIComponent(String(header ?? '')); }
  catch { throw new HttpError(400, 'INVALID_FILE_NAME', 'ファイル名を読み取れません。'); }
  if (!name || name.length > 180 || /[\\/\x00-\x1f]/u.test(name) || !['.mdb', '.accdb'].includes(extname(name).toLowerCase())) {
    throw new HttpError(400, 'UNSUPPORTED_FILE', '対応する .mdb または .accdb ファイルを選択してください。');
  }
  return name;
}

function checkLocalRequest(request, server) {
  const port = server.address()?.port;
  const hosts = new Set([`127.0.0.1:${port}`, `localhost:${port}`, `[::1]:${port}`]);
  if (!hosts.has(request.headers.host)) {
    throw new HttpError(403, 'LOCAL_REQUEST_ONLY', 'このアプリはローカル接続専用です。');
  }
  const origin = request.headers.origin;
  if (origin && !new Set([...hosts].map((host) => `http://${host}`)).has(origin)) {
    throw new HttpError(403, 'FOREIGN_ORIGIN', '別のサイトからの要求は受け付けません。');
  }
}

function responseHeaders(response) {
  response.setHeader('Cache-Control', 'no-store');
  response.setHeader('X-Content-Type-Options', 'nosniff');
  response.setHeader('Referrer-Policy', 'no-referrer');
  response.setHeader('Content-Security-Policy', "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data: blob:; font-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'");
}

const IMPORT_TIMEOUT_MS = 15 * 60_000; // importAccessへ渡す解析上限。ACCESS_TIMEOUTの文言と揃える

export function createAppServer(options = {}) {
  const tempDir = resolve(options.tempDir ?? resolve(root, '.local', 'uploads'));
  const maxFileBytes = options.maxFileBytes ?? 64 * 1024 * 1024;
  const maxJsonBytes = options.maxJsonBytes ?? 8 * 1024 * 1024;
  const importer = options.importer ?? (async (...args) => (await import('./access-import.mjs')).importAccess(...args));
  const capabilities = options.capabilities ?? (async () => (await import('./access-import.mjs')).getAccessCapabilities());
  let activeImport = false;
  let importJob = null;
  let lastImportProgress = null;
  let capabilityPromise;
  let capabilityCheckedAt = 0;

  async function accessState() {
    if (!capabilityPromise || Date.now() - capabilityCheckedAt > 60_000) {
      capabilityCheckedAt = Date.now();
      capabilityPromise = Promise.resolve().then(capabilities).catch(() => ({
        available: false, reason: 'Accessの解析環境を確認できません。JSON解析資料または合成サンプルを使用できます。',
      }));
    }
    return capabilityPromise;
  }

  const server = createServer(async (request, response) => {
    responseHeaders(response);
    try {
      checkLocalRequest(request, server);
      let pathname;
      try { pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname); }
      catch { throw new HttpError(400, 'INVALID_PATH', '要求されたパスを読み取れません。'); }
      if (pathname.includes('\\') || pathname.split('/').includes('..') || pathname.includes('\0')) {
        throw new HttpError(400, 'INVALID_PATH', 'このパスにはアクセスできません。');
      }

      if (request.method === 'GET' && pathname === '/api/health') {
        const access = await accessState();
        reply(response, 200, { version: appVersion, mode: 'local', access, maxFileBytes });
        return;
      }

      if (request.method === 'GET' && pathname === '/api/demo') {
        const demo = options.demoInventory ?? JSON.parse(await readFile(resolve(root, 'samples', 'demo.inventory.json'), 'utf8'));
        const normalized = checkedInventory(demo);
        reply(response, 200, { ...normalized, source: { ...normalized.source, kind: 'synthetic' } });
        return;
      }

      if (request.method === 'POST' && pathname === '/api/inventory') {
        const normalized = checkedInventory(await readJson(request, maxJsonBytes));
        reply(response, 200, { ...normalized, source: { ...normalized.source, kind: 'inventory' } });
        return;
      }

      if (request.method === 'POST' && pathname === '/api/merge') {
        const body = await readJson(request, maxJsonBytes);
        if (!Array.isArray(body?.inventories) || body.inventories.length < 2 || body.inventories.length > 8) {
          throw new HttpError(400, 'INVALID_INVENTORY_SET', '統合する解析資料を2〜8件指定してください。');
        }
        let merged;
        try { merged = mergeInventories(body.inventories); }
        catch { throw new HttpError(400, 'INVALID_INVENTORY_SET', '統合する資料の版と資産・リンク情報を確認してください。'); }
        reply(response, 200, merged);
        return;
      }

      if (request.method === 'POST' && pathname === '/api/analyze') {
        const body = await readJson(request, maxJsonBytes);
        if (!Array.isArray(body?.selectedIds) || !body.selectedIds.length) {
          throw new HttpError(400, 'NO_SELECTION', 'フォーム・ページ・帳票を1件以上選択してください。');
        }
        const inventory = checkedInventory(body.inventory);
        let analysis;
        let targets;
        try {
          analysis = analyzeSelection(inventory, body.selectedIds);
          targets = recommendTargets(inventory, body.usage ?? {}, analysis);
        } catch {
          throw new HttpError(400, 'INVALID_SELECTION', '選択対象と利用形態を確認してください。');
        }
        reply(response, 200, { analysis, targets });
        return;
      }

      if (request.method === 'POST' && pathname === '/api/plan') {
        const body = await readJson(request, maxJsonBytes);
        if (!body || !Array.isArray(body.selectedIds) || !body.selectedIds.length) {
          throw new HttpError(400, 'NO_SELECTION', '移行したいフォーム・ページ・帳票を一つ以上選択してください。');
        }
        const inventory = checkedInventory(body.inventory);
        let plan;
        try { plan = buildPlan(inventory, { selectedIds: body.selectedIds, usage: body.usage, targetId: body.targetId, notes: body.notes }); }
        catch { throw new HttpError(400, 'INVALID_PLAN_INPUT', '選択対象、利用形態、移行先の入力を確認してください。'); }
        reply(response, 200, { plan, markdown: renderPlanMarkdown(plan) });
        return;
      }

      if (request.method === 'GET' && pathname === '/api/import/progress') {
        const requestId = new URL(request.url, 'http://localhost').searchParams.get('requestId');
        if (!/^[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}$/i.test(requestId ?? '')) throw new HttpError(400, 'INVALID_IMPORT_ID', '解析の識別情報を確認してください。');
        const snapshot = importJob?.requestId === requestId ? importJob.snapshot : lastImportProgress?.requestId === requestId ? lastImportProgress : null;
        reply(response, 200, snapshot ?? { requestId, state: 'waiting', phase: 'preparing', completed: 0, total: null });
        return;
      }

      if (request.method === 'POST' && pathname === '/api/import/cancel') {
        const job = importJob;
        if (job) {
          job.controller.abort();
          const error = await job.done;
          if (error && error.code !== 'ACCESS_CANCELLED') throw new HttpError(500, 'CANCEL_FAILED', '解析の中止と一時ファイルの削除を確認できませんでした。解析環境を確認してください。');
        }
        reply(response, 200, { cancelled: !!job });
        return;
      }

      if (request.method === 'POST' && pathname === '/api/import') {
        if (!String(request.headers['content-type'] ?? '').startsWith('application/octet-stream')) {
          throw new HttpError(415, 'EXPECTED_ACCESS_FILE', 'ファイル本体を送信してください。ローカルパスの指定は受け付けません。');
        }
        const name = accessFileName(request.headers['x-file-name']);
        const requestId = request.headers['x-import-id'] ?? randomUUID();
        if (typeof requestId !== 'string' || !/^[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}$/i.test(requestId)) throw new HttpError(400, 'INVALID_IMPORT_ID', '解析の識別情報を確認してください。');
        if (activeImport) throw new HttpError(409, 'IMPORT_BUSY', '別のファイルを解析しています。完了してから再度お試しください。');
        const buffer = await readBody(request, maxFileBytes);
        const signature = buffer.subarray(4, 32).toString('ascii');
        if (!signature.startsWith('Standard Jet DB') && !signature.startsWith('Standard ACE DB')) {
          throw new HttpError(400, 'INVALID_ACCESS_FILE', 'Accessのデータベースとして確認できません。ファイル形式を確認してください。');
        }
        if (activeImport) throw new HttpError(409, 'IMPORT_BUSY', '別のファイルを解析しています。完了してから再度お試しください。');
        const uploadPath = resolve(tempDir, `${randomUUID()}${extname(name).toLowerCase()}`);
        activeImport = true;
        const controller = new AbortController();
        let finish;
        const done = new Promise(resolve => { finish = resolve; });
        const job = { controller, done, requestId, snapshot: { requestId, state: 'running', phase: 'preparing', completed: 0, total: null } };
        importJob = job;
        const onProgress = snapshot => {
          if (importJob !== job || controller.signal.aborted || !snapshot) return;
          const { phase, completed, total } = snapshot;
          const phases = ['preparing', 'enumerating', 'analysing', 'finishing'];
          if (!phases.includes(phase) || !Number.isSafeInteger(completed) || completed < 0 || (total !== null && (!Number.isSafeInteger(total) || total < completed)) || (total === null && completed !== 0)) return;
          const previous = job.snapshot;
          if (completed < previous.completed || phases.indexOf(phase) < phases.indexOf(previous.phase) || (previous.total !== null && total !== previous.total)) return;
          job.snapshot = { requestId, state: 'running', phase, completed, total };
        };
        const disconnected = () => { if (!response.writableEnded) controller.abort(); };
        response.once('close', disconnected);
        if (response.destroyed) controller.abort();
        let imported;
        let jobError;
        try {
          await mkdir(tempDir, { recursive: true });
          await writeFile(uploadPath, buffer, { flag: 'wx', mode: 0o600 });
          if (controller.signal.aborted) throw Object.assign(new Error('Cancelled'), { code: 'ACCESS_CANCELLED' });
          imported = checkedInventory(await importer(uploadPath, { originalName: name, timeoutMs: IMPORT_TIMEOUT_MS, signal: controller.signal, onProgress }));
          if (controller.signal.aborted) throw Object.assign(new Error('Cancelled'), { code: 'ACCESS_CANCELLED' });
          onProgress({ phase: 'finishing', completed: imported.assets.length, total: imported.assets.length });
        } catch (error) {
          jobError = error;
          throw error;
        } finally {
          try {
            await unlink(uploadPath).catch((error) => {
              if (error.code !== 'ENOENT') throw new HttpError(500, 'UPLOAD_CLEANUP_FAILED', '一時アップロードの削除を完了できませんでした。ローカルの解析フォルダーを確認してください。');
            });
          } catch (error) {
            jobError = error;
            throw error;
          } finally {
            response.removeListener('close', disconnected);
            lastImportProgress = { ...job.snapshot, state: controller.signal.aborted ? 'cancelled' : jobError ? 'failed' : 'completed' };
            activeImport = false; importJob = null; finish(jobError);
          }
        }
        reply(response, 200, { ...imported, source: { ...imported.source, name, kind: 'access' } });
        return;
      }

      if (request.method !== 'GET' && request.method !== 'HEAD') {
        throw new HttpError(405, 'METHOD_NOT_ALLOWED', 'この操作は受け付けません。');
      }
      if (pathname.startsWith('/api/')) throw new HttpError(404, 'NOT_FOUND', '要求された操作が見つかりません。');
      const filePath = pathname === '/modules/domain.mjs' ? resolve(root, 'src', 'domain.mjs') : resolve(publicRoot, `.${pathname === '/' ? '/index.html' : pathname}`);
      if (pathname !== '/modules/domain.mjs' && !filePath.startsWith(`${publicRoot}${sep}`)) {
        throw new HttpError(400, 'INVALID_PATH', 'このパスにはアクセスできません。');
      }
      const type = mimeTypes[extname(filePath)];
      if (!type) throw new HttpError(404, 'NOT_FOUND', '要求されたファイルが見つかりません。');
      let content;
      try { content = await readFile(filePath); }
      catch { throw new HttpError(404, 'NOT_FOUND', '要求されたファイルが見つかりません。'); }
      response.writeHead(200, { 'Content-Type': type });
      response.end(request.method === 'HEAD' ? undefined : content);
    } catch (error) {
      if (response.destroyed) return;
      if (response.headersSent) { response.end(); return; }
      const known = error instanceof HttpError;
      const importErrors = {
        ACCESS_CANCELLED: [499, 'Accessの解析を中止しました。ファイルを選び直して解析できます。'],
        ACCESS_UNAVAILABLE: [503, 'このPCでAccessの解析環境を利用できません。JSON解析資料または合成サンプルを使用してください。'],
        ACCESS_TIMEOUT: [504, 'Accessの解析が15分の制限時間内に完了しませんでした。解析結果は作成されていません。ファイルは残るので、確認済みの件数を参考に再試行するか、JSON解析資料を使用してください。'],
        UNSUPPORTED_FORMAT: [400, 'このAccessファイル形式には対応していません。'],
        INVALID_ACCESS_FILE: [400, 'Accessファイルを読み取れません。形式や保護の状態を確認してください。'],
      };
      const mapped = importErrors[error.code];
      const status = known ? error.status : mapped?.[0] ?? 500;
      const message = known ? error.message : mapped?.[1] ?? '処理を完了できませんでした。取得できる範囲や解析環境を確認してください。';
      reply(response, status, { error: { code: known ? error.code : mapped ? error.code : 'PROCESSING_FAILED', message } });
    }
  });
  server.requestTimeout = 180_000;
  server.headersTimeout = 15_000;
  return server;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const port = Number(process.env.PORT ?? 7331);
  if (!Number.isInteger(port) || port < 1 || port > 65535) throw new Error('PORTには1〜65535の番号を指定してください。');
  const server = createAppServer();
  server.on('error', () => { console.error('サーバーを開始できませんでした。ポート番号を確認してください。'); process.exitCode = 1; });
  server.listen(port, '127.0.0.1', () => console.log(`Access2Future: http://127.0.0.1:${port}`));
}

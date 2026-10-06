import test from 'node:test';
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { fileURLToPath } from 'node:url';

const run = promisify(execFile);

test('PowerShell依存抽出: 外部IN句の誤結合回避と未解決参照の保持（合成入力）', async () => {
  const { stdout } = await run('powershell.exe', ['-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', fileURLToPath(new URL('./dependencies.ps1', import.meta.url))], { windowsHide: true, timeout: 60000, encoding: 'utf8', maxBuffer: 1024 * 1024 });
  assert.equal(stdout.includes('FAIL'), false, stdout);
  assert.match(stdout, /ALL PASS/);
});

// Run in the local page. These native responses are synthetic protocol fixtures.
async function measuredProgressRegression() {
  const realFetch = window.fetch.bind(window);
  const demo = await (await realFetch('/api/demo')).json();
  const button = name => [...document.querySelectorAll('button')].find(el => el.textContent.trim() === name);
  const check = (condition, message) => { if (!condition) throw new Error(message); };
  const until = async (test, message) => {
    const end = Date.now() + 7000;
    while (Date.now() < end) { if (test()) return; await new Promise(resolve => setTimeout(resolve, 30)); }
    throw new Error(message);
  };
  let requestId;
  let resolveImport;
  let resolveCancel;
  let snapshot = { phase: 'analysing', completed: 2, total: 10 };
  let wrongId = false;
  let failImport = false; // synthetic 504 + server-side failed snapshot
  const originalText = File.prototype.text;
  let resolveText;
  File.prototype.text = function () {
    if (this.name === 'synthetic-slow.json') return new Promise(resolve => { resolveText = () => resolve(JSON.stringify(demo)); });
    return originalText.call(this);
  };
  window.fetch = async (url, init = {}) => {
    const path = String(url);
    if (path === '/api/import') {
      requestId = init.headers['X-Import-Id'];
      if (failImport) return new Response(JSON.stringify({ error: { code: 'ACCESS_TIMEOUT', message: 'Accessの解析が15分の制限時間内に完了しませんでした。' } }), { status: 504 });
      return new Promise(resolve => { resolveImport = () => resolve(new Response(JSON.stringify(demo), { status: 200 })); });
    }
    if (path.startsWith('/api/import/progress?')) {
      const id = new URL(path, location.href).searchParams.get('requestId');
      return new Response(JSON.stringify({ requestId: wrongId ? crypto.randomUUID() : id, state: failImport ? 'failed' : 'running', ...snapshot }), { status: 200 });
    }
    if (path === '/api/import/cancel') return new Promise(resolve => { resolveCancel = () => resolve(new Response(JSON.stringify({ cancelled: true }), { status: 200 })); });
    return realFetch(url, init);
  };
  try {
    const slowTransfer = new DataTransfer(); slowTransfer.items.add(new File([JSON.stringify(demo)], 'synthetic-slow.json'));
    const slowInput = document.getElementById('file-front'); slowInput.files = slowTransfer.files; slowInput.dispatchEvent(new Event('change', { bubbles: true }));
    button('解析開始').click(); await until(() => !!resolveText, 'Deferred JSON read not reached');
    button('解析を中止').click(); await until(() => !!resolveCancel, 'Deferred JSON cancellation not requested');
    resolveCancel(); await until(() => !button('解析開始').disabled, 'JSON cancellation did not complete');
    resolveCancel = null;
    const bytes = new Uint8Array(256); bytes.set(new TextEncoder().encode('Standard ACE DB'), 4);
    const transfer = new DataTransfer(); transfer.items.add(new File([bytes], 'synthetic-progress.accdb'));
    const input = document.getElementById('file-front'); input.files = transfer.files; input.dispatchEvent(new Event('change', { bubbles: true }));
    button('解析開始').click();
    await until(() => !!resolveImport, 'Native analysis request not started');
    check(/^[a-f0-9-]{36}$/.test(requestId ?? ''), 'Analysis request has no unique progress ID');
    await until(() => document.querySelector('#main progress.measured-progress')?.value === 2, 'Measured progress was not rendered');
    const bar = () => document.querySelector('#main progress.measured-progress');
    check(bar().max === 10, 'Wrong measured denominator');
    resolveText(); await new Promise(resolve => setTimeout(resolve, 600));
    check(bar()?.max === 10 && bar()?.value === 2, 'Late cancelled JSON read overwrote the current progress');
    check(document.getElementById('processing-status').textContent.includes('20%'), 'Measured percentage not displayed');
    snapshot = { phase: 'analysing', completed: 6, total: 10 };
    await until(() => bar()?.value === 6, 'Measured progress did not update');
    check(document.getElementById('processing-status').textContent.includes('60%'), 'Updated measured percentage missing');
    wrongId = true; snapshot = { phase: 'analysing', completed: 9, total: 10 };
    await new Promise(resolve => setTimeout(resolve, 600));
    check(bar().value === 6, 'Different file progress overwrote the current file');
    wrongId = false; snapshot = { phase: 'analysing', completed: 3, total: 10 };
    await new Promise(resolve => setTimeout(resolve, 600));
    check(bar().value === 6, 'Measured progress went backwards');
    button('解析を中止').click(); await until(() => !!resolveCancel, 'Cancel not requested');
    check(bar()?.value === 6, 'Cancellation replaced measured progress with a fabricated value');
    resolveCancel(); await until(() => !bar(), 'Progress remained after cancellation');
    resolveImport(); await new Promise(resolve => setTimeout(resolve, 100));
    check(button('選択へ進む').disabled, 'Late cancelled result allowed continuation');
    // Timeout: the last measured count/percent/phase stay near the error and the file stays retryable.
    const alertText = () => document.querySelector('#main [role=alert]')?.textContent ?? '';
    const failedText = () => document.getElementById('failed-progress')?.textContent ?? '';
    snapshot = { phase: 'analysing', completed: 74, total: 128 }; failImport = true;
    button('解析開始').click();
    await until(() => failedText() !== '', 'Failure did not keep the measured progress');
    check(alertText().includes('15分') && !/180|3分/.test(alertText()), 'Timeout error does not say 15 minutes');
    check(failedText().includes('74 / 128件（57%）') && failedText().includes('資産の確認'), 'Last measured count, percent and phase missing near the error');
    check(!button('解析開始').disabled && !bar(), 'Selected file was not kept for retry after failure');
    snapshot = { phase: 'enumerating', completed: 0, total: null };
    button('解析開始').click();
    await until(() => failedText().includes('総数を数えている段階で停止'), 'Unknown-total failure not described');
    check(!failedText().includes('%') && !failedText().includes('/'), 'A ratio was invented before the total was known');
    check(!button('解析開始').disabled, 'Retry unavailable after unknown-total failure');
    failImport = false;
    return { result: 'passed', checks: ['timeout-15min-text', 'failed-keeps-last-count-percent-phase', 'unknown-total-no-ratio', 'file-kept-for-retry','measured-counts-and-percent', 'late-json-read-discarded', 'updates-from-server-only', 'request-isolation', 'monotonic-progress', 'cancellation-keeps-real-counts', 'late-result-discarded'], source: 'synthetic_protocol_only' };
  } finally {
    window.fetch = realFetch;
    File.prototype.text = originalText;
    resolveText?.();
    resolveCancel?.(); resolveImport?.();
  }
}

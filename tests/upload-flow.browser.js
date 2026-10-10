async function uploadFlowRegression() {
  const originalFetch = window.fetch.bind(window);
  const demo = await (await originalFetch('/api/demo')).json();
  const accessAvailable = (await (await originalFetch('/api/health')).json()).access?.available === true;
  const requests = [];
  let mode = 'normal';
  let finalSnapshotReady = false;
  let releaseLate;
  let releaseLateMerge;
  let releaseCancel;
  let releaseNativeImport;
  let nativeImportHeaders;
  let cancelImportHeaders;
  const results = [];
  const pause = () => new Promise(resolve => setTimeout(resolve, 20));
  const check = (condition, message) => { if (!condition) throw new Error(message); };
  const until = async (test, message) => {
    const deadline = Date.now() + 7000;
    while (Date.now() < deadline) { if (test()) return; await pause(); }
    throw new Error(message);
  };
  const button = name => [...document.querySelectorAll('button')].find(el => el.textContent.trim() === name);
  const choose = (slot, name, contents = JSON.stringify(demo), type = 'application/json') => {
    const input = document.getElementById(`file-${slot}`);
    const transfer = new DataTransfer();
    transfer.items.add(new File([contents], name, { type }));
    input.files = transfer.files;
    input.dispatchEvent(new Event('change', { bubbles: true }));
  };
  const click = name => { const el = button(name); check(el && !el.disabled, `Button unavailable: ${name}`); el.click(); };
  const nextReady = () => !!button('選択へ進む') && !button('選択へ進む').disabled;
  const startReady = () => !!button('解析開始') && !button('解析開始').disabled;
  const clear = () => { for (const name of ['ファイル1を削除', 'ファイル2を削除']) if (button(name)) click(name); };
  window.fetch = async (url, init = {}) => {
    const path = String(url);
    if (/\/api\/(inventory|import|merge)(?:\/cancel)?$/.test(path)) requests.push(path);
    if (path.startsWith('/api/import/progress?')) {
      const requestId = new URL(path, location.href).searchParams.get('requestId');
      if (mode === 'cleanup-failure') return new Response(JSON.stringify({ requestId, state: finalSnapshotReady ? 'failed' : 'running', phase: finalSnapshotReady ? 'finishing' : 'analysing', completed: finalSnapshotReady ? 8 : 4, total: 10 }), { status: 200 });
      return new Response(JSON.stringify({ requestId, state: 'running', phase: 'preparing', completed: 0, total: null }), { status: 200 });
    }
    if (path.endsWith('/api/import/cancel')) return new Promise(resolve => {
      cancelImportHeaders = init.headers;
      releaseCancel = () => {
        if (mode === 'cleanup-failure') {
          finalSnapshotReady = true;
          resolve(new Response(JSON.stringify({ error: { message: '一時ファイルを削除できませんでした' } }), { status: 500 }));
        } else {
          resolve(new Response(JSON.stringify({ cancelled: mode === 'native' }), { status: 200 }));
        }
      };
    });
    if (path.endsWith('/api/import') && ['native', 'cleanup-failure'].includes(mode)) return new Promise((resolve, reject) => {
      nativeImportHeaders = init.headers;
      releaseNativeImport = () => reject(new DOMException('Aborted', 'AbortError'));
      init.signal?.addEventListener('abort', releaseNativeImport, { once: true });
    });
    if (path.endsWith('/api/merge') && mode === 'late-merge') return new Promise(resolve => {
      releaseLateMerge = () => resolve(new Response(JSON.stringify(demo), { status: 200 }));
    });
    if (path.endsWith('/api/inventory') && mode === 'ignores-abort') {
      return new Promise(resolve => { releaseLate = () => resolve(new Response(JSON.stringify({ ...demo, source: { ...demo.source, kind: 'inventory' } }), { status: 200 })); });
    }
    if (path.endsWith('/api/inventory') && mode === 'failure') return new Response(JSON.stringify({ error: { message: '合成試験の解析失敗' } }), { status: 500 });
    return originalFetch(url, init);
  };
  try {
    const atStart = requests.length;
    choose('front', 'synthetic-one.json');
    await new Promise(resolve => setTimeout(resolve, 100));
    check(requests.length === atStart, 'Selecting a file started analysis automatically');
    check(startReady(), 'Explicit analysis start is unavailable');
    check(!nextReady(), 'Unanalysed file allowed continuation');
    choose('front', 'synthetic-replaced.json');
    check(document.body.innerText.includes('synthetic-replaced.json'), 'Replacement name is not visible');
    check(document.querySelector('label[for=file-front]').textContent.includes('選び直す'), 'Replacement action is unclear');
    click('ファイル1を削除');
    check(!startReady(), 'Empty selection allowed analysis');
    check(requests.length === atStart, 'Replacement or deletion sent analysis requests');
    results.push('choose-replace-delete-without-analysis');

    for (const slot of ['front', 'back']) {
      choose(slot, `synthetic-${slot}.json`);
      click('解析開始');
      await until(nextReady, `Single ${slot} file did not finish`);
      check(!document.querySelector('#main [role=progressbar]'), 'Progress bar remained after completion');
      click('選択へ進む');
      await until(() => document.querySelectorAll('input[type=checkbox]').length === 4, 'Form, page and report selection not available');
      const firstCheckbox = document.querySelector('input[type=checkbox]');
      firstCheckbox.focus();
      firstCheckbox.click();
      check(document.activeElement?.matches('input[type=checkbox]'), 'Selection change dropped keyboard focus');
      await until(() => document.body.innerText.includes('帳票: 請求書'), 'Impact candidates did not finish rendering');
      check(document.body.innerText.includes('未選択の機能への影響'), 'Impact heading omits unselected reports and other features');
      check(document.body.innerText.includes('帳票: 請求書'), 'Unselected report is missing from the impact candidates');
      click('戻る');
      await until(() => !!document.getElementById('file-front'), 'Did not return to files');
      choose(slot, `synthetic-${slot}-changed.json`);
      check(!nextReady(), 'Changed file kept an obsolete analysis');
      clear();
      results.push(`single-${slot}-and-invalidation`);
    }

    const bothBefore = requests.length;
    choose('front', 'synthetic-first.json'); choose('back', 'synthetic-second.json');
    check(requests.length === bothBefore, 'Selecting two files started analysis');
    click('解析開始'); await until(nextReady, 'Two-file analysis did not finish');
    check(requests.slice(bothBefore).filter(path => path.endsWith('/api/inventory')).length === 2, 'Both files were not analysed exactly once');
    check(requests.slice(bothBefore).filter(path => path.endsWith('/api/merge')).length === 1, 'Two results were not merged exactly once');
    clear(); results.push('two-files-one-explicit-start');

    choose('front', 'synthetic-cancel.json'); mode = 'ignores-abort';
    const beforeCancel = requests.length;
    click('解析開始'); await until(() => !!releaseLate, 'Delayed analysis not reached');
    const progress = document.querySelector('#main [role=progressbar]');
    check(progress && progress.getBoundingClientRect().height > 0, 'Analysis progress bar is missing');
    check(progress.max === demo.assets.length && progress.value === 0, 'JSON validation progress was not based on its actual asset count');
    const mutable = [...document.querySelectorAll('input, button')].filter(el => el.tagName === 'INPUT' || !['解析を中止'].includes(el.textContent.trim()));
    check(mutable.every(el => el.disabled), 'Busy input or button was enabled');
    check(!document.querySelector('#steps a'), 'Busy workflow navigation was still active');
    click('解析を中止'); await until(() => !!releaseCancel, 'Cancellation request not reached');
    check(!!document.querySelector('#main [role=progressbar]'), 'Progress disappeared before cancellation finished');
    releaseCancel(); await until(startReady, 'Cancellation did not restore file selection');
    check(!document.querySelector('#main [role=progressbar]'), 'Progress remained after cancellation');
    check(!document.querySelector('[role=alert]'), 'No active native job was reported as a cancellation failure');
    check(document.body.innerText.includes('解析を中止しました'), 'Aborted JSON analysis was incorrectly reported as already ended');
    check(!nextReady(), 'Cancelled analysis allowed continuation');
    check(document.body.innerText.includes('synthetic-cancel.json'), 'Cancellation discarded selected file');
    check(requests.slice(beforeCancel).includes('/api/import/cancel'), 'Native cancellation was not requested');
    mode = 'normal'; click('解析開始'); await until(nextReady, 'Retry after cancellation did not finish');
    releaseLate(); await new Promise(resolve => setTimeout(resolve, 100));
    check(nextReady(), 'Late cancelled result overwrote retry state');
    clear(); results.push('busy-cancel-retry-and-late-result');

    releaseCancel = undefined; releaseLateMerge = undefined;
    choose('front', 'synthetic-merge-first.json'); choose('back', 'synthetic-merge-second.json');
    mode = 'late-merge'; click('解析開始');
    await until(() => !!releaseLateMerge, 'Delayed merge request not reached');
    click('解析を中止'); await until(() => !!releaseCancel, 'Merge cancellation request not reached');
    releaseCancel(); await until(startReady, 'Merge cancellation did not restore file selection');
    check(document.body.innerText.includes('解析を中止しました'), 'Aborted merge was incorrectly reported as already ended');
    releaseLateMerge(); await new Promise(resolve => setTimeout(resolve, 100));
    check(startReady(), 'Late merge result overwrote the cancellation state');
    clear(); mode = 'normal'; results.push('merge-cancel-reports-local-cancellation');

    if (accessAvailable) {
      const bytes = new Uint8Array(128);
      bytes.set(new TextEncoder().encode('Standard Jet DB\0'), 4);
      choose('front', 'synthetic-cancel.accdb', bytes, 'application/octet-stream');
      mode = 'native';
      click('解析開始'); await until(() => !!releaseNativeImport, 'Synthetic Access request was not intercepted');
      const importId = new Headers(nativeImportHeaders).get('X-Import-Id');
      check(/^[a-f0-9-]{36}$/i.test(importId ?? ''), 'Synthetic Access request did not carry its import ID');
      click('解析を中止'); await until(() => !!releaseCancel, 'Access cancellation request not reached');
      check(new Headers(cancelImportHeaders).get('X-Import-Id') === importId, 'Cancellation was not tied to the active import ID');
      releaseCancel(); await until(startReady, 'Access cancellation did not restore file selection');
      check(document.body.innerText.includes('解析を中止しました'), 'Matching active import was not reported as cancelled');
      clear(); mode = 'normal'; results.push('synthetic-access-cancel-carries-import-id');

      releaseNativeImport = undefined; releaseCancel = undefined; nativeImportHeaders = undefined;
      finalSnapshotReady = false;
      choose('front', 'synthetic-cleanup-failure.accdb', bytes, 'application/octet-stream');
      mode = 'cleanup-failure';
      click('解析開始'); await until(() => !!releaseNativeImport, 'Cleanup-failure Access request was not intercepted');
      click('解析を中止'); await until(() => !!releaseCancel, 'Cleanup-failure cancellation request not reached');
      releaseCancel();
      await until(() => !!document.querySelector('#failed-progress'), 'Last verified progress was not shown after cleanup failure');
      check(document.querySelector('#failed-progress').textContent.includes('8 / 10件（80%）'), 'Cleanup failure did not retain the final measured asset count');
      check(document.querySelector('#failed-progress').textContent.includes('後処理'), 'Cleanup failure did not identify the finishing phase');
      check(document.querySelector('[role=alert]')?.textContent.includes('一時ファイルを削除できませんでした'), 'Cleanup failure message was lost');
      clear(); mode = 'normal'; results.push('cleanup-failure-retains-final-progress');
    } else {
      results.push('synthetic-access-cancel-not-run-access-unavailable');
    }

    choose('back', 'synthetic-retry.json'); mode = 'failure';
    click('解析開始'); await until(() => !!document.querySelector('[role=alert]'), 'Analysis error not shown');
    check(!document.querySelector('#main [role=progressbar]'), 'Progress remained after failure');
    check(startReady() && !nextReady(), 'Failed analysis did not allow retry or exposed stale results');
    check(document.body.innerText.includes('synthetic-retry.json'), 'Failure discarded selected file');
    mode = 'normal'; click('解析開始'); await until(nextReady, 'Retry did not finish');
    clear(); results.push('error-preserves-files-and-retry');
    check(/どちら|1つ|一つ/.test(document.body.innerText), 'Single-file explanation missing');
    check(!document.body.innerText.includes('フロントエンド'), 'Initial copy requires frontend terminology');

    choose('front', 'synthetic-plan.json'); click('解析開始'); await until(nextReady, 'Plan diagram source inventory did not load');
    click('選択へ進む'); await until(() => document.querySelectorAll('#list input[type=checkbox]').length > 0, 'Plan selection list did not load');
    document.querySelector('#list input[type=checkbox]').click();
    click('利用形態へ'); await until(() => !!button('影響を確認') && !button('影響を確認').disabled, 'Usage step did not become ready'); click('影響を確認');
    await until(() => document.querySelector('input[name=target]'), 'Plan target unavailable'); const target = document.querySelector('input[name=target]'); target.click();
    click('計画を作成'); await until(() => document.querySelector('[data-diagram="flow"] img')?.naturalWidth > 0 && document.querySelector('[data-diagram="er"] img')?.naturalWidth > 0, 'Flow and ER diagrams were not rendered in the plan screen');
    check(document.querySelector('[data-diagram="flow"] img').alt === '業務フロー候補図', 'Flow diagram lacks accessible image text');
    check(document.querySelector('[data-diagram="er"] img').alt === 'ER図', 'ER diagram lacks accessible image text');
    results.push('plan-renders-accessible-flow-and-er-diagrams');

    return { result: 'passed', checks: results, source: 'synthetic_only' };
  } finally {
    window.fetch = originalFetch;
    if (releaseLate) releaseLate();
    if (releaseCancel) releaseCancel();
  }
}

import { renderDiagram } from './plan-diagrams.mjs';

globalThis.mermaid?.initialize({
  startOnLoad: false,
  securityLevel: 'strict',
  suppressErrorRendering: true,
  maxTextSize: 30000,
  maxEdges: 1000,
  flowchart: { htmlLabels: false },
  theme: 'base',
  themeVariables: {
    primaryColor: '#f4dcdc', primaryTextColor: '#292b30', primaryBorderColor: '#b51f2b',
    lineColor: '#6b252b', secondaryColor: '#fff', tertiaryColor: '#f5f3f0', fontFamily: 'system-ui'
  }
});

const STEPS = ['データベースを解析', 'フォーム・ページ・帳票を選択', '利用形態を確認', '影響を確認', '計画を作成'];
const KIND = { table: 'テーブル', query: 'クエリ', form: 'フォーム', page: 'ページ', report: '帳票', macro: 'マクロ', module: 'モジュール', external: '外部参照' };
const STATUS = { supported: '取得済み', partial: '部分取得', unsupported: '非対応' };
const FIT = { recommended: '推薦', conditional: '条件付き', 'not-recommended': '要確認' };
const JSON_HEADERS = { 'Content-Type': 'application/json' };

const state = {
  health: undefined, // undefined=確認中 / null=接続不可
  slots: { front: null, back: null },
  inventory: null, origin: null, byId: new Map(),
  selected: new Set(), query: '', kind: 'all',
  usage: { users: null, concurrentEditing: false, location: null, offlineRequired: false, permissions: null, coexistence: 'undecided' },
  targetId: null, notes: '',
  analysis: null, targets: [], analysisToken: 0, plan: null, markdown: '',
  runToken: 0, cancelCtl: null, activeImportId: null, notice: '',
  measured: null, progressUnavailable: false, fileIndex: 0, fileTotal: 0,
  step: 1, maxStep: 1, busy: '', error: '', errorProgress: '', planError: ''
};

const $ = (id) => document.getElementById(id);
const h = (tag, props = {}, ...kids) => {
  const el = document.createElement(tag);
  for (const [key, value] of Object.entries(props)) {
    if (value == null || value === false) continue;
    if (key === 'class') el.className = value;
    else if (key.startsWith('on')) el.addEventListener(key.slice(2), value);
    else el.setAttribute(key, value === true ? '' : value);
  }
  for (const kid of kids.flat()) if (kid != null && kid !== false) el.append(kid.nodeType ? kid : document.createTextNode(String(kid)));
  return el;
};
const fill = (el, ...kids) => { el.replaceChildren(...kids.flat().filter((kid) => kid != null && kid !== false)); return el; };

const nameOf = (id) => { const asset = state.byId.get(id); return asset ? asset.caption ?? asset.name : `${id}（未取得）`; };
const nameWithKind = (id) => { const asset = state.byId.get(id); return asset ? `${KIND[asset.kind] ?? asset.kind}: ${nameOf(id)}` : nameOf(id); };
const selectable = () => state.inventory.assets.filter((asset) => asset.kind === 'form' || asset.kind === 'page' || asset.kind === 'report');
const pagesUnavailable = (inventory) => inventory?.limitations.some((item) => item.code === 'DATA_ACCESS_PAGES_UNAVAILABLE');
const PAGE_NOTE = 'ページはAccess Data Access Pages（レガシー）という古い形式です。現状は移行先への対応をしていないため、一覧に取得できても非対応のままです。選択すると計画に確認事項として残ります。';
const PAGES_UNAVAILABLE_NOTE = 'Access Data Access Pages（レガシー）の一覧を取得できませんでした。ページがないとは判断しません。ページの有無はこの資料では確認できていません。';
const countKinds = (inventory) => Object.entries(inventory.assets.reduce((acc, asset) => ({ ...acc, [asset.kind]: (acc[asset.kind] ?? 0) + 1 }), {})).map(([kind, n]) => `${KIND[kind] ?? kind} ${n}`).join('、');

function sourceLabel() {
  if (!state.inventory) return '';
  if (state.origin === 'merged') return '2つのファイルを合わせた構造資料（メタデータのみ）';
  return { synthetic: '合成サンプル', access: 'Access解析結果', inventory: 'JSON資料（自己申告）' }[state.inventory.source.kind] ?? '構造資料';
}
function sourceNote() {
  if (state.origin === 'merged') return 'ファイル1とファイル2の構造資料を、明示的なリンク情報だけで合わせています。取得できた定義の範囲の結果で、移行の再現は保証しません。';
  return {
    synthetic: 'これは合成サンプルです。実在のAccess解析結果ではなく、名称や依存関係は説明用の例です。',
    access: 'アップロードされたAccessファイルから取得できた定義に基づきます。取得できない項目は未取得・非対応として示します。',
    inventory: 'JSON資料の内容は自己申告として扱います。実Accessの解析結果としては扱いません。'
  }[state.inventory.source.kind] ?? '';
}

async function api(path, init) {
  let response;
  const headers = new Headers(init?.headers ?? {});
  const token = document.querySelector('meta[name="access2future-token"]')?.content;
  if (token) headers.set('X-A2F-Token', token);
  try { response = await fetch(path, { ...init, headers }); }
  catch (error) {
    if (error?.name === 'AbortError') throw error; // 中止は正常な流れなので、接続エラーの文言に置き換えない
    throw new Error('ローカルサーバーに接続できません。起動状態を確認して再度お試しください。');
  }
  let body = null;
  try { body = await response.json(); } catch { /* 本文なし */ }
  if (!response.ok) throw new Error(body?.error?.message ?? `処理を完了できませんでした（${response.status}）。`);
  return body;
}

// ---- 資料の取込み ----
function setInventory(inventory, origin) {
  state.inventory = inventory;
  state.origin = origin;
  state.byId = new Map(inventory ? inventory.assets.map((asset) => [asset.id, asset]) : []);
  state.selected = new Set();
  state.plan = null; state.analysis = null; state.targets = []; state.analysisToken++; state.targetId = null;
  state.maxStep = 1;
}
const chosenSlots = () => ['front', 'back'].filter((slot) => state.slots[slot]);
const needsRun = () => {
  const slots = chosenSlots();
  return slots.length > 0 && (!state.inventory || slots.some((slot) => !state.slots[slot].inv));
};
function setSlot(slot, file) {
  state.slots[slot] = file ? { file, name: file.name, inv: null } : null;
  state.error = ''; state.errorProgress = '';
  state.notice = '';
  // ファイルの置換・削除で、古い解析結果・選択・計画を使えなくする
  if (state.inventory || state.maxStep > 1) {
    setInventory(null, null);
    if (state.step !== 1) show(1);
  }
  render();
}
async function analyzeFile(file, signal) {
  const isJson = /\.json$/i.test(file.name);
  if (!isJson && state.health?.access?.available === false) throw new Error('この環境ではAccessファイルを解析できません。JSON解析資料または合成サンプルを使用してください。');
  if (isJson) {
    const body = await file.text();
    signal.throwIfAborted();
    try {
      const document = JSON.parse(body);
      if (Array.isArray(document.assets)) { state.measured = { phase: 'analysing', completed: 0, total: document.assets.length }; refreshStatus(); }
    } catch { /* The API returns the validation error. */ }
    return api('/api/inventory', { method: 'POST', headers: JSON_HEADERS, body, signal });
  }
  const requestId = crypto.randomUUID();
  state.activeImportId = requestId;
  const token = state.runToken;
  const pollingController = new AbortController();
  let polling = true;
  let timer;
  const stop = () => { polling = false; clearTimeout(timer); pollingController.abort(); };
  const absorb = async () => {
    const snapshot = await api(`/api/import/progress?requestId=${requestId}`, { signal: pollingController.signal });
    if (!polling || signal.aborted || token !== state.runToken) return;
    const phases = ['preparing', 'enumerating', 'analysing', 'finishing'];
    const { completed, total, phase } = snapshot ?? {};
    const valid = snapshot?.requestId === requestId && phases.includes(phase) && Number.isSafeInteger(completed) && completed >= 0 && (total === null ? completed === 0 : Number.isSafeInteger(total) && total >= completed);
    const previous = state.measured;
    if (valid && (!previous || (completed >= previous.completed && (previous.total === null || previous.total === total) && phases.indexOf(phase) >= phases.indexOf(previous.phase)))) {
      const changed = !previous || completed !== previous.completed || total !== previous.total || phase !== previous.phase || state.progressUnavailable;
      state.measured = { completed, total, phase }; state.progressUnavailable = false;
      if (changed) refreshStatus();
    }
  };
  const poll = async () => {
    if (!polling || signal.aborted || token !== state.runToken) return;
    try { await absorb(); }
    catch (error) {
      if (polling && !signal.aborted && token === state.runToken && error.name !== 'AbortError') { state.progressUnavailable = true; refreshStatus(); }
    }
    if (polling && !signal.aborted && token === state.runToken) timer = setTimeout(poll, 250);
  };
  signal.addEventListener('abort', stop, { once: true });
  poll();
  try {
    return await api('/api/import', { method: 'POST', headers: { 'Content-Type': 'application/octet-stream', 'X-File-Name': encodeURIComponent(file.name), 'X-Import-Id': requestId }, body: file, signal });
  } catch (error) {
    if (!signal.aborted && token === state.runToken) { clearTimeout(timer); await absorb().catch(() => {}); } // 失敗時にサーバーが保持した最後の実測値を取り込む
    throw error;
  } finally {
    stop(); signal.removeEventListener('abort', stop);
    if (state.activeImportId === requestId && !signal.aborted) state.activeImportId = null;
  }
}
async function runAnalysis() {
  if (state.busy || !needsRun()) return; // 反復クリックと、することがない再実行の防止
  const token = ++state.runToken;
  const ctl = new AbortController();
  state.cancelCtl = ctl;
  state.activeImportId = null;
  state.measured = null; state.progressUnavailable = false;
  state.error = ''; state.errorProgress = ''; state.notice = '';
  state.busy = 'ファイルを解析しています…';
  render();
  const stale = (error) => token !== state.runToken || error?.name === 'AbortError';
  let failed = false;
  const slots = chosenSlots();
  state.fileTotal = slots.length;
  for (const [index, slot] of slots.entries()) {
    const picked = state.slots[slot];
    if (picked.inv) continue;
    state.fileIndex = index + 1; state.measured = null; state.progressUnavailable = false;
    state.busy = `${picked.name} を解析しています…`;
    render();
    try {
      const inv = await analyzeFile(picked.file, ctl.signal);
      if (token !== state.runToken) return; // 中止済みの遅い成功は捨てる
      picked.inv = inv;
    }
    catch (error) {
      if (stale(error)) return; // 中止済みのため結果を捨て、状態は中止処理に任せる
      state.error = `${picked.name}: ${error.message}`; // 失敗してもFileは保持し、再試行できる
      state.errorProgress = failedProgressText(state.measured);
      failed = true;
      break;
    }
    if (token !== state.runToken) return;
  }
  if (!failed) {
    const [a, b] = [state.slots.front, state.slots.back];
    try {
      if (a?.inv && b?.inv) {
        state.busy = '2つのファイルの解析結果を合わせています…';
        state.measured = null;
        render();
        const merged = await api('/api/merge', { method: 'POST', headers: JSON_HEADERS, body: JSON.stringify({ inventories: [a.inv, b.inv] }), signal: ctl.signal });
        if (token !== state.runToken) return;
        setInventory(merged.inventory ?? merged, 'merged'); // 両方の解析成功後にのみ統合結果を確定
      } else {
        const single = a?.inv ?? b?.inv; // どちらか1つだけでも解析結果として扱う
        setInventory(single, single.source.kind);
      }
    } catch (error) {
      if (stale(error)) return;
      state.error = `2つのファイルの統合に失敗しました: ${error.message}`;
    }
  }
  state.busy = '';
  state.cancelCtl = null;
  render();
}
async function cancelAnalysis() {
  const ctl = state.cancelCtl;
  if (!ctl) return;
  const importId = state.activeImportId;
  state.cancelCtl = null; // 二重の中止要求を防ぐ
  ctl.abort();
  state.runToken++; // 中止した解析の遅い成功・失敗が、あとの状態を上書きしないようにする
  state.busy = '解析の中止を完了しています…'; // 中止処理の応答まで、進行中の扱いを維持する
  render();
  try {
    const result = await api('/api/import/cancel', { method: 'POST', headers: importId ? { 'X-Import-Id': importId } : {} });
    state.notice = !importId || result.cancelled
      ? '解析を中止しました。選んだファイルは残っています。「解析開始」でもう一度解析できます。'
      : '解析はすでに終了していました。選んだファイルは残っています。「解析開始」でもう一度解析できます。';
  } catch (error) {
    state.error = `解析の中止に失敗しました: ${error.message}`; // エラー時だけ失敗として示す
    if (importId) {
      try {
        const snapshot = await api(`/api/import/progress?requestId=${encodeURIComponent(importId)}`);
        const phases = ['preparing', 'enumerating', 'analysing', 'finishing'];
        const { completed, total, phase } = snapshot ?? {};
        const valid = snapshot?.requestId === importId && phases.includes(phase)
          && Number.isSafeInteger(completed) && completed >= 0
          && (total === null ? completed === 0 : Number.isSafeInteger(total) && total >= completed);
        const previous = state.measured;
        if (valid && (!previous || (completed >= previous.completed
          && (previous.total === null || previous.total === total)
          && phases.indexOf(phase) >= phases.indexOf(previous.phase)))) {
          state.measured = { completed, total, phase };
        }
      } catch {}
    }
    state.errorProgress = failedProgressText(state.measured);
  } finally {
    state.activeImportId = null;
  }
  for (const slot of chosenSlots()) state.slots[slot].inv = null; // 未解析状態に戻す（Fileは保持）
  setInventory(null, null);
  state.busy = '';
  render();
}
async function loadDemo() {
  if (state.busy) return;
  state.measured = null; state.progressUnavailable = false; state.fileIndex = 0; state.fileTotal = 0;
  state.error = ''; state.errorProgress = ''; state.notice = ''; state.busy = '合成サンプルを読み込んでいます…';
  render();
  try {
    const demo = await api('/api/demo');
    state.slots = { front: null, back: null };
    setInventory(demo, 'synthetic');
    state.busy = '';
    advance(2);
  } catch (error) { state.busy = ''; state.error = error.message; render(); }
}

// ---- 工程移動 ----
const hashStep = () => Number(/^#\/([1-5])$/.exec(location.hash)?.[1] ?? 1);
function go(step) {
  if (state.busy) return; // 進行中の工程移動を阻止
  step = Math.max(1, Math.min(step, state.maxStep));
  if (hashStep() === step) show(step); else location.hash = `#/${step}`;
}
function advance(step) { state.maxStep = Math.max(state.maxStep, step); go(step); }
window.addEventListener('hashchange', () => {
  if (state.busy) { history.replaceState(null, '', `#/${state.step}`); return; } // 進行中のURL操作・戻る進むを阻止
  show(hashStep());
});

function show(step) {
  step = Math.max(1, Math.min(step, state.maxStep));
  if (hashStep() !== step) { history.replaceState(null, '', `#/${step}`); }
  state.step = step;
  state.planError = '';
  if (step === 4) computeAnalysis();
  render();
  if (step === 5) loadPlan();
  $('main').focus({ preventScroll: true });
  window.scrollTo(0, 0);
}

function releaseDiagramUrls() {
  for (const container of document.querySelectorAll('.diagram-canvas[data-object-url]')) {
    URL.revokeObjectURL(container.dataset.objectUrl);
    delete container.dataset.objectUrl;
  }
}

async function computeAnalysis() {
  const token = ++state.analysisToken;
  if (!state.inventory || !state.selected.size) { state.analysis = null; state.targets = []; state.error = ''; render(); return; }
  state.analysis = null; state.targets = []; state.error = '';
  const selectedIds = [...state.selected];
  try {
    const result = await api('/api/analyze', { method: 'POST', headers: JSON_HEADERS, body: JSON.stringify({ inventory: state.inventory, selectedIds, usage: state.usage }) });
    if (token !== state.analysisToken || selectedIds.some((id) => !state.selected.has(id)) || selectedIds.length !== state.selected.size) return;
    state.analysis = result.analysis; state.targets = result.targets; state.error = '';
  } catch (error) {
    if (token !== state.analysisToken) return;
    state.analysis = null; state.targets = []; state.error = `影響を計算できません: ${error.message}`;
  }
  render();
}

async function loadPlan() {
  const token = (state.planToken = (state.planToken ?? 0) + 1);
  state.plan = null; state.planError = '';
  state.busy = '移行計画の草案を作成しています…';
  render();
  try {
    const result = await api('/api/plan', { method: 'POST', headers: JSON_HEADERS, body: JSON.stringify({ inventory: state.inventory, selectedIds: [...state.selected], usage: state.usage, targetId: state.targetId, notes: state.notes }) });
    if (token !== state.planToken) return;
    state.plan = result.plan; state.markdown = result.markdown;
  } catch (error) { if (token !== state.planToken) return; state.planError = error.message; }
  state.busy = '';
  if (state.step === 5) render();
}

// ---- 共通部品 ----
function block(title, lines, empty) {
  const id = `b-${Math.random().toString(36).slice(2, 8)}`;
  return h('section', { class: 'block', 'aria-labelledby': id }, h('h3', { id }, title),
    lines.length ? h('ul', {}, lines.map((line) => h('li', {}, line))) : h('p', { class: 'muted' }, empty));
}
const issueLine = (item) => [item.message, item.code ? h('span', { class: 'code' }, ` ${item.code}`) : null];
const percentOf = (m) => m.completed === m.total ? 100 : Math.min(99, Math.floor(m.completed / m.total * 100));
const PHASE_LABELS = { preparing: '準備', enumerating: '資産の列挙', analysing: '資産の確認', finishing: '後処理' };
const failedProgressText = (m) => !m ? '解析の準備段階で停止しました。確認済みの資産はありません。'
  : m.total > 0 ? `最後に確認できた資産：${m.completed} / ${m.total}件（${percentOf(m)}%）。停止した段階：${PHASE_LABELS[m.phase]}。`
  : m.total === 0 ? '資産は0件でした。' : '総数を数えている段階で停止しました。割合は算出できません。';
function status() {
  if (!state.busy) return h('p', { class: 'status', role: 'status', id: 'processing-status' });
  const measured = state.measured;
  const known = measured && measured.total > 0;
  const percent = known ? percentOf(measured) : null;
  const text = known
    ? `確認済み資産：${measured.completed} / ${measured.total}件（${percent}%）${measured.phase === 'finishing' ? '。資産の確認を終え、後処理をしています。' : ''}`
    : measured?.total === 0 ? '資産は0件です。' : measured?.phase === 'enumerating' ? '資産の総数を数えています。' : '解析の準備・構造資料の確認をしています。';
  return h('div', { class: 'processing-status', role: 'status', id: 'processing-status' },
    state.step === 1 && known && h('progress', { class: 'measured-progress', role: 'progressbar', 'aria-label': '資産確認の進捗', max: measured.total, value: measured.completed, 'aria-valuemin': 0, 'aria-valuemax': measured.total, 'aria-valuenow': measured.completed, 'aria-valuetext': `${measured.completed} / ${measured.total}件（${percent}%）` }),
    state.step !== 1 && h('div', { class: 'processing-bar', role: 'progressbar', 'aria-label': '処理の進行状況' }, h('span', { 'aria-hidden': 'true' })),
    h('p', { class: 'status' }, state.busy),
    state.step === 1 && h('p', { class: 'muted' }, `${state.fileTotal ? `${state.fileIndex} / ${state.fileTotal}ファイル。` : ''}${text}`),
    state.step === 1 && state.progressUnavailable && h('p', { class: 'muted' }, '進捗情報を取得できません。解析は続いています。表示は最後に確認できた件数です。'));
}
function refreshStatus() { document.getElementById('processing-status')?.replaceWith(status()); }
const alertBox = (message, retry) => message && h('div', { class: 'alert', role: 'alert' }, h('p', {}, message), retry && h('button', { type: 'button', class: 'btn', onclick: retry }, '再試行'));
const heading = (text, lead) => [h('h1', {}, text), lead && h('p', { class: 'lead' }, lead)];

// ---- 枠（ヘッダ・工程・フッタ） ----
function renderChrome() {
  const badge = $('source-badge');
  badge.hidden = !state.inventory;
  badge.textContent = sourceLabel();
  fill($('steps'), STEPS.map((label, i) => {
    const n = i + 1;
    const current = n === state.step;
    const stateText = current ? '現在' : n < state.step ? '完了' : n <= state.maxStep ? '移動可' : '未到達';
    const inner = [h('span', { class: 'num', 'aria-hidden': 'true' }, n), h('span', { class: 'label' }, label), h('small', {}, stateText)];
    const cls = `step ${current ? 'current' : n < state.step ? 'done' : n <= state.maxStep ? 'open' : 'locked'}`;
    return h('li', { class: cls }, n <= state.maxStep && !current && !state.busy ? h('a', { href: `#/${n}` }, inner) : h('span', { 'aria-current': current ? 'step' : null, 'aria-disabled': current ? null : 'true' }, inner));
  }));
  fill($('progress'),
    h('p', { class: 'progress-text' }, h('strong', {}, `${state.step} / 5`), h('span', {}, STEPS[state.step - 1])),
    h('progress', { max: 5, value: state.step, 'aria-label': `工程 ${state.step} / 5` }));
}

function renderActions() {
  const back = h('button', { type: 'button', class: 'btn', disabled: !!state.busy, onclick: () => go(state.step - 1) }, '戻る');
  const next = (label, enabled, onclick) => h('button', { type: 'button', class: 'btn primary', disabled: !enabled, onclick }, label);
  const none = !state.selected.size;
  const parts = {
    1: [null, next('選択へ進む', state.inventory && !state.busy, () => advance(2))],
    2: [back, next('利用形態へ', !none, () => advance(3))],
    3: [back, next('影響を確認', true, () => advance(4))],
    4: [back, next('計画を作成', state.targetId && state.analysis && !state.busy, () => advance(5))],
    5: [back, null]
  }[state.step];
  fill($('actions'), parts);
}

function render() {
  if (state.step !== 5 || !state.plan) releaseDiagramUrls();
  renderChrome();
  const main = $('main');
  const aside = $('aside');
  aside.hidden = state.step !== 2 || !state.inventory;
  $('layout').classList.toggle('with-aside', !aside.hidden);
  [step1, step2, step3, step4, step5][state.step - 1](main);
  renderActions();
}

// ---- 1: データベースを解析 ----
function slotView(slot) {
  const name = slot === 'front' ? 'ファイル1' : 'ファイル2';
  const title = slot === 'front' ? name : `${name}（追加したい場合）`;
  const id = `file-${slot}`;
  const data = state.slots[slot];
  const kind = data?.inv ? { access: 'Access解析結果', inventory: 'JSON資料（自己申告）', synthetic: '合成サンプル' }[data.inv.source.kind] : null;
  return h('section', { class: 'slot', 'aria-labelledby': `${id}-t` },
    h('h3', { id: `${id}-t` }, title),
    h('p', { class: 'muted' }, slot === 'front'
      ? '移行したいAccessファイル、またはJSON解析資料を選びます。'
      : 'もう1つファイルがあれば追加できます。どちらか1つだけでも解析できます。'),
    h('label', { class: 'file-label', for: id }, data ? `${name}を選び直す` : `${name}を選ぶ`),
    h('input', { type: 'file', id, accept: '.accdb,.mdb,.json', disabled: !!state.busy,
      onchange: (event) => { const file = event.target.files[0]; event.target.value = ''; setSlot(slot, file); } }),
    data && h('div', { class: `loaded${data.inv ? '' : ' pending'}` },
      h('p', {}, h('strong', {}, data.name), data.inv ? `（${kind}・解析済み）` : '（未解析）'),
      h('p', { class: 'muted' }, data.inv ? `${data.inv.assets.length}件の資産: ${countKinds(data.inv)}` : '選んだだけでは解析しません。「解析開始」を押すと解析します。'),
      h('button', { type: 'button', class: 'btn', disabled: !!state.busy, onclick: () => setSlot(slot, null) }, `${name}を削除`)));
}
function dependencyWarnings() {
  const inv = state.inventory;
  if (!inv) return [];
  const lines = [];
  const linked = inv.assets.filter((asset) => asset.kind === 'external' || asset.isLinked || asset.linked || asset.linkedDatabaseName);
  if (chosenSlots().length === 1 && state.origin !== 'merged' && state.origin !== 'synthetic') {
    lines.push(linked.length
      ? `別のファイルへのリンクが${linked.length}件あります（${linked.slice(0, 5).map((asset) => asset.name).join('、')}${linked.length > 5 ? ' ほか' : ''}）。もう1つのファイルを追加して解析するまで、これらの依存は未確認です。`
      : 'この1つのファイルだけで解析しています。別のファイルへのリンクがある場合、もう1つのファイルを追加して解析するまで依存は確認できません。');
  }
  for (const item of inv.limitations.slice(0, 8)) lines.push(issueLine(item));
  if (inv.limitations.length > 8) lines.push(`ほか${inv.limitations.length - 8}件の取得制限があります。`);
  return lines;
}
function healthText() {
  return state.health === undefined ? 'Accessの解析環境を確認しています。'
    : state.health === null ? 'ローカルサーバーに接続できません。'
    : state.health.access?.available ? 'この環境ではAccessファイルを解析できます。'
    : `この環境ではAccessファイルを解析できません。${state.health.access?.reason ? `理由: ${state.health.access.reason}` : ''}JSON解析資料または合成サンプルを使用できます。`;
}
function step1(main) {
  const warnings = dependencyWarnings();
  fill(main,
    heading('データベースを解析', 'Accessファイル（.accdb / .mdb）またはJSON解析資料を取り込みます。ファイルはこのPC内のローカルサーバーだけで処理し、外部へ送信しません。1つのファイルだけでも解析できます。'),
    h('p', { class: 'notice', id: 'health-note', role: 'status' }, healthText()),
    h('section', { class: 'slot' }, h('h3', {}, 'ファイルの選び方'),
      h('ul', { class: 'muted' },
        h('li', {}, 'フォームとデータが同じファイルに入っていることもあります。その場合はファイル1だけ選んでください。'),
        h('li', {}, 'データを別のファイルに分けている場合は、ファイル2も選んでください。'),
        h('li', {}, 'どちらを選べばよいかわからないときは、まず手元の1つのファイルを選んで解析を始めてください。')),
      h('p', { class: 'muted' }, '解析はファイルの構造（テーブル・フォームなどの名前とつながり）だけを調べます。データの行は読み取らず、取得できない項目は「未確認」として扱います。')),
    slotView('front'),
    slotView('back'),
    h('div', { class: 'analyze-bar' },
      h('button', { type: 'button', class: 'btn primary', id: 'analyze-start', disabled: !needsRun() || !!state.busy, onclick: runAnalysis }, '解析開始'),
      state.cancelCtl && h('button', { type: 'button', class: 'btn', id: 'analyze-cancel', onclick: cancelAnalysis }, '解析を中止')),
    state.notice && h('p', { class: 'notice', role: 'status' }, state.notice),
    status(), alertBox(state.error),
    state.error && state.errorProgress && h('p', { class: 'muted', id: 'failed-progress' }, state.errorProgress),
    state.inventory && h('section', { class: 'block' }, h('h3', {}, '取り込んだ資料'),
      h('p', {}, `${sourceLabel()}: ${state.inventory.assets.length}件の資産（${countKinds(state.inventory)}）。${sourceNote()}`),
      warnings.length ? h('ul', { class: 'warn' }, warnings.map((line) => h('li', {}, line))) : null),
    h('section', { class: 'slot' }, h('h3', {}, '資料がない場合'), h('p', { class: 'muted' }, '説明用の合成サンプルで操作を試せます。実在のデータではありません。'),
      h('button', { type: 'button', class: 'btn', disabled: !!state.busy, onclick: loadDemo }, '合成サンプルを試す')));
}

// ---- 2: フォーム・ページを選択 ----
function step2(main) {
  if (!state.inventory) { fill(main, heading('フォーム・ページ・帳票を選択'), h('p', {}, '資料が未取込みです。'), h('a', { href: '#/1' }, 'データベースを解析へ戻る')); return; }
  fill(main,
    heading('フォーム・ページ・帳票を選択', '移行の対象にするフォーム・ページ・帳票を選びます。次の工程で利用形態を確認します。選択は自動では増えません。'),
    h('p', { class: 'notice' }, sourceNote()),
    h('p', { class: 'muted' }, PAGE_NOTE),
    pagesUnavailable(state.inventory) && h('p', { class: 'notice' }, PAGES_UNAVAILABLE_NOTE),
    h('div', { class: 'tools' },
      h('input', { type: 'search', class: 'field', 'aria-label': 'フォーム・ページ・帳票名で検索', placeholder: 'フォーム・ページ・帳票名で検索', value: state.query, oninput: (event) => { state.query = event.target.value; renderList(); } }),
      h('select', { class: 'field', 'aria-label': '種類で絞り込み', onchange: (event) => { state.kind = event.target.value; renderList(); } },
        [['all', 'すべての種類'], ['form', 'フォーム'], ['page', 'ページ'], ['report', '帳票']].map(([value, label]) => h('option', { value, selected: state.kind === value ? true : null }, label)))),
    h('p', { class: 'count', id: 'count', role: 'status' }),
    h('div', { id: 'list' }));
  renderList();
  renderAside();
}
function visibleAssets() {
  const q = state.query.trim().toLowerCase();
  return selectable().filter((asset) => (state.kind === 'all' || asset.kind === state.kind)
    && (!q || `${asset.name} ${asset.caption ?? ''} ${asset.id}`.toLowerCase().includes(q)));
}
function renderList() {
  const visible = visibleAssets();
  const all = selectable();
  const list = $('list');
  if (!all.length) {
    fill(list, h('p', { class: 'empty' }, pagesUnavailable(state.inventory) ? 'フォーム・帳票が取得できませんでした。ページは一覧を取得できていないため、ページがないとは判断しません。' : 'フォーム・ページ・帳票が取得できませんでした。資料を確認し、別の資料を取り込んでください。'), h('a', { href: '#/1' }, 'データベースを解析へ戻る'));
  } else if (!visible.length) {
    fill(list, h('div', { class: 'empty' }, h('p', {}, '条件に一致するフォーム・ページ・帳票はありません。選択済みの項目は保持されています。'),
      h('button', { type: 'button', class: 'btn', onclick: () => { state.query = ''; state.kind = 'all'; render(); } }, '検索条件をクリア')));
  } else {
    fill(list, h('div', { class: 'row head', 'aria-hidden': 'true' }, h('span', {}, '選択'), h('span', {}, '名前'), h('span', {}, '種類'), h('span', {}, '主な依存関係')),
      h('ul', { class: 'rows' }, visible.map((asset, i) => {
        const deps = asset.dependsOn.map(nameOf);
        const issue = asset.status !== 'supported' ? `${STATUS[asset.status]}: ${asset.issues[0]?.message ?? '取得できなかった項目があります。'}` : '';
        return h('li', {}, h('label', { class: 'row' },
          h('input', { type: 'checkbox', 'data-asset-id': asset.id, 'aria-labelledby': `n${i}`, 'aria-describedby': `d${i}`, checked: state.selected.has(asset.id) ? true : null, onchange: (event) => toggle(asset.id, event.target.checked) }),
          h('span', { class: 'name', id: `n${i}` }, asset.caption ?? asset.name),
          h('span', { class: 'type' }, KIND[asset.kind]),
          h('span', { class: 'deps', id: `d${i}` }, deps.length ? `${deps.slice(0, 4).join('、')}${deps.length > 4 ? ` ほか${deps.length - 4}件` : ''}` : '依存なし', issue && h('span', { class: 'issue' }, issue))));
      })));
  }
  const hidden = [...state.selected].filter((id) => !visible.some((asset) => asset.id === id)).length;
  $('count').textContent = `${state.selected.size}件選択中（全${all.length}件${visible.length !== all.length ? `、表示${visible.length}件` : ''}${hidden ? `、表示外の選択${hidden}件を含む` : ''}）`;
}
function toggle(id, on) {
  if (on) state.selected.add(id); else state.selected.delete(id);
  state.plan = null; state.analysis = null; state.targets = []; state.analysisToken++;
  if (!state.selected.size) state.maxStep = Math.min(state.maxStep, 2);
  renderList(); renderAside(); renderActions(); renderChrome();
  [...document.querySelectorAll('#list input[type="checkbox"]')].find((input) => input.dataset.assetId === id)?.focus({ preventScroll: true });
  if (state.selected.size) void computeAnalysis();
}
function renderAside() {
  const body = $('aside-body');
  if (!state.selected.size) { fill(body, h('p', { class: 'muted' }, 'フォーム・ページ・帳票を選ぶと、共有する資産と未選択の機能への影響をここに表示します。')); return; }
  const analysis = state.analysis;
  if (!analysis) { fill(body, h('p', { class: 'muted' }, state.error || '依存関係と影響を計算しています…')); return; }
  fill(body,
    block('選択したフォーム・ページ・帳票が使う資産', analysis.dependencyIds.map(nameWithKind), '選択した資産以外の依存は確認されていません。'),
    block('未選択の機能への影響', analysis.impactedIds.map((id) => `${nameWithKind(id)}は、選択範囲と共有する資産に依存します。`), '取得できた定義では、未選択の機能への影響は見つかっていません。'),
    block('共有される資産', analysis.sharedIds.map(nameWithKind), '共有される資産は確認されていません。'),
    h('p', { class: 'muted' }, '確認すべき影響の候補です。移行の成否や選択の変更を示すものではありません。'));
}

// ---- 3: 利用形態を確認 ----
function radioGroup(legend, key, options) {
  return h('fieldset', {}, h('legend', {}, legend), options.map(([value, label]) => h('label', { class: 'choice' },
    h('input', { type: 'radio', name: key, value, checked: state.usage[key] === value ? true : null, onchange: () => { state.usage[key] = value; state.plan = null; } }), h('span', {}, label))));
}
function checkField(legend, key, label) {
  return h('fieldset', {}, h('legend', {}, legend), h('label', { class: 'choice' },
    h('input', { type: 'checkbox', checked: state.usage[key] ? true : null, onchange: (event) => { state.usage[key] = event.target.checked; state.plan = null; } }), h('span', {}, label)));
}
function selectField(label, key, options, emptyToNull) {
  const id = `sel-${key}`;
  return h('div', { class: 'select-field' }, h('label', { for: id }, label),
    h('select', { id, class: 'field', onchange: (event) => { state.usage[key] = emptyToNull && !event.target.value ? null : event.target.value; state.plan = null; } },
      options.map(([value, text]) => h('option', { value, selected: (state.usage[key] ?? '') === value ? true : null }, text))));
}
function step3(main) {
  fill(main,
    heading('利用形態を確認', '人数・同時編集・場所・オフライン・権限は別の条件です。未回答の項目は「未確認」として計画に残ります。'),
    h('p', { class: 'muted' }, `選択中: ${state.selected.size}件`),
    h('div', { class: 'form' },
      radioGroup('利用人数', 'users', [['solo', '一人で使う'], ['team', '複数人で使う']]),
      checkField('同時編集', 'concurrentEditing', '同時に入力・編集する'),
      radioGroup('利用場所', 'location', [['device', '同じPCから'], ['lan', '社内の複数PCから'], ['remote', '社外・遠隔から']]),
      checkField('通信', 'offlineRequired', 'オフラインでも使う'),
      selectField('権限', 'permissions', [['', '未確認'], ['same', '全員同じ権限'], ['roles', '役割ごとに分ける']], true),
      selectField('既存Accessとの共存', 'coexistence', [['undecided', '未定'], ['keep-source', '既存Accessを残して併用する'], ['shared-store', '保存先を共有して併用する'], ['replace-scope', '対象範囲を置き換える']], false)),
    h('p', { class: 'muted' }, 'チェックなしの項目は「不要」として扱います。'));
}

// ---- 4: 依存関係と影響を確認 ----
function step4(main) {
  if (!state.analysis) { fill(main, heading('依存関係と影響を確認'), alertBox(state.error || '選択を確認してください。')); return; }
  const a = state.analysis;
  const targets = state.targets;
  if (state.targetId && !targets.some((target) => target.id === state.targetId)) state.targetId = null;
  const uncertain = [...a.unresolved, ...a.blockers];
  fill(main,
    heading('依存関係と影響を確認', '取得できた定義に基づく確認項目です。網羅や移行の成否を保証しません。'),
    h('div', { class: 'blocks' },
      block('選択したフォーム・ページ・帳票', a.selectedIds.map(nameWithKind), '選択がありません。'),
      block('依存する資産', a.dependencyIds.map(nameWithKind), '取得できた範囲では、依存する資産はありません。'),
      block('共有される資産', a.sharedIds.map(nameWithKind), '共有される資産は確認されていません。'),
      block('未選択の機能への影響', a.impactedIds.map(nameWithKind), '取得できた定義では、影響を受ける未選択の機能は見つかっていません。'),
      block('未取得・非対応', uncertain.map(issueLine), '未取得・非対応として報告された項目はありません。ただし動的参照などは検出できない場合があります。')),
    h('fieldset', { class: 'targets' }, h('legend', {}, '移行先'),
      h('p', { class: 'muted' }, '候補は利用形態からの相対的な目安です。適合や性能は未実証で、保証ではありません。'),
      targets.map((target) => h('div', { class: 'target' },
        h('label', { class: 'choice' }, h('input', { type: 'radio', name: 'target', value: target.id, 'aria-describedby': `t-${target.id}`, checked: state.targetId === target.id ? true : null,
          onchange: () => { state.targetId = target.id; state.plan = null; renderActions(); } }), h('span', {}, target.label)),
        h('div', { class: 'target-body', id: `t-${target.id}` },
          h('p', {}, h('strong', { class: `fit ${target.fit}` }, `${FIT[target.fit]}`), ' ', target.reasons.join(' ')),
          target.requirements.length ? h('p', { class: 'muted' }, `条件: ${target.requirements.join(' ')}`) : null,
          target.unknowns.length ? h('p', { class: 'muted' }, `未確認: ${target.unknowns.join(' ')}`) : null)))),
    h('div', { class: 'select-field' }, h('label', { for: 'notes' }, 'メモ（任意）'),
      h('textarea', { id: 'notes', class: 'field', rows: 3, maxlength: 2000, oninput: (event) => { state.notes = event.target.value; state.plan = null; } }, state.notes)),
    !state.targetId && h('p', { class: 'muted' }, '移行先を選ぶと「計画を作成」を押せます。'));
}

// ---- 5: 移行計画 ----
async function save(name, text, type, extension) {
  const status = document.querySelector('#save-status');
  status.textContent = '';
  try {
    if (typeof window.showSaveFilePicker === 'function') {
      const handle = await window.showSaveFilePicker({
        suggestedName: name,
        types: [{ description: extension.toUpperCase(), accept: { [type]: [extension] } }],
      });
      const writable = await handle.createWritable();
      await writable.write(text);
      await writable.close();
      status.textContent = `${name} を保存しました。`;
      return;
    }

    const url = URL.createObjectURL(new Blob([text], { type: `${type};charset=utf-8` }));
    const a = h('a', { href: url, download: name });
    document.body.append(a); a.click(); a.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
    status.textContent = `${name} のダウンロードを開始しました。ブラウザーのダウンロード先を確認してください。`;
  } catch (error) {
    status.textContent = error?.name === 'AbortError'
      ? '保存をキャンセルしました。'
      : `${name} を保存できませんでした。もう一度お試しください。`;
  }
}
async function renderPlanDiagrams(plan) {
  for (const id of ['flow', 'er']) {
    const container = document.querySelector(`[data-diagram="${id}"] .diagram-canvas`);
    if (container && state.plan === plan && state.step === 5) await renderDiagram(container, plan.diagrams?.[id] ?? '', id);
  }
}
function step5(main) {
  const plan = state.plan;
  if (!plan) { fill(main, heading('移行計画'), status(), alertBox(state.planError, loadPlan)); return; }
  const kind = { synthetic: '合成サンプル（実Accessの解析結果ではありません）', access: 'Access解析結果', inventory: 'JSON資料（自己申告）' }[plan.source.kind] ?? plan.source.kind;
  fill(main,
    heading('移行計画'),
    h('div', { class: 'notice strong' }, h('p', {}, h('strong', {}, plan.status === 'review-required' ? '草案・未確認の事項が残っています。要件はすべて未承認です。' : '草案です。要件はすべて未承認です。')),
      h('p', {}, `出典: ${plan.source.name}（${kind}）`)),
    h('div', { class: 'save' },
      h('button', { type: 'button', class: 'btn', onclick: () => save('access-migration-plan.md', '\uFEFF' + state.markdown, 'text/markdown', '.md') }, 'Markdownを保存'),
      h('button', { type: 'button', class: 'btn', onclick: () => save('access-migration-plan.json', JSON.stringify(plan, null, 2), 'application/json', '.json') }, 'JSONを保存')),
    h('p', { id: 'save-status', class: 'save-status', role: 'status', 'aria-live': 'polite' }),
    h('section', { class: 'block diagrams', 'aria-label': '依存関係の図' }, h('h3', {}, '依存関係の図'),
      ...[['flow', '業務フロー候補図'], ['er', 'ER図']].map(([id, label]) => h('figure', { class: 'diagram-panel', 'data-diagram': id },
        h('figcaption', {}, label), h('div', { class: 'diagram-canvas', 'aria-live': 'polite' })))),
    block('注意事項', plan.notices, ''),
    block('移行先候補', [`${plan.target.label}（${FIT[plan.target.fit]}）`, ...plan.target.reasons], ''),
    h('section', { class: 'block' }, h('h3', {}, '要件（未承認）'),
      plan.requirements.map((req) => h('article', { class: 'req' }, h('h4', {}, `${req.id} ${req.title}`),
        h('p', {}, req.description),
        h('p', { class: 'muted' }, '根拠'), h('ul', {}, req.evidence.map((text) => h('li', {}, text))),
        h('p', {}, h('span', { class: 'muted' }, '検証: '), req.verification),
        h('p', {}, h('strong', {}, req.confirmed ? '承認済み' : '未承認'))))),
    block('移行手順', plan.migrationSteps, ''),
    block('検証手順', plan.validationSteps, ''),
    block('未解決事項', plan.unresolved.map(issueLine), '未解決事項はありません。'),
    block('リスク', plan.risks, ''));
  void renderPlanDiagrams(plan);
}

// ---- 起動 ----
// 解析環境の確認はCOM起動のため遅い（約10秒）。初期表示を妨げないよう少し遅らせる。
setTimeout(() => api('/api/health').then((health) => { state.health = health; }, () => { state.health = null; }).then(() => { const note = $('health-note'); if (note) note.textContent = healthText(); }), 1500);
if (hashStep() !== 1) history.replaceState(null, '', '#/1');
show(1);

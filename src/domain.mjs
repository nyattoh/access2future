const assetKinds = ['table', 'query', 'form', 'page', 'report', 'macro', 'module', 'external'];
const statuses = ['supported', 'partial', 'unsupported'];
const usageChoices = { users: ['solo', 'team'], location: ['device', 'lan', 'remote'], permissions: ['same', 'roles'], coexistence: ['undecided', 'keep-source', 'shared-store', 'replace-scope'] };
const usageLabels = { users: '利用人数', concurrentEditing: '同時編集', location: '利用場所', offlineRequired: 'オフライン利用', permissions: '権限', coexistence: '既存Accessとの共存' };
const usageNames = {
  users: { solo: '一人', team: '複数人' },
  location: { device: '同じPC', lan: '社内の複数PC', remote: '社外・遠隔' },
  permissions: { same: '同じ権限', roles: '役割別' },
  coexistence: { undecided: '未定', 'keep-source': '現行Accessを維持', 'shared-store': '共有データで共存', 'replace-scope': '選択範囲を切替' },
};
function usageName(key, value) {
  if (value === null || value === undefined) return '未確認';
  if (key === 'offlineRequired') return value ? '必要' : '不要';
  if (key === 'concurrentEditing') return value ? 'あり' : 'なし';
  return usageNames[key]?.[value] ?? String(value);
}

function object(value, label) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new TypeError(`${label}はオブジェクトで指定してください。`);
  return value;
}

function array(value, label, max = 5000) {
  if (!Array.isArray(value) || value.length > max) throw new TypeError(`${label}は${max}件以下の配列で指定してください。`);
  return value;
}

function string(value, label, max = 4000) {
  if (typeof value !== 'string' || value.length > max || !value.trim()) throw new TypeError(`${label}は空でない${max}文字以下の文字列で指定してください。`);
  return value;
}

function choice(value, choices, label) {
  if (!choices.includes(value)) throw new TypeError(`${label}の値が不正です。`);
  return value;
}

function bool(value, label) {
  if (typeof value !== 'boolean') throw new TypeError(`${label}は真偽値で指定してください。`);
  return value;
}

// 生の定義は許可リストから除外し、表示用文字列にも混入する接続情報を取り除く。
function safeText(value, label, max = 4000) {
  return string(value, label, max)
    .replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/g, '')
    .replace(/\b(?:Provider|Data Source|Server|DSN|Database|DBQ|UID|User ID|UserName|PWD|Password|Jet OLEDB:Database Password|token|api[_-]?key|secret)\s*=\s*(?:"(?:[^"]|"")*"|'(?:[^']|'')*'|[^;\r\n])*/gi, '[接続情報を除去]')
    .replace(/\b(?:postgres(?:ql)?|mysql|sqlserver|mongodb(?:\+srv)?|https?):\/\/[^\s<>]+/gi, '[外部参照を除去]')
    .replace(/(?:[A-Za-z]:[\\/]|\\\\)[^\s<>"']+/g, '[元のパスを除去]')
    .replace(/(?:^|\s)\/(?:[^\s<>"']+\/)*[^\s<>"']+/g, ' [元のパスを除去]');
}

function safeId(value, label) {
  const id = string(value, label, 1000);
  if (/[\u0000-\u001f\u007f]/.test(id) || safeText(id, label, 1000) !== id) throw new TypeError(`${label}に秘密情報や絶対パスを含めないでください。`);
  return id;
}

function linkedName(value, label, filename = false) {
  const name = safeId(value, label);
  if (name.length > 255 || /[\\/]/.test(name) || (filename && (/[<>:"|?*]/.test(name) || ['.', '..'].includes(name)))) throw new TypeError(`${label}はパスを含まない安全な名前で指定してください。`);
  return name;
}

function procedureName(value, label) {
  if (typeof value !== 'string' || value.length > 255 || !/^[\p{L}_][\p{L}\p{N}_]*$/u.test(value)) throw new TypeError(`${label}はプロシージャ名（255字以下の安全な識別子）だけを指定してください。ソース本文・パス・秘密は含めないでください。`);
  return value;
}

function issue(value, label) {
  object(value, label);
  const result = { code: safeId(value.code, `${label}.code`), message: safeText(value.message, `${label}.message`) };
  if (value.assetId !== undefined) result.assetId = safeId(value.assetId, `${label}.assetId`);
  return result;
}

export function normalizeInventory(input) {
  object(input, 'Inventory');
  if (input.schemaVersion !== 1) throw new TypeError('InventoryのschemaVersionは1にしてください。');
  object(input.source, 'source');
  const sourceName = string(input.source.name, 'source.name', 1000).split(/[\\/]/).at(-1);
  const source = { name: safeText(sourceName, 'source.name'), kind: choice(input.source.kind, ['access', 'inventory', 'synthetic'], 'source.kind') };
  if (input.source.accessVersion !== undefined) source.accessVersion = safeText(input.source.accessVersion, 'source.accessVersion', 100);
  if (input.source.analysedAt !== undefined) {
    const date = string(input.source.analysedAt, 'source.analysedAt', 100);
    if (!/^\d{4}-\d{2}-\d{2}T/.test(date) || !Number.isFinite(Date.parse(date))) throw new TypeError('source.analysedAtはISO日時で指定してください。');
    source.analysedAt = date;
  }
  if (input.source.fingerprint !== undefined) {
    if (typeof input.source.fingerprint !== 'string' || !/^[a-f0-9]{64}$/i.test(input.source.fingerprint)) throw new TypeError('source.fingerprintはSHA256で指定してください。');
    source.fingerprint = input.source.fingerprint;
  }
  const ids = new Set();
  let edgeCount = 0;
  const assets = array(input.assets, 'assets').map((value, index) => {
    const label = `assets[${index}]`;
    object(value, label);
    const id = safeId(value.id, `${label}.id`);
    if (ids.has(id)) throw new TypeError(`重複した資産IDがあります: ${id}`);
    ids.add(id);
    const result = {
      id, kind: choice(value.kind, assetKinds, `${label}.kind`), name: safeText(value.name, `${label}.name`, 500),
      dependsOn: [...new Set(array(value.dependsOn === undefined ? [] : value.dependsOn, `${label}.dependsOn`).map(id => safeId(id, `${label}.dependsOn`)))],
      status: choice(value.status === undefined ? 'partial' : value.status, statuses, `${label}.status`),
      issues: array(value.issues === undefined ? [] : value.issues, `${label}.issues`, 500).map(item => issue(item, `${label}.issues`))
    };
    edgeCount += result.dependsOn.length;
    if (edgeCount > 50000) throw new TypeError('依存参照は合計50000件以下にしてください。');
    if (value.caption !== undefined) result.caption = safeText(value.caption, `${label}.caption`, 500);
    for (const key of ['isLinked', 'linked', 'local']) if (value[key] !== undefined) result[key] = bool(value[key], `${label}.${key}`);
    for (const key of ['linkedDatabaseName', 'linkedTableName']) if (value[key] !== undefined) {
      if (result.kind !== 'table') throw new TypeError(`${key}はtableだけに指定できます。`);
      result[key] = linkedName(value[key], `${label}.${key}`, key === 'linkedDatabaseName');
      result.isLinked = true;
    }
    if (value.fields !== undefined) {
      if (result.kind !== 'table') throw new TypeError('fieldsはtableだけに指定できます。');
      result.fields = array(value.fields, `${label}.fields`, 1000).map(field => {
        object(field, `${label}.fields`);
        const item = { name: safeText(field.name, 'field.name', 500) };
        if (field.dataType !== undefined) item.dataType = safeText(field.dataType, 'field.dataType', 100);
        for (const key of ['required', 'isPrimaryKey']) if (field[key] !== undefined) item[key] = bool(field[key], `field.${key}`);
        return item;
      });
    }
    if (value.rowCount !== undefined) {
      if (result.kind !== 'table' || !Number.isSafeInteger(value.rowCount) || value.rowCount < 0) throw new TypeError('rowCountはtableの0以上の整数で指定してください。');
      if (!result.isLinked && !result.linked && result.local !== false) result.rowCount = value.rowCount;
    }
    // VBAは静的解析の要約（安全な識別子と件数）だけを保持する。ソース本文・パス・秘密は受け入れず、フォーム・帳票・モジュール以外では拒否する。
    const codeKeys = ['hasCodeModule', 'moduleType', 'procedureNames', 'procedureCount'];
    if (codeKeys.some(key => value[key] !== undefined)) {
      if (!['form', 'report', 'module'].includes(result.kind)) throw new TypeError('VBA要約はフォーム・帳票・モジュールだけに指定できます。ページはAccess Data Access Pagesのため非対応です。');
      if (value.hasCodeModule !== undefined) result.hasCodeModule = bool(value.hasCodeModule, `${label}.hasCodeModule`);
      if (value.moduleType !== undefined) result.moduleType = choice(value.moduleType, result.kind === 'module' ? ['standard', 'class'] : ['class'], `${label}.moduleType`);
      if (value.procedureNames !== undefined) {
        const names = array(value.procedureNames, `${label}.procedureNames`, 1000);
        if (new Set(names).size !== names.length) throw new TypeError(`${label}.procedureNamesは重複しない識別子で指定してください。`);
        result.procedureNames = names.map(name => procedureName(name, `${label}.procedureNames`));
      }
      if (value.procedureCount !== undefined) {
        if (!Number.isSafeInteger(value.procedureCount) || value.procedureCount < 0 || value.procedureCount > 100000) throw new TypeError(`${label}.procedureCountは0以上100000以下の整数で指定してください。`);
        result.procedureCount = value.procedureCount;
      }
      if (result.moduleType !== undefined && result.hasCodeModule === undefined) result.hasCodeModule = true;
      const hasProcedures = (result.procedureNames?.length ?? 0) > 0 || (result.procedureCount ?? 0) > 0;
      if (result.hasCodeModule === false && (result.moduleType !== undefined || hasProcedures)) throw new TypeError('hasCodeModuleがfalseの資産にコードの要約を指定できません。');
      if (result.procedureCount !== undefined && result.procedureNames !== undefined && result.procedureCount < result.procedureNames.length) throw new TypeError(`${label}.procedureCountはprocedureNamesの件数以上にしてください。`);
    }
    return result;
  });
  const relations = array(input.relations === undefined ? [] : input.relations, 'relations', 10000).map(value => {
    object(value, 'relation');
    const result = { from: safeId(value.from, 'relation.from'), to: safeId(value.to, 'relation.to'), fields: array(value.fields === undefined ? [] : value.fields, 'relation.fields', 1000).map(field => {
      object(field, 'relation.field');
      return { from: safeText(field.from, 'relation.field.from', 500), to: safeText(field.to, 'relation.field.to', 500) };
    }) };
    if (value.enforced !== undefined) result.enforced = bool(value.enforced, 'relation.enforced');
    return result;
  });
  return { schemaVersion: 1, source, assets, relations, limitations: array(input.limitations === undefined ? [] : input.limitations, 'limitations', 5000).map(value => issue(value, 'limitation')) };
}

export function mergeInventories(inputs) {
  const inventories = array(inputs, 'inventories', 20).map(normalizeInventory);
  if (!inventories.length) throw new TypeError('統合するInventoryを1件以上指定してください。');
  const remap = (index, id) => `db${index + 1}:${id}`;
  const assets = [];
  const relations = [];
  const limitations = [];
  const sourceAssets = [];
  const isExternal = asset => asset.isLinked || asset.linked || asset.local === false || asset.linkedDatabaseName !== undefined || asset.linkedTableName !== undefined;
  for (const [index, inventory] of inventories.entries()) {
    const mapped = new Map();
    for (const asset of inventory.assets) {
      const result = { ...asset, id: remap(index, asset.id), dependsOn: asset.dependsOn.map(id => remap(index, id)), issues: asset.issues.map(item => ({ ...item, ...(item.assetId ? { assetId: remap(index, item.assetId) } : {}) })), caption: `${asset.caption ?? asset.name}【${index + 1}: ${inventory.source.name}】` };
      mapped.set(asset.id, result);
      assets.push(result);
    }
    sourceAssets.push(mapped);
    relations.push(...inventory.relations.map(item => ({ ...item, from: remap(index, item.from), to: remap(index, item.to) })));
    limitations.push(...inventory.limitations.map(item => ({ ...item, ...(item.assetId ? { assetId: remap(index, item.assetId) } : {}) })));
  }
  for (const [index, inventory] of inventories.entries()) {
    for (const original of inventory.assets) {
      if (original.kind !== 'table' || !isExternal(original)) continue;
      const alias = sourceAssets[index].get(original.id);
      const add = (code, message) => limitations.push({ code, assetId: alias.id, message });
      if (!original.linkedDatabaseName || !original.linkedTableName) {
        add('LINK_METADATA_MISSING', `${alias.name} のリンク先DB名またはテーブル名が未取得です。名前だけで統合していません。`);
        continue;
      }
      const databases = inventories.map((candidate, candidateIndex) => ({ candidate, candidateIndex })).filter(({ candidate }) => candidate.source.name === original.linkedDatabaseName);
      if (databases.length !== 1) {
        add(databases.length ? 'LINK_AMBIGUOUS' : 'LINK_BACKEND_MISSING', `${alias.name} のリンク先 ${original.linkedDatabaseName} は${databases.length ? '同名素材が複数あり対応が曖昧' : '提供されていない'}ため保留です。外部データは取得していません。`);
        continue;
      }
      const { candidate, candidateIndex } = databases[0];
      const tables = candidate.assets.filter(asset => asset.kind === 'table' && !isExternal(asset) && asset.name === original.linkedTableName);
      if (tables.length !== 1) {
        add(tables.length ? 'LINK_AMBIGUOUS' : 'LINK_TABLE_MISSING', `${alias.name} のリンク先 ${original.linkedDatabaseName} 内の ${original.linkedTableName} は${tables.length ? '同名テーブルが複数' : 'ローカルテーブルが未取得'}のため保留です。`);
        continue;
      }
      const target = sourceAssets[candidateIndex].get(tables[0].id);
      alias.dependsOn = [...new Set([...alias.dependsOn, target.id])];
      if (target.fields) alias.fields = target.fields.map(field => ({ ...field }));
      // 行データと件数はリンク先から転記しない。取得状態も自動で引き上げない。
      delete alias.rowCount;
    }
  }
  const localTables = new Map();
  for (const [index, inventory] of inventories.entries()) for (const asset of inventory.assets) {
    if (asset.kind !== 'table' || isExternal(asset)) continue;
    if (!localTables.has(asset.name)) localTables.set(asset.name, []);
    localTables.get(asset.name).push({ index, asset });
  }
  for (const [name, matches] of localTables) if (new Set(matches.map(item => item.index)).size > 1) limitations.push({ code: 'TABLE_NAME_COLLISION', message: `${name} は別DBに同名テーブルがあります。明示リンクを除き別資産として保持し、同一テーブルかの判断は保留です。` });
  limitations.push({ code: 'MERGED_METADATA_ONLY', message: `アップロード順の最初の素材 ${inventories[0].source.name} を主DBとして、提供されたメタデータだけを統合しました。素材の出典種別は ${inventories.map(item => `${item.source.name}: ${item.source.kind}`).join('、')} です。実Accessの取得・動作・外部データ・業務再現はこの統合では検証していません。` });
  return normalizeInventory({ schemaVersion: 1, source: { name: inventories.map(item => item.source.name).join(' + '), kind: 'inventory' }, assets, relations, limitations });
}

function normalizeUsage(input = {}) {
  object(input, 'usage');
  return Object.fromEntries(Object.keys(usageLabels).map(key => [key, input[key] === undefined || input[key] === null ? null : usageChoices[key] ? choice(input[key], usageChoices[key], `usage.${key}`) : bool(input[key], `usage.${key}`)]));
}

export function analyzeSelection(input, selectedIds) {
  const inventory = normalizeInventory(input);
  const selected = [...new Set(array(selectedIds, 'selectedIds').map(value => safeId(value, 'selectedIds')))];
  if (!selected.length) throw new TypeError('フォーム・ページ・帳票を1件以上選択してください。');
  const byId = new Map(inventory.assets.map(asset => [asset.id, asset]));
  for (const id of selected) if (!['form', 'page', 'report'].includes(byId.get(id)?.kind)) throw new TypeError(`選択対象は取得済みのフォーム・ページ・帳票にしてください: ${id}`);
  const edges = new Map(inventory.assets.map(asset => [asset.id, [...asset.dependsOn]]));
  for (const relation of inventory.relations) if (edges.has(relation.from)) edges.get(relation.from).push(relation.to);
  const unresolved = [];
  const blockers = [];
  const add = (list, value) => { if (!list.some(item => item.code === value.code && item.assetId === value.assetId && item.referenceId === value.referenceId)) list.push(value); };
  // 明示的なスタックにより深い依存と循環で再帰を停止させない。
  function closure(roots, report = false) {
    const visited = new Set();
    const active = new Set();
    const stack = roots.map(id => ({ id }));
    while (stack.length) {
      const { id, from, exit } = stack.pop();
      if (exit) { active.delete(id); continue; }
      if (!byId.has(id)) {
        if (report) add(unresolved, { code: 'MISSING_REFERENCE', assetId: from, referenceId: id, message: `参照先 ${id} が未取得です。依存の範囲を確認してください。` });
        continue;
      }
      if (active.has(id)) {
        if (report) add(unresolved, { code: 'CYCLE', assetId: from, referenceId: id, message: `${from} と ${id} を含む循環があります。移行順序と実行条件を確認してください。` });
        continue;
      }
      if (visited.has(id)) continue;
      visited.add(id); active.add(id);
      stack.push({ id, exit: true });
      for (const dependency of edges.get(id)) stack.push({ id: dependency, from: id });
    }
    return visited;
  }
  const included = closure(selected, true);
  for (const id of included) {
    const asset = byId.get(id);
    if (asset.status !== 'supported') add(asset.status === 'unsupported' ? blockers : unresolved, { code: asset.status === 'unsupported' ? 'UNSUPPORTED' : 'PARTIAL', assetId: id, message: `${asset.name} は${asset.status === 'unsupported' ? '非対応' : '部分取得'}です。再現できると判断する前に確認が必要です。` });
    if (asset.kind === 'external' || asset.isLinked || asset.linked || asset.local === false) add(unresolved, { code: 'EXTERNAL_REFERENCE', assetId: id, message: `${asset.name} は外部参照です。外部データの取得・更新・移行は行っていません。` });
    for (const item of asset.issues) add(unresolved, { ...item, assetId: id });
  }
  for (const item of inventory.limitations) if (!item.assetId || included.has(item.assetId) || !byId.has(item.assetId)) add(unresolved, item);
  const impactedIds = [];
  const shared = new Set();
  for (const asset of inventory.assets) {
    if (!['form', 'page', 'report'].includes(asset.kind) || selected.includes(asset.id)) continue;
    const overlap = [...closure([asset.id])].filter(id => included.has(id) && id !== asset.id);
    if (overlap.length) { impactedIds.push(asset.id); overlap.forEach(id => shared.add(id)); }
  }
  const dependencyIds = [...included].filter(id => !selected.includes(id));
  const evidence = [
    `選択対象: ${selected.join('、')}。選択を自動追加していません。`,
    `dependsOnとテーブル間の参照関係から、選択対象以外の依存を${dependencyIds.length}件確認しました。`,
    ...impactedIds.map(id => `未選択の ${id} は選択範囲と共有する資産に依存します。変更時の影響確認が必要です。`),
    '取得されたメタデータに基づく結果です。動的参照や未取得定義の依存が網羅されている保証はありません。'
  ];
  return { selectedIds: selected, dependencyIds, impactedIds, sharedIds: [...shared], unresolved, blockers, evidence };
}

export function recommendTargets(input, inputUsage, analysis) {
  normalizeInventory(input);
  const usage = normalizeUsage(inputUsage);
  const unknowns = Object.keys(usageLabels).filter(key => usage[key] === null || usage[key] === 'undecided').map(key => `${usageLabels[key]}が未確認です。`);
  const multi = usage.users === 'team' || usage.concurrentEditing === true;
  const complex = multi || usage.location === 'remote' || usage.permissions === 'roles';
  const constraints = [];
  if (usage.coexistence === 'shared-store') constraints.push('既存Accessとの共有保存先、書込み責任、同期、競合回避を設計して検証する。');
  if (usage.coexistence === 'keep-source') constraints.push('既存Accessを残し、移行範囲との境界、二重入力、データ同期を確認する。');
  if (analysis?.impactedIds?.length) constraints.push('共有資産に依存する未選択の機能を変更前後に検証する。');
  const hasUnknown = unknowns.length > 0;
  const candidates = [
    { id: 'web', label: 'Webアプリ', fit: complex ? 'recommended' : 'conditional', reasons: [complex ? '複数利用者・遠隔利用・役割別権限の管理を設計しやすい候補です。' : '個人端末中心の利用には運用基盤の負担も比較する必要があります。'], requirements: ['認証・認可、DB、バックアップ、運用担当、配備先を具体化する。'], unknowns: [...unknowns] },
    { id: 'excel', label: 'Excel', fit: complex ? 'not-recommended' : usage.users === 'solo' && usage.location === 'device' ? 'recommended' : 'conditional', reasons: [complex ? '同時編集・遠隔利用・役割別権限があると、ファイル運用と競合管理の負担が大きくなります。' : '個人端末での表形式作業とオフライン利用に合う可能性があります。'], requirements: ['フォーム、クエリ、帳票、VBAの代替方法とExcelでの再現範囲を確認する。', '一意性、参照整合性、同時書込み、バックアップを検証する。'], unknowns: [...unknowns] },
    { id: 'sheets-gas', label: 'Googleスプレッドシート + GAS', fit: usage.offlineRequired === true ? 'not-recommended' : usage.permissions === 'roles' ? 'conditional' : multi && usage.location === 'remote' ? 'recommended' : 'conditional', reasons: [usage.offlineRequired === true ? 'GASによる業務処理をオフラインで実行できる前提にはできません。' : 'オンラインの共同編集に合う可能性があります。権限・データ量・実行制限の確認が必要です。'], requirements: ['アカウント、情報管理方針、通信環境、GASの実行制限を確認する。', '行や役割別の権限が必要なら、シート共有だけで満たせると判断せず代替設計を検証する。'], unknowns: [...unknowns] }
  ];
  for (const target of candidates) {
    if (hasUnknown && target.fit === 'recommended') target.fit = 'conditional';
    if (target.id === 'web' && usage.offlineRequired === true) { target.fit = 'conditional'; target.requirements.push('オフライン時の保存、再接続時の同期、競合解決を別途設計する。'); target.reasons.push('オフライン対応の実装と検証が必要です。'); }
    target.requirements.push(...constraints);
    target.reasons.push('ローカル規則による相対的な計画候補です。互換性・性能・適合は未実証です。');
  }
  return candidates;
}

export function buildPlan(input, options) {
  const inventory = normalizeInventory(input);
  object(options, 'options');
  const analysis = analyzeSelection(inventory, options.selectedIds);
  const usage = normalizeUsage(options.usage);
  const candidates = recommendTargets(inventory, usage, analysis);
  const target = options.targetId === undefined ? candidates.find(candidate => candidate.fit === 'recommended') ?? candidates[0] : candidates.find(candidate => candidate.id === options.targetId);
  if (!target) throw new TypeError('移行先候補が不正です。');
  const notes = options.notes === undefined || options.notes === '' ? '' : safeText(options.notes, 'notes');
  const unresolved = [...analysis.unresolved, ...analysis.blockers, ...target.unknowns.map(message => ({ code: 'USAGE_UNKNOWN', message }))];
  if (usage.coexistence === 'undecided' && (analysis.sharedIds.length || analysis.impactedIds.length)) unresolved.push({ code: 'COEXISTENCE_UNDECIDED', message: '共有資産と未選択機能への影響があるため、既存Accessとの共存方法と変更境界の確認が必要です。' });
  if (target.fit === 'not-recommended') unresolved.push({ code: 'TARGET_MISMATCH', message: '選択した移行先と利用形態の制約を確認してください。' });
  const requirements = [];
  const addRequirement = (title, description, evidence, verification) => requirements.push({ id: `REQ-${String(requirements.length + 1).padStart(3, '0')}`, title, description, evidence, verification, confirmed: false });
  for (const id of analysis.selectedIds) {
    const asset = inventory.assets.find(item => item.id === id);
    addRequirement(`${asset.caption ?? asset.name}の業務を再現する`, '入力・表示・更新・例外処理・業務ルールを利用者と確認し、取得状態だけで再現可能と判断しない。', [`選択資産: ${id}`, `取得状態: ${asset.status}`], '現行業務の操作シナリオと期待結果を用意し、利用者が移行先の結果を確認する。');
  }
  const tables = inventory.assets.filter(asset => analysis.dependencyIds.includes(asset.id) && asset.kind === 'table');
  if (tables.length) addRequirement('データ構造と参照整合性を維持する', '型、必須項目、主キー、関連、件数・内容の比較方法を定義する。行データの移行はこの計画生成では実行しない。', tables.map(asset => `${asset.id}: ${asset.fields?.length ?? '未確認'}項目、件数${asset.rowCount === undefined ? '未計数' : asset.rowCount}`), '許可されたコピーで主キー重複、必須値、関連、件数、代表レコード、業務集計を照合する。');
  addRequirement('利用形態と移行先の制約を確認する', target.requirements.join(' '), Object.keys(usageLabels).map(key => `${usageLabels[key]}: ${usageName(key, usage[key])}`), '利用者、同時編集、通信断、権限、共存の各条件で操作とデータの整合性を確認する。');
  if (analysis.impactedIds.length) addRequirement('未選択機能への影響を確認する', '共有資産を変更する前に影響と変更境界を合意する。未選択機能を移行対象へ自動追加しない。', analysis.impactedIds.map(id => `影響候補: ${id}`), '未選択フォーム・ページ・帳票を現行側で回帰確認し、結果と復旧条件を記録する。');
  if (unresolved.length) addRequirement('未解決事項を解消する', '循環・動的参照・未取得・非対応・利用形態の不足を利用者と確認し、必要なら追加解析や個別設計を行う。', unresolved.map(item => `${item.code}: ${item.message}`), '各未解決事項に担当、確認方法、結果、残る制約を記録する。');
  const migrationSteps = [
    '1. 選択範囲・依存・影響候補・未解決事項を利用者と確認し、要件と変更境界の承認を得る。',
    `2. ${target.label}の試作方針、データ構造、画面・業務処理の代替方法、権限と利用環境を設計する。`,
    '3. 原本を保持し、許可されたコピーと合成データで移行手順・変換・検証を試行する。',
    '4. 既存Accessとの共存、同期、切替時の書込み停止、バックアップ、復旧条件を定義する。',
    '5. 要件を承認し検証結果を確認した後、別途承認された実装・移行作業で段階的に切り替える。'
  ];
  const validationSteps = [
    '選択業務の正常系・入力異常・取消・更新・帳票結果を現行側の期待結果と比較する。',
    '依存データの型・必須値・主キー・参照整合性・件数・代表値・業務集計を比較する。',
    '未選択の共有DB利用機能を回帰確認し、共存中の両側の整合性を確認する。',
    '利用人数・同時更新の競合・役割別アクセス・ネットワーク断と再接続を確認する。',
    'バックアップからの復元と切戻しを試行し、利用者の受入結果と未解決事項を記録する。'
  ];
  const risks = [
    '取得状態がsupportedでも、移行先での業務再現は未検証です。',
    '動的参照や未取得の定義により依存や影響候補が不足する可能性があります。',
    ...(analysis.impactedIds.length ? ['共有資産の変更は未選択の機能へ影響する可能性があります。'] : []),
    ...(usage.coexistence === 'undecided' || usage.coexistence === null ? ['既存Accessとの共存方法が未定です。切替を確定できません。'] : []),
    ...(usage.coexistence === 'shared-store' ? ['Accessと移行先の同時書込みでは競合・同期・障害復旧の個別検証が必要です。'] : [])
  ];
  const notices = ['この出力は移行計画の草案です。アプリ生成、データ移行、原本の更新、外部API呼出しを行っていません。', '要件はすべて未承認です。移行先候補の適合性・性能・互換性は未実証です。', inventory.source.kind === 'synthetic' ? '出典は完全な合成データです。実Accessの解析結果ではありません。' : inventory.source.kind === 'access' ? '入力はAccess由来のメタデータとして渡されています。実ファイルの取得・動作はこの処理では確認していません。取得不能な定義・外部データは対応済みと扱いません。' : '出典は入力されたInventoryメタデータです。実Accessでの取得・動作はこの処理では確認していません。'];
  if (notes) notices.push(`利用者メモ: ${notes}`);
  return {
    version: 1, status: unresolved.length ? 'review-required' : 'draft', source: inventory.source, analysis, usage, target,
    requirements, migrationSteps, validationSteps, risks, unresolved, notices,
    hand_over: { summary: '選択範囲から移行計画草案を生成しました。要件承認・実装・移行・受入検証は未実施です。', selectedIds: analysis.selectedIds, targetId: target.id, pending: ['要件と変更境界の利用者承認', '未解決事項の確認', '実装・移行と受入検証'] }
  };
}

function markdown(value) {
  return String(value).replace(/\\/g, '\\\\').replace(/([`*_{}\[\]()#+.!|>~-])/g, '\\$1').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/\r?\n/g, ' ');
}

export function renderPlanMarkdown(plan) {
  object(plan, 'plan');
  const statusName = { draft: '草案', 'review-required': '要確認' }[plan.status] ?? plan.status;
  const sourceName = { synthetic: '合成サンプル', access: 'Access由来の構造資料', inventory: '入力・統合された構造資料' }[plan.source.kind] ?? plan.source.kind;
  const fitName = { recommended: '推薦', conditional: '条件付き', 'not-recommended': '要確認' }[plan.target.fit] ?? plan.target.fit;
  const lines = ['# 移行計画（草案）', '', `状態: ${markdown(statusName)} / 要件は未承認`, `出典: ${markdown(plan.source.name)}（${markdown(sourceName)}）`, '', '## 注意事項', ...plan.notices.map(text => `- ${markdown(text)}`), '', '## 選択範囲と依存', `- 選択: ${plan.analysis.selectedIds.map(markdown).join('、')}`, `- 依存: ${plan.analysis.dependencyIds.map(markdown).join('、') || '取得された依存なし'}`, `- 共有資産: ${plan.analysis.sharedIds.map(markdown).join('、') || '確認された共有なし'}`, `- 未選択への影響候補: ${plan.analysis.impactedIds.map(markdown).join('、') || '取得済み定義からの候補なし'}`, '', '### 根拠', ...plan.analysis.evidence.map(text => `- ${markdown(text)}`), '', '## 利用形態', ...Object.keys(usageLabels).map(key => `- ${usageLabels[key]}: ${markdown(usageName(key, plan.usage[key]))}`), '', '## 移行先候補', `${markdown(plan.target.label)}（${markdown(fitName)}）`, ...plan.target.reasons.map(text => `- ${markdown(text)}`), '', '## 要件（すべて未承認）'];
  for (const requirement of plan.requirements) lines.push('', `### ${markdown(requirement.id)} ${markdown(requirement.title)}`, markdown(requirement.description), '', `根拠: ${requirement.evidence.map(markdown).join(' / ')}`, `検証方法: ${markdown(requirement.verification)}`, '承認: 未承認');
  for (const [title, items] of [['移行計画', plan.migrationSteps], ['検証計画', plan.validationSteps], ['リスク', plan.risks]]) lines.push('', `## ${title}`, ...items.map(text => `- ${markdown(text)}`));
  lines.push('', '## 未解決事項', ...(plan.unresolved.length ? plan.unresolved.map(item => `- ${markdown(item.code)}: ${markdown(item.message)}${item.assetId ? `（${markdown(item.assetId)}）` : ''}`) : ['- 取得済み情報からの指摘なし。要件・適合・動作の承認と検証は未実施です。']), '', '## 引き継ぎ', markdown(plan.hand_over.summary), ...plan.hand_over.pending.map(text => `- ${markdown(text)}`), '');
  return lines.join('\n');
}

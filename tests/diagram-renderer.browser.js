async function diagramRendererRegression() {
  const { renderDiagram } = await import('/plan-diagrams.mjs');
  const root = document.createElement('div');
  const source = 'flowchart LR\nA["受注入力"] --> B["確認"]';
  const rendered = await renderDiagram(root, source, 'flow');
  if (!rendered) throw new Error('Valid flowchart did not render');
  if (root.querySelector('img')?.alt !== '業務フロー候補図' || root.querySelector('img')?.naturalWidth === 0) throw new Error('Rendered diagram image is missing or inaccessible');
  const fallbackRoot = document.createElement('div');
  const invalid = 'not a valid Mermaid diagram';
  const fallback = await renderDiagram(fallbackRoot, invalid, 'er');
  if (fallback) throw new Error('Invalid diagram was reported as rendered');
  if (!fallbackRoot.querySelector('.diagram-warning') || fallbackRoot.querySelector('.diagram-source')?.textContent !== invalid) {
    throw new Error('Invalid diagram did not show the safe source fallback');
  }
  const emptyRoot = document.createElement('div');
  let emptyRendererCalls = 0;
  await renderDiagram(emptyRoot, '', 'er', { render: async () => { emptyRendererCalls += 1; throw new Error('empty source must not render'); } });
  if (emptyRendererCalls !== 0 || emptyRoot.querySelector('.diagram-empty')?.textContent !== '依存範囲にテーブルはありません。') {
    throw new Error('Empty ER diagram did not show the no-table state without calling Mermaid');
  }
  return { result: 'passed', checks: ['strict-svg-rendering', 'accessible-svg-image', 'invalid-source-text-fallback', 'empty-er-state'] };
}

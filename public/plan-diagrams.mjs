export async function renderDiagram(container, source, id, renderer = globalThis.mermaid) {
  try {
    if (!renderer) throw new Error('図の描画機能を読み込めませんでした。');
    const result = await renderer.render(`plan-${id}`, source);
    const objectUrl = URL.createObjectURL(new Blob([result.svg], { type: 'image/svg+xml;charset=utf-8' }));
    const image = document.createElement('img');
    image.className = 'diagram-image';
    image.alt = id === 'flow' ? '業務フロー候補図' : 'ER図';
    image.src = objectUrl;
    const previousUrl = container.dataset.objectUrl;
    if (previousUrl) URL.revokeObjectURL(previousUrl);
    container.dataset.objectUrl = objectUrl;
    container.replaceChildren(image);
    const loaded = await new Promise((resolve) => {
      image.addEventListener('load', () => resolve(true), { once: true });
      image.addEventListener('error', () => resolve(false), { once: true });
    });
    if (!loaded) throw new Error('図の画像を読み込めませんでした。');
    return true;
  } catch {
    const message = document.createElement('p');
    message.className = 'diagram-warning';
    message.textContent = '図として表示できませんでした。Mermaidの記述を確認してください。';
    const fallback = document.createElement('pre');
    fallback.className = 'diagram-source';
    fallback.textContent = source;
    const previousUrl = container.dataset.objectUrl;
    if (previousUrl) URL.revokeObjectURL(previousUrl);
    delete container.dataset.objectUrl;
    container.replaceChildren(message, fallback);
    return false;
  }
}

import { copyFile, mkdir, readFile, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';

const root = resolve(import.meta.dirname, '..');
const destination = resolve(root, 'public', 'vendor');
await mkdir(destination, { recursive: true });
const licenseDestination = resolve(destination, 'licenses');
await mkdir(licenseDestination, { recursive: true });
await Promise.all([
  (async () => {
    const source = await readFile(resolve(root, 'node_modules', '@mermaid-js', 'tiny', 'dist', 'mermaid.tiny.js'), 'utf8');
    await writeFile(resolve(destination, 'mermaid.tiny.js'), source.replace(/[ \t]+(?=\r?$)/gm, ''));
  })(),
  copyFile(resolve(root, 'node_modules', '@mermaid-js', 'tiny', 'LICENSE'), resolve(destination, 'mermaid-LICENSE.txt')),
  copyFile(resolve(root, 'licenses', 'third-party', 'THIRD_PARTY_NOTICES.txt'), resolve(destination, 'THIRD_PARTY_NOTICES.txt')),
  copyFile(resolve(root, 'licenses', 'third-party', 'DOMPurify-MPL-2.0.txt'), resolve(licenseDestination, 'DOMPurify-MPL-2.0.txt')),
  copyFile(resolve(root, 'licenses', 'third-party', 'lodash-MIT.txt'), resolve(licenseDestination, 'lodash-MIT.txt'))
]);

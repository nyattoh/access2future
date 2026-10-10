import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

test('Windows bundle includes notices and licence texts for Mermaid Tiny components', async () => {
  const [notice, lodashLicense, domPurifyMpl, tauriConfig, projectLicense] = await Promise.all([
    readFile(new URL('../public/vendor/THIRD_PARTY_NOTICES.txt', import.meta.url), 'utf8'),
    readFile(new URL('../public/vendor/licenses/lodash-MIT.txt', import.meta.url), 'utf8'),
    readFile(new URL('../public/vendor/licenses/DOMPurify-MPL-2.0.txt', import.meta.url), 'utf8'),
    readFile(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8'),
    readFile(new URL('../LICENSE', import.meta.url), 'utf8')
  ]);
  assert.match(projectLicense, /Apache License\s+Version 2\.0/);
  assert.match(notice, /Mermaid Tiny 12\.1\.0[\s\S]*Knut Sveidqvist/);
  assert.match(notice, /DOMPurify 3\.4\.12[\s\S]*Cure53[\s\S]*Apache-2\.0 OR MPL-2\.0/);
  assert.match(notice, /Lodash 4\.18\.1[\s\S]*OpenJS Foundation[\s\S]*Underscore\.js/);
  assert.match(lodashLicense, /Copyright OpenJS Foundation/);
  assert.match(lodashLicense, /Jeremy Ashkenas/);
  assert.match(lodashLicense, /Permission is hereby granted/);
  assert.match(domPurifyMpl, /Mozilla Public License Version 2\.0/);
  const resources = JSON.parse(tauriConfig).bundle.resources;
  assert.equal(resources['../LICENSE'], 'LICENSE');
  assert.equal(resources['../public/vendor/mermaid-LICENSE.txt'], 'licenses/mermaid-LICENSE.txt');
  assert.equal(resources['../public/vendor/THIRD_PARTY_NOTICES.txt'], 'THIRD_PARTY_NOTICES.txt');
  assert.equal(resources['../public/vendor/licenses/lodash-MIT.txt'], 'licenses/lodash-MIT.txt');
  assert.equal(resources['../public/vendor/licenses/DOMPurify-MPL-2.0.txt'], 'licenses/DOMPurify-MPL-2.0.txt');
});

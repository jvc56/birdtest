const assert = require('node:assert/strict');
const { readFileSync } = require('node:fs');
const vm = require('node:vm');
const source = readFileSync('infra/magpie/entry.js.tftpl', 'utf8');
function route(release, uri) {
    const context = { event: { request: { uri } } };
    return vm.runInNewContext(source.replaceAll('${release}', release) + '\nhandler(event)', context);
}
assert.equal(route('', '/').statusCode, 503);
for (const uri of ['/', '/index.html', '/wasmentry', '/wasmentry/']) {
    const result = route('wasm-preview-v0.1.0', uri);
    assert.equal(result.statusCode, 302);
    assert.equal(result.headers.location.value, '/releases/wasm-preview-v0.1.0/wasmentry/');
    assert.equal(result.headers['cache-control'].value, 'no-store');
}
// New default releases must not redirect an already-open older release.
const old = '/releases/wasm-preview-v0.1.0/';
assert.equal(route('wasm-preview-v0.2.0', old + 'wasmentry/').uri, old + 'wasmentry/index.html');
for (const asset of ['wasmentry/magpie_wasm.mjs', 'wasmentry/magpie_wasm.wasm', 'data/lexica/CSW24.kwg', 'missing.js']) {
    assert.equal(route('wasm-preview-v0.2.0', old + asset).uri, old + asset);
}
console.log('MAGPIE entry routing passed');

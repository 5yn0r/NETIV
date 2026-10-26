// Le vrai module WebAssembly, exécuté par Node : mêmes sorties qu'en natif.
//
//   crates/netiv-wasm/build-web.sh && node crates/netiv-wasm/tests/wasm.mjs [dist]
//
// Vérifie le scénario golden de la CLI (octet pour octet), un challenge
// débutant réussi avec score détaillé, l'édition d'une topologie, les erreurs
// { status, error } et l'export / import du profil.
import { readFileSync } from 'node:fs';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { dirname, join, resolve } from 'node:path';
import assert from 'node:assert/strict';

const here = dirname(fileURLToPath(import.meta.url));
const root = resolve(here, '../../..');
const dist = resolve(process.argv[2] || join(here, '../dist'));

const { default: init, NetivEngine } = await import(pathToFileURL(join(dist, 'netiv_wasm.js')).href);
await init({ module_or_path: readFileSync(join(dist, 'netiv_wasm_bg.wasm')) });

let passed = 0;
function test(name, fn) {
  fn();
  passed += 1;
  console.log(`ok - ${name}`);
}

const exec = (e, id, line) => e.request('POST', `/api/labs/${id}/exec`, { line });

test('le scénario golden donne la même sortie que la CLI native', () => {
  const e = new NetivEngine();
  const mvp = readFileSync(join(root, 'examples/mvp.yaml'), 'utf8');
  const golden = readFileSync(join(root, 'crates/netiv-cli/tests/golden/mvp_session.txt'), 'utf8');
  const lab = e.request('POST', '/api/labs', { yaml: mvp });
  const script = ['ping PC1 192.168.2.10 2', 'trace 1', 'inspect 19', 'explain 25',
    'show route R1', 'show arp PC1', 'show mac SW1'];
  let out = '';
  for (const line of script) {
    const r = exec(e, lab.id, line);
    assert.equal(r.error, null, `${line} : ${r.error}`);
    out += `netiv> ${line}\n${r.output}\n`;
  }
  assert.ok(out.includes('temps=16.000 ms') && out.includes('temps=8.000 ms'));
  assert.equal(out, golden);
});

test('un challenge débutant est réussi, avec score détaillé et profil exporté', () => {
  const e = new NetivEngine(JSON.stringify({ profileName: 'awa' }));
  const lab = e.request('POST', '/api/labs', { challenge: 'beginner-01' });
  assert.equal(lab.snapshot.challenge.id, 'beginner-01');
  assert.throws(() => e.request('GET', `/api/labs/${lab.id}/health`), (err) => err.status === 403);
  exec(e, lab.id, 'gateway PC1 192.168.1.1');
  exec(e, lab.id, 'challenge check');
  const r = exec(e, lab.id, 'answer PC1 wrong-gateway');
  const ch = r.snapshot.challenge;
  assert.equal(ch.finished, true);
  assert.equal(ch.score_detail.diagnosis, 40);
  assert.equal(ch.score_detail.fix, 40);
  assert.equal(ch.score_detail.total, ch.score);
  // L'horloge du navigateur est utilisée : la durée est réaliste, pas absurde.
  assert.ok(ch.score_detail.elapsed_secs < 60, `durée : ${ch.score_detail.elapsed_secs}`);
  const profile = e.exportProfile();
  assert.ok(profile.includes('beginner-01'));
  const again = new NetivEngine(JSON.stringify({ profileName: 'awa', profile }));
  assert.equal(again.request('GET', '/api/profile').completed, 1);
});

test('une topologie se construit et se modifie dans l\'éditeur', () => {
  const e = new NetivEngine();
  const lab = e.request('POST', '/api/labs', { empty: 'atelier' });
  const topology = {
    network: { name: 'atelier' },
    devices: [
      { name: 'PC1', type: 'host', interfaces: [{ name: 'eth0' }] },
      { name: 'PC2', type: 'host', interfaces: [{ name: 'eth0' }] },
    ],
    links: [{ a: 'PC1:eth0', b: 'PC2:eth0' }],
  };
  const v = e.request('POST', `/api/labs/${lab.id}/topology/validate`, { topology });
  assert.deepEqual(v.errors, []);
  e.request('PUT', `/api/labs/${lab.id}/topology`, { topology });
  const auto = e.request('POST', `/api/labs/${lab.id}/topology/autoconfig`);
  assert.ok(auto.changes.length > 0);
  const ping = exec(e, lab.id, 'ping PC1 PC2');
  assert.equal(ping.error, null, ping.error);
  const text = e.exportLab(lab.id);
  assert.ok(text.startsWith('format_version: 1'));
  assert.equal(e.importLab(text).snapshot.devices.length, 2);
});

test('les erreurs remontent en { status, error }', () => {
  const e = new NetivEngine();
  assert.throws(() => e.request('GET', '/api/labs/lab-9'), (err) => err.status === 404 && /lab inconnu/.test(err.error));
  assert.throws(() => e.request('POST', '/api/labs', {}), (err) => err.status === 400);
  assert.throws(() => e.request('GET', '/api/nope'), (err) => err.status === 404);
  assert.throws(() => new NetivEngine('{ cassé'), (err) => err.status === 400);
  assert.equal(typeof NetivEngine.version(), 'string');
  assert.equal(NetivEngine.catalog().entries.length, 31);
});

console.log(`${passed} tests WebAssembly réussis`);

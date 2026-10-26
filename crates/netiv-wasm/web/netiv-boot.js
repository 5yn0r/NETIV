/* NETIV — démarrage dans le navigateur (WebAssembly), sans serveur.
 *
 * 1. charge le module WebAssembly et crée le moteur (`NetivEngine`) ;
 * 2. l'installe dans `window.netivEngine` : l'interface (app.js) l'utilise à
 *    la place de l'API HTTP, avec le même contrat ;
 * 3. charge l'interface (app.js).
 *
 * Deux façons de l'utiliser :
 * * **seul** (démo) : le profil est gardé dans le stockage local du navigateur ;
 * * **intégré** dans une page hôte, dans une iframe de la même origine, avec
 *   `?host=1` dans l'URL : la page hôte fournit le profil et l'épreuve à ouvrir
 *   et reçoit la progression (voir « Protocole » dans docs/wasm.md).
 *
 * Messages reçus de la page hôte (même origine uniquement) :
 *   { source: 'netiv-host', type: 'init', profile?, profileName?, start?, labs?, layout? }
 *     layout : 'lab' (épreuve imposée : catalogue replié, panneau Défi ouvert)
 *   { source: 'netiv-host', type: 'export-lab', requestId }
 * Messages envoyés à la page hôte :
 *   { source: 'netiv', type: 'ready', version }            moteur prêt (attend « init » si host=1)
 *   { source: 'netiv', type: 'profile', profile }          le profil a changé (JSON)
 *   { source: 'netiv', type: 'challenge-finished', kind, id, title, score, scoreDetail, hintsUsed, profile }
 *   { source: 'netiv', type: 'lab', requestId, id, text }  réponse à « export-lab »
 *   { source: 'netiv', type: 'error', error }              le moteur n'a pas pu démarrer
 */
import init, { NetivEngine } from './netiv_wasm.js';

const params = new URLSearchParams(window.location.search);
const hosted = params.get('host') === '1' && window.parent !== window;
const ORIGIN = window.location.origin;
const PROFILE_KEY = 'netiv.profile';

function toHost(msg) {
  if (hosted) window.parent.postMessage({ source: 'netiv', ...msg }, ORIGIN);
}

function fromHost(type) {
  return new Promise((resolve) => {
    const onMessage = (e) => {
      if (e.origin !== ORIGIN || e.source !== window.parent) return;
      const m = e.data;
      if (!m || m.source !== 'netiv-host' || m.type !== type) return;
      window.removeEventListener('message', onMessage);
      resolve(m);
    };
    window.addEventListener('message', onMessage);
  });
}

function localProfile() {
  try { return localStorage.getItem(PROFILE_KEY) || undefined; } catch (_) { return undefined; }
}

function saveLocalProfile(json) {
  try { localStorage.setItem(PROFILE_KEY, json); } catch (_) { /* stockage indisponible */ }
}

/** Épreuve à ouvrir d'après l'URL : ?challenge=…, ?range=…, ?builtin=… */
function startFromUrl() {
  for (const key of ['challenge', 'range', 'builtin']) {
    const v = params.get(key);
    if (v) return { [key]: v };
  }
  return undefined;
}

/**
 * Enveloppe le moteur : après chaque requête, signale à l'hôte les épreuves
 * terminées et les changements de profil.
 */
function watch(engine) {
  let lastProfile = engine.exportProfile();
  const finished = new Set();
  const onProfile = (profile) => {
    if (profile === lastProfile) return;
    lastProfile = profile;
    if (hosted) toHost({ type: 'profile', profile });
    else saveLocalProfile(profile);
  };
  return {
    request(method, path, body) {
      const data = engine.request(method, path, body);
      const snap = data && data.snapshot;
      const ch = snap && snap.challenge;
      if (ch && ch.finished && ch.score_detail && !finished.has(`${snap.id}:${ch.id}`)) {
        finished.add(`${snap.id}:${ch.id}`);
        const profile = engine.exportProfile();
        onProfile(profile);
        toHost({
          type: 'challenge-finished',
          kind: ch.kind,
          id: ch.id,
          title: ch.title,
          score: ch.score_detail.total,
          scoreDetail: ch.score_detail,
          hintsUsed: ch.hints_used,
          profile,
        });
      } else if (method !== 'GET') {
        onProfile(engine.exportProfile());
      }
      return data;
    },
    exportProfile: () => engine.exportProfile(),
    importProfile: (json) => { engine.importProfile(json); lastProfile = json; },
    exportLab: (id) => engine.exportLab(id),
    importLab: (text) => engine.importLab(text),
    version: () => NetivEngine.version(),
  };
}

function loadInterface() {
  const script = document.createElement('script');
  script.src = new URL('app.js', import.meta.url).href;
  document.head.append(script);
}

async function start() {
  await init({ module_or_path: new URL('netiv_wasm_bg.wasm', import.meta.url) });
  toHost({ type: 'ready', version: NetivEngine.version() });
  const options = hosted ? await fromHost('init') : { profile: localProfile() };

  const engineOptions = {};
  if (options.profileName) engineOptions.profileName = String(options.profileName);
  if (options.labs) engineOptions.labs = options.labs;
  let engine;
  try {
    engine = new NetivEngine(JSON.stringify({ ...engineOptions, profile: options.profile || undefined }));
  } catch (e) {
    // Profil illisible (ou d'une version plus récente) : on démarre sans lui
    // plutôt que de bloquer l'élève, et on le signale.
    console.warn('NETIV : profil ignoré', e);
    engine = new NetivEngine(JSON.stringify(engineOptions));
  }
  window.netivEngine = watch(engine);
  window.NETIV_START = options.start || startFromUrl();
  window.NETIV_LAYOUT = options.layout || params.get('layout') || undefined;
  window.NETIV_EMBEDDED = hosted;

  if (hosted) {
    window.addEventListener('message', (e) => {
      if (e.origin !== ORIGIN || e.source !== window.parent) return;
      const m = e.data;
      if (!m || m.source !== 'netiv-host') return;
      if (m.type === 'export-lab') {
        const labs = window.netivEngine.request('GET', '/api/labs').labs;
        const id = m.labId || (labs.length ? labs[labs.length - 1].id : null);
        try {
          toHost({ type: 'lab', requestId: m.requestId, id, text: id ? window.netivEngine.exportLab(id) : null });
        } catch (err) {
          toHost({ type: 'lab', requestId: m.requestId, id, text: null, error: err.error || String(err) });
        }
      }
    });
  }
  loadInterface();
}

start().catch((e) => {
  const msg = (e && (e.error || e.message)) || String(e);
  toHost({ type: 'error', error: msg });
  const b = document.getElementById('banner');
  if (b) {
    b.className = 'banner error';
    b.textContent = `Le simulateur n'a pas pu démarrer : ${msg}`;
    b.hidden = false;
  }
});

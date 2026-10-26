/* NETIV — interface web locale.
 *
 * JavaScript sans dépendance. Règle d'or : tout texte venu du serveur (noms
 * d'équipements, sorties, traces) est inséré avec textContent / createTextNode,
 * jamais avec innerHTML — un nom comme « <img onerror=…> » reste du texte.
 */
(() => {
  'use strict';

  // ------------------------------------------------------------------ utilitaires DOM

  const $ = (sel, root = document) => root.querySelector(sel);
  const $$ = (sel, root = document) => Array.from(root.querySelectorAll(sel));
  const SVG = 'http://www.w3.org/2000/svg';

  /** Crée un élément. `attrs` : class, text, on<event>, data-*, et attributs. */
  function h(tag, attrs, ...kids) {
    return build(document.createElement(tag), attrs, kids);
  }
  function s(tag, attrs, ...kids) {
    return build(document.createElementNS(SVG, tag), attrs, kids);
  }
  function build(el, attrs, kids) {
    for (const [k, v] of Object.entries(attrs || {})) {
      if (v === null || v === undefined || v === false) continue;
      if (k === 'class') el.setAttribute('class', v);
      else if (k === 'text') el.textContent = v;
      else if (k.startsWith('on')) el.addEventListener(k.slice(2), v);
      else el.setAttribute(k, v === true ? '' : String(v));
    }
    for (const kid of kids.flat()) {
      if (kid === null || kid === undefined || kid === false) continue;
      el.append(kid instanceof Node ? kid : document.createTextNode(String(kid)));
    }
    return el;
  }
  // Icônes : tracés repris de Lucide (licence ISC), la famille d'icônes
  // d'ITEtude. Le HTML statique utilise les mêmes.
  const ICONS = {
    x: ['M18 6 6 18', 'm6 6 12 12'],
    first: ['M19 20 9 12l10-8z', 'M5 19V5'],
    prev: ['m15 18-6-6 6-6'],
    next: ['m9 18 6-6-6-6'],
    play: ['M6 3l14 9-14 9z'],
    pause: ['M7 4h3v16H7z', 'M14 4h3v16h-3z'],
  };
  function icon(name) {
    return s('svg', { class: 'ic', viewBox: '0 0 24 24', 'aria-hidden': 'true' }, ICONS[name].map((d) => s('path', { d })));
  }

  function clear(el) {
    while (el.firstChild) el.removeChild(el.firstChild);
    return el;
  }

  const reducedMotion = window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches;

  function store(key, value) {
    try {
      if (value === undefined) return JSON.parse(localStorage.getItem(key));
      localStorage.setItem(key, JSON.stringify(value));
    } catch (_) { /* stockage indisponible : on s'en passe */ }
    return null;
  }

  // ------------------------------------------------------------------ état

  const state = {
    info: null,
    library: null,
    profile: null,
    labs: [],          // [{id, name}]
    current: null,     // id du lab affiché
    snap: null,
    device: null,      // nom de l'équipement sélectionné
    tx: [],            // transactions du lab
    journey: null,     // {id, steps, text}
    step: -1,
    playing: null,     // identifiant du timer
    traceEvents: [],
    traceKinds: new Set(),
    traceText: '',
    libTab: 'networks',
    tab: 'device',
    positions: new Map(),
    view: { x: 0, y: 0, w: 1000, h: 560 }, // viewBox du schéma (zoom, déplacement)
    history: [],
    histPos: 0,
    deadline: null,    // horodatage de fin du temps conseillé
    busy: false,
    // Mode Construire (éditeur de réseau).
    build: {
      on: false,
      def: null,       // topologie appliquée (TopologyDef)
      draft: null,     // brouillon invalide, non appliqué
      past: [],        // historique pour « annuler »
      future: [],      // pour « rétablir »
      tool: 'select',  // 'select' | 'cable'
      cableFrom: null, // premier équipement choisi avec l'outil Câble
      sel: null,       // {kind:'device', name} | {kind:'link', a, b}
      errors: [],
      warnings: [],
      issueDevices: { err: new Set(), warn: new Set() },
      saving: false,
    },
  };

  // ------------------------------------------------------------------ API

  // Transport, choisi au démarrage :
  // * mode serveur (défaut) : `fetch` vers l'API HTTP de `netiv serve` ;
  // * mode WebAssembly : le moteur est dans la page (`window.netivEngine`,
  //   installé par crates/netiv-wasm/web/netiv-boot.js), même contrat que l'API.
  // `window.NETIV_MODE = 'server'` force le mode serveur.
  const engine = window.NETIV_MODE === 'server' ? null : (window.netivEngine || null);

  async function api(method, path, body) {
    if (engine) return engineApi(method, path, body);
    const init = { method, headers: {} };
    if (body !== undefined) {
      init.headers['Content-Type'] = 'application/json';
      init.body = JSON.stringify(body);
    }
    let res;
    try {
      res = await fetch(path, init);
    } catch (e) {
      throw new Error('Serveur injoignable : est-il toujours lancé ?');
    }
    let data = null;
    try { data = await res.json(); } catch (_) { /* corps vide */ }
    if (!res.ok) {
      const err = new Error((data && data.error) || `Erreur HTTP ${res.status}`);
      err.status = res.status;
      throw err;
    }
    return data;
  }

  /** Même requête, traitée par le moteur WebAssembly de la page. */
  async function engineApi(method, path, body) {
    try {
      return engine.request(method, path, body);
    } catch (e) {
      const err = new Error((e && e.error) || String(e));
      err.status = (e && e.status) || 500;
      throw err;
    }
  }

  function banner(msg, kind = 'error') {
    const b = $('#banner');
    if (!msg) { b.hidden = true; return; }
    b.className = `banner ${kind}`;
    b.textContent = msg;
    b.hidden = false;
    if (kind !== 'error') setTimeout(() => { if (b.textContent === msg) b.hidden = true; }, 4000);
  }

  async function guarded(fn) {
    if (state.busy) return;
    state.busy = true;
    try {
      banner('');
      await fn();
    } catch (e) {
      banner(e.message);
      consoleLine(e.message, 'err');
    } finally {
      state.busy = false;
    }
  }

  // ------------------------------------------------------------------ console

  // ---- console : complétion (Tab)

  const COMMANDS = ['acl', 'answer', 'arp', 'badges', 'baseline', 'cable', 'capture', 'challenge', 'chaos', 'clear',
    'course', 'curl', 'dashboard', 'devices', 'dhcp', 'diagnose', 'dns', 'docs', 'explain', 'forensic', 'forwarding',
    'gateway', 'health', 'help', 'hint', 'iface', 'inspect', 'ip', 'lab', 'latency', 'leaderboard', 'learn', 'load',
    'log', 'loss', 'nslookup', 'ping', 'port', 'range', 'route', 'run', 'security', 'service', 'show', 'step', 'time',
    'topology', 'trace', 'tutor', 'udp', 'what-if'];
  const SUBCOMMANDS = {
    show: ['ifaces', 'arp', 'route', 'mac', 'vlan', 'acl', 'services', 'dhcp', 'dns'],
    challenge: ['list', 'start', 'show', 'check', 'finish', 'giveup', 'causes', 'export'],
    range: ['list', 'start', 'show', 'check', 'finish', 'giveup'],
    route: ['add', 'del'],
    chaos: ['start', 'undo', 'reveal', 'list'],
    acl: ['add', 'insert', 'del', 'default', 'stateful'],
  };

  /** Candidats pour le mot en cours : commande, sous-commande ou équipement. */
  function candidates(words) {
    if (words.length === 1) return COMMANDS;
    const sub = SUBCOMMANDS[words[0]];
    if (sub && words.length === 2) return [...sub, ...(state.snap ? state.snap.devices.map((d) => d.name) : [])];
    return state.snap ? state.snap.devices.map((d) => d.name) : [];
  }

  function clearSuggestions() {
    const box = $('#console-suggest');
    if (box && !box.hidden) { clear(box); box.hidden = true; }
  }

  /** Tab : complète le mot en cours ; plusieurs possibilités → les propose. */
  function complete(input) {
    const value = input.value;
    const words = value.replace(/^\s+/, '').split(/\s+/);
    const word = words[words.length - 1];
    const lower = word.toLowerCase();
    const matches = candidates(words).filter((c) => c.toLowerCase().startsWith(lower));
    const box = $('#console-suggest');
    clearSuggestions();
    if (!matches.length) return;
    const replace = (choice) => {
      input.value = value.slice(0, value.length - word.length) + choice + ' ';
      input.focus();
    };
    if (matches.length === 1) { replace(matches[0]); return; }
    // Préfixe commun, puis la liste des choix.
    let common = matches[0];
    for (const m of matches) while (!m.toLowerCase().startsWith(common.toLowerCase())) common = common.slice(0, -1);
    if (common.length > word.length) input.value = value.slice(0, value.length - word.length) + common;
    for (const m of matches.slice(0, 24)) {
      box.append(h('button', { type: 'button', onclick: () => { replace(m); clearSuggestions(); } }, m));
    }
    box.hidden = false;
  }

  // ---- console : rendu
  //
  // La sortie vient du serveur : elle n'est jamais interprétée comme du HTML.
  // Elle est découpée en morceaux de texte, certains dans des <span> colorés
  // (adresses, durées, réussites, échecs, commandes suggérées).

  /** Motifs colorés, du plus précis au plus général. */
  const TOKENS = [
    // Commande suggérée entre guillemets : « trace 1 » (cliquable).
    ['t-cmd', /«\s?([a-z][^»]{0,80}?)\s?»/g],
    ['t-ip', /\b(?:\d{1,3}\.){3}\d{1,3}(?:\/\d{1,2})?\b/g],
    ['t-num', /\b\d+(?:[.,]\d+)?\s?(?:ms|µs|s)\b/g],
    ['t-ok', /\b(?:réponse de|réussi|objectif atteint|0 % de perte|en service|OK)\b|✓/g],
    ['t-bad', /\b(?:pas de réponse|délai dépassé|100 % de perte|inaccessible|injoignable|refusée?|coupée?|échec|abandon)\b|✗/g],
  ];

  /** Texte d'une sortie → nœuds, avec les motifs colorés. */
  function highlight(text) {
    const marks = [];
    for (const [cls, re] of TOKENS) {
      for (const m of text.matchAll(re)) {
        const start = m.index, end = start + m[0].length;
        if (marks.some((k) => start < k.end && end > k.start)) continue;
        // Entre guillemets, seules les vraies commandes deviennent cliquables
        // (« Réseau « office » chargé » cite un nom, pas une commande).
        if (cls === 't-cmd' && !COMMANDS.includes(m[1].trim().split(/\s+/)[0])) continue;
        marks.push({ start, end, cls, cmd: cls === 't-cmd' ? m[1].trim() : null });
      }
    }
    marks.sort((a, b) => a.start - b.start);
    const nodes = [];
    let at = 0;
    for (const k of marks) {
      if (k.start > at) nodes.push(text.slice(at, k.start));
      const part = text.slice(k.start, k.end);
      nodes.push(k.cmd
        ? h('button', { type: 'button', class: 't-cmd', title: 'Copier dans la ligne de commande', onclick: () => suggestCommand(k.cmd) }, part)
        : h('span', { class: k.cls }, part));
      at = k.end;
    }
    if (at < text.length) nodes.push(text.slice(at));
    return nodes;
  }

  /** Place une commande dans la ligne de saisie (sans l'exécuter). */
  function suggestCommand(cmd) {
    const input = $('#console-in');
    input.value = cmd.replace(/<[^>]*>/g, '').replace(/\s+/g, ' ').trim() + (/[<…]/.test(cmd) ? ' ' : '');
    input.focus();
    input.setSelectionRange(input.value.length, input.value.length);
  }

  function consoleAppend(el) {
    const out = $('#console-out');
    out.append(el);
    while (out.childElementCount > 600) out.removeChild(out.firstChild);
    out.scrollTop = out.scrollHeight;
  }

  /** Ligne d'information, de note ou d'erreur (hors commande). */
  function consoleLine(text, cls) {
    consoleAppend(h('div', { class: `entry ${cls || ''}` }, h('div', { class: 'out' }, cls === 'err' ? text : highlight(text))));
  }

  /** Bloc d'une commande : en-tête (invite, commande, heure) puis sortie. */
  function consoleCommand(line) {
    const now = new Date();
    const time = `${String(now.getHours()).padStart(2, '0')}:${String(now.getMinutes()).padStart(2, '0')}:${String(now.getSeconds()).padStart(2, '0')}`;
    const entry = h('div', { class: 'entry' },
      h('div', { class: 'cmd' }, h('span', { class: 'ps', 'aria-hidden': 'true' }, '›'), h('span', { class: 'line' }, line), h('time', null, time)));
    consoleAppend(entry);
    return entry;
  }

  /** Ajoute la sortie (ou l'erreur) d'une commande à son bloc. */
  function consoleResult(entry, output, error) {
    if (output) entry.append(h('div', { class: 'out' }, highlight(output)));
    if (error) entry.append(h('div', { class: 'out err' }, h('span', { class: 'err-tag' }, 'erreur'), ` ${error}`));
    if (!output && !error) entry.classList.add('silent');
    $('#console-out').scrollTop = $('#console-out').scrollHeight;
  }

  async function runCommand(line, { echo = true } = {}) {
    if (!state.current) throw new Error("Aucun lab ouvert : choisis un réseau dans le catalogue.");
    const entry = echo ? consoleCommand(line) : h('div', { class: 'entry' });
    if (!echo) consoleAppend(entry);
    const r = await api('POST', `/api/labs/${state.current}/exec`, { line });
    consoleResult(entry, r.output && r.output.replace(/\n+$/, ''), r.error);
    applySnapshot(r.snapshot);
    await refreshAfterCommand(r);
    return r;
  }

  async function refreshAfterCommand() {
    await loadTransactions({ selectNewest: true });
    if (state.tab === 'trace') await loadTrace();
    if (state.tab === 'device' && state.device) await loadDevice(state.device);
    if (state.snap && state.snap.challenge && state.snap.challenge.finished) await loadProfile();
    renderChallenge();
  }

  // ------------------------------------------------------------------ thème

  function applyTheme(theme) {
    if (theme) document.documentElement.setAttribute('data-theme', theme);
    else document.documentElement.removeAttribute('data-theme');
  }
  function initTheme() {
    applyTheme(store('netiv.theme'));
    $('#theme-toggle').addEventListener('click', () => {
      const dark = document.documentElement.getAttribute('data-theme') === 'dark'
        || (!document.documentElement.getAttribute('data-theme') && window.matchMedia('(prefers-color-scheme: dark)').matches);
      const next = dark ? 'light' : 'dark';
      applyTheme(next);
      store('netiv.theme', next);
    });
  }

  // ------------------------------------------------------------------ profil

  async function loadProfile() {
    try {
      state.profile = await api('GET', '/api/profile');
    } catch (_) { state.profile = null; }
    const chip = $('#profile-chip');
    const p = state.profile;
    chip.textContent = p ? `Niv. ${p.level} · ${p.level_name} · ${p.xp} XP` : 'Progression';
    renderLibrary();
  }

  async function openProfile() {
    const dlg = $('#profile-dialog');
    $('#profile-text').textContent = state.profile ? state.profile.dashboard : 'Profil indisponible.';
    let board = '(aucun profil enregistré)';
    try {
      const b = await api('GET', '/api/leaderboard');
      if (b.entries.length) {
        board = b.entries
          .map((e) => `${String(e.rank).padStart(2)}. ${e.name.padEnd(16)} ${String(e.xp).padStart(5)} XP   niv. ${e.level} ${e.level_name}   ${e.completed} réussis, ${e.badges} badge(s)`)
          .join('\n');
      }
    } catch (_) { /* hors ligne */ }
    $('#board-text').textContent = board;
    if (typeof dlg.showModal === 'function') dlg.showModal();
  }

  // ------------------------------------------------------------------ labs

  function applySnapshot(snap) {
    state.snap = snap;
    const lab = state.labs.find((l) => l.id === snap.id);
    if (lab) lab.name = snap.name || lab.name;
    if (snap.challenge && !snap.challenge.finished) {
      state.deadline = Date.now() + snap.challenge.remaining_secs * 1000;
    } else {
      state.deadline = null;
    }
    renderLabs();
    if (state.build.on && snap.editable === false) exitBuild();
    updateBuildToggle();
    renderStage();
  }

  /** Bouton « Construire » : indisponible sans lab ou pendant un défi. */
  function updateBuildToggle() {
    const t = $('#build-toggle');
    const on = state.build.on;
    const locked = state.snap && state.snap.editable === false;
    t.setAttribute('aria-pressed', String(on));
    t.disabled = !state.snap || (!on && locked);
    t.hidden = !on && locked;
    t.title = 'Construire ou modifier ce réseau  B';
  }

  async function createLab(body) {
    const r = await api('POST', '/api/labs', body);
    exitBuild();
    state.labs.push({ id: r.id, name: r.snapshot.name || r.id });
    state.current = r.id;
    resetViews();
    if (r.output) consoleLine(r.output.replace(/\n+$/, ''), 'note');
    applySnapshot(r.snapshot);
    await loadTransactions({ selectNewest: true });
    renderChallenge();
    if (r.snapshot.challenge) selectTab('challenge');
    return r;
  }

  function resetViews() {
    stopPlay();
    state.helpText = null;
    state.journey = null;
    state.step = -1;
    state.tx = [];
    state.device = null;
    state.traceEvents = [];
    renderJourney();
    renderDevice(null);
  }

  async function switchLab(id) {
    if (state.current === id) return;
    await guarded(async () => {
      exitBuild();
      state.current = id;
      resetViews();
      applySnapshot(await api('GET', `/api/labs/${id}`));
      await loadTransactions({ selectNewest: true });
      renderChallenge();
    });
  }

  async function closeLab(id) {
    await guarded(async () => {
      await api('DELETE', `/api/labs/${id}`);
      state.labs = state.labs.filter((l) => l.id !== id);
      if (state.current === id) {
        exitBuild();
        state.current = null;
        state.snap = null;
        resetViews();
        const next = state.labs[state.labs.length - 1];
        if (next) {
          state.current = next.id;
          applySnapshot(await api('GET', `/api/labs/${next.id}`));
          await loadTransactions({ selectNewest: true });
        }
      }
      renderLabs();
      renderStage();
      renderChallenge();
    });
  }

  function renderLabs() {
    const ul = clear($('#open-labs'));
    for (const lab of state.labs) {
      ul.append(
        h('li', null,
          h('button', {
            class: 'open', type: 'button', 'aria-current': lab.id === state.current ? 'true' : 'false',
            onclick: () => switchLab(lab.id),
          }, `${lab.name || lab.id}`),
          h('button', { class: 'close', type: 'button', 'aria-label': `Fermer ${lab.name || lab.id}`, title: 'Fermer', onclick: () => closeLab(lab.id) }, '×'),
        ),
      );
    }
    $('#no-labs').hidden = state.labs.length > 0;
  }

  // ------------------------------------------------------------------ catalogue

  function libButton(label, onclick, cls) {
    return h('button', { type: 'button', class: cls || null, onclick: () => guarded(onclick) }, label);
  }

  function renderLibrary() {
    const lib = state.library;
    const root = clear($('#library'));
    if (!lib) { root.append(h('p', { class: 'muted' }, 'Chargement…')); return; }
    const done = new Map((state.profile ? state.profile.completed_ids : []).map((c) => [c.id, c.score]));

    if (state.libTab === 'networks') {
      root.append(h('div', { class: 'btn-row create-row' },
        libButton('＋ Nouveau réseau', newEmptyNetwork, 'primary'),
        h('button', { type: 'button', title: 'Ouvrir un fichier .netiv, .yaml ou .json', onclick: () => $('#file-import').click() }, 'Importer un fichier')));
      for (const name of lib.topologies) {
        root.append(h('div', { class: 'item' },
          h('div', { class: 'name' }, name),
          libButton('Ouvrir', () => createLab({ builtin: name }))));
      }
      if (lib.labs.length) {
        root.append(h('h3', null, 'Bibliothèque de labs'));
        for (const l of lib.labs) {
          root.append(h('div', { class: 'item' },
            h('div', { class: 'name' }, l.title, h('small', null, `${l.kind === 'challenge' ? 'défi' : 'réseau'}${l.author ? ' · ' + l.author : ''}`)),
            libButton('Ouvrir', () => createLab({ lab: l.name }))));
        }
      }
      const ta = h('textarea', { rows: 5, placeholder: 'network:\n  name: mon-reseau\ndevices: …', 'aria-label': 'Topologie YAML' });
      root.append(h('details', null,
        h('summary', null, 'Coller un YAML'),
        h('div', { class: 'form', style: 'padding:0 10px 10px' }, ta,
          libButton('Ouvrir', () => createLab({ yaml: ta.value })))));
    } else if (state.libTab === 'challenges') {
      for (const diff of ['easy', 'medium', 'hard']) {
        const list = lib.challenges.filter((c) => c.difficulty === diff);
        if (!list.length) continue;
        root.append(h('h3', null, { easy: 'Débutant', medium: 'Intermédiaire', hard: 'Avancé' }[diff]));
        for (const c of list) {
          root.append(h('div', { class: 'item' },
            h('div', { class: 'name' }, c.title,
              h('small', null, h('span', { class: `pill ${diff}` }, diff), h('span', { class: 'pill' }, c.topic), `${c.minutes} min`)),
            done.has(c.id) ? h('span', { class: 'done', title: `Réussi : ${done.get(c.id)}/100` }, `✓${done.get(c.id)}`) : null,
            libButton(done.has(c.id) ? 'Rejouer' : 'Jouer', () => createLab({ challenge: c.id }))));
        }
      }
    } else if (state.libTab === 'ranges') {
      root.append(h('p', { class: 'muted' }, "Un incident a déjà eu lieu : enquête avec la forensique, puis corrige sans couper le trafic légitime."));
      for (const r of lib.ranges) {
        root.append(h('div', { class: 'item' },
          h('div', { class: 'name' }, r.title, h('small', null, h('span', { class: `pill ${r.difficulty}` }, r.difficulty), `${r.minutes} min`)),
          done.has(r.id) ? h('span', { class: 'done' }, `✓${done.get(r.id)}`) : null,
          libButton('Enquêter', () => createLab({ range: r.id }))));
      }
    } else {
      renderGenerate(root, lib);
    }
  }

  function renderGenerate(root, lib) {
    const type = h('select', { 'aria-label': 'Type de réseau' }, lib.generator.types.map((t) => h('option', { value: t }, t)));
    type.value = 'campus';
    const routers = h('input', { type: 'number', min: 0, max: 40, value: 5, 'aria-label': 'Routeurs' });
    const hosts = h('input', { type: 'number', min: 1, max: 300, value: 12, 'aria-label': 'Postes' });
    const seed = h('input', { type: 'number', min: 0, value: 1, 'aria-label': 'Graine' });
    root.append(
      h('h3', null, 'Générer un réseau'),
      h('div', { class: 'form' },
        h('label', null, 'Type', type),
        h('label', null, 'Routeurs', routers),
        h('label', null, 'Postes', hosts),
        h('label', null, 'Graine', seed),
        libButton('Générer et ouvrir', () => createLab({
          generate: { type: type.value, routers: +routers.value, hosts: +hosts.value, seed: +seed.value },
        }), 'primary')),
    );
    const topic = h('select', { 'aria-label': 'Thème' }, lib.generator.topics.map((t) => h('option', { value: t }, t)));
    topic.value = 'routing';
    const diff = h('select', { 'aria-label': 'Difficulté' },
      [['easy', 'facile'], ['medium', 'moyen'], ['hard', 'difficile']].map(([v, l]) => h('option', { value: v }, l)));
    diff.value = 'medium';
    const cseed = h('input', { type: 'number', min: 0, value: 1, 'aria-label': 'Graine du défi' });
    root.append(
      h('h3', null, 'Générer un défi'),
      h('div', { class: 'form' },
        h('label', null, 'Thème', topic),
        h('label', null, 'Difficulté', diff),
        h('label', null, 'Graine', cseed),
        libButton('Générer et jouer', () => createLab({
          generate_challenge: { topic: topic.value, difficulty: diff.value, seed: +cseed.value },
        }), 'primary')),
    );
  }

  // ------------------------------------------------------------------ schéma

  /** Disposition radiale : centre du graphe au milieu, couches en cercles. */
  function layout(devices, links) {
    const names = devices.map((d) => d.name);
    const adj = new Map(names.map((n) => [n, []]));
    for (const l of links) {
      if (adj.has(l.a_device) && adj.has(l.b_device) && l.a_device !== l.b_device) {
        adj.get(l.a_device).push(l.b_device);
        adj.get(l.b_device).push(l.a_device);
      }
    }
    for (const list of adj.values()) list.sort();

    const bfs = (root) => {
      const dist = new Map([[root, 0]]);
      const q = [root];
      while (q.length) {
        const x = q.shift();
        for (const y of adj.get(x)) if (!dist.has(y)) { dist.set(y, dist.get(x) + 1); q.push(y); }
      }
      return dist;
    };

    // Composantes connexes.
    const seen = new Set();
    const comps = [];
    for (const n of names) {
      if (seen.has(n)) continue;
      const d = bfs(n);
      const members = Array.from(d.keys());
      members.forEach((m) => seen.add(m));
      comps.push(members);
    }

    const pos = new Map();
    let offsetX = 0;
    for (const members of comps) {
      // Centre = plus petite excentricité (puis plus grand degré, puis nom).
      let best = null;
      for (const m of members) {
        const ecc = Math.max(...bfs(m).values());
        const key = [ecc, -adj.get(m).length, m];
        if (!best || key[0] < best.key[0] || (key[0] === best.key[0] && (key[1] < best.key[1] || (key[1] === best.key[1] && key[2] < best.key[2])))) best = { m, key };
      }
      const root = best.m;
      const dist = bfs(root);
      const children = new Map(members.map((m) => [m, []]));
      for (const m of members) {
        if (m === root) continue;
        const parent = adj.get(m).filter((p) => dist.get(p) === dist.get(m) - 1).sort()[0];
        children.get(parent).push(m);
      }
      const leaves = new Map();
      const countLeaves = (n) => {
        const c = children.get(n);
        const v = c.length ? c.reduce((a, k) => a + countLeaves(k), 0) : 1;
        leaves.set(n, v);
        return v;
      };
      const total = countLeaves(root);
      const maxLayer = Math.max(...dist.values());
      const place = (n, start, span) => {
        const mid = start + span / 2;
        const r = dist.get(n);
        const ang = 2 * Math.PI * mid - Math.PI / 2;
        pos.set(n, { x: r * Math.cos(ang), y: r * Math.sin(ang), layer: r });
        let at = start;
        for (const k of children.get(n)) {
          const w = (leaves.get(k) / leaves.get(n)) * span;
          place(k, at, w);
          at += w;
        }
      };
      pos.set(root, { x: 0, y: 0, layer: 0 });
      let at = 0;
      for (const k of children.get(root)) {
        const w = leaves.get(k) / total;
        place(k, at, w);
        at += w;
      }
      const extent = Math.max(1, maxLayer);
      for (const m of members) {
        const p = pos.get(m);
        p.x = offsetX + (p.x / extent);
        p.y = p.y / extent;
      }
      offsetX += 2.6;
    }
    return pos;
  }

  function fitToView(pos, W, H, pad) {
    const xs = Array.from(pos.values(), (p) => p.x);
    const ys = Array.from(pos.values(), (p) => p.y);
    const minX = Math.min(...xs), maxX = Math.max(...xs);
    const minY = Math.min(...ys), maxY = Math.max(...ys);
    const spanX = Math.max(maxX - minX, 0.001), spanY = Math.max(maxY - minY, 0.001);
    const scale = Math.min((W - 2 * pad) / spanX, (H - 2 * pad) / spanY);
    const cx = (minX + maxX) / 2, cy = (minY + maxY) / 2;
    const out = new Map();
    for (const [n, p] of pos) {
      out.set(n, { x: W / 2 + (p.x - cx) * scale, y: H / 2 + (p.y - cy) * scale });
    }
    return out;
  }

  const VIEW_W = 1000;
  const VIEW_H = 560;

  /** Coordonnées arrondies, au format du serveur (`[x, y]`). */
  const asPair = (p) => [Math.round(p.x), Math.round(p.y)];

  /**
   * Positions des équipements. Priorité : celles du serveur (enregistrées
   * dans le lab), puis celles mémorisées par le navigateur, puis la
   * disposition automatique, calculée seulement pour les équipements qui
   * n'en ont pas : ajouter un équipement ne déplace pas les autres.
   */
  function syncPositions(snap) {
    const key = `netiv.pos.${snap.name}`;
    if (state.posKey !== key) {
      const saved = store(key) || {};
      state.positions = new Map(Object.entries(saved).filter(([, p]) => p && Number.isFinite(p.x)));
      state.posKey = key;
      const v = store(`netiv.view.${key}`);
      state.view = v && v.w > 0 ? v : defaultView();
    }
    const names = new Set(snap.devices.map((d) => d.name));
    for (const n of Array.from(state.positions.keys())) if (!names.has(n)) state.positions.delete(n);
    const missing = [];
    for (const d of snap.devices) {
      if (Array.isArray(d.pos)) state.positions.set(d.name, { x: d.pos[0], y: d.pos[1] });
      else if (!state.positions.has(d.name)) missing.push(d.name);
    }
    if (missing.length) {
      const auto = fitToView(layout(snap.devices, snap.links), VIEW_W, VIEW_H, 70);
      for (const n of missing) {
        const p = auto.get(n) || { x: VIEW_W / 2, y: VIEW_H / 2 };
        state.positions.set(n, { x: Math.round(p.x), y: Math.round(p.y) });
      }
    }
  }

  /** Mémorise les positions dans le navigateur, et (en différé) dans le lab. */
  let posTimer = null;
  const posQueue = {};
  function savePositions(names) {
    const o = {};
    for (const [n, p] of state.positions) o[n] = { x: Math.round(p.x), y: Math.round(p.y) };
    if (state.posKey) store(state.posKey, o);
    for (const n of names) {
      const p = state.positions.get(n);
      if (p) posQueue[n] = asPair(p);
    }
    if (state.build.on) {
      for (const def of [state.build.def, state.build.draft]) {
        if (!def) continue;
        for (const d of def.devices) if (posQueue[d.name]) d.pos = posQueue[d.name];
      }
    }
    clearTimeout(posTimer);
    const lab = state.current;
    posTimer = setTimeout(() => {
      const positions = { ...posQueue };
      for (const k of Object.keys(posQueue)) delete posQueue[k];
      if (lab && Object.keys(positions).length) {
        api('PUT', `/api/labs/${lab}/positions`, { positions }).catch(() => { /* le navigateur les garde */ });
      }
    }, 400);
  }

  // ------------------------------------------------------------------ zoom et déplacement du schéma

  const MIN_W = VIEW_W / 8;   // zoom maximal ×8
  const MAX_W = VIEW_W * 4;   // zoom minimal ×¼

  function defaultView() {
    return { x: 0, y: 0, w: VIEW_W, h: VIEW_H };
  }

  function applyView() {
    const v = state.view;
    $('#canvas').setAttribute('viewBox', `${v.x} ${v.y} ${v.w} ${v.h}`);
    $('#z-level').textContent = `${Math.round((VIEW_W / v.w) * 100)} %`;
  }

  let viewSaveTimer = null;
  function saveView() {
    clearTimeout(viewSaveTimer);
    viewSaveTimer = setTimeout(() => { if (state.posKey) store(`netiv.view.${state.posKey}`, state.view); }, 300);
  }

  /** Zoome d'un facteur `k` autour du point (cx, cy) du schéma. */
  function zoomAt(k, cx, cy) {
    const v = state.view;
    const w = Math.max(MIN_W, Math.min(MAX_W, v.w / k));
    const r = w / v.w;
    const h = w * (VIEW_H / VIEW_W);
    if (cx === undefined) { cx = v.x + v.w / 2; cy = v.y + v.h / 2; }
    state.view = { x: cx - (cx - v.x) * r, y: cy - (cy - v.y) * r, w, h };
    applyView();
    saveView();
  }

  /** Cadre la vue sur tous les équipements. */
  function fitView() {
    if (!state.positions.size) { state.view = defaultView(); applyView(); saveView(); return; }
    const pts = Array.from(state.positions.values());
    const pad = 70;
    const minX = Math.min(...pts.map((p) => p.x)) - pad, maxX = Math.max(...pts.map((p) => p.x)) + pad;
    const minY = Math.min(...pts.map((p) => p.y)) - pad, maxY = Math.max(...pts.map((p) => p.y)) + pad;
    let w = Math.max(maxX - minX, MIN_W), h = Math.max(maxY - minY, MIN_W * (VIEW_H / VIEW_W));
    // Même proportions que la zone d'affichage.
    if (w / h > VIEW_W / VIEW_H) h = w * (VIEW_H / VIEW_W); else w = h * (VIEW_W / VIEW_H);
    w = Math.min(w, MAX_W); h = w * (VIEW_H / VIEW_W);
    state.view = { x: (minX + maxX - w) / 2, y: (minY + maxY - h) / 2, w, h };
    applyView();
    saveView();
  }

  /** Recalcule la disposition automatique et oublie les positions enregistrées. */
  function relayout() {
    const snap = state.snap;
    if (!snap) return;
    const auto = fitToView(layout(snap.devices, snap.links), VIEW_W, VIEW_H, 70);
    state.positions = new Map(Array.from(auto, ([n, p]) => [n, { x: Math.round(p.x), y: Math.round(p.y) }]));
    // Les positions du lab doivent suivre, sinon elles reprendraient le dessus.
    for (const d of snap.devices) d.pos = asPair(state.positions.get(d.name));
    savePositions(snap.devices.map((d) => d.name));
    renderCanvas();
    fitView();
  }

  function initCanvasControls() {
    const svg = $('#canvas');
    svg.addEventListener('wheel', (e) => {
      if (!state.snap) return;
      e.preventDefault();
      const p = svgPoint(svg, e);
      zoomAt(Math.exp(-e.deltaY * 0.0015), p.x, p.y);
    }, { passive: false });

    // Glisser le fond : déplacer la vue.
    let pan = null;
    svg.addEventListener('pointerdown', (e) => {
      if (e.button !== 0 || e.target.closest('.node, .link-hit') || !state.snap) return;
      const m = svg.getScreenCTM();
      if (!m) return;
      pan = { sx: e.clientX, sy: e.clientY, view: { ...state.view }, scale: m.a, moved: false };
      svg.setPointerCapture(e.pointerId);
      svg.classList.add('panning');
    });
    svg.addEventListener('pointermove', (e) => {
      if (!pan) return;
      if (Math.abs(e.clientX - pan.sx) + Math.abs(e.clientY - pan.sy) > 3) pan.moved = true;
      state.view = {
        ...pan.view,
        x: pan.view.x - (e.clientX - pan.sx) / pan.scale,
        y: pan.view.y - (e.clientY - pan.sy) / pan.scale,
      };
      applyView();
    });
    const endPan = () => {
      if (!pan) return;
      // Un clic (sans déplacement) dans le vide désélectionne.
      if (!pan.moved && state.build.on) {
        state.build.cableFrom = null;
        buildSelect(null);
      }
      pan = null;
      svg.classList.remove('panning');
      saveView();
    };
    svg.addEventListener('pointerup', endPan);
    svg.addEventListener('pointercancel', endPan);

    $('#z-in').addEventListener('click', () => zoomAt(1.25));
    $('#z-out').addEventListener('click', () => zoomAt(0.8));
    $('#z-fit').addEventListener('click', fitView);
    $('#z-relayout').addEventListener('click', relayout);
  }

  function glyph(kind) {
    const p = (d) => s('path', { class: 'glyph', d });
    switch (kind) {
      case 'host': return [p('M-10 -7h20v13h-20z'), p('M-4 10h8M0 6v4')];
      case 'server': return [p('M-10 -9h20v7h-20zM-10 0h20v7h-20z'), p('M-6 -5.5h1M-6 3.5h1')];
      case 'router': return [p('M0 -11v22M-11 0h22'), p('M-4 -7l4 -4 4 4M-4 7l4 4 4 -4M-7 -4l-4 4 4 4M7 -4l4 4 -4 4')];
      case 'switch': return [p('M-11 -4h22v8h-22z'), p('M-7 0h1M-2 0h1M3 0h1M8 0h1')];
      case 'firewall': return [p('M-10 -9h20v18h-20z'), p('M-10 -3h20M-10 3h20M-3 -9v6M4 -3v6M-3 3v6')];
      default: return [p('M-8 -8h16v16h-16z')];
    }
  }

  function subLabel(d, big) {
    if (big) return '';
    const withIp = d.ifaces.filter((i) => i.ip);
    if (d.kind === 'host' || d.kind === 'server') return withIp[0] ? withIp[0].ip.replace(/\/\d+$/, '') : '';
    if (d.kind === 'switch') return `${d.ifaces.length} ports`;
    return `${withIp.length} interface${withIp.length > 1 ? 's' : ''}`;
  }

  function renderCanvas() {
    const svg = clear($('#canvas'));
    const snap = state.snap;
    svg.setAttribute('preserveAspectRatio', 'xMidYMid meet');
    const empty = !(snap && snap.devices.length);
    $('#canvas-empty').hidden = !empty;
    const emptyText = clear($('#canvas-empty-text'));
    if (state.build.on) {
      emptyText.append(h('strong', null, 'Réseau vide'), ' : clique sur un équipement de la palette, ou glisse-le ici.');
    } else {
      emptyText.append(h('strong', null, 'Choisis un lab'), ' dans le catalogue pour voir son schéma.');
    }
    if (snap) syncPositions(snap);
    applyView();
    if (empty) return;
    const big = snap.devices.length > 40;
    const scale = snap.devices.length > 90 ? 0.55 : snap.devices.length > 40 ? 0.75 : 1;

    const linkLayer = s('g', { class: 'links' });
    const nodeLayer = s('g', { class: 'nodes' });
    const overlay = s('g', { class: 'overlay' });
    svg.append(linkLayer, nodeLayer, overlay);

    for (const l of snap.links) {
      const a = state.positions.get(l.a_device), b = state.positions.get(l.b_device);
      if (!a || !b) continue;
      const cls = ['link', l.up ? '' : 'down', l.loss > 0 ? 'lossy' : ''].filter(Boolean).join(' ');
      const line = s('line', { class: cls, x1: a.x, y1: a.y, x2: b.x, y2: b.y, 'data-a': l.a_device, 'data-b': l.b_device });
      const extra = [l.up ? null : 'coupé', l.loss > 0 ? `perte ${(l.loss * 100).toFixed(0)} %` : null, `latence ${(l.latency_us / 1000).toFixed(1)} ms`].filter(Boolean);
      line.append(s('title', null, `${l.a} ↔ ${l.b} — ${extra.join(', ')}`));
      if (state.build.on) {
        const sel = state.build.sel;
        if (sel && sel.kind === 'link' && sel.a === l.a && sel.b === l.b) line.classList.add('selected');
        // Zone de clic plus large que le trait.
        const hit = s('line', { class: 'link-hit', x1: a.x, y1: a.y, x2: b.x, y2: b.y, 'data-a': l.a_device, 'data-b': l.b_device });
        hit.addEventListener('click', (e) => { e.stopPropagation(); buildSelect({ kind: 'link', a: l.a, b: l.b }); });
        linkLayer.append(line, hit);
      } else {
        linkLayer.append(line);
      }
    }

    for (const d of snap.devices) {
      const p = state.positions.get(d.name);
      if (!p) continue;
      const anyDown = d.ifaces.some((i) => !i.up || i.link_up === false);
      const b = state.build;
      const selected = b.on ? (b.sel && b.sel.kind === 'device' && b.sel.name === d.name) : state.device === d.name;
      const issue = b.on ? (b.issueDevices.err.has(d.name) ? ' issue-err' : b.issueDevices.warn.has(d.name) ? ' issue-warn' : '') : '';
      const g = s('g', {
        class: `node kind-${d.kind} role-${d.role}${selected ? ' selected' : ''}${anyDown ? ' faulty' : ''}${issue}${b.on && b.cableFrom === d.name ? ' cable-from' : ''}`,
        transform: `translate(${p.x} ${p.y}) scale(${scale})`,
        'data-name': d.name, tabindex: 0, role: 'button', 'aria-label': `${d.name}, ${d.kind}${anyDown ? ', interface ou câble hors service' : ''}`,
      });
      const shape = d.kind === 'router'
        ? s('circle', { class: 'shape', r: 27 })
        : d.kind === 'switch'
          ? s('rect', { class: 'shape', x: -34, y: -17, width: 68, height: 34, rx: 7 })
          : s('rect', { class: 'shape', x: -29, y: -22, width: 58, height: 44, rx: 9 });
      g.append(s('circle', { class: 'ring', r: 36 }), shape, ...glyph(d.kind));
      g.append(s('text', { y: 42 }, d.name));
      const sub = subLabel(d, big);
      if (sub) g.append(s('text', { class: 'sub', y: 55 }, sub));
      const roles = [d.dhcp_server ? 'DHCP' : null, d.dns_server ? 'DNS' : null].filter(Boolean);
      if (roles.length) g.append(s('text', { class: 'svc-tag', y: -30 }, roles.join(' · ')));
      if (anyDown) g.append(s('circle', { class: 'badge', cx: 24, cy: -18, r: 6 }), s('text', { x: 24, y: -14.5, style: 'fill:#fff;font-size:10px;font-weight:700' }, '!'));
      g.append(s('title', null, `${d.name} (${d.kind}, rôle ${d.role})`));
      attachNode(g, d.name);
      nodeLayer.append(g);
    }
    highlightStep();
  }

  function svgPoint(svg, evt) {
    const pt = svg.createSVGPoint();
    pt.x = evt.clientX; pt.y = evt.clientY;
    const m = svg.getScreenCTM();
    return m ? pt.matrixTransform(m.inverse()) : { x: evt.clientX, y: evt.clientY };
  }

  /** Déplace un équipement et les extrémités de ses câbles, sans tout redessiner. */
  function moveNode(g, name, p) {
    state.positions.set(name, p);
    const t = g.getAttribute('transform') || '';
    g.setAttribute('transform', t.replace(/translate\([^)]*\)/, `translate(${p.x} ${p.y})`));
    for (const l of $$('#canvas .link, #canvas .link-hit')) {
      if (l.dataset.a === name) { l.setAttribute('x1', p.x); l.setAttribute('y1', p.y); }
      if (l.dataset.b === name) { l.setAttribute('x2', p.x); l.setAttribute('y2', p.y); }
    }
  }

  function attachNode(g, name) {
    const svg = $('#canvas');
    let drag = null;
    g.addEventListener('pointerdown', (e) => {
      if (e.button !== 0) return;
      drag = { moved: false, sx: e.clientX, sy: e.clientY };
      g.setPointerCapture(e.pointerId);
    });
    g.addEventListener('pointermove', (e) => {
      if (!drag) return;
      if (!drag.moved && Math.hypot(e.clientX - drag.sx, e.clientY - drag.sy) < 4) return;
      drag.moved = true;
      const p = svgPoint(svg, e);
      const lim = (v, max) => Math.max(-max, Math.min(2 * max, v));
      moveNode(g, name, { x: lim(p.x, VIEW_W), y: lim(p.y, VIEW_H) });
    });
    const end = () => {
      if (!drag) return;
      if (drag.moved) {
        savePositions([name]);
      } else if (state.build.on) {
        buildNodeClick(name);
      } else {
        selectDevice(name);
      }
      drag = null;
    };
    g.addEventListener('pointerup', end);
    g.addEventListener('pointercancel', () => { drag = null; });
    g.addEventListener('keydown', (e) => {
      if (e.key === 'Enter' || e.key === ' ') {
        e.preventDefault();
        if (state.build.on) buildNodeClick(name); else selectDevice(name);
      }
    });
  }

  // ------------------------------------------------------------------ en-tête du schéma, barre d'outils

  function renderStage() {
    const snap = state.snap;
    $('#lab-title').textContent = snap && snap.name ? snap.name : 'Aucun réseau';
    $('#lab-meta').textContent = snap && snap.name
      ? `${snap.devices.length} équipements · ${snap.links.length} câbles · t = ${snap.time}${snap.pending ? ` · ${snap.pending} en attente` : ''}`
      : '';
    renderToolbar();
    renderCanvas();
  }

  function renderToolbar() {
    const snap = state.snap;
    const pending = !!(snap && snap.pending);
    for (const id of ['#tb-sep', '#tb-step', '#tb-run']) $(id).hidden = !pending;
    const src = $('#tb-src'), dst = $('#tb-dst');
    const keepSrc = src.value, keepDst = dst.value;
    clear(src); clear(dst);
    if (!snap) return;
    const ends = snap.devices.filter((d) => d.kind === 'host' || d.kind === 'server' || d.kind === 'router' || d.kind === 'firewall');
    for (const d of ends) src.append(h('option', { value: d.name }, d.name));
    for (const d of snap.devices) {
      const ip = (d.ifaces.find((i) => i.ip) || {}).ip;
      if (ip) dst.append(h('option', { value: ip.replace(/\/\d+$/, '') }, `${d.name} (${ip.replace(/\/\d+$/, '')})`));
    }
    if (keepSrc && ends.some((d) => d.name === keepSrc)) src.value = keepSrc;
    if (keepDst && Array.from(dst.options).some((o) => o.value === keepDst)) dst.value = keepDst;
    else if (dst.options.length > 1) dst.selectedIndex = dst.options.length - 1;
  }

  async function sendProbe() {
    const src = $('#tb-src').value, dst = $('#tb-dst').value, port = $('#tb-port').value.trim();
    if (!src || !dst) throw new Error('Choisis une source et une destination.');
    let line;
    if (port) {
      if (!/^\d{1,5}$/.test(port)) throw new Error('Port invalide.');
      line = `curl ${src} ${dst} ${port}`;
    } else {
      line = `ping ${src} ${dst}${$('#tb-stepmode').checked ? ' 1 --step' : ''}`;
    }
    await runCommand(line);
  }

  // ------------------------------------------------------------------ panneaux

  function selectTab(name) {
    state.tab = name;
    for (const b of $$('.tabs button')) b.setAttribute('aria-selected', String(b.dataset.tab === name));
    for (const id of ['device', 'journey', 'trace', 'analysis', 'challenge']) $(`#tab-${id}`).hidden = id !== name;
    if (name === 'trace') guarded(loadTrace);
    if (name === 'device' && state.device) guarded(() => loadDevice(state.device));
  }

  // ------------------------------------------------------------------ mode Construire (éditeur)
  //
  // L'éditeur travaille sur la description du réseau (TopologyDef, le format
  // des fichiers YAML en JSON). Chaque modification envoie la description
  // complète au serveur, qui la valide puis reconstruit le lab : une seule
  // requête par modification, et la même validation que pour un fichier.

  const KIND_PREFIX = { host: 'PC', server: 'SRV', router: 'R', firewall: 'FW', switch: 'SW' };
  const KIND_LABEL = { host: 'Poste', server: 'Serveur', router: 'Routeur', firewall: 'Pare-feu', switch: 'Switch' };
  const ROLES = ['user', 'server', 'dmz', 'internet', 'network', 'guest', 'iot'];
  const HISTORY_MAX = 100;
  const isIp = (kind) => kind !== 'switch';
  const isRouter = (kind) => kind === 'router' || kind === 'firewall';

  /** Copie profonde d'une topologie (données JSON pures). */
  const cloneDef = (def) => JSON.parse(JSON.stringify(def));

  /** La topologie affichée dans l'inspecteur : le brouillon s'il existe. */
  const curDef = () => state.build.draft || state.build.def;

  function devOf(def, name) {
    return def.devices.find((d) => d.name === name);
  }

  /** Interfaces d'un équipement, ports implicites (`ports: N`) compris. */
  function ifaceNames(d) {
    const names = [];
    if (d.type === 'switch' && d.ports) for (let i = 1; i <= d.ports; i++) names.push(`port${i}`);
    for (const i of d.interfaces || []) names.push(i.name);
    return names;
  }

  /** Interfaces déjà câblées, sous la forme « Équipement:interface ». */
  function usedEndpoints(def) {
    const used = new Set();
    for (const l of def.links) { used.add(l.a); used.add(l.b); }
    return used;
  }

  function nextName(def, prefix) {
    const taken = new Set(def.devices.map((d) => d.name));
    for (let i = 1; ; i++) if (!taken.has(`${prefix}${i}`)) return `${prefix}${i}`;
  }

  function nextIfaceName(d, prefix) {
    const taken = new Set(ifaceNames(d));
    for (let i = d.type === 'switch' ? 1 : 0; ; i++) if (!taken.has(`${prefix}${i}`)) return `${prefix}${i}`;
  }

  /** Nouvel équipement prêt à câbler. */
  function newDevice(def, kind, pos) {
    const d = { name: nextName(def, KIND_PREFIX[kind] || 'X'), type: kind, interfaces: [], pos: asPair(pos) };
    const count = { host: 1, server: 1, router: 2, firewall: 3, switch: 8 }[kind] || 1;
    for (let i = 0; i < count; i++) {
      d.interfaces.push({ name: kind === 'switch' ? `port${i + 1}` : `eth${i}` });
    }
    return d;
  }

  /** Première interface libre de `d` ; en ajoute une si toutes sont prises. */
  function freeIface(def, d) {
    const used = usedEndpoints(def);
    const free = ifaceNames(d).find((n) => !used.has(`${d.name}:${n}`));
    if (free) return free;
    const name = nextIfaceName(d, d.type === 'switch' ? 'port' : 'eth');
    d.interfaces = d.interfaces || [];
    d.interfaces.push({ name });
    return name;
  }

  // ---- entrer / sortir

  async function enterBuild() {
    if (!state.current || !state.snap) throw new Error("Ouvre d'abord un lab (ou crée un réseau vide dans le catalogue).");
    if (state.snap.editable === false) throw new Error(state.snap.edit_lock || 'Ce lab ne peut pas être modifié.');
    const r = await api('GET', `/api/labs/${state.current}/topology`);
    const b = state.build;
    Object.assign(b, {
      on: true, def: r.topology, draft: null, past: [], future: [], tool: 'select',
      cableFrom: null, sel: null, errors: [], warnings: [],
    });
    // Les équipements sans position prennent celle affichée, pour que la
    // disposition soit enregistrée avec le lab.
    const missing = [];
    for (const d of b.def.devices) {
      if (!d.pos && state.positions.has(d.name)) { d.pos = asPair(state.positions.get(d.name)); missing.push(d.name); }
    }
    if (missing.length) savePositions(missing);
    await validateCurrent();
    renderBuild();
    selectTab('device');
  }

  function exitBuild() {
    const b = state.build;
    if (!b.on) return;
    b.on = false;
    b.draft = null;
    b.sel = null;
    b.cableFrom = null;
    renderBuild();
    renderDevice(null);
    if (state.device && state.snap && state.snap.devices.some((d) => d.name === state.device)) guarded(() => loadDevice(state.device));
  }

  async function toggleBuild() {
    if (state.build.on) exitBuild(); else await enterBuild();
  }

  // ---- appliquer une modification

  /** Messages d'erreur renvoyés par le serveur (« topologie invalide :\n- … »). */
  function parseErrors(message) {
    const parts = String(message).split('\n- ');
    return parts.length > 1 ? parts.slice(1) : [String(message)];
  }

  /** Équipements cités par un message (mot entier). */
  function devicesIn(messages, def) {
    const found = new Set();
    const names = (def ? def.devices : []).map((d) => d.name).sort((a, b) => b.length - a.length);
    for (const m of messages) {
      for (const n of names) {
        const esc = n.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
        if (new RegExp(`(^|[^\\w-])${esc}([^\\w-]|$)`).test(m)) found.add(n);
      }
    }
    return found;
  }

  function setIssues(errors, warnings) {
    const b = state.build;
    b.errors = errors;
    b.warnings = warnings;
    const def = curDef();
    b.issueDevices = { err: devicesIn(errors, def), warn: devicesIn(warnings, def) };
  }

  async function validateCurrent() {
    const r = await api('POST', `/api/labs/${state.current}/topology/validate`, { topology: curDef() });
    setIssues(r.errors, r.warnings);
  }

  /**
   * Applique une topologie. Valide : le lab est reconstruit et l'ancienne
   * version part dans l'historique. Invalide : elle reste en brouillon, le
   * lab garde sa dernière version valide.
   */
  async function applyDef(next, { record = true } = {}) {
    const b = state.build;
    const lab = state.current;
    try {
      const r = await api('PUT', `/api/labs/${lab}/topology`, { topology: next });
      if (record && b.def) {
        b.past.push(b.def);
        if (b.past.length > HISTORY_MAX) b.past.shift();
        b.future = [];
      }
      b.def = next;
      b.draft = null;
      setIssues([], r.warnings || []);
      // La trace repart de zéro : les parcours affichés ne valent plus.
      state.journey = null;
      state.tx = [];
      state.step = -1;
      applySnapshot(r.snapshot);
      renderJourney();
    } catch (e) {
      if (e.status === 409) { exitBuild(); throw e; }
      if (e.status !== 400) throw e;
      b.draft = next;
      setIssues(parseErrors(e.message), []);
      renderCanvas();
    }
    renderBuild();
  }

  /** Modifie une copie de la topologie courante puis l'applique. */
  async function commit(mutate) {
    const next = cloneDef(curDef());
    if (mutate(next) === false) return;
    await applyDef(next);
  }

  /** Les modifications passent l'une après l'autre (pas de requêtes croisées). */
  let commitChain = Promise.resolve();
  function edit(mutate) {
    commitChain = commitChain
      .then(() => commit(mutate))
      .catch((e) => { banner(e.message); consoleLine(e.message, 'err'); });
    return commitChain;
  }

  async function undo() {
    const b = state.build;
    if (b.draft) { b.draft = null; await validateCurrent(); renderCanvas(); renderBuild(); return; }
    if (!b.past.length) return;
    const prev = b.past.pop();
    b.future.push(b.def);
    await applyDef(prev, { record: false });
  }

  async function redo() {
    const b = state.build;
    if (!b.future.length) return;
    const next = b.future.pop();
    b.past.push(b.def);
    await applyDef(next, { record: false });
  }

  // ---- actions

  /**
   * Ajoute un équipement à `pos` (coordonnées du schéma) ; sans `pos`, à un
   * emplacement libre, calculé quand la modification s'exécute (après les
   * précédentes) pour que des clics rapides ne superposent pas les équipements.
   */
  function addDevice(kind, pos) {
    let name = null;
    return edit((def) => {
      const d = newDevice(def, kind, pos || dropPoint());
      name = d.name;
      def.devices.push(d);
    }).then(() => { if (name && devOf(curDef(), name)) buildSelect({ kind: 'device', name }); });
  }

  /**
   * Emplacement libre près du centre de la vue (ajout par clic) : on parcourt
   * une grille en spirale et on prend la première case assez loin de tous
   * les équipements existants.
   */
  function dropPoint() {
    const v = state.view;
    const cx = v.x + v.w / 2, cy = v.y + v.h / 2;
    const STEP = 110, MIN = 95;
    const taken = Array.from(state.positions.values());
    const free = (p) => taken.every((q) => Math.hypot(q.x - p.x, q.y - p.y) >= MIN);
    for (let ring = 0; ring < 12; ring++) {
      for (let dx = -ring; dx <= ring; dx++) {
        for (let dy = -ring; dy <= ring; dy++) {
          if (Math.max(Math.abs(dx), Math.abs(dy)) !== ring) continue;
          const p = { x: cx + dx * STEP, y: cy + dy * STEP * 0.8 };
          if (free(p)) return p;
        }
      }
    }
    return { x: cx, y: cy };
  }

  function connect(a, b) {
    if (a === b) return Promise.resolve();
    return edit((def) => {
      const da = devOf(def, a), db = devOf(def, b);
      if (!da || !db) return false;
      const ia = freeIface(def, da);
      const ib = freeIface(def, db);
      def.links.push({ a: `${a}:${ia}`, b: `${b}:${ib}` });
      return true;
    });
  }

  function deleteSelection() {
    const sel = state.build.sel;
    if (!sel) return Promise.resolve();
    state.build.sel = null;
    return edit((def) => {
      if (sel.kind === 'device') {
        def.devices = def.devices.filter((d) => d.name !== sel.name);
        def.links = def.links.filter((l) => !l.a.startsWith(`${sel.name}:`) && !l.b.startsWith(`${sel.name}:`));
      } else {
        const i = def.links.findIndex((l) => l.a === sel.a && l.b === sel.b);
        if (i < 0) return false;
        def.links.splice(i, 1);
      }
      return true;
    });
  }

  function buildSelect(sel) {
    state.build.sel = sel;
    renderCanvas();
    renderInspector();
  }

  function buildNodeClick(name) {
    const b = state.build;
    if (b.tool === 'cable') {
      if (!b.cableFrom) { b.cableFrom = name; renderCanvas(); renderIssues(); return; }
      const from = b.cableFrom;
      b.cableFrom = null;
      connect(from, name);
      return;
    }
    buildSelect({ kind: 'device', name });
  }

  function setTool(tool) {
    state.build.tool = tool;
    state.build.cableFrom = null;
    renderCanvas();
    renderBuildBar();
    renderIssues();
  }

  async function download(format) {
    const r = await api('GET', `/api/labs/${requireLab()}/topology?format=${format}`);
    const blob = new Blob([r.text], { type: 'text/plain;charset=utf-8' });
    const a = h('a', { href: URL.createObjectURL(blob), download: `${(state.snap && state.snap.name) || 'reseau'}.${format}` });
    document.body.append(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(a.href), 1000);
  }

  async function autoAddress() {
    const r = await api('POST', `/api/labs/${requireLab()}/topology/autoconfig`, {});
    if (!r.changes.length) {
      banner("Rien à compléter : toutes les interfaces câblées ont déjà une adresse.", 'info');
      return;
    }
    const b = state.build;
    b.past.push(b.def);
    b.future = [];
    b.def = (await api('GET', `/api/labs/${state.current}/topology`)).topology;
    b.draft = null;
    applySnapshot(r.snapshot);
    await validateCurrent();
    consoleLine(`Adressage automatique (${r.changes.length} changement(s)) :\n  ${r.changes.join('\n  ')}`, 'note');
    banner(`${r.changes.length} changement(s) — détail dans la console.`, 'info');
    renderBuild();
  }

  async function newEmptyNetwork() {
    const taken = new Set(state.labs.map((l) => l.name));
    let name = 'nouveau-reseau';
    for (let i = 2; taken.has(name); i++) name = `nouveau-reseau-${i}`;
    await createLab({ empty: name });
    await enterBuild();
  }

  async function importFile(file) {
    if (file.size > 1024 * 1024) throw new Error('Fichier trop gros (1 Mio au plus).');
    const text = await file.text();
    await createLab({ yaml: text });
  }

  // ---- rendu

  function renderBuild() {
    const on = state.build.on;
    $('#buildbar').hidden = !on;
    document.body.classList.toggle('building', on);
    updateBuildToggle();
    renderBuildBar();
    renderIssues();
    renderCanvas();
    renderInspector();
  }

  function renderBuildBar() {
    const b = state.build;
    if (!b.on) return;
    $('#bt-select').setAttribute('aria-pressed', String(b.tool === 'select'));
    $('#bt-cable').setAttribute('aria-pressed', String(b.tool === 'cable'));
    $('#bt-undo').disabled = !b.past.length && !b.draft;
    $('#bt-redo').disabled = !b.future.length;
    $('#bt-delete').disabled = !b.sel;
    $('#canvas').classList.toggle('cabling', b.tool === 'cable');
  }

  function renderIssues() {
    const b = state.build;
    const box = clear($('#build-issues'));
    const show = b.on && (b.draft || b.errors.length || b.warnings.length || b.tool === 'cable');
    box.hidden = !show;
    if (!show) return;
    if (b.tool === 'cable') {
      box.append(h('p', { class: 'hint' }, b.cableFrom
        ? `Câble depuis ${b.cableFrom} : clique sur le second équipement (Échap pour annuler).`
        : 'Outil Câble : clique sur un équipement, puis sur un autre. Une interface libre est choisie de chaque côté.'));
    }
    if (b.draft) {
      box.append(h('div', { class: 'draft' },
        h('span', null, 'Brouillon non appliqué : corrige les erreurs ci-dessous (le schéma montre la dernière version valide).'),
        h('button', { type: 'button', onclick: () => guarded(undo) }, 'Abandonner le brouillon')));
    }
    const list = h('ul', { class: 'issue-list' });
    for (const [cls, label, items] of [['err', 'Erreur', b.errors], ['warn', 'Attention', b.warnings]]) {
      for (const m of items) {
        const names = Array.from(devicesIn([m], curDef()));
        list.append(h('li', { class: cls },
          h('button', {
            type: 'button', title: names.length ? `Sélectionner ${names[0]}` : '',
            onclick: () => { if (names.length) buildSelect({ kind: 'device', name: names[0] }); },
          }, h('strong', null, `${label} · `), m)));
      }
    }
    if (list.childElementCount) box.append(list);
  }

  // ---- inspecteur

  /** Champ dont la valeur est appliquée au changement (Entrée ou sortie du champ). */
  function input(key, value, onCommit, attrs = {}) {
    const el = h('input', { type: 'text', value: value === null || value === undefined ? '' : String(value), 'data-k': key, spellcheck: 'false', ...attrs });
    el.addEventListener('change', () => onCommit(el.type === 'checkbox' ? el.checked : el.value.trim()));
    el.addEventListener('keydown', (e) => { if (e.key === 'Enter') el.blur(); });
    return el;
  }

  function checkbox(key, checked, onCommit, label) {
    const el = h('input', { type: 'checkbox', 'data-k': key });
    el.checked = !!checked;
    el.addEventListener('change', () => onCommit(el.checked));
    return h('label', { class: 'inline' }, el, ` ${label}`);
  }

  function select(key, value, options, onCommit, attrs = {}) {
    const el = h('select', { 'data-k': key, ...attrs }, options.map(([v, l]) => h('option', { value: v }, l)));
    el.value = value;
    el.addEventListener('change', () => onCommit(el.value));
    return el;
  }

  const del = (label, onclick) => h('button', { type: 'button', class: 'icon del', 'aria-label': label, title: label, onclick }, icon('x'));

  /** Modifie l'équipement `name` (copie) et applique. */
  function editDevice(name, mutate) {
    return edit((def) => {
      const d = devOf(def, name);
      if (!d) return false;
      return mutate(d, def);
    });
  }

  function renderInspector() {
    if (!state.build.on) return;
    const root = $('#tab-device');
    // Conserver le champ actif à travers le nouveau rendu.
    const active = document.activeElement && root.contains(document.activeElement) ? document.activeElement.getAttribute('data-k') : null;
    clear(root);
    const def = curDef();
    if (!def) return;
    const sel = state.build.sel;
    if (sel && sel.kind === 'device' && devOf(def, sel.name)) renderDeviceForm(root, def, devOf(def, sel.name));
    else if (sel && sel.kind === 'link') renderLinkForm(root, def, sel);
    else renderNetworkForm(root, def);
    if (active) {
      const el = root.querySelector(`[data-k="${CSS.escape(active)}"]`);
      if (el) el.focus();
    }
  }

  function renderNetworkForm(root, def) {
    root.append(
      h('h3', null, 'Réseau'),
      h('div', { class: 'form-grid' },
        h('label', null, 'Nom'),
        input('net-name', def.network.name, (v) => { if (v) edit((d) => { d.network.name = v; }); }),
        h('label', null, 'Graine'),
        input('net-seed', def.network.seed || 0, (v) => edit((d) => { d.network.seed = Math.max(0, parseInt(v, 10) || 0); }), { inputmode: 'numeric' })),
      h('p', { class: 'muted' }, `${def.devices.length} équipement(s), ${def.links.length} câble(s).`),
      h('h3', null, 'Pour commencer'),
      h('ol', { class: 'tips' },
        h('li', null, 'Ajoute des équipements : clic sur la palette, ou glisser-déposer sur le schéma.'),
        h('li', null, 'Relie-les avec l’outil Câble (touche C) : un clic sur chaque extrémité.'),
        h('li', null, '« Adresser automatiquement » donne les IP, passerelles et routes manquantes.'),
        h('li', null, 'Clique un équipement ou un câble pour le configurer ; Suppr le supprime.'),
        h('li', null, 'Teste avec la barre d’outils (ping) ; Ctrl+Z annule.')),
    );
  }

  function renderDeviceForm(root, def, d) {
    const name = d.name;
    const used = usedEndpoints(def);
    root.append(h('div', { class: 'insp-head' },
      h('h3', null, `${KIND_LABEL[d.type] || d.type} ${name}`),
      h('button', { type: 'button', class: 'danger', onclick: () => guarded(deleteSelection) }, 'Supprimer')));

    const general = h('div', { class: 'form-grid' },
      h('label', null, 'Nom'),
      input('dev-name', name, (v) => {
        if (!v || v === name) return;
        if (!/^[\w.-]+$/.test(v)) { banner('Nom invalide : lettres, chiffres, « - », « _ » et « . » seulement.'); renderInspector(); return; }
        if (devOf(curDef(), v)) { banner(`« ${v} » existe déjà.`); renderInspector(); return; }
        state.build.sel = { kind: 'device', name: v };
        const p = state.positions.get(name);
        if (p) { state.positions.delete(name); state.positions.set(v, p); }
        editDevice(name, (x, df) => {
          x.name = v;
          const ren = (ep) => (ep.startsWith(`${name}:`) ? `${v}:${ep.slice(name.length + 1)}` : ep);
          for (const l of df.links) { l.a = ren(l.a); l.b = ren(l.b); }
        });
      }),
      h('label', null, 'Rôle'),
      select('dev-role', d.role || '', [['', 'automatique'], ...ROLES.map((r) => [r, r])], (v) => editDevice(name, (x) => { if (v) x.role = v; else delete x.role; })),
    );
    if (isIp(d.type)) {
      general.append(
        h('label', null, 'Passerelle'),
        input('dev-gw', d.gateway || '', (v) => editDevice(name, (x) => { if (v) x.gateway = v; else delete x.gateway; }), { placeholder: 'aucune (ou reçue par DHCP)', class: 'mono' }),
        h('label', null, 'Serveur DNS'),
        input('dev-dns', d.dns || '', (v) => editDevice(name, (x) => { if (v) x.dns = v; else delete x.dns; }), { placeholder: 'aucun (ou reçu par DHCP)', class: 'mono' }));
    }
    root.append(general);

    // Interfaces.
    const ifs = d.interfaces || [];
    const table = h('table', { class: 'grid' });
    const head = d.type === 'switch' ? ['Port', 'Mode', 'VLAN', ''] : ['Interface', 'Adresse IP', 'DHCP', 'Admin', ''];
    table.append(h('thead', null, h('tr', null, head.map((t) => h('th', null, t)))));
    const body = h('tbody');
    ifs.forEach((i, k) => {
      const cabled = used.has(`${name}:${i.name}`);
      const nameCell = h('td', null, input(`if-${k}-name`, i.name, (v) => {
        if (!v || v === i.name) return;
        editDevice(name, (x, df) => {
          if ((x.interfaces || []).some((o) => o.name === v)) return false;
          const old = `${name}:${x.interfaces[k].name}`;
          x.interfaces[k].name = v;
          for (const l of df.links) { if (l.a === old) l.a = `${name}:${v}`; if (l.b === old) l.b = `${name}:${v}`; }
          return true;
        });
      }, { class: `mono${cabled ? ' cabled' : ''}`, title: cabled ? 'câblée' : 'libre' }));
      const remove = h('td', null, del(`Supprimer ${i.name}`, () => editDevice(name, (x, df) => {
        const ep = `${name}:${x.interfaces[k].name}`;
        x.interfaces.splice(k, 1);
        df.links = df.links.filter((l) => l.a !== ep && l.b !== ep);
      })));
      if (d.type === 'switch') {
        const trunk = Array.isArray(i.trunk);
        const mode = select(`if-${k}-mode`, trunk ? 'trunk' : 'access', [['access', 'accès'], ['trunk', 'trunk']], (v) => editDevice(name, (x) => {
          const it = x.interfaces[k];
          if (v === 'trunk') { it.trunk = []; delete it.vlan; } else { delete it.trunk; delete it.native_vlan; }
        }));
        const vlanCell = trunk
          ? h('td', { class: 'pair' },
            input(`if-${k}-allowed`, i.trunk.join(','), (v) => editDevice(name, (x) => {
              x.interfaces[k].trunk = v ? v.split(/[\s,]+/).filter(Boolean).map(Number).filter((n) => n > 0) : [];
            }), { placeholder: 'tous', title: 'VLAN transportés (ex. 10,20 ; vide = tous)', class: 'mono' }),
            input(`if-${k}-native`, i.native_vlan || '', (v) => editDevice(name, (x) => {
              if (v) x.interfaces[k].native_vlan = Number(v); else delete x.interfaces[k].native_vlan;
            }), { placeholder: 'natif 1', title: 'VLAN natif', class: 'mono narrow' }))
          : h('td', null, input(`if-${k}-vlan`, i.vlan || '', (v) => editDevice(name, (x) => {
            if (v) x.interfaces[k].vlan = Number(v); else delete x.interfaces[k].vlan;
          }), { placeholder: '1', class: 'mono narrow' }));
        body.append(h('tr', null, nameCell, h('td', null, mode), vlanCell, remove));
      } else {
        const isDhcp = i.ip === 'dhcp';
        body.append(h('tr', null,
          nameCell,
          h('td', null, isDhcp
            ? h('span', { class: 'muted', title: 'Adresse demandée à un serveur DHCP' }, 'obtenue par DHCP')
            : input(`if-${k}-ip`, i.ip || '', (v) => editDevice(name, (x) => { if (v) x.interfaces[k].ip = v; else delete x.interfaces[k].ip; }), { placeholder: 'aucune (ex. 10.0.1.10/24)', class: 'mono' })),
          h('td', null, (() => {
            const c = h('input', { type: 'checkbox', 'data-k': `if-${k}-dhcp`, title: "Demander l'adresse à un serveur DHCP" });
            c.checked = isDhcp;
            c.addEventListener('change', () => editDevice(name, (x) => { if (c.checked) x.interfaces[k].ip = 'dhcp'; else delete x.interfaces[k].ip; }));
            return c;
          })()),
          h('td', null, (() => {
            const c = h('input', { type: 'checkbox', 'data-k': `if-${k}-mgmt`, title: "Interface d'administration" });
            c.checked = !!i.management;
            c.addEventListener('change', () => editDevice(name, (x) => { if (c.checked) x.interfaces[k].management = true; else delete x.interfaces[k].management; }));
            return c;
          })()),
          remove));
      }
    });
    table.append(body);
    root.append(h('h3', null, 'Interfaces'), table,
      h('button', { type: 'button', class: 'add', onclick: () => editDevice(name, (x) => {
        x.interfaces = x.interfaces || [];
        x.interfaces.push({ name: nextIfaceName(x, x.type === 'switch' ? 'port' : 'eth') });
      }) }, d.type === 'switch' ? '+ port' : '+ interface'));

    if (!isIp(d.type)) return;

    // Routes.
    const routes = d.routes || [];
    const rt = h('table', { class: 'grid' }, h('thead', null, h('tr', null, ['Destination', 'Via', ''].map((t) => h('th', null, t)))));
    const rb = h('tbody');
    routes.forEach((r, k) => rb.append(h('tr', null,
      h('td', null, input(`rt-${k}-dest`, r.dest, (v) => editDevice(name, (x) => { x.routes[k].dest = v; }), { class: 'mono', placeholder: '10.0.2.0/24' })),
      h('td', null, input(`rt-${k}-via`, r.via, (v) => editDevice(name, (x) => { x.routes[k].via = v; }), { class: 'mono', placeholder: '10.255.0.2' })),
      h('td', null, del('Supprimer la route', () => editDevice(name, (x) => { x.routes.splice(k, 1); }))))));
    rt.append(rb);
    const addRoute = h('div', { class: 'add-row' },
      input('rt-new-dest', '', () => {}, { class: 'mono', placeholder: 'destination (10.0.2.0/24)' }),
      input('rt-new-via', '', () => {}, { class: 'mono', placeholder: 'via (10.255.0.2)' }),
      h('button', { type: 'button', class: 'add', onclick: () => {
        const dest = root.querySelector('[data-k="rt-new-dest"]').value.trim();
        const via = root.querySelector('[data-k="rt-new-via"]').value.trim();
        if (!dest || !via) { banner('Indique la destination et le prochain saut.'); return; }
        editDevice(name, (x) => { x.routes = x.routes || []; x.routes.push({ dest, via }); });
      } }, '+ route'));
    root.append(h('h3', null, 'Routes statiques'), routes.length ? rt : h('p', { class: 'muted' }, 'Aucune.'), addRoute);

    // Services.
    const svcs = d.services || [];
    const st = h('table', { class: 'grid' }, h('thead', null, h('tr', null, ['Proto', 'Port', 'Nom', ''].map((t) => h('th', null, t)))));
    const sb = h('tbody');
    svcs.forEach((sv, k) => sb.append(h('tr', null,
      h('td', null, select(`sv-${k}-proto`, sv.proto, [['tcp', 'tcp'], ['udp', 'udp']], (v) => editDevice(name, (x) => { x.services[k].proto = v; }))),
      h('td', null, input(`sv-${k}-port`, sv.port, (v) => editDevice(name, (x) => { x.services[k].port = Number(v); }), { class: 'mono narrow', inputmode: 'numeric' })),
      h('td', null, input(`sv-${k}-name`, sv.name, (v) => editDevice(name, (x) => { x.services[k].name = v || 'service'; }))),
      h('td', null, del('Arrêter le service', () => editDevice(name, (x) => { x.services.splice(k, 1); }))))));
    st.append(sb);
    root.append(h('h3', null, 'Services'), svcs.length ? st : h('p', { class: 'muted' }, 'Aucun.'),
      h('div', { class: 'btn-row' },
        ...[['http', 'tcp', 80], ['ssh', 'tcp', 22], ['echo', 'udp', 7]].map(([n, p, port]) => h('button', {
          type: 'button', class: 'add',
          onclick: () => editDevice(name, (x) => {
            x.services = x.services || [];
            if (x.services.some((o) => o.proto === p && o.port === port)) return false;
            x.services.push({ proto: p, port, name: n });
            return true;
          }),
        }, `+ ${n}`))));

    renderDhcpDnsForm(root, d);

    // Filtrage.
    if (!isRouter(d.type)) return;
    const fw = d.type === 'firewall';
    const acl = d.acl || {};
    const rules = acl.rules || [];
    const withAcl = (x) => { x.acl = x.acl || {}; x.acl.rules = x.acl.rules || []; return x.acl; };
    root.append(h('h3', null, 'Filtrage'),
      h('div', { class: 'form-grid' },
        h('label', null, 'Par défaut'),
        select('acl-default', acl.default || (fw ? 'deny' : 'allow'), [['allow', 'tout autoriser'], ['deny', 'tout refuser']], (v) => editDevice(name, (x) => { withAcl(x).default = v; })),
        h('label', null, 'Suivi'),
        checkbox('acl-stateful', acl.stateful === undefined || acl.stateful === null ? fw : acl.stateful, (v) => editDevice(name, (x) => { withAcl(x).stateful = v; }), 'les réponses repassent')));
    const ifOpts = [['', 'toutes'], ...(d.interfaces || []).map((i) => [i.name, i.name])];
    const at = h('table', { class: 'grid acl' }, h('thead', null, h('tr', null, ['#', 'Action', 'Proto', 'Source', 'Destination', 'Port', 'Entrée', ''].map((t) => h('th', null, t)))));
    const ab = h('tbody');
    rules.forEach((r, k) => {
      const set = (field) => (v) => editDevice(name, (x) => { const rr = withAcl(x).rules[k]; if (v) rr[field] = v; else delete rr[field]; });
      ab.append(h('tr', null,
        h('td', { class: 'muted' }, String(k + 1)),
        h('td', null, select(`acl-${k}-action`, r.action, [['allow', 'allow'], ['deny', 'deny']], set('action'))),
        h('td', null, select(`acl-${k}-proto`, r.proto || 'any', [['any', 'any'], ['icmp', 'icmp'], ['tcp', 'tcp'], ['udp', 'udp']], (v) => set('proto')(v === 'any' ? '' : v))),
        h('td', null, input(`acl-${k}-src`, r.src || '', set('src'), { class: 'mono', placeholder: 'tout' })),
        h('td', null, input(`acl-${k}-dst`, r.dst || '', set('dst'), { class: 'mono', placeholder: 'tout' })),
        h('td', null, input(`acl-${k}-port`, r.port || '', set('port'), { class: 'mono narrow', placeholder: '—' })),
        h('td', null, select(`acl-${k}-in`, r.in || '', ifOpts, set('in'))),
        h('td', null, del('Supprimer la règle', () => editDevice(name, (x) => { withAcl(x).rules.splice(k, 1); })))));
    });
    at.append(ab);
    root.append(rules.length ? at : h('p', { class: 'muted' }, 'Aucune règle.'),
      h('div', { class: 'btn-row' },
        h('button', { type: 'button', class: 'add', onclick: () => editDevice(name, (x) => { withAcl(x).rules.push({ action: 'allow', proto: 'tcp', port: '80' }); }) }, '+ autoriser'),
        h('button', { type: 'button', class: 'add', onclick: () => editDevice(name, (x) => { withAcl(x).rules.push({ action: 'deny' }); }) }, '+ refuser')));
  }

  /** Relais DHCP (routeurs), serveur DHCP et serveur DNS d'un équipement IP. */
  function renderDhcpDnsForm(root, d) {
    const name = d.name;
    // Relais : une interface de routeur transmet les demandes de son réseau au serveur.
    if (isRouter(d.type)) {
      const rows = (d.interfaces || []).map((i, k) => h('tr', null,
        h('td', { class: 'mono' }, i.name),
        h('td', null, input(`relay-${k}`, i.dhcp_relay || '', (v) => editDevice(name, (x) => {
          if (v) x.interfaces[k].dhcp_relay = v; else delete x.interfaces[k].dhcp_relay;
        }), { class: 'mono', placeholder: 'aucun (IP du serveur DHCP)' }))));
      root.append(h('h3', null, 'Relais DHCP'),
        h('p', { class: 'muted' }, 'Une demande DHCP est une diffusion : un routeur ne la transmet que si l’interface du réseau des postes la relaie vers le serveur.'),
        h('table', { class: 'grid' }, h('thead', null, h('tr', null, h('th', null, 'Interface'), h('th', null, 'Serveur visé'))), h('tbody', null, rows)));
    }

    // Serveur DHCP.
    const dhcp = d.dhcp;
    root.append(h('h3', null, 'Serveur DHCP'));
    if (!dhcp) {
      root.append(h('button', { type: 'button', class: 'add', onclick: () => editDevice(name, (x) => {
        // Plage proposée : le réseau de la première interface adressée.
        const addr = (x.interfaces || []).map((i) => i.ip).find((ip) => ip && ip !== 'dhcp');
        const m = addr && /^(\d+)\.(\d+)\.(\d+)\.\d+\/(\d+)$/.exec(addr);
        if (!m) { banner('Donne d’abord une adresse fixe à une interface de cet équipement.'); return false; }
        const net = `${m[1]}.${m[2]}.${m[3]}`;
        const gw = isRouter(x.type) ? addr.split('/')[0] : x.gateway;
        x.dhcp = { pools: [{ network: `${net}.0/24`, range: `${net}.100-${net}.150`, ...(gw ? { gateway: gw } : {}) }] };
        return true;
      }) }, '+ serveur DHCP'));
    } else {
      const pools = dhcp.pools || [];
      const pt = h('table', { class: 'grid' }, h('thead', null, h('tr', null, ['Réseau', 'Plage', 'Passerelle', 'DNS', ''].map((t) => h('th', null, t)))));
      const pb = h('tbody');
      pools.forEach((p, k) => {
        const set = (field) => (v) => editDevice(name, (x) => { const q = x.dhcp.pools[k]; if (v) q[field] = v; else delete q[field]; });
        pb.append(h('tr', null,
          h('td', null, input(`pool-${k}-net`, p.network, set('network'), { class: 'mono' })),
          h('td', null, input(`pool-${k}-range`, p.range, set('range'), { class: 'mono', placeholder: 'première-dernière' })),
          h('td', null, input(`pool-${k}-gw`, p.gateway || '', set('gateway'), { class: 'mono', placeholder: '—' })),
          h('td', null, input(`pool-${k}-dns`, p.dns || '', set('dns'), { class: 'mono', placeholder: '—' })),
          h('td', null, del('Supprimer la plage', () => editDevice(name, (x) => {
            x.dhcp.pools.splice(k, 1);
            if (!x.dhcp.pools.length) delete x.dhcp;
          })))));
      });
      pt.append(pb);
      root.append(
        checkbox('dhcp-on', dhcp.enabled !== false, (v) => editDevice(name, (x) => { if (v) delete x.dhcp.enabled; else x.dhcp.enabled = false; }), 'service démarré'),
        pt,
        h('div', { class: 'btn-row' },
          h('button', { type: 'button', class: 'add', onclick: () => editDevice(name, (x) => { x.dhcp.pools.push({ network: '10.0.9.0/24', range: '10.0.9.100-10.0.9.150' }); }) }, '+ plage'),
          h('button', { type: 'button', class: 'danger', onclick: () => editDevice(name, (x) => { delete x.dhcp; }) }, 'Supprimer le serveur')));
    }

    // Serveur DNS.
    const dns = d.dns_server;
    root.append(h('h3', null, 'Serveur DNS'));
    if (!dns) {
      root.append(h('button', { type: 'button', class: 'add', onclick: () => editDevice(name, (x) => { x.dns_server = { records: {} }; }) }, '+ serveur DNS'));
      return;
    }
    const records = Object.entries(dns.records || {});
    const rt = h('table', { class: 'grid' }, h('thead', null, h('tr', null, ['Nom', 'Adresse', ''].map((t) => h('th', null, t)))));
    const rb = h('tbody');
    records.forEach(([rname, rip], k) => rb.append(h('tr', null,
      h('td', null, input(`rec-${k}-name`, rname, (v) => editDevice(name, (x) => {
        if (!v || v === rname) return false;
        const recs = x.dns_server.records; const ip = recs[rname]; delete recs[rname]; recs[v] = ip;
        return true;
      }), { class: 'mono' })),
      h('td', null, input(`rec-${k}-ip`, rip, (v) => editDevice(name, (x) => { if (!v) return false; x.dns_server.records[rname] = v; return true; }), { class: 'mono' })),
      h('td', null, del(`Supprimer ${rname}`, () => editDevice(name, (x) => { delete x.dns_server.records[rname]; }))))));
    rt.append(rb);
    root.append(
      checkbox('dns-on', dns.enabled !== false, (v) => editDevice(name, (x) => { if (v) delete x.dns_server.enabled; else x.dns_server.enabled = false; }), 'service démarré'),
      records.length ? rt : h('p', { class: 'muted' }, 'Zone vide.'),
      h('div', { class: 'add-row' },
        input('rec-new-name', '', () => {}, { class: 'mono', placeholder: 'nom (web.lan)' }),
        input('rec-new-ip', '', () => {}, { class: 'mono', placeholder: 'adresse' }),
        h('button', { type: 'button', class: 'add', onclick: () => {
          const n = root.querySelector('[data-k="rec-new-name"]').value.trim();
          const a = root.querySelector('[data-k="rec-new-ip"]').value.trim();
          if (!n || !a) { banner('Indique le nom et l’adresse.'); return; }
          editDevice(name, (x) => { x.dns_server.records = x.dns_server.records || {}; x.dns_server.records[n] = a; });
        } }, '+ nom')),
      h('div', { class: 'btn-row' },
        h('button', { type: 'button', class: 'danger', onclick: () => editDevice(name, (x) => { delete x.dns_server; }) }, 'Supprimer le serveur DNS')));
  }

  function renderLinkForm(root, def, sel) {
    const i = def.links.findIndex((l) => l.a === sel.a && l.b === sel.b);
    if (i < 0) { renderNetworkForm(root, def); return; }
    const l = def.links[i];
    const used = usedEndpoints(def);
    const endSelect = (key, ep) => {
      const [dev, cur] = ep.split(/:(.*)/s);
      const d = devOf(def, dev);
      const opts = (d ? ifaceNames(d) : [cur]).filter((n) => n === cur || !used.has(`${dev}:${n}`)).map((n) => [n, n]);
      return h('div', { class: 'endpoint' }, h('strong', null, dev), ' : ',
        select(key, cur, opts, (v) => {
          const next = `${dev}:${v}`;
          state.build.sel = key === 'ln-a' ? { kind: 'link', a: next, b: l.b } : { kind: 'link', a: l.a, b: next };
          edit((df) => { const x = df.links[i]; if (key === 'ln-a') x.a = next; else x.b = next; });
        }));
    };
    root.append(
      h('div', { class: 'insp-head' }, h('h3', null, 'Câble'),
        h('button', { type: 'button', class: 'danger', onclick: () => guarded(deleteSelection) }, 'Supprimer')),
      endSelect('ln-a', l.a), endSelect('ln-b', l.b),
      h('div', { class: 'form-grid' },
        h('label', null, 'Latence (ms)'),
        input('ln-lat', l.latency_us === undefined || l.latency_us === null ? '1' : String(l.latency_us / 1000), (v) => edit((df) => {
          const ms = Number(v.replace(',', '.'));
          if (!Number.isFinite(ms) || ms < 0) return false;
          df.links[i].latency_us = Math.round(ms * 1000);
          return true;
        }), { class: 'mono narrow', inputmode: 'decimal' }),
        h('label', null, 'Perte (%)'),
        input('ln-loss', l.loss ? String(Math.round(l.loss * 1000) / 10) : '0', (v) => edit((df) => {
          const pct = Number(v.replace(',', '.'));
          if (!Number.isFinite(pct) || pct < 0 || pct > 100) return false;
          if (pct === 0) delete df.links[i].loss; else df.links[i].loss = pct / 100;
          return true;
        }), { class: 'mono narrow', inputmode: 'decimal' })),
    );
  }

  function initBuild() {
    $('#build-toggle').addEventListener('click', () => guarded(toggleBuild));
    $('#bt-exit').addEventListener('click', exitBuild);
    $('#bt-select').addEventListener('click', () => setTool('select'));
    $('#bt-cable').addEventListener('click', () => setTool(state.build.tool === 'cable' ? 'select' : 'cable'));
    $('#bt-undo').addEventListener('click', () => guarded(undo));
    $('#bt-redo').addEventListener('click', () => guarded(redo));
    $('#bt-delete').addEventListener('click', () => guarded(deleteSelection));
    $('#bt-auto').addEventListener('click', () => guarded(autoAddress));
    $('#bt-netiv').addEventListener('click', () => guarded(() => download('netiv')));
    $('#bt-yaml').addEventListener('click', () => guarded(() => download('yaml')));
    $('#file-import').addEventListener('change', (e) => {
      const f = e.target.files && e.target.files[0];
      e.target.value = '';
      if (f) guarded(() => importFile(f));
    });

    // Palette : clic = ajout au centre de la vue ; glisser = ajout à l'endroit visé.
    for (const b of $$('.palette button')) {
      const kind = b.dataset.kind;
      const icon = s('svg', { viewBox: '-14 -14 28 28', class: `pal-icon kind-${kind}`, 'aria-hidden': 'true' }, ...glyph(kind));
      b.prepend(icon);
      b.addEventListener('click', () => addDevice(kind, null));
      b.addEventListener('dragstart', (e) => {
        e.dataTransfer.setData('application/x-netiv-kind', kind);
        e.dataTransfer.effectAllowed = 'copy';
      });
    }
    const svg = $('#canvas');
    svg.addEventListener('dragover', (e) => {
      if (state.build.on && Array.from(e.dataTransfer.types).includes('application/x-netiv-kind')) {
        e.preventDefault();
        e.dataTransfer.dropEffect = 'copy';
      }
    });
    svg.addEventListener('drop', (e) => {
      const kind = e.dataTransfer.getData('application/x-netiv-kind');
      if (!state.build.on || !kind) return;
      e.preventDefault();
      addDevice(kind, svgPoint(svg, e));
    });

    // Câble « élastique » qui suit la souris.
    svg.addEventListener('pointermove', (e) => {
      const b = state.build;
      const overlay = $('#canvas .overlay');
      const old = $('#canvas .rubber');
      if (!b.on || !b.cableFrom || !overlay) { if (old) old.remove(); return; }
      const a = state.positions.get(b.cableFrom);
      if (!a) return;
      const p = svgPoint(svg, e);
      const line = old || s('line', { class: 'rubber' });
      line.setAttribute('x1', a.x); line.setAttribute('y1', a.y);
      line.setAttribute('x2', p.x); line.setAttribute('y2', p.y);
      if (!old) overlay.append(line);
    });

    document.addEventListener('keydown', (e) => {
      const t = e.target;
      const typing = t instanceof HTMLElement && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName));
      if (typing || document.querySelector('dialog[open]')) return;
      const b = state.build;
      const ctrl = e.ctrlKey || e.metaKey;
      if (!ctrl && !e.altKey && (e.key === 'b' || e.key === 'B')) { e.preventDefault(); guarded(toggleBuild); return; }
      if (!b.on) return;
      if (ctrl && (e.key === 'z' || e.key === 'Z') && !e.shiftKey) { e.preventDefault(); guarded(undo); }
      else if (ctrl && (e.key === 'y' || e.key === 'Y' || ((e.key === 'z' || e.key === 'Z') && e.shiftKey))) { e.preventDefault(); guarded(redo); }
      else if (ctrl || e.altKey) return;
      else if (e.key === 'c' || e.key === 'C') { e.preventDefault(); setTool(b.tool === 'cable' ? 'select' : 'cable'); }
      else if (e.key === 'v' || e.key === 'V') { e.preventDefault(); setTool('select'); }
      else if (e.key === 'Escape') { if (b.cableFrom) { b.cableFrom = null; renderCanvas(); renderIssues(); } else setTool('select'); }
      else if (e.key === 'Delete' || e.key === 'Backspace') { if (b.sel) { e.preventDefault(); guarded(deleteSelection); } }
    });
  }

  // ---- appareil

  async function selectDevice(name) {
    state.device = name;
    renderCanvas();
    selectTab('device');
    await guarded(() => loadDevice(name));
  }

  async function loadDevice(name) {
    if (!state.current) return;
    const d = await api('GET', `/api/labs/${state.current}/device/${encodeURIComponent(name)}`);
    renderDevice(d);
  }

  function renderDevice(d) {
    if (state.build.on) { renderInspector(); return; }
    const root = clear($('#tab-device'));
    if (!d) {
      root.append(h('p', { class: 'muted' }, 'Clique sur un équipement du schéma pour voir ses interfaces, ses routes, son cache ARP, ses règles…'));
      return;
    }
    root.append(
      h('h3', null, d.name),
      h('dl', { class: 'kv' }, h('dt', null, 'Type'), h('dd', null, d.kind), h('dt', null, 'Rôle'), h('dd', null, d.role)),
    );
    d.sections.forEach((sec, i) => {
      root.append(h('details', { open: i < 2 }, h('summary', null, sec.title), h('pre', null, sec.text)));
    });
  }

  // ---- parcours (Packet Journey)

  async function loadTransactions({ selectNewest } = {}) {
    if (!state.current) { state.tx = []; renderJourney(); return; }
    const r = await api('GET', `/api/labs/${state.current}/transactions`);
    const had = new Set(state.tx.map((t) => t.id));
    state.tx = r.transactions;
    const fresh = state.tx.find((t) => !had.has(t.id));
    if (selectNewest && fresh && state.tx.length) {
      await openJourney(state.tx[0].id, { switchTab: false });
    } else {
      renderJourney();
    }
  }

  async function openJourney(id, { switchTab = true } = {}) {
    stopPlay();
    const j = await api('GET', `/api/labs/${state.current}/journey/${id}`);
    state.journey = j;
    state.step = -1;
    renderJourney();
    if (switchTab) selectTab('journey');
    highlightStep();
  }

  function renderJourney() {
    const root = clear($('#tab-journey'));
    if (!state.tx.length) {
      root.append(h('p', { class: 'muted' }, "Aucun échange pour l'instant. Envoie un ping ou une requête (barre d'outils) : son parcours complet s'affiche ici, étape par étape."));
      return;
    }
    const sel = h('select', { 'aria-label': 'Échange à rejouer', onchange: (e) => guarded(() => openJourney(+e.target.value, { switchTab: false })) },
      state.tx.map((t) => h('option', { value: t.id, selected: state.journey && state.journey.id === t.id },
        `#${t.id} ${t.label} ${t.outcome === 'ok' ? '✓' : t.outcome === 'fail' ? '✗' : '…'}`)));
    root.append(h('label', { class: 'muted' }, 'Échange ', sel));

    const j = state.journey;
    if (!j) return;
    const total = j.steps.length;
    root.append(h('div', { class: 'player', role: 'group', 'aria-label': 'Lecteur' },
      h('button', { type: 'button', id: 'pl-first', class: 'icon', title: 'Début', 'aria-label': 'Début', onclick: () => gotoStep(-1) }, icon('first')),
      h('button', { type: 'button', id: 'pl-prev', class: 'icon', title: 'Étape précédente (←)', 'aria-label': 'Étape précédente', onclick: () => gotoStep(state.step - 1) }, icon('prev')),
      h('button', { type: 'button', id: 'pl-play', class: 'primary icon', title: 'Lecture / pause (espace)', 'aria-label': state.playing ? 'Pause' : 'Lecture', onclick: togglePlay }, icon(state.playing ? 'pause' : 'play')),
      h('button', { type: 'button', id: 'pl-next', class: 'icon', title: 'Étape suivante (→)', 'aria-label': 'Étape suivante', onclick: () => gotoStep(state.step + 1) }, icon('next')),
      h('select', { id: 'pl-speed', 'aria-label': 'Vitesse' }, [['1100', 'lent'], ['650', 'normal'], ['250', 'rapide']].map(([v, l]) => h('option', { value: v, selected: v === '650' }, l))),
      h('span', { class: 'count', id: 'pl-count' }, `${Math.max(0, state.step + 1)}/${total}`),
    ));
    const ol = h('ol', { class: 'steps', id: 'steps' });
    j.steps.forEach((st, i) => {
      ol.append(h('li', { 'data-i': i, class: `k-${st.kind}`, onclick: () => gotoStep(i) },
        h('span', { class: 't' }, st.time), h('span', { class: 'd' }, st.device), h('span', { class: `k-${st.kind}` }, st.text)));
    });
    root.append(ol);
    root.onkeydown = (e) => {
      if (e.key === 'ArrowRight') { e.preventDefault(); gotoStep(state.step + 1); }
      else if (e.key === 'ArrowLeft') { e.preventDefault(); gotoStep(state.step - 1); }
      else if (e.key === ' ' && e.target.tagName !== 'BUTTON' && e.target.tagName !== 'SELECT') { e.preventDefault(); togglePlay(); }
    };
    root.append(h('details', { style: 'margin-top:10px' }, h('summary', null, 'Version texte'), h('pre', null, j.text)));
    markSteps();
  }

  function markSteps() {
    const items = $$('#steps li');
    items.forEach((li, i) => {
      li.classList.toggle('current', i === state.step);
      li.classList.toggle('future', i > state.step);
    });
    const cur = items[state.step];
    if (cur && typeof cur.scrollIntoView === 'function') cur.scrollIntoView({ block: 'nearest' });
    const c = $('#pl-count');
    if (c && state.journey) c.textContent = `${Math.max(0, state.step + 1)}/${state.journey.steps.length}`;
  }

  function gotoStep(i) {
    const j = state.journey;
    if (!j) return;
    state.step = Math.max(-1, Math.min(j.steps.length - 1, i));
    markSteps();
    highlightStep(true);
    if (state.playing && state.step >= j.steps.length - 1) stopPlay();
  }

  function highlightStep(animate = false) {
    $$('.node.current').forEach((n) => n.classList.remove('current'));
    $$('.link.active').forEach((n) => n.classList.remove('active'));
    const j = state.journey;
    if (!j || state.step < 0) return;
    const st = j.steps[state.step];
    if (!st) return;
    const node = $(`.node[data-name="${CSS.escape(st.device)}"]`);
    if (node) node.classList.add('current');
    if (st.edge && animate) animatePacket(st.edge.from, st.edge.to);
  }

  function animatePacket(from, to) {
    const a = state.positions.get(from), b = state.positions.get(to);
    const overlay = $('#canvas .overlay');
    if (!a || !b || !overlay) return;
    const link = $$('.link').find((l) => (l.dataset.a === from && l.dataset.b === to) || (l.dataset.a === to && l.dataset.b === from));
    if (link) link.classList.add('active');
    clear(overlay);
    const dot = s('circle', { class: 'packet', r: 9, cx: a.x, cy: a.y });
    overlay.append(dot);
    if (reducedMotion) { dot.setAttribute('cx', b.x); dot.setAttribute('cy', b.y); return; }
    const speed = +($('#pl-speed') || { value: 650 }).value;
    const dur = Math.max(160, speed * 0.7);
    const t0 = performance.now();
    const tick = (now) => {
      const k = Math.min(1, (now - t0) / dur);
      dot.setAttribute('cx', a.x + (b.x - a.x) * k);
      dot.setAttribute('cy', a.y + (b.y - a.y) * k);
      if (k < 1 && dot.isConnected) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  }

  function togglePlay() {
    if (state.playing) { stopPlay(); return; }
    const j = state.journey;
    if (!j) return;
    if (state.step >= j.steps.length - 1) gotoStep(-1);
    const speed = +($('#pl-speed') || { value: 650 }).value;
    state.playing = setInterval(() => gotoStep(state.step + 1), speed);
    const b = $('#pl-play');
    if (b) { b.replaceChildren(icon('pause')); b.setAttribute('aria-label', 'Pause'); }
    gotoStep(state.step + 1);
  }

  function stopPlay() {
    if (state.playing) clearInterval(state.playing);
    state.playing = null;
    const b = $('#pl-play');
    if (b) { b.replaceChildren(icon('play')); b.setAttribute('aria-label', 'Lecture'); }
  }

  // ---- trace

  const KIND_FILTERS = [
    ['app', 'Échanges'], ['frame_sent', 'Envois'], ['frame_received', 'Réceptions'], ['switch', 'Commutation'],
    ['arp', 'ARP'], ['route', 'Routage'], ['dhcp', 'DHCP'], ['dns', 'DNS'], ['drop', 'Refus'], ['config', 'Config'],
  ];

  async function loadTrace() {
    if (!state.current) return;
    const len = state.snap ? state.snap.trace_len : 0;
    const since = Math.max(0, len - 400);
    const t = await api('GET', `/api/labs/${state.current}/trace?since=${since}&limit=400`);
    state.traceEvents = t.events;
    renderTrace();
  }

  function renderTrace() {
    const root = clear($('#tab-trace'));
    if (!state.traceEvents.length) {
      root.append(h('p', { class: 'muted' }, 'La trace est vide : rien ne s’est encore passé dans ce lab.'));
      return;
    }
    const filters = h('div', { class: 'filters', role: 'group', 'aria-label': 'Filtres' },
      KIND_FILTERS.map(([k, label]) => h('button', {
        type: 'button', 'aria-pressed': String(state.traceKinds.has(k)),
        onclick: () => { state.traceKinds.has(k) ? state.traceKinds.delete(k) : state.traceKinds.add(k); renderTrace(); },
      }, label)));
    const search = h('input', {
      type: 'text', placeholder: 'Filtrer le texte…', value: state.traceText, 'aria-label': 'Filtrer le texte',
      oninput: (e) => { state.traceText = e.target.value; fillTrace(); },
    });
    root.append(filters, h('div', { class: 'row' }, search), h('ol', { class: 'steps', id: 'trace-list' }));
    fillTrace();
  }

  function fillTrace() {
    const ol = $('#trace-list');
    if (!ol) return;
    clear(ol);
    const q = state.traceText.toLowerCase();
    const events = state.traceEvents.filter((e) =>
      (!state.traceKinds.size || state.traceKinds.has(e.kind)) && (!q || `${e.device} ${e.text}`.toLowerCase().includes(q)));
    for (const e of events) {
      ol.append(h('li', {
        class: `k-${e.kind}`, title: e.trace_id ? `Transaction #${e.trace_id}` : '',
        onclick: () => { if (e.trace_id) guarded(() => openJourney(e.trace_id)); },
      }, h('span', { class: 't' }, e.time), h('span', { class: 'd' }, e.device), h('span', { class: `k-${e.kind}` }, e.text)));
    }
    if (!events.length) ol.append(h('li', null, h('span'), h('span'), h('span', { class: 'muted' }, 'Aucun événement ne correspond.')));
  }

  // ---- analyse

  function renderAnalysis() {
    const root = clear($('#tab-analysis'));
    root.append(
      h('div', { class: 'btn-row' },
        libButton('Santé du réseau', showHealth),
        libButton('Analyse de sécurité', showSecurity)),
      h('h3', null, 'Et si… ?'),
      h('div', { class: 'row' },
        h('input', { type: 'text', id: 'wi-input', placeholder: 'R1 goes down · block tcp/443 · disconnect SW1', 'aria-label': 'Scénario what-if',
          onkeydown: (e) => { if (e.key === 'Enter') { e.preventDefault(); guarded(showWhatIf); } } }),
        libButton('Simuler', showWhatIf)),
      h('h3', null, 'Documentation'),
      h('div', { class: 'row' },
        h('select', { id: 'doc-format', 'aria-label': 'Format' }, ['md', 'json', 'yaml'].map((f) => h('option', { value: f }, f))),
        libButton('Générer', showDocs)),
      h('div', { id: 'analysis-out' }),
    );
  }

  function barFill(score) {
    return score >= 80 ? '' : score >= 50 ? 'mid' : 'low';
  }

  async function showHealth() {
    const r = await api('GET', `/api/labs/${requireLab()}/health`);
    const out = clear($('#analysis-out'));
    out.append(h('div', { class: 'global' }, `${r.global}/100`), h('p', { class: 'muted' }, 'Note globale (moyenne pondérée)'));
    const bars = h('div', { class: 'bars' });
    for (const c of r.categories) {
      bars.append(h('div', { class: 'bar' },
        h('span', null, c.name),
        h('div', { class: 'track', role: 'img', 'aria-label': `${c.name} : ${c.score} sur 100` }, h('div', { class: `fill ${barFill(c.score)}`, style: `width:${c.score}%` })),
        h('span', { class: 'val' }, `${c.score}`)));
    }
    out.append(bars);
    for (const c of r.categories) {
      if (c.notes.length) out.append(h('details', null, h('summary', null, `${c.name} — détail`), h('pre', null, c.notes.join('\n'))));
    }
  }

  async function showSecurity() {
    const r = await api('GET', `/api/labs/${requireLab()}/security`);
    const out = clear($('#analysis-out'));
    out.append(h('div', { class: 'global' }, `${r.score}/100`),
      h('p', { class: 'muted' }, `${r.counts.high} critique(s) · ${r.counts.medium} moyen(s) · ${r.counts.low} faible(s)`));
    if (!r.findings.length) out.append(h('p', null, 'Rien à signaler.'));
    for (const f of r.findings) {
      out.append(h('div', { class: `finding ${f.severity}` },
        h('div', { class: 'sev' }, `${f.severity} · ${f.id}`),
        h('strong', null, f.title),
        h('p', null, f.detail),
        h('p', { class: 'muted' }, `→ ${f.recommendation}`)));
    }
  }

  async function showWhatIf() {
    const text = $('#wi-input').value.trim();
    if (!text) throw new Error('Décris un scénario, par ex. « R1 goes down ».');
    const r = await api('POST', `/api/labs/${requireLab()}/what-if`, { scenario: text });
    clear($('#analysis-out')).append(h('pre', null, r.text));
  }

  async function showDocs() {
    const fmt = $('#doc-format').value;
    const r = await api('GET', `/api/labs/${requireLab()}/docs?format=${fmt}`);
    const out = clear($('#analysis-out'));
    const blob = new Blob([r.text], { type: 'text/plain;charset=utf-8' });
    const url = URL.createObjectURL(blob);
    out.append(h('p', null, h('a', { href: url, download: `${(state.snap && state.snap.name) || 'reseau'}.${fmt}` }, `Télécharger (.${fmt})`)),
      h('pre', null, r.text));
  }

  function requireLab() {
    if (!state.current) throw new Error('Ouvre d’abord un lab.');
    return state.current;
  }

  // ---- défi

  function formatTime(secs) {
    const sign = secs < 0 ? '+' : '';
    const a = Math.abs(secs);
    return `${sign}${Math.floor(a / 60)}:${String(a % 60).padStart(2, '0')}`;
  }

  function renderChallenge() {
    const root = clear($('#tab-challenge'));
    const c = state.snap && state.snap.challenge;
    if (!c) {
      root.append(h('p', { class: 'muted' }, 'Aucun défi en cours. Lance-en un depuis l’onglet « Défis » ou « Cyber range » du catalogue.'));
      return;
    }
    if (c.ticket) {
      root.append(h('div', { class: 'ticket-head' },
        h('span', { class: 'ticket-id' }, `Ticket #${c.ticket.id}`),
        h('span', null, `ouvert par ${c.ticket.requester}`)));
    }
    root.append(
      h('div', { class: 'chal-title' }, c.title),
      h('div', null, h('span', { class: 'pill' }, c.kind === 'range' ? 'cyber range' : 'défi'),
        c.finished ? h('span', { class: 'pill' }, c.score !== null && c.score !== undefined ? `terminé · ${c.score}/100` : 'terminé')
          : h('span', null, 'Temps restant : ', h('span', { class: 'timer', id: 'chal-timer' }, formatTime(Math.round((state.deadline - Date.now()) / 1000))))),
      h('div', { class: 'statement' }, c.description),
      h('p', { class: 'muted' }, `Indices utilisés : ${c.hints_used}${c.kind === 'challenge' ? ` · objectif atteint : ${c.fixed ? 'oui' : 'non'} · diagnostic : ${c.answered ? 'donné' : 'à donner'}` : ''}`),
    );
    if (c.finished) {
      root.append(h('p', null, 'Épreuve terminée. Les analyses (santé, diagnostic…) sont de nouveau disponibles.'));
      return;
    }
    const cmd = c.kind === 'range' ? 'range' : 'challenge';
    root.append(h('div', { class: 'btn-row' },
      libButton('Indice (−10)', () => runCommand('hint')),
      libButton('Vérifier', () => runCommand(`${cmd} check`)),
      libButton('Terminer', () => runCommand(`${cmd} finish`)),
      libButton('Abandonner', () => runCommand(`${cmd} giveup`), 'danger')));

    const lib = state.library;
    const dev = h('select', { id: 'ans-dev', 'aria-label': 'Équipement fautif' }, (state.snap.devices || []).map((d) => h('option', { value: d.name }, d.name)));
    const options = c.kind === 'range'
      ? lib.behaviors.map((b) => h('option', { value: b }, b))
      : lib.causes.map((x) => h('option', { value: x.slug }, `${x.label} (${x.slug})`));
    const cause = h('select', { id: 'ans-cause', 'aria-label': c.kind === 'range' ? 'Comportement' : 'Nature de la panne' }, options);
    root.append(h('h3', null, c.kind === 'range' ? 'Désigner le coupable' : 'Donner ton diagnostic'),
      h('div', { class: 'row' }, dev, cause),
      libButton('Valider la réponse', () => runCommand(`answer ${dev.value} ${cause.value}`), 'primary'));
  }

  function tickTimer() {
    const el = $('#chal-timer');
    if (!el || !state.deadline) return;
    const left = Math.round((state.deadline - Date.now()) / 1000);
    el.textContent = formatTime(left);
    el.classList.toggle('late', left < 0);
  }

  // ------------------------------------------------------------------ disposition : zones, tailles, réglages

  // Disposition choisie par la page hôte (`window.NETIV_LAYOUT`) :
  // * « lab » : l'épreuve est déjà choisie, le catalogue est replié et le
  //   panneau Défi ouvert ; ses réglages sont mémorisés à part pour ne pas
  //   modifier ceux du bac à sable ;
  // * sinon : l'interface complète.
  const LAB_LAYOUT = window.NETIV_LAYOUT === 'lab';
  const UI_KEY = LAB_LAYOUT ? 'netiv.ui.lab' : 'netiv.ui';
  const UI_DEFAULTS = {
    sideW: 280, panelW: 400, consoleH: 200,
    hideSide: LAB_LAYOUT, hidePanel: false, hideConsole: false, focus: false,
    font: 1, density: 'normal',
  };
  state.ui = { ...UI_DEFAULTS, ...(store(UI_KEY) || {}) };
  // Intégrée dans une page (iframe) : la page hôte affiche déjà le logo et le
  // titre. L'en-tête de NETIV disparaît et ses boutons rejoignent la barre du
  // réseau (une ligne de moins, pas de logo en double).
  if (window.NETIV_EMBEDDED) {
    document.body.classList.add('embedded');
    $('.stage-bar').append($('.top-actions'));
  }
  if (LAB_LAYOUT) document.body.classList.add('layout-lab');

  /** Bornes de chaque taille, recalculées selon la fenêtre. */
  function bounds(key) {
    switch (key) {
      case 'sideW': return [200, Math.max(220, Math.min(560, window.innerWidth * 0.4))];
      case 'panelW': return [280, Math.max(300, Math.min(820, window.innerWidth * 0.55))];
      default: return [80, Math.max(120, window.innerHeight * 0.7)];
    }
  }

  function clampSize(key, v) {
    const [lo, hi] = bounds(key);
    return Math.round(Math.max(lo, Math.min(hi, v)));
  }

  function applyUi() {
    const ui = state.ui;
    const root = document.documentElement.style;
    root.setProperty('--side-w', `${clampSize('sideW', ui.sideW)}px`);
    root.setProperty('--panel-w', `${clampSize('panelW', ui.panelW)}px`);
    root.setProperty('--console-h', `${clampSize('consoleH', ui.consoleH)}px`);
    root.setProperty('--font-scale', String(ui.font));
    document.body.classList.toggle('compact', ui.density === 'compact');
    const layoutEl = $('.layout');
    layoutEl.classList.toggle('hide-side', ui.hideSide);
    layoutEl.classList.toggle('hide-panel', ui.hidePanel);
    layoutEl.classList.toggle('hide-console', ui.hideConsole);
    layoutEl.classList.toggle('focus', ui.focus);
    $('#vb-side').setAttribute('aria-pressed', String(!ui.hideSide && !ui.focus));
    $('#vb-panel').setAttribute('aria-pressed', String(!ui.hidePanel && !ui.focus));
    $('#vb-console').setAttribute('aria-pressed', String(!ui.hideConsole && !ui.focus));
    $('#vb-focus').setAttribute('aria-pressed', String(ui.focus));
    $('#z-focus').setAttribute('aria-pressed', String(ui.focus));
    $('#z-focus').title = ui.focus ? 'Rétablir les panneaux  F' : 'Schéma seul (agrandir)  F';
    const consoleMax = atMax('consoleH');
    $('#console-max').setAttribute('aria-pressed', String(consoleMax));
    $('#console-max').title = consoleMax ? 'Rétablir la taille de la console' : 'Agrandir la console';
    $('#console-max').setAttribute('aria-label', $('#console-max').title);
    const panelMax = atMax('panelW');
    $('#panel-wide').setAttribute('aria-pressed', String(panelMax));
    $('#panel-wide').title = panelMax ? 'Rétablir la largeur du panneau' : 'Élargir le panneau';
    $('#panel-wide').setAttribute('aria-label', $('#panel-wide').title);
    for (const b of $$('#font-seg button')) b.setAttribute('aria-pressed', String(+b.dataset.font === ui.font));
    for (const b of $$('#density-seg button')) b.setAttribute('aria-pressed', String(b.dataset.density === ui.density));
    $('#console-size').value = String(clampSize('consoleH', ui.consoleH));
    for (const [id, key] of [['#split-side', 'sideW'], ['#split-panel', 'panelW'], ['#split-console', 'consoleH']]) {
      const [lo, hi] = bounds(key);
      const el = $(id);
      el.setAttribute('aria-valuemin', String(Math.round(lo)));
      el.setAttribute('aria-valuemax', String(Math.round(hi)));
      el.setAttribute('aria-valuenow', String(clampSize(key, ui[key])));
    }
  }

  function setUi(patch) {
    Object.assign(state.ui, patch);
    applyUi();
    store(UI_KEY, state.ui);
  }

  /** La taille `key` est-elle à son maximum ? */
  function atMax(key) {
    return clampSize(key, state.ui[key]) >= Math.round(bounds(key)[1]) - 1;
  }

  /** Agrandit un cadre au maximum, ou le rétablit à sa taille par défaut. */
  function toggleMax(key) {
    const show = key === 'consoleH' ? { hideConsole: false } : { hidePanel: false };
    setUi({ ...show, focus: false, [key]: atMax(key) ? UI_DEFAULTS[key] : bounds(key)[1] });
  }

  /** Affiche / masque une zone. Quitter le mode « schéma seul » d'abord. */
  function toggleZone(flag) {
    if (state.ui.focus) { setUi({ focus: false, [flag]: false }); return; }
    setUi({ [flag]: !state.ui[flag] });
  }

  /**
   * Poignée de redimensionnement. `sign` : +1 si glisser vers la droite / le
   * bas agrandit la zone, −1 sinon.
   */
  function initSplitter(el, key, axis, sign) {
    el.addEventListener('pointerdown', (e) => {
      if (e.button !== 0) return;
      e.preventDefault();
      const start = axis === 'x' ? e.clientX : e.clientY;
      const base = clampSize(key, state.ui[key]);
      el.setPointerCapture(e.pointerId);
      el.classList.add('dragging');
      document.body.classList.add('resizing', axis === 'x' ? 'col' : 'row');
      const move = (ev) => {
        const d = (axis === 'x' ? ev.clientX : ev.clientY) - start;
        state.ui[key] = clampSize(key, base + sign * d);
        applyUi();
      };
      const up = () => {
        el.removeEventListener('pointermove', move);
        el.classList.remove('dragging');
        document.body.classList.remove('resizing', 'col', 'row');
        store(UI_KEY, state.ui);
      };
      el.addEventListener('pointermove', move);
      el.addEventListener('pointerup', up, { once: true });
      el.addEventListener('pointercancel', up, { once: true });
    });
    el.addEventListener('dblclick', () => setUi({ [key]: UI_DEFAULTS[key] }));
    el.addEventListener('keydown', (e) => {
      const grow = axis === 'x' ? { ArrowRight: 1, ArrowLeft: -1 } : { ArrowDown: 1, ArrowUp: -1 };
      if (e.key in grow) {
        e.preventDefault();
        setUi({ [key]: clampSize(key, state.ui[key] + grow[e.key] * sign * (e.shiftKey ? 64 : 16)) });
      } else if (e.key === 'Enter' || e.key === 'Home') {
        e.preventDefault();
        setUi({ [key]: UI_DEFAULTS[key] });
      }
    });
  }

  async function openHelp() {
    const dlg = $('#help-dialog');
    if (state.current && !state.helpText) {
      try {
        const r = await api('POST', `/api/labs/${state.current}/exec`, { line: 'help' });
        state.helpText = (r.output || '').replace(/\n+$/, '');
      } catch (_) { /* l'aide des raccourcis suffit */ }
    }
    if (state.helpText) $('#help-commands').textContent = state.helpText;
    if (typeof dlg.showModal === 'function' && !dlg.open) dlg.showModal();
  }

  function initLayout() {
    applyUi();
    initSplitter($('#split-side'), 'sideW', 'x', 1);
    initSplitter($('#split-panel'), 'panelW', 'x', -1);
    initSplitter($('#split-console'), 'consoleH', 'y', -1);

    $('#vb-side').addEventListener('click', () => toggleZone('hideSide'));
    $('#vb-panel').addEventListener('click', () => toggleZone('hidePanel'));
    $('#vb-console').addEventListener('click', () => toggleZone('hideConsole'));
    $('#vb-focus').addEventListener('click', () => setUi({ focus: !state.ui.focus }));
    $('#z-focus').addEventListener('click', () => setUi({ focus: !state.ui.focus }));
    $('#console-max').addEventListener('click', () => toggleMax('consoleH'));
    $('#console-min').addEventListener('click', () => setUi({ consoleH: bounds('consoleH')[0], hideConsole: false }));
    $('#console-hide').addEventListener('click', () => setUi({ hideConsole: true }));
    $('#console-clear').addEventListener('click', () => { clear($('#console-out')); clearSuggestions(); $('#console-in').focus(); });
    $('#panel-wide').addEventListener('click', () => toggleMax('panelW'));
    $('#panel-hide').addEventListener('click', () => setUi({ hidePanel: true }));
    // Les bornes dépendent de la fenêtre : recalculer l'état des boutons.
    window.addEventListener('resize', applyUi);

    for (const b of $$('#font-seg button')) b.addEventListener('click', () => setUi({ font: +b.dataset.font }));
    for (const b of $$('#density-seg button')) b.addEventListener('click', () => setUi({ density: b.dataset.density }));
    $('#console-size').addEventListener('input', (e) => setUi({ consoleH: +e.target.value, hideConsole: false }));
    $('#layout-reset').addEventListener('click', () => { setUi({ ...UI_DEFAULTS }); fitView(); });
    $('#help-btn').addEventListener('click', openHelp);

    // Le menu Affichage se ferme en cliquant ailleurs ou avec Échap.
    const menu = $('#view-menu');
    document.addEventListener('pointerdown', (e) => { if (menu.open && !menu.contains(e.target)) menu.open = false; });

    document.addEventListener('keydown', (e) => {
      if (e.key === 'Escape' && menu.open) { menu.open = false; menu.querySelector('summary').focus(); return; }
      const t = e.target;
      const typing = t instanceof HTMLElement && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName));
      if (typing || e.ctrlKey || e.metaKey || e.altKey || document.querySelector('dialog[open]')) return;
      const actions = {
        '[': () => toggleZone('hideSide'),
        ']': () => toggleZone('hidePanel'),
        '`': () => toggleZone('hideConsole'),
        f: () => setUi({ focus: !state.ui.focus }),
        F: () => setUi({ focus: !state.ui.focus }),
        '+': () => zoomAt(1.25),
        '=': () => zoomAt(1.25),
        '-': () => zoomAt(0.8),
        0: fitView,
        '?': openHelp,
      };
      const act = actions[e.key];
      if (act) { e.preventDefault(); act(); }
    });

    // Les bornes dépendent de la taille de la fenêtre.
    let t = null;
    window.addEventListener('resize', () => { clearTimeout(t); t = setTimeout(applyUi, 120); });
  }

  // ------------------------------------------------------------------ amorçage

  function wire() {
    initTheme();
    initLayout();
    initCanvasControls();
    initBuild();
    $('#profile-chip').addEventListener('click', openProfile);

    for (const b of $$('.tabs button')) b.addEventListener('click', () => selectTab(b.dataset.tab));
    for (const b of $$('.side .seg button')) {
      b.addEventListener('click', () => {
        state.libTab = b.dataset.lib;
        for (const x of $$('.side .seg button')) x.setAttribute('aria-selected', String(x === b));
        renderLibrary();
      });
    }

    $('#tb-go').addEventListener('click', () => guarded(sendProbe));
    $('#tb-step').addEventListener('click', () => guarded(() => runCommand('step')));
    $('#tb-run').addEventListener('click', () => guarded(() => runCommand('run')));
    $('#tb-port').addEventListener('input', () => {
      const curl = $('#tb-port').value.trim() !== '';
      $('#tb-stepmode').disabled = curl;
      if (curl) $('#tb-stepmode').checked = false;
    });

    $('#console-form').addEventListener('submit', (e) => {
      e.preventDefault();
      const input = $('#console-in');
      const line = input.value.trim();
      if (!line) return;
      state.history.push(line);
      state.histPos = state.history.length;
      input.value = '';
      guarded(async () => {
        if (line === 'clear') { clear($('#console-out')); clearSuggestions(); return; }
        await runCommand(line);
      });
    });
    $('#console-in').addEventListener('keydown', (e) => {
      if (e.key === 'Tab' && !e.shiftKey && e.target.value.trim()) { e.preventDefault(); complete(e.target); return; }
      if (e.key !== 'Tab') clearSuggestions();
      if (e.key === 'ArrowUp' && state.histPos > 0) { e.preventDefault(); state.histPos -= 1; e.target.value = state.history[state.histPos]; }
      else if (e.key === 'ArrowDown') {
        e.preventDefault();
        state.histPos = Math.min(state.history.length, state.histPos + 1);
        e.target.value = state.history[state.histPos] || '';
      }
    });

    renderAnalysis();
    setInterval(tickTimer, 1000);
  }

  async function boot() {
    wire();
    renderLabs();
    renderStage();
    updateBuildToggle();
    renderDevice(null);
    renderJourney();
    renderChallenge();
    try {
      [state.info, state.library] = await Promise.all([api('GET', '/api/info'), api('GET', '/api/library')]);
      await loadProfile();
      consoleLine(`NETIV ${state.info.version} — tape « help » pour la liste des commandes.`, 'note');
      renderLibrary();
      // Labs encore ouverts sur le serveur (page rechargée) : on les reprend.
      const open = await api('GET', '/api/labs');
      if (open.labs.length) {
        state.labs = open.labs.map((l) => ({ id: l.id, name: l.name || l.id }));
        const last = state.labs[state.labs.length - 1];
        await guarded(async () => {
          state.current = last.id;
          applySnapshot(await api('GET', `/api/labs/${last.id}`));
          await loadTransactions({ selectNewest: true });
          renderChallenge();
        });
      } else {
        // Un premier réseau tout de suite, pour que l'écran ne soit pas vide.
        // Une page hôte peut choisir lequel (`window.NETIV_START`, par
        // exemple { challenge: 'beginner-01' }).
        await guarded(() => createLab(window.NETIV_START || { builtin: 'office' }));
      }
    } catch (e) {
      banner(e.message);
    }
  }

  // Chargé après coup (mode WebAssembly) : la page est déjà prête.
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', boot);
  else boot();
})();

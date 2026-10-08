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
    history: [],
    histPos: 0,
    deadline: null,    // horodatage de fin du temps conseillé
    busy: false,
  };

  // ------------------------------------------------------------------ API

  async function api(method, path, body) {
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
    if (!res.ok) throw new Error((data && data.error) || `Erreur HTTP ${res.status}`);
    return data;
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

  function consoleLine(text, cls) {
    const out = $('#console-out');
    const line = h('div', { class: cls || null }, text);
    out.append(line);
    while (out.childElementCount > 1500) out.removeChild(out.firstChild);
    out.scrollTop = out.scrollHeight;
  }

  async function runCommand(line, { echo = true } = {}) {
    if (!state.current) throw new Error("Aucun lab ouvert : choisis un réseau dans le catalogue.");
    if (echo) consoleLine(`netiv> ${line}`, 'cmd');
    const r = await api('POST', `/api/labs/${state.current}/exec`, { line });
    if (r.output) consoleLine(r.output.replace(/\n+$/, ''));
    if (r.error) consoleLine(`erreur : ${r.error}`, 'err');
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
    renderStage();
  }

  async function createLab(body) {
    const r = await api('POST', '/api/labs', body);
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

  function ensurePositions(snap) {
    const key = `netiv.pos.${snap.name}.${snap.devices.length}`;
    let saved = store(key);
    const names = snap.devices.map((d) => d.name);
    if (!saved || names.some((n) => !saved[n])) {
      const fitted = fitToView(layout(snap.devices, snap.links), VIEW_W, VIEW_H, 70);
      saved = {};
      for (const [n, p] of fitted) saved[n] = { x: Math.round(p.x), y: Math.round(p.y) };
    }
    state.positions = new Map(Object.entries(saved));
    state.posKey = key;
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
    $('#canvas-empty').hidden = !!(snap && snap.devices.length);
    if (!snap || !snap.devices.length) return;
    svg.setAttribute('viewBox', `0 0 ${VIEW_W} ${VIEW_H}`);
    svg.setAttribute('preserveAspectRatio', 'xMidYMid meet');
    if (!state.positions.size || state.posKey !== `netiv.pos.${snap.name}.${snap.devices.length}`) ensurePositions(snap);
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
      linkLayer.append(line);
    }

    for (const d of snap.devices) {
      const p = state.positions.get(d.name);
      if (!p) continue;
      const anyDown = d.ifaces.some((i) => !i.up || i.link_up === false);
      const g = s('g', {
        class: `node kind-${d.kind} role-${d.role}${state.device === d.name ? ' selected' : ''}${anyDown ? ' faulty' : ''}`,
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

  function attachNode(g, name) {
    const svg = $('#canvas');
    let drag = null;
    g.addEventListener('pointerdown', (e) => {
      drag = { moved: false, sx: e.clientX, sy: e.clientY };
      g.setPointerCapture(e.pointerId);
    });
    g.addEventListener('pointermove', (e) => {
      if (!drag) return;
      if (!drag.moved && Math.hypot(e.clientX - drag.sx, e.clientY - drag.sy) < 4) return;
      drag.moved = true;
      const p = svgPoint(svg, e);
      state.positions.set(name, { x: Math.max(30, Math.min(VIEW_W - 30, p.x)), y: Math.max(30, Math.min(VIEW_H - 50, p.y)) });
      renderCanvas();
    });
    const end = () => {
      if (!drag) return;
      if (drag.moved) {
        const o = {};
        for (const [n, p] of state.positions) o[n] = { x: Math.round(p.x), y: Math.round(p.y) };
        store(state.posKey, o);
      } else {
        selectDevice(name);
      }
      drag = null;
    };
    g.addEventListener('pointerup', end);
    g.addEventListener('pointercancel', () => { drag = null; });
    g.addEventListener('keydown', (e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); selectDevice(name); } });
  }

  // ------------------------------------------------------------------ en-tête du schéma, barre d'outils

  function renderStage() {
    const snap = state.snap;
    $('#lab-title').textContent = snap && snap.name ? snap.name : 'Aucun réseau';
    $('#lab-meta').textContent = snap && snap.name
      ? `${snap.devices.length} équipements · ${snap.links.length} câbles · t = ${snap.time} · ${snap.trace_len} événements${snap.pending ? ` · ${snap.pending} en attente` : ''}`
      : '';
    renderToolbar();
    renderCanvas();
  }

  function renderToolbar() {
    const snap = state.snap;
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
      h('button', { type: 'button', id: 'pl-first', title: 'Début', 'aria-label': 'Début', onclick: () => gotoStep(-1) }, '⏮'),
      h('button', { type: 'button', id: 'pl-prev', title: 'Étape précédente (←)', 'aria-label': 'Étape précédente', onclick: () => gotoStep(state.step - 1) }, '◀'),
      h('button', { type: 'button', id: 'pl-play', class: 'primary', title: 'Lecture / pause (espace)', 'aria-label': 'Lecture', onclick: togglePlay }, state.playing ? '⏸' : '▶'),
      h('button', { type: 'button', id: 'pl-next', title: 'Étape suivante (→)', 'aria-label': 'Étape suivante', onclick: () => gotoStep(state.step + 1) }, '▶|'),
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
    if (b) { b.textContent = '⏸'; b.setAttribute('aria-label', 'Pause'); }
    gotoStep(state.step + 1);
  }

  function stopPlay() {
    if (state.playing) clearInterval(state.playing);
    state.playing = null;
    const b = $('#pl-play');
    if (b) { b.textContent = '▶'; b.setAttribute('aria-label', 'Lecture'); }
  }

  // ---- trace

  const KIND_FILTERS = [
    ['app', 'Échanges'], ['frame_sent', 'Envois'], ['frame_received', 'Réceptions'], ['switch', 'Commutation'],
    ['arp', 'ARP'], ['route', 'Routage'], ['drop', 'Refus'], ['config', 'Config'],
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

  // ------------------------------------------------------------------ amorçage

  function wire() {
    initTheme();
    $('#profile-chip').addEventListener('click', openProfile);

    for (const b of $$('.tabs button')) b.addEventListener('click', () => selectTab(b.dataset.tab));
    for (const b of $$('.seg button')) {
      b.addEventListener('click', () => {
        state.libTab = b.dataset.lib;
        for (const x of $$('.seg button')) x.setAttribute('aria-selected', String(x === b));
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
        if (line === 'clear') { clear($('#console-out')); return; }
        await runCommand(line);
      });
    });
    $('#console-in').addEventListener('keydown', (e) => {
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
    renderDevice(null);
    renderJourney();
    renderChallenge();
    try {
      [state.info, state.library] = await Promise.all([api('GET', '/api/info'), api('GET', '/api/library')]);
      await loadProfile();
      consoleLine(`NETIV ${state.info.version} — tape « help » pour la liste des commandes.`, 'note');
      renderLibrary();
      // Un premier réseau tout de suite, pour que l'écran ne soit pas vide.
      await guarded(() => createLab({ builtin: 'office' }));
    } catch (e) {
      banner(e.message);
    }
  }

  document.addEventListener('DOMContentLoaded', boot);
})();

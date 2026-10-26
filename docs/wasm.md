# NETIV dans le navigateur (WebAssembly)

NETIV tourne aussi **entièrement dans le navigateur**, sans serveur : le moteur Rust est compilé en
WebAssembly (crate `netiv-wasm`) et l'interface web est la même que celle de `netiv serve`.
C'est ce qui permet de l'intégrer à une plateforme web (ITEtude) avec de simples fichiers
statiques.

## Compiler

Outils : la cible `wasm32-unknown-unknown`, [wasm-pack](https://github.com/wasm-bindgen/wasm-pack)
et `wasm-opt` ([binaryen](https://github.com/WebAssembly/binaryen)).

```bash
rustup target add wasm32-unknown-unknown
crates/netiv-wasm/build-web.sh            # → crates/netiv-wasm/dist/
python3 -m http.server -d crates/netiv-wasm/dist 8000   # démo : http://localhost:8000
node crates/netiv-wasm/tests/wasm.mjs     # tests du vrai module (Node 22)
```

Le script :

1. compile `netiv-wasm` avec le profil `wasm-release` du workspace : `opt-level = "z"`, LTO,
   `codegen-units = 1`, `panic = "abort"`. Le profil `release` de la CLI native n'est pas touché ;
2. optimise le module avec `wasm-opt -Oz` ;
3. assemble un dossier statique autonome (`dist/`) :

| Fichier | Rôle |
|---|---|
| `index.html`, `app.js`, `style.css`, `favicon.svg` | l'interface de `netiv serve`, inchangée (seul le chargement de `app.js` passe par `netiv-boot.js`) |
| `netiv-boot.js` | charge le module, crée le moteur, l'installe pour l'interface, dialogue avec la page hôte |
| `netiv_wasm.js`, `netiv_wasm_bg.wasm`, `netiv_wasm.d.ts` | le moteur (wasm-bindgen) |
| `catalog.json` | les 18 défis et 3 cyber ranges (titre, énoncé, thème, difficulté, durée), sans solutions |
| `manifest.json` | version, taille et empreinte SHA-256 de chaque fichier, poids à télécharger |

La démo se sert avec n'importe quel serveur de fichiers statiques. Elle ne fait **aucune
requête** en dehors de ces fichiers.

## Taille

Mesurée sur NETIV 0.4.0 (rustc 1.99, wasm-bindgen 0.2, binaryen 133) :

| | Octets |
|---|---|
| `.wasm` brut | 2 039 235 (1,95 Mo) |
| `.wasm` gzip -9 | **730 249 (713 Ko)** |
| `.wasm` brotli 11 | **542 755 (530 Ko)** |
| Tout le dossier à télécharger (gzip : wasm + js + css + html) | 777 154 (759 Ko) |

L'objectif (moins de 1,5 Mo compressé) est atteint sans retirer de module : sécurité,
forensique, générateurs et cyber ranges sont tous présents. La CI échoue si le module compressé
dépasse 1,5 Mo.

## API JavaScript

```js
import init, { NetivEngine } from './netiv_wasm.js';
await init({ module_or_path: new URL('netiv_wasm_bg.wasm', import.meta.url) });

const engine = new NetivEngine(JSON.stringify({
  profileName: 'awa',          // nom de l'élève (« eleve » par défaut)
  profile: savedProfileJson,   // profil sauvegardé ailleurs (facultatif)
  labs: { 'mon-lab': netivText }, // bibliothèque de labs (facultatif)
}));

const lab = engine.request('POST', '/api/labs', { builtin: 'office' });
const r = engine.request('POST', `/api/labs/${lab.id}/exec`, { line: 'ping PC1 192.168.1.1' });
```

| Méthode | Rôle |
|---|---|
| `new NetivEngine(options?)` | options : texte JSON, tout est facultatif |
| `request(method, path, body?)` | **même contrat que l'API HTTP** ([api.md](api.md)) : mêmes chemins, mêmes JSON. `body` : objet, texte JSON ou rien. Rend l'objet de la réponse ; en cas d'erreur, **lève `{ status, error }`** avec le code HTTP équivalent (400, 403, 404, 405, 409, 413, 422) |
| `exportProfile()` | le profil de l'élève (texte JSON, avec `schema_version`), à sauvegarder |
| `importProfile(json)` | remplace le profil (labs ouverts compris) |
| `exportLab(id)` | le réseau du lab en `.netiv` (texte) |
| `importLab(text)` | ouvre un lab depuis un texte `.netiv` ; rend `{ id, output, snapshot }` |
| `NetivEngine.version()` | version de NETIV |
| `NetivEngine.catalog()` | catalogue des épreuves (comme `catalog.json`) |

L'heure vient de `Date.now()` : elle sert à chronométrer les défis. Le moteur ne contient pas
d'horloge système, qui n'existe pas en WebAssembly.

Une épreuve réussie est visible dans `snapshot.challenge` : `finished`, `score`, et
`score_detail` (`diagnosis`, `fix`, `time`, `hint_penalty`, `total`, `xp`, `elapsed_secs`, `hints`).

## Intégrer l'interface dans une page

Le plus simple est une **iframe de la même origine** qui charge `index.html?host=1`. L'interface
garde son propre CSS, et la page hôte ne charge rien tant que l'iframe n'est pas affichée.

### Paramètres d'URL

| Paramètre | Effet |
|---|---|
| `host=1` | mode intégré : attend le message `init` de la page hôte, ne garde rien dans le stockage local |
| `challenge=<id>`, `range=<id>`, `builtin=<nom>` | épreuve ou réseau ouvert au démarrage (sans `host=1`) |
| `layout=lab` | disposition « lab » (sans `host=1`) : voir ci-dessous |

Sans `host=1` (démo), le profil est gardé dans le `localStorage` du navigateur (`netiv.profile`).

### Protocole (postMessage)

Les messages ne sont échangés qu'avec la **même origine** (`window.location.origin`), dans les
deux sens. Tout autre message est ignoré.

Page hôte → NETIV (`source: 'netiv-host'`) :

| `type` | Champs | Rôle |
|---|---|---|
| `init` | `profile?`, `profileName?`, `start?`, `labs?`, `layout?` | à envoyer en réponse à `ready`. `start` : par exemple `{ challenge: 'beginner-01' }`, `{ range: 'range-01' }`, `{ builtin: 'office' }`, `{ yaml: texte }`. `layout: 'lab'` : disposition « lab » |
| `export-lab` | `requestId`, `labId?` | demande le `.netiv` du lab affiché (ou de `labId`) |

NETIV → page hôte (`source: 'netiv'`) :

| `type` | Champs | Quand |
|---|---|---|
| `ready` | `version` | module chargé ; NETIV attend `init` |
| `profile` | `profile` (JSON) | le profil a changé (tentative, leçon lue, épreuve réussie) |
| `challenge-finished` | `kind`, `id`, `title`, `score`, `scoreDetail`, `hintsUsed`, `profile` | une épreuve vient d'être réussie |
| `lab` | `requestId`, `id`, `text`, `error?` | réponse à `export-lab` |
| `error` | `error` | le moteur n'a pas pu démarrer |

**Disposition « lab »** (`layout: 'lab'`) : pour une épreuve imposée par la page hôte, le
catalogue de gauche est replié et l'onglet Défi s'ouvre au démarrage du défi. L'élève peut
toujours réafficher le catalogue (bouton Affichage). Ces réglages d'affichage sont mémorisés à
part (`netiv.ui.lab`) : ils ne modifient pas ceux de l'interface complète. En mode intégré
(`host=1`), l'en-tête de NETIV est compact, et tient sur une ligne sur téléphone.

Un profil illisible ou trop récent est ignoré : NETIV démarre avec un profil neuf et l'écrit
dans la console. Il ne bloque jamais l'élève.

### En-têtes à prévoir côté serveur

- `Content-Type: application/wasm` pour le `.wasm` : nécessaire à la compilation en streaming.
- Une CSP qui autorise `'wasm-unsafe-eval'` dans `script-src`, sur ces pages uniquement.
- Un cache long : le dossier est versionné, par exemple `/netiv/0.4.0/`.

## Limites

- **Rien n'est envoyé à un serveur.** Les labs ouverts disparaissent au rechargement de la page.
  La progression (profil) est conservée par la page hôte, ou par le `localStorage` en démo.
- **Pas d'accès aux fichiers.** `load <fichier>`, `lab export <fichier>`, `capture save`, `docs … <fichier>`,
  `course new`… répondent « accès aux fichiers indisponible depuis l'interface web (utilise la
  CLI) ». L'interface propose à la place l'import d'un fichier et le téléchargement
  `.netiv` / `.yaml`.
- **Un seul thread.** Les sondes (diagnostic, what-if, santé, sécurité) s'exécutent l'une après
  l'autre, avec des résultats identiques. Les analyses de très grands réseaux sont donc plus
  lentes qu'en natif.
- **Classement local** : il ne contient que le profil de l'élève. Un classement entre élèves
  relève de la plateforme hôte.
- **Le score est calculé dans le navigateur.** Une plateforme ne doit pas le considérer comme
  infalsifiable : voir le cahier des charges d'intégration.

## Garanties

- **Mêmes résultats qu'en natif.** `crates/netiv-wasm/tests/wasm.mjs` rejoue le scénario golden
  de la CLI dans le vrai module et compare la sortie octet pour octet. Le 1er ping est à 16 ms, le
  2ᵉ à 8 ms.
- **Même API.** `crates/netiv-api/tests/parity.rs` vérifie que `Service::request` (utilisé par le
  moteur) répond comme l'API HTTP : mêmes codes, mêmes JSON.
- **CI** (job `wasm`) : compilation wasm32 de toutes les crates du moteur, construction du
  paquet, tests Node du module, contrôle de la taille.

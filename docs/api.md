# API HTTP

`netiv serve` démarre l'interface web et une API JSON, **en local** :

```bash
netiv serve                          # http://127.0.0.1:8787
netiv serve --bind 127.0.0.1:9000 --labs-dir mes-labs --profile alice
```

L'interface web n'utilise que cette API : tout ce qu'elle fait peut être scripté avec `curl`.

## Principes

- Un **lab** est une session NETIV en mémoire (un réseau, sa trace, un défi éventuel). Les labs
  ne survivent pas à l'arrêt du serveur ; la **progression** (XP, badges) est enregistrée dans le
  profil, comme en CLI.
- Toutes les réponses sont en JSON. Une erreur HTTP renvoie `{"error": "message"}` avec le code
  400 (requête invalide), 403 (hôte refusé, ou analyse demandée pendant un défi), 404 (lab ou
  ressource inconnus), 409 (réseau verrouillé pendant un défi), 413 (corps trop gros), 415 (corps
  qui n'est pas du JSON) ou 422 (JSON qui ne correspond pas au format attendu).
- Une **commande** qui échoue (`POST …/exec`) n'est pas une erreur HTTP : la réponse est 200 avec
  le message dans le champ `error`, comme la CLI qui affiche l'erreur et continue.

## Endpoints

| Méthode | Chemin | Rôle |
|---|---|---|
| GET | `/` , `/app.js`, `/style.css`, `/favicon.svg` | Interface web (fichiers embarqués dans le binaire) |
| GET | `/api/info` | Version, profil, limites |
| GET | `/api/library` | Catalogue : réseaux livrés, labs, défis, ranges, causes, comportements, générateur |
| GET | `/api/profile` | Profil : niveau, XP, badges, défis réussis, tableau de bord |
| GET | `/api/leaderboard` | Classement local |
| GET | `/api/labs` | Labs ouverts |
| POST | `/api/labs` | Crée un lab |
| GET | `/api/labs/{id}` | Instantané du lab (équipements, câbles, état, défi) |
| DELETE | `/api/labs/{id}` | Ferme le lab |
| POST | `/api/labs/{id}/exec` | Exécute une ligne de commande NETIV |
| GET | `/api/labs/{id}/trace?since=N&limit=M` | Événements de trace à partir de l'index `N` (500 par défaut, 2 000 au plus) |
| GET | `/api/labs/{id}/transactions` | Les 50 derniers échanges (ping, curl, udp, DHCP, DNS) et leur issue |
| GET | `/api/labs/{id}/journey/{tid}` | Parcours d'un échange : étapes animables + texte |
| GET | `/api/labs/{id}/device/{name}` | Tables d'un équipement (interfaces, routes, ARP, MAC, ACL, services) |
| GET | `/api/labs/{id}/health` | Score de santé par catégorie |
| GET | `/api/labs/{id}/security` | Analyse de sécurité |
| GET | `/api/labs/{id}/docs?format=md\|json\|yaml` | Documentation générée |
| POST | `/api/labs/{id}/what-if` | Impact d'un changement |
| GET | `/api/labs/{id}/topology?format=json\|yaml\|netiv` | Topologie actuelle (éditeur ; texte à télécharger) |
| PUT | `/api/labs/{id}/topology` | Remplace le réseau (éditeur) |
| POST | `/api/labs/{id}/topology/validate` | Vérifie une topologie sans rien changer |
| POST | `/api/labs/{id}/topology/autoconfig` | Adressage automatique |
| PUT | `/api/labs/{id}/positions` | Positions des équipements sur le schéma |

### Créer un lab

Le corps contient **exactement une** source :

| Corps | Effet |
|---|---|
| `{"empty": "mon-reseau"}` | Réseau vide, à construire dans l'éditeur |
| `{"builtin": "office"}` | Réseau livré (`office`, `dmz`, `campus`, `triangle`, `cyber`, `dhcp`, `pme`) |
| `{"yaml": "network: …"}` | Topologie fournie (YAML, JSON ou `.netiv`) |
| `{"generate": {"type": "campus", "routers": 5, "hosts": 12, "seed": 1}}` | Topologie générée |
| `{"challenge": "beginner-01"}` | Défi livré |
| `{"range": "range-01"}` | Scénario de cyber range |
| `{"lab": "bureau"}` | Lab de la bibliothèque (`--labs-dir`) |
| `{"generate_challenge": {"topic": "routing", "difficulty": "hard", "seed": 1}}` | Défi généré |

```bash
curl -s -X POST http://127.0.0.1:8787/api/labs \
     -H 'Content-Type: application/json' -d '{"builtin": "office"}'
```

```json
{ "id": "lab-1", "output": "Réseau « office » chargé : 6 équipement(s), 5 câble(s).\n", "snapshot": { "…": "…" } }
```

### Instantané (`snapshot`)

Renvoyé par `GET /api/labs/{id}`, à la création et après chaque commande :

```json
{
  "id": "lab-1", "name": "office", "time": "16.000 ms", "time_us": 16000,
  "pending": 0, "trace_len": 66,
  "devices": [
    { "name": "PC1", "kind": "host", "role": "user", "forwarding": false, "filtered": false,
      "dhcp_server": null, "dns_server": null,
      "ifaces": [ { "name": "eth0", "ip": "192.168.1.10/24", "mac": "02:4E:49:00:00:01",
                    "up": true, "link_up": true, "peer": "SW1:port1",
                    "dhcp": false, "dhcp_relay": null } ] }
  ],
  "links": [
    { "a": "PC1:eth0", "a_device": "PC1", "b": "SW1:port1", "b_device": "SW1",
      "up": true, "loss": 0.0, "latency_us": 1000 }
  ],
  "challenge": null
}
```

Pendant un défi, `challenge` vaut par exemple
`{"id": "beginner-01", "kind": "challenge", "title": "…", "description": "…", "remaining_secs": 900,
"hints_used": 0, "answered": false, "fixed": false, "finished": false, "score": null,
"score_detail": null}`.

Une fois l'épreuve réussie, `score` donne le total (défis seulement) et `score_detail` le détail,
pour un défi comme pour un cyber range : `{"diagnosis": 40, "fix": 40, "time": 15,
"hint_penalty": 10, "total": 85, "xp": 85, "elapsed_secs": 240, "hints": 1}`.

### Exécuter une commande

```bash
curl -s -X POST http://127.0.0.1:8787/api/labs/lab-1/exec \
     -H 'Content-Type: application/json' -d '{"line": "ping PC1 192.168.2.10"}'
```

```json
{
  "output": "PING 192.168.2.10 depuis PC1 (1 requête(s))\n  seq 1  réponse de 192.168.2.10  ttl=62  temps=16.000 ms   [parcours #1]\n…",
  "error": null,
  "new_events_from": 0,
  "quit": false,
  "snapshot": { "…": "…" }
}
```

`new_events_from` est l'index du premier événement de trace produit par la commande : pour
suivre la trace en continu, appeler ensuite `GET …/trace?since=<new_events_from>`. Une ligne fait
au plus 1 000 caractères.

### Événements de trace

Chaque événement : `seq`, `time`, `time_us`, `device`, `trace_id`, `kind` (catégorie :
`app`, `frame_sent`, `frame_received`, `switch`, `arp`, `route`, `dhcp`, `dns`, `drop`, `config`,
`fault`),
`text` (description lisible) et `edge` (`{"from": "R1", "to": "R2"}` quand une trame traverse un
câble, sinon `null` ; c'est ce qui anime le paquet sur le schéma).

### Éditeur de réseau

L'éditeur travaille sur la **description** du réseau (même format que les fichiers YAML, en JSON) :

```bash
# Lire
curl -s http://127.0.0.1:8787/api/labs/lab-1/topology            # {"topology": {...}}
# Vérifier sans appliquer
curl -s -X POST http://127.0.0.1:8787/api/labs/lab-1/topology/validate \
     -H 'Content-Type: application/json' -d '{"topology": {...}}'   # {"errors": [], "warnings": [...]}
# Appliquer
curl -s -X PUT http://127.0.0.1:8787/api/labs/lab-1/topology \
     -H 'Content-Type: application/json' -d '{"topology": {...}}'   # {"warnings": [...], "snapshot": {...}}
```

- Appliquer **reconstruit** le lab : la trace repart de zéro (environ 1 ms pour 150 équipements).
- Topologie invalide : 400, avec les problèmes dans `error` ; le lab garde son état précédent.
- Pendant un défi ou un cyber range : **409** (`edit_lock` dans l'instantané en donne la raison,
  `editable` vaut `false`).
- Au plus 2 000 équipements et 4 000 câbles par topologie.
- `autoconfig` renvoie `{"changes": ["PC1:eth0 → 10.0.1.10/24", …], "snapshot": …}`.
- `PUT …/positions` avec `{"positions": {"PC1": [120, 80]}}` mémorise la disposition sans
  reconstruire. Les positions sont aussi renvoyées dans l'instantané (`devices[].pos`) et exportées
  dans les fichiers.

### What-if

```bash
curl -s -X POST http://127.0.0.1:8787/api/labs/lab-1/what-if \
     -H 'Content-Type: application/json' -d '{"scenario": "R1 goes down"}'
```

Réponse : `broken`, `affected_hosts`, `affected_services`, `isolated_hosts`, `isolated_segments`,
`broken_routes`, `ok_before`, `ok_after`, `flows`, `harmless` et `text` (rendu lisible).

## Garde-fous de sécurité

L'API pilote un simulateur **sans authentification**. Elle est donc conçue pour rester locale :

| Mesure | Détail |
|---|---|
| Écoute locale | Toute adresse non locale est refusée au démarrage, sauf avec `--allow-remote` (déconseillé). |
| Contrôle de `Host` | Seuls `127.0.0.1:<port>`, `localhost:<port>` et `[::1]:<port>` sont acceptés (protection contre le *DNS rebinding*). Sinon : 403. |
| Bac à sable | Les sessions de l'API ne lisent ni n'écrivent aucun fichier : `load <fichier>`, `capture save`, `lab export`, `docs … <fichier>` sont refusés. Les réseaux viennent du catalogue, de la bibliothèque `labs/` ou du corps de la requête. |
| Limites | 16 labs ouverts au plus, corps de requête ≤ 1 Mio, ligne de commande ≤ 1 000 caractères, limite d'événements par commande (le moteur s'arrête proprement sur une boucle). |
| En-têtes | `Content-Security-Policy` stricte (aucune ressource externe), `X-Frame-Options: DENY`, `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, `Cache-Control: no-store` sur l'API. |
| Interface | Tout texte venant du serveur est inséré comme texte (jamais comme HTML) : un nom d'équipement piégé reste inoffensif. |
| 100 % simulé | Aucune socket réseau n'est ouverte par la simulation ; seul le serveur HTTP écoute. |

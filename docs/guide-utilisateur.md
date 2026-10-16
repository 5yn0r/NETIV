# Guide utilisateur

NETIV est un laboratoire réseau **entièrement simulé** : aucun paquet ne sort de ta machine.
On y construit un réseau, on l'observe paquet par paquet, on le casse, puis on comprend
pourquoi il ne marche plus.

## 1. Installer et lancer

Il faut Rust (installation via [rustup](https://rustup.rs)). Depuis la racine du dépôt :

```bash
cargo build --release
./target/release/netiv --help
```

Deux façons de travailler :

| Mode | Commande | Idéal pour |
|---|---|---|
| Terminal | `netiv run examples/mvp.yaml` | aller vite, écrire des scripts |
| Navigateur | `netiv serve` puis <http://127.0.0.1:8787> | voir le schéma et le parcours animé des paquets |

Les deux modes utilisent les mêmes commandes : la console de l'interface web accepte tout ce
que le terminal accepte.

## 2. Premier réseau, premier ping

```text
$ netiv run examples/mvp.yaml
Réseau « mvp » chargé : 5 équipement(s), 4 câble(s).

netiv> ping PC1 192.168.2.10 2
PING 192.168.2.10 depuis PC1 (2 requête(s))
  seq 1  réponse de 192.168.2.10  ttl=62  temps=16.000 ms   [parcours #1]
  seq 2  réponse de 192.168.2.10  ttl=62  temps=8.000 ms    [parcours #2]
```

Pourquoi le 1ᵉʳ ping prend-il deux fois plus de temps ? Demande-le au simulateur :

```text
netiv> trace 1      # Packet Journey : chaque étape, dans l'ordre (ARP compris)
netiv> inspect 49   # Packet Microscope : une trame, couche par couche
netiv> explain 25   # Explain Engine : pourquoi ce routeur a choisi cette route
```

Réponse : le 1ᵉʳ ping déclenche une résolution ARP sur chacun des trois segments ; le 2ᵉ
profite des caches. Tout est **déterministe** : même réseau, mêmes commandes, même graine →
exactement la même trace.

Pour avancer événement par événement : `ping PC1 192.168.2.10 --step`, puis `step` (ou `run`).

## 3. Casser, puis réparer

```text
netiv> gateway PC1 192.168.1.254     # mauvaise passerelle
netiv> ping PC1 192.168.2.10         # → pas de réponse
netiv> diagnose PC1 192.168.2.10     # contrôles [✓]/[✗] + cause racine
netiv> hint                          # indice, du plus vague à la solution
netiv> gateway PC1 192.168.1.1       # correction
```

Laisser NETIV choisir la panne (cachée) :

```text
netiv> chaos start PC1 192.168.2.10 --layer l3
netiv> diagnose PC1 192.168.2.10
netiv> chaos reveal                  # si tu sèches
netiv> chaos undo                    # tout réparer
```

## 4. Défis et progression

```text
netiv> challenge list                 # 15 défis, du débutant à l'avancé
netiv> challenge start beginner-01    # une panne cachée, un objectif
netiv> challenge show                 # énoncé, temps restant
netiv> answer PC1 wrong-gateway       # ton diagnostic (+40 s'il est juste)
netiv> challenge check                # l'objectif fonctionne-t-il ? (+40)
netiv> dashboard                      # niveau, XP, badges
```

Barème : diagnostic +40, correction +40, rapidité jusqu'à +20, chaque indice −10 et chaque
mauvaise réponse −10 (plancher 0). Les XP sont multipliés par 1, 1,5 ou 2 selon la difficulté.
La liste des causes acceptées par `answer` : `challenge causes`.

La progression est enregistrée dans `~/.local/share/netiv/profiles/<nom>.json`
(`--profile <nom>` pour choisir le profil). `leaderboard` affiche le classement local.

`learn` propose un parcours guidé (lectures + défis cochés au fur et à mesure) et `tutor <question>`
explique une notion, ou guide sur le défi en cours sans jamais donner la solution trop tôt.

## 5. Cyber range : enquêter sur un incident

```text
netiv> range start range-01           # l'incident a déjà eu lieu
netiv> forensic analyze live          # chronologie + détections (balayage, rebond…)
netiv> show acl FW1                   # pourquoi le pare-feu a laissé passer ?
netiv> acl del FW1 1                  # correction
netiv> range check                    # attaque bloquée ET trafic légitime intact ?
netiv> answer WEB01 port-scan         # le coupable et son comportement
```

## 6. L'interface web

`netiv serve` ouvre l'interface sur <http://127.0.0.1:8787>.

![Interface web de NETIV](img/interface.png)

- **Catalogue** (à gauche) : réseaux livrés, bibliothèque `labs/`, défis, cyber ranges, générateurs.
- **Schéma** (au centre) : clique un équipement pour voir ses tables. Glisse-le pour le déplacer.
  Molette pour zoomer, glisser le fond pour se déplacer ; boutons + − ⤢ (tout afficher) ↻ (réorganiser).
- **Barre d'outils** : ping ou requête TCP (remplir le port) d'un clic ; « pas à pas » pour suivre chaque événement.
- **Console** : les mêmes commandes que le terminal, avec historique (↑ ↓).
- **Panneau de détails** : Appareil, **Parcours** (rejoue un échange étape par étape, avec le paquet
  animé sur le schéma : ◀ ▶ lecture, ← → espace), Trace filtrable, Analyse (santé, sécurité, what-if,
  documentation), Défi (minuteur, indices, réponse).

### Adapter l'affichage

| Action | Souris | Clavier |
|---|---|---|
| Agrandir / réduire une zone | glisser son bord | Tab jusqu'au bord, puis ← → ou ↑ ↓ (Maj : pas plus grand) |
| Revenir à la taille par défaut | double-clic sur le bord | Entrée sur le bord |
| Masquer le catalogue / les détails / la console | boutons en haut à droite | `[` `]` `` ` `` |
| Schéma seul | bouton ⛶ | `F` |
| Zoom du schéma | molette, boutons + − ⤢ | `+` `−` `0` |
| Taille du texte, densité, hauteur de console | menu **Affichage** | — |
| Aide et liste des commandes | bouton **?** | `?` |

La disposition, le zoom et les positions des équipements sont mémorisés **dans ton navigateur**
(par réseau). « Réinitialiser la disposition » dans le menu Affichage remet tout à zéro.

### Sécurité de l'interface

Le serveur n'écoute que sur `127.0.0.1` ; une autre adresse est refusée sans `--allow-remote`
(l'API n'a pas d'authentification). Les sessions web ne peuvent pas lire ni écrire de fichiers
sur le disque. Détails dans [api.md](api.md).

## 7. Créer et partager ses réseaux

- Écrire un réseau à la main : [format-topologie.md](format-topologie.md), puis `netiv validate mon-reseau.yaml`.
- En générer un : `netiv generate --type campus --routers 5 --hosts 20 --seed 1 --out campus.yaml`.
- Le partager : `lab export mon-lab.netiv "Mon titre"` dans la session, puis déposer le fichier
  dans le dossier `labs/` de l'autre personne (`lab list`, `lab open <nom>`).
- Documenter un réseau : `netiv docs mon-reseau.yaml --format md > reseau.md`
  (PDF : `pandoc reseau.md -o reseau.pdf`).

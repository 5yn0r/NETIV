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

## 6. DHCP et DNS

Le réseau `dhcp` du catalogue (`netiv run` puis `load`, ou « Ouvrir » dans le navigateur) a des
postes sans adresse écrite : ils la demandent à un serveur au démarrage.

```text
netiv> show dhcp SRV            # plages, occupation, baux
netiv> trace 1                  # Discover → Offer → Request → Ack, relais compris
netiv> ping PC1 web.lan         # résolution DNS, puis ping
netiv> dhcp PC2 release         # rendre l'adresse…
netiv> dhcp PC2                 # … et la redemander (même MAC → même adresse)
netiv> dhcp relay R1 eth0 none  # casser le relais
netiv> dhcp PC1 release ; dhcp PC1
netiv> diagnose PC1 10.10.2.10  # → relais DHCP absent sur R1, et la commande pour réparer
netiv> diagnose PC1 web.lan     # contrôle la résolution DNS puis le chemin
```

Trois défis (`beginner-06`, `intermediate-06`, `advanced-06`) portent sur un serveur arrêté, un
relais oublié et un serveur DHCP pirate.

## 7. L'interface web

`netiv serve` ouvre l'interface sur <http://127.0.0.1:8787>.

![Interface web de NETIV](img/interface.png)

- **Catalogue** (à gauche) : réseaux livrés, bibliothèque `labs/`, défis, cyber ranges, générateurs.
- **Schéma** (au centre) : clique un équipement pour voir ses tables. Glisse-le pour le déplacer.
  Molette pour zoomer, glisser le fond pour se déplacer ; boutons + − ⤢ (tout afficher) ↻ (réorganiser).
- **Barre d'outils** : ping ou requête TCP (remplir le port) d'un clic ; « pas à pas » pour suivre chaque événement
  (les boutons « Pas à pas » et « Tout exécuter » apparaissent quand des événements attendent).
- **Console** : les mêmes commandes que le terminal, avec historique (↑ ↓) et complétion (**Tab** :
  commandes, sous-commandes, noms d'équipements). Chaque commande forme un bloc horodaté ; adresses,
  durées, réussites et échecs sont colorés ; les commandes suggérées par NETIV (« trace 1 »,
  « challenge check »…) se cliquent pour remplir la ligne de saisie. Bouton corbeille ou `clear` : effacer.
- **Panneau de détails** : Appareil, **Parcours** (rejoue un échange étape par étape, avec le paquet
  animé sur le schéma : ◀ ▶ lecture, ← → espace), Trace filtrable, Analyse (santé, sécurité, what-if,
  documentation), Défi (minuteur, indices, réponse).

### Adapter l'affichage

| Action | Souris | Clavier |
|---|---|---|
| Agrandir / réduire une zone | glisser la poignée (petite barre sur son bord) | Tab jusqu'à la poignée, puis ← → ou ↑ ↓ (Maj : pas plus grand) |
| Agrandir d'un clic, puis rétablir | boutons ⤢ dans la barre de la console et à côté des onglets du panneau | — |
| Réduire la console au minimum | bouton ▁ de la console | — |
| Revenir à la taille par défaut | double-clic sur le bord | Entrée sur le bord |
| Masquer le catalogue / les détails / la console | boutons en haut à droite, ou ✕ sur la console et le panneau | `[` `]` `` ` `` |
| Schéma seul | bouton ⤢ du schéma, ou ⛶ en haut | `F` |
| Zoom du schéma | molette, boutons + − ⤢ | `+` `−` `0` |
| Taille du texte, densité, hauteur de console | menu **Affichage** | — |
| Mode Construire (éditeur) | bouton **✎ Construire** | `B` |
| Aide et liste des commandes | bouton **?** | `?` |

La disposition, le zoom et les positions des équipements sont mémorisés **dans ton navigateur**
(par réseau). « Réinitialiser la disposition » dans le menu Affichage remet tout à zéro.

Dans un **lab intégré** (disposition « lab », par exemple sur ITEtude), le catalogue est replié,
l'onglet **Défi** passe en premier, et « Construire » n'apparaît pas pendant l'épreuve. Ces
réglages d'affichage sont mémorisés à part de ceux du bac à sable.

### Sécurité de l'interface

Le serveur n'écoute que sur `127.0.0.1` ; une autre adresse est refusée sans `--allow-remote`
(l'API n'a pas d'authentification). Les sessions web ne peuvent pas lire ni écrire de fichiers
sur le disque. Détails dans [api.md](api.md).

## 8. Construire un réseau dans le navigateur

Dans le catalogue : **＋ Nouveau réseau** (ou, sur un lab ouvert, le bouton **✎ Construire**,
touche `B`). Le mode Construire est indisponible pendant un défi ou un cyber range.

1. **Ajouter** : clic sur un équipement de la palette (il se place à un endroit libre), ou
   glisser-déposer sur le schéma. Les noms (`PC1`, `R1`, `SW1`…) et les interfaces sont prêts.
2. **Câbler** : outil **Câble** (touche `C`), puis un clic sur chaque extrémité. Une interface
   libre est choisie de chaque côté (un routeur en reçoit une nouvelle si besoin) ; on peut la
   changer en cliquant sur le câble.
3. **Adresser automatiquement** : IP de chaque segment (/24, ou /30 entre deux routeurs),
   passerelles et routes statiques manquantes. Rien de ce que tu as déjà écrit n'est remplacé.
4. **Configurer** : un clic sur un équipement ouvre ses réglages dans le panneau de droite (nom,
   rôle, interfaces et case DHCP, VLAN des ports de switch, passerelle, serveur DNS, routes,
   services, relais DHCP, serveurs DHCP et DNS, filtrage). Chaque
   champ s'applique quand tu le quittes ou appuies sur Entrée.
5. **Tester** : la barre d'outils (ping, curl) et la console restent disponibles.

| Action | Comment |
|---|---|
| Supprimer l'équipement ou le câble sélectionné | `Suppr`, ou le bouton 🗑 |
| Annuler / rétablir | `Ctrl+Z` / `Ctrl+Y` (100 étapes), ou ↶ ↷ |
| Revenir à l'outil Sélection, annuler un câble en cours | `V`, `Échap` |
| Télécharger | ⤓ .netiv (lab partageable) ou ⤓ .yaml (topologie seule) |
| Rouvrir un fichier | **Importer un fichier** dans le catalogue |

Chaque modification reconstruit le réseau : **la trace repart de zéro**. Une modification
invalide (adresse mal écrite, nom en double…) n'est pas appliquée. Elle reste en **brouillon**
avec la liste des erreurs, et les équipements concernés sont marqués en rouge ; un clic sur une
erreur sélectionne l'équipement. Les avertissements (IP en double, passerelle hors réseau…) sont
signalés sans bloquer, puisqu'un lab de dépannage peut être cassé exprès. Les positions des
équipements sont enregistrées avec le lab.

## 9. Créer et partager ses réseaux

- Écrire un réseau à la main : [format-topologie.md](format-topologie.md), puis `netiv validate mon-reseau.yaml`.
- En générer un : `netiv generate --type campus --routers 5 --hosts 20 --seed 1 --out campus.yaml`.
- Le partager : `lab export mon-lab.netiv "Mon titre"` dans la session, puis déposer le fichier
  dans le dossier `labs/` de l'autre personne (`lab list`, `lab open <nom>`).
- Documenter un réseau : `netiv docs mon-reseau.yaml --format md > reseau.md`
  (PDF : `pandoc reseau.md -o reseau.pdf`).

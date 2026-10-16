# Référence des commandes

Ces commandes s'utilisent dans `netiv run` (terminal), dans un script (`--script fichier.txt`)
et dans la console de l'interface web. `help` affiche le résumé ; cette page donne le détail.

Conventions : `<equip>` = nom d'un équipement (`PC1`), `<iface>` = nom d'une interface
(`eth0`), `<cidr>` = réseau ou adresse avec masque (`192.168.1.0/24`). Plusieurs commandes sur
une ligne : séparer par `;`. Dans un script, les lignes vides et celles qui commencent par `#`
sont ignorées. Les commandes qui touchent au disque (`load`, `capture save`, `lab export`…) sont
refusées dans l'interface web.

> Ce fichier est vérifié par un test : chaque commande de `help` doit y figurer.

## Topologie

| Commande | Effet |
|---|---|
| `load <fichier>` | Charge un réseau (`.yaml`, `.json` ou `.netiv`). |
| `devices` | Liste les équipements, leur type et leurs adresses. |
| `show ifaces <equip>` | Interfaces : MAC, IP, état, câble. |
| `show arp <equip>` | Cache ARP. |
| `show route <equip>` | Table de routage (routes connectées, statiques, passerelle). |
| `show mac <switch>` | Table d'adresses MAC du switch, par VLAN. |
| `show vlan <switch>` | Mode des ports : accès (VLAN) ou trunk (VLAN autorisés, natif). |
| `show acl <equip>` | Règles de filtrage, politique par défaut, suivi de connexion. |
| `show services <equip>` | Services TCP/UDP à l'écoute. |

## Simulation

| Commande | Effet | Exemple |
|---|---|---|
| `ping <equip> <ip> [n] [--step]` | Envoie `n` pings (1 par défaut) et affiche le résultat. Avec `--step`, programme seulement. | `ping PC1 192.168.2.10 3` |
| `step [n]` | Exécute `n` événements (1 par défaut) en les décrivant. | `step 5` |
| `run` | Exécute tous les événements en attente. | |
| `curl <equip> <ip> [port]` | Connexion TCP (poignée de main, requête, réponse, fermeture). Port 80 par défaut. | `curl PC1 192.168.2.10 80` |
| `udp <equip> <ip> <port> [texte]` | Envoie un datagramme UDP et attend une réponse. | `udp PC1 10.0.0.5 53 test` |
| `time` | Date simulée et nombre d'événements en attente. | |

## Observer

| Commande | Effet |
|---|---|
| `log [n\|all]` | Derniers événements de la trace (20 par défaut). |
| `trace <id>` | **Packet Journey** : toute une transaction (requête, réponse, ARP, erreurs ICMP), dans l'ordre. L'identifiant est affiché après chaque ping (`[parcours #3]`). |
| `inspect <n°>` | **Packet Microscope** : une trame couche par couche (Ethernet, IP, ICMP/TCP/UDP). Le numéro est celui de l'événement dans `log` ou `trace`. |
| `explain <n°>` | **Explain Engine** : sur un événement de routage, toutes les routes candidates, celle retenue et pourquoi. |

## Casser et diagnostiquer

| Commande | Effet |
|---|---|
| `chaos start <equip> <ip> [tcp\|udp <port>] [--layer l1\|l2\|l3\|services\|security] [--difficulty easy\|medium\|hard] [--seed N] [--visible]` | Injecte une panne qui casse ce flux. Cachée par défaut. |
| `chaos undo` | Répare toutes les pannes injectées. |
| `chaos reveal` | Montre les pannes en cours. |
| `chaos list` | Catalogue des 17 pannes (couche, difficulté). |
| `diagnose <equip> <ip> [tcp\|udp <port>]` | Contrôles `[✓]/[✗]` (interface, câble, VLAN, IP, ARP, passerelle, route aller et retour, filtrage, service) et **cause racine**. |
| `hint` | Indice suivant, du plus vague (couche) à la solution (dernier niveau). |
| `baseline save\|clear` | Mémorise l'état sain : `diagnose` indique alors la valeur attendue (« attendu : 192.168.1.1 »). |

## Configurer (ou casser à la main)

| Commande | Effet |
|---|---|
| `ip <equip> <iface> <cidr\|none>` | Change ou retire l'adresse IP. |
| `gateway <equip> <ip\|none>` | Change ou retire la passerelle par défaut. |
| `route add <equip> <cidr> via <ip>` | Ajoute une route statique. |
| `route del <equip> <cidr>` | Supprime les routes vers ce réseau. |
| `iface <equip> <iface> up\|down` | Active ou désactive une interface. |
| `cable <equip> <iface> cut\|restore` | Coupe ou rétablit le câble branché sur cette interface. |
| `loss <equip> <iface> <0..1>` | Taux de perte du câble (`0.3` = 30 %). Reproductible grâce à la graine. |
| `latency <equip> <iface> <ms>` | Latence du câble. |
| `forwarding <equip> on\|off` | Transfert de paquets d'un routeur. |
| `arp <equip> static <ip> <mac>` / `arp <equip> del <ip>` | Entrée ARP statique. |
| `port <switch> <iface> access <vlan>` | Port d'accès dans un VLAN. |
| `port <switch> <iface> trunk [vlans] [native N]` | Port trunk (`trunk 10,20 native 1` ; vide = tous les VLAN). |
| `service add <equip> tcp\|udp <port> <nom>` | Démarre un service (`http`, `ssh`, `echo`…). |
| `service del <equip> tcp\|udp <port>` | Arrête un service. |
| `acl add <equip> allow\|deny [tcp\|udp\|icmp\|any] [src <cidr>] [dst <cidr>] [port <n\|a-b>] [in <iface>]` | Ajoute une règle de filtrage en fin de liste. |
| `acl insert <equip> <n°> <options>` | Insère la règle en position `n°`. |
| `acl del <equip> <n°>` | Supprime la règle `n°`. |
| `acl default <equip> allow\|deny` | Politique par défaut. |
| `acl stateful <equip> on\|off` | Suivi de connexion : les réponses aux flux autorisés repassent. |

Chaque modification est tracée (`log`) : on voit dans la trace *quand* la configuration a changé.

## Apprendre

| Commande | Effet |
|---|---|
| `learn` | Parcours d'apprentissage ; `learn <parcours>`, `learn read <n°>`, `learn next`. |
| `challenge list [filtre]` | Défis (filtre : thème ou `facile`, `moyen`, `difficile`). |
| `challenge start <id\|fichier>` | Lance un défi : une panne cachée et un objectif. |
| `challenge show` | Énoncé, temps restant, indices utilisés. |
| `challenge check` | Vérifie l'objectif (+40 s'il fonctionne). |
| `challenge finish` / `challenge giveup` | Termine (score) ou abandonne (solution affichée). |
| `challenge causes` | Causes acceptées par `answer`. |
| `challenge export <fichier.netiv>` | Enregistre le défi en cours comme lab partageable. |
| `answer <equip> <cause>` | Ton diagnostic (+40 s'il est juste, −10 sinon). En cyber range : `answer <equip> <comportement>`. |
| `dashboard` | Niveau, XP, progression par thème. |
| `badges` | Badges obtenus et à obtenir. |
| `course new\|show\|validate\|export\|start\|grade` | Mode enseignant (voir [format-defis.md](format-defis.md)). |
| `tutor <question>` | Explique une notion, ou guide sur le défi en cours (un indice coûte −10). |

## Analyser et générer

| Commande | Effet | Exemple |
|---|---|---|
| `what-if <scénario>` | Impact d'un changement, mesuré sur une copie du réseau. | `what-if R1 goes down`, `what-if block tcp/443`, `what-if disconnect SW2`, `what-if remove route 10.10.0.0/16`, `what-if cut link R1:eth1` |
| `health` | Score de santé en 6 catégories (formules : annexe B de la spec). | |
| `docs md\|json\|yaml [fichier]` | Documentation générée du réseau. | `docs md reseau.md` |
| `topology generate --type <type> --routers N --hosts M [--seed S] [--save f.yaml]` | Génère et charge un réseau (`lan`, `enterprise`, `campus`, `datacenter`, `isp`, `hybrid`, `cyber-range`). | `topology generate --type campus --routers 5 --hosts 20` |
| `topology autoconfig` | Complète l'adressage d'un réseau dessiné : un réseau par segment (/30 entre deux routeurs, /24 sinon), IP des interfaces vides, passerelles, routes statiques par plus court chemin. N'écrase jamais une valeur existante. Refusé pendant un défi. | `topology autoconfig` |
| `challenge generate --topic T --difficulty facile\|moyen\|difficile [--seed N] [--save f.netiv]` | Génère un défi auto-validé et le lance. | `challenge generate --topic routing --difficulty difficile` |

## Sécurité

| Commande | Effet |
|---|---|
| `security analyze` | Défauts de conception (SEC-001 à SEC-011), gravité HIGH/MEDIUM/LOW, recommandations. |
| `capture save <fichier.nlv>` | Enregistre la trace dans une capture (JSON). |
| `forensic analyze <fichier.nlv\|live> [--packets]` | Chronologie et détections : `port-scan`, `host-sweep`, `lateral-movement`, `beaconing`, `repeated-denials`, `arp-spoofing`. |
| `range list` / `range start <id>` / `range show` / `range check` / `range finish` / `range giveup` | Cyber range : enquêter, corriger, désigner le coupable. |

## Partager

| Commande | Effet |
|---|---|
| `lab list` | Labs de la bibliothèque (dossier `labs/`). |
| `lab open <nom>` | Charge un lab de la bibliothèque. |
| `lab export <fichier.netiv> [titre]` | Enregistre le réseau actuel (avec ses modifications) comme lab partageable. |
| `leaderboard` | Classement local des profils. |

## Divers

| Commande | Effet |
|---|---|
| `help` | Résumé des commandes. |
| `quit` | Quitte la session (aussi : Ctrl-D). |

## Ligne de commande `netiv`

| Sous-commande | Effet |
|---|---|
| `netiv run [topologie] [--script f] [--profile nom] [--profile-dir d]` | Session interactive, ou exécution d'un script. |
| `netiv validate <topologie>` | Erreurs et avertissements, sans lancer. |
| `netiv generate --type T [--routers N] [--hosts M] [--seed S] [--out f]` | Génère une topologie YAML. |
| `netiv docs <topologie> [--format md\|json\|yaml]` | Documentation d'une topologie. |
| `netiv serve [--bind 127.0.0.1:8787] [--labs-dir labs] [--profile nom] [--allow-remote]` | Interface web et API HTTP. |
| `netiv bench [--routers N] [--hosts M] [--pings P]` | Mesures de performance (voir [performance.md](performance.md)). |

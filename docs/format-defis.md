# Écrire un défi, un cyber range, un cours

Tous ces contenus sont des fichiers YAML. Ceux livrés avec NETIV se trouvent dans
[`crates/netiv-education/content/`](../crates/netiv-education/content) ; un test les rejoue tous
avec leurs commandes de solution et vérifie qu'ils rapportent 100 points.

## Défi (challenge)

Un défi = un réseau + des **pannes cachées** + un **objectif** à rétablir + la **solution**.

```yaml
id: beginner-01
title: PC1 ne sort plus du bureau
topic: addressing          # physical | addressing | switching | routing | services | security
difficulty: easy           # easy | medium | hard
description: |
  PC1 n'arrive plus à joindre le serveur (192.168.2.10), alors que PC2 y arrivait
  ce matin. Trouve ce qui ne va pas sur PC1 et corrige-le.
time_limit_minutes: 15
topology_ref: office       # réseau livré (office, dmz, campus, triangle, cyber)…
# topology: { ... }        # … ou réseau complet en ligne (voir format-topologie.md)
objective: { from: PC1, to: 192.168.2.10 }   # ping ; pour un service :
# objective: { from: PC1, to: 10.9.0.10, service: { proto: tcp, port: 80 } }
faults:
  - set_gateway: { device: PC1, gateway: 192.168.1.254 }
solution:
  cause: wrong-gateway     # une des causes de « challenge causes »
  device: PC1
  explanation: |
    PC1 envoyait tout le trafic « hors réseau » à 192.168.1.254, une adresse que
    personne ne possède. La passerelle du bureau est le routeur R1 : 192.168.1.1.
  commands: ["gateway PC1 192.168.1.1"]
hints: []                  # optionnel ; sinon les indices sont calculés depuis la panne
```

Lancer un défi écrit à la main : `challenge start mon-defi.yaml`. Le partager :
`challenge export mon-defi.netiv` pendant qu'il est en cours.

Au démarrage, NETIV **valide** le défi : le réseau sain doit atteindre l'objectif, et le réseau
cassé ne doit plus l'atteindre. Un défi qui ne casse rien, ou qui part d'un réseau déjà en panne,
est refusé avec la raison.

### Pannes disponibles (`faults`)

Les équipements et interfaces sont désignés par leur nom (`R1`, `R1:eth1`).

| Panne | Champs | Exemple |
|---|---|---|
| `link_down` | `a` (une extrémité du câble) | `link_down: { a: "R1:eth1" }` |
| `interface_down` | `iface` | `interface_down: { iface: "R2:eth0" }` |
| `link_loss` | `a`, `loss` (0 à 1) | `link_loss: { a: "R1:eth1", loss: 0.6 }` |
| `link_latency` | `a`, `latency_ms` | `link_latency: { a: "R1:eth1", latency_ms: 900 }` |
| `set_ip` | `iface`, `ip` (CIDR, absent = aucune) | `set_ip: { iface: "PC1:eth0", ip: 192.168.1.10/16 }` |
| `set_gateway` | `device`, `gateway` (absent = aucune) | `set_gateway: { device: PC1, gateway: 192.168.1.254 }` |
| `remove_routes` | `device`, `dest` | `remove_routes: { device: R1, dest: 192.168.2.0/24 }` |
| `add_route` | `device`, `dest`, `via` | `add_route: { device: R2, dest: 192.168.1.0/24, via: 10.0.0.9 }` |
| `forwarding_off` | `device` | `forwarding_off: { device: R1 }` |
| `static_arp` | `device`, `ip`, `mac` | `static_arp: { device: PC1, ip: 192.168.1.1, mac: "02:00:00:00:99:99" }` |
| `port_access` | `iface` (port de switch), `vlan` | `port_access: { iface: "SW1:p1", vlan: 20 }` |
| `port_trunk` | `iface`, `allowed` (liste), `native` | `port_trunk: { iface: "SW1:tr", allowed: [10], native: 1 }` |
| `stop_service` | `device`, `proto`, `port` | `stop_service: { device: SERVER, proto: tcp, port: 80 }` |
| `acl_insert` | `device`, `rule` (format de règle ACL) | `acl_insert: { device: R1, rule: { action: deny, proto: icmp } }` |
| `acl_default` | `device`, `action` | `acl_default: { device: R1, action: deny }` |
| `dhcp_server_down` | `device` | `dhcp_server_down: { device: SRV }` |
| `dhcp_pools` | `device`, `pools` (format des plages de topologie ; vide = plus de serveur ; sur une machine sans serveur : serveur pirate) | `dhcp_pools: { device: IMP, pools: [{ network: 10.10.1.0/24, range: 10.10.1.200-10.10.1.220, gateway: 10.10.1.254 }] }` |
| `dhcp_relay` | `iface` (`Routeur:interface`), `server` (absent = plus de relais) | `dhcp_relay: { iface: "R1:eth0" }` |
| `set_dns` | `device`, `server` (absent = aucun) | `set_dns: { device: PC1, server: 10.10.2.99 }` |
| `dns_record` | `device`, `name`, `ip` (absente = nom supprimé) | `dns_record: { device: SRV, name: web.lan, ip: 10.10.2.77 }` |
| `dns_server_down` | `device` | `dns_server_down: { device: SRV }` |

Une panne DHCP n'a d'effet que lorsque les clients redemandent une adresse : au démarrage d'un
défi, les postes en DHCP redémarrent donc après l'injection des pannes (comme au matin d'une
journée de travail). Après avoir réparé, l'élève doit faire `dhcp <poste> renew`.

L'objectif d'un défi est une adresse IP : une panne DNS seule ne le casse pas. Les pannes DNS
servent dans les cours, les cyber ranges ou combinées à d'autres pannes ; leur diagnostic se fait
avec `diagnose <poste> <nom>`.

### Causes acceptées par `answer`

`link-down`, `interface-down`, `packet-loss`, `high-latency`, `wrong-vlan`, `arp-mismatch`,
`wrong-ip`, `wrong-mask`, `wrong-gateway`, `missing-route`, `wrong-route`, `routing-loop`,
`forwarding-disabled`, `service-down`, `filtered`, `dhcp-server-down`, `dhcp-relay-missing`,
`dhcp-pool-exhausted`, `rogue-dhcp`, `dns-wrong-server`, `dns-server-down`, `dns-record-wrong`.

### Générer un défi

```text
netiv> challenge generate --topic routing --difficulty difficile --seed 4 --save defi.netiv
```

Le générateur tire une topologie et une panne adaptées au thème et à la difficulté, puis
**auto-valide** le résultat (sain → OK, cassé → KO, corrigé par la solution → OK). Si une graine
ne donne pas de défi valide, il essaie la suivante.

## Scénario de cyber range

Un scénario part d'un réseau, y injecte une faille, puis **joue du trafic scripté** (l'attaque
et le trafic normal) avant que l'élève n'arrive : l'incident est dans la trace.

```yaml
id: range-01
title: Le balayage venu de la DMZ
difficulty: medium
description: |
  ALERTE — Cette nuit, le pare-feu a remonté un volume anormal de connexions…
time_limit_minutes: 30
topology_ref: cyber
faults:                              # la faille (mêmes pannes que les défis)
  - acl_insert:
      device: FW1
      rule: { action: allow, src: 10.9.0.0/24, dst: 192.168.0.0/16, in: eth1 }
traffic:                             # trafic joué dans le simulateur
  - { at_s: 5, from: WEB01, to: 192.168.20.20, proto: tcp,
      ports: [21, 22, 23, 25, 80, 135, 139, 443, 445, 3389], every_ms: 500 }
culprit:                             # ce que l'élève doit désigner avec « answer »
  device: WEB01
  behavior: port-scan
must_block:                          # flux qui doivent échouer après correction
  - { from: WEB01, to: 192.168.20.20, service: { proto: tcp, port: 22 } }
must_work:                           # flux qui doivent continuer à fonctionner
  - { from: PC01, to: 10.9.0.10, service: { proto: tcp, port: 80 } }
solution:
  explanation: |
    WEB01 a balayé 10 ports de FILE01 en 5 secondes…
  commands: ["acl del FW1 1"]
```

Champs d'un pas de trafic : `at_s` (départ, en secondes simulées), `from`, `to`, `proto`
(`tcp`/`udp`), `ports` (liste, dans l'ordre), `repeat` (nombre de passes, 1 par défaut),
`every_ms` (délai entre deux envois, 1 000 par défaut).

Comportements reconnus (`culprit.behavior`) : `port-scan`, `host-sweep`, `lateral-movement`,
`beaconing`, `repeated-denials`, `arp-spoofing`, `rogue-dhcp`.

`range check` réussit quand **tous** les `must_block` échouent **et** tous les `must_work`
fonctionnent : bloquer l'attaque en coupant le trafic légitime ne compte pas.

## Parcours d'apprentissage (`learn`)

```yaml
id: fundamentals
title: Fondamentaux des réseaux
description: Du câble au pare-feu, en dix leçons qui s'appuient sur des challenges.
lessons:
  - id: arp
    title: ARP, ou comment trouver une adresse MAC
    text: |
      Pour parler à 192.168.1.1, une machine connaît l'IP mais pas la MAC…
    try: ["ping PC1 192.168.2.10", "show arp PC1", "trace 1"]
    challenges: [beginner-01]   # validée quand ces défis sont réussis ; vide = validée à la lecture
```

## Cours (mode enseignant)

`course new <id> [fichier]` écrit un modèle à compléter :

```yaml
id: reseaux-1
title: TP reseaux-1 — Routage
objective: Configurer la communication entre trois réseaux.
level: intermediate
items:
- kind: lab                 # lab | challenge | exam
  title: Découvrir le réseau
  instructions: Charge le réseau « office », envoie un ping puis suis le paquet avec « trace ».
  points: 0
  minutes: 10
  allow_hints: true
- kind: challenge
  title: Il manque une route
  challenge: intermediate-02
  points: 10
  minutes: 20
  allow_hints: true
- kind: exam                # les indices sont toujours interdits en examen
  title: 'Contrôle : la réponse ne revient pas'
  challenge: advanced-01
  points: 10
  minutes: 25
  allow_hints: false
```

| Commande | Effet |
|---|---|
| `course validate <f>` | Vérifie le cours : titres en double, défi inconnu, durée trop courte, examen sans points… |
| `course show <f>` | Affiche le cours. |
| `course export <f> [md]` | Fiche à distribuer (Markdown). |
| `course start <f> <n°>` | Lance l'élément n° (son défi). |
| `course grade <f>` | Note de l'élève, d'après les scores enregistrés dans son profil. |

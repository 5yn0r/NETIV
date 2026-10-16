# Format des topologies

Un réseau NETIV se décrit en **YAML** (le JSON, sous-ensemble du YAML, est aussi accepté).
Vérifier un fichier sans le lancer : `netiv validate mon-reseau.yaml`.

Les clés inconnues sont refusées : une faute de frappe (`gatway:`) est signalée au lieu d'être
ignorée en silence.

## Exemple minimal

```yaml
network:
  name: mvp
  seed: 42            # graine : même graine → même simulation (pertes aléatoires comprises)

devices:
  - name: PC1
    type: host
    interfaces:
      - { name: eth0, ip: 192.168.1.10/24 }
    gateway: 192.168.1.1

  - name: SW1
    type: switch
    ports: 4          # crée port1..port4

  - name: R1
    type: router
    interfaces:
      - { name: eth0, ip: 192.168.1.1/24 }
      - { name: eth1, ip: 10.0.0.1/30 }
    routes:
      - { dest: 192.168.2.0/24, via: 10.0.0.2 }

links:
  - { a: "PC1:eth0", b: "SW1:port1" }
  - { a: "SW1:port2", b: "R1:eth0" }
```

Exemples complets dans le dépôt : [`examples/mvp.yaml`](../examples/mvp.yaml) (5 équipements),
[`examples/dmz.yaml`](../examples/dmz.yaml) (pare-feu et DMZ),
[`examples/cyber.yaml`](../examples/cyber.yaml) (Internet, pare-feu, DMZ, LAN).

## `network`

| Clé | Obligatoire | Description |
|---|---|---|
| `name` | oui | Nom du réseau (affiché partout). |
| `seed` | non (0) | Graine du générateur aléatoire de la simulation. |

## `devices`

| Clé | Description |
|---|---|
| `name` | Nom unique (`PC1`, `R1`…). |
| `type` | `host`, `server`, `router`, `firewall` ou `switch`. |
| `role` | Rôle pour l'analyse de sécurité : `user`, `server`, `dmz`, `internet`, `network`, `guest`, `iot`. Par défaut : `user` (host), `server` (server), `network` (les autres). |
| `interfaces` | Liste d'interfaces (voir ci-dessous). |
| `ports` | Switch seulement : crée `port1` … `portN`. |
| `gateway` | Passerelle par défaut (IPv4). |
| `routes` | Routes statiques : `{ dest: <cidr>, via: <ip> }`. |
| `services` | Services à l'écoute : `{ proto: tcp\|udp, port: 80, name: http }`. |
| `arp` | Entrées ARP statiques : `{ ip: <ip>, mac: 02:00:00:00:00:01 }`. |
| `acl` | Filtrage (voir ci-dessous). |

Comportement des types :

- **host / server** : ne transfèrent pas les paquets. Un serveur répond aux services déclarés
  (`curl` reçoit une réponse HTTP simulée ; un service UDP renvoie le datagramme).
- **router** : transfère les paquets selon la plus longue correspondance (routes connectées,
  statiques, puis passerelle). Les routes connectées sont **déduites** des interfaces actives.
- **firewall** : un routeur dont le filtrage par défaut est `deny` avec suivi de connexion.
- **switch** : apprend les adresses MAC (par VLAN), diffuse vers les ports câblés et actifs. Un
  switch n'a ni adresse IP, ni passerelle, ni route.

## `interfaces`

| Clé | Description |
|---|---|
| `name` | Nom (`eth0`, `port1`…). |
| `ip` | Adresse CIDR (`192.168.1.10/24`). Absente : interface sans adresse. |
| `management` | `true` : interface d'administration (l'analyse de sécurité vérifie qu'elle n'est pas joignable depuis les postes). |
| `vlan` | Switch : port d'**accès** dans ce VLAN (1 par défaut). |
| `trunk` | Switch : port **trunk** transportant ces VLAN (`[10, 20]` ; `[]` = tous). |
| `native_vlan` | Switch : VLAN natif du trunk (1 par défaut) ; ses trames circulent sans étiquette. |

VLAN — exemple tiré de `campus` : deux VLAN, un trunk, et un routeur avec une interface par VLAN
(pas de sous-interfaces) :

```yaml
  - name: SW1
    type: switch
    interfaces:
      - { name: p1, vlan: 10 }
      - { name: p2, vlan: 20 }
      - { name: tr, trunk: [10, 20], native_vlan: 1 }
```

## `acl` (routeurs et pare-feu)

```yaml
    acl:
      default: deny        # allow (routeur) | deny (pare-feu) par défaut
      stateful: true       # les réponses aux flux autorisés repassent
      rules:               # évaluées dans l'ordre, la première qui correspond gagne
        - { action: allow, proto: tcp, dst: 10.9.0.0/24, port: "80", in: eth0, comment: "LAN vers web DMZ" }
        - { action: allow, proto: icmp, in: eth0 }
        - { action: deny, src: 10.9.0.0/24, dst: 192.168.0.0/16 }
```

| Clé de règle | Description |
|---|---|
| `action` | `allow` ou `deny`. |
| `proto` | `any` (défaut), `icmp`, `tcp`, `udp`. |
| `src`, `dst` | Réseaux CIDR ; absent = tout. |
| `port` | Port de destination : `"80"` ou `"1000-2000"` (entre guillemets). |
| `in` | Interface d'entrée ; absent = toutes. |
| `comment` | Texte libre, affiché par `show acl`. |

Quand une règle bloque un paquet, la trace dit **laquelle** (numéro et texte).

## `links`

| Clé | Description |
|---|---|
| `a`, `b` | Extrémités au format `Équipement:interface`. |
| `latency_us` | Latence en microsecondes (défaut : 1 000 µs = 1 ms). |
| `loss` | Probabilité de perte entre 0 et 1 (défaut : 0). |

## Erreurs et avertissements

- Les **erreurs** empêchent le chargement : nom d'équipement en double, interface inexistante,
  câble branché deux fois, adresse invalide, switch avec une passerelle… Elles sont toutes
  listées en une fois.
- Les **avertissements** n'empêchent rien, car un lab de dépannage est volontairement cassé :
  IP utilisée plusieurs fois, passerelle hors des réseaux de l'équipement, route au prochain saut
  injoignable, interface adressée sans câble, câble entre deux réseaux différents, incohérence de
  VLAN entre deux switches (VLAN natif, accès contre trunk).

## Fichiers `.netiv` (labs partageables)

Un `.netiv` empaquette une topologie avec des métadonnées, et en option un défi ou un scénario.
`lab export` en produit un ; `load` et `lab open` les relisent.

```yaml
format_version: 1
metadata:
  title: "Bureau : deux routeurs et un serveur web"
  description: "Optionnel."
  author: NETIV
topology:
  network: { name: bureau, seed: 1 }
  devices: [ ... ]        # même format que ci-dessus
  links: [ ... ]
challenge: { ... }        # optionnel : voir format-defis.md
```

Les labs prêts à l'emploi sont dans [`labs/`](../labs).

## Générer plutôt qu'écrire

```bash
netiv generate --type datacenter --routers 8 --hosts 60 --seed 7 --out dc.yaml
```

Types : `lan`, `enterprise`, `campus`, `datacenter`, `isp`, `hybrid`, `cyber-range`. Le
générateur attribue les adresses (un /24 par réseau local, un /30 par lien entre routeurs) et
calcule les routes statiques par plus court chemin. Un test vérifie que tout réseau généré est
entièrement joignable.

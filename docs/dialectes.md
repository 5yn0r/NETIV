# Consoles : Cisco IOS et terminal Windows/Linux

NETIV a sa propre syntaxe, courte et régulière, pensée pour apprendre les notions vite. Pour
s'entraîner aux commandes du monde professionnel, chaque équipement a aussi une **console** dans
sa syntaxe réelle. C'est un **plus** : la syntaxe NETIV ne change pas, et les défis se résolvent
indifféremment avec l'une ou l'autre.

```text
console <équipement>   ouvre la console (Cisco sur routeur/switch/pare-feu, terminal sur poste/serveur)
exit                   remonte d'un niveau, puis revient à NETIV
netiv <commande>       une commande NETIV depuis la console
```

Chaque commande d'une console est **traduite** vers la commande NETIV équivalente, puis exécutée
avec les mêmes contrôles (validation, défis, trace). Elle ne peut donc rien faire que NETIV ne
sache déjà faire. Pendant une console, `hint`, `challenge …`, `answer`, `trace`, `inspect`,
`explain`, `log`, `step`, `run`, `devices` et `tutor` restent disponibles tels quels.

## Noms d'interfaces

| Équipement | Nom NETIV | Alias Cisco |
|---|---|---|
| Routeur, pare-feu | `eth0`, `eth1`… | `Gi0/0`, `Gi0/1`… (`GigabitEthernet0/0`) |
| Switch | `p1`, `p2`… ou `port1`… | `Gi0/1`, `Gi0/2`… (le premier port est `0/1`, comme sur un vrai switch) |

`Fa0/n`, `FastEthernet0/n`, `e0/n` sont aussi acceptés. `show ip interface brief` affiche
`GigabitEthernet0/2 (p2)`.

## Cisco IOS (routeurs, switches, pare-feu)

Invites : `R1>` (utilisateur), `R1#` (privilégié), `R1(config)#`, `R1(config-if)#`,
`SW1(config-vlan)#`. Les mots-clés s'abrègent tant qu'il n'y a pas d'ambiguïté (`en`, `conf t`,
`int gi0/1`, `sh ip int br`, `no shut`) ; sinon : `% Ambiguous command`. Une erreur donne
`% Invalid input detected`.

| Cisco IOS | Équivalent NETIV |
|---|---|
| `enable` / `disable` / `configure terminal` / `end` / `exit` | (changement de mode) |
| `show ip interface brief` | `show ifaces <equip>` |
| `show running-config` | (configuration générée : interfaces, VLAN, routes, relais) |
| `show ip route` | `show route <equip>` |
| `show vlan brief`, `show interfaces status` | `show vlan <switch>` |
| `show mac address-table` | `show mac <switch>` |
| `show arp` / `show ip arp` | `show arp <equip>` |
| `show access-lists` | `show acl <equip>` |
| `ping <ip\|nom>`, `traceroute <ip\|nom>` | `ping <equip> <cible> 5` (puis `trace <id>`) |
| `interface <if>` → `ip address <ip> <masque>` | `ip <equip> <if> <ip>/<préfixe>` |
| `ip address dhcp` / `no ip address` | `ip <equip> <if> dhcp` / `none` |
| `shutdown` / `no shutdown` | `iface <equip> <if> down` / `up` |
| `ip helper-address <ip>` | `dhcp relay <equip> <if> <ip>` |
| `switchport access vlan <n>` | `port <switch> <if> access <n>` |
| `switchport mode trunk`, `switchport trunk allowed vlan <liste>`, `switchport trunk native vlan <n>` | `port <switch> <if> trunk <liste> native <n>` |
| `ip route <réseau> <masque> <passerelle>` | `route add <equip> <cidr> via <passerelle>` (`0.0.0.0 0.0.0.0` : `gateway`) |
| `no ip route <réseau> <masque>` | `route del <equip> <cidr>` |
| `ip routing` / `no ip routing` | `forwarding <equip> on` / `off` |
| `vlan <n>` → `name <nom>` | (accepté : un VLAN existe par les ports qui le portent) |
| `do <commande>` | une commande du mode privilégié depuis la configuration |
| `copy running-config startup-config`, `write` | `[OK]` (rien à sauvegarder dans NETIV) |

Dans NETIV, un switch n'a pas de pile IP : pas d'`ip address` ni d'interface VLAN (SVI) sur un
switch. `hostname` n'est pas modifiable (les noms servent dans les défis).

## Terminal de poste (Windows et Linux)

Invite : `PC1:~$`. Les deux familles de commandes sont acceptées dans le même terminal.

| Windows | Linux | Équivalent NETIV |
|---|---|---|
| `ipconfig`, `ipconfig /all` | `ip a`, `ip addr` | `show ifaces <poste>` (+ DNS, passerelle) |
| `ipconfig /renew`, `ipconfig /release` | `dhclient [if]`, `dhclient -r [if]` | `dhcp <poste> renew\|release` |
| `netsh interface ip set address "<if>" dhcp` | `dhclient <if>` (sur une carte fixe) | `ip <poste> <if> dhcp` |
| `netsh interface ip set address "<if>" static <ip> <masque> [gw]` | `ip addr add <ip>/<préfixe> dev <if>` | `ip <poste> <if> <cidr>` (+ `gateway`) |
| | `ip addr flush dev <if>` | `ip <poste> <if> none` |
| | `ip link set <if> up\|down` | `iface <poste> <if> up\|down` |
| `route print` | `ip route` | `show route <poste>` |
| `route add 0.0.0.0 mask 0.0.0.0 <gw>` | `ip route add default via <gw>` | `gateway <poste> <gw>` |
| `route add <réseau> mask <masque> <gw>` | `ip route add <cidr> via <gw>` | `route add <poste> <cidr> via <gw>` |
| `route delete <réseau>` | `ip route del <cidr>\|default` | `route del` / `gateway none` |
| `netsh interface ip set dns "<if>" static <ip>` | `resolvectl dns <if> <ip>` | `dns <poste> <ip>` |
| `netsh interface ip set dns "<if>" dhcp` | `resolvectl revert <if>` | `dns <poste> none` |
| | `cat /etc/resolv.conf` | (serveur DNS utilisé) |
| `arp -a` | `ip neigh` | `show arp <poste>` |
| `ping <cible> [-n N]`, `tracert` | `ping <cible> [-c N]`, `traceroute` | `ping <poste> <cible> N` |
| `nslookup <nom>` | `nslookup <nom>`, `dig <nom>` | `nslookup <poste> <nom>` |
| `curl <url>` | `curl <url>`, `wget <url>` | `curl <poste> <hôte> <port>` |

Comme sous Windows, `ipconfig /renew` échoue si aucune carte n'est en DHCP : il faut d'abord
repasser la carte en DHCP (`netsh … dhcp` ou `dhclient`).

## Limites

- Seules les commandes listées ici sont simulées ; une commande Cisco connue mais non simulée
  répond `% Commande reconnue mais non simulée dans NETIV`.
- Les sorties des commandes de simulation (`ping`, `nslookup`, `curl`, DHCP) restent celles de
  NETIV, en français, avec le numéro de transaction pour `trace`.
- Les défis livrés documentent leur solution en syntaxe NETIV ; les deux syntaxes donnent le même
  score.

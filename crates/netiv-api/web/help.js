/* NETIV — contenu de la documentation (bouton « ? »).
 *
 * Données seules : app.js les met en page. Dans la syntaxe d'une commande,
 * <x> est un paramètre à remplacer, [x] est facultatif, a|b veut dire « a ou b ».
 * Dans les exemples, {host}, {server}, {router}, {fw}, {switch} sont remplacés
 * par des équipements du lab ouvert ; {hostIp}, {hostGw}, {serverIp} par leurs
 * adresses ; {hostIface}, {routerIface}, {switchPort} par une de leurs
 * interfaces. Les `keys` d'un équipement sont les débuts de syntaxe de ses
 * commandes clés.
 */
window.NETIV_HELP = (() => {
  // Types d'équipements (identiques au champ `kind` des instantanés).
  const IP = ['host', 'server', 'router', 'firewall'];
  const ALL = ['host', 'server', 'router', 'firewall', 'switch'];
  const ROUTING = ['router', 'firewall'];

  const devices = [
    {
      kind: 'host',
      name: 'Poste',
      role: 'L’ordinateur d’un utilisateur. C’est souvent le point de départ du test : « depuis PC1, j’arrive à joindre… ? ».',
      config: 'Adresse IP (fixe ou DHCP), passerelle, serveur DNS.',
      console: 'Terminal Windows / Linux',
      keys: ['ping', 'curl', 'nslookup', 'ip ', 'gateway', 'dns <', 'dhcp <'],
    },
    {
      kind: 'server',
      name: 'Serveur',
      role: 'Une machine qui rend un service : site web (port 80/443), DNS, DHCP, partage de fichiers…',
      config: 'Adresse IP, passerelle, services à l’écoute, pare-feu local, zone DNS, plages DHCP.',
      console: 'Terminal Windows / Linux',
      keys: ['show acl', 'service add', 'acl add', 'dns record', 'dns server', 'dhcp server', 'dhcp pool add'],
    },
    {
      kind: 'router',
      name: 'Routeur',
      role: 'Relie plusieurs réseaux entre eux et choisit le chemin des paquets grâce à sa table de routage.',
      config: 'Une adresse IP par interface, routes, passerelle par défaut, relais DHCP, filtrage.',
      console: 'Console Cisco IOS',
      keys: ['show ifaces', 'route add', 'route del', 'forwarding', 'iface', 'cable', 'dhcp relay', 'explain'],
    },
    {
      kind: 'firewall',
      name: 'Pare-feu',
      role: 'Un routeur qui filtre : il laisse passer ou bloque le trafic selon des règles (ACL). Par défaut, il bloque tout.',
      config: 'Comme un routeur, plus les règles de filtrage et le suivi de connexion.',
      console: 'Console Cisco IOS',
      keys: ['show acl', 'acl insert', 'acl del', 'acl add', 'acl default', 'acl stateful', 'route add'],
    },
    {
      kind: 'switch',
      name: 'Switch',
      role: 'Relie les machines d’un même réseau. Il n’a pas d’adresse IP : il sépare les réseaux avec des VLAN.',
      config: 'Mode de chaque port : access (un VLAN) ou trunk (plusieurs VLAN, entre switches).',
      console: 'Console Cisco IOS',
      keys: ['show vlan', 'port <switch> <port> access', 'port <switch> <port> trunk', 'cable', 'iface'],
    },
  ];

  const method = [
    ['Lire la mission', 'Repérez la machine de départ, la destination et le service attendu (ping, page web, nom DNS).'],
    ['Constater le problème', 'Depuis la machine de départ : « ping » pour le réseau, « curl » pour un site, « nslookup » pour un nom.'],
    ['Trouver où ça bloque', 'Onglet Parcours : le paquet s’arrête sur un équipement. C’est là qu’il faut regarder.'],
    ['Inspecter cet équipement', 'Cliquez dessus dans le schéma : la fiche montre interfaces, routes, VLAN, filtrage, services.'],
    ['Corriger', 'Avec une commande des sections Adresses, Câbles, VLAN, Filtrage, DHCP et DNS, ou dans la console Cisco / Windows / Linux.'],
    ['Vérifier', '« challenge check » relance le test de la mission.'],
    ['Donner le diagnostic', '« answer <équipement> <cause> » : l’équipement en faute et la nature de la panne (liste dans « Causes »).'],
  ];

  // Paramètres qui reviennent souvent : définis une fois.
  const P = {
    dev: ['équipement', 'Nom d’un équipement du schéma (PC1, R1, SW1…).'],
    from: ['équipement', 'La machine d’où part le test.'],
    target: ['cible', 'Adresse IP (192.168.1.10) ou nom DNS (intranet.pme).'],
    iface: ['interface', 'Nom d’une interface de l’équipement (eth0, eth1, p1…), visible dans sa fiche.'],
    cidr: ['réseau', 'Adresse de réseau avec son préfixe, ex. 192.168.2.0/24.'],
    ip: ['ip', 'Une adresse IPv4, ex. 192.168.1.1.'],
    port: ['port', 'Numéro de port : 80 web, 443 web sécurisé, 22 SSH, 53 DNS, 445 partage de fichiers.'],
    proto: ['tcp|udp', 'Protocole de transport.'],
  };

  const groups = [
    {
      id: 'tester',
      title: 'Tester la connexion',
      intro: 'Pour constater une panne et vérifier une correction.',
      commands: [
        {
          syntax: 'ping <équipement> <cible> [nombre]',
          summary: 'Envoie un ping : la cible répond-elle ? C’est le premier test à faire.',
          params: [P.from, P.target, ['nombre', 'Nombre de pings (1 par défaut).']],
          devices: IP,
          examples: ['ping {host} {serverIp}', 'ping {host} {serverIp} 4'],
        },
        {
          syntax: 'curl <équipement> <cible> [port]',
          summary: 'Ouvre une connexion vers un service (une page web, par défaut port 80). Montre si le service répond, refuse ou est filtré.',
          params: [P.from, P.target, P.port],
          devices: IP,
          examples: ['curl {host} {serverIp}', 'curl {host} {serverIp} 443'],
        },
        {
          syntax: 'nslookup <équipement> <nom>',
          summary: 'Demande au serveur DNS l’adresse d’un nom. Compare la réponse à l’adresse attendue.',
          params: [P.from, ['nom', 'Nom de domaine, ex. intranet.pme.']],
          devices: IP,
          examples: ['nslookup {host} intranet.pme'],
        },
        {
          syntax: 'udp <équipement> <cible> <port> [texte]',
          summary: 'Envoie un datagramme UDP (DNS, supervision…).',
          params: [P.from, P.target, P.port, ['texte', 'Contenu du message (facultatif).']],
          devices: IP,
          examples: ['udp {host} {serverIp} 53'],
        },
      ],
    },
    {
      id: 'observer',
      title: 'Observer un équipement',
      intro: 'Le même contenu que la fiche affichée en cliquant sur l’équipement.',
      commands: [
        {
          syntax: 'devices',
          summary: 'Liste tous les équipements et leurs adresses.',
          params: [],
          devices: ALL,
          examples: ['devices'],
        },
        {
          syntax: 'show ifaces|route|arp <équipement>',
          summary: 'Interfaces (adresse, état, câble), table de routage, ou cache ARP (correspondance IP → MAC).',
          params: [['ifaces|route|arp', 'Ce que vous voulez voir.'], P.dev],
          devices: IP,
          examples: ['show ifaces {router}', 'show route {router}', 'show arp {host}'],
        },
        {
          syntax: 'show vlan|mac <switch>',
          summary: 'Mode des ports (VLAN access ou trunk), ou table des adresses MAC apprises.',
          params: [['vlan|mac', 'Ce que vous voulez voir.'], ['switch', 'Nom du switch.']],
          devices: ['switch'],
          examples: ['show vlan {switch}', 'show mac {switch}'],
        },
        {
          syntax: 'show acl|services|dhcp|dns <équipement>',
          summary: 'Règles de filtrage, services à l’écoute, état DHCP ou DNS.',
          params: [['acl|services|dhcp|dns', 'Ce que vous voulez voir.'], P.dev],
          devices: IP,
          examples: ['show acl {fw}', 'show services {server}', 'show dns {host}'],
        },
      ],
    },
    {
      id: 'suivre',
      title: 'Suivre les paquets',
      intro: 'Chaque échange reçoit un numéro (#1, #2…) : il sert à rejouer son parcours.',
      commands: [
        {
          syntax: 'trace <n°>',
          summary: 'Le parcours complet d’un échange, équipement par équipement (comme l’onglet Parcours).',
          params: [['n°', 'Numéro de l’échange, affiché après un ping ou un curl.']],
          devices: ALL,
          examples: ['trace 1'],
        },
        {
          syntax: 'log [nombre|all]',
          summary: 'Les derniers événements du réseau (envois, réceptions, refus…).',
          params: [['nombre|all', 'Combien d’événements (20 par défaut), ou tous.']],
          devices: ALL,
          examples: ['log', 'log all'],
        },
        {
          syntax: 'inspect <n°>',
          summary: 'Le détail d’une trame : adresses MAC et IP, protocole, TTL.',
          params: [['n°', 'Numéro d’événement, affiché par « log » ou « trace ».']],
          devices: ALL,
          examples: ['inspect 3'],
        },
        {
          syntax: 'explain <n°>',
          summary: 'Pourquoi un routeur a choisi cette route (ou n’en a trouvé aucune).',
          params: [['n°', 'Numéro d’une décision de route, affiché par « trace ».']],
          devices: IP,
          examples: ['explain 4'],
        },
        {
          syntax: 'step [nombre] | run',
          summary: 'Avance la simulation d’un ou plusieurs événements, ou jusqu’au bout (avec « ping … --step »).',
          params: [['nombre', 'Événements à exécuter (1 par défaut).']],
          devices: ALL,
          examples: ['ping {host} {serverIp} --step', 'step', 'run'],
        },
      ],
    },
    {
      id: 'adresses',
      title: 'Adresses et routage',
      intro: 'Pour qu’un paquet arrive, il faut une bonne adresse, une passerelle pour sortir du réseau, et des routes pour revenir.',
      commands: [
        {
          syntax: 'ip <équipement> <interface> <adresse/préfixe>|dhcp|none',
          summary: 'Donne une adresse IP à une interface, la demande au DHCP, ou la retire.',
          params: [P.dev, P.iface, ['adresse/préfixe', 'Adresse et masque en notation courte, ex. 192.168.1.10/24 (/24 = 255.255.255.0).']],
          devices: IP,
          examples: ['ip {host} {hostIface} {hostIp}', 'ip {host} {hostIface} dhcp'],
        },
        {
          syntax: 'gateway <équipement> <ip>|none',
          summary: 'Passerelle par défaut : le routeur à qui confier tout ce qui sort du réseau local.',
          params: [P.dev, ['ip', 'Adresse du routeur, dans le même réseau que l’équipement.']],
          devices: IP,
          examples: ['gateway {host} {hostGw}'],
        },
        {
          syntax: 'route add <équipement> <réseau> via <ip>',
          summary: 'Ajoute une route : « pour joindre ce réseau, passe par ce routeur voisin ».',
          params: [P.dev, P.cidr, ['ip', 'Adresse du routeur voisin (prochain saut), joignable directement.']],
          devices: ROUTING,
          examples: ['route add {router} 192.168.2.0/24 via 10.0.0.2'],
        },
        {
          syntax: 'route del <équipement> <réseau>',
          summary: 'Supprime une route statique (une route fausse ou qui crée une boucle).',
          params: [P.dev, P.cidr],
          devices: ROUTING,
          examples: ['route del {router} 192.168.2.0/24'],
        },
        {
          syntax: 'forwarding <équipement> on|off',
          summary: 'Active le transfert des paquets d’un réseau à l’autre. Un routeur doit l’avoir à « on ».',
          params: [P.dev, ['on|off', 'Activé ou désactivé.']],
          devices: ROUTING,
          examples: ['forwarding {router} on'],
        },
        {
          syntax: 'arp <équipement> static <ip> <mac> | arp <équipement> del <ip>',
          summary: 'Fixe ou supprime une correspondance IP → MAC (une entrée fausse empêche de joindre la machine).',
          params: [P.dev, P.ip, ['mac', 'Adresse matérielle, ex. 02:00:00:00:00:01.']],
          devices: IP,
          examples: ['arp {host} del 192.168.1.1'],
        },
      ],
    },
    {
      id: 'cables',
      title: 'Câbles et interfaces',
      commands: [
        {
          syntax: 'cable <équipement> <interface> cut|restore',
          summary: 'Coupe ou rebranche le câble relié à cette interface.',
          params: [P.dev, P.iface, ['cut|restore', 'Couper ou rebrancher.']],
          devices: ALL,
          examples: ['cable {router} {routerIface} restore'],
        },
        {
          syntax: 'iface <équipement> <interface> up|down',
          summary: 'Allume ou éteint une interface (comme « no shutdown » / « shutdown » chez Cisco).',
          params: [P.dev, P.iface, ['up|down', 'Allumer ou éteindre.']],
          devices: ALL,
          examples: ['iface {router} {routerIface} up'],
        },
        {
          syntax: 'loss <équipement> <interface> <taux>',
          summary: 'Règle le taux de perte d’un câble (0 = aucune, 1 = tout est perdu).',
          params: [P.dev, P.iface, ['taux', 'Nombre entre 0 et 1, ex. 0.3 pour 30 %.']],
          devices: ALL,
          examples: ['loss {router} {routerIface} 0'],
        },
        {
          syntax: 'latency <équipement> <interface> <ms>',
          summary: 'Règle le délai d’un câble, en millisecondes.',
          params: [P.dev, P.iface, ['ms', 'Délai en millisecondes, ex. 1.']],
          devices: ALL,
          examples: ['latency {router} {routerIface} 1'],
        },
      ],
    },
    {
      id: 'vlan',
      title: 'Switch et VLAN',
      intro: 'Un port access appartient à un seul VLAN (un poste). Un trunk transporte plusieurs VLAN (entre switches ou vers un routeur). Les deux bouts d’un trunk doivent autoriser les mêmes VLAN.',
      commands: [
        {
          syntax: 'port <switch> <port> access <vlan>',
          summary: 'Met un port dans un VLAN.',
          params: [['switch', 'Nom du switch.'], ['port', 'Nom du port, ex. p1.'], ['vlan', 'Numéro de VLAN (1 à 4094).']],
          devices: ['switch'],
          examples: ['port {switch} {switchPort} access 10'],
        },
        {
          syntax: 'port <switch> <port> trunk [vlans] [native <n>]',
          summary: 'Fait du port un trunk. Sans liste, tous les VLAN passent.',
          params: [['switch', 'Nom du switch.'], ['port', 'Nom du port.'], ['vlans', 'VLAN autorisés, séparés par des virgules : 10,20,30.'], ['native <n>', 'VLAN natif, non étiqueté (1 par défaut).']],
          devices: ['switch'],
          examples: ['port {switch} {switchPort} trunk 10,20 native 1'],
        },
      ],
    },
    {
      id: 'services',
      title: 'Services et filtrage',
      intro: 'Une règle de filtrage (ACL) dit « allow » ou « deny ». Les règles sont lues dans l’ordre : la première qui correspond décide. Si aucune ne correspond, la politique par défaut s’applique.',
      commands: [
        {
          syntax: 'service add <équipement> tcp|udp <port> <nom>',
          summary: 'Démarre un service à l’écoute (serveur web, SSH…).',
          params: [P.dev, P.proto, P.port, ['nom', 'Nom affiché, ex. http.']],
          devices: IP,
          examples: ['service add {server} tcp 80 http'],
        },
        {
          syntax: 'service del <équipement> tcp|udp <port>',
          summary: 'Arrête un service.',
          params: [P.dev, P.proto, P.port],
          devices: IP,
          examples: ['service del {server} tcp 23'],
        },
        {
          syntax: 'acl add <équipement> allow|deny [protocole] [src <réseau>] [dst <réseau>] [port <n>] [in <interface>]',
          summary: 'Ajoute une règle en fin de liste. Les critères absents valent « tout ».',
          params: [
            P.dev,
            ['allow|deny', 'Laisser passer ou bloquer.'],
            ['protocole', 'tcp, udp, icmp (ping) ou any.'],
            ['src <réseau>', 'Source, ex. 192.168.10.0/24 ou une machine : 192.168.10.12/32.'],
            ['dst <réseau>', 'Destination.'],
            ['port <n>', 'Port de destination, ou plage 1000-2000.'],
            ['in <interface>', 'Seulement le trafic qui entre par cette interface.'],
          ],
          devices: IP,
          examples: ['acl add {server} allow tcp port 443', 'acl add {fw} deny src 192.168.10.12/32'],
        },
        {
          syntax: 'acl insert <équipement> <position> <règle>',
          summary: 'Insère une règle à une position (1 = en tête, lue en premier). Même format de règle que « acl add ».',
          params: [P.dev, ['position', 'Numéro de la règle dans la liste.'], ['règle', 'allow|deny et ses critères.']],
          devices: IP,
          examples: ['acl insert {fw} 1 deny tcp port 4444'],
        },
        {
          syntax: 'acl del <équipement> <position>',
          summary: 'Supprime la règle à cette position (voir « show acl »).',
          params: [P.dev, ['position', 'Numéro de la règle.']],
          devices: IP,
          examples: ['acl del {fw} 1'],
        },
        {
          syntax: 'acl default <équipement> allow|deny',
          summary: 'Politique par défaut, appliquée quand aucune règle ne correspond.',
          params: [P.dev, ['allow|deny', 'Tout laisser passer ou tout bloquer.']],
          devices: IP,
          examples: ['acl default {fw} deny'],
        },
        {
          syntax: 'acl stateful <équipement> on|off',
          summary: 'Suivi de connexion : les réponses à un échange autorisé passent automatiquement.',
          params: [P.dev, ['on|off', 'Activé ou désactivé.']],
          devices: IP,
          examples: ['acl stateful {fw} on'],
        },
      ],
    },
    {
      id: 'dhcp-dns',
      title: 'DHCP et DNS',
      intro: 'Le DHCP distribue automatiquement adresse, passerelle et DNS aux postes. Le DNS traduit un nom (intranet.pme) en adresse IP.',
      commands: [
        {
          syntax: 'dhcp <équipement> [renew|release] [interface]',
          summary: 'Le poste redemande (renew) ou rend (release) son adresse. À faire après avoir réparé le DHCP.',
          params: [P.dev, ['renew|release', 'Redemander (par défaut) ou rendre.'], P.iface],
          devices: IP,
          examples: ['dhcp {host}', 'dhcp {host} release'],
        },
        {
          syntax: 'dhcp server <équipement> on|off',
          summary: 'Démarre ou arrête le serveur DHCP.',
          params: [P.dev, ['on|off', 'Démarrer ou arrêter.']],
          devices: ['server', 'router', 'firewall'],
          examples: ['dhcp server {server} on'],
        },
        {
          syntax: 'dhcp pool add <équipement> <réseau> <première-dernière> [gw <ip>] [dns <ip>] [lease <s>]',
          summary: 'Ajoute une plage d’adresses à distribuer.',
          params: [P.dev, P.cidr, ['première-dernière', 'Plage, ex. 192.168.1.100-192.168.1.200.'], ['gw <ip>', 'Passerelle donnée aux postes.'], ['dns <ip>', 'Serveur DNS donné aux postes.'], ['lease <s>', 'Durée du bail en secondes.']],
          devices: ['server', 'router', 'firewall'],
          examples: ['dhcp pool add {server} 192.168.1.0/24 192.168.1.100-192.168.1.200 gw 192.168.1.1'],
        },
        {
          syntax: 'dhcp pool del <équipement> <réseau>',
          summary: 'Supprime une plage.',
          params: [P.dev, P.cidr],
          devices: ['server', 'router', 'firewall'],
          examples: ['dhcp pool del {server} 192.168.1.0/24'],
        },
        {
          syntax: 'dhcp relay <routeur> <interface> <ip>|none',
          summary: 'Relais DHCP : le routeur transmet les demandes d’un réseau sans serveur vers le serveur DHCP d’un autre réseau.',
          params: [['routeur', 'Nom du routeur.'], ['interface', 'Interface côté postes.'], ['ip', 'Adresse du serveur DHCP.']],
          devices: ROUTING,
          examples: ['dhcp relay {router} {routerIface} {serverIp}'],
        },
        {
          syntax: 'dns <équipement> <ip>|none',
          summary: 'Le serveur DNS utilisé par cette machine.',
          params: [P.dev, ['ip', 'Adresse du serveur DNS.']],
          devices: IP,
          examples: ['dns {host} {serverIp}'],
        },
        {
          syntax: 'dns server <équipement> on|off',
          summary: 'Démarre ou arrête le serveur DNS.',
          params: [P.dev, ['on|off', 'Démarrer ou arrêter.']],
          devices: ['server'],
          examples: ['dns server {server} on'],
        },
        {
          syntax: 'dns record add <équipement> <nom> <ip> | dns record del <équipement> <nom>',
          summary: 'Ajoute (ou corrige) et supprime un nom dans la zone du serveur DNS.',
          params: [P.dev, ['nom', 'Nom de domaine, ex. intranet.pme.'], ['ip', 'Adresse vers laquelle le nom doit pointer.']],
          devices: ['server'],
          examples: ['dns record add {server} intranet.pme {serverIp}'],
        },
      ],
    },
    {
      id: 'defis',
      title: 'Défis et cyber ranges',
      intro: 'Pendant une épreuve, « diagnose », « health », « chaos » et « baseline » sont bloqués : ils donneraient la réponse.',
      commands: [
        {
          syntax: 'challenge check',
          summary: 'Vérifie si la mission est remplie (le test de l’énoncé passe).',
          params: [],
          devices: ALL,
          examples: ['challenge check'],
        },
        {
          syntax: 'answer <équipement> <cause>',
          summary: 'Votre diagnostic : l’équipement en faute et la nature de la panne. Dans un cyber range : la machine coupable et son comportement.',
          params: [['équipement', 'Là où se trouve la panne (ou la machine attaquante).'], ['cause', 'Un identifiant de la section « Causes », ex. wrong-gateway.']],
          devices: ALL,
          examples: ['answer {host} wrong-gateway'],
        },
        {
          syntax: 'hint',
          summary: 'Un indice. Chaque indice retire 10 points.',
          params: [],
          devices: ALL,
          examples: ['hint'],
        },
        {
          syntax: 'challenge show | challenge causes | challenge giveup',
          summary: 'Revoir l’énoncé, lister les causes possibles, ou abandonner.',
          params: [],
          devices: ALL,
          examples: ['challenge show', 'challenge causes'],
        },
        {
          syntax: 'forensic analyze live',
          summary: 'Cyber range : analyse le trafic enregistré et signale les comportements suspects (balayage, balise…).',
          params: [],
          devices: ALL,
          examples: ['forensic analyze live'],
        },
        {
          syntax: 'range check',
          summary: 'Cyber range : vérifie que l’attaque est bloquée et que le trafic normal passe toujours.',
          params: [],
          devices: ALL,
          examples: ['range check'],
        },
        {
          syntax: 'security analyze',
          summary: 'Repère les défauts de conception (filtrage trop ouvert, services exposés…).',
          params: [],
          devices: ALL,
          examples: ['security analyze'],
        },
      ],
    },
    {
      id: 'consoles',
      title: 'Ouvrir une console',
      intro: 'Pour taper les commandes d’un vrai équipement : Cisco IOS sur un routeur, un pare-feu ou un switch ; Windows ou Linux sur un poste ou un serveur.',
      commands: [
        {
          syntax: 'console <équipement>',
          summary: 'Ouvre la console de l’équipement (aussi : bouton « Console » de sa fiche). « exit » pour revenir aux commandes NETIV.',
          params: [P.dev],
          devices: ALL,
          examples: ['console {router}', 'console {host}'],
        },
      ],
    },
  ];

  const ios = {
    intro: 'La console Cisco a des modes : l’invite change selon où vous êtes. Les abréviations sont acceptées (conf t, int gi0/1, sh ip int br, no shut).',
    modes: [
      ['R1>', 'utilisateur', 'consultation limitée', 'enable'],
      ['R1#', 'privilégié', 'toutes les commandes show, ping', 'configure terminal'],
      ['R1(config)#', 'configuration', 'routes, VLAN, choisir une interface', 'interface Gi0/1'],
      ['R1(config-if)#', 'interface', 'adresse, allumer / éteindre, mode du port', 'exit, end'],
    ],
    commands: [
      ['show ip interface brief', 'R1#', 'Interfaces, adresses, état.'],
      ['show ip route', 'R1#', 'Table de routage.'],
      ['show running-config', 'R1#', 'Configuration complète.'],
      ['show vlan brief', 'SW1#', 'VLAN et ports (switch).'],
      ['show interfaces status', 'SW1#', 'État et mode des ports (switch).'],
      ['show mac address-table', 'SW1#', 'Adresses MAC apprises (switch).'],
      ['show arp | show access-lists', 'R1#', 'Cache ARP, règles de filtrage.'],
      ['ping <ip> | traceroute <ip>', 'R1#', 'Tester depuis l’équipement.'],
      ['ip route <réseau> <masque> <passerelle>', '(config)#', 'Ajouter une route (no ip route … pour la retirer).'],
      ['ip routing', '(config)#', 'Activer le routage.'],
      ['vlan <n>', '(config)#', 'Créer un VLAN.'],
      ['interface <Gi0/n>', '(config)#', 'Choisir une interface. Gi0/0 = eth0 sur un routeur ; Gi0/1 = premier port sur un switch.'],
      ['ip address <ip> <masque>', '(config-if)#', 'Adresse de l’interface (ip address dhcp, no ip address).'],
      ['no shutdown | shutdown', '(config-if)#', 'Allumer ou éteindre l’interface.'],
      ['ip helper-address <ip>', '(config-if)#', 'Relais DHCP vers ce serveur.'],
      ['switchport mode access|trunk', '(config-if)#', 'Mode du port (switch).'],
      ['switchport access vlan <n>', '(config-if)#', 'VLAN d’un port access.'],
      ['switchport trunk allowed vlan <liste>', '(config-if)#', 'VLAN autorisés sur le trunk, ex. 10,20.'],
      ['switchport trunk native vlan <n>', '(config-if)#', 'VLAN natif du trunk.'],
      ['do <commande>', '(config)#', 'Lancer une commande show sans quitter la configuration.'],
    ],
  };

  const host = {
    intro: 'Sur un poste ou un serveur, la console accepte les commandes Windows et Linux usuelles.',
    rows: [
      ['Voir son adresse', 'ipconfig  ·  ipconfig /all', 'ip a'],
      ['Tester une machine', 'ping <cible> [-n N]', 'ping -c N <cible>'],
      ['Suivre le chemin', 'tracert <cible>', 'traceroute <cible>'],
      ['Résoudre un nom', 'nslookup <nom>', 'nslookup <nom>  ·  dig <nom>'],
      ['Ouvrir une page web', 'curl <url>', 'curl <url>'],
      ['Table de routage', 'route print', 'ip route'],
      ['Ajouter une route', 'route add <réseau> mask <masque> <passerelle>', 'ip route add default via <passerelle>'],
      ['Cache ARP', 'arp -a', 'ip neigh'],
      ['Adresse fixe', 'netsh interface ip set address "<carte>" static <ip> <masque> [passerelle]', 'ip addr add <ip/préfixe> dev <if>'],
      ['Adresse DHCP', 'netsh interface ip set address "<carte>" dhcp  ·  ipconfig /renew', 'dhclient [if]'],
      ['Rendre l’adresse DHCP', 'ipconfig /release', 'dhclient -r [if]'],
      ['Serveur DNS', 'netsh interface ip set dns "<carte>" static <serveur>', 'resolvectl dns <if> <serveur>'],
      ['Allumer / éteindre', '—', 'ip link set <if> up|down'],
    ],
  };

  // Comment reconnaître chaque cause (complète le libellé fourni par le moteur).
  const causeHints = {
    'link-down': 'Le câble est marqué coupé ; plus rien ne passe au-delà.',
    'interface-down': 'L’interface est éteinte (état « down ») alors que le câble est branché.',
    'packet-loss': 'Certains pings passent, d’autres non.',
    'high-latency': 'Tout passe, mais avec un temps de réponse anormal.',
    'wrong-vlan': 'Deux machines du même réseau ne se voient pas : ports dans des VLAN différents, ou VLAN absent du trunk.',
    'arp-mismatch': 'La MAC enregistrée pour une IP n’est pas la bonne (entrée ARP statique fausse).',
    'wrong-ip': 'L’adresse n’est pas dans le bon réseau, ou est déjà utilisée.',
    'wrong-mask': 'Le préfixe (/16, /24…) ne correspond pas au réseau : la machine croit sa destination locale.',
    'wrong-gateway': 'La machine joint son réseau mais pas au-delà : passerelle absente ou fausse.',
    'missing-route': 'Un routeur ne sait pas où envoyer les paquets vers ce réseau (aucune route).',
    'wrong-route': 'Une route existe mais envoie les paquets au mauvais endroit.',
    'routing-loop': 'Les paquets tournent entre routeurs jusqu’à « TTL épuisé ».',
    'forwarding-disabled': 'Le routeur reçoit les paquets mais ne les transmet pas.',
    'service-down': 'La machine répond au ping mais refuse la connexion sur le port.',
    'filtered': 'Pas de réponse sur un port, ou refus d’un pare-feu : une règle ACL bloque.',
    'dhcp-server-down': 'Les postes n’obtiennent pas d’adresse (169.254.x.x ou aucune).',
    'dhcp-relay-missing': 'Seuls les postes d’un autre réseau que le serveur DHCP n’ont pas d’adresse.',
    'dhcp-pool-exhausted': 'Plus d’adresse libre dans la plage, ou plage absente pour ce réseau.',
    'rogue-dhcp': 'Des postes reçoivent une adresse ou une passerelle inattendue d’un autre serveur.',
    'dns-wrong-server': 'Le ping par adresse passe, par nom non : le poste interroge un mauvais serveur DNS.',
    'dns-server-down': 'Le serveur DNS ne répond plus aux demandes de noms.',
    'dns-record-wrong': 'Le nom se résout, mais vers une mauvaise adresse (ou plus du tout).',
  };

  const behaviors = {
    'port-scan': ['Balayage de ports', 'Une machine essaie de nombreux ports sur la même cible.'],
    'host-sweep': ['Balayage d’hôtes', 'Une machine contacte de nombreuses adresses à la suite.'],
    'lateral-movement': ['Rebond', 'Une machine compromise s’attaque à une autre zone du réseau.'],
    'beaconing': ['Balise', 'Contact régulier, à intervalle fixe, vers un serveur extérieur.'],
    'arp-spoofing': ['Usurpation ARP', 'Une machine se fait passer pour une autre (souvent la passerelle).'],
    'repeated-denials': ['Refus répétés', 'Beaucoup de tentatives bloquées par le filtrage, depuis la même source.'],
    'rogue-dhcp': ['DHCP pirate', 'Un serveur DHCP non autorisé distribue des adresses.'],
  };

  const glossary = [
    ['Adresse IP', '192.168.1.10 : l’adresse d’une machine sur le réseau.'],
    ['Préfixe (/24)', 'La taille du réseau : /24 = 255.255.255.0, les 3 premiers nombres désignent le réseau.'],
    ['Réseau', '192.168.1.0/24 : toutes les adresses de 192.168.1.0 à 192.168.1.255.'],
    ['Passerelle', 'Le routeur par lequel une machine sort de son réseau.'],
    ['Route', 'Indique au routeur par où joindre un réseau qui n’est pas le sien.'],
    ['MAC', 'Adresse matérielle d’une carte réseau, utilisée sur le réseau local.'],
    ['ARP', 'Trouve la MAC qui correspond à une adresse IP du réseau local.'],
    ['VLAN', 'Un réseau séparé à l’intérieur d’un switch (numéro 1 à 4094).'],
    ['Trunk', 'Lien qui transporte plusieurs VLAN entre deux switches.'],
    ['Port / service', 'Un programme à l’écoute sur une machine : web 80/443, SSH 22, DNS 53, partage 445.'],
    ['ACL', 'Liste de règles allow / deny d’un pare-feu ou d’un routeur.'],
    ['DHCP', 'Distribue automatiquement adresse, passerelle et DNS.'],
    ['DNS', 'Traduit un nom (intranet.pme) en adresse IP.'],
    ['TTL', 'Compteur décrémenté à chaque routeur : à 0, le paquet est détruit (signe de boucle).'],
  ];

  const shortcuts = [
    ['?', 'cette documentation'],
    ['[', 'afficher / masquer le catalogue'],
    [']', 'afficher / masquer le panneau de détails'],
    ['`', 'afficher / masquer la console'],
    ['F', 'schéma seul (plein cadre)'],
    ['+  −  0', 'zoomer, dézoomer, tout afficher'],
    ['←  →  espace', 'dans « Parcours » : étape précédente, suivante, lecture'],
    ['↑  ↓', 'dans la console : commandes précédentes'],
    ['Tab', 'dans la console : compléter une commande ou un nom'],
    ['B', 'mode Construire (dessiner, câbler, configurer)'],
    ['C  V', 'en mode Construire : outil Câble, outil Sélection'],
    ['Suppr', 'supprimer l’équipement ou le câble sélectionné'],
    ['Ctrl+Z  Ctrl+Y', 'annuler / rétablir une modification'],
  ];

  return { devices, method, groups, ios, host, causeHints, behaviors, glossary, shortcuts };
})();

# SPEC D'ARCHITECTURE — NETIV Engine

**Version :** 0.2 (MVP implémenté — voir l'annexe A pour les écarts)
**Périmètre :** moteur de simulation du MVP v0.1 (Phases 0 à 3 du cahier des charges)
**Document lié :** `Cahier_de_charge.md`

Cette spec décrit **comment le moteur fonctionne**. Elle tranche les décisions que le cahier des charges laisse ouvertes. Tout ce qui n'est pas dans le MVP est listé en section 14 (hors périmètre).

---

## 1. Principes directeurs

1. **Déterminisme.** Même topologie + mêmes commandes + même graine = exactement la même trace. C'est indispensable pour les tests, les challenges et le pas-à-pas.
2. **Tout est un événement explicable.** Chaque décision du moteur (flood, apprentissage MAC, choix de route, TTL, drop) émet un événement de trace structuré. Packet Journey, Packet Microscope, Explain Engine et Troubleshooting sont des *vues* sur cette trace, pas des modules séparés qui re-calculent.
3. **Le moteur ignore l'UI.** `netiv-core` ne dépend ni de `clap`, ni de `ratatui`, ni de `tokio`. Il expose une API Rust synchrone.
4. **Les équipements ne touchent pas au monde.** Un équipement reçoit un événement et retourne des *effets* (envoyer une trame, programmer un timer, émettre une trace). Seul le moteur applique les effets. Cela évite les problèmes de borrow checker et rend chaque équipement testable seul.
5. **Structuré d'abord, octets ensuite.** Les trames sont des `struct`/`enum` Rust, pas des `Vec<u8>`. La sérialisation en octets (pcap, Wireshark) viendra plus tard sans changer le modèle.

---

## 2. Décision : simulation à événements discrets

**Choix : moteur mono-thread, à événements discrets, avec horloge virtuelle.** Pas de Tokio dans le cœur.

Justification :

| Critère | Événements discrets | Tâches async (Tokio) |
|---|---|---|
| Déterminisme | Natif | Difficile (ordonnancement) |
| Pas-à-pas (Packet Journey) | Trivial (`step()`) | Complexe |
| Tests reproductibles | Oui | Fragile |
| Temps simulé vs temps réel | Découplés | Liés au temps réel |
| Performance (10⁴ équipements) | Très bonne (une file) | Surcoût par tâche |

Tokio reste pertinent plus tard pour une API réseau ou un mode « réseau réel », en couche externe au-dessus du moteur.

### 2.1 Temps simulé

```rust
/// Microsecondes depuis le début de la simulation.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SimTime(pub u64);
```

Le temps ne progresse que lorsque le moteur dépile un événement. Un `ping` de 2 ms simulées s'exécute en microsecondes réelles.

### 2.2 File d'événements

```rust
struct Scheduled {
    time: SimTime,
    seq: u64,          // ordre d'insertion : départage les événements simultanés
    event: Event,
}
// BinaryHeap<Reverse<Scheduled>> ordonné par (time, seq)
```

**Règle clé :** à temps égal, l'ordre est celui d'insertion (`seq`). Aucune autre source d'ordre (pas de `HashMap` itéré pour décider d'un ordre, utiliser `BTreeMap` ou des `Vec` indexés).

### 2.3 Événements du moteur

```rust
pub enum Event {
    /// Une trame arrive sur l'interface d'un équipement.
    FrameArrival { device: DeviceId, iface: IfaceId, frame: Frame },
    /// Un timer d'équipement expire (timeout ARP, timeout ping...).
    Timer { device: DeviceId, timer: TimerId },
    /// Une commande utilisateur (ping, set ip...) destinée à un équipement.
    Command { device: DeviceId, cmd: Command },
    /// Une panne injectée ou réparée (Chaos Engine).
    Fault { fault: Fault, action: FaultAction },
}
```

### 2.4 Boucle principale

```rust
impl Simulation {
    /// Exécute un seul événement. Retourne false si la file est vide.
    pub fn step(&mut self) -> bool { /* ... */ }
    pub fn run_until_idle(&mut self) { while self.step() {} }
    pub fn run_until(&mut self, t: SimTime) { /* ... */ }
}
```

Algorithme de `step()` :

1. Dépiler le prochain `Scheduled`, avancer `now`.
2. Dispatcher selon le type d'événement (voir 2.5).
3. Récupérer les `Effects` retournés par l'équipement.
4. Appliquer chaque effet (section 5).

### 2.5 Dispatch

- `FrameArrival` : appelle `device.on_frame(ctx, iface, frame, &mut effects)`.
- `Timer` : appelle `device.on_timer(...)`.
- `Command` : appelle `device.on_command(...)`.
- `Fault` : modifie directement l'état du monde (lien, interface, config), émet une trace.

---

## 3. Modèle de données

### 3.1 Identifiants

Des newtypes sur `u32`/`usize`, jamais des `String` dans le chemin chaud :

```rust
pub struct DeviceId(pub u32);
pub struct IfaceId(pub u32);   // local à un device
pub struct LinkId(pub u32);
pub struct TimerId(pub u64);
pub struct TraceId(pub u64);   // identité logique d'un paquet (voir 6.2)
```

Les noms lisibles (`PC1`, `eth0`) vivent dans la topologie et sont résolus à l'entrée (CLI, YAML).

### 3.2 Adresses

```rust
pub struct MacAddr(pub [u8; 6]);
pub struct Ipv4Net { pub addr: Ipv4Addr, pub prefix: u8 }  // 0..=32, validé
```

Utiliser `std::net::Ipv4Addr`. Les MAC sont générées de façon déterministe (préfixe fixe + compteur) à la création des interfaces.

### 3.3 Trames et paquets

```rust
pub struct Frame {
    pub src: MacAddr,
    pub dst: MacAddr,
    pub vlan: Option<u16>,        // réservé, inutilisé au MVP
    pub payload: EtherPayload,
    pub meta: FrameMeta,          // métadonnées de simulation, absentes du "fil"
}

pub enum EtherPayload {
    Arp(ArpPacket),
    Ipv4(Ipv4Packet),
}

pub struct ArpPacket {
    pub op: ArpOp,                // Request | Reply
    pub sender_mac: MacAddr, pub sender_ip: Ipv4Addr,
    pub target_mac: MacAddr, pub target_ip: Ipv4Addr,
}

pub struct Ipv4Packet {
    pub src: Ipv4Addr, pub dst: Ipv4Addr,
    pub ttl: u8,
    pub payload: IpPayload,
}

pub enum IpPayload {
    Icmp(IcmpMessage),
    // Udp, Tcp : plus tard
}

pub enum IcmpMessage {
    EchoRequest { id: u16, seq: u16 },
    EchoReply   { id: u16, seq: u16 },
    DestUnreachable { code: UnreachCode, original: Box<Ipv4Packet> },
    TimeExceeded    { original: Box<Ipv4Packet> },
}

pub struct FrameMeta {
    pub trace_id: TraceId,        // identité du "voyage" (voir 6.2)
    pub hops: u16,
}
```

`EtherPayload`/`IpPayload` sont des `enum` : ajouter TCP/UDP plus tard force le compilateur à signaler tous les `match` à compléter.

### 3.4 Équipements

```rust
pub struct Device {
    pub id: DeviceId,
    pub name: String,
    pub ifaces: Vec<Interface>,
    pub behavior: Box<dyn Node>,    // Host | Switch | Router
}

pub struct Interface {
    pub id: IfaceId,
    pub name: String,               // "eth0"
    pub mac: MacAddr,
    pub ip: Option<Ipv4Net>,
    pub admin_up: bool,
    pub link: Option<LinkId>,
}
```

Le trait `Node` définit le comportement :

```rust
pub trait Node {
    fn on_frame(&mut self, ctx: &Ctx, iface: IfaceId, frame: Frame, out: &mut Effects);
    fn on_timer(&mut self, ctx: &Ctx, timer: TimerId, out: &mut Effects);
    fn on_command(&mut self, ctx: &Ctx, cmd: Command, out: &mut Effects);
}
```

`Ctx` est en **lecture seule** : heure courante, interfaces du device. Pas de référence au reste du monde.

Choix `dyn Node` plutôt qu'un `enum` : trois comportements au MVP, mais le trait facilitera Firewall, ACL, DHCP/DNS serveurs sans toucher au moteur. Si le coût du dispatch dynamique devenait un problème (il ne le sera pas avant 10⁴ équipements), on passerait à un `enum`.

### 3.5 Liens

```rust
pub struct Link {
    pub id: LinkId,
    pub a: (DeviceId, IfaceId),
    pub b: (DeviceId, IfaceId),
    pub latency: SimTime,           // défaut : 100 µs
    pub loss: f32,                  // 0.0..=1.0, tirage via RNG seedé
    pub up: bool,
}
```

---

## 4. Comportements des équipements

### 4.1 Host (PC / serveur)

État :

```rust
struct Host {
    arp_cache: BTreeMap<Ipv4Addr, ArpEntry>,
    routes: RouteTable,             // route connectée + default gateway
    pending: Vec<PendingPacket>,    // paquets en attente de résolution ARP
    pings: BTreeMap<u16, PingState>,
}
```

Algorithme d'envoi d'un paquet IP (`send_ip`) :

1. Lookup de route sur `dst` → `(iface, next_hop)`. Aucune route → trace `NoRoute`, abandon.
2. `next_hop` = `dst` si on-link, sinon la gateway.
3. Si `arp_cache[next_hop]` existe → construire la trame, effet `Send`.
4. Sinon → mettre le paquet dans `pending`, envoyer ARP Request broadcast, programmer un timer de timeout ARP.

Réception :

- ARP Request pour notre IP → apprendre l'émetteur, répondre.
- ARP Reply → mettre à jour le cache, vider `pending` correspondant.
- ICMP Echo Request pour nous → Echo Reply.
- ICMP Echo Reply → clôturer le ping correspondant (RTT = `now - envoi`).
- Autre → trace `Dropped(NotForUs)`.

### 4.2 Switch (L2)

État : table MAC `BTreeMap<MacAddr, IfaceId>` (sans vieillissement au MVP, ou avec timer simple).

À la réception d'une trame sur `iface_in` :

1. **Apprendre** : `table[frame.src] = iface_in` (trace `MacLearned`).
2. Si `dst` est broadcast ou inconnue → **flood** sur toutes les interfaces actives sauf `iface_in` (trace `Flood { reason }`).
3. Sinon → **forward** vers `table[dst]` (trace `Forward`). Si le port de sortie est `iface_in` → drop (filtrage).

### 4.3 Router (L3)

À la réception d'une trame IPv4 :

1. Trame non adressée à notre MAC (ni broadcast) → drop.
2. Paquet pour une de nos IP → traitement local (ICMP echo).
3. Sinon, **forward** :
   1. `ttl <= 1` → ICMP Time Exceeded vers la source, trace `TtlExpired`.
   2. `ttl -= 1` (trace `TtlDecremented { from, to }`).
   3. Lookup LPM → `RouteDecision` (section 4.4).
   4. Aucune route → ICMP Destination Unreachable, trace `NoRoute`.
   5. Résolution ARP du next-hop (mêmes mécanismes que l'hôte), puis envoi.

Le routeur répond aussi aux ARP Requests pour ses propres IP.

### 4.4 Table de routage et décision explicable

```rust
pub struct Route {
    pub dest: Ipv4Net,
    pub next_hop: Option<Ipv4Addr>,   // None = directement connecté
    pub iface: IfaceId,
    pub origin: RouteOrigin,          // Connected | Static
}

pub struct RouteDecision {
    pub dst: Ipv4Addr,
    pub candidates: Vec<RouteCandidate>,  // TOUTES les routes, matchées ou non
    pub chosen: Option<usize>,
}
pub struct RouteCandidate { pub route: Route, pub matches: bool }
```

Le lookup **retourne toujours** une `RouteDecision` complète, pas seulement la route gagnante. C'est ce qui rend l'Explain Engine (section 12 du cahier) gratuit : il suffit d'afficher la structure. Sélection : plus long préfixe, puis ordre d'insertion en cas d'égalité.

---

## 5. Effets

```rust
pub enum Effect {
    Send { iface: IfaceId, frame: Frame },
    SetTimer { after: SimTime, timer: TimerId },
    CancelTimer { timer: TimerId },
    Trace(TraceKind),
}

pub struct Effects(Vec<Effect>);
```

Application d'un `Send` par le moteur :

1. L'interface est-elle `admin_up` et reliée à un lien ? Sinon → trace `Dropped(InterfaceDown | NoLink)`.
2. Le lien est-il `up` ? Sinon → trace `Dropped(LinkDown)`.
3. Tirage de perte (`rng.gen::<f32>() < loss`) → trace `Dropped(LinkLoss)`.
4. Sinon programmer `FrameArrival` sur l'autre extrémité à `now + latency`, trace `FrameSent`.

Le RNG est `rand::rngs::StdRng` initialisé avec la graine de la simulation, propriété du `Simulation`.

---

## 6. Trace : la colonne vertébrale

### 6.1 Structure

```rust
pub struct TraceEvent {
    pub seq: u64,                       // numéro global, croissant
    pub time: SimTime,
    pub device: DeviceId,
    pub trace_id: Option<TraceId>,      // à quel voyage appartient l'événement
    pub caused_by: Option<u64>,         // seq de l'événement parent
    pub kind: TraceKind,
}

pub enum TraceKind {
    // Cycle de vie d'une trame
    FrameSent     { iface: IfaceId, summary: FrameSummary },
    FrameReceived { iface: IfaceId, summary: FrameSummary },
    Dropped       { reason: DropReason },

    // L2
    MacLearned    { mac: MacAddr, iface: IfaceId },
    Flood         { reason: FloodReason, out_ifaces: Vec<IfaceId> },
    Forward       { out_iface: IfaceId },

    // ARP
    ArpRequestSent { target: Ipv4Addr },
    ArpCacheUpdated { ip: Ipv4Addr, mac: MacAddr },
    ArpTimeout    { target: Ipv4Addr },

    // L3
    RouteLookup   (RouteDecision),
    TtlDecremented { from: u8, to: u8 },
    TtlExpired,

    // ICMP / applicatif
    PingSent      { dst: Ipv4Addr, seq: u16 },
    PingReply     { from: Ipv4Addr, seq: u16, rtt: SimTime },
    PingTimeout   { seq: u16 },

    // Chaos
    FaultApplied  { fault: Fault },
    FaultCleared  { fault: Fault },
}

pub enum DropReason {
    InterfaceDown, NoLink, LinkDown, LinkLoss,
    NoRoute, TtlExceeded, NotForUs, ArpUnresolved,
}
```

`summary` est une copie légère (types de couches + champs clés), pas la trame entière, pour que la trace reste compacte.

### 6.2 Identité d'un « voyage »

Une trame change à chaque saut (MAC réécrites, TTL), mais l'utilisateur pense « le ping PC1 → SERVER ». Chaque *origine* (création d'un Echo Request) reçoit un `TraceId`. Il est copié dans `FrameMeta` et **préservé par les routeurs et switches**.

- Echo Request : `TraceId = 7`.
- Echo Reply : nouveau `TraceId = 8`, avec `caused_by` pointant vers l'événement de réception de la requête.
- ARP : son propre `TraceId`, relié au ping qui l'a déclenché via `caused_by`.

Le **Packet Journey** = filtrer la trace sur un `trace_id` et l'afficher dans l'ordre. Le **Packet Microscope** = afficher le `FrameSummary` d'un `FrameSent` donné.

### 6.3 Stockage et accès

```rust
pub struct TraceLog { events: Vec<TraceEvent> }

impl TraceLog {
    pub fn by_trace(&self, id: TraceId) -> impl Iterator<Item = &TraceEvent>;
    pub fn by_device(&self, id: DeviceId) -> impl Iterator<Item = &TraceEvent>;
    pub fn since(&self, seq: u64) -> &[TraceEvent];   // flux incrémental pour la TUI
}
```

Au MVP : un `Vec` en mémoire. Plus tard : export `.nlv` (sérialisé avec `serde`) pour le Forensics Mode.

---

## 7. Injection de pannes (base du Chaos Engine)

Une panne est une **mutation d'état planifiée**, réversible, tracée.

```rust
pub enum Fault {
    LinkDown(LinkId),
    InterfaceDown(DeviceId, IfaceId),
    LinkLoss(LinkId, f32),
    LinkLatency(LinkId, SimTime),
    SetGateway(DeviceId, Ipv4Addr),
    SetMask(DeviceId, IfaceId, u8),
    RemoveRoute(DeviceId, Ipv4Net),
    // ...
}
pub enum FaultAction { Apply, Clear }
```

Chaque `Fault` stocke ce qu'il faut pour être annulé (ancienne valeur). Cela alimente directement :

- le Chaos Engine (scénarios = listes de `Fault` planifiés) ;
- les Challenges (panne cachée + solution attendue) ;
- le Troubleshooting Engine : un diagnostic peut comparer la config *actuelle* à la config *saine*.

Le Troubleshooting Engine (Phase 4) s'appuiera sur deux sources : la trace (où le paquet est mort et pourquoi) et un jeu de **règles de cohérence** sur la config (gateway hors du sous-réseau de l'interface, masque incohérent, etc.).

---

## 8. Format de topologie (YAML)

```yaml
network:
  name: demo-mvp
  seed: 42

devices:
  - name: PC1
    type: host
    interfaces:
      - { name: eth0, ip: 192.168.1.10/24 }
    gateway: 192.168.1.1

  - name: SW1
    type: switch
    ports: 4

  - name: R1
    type: router
    interfaces:
      - { name: eth0, ip: 192.168.1.1/24 }
      - { name: eth1, ip: 10.0.0.1/30 }
    routes:
      - { dest: 192.168.2.0/24, via: 10.0.0.2 }

links:
  - { a: PC1:eth0, b: SW1:port1 }
  - { a: SW1:port2, b: R1:eth0 }
  - { a: R1:eth1, b: R2:eth1, latency_us: 500 }
```

Pipeline : `YAML → TopologyDef (serde) → validation → Simulation`.

La validation est une étape à part et produit des erreurs lisibles : nom dupliqué, interface inexistante, lien vers un port déjà utilisé, IP en doublon dans un même sous-réseau, préfixe invalide.

---

## 9. API publique du moteur (`netiv-core`)

```rust
let topo = TopologyDef::from_yaml(&text)?;
let mut sim = Simulation::build(topo)?;           // valide + instancie

let pc1 = sim.device_id("PC1")?;
sim.command(pc1, Command::Ping { dst: "192.168.2.10".parse()?, count: 4 });

sim.run_until_idle();

for ev in sim.trace().by_trace(TraceId(1)) {
    println!("{ev}");
}
```

Cette API est la **seule** surface utilisée par la CLI, la TUI et les tests. Si une fonctionnalité nécessite d'ouvrir les entrailles du moteur, c'est un signal que l'API est incomplète.

---

## 10. CLI : état entre les commandes

Le cahier des charges montre des commandes isolées (`netiv ping`, `netiv packet inspect 42`). Un processus par commande n'a pas d'état partagé. **Décision :**

- **Mode REPL (principal)** : `netiv run topo.yaml` ouvre une session interactive. L'état de la simulation vit dans le processus.

  ```
  netiv> ping PC1 192.168.2.10
  netiv> trace 1
  netiv> show arp PC1
  netiv> step
  ```
- **Mode batch** : `netiv run topo.yaml --script lab.txt` exécute des commandes puis quitte (utile pour tests et CI).
- **Mode one-shot** (cas simples sans état) : `netiv topology validate topo.yaml`.

Les commandes du REPL sont les mêmes que celles du cahier des charges, sans le préfixe `netiv`.

---

## 11. Organisation des crates (MVP)

Trois crates suffisent pour la Phase 0-3. On découpera davantage quand un besoin réel apparaîtra.

```
netiv/
├── Cargo.toml                 # workspace
└── crates/
    ├── netiv-core/            # moteur complet : types, protocoles, devices,
    │                          #   scheduler, trace, faults. AUCUNE dépendance UI/async.
    ├── netiv-topology/        # parsing YAML + validation + génération → TopologyDef
    └── netiv-cli/             # clap + REPL + rendu texte de la trace
```

Organisation interne de `netiv-core` (modules, futurs candidats à devenir des crates) :

```
netiv-core/src/
├── lib.rs
├── time.rs          # SimTime
├── ids.rs           # DeviceId, IfaceId, ...
├── addr.rs          # MacAddr, Ipv4Net
├── packet.rs        # Frame, ArpPacket, Ipv4Packet, IcmpMessage
├── route.rs         # RouteTable, RouteDecision, LPM
├── device/
│   ├── mod.rs       # Device, Interface, trait Node, Effects
│   ├── host.rs
│   ├── switch.rs
│   └── router.rs
├── sim.rs           # Simulation, scheduler, step()
├── trace.rs         # TraceEvent, TraceKind, TraceLog
└── fault.rs         # Fault
```

Dépendances externes autorisées dans `netiv-core` : `serde` (derive), `rand`, `thiserror`, `tracing` (logs de debug du moteur). Rien d'autre.

---

## 12. Gestion d'erreurs

Deux catégories distinctes, à ne pas mélanger :

1. **Erreurs d'utilisation** (topologie invalide, device inconnu, commande mal formée) → `Result<_, NetivError>` avec `thiserror`, messages lisibles pour l'utilisateur.
2. **Événements réseau** (paquet perdu, TTL expiré, pas de route) → **ne sont pas des erreurs Rust**. Ce sont des `TraceKind::Dropped`. Un réseau qui ne fonctionne pas est le fonctionnement normal du simulateur.

Aucun `unwrap()` ni `panic!` dans le moteur sur des données issues de l'utilisateur. Les invariants internes utilisent `debug_assert!`.

---

## 13. Stratégie de tests

| Niveau | Quoi | Outil |
|---|---|---|
| Unitaire | LPM, parsing CIDR, table MAC, cache ARP, chaque `Node` isolé (event en entrée → effets en sortie) | `#[test]` |
| Propriétés | LPM : la route retournée est toujours la plus spécifique ; parse/format CIDR aller-retour | `proptest` |
| Intégration | Topologies complètes : ping direct, via switch, via 2 routeurs, ARP, TTL expiré, no route | `tests/` |
| Déterminisme | Deux exécutions avec la même graine donnent une trace identique | `assert_eq!` sur `Vec<TraceEvent>` |
| Golden traces | La trace du scénario MVP est comparée à un fichier de référence versionné | `insta` (snapshots) |

Critère d'acceptation du MVP : le test d'intégration `PC1 ── SW1 ── R1 ── R2 ── SERVER` avec `ping` produit la trace attendue (ARP aux deux premiers sauts, TTL 64 → 63 → 62, Echo Reply en retour) et se réexécute à l'identique.

---

## 14. Hors périmètre du MVP (décisions différées)

| Sujet | Pourquoi différé | Impact sur l'architecture actuelle |
|---|---|---|
| VLAN | Pas nécessaire au premier ping | `Frame.vlan` déjà réservé ; le switch gagnera un mode access/trunk |
| TCP / UDP | Machine à états à part entière | `IpPayload` est un `enum` extensible |
| DHCP / DNS | Nécessitent UDP | Seront des `Node` de services côté Host |
| Firewall / ACL | Phase 6 | Nouveau `Node`, ou filtre dans le routeur |
| Vieillissement ARP/MAC | Complexité de timers | Les timers existent déjà (`SetTimer`) |
| STP | Boucles L2 | Hors MVP ; éviter les boucles dans les topologies de test |
| IPv6 | Hors cahier des charges initial | `Ipv4Net` isolé, remplaçable |
| Export pcap | Besoin de sérialiser en octets | Ajouter `to_bytes()` sur les couches sans changer le modèle |
| Mode réel / API / Tokio | Phase 8 | Couche externe qui pilote `Simulation` |
| AI Tutor | Phase 7 | Consomme trace + config, ne touche pas le moteur |

---

## 15. Plan d'implémentation du MVP

Chaque étape se termine par un test qui passe.

1. **Fondations** : workspace, CI (`cargo fmt`, `clippy`, `test`), `SimTime`, ids, `MacAddr`, `Ipv4Net` + tests CIDR.
2. **Scheduler** : `Simulation`, file d'événements, `step()`, RNG seedé. Test : ordre des événements simultanés.
3. **Lien + deux hôtes** : trame brute entre PC1 et PC2, `TraceLog` basique.
4. **ARP + ICMP** : cache ARP, file d'attente `pending`, ping direct. **Jalon : `PC1 ─── PC2`, `ping` fonctionne.**
5. **Switch** : apprentissage MAC, flood, forward. Test : premier ping flood, second unicast.
6. **Table de routage + LPM + `RouteDecision`**.
7. **Routeur** : forward, TTL, ICMP Time Exceeded / Unreachable.
8. **Topologie YAML** + validation d'erreurs.
9. **CLI REPL** : `ping`, `trace <id>`, `show arp|route|mac`, `step`.
10. **Packet Journey** : rendu texte de la trace d'un `TraceId` (avec TTL par saut).
11. **Scénario MVP complet** + golden trace + test de déterminisme. **Jalon : MVP v0.1.**

---

## 16. Questions ouvertes

1. **Latence par défaut des liens** : 100 µs (proposé) ou valeur plus « pédagogique » (1 ms) pour que les RTT soient lisibles ?
2. **Granularité du pas-à-pas** : `step` = un événement du moteur, ou un « saut » complet de paquet ? Proposition : événement par défaut, avec une commande `step hop` en plus.
3. **Switch sans configuration** : ports toujours actifs, sans VLAN. Confirmer que c'est acceptable pour le MVP.
4. **Vieillissement des entrées ARP/MAC** : absent au MVP (proposé), à ajouter avec les pannes ARP du Chaos Engine.
5. **Licence** du dépôt (MIT, Apache-2.0, double licence) à fixer avant la première publication.

---

## Annexe A — Écarts constatés à l'implémentation du MVP

Le MVP v0.1 est implémenté. Voici ce qui diffère de la spec ci-dessus, et pourquoi.

| Sujet | Spec initiale | Implémentation | Raison |
|---|---|---|---|
| RNG | `rand::StdRng` | SplitMix64 maison (`rng.rs`) | Séquence figée pour toujours : les traces de référence ne changent pas à une mise à jour de dépendance. Une dépendance de moins. |
| Comportement des équipements | `Box<dyn Node>` | `enum Behavior { Ip(IpNode), Switch(Switch) }` | Un routeur est un hôte avec `forwarding = true` : ARP, ICMP et routage sont partagés. L'enum donne un accès direct à l'état pour la CLI (caches, tables) sans `Any`. |
| `Frame.meta` | `FrameMeta { trace_id, hops }` | `Frame.trace_id` seul | `hops` n'avait pas d'usage. |
| `FrameSummary` | Résumé léger dans la trace | `Frame` complète clonée | Le Packet Microscope a besoin de tous les champs ; les trames sont petites. |
| `TraceId` | Nouvel id pour la réponse (`caused_by` vers la requête) | **Même id pour toute la transaction** (requête, réponse, ARP déclenché, erreur ICMP) | Un `trace N` montre l'aller-retour complet, ARP compris, ce qui explique pourquoi le 1er ping est plus lent. `caused_by` relie toujours chaque événement à son parent. |
| `step()` | `-> bool` | `-> Option<Fired>` avec la plage de trace produite | Permet d'afficher ce que chaque pas a fait (pas-à-pas). |
| Routes connectées | Table stockée | **Dérivées** des interfaces à chaque lookup | Impossible de les désynchroniser de la configuration. Une interface `down` perd sa route automatiquement. |
| `Command` | `Ping`, `SetIp`... | `Ping` seulement ; la configuration passe par des méthodes de `Simulation` (tracées en `ConfigChanged`) | La configuration n'est pas un événement temporel. |
| Timers | `CancelTimer` | Absent : les timers obsolètes sont ignorés par l'équipement | Plus simple, pas de suppression dans le tas. |
| `RouteDecision` | `candidates` + `chosen` | + `inactive` | Une route dont le prochain saut est hors de tout réseau connecté (mauvaise passerelle) est signalée au lieu de disparaître en silence. |
| Rendu texte | CLI | `netiv-core::render` (pur, sans dépendance terminal) | Testable avec des traces de référence ; la CLI reste fine. |
| Topologie | Erreurs | Erreurs **et avertissements** | Un lab de dépannage contient volontairement des IP en double, etc. |

### Questions ouvertes de la section 16 : décisions prises

1. **Latence par défaut** : 1 ms (lisible : un RTT de 8 ms = 8 traversées de lien).
2. **Pas-à-pas** : `step` = un événement du moteur. Un `step hop` reste à ajouter.
3. **Switch** : tous les ports actifs, sans VLAN.
4. **Vieillissement ARP/MAC** : absent.
5. **Licence** : `MIT OR Apache-2.0` déclarée dans `Cargo.toml`, à confirmer.

### Comportements protocolaires implémentés

- **ARP** : l'hôte qui reçoit une requête qui le concerne apprend l'adresse de l'émetteur (RFC 826). Conséquence visible : SERVER n'a pas besoin de redemander la MAC de R2.
- **Résolution ARP** : 3 tentatives espacées de 1 s, puis abandon des paquets en attente.
- **Ping** : 1 requête par seconde simulée, délai d'attente 2 s. Ping de soi-même = bouclage local.
- **ICMP** : Time Exceeded (TTL) et Destination Unreachable (aucune route) ; jamais d'erreur en réponse à une erreur.
- **Switch** : apprend la MAC source, flood si destination broadcast/inconnue (uniquement vers les ports câblés et actifs), sinon forward ; abandon si la destination est sur le port d'arrivée.

### Reste à faire

Les phases 4 à 8 sont livrées (voir annexe B). Il reste hors périmètre : DHCP/DNS, STP, TUI, PDF natif, comptes/marketplace en ligne, mode réseau réel.


## Annexe B — Phases 4 à 8 : écarts et choix

### Crates ajoutées
`netiv-diagnostics`, `netiv-chaos`, `netiv-security`, `netiv-analysis`, `netiv-education`, `netiv-api`, `netiv-bin`.
Dépendances : `core ← topology ← {diagnostics, chaos, security, analysis} ← education ← {cli, api} ← bin`.
Tokio/axum uniquement dans `netiv-api`.

### Écarts avec le plan initial
| Sujet | Plan | Réalisé | Raison |
|---|---|---|---|
| Protocoles | VLAN, UDP, TCP minimal | idem ; TCP : handshake, données, FIN, RST, 3 retransmissions SYN | DHCP/DNS exclus (périmètre validé). |
| Inter-VLAN | routeur, 1 interface par VLAN | idem (pas de sous-interfaces 802.1Q) | Simplicité. |
| Pannes | rétablies par l'inverse | `Fault` stocke l'état précédent (snapshot/restore) | Réversibilité exacte. |
| Sondes | clone de la simulation | idem (`fresh_clone`, `unfiltered_clone`) | La simulation de l'utilisateur n'est jamais altérée. |
| Tuteur | indices par mots-clés | idem ; aucun LLM | Hors ligne, déterministe. |
| Export | md/json/yaml | idem | PDF : `pandoc` externe. |
| API | sessions en mémoire | idem, **mode bac à sable** (pas d'accès fichier) | Une API locale ne doit pas lire/écrire le disque à la demande d'une page web. |
| Binaire | dans `netiv-cli` | déplacé dans `netiv-bin` | `netiv-cli` devient une bibliothèque réutilisée par l'API. |

### Format `.netiv` (version 1)
YAML : `format_version`, `metadata` (titre, auteur, description), `topology`, et optionnellement `scenario` ou `challenge`.

### Score de santé
Six catégories de 0 à 100, global = moyenne pondérée (poids 25/10/15/20/10/20) :

| Catégorie | Poids | Calcul |
|---|---|---|
| Connectivity | 25 | 100 × paires joignables / paires routables (pings sans filtrage). |
| Addressing | 10 | 100 − 20 × avertissements d'adressage (IP dupliquée, hors réseau, masque incohérent). |
| Routing | 15 | 100 × routes utilisables / routes configurées − 10 × échanges asymétriques. |
| Segmentation | 20 | 100 − pénalités des constats SEC-001 (40), 002 (35), 003 (20), 005 (25), 006 (25). |
| Availability | 10 | 100 − pénalités par point unique de défaillance, câble coupé (−10), interface désactivée (−10). |
| Security | 20 | 100 − pénalités par gravité des constats : HIGH 25, MEDIUM 10, LOW 3. |

Toutes les valeurs sont plafonnées à [0, 100].

### Scoring des défis
Diagnostic +40, correction +40, temps +20 (décroissance linéaire sur le temps conseillé), indice −10, mauvaise réponse −10 ; plancher 0. XP × 1 / 1,5 / 2 selon la difficulté.

### Limites connues
- L'interface web n'est validée que par tests de l'API et un smoke test navigateur ; sa mise en page n'est pas testée automatiquement.
- Pas de vieillissement ARP/MAC ; pas de STP (une boucle L2 n'est pas gérée).

//! Adressage automatique d'une topologie dessinée à la main.
//!
//! [`autoconfigure`] **complète** une topologie sans jamais écraser ce qui
//! est déjà écrit :
//!
//! 1. il calcule les segments de niveau 2 (câbles directs, switches, VLAN) ;
//! 2. il donne un réseau à chaque segment qui n'en a pas (un /30 entre deux
//!    routeurs, un /24 sinon), en réutilisant celui d'une interface déjà
//!    adressée du segment ;
//! 3. il adresse les interfaces vides (routeurs à partir de `.1`, machines à
//!    partir de `.10`) ;
//! 4. il donne une passerelle aux machines qui n'en ont pas ;
//! 5. il ajoute aux routeurs les routes statiques manquantes, par plus court
//!    chemin.
//!
//! Tout est déterministe : même topologie → même résultat. Une seconde
//! exécution ne change plus rien.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::net::Ipv4Addr;

use netiv_core::{DeviceKind, Ipv4Net};

use crate::{InterfaceDef, RouteDef, TopologyDef};

/// Valeur d'adresse qui désigne une interface configurée par DHCP : elle
/// n'est jamais adressée statiquement.
pub const DHCP: &str = "dhcp";

/// Mode d'un port de switch, vu par l'adressage.
#[derive(Clone, Debug)]
enum Port {
    Access(u16),
    Trunk { allowed: Vec<u16>, native: u16 },
}

/// Nœud du graphe de niveau 2 : une interface IP, ou un VLAN d'un switch.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Node {
    Ip { dev: usize, iface: usize },
    Vlan { dev: usize, vlan: u16 },
}

/// Union-find sur des indices.
struct Dsu(Vec<usize>);

impl Dsu {
    fn find(&mut self, x: usize) -> usize {
        let mut r = x;
        while self.0[r] != r {
            r = self.0[r];
        }
        let mut c = x;
        while self.0[c] != r {
            let next = self.0[c];
            self.0[c] = r;
            c = next;
        }
        r
    }

    fn union(&mut self, a: usize, b: usize) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            // Le plus petit indice devient la racine : résultat déterministe.
            let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
            self.0[hi] = lo;
        }
    }
}

/// Un segment de niveau 2 : les interfaces IP qui s'y trouvent.
#[derive(Clone, Debug)]
struct Segment {
    /// `(équipement, interface)`, dans l'ordre de la topologie.
    members: Vec<(usize, usize)>,
    net: Option<Ipv4Net>,
}

fn is_router(kind: DeviceKind) -> bool {
    matches!(kind, DeviceKind::Router | DeviceKind::Firewall)
}

fn parse_ip(i: &InterfaceDef) -> Option<Ipv4Net> {
    i.ip.as_deref()
        .filter(|s| *s != DHCP)
        .and_then(|s| s.parse().ok())
}

fn overlaps(a: Ipv4Net, b: Ipv4Net) -> bool {
    a.contains(b.network()) || b.contains(a.network())
}

fn net(addr: u32, prefix: u8) -> Option<Ipv4Net> {
    Ipv4Net::new(Ipv4Addr::from(addr), prefix).ok()
}

/// Réserve le prochain réseau libre de taille `prefix` à partir de `start`,
/// par pas de la taille du réseau.
fn allocate(used: &mut Vec<Ipv4Net>, start: u32, prefix: u8) -> Option<Ipv4Net> {
    let step = 1u32 << (32 - u32::from(prefix));
    let mut cur = start;
    for _ in 0..65_536 {
        let n = net(cur, prefix)?;
        if !used.iter().any(|u| overlaps(*u, n)) {
            used.push(n);
            return Some(n);
        }
        cur = cur.checked_add(step)?;
    }
    None
}

/// Première adresse hôte libre de `n`, à partir de l'offset `from`.
fn free_host(n: Ipv4Net, taken: &BTreeSet<Ipv4Addr>, from: u32) -> Option<Ipv4Addr> {
    let base = u32::from(n.network());
    let last = u32::from(n.broadcast());
    let mut a = base.checked_add(from.max(1))?;
    // Petit réseau (/30) : on commence au premier hôte.
    if a >= last {
        a = base + 1;
    }
    while a < last {
        let ip = Ipv4Addr::from(a);
        if !taken.contains(&ip) {
            return Some(ip);
        }
        a += 1;
    }
    None
}

/// Complète `def` et rend la liste des changements (vide : rien à faire).
pub fn autoconfigure(def: &mut TopologyDef) -> Vec<String> {
    let kinds: Vec<Option<DeviceKind>> = def.devices.iter().map(|d| d.kind.parse().ok()).collect();
    let index: BTreeMap<&str, usize> = def
        .devices
        .iter()
        .enumerate()
        .map(|(i, d)| (d.name.as_str(), i))
        .collect();

    // Interfaces de chaque équipement, ports implicites des switches compris.
    let mut iface_names: Vec<Vec<String>> = Vec::new();
    let mut ports: BTreeMap<(usize, String), Port> = BTreeMap::new();
    let mut vlans_used: BTreeSet<u16> = BTreeSet::from([1]);
    for (di, d) in def.devices.iter().enumerate() {
        let mut names = Vec::new();
        if kinds[di] == Some(DeviceKind::Switch) {
            for n in 1..=d.ports.unwrap_or(0) {
                let name = format!("port{n}");
                ports.insert((di, name.clone()), Port::Access(1));
                names.push(name);
            }
            for i in &d.interfaces {
                let mode = match (&i.trunk, i.vlan) {
                    (Some(allowed), _) => Port::Trunk {
                        allowed: allowed.clone(),
                        native: i.native_vlan.unwrap_or(1),
                    },
                    (None, v) => Port::Access(v.unwrap_or(1)),
                };
                if let Port::Access(v) = &mode {
                    vlans_used.insert(*v);
                }
                if let Port::Trunk { allowed, native } = &mode {
                    vlans_used.extend(allowed.iter().copied());
                    vlans_used.insert(*native);
                }
                ports.insert((di, i.name.clone()), mode);
                names.push(i.name.clone());
            }
        } else {
            names.extend(d.interfaces.iter().map(|i| i.name.clone()));
        }
        iface_names.push(names);
    }

    // Nœuds du graphe.
    let mut nodes: Vec<Node> = Vec::new();
    let mut node_of: BTreeMap<Node, usize> = BTreeMap::new();
    let mut intern = |n: Node, nodes: &mut Vec<Node>| -> usize {
        *node_of.entry(n.clone()).or_insert_with(|| {
            nodes.push(n);
            nodes.len() - 1
        })
    };
    for (di, d) in def.devices.iter().enumerate() {
        if kinds[di].is_some_and(|k| k != DeviceKind::Switch) {
            for ii in 0..d.interfaces.len() {
                intern(Node::Ip { dev: di, iface: ii }, &mut nodes);
            }
        }
    }

    /// Ce à quoi se raccorde une extrémité de câble.
    enum End {
        Ip(Node),
        Port(usize, Port),
    }
    let resolve = |ep: &str| -> Option<End> {
        let (dev, iface) = ep.split_once(':')?;
        let di = *index.get(dev)?;
        if kinds[di] == Some(DeviceKind::Switch) {
            return ports
                .get(&(di, iface.to_string()))
                .map(|p| End::Port(di, p.clone()));
        }
        let ii = def.devices[di]
            .interfaces
            .iter()
            .position(|i| i.name == iface)?;
        Some(End::Ip(Node::Ip { dev: di, iface: ii }))
    };

    let mut pairs: Vec<(Node, Node)> = Vec::new();
    for l in &def.links {
        let (Some(a), Some(b)) = (resolve(&l.a), resolve(&l.b)) else {
            continue;
        };
        let vl = |dev: usize, vlan: u16| Node::Vlan { dev, vlan };
        // Trames non étiquetées d'un port : VLAN d'accès ou VLAN natif.
        let untagged = |p: &Port| match p {
            Port::Access(v) => *v,
            Port::Trunk { native, .. } => *native,
        };
        match (a, b) {
            (End::Ip(x), End::Ip(y)) => pairs.push((x, y)),
            (End::Ip(x), End::Port(s, p)) | (End::Port(s, p), End::Ip(x)) => {
                pairs.push((x, vl(s, untagged(&p))));
            }
            (End::Port(s1, p1), End::Port(s2, p2)) => {
                pairs.push((vl(s1, untagged(&p1)), vl(s2, untagged(&p2))));
                if let (Port::Trunk { allowed: a1, .. }, Port::Trunk { allowed: a2, .. }) =
                    (&p1, &p2)
                {
                    let carries = |set: &Vec<u16>, v: u16| set.is_empty() || set.contains(&v);
                    for &v in &vlans_used {
                        if carries(a1, v) && carries(a2, v) {
                            pairs.push((vl(s1, v), vl(s2, v)));
                        }
                    }
                }
            }
        }
    }
    let ids: Vec<(usize, usize)> = pairs
        .into_iter()
        .map(|(x, y)| (intern(x, &mut nodes), intern(y, &mut nodes)))
        .collect();
    let mut dsu = Dsu((0..nodes.len()).collect());
    for (x, y) in ids {
        dsu.union(x, y);
    }

    // Segments : composantes contenant au moins une interface IP.
    let mut by_root: BTreeMap<usize, Vec<(usize, usize)>> = BTreeMap::new();
    for (i, n) in nodes.iter().enumerate() {
        if let Node::Ip { dev, iface } = n {
            by_root.entry(dsu.find(i)).or_default().push((*dev, *iface));
        }
    }
    let mut segments: Vec<Segment> = by_root
        .into_values()
        .map(|mut members| {
            members.sort_unstable();
            Segment { members, net: None }
        })
        .collect();
    segments.sort_by_key(|s| s.members[0]);

    // Réseaux déjà utilisés (interfaces et routes) : on n'y touche pas.
    let mut used: Vec<Ipv4Net> = Vec::new();
    for d in &def.devices {
        for i in &d.interfaces {
            if let Some(n) = parse_ip(i) {
                used.push(n.trunc());
            }
        }
        for r in &d.routes {
            if let Ok(n) = r.dest.parse::<Ipv4Net>() {
                used.push(n.trunc());
            }
        }
    }

    let mut report: Vec<String> = Vec::new();

    // 2–3. Réseau de chaque segment, puis adresses.
    for seg in &mut segments {
        let iface = |m: &(usize, usize)| &def.devices[m.0].interfaces[m.1];
        // Une interface DHCP ou une interface isolée sans câble ne décide rien.
        let addressable: Vec<(usize, usize)> = seg
            .members
            .iter()
            .copied()
            .filter(|m| iface(m).ip.as_deref() != Some(DHCP))
            .collect();
        seg.net = addressable
            .iter()
            .find_map(|m| parse_ip(iface(m)))
            .map(Ipv4Net::trunc);
        let needs = addressable.iter().any(|m| iface(m).ip.is_none());
        if seg.net.is_none() && needs {
            // Un segment seul (interface sans câble) n'a pas besoin d'adresse.
            if seg.members.len() < 2 {
                continue;
            }
            let routers_only = addressable.len() == 2
                && addressable
                    .iter()
                    .all(|m| kinds[m.0].is_some_and(is_router));
            seg.net = if routers_only {
                allocate(&mut used, u32::from(Ipv4Addr::new(10, 255, 0, 0)), 30)
            } else {
                allocate(&mut used, u32::from(Ipv4Addr::new(10, 0, 1, 0)), 24)
            };
        }
        let Some(n) = seg.net else { continue };
        let mut taken: BTreeSet<Ipv4Addr> = seg
            .members
            .iter()
            .filter_map(|m| parse_ip(iface(m)).map(|x| x.addr()))
            .collect();
        // Routeurs d'abord (à partir de .1), puis les machines (à partir de .10).
        let mut order = addressable.clone();
        order.sort_by_key(|m| (!kinds[m.0].is_some_and(is_router), *m));
        for m in order {
            if def.devices[m.0].interfaces[m.1].ip.is_some() {
                continue;
            }
            let from = if kinds[m.0].is_some_and(is_router) {
                1
            } else {
                10
            };
            let Some(ip) = free_host(n, &taken, from) else {
                report.push(format!(
                    "réseau {n} plein : {} non adressé",
                    def.devices[m.0].name
                ));
                continue;
            };
            taken.insert(ip);
            let cidr = format!("{ip}/{}", n.prefix());
            let d = &mut def.devices[m.0];
            report.push(format!("{}:{} → {cidr}", d.name, d.interfaces[m.1].name));
            d.interfaces[m.1].ip = Some(cidr);
        }
    }

    // Adresses (après affectation) et segment de chaque interface.
    let addr_of =
        |def: &TopologyDef, m: (usize, usize)| parse_ip(&def.devices[m.0].interfaces[m.1]);
    let mut seg_of: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for (si, seg) in segments.iter().enumerate() {
        for m in &seg.members {
            seg_of.insert(*m, si);
        }
    }

    // 4. Passerelles des machines.
    for di in 0..def.devices.len() {
        let Some(kind) = kinds[di] else { continue };
        if !matches!(kind, DeviceKind::Host | DeviceKind::Server)
            || def.devices[di].gateway.is_some()
        {
            continue;
        }
        let gw = (0..def.devices[di].interfaces.len()).find_map(|ii| {
            let si = *seg_of.get(&(di, ii))?;
            addr_of(def, (di, ii))?;
            segments[si]
                .members
                .iter()
                .filter(|m| kinds[m.0].is_some_and(is_router))
                .find_map(|m| addr_of(def, *m).map(|n| n.addr()))
        });
        if let Some(gw) = gw {
            report.push(format!("{} : passerelle {gw}", def.devices[di].name));
            def.devices[di].gateway = Some(gw.to_string());
        }
    }

    // 5. Routes statiques entre routeurs, par plus court chemin.
    let routers: Vec<usize> = (0..def.devices.len())
        .filter(|&d| kinds[d].is_some_and(is_router))
        .collect();
    // Segments où chaque routeur a une interface adressée.
    let seg_ifaces = |def: &TopologyDef, r: usize| -> Vec<(usize, Ipv4Addr)> {
        (0..def.devices[r].interfaces.len())
            .filter_map(|ii| Some((*seg_of.get(&(r, ii))?, addr_of(def, (r, ii))?.addr())))
            .collect()
    };
    let attached: BTreeMap<usize, Vec<(usize, Ipv4Addr)>> =
        routers.iter().map(|&r| (r, seg_ifaces(def, r))).collect();
    for &r in &routers {
        let mine: BTreeSet<usize> = attached[&r].iter().map(|x| x.0).collect();
        // BFS sur les routeurs ; `first[x]` = adresse du premier saut vers x.
        let mut first: BTreeMap<usize, Ipv4Addr> = BTreeMap::new();
        // Ordre de découverte = distance croissante (propriété du BFS).
        let mut discovered: Vec<usize> = Vec::new();
        let mut seen: BTreeSet<usize> = BTreeSet::from([r]);
        let mut queue: VecDeque<usize> = VecDeque::from([r]);
        while let Some(x) = queue.pop_front() {
            for &(seg, _) in &attached[&x] {
                for &y in &routers {
                    if seen.contains(&y) {
                        continue;
                    }
                    let Some(&(_, y_ip)) = attached[&y].iter().find(|(s, _)| *s == seg) else {
                        continue;
                    };
                    seen.insert(y);
                    let hop = if x == r { y_ip } else { first[&x] };
                    first.insert(y, hop);
                    discovered.push(y);
                    queue.push_back(y);
                }
            }
        }
        // Le routeur le plus proche qui touche chaque segment distant.
        let mut targets: BTreeMap<usize, Ipv4Addr> = BTreeMap::new();
        for y in discovered {
            for &(seg, _) in &attached[&y] {
                if !mine.contains(&seg) {
                    targets.entry(seg).or_insert(first[&y]);
                }
            }
        }
        for (seg, via) in targets {
            let Some(n) = segments[seg].net else { continue };
            let dest = n.trunc();
            let d = &mut def.devices[r];
            let exists = d
                .routes
                .iter()
                .any(|x| x.dest.parse::<Ipv4Net>().is_ok_and(|e| e.trunc() == dest));
            if exists {
                continue;
            }
            report.push(format!("{} : route {dest} via {via}", d.name));
            d.routes.push(RouteDef {
                dest: dest.to_string(),
                via: via.to_string(),
            });
        }
    }
    report
}

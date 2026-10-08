//! Générateur de topologies : un plan d'adressage et un routage statique
//! complets, calculés automatiquement, pour des réseaux de taille variable.
//!
//! Le résultat est **toujours entièrement joignable** (testé pour chaque type) :
//! chaque LAN est un /24 `10.i.0.0/24` dont le routeur est `.1`, chaque lien
//! entre routeurs est un /30 pris dans `172.16.0.0/16`, et chaque routeur reçoit
//! une route statique vers chaque réseau distant, via le plus court chemin.

use std::collections::VecDeque;
use std::net::Ipv4Addr;
use std::str::FromStr;

use crate::{
    AclDef, DeviceDef, InterfaceDef, LinkDef, NetworkDef, RouteDef, RuleDefAcl, ServiceDef,
    TopologyDef,
};

/// Nombre maximal de routeurs (un /24 par routeur dans 10.0.0.0/8).
pub const MAX_ROUTERS: usize = 200;
/// Nombre maximal de machines.
pub const MAX_HOSTS: usize = 5_000;
/// Machines maximum par LAN (un /24 en contient 253 utilisables).
const MAX_PER_LAN: usize = 240;

/// Type de réseau à générer.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TopoKind {
    /// Un seul switch, pas de routeur.
    Lan,
    /// Étoile : un routeur central (serveurs) et des sites.
    Enterprise,
    /// Deux routeurs de cœur reliés, et des routeurs de distribution doublés.
    Campus,
    /// Spine-leaf : deux spines et des leaves reliés aux deux.
    Datacenter,
    /// Routeurs en ligne, clients aux deux extrémités.
    Isp,
    /// Anneau de routeurs, chacun avec son LAN.
    Hybrid,
    /// Internet / pare-feu / DMZ / LAN / serveurs.
    CyberRange,
}

impl TopoKind {
    /// Tous les types.
    pub const ALL: [TopoKind; 7] = [
        TopoKind::Lan,
        TopoKind::Enterprise,
        TopoKind::Campus,
        TopoKind::Datacenter,
        TopoKind::Isp,
        TopoKind::Hybrid,
        TopoKind::CyberRange,
    ];

    /// Nom court.
    pub fn slug(self) -> &'static str {
        match self {
            TopoKind::Lan => "lan",
            TopoKind::Enterprise => "enterprise",
            TopoKind::Campus => "campus",
            TopoKind::Datacenter => "datacenter",
            TopoKind::Isp => "isp",
            TopoKind::Hybrid => "hybrid",
            TopoKind::CyberRange => "cyber-range",
        }
    }

    /// Nombre minimal de routeurs.
    pub fn min_routers(self) -> usize {
        match self {
            TopoKind::Lan | TopoKind::CyberRange => 0,
            TopoKind::Enterprise | TopoKind::Isp => 2,
            TopoKind::Campus | TopoKind::Datacenter | TopoKind::Hybrid => 3,
        }
    }
}

impl FromStr for TopoKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let t = s.to_ascii_lowercase().replace(['_', ' '], "-");
        TopoKind::ALL
            .iter()
            .copied()
            .find(|k| {
                k.slug() == t
                    || (t == "entreprise" && *k == TopoKind::Enterprise)
                    || (t == "cyberrange" && *k == TopoKind::CyberRange)
            })
            .ok_or_else(|| {
                format!(
                    "type de réseau inconnu : « {s} » (types : {})",
                    TopoKind::ALL.map(TopoKind::slug).join(", ")
                )
            })
    }
}

/// Paramètres de génération.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct GenSpec {
    /// Type de réseau.
    pub kind: TopoKind,
    /// Nombre de routeurs (ignoré pour `lan` et `cyber-range`).
    pub routers: usize,
    /// Nombre de postes utilisateurs.
    pub hosts: usize,
    /// Graine : fait varier la répartition des machines, pas la structure.
    pub seed: u64,
}

struct Router {
    name: String,
    ifaces: Vec<InterfaceDef>,
    routes: Vec<RouteDef>,
    /// Réseau du LAN rattaché, s'il y en a un.
    lan: Option<(u32, usize)>, // (numéro de LAN, hôtes prévus)
    /// Voisins : (index, mon adresse sur le lien, adresse du voisin).
    neighbors: Vec<(usize, Ipv4Addr, Ipv4Addr)>,
}

fn iface(name: &str, ip: String) -> InterfaceDef {
    InterfaceDef {
        name: name.to_string(),
        ip: Some(ip),
        management: false,
        vlan: None,
        trunk: None,
        native_vlan: None,
    }
}

fn device(name: &str, kind: &str, role: Option<&str>) -> DeviceDef {
    DeviceDef {
        name: name.to_string(),
        kind: kind.to_string(),
        role: role.map(str::to_string),
        interfaces: Vec::new(),
        ports: None,
        gateway: None,
        routes: Vec::new(),
        services: Vec::new(),
        arp: Vec::new(),
        acl: None,
    }
}

fn link(a: String, b: String) -> LinkDef {
    LinkDef {
        a,
        b,
        latency_us: None,
        loss: None,
    }
}

/// Génère une topologie.
pub fn generate(spec: &GenSpec) -> Result<TopologyDef, String> {
    if spec.hosts > MAX_HOSTS {
        return Err(format!(
            "trop de machines : {} (maximum {MAX_HOSTS})",
            spec.hosts
        ));
    }
    match spec.kind {
        TopoKind::Lan => return lan(spec),
        TopoKind::CyberRange => return cyber_range(spec),
        _ => {}
    }
    let n = spec.routers;
    if n < spec.kind.min_routers() {
        return Err(format!(
            "le type « {} » demande au moins {} routeurs (reçu : {n})",
            spec.kind.slug(),
            spec.kind.min_routers()
        ));
    }
    if n > MAX_ROUTERS {
        return Err(format!("trop de routeurs : {n} (maximum {MAX_ROUTERS})"));
    }

    // 1. Graphe des routeurs et emplacement des LAN.
    let (edges, user_lans, server_lans) = shape(spec.kind, n);
    let servers = (spec.hosts / 20).max(1);

    // 2. Routeurs, adresses des liens.
    let mut routers: Vec<Router> = (0..n)
        .map(|i| Router {
            name: format!("R{}", i + 1),
            ifaces: Vec::new(),
            routes: Vec::new(),
            lan: None,
            neighbors: Vec::new(),
        })
        .collect();
    let mut lan_counter = 0u32;
    // Un LAN par routeur concerné, dans l'ordre des indices.
    let mut lan_of: Vec<Option<u32>> = vec![None; n];
    for (i, slot) in lan_of.iter_mut().enumerate() {
        if user_lans.contains(&i) || server_lans.contains(&i) {
            lan_counter += 1;
            *slot = Some(lan_counter);
        }
    }
    // Répartition des machines.
    let mut user_per_lan = vec![0usize; n];
    let mut server_per_lan = vec![0usize; n];
    let offset = (spec.seed % user_lans.len().max(1) as u64) as usize;
    for k in 0..spec.hosts {
        let lan = user_lans[(k + offset) % user_lans.len()];
        user_per_lan[lan] += 1;
    }
    for k in 0..servers {
        let lan = server_lans[k % server_lans.len()];
        server_per_lan[lan] += 1;
    }
    for (i, r) in routers.iter_mut().enumerate() {
        if let Some(num) = lan_of[i] {
            if num > 250 {
                return Err("plan d'adressage épuisé : trop de LAN".to_string());
            }
            let hosts = user_per_lan[i] + server_per_lan[i];
            if hosts > MAX_PER_LAN {
                return Err(format!(
                    "{} machines sur un même LAN : le maximum est {MAX_PER_LAN} (ajoute des routeurs)",
                    hosts
                ));
            }
            r.lan = Some((num, hosts));
            r.ifaces.push(iface("eth0", format!("10.{num}.0.1/24")));
        }
    }
    let mut links: Vec<LinkDef> = Vec::new();
    for (k, (a, b)) in edges.iter().enumerate() {
        if k >= 16_000 {
            return Err("trop de liens entre routeurs".to_string());
        }
        let base = (k as u32) * 4;
        let net =
            |host: u32| Ipv4Addr::new(172, 16, (base >> 8) as u8, ((base & 0xff) + host) as u8);
        let (ia, ib) = (net(1), net(2));
        let na = format!("eth{}", routers[*a].ifaces.len());
        let nb = format!("eth{}", routers[*b].ifaces.len());
        routers[*a].ifaces.push(iface(&na, format!("{ia}/30")));
        routers[*b].ifaces.push(iface(&nb, format!("{ib}/30")));
        routers[*a].neighbors.push((*b, ia, ib));
        routers[*b].neighbors.push((*a, ib, ia));
        links.push(link(
            format!("{}:{na}", routers[*a].name),
            format!("{}:{nb}", routers[*b].name),
        ));
    }

    // 3. Routes statiques : plus court chemin (BFS, ordre déterministe).
    let adj: Vec<Vec<usize>> = routers
        .iter()
        .map(|r| {
            let mut v: Vec<usize> = r.neighbors.iter().map(|(j, _, _)| *j).collect();
            v.sort_unstable();
            v
        })
        .collect();
    for src in 0..n {
        // premier saut vers chaque routeur
        let mut first_hop: Vec<Option<usize>> = vec![None; n];
        let mut dist: Vec<usize> = vec![usize::MAX; n];
        dist[src] = 0;
        let mut q = VecDeque::from([src]);
        while let Some(x) = q.pop_front() {
            for &y in &adj[x] {
                if dist[y] == usize::MAX {
                    dist[y] = dist[x] + 1;
                    first_hop[y] = if x == src { Some(y) } else { first_hop[x] };
                    q.push_back(y);
                }
            }
        }
        let next_ip = |hop: usize| -> Option<Ipv4Addr> {
            routers[src]
                .neighbors
                .iter()
                .find(|(j, _, _)| *j == hop)
                .map(|(_, _, ip)| *ip)
        };
        let mut routes: Vec<RouteDef> = Vec::new();
        // LAN distants
        for (j, rj) in routers.iter().enumerate() {
            if j == src {
                continue;
            }
            if let (Some((num, _)), Some(hop)) = (rj.lan, first_hop[j]) {
                if let Some(via) = next_ip(hop) {
                    routes.push(RouteDef {
                        dest: format!("10.{num}.0.0/24"),
                        via: via.to_string(),
                    });
                }
            }
        }
        // liaisons point-à-point non directement attachées
        for (k, (a, b)) in edges.iter().enumerate() {
            if *a == src || *b == src {
                continue;
            }
            let target = if (dist[*a], *a) <= (dist[*b], *b) {
                *a
            } else {
                *b
            };
            if let Some(hop) = first_hop[target] {
                if let Some(via) = next_ip(hop) {
                    let base = (k as u32) * 4;
                    routes.push(RouteDef {
                        dest: format!("172.16.{}.{}/30", base >> 8, base & 0xff),
                        via: via.to_string(),
                    });
                }
            }
        }
        routers[src].routes = routes;
    }

    // 4. Équipements finaux.
    let mut devices: Vec<DeviceDef> = Vec::new();
    let mut pc = 0usize;
    let mut srv = 0usize;
    let mut host_defs: Vec<DeviceDef> = Vec::new();
    let mut switch_defs: Vec<DeviceDef> = Vec::new();
    for (i, r) in routers.iter().enumerate() {
        let Some((num, hosts)) = r.lan else { continue };
        let sw = format!("SW{}", i + 1);
        let mut s = device(&sw, "switch", None);
        s.ports = Some(u32::try_from(hosts + 2).unwrap_or(u32::MAX));
        switch_defs.push(s);
        links.push(link(format!("{}:eth0", r.name), format!("{sw}:port1")));
        for (k, kind) in std::iter::repeat_n("host", user_per_lan[i])
            .chain(std::iter::repeat_n("server", server_per_lan[i]))
            .enumerate()
        {
            let host_no = u32::try_from(k).unwrap_or(0) + 1;
            let port = k + 2;
            let addr = format!("10.{num}.0.{}", 10 + host_no);
            let name = if kind == "server" {
                srv += 1;
                format!("SRV{srv}")
            } else {
                pc += 1;
                format!("PC{pc}")
            };
            let mut h = device(&name, kind, None);
            h.interfaces = vec![iface("eth0", format!("{addr}/24"))];
            h.gateway = Some(format!("10.{num}.0.1"));
            if kind == "server" {
                h.services = vec![
                    ServiceDef {
                        proto: "tcp".into(),
                        port: 80,
                        name: "http".into(),
                    },
                    ServiceDef {
                        proto: "tcp".into(),
                        port: 22,
                        name: "ssh".into(),
                    },
                ];
            }
            host_defs.push(h);
            links.push(link(format!("{name}:eth0"), format!("{sw}:port{port}")));
        }
    }
    for r in routers {
        let mut d = device(&r.name, "router", None);
        d.interfaces = r.ifaces;
        d.routes = r.routes;
        devices.push(d);
    }
    devices.extend(switch_defs);
    devices.extend(host_defs);
    Ok(TopologyDef {
        network: NetworkDef {
            name: format!("{}-{}r-{}h", spec.kind.slug(), n, spec.hosts),
            seed: spec.seed,
        },
        devices,
        links,
    })
}

/// Arêtes entre routeurs, routeurs ayant un LAN d'utilisateurs, routeurs ayant un LAN de serveurs.
fn shape(kind: TopoKind, n: usize) -> (Vec<(usize, usize)>, Vec<usize>, Vec<usize>) {
    match kind {
        TopoKind::Enterprise => {
            let edges = (1..n).map(|i| (0, i)).collect();
            (edges, (1..n).collect(), vec![0])
        }
        TopoKind::Campus => {
            let mut edges = vec![(0, 1)];
            for i in 2..n {
                edges.push((0, i));
                edges.push((1, i));
            }
            (edges, (2..n).collect(), vec![0])
        }
        TopoKind::Datacenter => {
            let mut edges = Vec::new();
            for i in 2..n {
                edges.push((0, i));
                edges.push((1, i));
            }
            // Les serveurs sont répartis sur toutes les leaves sauf la dernière,
            // qui héberge les postes ; avec une seule leaf, tout y est mêlé.
            let leaves: Vec<usize> = (2..n).collect();
            let servers: Vec<usize> = if leaves.len() > 1 {
                leaves[..leaves.len() - 1].to_vec()
            } else {
                leaves.clone()
            };
            let users = vec![leaves[leaves.len() - 1]];
            (edges, users, servers)
        }
        TopoKind::Isp => {
            let edges = (0..n - 1).map(|i| (i, i + 1)).collect();
            (edges, vec![0], vec![n - 1])
        }
        TopoKind::Hybrid => {
            let mut edges: Vec<(usize, usize)> = (0..n - 1).map(|i| (i, i + 1)).collect();
            edges.push((n - 1, 0));
            (edges, (1..n).collect(), vec![0])
        }
        TopoKind::Lan | TopoKind::CyberRange => (Vec::new(), Vec::new(), Vec::new()),
    }
}

fn lan(spec: &GenSpec) -> Result<TopologyDef, String> {
    let servers = (spec.hosts / 20).max(1);
    let total = spec.hosts + servers;
    if total > MAX_PER_LAN {
        return Err(format!(
            "{total} machines sur un seul LAN : le maximum est {MAX_PER_LAN} (utilise un type avec routeurs)"
        ));
    }
    let mut devices = vec![{
        let mut s = device("SW1", "switch", None);
        s.ports = Some(u32::try_from(total + 1).unwrap_or(u32::MAX));
        s
    }];
    let mut links = Vec::new();
    for k in 0..total {
        let is_server = k >= spec.hosts;
        let name = if is_server {
            format!("SRV{}", k - spec.hosts + 1)
        } else {
            format!("PC{}", k + 1)
        };
        let mut h = device(&name, if is_server { "server" } else { "host" }, None);
        h.interfaces = vec![iface("eth0", format!("192.168.0.{}/24", 10 + k))];
        if is_server {
            h.services = vec![ServiceDef {
                proto: "tcp".into(),
                port: 80,
                name: "http".into(),
            }];
        }
        links.push(link(format!("{name}:eth0"), format!("SW1:port{}", k + 1)));
        devices.push(h);
    }
    Ok(TopologyDef {
        network: NetworkDef {
            name: format!("lan-{}h", spec.hosts),
            seed: spec.seed,
        },
        devices,
        links,
    })
}

fn cyber_range(spec: &GenSpec) -> Result<TopologyDef, String> {
    let users = spec.hosts.max(1);
    let web = (users / 10).clamp(1, 20);
    if users > MAX_PER_LAN {
        return Err(format!(
            "{users} postes sur un seul LAN : le maximum est {MAX_PER_LAN}"
        ));
    }
    let mut devices: Vec<DeviceDef> = Vec::new();
    let mut links: Vec<LinkDef> = Vec::new();

    let mut inet = device("INET", "host", Some("internet"));
    inet.interfaces = vec![iface("eth0", "203.0.113.10/24".into())];
    inet.gateway = Some("203.0.113.1".into());
    inet.services = vec![ServiceDef {
        proto: "tcp".into(),
        port: 80,
        name: "http".into(),
    }];
    devices.push(inet);

    let rule = |action: &str,
                proto: Option<&str>,
                dst: &str,
                port: Option<&str>,
                inn: &str,
                comment: &str| RuleDefAcl {
        action: action.into(),
        proto: proto.map(str::to_string),
        src: None,
        dst: Some(dst.into()),
        port: port.map(str::to_string),
        in_iface: Some(inn.into()),
        comment: comment.into(),
    };
    let mut fw = device("FW1", "firewall", None);
    fw.interfaces = vec![
        iface("eth0", "203.0.113.1/24".into()),
        iface("eth1", "10.9.0.1/24".into()),
        iface("eth2", "192.168.10.1/24".into()),
        iface("eth3", "192.168.20.1/24".into()),
    ];
    fw.acl = Some(AclDef {
        default: None,
        stateful: None,
        rules: vec![
            rule(
                "allow",
                Some("tcp"),
                "10.9.0.0/24",
                Some("80"),
                "eth0",
                "Internet vers web DMZ",
            ),
            rule(
                "allow",
                Some("tcp"),
                "10.9.0.0/24",
                Some("80"),
                "eth2",
                "LAN vers web DMZ",
            ),
            rule(
                "allow",
                None,
                "203.0.113.0/24",
                None,
                "eth2",
                "LAN vers Internet",
            ),
            rule(
                "allow",
                Some("tcp"),
                "192.168.20.0/24",
                Some("445"),
                "eth2",
                "LAN vers partage de fichiers",
            ),
            rule(
                "allow",
                Some("icmp"),
                "0.0.0.0/0",
                None,
                "eth2",
                "ping depuis le LAN",
            ),
        ],
    });
    devices.push(fw);
    links.push(link("INET:eth0".into(), "FW1:eth0".into()));

    // DMZ
    let mut swd = device("SWD", "switch", None);
    swd.ports = Some(u32::try_from(web + 1).unwrap_or(u32::MAX));
    devices.push(swd);
    links.push(link("FW1:eth1".into(), "SWD:port1".into()));
    for k in 0..web {
        let name = format!("WEB{:02}", k + 1);
        let mut h = device(&name, "server", Some("dmz"));
        h.interfaces = vec![iface("eth0", format!("10.9.0.{}/24", 10 + k))];
        h.gateway = Some("10.9.0.1".into());
        h.services = vec![ServiceDef {
            proto: "tcp".into(),
            port: 80,
            name: "http".into(),
        }];
        links.push(link(format!("{name}:eth0"), format!("SWD:port{}", k + 2)));
        devices.push(h);
    }
    // LAN
    let mut swl = device("SWL", "switch", None);
    swl.ports = Some(u32::try_from(users + 1).unwrap_or(u32::MAX));
    devices.push(swl);
    links.push(link("FW1:eth2".into(), "SWL:port1".into()));
    for k in 0..users {
        let name = format!("PC{:02}", k + 1);
        let mut h = device(&name, "host", None);
        h.interfaces = vec![iface("eth0", format!("192.168.10.{}/24", 10 + k))];
        h.gateway = Some("192.168.10.1".into());
        links.push(link(format!("{name}:eth0"), format!("SWL:port{}", k + 2)));
        devices.push(h);
    }
    // Serveur de fichiers
    let mut file = device("FILE01", "server", None);
    file.interfaces = vec![iface("eth0", "192.168.20.20/24".into())];
    file.gateway = Some("192.168.20.1".into());
    file.services = vec![
        ServiceDef {
            proto: "tcp".into(),
            port: 445,
            name: "smb".into(),
        },
        ServiceDef {
            proto: "tcp".into(),
            port: 22,
            name: "ssh".into(),
        },
    ];
    devices.push(file);
    links.push(link("FW1:eth3".into(), "FILE01:eth0".into()));
    Ok(TopologyDef {
        network: NetworkDef {
            name: format!("cyber-range-{users}h"),
            seed: spec.seed,
        },
        devices,
        links,
    })
}

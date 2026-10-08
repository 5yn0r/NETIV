//! Catalogue des pannes que le Chaos Engine sait provoquer.
//!
//! Chaque gabarit décrit **une** nature de panne (`CauseKind`) et sait la
//! construire sur un chemin donné, ou répondre `None` si elle n'y a pas de
//! sens (pas de switch sur le chemin, pas de route statique à retirer...).

use std::net::Ipv4Addr;

use netiv_core::acl::{AclAction, AclRule, RuleProto};
use netiv_core::device::Behavior;
use netiv_core::packet::L4Proto;
use netiv_core::route::StaticRoute;
use netiv_core::switch::PortMode;
use netiv_core::{DeviceId, Fault, Ipv4Net, LinkId, MacAddr, SimRng, SimTime, Simulation};
use netiv_diagnostics::CauseKind;
use netiv_diagnostics::l2::owner_of;

use crate::{Difficulty, PathInfo};

/// Construit la panne ou renonce.
pub type Builder = fn(&Simulation, &PathInfo, &mut SimRng) -> Option<Fault>;

/// Un type de panne.
pub struct Template {
    /// Identifiant stable.
    pub id: &'static str,
    /// Nature de la panne (ce que le diagnostic doit retrouver).
    pub kind: CauseKind,
    /// Difficulté.
    pub difficulty: Difficulty,
    /// Explication pédagogique en une phrase.
    pub summary: &'static str,
    /// Constructeur.
    pub build: Builder,
}

/// Tous les gabarits.
pub fn catalog() -> &'static [Template] {
    &CATALOG
}

static CATALOG: [Template; 17] = [
    Template {
        id: "link-down",
        kind: CauseKind::LinkDown,
        difficulty: Difficulty::Easy,
        summary: "un câble du chemin est coupé",
        build: link_down,
    },
    Template {
        id: "interface-down",
        kind: CauseKind::InterfaceDown,
        difficulty: Difficulty::Easy,
        summary: "une interface du chemin est désactivée",
        build: interface_down,
    },
    Template {
        id: "packet-loss",
        kind: CauseKind::PacketLoss,
        difficulty: Difficulty::Medium,
        summary: "un câble perd une grande partie des trames",
        build: packet_loss,
    },
    Template {
        id: "high-latency",
        kind: CauseKind::HighLatency,
        difficulty: Difficulty::Medium,
        summary: "un lien est congestionné (latence très élevée)",
        build: high_latency,
    },
    Template {
        id: "wrong-vlan",
        kind: CauseKind::WrongVlan,
        difficulty: Difficulty::Medium,
        summary: "un port de switch est dans le mauvais VLAN",
        build: wrong_vlan,
    },
    Template {
        id: "static-arp",
        kind: CauseKind::ArpMismatch,
        difficulty: Difficulty::Hard,
        summary: "une entrée ARP statique pointe vers la mauvaise MAC",
        build: static_arp,
    },
    Template {
        id: "wrong-ip",
        kind: CauseKind::WrongIp,
        difficulty: Difficulty::Easy,
        summary: "la machine source a une adresse dans un autre réseau",
        build: wrong_ip,
    },
    Template {
        id: "wrong-mask",
        kind: CauseKind::WrongMask,
        difficulty: Difficulty::Medium,
        summary: "le masque de la machine source est faux",
        build: wrong_mask,
    },
    Template {
        id: "wrong-gateway",
        kind: CauseKind::WrongGateway,
        difficulty: Difficulty::Easy,
        summary: "la passerelle par défaut ne correspond à aucune machine",
        build: wrong_gateway,
    },
    Template {
        id: "no-gateway",
        kind: CauseKind::MissingRoute,
        difficulty: Difficulty::Easy,
        summary: "la machine source n'a plus de passerelle par défaut",
        build: no_gateway,
    },
    Template {
        id: "missing-route",
        kind: CauseKind::MissingRoute,
        difficulty: Difficulty::Medium,
        summary: "un routeur a perdu une route statique du chemin",
        build: missing_route,
    },
    Template {
        id: "wrong-route",
        kind: CauseKind::WrongRoute,
        difficulty: Difficulty::Hard,
        summary: "une route statique pointe vers un prochain saut inexistant",
        build: wrong_route,
    },
    Template {
        id: "routing-loop",
        kind: CauseKind::RoutingLoop,
        difficulty: Difficulty::Hard,
        summary: "deux routeurs se renvoient les paquets",
        build: routing_loop,
    },
    Template {
        id: "forwarding-off",
        kind: CauseKind::ForwardingDisabled,
        difficulty: Difficulty::Medium,
        summary: "un routeur ne transfère plus les paquets",
        build: forwarding_off,
    },
    Template {
        id: "service-down",
        kind: CauseKind::ServiceDown,
        difficulty: Difficulty::Easy,
        summary: "le service visé est arrêté sur le serveur",
        build: service_down,
    },
    Template {
        id: "acl-deny",
        kind: CauseKind::Filtered,
        difficulty: Difficulty::Medium,
        summary: "une règle de filtrage bloque le flux",
        build: acl_deny,
    },
    Template {
        id: "acl-default-deny",
        kind: CauseKind::Filtered,
        difficulty: Difficulty::Hard,
        summary: "un équipement refuse tout par défaut",
        build: acl_default_deny,
    },
];

// ---------------------------------------------------------------- utilitaires

fn pick<T: Copy>(rng: &mut SimRng, items: &[T]) -> Option<T> {
    if items.is_empty() {
        None
    } else {
        Some(items[(rng.next_u64() % items.len() as u64) as usize])
    }
}

fn ip_node(sim: &Simulation, d: DeviceId) -> Option<&netiv_core::ip_node::IpNode> {
    match &sim.device(d)?.behavior {
        Behavior::Ip(n) => Some(n),
        Behavior::Switch(_) => None,
    }
}

/// Une adresse libre (que personne ne possède) dans `net`, en partant du haut.
fn unused_in(sim: &Simulation, net: Ipv4Net) -> Option<Ipv4Addr> {
    let base = u32::from(net.network());
    let top = u32::from(net.broadcast());
    (base + 1..top)
        .rev()
        .map(Ipv4Addr::from)
        .find(|ip| owner_of(sim, *ip).is_none())
}

/// Premier réseau de `d` qui contient `ip`.
fn net_containing(sim: &Simulation, d: DeviceId, ip: Ipv4Addr) -> Option<Ipv4Net> {
    sim.device(d)?
        .ifaces
        .iter()
        .filter_map(|i| i.ip)
        .find(|n| n.contains(ip))
}

// ---------------------------------------------------------------- couche 1

fn link_down(_: &Simulation, p: &PathInfo, rng: &mut SimRng) -> Option<Fault> {
    pick(rng, &p.links).map(|link| Fault::LinkDown { link })
}

fn interface_down(_: &Simulation, p: &PathInfo, rng: &mut SimRng) -> Option<Fault> {
    pick(rng, &p.ifaces).map(|(device, iface)| Fault::InterfaceDown { device, iface })
}

fn packet_loss(_: &Simulation, p: &PathInfo, rng: &mut SimRng) -> Option<Fault> {
    pick(rng, &p.links).map(|link| Fault::LinkLoss { link, loss: 0.6 })
}

fn high_latency(_: &Simulation, p: &PathInfo, rng: &mut SimRng) -> Option<Fault> {
    let link: LinkId = pick(rng, &p.links)?;
    Some(Fault::LinkLatency {
        link,
        latency: SimTime::from_millis(300),
    })
}

// ---------------------------------------------------------------- couche 2

fn wrong_vlan(sim: &Simulation, p: &PathInfo, rng: &mut SimRng) -> Option<Fault> {
    let access: Vec<_> = p
        .switch_ports
        .iter()
        .copied()
        .filter(|(d, i)| {
            matches!(
                &sim.device(*d).map(|x| &x.behavior),
                Some(Behavior::Switch(sw)) if matches!(sw.port_mode(*i), PortMode::Access { .. })
            )
        })
        .collect();
    let (device, iface) = pick(rng, &access)?;
    let Some(Behavior::Switch(sw)) = sim.device(device).map(|d| &d.behavior) else {
        return None;
    };
    let PortMode::Access { vlan } = sw.port_mode(iface) else {
        return None;
    };
    let wrong = if vlan == 99 { 98 } else { 99 };
    Some(Fault::PortMode {
        device,
        iface,
        mode: PortMode::Access { vlan: wrong },
    })
}

fn static_arp(_: &Simulation, p: &PathInfo, rng: &mut SimRng) -> Option<Fault> {
    let (device, ip) = pick(rng, &p.arp)?;
    Some(Fault::StaticArp {
        device,
        ip,
        mac: MacAddr([0x02, 0x00, 0x00, 0x00, 0xBA, 0xD0]),
    })
}

// ---------------------------------------------------------------- couche 3

/// Interface de la source qui sert au flux.
fn src_iface(
    sim: &Simulation,
    p: &PathInfo,
    src: DeviceId,
) -> Option<(netiv_core::IfaceId, Ipv4Net)> {
    let (_, iface) = p.ifaces.iter().find(|(d, _)| *d == src)?;
    let net = sim.device(src)?.iface(*iface)?.ip?;
    Some((*iface, net))
}

fn wrong_ip(sim: &Simulation, p: &PathInfo, rng: &mut SimRng) -> Option<Fault> {
    let src = flow_src(p)?;
    let (iface, net) = src_iface(sim, p, src)?;
    // Change de réseau en inversant un bit du réseau (reste un réseau valide
    // de même taille) ; l'hôte garde son numéro.
    let bit = 8 + (rng.next_u64() % 8) as u32;
    let flipped = u32::from(net.addr()) ^ (1 << bit.max(32 - u32::from(net.prefix()) + 1));
    let new = Ipv4Addr::from(flipped);
    if owner_of(sim, new).is_some() {
        return None;
    }
    let new_net = Ipv4Net::new(new, net.prefix()).ok()?;
    if new_net.trunc() == net.trunc() {
        return None;
    }
    Some(Fault::SetIp {
        device: src,
        iface,
        ip: Some(new_net),
    })
}

fn wrong_mask(sim: &Simulation, p: &PathInfo, rng: &mut SimRng) -> Option<Fault> {
    let src = flow_src(p)?;
    let (iface, net) = src_iface(sim, p, src)?;
    let node = ip_node(sim, src)?;
    let dst = dst_ip(sim, p)?;
    if net.contains(dst) {
        return None; // destination locale : le masque ne change rien
    }
    // Masque trop large : on cherche un préfixe qui « avale » la destination.
    let too_big = (8..net.prefix())
        .rev()
        .filter_map(|pr| Ipv4Net::new(net.addr(), pr).ok())
        .find(|n| n.contains(dst));
    // Masque trop étroit : la passerelle sort du réseau.
    let too_small = node.gateway.and_then(|gw| {
        Ipv4Net::new(net.addr(), 30)
            .ok()
            .filter(|n| !n.contains(gw))
    });
    let choices: Vec<Ipv4Net> = [too_big, too_small].into_iter().flatten().collect();
    let new = pick(rng, &choices)?;
    Some(Fault::SetIp {
        device: src,
        iface,
        ip: Some(new),
    })
}

fn wrong_gateway(sim: &Simulation, p: &PathInfo, rng: &mut SimRng) -> Option<Fault> {
    let _ = rng;
    let src = flow_src(p)?;
    let gw = ip_node(sim, src)?.gateway?;
    let net = net_containing(sim, src, gw)?;
    let bad = unused_in(sim, net)?;
    Some(Fault::SetGateway {
        device: src,
        gateway: Some(bad),
    })
}

fn no_gateway(sim: &Simulation, p: &PathInfo, _: &mut SimRng) -> Option<Fault> {
    let src = flow_src(p)?;
    ip_node(sim, src)?.gateway?;
    Some(Fault::SetGateway {
        device: src,
        gateway: None,
    })
}

/// Routes statiques des routeurs du chemin qui servent à l'aller ou au retour.
fn path_routes(sim: &Simulation, p: &PathInfo) -> Vec<(DeviceId, StaticRoute)> {
    let dst = dst_ip(sim, p);
    let src_ips: Vec<Ipv4Addr> = flow_src(p)
        .and_then(|s| sim.device(s))
        .map(|d| {
            d.ifaces
                .iter()
                .filter_map(|i| i.ip.map(|n| n.addr()))
                .collect()
        })
        .unwrap_or_default();
    let mut out = Vec::new();
    for r in &p.routers {
        let Some(node) = ip_node(sim, *r) else {
            continue;
        };
        for route in &node.static_routes {
            let on_path = dst.is_some_and(|d| route.dest.contains(d))
                || src_ips.iter().any(|ip| route.dest.contains(*ip));
            if on_path {
                out.push((*r, *route));
            }
        }
    }
    out
}

fn missing_route(sim: &Simulation, p: &PathInfo, rng: &mut SimRng) -> Option<Fault> {
    let (device, route) = pick(rng, &path_routes(sim, p))?;
    Some(Fault::RemoveRoutes {
        device,
        dest: route.dest,
    })
}

fn wrong_route(sim: &Simulation, p: &PathInfo, rng: &mut SimRng) -> Option<Fault> {
    let (device, route) = pick(rng, &path_routes(sim, p))?;
    // Une adresse libre dans le même réseau (ARP sans réponse)... ou, sur un
    // lien point-à-point sans adresse libre, une adresse hors de tout réseau
    // connecté (route inutilisable).
    let bad = net_containing(sim, device, route.via)
        .and_then(|net| unused_in(sim, net))
        .or_else(|| {
            let far = Ipv4Addr::new(172, 31, 255, 254);
            owner_of(sim, far).is_none().then_some(far)
        })?;
    Some(Fault::Multi(vec![
        Fault::RemoveRoutes {
            device,
            dest: route.dest,
        },
        Fault::AddRoute {
            device,
            route: StaticRoute {
                dest: route.dest,
                via: bad,
            },
        },
    ]))
}

fn routing_loop(sim: &Simulation, p: &PathInfo, _: &mut SimRng) -> Option<Fault> {
    let dst = dst_ip(sim, p)?;
    // Deux routeurs consécutifs, reliés directement : le dernier renvoie vers
    // le précédent, avec une route plus spécifique (/32) que la route connectée.
    p.routers.windows(2).find_map(|pair| {
        let (prev, last) = (pair[0], pair[1]);
        let link = sim
            .links()
            .iter()
            .find(|l| (l.a.0 == prev && l.b.0 == last) || (l.a.0 == last && l.b.0 == prev))?;
        let prev_end = if link.a.0 == prev { link.a } else { link.b };
        let via = sim.device(prev)?.iface(prev_end.1)?.ip?.addr();
        let host = Ipv4Net::new(dst, 32).ok()?;
        Some(Fault::AddRoute {
            device: last,
            route: StaticRoute { dest: host, via },
        })
    })
}

fn forwarding_off(_: &Simulation, p: &PathInfo, rng: &mut SimRng) -> Option<Fault> {
    pick(rng, &p.routers).map(|device| Fault::ForwardingOff { device })
}

// ---------------------------------------------------------------- services, filtrage

fn service_down(sim: &Simulation, p: &PathInfo, _: &mut SimRng) -> Option<Fault> {
    let (proto, port) = flow_service(p, sim)?;
    let device = p.dst_device?;
    Some(Fault::StopService {
        device,
        proto,
        port,
    })
}

fn acl_deny(sim: &Simulation, p: &PathInfo, rng: &mut SimRng) -> Option<Fault> {
    let mut targets = p.routers.clone();
    targets.extend(p.dst_device);
    let device = pick(rng, &targets)?;
    let rule = match flow_service(p, sim) {
        None => AclRule::new(AclAction::Deny, RuleProto::Icmp),
        Some((proto, port)) => {
            let mut r = AclRule::new(
                AclAction::Deny,
                match proto {
                    L4Proto::Tcp => RuleProto::Tcp,
                    L4Proto::Udp => RuleProto::Udp,
                },
            );
            r.port_from = Some(port);
            r.port_to = Some(port);
            r
        }
    };
    Some(Fault::AclInsert { device, rule })
}

fn acl_default_deny(sim: &Simulation, p: &PathInfo, rng: &mut SimRng) -> Option<Fault> {
    let open: Vec<DeviceId> = p
        .routers
        .iter()
        .copied()
        .filter(|d| ip_node(sim, *d).is_some_and(|n| n.acl.default == AclAction::Allow))
        .collect();
    pick(rng, &open).map(|device| Fault::AclDefault {
        device,
        action: AclAction::Deny,
    })
}

// ---------------------------------------------------------------- flux

fn flow_src(p: &PathInfo) -> Option<DeviceId> {
    // Le premier équipement IP du chemin relevé est la source du flux.
    p.ifaces.first().map(|(d, _)| *d)
}

fn dst_ip(sim: &Simulation, p: &PathInfo) -> Option<Ipv4Addr> {
    let d = p.dst_device?;
    // Adresse de la dernière interface vue sur la destination.
    p.ifaces
        .iter()
        .rev()
        .find(|(x, _)| *x == d)
        .and_then(|(x, i)| sim.device(*x)?.iface(*i)?.ip)
        .map(|n| n.addr())
}

fn flow_service(p: &PathInfo, _sim: &Simulation) -> Option<(L4Proto, u16)> {
    p.service
}

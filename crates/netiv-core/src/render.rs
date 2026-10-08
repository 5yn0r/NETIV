//! Rendu texte de la trace et de l'état du réseau.
//!
//! Ces fonctions ne font que *lire* la simulation : Packet Journey, Packet
//! Microscope, Explain Engine et tables d'état sont des vues sur la trace.

use std::fmt::Write as _;

use crate::device::{Behavior, DeviceKind};
use crate::error::NetivError;
use crate::ids::{DeviceId, IfaceId, TraceId};
use crate::packet::{ArpOp, EtherPayload, Frame, IcmpMessage, IpPayload, UnreachCode};
use crate::route::RouteDecision;
use crate::sim::Simulation;
use crate::switch::PortMode;
use crate::trace::{TraceEvent, TraceKind};
use std::collections::BTreeMap;

/// Nom d'un équipement (`?` s'il n'existe pas).
pub fn device_name(sim: &Simulation, id: DeviceId) -> &str {
    sim.device(id).map_or("?", |d| d.name.as_str())
}

/// Nom d'une interface (`?` si elle n'existe pas).
pub fn iface_name(sim: &Simulation, dev: DeviceId, iface: IfaceId) -> &str {
    sim.device(dev)
        .and_then(|d| d.iface(iface))
        .map_or("?", |i| i.name.as_str())
}

fn iface_list(sim: &Simulation, dev: DeviceId, ifaces: &[IfaceId]) -> String {
    if ifaces.is_empty() {
        return "aucun port".to_string();
    }
    ifaces
        .iter()
        .map(|i| iface_name(sim, dev, *i))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Description lisible d'un événement (sans date ni équipement).
pub fn describe(sim: &Simulation, ev: &TraceEvent) -> String {
    let dev = ev.device;
    match &ev.kind {
        TraceKind::CommandIssued { description } => format!("commande : {description}"),
        TraceKind::ConfigChanged { description } => format!("configuration : {description}"),
        TraceKind::FrameSent { iface, frame } => {
            format!("envoie sur {} : {frame}", iface_name(sim, dev, *iface))
        }
        TraceKind::FrameReceived { iface, frame } => {
            format!("reçoit sur {} : {frame}", iface_name(sim, dev, *iface))
        }
        TraceKind::Dropped { reason } => format!("abandonne : {reason}"),
        TraceKind::MacLearned {
            mac,
            vlan,
            iface,
            previous,
        } => {
            let name = iface_name(sim, dev, *iface);
            let v = if *vlan == 1 {
                String::new()
            } else {
                format!(" (VLAN {vlan})")
            };
            match previous {
                Some(old) => format!("déplace {mac}{v} : {} → {name}", iface_name(sim, dev, *old)),
                None => format!("apprend {mac}{v} sur {name}"),
            }
        }
        TraceKind::Flood { reason, out_ifaces } => format!(
            "diffuse sur tous les ports ({reason}) → {}",
            iface_list(sim, dev, out_ifaces)
        ),
        TraceKind::Forward { out_iface } => {
            format!("transmet vers {}", iface_name(sim, dev, *out_iface))
        }
        TraceKind::ArpRequestSent { target, attempt } => {
            format!("ARP : cherche la MAC de {target} (tentative {attempt})")
        }
        TraceKind::ArpReplySent { to } => format!("ARP : répond à {to}"),
        TraceKind::ArpCacheUpdated { ip, mac } => format!("cache ARP : {ip} → {mac}"),
        TraceKind::ArpTimeout { target, dropped } => {
            format!("ARP : aucune réponse pour {target}, {dropped} paquet(s) abandonné(s)")
        }
        TraceKind::RouteLookup(d) => describe_lookup(sim, dev, d),
        TraceKind::TtlDecremented { from, to } => format!("TTL {from} → {to}"),
        TraceKind::TtlExpired => "TTL expiré : le paquet est détruit".to_string(),
        TraceKind::PingSent { dst, seq } => format!("ping vers {dst} (seq {seq})"),
        TraceKind::PingReply {
            from,
            seq,
            ttl,
            rtt,
        } => format!("réponse de {from} (seq {seq}, ttl {ttl}, temps {rtt})"),
        TraceKind::PingError { from, seq, error } => {
            format!("erreur de {from} (seq {seq}) : {error}")
        }
        TraceKind::PingTimeout { dst, seq } => {
            format!("pas de réponse de {dst} (seq {seq}) : délai dépassé")
        }
        TraceKind::CurlStarted { dst, port } => format!("curl {dst}:{port}"),
        TraceKind::CurlResult { dst, port, outcome } => format!("curl {dst}:{port} → {outcome}"),
        TraceKind::TcpEstablished {
            peer,
            peer_port,
            local_port,
        } => format!("connexion TCP établie avec {peer}:{peer_port} (port local {local_port})"),
        TraceKind::TcpResetSent { to, port } => {
            format!("envoie RST à {to} (rien n'écoute sur le port {port})")
        }
        TraceKind::ServiceRequest {
            from,
            port,
            service,
        } => format!("service {service} (port {port}) : requête de {from}"),
        TraceKind::UdpSent { dst, port } => format!("envoie UDP vers {dst}:{port}"),
        TraceKind::UdpResult { dst, port, outcome } => {
            format!("UDP {dst}:{port} → {outcome}")
        }
        TraceKind::AclDecision { text, .. } => format!("filtrage : {text}"),
        TraceKind::FaultApplied { description } => format!("PANNE : {description}"),
        TraceKind::FaultRepaired { description } => format!("réparé : {description}"),
    }
}

fn describe_lookup(sim: &Simulation, dev: DeviceId, d: &RouteDecision) -> String {
    match d.chosen_route() {
        Some(r) => {
            let via = r
                .next_hop
                .map_or_else(|| "directement".to_string(), |h| format!("via {h}"));
            format!(
                "route pour {} : {} {via} par {}",
                d.dst,
                r.dest,
                iface_name(sim, dev, r.iface)
            )
        }
        None => format!("route pour {} : aucune", d.dst),
    }
}

/// Ligne complète : numéro, date, équipement, description.
pub fn event_line(sim: &Simulation, ev: &TraceEvent) -> String {
    format!(
        "[{:>4}] {:>10}  {:<8} {}",
        ev.seq,
        ev.time.to_string(),
        device_name(sim, ev.device),
        describe(sim, ev)
    )
}

// ---------------------------------------------------------------------------
// Packet Journey
// ---------------------------------------------------------------------------

/// Identité d'un message IP indépendante du saut (le TTL et les MAC changent
/// à chaque routeur) : `(étiquette affichée, clé d'identité)`.
fn message_identity(frame: &Frame) -> Option<(String, String)> {
    let EtherPayload::Ipv4(p) = &frame.payload else {
        return None;
    };
    Some(match &p.payload {
        IpPayload::Icmp(msg) => {
            let (label, extra) = match msg {
                IcmpMessage::EchoRequest { id, seq } => ("Echo Request", format!("{id}/{seq}")),
                IcmpMessage::EchoReply { id, seq } => ("Echo Reply", format!("{id}/{seq}")),
                IcmpMessage::DestUnreachable { original, .. } => (
                    "Destination Unreachable",
                    format!("{}>{}", original.src, original.dst),
                ),
                IcmpMessage::TimeExceeded { original } => (
                    "Time Exceeded",
                    format!("{}>{}", original.src, original.dst),
                ),
            };
            (
                label.to_string(),
                format!("{label}|{}|{}|{extra}", p.src, p.dst),
            )
        }
        IpPayload::Udp(u) => (
            "UDP".to_string(),
            format!(
                "UDP|{}:{}>{}:{}|{}",
                p.src, u.src_port, p.dst, u.dst_port, u.data
            ),
        ),
        IpPayload::Tcp(t) => (
            format!("TCP [{}]", t.flags),
            format!(
                "TCP|{}:{}>{}:{}|{}|{}|{}",
                p.src, t.src_port, p.dst, t.dst_port, t.seq, t.ack, t.flags
            ),
        ),
    })
}

/// Chemin suivi par chaque message IP d'une transaction : l'émetteur, puis
/// chaque équipement qui reçoit ce message, dans l'ordre. Un message ré-émis
/// par son émetteur d'origine (retransmission) ouvre un nouveau chemin.
fn paths(sim: &Simulation, events: &[&TraceEvent]) -> Vec<(String, String)> {
    // identifiant de groupe -> (étiquette, nœuds)
    let mut order: Vec<String> = Vec::new();
    let mut groups: BTreeMap<String, (String, Vec<String>)> = BTreeMap::new();
    // clé de message -> groupe courant
    let mut current: BTreeMap<String, String> = BTreeMap::new();
    let mut attempts: BTreeMap<String, usize> = BTreeMap::new();

    for e in events {
        match &e.kind {
            TraceKind::FrameSent { frame, .. } => {
                let Some((label, key)) = message_identity(frame) else {
                    continue;
                };
                let sender = device_name(sim, e.device).to_string();
                let restart = current
                    .get(&key)
                    .and_then(|g| groups.get(g))
                    .is_some_and(|(_, nodes)| nodes.len() > 1 && nodes.first() == Some(&sender));
                if restart || !current.contains_key(&key) {
                    let n = attempts.entry(key.clone()).or_insert(0);
                    *n += 1;
                    let gid = format!("{key}#{n}");
                    let label = if *n > 1 {
                        format!("{label} (retransmission {n})")
                    } else {
                        label
                    };
                    order.push(gid.clone());
                    groups.insert(gid.clone(), (label, vec![sender]));
                    current.insert(key, gid);
                }
            }
            TraceKind::FrameReceived { frame, .. } => {
                let Some((label, key)) = message_identity(frame) else {
                    continue;
                };
                let gid = current.entry(key.clone()).or_insert_with(|| {
                    let gid = format!("{key}#1");
                    order.push(gid.clone());
                    groups.insert(gid.clone(), (label, Vec::new()));
                    gid
                });
                let name = device_name(sim, e.device);
                let is_switch = sim
                    .device(e.device)
                    .is_some_and(|d| d.kind == DeviceKind::Switch);
                let node = match (&frame.payload, is_switch) {
                    (EtherPayload::Ipv4(p), false) => format!("{name} (ttl {})", p.ttl),
                    _ => name.to_string(),
                };
                if let Some((_, nodes)) = groups.get_mut(gid.as_str()) {
                    nodes.push(node);
                }
            }
            _ => {}
        }
    }

    order
        .into_iter()
        .filter_map(|gid| groups.remove(&gid))
        .map(|(label, mut nodes)| {
            // Chemins très longs (boucles de routage) : on abrège le milieu.
            if nodes.len() > 12 {
                let hidden = nodes.len() - 10;
                let tail = nodes.split_off(nodes.len() - 4);
                nodes.truncate(6);
                nodes.push(format!("… {hidden} sauts …"));
                nodes.extend(tail);
            }
            (label, nodes.join(" → "))
        })
        .collect()
}

/// Parcours complet d'une transaction (ex. un `ping`) : tous les événements
/// dans l'ordre, puis le chemin suivi par chaque message.
pub fn journey(sim: &Simulation, id: TraceId) -> Result<String, NetivError> {
    let events: Vec<&TraceEvent> = sim.trace().by_trace(id).collect();
    if events.is_empty() {
        return Err(NetivError::NoSuchTrace(id.0));
    }
    let mut s = String::new();
    let title = events
        .iter()
        .find_map(|e| match &e.kind {
            TraceKind::PingSent { dst, seq } => Some(format!(
                "ping {} → {dst} (seq {seq})",
                device_name(sim, e.device)
            )),
            TraceKind::CurlStarted { dst, port } => Some(format!(
                "curl {} → {dst}:{port}",
                device_name(sim, e.device)
            )),
            TraceKind::UdpSent { dst, port } => {
                Some(format!("udp {} → {dst}:{port}", device_name(sim, e.device)))
            }
            _ => None,
        })
        .unwrap_or_else(|| "transaction".to_string());
    let _ = writeln!(s, "PARCOURS {id} — {title}");
    let _ = writeln!(s);
    for e in &events {
        let _ = writeln!(s, "{}", event_line(sim, e));
    }
    let paths = paths(sim, &events);
    if !paths.is_empty() {
        let _ = writeln!(s);
        for (label, path) in paths {
            let _ = writeln!(s, "{label:<24} {path}");
        }
    }
    Ok(s)
}

// ---------------------------------------------------------------------------
// Packet Microscope
// ---------------------------------------------------------------------------

/// Détail couche par couche de la trame associée à l'événement `seq`
/// (un événement `FrameSent` ou `FrameReceived`).
pub fn inspect(sim: &Simulation, seq: u64) -> Result<String, NetivError> {
    let ev = sim.trace().get(seq).ok_or(NetivError::NoSuchEvent(seq))?;
    let (verb, iface, frame) = match &ev.kind {
        TraceKind::FrameSent { iface, frame } => ("émise", *iface, frame),
        TraceKind::FrameReceived { iface, frame } => ("reçue", *iface, frame),
        _ => {
            return Err(NetivError::WrongEventKind {
                seq,
                expected: "une trame émise ou reçue",
            });
        }
    };
    let mut s = String::new();
    let _ = writeln!(
        s,
        "PAQUET #{seq} — trame {verb} par {}/{} à {}",
        device_name(sim, ev.device),
        iface_name(sim, ev.device, iface),
        ev.time
    );
    let _ = writeln!(s, "Transaction : {}", frame.trace_id);
    let _ = writeln!(s);
    let _ = writeln!(s, "Ethernet");
    let _ = writeln!(s, "  Source      : {}", frame.src);
    let _ = writeln!(s, "  Destination : {}", frame.dst);
    if let Some(v) = frame.vlan {
        let _ = writeln!(s, "  VLAN (802.1Q): {v}");
    }
    match &frame.payload {
        EtherPayload::Arp(a) => {
            let _ = writeln!(s);
            let _ = writeln!(s, "ARP");
            let _ = writeln!(
                s,
                "  Opération   : {}",
                match a.op {
                    ArpOp::Request => "Request",
                    ArpOp::Reply => "Reply",
                }
            );
            let _ = writeln!(s, "  MAC émetteur: {}", a.sender_mac);
            let _ = writeln!(s, "  IP émetteur : {}", a.sender_ip);
            let _ = writeln!(s, "  MAC cible   : {}", a.target_mac);
            let _ = writeln!(s, "  IP cible    : {}", a.target_ip);
        }
        EtherPayload::Ipv4(p) => {
            let _ = writeln!(s);
            let _ = writeln!(s, "IPv4");
            let _ = writeln!(s, "  Source      : {}", p.src);
            let _ = writeln!(s, "  Destination : {}", p.dst);
            let _ = writeln!(s, "  TTL         : {}", p.ttl);
            let proto = match &p.payload {
                IpPayload::Icmp(_) => "ICMP",
                IpPayload::Udp(_) => "UDP",
                IpPayload::Tcp(_) => "TCP",
            };
            let _ = writeln!(s, "  Protocole   : {proto}");
            let _ = writeln!(s);
            let _ = writeln!(s, "{proto}");
            match &p.payload {
                IpPayload::Icmp(msg) => match msg {
                    IcmpMessage::EchoRequest { id, seq } | IcmpMessage::EchoReply { id, seq } => {
                        let kind = if matches!(msg, IcmpMessage::EchoRequest { .. }) {
                            "Echo Request"
                        } else {
                            "Echo Reply"
                        };
                        let _ = writeln!(s, "  Type        : {kind}");
                        let _ = writeln!(s, "  Identifiant : {id}");
                        let _ = writeln!(s, "  Séquence    : {seq}");
                    }
                    IcmpMessage::DestUnreachable { code, original } => {
                        let _ = writeln!(s, "  Type        : Destination Unreachable");
                        let _ = writeln!(
                            s,
                            "  Code        : {}",
                            match code {
                                UnreachCode::Net => "Network Unreachable",
                                UnreachCode::Port => "Port Unreachable",
                            }
                        );
                        let _ =
                            writeln!(s, "  Paquet fautif : {} → {}", original.src, original.dst);
                    }
                    IcmpMessage::TimeExceeded { original } => {
                        let _ = writeln!(s, "  Type        : Time Exceeded (TTL)");
                        let _ =
                            writeln!(s, "  Paquet fautif : {} → {}", original.src, original.dst);
                    }
                },
                IpPayload::Udp(u) => {
                    let _ = writeln!(s, "  Port source : {}", u.src_port);
                    let _ = writeln!(s, "  Port dest.  : {}", u.dst_port);
                    let _ = writeln!(s, "  Données     : {}", u.data);
                }
                IpPayload::Tcp(t) => {
                    let _ = writeln!(s, "  Port source : {}", t.src_port);
                    let _ = writeln!(s, "  Port dest.  : {}", t.dst_port);
                    let _ = writeln!(s, "  Drapeaux    : {}", t.flags);
                    let _ = writeln!(s, "  Séquence    : {}", t.seq);
                    let _ = writeln!(s, "  Acquitt.    : {}", t.ack);
                    if !t.data.is_empty() {
                        let _ = writeln!(s, "  Données     : {}", t.data);
                    }
                }
            }
        }
    }
    Ok(s)
}

// ---------------------------------------------------------------------------
// Explain Engine
// ---------------------------------------------------------------------------

/// Explique une décision de routage (événement `RouteLookup`).
pub fn explain_route(sim: &Simulation, seq: u64) -> Result<String, NetivError> {
    let ev = sim.trace().get(seq).ok_or(NetivError::NoSuchEvent(seq))?;
    let TraceKind::RouteLookup(d) = &ev.kind else {
        return Err(NetivError::WrongEventKind {
            seq,
            expected: "une décision de routage",
        });
    };
    let dev = ev.device;
    let mut s = String::new();
    let _ = writeln!(
        s,
        "POURQUOI {} choisit cette interface ? (événement #{seq})",
        device_name(sim, dev)
    );
    let _ = writeln!(s);
    let _ = writeln!(s, "Destination : {}", d.dst);
    let _ = writeln!(s);
    let _ = writeln!(s, "Routes examinées :");
    for (i, c) in d.candidates.iter().enumerate() {
        let mark = if !c.matches {
            "✗"
        } else if d.chosen == Some(i) {
            "✓"
        } else {
            "~"
        };
        let via = c
            .route
            .next_hop
            .map_or_else(|| "directe".to_string(), |h| format!("via {h}"));
        let _ = writeln!(
            s,
            "  {mark} {:<18} {:<10} {:<16} → {}",
            c.route.dest.to_string(),
            c.route.origin.to_string(),
            via,
            iface_name(sim, dev, c.route.iface)
        );
    }
    let _ = writeln!(
        s,
        "  (✓ retenue, ~ correspond mais moins précise, ✗ ne correspond pas)"
    );
    for r in &d.inactive {
        let _ = writeln!(s);
        let _ = writeln!(
            s,
            "Route inutilisable : {} via {} ({}) — le prochain saut n'est sur aucun réseau connecté",
            r.dest, r.via, r.origin
        );
    }
    let _ = writeln!(s);
    let matching: Vec<_> = d.candidates.iter().filter(|c| c.matches).collect();
    match d.chosen_route() {
        Some(r) => {
            if matching.len() > 1 {
                let prefixes: Vec<String> = matching
                    .iter()
                    .map(|c| format!("/{}", c.route.dest.prefix()))
                    .collect();
                let _ = writeln!(
                    s,
                    "Longest Prefix Match : {} routes correspondent ({}), la plus précise est {}.",
                    matching.len(),
                    prefixes.join(", "),
                    r.dest
                );
            } else {
                let _ = writeln!(s, "Une seule route correspond : {}.", r.dest);
            }
            let via = r
                .next_hop
                .map_or_else(|| format!("{} directement", d.dst), |h| format!("via {h}"));
            let _ = writeln!(s, "Décision : → {} {via}", iface_name(sim, dev, r.iface));
        }
        None => {
            let _ = writeln!(s, "Aucune route ne correspond : le paquet est abandonné.");
        }
    }
    Ok(s)
}

// ---------------------------------------------------------------------------
// Tables d'état
// ---------------------------------------------------------------------------

/// Liste des équipements.
pub fn devices_table(sim: &Simulation) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "{:<10} {:<8} INTERFACES", "NOM", "TYPE");
    for d in sim.devices() {
        let ifaces: Vec<String> = d
            .ifaces
            .iter()
            .map(|i| match i.ip {
                Some(ip) => format!("{} {ip}", i.name),
                None => i.name.clone(),
            })
            .collect();
        let _ = writeln!(s, "{:<10} {:<8} {}", d.name, d.kind, ifaces.join(", "));
    }
    s
}

/// Interfaces d'un équipement.
pub fn show_interfaces(sim: &Simulation, dev: DeviceId) -> Result<String, NetivError> {
    let d = sim
        .device(dev)
        .ok_or_else(|| NetivError::UnknownDevice(dev.to_string()))?;
    let mut s = String::new();
    let _ = writeln!(
        s,
        "{:<8} {:<19} {:<18} {:<8} CÂBLE",
        "NOM", "MAC", "IP", "ÉTAT"
    );
    for i in &d.ifaces {
        let ip = i.ip.map_or_else(|| "-".to_string(), |n| n.to_string());
        let state = if i.admin_up { "up" } else { "down" };
        let cable = match i.link.and_then(|l| sim.link(l)) {
            None => "non branché".to_string(),
            Some(l) => {
                let (pd, pi) = l.peer_of(dev, i.id).unwrap_or(l.a);
                let up = if l.up { "" } else { " (COUPÉ)" };
                format!("{}/{}{up}", device_name(sim, pd), iface_name(sim, pd, pi))
            }
        };
        let _ = writeln!(
            s,
            "{:<8} {:<19} {:<18} {:<8} {cable}",
            i.name, i.mac, ip, state
        );
    }
    Ok(s)
}

/// Cache ARP d'un équipement IP.
pub fn show_arp(sim: &Simulation, dev: DeviceId) -> Result<String, NetivError> {
    let d = sim
        .device(dev)
        .ok_or_else(|| NetivError::UnknownDevice(dev.to_string()))?;
    let Behavior::Ip(node) = &d.behavior else {
        return Err(NetivError::NotAnIpDevice(d.name.clone()));
    };
    let mut s = String::new();
    let _ = writeln!(s, "{:<16} MAC", "IP");
    if node.arp_cache.is_empty() {
        let _ = writeln!(s, "(cache ARP vide)");
    }
    for (ip, mac) in &node.arp_cache {
        let _ = writeln!(s, "{:<16} {mac}", ip.to_string());
    }
    Ok(s)
}

/// Table de routage d'un équipement IP.
pub fn show_routes(sim: &Simulation, dev: DeviceId) -> Result<String, NetivError> {
    let d = sim
        .device(dev)
        .ok_or_else(|| NetivError::UnknownDevice(dev.to_string()))?;
    let Behavior::Ip(node) = &d.behavior else {
        return Err(NetivError::NotAnIpDevice(d.name.clone()));
    };
    let (active, inactive) = node.active_routes(&d.ifaces);
    let mut s = String::new();
    let _ = writeln!(
        s,
        "{:<18} {:<16} {:<8} ORIGINE",
        "DESTINATION", "PROCHAIN SAUT", "IFACE"
    );
    for r in &active {
        let via = r
            .next_hop
            .map_or_else(|| "-".to_string(), |h| h.to_string());
        let _ = writeln!(
            s,
            "{:<18} {:<16} {:<8} {}",
            r.dest.to_string(),
            via,
            iface_name(sim, dev, r.iface),
            r.origin
        );
    }
    for r in &inactive {
        let _ = writeln!(
            s,
            "{:<18} {:<16} {:<8} {} — INUTILISABLE : prochain saut hors de tout réseau connecté",
            r.dest.to_string(),
            r.via.to_string(),
            "-",
            r.origin
        );
    }
    Ok(s)
}

/// Table d'adresses MAC d'un switch.
pub fn show_mac_table(sim: &Simulation, dev: DeviceId) -> Result<String, NetivError> {
    let d = sim
        .device(dev)
        .ok_or_else(|| NetivError::UnknownDevice(dev.to_string()))?;
    let Behavior::Switch(sw) = &d.behavior else {
        return Err(NetivError::WrongEventKind {
            seq: 0,
            expected: "un switch",
        });
    };
    let mut s = String::new();
    let _ = writeln!(s, "{:<6} {:<19} PORT", "VLAN", "MAC");
    if sw.mac_table.is_empty() {
        let _ = writeln!(s, "(table vide)");
    }
    for ((vlan, mac), port) in &sw.mac_table {
        let _ = writeln!(s, "{:<6} {:<19} {}", vlan, mac, iface_name(sim, dev, *port));
    }
    Ok(s)
}

/// Mode des ports d'un switch (access / trunk).
pub fn show_vlans(sim: &Simulation, dev: DeviceId) -> Result<String, NetivError> {
    let d = sim
        .device(dev)
        .ok_or_else(|| NetivError::UnknownDevice(dev.to_string()))?;
    let Behavior::Switch(sw) = &d.behavior else {
        return Err(NetivError::WrongEventKind {
            seq: 0,
            expected: "un switch",
        });
    };
    let mut s = String::new();
    let _ = writeln!(s, "{:<8} {:<8} DÉTAIL", "PORT", "MODE");
    for i in &d.ifaces {
        match sw.port_mode(i.id) {
            PortMode::Access { vlan } => {
                let _ = writeln!(s, "{:<8} {:<8} VLAN {vlan}", i.name, "access");
            }
            PortMode::Trunk { allowed, native } => {
                let list = if allowed.is_empty() {
                    "tous les VLAN".to_string()
                } else {
                    format!(
                        "VLAN {}",
                        allowed
                            .iter()
                            .map(u16::to_string)
                            .collect::<Vec<_>>()
                            .join(",")
                    )
                };
                let _ = writeln!(s, "{:<8} {:<8} {list}, natif {native}", i.name, "trunk");
            }
        }
    }
    Ok(s)
}

/// Règles de filtrage d'un équipement IP.
pub fn show_acl(sim: &Simulation, dev: DeviceId) -> Result<String, NetivError> {
    let d = sim
        .device(dev)
        .ok_or_else(|| NetivError::UnknownDevice(dev.to_string()))?;
    let Behavior::Ip(node) = &d.behavior else {
        return Err(NetivError::NotAnIpDevice(d.name.clone()));
    };
    let mut s = String::new();
    let _ = writeln!(
        s,
        "Politique par défaut : {} — suivi de connexion : {}",
        node.acl.default,
        if node.acl.stateful {
            format!("actif ({} flux)", node.acl.tracked_flows())
        } else {
            "inactif".to_string()
        }
    );
    if node.acl.rules.is_empty() {
        let _ = writeln!(s, "(aucune règle)");
    }
    for (i, r) in node.acl.rules.iter().enumerate() {
        let text = r.describe(|id| iface_name(sim, dev, id).to_string());
        let _ = writeln!(s, "  {}. {text}", i + 1);
    }
    Ok(s)
}

/// Services à l'écoute sur un équipement IP.
pub fn show_services(sim: &Simulation, dev: DeviceId) -> Result<String, NetivError> {
    let d = sim
        .device(dev)
        .ok_or_else(|| NetivError::UnknownDevice(dev.to_string()))?;
    let Behavior::Ip(node) = &d.behavior else {
        return Err(NetivError::NotAnIpDevice(d.name.clone()));
    };
    let mut s = String::new();
    if node.services.is_empty() {
        let _ = writeln!(s, "(aucun service à l'écoute)");
    }
    for svc in &node.services {
        let _ = writeln!(s, "{}/{:<6} {}", svc.proto, svc.port, svc.name);
    }
    Ok(s)
}

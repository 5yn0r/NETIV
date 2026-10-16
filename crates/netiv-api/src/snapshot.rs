//! Mise en forme JSON de l'état d'une simulation, pour les interfaces.

use netiv_cli::Session;
use netiv_core::device::Behavior;
use netiv_core::{DeviceId, Simulation, TraceEvent, TraceId, TraceKind, render};
use serde_json::{Value, json};

/// Nom court du type d'un événement (utile aux couleurs et aux animations).
pub fn kind_slug(k: &TraceKind) -> &'static str {
    match k {
        TraceKind::CommandIssued { .. } | TraceKind::ConfigChanged { .. } => "config",
        TraceKind::FrameSent { .. } => "frame_sent",
        TraceKind::FrameReceived { .. } => "frame_received",
        TraceKind::Dropped { .. } | TraceKind::AclDecision { .. } => "drop",
        TraceKind::MacLearned { .. } | TraceKind::Flood { .. } | TraceKind::Forward { .. } => {
            "switch"
        }
        TraceKind::ArpRequestSent { .. }
        | TraceKind::ArpReplySent { .. }
        | TraceKind::ArpCacheUpdated { .. }
        | TraceKind::ArpTimeout { .. } => "arp",
        TraceKind::RouteLookup(_) | TraceKind::TtlDecremented { .. } | TraceKind::TtlExpired => {
            "route"
        }
        TraceKind::PingSent { .. }
        | TraceKind::PingReply { .. }
        | TraceKind::PingError { .. }
        | TraceKind::PingTimeout { .. }
        | TraceKind::CurlStarted { .. }
        | TraceKind::CurlResult { .. }
        | TraceKind::UdpSent { .. }
        | TraceKind::UdpResult { .. }
        | TraceKind::TcpEstablished { .. }
        | TraceKind::TcpResetSent { .. }
        | TraceKind::ServiceRequest { .. } => "app",
        TraceKind::FaultApplied { .. } | TraceKind::FaultRepaired { .. } => "fault",
        TraceKind::Dhcp { .. } => "dhcp",
        TraceKind::DnsLookup { .. }
        | TraceKind::DnsResult { .. }
        | TraceKind::DnsAnswered { .. } => "dns",
    }
}

fn device_name(sim: &Simulation, d: DeviceId) -> String {
    render::device_name(sim, d).to_string()
}

/// Un événement de trace, prêt pour le navigateur.
pub fn event_json(sim: &Simulation, e: &TraceEvent) -> Value {
    // Pour une trame émise, le câble emprunté : de l'émetteur vers son voisin.
    let edge = match &e.kind {
        TraceKind::FrameSent { iface, .. } => sim
            .device(e.device)
            .and_then(|d| d.iface(*iface))
            .and_then(|i| i.link)
            .and_then(|l| sim.link(l))
            .and_then(|l| l.peer_of(e.device, *iface))
            .map(
                |(pd, _)| json!({ "from": device_name(sim, e.device), "to": device_name(sim, pd) }),
            ),
        _ => None,
    };
    json!({
        "seq": e.seq,
        "time_us": e.time.as_micros(),
        "time": e.time.to_string(),
        "device": device_name(sim, e.device),
        "trace_id": e.trace_id.map(|t| t.0),
        "kind": kind_slug(&e.kind),
        "text": render::describe(sim, e),
        "edge": edge,
    })
}

/// État complet d'un lab.
pub fn snapshot(id: &str, session: &Session) -> Value {
    let challenge = session.challenge_info().map(|c| {
        json!({
            "kind": c.kind,
            "id": c.id,
            "title": c.title,
            "description": c.description,
            "finished": c.finished,
            "fixed": c.fixed,
            "answered": c.answered,
            "hints_used": c.hints_used,
            "remaining_secs": c.remaining_secs,
            "score": c.score,
        })
    });
    let edit_lock = session.edit_lock();
    let Some(sim) = session.simulation() else {
        return json!({
            "id": id, "name": null, "devices": [], "links": [], "time_us": 0,
            "pending": 0, "trace_len": 0, "challenge": challenge,
            "editable": edit_lock.is_none(), "edit_lock": edit_lock,
        });
    };
    let positions = session.positions();
    let devices: Vec<Value> = sim
        .devices()
        .iter()
        .map(|d| {
            let ifaces: Vec<Value> = d
                .ifaces
                .iter()
                .map(|i| {
                    let peer = i.link.and_then(|l| sim.link(l)).and_then(|l| l.peer_of(d.id, i.id));
                    let (dhcp, relay) = match &d.behavior {
                        Behavior::Ip(n) => (n.dhcp_ifaces.contains(&i.id), n.dhcp_relay.get(&i.id).map(ToString::to_string)),
                        Behavior::Switch(_) => (false, None),
                    };
                    json!({
                        "name": i.name,
                        "ip": i.ip.map(|n| n.to_string()),
                        "dhcp": dhcp,
                        "dhcp_relay": relay,
                        "mac": i.mac.to_string(),
                        "up": i.admin_up,
                        "link_up": i.link.and_then(|l| sim.link(l)).map(|l| l.up),
                        "peer": peer.map(|(pd, pi)| format!("{}:{}", device_name(sim, pd), render::iface_name(sim, pd, pi))),
                    })
                })
                .collect();
            let (forwarding, filtered) = match &d.behavior {
                Behavior::Ip(n) => (n.forwarding, !n.acl.rules.is_empty() || n.acl.default == netiv_core::acl::AclAction::Deny),
                Behavior::Switch(_) => (false, false),
            };
            let (dhcp_server, dns_server) = match &d.behavior {
                Behavior::Ip(n) => (
                    n.dhcp_server.as_ref().map(|s| json!({ "enabled": s.enabled })),
                    n.dns_server.as_ref().map(|s| json!({ "enabled": s.enabled })),
                ),
                Behavior::Switch(_) => (None, None),
            };
            json!({
                "name": d.name,
                "kind": d.kind.to_string(),
                "role": d.effective_role(),
                "ifaces": ifaces,
                "forwarding": forwarding,
                "filtered": filtered,
                "dhcp_server": dhcp_server,
                "dns_server": dns_server,
                "pos": positions.get(&d.name),
            })
        })
        .collect();
    let links: Vec<Value> = sim
        .links()
        .iter()
        .map(|l| {
            json!({
                "a": format!("{}:{}", device_name(sim, l.a.0), render::iface_name(sim, l.a.0, l.a.1)),
                "b": format!("{}:{}", device_name(sim, l.b.0), render::iface_name(sim, l.b.0, l.b.1)),
                "a_device": device_name(sim, l.a.0),
                "b_device": device_name(sim, l.b.0),
                "up": l.up,
                "latency_us": l.latency.as_micros(),
                "loss": l.loss,
            })
        })
        .collect();
    json!({
        "id": id,
        "name": session.network_name(),
        "time_us": sim.now().as_micros(),
        "time": sim.now().to_string(),
        "pending": sim.pending(),
        "trace_len": sim.trace().len(),
        "devices": devices,
        "links": links,
        "challenge": challenge,
        "editable": edit_lock.is_none(),
        "edit_lock": edit_lock,
    })
}

/// Événements à partir de `since`, au plus `limit`.
pub fn trace_json(sim: &Simulation, since: usize, limit: usize) -> Value {
    let all = sim.trace().events();
    let events: Vec<Value> = all
        .iter()
        .skip(since)
        .take(limit)
        .map(|e| event_json(sim, e))
        .collect();
    json!({ "events": events, "len": all.len(), "since": since })
}

/// Transactions (ping, curl, udp) de la trace, de la plus récente à la plus ancienne.
pub fn transactions(sim: &Simulation, limit: usize) -> Value {
    let mut out: Vec<Value> = Vec::new();
    for e in sim.trace().events().iter().rev() {
        let (what, label) = match &e.kind {
            TraceKind::PingSent { dst, seq } => ("ping", format!("ping → {dst} (seq {seq})")),
            TraceKind::CurlStarted { dst, port } => ("curl", format!("curl → {dst}:{port}")),
            TraceKind::UdpSent { dst, port } => ("udp", format!("udp → {dst}:{port}")),
            // Un échange DHCP compte une fois (première tentative), une résolution aussi.
            TraceKind::Dhcp {
                iface,
                event: netiv_core::dhcp::DhcpEvent::ClientDiscover { attempt: 1 },
            } => (
                "dhcp",
                format!("DHCP sur {}", render::iface_name(sim, e.device, *iface)),
            ),
            TraceKind::DnsLookup { name, .. } => ("dns", format!("DNS → {name}")),
            _ => continue,
        };
        let Some(tid) = e.trace_id else { continue };
        // Résultat : le dernier événement applicatif de la transaction.
        let outcome = sim
            .trace()
            .by_trace(tid)
            .filter_map(|x| match &x.kind {
                TraceKind::PingReply { .. } => Some("ok"),
                TraceKind::CurlResult { outcome, .. } => Some(
                    if matches!(outcome, netiv_core::trace::CurlOutcome::Ok { .. }) {
                        "ok"
                    } else {
                        "fail"
                    },
                ),
                TraceKind::UdpResult { outcome, .. } => Some(
                    if matches!(outcome, netiv_core::trace::UdpOutcome::Reply { .. }) {
                        "ok"
                    } else {
                        "fail"
                    },
                ),
                TraceKind::PingError { .. } | TraceKind::PingTimeout { .. } => Some("fail"),
                TraceKind::Dhcp { event, .. } => match event {
                    netiv_core::dhcp::DhcpEvent::ClientBound(_) => Some("ok"),
                    netiv_core::dhcp::DhcpEvent::ClientFailed { .. } => Some("fail"),
                    _ => None,
                },
                TraceKind::DnsResult { outcome, .. } => Some(
                    if matches!(outcome, netiv_core::dns::DnsOutcome::Resolved(_)) {
                        "ok"
                    } else {
                        "fail"
                    },
                ),
                _ => None,
            })
            .last()
            .unwrap_or("pending");
        out.push(json!({
            "id": tid.0,
            "kind": what,
            "from": device_name(sim, e.device),
            "label": format!("{} : {label}", device_name(sim, e.device)),
            "time": e.time.to_string(),
            "outcome": outcome,
        }));
        if out.len() >= limit {
            break;
        }
    }
    json!({ "transactions": out })
}

/// Parcours d'une transaction : étapes animables et texte.
pub fn journey_json(sim: &Simulation, tid: TraceId) -> Option<Value> {
    let steps: Vec<Value> = sim
        .trace()
        .by_trace(tid)
        .map(|e| event_json(sim, e))
        .collect();
    if steps.is_empty() {
        return None;
    }
    let text = render::journey(sim, tid).ok()?;
    Some(json!({ "id": tid.0, "steps": steps, "text": text }))
}

/// Tables d'état d'un équipement.
pub fn device_json(sim: &Simulation, name: &str) -> Option<Value> {
    let id = sim.device_id(name).ok()?;
    let dev = sim.device(id)?;
    let mut sections: Vec<Value> = Vec::new();
    let mut add = |title: &str, text: Result<String, netiv_core::NetivError>| {
        if let Ok(t) = text {
            sections.push(json!({ "title": title, "text": t }));
        }
    };
    add("Interfaces", render::show_interfaces(sim, id));
    match &dev.behavior {
        Behavior::Ip(n) => {
            add("Routes", render::show_routes(sim, id));
            add("Cache ARP", render::show_arp(sim, id));
            add("Services", render::show_services(sim, id));
            add("Filtrage", render::show_acl(sim, id));
            if !n.dhcp_ifaces.is_empty() || !n.dhcp_relay.is_empty() || n.dhcp_server.is_some() {
                add("DHCP", render::show_dhcp(sim, id));
            }
            if n.effective_dns().is_some() || n.dns_server.is_some() {
                add("DNS", render::show_dns(sim, id));
            }
        }
        Behavior::Switch(_) => {
            add("VLAN et ports", render::show_vlans(sim, id));
            add("Table MAC", render::show_mac_table(sim, id));
        }
    }
    Some(json!({
        "name": dev.name,
        "kind": dev.kind.to_string(),
        "role": dev.effective_role(),
        "sections": sections,
    }))
}

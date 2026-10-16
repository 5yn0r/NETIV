//! Diagnostic de DHCP (« pourquoi ce poste n'a-t-il pas d'adresse ? ») et de
//! DNS (« pourquoi ce nom ne marche-t-il pas ? »).
//!
//! DHCP : on part du domaine de diffusion du client (là où sa demande peut
//! aller), on y cherche un serveur ou un relais, puis on vérifie l'état du
//! serveur et de ses plages. En dernier recours, on rejoue l'échange DORA
//! dans une copie du réseau.
//!
//! DNS : on résout le nom dans une copie du réseau, puis on interprète la
//! réponse (nom inconnu, serveur arrêté, mauvais serveur, adresse fausse).
//! Si le nom est bon, le reste est un diagnostic IP ordinaire.

use std::net::Ipv4Addr;

use netiv_core::device::Behavior;
use netiv_core::dhcp::ClientState;
use netiv_core::dns::{AfterResolve, DnsOutcome};
use netiv_core::packet::L4Proto;
use netiv_core::{
    Command, DeviceId, DeviceKind, IfaceId, NetivError, Simulation, TraceEvent, TraceKind,
};

use crate::cause::{CauseKind, RootCause};
use crate::diagnose::{Check, CheckStatus, Diagnosis, Request, Status, diagnose};
use crate::l2::{L2Options, l2_domain, owner_of};
use crate::probe::{Probe, ProbeResult};

fn node(sim: &Simulation, d: DeviceId) -> Option<&netiv_core::ip_node::IpNode> {
    match &sim.device(d)?.behavior {
        Behavior::Ip(n) => Some(n),
        Behavior::Switch(_) => None,
    }
}

fn name(sim: &Simulation, d: DeviceId) -> String {
    sim.device(d).map_or("?".into(), |x| x.name.clone())
}

fn iface_name(sim: &Simulation, d: DeviceId, i: IfaceId) -> String {
    sim.device(d)
        .and_then(|x| x.iface(i))
        .map_or("?".into(), |x| x.name.clone())
}

fn first_ip(sim: &Simulation, d: DeviceId) -> Option<Ipv4Addr> {
    sim.device(d)?
        .ifaces
        .iter()
        .find_map(|i| i.ip.map(|n| n.addr()))
}

/// Un serveur DHCP sur une machine d'utilisateur n'a rien à faire là.
pub fn is_rogue_server(sim: &Simulation, d: DeviceId) -> bool {
    sim.device(d).is_some_and(|x| {
        matches!(x.effective_role(), "user" | "guest" | "iot")
            && node(sim, d).is_some_and(|n| n.dhcp_server.is_some())
    })
}

/// Serveurs DHCP légitimes du réseau.
fn legit_servers(sim: &Simulation) -> Vec<DeviceId> {
    sim.devices()
        .iter()
        .filter(|d| {
            node(sim, d.id).is_some_and(|n| n.dhcp_server.is_some()) && !is_rogue_server(sim, d.id)
        })
        .map(|d| d.id)
        .collect()
}

/// Cause d'un problème DHCP sur `src`, s'il y en a un : client sans adresse,
/// ou configuration reçue d'un serveur pirate.
pub fn dhcp_cause(sim: &Simulation, src: DeviceId) -> Option<RootCause> {
    let dev = sim.device(src)?;
    let n = node(sim, src)?;
    let me = dev.name.clone();

    // Configuration reçue d'un serveur pirate.
    for st in n.dhcp_clients.values() {
        if let ClientState::Bound(b) = st {
            if let Some((sd, _)) = owner_of(sim, b.server) {
                if is_rogue_server(sim, sd) {
                    let rogue = name(sim, sd);
                    let mut c = RootCause::new(
                        CauseKind::RogueDhcp,
                        &rogue,
                        format!(
                            "{me} a reçu sa configuration de {rogue} ({}), une machine d'utilisateur qui distribue des adresses : passerelle {}",
                            b.server,
                            b.gateway.map_or("aucune".to_string(), |g| g.to_string())
                        ),
                    );
                    c.actual = Some(format!("serveur {}", b.server));
                    c.expected = legit_servers(sim)
                        .first()
                        .and_then(|d| first_ip(sim, *d))
                        .map(|ip| format!("serveur {ip}"));
                    c.fix = Some(format!("dhcp server {rogue} off ; dhcp {me} renew"));
                    return Some(c);
                }
            }
        }
    }

    // Une interface DHCP sans adresse.
    let iface = *n.dhcp_ifaces.iter().find(|i| {
        dev.iface(**i)
            .is_some_and(|x| x.ip.is_none() && x.link.is_some())
    })?;
    let domain = l2_domain(
        sim,
        src,
        iface,
        L2Options {
            vlan_aware: true,
            honor_state: true,
        },
    );
    let local_server = domain
        .iter()
        .find(|(d, _)| {
            node(sim, *d).is_some_and(|x| x.dhcp_server.is_some()) && !is_rogue_server(sim, *d)
        })
        .copied();
    let relay = domain.iter().find_map(|(d, i)| {
        node(sim, *d)
            .and_then(|x| x.dhcp_relay.get(i))
            .map(|s| (*d, *i, *s))
    });

    // Où est le serveur, et quelle adresse désigne le réseau du client ?
    let (server, hint, relay_dev) = match (local_server, relay) {
        (Some((sd, si)), _) => {
            let hint = sim
                .device(sd)
                .and_then(|x| x.iface(si))
                .and_then(|x| x.ip)
                .map(|x| x.addr());
            (sd, hint, None)
        }
        (None, Some((rd, ri, target))) => {
            let relay_label = format!("{}/{}", name(sim, rd), iface_name(sim, rd, ri));
            match owner_of(sim, target) {
                Some((sd, _)) if node(sim, sd).is_some_and(|x| x.dhcp_server.is_some()) => {
                    let hint = sim
                        .device(rd)
                        .and_then(|x| x.iface(ri))
                        .and_then(|x| x.ip)
                        .map(|x| x.addr());
                    (sd, hint, Some(rd))
                }
                _ => {
                    let real = legit_servers(sim).first().and_then(|d| first_ip(sim, *d));
                    let mut c = RootCause::new(
                        CauseKind::DhcpRelayMissing,
                        &name(sim, rd),
                        format!(
                            "le relais {relay_label} transmet les demandes à {target}, où aucun serveur DHCP ne tourne"
                        ),
                    );
                    c.actual = Some(format!("relais vers {target}"));
                    if let Some(ip) = real {
                        c.expected = Some(format!("relais vers {ip}"));
                        c.fix = Some(format!(
                            "dhcp relay {} {} {ip} ; dhcp {me} renew",
                            name(sim, rd),
                            iface_name(sim, rd, ri)
                        ));
                    }
                    return Some(c);
                }
            }
        }
        (None, None) => {
            let servers = legit_servers(sim);
            let router = domain.iter().find(|(d, _)| {
                sim.device(*d)
                    .is_some_and(|x| matches!(x.kind, DeviceKind::Router | DeviceKind::Firewall))
            });
            return Some(match (servers.first(), router) {
                (Some(sd), Some((rd, ri))) => {
                    let ip = first_ip(sim, *sd);
                    let mut c = RootCause::new(
                        CauseKind::DhcpRelayMissing,
                        &name(sim, *rd),
                        format!(
                            "le serveur DHCP {} est dans un autre réseau et {}/{} ne relaie pas les demandes (une diffusion ne traverse jamais un routeur)",
                            name(sim, *sd),
                            name(sim, *rd),
                            iface_name(sim, *rd, *ri)
                        ),
                    );
                    c.actual = Some("aucun relais".to_string());
                    if let Some(ip) = ip {
                        c.expected = Some(format!("relais vers {ip}"));
                        c.fix = Some(format!(
                            "dhcp relay {} {} {ip} ; dhcp {me} renew",
                            name(sim, *rd),
                            iface_name(sim, *rd, *ri)
                        ));
                    }
                    c
                }
                _ => RootCause::new(
                    CauseKind::DhcpServerDown,
                    &me,
                    "aucun serveur DHCP n'est joignable depuis ce réseau",
                ),
            });
        }
    };

    let sname = name(sim, server);
    let srv = node(sim, server)?.dhcp_server.as_ref()?;
    if !srv.enabled {
        let mut c = RootCause::new(
            CauseKind::DhcpServerDown,
            &sname,
            format!("le serveur DHCP de {sname} est arrêté"),
        );
        c.actual = Some("arrêté".into());
        c.expected = Some("démarré".into());
        c.fix = Some(format!("dhcp server {sname} on ; dhcp {me} renew"));
        return Some(c);
    }
    let hint = hint?;
    let Some(pi) = srv.pool_for(hint) else {
        let mut c = RootCause::new(
            CauseKind::DhcpPoolExhausted,
            &sname,
            format!("{sname} n'a aucune plage pour le réseau du client (celui de {hint})"),
        );
        c.fix = Some(format!(
            "dhcp pool add {sname} <réseau> <première>-<dernière> gw <passerelle> ; dhcp {me} renew"
        ));
        return Some(c);
    };
    let pool = &srv.pools[pi];
    let mac = dev.iface(iface).map(|x| x.mac);
    let others = srv
        .leases
        .iter()
        .filter(|(ip, l)| {
            l.bound && l.expires > sim.now() && pool.contains(**ip) && Some(l.mac) != mac
        })
        .count() as u32;
    if others >= pool.size() {
        let mut c = RootCause::new(
            CauseKind::DhcpPoolExhausted,
            &sname,
            format!(
                "la plage {} de {sname} est pleine : {others} adresse(s) sur {} déjà attribuée(s)",
                pool.range_text(),
                pool.size()
            ),
        );
        c.actual = Some(format!("{} adresse(s)", pool.size()));
        c.expected = Some("une plage plus grande".into());
        c.fix = Some(format!(
            "dhcp pool add {sname} {} {}-<dernière plus haute> ; dhcp {me} renew",
            pool.network.trunc(),
            pool.first
        ));
        return Some(c);
    }

    // La configuration semble bonne : on rejoue l'échange dans une copie.
    let mut probe = sim.fresh_clone();
    let ok = probe
        .command(
            src,
            Command::DhcpRenew {
                iface: Some(iface),
                fresh: true,
            },
        )
        .is_ok()
        && probe.run_until_idle().is_ok()
        && probe
            .device(src)
            .and_then(|x| x.iface(iface))
            .is_some_and(|x| x.ip.is_some());
    if ok {
        let mut c = RootCause::new(
            CauseKind::Unknown,
            &me,
            format!("le serveur DHCP répond désormais, mais {me} n'a pas redemandé d'adresse"),
        );
        c.fix = Some(format!("dhcp {me} renew"));
        return Some(c);
    }
    // Les demandes relayées n'atteignent pas le serveur : problème de routage.
    if let (Some(rd), Some(sip)) = (relay_dev, first_ip(sim, server)) {
        if let Ok(d) = diagnose(
            sim,
            None,
            &Request {
                src: rd,
                dst: sip,
                service: None,
            },
        ) {
            if let Some(c) = d.cause.filter(|c| c.kind != CauseKind::Unknown) {
                return Some(c);
            }
        }
    }
    Some(RootCause::new(
        CauseKind::Unknown,
        &me,
        "les demandes DHCP n'aboutissent pas : suivre l'échange avec « dhcp <poste> renew » puis « trace »",
    ))
}

// ---------------------------------------------------------------------------
// DNS

/// Résout `name` depuis `src` dans une copie neuve du réseau.
pub fn resolve_in_copy(
    sim: &Simulation,
    src: DeviceId,
    name: &str,
) -> (DnsOutcome, Vec<TraceEvent>) {
    let mut c = sim.fresh_clone();
    let cmd = Command::Resolve {
        name: name.to_string(),
        then: AfterResolve::Nothing,
    };
    if c.command(src, cmd).is_err() || c.run_until_idle().is_err() {
        return (DnsOutcome::Timeout, Vec::new());
    }
    let events = c.take_trace();
    let outcome = events
        .iter()
        .find_map(|e| match &e.kind {
            TraceKind::DnsResult { outcome, .. } => Some(outcome.clone()),
            _ => None,
        })
        .unwrap_or(DnsOutcome::Timeout);
    (outcome, events)
}

/// Serveur DNS qui fait autorité sur `name` (le premier qui le connaît, sinon
/// le premier serveur DNS du réseau).
fn zone_owner(sim: &Simulation, dns_name: &str) -> Option<DeviceId> {
    let servers: Vec<DeviceId> = sim
        .devices()
        .iter()
        .filter(|d| node(sim, d.id).is_some_and(|n| n.dns_server.is_some()))
        .map(|d| d.id)
        .collect();
    servers
        .iter()
        .copied()
        .find(|d| {
            node(sim, *d)
                .and_then(|n| n.dns_server.as_ref())
                .is_some_and(|z| z.lookup(dns_name).is_some())
        })
        .or_else(|| servers.first().copied())
}

fn dns_diagnosis(
    src: DeviceId,
    server: Option<Ipv4Addr>,
    title: String,
    cause: RootCause,
    events: Vec<TraceEvent>,
) -> Diagnosis {
    let dst = server.unwrap_or(Ipv4Addr::UNSPECIFIED);
    Diagnosis {
        request: Request {
            src,
            dst,
            service: Some((L4Proto::Udp, netiv_core::dns::PORT)),
        },
        status: Status::Broken,
        checks: vec![Check {
            name: "Résolution DNS",
            status: CheckStatus::Failed,
        }],
        cause: Some(cause),
        probe: ProbeResult {
            probe: Probe::udp(src, dst, netiv_core::dns::PORT),
            success: false,
            replies: 0,
            sent: 1,
            max_rtt: None,
            icmp_error: None,
            refused: false,
            events,
        },
        title,
    }
}

/// Diagnostique « `src` peut-il joindre `dns_name` (sur ce service) ? » :
/// résolution du nom d'abord, puis le chemin vers l'adresse trouvée.
pub fn diagnose_name(
    sim: &Simulation,
    baseline: Option<&Simulation>,
    src: DeviceId,
    dns_name: &str,
    service: Option<(L4Proto, u16)>,
) -> Result<Diagnosis, NetivError> {
    let me = name(sim, src);
    let what = match service {
        None => "ping".to_string(),
        Some((p, port)) => format!("{p}/{port}"),
    };
    let server = node(sim, src).and_then(|n| n.effective_dns());

    // Pas d'adresse du tout : c'est d'abord un problème DHCP.
    if let Some(c) = dhcp_cause(sim, src).filter(|c| c.kind != CauseKind::RogueDhcp) {
        return Ok(dns_diagnosis(
            src,
            server,
            format!("{what} {me} → {dns_name}"),
            c,
            Vec::new(),
        ));
    }

    let (outcome, events) = resolve_in_copy(sim, src, dns_name);
    let expected = baseline.and_then(|b| match resolve_in_copy(b, src, dns_name).0 {
        DnsOutcome::Resolved(ip) => Some(ip),
        _ => None,
    });
    let title = format!("{what} {me} → {dns_name}");
    let cause = match outcome {
        DnsOutcome::Resolved(ip) => {
            let wrong = owner_of(sim, ip).is_none() || expected.is_some_and(|e| e != ip);
            if !wrong {
                let mut d = diagnose(
                    sim,
                    baseline,
                    &Request {
                        src,
                        dst: ip,
                        service,
                    },
                )?;
                d.title = format!("{what} {me} → {dns_name} ({ip})");
                d.checks.insert(
                    0,
                    Check {
                        name: "Résolution DNS",
                        status: CheckStatus::Ok,
                    },
                );
                return Ok(d);
            }
            let owner = zone_owner(sim, dns_name).map_or(me.clone(), |d| name(sim, d));
            let mut c = RootCause::new(
                CauseKind::DnsRecordWrong,
                &owner,
                format!(
                    "{dns_name} est enregistré avec l'adresse {ip}, qui n'est pas celle du serveur visé"
                ),
            );
            c.actual = Some(ip.to_string());
            c.expected = expected.map(|e| e.to_string());
            c.fix = Some(format!(
                "dns record add {owner} {dns_name} {}",
                expected.map_or("<adresse>".to_string(), |e| e.to_string())
            ));
            c
        }
        DnsOutcome::NxDomain => {
            let owner = zone_owner(sim, dns_name).map_or(me.clone(), |d| name(sim, d));
            let mut c = RootCause::new(
                CauseKind::DnsRecordWrong,
                &owner,
                format!("le serveur DNS ne connaît pas le nom {dns_name} (NXDOMAIN)"),
            );
            c.expected = expected.map(|e| format!("{dns_name} = {e}"));
            c.fix = Some(format!(
                "dns record add {owner} {dns_name} {}",
                expected.map_or("<adresse>".to_string(), |e| e.to_string())
            ));
            c
        }
        DnsOutcome::NoServer => {
            let real = zone_owner(sim, dns_name).and_then(|d| first_ip(sim, d));
            let mut c = RootCause::new(
                CauseKind::DnsWrongServer,
                &me,
                format!("{me} n'a aucun serveur DNS (ni configuré, ni reçu par DHCP)"),
            );
            c.actual = Some("aucun".into());
            c.expected = real.map(|ip| ip.to_string());
            c.fix = real.map(|ip| format!("dns {me} {ip}"));
            c
        }
        DnsOutcome::Timeout | DnsOutcome::Unreachable => {
            let server_ip = server.unwrap_or(Ipv4Addr::UNSPECIFIED);
            let real = zone_owner(sim, dns_name).and_then(|d| first_ip(sim, d));
            match owner_of(sim, server_ip) {
                Some((sd, _)) if node(sim, sd).and_then(|n| n.dns_server.as_ref()).is_some() => {
                    let sname = name(sim, sd);
                    let running = node(sim, sd)
                        .and_then(|n| n.dns_server.as_ref())
                        .is_some_and(|z| z.enabled);
                    if !running {
                        let mut c = RootCause::new(
                            CauseKind::DnsServerDown,
                            &sname,
                            format!("le serveur DNS de {sname} est arrêté"),
                        );
                        c.actual = Some("arrêté".into());
                        c.expected = Some("démarré".into());
                        c.fix = Some(format!("dns server {sname} on"));
                        c
                    } else {
                        // Le serveur tourne : c'est le chemin jusqu'à lui qui est cassé.
                        let mut d = diagnose(
                            sim,
                            baseline,
                            &Request {
                                src,
                                dst: server_ip,
                                service: None,
                            },
                        )?;
                        d.title = format!(
                            "{what} {me} → {dns_name} : serveur DNS {server_ip} injoignable"
                        );
                        d.checks.insert(
                            0,
                            Check {
                                name: "Résolution DNS",
                                status: CheckStatus::Failed,
                            },
                        );
                        return Ok(d);
                    }
                }
                _ => {
                    let mut c = RootCause::new(
                        CauseKind::DnsWrongServer,
                        &me,
                        format!("{me} interroge {server_ip}, où aucun serveur DNS ne répond"),
                    );
                    c.actual = Some(server_ip.to_string());
                    c.expected = real.map(|ip| ip.to_string());
                    c.fix = real.map(|ip| format!("dns {me} {ip}"));
                    c
                }
            }
        }
    };
    Ok(dns_diagnosis(src, server, title, cause, events))
}

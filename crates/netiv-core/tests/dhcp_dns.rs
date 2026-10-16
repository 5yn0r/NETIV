#![allow(clippy::unwrap_used, clippy::expect_used)]

//! DHCP (client, serveur, relais) et DNS (résolution, cache, pannes).

mod common;

use common::*;
use netiv_core::dhcp::{ClientState, DhcpEvent, DhcpPool};
use netiv_core::dns::{AfterResolve, DnsOutcome};
use netiv_core::packet::{EtherPayload, IpPayload, UdpPayload};
use netiv_core::{Command, DeviceId, DeviceKind, Fault, Simulation, TraceKind};

fn pool(net: &str, first: &str, last: &str, gw: &str) -> DhcpPool {
    DhcpPool {
        network: net.parse().unwrap(),
        first: ip(first),
        last: ip(last),
        gateway: Some(ip(gw)),
        dns: Some(ip("10.0.2.10")),
        lease_s: 3_600,
    }
}

/// PC1, PC2 ── SW1 ── R1 ── SW2 ── SRV (DHCP + DNS), PC3
///   (DHCP)        relais         (DHCP local)
fn net(relay: bool) -> Simulation {
    let mut sim = Simulation::new(7);
    let pc1 = add(&mut sim, "PC1", DeviceKind::Host, &[("eth0", None)]);
    let pc2 = add(&mut sim, "PC2", DeviceKind::Host, &[("eth0", None)]);
    let pc3 = add(&mut sim, "PC3", DeviceKind::Host, &[("eth0", None)]);
    add(
        &mut sim,
        "SW1",
        DeviceKind::Switch,
        &[("p1", None), ("p2", None), ("p3", None)],
    );
    add(
        &mut sim,
        "SW2",
        DeviceKind::Switch,
        &[("p1", None), ("p2", None), ("p3", None)],
    );
    let r1 = add(
        &mut sim,
        "R1",
        DeviceKind::Router,
        &[("eth0", Some("10.0.1.1/24")), ("eth1", Some("10.0.2.1/24"))],
    );
    let srv = add(
        &mut sim,
        "SRV",
        DeviceKind::Server,
        &[("eth0", Some("10.0.2.10/24"))],
    );
    gw(&mut sim, srv, "10.0.2.1");
    link(&mut sim, "PC1:eth0", "SW1:p1");
    link(&mut sim, "PC2:eth0", "SW1:p2");
    link(&mut sim, "SW1:p3", "R1:eth0");
    link(&mut sim, "R1:eth1", "SW2:p1");
    link(&mut sim, "SW2:p2", "SRV:eth0");
    link(&mut sim, "SW2:p3", "PC3:eth0");
    for d in [pc1, pc2, pc3] {
        sim.set_dhcp_client(d, sim.iface_id(d, "eth0").unwrap(), true)
            .unwrap();
    }
    sim.set_dhcp_pools(
        srv,
        vec![
            pool("10.0.1.0/24", "10.0.1.100", "10.0.1.110", "10.0.1.1"),
            pool("10.0.2.0/24", "10.0.2.100", "10.0.2.110", "10.0.2.1"),
        ],
    )
    .unwrap();
    if relay {
        sim.set_dhcp_relay(r1, sim.iface_id(r1, "eth0").unwrap(), Some(ip("10.0.2.10")))
            .unwrap();
    }
    sim.set_dns_record(srv, "web.lan", Some(ip("10.0.2.10")))
        .unwrap();
    sim.clear_trace();
    sim
}

fn dev(sim: &Simulation, n: &str) -> DeviceId {
    sim.device_id(n).unwrap()
}

fn addr(sim: &Simulation, n: &str) -> Option<String> {
    sim.device(dev(sim, n)).unwrap().ifaces[0]
        .ip
        .map(|x| x.to_string())
}

fn node<'a>(sim: &'a Simulation, n: &str) -> &'a netiv_core::ip_node::IpNode {
    match &sim.device(dev(sim, n)).unwrap().behavior {
        netiv_core::device::Behavior::Ip(x) => x,
        netiv_core::device::Behavior::Switch(_) => panic!("{n} est un switch"),
    }
}

fn dhcp_events(sim: &Simulation) -> Vec<DhcpEvent> {
    events(sim)
        .iter()
        .filter_map(|e| match &e.kind {
            TraceKind::Dhcp { event, .. } => Some(event.clone()),
            _ => None,
        })
        .collect()
}

fn ping(sim: &mut Simulation, from: &str, to: &str) -> bool {
    let start = sim.trace().len();
    sim.ping(dev(sim, from), ip(to), 1).unwrap();
    sim.run_until_idle().unwrap();
    sim.trace()
        .since(start)
        .iter()
        .any(|e| matches!(e.kind, TraceKind::PingReply { .. }))
}

#[test]
fn clients_get_addresses_through_a_relay_and_locally() {
    let mut sim = net(true);
    sim.restart_dhcp().unwrap();
    assert_eq!(addr(&sim, "PC1").as_deref(), Some("10.0.1.100/24"));
    assert_eq!(addr(&sim, "PC2").as_deref(), Some("10.0.1.101/24"));
    assert_eq!(addr(&sim, "PC3").as_deref(), Some("10.0.2.100/24"));
    // Passerelle et DNS appris.
    assert_eq!(node(&sim, "PC1").gateway, Some(ip("10.0.1.1")));
    assert!(node(&sim, "PC1").gateway_learned);
    assert_eq!(node(&sim, "PC1").effective_dns(), Some(ip("10.0.2.10")));
    // Le relais a travaillé, dans les deux sens.
    let ev = dhcp_events(&sim);
    assert!(ev.iter().any(|e| matches!(e, DhcpEvent::Relayed { .. })));
    assert!(
        ev.iter()
            .any(|e| matches!(e, DhcpEvent::RelayedBack { .. }))
    );
    assert_eq!(
        ev.iter()
            .filter(|e| matches!(e, DhcpEvent::ClientBound(_)))
            .count(),
        3
    );
    // Et le réseau fonctionne.
    assert!(ping(&mut sim, "PC1", "10.0.2.10"));
    assert!(ping(&mut sim, "PC3", "10.0.1.101"));
}

#[test]
fn dhcp_is_deterministic() {
    let run = || {
        let mut s = net(true);
        s.restart_dhcp().unwrap();
        s.trace().events().to_vec()
    };
    assert_eq!(run(), run());
}

#[test]
fn a_router_never_forwards_a_broadcast() {
    let mut sim = net(false); // pas de relais
    sim.restart_dhcp().unwrap();
    assert_eq!(addr(&sim, "PC1"), None);
    assert!(matches!(
        node(&sim, "PC1").dhcp_clients.values().next(),
        Some(ClientState::Failed)
    ));
    assert!(
        dhcp_events(&sim)
            .iter()
            .any(|e| matches!(e, DhcpEvent::ClientFailed { attempts: 4 }))
    );
    // PC3, sur le réseau du serveur, a son adresse.
    assert_eq!(addr(&sim, "PC3").as_deref(), Some("10.0.2.100/24"));
    // Aucune diffusion n'est sortie de R1 côté serveur pour PC1.
    let r1 = dev(&sim, "R1");
    let leaked = events(&sim).iter().any(|e| {
        e.device == r1
            && matches!(&e.kind, TraceKind::FrameSent { frame, .. }
                if matches!(&frame.payload, EtherPayload::Ipv4(p) if p.dst == std::net::Ipv4Addr::BROADCAST))
    });
    assert!(!leaked);
}

#[test]
fn an_exhausted_pool_leaves_the_last_client_without_address() {
    let mut sim = net(true);
    let srv = dev(&sim, "SRV");
    sim.set_dhcp_pools(
        srv,
        vec![pool("10.0.1.0/24", "10.0.1.100", "10.0.1.100", "10.0.1.1")],
    )
    .unwrap();
    sim.restart_dhcp().unwrap();
    let got = [addr(&sim, "PC1"), addr(&sim, "PC2")];
    assert_eq!(got.iter().filter(|a| a.is_some()).count(), 1, "{got:?}");
    assert!(
        dhcp_events(&sim)
            .iter()
            .any(|e| matches!(e, DhcpEvent::ServerExhausted { .. }))
    );
    // PC3 : aucune plage pour son réseau.
    assert!(
        dhcp_events(&sim)
            .iter()
            .any(|e| matches!(e, DhcpEvent::ServerNoPool { .. }))
    );
}

#[test]
fn a_stopped_server_then_restarted() {
    let mut sim = net(true);
    let srv = dev(&sim, "SRV");
    sim.set_dhcp_enabled(srv, false).unwrap();
    sim.restart_dhcp().unwrap();
    assert_eq!(addr(&sim, "PC1"), None);
    sim.set_dhcp_enabled(srv, true).unwrap();
    let pc1 = dev(&sim, "PC1");
    sim.command(
        pc1,
        Command::DhcpRenew {
            iface: None,
            fresh: false,
        },
    )
    .unwrap();
    sim.run_until_idle().unwrap();
    assert_eq!(addr(&sim, "PC1").as_deref(), Some("10.0.1.100/24"));
}

#[test]
fn release_then_renew_gives_back_the_same_address() {
    let mut sim = net(true);
    sim.restart_dhcp().unwrap();
    let pc2 = dev(&sim, "PC2");
    sim.command(pc2, Command::DhcpRelease { iface: None })
        .unwrap();
    sim.run_until_idle().unwrap();
    assert_eq!(addr(&sim, "PC2"), None);
    assert_eq!(
        node(&sim, "PC2").gateway,
        None,
        "passerelle apprise oubliée"
    );
    assert!(
        dhcp_events(&sim)
            .iter()
            .any(|e| matches!(e, DhcpEvent::ServerReleased { .. }))
    );
    sim.command(
        pc2,
        Command::DhcpRenew {
            iface: None,
            fresh: false,
        },
    )
    .unwrap();
    sim.run_until_idle().unwrap();
    assert_eq!(addr(&sim, "PC2").as_deref(), Some("10.0.1.101/24"));
}

#[test]
fn a_fresh_clone_keeps_leases_so_probes_need_no_dhcp() {
    let mut sim = net(true);
    sim.restart_dhcp().unwrap();
    let mut probe = sim.fresh_clone();
    assert_eq!(addr(&probe, "PC1").as_deref(), Some("10.0.1.100/24"));
    assert!(ping(&mut probe, "PC1", "10.0.2.10"));
    assert!(
        !probe
            .trace()
            .events()
            .iter()
            .any(|e| matches!(e.kind, TraceKind::Dhcp { .. }))
    );
    // Une machine qui redémarre retrouve sa place, sans conflit.
    probe.restart_dhcp().unwrap();
    assert_eq!(addr(&probe, "PC1").as_deref(), Some("10.0.1.100/24"));
    assert_eq!(addr(&probe, "PC2").as_deref(), Some("10.0.1.101/24"));
}

#[test]
fn dhcp_faults_are_reversible() {
    let mut sim = net(true);
    sim.restart_dhcp().unwrap();
    let before = sim.config_fingerprint();
    let srv = dev(&sim, "SRV");
    let r1 = dev(&sim, "R1");
    let id = sim
        .inject_fault(
            Fault::Multi(vec![
                Fault::DhcpServerDown { device: srv },
                Fault::DhcpRelay {
                    device: r1,
                    iface: sim.iface_id(r1, "eth0").unwrap(),
                    server: None,
                },
                Fault::DnsRecord {
                    device: srv,
                    name: "web.lan".into(),
                    ip: None,
                },
            ]),
            false,
        )
        .unwrap();
    assert_ne!(sim.config_fingerprint(), before);
    sim.repair_fault(id, false).unwrap();
    assert_eq!(sim.config_fingerprint(), before);
}

// ------------------------------------------------------------------ DNS

fn resolve(sim: &mut Simulation, from: &str, name: &str, then: AfterResolve) -> Vec<TraceKind> {
    let start = sim.trace().len();
    sim.command(
        dev(sim, from),
        Command::Resolve {
            name: name.into(),
            then,
        },
    )
    .unwrap();
    sim.run_until_idle().unwrap();
    sim.trace()
        .since(start)
        .iter()
        .map(|e| e.kind.clone())
        .collect()
}

fn outcome(kinds: &[TraceKind]) -> Option<(DnsOutcome, bool)> {
    kinds.iter().find_map(|k| match k {
        TraceKind::DnsResult {
            outcome, cached, ..
        } => Some((outcome.clone(), *cached)),
        _ => None,
    })
}

#[test]
fn a_name_is_resolved_then_pinged_and_cached() {
    let mut sim = net(true);
    sim.restart_dhcp().unwrap();
    let k = resolve(&mut sim, "PC1", "WEB.lan", AfterResolve::Ping { count: 1 });
    assert_eq!(
        outcome(&k),
        Some((DnsOutcome::Resolved(ip("10.0.2.10")), false))
    );
    assert!(
        k.iter().any(|x| matches!(x, TraceKind::DnsAnswered { .. })),
        "le serveur a répondu"
    );
    assert!(
        k.iter().any(|x| matches!(x, TraceKind::PingReply { .. })),
        "puis le ping"
    );
    // Deuxième fois : depuis le cache, sans question au serveur.
    let k = resolve(&mut sim, "PC1", "web.lan", AfterResolve::Ping { count: 1 });
    assert_eq!(
        outcome(&k),
        Some((DnsOutcome::Resolved(ip("10.0.2.10")), true))
    );
    assert!(!k.iter().any(|x| matches!(x, TraceKind::DnsAnswered { .. })));
    // Le message DNS voyage bien dans un datagramme UDP vers le port 53.
    assert!(events(&sim).iter().any(|e| matches!(&e.kind, TraceKind::FrameSent { frame, .. }
        if matches!(&frame.payload, EtherPayload::Ipv4(p)
            if matches!(&p.payload, IpPayload::Udp(u) if u.dst_port == 53 && matches!(u.payload, UdpPayload::Dns(_)))))));
}

#[test]
fn an_unknown_name_is_nxdomain() {
    let mut sim = net(true);
    sim.restart_dhcp().unwrap();
    let k = resolve(&mut sim, "PC1", "mail.lan", AfterResolve::Ping { count: 1 });
    assert_eq!(outcome(&k).unwrap().0, DnsOutcome::NxDomain);
    assert!(
        !k.iter().any(|x| matches!(x, TraceKind::PingSent { .. })),
        "pas de ping sans adresse"
    );
}

#[test]
fn dns_server_down_wrong_server_and_no_server() {
    let mut sim = net(true);
    sim.restart_dhcp().unwrap();
    // Serveur arrêté : le port 53 est fermé, erreur ICMP.
    let srv = dev(&sim, "SRV");
    sim.set_dns_enabled(srv, false).unwrap();
    let k = resolve(&mut sim, "PC2", "web.lan", AfterResolve::Nothing);
    assert_eq!(outcome(&k).unwrap().0, DnsOutcome::Unreachable);
    sim.set_dns_enabled(srv, true).unwrap();
    // Mauvais serveur (personne à cette adresse) : délai dépassé.
    let pc2 = dev(&sim, "PC2");
    sim.set_dns(pc2, Some(ip("10.0.2.99"))).unwrap();
    let k = resolve(&mut sim, "PC2", "web.lan", AfterResolve::Nothing);
    assert_eq!(outcome(&k).unwrap().0, DnsOutcome::Timeout);
    // Le DNS configuré à la main l'emporte sur celui reçu par DHCP ; sans lui, retour au DHCP.
    sim.set_dns(pc2, None).unwrap();
    let k = resolve(&mut sim, "PC2", "web.lan", AfterResolve::Nothing);
    assert_eq!(
        outcome(&k).unwrap().0,
        DnsOutcome::Resolved(ip("10.0.2.10"))
    );
    // Aucun serveur du tout (pas de bail DHCP).
    let mut bare = net(false);
    let k = resolve(&mut bare, "PC1", "web.lan", AfterResolve::Nothing);
    assert_eq!(outcome(&k).unwrap().0, DnsOutcome::NoServer);
}

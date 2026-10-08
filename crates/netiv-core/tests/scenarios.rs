#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Scénarios réseau complets, via l'API publique du moteur.

mod common;

use common::*;
use netiv_core::trace::{DropReason, FloodReason, IcmpErrorKind};
use netiv_core::{DeviceKind, LinkId, LinkParams, SimTime, Simulation, TraceEvent, TraceKind};

// ---------------------------------------------------------------- étape 4

#[test]
fn direct_ping_resolves_arp_then_gets_a_reply() {
    let mut sim = Simulation::new(1);
    add(
        &mut sim,
        "PC1",
        DeviceKind::Host,
        &[("eth0", Some("10.0.0.1/24"))],
    );
    add(
        &mut sim,
        "PC2",
        DeviceKind::Host,
        &[("eth0", Some("10.0.0.2/24"))],
    );
    link(&mut sim, "PC1:eth0", "PC2:eth0");

    ping(&mut sim, "PC1", "10.0.0.2", 1);

    let ok = ping_ok(&sim);
    assert_eq!(ok.len(), 1);
    // Aller-retour ARP (2 ms) + aller-retour ICMP (2 ms).
    assert_eq!(ok[0], (1, 64, SimTime::from_millis(4)));
    assert_eq!(
        count(&sim, |k| matches!(k, TraceKind::ArpRequestSent { .. })),
        1
    );
    assert_eq!(
        count(&sim, |k| matches!(k, TraceKind::ArpReplySent { .. })),
        1
    );
}

#[test]
fn second_ping_is_faster_thanks_to_the_arp_cache() {
    let mut sim = Simulation::new(1);
    add(
        &mut sim,
        "PC1",
        DeviceKind::Host,
        &[("eth0", Some("10.0.0.1/24"))],
    );
    add(
        &mut sim,
        "PC2",
        DeviceKind::Host,
        &[("eth0", Some("10.0.0.2/24"))],
    );
    link(&mut sim, "PC1:eth0", "PC2:eth0");

    ping(&mut sim, "PC1", "10.0.0.2", 3);

    let rtts: Vec<SimTime> = ping_ok(&sim).iter().map(|r| r.2).collect();
    assert_eq!(
        rtts,
        [
            SimTime::from_millis(4),
            SimTime::from_millis(2),
            SimTime::from_millis(2)
        ]
    );
    assert_eq!(
        count(&sim, |k| matches!(k, TraceKind::ArpRequestSent { .. })),
        1
    );
}

#[test]
fn pings_are_spaced_one_second_apart() {
    let mut sim = Simulation::new(1);
    add(
        &mut sim,
        "PC1",
        DeviceKind::Host,
        &[("eth0", Some("10.0.0.1/24"))],
    );
    add(
        &mut sim,
        "PC2",
        DeviceKind::Host,
        &[("eth0", Some("10.0.0.2/24"))],
    );
    link(&mut sim, "PC1:eth0", "PC2:eth0");
    ping(&mut sim, "PC1", "10.0.0.2", 3);
    let sent: Vec<u64> = events(&sim)
        .iter()
        .filter(|e| matches!(e.kind, TraceKind::PingSent { .. }))
        .map(|e| e.time.as_micros())
        .collect();
    assert_eq!(sent, [0, 1_000_000, 2_000_000]);
}

#[test]
fn ping_to_self_is_a_local_loopback() {
    let mut sim = Simulation::new(1);
    add(
        &mut sim,
        "PC1",
        DeviceKind::Host,
        &[("eth0", Some("10.0.0.1/24"))],
    );
    ping(&mut sim, "PC1", "10.0.0.1", 1);
    assert_eq!(ping_ok(&sim), [(1, 64, SimTime::ZERO)]);
    assert_eq!(count(&sim, |k| matches!(k, TraceKind::FrameSent { .. })), 0);
}

// ---------------------------------------------------------------- étape 5

#[test]
fn switch_floods_arp_request_then_forwards_unicast() {
    let mut sim = Simulation::new(1);
    add(
        &mut sim,
        "PC1",
        DeviceKind::Host,
        &[("eth0", Some("10.0.0.1/24"))],
    );
    add(
        &mut sim,
        "PC2",
        DeviceKind::Host,
        &[("eth0", Some("10.0.0.2/24"))],
    );
    add(
        &mut sim,
        "PC3",
        DeviceKind::Host,
        &[("eth0", Some("10.0.0.3/24"))],
    );
    add(
        &mut sim,
        "SW1",
        DeviceKind::Switch,
        &[("p1", None), ("p2", None), ("p3", None), ("p4", None)],
    );
    link(&mut sim, "PC1:eth0", "SW1:p1");
    link(&mut sim, "PC2:eth0", "SW1:p2");
    link(&mut sim, "PC3:eth0", "SW1:p3");

    ping(&mut sim, "PC1", "10.0.0.2", 1);

    assert_eq!(ping_ok(&sim).len(), 1);
    // Un seul flood : la requête ARP (broadcast). Le port p4 n'a pas de câble.
    let floods: Vec<_> = events(&sim)
        .iter()
        .filter_map(|e| match &e.kind {
            TraceKind::Flood { reason, out_ifaces } => Some((*reason, out_ifaces.len())),
            _ => None,
        })
        .collect();
    assert_eq!(floods, [(FloodReason::Broadcast, 2)]);
    // Deux MAC apprises (PC1 puis PC2), rien d'autre.
    assert_eq!(
        count(&sim, |k| matches!(k, TraceKind::MacLearned { .. })),
        2
    );
    // PC3 a reçu la requête ARP sans être concerné.
    let pc3 = sim.device_id("PC3").unwrap();
    assert!(sim.trace().by_device(pc3).any(|e| matches!(
        e.kind,
        TraceKind::Dropped {
            reason: DropReason::NotForUs
        }
    )));
    // Tout le reste a été transmis en unicast.
    assert!(count(&sim, |k| matches!(k, TraceKind::Forward { .. })) >= 3);
}

#[test]
fn switch_floods_unknown_unicast() {
    let mut sim = Simulation::new(1);
    add(
        &mut sim,
        "PC1",
        DeviceKind::Host,
        &[("eth0", Some("10.0.0.1/24"))],
    );
    add(
        &mut sim,
        "PC2",
        DeviceKind::Host,
        &[("eth0", Some("10.0.0.2/24"))],
    );
    add(
        &mut sim,
        "PC3",
        DeviceKind::Host,
        &[("eth0", Some("10.0.0.3/24"))],
    );
    add(
        &mut sim,
        "SW1",
        DeviceKind::Switch,
        &[("p1", None), ("p2", None), ("p3", None)],
    );
    link(&mut sim, "PC1:eth0", "SW1:p1");
    link(&mut sim, "PC2:eth0", "SW1:p2");
    link(&mut sim, "PC3:eth0", "SW1:p3");
    // Le cache ARP de PC1 connaît PC3 mais le switch n'a jamais vu PC3 parler.
    ping(&mut sim, "PC1", "10.0.0.3", 1);
    ping(&mut sim, "PC3", "10.0.0.1", 1);
    // Les deux pings aboutissent malgré l'ordre d'apprentissage.
    assert_eq!(ping_ok(&sim).len(), 2);
}

// ---------------------------------------------------------------- étape 7

#[test]
fn routed_ping_decrements_ttl_at_each_router() {
    let mut sim = mvp();
    ping(&mut sim, "PC1", "192.168.2.10", 1);

    // La réponse part de SERVER avec TTL 64 et traverse R2 puis R1.
    // 8 traversées de lien pour l'écho + 3 résolutions ARP (un aller-retour par
    // segment : 4 + 2 + 2 ms) = 16 ms au total.
    assert_eq!(ping_ok(&sim), [(1, 62, SimTime::from_millis(16))]);

    let decs: Vec<(u8, u8)> = events(&sim)
        .iter()
        .filter_map(|e| match e.kind {
            TraceKind::TtlDecremented { from, to } => Some((from, to)),
            _ => None,
        })
        .collect();
    // Requête : R1 64→63, R2 63→62. Réponse : R2 64→63, R1 63→62.
    assert_eq!(decs, [(64, 63), (63, 62), (64, 63), (63, 62)]);
}

#[test]
fn routed_ping_arps_on_each_segment_only_once() {
    // Pas de requête ARP de SERVER vers R2 : SERVER a déjà appris la MAC de R2
    // en recevant la requête ARP de R2 (RFC 826).
    let mut sim = mvp();
    ping(&mut sim, "PC1", "192.168.2.10", 2);
    let arps: Vec<String> = events(&sim)
        .iter()
        .filter_map(|e| match &e.kind {
            TraceKind::ArpRequestSent { target, .. } => {
                Some(format!("{}→{}", sim.device(e.device).unwrap().name, target))
            }
            _ => None,
        })
        .collect();
    assert_eq!(arps, ["PC1→192.168.1.1", "R1→10.0.0.2", "R2→192.168.2.10"]);
    assert_eq!(ping_ok(&sim).len(), 2);
}

#[test]
fn route_lookup_trace_explains_the_choice() {
    let mut sim = mvp();
    ping(&mut sim, "PC1", "192.168.2.10", 1);
    let r1 = sim.device_id("R1").unwrap();
    let decision = sim
        .trace()
        .by_device(r1)
        .find_map(|e| match &e.kind {
            TraceKind::RouteLookup(d) if d.dst == ip("192.168.2.10") => Some(d.clone()),
            _ => None,
        })
        .expect("R1 doit consulter sa table");
    // R1 a 2 routes connectées + 1 statique : trois candidates, une seule retenue.
    assert_eq!(decision.candidates.len(), 3);
    assert_eq!(decision.candidates.iter().filter(|c| c.matches).count(), 1);
    let chosen = decision.chosen_route().unwrap();
    assert_eq!(chosen.dest.to_string(), "192.168.2.0/24");
    assert_eq!(chosen.next_hop, Some(ip("10.0.0.2")));
}

#[test]
fn router_without_route_answers_net_unreachable() {
    let mut sim = mvp();
    let r1 = sim.device_id("R1").unwrap();
    sim.remove_route(r1, "192.168.2.0/24".parse().unwrap())
        .unwrap();
    ping(&mut sim, "PC1", "192.168.2.10", 1);
    assert!(ping_ok(&sim).is_empty());
    assert_eq!(
        count(&sim, |k| matches!(
            k,
            TraceKind::PingError {
                error: IcmpErrorKind::NetUnreachable,
                ..
            }
        )),
        1
    );
    assert_eq!(
        count(&sim, |k| matches!(k, TraceKind::PingTimeout { .. })),
        0
    );
}

#[test]
fn routing_loop_ends_with_ttl_expired_error() {
    let mut sim = Simulation::new(1);
    let pc = add(
        &mut sim,
        "PC1",
        DeviceKind::Host,
        &[("eth0", Some("10.1.0.10/24"))],
    );
    let r1 = add(
        &mut sim,
        "R1",
        DeviceKind::Router,
        &[("eth0", Some("10.1.0.1/24")), ("eth1", Some("10.0.0.1/30"))],
    );
    let r2 = add(
        &mut sim,
        "R2",
        DeviceKind::Router,
        &[("eth0", Some("10.0.0.2/30"))],
    );
    gw(&mut sim, pc, "10.1.0.1");
    route(&mut sim, r1, "10.9.0.0/16", "10.0.0.2");
    route(&mut sim, r2, "10.9.0.0/16", "10.0.0.1");
    // Route de retour : sans elle, R2 ne pourrait pas renvoyer l'erreur ICMP.
    route(&mut sim, r2, "10.1.0.0/24", "10.0.0.1");
    link(&mut sim, "PC1:eth0", "R1:eth0");
    link(&mut sim, "R1:eth1", "R2:eth0");

    ping(&mut sim, "PC1", "10.9.9.9", 1);

    assert_eq!(count(&sim, |k| matches!(k, TraceKind::TtlExpired)), 1);
    assert_eq!(
        count(&sim, |k| matches!(
            k,
            TraceKind::PingError {
                error: IcmpErrorKind::TtlExpired,
                ..
            }
        )),
        1
    );
}

// ---------------------------------------------------------------- pannes

#[test]
fn host_without_route_drops_locally() {
    let mut sim = Simulation::new(1);
    add(
        &mut sim,
        "PC1",
        DeviceKind::Host,
        &[("eth0", Some("10.0.0.1/24"))],
    );
    ping(&mut sim, "PC1", "8.8.8.8", 1);
    assert_eq!(
        count(&sim, |k| matches!(
            k,
            TraceKind::Dropped {
                reason: DropReason::NoRoute
            }
        )),
        1
    );
    assert_eq!(
        count(&sim, |k| matches!(k, TraceKind::PingTimeout { .. })),
        1
    );
    assert_eq!(count(&sim, |k| matches!(k, TraceKind::FrameSent { .. })), 0);
}

#[test]
fn wrong_gateway_ends_in_arp_timeout() {
    let mut sim = mvp();
    let pc1 = sim.device_id("PC1").unwrap();
    // 192.168.1.254 est dans le bon réseau mais personne ne l'a.
    gw(&mut sim, pc1, "192.168.1.254");
    ping(&mut sim, "PC1", "192.168.2.10", 1);

    assert!(ping_ok(&sim).is_empty());
    assert_eq!(
        count(&sim, |k| matches!(k, TraceKind::ArpRequestSent { .. })),
        3
    );
    assert_eq!(
        count(&sim, |k| matches!(
            k,
            TraceKind::ArpTimeout { dropped: 1, .. }
        )),
        1
    );
    assert_eq!(
        count(&sim, |k| matches!(
            k,
            TraceKind::Dropped {
                reason: DropReason::ArpUnresolved
            }
        )),
        1
    );
}

#[test]
fn gateway_outside_the_subnet_means_no_default_route() {
    let mut sim = mvp();
    let pc1 = sim.device_id("PC1").unwrap();
    gw(&mut sim, pc1, "192.168.10.254");
    ping(&mut sim, "PC1", "192.168.2.10", 1);
    let decision = events(&sim)
        .iter()
        .find_map(|e| match &e.kind {
            TraceKind::RouteLookup(d) => Some(d.clone()),
            _ => None,
        })
        .unwrap();
    assert!(decision.chosen.is_none());
    assert_eq!(decision.inactive.len(), 1);
}

#[test]
fn wrong_mask_makes_a_remote_host_look_local() {
    // Scénario du cahier des charges : /16 au lieu de /24.
    let mut sim = mvp();
    let pc1 = sim.device_id("PC1").unwrap();
    let eth0 = sim.iface_id(pc1, "eth0").unwrap();
    sim.set_ip(pc1, eth0, Some("192.168.1.10/16".parse().unwrap()))
        .unwrap();
    ping(&mut sim, "PC1", "192.168.2.10", 1);
    // PC1 croit SERVER local : il fait de l'ARP pour 192.168.2.10 directement.
    assert!(events(&sim).iter().any(|e| matches!(
        e.kind,
        TraceKind::ArpRequestSent { target, .. } if target == ip("192.168.2.10")
    )));
    assert!(ping_ok(&sim).is_empty());
}

#[test]
fn cut_cable_drops_frames_and_ping_times_out() {
    let mut sim = mvp();
    sim.set_link_up(LinkId(1), false).unwrap(); // SW1 ↔ R1
    ping(&mut sim, "PC1", "192.168.2.10", 1);
    assert!(ping_ok(&sim).is_empty());
    assert!(
        count(&sim, |k| matches!(
            k,
            TraceKind::Dropped {
                reason: DropReason::LinkDown
            }
        )) >= 1
    );
    assert_eq!(
        count(&sim, |k| matches!(k, TraceKind::PingTimeout { .. })),
        1
    );
}

#[test]
fn disabled_interface_blocks_traffic() {
    let mut sim = mvp();
    let r1 = sim.device_id("R1").unwrap();
    let eth1 = sim.iface_id(r1, "eth1").unwrap();
    sim.set_iface_admin(r1, eth1, false).unwrap();
    ping(&mut sim, "PC1", "192.168.2.10", 1);
    assert!(ping_ok(&sim).is_empty());
    // Sans interface eth1, R1 n'a plus de route vers 192.168.2.0/24 :
    // la route statique est devenue inutilisable → net unreachable.
    assert_eq!(
        count(&sim, |k| matches!(
            k,
            TraceKind::PingError {
                error: IcmpErrorKind::NetUnreachable,
                ..
            }
        )),
        1
    );
}

#[test]
fn total_loss_link_loses_every_frame() {
    let mut sim = Simulation::new(1);
    add(
        &mut sim,
        "PC1",
        DeviceKind::Host,
        &[("eth0", Some("10.0.0.1/24"))],
    );
    add(
        &mut sim,
        "PC2",
        DeviceKind::Host,
        &[("eth0", Some("10.0.0.2/24"))],
    );
    link_with(
        &mut sim,
        "PC1:eth0",
        "PC2:eth0",
        LinkParams {
            loss: 1.0,
            ..LinkParams::default()
        },
    );
    ping(&mut sim, "PC1", "10.0.0.2", 1);
    assert!(ping_ok(&sim).is_empty());
    assert!(
        count(&sim, |k| matches!(
            k,
            TraceKind::Dropped {
                reason: DropReason::LinkLoss
            }
        )) >= 1
    );
}

// ---------------------------------------------------------------- déterminisme

fn lossy_run(seed: u64) -> Vec<TraceEvent> {
    let mut sim = Simulation::new(seed);
    add(
        &mut sim,
        "PC1",
        DeviceKind::Host,
        &[("eth0", Some("10.0.0.1/24"))],
    );
    add(
        &mut sim,
        "PC2",
        DeviceKind::Host,
        &[("eth0", Some("10.0.0.2/24"))],
    );
    link_with(
        &mut sim,
        "PC1:eth0",
        "PC2:eth0",
        LinkParams {
            loss: 0.5,
            ..LinkParams::default()
        },
    );
    ping(&mut sim, "PC1", "10.0.0.2", 20);
    sim.trace().events().to_vec()
}

#[test]
fn same_seed_gives_identical_traces() {
    assert_eq!(lossy_run(7), lossy_run(7));
}

#[test]
fn different_seeds_change_which_frames_are_lost() {
    assert_ne!(lossy_run(7), lossy_run(8));
}

#[test]
fn full_run_is_reproducible() {
    let run = || {
        let mut sim = mvp();
        ping(&mut sim, "PC1", "192.168.2.10", 4);
        sim.trace().events().to_vec()
    };
    assert_eq!(run(), run());
}

#[test]
fn causality_links_point_backwards() {
    let mut sim = mvp();
    ping(&mut sim, "PC1", "192.168.2.10", 1);
    for e in events(&sim) {
        if let Some(c) = e.caused_by {
            assert!(c < e.seq, "l'événement {} dit être causé par {c}", e.seq);
        }
        assert_eq!(
            e.seq as usize,
            events(&sim).iter().position(|x| x.seq == e.seq).unwrap()
        );
    }
}

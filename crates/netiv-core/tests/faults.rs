#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Pannes réversibles : injection, effet observable, réparation exacte.

mod common;

use common::*;
use netiv_core::acl::{AclAction, AclRule, RuleProto};
use netiv_core::packet::L4Proto;
use netiv_core::service::Service;
use netiv_core::switch::PortMode;
use netiv_core::{Fault, LinkId, MacAddr, SimTime, Simulation, StaticRoute, TraceKind};

fn lab() -> Simulation {
    let mut sim = mvp();
    let srv = dev(&sim, "SERVER");
    sim.add_service(srv, Service::tcp(80, "http")).unwrap();
    sim.clear_trace();
    sim
}

/// Injecte, vérifie que « avant OK / pendant KO », répare, vérifie « OK ».
fn breaks_then_heals(fault: impl FnOnce(&Simulation) -> Fault) {
    let mut sim = lab();
    ping(&mut sim, "PC1", "192.168.2.10", 1);
    assert_eq!(ping_ok(&sim).len(), 1, "le réseau sain doit répondre");
    let healthy = sim.config_fingerprint();

    let f = fault(&sim);
    let id = sim.inject_fault(f, false).unwrap();
    assert_ne!(
        sim.config_fingerprint(),
        healthy,
        "la panne doit changer la config"
    );
    let before = ping_ok(&sim).len();
    sim.reset_runtime();
    ping(&mut sim, "PC1", "192.168.2.10", 1);
    assert_eq!(
        ping_ok(&sim).len(),
        0,
        "pendant la panne, plus de réponse (avant : {before})"
    );

    sim.repair_fault(id, false).unwrap();
    assert_eq!(
        sim.config_fingerprint(),
        healthy,
        "la réparation doit restaurer exactement la configuration"
    );
    sim.reset_runtime();
    ping(&mut sim, "PC1", "192.168.2.10", 1);
    assert_eq!(ping_ok(&sim).len(), 1, "après réparation, le réseau répond");
}

#[test]
fn link_down() {
    breaks_then_heals(|_| Fault::LinkDown { link: LinkId(2) });
}

#[test]
fn interface_down() {
    breaks_then_heals(|s| {
        let r1 = dev(s, "R1");
        Fault::InterfaceDown {
            device: r1,
            iface: s.iface_id(r1, "eth1").unwrap(),
        }
    });
}

#[test]
fn total_loss() {
    breaks_then_heals(|_| Fault::LinkLoss {
        link: LinkId(2),
        loss: 1.0,
    });
}

#[test]
fn wrong_ip_on_the_source() {
    breaks_then_heals(|s| {
        let pc = dev(s, "PC1");
        Fault::SetIp {
            device: pc,
            iface: s.iface_id(pc, "eth0").unwrap(),
            ip: Some("10.99.0.10/24".parse().unwrap()),
        }
    });
}

#[test]
fn wrong_mask_on_the_source() {
    breaks_then_heals(|s| {
        let pc = dev(s, "PC1");
        Fault::SetIp {
            device: pc,
            iface: s.iface_id(pc, "eth0").unwrap(),
            ip: Some("192.168.1.10/16".parse().unwrap()),
        }
    });
}

#[test]
fn wrong_gateway() {
    breaks_then_heals(|s| Fault::SetGateway {
        device: dev(s, "PC1"),
        gateway: Some(ip("192.168.1.254")),
    });
}

#[test]
fn missing_route_on_a_router() {
    breaks_then_heals(|s| Fault::RemoveRoutes {
        device: dev(s, "R1"),
        dest: "192.168.2.0/24".parse().unwrap(),
    });
}

#[test]
fn routing_loop_between_two_routers() {
    // Une route plus spécifique (/32) l'emporte sur la route connectée /24 :
    // R2 renvoie vers R1 les paquets pour SERVER, que R1 renvoie vers R2...
    breaks_then_heals(|s| Fault::AddRoute {
        device: dev(s, "R2"),
        route: StaticRoute {
            dest: "192.168.2.10/32".parse().unwrap(),
            via: ip("10.0.0.1"),
        },
    });
}

#[test]
fn forwarding_disabled_on_a_router() {
    breaks_then_heals(|s| Fault::ForwardingOff {
        device: dev(s, "R1"),
    });
}

#[test]
fn wrong_static_arp_entry() {
    breaks_then_heals(|s| Fault::StaticArp {
        device: dev(s, "PC1"),
        ip: ip("192.168.1.1"),
        mac: "02:00:00:00:be:ef".parse::<MacAddr>().unwrap(),
    });
}

#[test]
fn wrong_vlan_on_an_access_port() {
    breaks_then_heals(|s| {
        let sw = dev(s, "SW1");
        Fault::PortMode {
            device: sw,
            iface: s.iface_id(sw, "port1").unwrap(),
            mode: PortMode::Access { vlan: 99 },
        }
    });
}

#[test]
fn acl_rule_blocking_icmp() {
    breaks_then_heals(|s| Fault::AclInsert {
        device: dev(s, "R2"),
        rule: AclRule::new(AclAction::Deny, RuleProto::Icmp),
    });
}

#[test]
fn acl_default_deny() {
    breaks_then_heals(|s| Fault::AclDefault {
        device: dev(s, "R1"),
        action: AclAction::Deny,
    });
}

#[test]
fn stopping_a_service_makes_curl_refused() {
    let mut sim = lab();
    let healthy = sim.config_fingerprint();
    let id = sim
        .inject_fault(
            Fault::StopService {
                device: dev(&sim, "SERVER"),
                proto: L4Proto::Tcp,
                port: 80,
            },
            false,
        )
        .unwrap();
    curl(&mut sim, "PC1", "192.168.2.10", 80);
    assert_eq!(
        curl_results(&sim),
        [netiv_core::trace::CurlOutcome::Refused]
    );
    sim.repair_fault(id, false).unwrap();
    assert_eq!(sim.config_fingerprint(), healthy);
    curl(&mut sim, "PC1", "192.168.2.10", 80);
    assert!(matches!(
        curl_results(&sim)[1],
        netiv_core::trace::CurlOutcome::Ok { .. }
    ));
}

#[test]
fn latency_fault_slows_but_does_not_break() {
    let mut sim = lab();
    sim.inject_fault(
        Fault::LinkLatency {
            link: LinkId(2),
            latency: SimTime::from_millis(200),
        },
        false,
    )
    .unwrap();
    ping(&mut sim, "PC1", "192.168.2.10", 2);
    let ok = ping_ok(&sim);
    assert_eq!(ok.len(), 2);
    assert!(ok[1].2 >= SimTime::from_millis(400), "{}", ok[1].2);
}

// ------------------------------------------------------------------ mécanique

#[test]
fn silent_injection_leaves_no_trace_but_public_injection_does() {
    let mut silent = lab();
    silent
        .inject_fault(Fault::LinkDown { link: LinkId(0) }, false)
        .unwrap();
    assert!(
        silent.trace().is_empty(),
        "un challenge ne doit rien révéler"
    );

    let mut public = lab();
    public
        .inject_fault(Fault::LinkDown { link: LinkId(0) }, true)
        .unwrap();
    assert!(matches!(
        public.trace().events()[0].kind,
        TraceKind::FaultApplied { .. }
    ));
    public.repair_fault(1, true).unwrap();
    assert!(matches!(
        public.trace().events()[1].kind,
        TraceKind::FaultRepaired { .. }
    ));
}

#[test]
fn faults_are_numbered_and_described() {
    let mut sim = lab();
    let a = sim
        .inject_fault(
            Fault::SetGateway {
                device: dev(&sim, "PC1"),
                gateway: Some(ip("192.168.1.254")),
            },
            false,
        )
        .unwrap();
    let b = sim
        .inject_fault(Fault::LinkDown { link: LinkId(1) }, false)
        .unwrap();
    assert_eq!((a, b), (1, 2));
    assert_eq!(
        sim.faults()[0].description,
        "PC1 : passerelle 192.168.1.254 au lieu de 192.168.1.1"
    );
    assert!(sim.faults()[1].description.contains("SW1/port2 ↔ R1/eth0"));
    assert!(sim.faults().iter().all(|f| f.active));
}

#[test]
fn repairing_twice_or_an_unknown_fault_is_an_error() {
    let mut sim = lab();
    let id = sim
        .inject_fault(Fault::LinkDown { link: LinkId(0) }, false)
        .unwrap();
    sim.repair_fault(id, false).unwrap();
    assert!(sim.repair_fault(id, false).is_err());
    assert!(sim.repair_fault(99, false).is_err());
}

#[test]
fn repair_all_restores_the_original_configuration() {
    let mut sim = lab();
    let healthy = sim.config_fingerprint();
    sim.inject_fault(Fault::LinkDown { link: LinkId(0) }, false)
        .unwrap();
    sim.inject_fault(
        Fault::SetGateway {
            device: dev(&sim, "PC1"),
            gateway: None,
        },
        false,
    )
    .unwrap();
    sim.inject_fault(
        Fault::ForwardingOff {
            device: dev(&sim, "R2"),
        },
        false,
    )
    .unwrap();
    assert_eq!(sim.repair_all_faults(false), 3);
    assert_eq!(sim.config_fingerprint(), healthy);
    assert_eq!(sim.repair_all_faults(false), 0);
}

#[test]
fn invalid_faults_are_rejected_without_side_effects() {
    let mut sim = lab();
    let healthy = sim.config_fingerprint();
    // Un switch n'a pas de passerelle ; une interface inconnue ; un lien inconnu.
    assert!(
        sim.inject_fault(
            Fault::SetGateway {
                device: dev(&sim, "SW1"),
                gateway: None
            },
            false
        )
        .is_err()
    );
    assert!(
        sim.inject_fault(
            Fault::InterfaceDown {
                device: dev(&sim, "PC1"),
                iface: netiv_core::ids::IfaceId(9)
            },
            false
        )
        .is_err()
    );
    assert!(
        sim.inject_fault(Fault::LinkDown { link: LinkId(77) }, false)
            .is_err()
    );
    assert!(
        sim.inject_fault(
            Fault::LinkLoss {
                link: LinkId(0),
                loss: 3.0
            },
            false
        )
        .is_err()
    );
    assert!(
        sim.inject_fault(
            Fault::PortMode {
                device: dev(&sim, "PC1"),
                iface: netiv_core::ids::IfaceId(0),
                mode: PortMode::Access { vlan: 2 }
            },
            false
        )
        .is_err()
    );
    // Une panne composite dont un élément est invalide ne doit rien changer.
    assert!(
        sim.inject_fault(
            Fault::Multi(vec![
                Fault::LinkDown { link: LinkId(0) },
                Fault::LinkDown { link: LinkId(77) }
            ]),
            false
        )
        .is_err()
    );
    assert_eq!(sim.config_fingerprint(), healthy);
    assert!(sim.faults().is_empty());
}

#[test]
fn scheduled_fault_strikes_in_the_middle_of_a_ping_series() {
    let mut sim = lab();
    sim.schedule_fault(
        SimTime::from_millis(2_500),
        Fault::LinkDown { link: LinkId(2) },
        false,
    )
    .unwrap();
    ping(&mut sim, "PC1", "192.168.2.10", 5);
    // Pings à t = 0, 1, 2 s réussissent ; à partir de 3 s le câble est coupé.
    assert_eq!(ping_ok(&sim).len(), 3);
    assert_eq!(
        count(&sim, |k| matches!(k, TraceKind::PingTimeout { .. })),
        2
    );
    assert_eq!(sim.faults().len(), 1);
}

#[test]
fn scheduling_an_invalid_fault_fails_immediately() {
    let mut sim = lab();
    assert!(
        sim.schedule_fault(
            SimTime::from_millis(1),
            Fault::LinkDown { link: LinkId(50) },
            false
        )
        .is_err()
    );
}

#[test]
fn fresh_clone_keeps_configuration_but_drops_runtime_state() {
    let mut sim = lab();
    ping(&mut sim, "PC1", "192.168.2.10", 1);
    assert!(!sim.trace().is_empty());
    let fresh = sim.fresh_clone();
    assert!(fresh.trace().is_empty());
    assert_eq!(fresh.pending(), 0);
    assert_eq!(fresh.now(), SimTime::ZERO);
    assert_eq!(fresh.config_fingerprint(), sim.config_fingerprint());
    let pc = fresh.device(dev(&fresh, "PC1")).unwrap();
    let netiv_core::device::Behavior::Ip(node) = &pc.behavior else {
        panic!()
    };
    assert!(node.arp_cache.is_empty());
    // L'original n'a pas été touché.
    let pc0 = sim.device(dev(&sim, "PC1")).unwrap();
    let netiv_core::device::Behavior::Ip(node0) = &pc0.behavior else {
        panic!()
    };
    assert!(!node0.arp_cache.is_empty());
}

#[test]
fn a_fresh_clone_is_independent_of_the_original() {
    let sim = lab();
    let mut fresh = sim.fresh_clone();
    ping(&mut fresh, "PC1", "192.168.2.10", 1);
    assert_eq!(ping_ok(&fresh).len(), 1);
    assert!(sim.trace().is_empty());
}

#[test]
fn fault_runs_are_deterministic() {
    let run = || {
        let mut sim = lab();
        sim.inject_fault(
            Fault::LinkLoss {
                link: LinkId(0),
                loss: 0.5,
            },
            true,
        )
        .unwrap();
        ping(&mut sim, "PC1", "192.168.2.10", 8);
        sim.trace().events().to_vec()
    };
    assert_eq!(run(), run());
}

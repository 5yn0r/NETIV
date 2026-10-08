#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Chaque panne du catalogue doit être retrouvée par le diagnostic.

use std::net::Ipv4Addr;

use netiv_core::acl::{AclAction, AclRule, RuleProto};
use netiv_core::packet::L4Proto;
use netiv_core::service::Service;
use netiv_core::switch::PortMode;
use netiv_core::{DeviceId, Fault, LinkId, MacAddr, SimTime, Simulation, StaticRoute};
use netiv_diagnostics::{CauseKind, CheckStatus, Request, Status, diagnose, hints_for};

fn lab() -> Simulation {
    let yaml = include_str!("../../../examples/mvp.yaml");
    let mut sim = netiv_topology::load_yaml(yaml).unwrap().sim;
    let srv = sim.device_id("SERVER").unwrap();
    sim.add_service(srv, Service::tcp(80, "http")).unwrap();
    sim.clear_trace();
    sim
}

fn dev(sim: &Simulation, n: &str) -> DeviceId {
    sim.device_id(n).unwrap()
}

fn ip(s: &str) -> Ipv4Addr {
    s.parse().unwrap()
}

fn ping_req(sim: &Simulation) -> Request {
    Request {
        src: dev(sim, "PC1"),
        dst: ip("192.168.2.10"),
        service: None,
    }
}

fn web_req(sim: &Simulation) -> Request {
    Request {
        src: dev(sim, "PC1"),
        dst: ip("192.168.2.10"),
        service: Some((L4Proto::Tcp, 80)),
    }
}

/// Injecte `fault`, diagnostique avec la copie saine, vérifie cause et équipement.
fn expect(
    fault: impl FnOnce(&Simulation) -> Fault,
    req: impl FnOnce(&Simulation) -> Request,
    kind: CauseKind,
    device: &str,
) -> netiv_diagnostics::Diagnosis {
    let mut sim = lab();
    let baseline = sim.clone();
    let f = fault(&sim);
    sim.inject_fault(f, false).unwrap();
    let r = req(&sim);
    let d = diagnose(&sim, Some(&baseline), &r).unwrap();
    let cause = d.cause.clone().expect("une cause doit être trouvée");
    assert_eq!(
        cause.kind,
        kind,
        "{}\n{}",
        d.render(true, true),
        cause.detail
    );
    assert_eq!(cause.device, device, "{}", d.render(true, true));
    d
}

#[test]
fn healthy_network_is_reported_healthy() {
    let sim = lab();
    for req in [ping_req(&sim), web_req(&sim)] {
        let d = diagnose(&sim, None, &req).unwrap();
        assert_eq!(d.status, Status::Healthy);
        assert!(d.cause.is_none());
        assert!(d.checks.iter().all(|c| c.status == CheckStatus::Ok));
    }
}

#[test]
fn diagnosing_never_modifies_the_network() {
    let mut sim = lab();
    sim.inject_fault(Fault::LinkDown { link: LinkId(2) }, false)
        .unwrap();
    let (fp, trace_len, pending) = (sim.config_fingerprint(), sim.trace().len(), sim.pending());
    let _ = diagnose(&sim, None, &ping_req(&sim)).unwrap();
    assert_eq!(sim.config_fingerprint(), fp);
    assert_eq!(sim.trace().len(), trace_len);
    assert_eq!(sim.pending(), pending);
}

// ---------------------------------------------------------------- couche 1

#[test]
fn cut_cable() {
    expect(
        |_| Fault::LinkDown { link: LinkId(2) },
        ping_req,
        CauseKind::LinkDown,
        "R1",
    );
}

#[test]
fn disabled_transit_interface() {
    expect(
        |s| {
            let r = dev(s, "R1");
            Fault::InterfaceDown {
                device: r,
                iface: s.iface_id(r, "eth1").unwrap(),
            }
        },
        ping_req,
        CauseKind::InterfaceDown,
        "R1",
    );
}

#[test]
fn disabled_source_interface() {
    expect(
        |s| {
            let p = dev(s, "PC1");
            Fault::InterfaceDown {
                device: p,
                iface: s.iface_id(p, "eth0").unwrap(),
            }
        },
        ping_req,
        CauseKind::InterfaceDown,
        "PC1",
    );
}

#[test]
fn disabled_destination_interface() {
    expect(
        |s| {
            let p = dev(s, "SERVER");
            Fault::InterfaceDown {
                device: p,
                iface: s.iface_id(p, "eth0").unwrap(),
            }
        },
        ping_req,
        CauseKind::InterfaceDown,
        "SERVER",
    );
}

#[test]
fn lossy_link() {
    let d = expect(
        |_| Fault::LinkLoss {
            link: LinkId(2),
            loss: 0.6,
        },
        ping_req,
        CauseKind::PacketLoss,
        "R1",
    );
    assert_eq!(d.cause.unwrap().actual.as_deref(), Some("60 % de perte"));
}

#[test]
fn slow_link_is_degraded_not_broken() {
    let d = expect(
        |_| Fault::LinkLatency {
            link: LinkId(2),
            latency: SimTime::from_millis(300),
        },
        ping_req,
        CauseKind::HighLatency,
        "R1",
    );
    assert_eq!(d.status, Status::Degraded);
    assert!(d.probe.success, "le ping aboutit malgré tout");
}

// ---------------------------------------------------------------- couche 2

#[test]
fn wrong_vlan_names_the_port_and_the_expected_mode() {
    let d = expect(
        |s| {
            let sw = dev(s, "SW1");
            Fault::PortMode {
                device: sw,
                iface: s.iface_id(sw, "port1").unwrap(),
                mode: PortMode::Access { vlan: 99 },
            }
        },
        ping_req,
        CauseKind::WrongVlan,
        "SW1",
    );
    let c = d.cause.unwrap();
    assert_eq!(c.expected.as_deref(), Some("accès VLAN 1"));
    assert_eq!(c.fix.as_deref(), Some("port SW1 port1 access 1"));
}

#[test]
fn wrong_static_arp_entry() {
    let d = expect(
        |s| Fault::StaticArp {
            device: dev(s, "PC1"),
            ip: ip("192.168.1.1"),
            mac: "02:00:00:00:be:ef".parse::<MacAddr>().unwrap(),
        },
        ping_req,
        CauseKind::ArpMismatch,
        "PC1",
    );
    assert_eq!(
        d.cause.unwrap().fix.as_deref(),
        Some("arp PC1 del 192.168.1.1")
    );
}

// ---------------------------------------------------------------- couche 3

#[test]
fn wrong_ip_off_subnet() {
    let d = expect(
        |s| {
            let p = dev(s, "PC1");
            Fault::SetIp {
                device: p,
                iface: s.iface_id(p, "eth0").unwrap(),
                ip: Some("10.99.0.10/24".parse().unwrap()),
            }
        },
        ping_req,
        CauseKind::WrongIp,
        "PC1",
    );
    let c = d.cause.unwrap();
    assert_eq!(c.expected.as_deref(), Some("192.168.1.10/24"));
    assert_eq!(c.fix.as_deref(), Some("ip PC1 eth0 192.168.1.10/24"));
}

#[test]
fn mask_too_big_makes_a_remote_host_look_local() {
    let d = expect(
        |s| {
            let p = dev(s, "PC1");
            Fault::SetIp {
                device: p,
                iface: s.iface_id(p, "eth0").unwrap(),
                ip: Some("192.168.1.10/16".parse().unwrap()),
            }
        },
        ping_req,
        CauseKind::WrongMask,
        "PC1",
    );
    assert_eq!(
        d.cause.unwrap().expected.as_deref(),
        Some("192.168.1.10/24")
    );
}

#[test]
fn mask_too_small_hides_the_gateway() {
    expect(
        |s| {
            let p = dev(s, "PC1");
            Fault::SetIp {
                device: p,
                iface: s.iface_id(p, "eth0").unwrap(),
                ip: Some("192.168.1.10/30".parse().unwrap()),
            }
        },
        ping_req,
        CauseKind::WrongMask,
        "PC1",
    );
}

#[test]
fn gateway_pointing_to_nobody() {
    let d = expect(
        |s| Fault::SetGateway {
            device: dev(s, "PC1"),
            gateway: Some(ip("192.168.1.254")),
        },
        ping_req,
        CauseKind::WrongGateway,
        "PC1",
    );
    let c = d.cause.unwrap();
    assert_eq!(c.actual.as_deref(), Some("192.168.1.254"));
    assert_eq!(c.expected.as_deref(), Some("192.168.1.1"));
    assert_eq!(c.fix.as_deref(), Some("gateway PC1 192.168.1.1"));
}

#[test]
fn gateway_outside_the_subnet() {
    expect(
        |s| Fault::SetGateway {
            device: dev(s, "PC1"),
            gateway: Some(ip("192.168.10.254")),
        },
        ping_req,
        CauseKind::WrongGateway,
        "PC1",
    );
}

#[test]
fn no_gateway_at_all() {
    let d = expect(
        |s| Fault::SetGateway {
            device: dev(s, "PC1"),
            gateway: None,
        },
        ping_req,
        CauseKind::MissingRoute,
        "PC1",
    );
    assert_eq!(
        d.cause.unwrap().fix.as_deref(),
        Some("gateway PC1 192.168.1.1")
    );
}

#[test]
fn missing_route_on_a_transit_router() {
    let d = expect(
        |s| Fault::RemoveRoutes {
            device: dev(s, "R1"),
            dest: "192.168.2.0/24".parse().unwrap(),
        },
        ping_req,
        CauseKind::MissingRoute,
        "R1",
    );
    let c = d.cause.unwrap();
    assert!(!c.on_return_path);
    assert_eq!(
        c.fix.as_deref(),
        Some("route add R1 192.168.2.0/24 via 10.0.0.2")
    );
}

#[test]
fn missing_return_route_is_flagged_as_return_path() {
    let d = expect(
        |s| Fault::RemoveRoutes {
            device: dev(s, "R2"),
            dest: "192.168.1.0/24".parse().unwrap(),
        },
        ping_req,
        CauseKind::MissingRoute,
        "R2",
    );
    assert!(
        d.cause.unwrap().on_return_path,
        "la requête arrive, la réponse ne revient pas"
    );
    let retour = d
        .checks
        .iter()
        .position(|c| c.status == CheckStatus::Failed)
        .unwrap();
    assert_eq!(d.checks[retour].name, "Routage (chemin retour)");
}

#[test]
fn route_to_a_nonexistent_next_hop() {
    expect(
        |s| {
            Fault::Multi(vec![
                Fault::RemoveRoutes {
                    device: dev(s, "R1"),
                    dest: "192.168.2.0/24".parse().unwrap(),
                },
                Fault::AddRoute {
                    device: dev(s, "R1"),
                    route: StaticRoute {
                        dest: "192.168.2.0/24".parse().unwrap(),
                        via: ip("172.31.0.1"),
                    },
                },
            ])
        },
        ping_req,
        CauseKind::WrongRoute,
        "R1",
    );
}

#[test]
fn routing_loop_names_the_culprit_route() {
    let d = expect(
        |s| Fault::AddRoute {
            device: dev(s, "R2"),
            route: StaticRoute {
                dest: "192.168.2.10/32".parse().unwrap(),
                via: ip("10.0.0.1"),
            },
        },
        ping_req,
        CauseKind::RoutingLoop,
        "R2",
    );
    assert_eq!(
        d.cause.unwrap().fix.as_deref(),
        Some("route del R2 192.168.2.10/32")
    );
}

#[test]
fn router_that_stopped_forwarding() {
    let d = expect(
        |s| Fault::ForwardingOff {
            device: dev(s, "R1"),
        },
        ping_req,
        CauseKind::ForwardingDisabled,
        "R1",
    );
    assert_eq!(d.cause.unwrap().fix.as_deref(), Some("forwarding R1 on"));
}

// ---------------------------------------------------------------- services et filtrage

#[test]
fn stopped_service_is_distinguished_from_unreachable_host() {
    let d = expect(
        |s| Fault::StopService {
            device: dev(s, "SERVER"),
            proto: L4Proto::Tcp,
            port: 80,
        },
        web_req,
        CauseKind::ServiceDown,
        "SERVER",
    );
    assert_eq!(
        d.cause.unwrap().fix.as_deref(),
        Some("service add SERVER tcp 80 http")
    );
    // Le ping, lui, passe toujours : la machine est joignable.
    let mut sim = lab();
    sim.inject_fault(
        Fault::StopService {
            device: dev(&sim, "SERVER"),
            proto: L4Proto::Tcp,
            port: 80,
        },
        false,
    )
    .unwrap();
    assert_eq!(
        diagnose(&sim, None, &ping_req(&sim)).unwrap().status,
        Status::Healthy
    );
}

#[test]
fn acl_rule_names_the_blocking_rule() {
    let d = expect(
        |s| Fault::AclInsert {
            device: dev(s, "R2"),
            rule: AclRule::new(AclAction::Deny, RuleProto::Icmp),
        },
        ping_req,
        CauseKind::Filtered,
        "R2",
    );
    assert_eq!(d.cause.unwrap().fix.as_deref(), Some("acl del R2 1"));
}

#[test]
fn default_deny_policy_is_restored_when_the_baseline_is_known() {
    let d = expect(
        |s| Fault::AclDefault {
            device: dev(s, "R1"),
            action: AclAction::Deny,
        },
        web_req,
        CauseKind::Filtered,
        "R1",
    );
    assert_eq!(
        d.cause.unwrap().fix.as_deref(),
        Some("acl default R1 allow")
    );
}

#[test]
fn default_deny_without_a_baseline_proposes_an_allow_rule() {
    let mut sim = lab();
    sim.inject_fault(
        Fault::AclDefault {
            device: dev(&sim, "R1"),
            action: AclAction::Deny,
        },
        false,
    )
    .unwrap();
    let c = diagnose(&sim, None, &web_req(&sim)).unwrap().cause.unwrap();
    assert_eq!(c.kind, CauseKind::Filtered);
    let fix = c.fix.unwrap();
    assert!(
        fix.starts_with("acl add R1 allow tcp dst 192.168.2.10/32 port 80 in eth0"),
        "{fix}"
    );
}

// ---------------------------------------------------------------- mécanique

#[test]
fn checks_stop_at_the_failing_layer() {
    let mut sim = lab();
    sim.inject_fault(
        Fault::SetGateway {
            device: dev(&sim, "PC1"),
            gateway: Some(ip("192.168.1.254")),
        },
        false,
    )
    .unwrap();
    let d = diagnose(&sim, None, &ping_req(&sim)).unwrap();
    let status: Vec<(&str, CheckStatus)> = d.checks.iter().map(|c| (c.name, c.status)).collect();
    let failing = status
        .iter()
        .position(|(_, s)| *s == CheckStatus::Failed)
        .unwrap();
    assert_eq!(status[failing].0, "Passerelle par défaut");
    assert!(status[..failing].iter().all(|(_, s)| *s == CheckStatus::Ok));
    assert!(
        status[failing + 1..]
            .iter()
            .all(|(_, s)| *s == CheckStatus::NotTested)
    );
}

#[test]
fn diagnosis_without_a_baseline_still_finds_the_cause_but_not_the_expected_value() {
    let mut sim = lab();
    sim.inject_fault(
        Fault::SetGateway {
            device: dev(&sim, "PC1"),
            gateway: Some(ip("192.168.1.254")),
        },
        false,
    )
    .unwrap();
    let c = diagnose(&sim, None, &ping_req(&sim))
        .unwrap()
        .cause
        .unwrap();
    assert_eq!(c.kind, CauseKind::WrongGateway);
    assert_eq!(c.device, "PC1");
    assert!(c.expected.is_none());
}

#[test]
fn rendering_hides_the_answer_unless_asked() {
    let mut sim = lab();
    let base = sim.clone();
    sim.inject_fault(
        Fault::SetGateway {
            device: dev(&sim, "PC1"),
            gateway: Some(ip("192.168.1.254")),
        },
        false,
    )
    .unwrap();
    let d = diagnose(&sim, Some(&base), &ping_req(&sim)).unwrap();
    let hidden = d.render(false, false);
    assert!(hidden.contains("[✗] Passerelle par défaut"));
    assert!(
        !hidden.contains("Attendu") && !hidden.contains("Pour réparer"),
        "{hidden}"
    );
    let full = d.render(true, true);
    assert!(full.contains("Attendu  : 192.168.1.1"), "{full}");
    assert!(
        full.contains("Pour réparer : gateway PC1 192.168.1.1"),
        "{full}"
    );
}

#[test]
fn diagnosis_is_deterministic() {
    let run = || {
        let mut sim = lab();
        sim.inject_fault(
            Fault::LinkLoss {
                link: LinkId(2),
                loss: 0.6,
            },
            false,
        )
        .unwrap();
        let d = diagnose(&sim, None, &ping_req(&sim)).unwrap();
        (d.render(true, true), d.probe.events)
    };
    assert_eq!(run(), run());
}

#[test]
fn hints_never_leak_the_fix_before_the_last_level() {
    let mut sim = lab();
    let base = sim.clone();
    sim.inject_fault(
        Fault::RemoveRoutes {
            device: dev(&sim, "R1"),
            dest: "192.168.2.0/24".parse().unwrap(),
        },
        false,
    )
    .unwrap();
    let c = diagnose(&sim, Some(&base), &ping_req(&sim))
        .unwrap()
        .cause
        .unwrap();
    let h = hints_for(&c);
    let fix = c.fix.unwrap();
    assert!(h[..3].iter().all(|t| !t.contains(&fix)));
    assert!(h[3].contains(&fix));
    assert!(h[0].contains("couche 3"));
}

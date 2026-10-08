#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Filtrage : ACL sur routeur, pare-feu stateful, filtrage hôte.

mod common;

use common::*;
use netiv_core::acl::{AclAction, AclRule, RuleProto};
use netiv_core::service::Service;
use netiv_core::trace::{CurlOutcome, DropReason};
use netiv_core::{DeviceId, DeviceKind, Simulation, TraceKind};

/// PC1 (LAN) ── FW1 ── SRV (DMZ).
fn dmz_lab() -> Simulation {
    let mut sim = Simulation::new(7);
    let pc1 = add(
        &mut sim,
        "PC1",
        DeviceKind::Host,
        &[("eth0", Some("192.168.1.10/24"))],
    );
    add(
        &mut sim,
        "FW1",
        DeviceKind::Firewall,
        &[
            ("eth0", Some("192.168.1.1/24")),
            ("eth1", Some("10.9.0.1/24")),
        ],
    );
    let srv = add(
        &mut sim,
        "SRV",
        DeviceKind::Server,
        &[("eth0", Some("10.9.0.10/24"))],
    );
    gw(&mut sim, pc1, "192.168.1.1");
    gw(&mut sim, srv, "10.9.0.1");
    link(&mut sim, "PC1:eth0", "FW1:eth0");
    link(&mut sim, "FW1:eth1", "SRV:eth0");
    sim.add_service(srv, Service::tcp(80, "http")).unwrap();
    sim.add_service(srv, Service::tcp(443, "https")).unwrap();
    sim.add_service(pc1, Service::tcp(22, "ssh")).unwrap();
    sim.clear_trace();
    sim
}

fn rule(
    sim: &Simulation,
    action: AclAction,
    proto: RuleProto,
    in_iface: Option<(&str, &str)>,
    dst: Option<&str>,
    port: Option<u16>,
) -> AclRule {
    let mut r = AclRule::new(action, proto);
    r.in_iface = in_iface.map(|(d, i)| sim.iface_id(dev(sim, d), i).unwrap());
    r.dst = dst.map(|d| d.parse().unwrap());
    r.port_from = port;
    r
}

fn fw(sim: &Simulation) -> DeviceId {
    dev(sim, "FW1")
}

#[test]
fn firewall_denies_everything_by_default() {
    let mut sim = dmz_lab();
    curl(&mut sim, "PC1", "10.9.0.10", 80);
    ping(&mut sim, "PC1", "10.9.0.10", 1);
    assert_eq!(curl_results(&sim), [CurlOutcome::Timeout]);
    assert!(ping_ok(&sim).is_empty());
    // La trace dit pourquoi : politique par défaut.
    let decision = sim
        .trace()
        .events()
        .iter()
        .find_map(|e| match &e.kind {
            TraceKind::AclDecision { rule, text } => Some((*rule, text.clone())),
            _ => None,
        })
        .unwrap();
    assert_eq!(decision.0, None);
    assert!(
        decision.1.contains("politique par défaut"),
        "{}",
        decision.1
    );
}

#[test]
fn an_allow_rule_opens_exactly_that_service() {
    let mut sim = dmz_lab();
    let r = rule(
        &sim,
        AclAction::Allow,
        RuleProto::Tcp,
        Some(("FW1", "eth0")),
        Some("10.9.0.0/24"),
        Some(80),
    );
    sim.acl_add_rule(fw(&sim), r).unwrap();

    curl(&mut sim, "PC1", "10.9.0.10", 80);
    curl(&mut sim, "PC1", "10.9.0.10", 443);
    let results = curl_results(&sim);
    assert!(matches!(results[0], CurlOutcome::Ok { .. }), "{results:?}");
    assert_eq!(results[1], CurlOutcome::Timeout, "443 reste bloqué");
}

#[test]
fn stateful_tracking_lets_the_reply_through_but_not_new_inbound_flows() {
    let mut sim = dmz_lab();
    let r = rule(
        &sim,
        AclAction::Allow,
        RuleProto::Tcp,
        Some(("FW1", "eth0")),
        Some("10.9.0.0/24"),
        Some(80),
    );
    sim.acl_add_rule(fw(&sim), r).unwrap();
    curl(&mut sim, "PC1", "10.9.0.10", 80);
    assert!(matches!(curl_results(&sim)[0], CurlOutcome::Ok { .. }));

    // La DMZ ne peut pas initier de connexion vers le LAN.
    curl(&mut sim, "SRV", "192.168.1.10", 22);
    assert_eq!(curl_results(&sim)[1], CurlOutcome::Timeout);
}

#[test]
fn without_state_the_reply_is_blocked_unless_a_rule_allows_it() {
    let mut sim = dmz_lab();
    sim.acl_set_stateful(fw(&sim), false).unwrap();
    let r = rule(
        &sim,
        AclAction::Allow,
        RuleProto::Tcp,
        Some(("FW1", "eth0")),
        Some("10.9.0.0/24"),
        Some(80),
    );
    sim.acl_add_rule(fw(&sim), r).unwrap();
    curl(&mut sim, "PC1", "10.9.0.10", 80);
    // Le SYN passe, mais le SYN-ACK de retour est refusé : la connexion échoue.
    assert_eq!(curl_results(&sim), [CurlOutcome::Timeout]);
}

#[test]
fn the_trace_cites_the_rule_that_blocked_the_packet() {
    let mut sim = dmz_lab();
    sim.acl_set_default(fw(&sim), AclAction::Allow).unwrap();
    let mut deny = rule(&sim, AclAction::Deny, RuleProto::Tcp, None, None, Some(443));
    deny.comment = "pas de HTTPS".into();
    let idx = sim.acl_add_rule(fw(&sim), deny).unwrap();
    assert_eq!(idx, 0);

    curl(&mut sim, "PC1", "10.9.0.10", 80);
    curl(&mut sim, "PC1", "10.9.0.10", 443);
    let results = curl_results(&sim);
    assert!(matches!(results[0], CurlOutcome::Ok { .. }));
    assert_eq!(results[1], CurlOutcome::Timeout);

    let (rule_idx, text) = sim
        .trace()
        .events()
        .iter()
        .find_map(|e| match &e.kind {
            TraceKind::AclDecision { rule, text } => Some((*rule, text.clone())),
            _ => None,
        })
        .unwrap();
    assert_eq!(rule_idx, Some(0));
    assert!(
        text.contains("règle n°1") && text.contains("port 443") && text.contains("pas de HTTPS"),
        "{text}"
    );
    // Et le paquet est bien marqué comme filtré.
    assert!(sim.trace().events().iter().any(|e| matches!(
        e.kind,
        TraceKind::Dropped {
            reason: DropReason::Filtered
        }
    )));
}

#[test]
fn removing_the_blocking_rule_restores_the_service() {
    let mut sim = dmz_lab();
    sim.acl_set_default(fw(&sim), AclAction::Allow).unwrap();
    let deny = rule(&sim, AclAction::Deny, RuleProto::Tcp, None, None, Some(443));
    sim.acl_add_rule(fw(&sim), deny).unwrap();
    curl(&mut sim, "PC1", "10.9.0.10", 443);
    sim.acl_remove_rule(fw(&sim), 0).unwrap();
    curl(&mut sim, "PC1", "10.9.0.10", 443);
    let r = curl_results(&sim);
    assert_eq!(r[0], CurlOutcome::Timeout);
    assert!(matches!(r[1], CurlOutcome::Ok { .. }));
}

#[test]
fn rule_order_matters() {
    let mut sim = dmz_lab();
    let allow = rule(&sim, AclAction::Allow, RuleProto::Tcp, None, None, Some(80));
    let deny_all = rule(&sim, AclAction::Deny, RuleProto::Any, None, None, None);
    sim.acl_set_default(fw(&sim), AclAction::Allow).unwrap();
    sim.acl_add_rule(fw(&sim), deny_all.clone()).unwrap();
    curl(&mut sim, "PC1", "10.9.0.10", 80);
    // Insérée avant le « deny any », la règle d'autorisation prend le dessus.
    sim.acl_insert_rule(fw(&sim), 0, allow).unwrap();
    curl(&mut sim, "PC1", "10.9.0.10", 80);
    let r = curl_results(&sim);
    assert_eq!(r[0], CurlOutcome::Timeout);
    assert!(matches!(r[1], CurlOutcome::Ok { .. }));
}

#[test]
fn removing_a_missing_rule_is_an_error() {
    let mut sim = dmz_lab();
    assert!(sim.acl_remove_rule(fw(&sim), 3).is_err());
}

#[test]
fn host_firewall_blocks_icmp_but_not_http() {
    let mut sim = dmz_lab();
    sim.acl_set_default(fw(&sim), AclAction::Allow).unwrap();
    let srv = dev(&sim, "SRV");
    sim.acl_add_rule(
        srv,
        rule(&sim, AclAction::Deny, RuleProto::Icmp, None, None, None),
    )
    .unwrap();
    ping(&mut sim, "PC1", "10.9.0.10", 1);
    curl(&mut sim, "PC1", "10.9.0.10", 80);
    assert!(
        ping_ok(&sim).is_empty(),
        "le serveur ne répond plus au ping"
    );
    assert!(matches!(curl_results(&sim)[0], CurlOutcome::Ok { .. }));
}

#[test]
fn filtering_applies_to_the_firewall_in_transit_for_icmp_too() {
    let mut sim = dmz_lab();
    let allow_icmp = rule(
        &sim,
        AclAction::Allow,
        RuleProto::Icmp,
        Some(("FW1", "eth0")),
        None,
        None,
    );
    sim.acl_add_rule(fw(&sim), allow_icmp).unwrap();
    ping(&mut sim, "PC1", "10.9.0.10", 2);
    assert_eq!(
        ping_ok(&sim).len(),
        2,
        "écho aller et retour (suivi de connexion)"
    );
}

#[test]
fn the_firewall_reports_a_missing_route_to_the_sender() {
    // Le pare-feu n'a pas de route vers 10.77.0.1 : l'erreur « réseau inaccessible »
    // qu'il génère doit atteindre l'émetteur malgré la politique « deny ».
    let mut sim = dmz_lab();
    let allow = rule(
        &sim,
        AclAction::Allow,
        RuleProto::Icmp,
        Some(("FW1", "eth0")),
        None,
        None,
    );
    sim.acl_add_rule(fw(&sim), allow).unwrap();
    ping(&mut sim, "PC1", "10.77.0.1", 1);
    assert!(
        sim.trace()
            .events()
            .iter()
            .any(|e| matches!(e.kind, TraceKind::PingError { .. }))
    );
}

#[test]
fn firewall_decisions_are_deterministic() {
    let run = || {
        let mut sim = dmz_lab();
        let r = rule(
            &sim,
            AclAction::Allow,
            RuleProto::Tcp,
            Some(("FW1", "eth0")),
            Some("10.9.0.0/24"),
            Some(80),
        );
        sim.acl_add_rule(fw(&sim), r).unwrap();
        for p in [80, 443, 80] {
            curl(&mut sim, "PC1", "10.9.0.10", p);
        }
        sim.trace().events().to_vec()
    };
    assert_eq!(run(), run());
}

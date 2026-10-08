#![allow(clippy::unwrap_used, clippy::expect_used)]

//! TCP et UDP : handshake, services, refus, timeouts.

mod common;

use common::*;
use netiv_core::packet::{EtherPayload, IpPayload, L4Proto};
use netiv_core::service::Service;
use netiv_core::trace::{CurlOutcome, IcmpErrorKind, UdpOutcome};
use netiv_core::{DeviceKind, LinkParams, SimTime, Simulation, StaticRoute, TraceKind};

/// PC1 ── SW1 ── R1 ── R2 ── SERVER (comme `examples/mvp.yaml`).
fn lab() -> Simulation {
    let mut sim = Simulation::new(42);
    let pc1 = add(
        &mut sim,
        "PC1",
        DeviceKind::Host,
        &[("eth0", Some("192.168.1.10/24"))],
    );
    add(
        &mut sim,
        "SW1",
        DeviceKind::Switch,
        &[("p1", None), ("p2", None)],
    );
    let r1 = add(
        &mut sim,
        "R1",
        DeviceKind::Router,
        &[
            ("eth0", Some("192.168.1.1/24")),
            ("eth1", Some("10.0.0.1/30")),
        ],
    );
    let r2 = add(
        &mut sim,
        "R2",
        DeviceKind::Router,
        &[
            ("eth0", Some("10.0.0.2/30")),
            ("eth1", Some("192.168.2.1/24")),
        ],
    );
    let srv = add(
        &mut sim,
        "SERVER",
        DeviceKind::Server,
        &[("eth0", Some("192.168.2.10/24"))],
    );
    gw(&mut sim, pc1, "192.168.1.1");
    gw(&mut sim, srv, "192.168.2.1");
    route(&mut sim, r1, "192.168.2.0/24", "10.0.0.2");
    route(&mut sim, r2, "192.168.1.0/24", "10.0.0.1");
    link(&mut sim, "PC1:eth0", "SW1:p1");
    link(&mut sim, "SW1:p2", "R1:eth0");
    link(&mut sim, "R1:eth1", "R2:eth0");
    link(&mut sim, "R2:eth1", "SERVER:eth0");
    sim.add_service(srv, Service::tcp(80, "http")).unwrap();
    sim.add_service(srv, Service::tcp(22, "ssh")).unwrap();
    sim.add_service(srv, Service::udp(7, "echo")).unwrap();
    sim.clear_trace();
    sim
}

/// Date du dernier résultat (curl ou udp) consigné.
fn result_time(sim: &Simulation) -> SimTime {
    sim.trace()
        .events()
        .iter()
        .rev()
        .find(|e| {
            matches!(
                e.kind,
                TraceKind::CurlResult { .. } | TraceKind::UdpResult { .. }
            )
        })
        .map(|e| e.time)
        .expect("aucun résultat dans la trace")
}

fn first_trace_id(sim: &Simulation) -> netiv_core::TraceId {
    sim.trace()
        .events()
        .iter()
        .find_map(|e| e.trace_id)
        .expect("aucune transaction")
}

fn tcp_flags_on_the_wire(sim: &Simulation, device: &str) -> Vec<String> {
    let d = dev(sim, device);
    sim.trace()
        .by_device(d)
        .filter_map(|e| match &e.kind {
            TraceKind::FrameSent { frame, .. } => match &frame.payload {
                EtherPayload::Ipv4(p) => match &p.payload {
                    IpPayload::Tcp(t) => Some(t.flags.to_string()),
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        })
        .collect()
}

// ------------------------------------------------------------------ TCP

#[test]
fn curl_completes_a_handshake_and_gets_the_response() {
    let mut sim = lab();
    curl(&mut sim, "PC1", "192.168.2.10", 80);
    assert_eq!(
        curl_results(&sim),
        [CurlOutcome::Ok {
            response: "HTTP/1.1 200 OK".into()
        }]
    );
    // Côté client : SYN, ACK(+requête), FIN,ACK. Côté serveur : SYN,ACK puis FIN,ACK(+réponse).
    assert_eq!(
        tcp_flags_on_the_wire(&sim, "PC1"),
        ["SYN", "ACK", "FIN,ACK"]
    );
    assert_eq!(
        tcp_flags_on_the_wire(&sim, "SERVER"),
        ["SYN,ACK", "FIN,ACK"]
    );
    // La connexion est établie des deux côtés.
    let established = sim
        .trace()
        .events()
        .iter()
        .filter(|e| matches!(e.kind, TraceKind::TcpEstablished { .. }))
        .count();
    assert_eq!(established, 2);
    assert_eq!(
        sim.trace()
            .events()
            .iter()
            .filter(|e| matches!(&e.kind, TraceKind::ServiceRequest { service, .. } if service == "http"))
            .count(),
        1
    );
}

#[test]
fn a_whole_curl_is_a_single_transaction() {
    let mut sim = lab();
    curl(&mut sim, "PC1", "192.168.2.10", 80);
    let first = first_trace_id(&sim);
    let in_tx = sim.trace().by_trace(first).count();
    // Tous les événements (ARP compris) appartiennent à la même transaction,
    // sauf l'événement de commande qui la déclenche.
    assert_eq!(in_tx, sim.trace().len() - 1);
    assert!(matches!(
        sim.trace().events()[0].kind,
        TraceKind::CommandIssued { .. }
    ));
}

#[test]
fn closed_port_is_refused_with_a_reset() {
    let mut sim = lab();
    curl(&mut sim, "PC1", "192.168.2.10", 8080);
    assert_eq!(curl_results(&sim), [CurlOutcome::Refused]);
    assert!(
        sim.trace()
            .events()
            .iter()
            .any(|e| matches!(e.kind, TraceKind::TcpResetSent { port: 8080, .. }))
    );
    // Un refus est immédiat : bien avant les 3 s du timeout.
    assert!(
        result_time(&sim) < SimTime::from_millis(100),
        "{}",
        result_time(&sim)
    );
}

#[test]
fn unreachable_server_times_out_after_three_syns() {
    let mut sim = lab();
    sim.set_link_up(netiv_core::LinkId(2), false).unwrap(); // R1 ↔ R2
    curl(&mut sim, "PC1", "192.168.2.10", 80);
    assert_eq!(curl_results(&sim), [CurlOutcome::Timeout]);
    let syns = tcp_flags_on_the_wire(&sim, "PC1")
        .into_iter()
        .filter(|f| f == "SYN")
        .count();
    assert_eq!(syns, 3);
    // 3 envois de SYN espacés d'une seconde : abandon à t = 3 s.
    assert_eq!(result_time(&sim), SimTime::from_millis(3_000));
}

#[test]
fn timeout_and_refusal_are_distinguishable() {
    // C'est le cœur du diagnostic : RST = machine joignable, port fermé.
    let mut sim = lab();
    curl(&mut sim, "PC1", "192.168.2.10", 9999);
    let refused = curl_results(&sim);
    sim.set_link_up(netiv_core::LinkId(2), false).unwrap();
    curl(&mut sim, "PC1", "192.168.2.10", 80);
    let all = curl_results(&sim);
    assert_eq!(refused, [CurlOutcome::Refused]);
    assert_eq!(all[1], CurlOutcome::Timeout);
}

#[test]
fn router_without_route_reports_network_unreachable_to_curl() {
    let mut sim = lab();
    let r1 = dev(&sim, "R1");
    sim.remove_route(r1, "192.168.2.0/24".parse().unwrap())
        .unwrap();
    curl(&mut sim, "PC1", "192.168.2.10", 80);
    assert_eq!(
        curl_results(&sim),
        [CurlOutcome::Error(IcmpErrorKind::NetUnreachable)]
    );
}

#[test]
fn host_without_route_fails_immediately() {
    let mut sim = lab();
    let pc1 = dev(&sim, "PC1");
    sim.set_gateway(pc1, None).unwrap();
    curl(&mut sim, "PC1", "192.168.2.10", 80);
    assert_eq!(
        curl_results(&sim),
        [CurlOutcome::Error(IcmpErrorKind::NetUnreachable)]
    );
    assert_eq!(result_time(&sim), SimTime::ZERO);
}

#[test]
fn stopping_the_service_turns_success_into_refusal() {
    let mut sim = lab();
    let srv = dev(&sim, "SERVER");
    curl(&mut sim, "PC1", "192.168.2.10", 80);
    assert_eq!(sim.remove_service(srv, L4Proto::Tcp, 80).unwrap(), 1);
    curl(&mut sim, "PC1", "192.168.2.10", 80);
    let r = curl_results(&sim);
    assert!(matches!(r[0], CurlOutcome::Ok { .. }));
    assert_eq!(r[1], CurlOutcome::Refused);
}

#[test]
fn services_answer_differently() {
    let mut sim = lab();
    curl(&mut sim, "PC1", "192.168.2.10", 22);
    assert_eq!(
        curl_results(&sim),
        [CurlOutcome::Ok {
            response: "ssh prêt".into()
        }]
    );
}

#[test]
fn several_connections_use_distinct_ports() {
    let mut sim = lab();
    curl(&mut sim, "PC1", "192.168.2.10", 80);
    curl(&mut sim, "PC1", "192.168.2.10", 80);
    let ports: Vec<u16> = sim
        .trace()
        .events()
        .iter()
        .filter_map(|e| match &e.kind {
            TraceKind::TcpEstablished { local_port, .. }
                if sim.device(e.device).unwrap().name == "PC1" =>
            {
                Some(*local_port)
            }
            _ => None,
        })
        .collect();
    assert_eq!(ports.len(), 2);
    assert_ne!(ports[0], ports[1]);
    assert!(ports.iter().all(|p| *p >= 49_152));
}

#[test]
fn curl_to_self_is_a_loopback() {
    let mut sim = lab();
    let srv = dev(&sim, "SERVER");
    sim.command(
        srv,
        netiv_core::Command::Curl {
            dst: ip("192.168.2.10"),
            port: 80,
        },
    )
    .unwrap();
    sim.run_until_idle().unwrap();
    assert!(matches!(curl_results(&sim)[0], CurlOutcome::Ok { .. }));
    assert_eq!(count(&sim, |k| matches!(k, TraceKind::FrameSent { .. })), 0);
}

#[test]
fn lost_data_segments_end_in_a_timeout_not_a_hang() {
    // 100 % de perte sur le dernier lien : le SYN n'arrive jamais.
    let mut sim = lab();
    sim.set_link_loss(netiv_core::LinkId(3), 1.0).unwrap();
    curl(&mut sim, "PC1", "192.168.2.10", 80);
    assert_eq!(curl_results(&sim), [CurlOutcome::Timeout]);
}

#[test]
fn curl_runs_are_deterministic_even_with_loss() {
    let run = || {
        let mut sim = lab();
        sim.set_link_loss(netiv_core::LinkId(0), 0.3).unwrap();
        for _ in 0..5 {
            curl(&mut sim, "PC1", "192.168.2.10", 80);
        }
        sim.trace().events().to_vec()
    };
    assert_eq!(run(), run());
}

// ------------------------------------------------------------------ UDP

#[test]
fn udp_echo_service_replies() {
    let mut sim = lab();
    udp(&mut sim, "PC1", "192.168.2.10", 7, "salut");
    assert_eq!(
        udp_results(&sim),
        [UdpOutcome::Reply {
            data: "salut".into()
        }]
    );
}

#[test]
fn udp_to_a_closed_port_gets_icmp_port_unreachable() {
    let mut sim = lab();
    udp(&mut sim, "PC1", "192.168.2.10", 5353, "x");
    assert_eq!(udp_results(&sim), [UdpOutcome::PortUnreachable]);
}

#[test]
fn udp_without_answer_times_out() {
    let mut sim = lab();
    sim.set_link_up(netiv_core::LinkId(2), false).unwrap();
    udp(&mut sim, "PC1", "192.168.2.10", 7, "x");
    assert_eq!(udp_results(&sim), [UdpOutcome::Timeout]);
    assert_eq!(result_time(&sim), SimTime::from_millis(2_000));
}

#[test]
fn udp_through_a_router_without_route_reports_the_error() {
    let mut sim = lab();
    let r1 = dev(&sim, "R1");
    sim.remove_route(r1, "192.168.2.0/24".parse().unwrap())
        .unwrap();
    udp(&mut sim, "PC1", "192.168.2.10", 7, "x");
    assert_eq!(
        udp_results(&sim),
        [UdpOutcome::Error(IcmpErrorKind::NetUnreachable)]
    );
}

#[test]
fn replacing_a_service_on_the_same_port_keeps_one_listener() {
    let mut sim = lab();
    let srv = dev(&sim, "SERVER");
    sim.add_service(srv, Service::tcp(80, "other")).unwrap();
    curl(&mut sim, "PC1", "192.168.2.10", 80);
    assert_eq!(
        curl_results(&sim),
        [CurlOutcome::Ok {
            response: "other prêt".into()
        }]
    );
}

#[test]
fn journey_of_a_curl_lists_every_segment_path() {
    let mut sim = lab();
    curl(&mut sim, "PC1", "192.168.2.10", 80);
    let id = first_trace_id(&sim);
    let text = netiv_core::render::journey(&sim, id).unwrap();
    assert!(text.contains("curl PC1 → 192.168.2.10:80"), "{text}");
    assert!(
        text.contains("TCP [SYN]") && text.contains("TCP [SYN,ACK]"),
        "{text}"
    );
    assert!(
        text.contains("PC1 → SW1 → R1 (ttl 64) → R2 (ttl 63) → SERVER (ttl 62)"),
        "{text}"
    );
    let _ = (
        LinkParams::default(),
        StaticRoute {
            dest: "0.0.0.0/0".parse().unwrap(),
            via: ip("1.1.1.1"),
        },
    );
}

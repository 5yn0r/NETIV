//! Utilitaires partagés par les tests de scénarios.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::net::Ipv4Addr;

use netiv_core::{
    DeviceId, DeviceKind, LinkId, LinkParams, SimTime, Simulation, StaticRoute, TraceEvent,
    TraceKind,
};

pub fn ip(s: &str) -> Ipv4Addr {
    s.parse().unwrap()
}

pub fn add(
    sim: &mut Simulation,
    name: &str,
    kind: DeviceKind,
    ifaces: &[(&str, Option<&str>)],
) -> DeviceId {
    let d = sim.add_device(name, kind).unwrap();
    for (n, cidr) in ifaces {
        sim.add_interface(d, n, cidr.map(|c| c.parse().unwrap()))
            .unwrap();
    }
    d
}

pub fn link(sim: &mut Simulation, a: &str, b: &str) -> LinkId {
    link_with(sim, a, b, LinkParams::default())
}

pub fn link_with(sim: &mut Simulation, a: &str, b: &str, p: LinkParams) -> LinkId {
    let end = |sim: &Simulation, s: &str| {
        let (d, i) = s.split_once(':').unwrap();
        let d = sim.device_id(d).unwrap();
        (d, sim.iface_id(d, i).unwrap())
    };
    let (ea, eb) = (end(sim, a), end(sim, b));
    sim.connect(ea, eb, p).unwrap()
}

pub fn gw(sim: &mut Simulation, dev: DeviceId, gw: &str) {
    sim.set_gateway(dev, Some(ip(gw))).unwrap();
}

pub fn route(sim: &mut Simulation, dev: DeviceId, dest: &str, via: &str) {
    sim.add_route(
        dev,
        StaticRoute {
            dest: dest.parse().unwrap(),
            via: ip(via),
        },
    )
    .unwrap();
}

pub fn events(sim: &Simulation) -> &[TraceEvent] {
    sim.trace().events()
}

pub fn count(sim: &Simulation, pred: impl Fn(&TraceKind) -> bool) -> usize {
    events(sim).iter().filter(|e| pred(&e.kind)).count()
}

pub fn ping_ok(sim: &Simulation) -> Vec<(u16, u8, SimTime)> {
    events(sim)
        .iter()
        .filter_map(|e| match e.kind {
            TraceKind::PingReply { seq, ttl, rtt, .. } => Some((seq, ttl, rtt)),
            _ => None,
        })
        .collect()
}

/// PC1 ── SW1 ── R1 ── R2 ── SERVER
pub fn mvp() -> Simulation {
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
        &[("port1", None), ("port2", None), ("port3", None)],
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
    link(&mut sim, "PC1:eth0", "SW1:port1");
    link(&mut sim, "SW1:port2", "R1:eth0");
    link(&mut sim, "R1:eth1", "R2:eth0");
    link(&mut sim, "R2:eth1", "SERVER:eth0");
    sim
}

pub fn ping(sim: &mut Simulation, from: &str, dst: &str, n: u16) {
    let d = sim.device_id(from).unwrap();
    sim.ping(d, ip(dst), n).unwrap();
    sim.run_until_idle().unwrap();
}

pub fn curl(sim: &mut Simulation, from: &str, dst: &str, port: u16) {
    let d = sim.device_id(from).unwrap();
    sim.command(d, netiv_core::Command::Curl { dst: ip(dst), port })
        .unwrap();
    sim.run_until_idle().unwrap();
}

pub fn udp(sim: &mut Simulation, from: &str, dst: &str, port: u16, data: &str) {
    let d = sim.device_id(from).unwrap();
    sim.command(
        d,
        netiv_core::Command::SendUdp {
            dst: ip(dst),
            port,
            data: data.to_string(),
        },
    )
    .unwrap();
    sim.run_until_idle().unwrap();
}

/// Résultats de `curl` consignés dans la trace.
pub fn curl_results(sim: &Simulation) -> Vec<netiv_core::trace::CurlOutcome> {
    events(sim)
        .iter()
        .filter_map(|e| match &e.kind {
            TraceKind::CurlResult { outcome, .. } => Some(outcome.clone()),
            _ => None,
        })
        .collect()
}

pub fn udp_results(sim: &Simulation) -> Vec<netiv_core::trace::UdpOutcome> {
    events(sim)
        .iter()
        .filter_map(|e| match &e.kind {
            TraceKind::UdpResult { outcome, .. } => Some(outcome.clone()),
            _ => None,
        })
        .collect()
}

pub fn dev(sim: &Simulation, name: &str) -> DeviceId {
    sim.device_id(name).unwrap()
}

pub fn port(sim: &Simulation, endpoint: &str) -> (DeviceId, netiv_core::ids::IfaceId) {
    let (d, i) = endpoint.split_once(':').unwrap();
    let d = sim.device_id(d).unwrap();
    (d, sim.iface_id(d, i).unwrap())
}

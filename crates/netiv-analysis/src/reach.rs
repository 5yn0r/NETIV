//! Matrice d'accessibilité : pour chaque couple de machines, ping et services.

use std::collections::BTreeMap;
use std::net::Ipv4Addr;

use netiv_core::packet::L4Proto;
use netiv_core::{DeviceId, DeviceKind, Simulation};
use netiv_diagnostics::{Probe, ProbeKind, run_probe};

/// Nombre maximal de flux mesurés. Au-delà, on échantillonne à pas régulier
/// (déterministe) : sur un très grand réseau, la mesure reste en quelques
/// secondes.
pub const MAX_FLOWS: usize = 4_000;

/// Un flux mesuré.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FlowId {
    /// Émetteur.
    pub src: DeviceId,
    /// Destinataire.
    pub dst: DeviceId,
    /// Service visé ; `None` = ping.
    pub service: Option<(L4Proto, u16)>,
}

/// Résultats : `true` = le flux aboutit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Matrix {
    /// Un résultat par flux.
    pub results: BTreeMap<FlowId, bool>,
}

impl Matrix {
    /// Flux qui aboutissent.
    pub fn ok_count(&self) -> usize {
        self.results.values().filter(|v| **v).count()
    }

    /// Nombre de flux mesurés.
    pub fn len(&self) -> usize {
        self.results.len()
    }

    /// Aucun flux mesuré ?
    pub fn is_empty(&self) -> bool {
        self.results.is_empty()
    }
}

/// Les machines (hôtes et serveurs).
pub fn end_devices(sim: &Simulation) -> Vec<DeviceId> {
    sim.devices()
        .iter()
        .filter(|d| matches!(d.kind, DeviceKind::Host | DeviceKind::Server))
        .map(|d| d.id)
        .collect()
}

/// Première adresse IP d'un équipement.
pub fn address_of(sim: &Simulation, d: DeviceId) -> Option<Ipv4Addr> {
    sim.device(d)?
        .ifaces
        .iter()
        .find_map(|i| i.ip.map(|n| n.addr()))
}

/// Tous les flux à mesurer : ping, plus chaque service TCP/UDP de la
/// destination, pour chaque couple de machines distinctes.
pub fn flows(sim: &Simulation) -> Vec<FlowId> {
    let ends = end_devices(sim);
    let mut out = Vec::new();
    for &src in &ends {
        for &dst in &ends {
            if src == dst {
                continue;
            }
            out.push(FlowId {
                src,
                dst,
                service: None,
            });
            if let Some(netiv_core::device::Behavior::Ip(n)) = sim.device(dst).map(|d| &d.behavior)
            {
                for s in &n.services {
                    out.push(FlowId {
                        src,
                        dst,
                        service: Some((s.proto, s.port)),
                    });
                }
            }
        }
    }
    sample(out)
}

/// Échantillon régulier de `MAX_FLOWS` flux au plus.
fn sample(all: Vec<FlowId>) -> Vec<FlowId> {
    if all.len() <= MAX_FLOWS {
        return all;
    }
    let step = all.len().div_ceil(MAX_FLOWS);
    all.into_iter().step_by(step).collect()
}

/// Mesure un flux.
pub fn measure(sim: &Simulation, f: &FlowId) -> bool {
    let Some(dst) = address_of(sim, f.dst) else {
        return false;
    };
    let kind = match f.service {
        None => ProbeKind::Ping { count: 1 },
        Some((L4Proto::Tcp, port)) => ProbeKind::Tcp { port },
        Some((L4Proto::Udp, port)) => ProbeKind::Udp { port },
    };
    run_probe(
        sim,
        &Probe {
            src: f.src,
            dst,
            kind,
        },
    )
    .is_ok_and(|r| r.success)
}

/// Mesure ces flux.
pub fn measure_all(sim: &Simulation, flows: &[FlowId]) -> Matrix {
    Matrix {
        results: flows.iter().map(|f| (*f, measure(sim, f))).collect(),
    }
}

/// Matrice complète du réseau actuel (filtrage compris).
pub fn matrix(sim: &Simulation) -> Matrix {
    measure_all(sim, &flows(sim))
}

/// Pings entre machines, **sans** filtrage : ce que le routage permet.
pub fn routable(sim: &Simulation) -> Matrix {
    let open = sim.unfiltered_clone();
    let pings: Vec<FlowId> = flows(sim)
        .into_iter()
        .filter(|f| f.service.is_none())
        .collect();
    measure_all(&open, &pings)
}

/// Nom lisible d'un flux : `PC1 → SRV1 (tcp/80)`.
pub fn label(sim: &Simulation, f: &FlowId) -> String {
    let name = |d: DeviceId| sim.device(d).map_or("?".to_string(), |x| x.name.clone());
    match f.service {
        None => format!("{} → {} (ping)", name(f.src), name(f.dst)),
        Some((p, port)) => format!("{} → {} ({p}/{port})", name(f.src), name(f.dst)),
    }
}

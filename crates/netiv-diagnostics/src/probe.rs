//! Sondes : lancer un ping / une requête TCP / un datagramme UDP sur une
//! copie propre de la simulation et en lire le résultat.

use std::net::Ipv4Addr;

use netiv_core::trace::{CurlOutcome, IcmpErrorKind, UdpOutcome};
use netiv_core::{Command, DeviceId, NetivError, SimTime, Simulation, TraceEvent, TraceKind};

/// Type de sonde.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ProbeKind {
    /// `count` requêtes d'écho.
    Ping {
        /// Nombre de requêtes.
        count: u16,
    },
    /// Connexion TCP + requête.
    Tcp {
        /// Port.
        port: u16,
    },
    /// Datagramme UDP.
    Udp {
        /// Port.
        port: u16,
    },
}

/// Une question posée au réseau.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Probe {
    /// Émetteur.
    pub src: DeviceId,
    /// Destination.
    pub dst: Ipv4Addr,
    /// Type de sonde.
    pub kind: ProbeKind,
}

impl Probe {
    /// Ping simple (4 requêtes).
    pub fn ping(src: DeviceId, dst: Ipv4Addr) -> Self {
        Probe {
            src,
            dst,
            kind: ProbeKind::Ping { count: 4 },
        }
    }

    /// Requête TCP.
    pub fn tcp(src: DeviceId, dst: Ipv4Addr, port: u16) -> Self {
        Probe {
            src,
            dst,
            kind: ProbeKind::Tcp { port },
        }
    }

    /// Datagramme UDP.
    pub fn udp(src: DeviceId, dst: Ipv4Addr, port: u16) -> Self {
        Probe {
            src,
            dst,
            kind: ProbeKind::Udp { port },
        }
    }
}

/// Résultat d'une sonde.
#[derive(Clone, Debug)]
pub struct ProbeResult {
    /// La sonde.
    pub probe: Probe,
    /// Tous les envois ont reçu une réponse.
    pub success: bool,
    /// Réponses reçues.
    pub replies: u16,
    /// Envois effectués.
    pub sent: u16,
    /// Plus grand temps de réponse observé.
    pub max_rtt: Option<SimTime>,
    /// Erreur ICMP reçue, s'il y en a une.
    pub icmp_error: Option<IcmpErrorKind>,
    /// Le serveur a refusé la connexion (RST) ou répondu « port fermé ».
    pub refused: bool,
    /// Événements de trace produits par la sonde (trace vierge au départ).
    pub events: Vec<TraceEvent>,
}

/// Exécute une sonde sur une copie propre de `sim`.
pub fn run_probe(sim: &Simulation, probe: &Probe) -> Result<ProbeResult, NetivError> {
    let mut work = sim.fresh_clone();
    let cmd = match probe.kind {
        ProbeKind::Ping { count } => Command::Ping {
            dst: probe.dst,
            count,
        },
        ProbeKind::Tcp { port } => Command::Curl {
            dst: probe.dst,
            port,
        },
        ProbeKind::Udp { port } => Command::SendUdp {
            dst: probe.dst,
            port,
            data: "sonde".to_string(),
        },
    };
    work.command(probe.src, cmd)?;
    work.run_until_idle()?;

    let events = work.take_trace();
    let (mut sent, mut replies) = (0u16, 0u16);
    let mut max_rtt: Option<SimTime> = None;
    let mut icmp_error = None;
    let mut refused = false;

    for e in &events {
        match &e.kind {
            TraceKind::PingSent { .. }
            | TraceKind::CurlStarted { .. }
            | TraceKind::UdpSent { .. } => {
                sent += 1;
            }
            TraceKind::PingReply { rtt, .. } => {
                replies += 1;
                max_rtt = max_rtt.max(Some(*rtt));
            }
            TraceKind::PingError { error, .. } => icmp_error = Some(*error),
            TraceKind::CurlResult { outcome, .. } => match outcome {
                CurlOutcome::Ok { .. } => replies += 1,
                CurlOutcome::Refused => refused = true,
                CurlOutcome::Error(k) => icmp_error = Some(*k),
                CurlOutcome::Timeout => {}
            },
            TraceKind::UdpResult { outcome, .. } => match outcome {
                UdpOutcome::Reply { .. } => replies += 1,
                UdpOutcome::PortUnreachable => refused = true,
                UdpOutcome::Error(k) => icmp_error = Some(*k),
                UdpOutcome::Timeout => {}
            },
            _ => {}
        }
    }
    Ok(ProbeResult {
        probe: *probe,
        success: sent > 0 && replies == sent,
        replies,
        sent,
        max_rtt,
        icmp_error,
        refused,
        events,
    })
}

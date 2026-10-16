//! Relevé du chemin d'un flux sain.

use std::collections::BTreeSet;
use std::net::Ipv4Addr;

use netiv_core::packet::{EtherPayload, L4Proto};
use netiv_core::{DeviceId, DeviceKind, Fault, IfaceId, LinkId, Simulation, TraceKind};
use netiv_diagnostics::l2::owner_of;
use netiv_diagnostics::{Probe, ProbeKind, run_probe};

use crate::{ChaosError, Flow};

/// Tout ce qu'un flux sain utilise.
#[derive(Clone, Debug, Default)]
pub struct PathInfo {
    /// Équipement source du flux.
    pub src: Option<DeviceId>,
    /// Équipement de destination.
    pub dst_device: Option<DeviceId>,
    /// Routeurs et pare-feu traversés à l'aller, dans l'ordre.
    pub routers: Vec<DeviceId>,
    /// Switches traversés.
    pub switches: Vec<DeviceId>,
    /// Câbles utilisés (aller et retour).
    pub links: Vec<LinkId>,
    /// Interfaces des équipements IP (hors switches) utilisées.
    pub ifaces: Vec<(DeviceId, IfaceId)>,
    /// Ports de switch par lesquels entrent les trames.
    pub switch_ports: Vec<(DeviceId, IfaceId)>,
    /// Résolutions ARP effectuées : (qui demande, adresse cherchée).
    pub arp: Vec<(DeviceId, Ipv4Addr)>,
    /// Service visé par le flux.
    pub service: Option<(L4Proto, u16)>,
}

impl PathInfo {
    /// Relève le chemin de `flow` sur `sim` (qui doit fonctionner).
    pub fn trace(sim: &Simulation, flow: &Flow) -> Result<Self, ChaosError> {
        let kind = match flow.service {
            None => ProbeKind::Ping { count: 1 },
            Some((L4Proto::Tcp, port)) => ProbeKind::Tcp { port },
            Some((L4Proto::Udp, port)) => ProbeKind::Udp { port },
        };
        let result = run_probe(
            sim,
            &Probe {
                src: flow.src,
                dst: flow.dst,
                kind,
            },
        )?;
        if !result.success {
            return Err(ChaosError::FlowNotWorking);
        }

        let mut info = PathInfo {
            src: Some(flow.src),
            dst_device: owner_of(sim, flow.dst).map(|(d, _)| d),
            service: flow.service,
            ..PathInfo::default()
        };
        let mut seen_links = BTreeSet::new();
        let mut seen_ifaces = BTreeSet::new();
        let mut seen_ports = BTreeSet::new();
        for e in &result.events {
            let is_switch =
                |d: DeviceId| sim.device(d).is_some_and(|x| x.kind == DeviceKind::Switch);
            match &e.kind {
                TraceKind::FrameSent { iface, .. } | TraceKind::FrameReceived { iface, .. } => {
                    if let Some(l) = sim
                        .device(e.device)
                        .and_then(|d| d.iface(*iface))
                        .and_then(|i| i.link)
                    {
                        if seen_links.insert(l) {
                            info.links.push(l);
                        }
                    }
                    if !is_switch(e.device) && seen_ifaces.insert((e.device, *iface)) {
                        info.ifaces.push((e.device, *iface));
                    }
                    if let TraceKind::FrameReceived { frame, .. } = &e.kind {
                        if is_switch(e.device) {
                            if seen_ports.insert((e.device, *iface)) {
                                info.switch_ports.push((e.device, *iface));
                            }
                            if !info.switches.contains(&e.device) {
                                info.switches.push(e.device);
                            }
                        } else if let EtherPayload::Ipv4(p) = &frame.payload {
                            let forward = p.dst == flow.dst;
                            let router = sim.device(e.device).is_some_and(|d| {
                                matches!(d.kind, DeviceKind::Router | DeviceKind::Firewall)
                            });
                            if forward && router && !info.routers.contains(&e.device) {
                                info.routers.push(e.device);
                            }
                        }
                    }
                }
                TraceKind::ArpRequestSent { target, .. }
                    if !info.arp.contains(&(e.device, *target)) =>
                {
                    info.arp.push((e.device, *target));
                }
                _ => {}
            }
        }
        // Les trames diffusées (ARP) touchent des branches qui ne servent pas au
        // flux. On ne garde que les éléments dont la panne coupe réellement le
        // flux : c'est vérifié sur une copie.
        let breaks = |fault: Fault| -> bool {
            let mut work = sim.fresh_clone();
            if work.inject_fault(fault, false).is_err() {
                return false;
            }
            run_probe(
                &work,
                &Probe {
                    src: flow.src,
                    dst: flow.dst,
                    kind,
                },
            )
            .is_ok_and(|r| !r.success)
        };
        info.links.retain(|l| breaks(Fault::LinkDown { link: *l }));
        info.ifaces.retain(|(d, i)| {
            breaks(Fault::InterfaceDown {
                device: *d,
                iface: *i,
            })
        });
        info.switch_ports.retain(|(sw, port)| {
            // Un port de switch est utile si son câble l'est.
            sim.device(*sw)
                .and_then(|d| d.iface(*port))
                .and_then(|i| i.link)
                .is_some_and(|l| info.links.contains(&l))
        });
        info.switches
            .retain(|sw| info.switch_ports.iter().any(|(d, _)| d == sw));
        Ok(info)
    }
}

//! Analyse de la couche 2 : qui peut joindre qui à travers les switches ?

use std::collections::{BTreeSet, VecDeque};

use netiv_core::device::Behavior;
use netiv_core::{DeviceId, DeviceKind, IfaceId, Simulation};

/// Options de l'analyse.
#[derive(Copy, Clone, Debug)]
pub struct L2Options {
    /// Respecter les VLAN (sinon, les switches laissent tout passer).
    pub vlan_aware: bool,
    /// Respecter l'état des câbles et des interfaces.
    pub honor_state: bool,
}

/// Extrémités (équipements non-switch) qu'une trame diffusée par
/// `(device, iface)` peut atteindre à travers les switches.
pub fn l2_domain(
    sim: &Simulation,
    device: DeviceId,
    iface: IfaceId,
    opts: L2Options,
) -> BTreeSet<(DeviceId, IfaceId)> {
    let mut found = BTreeSet::new();
    let mut seen: BTreeSet<(DeviceId, IfaceId, Option<u16>)> = BTreeSet::new();
    // (équipement, port d'entrée, étiquette portée par la trame)
    let mut queue: VecDeque<(DeviceId, IfaceId, Option<u16>)> = VecDeque::new();

    let Some(first) = peer_of(sim, device, iface, opts) else {
        return found;
    };
    queue.push_back((first.0, first.1, None));

    while let Some((d, port, tag)) = queue.pop_front() {
        let Some(dev) = sim.device(d) else { continue };
        let Behavior::Switch(sw) = &dev.behavior else {
            if (d, port) != (device, iface) {
                found.insert((d, port));
            }
            continue;
        };
        let vlan = if opts.vlan_aware {
            match sw.port_mode(port).ingress_vlan(tag) {
                Some(v) => Some(v),
                None => continue,
            }
        } else {
            None
        };
        if !seen.insert((d, port, vlan)) {
            continue;
        }
        for out in &dev.ifaces {
            if out.id == port {
                continue;
            }
            if opts.honor_state && !out.admin_up {
                continue;
            }
            if opts.vlan_aware {
                let Some(v) = vlan else { continue };
                if !sw.port_mode(out.id).carries(v) {
                    continue;
                }
            }
            let Some((pd, pi)) = peer_of(sim, d, out.id, opts) else {
                continue;
            };
            let next_tag = match vlan {
                Some(v) => sw.port_mode(out.id).egress_tag(v),
                None => None,
            };
            queue.push_back((pd, pi, next_tag));
        }
    }
    found
}

fn peer_of(
    sim: &Simulation,
    device: DeviceId,
    iface: IfaceId,
    opts: L2Options,
) -> Option<(DeviceId, IfaceId)> {
    let dev = sim.device(device)?;
    let ifc = dev.iface(iface)?;
    if opts.honor_state && !ifc.admin_up {
        return None;
    }
    let link = sim.link(ifc.link?)?;
    if opts.honor_state && !link.up {
        return None;
    }
    let (pd, pi) = link.peer_of(device, iface)?;
    if opts.honor_state && !sim.device(pd)?.iface(pi)?.admin_up {
        return None;
    }
    Some((pd, pi))
}

/// Équipement propriétaire d'une adresse IP.
pub fn owner_of(sim: &Simulation, ip: std::net::Ipv4Addr) -> Option<(DeviceId, IfaceId)> {
    sim.devices().iter().find_map(|d| {
        d.ifaces
            .iter()
            .find(|i| i.ip.is_some_and(|n| n.addr() == ip))
            .map(|i| (d.id, i.id))
    })
}

/// `true` si l'équipement est un switch.
pub fn is_switch(sim: &Simulation, d: DeviceId) -> bool {
    sim.device(d).is_some_and(|x| x.kind == DeviceKind::Switch)
}

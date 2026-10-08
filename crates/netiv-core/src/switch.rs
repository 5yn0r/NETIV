//! Switch de niveau 2 : VLAN (access / trunk), apprentissage MAC, flood, forward.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::addr::MacAddr;
use crate::device::{Ctx, Effects};
use crate::ids::IfaceId;
use crate::packet::Frame;
use crate::trace::{DropReason, FloodReason, TraceKind};

/// VLAN par défaut de tous les ports.
pub const DEFAULT_VLAN: u16 = 1;

/// Mode d'un port de switch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PortMode {
    /// Port d'accès : appartient à un seul VLAN, trames non étiquetées.
    Access {
        /// VLAN du port.
        vlan: u16,
    },
    /// Port trunk : transporte plusieurs VLAN, étiquetés 802.1Q.
    Trunk {
        /// VLAN autorisés ; vide = tous.
        allowed: Vec<u16>,
        /// VLAN natif : ses trames circulent sans étiquette.
        native: u16,
    },
}

impl Default for PortMode {
    fn default() -> Self {
        PortMode::Access { vlan: DEFAULT_VLAN }
    }
}

impl PortMode {
    /// Ce port transporte-t-il ce VLAN ?
    pub fn carries(&self, vlan: u16) -> bool {
        match self {
            PortMode::Access { vlan: v } => *v == vlan,
            PortMode::Trunk { allowed, .. } => allowed.is_empty() || allowed.contains(&vlan),
        }
    }

    /// VLAN d'une trame qui arrive avec l'étiquette `tag`, ou `None` si le
    /// port la refuse.
    pub fn ingress_vlan(&self, tag: Option<u16>) -> Option<u16> {
        match (self, tag) {
            (PortMode::Access { vlan }, None) => Some(*vlan),
            (PortMode::Access { vlan }, Some(t)) => (t == *vlan).then_some(t),
            (PortMode::Trunk { native, .. }, None) => self.carries(*native).then_some(*native),
            (PortMode::Trunk { .. }, Some(t)) => self.carries(t).then_some(t),
        }
    }

    /// Étiquette à poser sur une trame du VLAN `vlan` qui sort par ce port.
    pub fn egress_tag(&self, vlan: u16) -> Option<u16> {
        match self {
            PortMode::Access { .. } => None,
            PortMode::Trunk { native, .. } => (vlan != *native).then_some(vlan),
        }
    }
}

/// État d'un switch.
#[derive(Clone, Debug, Default)]
pub struct Switch {
    /// Table d'adresses MAC : `(vlan, adresse)` → port. Pas de vieillissement.
    pub mac_table: BTreeMap<(u16, MacAddr), IfaceId>,
    /// Mode des ports configurés (les autres sont en accès, VLAN 1).
    pub ports: BTreeMap<IfaceId, PortMode>,
}

impl Switch {
    /// Switch avec une table vide.
    pub fn new() -> Self {
        Self::default()
    }

    /// Oublie les adresses apprises (la configuration des ports reste).
    pub fn reset_runtime(&mut self) {
        self.mac_table.clear();
    }

    /// Mode d'un port.
    pub fn port_mode(&self, iface: IfaceId) -> PortMode {
        self.ports.get(&iface).cloned().unwrap_or_default()
    }

    /// Traite une trame reçue sur le port `in_iface`.
    pub fn on_frame(&mut self, ctx: &Ctx, in_iface: IfaceId, frame: Frame, out: &mut Effects) {
        let trace_id = Some(frame.trace_id);

        let Some(vlan) = self.port_mode(in_iface).ingress_vlan(frame.vlan) else {
            out.trace(
                trace_id,
                TraceKind::Dropped {
                    reason: DropReason::VlanMismatch,
                },
            );
            return;
        };

        // 1. Apprendre où se trouve la source, dans ce VLAN.
        if !frame.src.is_multicast() {
            let previous = self.mac_table.insert((vlan, frame.src), in_iface);
            if previous != Some(in_iface) {
                out.trace(
                    trace_id,
                    TraceKind::MacLearned {
                        mac: frame.src,
                        vlan,
                        iface: in_iface,
                        previous,
                    },
                );
            }
        }

        // 2. Décider : flood, forward ou abandon.
        if frame.dst.is_multicast() {
            self.flood(ctx, in_iface, vlan, frame, FloodReason::Broadcast, out);
            return;
        }
        match self.mac_table.get(&(vlan, frame.dst)).copied() {
            Some(port) if port == in_iface => {
                out.trace(
                    trace_id,
                    TraceKind::Dropped {
                        reason: DropReason::SameSegment,
                    },
                );
            }
            Some(port) if self.port_mode(port).carries(vlan) => {
                out.trace(trace_id, TraceKind::Forward { out_iface: port });
                out.send(port, self.tagged(frame, port, vlan));
            }
            _ => self.flood(ctx, in_iface, vlan, frame, FloodReason::UnknownUnicast, out),
        }
    }

    fn tagged(&self, mut frame: Frame, out_port: IfaceId, vlan: u16) -> Frame {
        frame.vlan = self.port_mode(out_port).egress_tag(vlan);
        frame
    }

    fn flood(
        &self,
        ctx: &Ctx,
        in_iface: IfaceId,
        vlan: u16,
        frame: Frame,
        reason: FloodReason,
        out: &mut Effects,
    ) {
        let out_ifaces: Vec<IfaceId> = ctx
            .ifaces
            .iter()
            .filter(|i| {
                i.id != in_iface
                    && i.admin_up
                    && i.link.is_some()
                    && self.port_mode(i.id).carries(vlan)
            })
            .map(|i| i.id)
            .collect();
        out.trace(
            Some(frame.trace_id),
            TraceKind::Flood {
                reason,
                out_ifaces: out_ifaces.clone(),
            },
        );
        for iface in out_ifaces {
            let f = self.tagged(frame.clone(), iface, vlan);
            out.send(iface, f);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_port_semantics() {
        let p = PortMode::Access { vlan: 10 };
        assert!(p.carries(10) && !p.carries(20));
        assert_eq!(p.ingress_vlan(None), Some(10));
        assert_eq!(p.ingress_vlan(Some(10)), Some(10));
        assert_eq!(p.ingress_vlan(Some(20)), None);
        assert_eq!(p.egress_tag(10), None);
    }

    #[test]
    fn trunk_port_semantics() {
        let p = PortMode::Trunk {
            allowed: vec![10, 20],
            native: 10,
        };
        assert!(p.carries(10) && p.carries(20) && !p.carries(30));
        assert_eq!(p.ingress_vlan(None), Some(10), "non étiqueté = VLAN natif");
        assert_eq!(p.ingress_vlan(Some(20)), Some(20));
        assert_eq!(p.ingress_vlan(Some(30)), None);
        assert_eq!(p.egress_tag(10), None, "le natif sort sans étiquette");
        assert_eq!(p.egress_tag(20), Some(20));
    }

    #[test]
    fn trunk_with_empty_allowed_list_carries_everything() {
        let p = PortMode::Trunk {
            allowed: vec![],
            native: 1,
        };
        assert!(p.carries(999));
    }

    #[test]
    fn trunk_refuses_untagged_when_native_is_not_allowed() {
        let p = PortMode::Trunk {
            allowed: vec![20],
            native: 1,
        };
        assert_eq!(p.ingress_vlan(None), None);
    }
}

//! Pannes réversibles : le socle du Chaos Engine.
//!
//! Une panne est une mutation d'état *tracée ou silencieuse* (silencieuse
//! pour les challenges, où l'élève doit la découvrir). Chaque injection garde
//! un instantané de ce qu'elle modifie : la réparer, c'est le restaurer.

use std::collections::{BTreeMap, BTreeSet};
use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};

use crate::acl::{Acl, AclAction, AclRule};
use crate::addr::{Ipv4Net, MacAddr};
use crate::device::Behavior;
use crate::error::NetivError;
use crate::ids::{DeviceId, IfaceId, LinkId};
use crate::packet::L4Proto;
use crate::route::StaticRoute;
use crate::service::Service;
use crate::sim::{Event, Simulation};
use crate::switch::PortMode;
use crate::time::SimTime;
use crate::trace::TraceKind;

/// Une modification de l'état du réseau.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Fault {
    /// Câble coupé.
    LinkDown {
        /// Lien.
        link: LinkId,
    },
    /// Interface désactivée.
    InterfaceDown {
        /// Équipement.
        device: DeviceId,
        /// Interface.
        iface: IfaceId,
    },
    /// Perte de paquets sur un lien.
    LinkLoss {
        /// Lien.
        link: LinkId,
        /// Probabilité de perte (0 à 1).
        loss: f64,
    },
    /// Latence anormale (congestion).
    LinkLatency {
        /// Lien.
        link: LinkId,
        /// Nouvelle latence.
        latency: SimTime,
    },
    /// Mauvaise adresse IP ou mauvais masque.
    SetIp {
        /// Équipement.
        device: DeviceId,
        /// Interface.
        iface: IfaceId,
        /// Nouvelle adresse.
        ip: Option<Ipv4Net>,
    },
    /// Mauvaise passerelle par défaut.
    SetGateway {
        /// Équipement.
        device: DeviceId,
        /// Nouvelle passerelle.
        gateway: Option<Ipv4Addr>,
    },
    /// Route statique supprimée.
    RemoveRoutes {
        /// Équipement.
        device: DeviceId,
        /// Réseau dont on retire les routes.
        dest: Ipv4Net,
    },
    /// Route statique ajoutée (incorrecte, ou formant une boucle).
    AddRoute {
        /// Équipement.
        device: DeviceId,
        /// Route.
        route: StaticRoute,
    },
    /// Un routeur cesse de transférer les paquets.
    ForwardingOff {
        /// Équipement.
        device: DeviceId,
    },
    /// Entrée ARP statique incorrecte.
    StaticArp {
        /// Équipement.
        device: DeviceId,
        /// IP.
        ip: Ipv4Addr,
        /// MAC (erronée).
        mac: MacAddr,
    },
    /// Mauvais mode ou mauvais VLAN sur un port de switch.
    PortMode {
        /// Switch.
        device: DeviceId,
        /// Port.
        iface: IfaceId,
        /// Nouveau mode.
        mode: PortMode,
    },
    /// Un service s'arrête.
    StopService {
        /// Équipement.
        device: DeviceId,
        /// Protocole.
        proto: L4Proto,
        /// Port.
        port: u16,
    },
    /// Une règle de filtrage est insérée en tête.
    AclInsert {
        /// Équipement.
        device: DeviceId,
        /// Règle.
        rule: AclRule,
    },
    /// La politique de filtrage par défaut change.
    AclDefault {
        /// Équipement.
        device: DeviceId,
        /// Nouvelle politique.
        action: AclAction,
    },
    /// Le serveur DHCP s'arrête.
    DhcpServerDown {
        /// Équipement qui héberge le serveur.
        device: DeviceId,
    },
    /// Les plages d'un serveur DHCP changent (plage réduite, absente…) ; sur
    /// un équipement sans serveur, en installe un (serveur pirate).
    DhcpPools {
        /// Équipement.
        device: DeviceId,
        /// Nouvelles plages (vide : plus de serveur).
        pools: Vec<crate::dhcp::DhcpPool>,
    },
    /// Le relais DHCP d'une interface de routeur change ou disparaît.
    DhcpRelay {
        /// Routeur.
        device: DeviceId,
        /// Interface.
        iface: IfaceId,
        /// Serveur visé (`None` : plus de relais).
        server: Option<Ipv4Addr>,
    },
    /// Mauvais serveur DNS sur un équipement.
    SetDns {
        /// Équipement.
        device: DeviceId,
        /// Serveur configuré (`None` : aucun).
        server: Option<Ipv4Addr>,
    },
    /// Enregistrement DNS modifié ou supprimé.
    DnsRecord {
        /// Serveur DNS.
        device: DeviceId,
        /// Nom.
        name: String,
        /// Nouvelle adresse (`None` : nom supprimé).
        ip: Option<Ipv4Addr>,
    },
    /// Le serveur DNS s'arrête.
    DnsServerDown {
        /// Équipement qui héberge le serveur.
        device: DeviceId,
    },
    /// Plusieurs pannes simultanées.
    Multi(Vec<Fault>),
}

/// Ce qu'une panne touche.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Target {
    /// Un équipement.
    Device(DeviceId),
    /// Un lien.
    Link(LinkId),
}

impl Fault {
    /// Équipements et liens modifiés par cette panne.
    pub fn targets(&self) -> Vec<Target> {
        match self {
            Fault::LinkDown { link }
            | Fault::LinkLoss { link, .. }
            | Fault::LinkLatency { link, .. } => vec![Target::Link(*link)],
            Fault::InterfaceDown { device, .. }
            | Fault::SetIp { device, .. }
            | Fault::SetGateway { device, .. }
            | Fault::RemoveRoutes { device, .. }
            | Fault::AddRoute { device, .. }
            | Fault::ForwardingOff { device }
            | Fault::StaticArp { device, .. }
            | Fault::PortMode { device, .. }
            | Fault::StopService { device, .. }
            | Fault::AclInsert { device, .. }
            | Fault::AclDefault { device, .. }
            | Fault::DhcpServerDown { device }
            | Fault::DhcpPools { device, .. }
            | Fault::DhcpRelay { device, .. }
            | Fault::SetDns { device, .. }
            | Fault::DnsRecord { device, .. }
            | Fault::DnsServerDown { device } => vec![Target::Device(*device)],
            Fault::Multi(list) => {
                let mut all: Vec<Target> = list.iter().flat_map(Fault::targets).collect();
                all.sort();
                all.dedup();
                all
            }
        }
    }
}

/// Configuration d'un équipement (sans état dynamique).
#[derive(Clone, Debug)]
struct DeviceConfig {
    ifaces: Vec<(Option<Ipv4Net>, bool)>,
    ip: Option<IpConfig>,
    ports: Option<BTreeMap<IfaceId, PortMode>>,
}

#[derive(Clone, Debug)]
struct IpConfig {
    forwarding: bool,
    gateway: Option<Ipv4Addr>,
    routes: Vec<StaticRoute>,
    services: Vec<Service>,
    static_arp: BTreeMap<Ipv4Addr, MacAddr>,
    acl: Acl,
    dhcp_ifaces: BTreeSet<IfaceId>,
    dhcp_relay: BTreeMap<IfaceId, Ipv4Addr>,
    /// Serveur DHCP : actif ?, plages. Les baux (état dynamique) n'en font pas partie.
    dhcp_server: Option<(bool, Vec<crate::dhcp::DhcpPool>)>,
    dns: Option<Ipv4Addr>,
    dns_server: Option<crate::dns::DnsServer>,
}

#[derive(Clone, Debug)]
enum Snapshot {
    Device(DeviceId, Box<DeviceConfig>),
    Link(LinkId, bool, f64, SimTime),
}

/// Une panne appliquée au réseau.
#[derive(Clone, Debug)]
pub struct ActiveFault {
    /// Numéro de la panne.
    pub id: u64,
    /// La panne.
    pub fault: Fault,
    /// Description lisible (avec les valeurs d'avant).
    pub description: String,
    /// Annoncée dans la trace ?
    pub visible: bool,
    /// Date d'injection.
    pub applied_at: SimTime,
    /// Toujours en vigueur ?
    pub active: bool,
    snapshots: Vec<Snapshot>,
}

impl Simulation {
    /// Pannes injectées jusqu'ici (réparées ou non).
    pub fn faults(&self) -> &[ActiveFault] {
        &self.faults
    }

    /// Injecte une panne. `visible = false` : aucune trace (challenge).
    /// Retourne son numéro.
    pub fn inject_fault(&mut self, fault: Fault, visible: bool) -> Result<u64, NetivError> {
        self.check_fault(&fault)?;
        let description = self.describe_fault(&fault);
        let snapshots = fault
            .targets()
            .into_iter()
            .filter_map(|t| self.snapshot(t))
            .collect();
        self.mutate(&fault)?;
        let id = self.faults.len() as u64 + 1;
        if visible {
            let dev = self.first_device(&fault);
            self.record_public(
                dev,
                TraceKind::FaultApplied {
                    description: description.clone(),
                },
            );
        }
        self.faults.push(ActiveFault {
            id,
            fault,
            description,
            visible,
            applied_at: self.now,
            active: true,
            snapshots,
        });
        Ok(id)
    }

    /// Programme une panne dans `delay`.
    pub fn schedule_fault(
        &mut self,
        delay: SimTime,
        fault: Fault,
        visible: bool,
    ) -> Result<(), NetivError> {
        self.check_fault(&fault)?;
        self.schedule_in(delay, Event::Fault { fault, visible })?;
        Ok(())
    }

    /// Répare une panne : restaure ce qu'elle avait modifié.
    pub fn repair_fault(&mut self, id: u64, visible: bool) -> Result<(), NetivError> {
        let idx = self
            .faults
            .iter()
            .position(|f| f.id == id)
            .ok_or(NetivError::NoSuchFault(id))?;
        if !self.faults[idx].active {
            return Err(NetivError::FaultAlreadyRepaired(id));
        }
        let snaps = std::mem::take(&mut self.faults[idx].snapshots);
        for snap in snaps.iter().rev() {
            self.restore(snap);
        }
        self.faults[idx].snapshots = snaps;
        self.faults[idx].active = false;
        if visible {
            let description = self.faults[idx].description.clone();
            let dev = self.first_device(&self.faults[idx].fault.clone());
            self.record_public(dev, TraceKind::FaultRepaired { description });
        }
        Ok(())
    }

    /// Répare toutes les pannes en vigueur, de la plus récente à la plus ancienne.
    pub fn repair_all_faults(&mut self, visible: bool) -> usize {
        let ids: Vec<u64> = self
            .faults
            .iter()
            .filter(|f| f.active)
            .map(|f| f.id)
            .collect();
        let n = ids.len();
        for id in ids.into_iter().rev() {
            let _ = self.repair_fault(id, visible);
        }
        n
    }

    /// Représentation textuelle déterministe de toute la configuration
    /// (adresses, routes, services, filtrage, VLAN, état des câbles), sans
    /// l'état dynamique. Deux réseaux configurés pareil ont la même empreinte.
    pub fn config_fingerprint(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        for d in &self.devices {
            if let Some(s) = self.snapshot(Target::Device(d.id)) {
                parts.push(format!("{} {:?}", d.name, s));
            }
        }
        for l in &self.links {
            if let Some(s) = self.snapshot(Target::Link(l.id)) {
                parts.push(format!("{:?}", s));
            }
        }
        parts.join("\n")
    }

    /// Copie de la simulation sans aucun état dynamique : files d'événements,
    /// caches ARP, tables MAC, connexions et trace sont vidés. C'est le point
    /// de départ des sondes (diagnostic, what-if) : le résultat ne dépend pas
    /// de ce qui s'est passé avant.
    pub fn fresh_clone(&self) -> Simulation {
        let mut sim = self.clone_without_history();
        sim.reset_runtime();
        sim
    }

    /// Copie propre dont tout le filtrage est retiré (aucune règle, politique
    /// « allow »). Sert à mesurer ce que le **routage** permet, indépendamment
    /// de ce que le filtrage autorise.
    pub fn unfiltered_clone(&self) -> Simulation {
        let mut sim = self.fresh_clone();
        for d in &mut sim.devices {
            if let Behavior::Ip(n) = &mut d.behavior {
                n.acl = crate::acl::Acl::default();
            }
        }
        sim
    }

    /// Vide l'état dynamique en conservant la configuration.
    pub fn reset_runtime(&mut self) {
        self.queue = crate::queue::EventQueue::new();
        self.now = SimTime::ZERO;
        self.trace.clear();
        for d in &mut self.devices {
            match &mut d.behavior {
                Behavior::Ip(n) => n.reset_runtime(),
                Behavior::Switch(s) => s.reset_runtime(),
            }
        }
    }

    fn first_device(&self, fault: &Fault) -> DeviceId {
        for t in fault.targets() {
            match t {
                Target::Device(d) => return d,
                Target::Link(l) => {
                    if let Some(link) = self.link(l) {
                        return link.a.0;
                    }
                }
            }
        }
        DeviceId(0)
    }

    fn record_public(&mut self, device: DeviceId, kind: TraceKind) {
        self.trace.push(self.now, device, None, None, kind);
    }

    fn check_fault(&self, fault: &Fault) -> Result<(), NetivError> {
        match fault {
            Fault::Multi(list) => list.iter().try_for_each(|f| self.check_fault(f)),
            Fault::LinkLoss { loss, .. } if !(0.0..=1.0).contains(loss) => {
                Err(NetivError::InvalidLoss(*loss))
            }
            _ => {
                for t in fault.targets() {
                    match t {
                        Target::Device(d) => {
                            self.dev_ref(d)?;
                        }
                        Target::Link(l) => {
                            self.link(l)
                                .ok_or_else(|| NetivError::UnknownDevice(l.to_string()))?;
                        }
                    }
                }
                // Cas particuliers : les interfaces et les types doivent exister.
                match fault {
                    Fault::InterfaceDown { device, iface }
                    | Fault::SetIp { device, iface, .. }
                    | Fault::PortMode { device, iface, .. } => {
                        let dev = self.dev_ref(*device)?;
                        dev.iface(*iface)
                            .ok_or_else(|| NetivError::UnknownInterface {
                                device: dev.name.clone(),
                                iface: iface.to_string(),
                            })?;
                        if matches!(fault, Fault::PortMode { .. })
                            && !matches!(dev.behavior, Behavior::Switch(_))
                        {
                            return Err(NetivError::NotASwitch(dev.name.clone()));
                        }
                        if matches!(fault, Fault::SetIp { .. })
                            && matches!(dev.behavior, Behavior::Switch(_))
                        {
                            return Err(NetivError::NotAnIpDevice(dev.name.clone()));
                        }
                    }
                    Fault::SetGateway { device, .. }
                    | Fault::RemoveRoutes { device, .. }
                    | Fault::AddRoute { device, .. }
                    | Fault::ForwardingOff { device }
                    | Fault::StaticArp { device, .. }
                    | Fault::StopService { device, .. }
                    | Fault::AclInsert { device, .. }
                    | Fault::AclDefault { device, .. }
                    | Fault::DhcpServerDown { device }
                    | Fault::DhcpPools { device, .. }
                    | Fault::DhcpRelay { device, .. }
                    | Fault::SetDns { device, .. }
                    | Fault::DnsRecord { device, .. }
                    | Fault::DnsServerDown { device } => {
                        let dev = self.dev_ref(*device)?;
                        if let Fault::DhcpRelay { iface, .. } = fault {
                            dev.iface(*iface)
                                .ok_or_else(|| NetivError::UnknownInterface {
                                    device: dev.name.clone(),
                                    iface: iface.to_string(),
                                })?;
                        }
                        if matches!(dev.behavior, Behavior::Switch(_)) {
                            return Err(NetivError::NotAnIpDevice(dev.name.clone()));
                        }
                    }
                    _ => {}
                }
                Ok(())
            }
        }
    }

    fn dev_ref(&self, id: DeviceId) -> Result<&crate::device::Device, NetivError> {
        self.device(id)
            .ok_or_else(|| NetivError::UnknownDevice(id.to_string()))
    }

    fn snapshot(&self, t: Target) -> Option<Snapshot> {
        match t {
            Target::Link(l) => self
                .link(l)
                .map(|k| Snapshot::Link(l, k.up, k.loss, k.latency)),
            Target::Device(d) => {
                let dev = self.device(d)?;
                let (ip, ports) = match &dev.behavior {
                    Behavior::Ip(n) => {
                        let mut acl = n.acl.clone();
                        acl.reset_tracking();
                        (
                            Some(IpConfig {
                                forwarding: n.forwarding,
                                gateway: n.gateway,
                                routes: n.static_routes.clone(),
                                services: n.services.clone(),
                                static_arp: n.static_arp.clone(),
                                acl,
                                dhcp_ifaces: n.dhcp_ifaces.clone(),
                                dhcp_relay: n.dhcp_relay.clone(),
                                dhcp_server: n
                                    .dhcp_server
                                    .as_ref()
                                    .map(|srv| (srv.enabled, srv.pools.clone())),
                                dns: n.dns,
                                dns_server: n.dns_server.clone(),
                            }),
                            None,
                        )
                    }
                    Behavior::Switch(s) => (None, Some(s.ports.clone())),
                };
                Some(Snapshot::Device(
                    d,
                    Box::new(DeviceConfig {
                        ifaces: dev.ifaces.iter().map(|i| (i.ip, i.admin_up)).collect(),
                        ip,
                        ports,
                    }),
                ))
            }
        }
    }

    fn restore(&mut self, snap: &Snapshot) {
        match snap {
            Snapshot::Link(l, up, loss, latency) => {
                if let Some(k) = self.links.get_mut(l.0 as usize) {
                    k.up = *up;
                    k.loss = *loss;
                    k.latency = *latency;
                }
            }
            Snapshot::Device(d, cfg) => {
                let Some(dev) = self.devices.get_mut(d.0 as usize) else {
                    return;
                };
                for (ifc, (ip, up)) in dev.ifaces.iter_mut().zip(&cfg.ifaces) {
                    ifc.ip = *ip;
                    ifc.admin_up = *up;
                }
                match (&mut dev.behavior, &cfg.ip, &cfg.ports) {
                    (Behavior::Ip(n), Some(c), _) => {
                        n.forwarding = c.forwarding;
                        n.gateway = c.gateway;
                        n.static_routes = c.routes.clone();
                        n.services = c.services.clone();
                        n.static_arp = c.static_arp.clone();
                        n.acl = c.acl.clone();
                        n.dhcp_ifaces = c.dhcp_ifaces.clone();
                        n.dhcp_relay = c.dhcp_relay.clone();
                        match (&c.dhcp_server, &mut n.dhcp_server) {
                            (None, slot) => *slot = None,
                            (Some((on, pools)), Some(srv)) => {
                                srv.enabled = *on;
                                srv.pools = pools.clone();
                            }
                            (Some((on, pools)), slot) => {
                                let mut srv = crate::dhcp::DhcpServer::new(pools.clone());
                                srv.enabled = *on;
                                *slot = Some(srv);
                            }
                        }
                        n.dns = c.dns;
                        n.dns_server = c.dns_server.clone();
                    }
                    (Behavior::Switch(s), _, Some(p)) => {
                        s.ports = p.clone();
                        s.mac_table.clear();
                    }
                    _ => {}
                }
            }
        }
    }

    /// Applique la mutation, sans aucune trace.
    fn mutate(&mut self, fault: &Fault) -> Result<(), NetivError> {
        match fault {
            Fault::Multi(list) => {
                for f in list {
                    self.mutate(f)?;
                }
            }
            Fault::LinkDown { link } => self.link_mut(*link)?.up = false,
            Fault::LinkLoss { link, loss } => self.link_mut(*link)?.loss = *loss,
            Fault::LinkLatency { link, latency } => self.link_mut(*link)?.latency = *latency,
            Fault::InterfaceDown { device, iface } => {
                self.iface_mut(*device, *iface)?.admin_up = false
            }
            Fault::SetIp { device, iface, ip } => {
                self.iface_mut(*device, *iface)?.ip = *ip;
                // Une adresse fixée à la main : l'interface n'est plus cliente
                // DHCP (sinon le prochain renouvellement effacerait la panne).
                // L'état d'avant (DHCP compris) est restauré par l'annulation.
                if ip.is_some() {
                    if let Ok(node) = self.ip_cfg(*device) {
                        node.dhcp_ifaces.remove(iface);
                        node.dhcp_clients.remove(iface);
                    }
                }
            }
            Fault::SetGateway { device, gateway } => self.ip_cfg(*device)?.gateway = *gateway,
            Fault::RemoveRoutes { device, dest } => {
                let dest = dest.trunc();
                self.ip_cfg(*device)?
                    .static_routes
                    .retain(|r| r.dest.trunc() != dest);
            }
            Fault::AddRoute { device, route } => self.ip_cfg(*device)?.static_routes.push(*route),
            Fault::ForwardingOff { device } => self.ip_cfg(*device)?.forwarding = false,
            Fault::StaticArp { device, ip, mac } => {
                self.ip_cfg(*device)?.static_arp.insert(*ip, *mac);
            }
            Fault::PortMode {
                device,
                iface,
                mode,
            } => {
                let dev = self
                    .devices
                    .get_mut(device.0 as usize)
                    .ok_or_else(|| NetivError::UnknownDevice(device.to_string()))?;
                if let Behavior::Switch(s) = &mut dev.behavior {
                    s.ports.insert(*iface, mode.clone());
                    s.mac_table.clear();
                }
            }
            Fault::StopService {
                device,
                proto,
                port,
            } => self
                .ip_cfg(*device)?
                .services
                .retain(|s| !(s.proto == *proto && s.port == *port)),
            Fault::AclInsert { device, rule } => {
                let n = self.ip_cfg(*device)?;
                n.acl.rules.insert(0, rule.clone());
                n.acl.reset_tracking();
            }
            Fault::AclDefault { device, action } => {
                let n = self.ip_cfg(*device)?;
                n.acl.default = *action;
                n.acl.reset_tracking();
            }
            Fault::DhcpServerDown { device } => {
                if let Some(srv) = &mut self.ip_cfg(*device)?.dhcp_server {
                    srv.enabled = false;
                }
            }
            Fault::DhcpPools { device, pools } => {
                let n = self.ip_cfg(*device)?;
                if pools.is_empty() {
                    n.dhcp_server = None;
                } else {
                    match &mut n.dhcp_server {
                        Some(srv) => srv.pools = pools.clone(),
                        None => n.dhcp_server = Some(crate::dhcp::DhcpServer::new(pools.clone())),
                    }
                }
            }
            Fault::DhcpRelay {
                device,
                iface,
                server,
            } => {
                let n = self.ip_cfg(*device)?;
                match server {
                    Some(s) => n.dhcp_relay.insert(*iface, *s),
                    None => n.dhcp_relay.remove(iface),
                };
            }
            Fault::SetDns { device, server } => self.ip_cfg(*device)?.dns = *server,
            Fault::DnsRecord { device, name, ip } => {
                let key = crate::dns::normalize(name);
                let srv = self
                    .ip_cfg(*device)?
                    .dns_server
                    .get_or_insert_with(|| crate::dns::DnsServer::new(BTreeMap::new()));
                match ip {
                    Some(a) => srv.zone.insert(key, *a),
                    None => srv.zone.remove(&key),
                };
            }
            Fault::DnsServerDown { device } => {
                if let Some(srv) = &mut self.ip_cfg(*device)?.dns_server {
                    srv.enabled = false;
                }
            }
        }
        Ok(())
    }

    fn link_mut(&mut self, id: LinkId) -> Result<&mut crate::sim::Link, NetivError> {
        self.links
            .get_mut(id.0 as usize)
            .ok_or_else(|| NetivError::UnknownDevice(id.to_string()))
    }

    fn iface_mut(
        &mut self,
        dev: DeviceId,
        iface: IfaceId,
    ) -> Result<&mut crate::device::Interface, NetivError> {
        let d = self
            .devices
            .get_mut(dev.0 as usize)
            .ok_or_else(|| NetivError::UnknownDevice(dev.to_string()))?;
        let name = d.name.clone();
        d.ifaces
            .get_mut(iface.0 as usize)
            .ok_or(NetivError::UnknownInterface {
                device: name,
                iface: iface.to_string(),
            })
    }

    fn ip_cfg(&mut self, dev: DeviceId) -> Result<&mut crate::ip_node::IpNode, NetivError> {
        let d = self
            .devices
            .get_mut(dev.0 as usize)
            .ok_or_else(|| NetivError::UnknownDevice(dev.to_string()))?;
        let name = d.name.clone();
        match &mut d.behavior {
            Behavior::Ip(n) => Ok(n),
            Behavior::Switch(_) => Err(NetivError::NotAnIpDevice(name)),
        }
    }

    /// Description lisible d'une panne, avec les valeurs actuelles
    /// (c'est-à-dire « d'avant » : à appeler avant la mutation).
    pub fn describe_fault(&self, fault: &Fault) -> String {
        let dn = |d: &DeviceId| self.device(*d).map_or("?".to_string(), |x| x.name.clone());
        let inn = |d: &DeviceId, i: &IfaceId| {
            self.device(*d)
                .and_then(|x| x.iface(*i))
                .map_or("?".to_string(), |x| format!("{}/{}", dn(d), x.name))
        };
        let ln = |l: &LinkId| match self.link(*l) {
            Some(k) => format!("câble {} ↔ {}", inn(&k.a.0, &k.a.1), inn(&k.b.0, &k.b.1)),
            None => l.to_string(),
        };
        match fault {
            Fault::LinkDown { link } => format!("{} coupé", ln(link)),
            Fault::InterfaceDown { device, iface } => {
                format!("interface {} désactivée", inn(device, iface))
            }
            Fault::LinkLoss { link, loss } => {
                format!("{} : {:.0} % de perte de paquets", ln(link), loss * 100.0)
            }
            Fault::LinkLatency { link, latency } => {
                format!("{} : latence portée à {latency}", ln(link))
            }
            Fault::SetIp { device, iface, ip } => {
                let before = self
                    .device(*device)
                    .and_then(|d| d.iface(*iface))
                    .and_then(|i| i.ip)
                    .map_or("aucune".to_string(), |n| n.to_string());
                let after = ip.map_or("aucune".to_string(), |n| n.to_string());
                format!(
                    "{} : adresse {after} au lieu de {before}",
                    inn(device, iface)
                )
            }
            Fault::SetGateway { device, gateway } => {
                let before = match self.device(*device).map(|d| &d.behavior) {
                    Some(Behavior::Ip(n)) => {
                        n.gateway.map_or("aucune".to_string(), |g| g.to_string())
                    }
                    _ => "?".to_string(),
                };
                let after = gateway.map_or("aucune".to_string(), |g| g.to_string());
                format!("{} : passerelle {after} au lieu de {before}", dn(device))
            }
            Fault::RemoveRoutes { device, dest } => {
                format!("{} : route vers {} supprimée", dn(device), dest.trunc())
            }
            Fault::AddRoute { device, route } => format!(
                "{} : route {} via {} ajoutée",
                dn(device),
                route.dest.trunc(),
                route.via
            ),
            Fault::ForwardingOff { device } => {
                format!("{} : le transfert de paquets est désactivé", dn(device))
            }
            Fault::StaticArp { device, ip, mac } => {
                format!("{} : entrée ARP statique erronée {ip} → {mac}", dn(device))
            }
            Fault::PortMode {
                device,
                iface,
                mode,
            } => {
                let m = match mode {
                    PortMode::Access { vlan } => format!("accès VLAN {vlan}"),
                    PortMode::Trunk { allowed, native } => format!(
                        "trunk ({}) natif {native}",
                        if allowed.is_empty() {
                            "tous".to_string()
                        } else {
                            allowed
                                .iter()
                                .map(u16::to_string)
                                .collect::<Vec<_>>()
                                .join(",")
                        }
                    ),
                };
                format!("{} : port configuré en {m}", inn(device, iface))
            }
            Fault::StopService {
                device,
                proto,
                port,
            } => format!("{} : service {proto}/{port} arrêté", dn(device)),
            Fault::AclInsert { device, rule } => {
                let dev = self.device(*device);
                let text = rule.describe(|i| {
                    dev.and_then(|d| d.iface(i))
                        .map_or("?".to_string(), |x| x.name.clone())
                });
                format!("{} : règle de filtrage ajoutée — {text}", dn(device))
            }
            Fault::AclDefault { device, action } => {
                format!(
                    "{} : politique de filtrage par défaut = {action}",
                    dn(device)
                )
            }
            Fault::DhcpServerDown { device } => format!("{} : serveur DHCP arrêté", dn(device)),
            Fault::DhcpPools { device, pools } => {
                if pools.is_empty() {
                    format!("{} : serveur DHCP supprimé", dn(device))
                } else {
                    let list = pools
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(" ; ");
                    format!("{} : serveur DHCP avec les plages {list}", dn(device))
                }
            }
            Fault::DhcpRelay {
                device,
                iface,
                server,
            } => match server {
                Some(srv) => format!("{} : relais DHCP vers {srv}", inn(device, iface)),
                None => format!("{} : relais DHCP supprimé", inn(device, iface)),
            },
            Fault::SetDns { device, server } => {
                let after = server.map_or("aucun".to_string(), |s| s.to_string());
                format!("{} : serveur DNS {after}", dn(device))
            }
            Fault::DnsRecord { device, name, ip } => match ip {
                Some(a) => format!("{} : enregistrement DNS {name} = {a}", dn(device)),
                None => format!("{} : enregistrement DNS {name} supprimé", dn(device)),
            },
            Fault::DnsServerDown { device } => format!("{} : serveur DNS arrêté", dn(device)),
            Fault::Multi(list) => list
                .iter()
                .map(|f| self.describe_fault(f))
                .collect::<Vec<_>>()
                .join(" ; "),
        }
    }
}

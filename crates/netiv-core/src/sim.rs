//! Simulation à événements discrets : le monde (équipements, liens), la file
//! d'événements, l'horloge virtuelle et la trace.

use std::collections::BTreeMap;
use std::fmt;
use std::net::Ipv4Addr;
use std::ops::Range;

use crate::acl::{AclAction, AclRule};
use crate::addr::{Ipv4Net, MacAddr};
use crate::device::{Behavior, Command, Ctx, Device, DeviceKind, Effect, Effects, Interface};
use crate::error::NetivError;
use crate::ids::{DeviceId, IfaceId, LinkId, TimerId, TraceId};
use crate::ip_node::IpNode;
use crate::packet::{Frame, L4Proto};
use crate::queue::EventQueue;
use crate::rng::SimRng;
use crate::route::StaticRoute;
use crate::service::Service;
use crate::switch::{PortMode, Switch};
use crate::time::SimTime;
use crate::trace::{DropReason, TraceKind, TraceLog};

/// Nombre maximal d'événements traités par `run_until_idle` avant abandon.
/// Protège contre les simulations qui ne s'arrêtent jamais.
pub const DEFAULT_EVENT_LIMIT: u64 = 10_000_000;

/// Latence par défaut d'un lien (1 ms : lisible pour l'apprentissage).
pub const DEFAULT_LINK_LATENCY: SimTime = SimTime::from_millis(1);

/// Événement du moteur.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// Une trame arrive sur l'interface d'un équipement.
    FrameArrival {
        /// Équipement récepteur.
        device: DeviceId,
        /// Interface de réception.
        iface: IfaceId,
        /// Trame.
        frame: Frame,
        /// Événement de trace `FrameSent` correspondant.
        cause: Option<u64>,
    },
    /// Un timer d'équipement expire.
    Timer {
        /// Équipement concerné.
        device: DeviceId,
        /// Timer qui expire.
        timer: TimerId,
    },
    /// Une panne planifiée se déclenche.
    Fault {
        /// La panne.
        fault: crate::fault::Fault,
        /// Annoncée dans la trace ?
        visible: bool,
    },
    /// Une commande utilisateur arrive à un équipement.
    Command {
        /// Équipement concerné.
        device: DeviceId,
        /// Commande.
        cmd: Command,
    },
}

/// Résumé d'un événement traité par [`Simulation::step`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fired {
    /// Date de l'événement (la simulation est maintenant à cette date).
    pub time: SimTime,
    /// Rang d'insertion de l'événement dans la file.
    pub seq: u64,
    /// Indices des événements de trace produits par ce pas.
    pub trace: Range<usize>,
}

/// Paramètres d'un lien.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct LinkParams {
    /// Délai de propagation.
    pub latency: SimTime,
    /// Probabilité de perte d'une trame, dans `[0, 1]`.
    pub loss: f64,
}

impl Default for LinkParams {
    fn default() -> Self {
        LinkParams {
            latency: DEFAULT_LINK_LATENCY,
            loss: 0.0,
        }
    }
}

/// Câble entre deux interfaces.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Link {
    /// Identifiant.
    pub id: LinkId,
    /// Première extrémité.
    pub a: (DeviceId, IfaceId),
    /// Seconde extrémité.
    pub b: (DeviceId, IfaceId),
    /// Délai de propagation.
    pub latency: SimTime,
    /// Probabilité de perte.
    pub loss: f64,
    /// Câble en état de marche.
    pub up: bool,
}

impl Link {
    /// Extrémité opposée à `(device, iface)`.
    pub fn peer_of(&self, device: DeviceId, iface: IfaceId) -> Option<(DeviceId, IfaceId)> {
        if self.a == (device, iface) {
            Some(self.b)
        } else if self.b == (device, iface) {
            Some(self.a)
        } else {
            None
        }
    }
}

/// Une simulation en cours.
#[derive(Debug, Clone)]
pub struct Simulation {
    seed: u64,
    pub(crate) now: SimTime,
    pub(crate) queue: EventQueue<Event>,
    pub(crate) rng: SimRng,
    event_limit: u64,
    processed: u64,
    pub(crate) devices: Vec<Device>,
    pub(crate) links: Vec<Link>,
    names: BTreeMap<String, DeviceId>,
    pub(crate) trace: TraceLog,
    pub(crate) faults: Vec<crate::fault::ActiveFault>,
    next_trace: u64,
    next_mac: u32,
}

impl Simulation {
    /// Crée une simulation vide à `t = 0`, avec la graine donnée.
    pub fn new(seed: u64) -> Self {
        Simulation {
            seed,
            now: SimTime::ZERO,
            queue: EventQueue::new(),
            rng: SimRng::new(seed),
            event_limit: DEFAULT_EVENT_LIMIT,
            processed: 0,
            devices: Vec::new(),
            links: Vec::new(),
            names: BTreeMap::new(),
            trace: TraceLog::default(),
            faults: Vec::new(),
            next_trace: 1,
            next_mac: 1,
        }
    }

    /// Copie de la simulation **sans** sa file d'événements ni sa trace.
    ///
    /// Bien moins coûteux que `clone()` après une longue session : la trace
    /// peut compter des centaines de milliers d'événements. Le reste de l'état
    /// (horloge, caches, sessions) est copié tel quel ; voir
    /// [`Simulation::fresh_clone`] pour une copie remise à zéro.
    pub(crate) fn clone_without_history(&self) -> Simulation {
        Simulation {
            seed: self.seed,
            now: self.now,
            queue: EventQueue::new(),
            rng: self.rng.clone(),
            event_limit: self.event_limit,
            processed: self.processed,
            devices: self.devices.clone(),
            links: self.links.clone(),
            names: self.names.clone(),
            trace: TraceLog::default(),
            faults: self.faults.clone(),
            next_trace: self.next_trace,
            next_mac: self.next_mac,
        }
    }

    // ------------------------------------------------------------------
    // Lecture
    // ------------------------------------------------------------------

    /// Graine du générateur aléatoire.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Date courante de la simulation.
    pub fn now(&self) -> SimTime {
        self.now
    }

    /// Nombre d'événements en attente.
    pub fn pending(&self) -> usize {
        self.queue.len()
    }

    /// Nombre d'événements déjà traités.
    pub fn processed(&self) -> u64 {
        self.processed
    }

    /// Vide le journal de trace et rend ses événements, sans les copier
    /// (utile aux sondes qui jettent la simulation ensuite).
    pub fn take_trace(&mut self) -> Vec<crate::trace::TraceEvent> {
        self.trace.take()
    }

    /// Journal de trace.
    pub fn trace(&self) -> &TraceLog {
        &self.trace
    }

    /// Efface la trace. À appeler une fois la construction terminée, pour que
    /// la configuration initiale n'encombre pas le journal.
    pub fn clear_trace(&mut self) {
        self.trace.clear();
    }

    /// Tous les équipements.
    pub fn devices(&self) -> &[Device] {
        &self.devices
    }

    /// Équipement par identifiant.
    pub fn device(&self, id: DeviceId) -> Option<&Device> {
        self.devices.get(id.0 as usize)
    }

    /// Tous les liens.
    pub fn links(&self) -> &[Link] {
        &self.links
    }

    /// Lien par identifiant.
    pub fn link(&self, id: LinkId) -> Option<&Link> {
        self.links.get(id.0 as usize)
    }

    /// Identifiant d'un équipement à partir de son nom.
    pub fn device_id(&self, name: &str) -> Result<DeviceId, NetivError> {
        self.names
            .get(name)
            .copied()
            .ok_or_else(|| NetivError::UnknownDevice(name.to_string()))
    }

    /// Identifiant d'une interface à partir du nom de l'équipement et du sien.
    pub fn iface_id(&self, device: DeviceId, name: &str) -> Result<IfaceId, NetivError> {
        let dev = self.dev(device)?;
        dev.iface_by_name(name)
            .map(|i| i.id)
            .ok_or_else(|| NetivError::UnknownInterface {
                device: dev.name.clone(),
                iface: name.to_string(),
            })
    }

    /// Générateur aléatoire de la simulation.
    pub fn rng_mut(&mut self) -> &mut SimRng {
        &mut self.rng
    }

    /// Change la limite d'événements de `run_until_idle`.
    pub fn set_event_limit(&mut self, limit: u64) {
        self.event_limit = limit;
    }

    fn dev(&self, id: DeviceId) -> Result<&Device, NetivError> {
        self.device(id)
            .ok_or_else(|| NetivError::UnknownDevice(id.to_string()))
    }

    fn dev_mut(&mut self, id: DeviceId) -> Result<&mut Device, NetivError> {
        self.devices
            .get_mut(id.0 as usize)
            .ok_or_else(|| NetivError::UnknownDevice(id.to_string()))
    }

    // ------------------------------------------------------------------
    // Construction
    // ------------------------------------------------------------------

    /// Ajoute un équipement.
    pub fn add_device(&mut self, name: &str, kind: DeviceKind) -> Result<DeviceId, NetivError> {
        if name.is_empty() || name.contains(char::is_whitespace) || name.contains(':') {
            return Err(NetivError::InvalidName(name.to_string()));
        }
        if self.names.contains_key(name) {
            return Err(NetivError::DuplicateDevice(name.to_string()));
        }
        let id = DeviceId(self.devices.len() as u32);
        let behavior = match kind {
            DeviceKind::Host | DeviceKind::Server => Behavior::Ip(IpNode::new(false)),
            DeviceKind::Router => Behavior::Ip(IpNode::new(true)),
            DeviceKind::Firewall => Behavior::Ip(IpNode::firewall()),
            DeviceKind::Switch => Behavior::Switch(Switch::new()),
        };
        self.devices.push(Device {
            id,
            name: name.to_string(),
            kind,
            role: None,
            ifaces: Vec::new(),
            behavior,
        });
        self.names.insert(name.to_string(), id);
        Ok(id)
    }

    /// Ajoute une interface (avec une MAC déterministe).
    pub fn add_interface(
        &mut self,
        device: DeviceId,
        name: &str,
        ip: Option<Ipv4Net>,
    ) -> Result<IfaceId, NetivError> {
        let mac = MacAddr::from_index(self.next_mac);
        let dev = self.dev_mut(device)?;
        if name.is_empty() || name.contains(char::is_whitespace) || name.contains(':') {
            return Err(NetivError::InvalidName(name.to_string()));
        }
        if dev.iface_by_name(name).is_some() {
            return Err(NetivError::DuplicateInterface {
                device: dev.name.clone(),
                iface: name.to_string(),
            });
        }
        if ip.is_some() && dev.kind == DeviceKind::Switch {
            return Err(NetivError::NotAnIpDevice(dev.name.clone()));
        }
        let id = IfaceId(dev.ifaces.len() as u32);
        dev.ifaces.push(Interface {
            id,
            name: name.to_string(),
            mac,
            ip,
            admin_up: true,
            management: false,
            link: None,
        });
        self.next_mac += 1;
        Ok(id)
    }

    /// Branche un câble entre deux interfaces.
    pub fn connect(
        &mut self,
        a: (DeviceId, IfaceId),
        b: (DeviceId, IfaceId),
        params: LinkParams,
    ) -> Result<LinkId, NetivError> {
        if !(0.0..=1.0).contains(&params.loss) {
            return Err(NetivError::InvalidLoss(params.loss));
        }
        if a == b {
            let (d, i) = self.describe_end(a)?;
            return Err(NetivError::SelfLink {
                device: d,
                iface: i,
            });
        }
        for end in [a, b] {
            let (dname, iname) = self.describe_end(end)?;
            let busy = self
                .device(end.0)
                .and_then(|d| d.iface(end.1))
                .is_some_and(|i| i.link.is_some());
            if busy {
                return Err(NetivError::InterfaceAlreadyConnected {
                    device: dname,
                    iface: iname,
                });
            }
        }
        let id = LinkId(self.links.len() as u32);
        self.links.push(Link {
            id,
            a,
            b,
            latency: params.latency,
            loss: params.loss,
            up: true,
        });
        for (d, i) in [a, b] {
            if let Some(ifc) = self
                .devices
                .get_mut(d.0 as usize)
                .and_then(|dev| dev.ifaces.get_mut(i.0 as usize))
            {
                ifc.link = Some(id);
            }
        }
        Ok(id)
    }

    fn describe_end(&self, end: (DeviceId, IfaceId)) -> Result<(String, String), NetivError> {
        let dev = self.dev(end.0)?;
        let ifc = dev
            .iface(end.1)
            .ok_or_else(|| NetivError::UnknownInterface {
                device: dev.name.clone(),
                iface: end.1.to_string(),
            })?;
        Ok((dev.name.clone(), ifc.name.clone()))
    }

    // ------------------------------------------------------------------
    // Configuration (tracée)
    // ------------------------------------------------------------------

    fn config_changed(&mut self, device: DeviceId, description: String) {
        self.record(device, None, None, TraceKind::ConfigChanged { description });
    }

    fn ip_node_mut(&mut self, device: DeviceId) -> Result<&mut IpNode, NetivError> {
        let dev = self.dev_mut(device)?;
        match &mut dev.behavior {
            Behavior::Ip(node) => Ok(node),
            Behavior::Switch(_) => Err(NetivError::NotAnIpDevice(dev.name.clone())),
        }
    }

    /// Change l'adresse IP d'une interface.
    pub fn set_ip(
        &mut self,
        device: DeviceId,
        iface: IfaceId,
        ip: Option<Ipv4Net>,
    ) -> Result<(), NetivError> {
        let dev = self.dev_mut(device)?;
        if dev.kind == DeviceKind::Switch {
            return Err(NetivError::NotAnIpDevice(dev.name.clone()));
        }
        let dname = dev.name.clone();
        let ifc =
            dev.ifaces
                .get_mut(iface.0 as usize)
                .ok_or_else(|| NetivError::UnknownInterface {
                    device: dname.clone(),
                    iface: iface.to_string(),
                })?;
        ifc.ip = ip;
        let text = match ip {
            Some(n) => format!("{dname}/{} : adresse IP {n}", ifc.name),
            None => format!("{dname}/{} : adresse IP supprimée", ifc.name),
        };
        self.config_changed(device, text);
        Ok(())
    }

    /// Change la passerelle par défaut.
    pub fn set_gateway(
        &mut self,
        device: DeviceId,
        gateway: Option<Ipv4Addr>,
    ) -> Result<(), NetivError> {
        self.ip_node_mut(device)?.gateway = gateway;
        let name = self.dev(device)?.name.clone();
        let text = match gateway {
            Some(g) => format!("{name} : passerelle par défaut {g}"),
            None => format!("{name} : passerelle par défaut supprimée"),
        };
        self.config_changed(device, text);
        Ok(())
    }

    /// Ajoute une route statique.
    pub fn add_route(&mut self, device: DeviceId, route: StaticRoute) -> Result<(), NetivError> {
        self.ip_node_mut(device)?.static_routes.push(route);
        let name = self.dev(device)?.name.clone();
        self.config_changed(
            device,
            format!("{name} : route {} via {}", route.dest.trunc(), route.via),
        );
        Ok(())
    }

    /// Supprime les routes statiques vers `dest`. Retourne leur nombre.
    pub fn remove_route(&mut self, device: DeviceId, dest: Ipv4Net) -> Result<usize, NetivError> {
        let node = self.ip_node_mut(device)?;
        let before = node.static_routes.len();
        node.static_routes
            .retain(|r| r.dest.trunc() != dest.trunc());
        let removed = before - node.static_routes.len();
        let name = self.dev(device)?.name.clone();
        self.config_changed(
            device,
            format!("{name} : route vers {} supprimée ({removed})", dest.trunc()),
        );
        Ok(removed)
    }

    /// Définit le mode (access / trunk) d'un port de switch.
    pub fn set_port_mode(
        &mut self,
        device: DeviceId,
        iface: IfaceId,
        mode: PortMode,
    ) -> Result<(), NetivError> {
        let dev = self.dev_mut(device)?;
        let dname = dev.name.clone();
        let Some(ifname) = dev.iface(iface).map(|i| i.name.clone()) else {
            return Err(NetivError::UnknownInterface {
                device: dname,
                iface: iface.to_string(),
            });
        };
        let Behavior::Switch(sw) = &mut dev.behavior else {
            return Err(NetivError::NotASwitch(dname));
        };
        let text = match &mode {
            PortMode::Access { vlan } => format!("{dname}/{ifname} : port d'accès, VLAN {vlan}"),
            PortMode::Trunk { allowed, native } => format!(
                "{dname}/{ifname} : port trunk ({}), VLAN natif {native}",
                if allowed.is_empty() {
                    "tous les VLAN".to_string()
                } else {
                    format!(
                        "VLAN {}",
                        allowed
                            .iter()
                            .map(u16::to_string)
                            .collect::<Vec<_>>()
                            .join(",")
                    )
                }
            ),
        };
        sw.ports.insert(iface, mode);
        // Les adresses apprises sur ce port ne sont plus fiables.
        sw.mac_table.retain(|_, p| *p != iface);
        self.config_changed(device, text);
        Ok(())
    }

    /// Ajoute un service à l'écoute.
    pub fn add_service(&mut self, device: DeviceId, service: Service) -> Result<(), NetivError> {
        let text = format!(
            "service {} sur {}/{}",
            service.name, service.proto, service.port
        );
        let node = self.ip_node_mut(device)?;
        node.services
            .retain(|s| !(s.proto == service.proto && s.port == service.port));
        node.services.push(service);
        let name = self.dev(device)?.name.clone();
        self.config_changed(device, format!("{name} : {text} démarré"));
        Ok(())
    }

    /// Arrête les services d'un port. Retourne leur nombre.
    pub fn remove_service(
        &mut self,
        device: DeviceId,
        proto: L4Proto,
        port: u16,
    ) -> Result<usize, NetivError> {
        let node = self.ip_node_mut(device)?;
        let before = node.services.len();
        node.services
            .retain(|s| !(s.proto == proto && s.port == port));
        let removed = before - node.services.len();
        let name = self.dev(device)?.name.clone();
        self.config_changed(device, format!("{name} : service {proto}/{port} arrêté"));
        Ok(removed)
    }

    /// Ajoute une règle de filtrage en fin de liste. Retourne son index (0-based).
    pub fn acl_add_rule(&mut self, device: DeviceId, rule: AclRule) -> Result<usize, NetivError> {
        let node = self.ip_node_mut(device)?;
        node.acl.rules.push(rule.clone());
        let index = node.acl.rules.len() - 1;
        let dev = self.dev(device)?;
        let text = rule.describe(|id| dev.iface(id).map_or("?".to_string(), |i| i.name.clone()));
        let name = dev.name.clone();
        self.config_changed(
            device,
            format!("{name} : règle {} ajoutée — {text}", index + 1),
        );
        Ok(index)
    }

    /// Insère une règle à la position `index` (0-based).
    pub fn acl_insert_rule(
        &mut self,
        device: DeviceId,
        index: usize,
        rule: AclRule,
    ) -> Result<(), NetivError> {
        let node = self.ip_node_mut(device)?;
        let index = index.min(node.acl.rules.len());
        node.acl.rules.insert(index, rule.clone());
        let dev = self.dev(device)?;
        let text = rule.describe(|id| dev.iface(id).map_or("?".to_string(), |i| i.name.clone()));
        let name = dev.name.clone();
        self.config_changed(
            device,
            format!("{name} : règle {} insérée — {text}", index + 1),
        );
        Ok(())
    }

    /// Supprime la règle `index` (0-based) et la retourne.
    pub fn acl_remove_rule(
        &mut self,
        device: DeviceId,
        index: usize,
    ) -> Result<AclRule, NetivError> {
        let node = self.ip_node_mut(device)?;
        if index >= node.acl.rules.len() {
            return Err(NetivError::NoSuchRule(index + 1));
        }
        let rule = node.acl.rules.remove(index);
        node.acl.reset_tracking();
        let name = self.dev(device)?.name.clone();
        self.config_changed(device, format!("{name} : règle {} supprimée", index + 1));
        Ok(rule)
    }

    /// Change la politique par défaut du filtrage.
    pub fn acl_set_default(
        &mut self,
        device: DeviceId,
        action: AclAction,
    ) -> Result<(), NetivError> {
        let node = self.ip_node_mut(device)?;
        node.acl.default = action;
        node.acl.reset_tracking();
        let name = self.dev(device)?.name.clone();
        self.config_changed(
            device,
            format!("{name} : politique de filtrage par défaut = {action}"),
        );
        Ok(())
    }

    /// Active ou désactive le suivi de connexion.
    pub fn acl_set_stateful(&mut self, device: DeviceId, stateful: bool) -> Result<(), NetivError> {
        let node = self.ip_node_mut(device)?;
        node.acl.stateful = stateful;
        node.acl.reset_tracking();
        let name = self.dev(device)?.name.clone();
        self.config_changed(
            device,
            format!(
                "{name} : suivi de connexion {}",
                if stateful { "activé" } else { "désactivé" }
            ),
        );
        Ok(())
    }

    // ------------------------------------------------------------------
    // DHCP et DNS
    // ------------------------------------------------------------------

    fn iface_label(&self, device: DeviceId, iface: IfaceId) -> Result<String, NetivError> {
        let dev = self.dev(device)?;
        let i = dev
            .iface(iface)
            .ok_or_else(|| NetivError::UnknownInterface {
                device: dev.name.clone(),
                iface: format!("#{}", iface.0),
            })?;
        Ok(format!("{}/{}", dev.name, i.name))
    }

    /// Configure une interface par DHCP (`true`) ou à la main (`false`).
    /// Passer en DHCP retire l'adresse actuelle : il faut ensuite la demander
    /// (`dhcp_renew`, ou `restart_dhcp`).
    pub fn set_dhcp_client(
        &mut self,
        device: DeviceId,
        iface: IfaceId,
        on: bool,
    ) -> Result<(), NetivError> {
        let label = self.iface_label(device, iface)?;
        let node = self.ip_node_mut(device)?;
        if on {
            node.dhcp_ifaces.insert(iface);
        } else {
            node.dhcp_ifaces.remove(&iface);
            node.dhcp_clients.remove(&iface);
        }
        if on {
            if let Some(i) = self.dev_mut(device)?.ifaces.get_mut(iface.0 as usize) {
                i.ip = None;
            }
        }
        let text = if on {
            format!("{label} : adresse obtenue par DHCP")
        } else {
            format!("{label} : adresse fixe (DHCP désactivé)")
        };
        self.config_changed(device, text);
        Ok(())
    }

    /// Installe (ou remplace) les plages du serveur DHCP d'un équipement ;
    /// les baux déjà attribués sont conservés. Liste vide : plus de serveur.
    pub fn set_dhcp_pools(
        &mut self,
        device: DeviceId,
        pools: Vec<crate::dhcp::DhcpPool>,
    ) -> Result<(), NetivError> {
        let name = self.dev(device)?.name.clone();
        let node = self.ip_node_mut(device)?;
        let text = if pools.is_empty() {
            node.dhcp_server = None;
            format!("{name} : serveur DHCP supprimé")
        } else {
            let list = pools
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" ; ");
            match &mut node.dhcp_server {
                Some(srv) => srv.pools = pools,
                None => node.dhcp_server = Some(crate::dhcp::DhcpServer::new(pools)),
            }
            format!("{name} : serveur DHCP, plages {list}")
        };
        self.config_changed(device, text);
        Ok(())
    }

    /// Démarre ou arrête le serveur DHCP d'un équipement.
    pub fn set_dhcp_enabled(&mut self, device: DeviceId, on: bool) -> Result<(), NetivError> {
        let name = self.dev(device)?.name.clone();
        let node = self.ip_node_mut(device)?;
        let Some(srv) = node.dhcp_server.as_mut() else {
            return Err(NetivError::InvalidConfig(format!(
                "{name} n'a pas de serveur DHCP"
            )));
        };
        srv.enabled = on;
        let text = format!(
            "{name} : serveur DHCP {}",
            if on { "démarré" } else { "arrêté" }
        );
        self.config_changed(device, text);
        Ok(())
    }

    /// Fait d'une interface de routeur un relais DHCP vers `server` (`None` : plus de relais).
    pub fn set_dhcp_relay(
        &mut self,
        device: DeviceId,
        iface: IfaceId,
        server: Option<Ipv4Addr>,
    ) -> Result<(), NetivError> {
        let label = self.iface_label(device, iface)?;
        let node = self.ip_node_mut(device)?;
        let text = match server {
            Some(s) => {
                node.dhcp_relay.insert(iface, s);
                format!("{label} : relais DHCP vers {s}")
            }
            None => {
                node.dhcp_relay.remove(&iface);
                format!("{label} : relais DHCP supprimé")
            }
        };
        self.config_changed(device, text);
        Ok(())
    }

    /// Serveur DNS utilisé par un équipement (`None` : aucun, ou celui reçu par DHCP).
    pub fn set_dns(
        &mut self,
        device: DeviceId,
        server: Option<Ipv4Addr>,
    ) -> Result<(), NetivError> {
        let name = self.dev(device)?.name.clone();
        self.ip_node_mut(device)?.dns = server;
        let text = match server {
            Some(s) => format!("{name} : serveur DNS {s}"),
            None => format!("{name} : serveur DNS supprimé"),
        };
        self.config_changed(device, text);
        Ok(())
    }

    /// Ajoute (ou remplace) un enregistrement dans la zone du serveur DNS
    /// d'un équipement, créé au besoin. `ip = None` : supprime le nom.
    pub fn set_dns_record(
        &mut self,
        device: DeviceId,
        name: &str,
        ip: Option<Ipv4Addr>,
    ) -> Result<(), NetivError> {
        if !crate::dns::is_valid_name(name) {
            return Err(NetivError::InvalidConfig(format!(
                "nom de domaine invalide : « {name} »"
            )));
        }
        let dname = self.dev(device)?.name.clone();
        let key = crate::dns::normalize(name);
        let node = self.ip_node_mut(device)?;
        let srv = node
            .dns_server
            .get_or_insert_with(|| crate::dns::DnsServer::new(Default::default()));
        let text = match ip {
            Some(a) => {
                srv.zone.insert(key.clone(), a);
                format!("{dname} : DNS {key} = {a}")
            }
            None => {
                srv.zone.remove(&key);
                format!("{dname} : DNS {key} supprimé")
            }
        };
        self.config_changed(device, text);
        Ok(())
    }

    /// Démarre ou arrête le serveur DNS d'un équipement (créé vide au besoin).
    pub fn set_dns_enabled(&mut self, device: DeviceId, on: bool) -> Result<(), NetivError> {
        let name = self.dev(device)?.name.clone();
        let node = self.ip_node_mut(device)?;
        node.dns_server
            .get_or_insert_with(|| crate::dns::DnsServer::new(Default::default()))
            .enabled = on;
        let text = format!(
            "{name} : serveur DNS {}",
            if on { "démarré" } else { "arrêté" }
        );
        self.config_changed(device, text);
        Ok(())
    }

    /// Supprime le serveur DNS d'un équipement.
    pub fn remove_dns_server(&mut self, device: DeviceId) -> Result<(), NetivError> {
        let name = self.dev(device)?.name.clone();
        self.ip_node_mut(device)?.dns_server = None;
        self.config_changed(device, format!("{name} : serveur DNS supprimé"));
        Ok(())
    }

    /// `true` si au moins une interface du réseau est configurée par DHCP.
    pub fn has_dhcp_clients(&self) -> bool {
        self.devices
            .iter()
            .any(|d| matches!(&d.behavior, Behavior::Ip(n) if !n.dhcp_ifaces.is_empty()))
    }

    /// Redémarre tous les clients DHCP (comme au démarrage des machines) :
    /// chacun oublie son adresse et en redemande une. Exécute la simulation
    /// jusqu'à ce que tout soit terminé ; retourne le nombre d'événements.
    pub fn restart_dhcp(&mut self) -> Result<u64, NetivError> {
        let clients: Vec<DeviceId> = self
            .devices
            .iter()
            .filter(|d| matches!(&d.behavior, Behavior::Ip(n) if !n.dhcp_ifaces.is_empty()))
            .map(|d| d.id)
            .collect();
        if clients.is_empty() {
            return Ok(0);
        }
        for d in clients {
            self.command(
                d,
                Command::DhcpRenew {
                    iface: None,
                    fresh: true,
                },
            )?;
        }
        self.run_until_idle()
    }

    /// Active ou désactive le transfert de paquets d'un routeur.
    pub fn set_forwarding(&mut self, device: DeviceId, on: bool) -> Result<(), NetivError> {
        self.ip_node_mut(device)?.forwarding = on;
        let name = self.dev(device)?.name.clone();
        self.config_changed(
            device,
            format!(
                "{name} : transfert de paquets {}",
                if on { "activé" } else { "désactivé" }
            ),
        );
        Ok(())
    }

    /// Modifie la latence d'un lien.
    pub fn set_link_latency(&mut self, link: LinkId, latency: SimTime) -> Result<(), NetivError> {
        let l = self
            .links
            .get_mut(link.0 as usize)
            .ok_or_else(|| NetivError::UnknownDevice(link.to_string()))?;
        l.latency = latency;
        let (a, b) = (l.a, l.b);
        let (da, ia) = self.describe_end(a)?;
        let (db, ib) = self.describe_end(b)?;
        self.config_changed(
            a.0,
            format!("câble {da}/{ia} ↔ {db}/{ib} : latence {latency}"),
        );
        Ok(())
    }

    /// Définit le rôle d'un équipement (voir `device::ROLES`).
    pub fn set_role(&mut self, device: DeviceId, role: Option<&str>) -> Result<(), NetivError> {
        if let Some(r) = role {
            if !crate::device::ROLES.contains(&r) {
                return Err(NetivError::UnknownRole(r.to_string()));
            }
        }
        self.dev_mut(device)?.role = role.map(str::to_string);
        Ok(())
    }

    /// Marque une interface comme interface d'administration.
    pub fn set_management(
        &mut self,
        device: DeviceId,
        iface: IfaceId,
        management: bool,
    ) -> Result<(), NetivError> {
        let dev = self.dev_mut(device)?;
        let dname = dev.name.clone();
        let ifc = dev
            .ifaces
            .get_mut(iface.0 as usize)
            .ok_or(NetivError::UnknownInterface {
                device: dname,
                iface: iface.to_string(),
            })?;
        ifc.management = management;
        Ok(())
    }

    /// Planifie une commande dans `delay` (trafic scripté, scénarios).
    pub fn schedule_command(
        &mut self,
        delay: SimTime,
        device: DeviceId,
        cmd: Command,
    ) -> Result<(), NetivError> {
        let dev = self.dev(device)?;
        if matches!(dev.behavior, Behavior::Switch(_)) {
            return Err(NetivError::NotAnIpDevice(dev.name.clone()));
        }
        self.schedule_in(delay, Event::Command { device, cmd })?;
        Ok(())
    }

    /// Définit (ou supprime avec `None`) une entrée ARP statique.
    pub fn set_static_arp(
        &mut self,
        device: DeviceId,
        ip: Ipv4Addr,
        mac: Option<MacAddr>,
    ) -> Result<(), NetivError> {
        let node = self.ip_node_mut(device)?;
        let text = match mac {
            Some(m) => {
                node.static_arp.insert(ip, m);
                format!("entrée ARP statique {ip} → {m}")
            }
            None => {
                node.static_arp.remove(&ip);
                format!("entrée ARP statique {ip} supprimée")
            }
        };
        let name = self.dev(device)?.name.clone();
        self.config_changed(device, format!("{name} : {text}"));
        Ok(())
    }

    /// Active ou désactive une interface.
    pub fn set_iface_admin(
        &mut self,
        device: DeviceId,
        iface: IfaceId,
        up: bool,
    ) -> Result<(), NetivError> {
        let dev = self.dev_mut(device)?;
        let dname = dev.name.clone();
        let ifc =
            dev.ifaces
                .get_mut(iface.0 as usize)
                .ok_or_else(|| NetivError::UnknownInterface {
                    device: dname.clone(),
                    iface: iface.to_string(),
                })?;
        ifc.admin_up = up;
        let text = format!(
            "{dname}/{} : interface {}",
            ifc.name,
            if up { "activée" } else { "désactivée" }
        );
        self.config_changed(device, text);
        Ok(())
    }

    /// Coupe ou rétablit un câble.
    pub fn set_link_up(&mut self, link: LinkId, up: bool) -> Result<(), NetivError> {
        let l = self
            .links
            .get_mut(link.0 as usize)
            .ok_or_else(|| NetivError::UnknownDevice(link.to_string()))?;
        l.up = up;
        let (a, b) = (l.a, l.b);
        let (da, ia) = self.describe_end(a)?;
        let (db, ib) = self.describe_end(b)?;
        self.config_changed(
            a.0,
            format!(
                "câble {da}/{ia} ↔ {db}/{ib} : {}",
                if up { "rétabli" } else { "coupé" }
            ),
        );
        Ok(())
    }

    /// Modifie le taux de perte d'un lien.
    pub fn set_link_loss(&mut self, link: LinkId, loss: f64) -> Result<(), NetivError> {
        if !(0.0..=1.0).contains(&loss) {
            return Err(NetivError::InvalidLoss(loss));
        }
        let l = self
            .links
            .get_mut(link.0 as usize)
            .ok_or_else(|| NetivError::UnknownDevice(link.to_string()))?;
        l.loss = loss;
        let (a, b) = (l.a, l.b);
        let (da, ia) = self.describe_end(a)?;
        let (db, ib) = self.describe_end(b)?;
        self.config_changed(
            a.0,
            format!("câble {da}/{ia} ↔ {db}/{ib} : perte {:.0} %", loss * 100.0),
        );
        Ok(())
    }

    // ------------------------------------------------------------------
    // Programmation d'événements
    // ------------------------------------------------------------------

    /// Programme un événement à une date absolue. Refuse le passé.
    pub fn schedule_at(&mut self, time: SimTime, event: Event) -> Result<u64, NetivError> {
        if time < self.now {
            return Err(NetivError::ScheduleInPast {
                requested: time,
                now: self.now,
            });
        }
        Ok(self.queue.push(time, event))
    }

    /// Programme un événement `delay` après maintenant.
    pub fn schedule_in(&mut self, delay: SimTime, event: Event) -> Result<u64, NetivError> {
        let time = self
            .now
            .checked_add(delay)
            .ok_or(NetivError::TimeOverflow)?;
        self.schedule_at(time, event)
    }

    /// Envoie une commande à un équipement, exécutée à la date courante.
    pub fn command(&mut self, device: DeviceId, cmd: Command) -> Result<(), NetivError> {
        let dev = self.dev(device)?;
        if matches!(dev.behavior, Behavior::Switch(_)) {
            return Err(NetivError::NotAnIpDevice(dev.name.clone()));
        }
        self.schedule_in(SimTime::ZERO, Event::Command { device, cmd })?;
        Ok(())
    }

    /// Lance `count` pings de `device` vers `dst`.
    pub fn ping(&mut self, device: DeviceId, dst: Ipv4Addr, count: u16) -> Result<(), NetivError> {
        self.command(device, Command::Ping { dst, count })
    }

    // ------------------------------------------------------------------
    // Exécution
    // ------------------------------------------------------------------

    /// Traite le prochain événement : avance l'horloge, l'exécute et décrit
    /// ce qu'il a produit. `None` si la file est vide.
    pub fn step(&mut self) -> Option<Fired> {
        let s = self.queue.pop()?;
        self.now = s.time;
        self.processed += 1;
        let start = self.trace.len();
        self.dispatch(s.event);
        Some(Fired {
            time: s.time,
            seq: s.seq,
            trace: start..self.trace.len(),
        })
    }

    /// Traite tous les événements jusqu'à épuisement de la file.
    /// Retourne le nombre d'événements traités par cet appel.
    pub fn run_until_idle(&mut self) -> Result<u64, NetivError> {
        let mut count = 0;
        while self.step().is_some() {
            count += 1;
            if count >= self.event_limit && !self.queue.is_empty() {
                return Err(NetivError::EventLimitExceeded(self.event_limit));
            }
        }
        Ok(count)
    }

    /// Traite les événements dont la date est `<= until`, puis place
    /// l'horloge à `until`. Retourne le nombre d'événements traités.
    pub fn run_until(&mut self, until: SimTime) -> Result<u64, NetivError> {
        if until < self.now {
            return Err(NetivError::ScheduleInPast {
                requested: until,
                now: self.now,
            });
        }
        let mut count = 0;
        while self.queue.peek_time().is_some_and(|t| t <= until) {
            self.step();
            count += 1;
            if count >= self.event_limit {
                return Err(NetivError::EventLimitExceeded(self.event_limit));
            }
        }
        self.now = until;
        Ok(count)
    }

    fn record(
        &mut self,
        device: DeviceId,
        trace_id: Option<TraceId>,
        caused_by: Option<u64>,
        kind: TraceKind,
    ) -> u64 {
        self.trace.push(self.now, device, trace_id, caused_by, kind)
    }

    fn dispatch(&mut self, event: Event) {
        match event {
            Event::FrameArrival {
                device,
                iface,
                frame,
                cause,
            } => {
                let tid = Some(frame.trace_id);
                let up = self
                    .device(device)
                    .and_then(|d| d.iface(iface))
                    .is_some_and(|i| i.admin_up);
                if !up {
                    self.record(
                        device,
                        tid,
                        cause,
                        TraceKind::Dropped {
                            reason: DropReason::InterfaceDown,
                        },
                    );
                    return;
                }
                let rx = self.record(
                    device,
                    tid,
                    cause,
                    TraceKind::FrameReceived {
                        iface,
                        frame: frame.clone(),
                    },
                );
                self.run_node(device, Some(rx), |behavior, ctx, out| match behavior {
                    Behavior::Ip(node) => node.on_frame(ctx, iface, frame, out),
                    Behavior::Switch(sw) => sw.on_frame(ctx, iface, frame, out),
                });
            }
            Event::Timer { device, timer } => {
                self.run_node(device, None, |behavior, ctx, out| {
                    if let Behavior::Ip(node) = behavior {
                        node.on_timer(ctx, timer, out);
                    }
                });
            }
            Event::Fault { fault, visible } => {
                // Validée à la programmation : une erreur ici ne peut venir que
                // d'un équipement supprimé depuis, ce qui n'existe pas.
                let _ = self.inject_fault(fault, visible);
            }
            Event::Command { device, cmd } => {
                let issued = self.record(
                    device,
                    None,
                    None,
                    TraceKind::CommandIssued {
                        description: cmd.to_string(),
                    },
                );
                self.run_node(device, Some(issued), |behavior, ctx, out| {
                    if let Behavior::Ip(node) = behavior {
                        node.on_command(ctx, cmd, out);
                    }
                });
            }
        }
    }

    /// Donne la main à un équipement puis applique ses effets.
    fn run_node<F>(&mut self, device: DeviceId, cause: Option<u64>, f: F)
    where
        F: FnOnce(&mut Behavior, &Ctx, &mut Effects),
    {
        let Some(dev) = self.devices.get_mut(device.0 as usize) else {
            return;
        };
        let mut out = Effects::new(self.next_trace);
        let ctx = Ctx {
            now: self.now,
            device,
            ifaces: &dev.ifaces,
        };
        f(&mut dev.behavior, &ctx, &mut out);
        self.next_trace = out.next_trace();
        self.apply(device, out.into_vec(), cause);
    }

    fn apply(&mut self, device: DeviceId, effects: Vec<Effect>, cause: Option<u64>) {
        for effect in effects {
            match effect {
                Effect::Trace { trace_id, kind } => {
                    self.record(device, trace_id, cause, kind);
                }
                Effect::SetTimer { after, timer } => {
                    // Seul un dépassement de l'horloge (u64) peut échouer ici ;
                    // le timer est alors simplement perdu.
                    let _ = self.schedule_in(after, Event::Timer { device, timer });
                }
                Effect::Send { iface, frame } => self.transmit(device, iface, frame, cause),
                Effect::ConfigureIface { iface, ip } => {
                    if let Some(i) = self
                        .devices
                        .get_mut(device.0 as usize)
                        .and_then(|d| d.ifaces.get_mut(iface.0 as usize))
                    {
                        i.ip = ip;
                    }
                }
            }
        }
    }

    /// Fait sortir une trame par une interface : vérifie l'état de
    /// l'interface et du câble, applique la perte, programme l'arrivée.
    fn transmit(&mut self, device: DeviceId, iface: IfaceId, frame: Frame, cause: Option<u64>) {
        let tid = Some(frame.trace_id);
        let Some((admin_up, link_id)) = self
            .device(device)
            .and_then(|d| d.iface(iface))
            .map(|i| (i.admin_up, i.link))
        else {
            return;
        };
        if !admin_up {
            self.record(
                device,
                tid,
                cause,
                TraceKind::Dropped {
                    reason: DropReason::InterfaceDown,
                },
            );
            return;
        }
        let Some(link) = link_id.and_then(|l| self.link(l)).copied() else {
            self.record(
                device,
                tid,
                cause,
                TraceKind::Dropped {
                    reason: DropReason::NoLink,
                },
            );
            return;
        };
        let sent = self.record(
            device,
            tid,
            cause,
            TraceKind::FrameSent {
                iface,
                frame: frame.clone(),
            },
        );
        if !link.up {
            self.record(
                device,
                tid,
                Some(sent),
                TraceKind::Dropped {
                    reason: DropReason::LinkDown,
                },
            );
            return;
        }
        if link.loss > 0.0 && self.rng.chance(link.loss) {
            self.record(
                device,
                tid,
                Some(sent),
                TraceKind::Dropped {
                    reason: DropReason::LinkLoss,
                },
            );
            return;
        }
        let Some((peer_dev, peer_iface)) = link.peer_of(device, iface) else {
            return;
        };
        let _ = self.schedule_in(
            link.latency,
            Event::FrameArrival {
                device: peer_dev,
                iface: peer_iface,
                frame,
                cause: Some(sent),
            },
        );
    }
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Command::Ping { dst, count } => write!(f, "ping {dst} ({count} requête(s))"),
            Command::Curl { dst, port } => write!(f, "curl {dst}:{port}"),
            Command::SendUdp { dst, port, data } => write!(f, "udp {dst}:{port} « {data} »"),
            Command::DhcpRenew { fresh: true, .. } => f.write_str("DHCP : démarrage"),
            Command::DhcpRenew { .. } => f.write_str("DHCP : renouvellement"),
            Command::DhcpRelease { .. } => f.write_str("DHCP : libération de l'adresse"),
            Command::Resolve { name, then } => match then {
                crate::dns::AfterResolve::Nothing => write!(f, "nslookup {name}"),
                crate::dns::AfterResolve::Ping { count } => {
                    write!(f, "ping {name} ({count} requête(s))")
                }
                crate::dns::AfterResolve::Curl { port } => write!(f, "curl {name}:{port}"),
                crate::dns::AfterResolve::Udp { port, data } => {
                    write!(f, "udp {name}:{port} « {data} »")
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sim_with_host() -> (Simulation, DeviceId) {
        let mut sim = Simulation::new(1);
        let d = sim.add_device("A", DeviceKind::Host).unwrap();
        (sim, d)
    }

    fn timer(d: DeviceId, n: u64) -> Event {
        Event::Timer {
            device: d,
            timer: TimerId(n),
        }
    }

    #[test]
    fn starts_at_zero_and_empty() {
        let sim = Simulation::new(1);
        assert_eq!(sim.now(), SimTime::ZERO);
        assert_eq!(sim.pending(), 0);
        assert!(sim.trace().is_empty());
    }

    #[test]
    fn step_on_empty_returns_none() {
        assert_eq!(Simulation::new(1).step(), None);
    }

    #[test]
    fn step_advances_clock() {
        let (mut sim, d) = sim_with_host();
        sim.schedule_at(SimTime(500), timer(d, 1)).unwrap();
        let fired = sim.step().unwrap();
        assert_eq!(fired.time, SimTime(500));
        assert_eq!(sim.now(), SimTime(500));
        assert_eq!(sim.processed(), 1);
    }

    #[test]
    fn simultaneous_events_run_in_insertion_order() {
        let (mut sim, d) = sim_with_host();
        for n in [10, 20, 30] {
            sim.schedule_at(SimTime(100), timer(d, n)).unwrap();
        }
        sim.schedule_at(SimTime(50), timer(d, 99)).unwrap();
        let order: Vec<u64> = std::iter::from_fn(|| sim.step()).map(|f| f.seq).collect();
        assert_eq!(order, [3, 0, 1, 2]);
    }

    #[test]
    fn schedule_in_is_relative_to_now() {
        let (mut sim, d) = sim_with_host();
        sim.schedule_at(SimTime(1_000), timer(d, 1)).unwrap();
        sim.step();
        sim.schedule_in(SimTime(250), timer(d, 2)).unwrap();
        assert_eq!(sim.step().unwrap().time, SimTime(1_250));
    }

    #[test]
    fn scheduling_in_the_past_is_refused() {
        let (mut sim, d) = sim_with_host();
        sim.schedule_at(SimTime(1_000), timer(d, 1)).unwrap();
        sim.step();
        let err = sim.schedule_at(SimTime(999), timer(d, 2)).unwrap_err();
        assert_eq!(
            err,
            NetivError::ScheduleInPast {
                requested: SimTime(999),
                now: SimTime(1_000)
            }
        );
        assert!(sim.schedule_at(SimTime(1_000), timer(d, 3)).is_ok());
    }

    #[test]
    fn schedule_in_detects_overflow() {
        let (mut sim, d) = sim_with_host();
        sim.schedule_at(SimTime(u64::MAX), timer(d, 1)).unwrap();
        sim.step();
        assert_eq!(
            sim.schedule_in(SimTime(1), timer(d, 2)),
            Err(NetivError::TimeOverflow)
        );
    }

    #[test]
    fn run_until_idle_drains_queue() {
        let (mut sim, d) = sim_with_host();
        for t in [30, 10, 20] {
            sim.schedule_at(SimTime(t), timer(d, t)).unwrap();
        }
        assert_eq!(sim.run_until_idle(), Ok(3));
        assert_eq!(sim.pending(), 0);
        assert_eq!(sim.now(), SimTime(30));
    }

    #[test]
    fn run_until_idle_stops_runaway_simulations() {
        let (mut sim, d) = sim_with_host();
        sim.set_event_limit(100);
        for t in 0..200 {
            sim.schedule_at(SimTime(t), timer(d, t)).unwrap();
        }
        assert_eq!(
            sim.run_until_idle(),
            Err(NetivError::EventLimitExceeded(100))
        );
    }

    #[test]
    fn run_until_processes_only_due_events_and_sets_clock() {
        let (mut sim, d) = sim_with_host();
        for t in [10, 20, 30] {
            sim.schedule_at(SimTime(t), timer(d, t)).unwrap();
        }
        assert_eq!(sim.run_until(SimTime(25)), Ok(2));
        assert_eq!(sim.now(), SimTime(25));
        assert_eq!(sim.pending(), 1);
        assert!(sim.run_until(SimTime(5)).is_err());
    }

    #[test]
    fn same_seed_gives_same_random_draws() {
        let mut a = Simulation::new(1234);
        let mut b = Simulation::new(1234);
        let da: Vec<bool> = (0..50).map(|_| a.rng_mut().chance(0.5)).collect();
        let db: Vec<bool> = (0..50).map(|_| b.rng_mut().chance(0.5)).collect();
        assert_eq!(da, db);
    }

    #[test]
    fn construction_rules() {
        let mut sim = Simulation::new(1);
        let a = sim.add_device("PC1", DeviceKind::Host).unwrap();
        let sw = sim.add_device("SW1", DeviceKind::Switch).unwrap();
        assert_eq!(
            sim.add_device("PC1", DeviceKind::Host),
            Err(NetivError::DuplicateDevice("PC1".into()))
        );
        assert!(matches!(
            sim.add_device("bad name", DeviceKind::Host),
            Err(NetivError::InvalidName(_))
        ));
        let eth0 = sim.add_interface(a, "eth0", None).unwrap();
        assert!(matches!(
            sim.add_interface(a, "eth0", None),
            Err(NetivError::DuplicateInterface { .. })
        ));
        let ip: Ipv4Net = "10.0.0.1/24".parse().unwrap();
        assert!(matches!(
            sim.add_interface(sw, "port1", Some(ip)),
            Err(NetivError::NotAnIpDevice(_))
        ));
        let port1 = sim.add_interface(sw, "port1", None).unwrap();
        sim.connect((a, eth0), (sw, port1), LinkParams::default())
            .unwrap();
        // Une interface ne peut avoir qu'un câble.
        assert!(matches!(
            sim.connect((a, eth0), (sw, port1), LinkParams::default()),
            Err(NetivError::InterfaceAlreadyConnected { .. })
        ));
        assert_eq!(sim.device_id("SW1"), Ok(sw));
        assert!(sim.device_id("nope").is_err());
        assert_eq!(sim.iface_id(a, "eth0"), Ok(eth0));
    }

    #[test]
    fn macs_are_unique_and_deterministic() {
        let build = || {
            let mut sim = Simulation::new(1);
            let a = sim.add_device("A", DeviceKind::Host).unwrap();
            let b = sim.add_device("B", DeviceKind::Host).unwrap();
            sim.add_interface(a, "eth0", None).unwrap();
            sim.add_interface(b, "eth0", None).unwrap();
            (
                sim.devices()[0].ifaces[0].mac,
                sim.devices()[1].ifaces[0].mac,
            )
        };
        let (m1, m2) = build();
        assert_ne!(m1, m2);
        assert_eq!((m1, m2), build());
    }
}

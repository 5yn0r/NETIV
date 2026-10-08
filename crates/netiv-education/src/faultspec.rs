//! Description des pannes **par noms** (`R1:eth1`, `PC1`...), lisible dans un
//! fichier de challenge. Se convertit dans les deux sens avec [`Fault`].

use std::net::Ipv4Addr;

use netiv_core::acl::AclAction;
use netiv_core::packet::L4Proto;
use netiv_core::route::StaticRoute;
use netiv_core::switch::PortMode;
use netiv_core::{DeviceId, Fault, IfaceId, Ipv4Net, LinkId, MacAddr, SimTime, Simulation};
use netiv_topology::{RuleDefAcl, rule_from_def, rule_to_def};
use serde::{Deserialize, Serialize};

/// Une panne décrite par noms.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum FaultSpec {
    /// Câble coupé (l'une de ses extrémités : `R1:eth1`).
    LinkDown {
        /// Extrémité.
        a: String,
    },
    /// Interface désactivée.
    InterfaceDown {
        /// `Équipement:interface`.
        iface: String,
    },
    /// Perte de paquets sur un câble.
    LinkLoss {
        /// Extrémité du câble.
        a: String,
        /// Probabilité de perte (0 à 1).
        loss: f64,
    },
    /// Latence anormale.
    LinkLatency {
        /// Extrémité du câble.
        a: String,
        /// Latence en millisecondes.
        latency_ms: u64,
    },
    /// Mauvaise adresse ou mauvais masque.
    SetIp {
        /// `Équipement:interface`.
        iface: String,
        /// Nouvelle adresse (CIDR) ; absent = aucune.
        ip: Option<String>,
    },
    /// Mauvaise passerelle.
    SetGateway {
        /// Équipement.
        device: String,
        /// Nouvelle passerelle ; absent = aucune.
        gateway: Option<String>,
    },
    /// Routes statiques supprimées.
    RemoveRoutes {
        /// Équipement.
        device: String,
        /// Réseau (CIDR).
        dest: String,
    },
    /// Route statique ajoutée.
    AddRoute {
        /// Équipement.
        device: String,
        /// Réseau (CIDR).
        dest: String,
        /// Prochain saut.
        via: String,
    },
    /// Un routeur ne transfère plus.
    ForwardingOff {
        /// Équipement.
        device: String,
    },
    /// Entrée ARP statique erronée.
    StaticArp {
        /// Équipement.
        device: String,
        /// IP.
        ip: String,
        /// MAC.
        mac: String,
    },
    /// Port d'accès dans un autre VLAN.
    PortAccess {
        /// `Switch:port`.
        iface: String,
        /// VLAN.
        vlan: u16,
    },
    /// Port trunk reconfiguré.
    PortTrunk {
        /// `Switch:port`.
        iface: String,
        /// VLAN autorisés (vide = tous).
        allowed: Vec<u16>,
        /// VLAN natif.
        native: u16,
    },
    /// Service arrêté.
    StopService {
        /// Équipement.
        device: String,
        /// `tcp` ou `udp`.
        proto: String,
        /// Port.
        port: u16,
    },
    /// Règle de filtrage insérée en tête.
    AclInsert {
        /// Équipement.
        device: String,
        /// Règle.
        rule: RuleDefAcl,
    },
    /// Politique de filtrage par défaut.
    AclDefault {
        /// Équipement.
        device: String,
        /// `allow` ou `deny`.
        action: String,
    },
    /// Plusieurs pannes à la fois.
    Multi(Vec<FaultSpec>),
}

fn device(sim: &Simulation, name: &str) -> Result<DeviceId, String> {
    sim.device_id(name).map_err(|e| e.to_string())
}

fn endpoint(sim: &Simulation, text: &str) -> Result<(DeviceId, IfaceId), String> {
    let (d, i) = text
        .split_once(':')
        .ok_or_else(|| format!("« {text} » : attendu Équipement:interface"))?;
    let dev = device(sim, d)?;
    let iface = sim.iface_id(dev, i).map_err(|e| e.to_string())?;
    Ok((dev, iface))
}

fn link_at(sim: &Simulation, text: &str) -> Result<LinkId, String> {
    let (d, i) = endpoint(sim, text)?;
    sim.device(d)
        .and_then(|x| x.iface(i))
        .and_then(|x| x.link)
        .ok_or_else(|| format!("{text} n'est reliée à aucun câble"))
}

fn ip(text: &str) -> Result<Ipv4Addr, String> {
    text.parse()
        .map_err(|_| format!("adresse IPv4 invalide : « {text} »"))
}

fn cidr(text: &str) -> Result<Ipv4Net, String> {
    text.parse::<Ipv4Net>().map_err(|e| e.to_string())
}

fn proto(text: &str) -> Result<L4Proto, String> {
    match text {
        "tcp" => Ok(L4Proto::Tcp),
        "udp" => Ok(L4Proto::Udp),
        other => Err(format!("protocole inconnu : « {other} »")),
    }
}

fn action(text: &str) -> Result<AclAction, String> {
    match text {
        "allow" => Ok(AclAction::Allow),
        "deny" => Ok(AclAction::Deny),
        other => Err(format!("action inconnue : « {other} »")),
    }
}

impl FaultSpec {
    /// Résout les noms en identifiants du moteur.
    pub fn resolve(&self, sim: &Simulation) -> Result<Fault, String> {
        Ok(match self {
            FaultSpec::LinkDown { a } => Fault::LinkDown {
                link: link_at(sim, a)?,
            },
            FaultSpec::InterfaceDown { iface } => {
                let (device, iface) = endpoint(sim, iface)?;
                Fault::InterfaceDown { device, iface }
            }
            FaultSpec::LinkLoss { a, loss } => Fault::LinkLoss {
                link: link_at(sim, a)?,
                loss: *loss,
            },
            FaultSpec::LinkLatency { a, latency_ms } => Fault::LinkLatency {
                link: link_at(sim, a)?,
                latency: SimTime::from_millis(*latency_ms),
            },
            FaultSpec::SetIp { iface, ip: new } => {
                let (device, iface) = endpoint(sim, iface)?;
                Fault::SetIp {
                    device,
                    iface,
                    ip: new.as_deref().map(cidr).transpose()?,
                }
            }
            FaultSpec::SetGateway { device: d, gateway } => Fault::SetGateway {
                device: device(sim, d)?,
                gateway: gateway.as_deref().map(self::ip).transpose()?,
            },
            FaultSpec::RemoveRoutes { device: d, dest } => Fault::RemoveRoutes {
                device: device(sim, d)?,
                dest: cidr(dest)?,
            },
            FaultSpec::AddRoute {
                device: d,
                dest,
                via,
            } => Fault::AddRoute {
                device: device(sim, d)?,
                route: StaticRoute {
                    dest: cidr(dest)?,
                    via: ip(via)?,
                },
            },
            FaultSpec::ForwardingOff { device: d } => Fault::ForwardingOff {
                device: device(sim, d)?,
            },
            FaultSpec::StaticArp {
                device: d,
                ip: a,
                mac,
            } => Fault::StaticArp {
                device: device(sim, d)?,
                ip: ip(a)?,
                mac: mac.parse::<MacAddr>().map_err(|e| e.to_string())?,
            },
            FaultSpec::PortAccess { iface, vlan } => {
                let (device, iface) = endpoint(sim, iface)?;
                Fault::PortMode {
                    device,
                    iface,
                    mode: PortMode::Access { vlan: *vlan },
                }
            }
            FaultSpec::PortTrunk {
                iface,
                allowed,
                native,
            } => {
                let (device, iface) = endpoint(sim, iface)?;
                Fault::PortMode {
                    device,
                    iface,
                    mode: PortMode::Trunk {
                        allowed: allowed.clone(),
                        native: *native,
                    },
                }
            }
            FaultSpec::StopService {
                device: d,
                proto: p,
                port,
            } => Fault::StopService {
                device: device(sim, d)?,
                proto: proto(p)?,
                port: *port,
            },
            FaultSpec::AclInsert { device: d, rule } => {
                let dev = device(sim, d)?;
                let rule = rule_from_def(sim, dev, rule).map_err(|p| p.join(" ; "))?;
                Fault::AclInsert { device: dev, rule }
            }
            FaultSpec::AclDefault {
                device: d,
                action: a,
            } => Fault::AclDefault {
                device: device(sim, d)?,
                action: action(a)?,
            },
            FaultSpec::Multi(list) => Fault::Multi(
                list.iter()
                    .map(|f| f.resolve(sim))
                    .collect::<Result<_, _>>()?,
            ),
        })
    }

    /// Décrit une panne du moteur par noms.
    pub fn from_fault(sim: &Simulation, fault: &Fault) -> FaultSpec {
        let dn = |d: &DeviceId| {
            sim.device(*d)
                .map_or_else(|| "?".to_string(), |x| x.name.clone())
        };
        let ep = |d: &DeviceId, i: &IfaceId| {
            format!(
                "{}:{}",
                dn(d),
                sim.device(*d)
                    .and_then(|x| x.iface(*i))
                    .map_or("?", |x| x.name.as_str())
            )
        };
        let link_end = |l: &LinkId| {
            sim.link(*l)
                .map_or_else(|| "?".to_string(), |k| ep(&k.a.0, &k.a.1))
        };
        match fault {
            Fault::LinkDown { link } => FaultSpec::LinkDown { a: link_end(link) },
            Fault::InterfaceDown { device, iface } => FaultSpec::InterfaceDown {
                iface: ep(device, iface),
            },
            Fault::LinkLoss { link, loss } => FaultSpec::LinkLoss {
                a: link_end(link),
                loss: *loss,
            },
            Fault::LinkLatency { link, latency } => FaultSpec::LinkLatency {
                a: link_end(link),
                latency_ms: latency.as_micros() / 1_000,
            },
            Fault::SetIp { device, iface, ip } => FaultSpec::SetIp {
                iface: ep(device, iface),
                ip: ip.map(|n| n.to_string()),
            },
            Fault::SetGateway { device, gateway } => FaultSpec::SetGateway {
                device: dn(device),
                gateway: gateway.map(|g| g.to_string()),
            },
            Fault::RemoveRoutes { device, dest } => FaultSpec::RemoveRoutes {
                device: dn(device),
                dest: dest.trunc().to_string(),
            },
            Fault::AddRoute { device, route } => FaultSpec::AddRoute {
                device: dn(device),
                dest: route.dest.trunc().to_string(),
                via: route.via.to_string(),
            },
            Fault::ForwardingOff { device } => FaultSpec::ForwardingOff { device: dn(device) },
            Fault::StaticArp { device, ip, mac } => FaultSpec::StaticArp {
                device: dn(device),
                ip: ip.to_string(),
                mac: mac.to_string(),
            },
            Fault::PortMode {
                device,
                iface,
                mode,
            } => match mode {
                PortMode::Access { vlan } => FaultSpec::PortAccess {
                    iface: ep(device, iface),
                    vlan: *vlan,
                },
                PortMode::Trunk { allowed, native } => FaultSpec::PortTrunk {
                    iface: ep(device, iface),
                    allowed: allowed.clone(),
                    native: *native,
                },
            },
            Fault::StopService {
                device,
                proto,
                port,
            } => FaultSpec::StopService {
                device: dn(device),
                proto: proto.to_string(),
                port: *port,
            },
            Fault::AclInsert { device, rule } => FaultSpec::AclInsert {
                device: dn(device),
                rule: rule_to_def(sim, *device, rule),
            },
            Fault::AclDefault { device, action } => FaultSpec::AclDefault {
                device: dn(device),
                action: action.to_string(),
            },
            Fault::Multi(list) => {
                FaultSpec::Multi(list.iter().map(|f| FaultSpec::from_fault(sim, f)).collect())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use netiv_core::acl::{AclRule, RuleProto};

    /// Les enums s'écrivent `nom: { champs }` dans les fichiers de challenge.
    #[derive(Serialize, Deserialize)]
    struct Wrap {
        #[serde(with = "serde_yaml_ng::with::singleton_map_recursive")]
        fault: FaultSpec,
    }

    fn lab() -> Simulation {
        netiv_topology::load_yaml(include_str!("../../../examples/mvp.yaml"))
            .unwrap()
            .sim
    }

    #[test]
    fn every_fault_kind_roundtrips_through_names() {
        let sim = lab();
        let dev = |n: &str| sim.device_id(n).unwrap();
        let mut deny = AclRule::new(AclAction::Deny, RuleProto::Tcp);
        deny.port_from = Some(443);
        deny.port_to = Some(443);
        deny.in_iface = Some(sim.iface_id(dev("R1"), "eth0").unwrap());
        let faults = vec![
            Fault::LinkDown { link: LinkId(2) },
            Fault::InterfaceDown {
                device: dev("R1"),
                iface: sim.iface_id(dev("R1"), "eth1").unwrap(),
            },
            Fault::LinkLoss {
                link: LinkId(1),
                loss: 0.25,
            },
            Fault::LinkLatency {
                link: LinkId(0),
                latency: SimTime::from_millis(300),
            },
            Fault::SetIp {
                device: dev("PC1"),
                iface: sim.iface_id(dev("PC1"), "eth0").unwrap(),
                ip: Some("10.9.9.9/24".parse().unwrap()),
            },
            Fault::SetGateway {
                device: dev("PC1"),
                gateway: Some("192.168.1.254".parse().unwrap()),
            },
            Fault::SetGateway {
                device: dev("PC1"),
                gateway: None,
            },
            Fault::RemoveRoutes {
                device: dev("R1"),
                dest: "192.168.2.0/24".parse().unwrap(),
            },
            Fault::AddRoute {
                device: dev("R2"),
                route: StaticRoute {
                    dest: "192.168.2.10/32".parse().unwrap(),
                    via: "10.0.0.1".parse().unwrap(),
                },
            },
            Fault::ForwardingOff { device: dev("R1") },
            Fault::StaticArp {
                device: dev("PC1"),
                ip: "192.168.1.1".parse().unwrap(),
                mac: "02:00:00:00:be:ef".parse().unwrap(),
            },
            Fault::PortMode {
                device: dev("SW1"),
                iface: sim.iface_id(dev("SW1"), "port1").unwrap(),
                mode: PortMode::Access { vlan: 99 },
            },
            Fault::PortMode {
                device: dev("SW1"),
                iface: sim.iface_id(dev("SW1"), "port2").unwrap(),
                mode: PortMode::Trunk {
                    allowed: vec![10, 20],
                    native: 10,
                },
            },
            Fault::StopService {
                device: dev("SERVER"),
                proto: L4Proto::Tcp,
                port: 80,
            },
            Fault::AclInsert {
                device: dev("R1"),
                rule: deny,
            },
            Fault::AclDefault {
                device: dev("R1"),
                action: AclAction::Deny,
            },
            Fault::Multi(vec![
                Fault::LinkDown { link: LinkId(0) },
                Fault::ForwardingOff { device: dev("R2") },
            ]),
        ];
        for f in faults {
            let spec = FaultSpec::from_fault(&sim, &f);
            // Via YAML, comme dans un fichier de challenge.
            let yaml = serde_yaml_ng::to_string(&Wrap {
                fault: spec.clone(),
            })
            .unwrap();
            let back: Wrap = serde_yaml_ng::from_str(&yaml).unwrap();
            assert_eq!(back.fault, spec, "{yaml}");
            assert_eq!(back.fault.resolve(&sim).unwrap(), f, "{yaml}");
        }
    }

    #[test]
    fn readable_yaml_form() {
        let spec: Wrap = serde_yaml_ng::from_str(
            "fault:\n  set_gateway: { device: PC1, gateway: 192.168.1.254 }",
        )
        .unwrap();
        assert_eq!(
            spec.fault,
            FaultSpec::SetGateway {
                device: "PC1".into(),
                gateway: Some("192.168.1.254".into())
            }
        );
        let multi: Wrap = serde_yaml_ng::from_str(
            "fault:\n  multi:\n    - link_down: { a: \"R1:eth1\" }\n    - forwarding_off: { device: R2 }",
        )
        .unwrap();
        assert!(matches!(multi.fault, FaultSpec::Multi(ref l) if l.len() == 2));
    }

    #[test]
    fn unknown_names_are_reported_not_panicked() {
        let sim = lab();
        for bad in [
            FaultSpec::LinkDown {
                a: "NOPE:eth0".into(),
            },
            FaultSpec::LinkDown { a: "R1".into() },
            FaultSpec::LinkDown {
                a: "R1:eth9".into(),
            },
            FaultSpec::SetGateway {
                device: "PC1".into(),
                gateway: Some("pas-une-ip".into()),
            },
            FaultSpec::StopService {
                device: "SERVER".into(),
                proto: "sctp".into(),
                port: 80,
            },
            FaultSpec::AclDefault {
                device: "R1".into(),
                action: "maybe".into(),
            },
            FaultSpec::StaticArp {
                device: "PC1".into(),
                ip: "1.1.1.1".into(),
                mac: "zz".into(),
            },
            FaultSpec::RemoveRoutes {
                device: "R1".into(),
                dest: "nope".into(),
            },
        ] {
            assert!(bad.resolve(&sim).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn typos_in_field_names_are_rejected() {
        assert!(
            serde_yaml_ng::from_str::<Wrap>(
                "fault:\n  set_gateway: { device: PC1, gatewey: 1.1.1.1 }"
            )
            .is_err()
        );
        assert!(serde_yaml_ng::from_str::<Wrap>("fault:\n  explode: {}").is_err());
    }
}

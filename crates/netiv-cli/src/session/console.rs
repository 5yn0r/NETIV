//! Console d'un équipement : CLI Cisco IOS (routeurs, switches, pare-feu) ou
//! terminal de poste (Windows et Linux). Un plus par rapport à la syntaxe
//! NETIV, qui reste la syntaxe par défaut.
//!
//! Chaque ligne est traduite par `dialect` en commandes NETIV, exécutées par
//! les mêmes fonctions que si l'élève les avait tapées : mêmes contrôles,
//! même verrouillage pendant un défi, même trace.

use std::fmt::Write as _;
use std::io::Write;

use netiv_core::device::Behavior;
use netiv_core::route::RouteOrigin;
use netiv_core::switch::PortMode;
use netiv_core::{DeviceId, DeviceKind, Interface, MacAddr, Simulation, render};

use super::dialect::{self, Action, ConsoleKind, DeviceCtx, HostShow, IfaceCtx, IosMode, IosShow};
use super::{CliError, Flow, Session, err};

/// Console ouverte sur un équipement.
#[derive(Clone, Debug)]
pub struct Console {
    /// Nom de l'équipement.
    pub device: String,
    /// Cisco ou terminal.
    pub kind: ConsoleKind,
    /// Mode Cisco (ignoré pour un terminal).
    pub mode: IosMode,
}

/// Commandes NETIV qui restent disponibles pendant une console (pilotage du
/// défi, observation, aide) : les boutons de l'interface continuent de marcher.
pub(super) const PILOT: &[&str] = &[
    "challenge",
    "hint",
    "answer",
    "trace",
    "inspect",
    "explain",
    "log",
    "step",
    "run",
    "time",
    "devices",
    "tutor",
    "console",
    "range",
    "dashboard",
    "badges",
    "learn",
    "docs",
    "netiv",
];

/// État de la console pour l'interface (snapshot).
#[derive(Clone, Debug)]
pub struct ConsoleInfo {
    /// Équipement.
    pub device: String,
    /// `ios` ou `host`.
    pub kind: &'static str,
    /// Invite (`R1(config-if)#`, `PC1:~$`).
    pub prompt: String,
}

const HELP_IOS: &str = "\
Cisco IOS. Abréviations : conf t, int gi0/1, sh ip int br, no shut.
  enable | disable | configure terminal | end | exit
  show ip interface brief | running-config | ip route | vlan brief | interfaces status
       mac address-table | arp | access-lists
  ping <ip|nom> | traceroute <ip|nom>
  (config)     interface <Gi0/n|nom> | ip route <réseau> <masque> <passerelle> | no ip route …
               vlan <n> | ip routing | do <commande>
  (config-if)  ip address <ip> <masque> | ip address dhcp | no ip address | [no] shutdown
               ip helper-address <ip> | switchport mode access|trunk
               switchport access vlan <n> | switchport trunk allowed vlan <liste> | switchport trunk native vlan <n>
Aussi : hint, challenge check, answer, trace, log.  « exit » : retour.
";

const HELP_HOST: &str = "\
Terminal (Windows et Linux).
  Windows : ipconfig [/all|/release|/renew] | ping <cible> [-n N] | tracert | nslookup <nom> | arp -a
            route print | route add <réseau> mask <masque> <passerelle> | route delete <réseau>
            netsh interface ip set address \"<carte>\" static <ip> <masque> [passerelle] | dhcp
            netsh interface ip set dns \"<carte>\" static <serveur> | dhcp | curl <url>
  Linux   : ip a | ip link set <if> up|down | ip addr add <ip/préfixe> dev <if> | ip addr flush dev <if>
            ip route | ip route add default via <passerelle> | ip neigh | ping -c N | traceroute
            nslookup | dig | curl | dhclient [-r] [if] | resolvectl dns <if> <serveur> | cat /etc/resolv.conf
Aussi : hint, challenge check, answer, trace, log.  « exit » : retour.
";

impl Session {
    /// État de la console ouverte, s'il y en a une.
    pub fn console_info(&self) -> Option<ConsoleInfo> {
        self.console.as_ref().map(|c| ConsoleInfo {
            device: c.device.clone(),
            kind: c.kind.slug(),
            prompt: prompt(c),
        })
    }

    /// `console [<équipement> | exit]`
    pub(super) fn cmd_console(
        &mut self,
        args: &[&str],
        out: &mut dyn Write,
    ) -> Result<(), CliError> {
        match args {
            [] => match &self.console {
                Some(c) => writeln!(
                    out,
                    "Console ouverte sur {} ({}).",
                    c.device,
                    kind_label(c.kind)
                )?,
                None => writeln!(out, "Aucune console ouverte (« console <équipement> »).")?,
            },
            ["exit" | "off" | "close"] => {
                if let Some(c) = self.console.take() {
                    writeln!(out, "Console de {} fermée.", c.device)?;
                }
            }
            [dev] => {
                let id = self.device(dev)?;
                let d = self
                    .sim()?
                    .device(id)
                    .ok_or_else(|| CliError(format!("équipement inconnu : {dev}")))?;
                let kind = match d.kind {
                    DeviceKind::Router | DeviceKind::Switch | DeviceKind::Firewall => {
                        ConsoleKind::Ios
                    }
                    DeviceKind::Host | DeviceKind::Server => ConsoleKind::Host,
                };
                let c = Console {
                    device: d.name.clone(),
                    kind,
                    mode: IosMode::User,
                };
                match kind {
                    ConsoleKind::Ios => writeln!(
                        out,
                        "Console Cisco IOS de {}. « ? » : aide, « exit » : retour.",
                        c.device
                    )?,
                    ConsoleKind::Host => writeln!(
                        out,
                        "Terminal de {} (Windows et Linux). « help » : aide, « exit » : retour.",
                        c.device
                    )?,
                }
                self.console = Some(c);
            }
            _ => return err("usage : console <équipement> | console exit"),
        }
        Ok(())
    }

    /// Une ligne tapée pendant qu'une console est ouverte.
    pub(super) fn console_line(
        &mut self,
        line: &str,
        out: &mut dyn Write,
    ) -> Result<Flow, CliError> {
        let Some(console) = self.console.clone() else {
            return Ok(Flow::Continue);
        };
        let trimmed = line.trim();
        if trimmed == "?" || trimmed.eq_ignore_ascii_case("help") {
            write!(
                out,
                "{}",
                if console.kind == ConsoleKind::Ios {
                    HELP_IOS
                } else {
                    HELP_HOST
                }
            )?;
            return Ok(Flow::Continue);
        }
        let ctx = self.device_ctx(&console.device)?;
        let actions = match console.kind {
            ConsoleKind::Ios => dialect::ios(&console.mode, trimmed, &ctx),
            ConsoleKind::Host => dialect::host(trimmed, &ctx),
        }
        .map_err(CliError)?;
        let id = self.device(&console.device)?;
        for action in actions {
            match action {
                Action::Netiv { line, quiet } => {
                    if quiet {
                        let mut sink = Vec::new();
                        self.exec_netiv(&line, &mut sink)?;
                    } else {
                        self.exec_netiv(&line, out)?;
                    }
                }
                Action::Mode(mode) => {
                    if let Some(c) = &mut self.console {
                        c.mode = mode;
                    }
                }
                Action::Leave => {
                    self.console = None;
                    writeln!(out, "Retour à NETIV.")?;
                }
                Action::Say(text) => writeln!(out, "{text}")?,
                Action::Ios(show) => write!(out, "{}", ios_show(self.sim()?, id, &ctx, &show)?)?,
                Action::Host(show) => write!(out, "{}", host_show(self.sim()?, id, &show)?)?,
            }
        }
        Ok(Flow::Continue)
    }

    /// Ce que la console doit savoir de l'équipement.
    fn device_ctx(&self, name: &str) -> Result<DeviceCtx, CliError> {
        let id = self.device(name)?;
        let d = self
            .sim()?
            .device(id)
            .ok_or_else(|| CliError(format!("équipement inconnu : {name}")))?;
        let ifaces = d
            .ifaces
            .iter()
            .map(|i| IfaceCtx {
                name: i.name.clone(),
                dhcp: matches!(&d.behavior, Behavior::Ip(n) if n.dhcp_ifaces.contains(&i.id)),
                port: match &d.behavior {
                    Behavior::Switch(sw) => Some(sw.port_mode(i.id)),
                    Behavior::Ip(_) => None,
                },
            })
            .collect();
        Ok(DeviceCtx {
            name: d.name.clone(),
            is_switch: d.kind == DeviceKind::Switch,
            ifaces,
        })
    }
}

fn kind_label(kind: ConsoleKind) -> &'static str {
    match kind {
        ConsoleKind::Ios => "Cisco IOS",
        ConsoleKind::Host => "terminal",
    }
}

fn prompt(c: &Console) -> String {
    match c.kind {
        ConsoleKind::Ios => dialect::ios_prompt(&c.device, &c.mode),
        ConsoleKind::Host => format!("{}:~$", c.device),
    }
}

// ------------------------------------------------------------ affichages

/// L'interface est-elle reliée, avec un câble en marche et l'autre bout actif ?
fn line_up(sim: &Simulation, dev: DeviceId, i: &Interface) -> bool {
    let Some(link) = i.link.and_then(|l| sim.link(l)) else {
        return false;
    };
    if !link.up {
        return false;
    }
    link.peer_of(dev, i.id)
        .and_then(|(pd, pi)| sim.device(pd).and_then(|d| d.ifaces.get(pi.0 as usize)))
        .is_some_and(|peer| peer.admin_up)
}

fn cisco_mac(mac: MacAddr) -> String {
    let hex: String = mac.to_string().replace(':', "").to_ascii_lowercase();
    format!("{}.{}.{}", &hex[0..4], &hex[4..8], &hex[8..12])
}

fn ios_show(
    sim: &Simulation,
    dev: DeviceId,
    ctx: &DeviceCtx,
    show: &IosShow,
) -> Result<String, CliError> {
    let d = sim
        .device(dev)
        .ok_or_else(|| CliError("équipement inconnu".into()))?;
    let mut s = String::new();
    let full = |n: usize| ctx.alias(n).replace("Gi", "GigabitEthernet");
    match show {
        IosShow::IpInterfaceBrief => {
            let _ = writeln!(
                s,
                "{:<30} {:<15} {:<3} {:<6} {:<21} Protocol",
                "Interface", "IP-Address", "OK?", "Method", "Status"
            );
            for (n, i) in d.ifaces.iter().enumerate() {
                let dhcp =
                    matches!(&d.behavior, Behavior::Ip(node) if node.dhcp_ifaces.contains(&i.id));
                let addr =
                    i.ip.map_or_else(|| "unassigned".to_string(), |ip| ip.addr().to_string());
                let method = if dhcp {
                    "DHCP"
                } else if i.ip.is_some() {
                    "manual"
                } else {
                    "unset"
                };
                let up = line_up(sim, dev, i);
                let status = if !i.admin_up {
                    "administratively down"
                } else if up {
                    "up"
                } else {
                    "down"
                };
                let proto = if i.admin_up && up { "up" } else { "down" };
                let name = format!("{} ({})", full(n), i.name);
                let _ = writeln!(
                    s,
                    "{name:<30} {addr:<15} YES {method:<6} {status:<21} {proto}"
                );
            }
        }
        IosShow::RunningConfig => {
            let _ = writeln!(
                s,
                "Building configuration...\n\nCurrent configuration :\n!\nhostname {}\n!",
                d.name
            );
            let ip_node = match &d.behavior {
                Behavior::Ip(n) => Some(n),
                Behavior::Switch(_) => None,
            };
            if let Some(n) = ip_node {
                if d.kind != DeviceKind::Host && d.kind != DeviceKind::Server && !n.forwarding {
                    let _ = writeln!(s, "no ip routing\n!");
                }
            }
            for (idx, i) in d.ifaces.iter().enumerate() {
                let _ = writeln!(s, "interface {}\n description NETIV {}", full(idx), i.name);
                match &d.behavior {
                    Behavior::Switch(sw) => match sw.port_mode(i.id) {
                        PortMode::Access { vlan } => {
                            let _ = writeln!(
                                s,
                                " switchport mode access\n switchport access vlan {vlan}"
                            );
                        }
                        PortMode::Trunk { allowed, native } => {
                            let _ = writeln!(s, " switchport mode trunk");
                            if !allowed.is_empty() {
                                let list: Vec<String> =
                                    allowed.iter().map(u16::to_string).collect();
                                let _ = writeln!(
                                    s,
                                    " switchport trunk allowed vlan {}",
                                    list.join(",")
                                );
                            }
                            if native != 1 {
                                let _ = writeln!(s, " switchport trunk native vlan {native}");
                            }
                        }
                    },
                    Behavior::Ip(n) => {
                        if n.dhcp_ifaces.contains(&i.id) {
                            let _ = writeln!(s, " ip address dhcp");
                        } else if let Some(ip) = i.ip {
                            let _ = writeln!(s, " ip address {} {}", ip.addr(), ip.mask());
                        } else {
                            let _ = writeln!(s, " no ip address");
                        }
                        if let Some(relay) = n.dhcp_relay.get(&i.id) {
                            let _ = writeln!(s, " ip helper-address {relay}");
                        }
                    }
                }
                if !i.admin_up {
                    let _ = writeln!(s, " shutdown");
                }
                let _ = writeln!(s, "!");
            }
            if let Some(n) = ip_node {
                for r in &n.static_routes {
                    let _ = writeln!(
                        s,
                        "ip route {} {} {}",
                        r.dest.network(),
                        r.dest.mask(),
                        r.via
                    );
                }
                if let Some(gw) = n.gateway {
                    let _ = writeln!(s, "ip route 0.0.0.0 0.0.0.0 {gw}");
                }
            }
            let _ = writeln!(s, "!\nend");
        }
        IosShow::IpRoute => {
            let Behavior::Ip(n) = &d.behavior else {
                return Err(CliError(
                    "% Pas de table de routage sur un switch NETIV".into(),
                ));
            };
            let _ = writeln!(
                s,
                "Codes: C - connected, S - static, * - candidate default\n"
            );
            // Passerelle de dernier recours : la passerelle, ou une route statique 0.0.0.0/0.
            let last_resort = n.gateway.or_else(|| {
                n.static_routes
                    .iter()
                    .find(|r| r.dest.prefix() == 0)
                    .map(|r| r.via)
            });
            match last_resort {
                Some(gw) => {
                    let _ = writeln!(s, "Gateway of last resort is {gw} to network 0.0.0.0\n");
                }
                None => {
                    let _ = writeln!(s, "Gateway of last resort is not set\n");
                }
            }
            let (active, _) = n.active_routes(&d.ifaces);
            let alias = |iface: netiv_core::IfaceId| {
                d.ifaces
                    .iter()
                    .position(|x| x.id == iface)
                    .map_or_else(|| "?".to_string(), full)
            };
            for r in &active {
                match (r.origin, r.next_hop) {
                    (RouteOrigin::Connected, _) => {
                        let _ = writeln!(
                            s,
                            "C        {} is directly connected, {}",
                            r.dest,
                            alias(r.iface)
                        );
                    }
                    (RouteOrigin::Default, Some(via)) => {
                        let _ = writeln!(s, "S*       0.0.0.0/0 [1/0] via {via}");
                    }
                    (_, Some(via)) if r.dest.prefix() == 0 => {
                        let _ = writeln!(s, "S*       0.0.0.0/0 [1/0] via {via}");
                    }
                    (_, Some(via)) => {
                        let _ = writeln!(s, "S        {} [1/0] via {via}", r.dest);
                    }
                    (_, None) => {
                        let _ = writeln!(
                            s,
                            "S        {} is directly connected, {}",
                            r.dest,
                            alias(r.iface)
                        );
                    }
                }
            }
        }
        IosShow::VlanBrief => {
            let Behavior::Switch(sw) = &d.behavior else {
                return Err(CliError("% Commande réservée aux switches".into()));
            };
            let mut vlans: std::collections::BTreeMap<u16, Vec<String>> =
                std::collections::BTreeMap::new();
            vlans.entry(1).or_default();
            for (n, i) in d.ifaces.iter().enumerate() {
                match sw.port_mode(i.id) {
                    PortMode::Access { vlan } => vlans.entry(vlan).or_default().push(format!(
                        "{} ({})",
                        ctx.alias(n),
                        i.name
                    )),
                    PortMode::Trunk { allowed, .. } => {
                        for v in allowed {
                            vlans.entry(v).or_default();
                        }
                    }
                }
            }
            let _ = writeln!(s, "{:<4} {:<32} {:<9} Ports", "VLAN", "Name", "Status");
            let _ = writeln!(
                s,
                "---- -------------------------------- --------- -------------------------------"
            );
            for (v, ports) in vlans {
                let name = if v == 1 {
                    "default".to_string()
                } else {
                    format!("VLAN{v:04}")
                };
                let _ = writeln!(s, "{v:<4} {name:<32} {:<9} {}", "active", ports.join(", "));
            }
        }
        IosShow::InterfacesStatus => {
            let Behavior::Switch(sw) = &d.behavior else {
                return Err(CliError("% Commande réservée aux switches".into()));
            };
            let _ = writeln!(s, "{:<16} {:<12} {:<6} Name", "Port", "Status", "Vlan");
            for (n, i) in d.ifaces.iter().enumerate() {
                let status = if !i.admin_up {
                    "disabled"
                } else if line_up(sim, dev, i) {
                    "connected"
                } else {
                    "notconnect"
                };
                let vlan = match sw.port_mode(i.id) {
                    PortMode::Access { vlan } => vlan.to_string(),
                    PortMode::Trunk { .. } => "trunk".to_string(),
                };
                let _ = writeln!(s, "{:<16} {status:<12} {vlan:<6} {}", ctx.alias(n), i.name);
            }
        }
        IosShow::MacTable => {
            let Behavior::Switch(sw) = &d.behavior else {
                return Err(CliError("% Commande réservée aux switches".into()));
            };
            let _ = writeln!(
                s,
                "          Mac Address Table\n-------------------------------------------\n\nVlan    Mac Address       Type        Ports\n----    -----------       --------    -----"
            );
            for ((vlan, mac), port) in &sw.mac_table {
                let n = d.ifaces.iter().position(|x| x.id == *port).unwrap_or(0);
                let _ = writeln!(
                    s,
                    "{vlan:<7} {:<17} {:<11} {}",
                    cisco_mac(*mac),
                    "DYNAMIC",
                    ctx.alias(n)
                );
            }
            let _ = writeln!(
                s,
                "Total Mac Addresses for this criterion: {}",
                sw.mac_table.len()
            );
        }
        IosShow::Arp => {
            let Behavior::Ip(n) = &d.behavior else {
                return Err(CliError("% Pas de table ARP sur un switch NETIV".into()));
            };
            let _ = writeln!(
                s,
                "Protocol  Address          Age (min)  Hardware Addr   Type   Interface"
            );
            for (ip, mac) in n.static_arp.iter().chain(n.arp_cache.iter()) {
                let _ = writeln!(s, "Internet  {ip:<16} {:<10} {} ARPA", "-", cisco_mac(*mac));
            }
        }
        IosShow::AccessLists => {
            s = render::show_acl(sim, dev).map_err(|e| CliError(e.to_string()))?
        }
    }
    Ok(s)
}

/// Libellé à la Windows : « Adresse IPv4. . . . . . . . . . . : ».
fn win(label: &str) -> String {
    // Points et espaces alternés selon la colonne : les « : » sont tous alignés.
    let mut l = label.to_string();
    let mut col = l.chars().count();
    while col < 38 {
        l.push(if col % 2 == 0 { ' ' } else { '.' });
        col += 1;
    }
    format!("   {l} : ")
}

fn host_show(sim: &Simulation, dev: DeviceId, show: &HostShow) -> Result<String, CliError> {
    let d = sim
        .device(dev)
        .ok_or_else(|| CliError("équipement inconnu".into()))?;
    let Behavior::Ip(n) = &d.behavior else {
        return Err(CliError("ce n'est pas un poste".into()));
    };
    let mut s = String::new();
    let dhcp = |i: &Interface| n.dhcp_ifaces.contains(&i.id);
    match show {
        HostShow::Hostname => {
            let _ = writeln!(s, "{}", d.name);
        }
        HostShow::Ipconfig { all } => {
            let _ = writeln!(s, "\nConfiguration IP de Windows\n");
            if *all {
                let _ = writeln!(s, "{}{}\n", win("Nom de l'hôte"), d.name);
            }
            for i in &d.ifaces {
                let _ = writeln!(s, "Carte Ethernet {} :\n", i.name);
                if !i.admin_up || !line_up(sim, dev, i) {
                    let _ = writeln!(s, "{}Média déconnecté", win("Statut du média"));
                }
                if *all {
                    let _ = writeln!(
                        s,
                        "{}{}",
                        win("Adresse physique"),
                        i.mac.to_string().replace(':', "-")
                    );
                    let _ = writeln!(
                        s,
                        "{}{}",
                        win("DHCP activé"),
                        if dhcp(i) { "Oui" } else { "Non" }
                    );
                }
                match i.ip {
                    Some(ip) => {
                        let auto = ip.addr().octets()[0] == 169 && ip.addr().octets()[1] == 254;
                        let label = if auto {
                            "Adresse IPv4 d'autoconfiguration"
                        } else {
                            "Adresse IPv4"
                        };
                        let _ = writeln!(s, "{}{}", win(label), ip.addr());
                        let _ = writeln!(s, "{}{}", win("Masque de sous-réseau"), ip.mask());
                    }
                    None => {
                        let _ = writeln!(s, "{}(aucune)", win("Adresse IPv4"));
                    }
                }
                let gw = n.gateway.filter(|g| i.ip.is_some_and(|ip| ip.contains(*g)));
                let _ = writeln!(
                    s,
                    "{}{}",
                    win("Passerelle par défaut"),
                    gw.map(|g| g.to_string()).unwrap_or_default()
                );
                if *all {
                    if let Some(dns) = n.effective_dns() {
                        let _ = writeln!(s, "{}{dns}", win("Serveurs DNS"));
                    }
                }
                let _ = writeln!(s);
            }
        }
        HostShow::IpAddr => {
            let _ = writeln!(
                s,
                "1: lo: <LOOPBACK,UP,LOWER_UP> mtu 65536 state UNKNOWN\n    inet 127.0.0.1/8 scope host lo"
            );
            for (k, i) in d.ifaces.iter().enumerate() {
                let up = i.admin_up && line_up(sim, dev, i);
                let flags = if i.admin_up {
                    if up {
                        "BROADCAST,MULTICAST,UP,LOWER_UP"
                    } else {
                        "NO-CARRIER,BROADCAST,MULTICAST,UP"
                    }
                } else {
                    "BROADCAST,MULTICAST"
                };
                let _ = writeln!(
                    s,
                    "{}: {}: <{flags}> mtu 1500 state {}",
                    k + 2,
                    i.name,
                    if up { "UP" } else { "DOWN" }
                );
                let _ = writeln!(
                    s,
                    "    link/ether {} brd ff:ff:ff:ff:ff:ff",
                    i.mac.to_string().to_ascii_lowercase()
                );
                if let Some(ip) = i.ip {
                    let _ = writeln!(
                        s,
                        "    inet {ip} brd {} scope global{} {}",
                        ip.broadcast(),
                        if dhcp(i) { " dynamic" } else { "" },
                        i.name
                    );
                }
            }
        }
        HostShow::IpRoute => {
            if let Some(gw) = n.gateway {
                let dev_name = d
                    .ifaces
                    .iter()
                    .find(|i| i.ip.is_some_and(|ip| ip.contains(gw)))
                    .map_or("?", |i| i.name.as_str());
                let _ = writeln!(
                    s,
                    "default via {gw} dev {dev_name}{}",
                    if n.gateway_learned { " proto dhcp" } else { "" }
                );
            }
            for i in &d.ifaces {
                if let Some(ip) = i.ip {
                    let _ = writeln!(
                        s,
                        "{}/{} dev {} proto kernel scope link src {}",
                        ip.network(),
                        ip.prefix(),
                        i.name,
                        ip.addr()
                    );
                }
            }
            for r in &n.static_routes {
                let _ = writeln!(s, "{} via {}", r.dest, r.via);
            }
        }
        HostShow::RoutePrint => {
            let _ = writeln!(
                s,
                "===========================================================================\nItinéraires actifs :\nDestination réseau    Masque réseau  Adr. passerelle   Adr. interface Métrique"
            );
            let iface_for = |target: std::net::Ipv4Addr| {
                d.ifaces
                    .iter()
                    .find_map(|i| i.ip.filter(|ip| ip.contains(target)))
                    .map_or_else(|| "?".to_string(), |ip| ip.addr().to_string())
            };
            if let Some(gw) = n.gateway {
                let _ = writeln!(
                    s,
                    "{:>18} {:>16} {:>16} {:>16} {:>6}",
                    "0.0.0.0",
                    "0.0.0.0",
                    gw,
                    iface_for(gw),
                    25
                );
            }
            for i in &d.ifaces {
                if let Some(ip) = i.ip {
                    let _ = writeln!(
                        s,
                        "{:>18} {:>16} {:>16} {:>16} {:>6}",
                        ip.network(),
                        ip.mask(),
                        "On-link",
                        ip.addr(),
                        281
                    );
                }
            }
            for r in &n.static_routes {
                let _ = writeln!(
                    s,
                    "{:>18} {:>16} {:>16} {:>16} {:>6}",
                    r.dest.network(),
                    r.dest.mask(),
                    r.via,
                    iface_for(r.via),
                    26
                );
            }
            let _ = writeln!(
                s,
                "==========================================================================="
            );
        }
        HostShow::Arp => {
            for i in d.ifaces.iter().filter(|i| i.ip.is_some()) {
                let _ = writeln!(
                    s,
                    "\nInterface : {} --- 0x{:x}\n  Adresse Internet      Adresse physique      Type",
                    i.ip.map(|ip| ip.addr().to_string()).unwrap_or_default(),
                    i.id.0 + 2
                );
                for (ip, mac) in n
                    .static_arp
                    .iter()
                    .chain(n.arp_cache.iter())
                    .filter(|(ip, _)| i.ip.is_some_and(|net| net.contains(**ip)))
                {
                    let kind = if n.static_arp.contains_key(ip) {
                        "statique"
                    } else {
                        "dynamique"
                    };
                    let _ = writeln!(
                        s,
                        "  {ip:<21} {:<21} {kind}",
                        mac.to_string().replace(':', "-").to_ascii_lowercase()
                    );
                }
            }
        }
        HostShow::Neigh => {
            for (ip, mac) in n.static_arp.iter().chain(n.arp_cache.iter()) {
                let dev_name = d
                    .ifaces
                    .iter()
                    .find(|i| i.ip.is_some_and(|net| net.contains(*ip)))
                    .map_or("?", |i| i.name.as_str());
                let state = if n.static_arp.contains_key(ip) {
                    "PERMANENT"
                } else {
                    "REACHABLE"
                };
                let _ = writeln!(
                    s,
                    "{ip} dev {dev_name} lladdr {} {state}",
                    mac.to_string().to_ascii_lowercase()
                );
            }
        }
        HostShow::ResolvConf => match n.effective_dns() {
            Some(dns) => {
                let _ = writeln!(
                    s,
                    "# généré par NETIV{}\nnameserver {dns}",
                    if n.dns.is_none() {
                        " (reçu par DHCP)"
                    } else {
                        ""
                    }
                );
            }
            None => {
                let _ = writeln!(s, "# aucun serveur DNS configuré");
            }
        },
    }
    Ok(s)
}

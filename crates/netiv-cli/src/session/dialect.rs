//! Dialectes de console : Cisco IOS (routeurs, switches) et terminal de poste
//! (commandes Windows et Linux). Ils s'ajoutent à la syntaxe NETIV sans la
//! remplacer.
//!
//! Ce module est **pur** : il traduit une ligne en actions (commandes NETIV
//! équivalentes, changement de mode, affichage) à partir d'une description de
//! l'équipement, sans toucher à la simulation. Les commandes NETIV produites
//! passent ensuite par les mêmes contrôles que si l'élève les avait tapées.

use std::net::Ipv4Addr;

use netiv_core::switch::PortMode;

/// Type de console ouverte sur un équipement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsoleKind {
    /// CLI Cisco IOS (routeur, switch).
    Ios,
    /// Terminal de poste ou de serveur (Windows et Linux).
    Host,
}

impl ConsoleKind {
    /// Identifiant stable (snapshot JSON, interface web).
    pub fn slug(self) -> &'static str {
        match self {
            ConsoleKind::Ios => "ios",
            ConsoleKind::Host => "host",
        }
    }
}

/// Mode de la CLI Cisco.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IosMode {
    /// `R1>`
    User,
    /// `R1#`
    Exec,
    /// `R1(config)#`
    Config,
    /// `R1(config-if)#` sur une interface (nom NETIV).
    Interface(String),
    /// `SW1(config-vlan)#`
    Vlan(u16),
}

/// Ce que la console sait de l'équipement (copie, sans la simulation).
#[derive(Clone, Debug)]
pub struct DeviceCtx {
    /// Nom NETIV (`R1`).
    pub name: String,
    /// Switch (sinon routeur, poste ou serveur).
    pub is_switch: bool,
    /// Interfaces, dans l'ordre de l'équipement.
    pub ifaces: Vec<IfaceCtx>,
}

/// Une interface vue par la console.
#[derive(Clone, Debug)]
pub struct IfaceCtx {
    /// Nom NETIV (`eth0`, `p5`).
    pub name: String,
    /// Adresse configurée par DHCP.
    pub dhcp: bool,
    /// Mode du port (switches).
    pub port: Option<PortMode>,
}

impl DeviceCtx {
    /// Premier numéro des alias Cisco : `Gi0/1` sur un switch, `Gi0/0` sur un
    /// routeur (comme sur le vrai matériel).
    pub fn alias_base(&self) -> usize {
        usize::from(self.is_switch)
    }

    /// Alias Cisco d'une interface (`Gi0/2`).
    pub fn alias(&self, index: usize) -> String {
        format!("Gi0/{}", index + self.alias_base())
    }

    /// Nom NETIV d'une interface désignée par son nom ou un alias Cisco
    /// (`GigabitEthernet0/1`, `gi0/1`, `Fa0/1`, `e0/1`, `g0/1`).
    pub fn resolve_iface(&self, raw: &str) -> Option<&IfaceCtx> {
        let lower = raw.to_ascii_lowercase();
        if let Some(i) = self
            .ifaces
            .iter()
            .find(|i| i.name.eq_ignore_ascii_case(raw))
        {
            return Some(i);
        }
        let digits = lower.trim_start_matches(|c: char| c.is_ascii_alphabetic());
        let prefix = &lower[..lower.len() - digits.len()];
        let known = [
            "g",
            "gi",
            "gig",
            "giga",
            "gigabit",
            "gigabitethernet",
            "f",
            "fa",
            "fast",
            "fastethernet",
            "e",
            "eth",
            "ethernet",
        ];
        if !known.contains(&prefix) {
            return None;
        }
        let (slot, port) = digits.split_once('/')?;
        if slot != "0" {
            return None;
        }
        let n: usize = port.parse().ok()?;
        n.checked_sub(self.alias_base())
            .and_then(|i| self.ifaces.get(i))
    }
}

/// Ce que l'IOS sait afficher.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IosShow {
    /// `show ip interface brief`
    IpInterfaceBrief,
    /// `show running-config`
    RunningConfig,
    /// `show ip route`
    IpRoute,
    /// `show vlan brief`
    VlanBrief,
    /// `show interfaces status`
    InterfacesStatus,
    /// `show mac address-table`
    MacTable,
    /// `show arp`
    Arp,
    /// `show access-lists`
    AccessLists,
}

/// Ce que le terminal sait afficher.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostShow {
    /// `ipconfig` (`all` : `ipconfig /all`)
    Ipconfig { all: bool },
    /// `ip addr`
    IpAddr,
    /// `ip route`
    IpRoute,
    /// `route print`
    RoutePrint,
    /// `arp -a`
    Arp,
    /// `ip neigh`
    Neigh,
    /// `cat /etc/resolv.conf`
    ResolvConf,
    /// `hostname`
    Hostname,
}

/// Une action produite par une ligne de console.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Commande NETIV équivalente. `quiet` : comme sur le vrai matériel, une
    /// commande de configuration réussie n'affiche rien.
    Netiv { line: String, quiet: bool },
    /// Affichage Cisco.
    Ios(IosShow),
    /// Affichage du terminal.
    Host(HostShow),
    /// Nouveau mode Cisco.
    Mode(IosMode),
    /// Quitte la console (retour à NETIV).
    Leave,
    /// Message à afficher tel quel.
    Say(String),
}

type Res = Result<Vec<Action>, String>;

const INVALID: &str = "% Invalid input detected at '^' marker.";
const INCOMPLETE: &str = "% Incomplete command.";

fn netiv(line: String) -> Action {
    Action::Netiv { line, quiet: false }
}

fn quiet(line: String) -> Action {
    Action::Netiv { line, quiet: true }
}

/// Mot-clé Cisco abrégé : le préfixe doit désigner un seul mot-clé.
fn kw<'a>(word: &str, options: &[&'a str]) -> Result<&'a str, String> {
    let w = word.to_ascii_lowercase();
    if let Some(exact) = options.iter().find(|o| **o == w) {
        return Ok(exact);
    }
    let found: Vec<&&str> = options.iter().filter(|o| o.starts_with(&w)).collect();
    match found.as_slice() {
        [one] => Ok(one),
        [] => Err(INVALID.to_string()),
        _ => Err(format!("% Ambiguous command:  \"{word}\"")),
    }
}

/// Masque décimal pointé → longueur de préfixe (masque contigu uniquement).
pub fn mask_to_prefix(mask: &str) -> Option<u8> {
    let m: Ipv4Addr = mask.parse().ok()?;
    let bits = u32::from(m);
    let ones = bits.leading_ones();
    (bits.checked_shl(ones).unwrap_or(0) == 0).then_some(ones as u8)
}

fn ip(s: &str) -> Result<Ipv4Addr, String> {
    s.parse().map_err(|_| INVALID.to_string())
}

/// Réseau Cisco (`10.0.0.0 255.255.255.0`) → CIDR (`10.0.0.0/24`).
fn cidr(addr: &str, mask: &str) -> Result<String, String> {
    let a = ip(addr)?;
    let p = mask_to_prefix(mask).ok_or_else(|| "% Inconsistent address and mask".to_string())?;
    Ok(format!("{a}/{p}"))
}

/// Liste de VLAN Cisco (`10,20,30-32`) → liste NETIV (`10,20,30,31,32`).
fn vlan_list(raw: &str) -> Result<Vec<u16>, String> {
    let mut out = Vec::new();
    for part in raw.split(',') {
        match part.split_once('-') {
            Some((a, b)) => {
                let (a, b): (u16, u16) = (
                    a.parse().map_err(|_| INVALID)?,
                    b.parse().map_err(|_| INVALID)?,
                );
                if a > b || b - a > 200 {
                    return Err(INVALID.to_string());
                }
                out.extend(a..=b);
            }
            None => out.push(part.parse().map_err(|_| INVALID)?),
        }
    }
    Ok(out)
}

fn join_vlans(v: &[u16]) -> String {
    v.iter().map(u16::to_string).collect::<Vec<_>>().join(",")
}

// ----------------------------------------------------------------- Cisco IOS

/// Une ligne tapée dans la CLI Cisco.
pub fn ios(mode: &IosMode, line: &str, dev: &DeviceCtx) -> Res {
    let words: Vec<&str> = line.split_whitespace().collect();
    let Some(&first) = words.first() else {
        return Ok(vec![]);
    };
    // `do <commande>` : une commande du mode privilégié depuis la configuration.
    if matches!(
        mode,
        IosMode::Config | IosMode::Interface(_) | IosMode::Vlan(_)
    ) && first.eq_ignore_ascii_case("do")
    {
        return ios_exec(&IosMode::Exec, &words[1..], dev);
    }
    match mode {
        IosMode::User | IosMode::Exec => ios_exec(mode, &words, dev),
        IosMode::Config => ios_config(&words, dev),
        IosMode::Interface(iface) => ios_interface(iface, &words, dev),
        IosMode::Vlan(_) => ios_vlan(&words, dev),
    }
}

fn ios_exec(mode: &IosMode, w: &[&str], dev: &DeviceCtx) -> Res {
    let Some(&first) = w.first() else {
        return Ok(vec![]);
    };
    let privileged = *mode == IosMode::Exec;
    let verbs: &[&str] = if privileged {
        &[
            "configure",
            "copy",
            "disable",
            "enable",
            "end",
            "exit",
            "logout",
            "ping",
            "show",
            "traceroute",
            "write",
        ]
    } else {
        &["enable", "exit", "logout", "ping", "show", "traceroute"]
    };
    match kw(first, verbs)? {
        "enable" => Ok(vec![Action::Mode(IosMode::Exec)]),
        "disable" => Ok(vec![Action::Mode(IosMode::User)]),
        "end" => Ok(vec![]),
        "exit" | "logout" => Ok(vec![Action::Leave]),
        "configure" => match w.get(1) {
            None => Ok(vec![Action::Say("Configuring from terminal, memory, or network [terminal]? terminal".into()), Action::Mode(IosMode::Config)]),
            Some(t) if kw(t, &["terminal"]).is_ok() => Ok(vec![
                Action::Say("Enter configuration commands, one per line.  End with CNTL/Z.".into()),
                Action::Mode(IosMode::Config),
            ]),
            Some(_) => Err(INVALID.into()),
        },
        "copy" | "write" => Ok(vec![Action::Say(
            "[OK]  (NETIV : la configuration est déjà appliquée, il n'y a pas de mémoire à sauvegarder)".into(),
        )]),
        "ping" | "traceroute" => {
            let target = w.get(1).ok_or(INCOMPLETE)?;
            let mut actions = vec![netiv(format!("ping {} {target} 5", dev.name))];
            if w[0].to_ascii_lowercase().starts_with('t') {
                actions.push(Action::Say("(NETIV : « trace <id> » montre chaque saut du paquet.)".into()));
            }
            Ok(actions)
        }
        "show" => ios_show(&w[1..], dev),
        _ => Err(INVALID.into()),
    }
}

fn ios_show(w: &[&str], dev: &DeviceCtx) -> Res {
    let Some(&what) = w.first() else {
        return Err(INCOMPLETE.into());
    };
    let show = match kw(
        what,
        &[
            "access-lists",
            "arp",
            "interfaces",
            "ip",
            "mac",
            "running-config",
            "vlan",
        ],
    )? {
        "running-config" => IosShow::RunningConfig,
        "arp" => IosShow::Arp,
        "access-lists" => IosShow::AccessLists,
        "ip" => match w
            .get(1)
            .map(|x| kw(x, &["arp", "interface", "route"]))
            .transpose()?
        {
            Some("interface") => match w.get(2) {
                Some(b) if kw(b, &["brief"]).is_ok() => IosShow::IpInterfaceBrief,
                None => IosShow::IpInterfaceBrief,
                _ => return Err(INVALID.into()),
            },
            Some("route") => IosShow::IpRoute,
            Some("arp") => IosShow::Arp,
            _ => return Err(INCOMPLETE.into()),
        },
        "interfaces" => match w.get(1) {
            Some(s) if kw(s, &["status"]).is_ok() => IosShow::InterfacesStatus,
            None => IosShow::IpInterfaceBrief,
            _ => return Err(INVALID.into()),
        },
        "vlan" => IosShow::VlanBrief,
        "mac" => IosShow::MacTable,
        _ => return Err(INVALID.into()),
    };
    let switch_only = matches!(
        show,
        IosShow::VlanBrief | IosShow::InterfacesStatus | IosShow::MacTable
    );
    let ip_only = matches!(show, IosShow::IpRoute | IosShow::Arp | IosShow::AccessLists);
    if switch_only && !dev.is_switch {
        return Err("% Commande réservée aux switches (NETIV)".into());
    }
    if ip_only && dev.is_switch {
        return Err("% Ce switch NETIV n'a pas de pile IP (commande réservée aux routeurs)".into());
    }
    Ok(vec![Action::Ios(show)])
}

fn ios_config(w: &[&str], dev: &DeviceCtx) -> Res {
    let negate = w[0].eq_ignore_ascii_case("no");
    let w = if negate { &w[1..] } else { w };
    let Some(&first) = w.first() else {
        return Err(INCOMPLETE.into());
    };
    match kw(
        first,
        &["end", "exit", "hostname", "interface", "ip", "vlan"],
    )? {
        "end" => Ok(vec![Action::Mode(IosMode::Exec)]),
        "exit" => Ok(vec![Action::Mode(IosMode::Exec)]),
        "hostname" => Ok(vec![Action::Say(
            "% NETIV : le nom de l'équipement ne se change pas (il sert dans les labs)".into(),
        )]),
        "interface" => {
            let raw: String = w[1..].concat();
            if raw.is_empty() {
                return Err(INCOMPLETE.into());
            }
            if raw.to_ascii_lowercase().starts_with("vlan") {
                return Err("% NETIV : les interfaces VLAN (SVI) ne sont pas simulées".into());
            }
            let i = dev.resolve_iface(&raw).ok_or_else(|| {
                let names: Vec<String> = dev
                    .ifaces
                    .iter()
                    .enumerate()
                    .map(|(n, i)| format!("{} ({})", dev.alias(n), i.name))
                    .collect();
                format!(
                    "% Invalid interface « {raw} ». Interfaces : {}",
                    names.join(", ")
                )
            })?;
            Ok(vec![Action::Mode(IosMode::Interface(i.name.clone()))])
        }
        "vlan" => {
            if !dev.is_switch {
                return Err(INVALID.into());
            }
            let id: u16 = w.get(1).ok_or(INCOMPLETE)?.parse().map_err(|_| INVALID)?;
            if !(1..=4094).contains(&id) {
                return Err("% Bad VLAN list".into());
            }
            if negate {
                return Ok(vec![Action::Say(format!(
                    "% NETIV : VLAN {id} : les VLAN n'existent que par les ports qui les portent"
                ))]);
            }
            Ok(vec![Action::Mode(IosMode::Vlan(id))])
        }
        "ip" => {
            let Some(&sub) = w.get(1) else {
                return Err(INCOMPLETE.into());
            };
            match kw(
                sub,
                &["default-gateway", "domain-lookup", "route", "routing"],
            )? {
                "route" => {
                    if dev.is_switch {
                        return Err(
                            "% Ce switch NETIV n'a pas de pile IP (routes : sur un routeur)".into(),
                        );
                    }
                    let (Some(a), Some(m)) = (w.get(2), w.get(3)) else {
                        return Err(INCOMPLETE.into());
                    };
                    let net = cidr(a, m)?;
                    if negate {
                        return Ok(vec![quiet(format!("route del {} {net}", dev.name))]);
                    }
                    let gw = ip(w.get(4).ok_or(INCOMPLETE)?)?;
                    // Route par défaut : la passerelle NETIV.
                    if net == "0.0.0.0/0" {
                        return Ok(vec![quiet(format!("gateway {} {gw}", dev.name))]);
                    }
                    Ok(vec![quiet(format!(
                        "route add {} {net} via {gw}",
                        dev.name
                    ))])
                }
                "routing" => Ok(vec![quiet(format!(
                    "forwarding {} {}",
                    dev.name,
                    if negate { "off" } else { "on" }
                ))]),
                _ => Ok(vec![Action::Say(
                    "% Commande reconnue mais non simulée dans NETIV".into(),
                )]),
            }
        }
        _ => Err(INVALID.into()),
    }
}

fn ios_interface(iface: &str, w: &[&str], dev: &DeviceCtx) -> Res {
    let negate = w[0].eq_ignore_ascii_case("no");
    let rest = if negate { &w[1..] } else { w };
    let Some(&first) = rest.first() else {
        return Err(INCOMPLETE.into());
    };
    let d = &dev.name;
    let current = dev.ifaces.iter().find(|i| i.name == iface);
    match kw(
        first,
        &[
            "description",
            "duplex",
            "end",
            "exit",
            "interface",
            "ip",
            "shutdown",
            "speed",
            "switchport",
        ],
    )? {
        "end" => Ok(vec![Action::Mode(IosMode::Exec)]),
        "exit" => Ok(vec![Action::Mode(IosMode::Config)]),
        "interface" => ios_config(w, dev),
        "description" | "duplex" | "speed" => Ok(vec![]),
        "shutdown" => Ok(vec![quiet(format!(
            "iface {d} {iface} {}",
            if negate { "up" } else { "down" }
        ))]),
        "ip" => {
            if dev.is_switch {
                return Err("% Ce switch NETIV n'a pas de pile IP : l'adressage se fait sur les routeurs et les postes".into());
            }
            let Some(&sub) = rest.get(1) else {
                return Err(INCOMPLETE.into());
            };
            match kw(sub, &["address", "helper-address"])? {
                "address" => {
                    if negate {
                        return Ok(vec![quiet(format!("ip {d} {iface} none"))]);
                    }
                    match (rest.get(2), rest.get(3)) {
                        (Some(x), None) if x.eq_ignore_ascii_case("dhcp") => {
                            Ok(vec![netiv(format!("ip {d} {iface} dhcp"))])
                        }
                        (Some(a), Some(m)) => {
                            let p = mask_to_prefix(m).ok_or("% Bad mask")?;
                            Ok(vec![quiet(format!("ip {d} {iface} {}/{p}", ip(a)?))])
                        }
                        _ => Err(INCOMPLETE.into()),
                    }
                }
                "helper-address" => {
                    if negate {
                        return Ok(vec![quiet(format!("dhcp relay {d} {iface} none"))]);
                    }
                    let server = ip(rest.get(2).ok_or(INCOMPLETE)?)?;
                    Ok(vec![quiet(format!("dhcp relay {d} {iface} {server}"))])
                }
                _ => Err(INVALID.into()),
            }
        }
        "switchport" => {
            if !dev.is_switch {
                return Err(INVALID.into());
            }
            let Some(&sub) = rest.get(1) else {
                return Err(INCOMPLETE.into());
            };
            let mode = current.and_then(|i| i.port.clone()).unwrap_or_default();
            match kw(sub, &["access", "mode", "trunk"])? {
                "mode" => match rest
                    .get(2)
                    .map(|m| kw(m, &["access", "trunk"]))
                    .transpose()?
                {
                    Some("access") => match mode {
                        PortMode::Trunk { native, .. } => {
                            Ok(vec![quiet(format!("port {d} {iface} access {native}"))])
                        }
                        PortMode::Access { .. } => Ok(vec![]),
                    },
                    Some("trunk") => match mode {
                        PortMode::Access { vlan } => {
                            Ok(vec![quiet(format!("port {d} {iface} trunk native {vlan}"))])
                        }
                        PortMode::Trunk { .. } => Ok(vec![]),
                    },
                    _ => Err(INCOMPLETE.into()),
                },
                "access" => {
                    let (Some(v), Some(id)) = (rest.get(2), rest.get(3)) else {
                        return Err(INCOMPLETE.into());
                    };
                    kw(v, &["vlan"])?;
                    let id: u16 = if negate {
                        1
                    } else {
                        id.parse().map_err(|_| INVALID)?
                    };
                    Ok(vec![quiet(format!("port {d} {iface} access {id}"))])
                }
                "trunk" => {
                    let (allowed, native) = match mode {
                        PortMode::Trunk { allowed, native } => (allowed, native),
                        PortMode::Access { vlan } => (Vec::new(), vlan),
                    };
                    match rest
                        .get(2)
                        .map(|x| kw(x, &["allowed", "native"]))
                        .transpose()?
                    {
                        Some("native") => {
                            let id: u16 = rest
                                .get(4)
                                .ok_or(INCOMPLETE)?
                                .parse()
                                .map_err(|_| INVALID)?;
                            Ok(vec![quiet(
                                format!(
                                    "port {d} {iface} trunk {} native {id}",
                                    join_vlans(&allowed)
                                )
                                .replace("  ", " "),
                            )])
                        }
                        Some("allowed") => {
                            // switchport trunk allowed vlan <liste | all | add <liste> | remove <liste>>
                            let spec = rest.get(4).ok_or(INCOMPLETE)?;
                            let list = match spec.to_ascii_lowercase().as_str() {
                                "all" => Vec::new(),
                                "add" => {
                                    let mut l = allowed.clone();
                                    l.extend(vlan_list(rest.get(5).ok_or(INCOMPLETE)?)?);
                                    l.sort_unstable();
                                    l.dedup();
                                    l
                                }
                                "remove" => {
                                    let del = vlan_list(rest.get(5).ok_or(INCOMPLETE)?)?;
                                    allowed
                                        .iter()
                                        .copied()
                                        .filter(|v| !del.contains(v))
                                        .collect()
                                }
                                _ => vlan_list(spec)?,
                            };
                            let vl = join_vlans(&list);
                            Ok(vec![quiet(if vl.is_empty() {
                                format!("port {d} {iface} trunk native {native}")
                            } else {
                                format!("port {d} {iface} trunk {vl} native {native}")
                            })])
                        }
                        _ => Err(INCOMPLETE.into()),
                    }
                }
                _ => Err(INVALID.into()),
            }
        }
        _ => Err(INVALID.into()),
    }
}

fn ios_vlan(w: &[&str], dev: &DeviceCtx) -> Res {
    match kw(w[0], &["end", "exit", "name", "vlan"])? {
        "end" => Ok(vec![Action::Mode(IosMode::Exec)]),
        "exit" => Ok(vec![Action::Mode(IosMode::Config)]),
        "vlan" => ios_config(w, dev),
        // Le nom d'un VLAN n'a pas d'effet sur le trafic : accepté.
        _ => Ok(vec![]),
    }
}

/// Invite Cisco.
pub fn ios_prompt(name: &str, mode: &IosMode) -> String {
    match mode {
        IosMode::User => format!("{name}>"),
        IosMode::Exec => format!("{name}#"),
        IosMode::Config => format!("{name}(config)#"),
        IosMode::Interface(_) => format!("{name}(config-if)#"),
        IosMode::Vlan(_) => format!("{name}(config-vlan)#"),
    }
}

// ------------------------------------------------------- terminal de poste

/// Une ligne tapée dans le terminal d'un poste (commandes Windows ou Linux).
pub fn host(line: &str, dev: &DeviceCtx) -> Res {
    let words = shell_words(line);
    let w: Vec<&str> = words.iter().map(String::as_str).collect();
    let Some(&first) = w.first() else {
        return Ok(vec![]);
    };
    let d = &dev.name;
    let first_iface = || {
        dev.ifaces
            .first()
            .map(|i| i.name.clone())
            .ok_or_else(|| "aucune interface".to_string())
    };
    let iface = |raw: &str| {
        dev.resolve_iface(raw)
            .map(|i| i.name.clone())
            .ok_or_else(|| {
                format!(
                    "interface inconnue : « {raw} » (interfaces : {})",
                    dev.ifaces
                        .iter()
                        .map(|i| i.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
    };
    match first.to_ascii_lowercase().as_str() {
        "exit" | "logout" => Ok(vec![Action::Leave]),
        "hostname" => Ok(vec![Action::Host(HostShow::Hostname)]),
        "cls" | "clear" => Ok(vec![]),

        // ----- Windows
        "ipconfig" => match w.get(1).map(|s| s.to_ascii_lowercase()) {
            None => Ok(vec![Action::Host(HostShow::Ipconfig { all: false })]),
            Some(f) if f == "/all" => Ok(vec![Action::Host(HostShow::Ipconfig { all: true })]),
            Some(f) if f == "/renew" || f == "/release" => {
                let renew = f == "/renew";
                let targets: Vec<&IfaceCtx> = match w.get(2) {
                    Some(raw) => vec![
                        dev.resolve_iface(raw)
                            .ok_or_else(|| format!("aucun adaptateur ne correspond à « {raw} »"))?,
                    ],
                    None => dev.ifaces.iter().filter(|i| i.dhcp).collect(),
                };
                if targets.iter().all(|i| !i.dhcp) {
                    return Err("L'opération a échoué : aucune carte n'est configurée en DHCP (essayez « netsh interface ip set address … dhcp »).".into());
                }
                Ok(targets
                    .into_iter()
                    .filter(|i| i.dhcp)
                    .map(|i| {
                        netiv(format!(
                            "dhcp {d} {} {}",
                            if renew { "renew" } else { "release" },
                            i.name
                        ))
                    })
                    .collect())
            }
            Some(f) if f == "/flushdns" => Ok(vec![Action::Say(
                "Le cache de résolution DNS a été vidé. (NETIV n'en garde pas.)".into(),
            )]),
            Some(_) => {
                Err("Option non reconnue : ipconfig [/all | /release | /renew | /flushdns]".into())
            }
        },
        "tracert" => Ok(vec![
            netiv(format!(
                "ping {d} {} 1",
                w.get(1).ok_or("usage : tracert <cible>")?
            )),
            Action::Say("(NETIV : « trace <id> » montre chaque saut du paquet.)".into()),
        ]),
        "route" => windows_route(&w, d),
        "netsh" => netsh(&w, dev),
        "arp" => match w.get(1) {
            Some(a) if a.eq_ignore_ascii_case("-a") => Ok(vec![Action::Host(HostShow::Arp)]),
            _ => Err("usage : arp -a".into()),
        },

        // ----- Linux
        "ip" => linux_ip(&w, dev, &iface),
        "dhclient" => {
            let release = w.contains(&"-r");
            let name = match w.iter().skip(1).find(|x| !x.starts_with('-')) {
                Some(raw) => iface(raw)?,
                None => first_iface()?,
            };
            let is_dhcp = dev.ifaces.iter().any(|i| i.name == name && i.dhcp);
            Ok(vec![netiv(match (release, is_dhcp) {
                (true, _) => format!("dhcp {d} release {name}"),
                (false, true) => format!("dhcp {d} renew {name}"),
                // dhclient sur une interface fixe : elle passe en DHCP.
                (false, false) => format!("ip {d} {name} dhcp"),
            })])
        }
        "resolvectl" => match (w.get(1), w.get(2), w.get(3)) {
            (Some(&"dns"), Some(_), Some(server)) => Ok(vec![quiet(format!(
                "dns {d} {}",
                ip(server).map_err(|_| "adresse de serveur DNS invalide".to_string())?
            ))]),
            (Some(&"revert"), Some(_), None) => Ok(vec![quiet(format!("dns {d} none"))]),
            _ => Err(
                "usage : resolvectl dns <interface> <serveur> | resolvectl revert <interface>"
                    .into(),
            ),
        },
        "cat" => match w.get(1) {
            Some(&"/etc/resolv.conf") => Ok(vec![Action::Host(HostShow::ResolvConf)]),
            Some(&"/etc/hostname") => Ok(vec![Action::Host(HostShow::Hostname)]),
            _ => Err("NETIV : seuls /etc/resolv.conf et /etc/hostname sont simulés".into()),
        },
        "traceroute" => Ok(vec![
            netiv(format!(
                "ping {d} {} 1",
                w.get(1).ok_or("usage : traceroute <cible>")?
            )),
            Action::Say("(NETIV : « trace <id> » montre chaque saut du paquet.)".into()),
        ]),
        "dig" => Ok(vec![netiv(format!(
            "nslookup {d} {}",
            w.iter()
                .skip(1)
                .find(|x| !x.starts_with('@') && !x.starts_with('+'))
                .ok_or("usage : dig <nom>")?
        ))]),

        // ----- communes
        "ping" => {
            let mut count = 4u32;
            let mut target = None;
            let mut it = w.iter().skip(1);
            while let Some(a) = it.next() {
                match *a {
                    "-n" | "-c" => {
                        count = it
                            .next()
                            .and_then(|n| n.parse().ok())
                            .ok_or("nombre de paquets invalide")?
                    }
                    "-t" => count = 4,
                    t => target = Some(t),
                }
            }
            Ok(vec![netiv(format!(
                "ping {d} {} {}",
                target.ok_or("usage : ping <cible> [-n N | -c N]")?,
                count.clamp(1, 10)
            ))])
        }
        "nslookup" => {
            let name = w.get(1).ok_or("usage : nslookup <nom>")?;
            let mut actions = vec![netiv(format!("nslookup {d} {name}"))];
            if w.len() > 2 {
                actions.push(Action::Say("(NETIV : le serveur DNS interrogé est celui du poste ; le serveur donné en argument est ignoré.)".into()));
            }
            Ok(actions)
        }
        "curl" | "wget" => {
            let url = w
                .iter()
                .skip(1)
                .find(|x| !x.starts_with('-'))
                .ok_or("usage : curl http://<hôte>[:port]")?;
            let (host, port) = parse_url(url)?;
            Ok(vec![netiv(format!("curl {d} {host} {port}"))])
        }
        other => Err(format!(
            "{other} : commande introuvable (dans ce terminal : ipconfig, ping, tracert, nslookup, netsh, route, arp -a, ip, dhclient, resolvectl, curl… ; « exit » pour revenir à NETIV)"
        )),
    }
}

fn windows_route(w: &[&str], d: &str) -> Res {
    match w.get(1).map(|s| s.to_ascii_lowercase()).as_deref() {
        Some("print") | None => Ok(vec![Action::Host(HostShow::RoutePrint)]),
        Some("add") => {
            // route add <réseau> mask <masque> <passerelle>
            let (Some(net), Some(m), Some(mask), Some(gw)) =
                (w.get(2), w.get(3), w.get(4), w.get(5))
            else {
                return Err("usage : route add <réseau> mask <masque> <passerelle>".into());
            };
            if !m.eq_ignore_ascii_case("mask") {
                return Err("usage : route add <réseau> mask <masque> <passerelle>".into());
            }
            let c = cidr(net, mask).map_err(|_| "réseau ou masque invalide".to_string())?;
            let gw = ip(gw).map_err(|_| "passerelle invalide".to_string())?;
            Ok(vec![
                if c == "0.0.0.0/0" {
                    quiet(format!("gateway {d} {gw}"))
                } else {
                    quiet(format!("route add {d} {c} via {gw}"))
                },
                Action::Say(" OK!".into()),
            ])
        }
        Some("delete") => {
            let net = w
                .get(2)
                .ok_or("usage : route delete <réseau> [mask <masque>]")?;
            if *net == "0.0.0.0" {
                return Ok(vec![
                    quiet(format!("gateway {d} none")),
                    Action::Say(" OK!".into()),
                ]);
            }
            let mask = if w.get(3).is_some_and(|m| m.eq_ignore_ascii_case("mask")) {
                w.get(4).copied().unwrap_or("255.255.255.255")
            } else {
                "255.255.255.255"
            };
            let c = cidr(net, mask).map_err(|_| "réseau ou masque invalide".to_string())?;
            Ok(vec![
                quiet(format!("route del {d} {c}")),
                Action::Say(" OK!".into()),
            ])
        }
        _ => Err("usage : route print | route add … | route delete …".into()),
    }
}

/// `netsh interface ip set address|dns …` (formes `name="eth0"` ou `"eth0"`).
fn netsh(w: &[&str], dev: &DeviceCtx) -> Res {
    let usage = "usage : netsh interface ip set address \"<carte>\" static <ip> <masque> [passerelle] | dhcp\n        netsh interface ip set dns \"<carte>\" static <serveur> | dhcp";
    let lower: Vec<String> = w.iter().map(|s| s.to_ascii_lowercase()).collect();
    let l: Vec<&str> = lower.iter().map(String::as_str).collect();
    let (Some(&"interface"), Some(&ipv), Some(&"set")) = (l.get(1), l.get(2), l.get(3)) else {
        return Err(usage.into());
    };
    if ipv != "ip" && ipv != "ipv4" {
        return Err(usage.into());
    }
    let what = l.get(4).copied().ok_or(usage)?;
    let raw_name = w.get(5).ok_or(usage)?;
    let raw_name = raw_name.strip_prefix("name=").unwrap_or(raw_name);
    let name = dev
        .resolve_iface(raw_name)
        .map(|i| i.name.clone())
        .ok_or_else(|| format!("Le nom de l'interface « {raw_name} » n'est pas valide."))?;
    let d = &dev.name;
    let source = l.get(6).map(|s| s.trim_start_matches("source="));
    match (what, source) {
        ("address", Some("dhcp")) => Ok(vec![netiv(format!("ip {d} {name} dhcp"))]),
        ("address", Some("static")) => {
            let (Some(a), Some(m)) = (w.get(7), w.get(8)) else {
                return Err(usage.into());
            };
            let p = mask_to_prefix(m).ok_or("masque invalide")?;
            let mut actions = vec![quiet(format!(
                "ip {d} {name} {}/{p}",
                ip(a).map_err(|_| "adresse invalide".to_string())?
            ))];
            if let Some(gw) = w.get(9) {
                actions.push(quiet(format!(
                    "gateway {d} {}",
                    ip(gw).map_err(|_| "passerelle invalide".to_string())?
                )));
            }
            Ok(actions)
        }
        ("dns" | "dnsservers" | "dnsserver", Some("dhcp")) => {
            Ok(vec![quiet(format!("dns {d} none"))])
        }
        ("dns" | "dnsservers" | "dnsserver", Some("static")) => {
            let s = w.get(7).ok_or(usage)?;
            Ok(vec![quiet(format!(
                "dns {d} {}",
                ip(s).map_err(|_| "adresse de serveur DNS invalide".to_string())?
            ))])
        }
        _ => Err(usage.into()),
    }
}

fn linux_ip(w: &[&str], dev: &DeviceCtx, iface: &dyn Fn(&str) -> Result<String, String>) -> Res {
    let d = &dev.name;
    let obj = w.get(1).map(|s| s.to_ascii_lowercase());
    let verb = w.get(2).map(|s| s.to_ascii_lowercase());
    let dev_arg = || -> Result<String, String> {
        let pos = w
            .iter()
            .position(|x| *x == "dev")
            .ok_or("argument « dev <interface> » manquant")?;
        iface(
            w.get(pos + 1)
                .ok_or("argument « dev <interface> » manquant")?,
        )
    };
    match obj.as_deref() {
        Some("a" | "addr" | "address") => match verb.as_deref() {
            None | Some("show" | "list" | "ls") => Ok(vec![Action::Host(HostShow::IpAddr)]),
            Some("add" | "replace") => {
                let net = w.get(3).ok_or("usage : ip addr add <ip/préfixe> dev <interface>")?;
                if !net.contains('/') {
                    return Err("préciser le préfixe : ip addr add 192.168.1.10/24 dev eth0".into());
                }
                Ok(vec![quiet(format!("ip {d} {} {net}", dev_arg()?))])
            }
            Some("del" | "delete" | "flush") => Ok(vec![quiet(format!("ip {d} {} none", dev_arg()?))]),
            _ => Err("usage : ip addr [show | add <ip/préfixe> dev <if> | del … dev <if> | flush dev <if>]".into()),
        },
        Some("link" | "l") => match (verb.as_deref(), w.get(3), w.get(4)) {
            (None | Some("show"), _, _) => Ok(vec![Action::Host(HostShow::IpAddr)]),
            (Some("set"), Some(name), Some(_)) => {
                let name = if *name == "dev" { iface(w.get(4).ok_or("interface manquante")?)? } else { iface(name)? };
                let state = if w.contains(&"up") {
                    "up"
                } else if w.contains(&"down") {
                    "down"
                } else {
                    return Err("préciser up ou down".into());
                };
                Ok(vec![quiet(format!("iface {d} {name} {state}"))])
            }
            _ => Err("usage : ip link set <interface> up|down".into()),
        },
        Some("r" | "route" | "ro") => match verb.as_deref() {
            None | Some("show" | "list") => Ok(vec![Action::Host(HostShow::IpRoute)]),
            Some("add" | "replace") => {
                let dest = w.get(3).ok_or("usage : ip route add <réseau>|default via <passerelle>")?;
                let pos = w.iter().position(|x| *x == "via").ok_or("préciser « via <passerelle> »")?;
                let gw = ip(w.get(pos + 1).ok_or("passerelle manquante")?).map_err(|_| "passerelle invalide".to_string())?;
                Ok(vec![quiet(if *dest == "default" || *dest == "0.0.0.0/0" { format!("gateway {d} {gw}") } else { format!("route add {d} {dest} via {gw}") })])
            }
            Some("del" | "delete") => {
                let dest = w.get(3).ok_or("usage : ip route del <réseau>|default")?;
                Ok(vec![quiet(if *dest == "default" || *dest == "0.0.0.0/0" { format!("gateway {d} none") } else { format!("route del {d} {dest}") })])
            }
            _ => Err("usage : ip route [show | add … via … | del …]".into()),
        },
        Some("n" | "neigh" | "neighbor" | "neighbour") => Ok(vec![Action::Host(HostShow::Neigh)]),
        _ => Err("usage : ip addr | ip link | ip route | ip neigh".into()),
    }
}

/// `http://intranet.pme:8080/x` → (`intranet.pme`, `8080`).
fn parse_url(url: &str) -> Result<(String, u16), String> {
    let (scheme, rest) = url.split_once("://").unwrap_or(("http", url));
    let default = match scheme {
        "http" => 80,
        "https" => 443,
        _ => return Err("seules les adresses http:// et https:// sont simulées".into()),
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    match authority.rsplit_once(':') {
        Some((h, p)) => Ok((
            h.to_string(),
            p.parse().map_err(|_| "port invalide".to_string())?,
        )),
        None if !authority.is_empty() => Ok((authority.to_string(), default)),
        None => Err("adresse incomplète".into()),
    }
}

/// Découpe une ligne comme un shell simple (guillemets doubles acceptés).
fn shell_words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut has = false;
    for c in line.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                has = true;
            }
            c if c.is_whitespace() && !quoted => {
                if has {
                    out.push(std::mem::take(&mut cur));
                    has = false;
                }
            }
            c => {
                cur.push(c);
                has = true;
            }
        }
    }
    if has {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn router() -> DeviceCtx {
        DeviceCtx {
            name: "R1".into(),
            is_switch: false,
            ifaces: ["eth0", "eth1"]
                .iter()
                .map(|n| IfaceCtx {
                    name: (*n).into(),
                    dhcp: false,
                    port: None,
                })
                .collect(),
        }
    }

    fn switch() -> DeviceCtx {
        DeviceCtx {
            name: "SW1".into(),
            is_switch: true,
            ifaces: ["p1", "p2", "p3"]
                .iter()
                .map(|n| IfaceCtx {
                    name: (*n).into(),
                    dhcp: false,
                    port: Some(PortMode::Access { vlan: 10 }),
                })
                .collect(),
        }
    }

    fn pc(dhcp: bool) -> DeviceCtx {
        DeviceCtx {
            name: "PC1".into(),
            is_switch: false,
            ifaces: vec![IfaceCtx {
                name: "eth0".into(),
                dhcp,
                port: None,
            }],
        }
    }

    fn lines(actions: Res) -> Vec<String> {
        actions
            .unwrap()
            .into_iter()
            .filter_map(|a| match a {
                Action::Netiv { line, .. } => Some(line),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn masks_and_aliases() {
        assert_eq!(mask_to_prefix("255.255.255.0"), Some(24));
        assert_eq!(mask_to_prefix("255.255.255.252"), Some(30));
        assert_eq!(mask_to_prefix("0.0.0.0"), Some(0));
        assert_eq!(mask_to_prefix("255.0.255.0"), None);
        let r = router();
        assert_eq!(r.resolve_iface("Gi0/0").unwrap().name, "eth0");
        assert_eq!(r.resolve_iface("GigabitEthernet0/1").unwrap().name, "eth1");
        assert_eq!(r.resolve_iface("eth1").unwrap().name, "eth1");
        assert!(r.resolve_iface("Gi0/9").is_none());
        let s = switch();
        assert_eq!(
            s.resolve_iface("fa0/1").unwrap().name,
            "p1",
            "switch : Gi0/1 = premier port"
        );
        assert!(s.resolve_iface("gi0/0").is_none());
        assert!(s.resolve_iface("serial0/1").is_none());
    }

    #[test]
    fn modes_and_abbreviations() {
        let r = router();
        assert_eq!(
            ios(&IosMode::User, "en", &r),
            Ok(vec![Action::Mode(IosMode::Exec)])
        );
        assert!(
            ios(&IosMode::User, "conf t", &r).is_err(),
            "configure : mode privilégié seulement"
        );
        assert!(matches!(
            ios(&IosMode::Exec, "conf t", &r).unwrap().last(),
            Some(Action::Mode(IosMode::Config))
        ));
        assert_eq!(
            ios(&IosMode::Config, "int gi0/1", &r),
            Ok(vec![Action::Mode(IosMode::Interface("eth1".into()))])
        );
        assert_eq!(
            ios(&IosMode::Config, "interface GigabitEthernet 0/1", &r),
            Ok(vec![Action::Mode(IosMode::Interface("eth1".into()))])
        );
        assert_eq!(
            ios(&IosMode::Interface("eth1".into()), "exit", &r),
            Ok(vec![Action::Mode(IosMode::Config)])
        );
        assert_eq!(
            ios(&IosMode::Interface("eth1".into()), "end", &r),
            Ok(vec![Action::Mode(IosMode::Exec)])
        );
        assert_eq!(
            ios(&IosMode::Exec, "sh ip int br", &r),
            Ok(vec![Action::Ios(IosShow::IpInterfaceBrief)])
        );
        assert_eq!(
            ios(&IosMode::Exec, "show run", &r),
            Ok(vec![Action::Ios(IosShow::RunningConfig)])
        );
        assert!(
            ios(&IosMode::Exec, "e", &r)
                .unwrap_err()
                .contains("Ambiguous"),
            "e : enable, end ou exit"
        );
        assert!(
            ios(&IosMode::Exec, "frobnicate", &r)
                .unwrap_err()
                .contains("Invalid input")
        );
        assert_eq!(
            ios(&IosMode::Interface("eth0".into()), "do sh ip route", &r),
            Ok(vec![Action::Ios(IosShow::IpRoute)])
        );
    }

    #[test]
    fn router_configuration() {
        let r = router();
        let i = IosMode::Interface("eth1".into());
        assert_eq!(
            lines(ios(&i, "ip address 10.0.0.1 255.255.255.0", &r)),
            ["ip R1 eth1 10.0.0.1/24"]
        );
        assert_eq!(lines(ios(&i, "no shut", &r)), ["iface R1 eth1 up"]);
        assert_eq!(lines(ios(&i, "shutdown", &r)), ["iface R1 eth1 down"]);
        assert_eq!(
            lines(ios(&i, "ip helper-address 192.168.30.10", &r)),
            ["dhcp relay R1 eth1 192.168.30.10"]
        );
        assert_eq!(lines(ios(&i, "no ip address", &r)), ["ip R1 eth1 none"]);
        assert!(ios(&i, "ip address 10.0.0.1 255.0.255.0", &r).is_err());
        assert_eq!(
            lines(ios(
                &IosMode::Config,
                "ip route 10.20.0.0 255.255.0.0 10.0.0.2",
                &r
            )),
            ["route add R1 10.20.0.0/16 via 10.0.0.2"]
        );
        assert_eq!(
            lines(ios(
                &IosMode::Config,
                "ip route 0.0.0.0 0.0.0.0 203.0.113.1",
                &r
            )),
            ["gateway R1 203.0.113.1"]
        );
        assert_eq!(
            lines(ios(
                &IosMode::Config,
                "no ip route 10.20.0.0 255.255.0.0",
                &r
            )),
            ["route del R1 10.20.0.0/16"]
        );
        assert!(
            ios(&i, "switchport mode access", &r).is_err(),
            "switchport : switches seulement"
        );
    }

    #[test]
    fn switch_configuration() {
        let s = switch();
        let i = IosMode::Interface("p2".into());
        assert_eq!(
            lines(ios(&i, "switchport access vlan 20", &s)),
            ["port SW1 p2 access 20"]
        );
        assert_eq!(
            lines(ios(&i, "sw mode trunk", &s)),
            ["port SW1 p2 trunk native 10"]
        );
        assert_eq!(
            lines(ios(&i, "switchport trunk allowed vlan 10,20,30-31", &s)),
            ["port SW1 p2 trunk 10,20,30,31 native 10"]
        );
        assert_eq!(
            lines(ios(&i, "switchport mode access", &s)),
            Vec::<String>::new(),
            "déjà en accès"
        );
        assert!(
            ios(&i, "ip address 1.2.3.4 255.0.0.0", &s).is_err(),
            "pas d'IP sur un switch NETIV"
        );
        assert_eq!(
            ios(&IosMode::Config, "vlan 20", &s),
            Ok(vec![Action::Mode(IosMode::Vlan(20))])
        );
        assert_eq!(ios(&IosMode::Vlan(20), "name COMPTA", &s), Ok(vec![]));
        assert_eq!(
            ios(&IosMode::Exec, "sh vlan br", &s),
            Ok(vec![Action::Ios(IosShow::VlanBrief)])
        );
        assert!(ios(&IosMode::Exec, "show ip route", &s).is_err());
    }

    #[test]
    fn windows_commands() {
        let p = pc(true);
        assert_eq!(
            host("ipconfig /all", &p),
            Ok(vec![Action::Host(HostShow::Ipconfig { all: true })])
        );
        assert_eq!(lines(host("ipconfig /renew", &p)), ["dhcp PC1 renew eth0"]);
        assert_eq!(
            lines(host("ipconfig /release", &p)),
            ["dhcp PC1 release eth0"]
        );
        assert!(
            host("ipconfig /renew", &pc(false))
                .unwrap_err()
                .contains("aucune carte"),
            "comme Windows : rien en DHCP"
        );
        assert_eq!(
            lines(host("route add 0.0.0.0 mask 0.0.0.0 192.168.1.1", &p)),
            ["gateway PC1 192.168.1.1"]
        );
        assert_eq!(
            lines(host("route add 10.0.0.0 mask 255.0.0.0 192.168.1.254", &p)),
            ["route add PC1 10.0.0.0/8 via 192.168.1.254"]
        );
        assert_eq!(
            lines(host(r#"netsh interface ip set address "eth0" dhcp"#, &p)),
            ["ip PC1 eth0 dhcp"]
        );
        assert_eq!(
            lines(host(
                "netsh interface ipv4 set address name=eth0 static 192.168.1.20 255.255.255.0 192.168.1.1",
                &p
            )),
            ["ip PC1 eth0 192.168.1.20/24", "gateway PC1 192.168.1.1"]
        );
        assert_eq!(
            lines(host(r#"netsh interface ip set dns "eth0" dhcp"#, &p)),
            ["dns PC1 none"]
        );
        assert_eq!(
            lines(host(
                r#"netsh interface ip set dns "eth0" static 192.168.30.10"#,
                &p
            )),
            ["dns PC1 192.168.30.10"]
        );
        assert_eq!(
            lines(host("ping 192.168.1.1", &p)),
            ["ping PC1 192.168.1.1 4"]
        );
        assert_eq!(
            lines(host("ping -n 2 intranet.pme", &p)),
            ["ping PC1 intranet.pme 2"]
        );
        assert!(
            host("format c:", &p)
                .unwrap_err()
                .contains("commande introuvable")
        );
    }

    #[test]
    fn linux_commands() {
        let p = pc(false);
        assert_eq!(host("ip a", &p), Ok(vec![Action::Host(HostShow::IpAddr)]));
        assert_eq!(
            lines(host("ip addr add 192.168.1.20/24 dev eth0", &p)),
            ["ip PC1 eth0 192.168.1.20/24"]
        );
        assert_eq!(
            lines(host("ip addr flush dev eth0", &p)),
            ["ip PC1 eth0 none"]
        );
        assert_eq!(
            lines(host("ip link set eth0 up", &p)),
            ["iface PC1 eth0 up"]
        );
        assert_eq!(
            lines(host("ip link set dev eth0 down", &p)),
            ["iface PC1 eth0 down"]
        );
        assert_eq!(
            lines(host("ip route add default via 192.168.1.1", &p)),
            ["gateway PC1 192.168.1.1"]
        );
        assert_eq!(
            lines(host("ip route add 10.0.0.0/8 via 192.168.1.254", &p)),
            ["route add PC1 10.0.0.0/8 via 192.168.1.254"]
        );
        assert_eq!(
            lines(host("dhclient eth0", &p)),
            ["ip PC1 eth0 dhcp"],
            "interface fixe : passe en DHCP"
        );
        assert_eq!(
            lines(host("dhclient eth0", &pc(true))),
            ["dhcp PC1 renew eth0"]
        );
        assert_eq!(
            lines(host("dhclient -r", &pc(true))),
            ["dhcp PC1 release eth0"]
        );
        assert_eq!(
            lines(host("resolvectl dns eth0 192.168.30.10", &p)),
            ["dns PC1 192.168.30.10"]
        );
        assert_eq!(
            lines(host("dig intranet.pme", &p)),
            ["nslookup PC1 intranet.pme"]
        );
        assert_eq!(
            lines(host("curl http://intranet.pme:8080/index.html", &p)),
            ["curl PC1 intranet.pme 8080"]
        );
        assert_eq!(
            lines(host("curl https://appli.pme", &p)),
            ["curl PC1 appli.pme 443"]
        );
        assert_eq!(
            lines(host("ping -c 3 10.0.0.1", &p)),
            ["ping PC1 10.0.0.1 3"]
        );
        assert!(
            host("ip addr add 10.0.0.1 dev eth0", &p).is_err(),
            "préfixe obligatoire"
        );
    }

    #[test]
    fn shell_words_handle_quotes() {
        assert_eq!(
            shell_words(r#"netsh interface ip set address "Ethernet 0" dhcp"#),
            [
                "netsh",
                "interface",
                "ip",
                "set",
                "address",
                "Ethernet 0",
                "dhcp"
            ]
        );
    }
}

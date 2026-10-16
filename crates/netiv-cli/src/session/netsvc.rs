//! Services réseau : DHCP (client, serveur, relais) et DNS (client, serveur,
//! résolution de noms).

use netiv_core::IfaceId;
use netiv_core::dhcp::{DEFAULT_LEASE_S, DhcpEvent, DhcpPool};
use netiv_core::dns::{AfterResolve, DnsOutcome, is_valid_name};

use super::*;

fn parse_ip(s: &str) -> Result<Ipv4Addr, CliError> {
    s.parse()
        .map_err(|_| CliError(format!("adresse IPv4 invalide : « {s} »")))
}

fn on_off(s: &str) -> Result<bool, CliError> {
    match s {
        "on" | "start" => Ok(true),
        "off" | "stop" => Ok(false),
        other => err(format!("« {other} » : attendu on ou off")),
    }
}

/// Lignes DHCP (obtention, échec, libération) produites depuis `start`.
pub(super) fn print_dhcp_report(
    sim: &Simulation,
    start: usize,
    out: &mut dyn Write,
) -> io::Result<bool> {
    let mut any = false;
    for e in sim.trace().since(start) {
        if let TraceKind::Dhcp { event, .. } = &e.kind {
            if matches!(
                event,
                DhcpEvent::ClientBound(_)
                    | DhcpEvent::ClientFailed { .. }
                    | DhcpEvent::ClientReleased { .. }
            ) {
                any = true;
                let tid = e
                    .trace_id
                    .map_or(String::new(), |t| format!("   [parcours {t}]"));
                writeln!(
                    out,
                    "{} : {}{tid}",
                    render::device_name(sim, e.device),
                    render::describe(sim, e)
                )?;
            }
        }
    }
    Ok(any)
}

/// Résultat de la résolution de nom produite depuis `start`, affiché.
pub(super) fn print_dns_report(
    sim: &Simulation,
    start: usize,
    out: &mut dyn Write,
) -> io::Result<Option<DnsOutcome>> {
    for e in sim.trace().since(start) {
        if let TraceKind::DnsResult {
            name,
            outcome,
            cached,
        } = &e.kind
        {
            let tid = e
                .trace_id
                .map_or(String::new(), |t| format!("   [parcours {t}]"));
            match outcome {
                DnsOutcome::Resolved(ip) => {
                    let src = if *cached { " (cache)" } else { "" };
                    writeln!(out, "DNS : {name} = {ip}{src}{tid}")?;
                }
                other => writeln!(out, "DNS : {name} → {other}{tid}")?,
            }
            return Ok(Some(outcome.clone()));
        }
    }
    Ok(None)
}

/// Une destination : une adresse IP, ou un nom à résoudre.
pub(super) enum Target {
    Ip(Ipv4Addr),
    Name(String),
}

pub(super) fn parse_target(s: &str) -> Result<Target, CliError> {
    if let Ok(ip) = s.parse::<Ipv4Addr>() {
        return Ok(Target::Ip(ip));
    }
    if is_valid_name(s) {
        return Ok(Target::Name(s.to_string()));
    }
    err(format!(
        "« {s} » n'est ni une adresse IPv4 ni un nom de domaine valide"
    ))
}

impl Session {
    /// Résout un nom, lance la suite, exécute tout et affiche le résultat DNS.
    /// Rend l'adresse trouvée.
    pub(super) fn resolve_then(
        &mut self,
        dev: DeviceId,
        name: &str,
        then: AfterResolve,
        out: &mut dyn Write,
    ) -> Result<(usize, Option<Ipv4Addr>), CliError> {
        let sim = self.sim_mut()?;
        let start = sim.trace().len();
        sim.command(
            dev,
            Command::Resolve {
                name: name.to_string(),
                then,
            },
        )?;
        sim.run_until_idle()?;
        let outcome = print_dns_report(sim, start, out)?;
        let ip = match outcome {
            Some(DnsOutcome::Resolved(ip)) => Some(ip),
            Some(_) => {
                writeln!(
                    out,
                    "Pour comprendre : « diagnose {} {name} »",
                    render::device_name(sim, dev)
                )?;
                None
            }
            None => None,
        };
        Ok((start, ip))
    }

    pub(super) fn cmd_nslookup(
        &mut self,
        args: &[&str],
        out: &mut dyn Write,
    ) -> Result<(), CliError> {
        let [dev, name] = args else {
            return err("usage : nslookup <équipement> <nom>");
        };
        if !is_valid_name(name) {
            return err(format!("nom de domaine invalide : « {name} »"));
        }
        let id = self.device(dev)?;
        self.resolve_then(id, name, AfterResolve::Nothing, out)?;
        Ok(())
    }

    pub(super) fn cmd_dhcp(&mut self, args: &[&str], out: &mut dyn Write) -> Result<(), CliError> {
        let usage = "usage : dhcp <équipement> [renew|release] [interface] | dhcp server <équipement> on|off | \
                     dhcp pool add <équipement> <réseau> <première>-<dernière> [gw <ip>] [dns <ip>] [lease <s>] | \
                     dhcp pool del <équipement> <réseau> | dhcp relay <routeur> <interface> <ip|none>";
        match args {
            ["server", dev, state] => {
                let id = self.device(dev)?;
                let on = on_off(state)?;
                let sim = self.sim_mut()?;
                sim.set_dhcp_enabled(id, on)?;
                last_event(sim, out)?;
                if on {
                    writeln!(
                        out,
                        "Les clients sans adresse doivent la redemander : « dhcp <poste> renew »."
                    )?;
                }
                Ok(())
            }
            ["pool", "add", dev, network, range, opts @ ..] => {
                let id = self.device(dev)?;
                let network: Ipv4Net = network.parse()?;
                let (a, b) = range.split_once('-').ok_or_else(|| {
                    CliError(format!("plage « {range} » : attendu première-dernière"))
                })?;
                let (first, last) = (parse_ip(a)?, parse_ip(b)?);
                if u32::from(first) > u32::from(last)
                    || !network.contains(first)
                    || !network.contains(last)
                {
                    return err(format!(
                        "plage {range} invalide pour le réseau {}",
                        network.trunc()
                    ));
                }
                let mut pool = DhcpPool {
                    network: network.trunc(),
                    first,
                    last,
                    gateway: None,
                    dns: None,
                    lease_s: DEFAULT_LEASE_S,
                };
                let mut it = opts.iter();
                while let Some(k) = it.next() {
                    let v = it
                        .next()
                        .ok_or_else(|| CliError(format!("{k} attend une valeur")))?;
                    match *k {
                        "gw" | "gateway" => pool.gateway = Some(parse_ip(v)?),
                        "dns" => pool.dns = Some(parse_ip(v)?),
                        "lease" => {
                            pool.lease_s = v
                                .parse()
                                .map_err(|_| CliError(format!("durée invalide : « {v} »")))?;
                        }
                        other => {
                            return err(format!("option inconnue : « {other} » (gw, dns, lease)"));
                        }
                    }
                }
                let sim = self.sim_mut()?;
                let mut pools = dhcp_pools(sim, id);
                pools.retain(|p| p.network != pool.network);
                pools.push(pool);
                sim.set_dhcp_pools(id, pools)?;
                last_event(sim, out)
            }
            ["pool", "del", dev, network] => {
                let id = self.device(dev)?;
                let network: Ipv4Net = network.parse()?;
                let sim = self.sim_mut()?;
                let mut pools = dhcp_pools(sim, id);
                let before = pools.len();
                pools.retain(|p| p.network != network.trunc());
                if pools.len() == before {
                    return err(format!("aucune plage pour {} sur {dev}", network.trunc()));
                }
                sim.set_dhcp_pools(id, pools)?;
                last_event(sim, out)
            }
            ["relay", dev, iface, target] => {
                let id = self.device(dev)?;
                let server = if *target == "none" {
                    None
                } else {
                    Some(parse_ip(target)?)
                };
                let sim = self.sim_mut()?;
                let iface = sim.iface_id(id, iface)?;
                sim.set_dhcp_relay(id, iface, server)?;
                last_event(sim, out)
            }
            [dev, rest @ ..] if !["server", "pool", "relay"].contains(dev) => {
                let id = self.device(dev)?;
                let (action, iface) = match rest {
                    [] => ("renew", None),
                    [a] if *a == "renew" || *a == "release" => (*a, None),
                    [a, i] if *a == "renew" || *a == "release" => (*a, Some(*i)),
                    _ => return err(usage),
                };
                let sim = self.sim_mut()?;
                let iface = iface.map(|i| sim.iface_id(id, i)).transpose()?;
                let has_dhcp = match &sim.device(id).map(|d| &d.behavior) {
                    Some(netiv_core::device::Behavior::Ip(n)) => match iface {
                        Some(i) => n.dhcp_ifaces.contains(&i),
                        None => !n.dhcp_ifaces.is_empty(),
                    },
                    _ => false,
                };
                if !has_dhcp {
                    return err(format!(
                        "{dev} n'a pas d'interface configurée par DHCP (« ip {dev} <interface> dhcp »)"
                    ));
                }
                let start = sim.trace().len();
                let cmd = if action == "renew" {
                    Command::DhcpRenew {
                        iface,
                        fresh: false,
                    }
                } else {
                    Command::DhcpRelease { iface }
                };
                sim.command(id, cmd)?;
                sim.run_until_idle()?;
                if !print_dhcp_report(sim, start, out)? {
                    writeln!(out, "(rien à faire)")?;
                }
                Ok(())
            }
            _ => err(usage),
        }
    }

    pub(super) fn cmd_dns(&mut self, args: &[&str], out: &mut dyn Write) -> Result<(), CliError> {
        let usage = "usage : dns <équipement> <ip|none> | dns server <équipement> on|off | \
                     dns record add <équipement> <nom> <ip> | dns record del <équipement> <nom>";
        match args {
            ["server", dev, state] => {
                let id = self.device(dev)?;
                let on = on_off(state)?;
                let sim = self.sim_mut()?;
                sim.set_dns_enabled(id, on)?;
                last_event(sim, out)
            }
            ["record", "add", dev, name, ip] => {
                let id = self.device(dev)?;
                let ip = parse_ip(ip)?;
                let sim = self.sim_mut()?;
                sim.set_dns_record(id, name, Some(ip))?;
                last_event(sim, out)
            }
            ["record", "del", dev, name] => {
                let id = self.device(dev)?;
                let sim = self.sim_mut()?;
                sim.set_dns_record(id, name, None)?;
                last_event(sim, out)
            }
            [dev, value] if !["server", "record"].contains(dev) => {
                let id = self.device(dev)?;
                let server = if *value == "none" {
                    None
                } else {
                    Some(parse_ip(value)?)
                };
                let sim = self.sim_mut()?;
                sim.set_dns(id, server)?;
                last_event(sim, out)
            }
            _ => err(usage),
        }
    }

    /// `ip <équipement> <interface> dhcp` : passe l'interface en DHCP et
    /// demande une adresse.
    pub(super) fn ip_to_dhcp(
        &mut self,
        id: DeviceId,
        iface: IfaceId,
        out: &mut dyn Write,
    ) -> Result<(), CliError> {
        let sim = self.sim_mut()?;
        sim.set_dhcp_client(id, iface, true)?;
        last_event(sim, out)?;
        let start = sim.trace().len();
        sim.command(
            id,
            Command::DhcpRenew {
                iface: Some(iface),
                fresh: true,
            },
        )?;
        sim.run_until_idle()?;
        print_dhcp_report(sim, start, out)?;
        Ok(())
    }
}

/// Plages actuelles du serveur DHCP d'un équipement.
fn dhcp_pools(sim: &Simulation, id: DeviceId) -> Vec<DhcpPool> {
    match sim.device(id).map(|d| &d.behavior) {
        Some(netiv_core::device::Behavior::Ip(n)) => n
            .dhcp_server
            .as_ref()
            .map(|s| s.pools.clone())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

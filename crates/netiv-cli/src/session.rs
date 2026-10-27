//! Une session NETIV : une simulation chargée et les commandes pour la piloter.

use std::collections::HashMap;
use std::fmt;
use std::io::{self, Write};
use std::net::Ipv4Addr;

mod build;
pub use build::EditError;
mod edu;
pub use edu::ChallengeInfo;
mod intel;
mod lab;
mod netsvc;
mod security;

use netiv_chaos::{Chaos, Difficulty, Filter, Flow as ChaosFlow};
use netiv_core::acl::{AclAction, AclRule, RuleProto};
use netiv_core::packet::L4Proto;
use netiv_core::render;
use netiv_core::service::Service;
use netiv_core::switch::PortMode;
use netiv_core::trace::TraceKind;
use netiv_core::{
    Command, DeviceId, Ipv4Net, MacAddr, SimTime, Simulation, StaticRoute, TraceEvent, TraceId,
};
use netiv_diagnostics::{Layer, Request, RootCause, diagnose, hints_for};
use netiv_education::profile::ProfileError;
use netiv_education::{ChallengeRun, Profile, RangeRun};
use netiv_topology::{Layout, Loaded, load_any};

use crate::storage::{Storage, default_storage, load_profile};

/// Erreur affichable à l'utilisateur.
#[derive(Debug)]
pub struct CliError(pub String);

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<E: std::error::Error> From<E> for CliError {
    fn from(e: E) -> Self {
        CliError(e.to_string())
    }
}

fn err<T>(msg: impl Into<String>) -> Result<T, CliError> {
    Err(CliError(msg.into()))
}

/// Ce que la session doit faire après une commande.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Flow {
    /// Continuer.
    Continue,
    /// Quitter.
    Quit,
}

const HELP: &str = "\
COMMANDES

  Topologie
    load <fichier.yaml>            charge un réseau
    devices                        liste les équipements
    show ifaces|arp|route <equip>  interfaces, cache ARP, table de routage
    show mac <switch>              table d'adresses MAC d'un switch
    show vlan <switch>             mode des ports (access / trunk)
    show acl|services <equip>      règles de filtrage / services à l'écoute
    show dhcp|dns <equip>          client, relais et serveur DHCP / serveur DNS, zone

  Simulation
    ping <equip> <ip|nom> [n] [--step]  envoie n pings (1 par défaut) ; un nom est d'abord résolu par DNS
                                   (--step : programme seulement, avance avec step/run)
    step [n]                       exécute n événements (1 par défaut) en les détaillant
    run                            exécute tous les événements en attente
    curl <equip> <ip|nom> [port]   connexion TCP + requête (port 80 par défaut)
    udp <equip> <ip|nom> <port> [txt]  datagramme UDP, attend une réponse
    nslookup <equip> <nom>         résout un nom auprès du serveur DNS de l'équipement
    time                           date simulée et événements en attente

  Observer
    log [n|all]                    derniers événements de trace (20 par défaut)
    trace <id>                     parcours complet d'une transaction (Packet Journey)
    inspect <n°>                   détail couche par couche d'une trame (Packet Microscope)
    explain <n°>                   pourquoi ce choix de route ? (Explain Engine)

  Apprendre
    learn                          parcours d'apprentissage (learn <parcours>, learn read <n°>, learn next)
    challenge list [filtre]        challenges (filtre : thème ou facile|moyen|difficile)
    challenge start <id|fichier>   lance un challenge : une panne cachée à trouver
    challenge show|check|finish|giveup|causes|export
    answer <équipement> <cause>    donne ton diagnostic (+40 si juste)
    dashboard | badges             progression : niveau, XP, thèmes, badges
    course new|show|validate|export|start|grade   mode enseignant (voir « course »)

  Partager
    lab list                       labs de la bibliothèque (dossier « labs »)
    lab open <nom>                 charge un lab de la bibliothèque
    lab export <fichier.netiv> [titre]   enregistre le réseau actuel comme lab partageable
    leaderboard                    classement local des profils

  Analyser et générer
    what-if <scénario>             impact d'un changement : « R1 goes down », « block tcp/443 »,
                                   « disconnect SW2 », « remove route 10.10.0.0/16 », « cut link R1:eth1 »
    health                         score de santé (connectivité, adressage, routage, segmentation…)
    docs md|json|yaml [fichier]    documentation générée du réseau
    topology generate --type lan|enterprise|campus|datacenter|isp|hybrid|cyber-range
             --routers N --hosts M [--seed S] [--save fichier.yaml]   génère et charge un réseau
    topology autoconfig            complète l'adressage : IP, passerelles, routes statiques
    challenge generate --topic T --difficulty facile|moyen|difficile [--seed N] [--save f.netiv]
    tutor <question>               explique une notion (gratuit) ou guide sur le challenge en cours

  Sécurité
    security analyze               défauts de conception (HIGH / MEDIUM / LOW) avec recommandations
    capture save <fichier.nlv>     enregistre la trace actuelle dans une capture
    forensic analyze <f.nlv|live> [--packets]   chronologie + anomalies (balayage, rebond, balise...)
    range list | start <id> | show | check | finish | giveup   cyber range : enquêter puis corriger

  Casser et diagnostiquer
    chaos start <equip> <ip> [tcp|udp <port>] [--layer l1|l2|l3|services|security]
                [--difficulty easy|medium|hard] [--seed N] [--visible]
                                   injecte une panne qui touche ce flux (cachée par défaut)
    chaos undo | reveal | list     répare tout / révèle les pannes / liste le catalogue
    diagnose <equip> <ip|nom> [tcp|udp <port>]   cause racine d'un échec (DHCP et DNS compris)
    hint                           indice suivant (du plus vague à la solution)
    baseline save|clear            mémorise l'état « sain » pour comparer avec « diagnose »

  Configurer / casser
    ip <equip> <iface> <cidr|none|dhcp>    change l'adresse IP (dhcp : la demander à un serveur)
    gateway <equip> <ip|none>              change la passerelle par défaut
    route add <equip> <cidr> via <ip>      ajoute une route statique
    route del <equip> <cidr>               supprime les routes vers ce réseau
    iface <equip> <iface> up|down          active / désactive une interface
    cable <equip> <iface> cut|restore      coupe / rétablit le câble de cette interface
    loss <equip> <iface> <0..1>            taux de perte du câble (ex. 0.3 = 30 %)
    latency <equip> <iface> <ms>           latence du câble
    forwarding <equip> on|off              transfert de paquets d'un routeur
    arp <equip> static <ip> <mac> | arp <equip> del <ip>   entrée ARP statique
    port <switch> <iface> access <vlan>    port d'accès
    port <switch> <iface> trunk [vlans] [native N]   port trunk (tous les VLAN si vide)
    service add <equip> tcp|udp <port> <nom>   démarre un service (http, ssh, echo...)
    service del <equip> tcp|udp <port>         arrête un service
    acl add <equip> allow|deny [tcp|udp|icmp|any] [src <cidr>] [dst <cidr>] [port <n|a-b>] [in <iface>]
    acl insert <equip> <n°> <mêmes options>    insère la règle en position n°
    acl del <equip> <n°>                       supprime la règle n°
    acl default <equip> allow|deny             politique par défaut
    acl stateful <equip> on|off                suivi de connexion

  DHCP et DNS
    dhcp <equip> [renew|release] [iface]       redemande / rend l'adresse obtenue par DHCP
    dhcp server <equip> on|off                 démarre / arrête le serveur DHCP
    dhcp pool add <equip> <réseau> <première>-<dernière> [gw <ip>] [dns <ip>] [lease <s>]
    dhcp pool del <equip> <réseau>             supprime une plage
    dhcp relay <routeur> <iface> <ip|none>     relaie les demandes DHCP de ce réseau vers un serveur
    dns <equip> <ip|none>                      serveur DNS utilisé par l'équipement
    dns server <equip> on|off                  démarre / arrête le serveur DNS
    dns record add <equip> <nom> <ip>          ajoute ou modifie un nom dans la zone
    dns record del <equip> <nom>               supprime un nom

  Plusieurs commandes sur une ligne : séparer par « ; ».
  help, quit
";

/// Une session : une simulation (éventuellement) chargée.
pub struct Session {
    sim: Option<Simulation>,
    name: String,
    chaos: Option<Chaos>,
    baseline: Option<Simulation>,
    last_cause: Option<RootCause>,
    hint_level: usize,
    profile: Profile,
    profile_path: Option<std::path::PathBuf>,
    labs_dir: std::path::PathBuf,
    run: Option<ChallengeRun>,
    range_run: Option<RangeRun>,
    clock: Box<dyn Fn() -> u64 + Send>,
    sandbox: bool,
    /// Où sont lus et écrits les fichiers (disque ou mémoire).
    storage: Box<dyn Storage>,
    /// Positions des équipements sur le schéma (interface web, labs partagés).
    layout: Layout,
}

impl Default for Session {
    fn default() -> Self {
        Session {
            sim: None,
            name: String::new(),
            chaos: None,
            baseline: None,
            last_cause: None,
            hint_level: 0,
            profile: Profile::new("invité"),
            profile_path: None,
            labs_dir: std::path::PathBuf::from("labs"),
            run: None,
            range_run: None,
            clock: Box::new(default_clock),
            sandbox: false,
            storage: default_storage(),
            layout: Layout::new(),
        }
    }
}

/// Horloge réglable à la main, partageable entre threads.
#[derive(Clone, Default, Debug)]
pub struct ManualClock(std::sync::Arc<std::sync::atomic::AtomicU64>);

impl ManualClock {
    /// Horloge réglée sur `start` secondes.
    pub fn new(start: u64) -> Self {
        ManualClock(std::sync::Arc::new(std::sync::atomic::AtomicU64::new(
            start,
        )))
    }

    /// Valeur actuelle.
    pub fn get(&self) -> u64 {
        self.0.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Règle l'horloge.
    pub fn set(&self, value: u64) {
        self.0.store(value, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Horloge par défaut : secondes écoulées depuis l'epoch Unix.
#[cfg(not(target_arch = "wasm32"))]
fn default_clock() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Horloge par défaut en WebAssembly : `SystemTime` n'y existe pas (il
/// panique), l'hôte doit fournir l'heure avec [`Session::set_clock`]. En
/// attendant, le temps est figé à 0 : rien ne panique, les chronomètres de
/// challenge restent simplement à zéro.
#[cfg(target_arch = "wasm32")]
fn default_clock() -> u64 {
    0
}

impl Session {
    /// Session sans topologie.
    pub fn new() -> Self {
        Self::default()
    }

    /// Session sans topologie dont les fichiers passent par `storage`.
    pub fn with_storage(storage: Box<dyn Storage>) -> Self {
        Session {
            storage,
            ..Session::default()
        }
    }

    /// Session rattachée à un profil sauvegardé dans `path` (stockage par
    /// défaut : le disque en natif).
    pub fn with_profile(name: &str, path: std::path::PathBuf) -> Result<Self, CliError> {
        Self::with_profile_in(default_storage(), name, path)
    }

    /// Session rattachée à un profil sauvegardé dans `path`, au sein de
    /// `storage`. Un profil absent est créé à la première sauvegarde.
    pub fn with_profile_in(
        storage: Box<dyn Storage>,
        name: &str,
        path: std::path::PathBuf,
    ) -> Result<Self, CliError> {
        let profile = load_profile(storage.as_ref(), &path, name)?;
        Ok(Session {
            profile,
            profile_path: Some(path),
            storage,
            ..Session::default()
        })
    }

    /// Remplace l'horloge (secondes) : permet de tester le chronométrage.
    pub fn set_clock(&mut self, clock: Box<dyn Fn() -> u64 + Send>) {
        self.clock = clock;
    }

    /// Dossier de la bibliothèque de labs (« labs » par défaut).
    pub fn set_labs_dir(&mut self, dir: std::path::PathBuf) {
        self.labs_dir = dir;
    }

    /// Branche une horloge manuelle (tests, démonstrations).
    pub fn set_manual_clock(&mut self, clock: &ManualClock) {
        let c = clock.clone();
        self.clock = Box::new(move || c.get());
    }

    /// Interdit tout accès aux fichiers de l'utilisateur (lecture comme
    /// écriture) : utilisé quand la session est pilotée depuis un navigateur.
    /// Les fichiers de profil, gérés par NETIV lui-même, restent écrits.
    #[must_use]
    pub fn sandboxed(mut self) -> Self {
        self.sandbox = true;
        self
    }

    /// La session est-elle en mode bac à sable ?
    pub fn is_sandboxed(&self) -> bool {
        self.sandbox
    }

    /// Relit le profil depuis le disque (plusieurs sessions peuvent partager
    /// le même profil).
    pub fn reload_profile(&mut self) {
        if let Some(path) = &self.profile_path {
            if let Ok(p) = load_profile(self.storage.as_ref(), path, &self.profile.name) {
                self.profile = p;
            }
        }
    }

    /// Écrit le profil à son emplacement (sans effet s'il n'en a pas).
    fn save_profile_file(&self) -> Result<(), CliError> {
        if let Some(path) = &self.profile_path {
            let text = self.profile.to_json()?;
            self.storage
                .write_atomic(path, &text)
                .map_err(|e| ProfileError::Io {
                    path: path.display().to_string(),
                    reason: e.to_string(),
                })?;
        }
        Ok(())
    }

    /// La simulation chargée, en lecture seule.
    pub fn simulation(&self) -> Option<&Simulation> {
        self.sim.as_ref()
    }

    fn fs_allowed(&self) -> Result<(), CliError> {
        if self.sandbox {
            return err("accès aux fichiers indisponible depuis l'interface web (utilise la CLI)");
        }
        Ok(())
    }

    fn read_text(&self, path: &str) -> Result<String, CliError> {
        self.fs_allowed()?;
        self.storage
            .read(std::path::Path::new(path))
            .map_err(|e| CliError(format!("lecture de « {path} » impossible : {e}")))
    }

    fn write_text(&self, path: &str, text: &str) -> Result<(), CliError> {
        self.fs_allowed()?;
        self.storage
            .write(std::path::Path::new(path), text)
            .map_err(|e| CliError(format!("écriture de « {path} » impossible : {e}")))
    }

    /// Le profil de l'élève.
    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    /// Nom du réseau chargé.
    pub fn network_name(&self) -> Option<&str> {
        self.sim.as_ref().map(|_| self.name.as_str())
    }

    /// Charge une topologie depuis du YAML et affiche les avertissements.
    pub fn load_yaml(&mut self, yaml: &str, out: &mut dyn Write) -> Result<(), CliError> {
        let (
            bundle,
            Loaded {
                name,
                sim,
                warnings,
                layout,
            },
        ) = load_any(yaml)?;
        writeln!(
            out,
            "Réseau « {name} » chargé : {} équipement(s), {} câble(s).",
            sim.devices().len(),
            sim.links().len()
        )?;
        for w in &warnings {
            writeln!(out, "  ⚠ {w}")?;
        }
        if let Some(b) = &bundle {
            writeln!(
                out,
                "  Lab « {} »{}",
                b.metadata.title,
                if b.metadata.author.is_empty() {
                    String::new()
                } else {
                    format!(" par {}", b.metadata.author)
                }
            )?;
            if b.challenge.is_some() {
                writeln!(
                    out,
                    "  Ce fichier contient aussi un challenge : « challenge start <fichier> » pour le jouer."
                )?;
            }
        }
        self.sim = Some(sim);
        self.name = name;
        self.layout = layout;
        self.chaos = None;
        self.baseline = None;
        self.last_cause = None;
        self.hint_level = 0;
        self.run = None;
        self.range_run = None;
        Ok(())
    }

    fn sim(&self) -> Result<&Simulation, CliError> {
        match &self.sim {
            Some(s) => Ok(s),
            None => err("aucune topologie chargée (utilise « load fichier.yaml »)"),
        }
    }

    fn sim_mut(&mut self) -> Result<&mut Simulation, CliError> {
        match &mut self.sim {
            Some(s) => Ok(s),
            None => err("aucune topologie chargée (utilise « load fichier.yaml »)"),
        }
    }

    fn device(&self, name: &str) -> Result<DeviceId, CliError> {
        let sim = self.sim()?;
        sim.device_id(name).or_else(|_| {
            let known: Vec<&str> = sim.devices().iter().map(|d| d.name.as_str()).collect();
            err(format!(
                "équipement inconnu : « {name} » (équipements : {})",
                known.join(", ")
            ))
        })
    }

    /// Exécute une ligne de commande.
    pub fn exec(&mut self, line: &str, out: &mut dyn Write) -> Result<Flow, CliError> {
        if line.contains(';') {
            for part in line.split(';') {
                if self.exec_one(part, out)? == Flow::Quit {
                    return Ok(Flow::Quit);
                }
            }
            return Ok(Flow::Continue);
        }
        self.exec_one(line, out)
    }

    fn exec_one(&mut self, line: &str, out: &mut dyn Write) -> Result<Flow, CliError> {
        let words: Vec<&str> = line.split_whitespace().collect();
        let Some((&cmd, args)) = words.split_first() else {
            return Ok(Flow::Continue);
        };
        match cmd {
            "help" | "?" => write!(out, "{HELP}")?,
            "quit" | "exit" | "q" => return Ok(Flow::Quit),
            "load" => {
                let [path] = args else {
                    return err("usage : load <fichier.yaml>");
                };
                let text = self.read_text(path)?;
                self.load_yaml(&text, out)?;
            }
            "devices" => write!(out, "{}", render::devices_table(self.sim()?))?,
            "show" => self.cmd_show(args, out)?,
            "ping" => self.cmd_ping(args, out)?,
            "curl" => self.cmd_curl(args, out)?,
            "udp" => self.cmd_udp(args, out)?,
            "port" => self.cmd_port(args, out)?,
            "service" => self.cmd_service(args, out)?,
            "acl" => self.cmd_acl(args, out)?,
            "step" => self.cmd_step(args, out)?,
            "run" => {
                let sim = self.sim_mut()?;
                let n = sim.run_until_idle()?;
                writeln!(
                    out,
                    "{n} événement(s) exécuté(s). Temps simulé : {}.",
                    sim.now()
                )?;
            }
            "time" => {
                let sim = self.sim()?;
                writeln!(
                    out,
                    "Temps simulé : {}. Événements en attente : {}.",
                    sim.now(),
                    sim.pending()
                )?;
            }
            "log" => self.cmd_log(args, out)?,
            "trace" => {
                let [id] = args else {
                    return err("usage : trace <id>   (l'id est affiché par ping)");
                };
                let id = parse_num(id)?;
                write!(out, "{}", render::journey(self.sim()?, TraceId(id))?)?;
            }
            "inspect" => {
                let [n] = args else {
                    return err(
                        "usage : inspect <n°>   (n° d'une trame, voir « trace » ou « log »)",
                    );
                };
                write!(out, "{}", render::inspect(self.sim()?, parse_num(n)?)?)?;
            }
            "explain" => {
                let [n] = args else {
                    return err("usage : explain <n°>   (n° d'une décision de route)");
                };
                write!(
                    out,
                    "{}",
                    render::explain_route(self.sim()?, parse_num(n)?)?
                )?;
            }
            "ip" => self.cmd_ip(args, out)?,
            "dhcp" => self.cmd_dhcp(args, out)?,
            "dns" => self.cmd_dns(args, out)?,
            "nslookup" => self.cmd_nslookup(args, out)?,
            "gateway" => self.cmd_gateway(args, out)?,
            "route" => self.cmd_route(args, out)?,
            "iface" => self.cmd_iface(args, out)?,
            "cable" => self.cmd_cable(args, out)?,
            "loss" => self.cmd_loss(args, out)?,
            "latency" => self.cmd_latency(args, out)?,
            "forwarding" => self.cmd_forwarding(args, out)?,
            "arp" => self.cmd_arp(args, out)?,
            "chaos" => {
                self.deny_during_challenge()?;
                self.cmd_chaos(args, out)?;
            }
            "diagnose" => {
                self.deny_during_challenge()?;
                self.cmd_diagnose(args, out)?;
            }
            "challenge" => self.cmd_challenge(args, out)?,
            "security" => self.cmd_security(args, out)?,
            "what-if" | "whatif" => self.cmd_what_if(args, out)?,
            "health" => {
                self.deny_during_challenge()?;
                self.cmd_health(out)?;
            }
            "docs" => self.cmd_docs(args, out)?,
            "topology" => self.cmd_topology(args, out)?,
            "tutor" => self.cmd_tutor(args, out)?,
            "lab" => self.cmd_lab(args, out)?,
            "leaderboard" | "classement" => self.cmd_leaderboard(out)?,
            "capture" => self.cmd_capture(args, out)?,
            "forensic" | "forensics" => self.cmd_forensic(args, out)?,
            "range" => self.cmd_range(args, out)?,
            "answer" => self.cmd_answer(args, out)?,
            "dashboard" | "profile" => self.cmd_dashboard(out)?,
            "badges" => self.cmd_badges(out)?,
            "learn" => self.cmd_learn(args, out)?,
            "course" => self.cmd_course(args, out)?,
            "hint" => self.cmd_hint(args, out)?,
            "baseline" => {
                self.deny_during_challenge()?;
                self.cmd_baseline(args, out)?;
            }
            other => return err(format!("commande inconnue : « {other} » (essaie « help »)")),
        }
        Ok(Flow::Continue)
    }

    // ------------------------------------------------------------------

    fn cmd_show(&self, args: &[&str], out: &mut dyn Write) -> Result<(), CliError> {
        let [what, dev] = args else {
            return err("usage : show ifaces|arp|route|mac <équipement>");
        };
        let id = self.device(dev)?;
        let sim = self.sim()?;
        let text = match *what {
            "ifaces" | "interfaces" => render::show_interfaces(sim, id)?,
            "arp" => render::show_arp(sim, id)?,
            "route" | "routes" => render::show_routes(sim, id)?,
            "vlan" | "vlans" => render::show_vlans(sim, id).map_err(|_| {
                CliError(format!(
                    "« {dev} » n'est pas un switch (show vlan ne s'applique qu'aux switches)"
                ))
            })?,
            "acl" => render::show_acl(sim, id)?,
            "services" | "service" => render::show_services(sim, id)?,
            "dhcp" => render::show_dhcp(sim, id)?,
            "dns" => render::show_dns(sim, id)?,
            "mac" => render::show_mac_table(sim, id).map_err(|_| {
                CliError(format!(
                    "« {dev} » n'est pas un switch (show mac ne s'applique qu'aux switches)"
                ))
            })?,
            other => {
                return err(format!(
                    "« show {other} » inconnu (ifaces, arp, route, mac, vlan, acl, services, dhcp, dns)"
                ));
            }
        };
        write!(out, "{text}")?;
        Ok(())
    }

    fn cmd_ping(&mut self, args: &[&str], out: &mut dyn Write) -> Result<(), CliError> {
        let step_only = args.contains(&"--step");
        let args: Vec<&str> = args.iter().copied().filter(|a| *a != "--step").collect();
        let (dev, dst, count) = match args.as_slice() {
            [d, ip] => (*d, *ip, 1u16),
            [d, ip, n] => (
                *d,
                *ip,
                n.parse::<u16>()
                    .map_err(|_| CliError(format!("nombre de pings invalide : « {n} »")))?,
            ),
            _ => return err("usage : ping <équipement> <ip|nom> [n] [--step]"),
        };
        let id = self.device(dev)?;
        let dst = match netsvc::parse_target(dst)? {
            netsvc::Target::Ip(ip) => ip,
            netsvc::Target::Name(name) => {
                if step_only {
                    let sim = self.sim_mut()?;
                    let then = netiv_core::dns::AfterResolve::Ping { count };
                    sim.command(id, Command::Resolve { name, then })?;
                    writeln!(
                        out,
                        "Résolution puis ping programmés. Avance avec « step » ou « run »."
                    )?;
                    return Ok(());
                }
                let then = netiv_core::dns::AfterResolve::Ping { count };
                let (start, ip) = self.resolve_then(id, &name, then, out)?;
                if let Some(ip) = ip {
                    print_ping_report(self.sim()?, dev, ip, start, out)?;
                }
                return Ok(());
            }
        };
        let sim = self.sim_mut()?;
        let start = sim.trace().len();
        sim.ping(id, dst, count)?;
        if step_only {
            writeln!(
                out,
                "Ping programmé. Avance avec « step » (un événement) ou « run » (tout)."
            )?;
            return Ok(());
        }
        sim.run_until_idle()?;
        print_ping_report(sim, dev, dst, start, out)?;
        Ok(())
    }

    fn cmd_curl(&mut self, args: &[&str], out: &mut dyn Write) -> Result<(), CliError> {
        let (dev, dst, port) = match args {
            [d, ip] => (*d, *ip, 80u16),
            [d, ip, p] => (*d, *ip, parse_port(p)?),
            _ => return err("usage : curl <équipement> <ip|nom> [port]"),
        };
        let id = self.device(dev)?;
        let dst = match netsvc::parse_target(dst)? {
            netsvc::Target::Ip(ip) => ip,
            netsvc::Target::Name(name) => {
                let then = netiv_core::dns::AfterResolve::Curl { port };
                let (start, ip) = self.resolve_then(id, &name, then, out)?;
                if ip.is_some() {
                    print_l4_report(self.sim()?, start, out)?;
                }
                return Ok(());
            }
        };
        let sim = self.sim_mut()?;
        let start = sim.trace().len();
        sim.command(id, Command::Curl { dst, port })?;
        sim.run_until_idle()?;
        print_l4_report(sim, start, out)?;
        Ok(())
    }

    fn cmd_udp(&mut self, args: &[&str], out: &mut dyn Write) -> Result<(), CliError> {
        let [dev, dst, port, text @ ..] = args else {
            return err("usage : udp <équipement> <ip|nom> <port> [texte]");
        };
        let port = parse_port(port)?;
        let data = if text.is_empty() {
            "hello".to_string()
        } else {
            text.join(" ")
        };
        let id = self.device(dev)?;
        let dst = match netsvc::parse_target(dst)? {
            netsvc::Target::Ip(ip) => ip,
            netsvc::Target::Name(name) => {
                let then = netiv_core::dns::AfterResolve::Udp { port, data };
                let (start, ip) = self.resolve_then(id, &name, then, out)?;
                if ip.is_some() {
                    print_l4_report(self.sim()?, start, out)?;
                }
                return Ok(());
            }
        };
        let sim = self.sim_mut()?;
        let start = sim.trace().len();
        sim.command(id, Command::SendUdp { dst, port, data })?;
        sim.run_until_idle()?;
        print_l4_report(sim, start, out)?;
        Ok(())
    }

    fn cmd_port(&mut self, args: &[&str], out: &mut dyn Write) -> Result<(), CliError> {
        let usage = "usage : port <switch> <iface> access <vlan>  |  port <switch> <iface> trunk [vlans] [native N]";
        let [dev, iface, kind, rest @ ..] = args else {
            return err(usage);
        };
        let mode = match (*kind, rest) {
            ("access", [v]) => PortMode::Access {
                vlan: parse_vlan(v)?,
            },
            ("trunk", rest) => {
                let (list, native) = match rest.iter().position(|w| *w == "native") {
                    Some(i) => {
                        let n = rest.get(i + 1).ok_or_else(|| {
                            CliError("« native » attend un numéro de VLAN".into())
                        })?;
                        (&rest[..i], parse_vlan(n)?)
                    }
                    None => (rest, 1),
                };
                let mut allowed = Vec::new();
                for chunk in list {
                    for v in chunk.split(',').filter(|v| !v.is_empty()) {
                        allowed.push(parse_vlan(v)?);
                    }
                }
                PortMode::Trunk { allowed, native }
            }
            _ => return err(usage),
        };
        let id = self.device(dev)?;
        let sim = self.sim_mut()?;
        let iface = sim.iface_id(id, iface)?;
        sim.set_port_mode(id, iface, mode)?;
        last_event(sim, out)
    }

    fn cmd_service(&mut self, args: &[&str], out: &mut dyn Write) -> Result<(), CliError> {
        match args {
            ["add", dev, proto, port, name] => {
                let id = self.device(dev)?;
                let service = Service {
                    proto: parse_l4(proto)?,
                    port: parse_port(port)?,
                    name: (*name).to_string(),
                };
                let sim = self.sim_mut()?;
                sim.add_service(id, service)?;
                last_event(sim, out)
            }
            ["del", dev, proto, port] => {
                let id = self.device(dev)?;
                let (proto, port) = (parse_l4(proto)?, parse_port(port)?);
                let sim = self.sim_mut()?;
                let n = sim.remove_service(id, proto, port)?;
                if n == 0 {
                    writeln!(out, "Aucun service {proto}/{port} sur {dev}.")?;
                    return Ok(());
                }
                last_event(sim, out)
            }
            _ => err(
                "usage : service add <équipement> tcp|udp <port> <nom>  |  service del <équipement> tcp|udp <port>",
            ),
        }
    }

    fn cmd_acl(&mut self, args: &[&str], out: &mut dyn Write) -> Result<(), CliError> {
        match args {
            ["add", dev, rest @ ..] => {
                let id = self.device(dev)?;
                let rule = self.parse_rule(id, rest)?;
                let sim = self.sim_mut()?;
                sim.acl_add_rule(id, rule)?;
                last_event(sim, out)
            }
            ["insert", dev, pos, rest @ ..] => {
                let id = self.device(dev)?;
                let pos = parse_rule_number(pos)?;
                let rule = self.parse_rule(id, rest)?;
                let sim = self.sim_mut()?;
                sim.acl_insert_rule(id, pos, rule)?;
                last_event(sim, out)
            }
            ["del", dev, n] => {
                let id = self.device(dev)?;
                let n = parse_rule_number(n)?;
                let sim = self.sim_mut()?;
                sim.acl_remove_rule(id, n)?;
                last_event(sim, out)
            }
            ["default", dev, action] => {
                let id = self.device(dev)?;
                let a = parse_action(action)?;
                let sim = self.sim_mut()?;
                sim.acl_set_default(id, a)?;
                last_event(sim, out)
            }
            ["stateful", dev, state] => {
                let id = self.device(dev)?;
                let on = match *state {
                    "on" => true,
                    "off" => false,
                    other => return err(format!("« {other} » : attendu on ou off")),
                };
                let sim = self.sim_mut()?;
                sim.acl_set_stateful(id, on)?;
                last_event(sim, out)
            }
            _ => {
                err("usage : acl add|insert|del|default|stateful <équipement> ... (voir « help »)")
            }
        }
    }

    /// `allow|deny [proto] [src <cidr>] [dst <cidr>] [port <n|a-b>] [in <iface>]`
    fn parse_rule(&self, dev: DeviceId, words: &[&str]) -> Result<AclRule, CliError> {
        let usage = "règle : allow|deny [tcp|udp|icmp|any] [src <cidr>] [dst <cidr>] [port <n|a-b>] [in <iface>]";
        let mut it = words.iter().copied().peekable();
        let action = parse_action(it.next().ok_or_else(|| CliError(usage.into()))?)?;
        let proto = match it.peek().copied() {
            Some("tcp") => RuleProto::Tcp,
            Some("udp") => RuleProto::Udp,
            Some("icmp") => RuleProto::Icmp,
            _ => RuleProto::Any,
        };
        if matches!(it.peek().copied(), Some("tcp" | "udp" | "icmp" | "any")) {
            it.next();
        }
        let mut rule = AclRule::new(action, proto);
        while let Some(key) = it.next() {
            let value = it
                .next()
                .ok_or_else(|| CliError(format!("« {key} » attend une valeur ({usage})")))?;
            match key {
                "src" => rule.src = Some(value.parse::<Ipv4Net>()?),
                "dst" => rule.dst = Some(value.parse::<Ipv4Net>()?),
                "port" => {
                    let (a, b) = match value.split_once('-') {
                        Some((a, b)) => (parse_port(a)?, parse_port(b)?),
                        None => {
                            let p = parse_port(value)?;
                            (p, p)
                        }
                    };
                    if a > b {
                        return err(format!("plage de ports invalide : {value}"));
                    }
                    rule.port_from = Some(a);
                    rule.port_to = Some(b);
                }
                "in" => rule.in_iface = Some(self.sim()?.iface_id(dev, value)?),
                other => return err(format!("option inconnue : « {other} » ({usage})")),
            }
        }
        Ok(rule)
    }

    fn cmd_latency(&mut self, args: &[&str], out: &mut dyn Write) -> Result<(), CliError> {
        let [dev, iface, ms] = args else {
            return err("usage : latency <équipement> <interface> <ms>");
        };
        let ms: u64 = ms
            .parse()
            .map_err(|_| CliError(format!("durée invalide : « {ms} » (millisecondes)")))?;
        let id = self.device(dev)?;
        let sim = self.sim_mut()?;
        let iface_id = sim.iface_id(id, iface)?;
        let Some(link) = sim
            .device(id)
            .and_then(|d| d.iface(iface_id))
            .and_then(|i| i.link)
        else {
            return err(format!("{dev}/{iface} n'est reliée à aucun câble"));
        };
        sim.set_link_latency(link, SimTime::from_millis(ms))?;
        last_event(sim, out)
    }

    fn cmd_forwarding(&mut self, args: &[&str], out: &mut dyn Write) -> Result<(), CliError> {
        let [dev, state] = args else {
            return err("usage : forwarding <équipement> on|off");
        };
        let on = match *state {
            "on" => true,
            "off" => false,
            other => return err(format!("« {other} » : attendu on ou off")),
        };
        let id = self.device(dev)?;
        let sim = self.sim_mut()?;
        sim.set_forwarding(id, on)?;
        last_event(sim, out)
    }

    fn cmd_arp(&mut self, args: &[&str], out: &mut dyn Write) -> Result<(), CliError> {
        let usage = "usage : arp <équipement> static <ip> <mac>  |  arp <équipement> del <ip>";
        match args {
            [dev, "static", ip, mac] => {
                let id = self.device(dev)?;
                let ip: Ipv4Addr = ip
                    .parse()
                    .map_err(|_| CliError(format!("adresse IPv4 invalide : « {ip} »")))?;
                let mac: MacAddr = mac.parse()?;
                let sim = self.sim_mut()?;
                sim.set_static_arp(id, ip, Some(mac))?;
                last_event(sim, out)
            }
            [dev, "del", ip] => {
                let id = self.device(dev)?;
                let ip: Ipv4Addr = ip
                    .parse()
                    .map_err(|_| CliError(format!("adresse IPv4 invalide : « {ip} »")))?;
                let sim = self.sim_mut()?;
                sim.set_static_arp(id, ip, None)?;
                last_event(sim, out)
            }
            _ => err(usage),
        }
    }

    // ------------------------------------------------------------------
    // Chaos et diagnostic
    // ------------------------------------------------------------------

    /// `<équipement> <ip> [tcp|udp <port>]`
    fn parse_flow(&self, words: &[&str]) -> Result<ChaosFlow, CliError> {
        let usage = "attendu : <équipement> <ip> [tcp|udp <port>]";
        let (dev, ip, rest) = match words {
            [d, ip, rest @ ..] => (*d, *ip, rest),
            _ => return err(usage),
        };
        let src = self.device(dev)?;
        let dst: Ipv4Addr = ip
            .parse()
            .map_err(|_| CliError(format!("adresse IPv4 invalide : « {ip} »")))?;
        let service = match rest {
            [] => None,
            [proto, port] => Some((parse_l4(proto)?, parse_port(port)?)),
            _ => return err(usage),
        };
        Ok(ChaosFlow { src, dst, service })
    }

    fn cmd_chaos(&mut self, args: &[&str], out: &mut dyn Write) -> Result<(), CliError> {
        match args {
            ["list"] => {
                writeln!(
                    out,
                    "{:<18} {:<8} {:<20} DESCRIPTION",
                    "ID", "NIVEAU", "COUCHE"
                )?;
                for t in netiv_chaos::catalog() {
                    writeln!(
                        out,
                        "{:<18} {:<8} {:<20} {}",
                        t.id,
                        t.difficulty.slug(),
                        t.kind.layer().to_string(),
                        t.summary
                    )?;
                }
                Ok(())
            }
            ["undo"] => {
                let Some(chaos) = &mut self.chaos else {
                    return err("aucune panne n'a été injectée par « chaos start »");
                };
                let sim = self
                    .sim
                    .as_mut()
                    .ok_or_else(|| CliError("aucune topologie chargée".into()))?;
                let n = chaos.undo_all(sim, false);
                writeln!(out, "{n} panne(s) réparée(s).")?;
                Ok(())
            }
            ["reveal"] => {
                let Some(chaos) = &self.chaos else {
                    return err("aucune panne n'a été injectée par « chaos start »");
                };
                let sim = self.sim()?;
                writeln!(out, "Pannes injectées :")?;
                for (i, inj) in chaos.injected.iter().enumerate() {
                    let active = sim
                        .faults()
                        .iter()
                        .find(|f| f.id == inj.fault_id)
                        .is_some_and(|f| f.active);
                    writeln!(
                        out,
                        "  {}. [{}] {} — {}{}",
                        i + 1,
                        inj.difficulty.slug(),
                        inj.template,
                        inj.description,
                        if active { "" } else { "  (réparée)" }
                    )?;
                }
                Ok(())
            }
            ["start", rest @ ..] => self.chaos_start(rest, out),
            _ => err(
                "usage : chaos start <équipement> <ip> [tcp|udp <port>] [options] | chaos undo | chaos reveal | chaos list",
            ),
        }
    }

    fn chaos_start(&mut self, args: &[&str], out: &mut dyn Write) -> Result<(), CliError> {
        // Sépare les options (--layer X, ...) du flux.
        let mut flow_words: Vec<&str> = Vec::new();
        let mut filter = Filter::default();
        let mut seed: Option<u64> = None;
        let mut visible = false;
        let mut it = args.iter().copied();
        while let Some(w) = it.next() {
            match w {
                "--visible" => visible = true,
                "--layer" => {
                    let v = it
                        .next()
                        .ok_or_else(|| CliError("--layer attend une valeur".into()))?;
                    filter.layer = Some(parse_layer(v)?);
                }
                "--difficulty" => {
                    let v = it
                        .next()
                        .ok_or_else(|| CliError("--difficulty attend une valeur".into()))?;
                    filter.difficulty = Some(Difficulty::from_slug(v).ok_or_else(|| {
                        CliError(format!(
                            "difficulté inconnue : « {v} » (easy, medium, hard)"
                        ))
                    })?);
                }
                "--seed" => {
                    let v = it
                        .next()
                        .ok_or_else(|| CliError("--seed attend un nombre".into()))?;
                    seed = Some(
                        v.parse()
                            .map_err(|_| CliError(format!("graine invalide : « {v} »")))?,
                    );
                }
                other => flow_words.push(other),
            }
        }
        let flow = self.parse_flow(&flow_words)?;

        // Réutilise le moteur en cours s'il observe le même flux : sa copie saine
        // reste la référence même après plusieurs pannes.
        let reuse = self.chaos.as_ref().is_some_and(|c| c.flow == flow);
        if self.chaos.is_some() && !reuse {
            return err(
                "un autre flux est déjà observé : « chaos undo » puis relance, ou garde le même flux",
            );
        }
        let mut fresh = if reuse {
            None
        } else {
            Some(Chaos::new(self.sim()?, flow).map_err(|e| CliError(e.to_string()))?)
        };
        let seed = seed.unwrap_or_else(|| {
            self.sim.as_ref().map_or(0, |s| s.seed())
                ^ self.chaos.as_ref().map_or(0, |c| c.injected.len() as u64)
        });
        let Some(sim) = self.sim.as_mut() else {
            return err("aucune topologie chargée");
        };
        let chaos = match fresh.as_mut() {
            Some(c) => c,
            None => match self.chaos.as_mut() {
                Some(c) => c,
                None => return err("aucun moteur de panne actif"),
            },
        };
        let inj = chaos
            .start(sim, &filter, seed, visible)
            .map_err(|e| CliError(e.to_string()))?;
        // L'injection a réussi : le moteur neuf devient le moteur de la session.
        if let Some(c) = fresh {
            self.chaos = Some(c);
        }
        self.last_cause = None;
        self.hint_level = 0;
        if visible {
            writeln!(out, "Panne injectée : {}", inj.description)?;
        } else {
            writeln!(
                out,
                "Une panne a été injectée (difficulté : {}). Le flux est peut-être cassé : à toi de trouver pourquoi.",
                inj.difficulty.slug()
            )?;
            writeln!(
                out,
                "Pistes : ping, curl, trace, diagnose, hint. « chaos reveal » révèle la panne."
            )?;
        }
        Ok(())
    }

    /// Copie saine disponible : celle du Chaos Engine, ou une sauvegarde manuelle.
    fn baseline_ref(&self) -> Option<&Simulation> {
        self.chaos
            .as_ref()
            .map(|c| &c.baseline)
            .or(self.baseline.as_ref())
    }

    fn cmd_diagnose(&mut self, args: &[&str], out: &mut dyn Write) -> Result<(), CliError> {
        if let [dev, target, rest @ ..] = args {
            if let Ok(netsvc::Target::Name(name)) = netsvc::parse_target(target) {
                let src = self.device(dev)?;
                let service = match rest {
                    [] => None,
                    [proto, port] => Some((parse_l4(proto)?, parse_port(port)?)),
                    _ => return err("usage : diagnose <équipement> <ip|nom> [tcp|udp <port>]"),
                };
                let sim = self.sim()?;
                let base = self.baseline_ref();
                let d = netiv_diagnostics::diagnose_name(sim, base, src, &name, service)?;
                write!(out, "{}", d.render(base.is_some(), true))?;
                self.hint_level = 0;
                self.last_cause = d.cause;
                return Ok(());
            }
        }
        let flow = if args.is_empty() {
            self.chaos.as_ref().map(|c| c.flow).ok_or_else(|| {
                CliError("usage : diagnose <équipement> <ip|nom> [tcp|udp <port>]".into())
            })?
        } else {
            self.parse_flow(args)?
        };
        let req = Request {
            src: flow.src,
            dst: flow.dst,
            service: flow.service,
        };
        let sim = self.sim()?;
        let base = self.baseline_ref();
        let d = diagnose(sim, base, &req)?;
        write!(out, "{}", d.render(base.is_some(), true))?;
        self.hint_level = 0;
        self.last_cause = d.cause;
        Ok(())
    }

    fn cmd_hint(&mut self, args: &[&str], out: &mut dyn Write) -> Result<(), CliError> {
        if !args.is_empty() {
            return err("usage : hint");
        }
        if let Some(run) = self.range_run.as_mut().filter(|r| !r.is_finished()) {
            let text = run.next_hint().map_err(|e| CliError(e.to_string()))?;
            writeln!(out, "{text}")?;
            writeln!(
                out,
                "(−10 points — indices utilisés : {})",
                run.hints_used()
            )?;
            return Ok(());
        }
        if let Some(run) = self.run.as_mut().filter(|r| !r.is_finished()) {
            let text = run.next_hint().map_err(|e| CliError(e.to_string()))?;
            writeln!(out, "{text}")?;
            writeln!(
                out,
                "(−10 points — indices utilisés : {}{})",
                run.hints_used(),
                if run.hints_left() > 0 {
                    String::new()
                } else {
                    " ; c'était le dernier niveau".to_string()
                }
            )?;
            return Ok(());
        }
        if self.last_cause.is_none() {
            let Some(flow) = self.chaos.as_ref().map(|c| c.flow) else {
                return err(
                    "lance d'abord « diagnose <équipement> <ip> » : les indices portent sur la cause trouvée",
                );
            };
            let req = Request {
                src: flow.src,
                dst: flow.dst,
                service: flow.service,
            };
            let d = diagnose(self.sim()?, self.baseline_ref(), &req)?;
            self.last_cause = d.cause;
        }
        let Some(cause) = &self.last_cause else {
            writeln!(out, "Aucun problème détecté : il n'y a rien à indiquer.")?;
            return Ok(());
        };
        let hints = hints_for(cause);
        let level = self.hint_level.min(hints.len() - 1);
        writeln!(out, "{}", hints[level])?;
        if level + 1 < hints.len() {
            writeln!(
                out,
                "(« hint » pour l'indice suivant — {} restant(s))",
                hints.len() - level - 1
            )?;
        }
        self.hint_level = (level + 1).min(hints.len());
        Ok(())
    }

    fn cmd_baseline(&mut self, args: &[&str], out: &mut dyn Write) -> Result<(), CliError> {
        match args {
            ["save"] => {
                let snap = self.sim()?.fresh_clone();
                self.baseline = Some(snap);
                writeln!(out, "État actuel mémorisé comme référence « saine ».")?;
                Ok(())
            }
            ["clear"] => {
                self.baseline = None;
                writeln!(out, "Référence effacée.")?;
                Ok(())
            }
            _ => err("usage : baseline save | baseline clear"),
        }
    }

    fn cmd_step(&mut self, args: &[&str], out: &mut dyn Write) -> Result<(), CliError> {
        let n: usize = match args {
            [] => 1,
            [n] => n
                .parse()
                .map_err(|_| CliError(format!("nombre invalide : « {n} »")))?,
            _ => return err("usage : step [n]"),
        };
        let sim = self.sim_mut()?;
        for _ in 0..n {
            let Some(fired) = sim.step() else {
                writeln!(out, "Plus rien à exécuter.")?;
                break;
            };
            let events: &[TraceEvent] = &sim.trace().events()[fired.trace.clone()];
            if events.is_empty() {
                writeln!(
                    out,
                    "[t={}] (événement interne sans effet visible)",
                    fired.time
                )?;
            }
            for e in events {
                writeln!(out, "{}", render::event_line(sim, e))?;
            }
        }
        Ok(())
    }

    fn cmd_log(&self, args: &[&str], out: &mut dyn Write) -> Result<(), CliError> {
        let sim = self.sim()?;
        let all = sim.trace().events();
        let shown = match args {
            [] => &all[all.len().saturating_sub(20)..],
            ["all"] => all,
            [n] => {
                let n: usize = n
                    .parse()
                    .map_err(|_| CliError(format!("nombre invalide : « {n} »")))?;
                &all[all.len().saturating_sub(n)..]
            }
            _ => return err("usage : log [n|all]"),
        };
        if shown.is_empty() {
            writeln!(out, "(trace vide)")?;
        }
        for e in shown {
            writeln!(out, "{}", render::event_line(sim, e))?;
        }
        Ok(())
    }

    fn cmd_ip(&mut self, args: &[&str], out: &mut dyn Write) -> Result<(), CliError> {
        let [dev, iface, value] = args else {
            return err("usage : ip <équipement> <interface> <cidr|none|dhcp>");
        };
        let id = self.device(dev)?;
        if *value == "dhcp" {
            let iface = self.sim()?.iface_id(id, iface)?;
            return self.ip_to_dhcp(id, iface, out);
        }
        // Une adresse fixe remplace le DHCP sur cette interface.
        {
            let sim = self.sim_mut()?;
            let iface_id = sim.iface_id(id, iface)?;
            let dhcp = matches!(sim.device(id).map(|d| &d.behavior),
                Some(netiv_core::device::Behavior::Ip(n)) if n.dhcp_ifaces.contains(&iface_id));
            if dhcp {
                sim.set_dhcp_client(id, iface_id, false)?;
            }
        }
        let ip = if *value == "none" {
            None
        } else {
            Some(value.parse::<Ipv4Net>()?)
        };
        let sim = self.sim_mut()?;
        let iface = sim.iface_id(id, iface)?;
        sim.set_ip(id, iface, ip)?;
        last_event(sim, out)
    }

    fn cmd_gateway(&mut self, args: &[&str], out: &mut dyn Write) -> Result<(), CliError> {
        let [dev, value] = args else {
            return err("usage : gateway <équipement> <ip|none>");
        };
        let id = self.device(dev)?;
        let gw = if *value == "none" {
            None
        } else {
            Some(
                value
                    .parse::<Ipv4Addr>()
                    .map_err(|_| CliError(format!("adresse IPv4 invalide : « {value} »")))?,
            )
        };
        let sim = self.sim_mut()?;
        sim.set_gateway(id, gw)?;
        last_event(sim, out)
    }

    fn cmd_route(&mut self, args: &[&str], out: &mut dyn Write) -> Result<(), CliError> {
        match args {
            ["add", dev, dest, "via", via] => {
                let id = self.device(dev)?;
                let dest = dest.parse::<Ipv4Net>()?;
                let via = via
                    .parse::<Ipv4Addr>()
                    .map_err(|_| CliError(format!("adresse IPv4 invalide : « {via} »")))?;
                let sim = self.sim_mut()?;
                sim.add_route(id, StaticRoute { dest, via })?;
                last_event(sim, out)
            }
            ["del", dev, dest] => {
                let id = self.device(dev)?;
                let dest = dest.parse::<Ipv4Net>()?;
                let sim = self.sim_mut()?;
                let n = sim.remove_route(id, dest)?;
                if n == 0 {
                    writeln!(
                        out,
                        "Aucune route statique vers {} à supprimer.",
                        dest.trunc()
                    )?;
                    return Ok(());
                }
                last_event(sim, out)
            }
            _ => err(
                "usage : route add <équipement> <cidr> via <ip>  |  route del <équipement> <cidr>",
            ),
        }
    }

    fn cmd_iface(&mut self, args: &[&str], out: &mut dyn Write) -> Result<(), CliError> {
        let [dev, iface, state] = args else {
            return err("usage : iface <équipement> <interface> up|down");
        };
        let up = match *state {
            "up" => true,
            "down" => false,
            other => return err(format!("état inconnu : « {other} » (up ou down)")),
        };
        let id = self.device(dev)?;
        let sim = self.sim_mut()?;
        let iface = sim.iface_id(id, iface)?;
        sim.set_iface_admin(id, iface, up)?;
        last_event(sim, out)
    }

    fn cmd_cable(&mut self, args: &[&str], out: &mut dyn Write) -> Result<(), CliError> {
        let [dev, iface, action] = args else {
            return err("usage : cable <équipement> <interface> cut|restore");
        };
        let up = match *action {
            "restore" => true,
            "cut" => false,
            other => return err(format!("action inconnue : « {other} » (cut ou restore)")),
        };
        let id = self.device(dev)?;
        let sim = self.sim_mut()?;
        let iface_id = sim.iface_id(id, iface)?;
        let Some(link) = sim
            .device(id)
            .and_then(|d| d.iface(iface_id))
            .and_then(|i| i.link)
        else {
            return err(format!("{dev}/{iface} n'est reliée à aucun câble"));
        };
        sim.set_link_up(link, up)?;
        last_event(sim, out)
    }

    fn cmd_loss(&mut self, args: &[&str], out: &mut dyn Write) -> Result<(), CliError> {
        let [dev, iface, value] = args else {
            return err("usage : loss <équipement> <interface> <0..1>");
        };
        let loss: f64 = value
            .parse()
            .map_err(|_| CliError(format!("taux invalide : « {value} »")))?;
        let id = self.device(dev)?;
        let sim = self.sim_mut()?;
        let iface_id = sim.iface_id(id, iface)?;
        let Some(link) = sim
            .device(id)
            .and_then(|d| d.iface(iface_id))
            .and_then(|i| i.link)
        else {
            return err(format!("{dev}/{iface} n'est reliée à aucun câble"));
        };
        sim.set_link_loss(link, loss)?;
        writeln!(
            out,
            "Perte du câble de {dev}/{iface} : {:.0} %.",
            loss * 100.0
        )?;
        Ok(())
    }
}

fn parse_layer(s: &str) -> Result<Layer, CliError> {
    match s.to_ascii_lowercase().as_str() {
        "l1" | "1" | "physique" => Ok(Layer::L1),
        "l2" | "2" | "liaison" => Ok(Layer::L2),
        "l3" | "3" | "reseau" | "réseau" => Ok(Layer::L3),
        "services" | "service" => Ok(Layer::Services),
        "security" | "securite" | "sécurité" | "filtrage" => Ok(Layer::Security),
        other => err(format!(
            "couche inconnue : « {other} » (l1, l2, l3, services, security)"
        )),
    }
}

fn parse_vlan(s: &str) -> Result<u16, CliError> {
    match s.parse::<u16>() {
        Ok(v) if (1..=4094).contains(&v) => Ok(v),
        _ => err(format!("VLAN invalide : « {s} » (attendu 1 à 4094)")),
    }
}

fn parse_port(s: &str) -> Result<u16, CliError> {
    match s.parse::<u16>() {
        Ok(p) if p > 0 => Ok(p),
        _ => err(format!("port invalide : « {s} » (attendu 1 à 65535)")),
    }
}

fn parse_l4(s: &str) -> Result<L4Proto, CliError> {
    match s {
        "tcp" => Ok(L4Proto::Tcp),
        "udp" => Ok(L4Proto::Udp),
        other => err(format!("protocole inconnu : « {other} » (tcp ou udp)")),
    }
}

fn parse_action(s: &str) -> Result<AclAction, CliError> {
    match s {
        "allow" | "permit" => Ok(AclAction::Allow),
        "deny" | "drop" => Ok(AclAction::Deny),
        other => err(format!("action inconnue : « {other} » (allow ou deny)")),
    }
}

/// Numéro de règle saisi par l'utilisateur (à partir de 1) → index.
fn parse_rule_number(s: &str) -> Result<usize, CliError> {
    match s.parse::<usize>() {
        Ok(n) if n >= 1 => Ok(n - 1),
        _ => err(format!(
            "numéro de règle invalide : « {s} » (à partir de 1)"
        )),
    }
}

/// Résultats TCP/UDP d'une commande, en langage clair.
fn print_l4_report(sim: &Simulation, start: usize, out: &mut dyn Write) -> io::Result<()> {
    let events = sim.trace().since(start);
    let mut first_tid = None;
    let mut printed = false;
    for e in events {
        first_tid = first_tid.or(e.trace_id);
        match &e.kind {
            TraceKind::CurlResult { dst, port, outcome } => {
                writeln!(out, "curl {dst}:{port} → {outcome}   [parcours {}]", tid(e))?;
                printed = true;
            }
            TraceKind::UdpResult { dst, port, outcome } => {
                writeln!(out, "udp {dst}:{port} → {outcome}   [parcours {}]", tid(e))?;
                printed = true;
            }
            _ => {}
        }
    }
    if !printed {
        writeln!(out, "(aucun résultat)")?;
    }
    if let Some(t) = first_tid {
        writeln!(out, "Pour voir le détail : « trace {} »", t.0)?;
    }
    Ok(())
}

fn tid(e: &TraceEvent) -> String {
    e.trace_id
        .map_or_else(|| "-".to_string(), |t| t.to_string())
}

fn parse_num(s: &str) -> Result<u64, CliError> {
    s.trim_start_matches('#')
        .parse()
        .map_err(|_| CliError(format!("numéro invalide : « {s} »")))
}

/// Affiche la description du dernier événement de trace (confirmation d'une
/// modification de configuration).
fn last_event(sim: &Simulation, out: &mut dyn Write) -> Result<(), CliError> {
    if let Some(e) = sim.trace().events().last() {
        if let TraceKind::ConfigChanged { description } = &e.kind {
            writeln!(out, "OK — {description}")?;
        }
    }
    Ok(())
}

fn print_ping_report(
    sim: &Simulation,
    from: &str,
    dst: Ipv4Addr,
    start: usize,
    out: &mut dyn Write,
) -> io::Result<()> {
    let events = sim.trace().since(start);
    let sent: Vec<&TraceEvent> = events
        .iter()
        .filter(|e| matches!(e.kind, TraceKind::PingSent { .. }))
        .collect();
    writeln!(out, "PING {dst} depuis {from} ({} requête(s))", sent.len())?;

    // Une seule passe : le résultat de chaque transaction (réponse, erreur ou
    // délai dépassé). Chercher dans la trace pour chaque ping serait
    // quadratique sur une longue série.
    let mut results: HashMap<TraceId, &TraceEvent> = HashMap::new();
    for e in events {
        if let (
            TraceKind::PingReply { .. }
            | TraceKind::PingError { .. }
            | TraceKind::PingTimeout { .. },
            Some(tid),
        ) = (&e.kind, e.trace_id)
        {
            results.entry(tid).or_insert(e);
        }
    }

    let (mut ok, mut last_failed_trace) = (0usize, None);
    for s in &sent {
        let (TraceKind::PingSent { seq, .. }, Some(tid)) = (&s.kind, s.trace_id) else {
            continue;
        };
        let result = results.get(&tid);
        let text = match result.map(|e| &e.kind) {
            Some(TraceKind::PingReply { from, ttl, rtt, .. }) => {
                ok += 1;
                format!("réponse de {from}  ttl={ttl}  temps={rtt}")
            }
            Some(TraceKind::PingError { from, error, .. }) => {
                last_failed_trace = Some(tid);
                format!("erreur de {from} : {error}")
            }
            Some(TraceKind::PingTimeout { .. }) => {
                last_failed_trace = Some(tid);
                "pas de réponse (délai dépassé)".to_string()
            }
            _ => {
                last_failed_trace = Some(tid);
                "résultat inconnu".to_string()
            }
        };
        writeln!(out, "  seq {seq}  {text}   [parcours {tid}]")?;
    }
    let total = sent.len();
    let loss = ((total - ok) * 100).checked_div(total).unwrap_or(0);
    writeln!(
        out,
        "--- {total} envoyé(s), {ok} reçu(s), {loss} % de perte"
    )?;
    if let Some(tid) = last_failed_trace {
        writeln!(out, "Pour comprendre l'échec : « trace {} »", tid.0)?;
    } else if let Some(first) = sent.first().and_then(|e| e.trace_id) {
        writeln!(out, "Pour voir le parcours : « trace {} »", first.0)?;
    }
    Ok(())
}

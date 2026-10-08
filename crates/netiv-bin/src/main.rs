//! Point d'entrée de la commande `netiv`.

use std::io::{self, BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use netiv_cli::{Flow, Session};

/// NETIV — Network Côte d'Ivoire : laboratoire réseau interactif.
#[derive(Parser)]
#[command(name = "netiv", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Ouvre une session interactive sur une topologie (ou exécute un script).
    Run {
        /// Fichier de topologie YAML (facultatif : un challenge apporte son réseau).
        topology: Option<PathBuf>,
        /// Exécute les commandes de ce fichier puis quitte.
        #[arg(long)]
        script: Option<PathBuf>,
        /// Nom du profil d'élève (progression sauvegardée). Défaut : $USER.
        #[arg(long)]
        profile: Option<String>,
        /// Dossier des profils. Défaut : ~/.local/share/netiv/profiles.
        #[arg(long)]
        profile_dir: Option<PathBuf>,
    },
    /// Vérifie une topologie sans la lancer.
    Validate {
        /// Fichier de topologie YAML.
        topology: PathBuf,
    },
    /// Génère une topologie (plan d'adressage et routage compris).
    Generate {
        /// Type : lan, enterprise, campus, datacenter, isp, hybrid, cyber-range.
        #[arg(long = "type")]
        kind: String,
        /// Nombre de routeurs.
        #[arg(long, default_value_t = 4)]
        routers: usize,
        /// Nombre de postes.
        #[arg(long, default_value_t = 10)]
        hosts: usize,
        /// Graine.
        #[arg(long, default_value_t = 1)]
        seed: u64,
        /// Fichier de sortie (sinon : sortie standard).
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Lance l'interface web et l'API HTTP, en local.
    Serve {
        /// Adresse d'écoute. Seules les adresses locales sont acceptées sans `--allow-remote`.
        #[arg(long, default_value = "127.0.0.1:8787")]
        bind: String,
        /// Nom du profil d'élève (progression sauvegardée). Défaut : $USER.
        #[arg(long)]
        profile: Option<String>,
        /// Dossier des profils. Défaut : ~/.local/share/netiv/profiles.
        #[arg(long)]
        profile_dir: Option<PathBuf>,
        /// Dossier de la bibliothèque de labs.
        #[arg(long, default_value = "labs")]
        labs_dir: PathBuf,
        /// Autorise l'écoute sur une adresse non locale (déconseillé : l'API n'a pas d'authentification).
        #[arg(long)]
        allow_remote: bool,
    },
    /// Documentation générée d'une topologie.
    Docs {
        /// Fichier de topologie YAML.
        topology: PathBuf,
        /// Format : md, json ou yaml.
        #[arg(long, default_value = "md")]
        format: String,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Validate { topology } => validate(&topology),
        Command::Generate {
            kind,
            routers,
            hosts,
            seed,
            out,
        } => generate_cmd(&kind, routers, hosts, seed, out.as_deref()),
        Command::Docs { topology, format } => docs_cmd(&topology, &format),
        Command::Serve {
            bind,
            profile,
            profile_dir,
            labs_dir,
            allow_remote,
        } => serve_cmd(&bind, profile, profile_dir, labs_dir, allow_remote),
        Command::Run {
            topology,
            script,
            profile,
            profile_dir,
        } => run(
            topology.as_deref(),
            script.as_deref(),
            profile.as_deref(),
            profile_dir,
        ),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("erreur : {msg}");
            ExitCode::FAILURE
        }
    }
}

fn read(path: &std::path::Path) -> Result<String, String> {
    std::fs::read_to_string(path)
        .map_err(|e| format!("lecture de « {} » impossible : {e}", path.display()))
}

fn validate(path: &std::path::Path) -> Result<(), String> {
    let mut session = Session::new();
    let mut out = io::stdout().lock();
    session
        .load_yaml(&read(path)?, &mut out)
        .map_err(|e| format!("topologie invalide :\n{e}"))
}

fn generate_cmd(
    kind: &str,
    routers: usize,
    hosts: usize,
    seed: u64,
    out: Option<&std::path::Path>,
) -> Result<(), String> {
    let kind: netiv_topology::generate::TopoKind = kind.parse()?;
    let def = netiv_topology::generate::generate(&netiv_topology::generate::GenSpec {
        kind,
        routers,
        hosts,
        seed,
    })?;
    let yaml = serde_yaml_ng::to_string(&def).map_err(|e| e.to_string())?;
    match out {
        Some(path) => std::fs::write(path, yaml)
            .map_err(|e| format!("écriture de « {} » impossible : {e}", path.display())),
        None => {
            print!("{yaml}");
            Ok(())
        }
    }
}

fn docs_cmd(topology: &std::path::Path, format: &str) -> Result<(), String> {
    let format: netiv_analysis::DocFormat = format.parse()?;
    let loaded = netiv_topology::load_yaml(&read(topology)?)
        .map_err(|e| format!("topologie invalide :\n{e}"))?;
    print!(
        "{}",
        netiv_analysis::generate_docs(&loaded.sim, &loaded.name, format)?
    );
    Ok(())
}

fn serve_cmd(
    bind: &str,
    profile: Option<String>,
    profile_dir: Option<PathBuf>,
    labs_dir: PathBuf,
    allow_remote: bool,
) -> Result<(), String> {
    let addr: std::net::SocketAddr = bind
        .parse()
        .map_err(|_| format!("adresse invalide : « {bind} » (attendu 127.0.0.1:8787)"))?;
    if !addr.ip().is_loopback() && !allow_remote {
        return Err(format!(
            "écoute sur {addr} refusée : l'API pilote un simulateur sans authentification et ne doit être joignable qu'en local. Utilise 127.0.0.1, ou --allow-remote si tu sais ce que tu fais."
        ));
    }
    let name = profile
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_else(|| "eleve".to_string());
    let cfg = netiv_api::ApiConfig {
        profile_name: name,
        profile_dir: profile_dir.or_else(default_profile_dir),
        labs_dir,
        // En écoute non locale, on ne peut pas deviner les noms d'hôte valides.
        allowed_hosts: if addr.ip().is_loopback() {
            netiv_api::ApiConfig::allowed_hosts_for(addr)
        } else {
            Vec::new()
        },
    };
    eprintln!("NETIV — interface web sur http://{addr}  (Ctrl-C pour arrêter)");
    if !addr.ip().is_loopback() {
        eprintln!("ATTENTION : écoute non locale et sans authentification.");
    }
    let runtime = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    runtime
        .block_on(netiv_api::serve(addr, cfg))
        .map_err(|e| format!("serveur arrêté : {e}"))
}

/// Dossier par défaut des profils.
fn default_profile_dir() -> Option<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
        return Some(PathBuf::from(xdg).join("netiv").join("profiles"));
    }
    std::env::var_os("HOME").map(|h| {
        PathBuf::from(h)
            .join(".local")
            .join("share")
            .join("netiv")
            .join("profiles")
    })
}

/// Nom de fichier sûr pour un profil.
fn profile_file(name: &str) -> String {
    let safe: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{safe}.json")
}

fn run(
    topology: Option<&std::path::Path>,
    script: Option<&std::path::Path>,
    profile: Option<&str>,
    profile_dir: Option<PathBuf>,
) -> Result<(), String> {
    let name = profile
        .map(str::to_string)
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_else(|| "eleve".to_string());
    let mut session = match profile_dir.or_else(default_profile_dir) {
        Some(dir) => Session::with_profile(&name, dir.join(profile_file(&name)))
            .map_err(|e| e.to_string())?,
        None => Session::new(),
    };
    let mut out = io::stdout().lock();
    if let Some(topology) = topology {
        session
            .load_yaml(&read(topology)?, &mut out)
            .map_err(|e| format!("topologie invalide :\n{e}"))?;
    } else {
        writeln!(
            out,
            "Aucun réseau chargé : « challenge list » pour commencer, ou « load fichier.yaml »."
        )
        .map_err(|e| e.to_string())?;
    }

    if let Some(script) = script {
        let text = read(script)?;
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            writeln!(out, "\nnetiv> {line}").map_err(|e| e.to_string())?;
            match session.exec(line, &mut out) {
                Ok(Flow::Continue) => {}
                Ok(Flow::Quit) => break,
                Err(e) => return Err(format!("{}, ligne {} : {e}", script.display(), n + 1)),
            }
        }
        return Ok(());
    }

    let interactive = io::stdin().is_terminal();
    if interactive {
        writeln!(
            out,
            "Tape « help » pour la liste des commandes, « quit » pour sortir."
        )
        .map_err(|e| e.to_string())?;
    }
    let stdin = io::stdin();
    loop {
        if interactive {
            write!(out, "netiv> ").map_err(|e| e.to_string())?;
            out.flush().map_err(|e| e.to_string())?;
        }
        let mut line = String::new();
        if stdin
            .lock()
            .read_line(&mut line)
            .map_err(|e| e.to_string())?
            == 0
        {
            break;
        }
        match session.exec(line.trim(), &mut out) {
            Ok(Flow::Continue) => {}
            Ok(Flow::Quit) => break,
            Err(e) => writeln!(out, "erreur : {e}").map_err(|e| e.to_string())?,
        }
    }
    Ok(())
}

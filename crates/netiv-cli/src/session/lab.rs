//! Bibliothèque de labs partageables (`.netiv`) et classement local.

use std::path::Path;

use netiv_education::profile::leaderboard;
use netiv_topology::{Bundle, BundleMeta};

use super::*;

/// Extensions reconnues dans la bibliothèque.
const EXTENSIONS: [&str; 3] = ["netiv", "yaml", "yml"];

impl Session {
    pub(super) fn cmd_lab(&mut self, args: &[&str], out: &mut dyn Write) -> Result<(), CliError> {
        match args {
            ["list"] => self.lab_list(out),
            ["open", name] => {
                self.fs_allowed()?;
                let dir = self.labs_dir.clone();
                let path = EXTENSIONS
                    .iter()
                    .map(|e| dir.join(format!("{name}.{e}")))
                    .find(|p| p.is_file())
                    .ok_or_else(|| {
                        CliError(format!("lab inconnu : « {name} » (voir « lab list »)"))
                    })?;
                let text = self.read_text(&path.display().to_string())?;
                self.load_yaml(&text, out)
            }
            ["export", file, title @ ..] => {
                let topology = self.current_def()?;
                let title = if title.is_empty() {
                    self.name.clone()
                } else {
                    // « lab export f.netiv "Mon titre" » : les guillemets ne font
                    // pas partie du titre.
                    title.join(" ").trim_matches('"').to_string()
                };
                let bundle = Bundle::new(
                    BundleMeta {
                        title,
                        description: String::new(),
                        author: self.profile.name.clone(),
                    },
                    topology,
                );
                self.write_text(file, &bundle.to_yaml()?)?;
                writeln!(
                    out,
                    "Lab enregistré dans {file}. Partage ce fichier : « load {file} » le rouvre."
                )?;
                Ok(())
            }
            _ => err("usage : lab list | lab open <nom> | lab export <fichier.netiv> [titre]"),
        }
    }

    fn lab_list(&self, out: &mut dyn Write) -> Result<(), CliError> {
        self.fs_allowed()?;
        let entries = std::fs::read_dir(&self.labs_dir).map_err(|e| {
            CliError(format!(
                "bibliothèque « {} » inaccessible : {e}",
                self.labs_dir.display()
            ))
        })?;
        let mut rows: Vec<(String, String, String)> = Vec::new();
        for e in entries.flatten() {
            let path = e.path();
            let ext = path.extension().and_then(|x| x.to_str()).unwrap_or("");
            if !EXTENSIONS.contains(&ext) {
                continue;
            }
            let name = path
                .file_stem()
                .and_then(|x| x.to_str())
                .unwrap_or("?")
                .to_string();
            let info = std::fs::read_to_string(&path)
                .ok()
                .and_then(|t| Bundle::from_yaml(&t).ok())
                .map_or_else(
                    || ("(topologie simple)".to_string(), String::new()),
                    |b| {
                        let kind = if b.challenge.is_some() {
                            "challenge"
                        } else {
                            "réseau"
                        };
                        (format!("{} [{kind}]", b.metadata.title), b.metadata.author)
                    },
                );
            rows.push((name, info.0, info.1));
        }
        rows.sort();
        if rows.is_empty() {
            writeln!(out, "(aucun lab dans « {} »)", self.labs_dir.display())?;
            return Ok(());
        }
        writeln!(out, "{:<22} {:<12} TITRE", "NOM", "AUTEUR")?;
        for (name, title, author) in rows {
            writeln!(out, "{name:<22} {author:<12} {title}")?;
        }
        Ok(())
    }

    pub(super) fn cmd_leaderboard(&self, out: &mut dyn Write) -> Result<(), CliError> {
        self.fs_allowed()?;
        let Some(dir) = self.profile_path.as_ref().and_then(|p| p.parent()) else {
            return err("aucun dossier de profils : lance « netiv run --profile-dir <dossier> »");
        };
        write_leaderboard(dir, out)
    }
}

/// Affiche le classement des profils d'un dossier.
pub(super) fn write_leaderboard(dir: &Path, out: &mut dyn Write) -> Result<(), CliError> {
    let (board, skipped) = leaderboard(dir)?;
    if board.is_empty() {
        writeln!(out, "(aucun profil dans « {} »)", dir.display())?;
        return Ok(());
    }
    writeln!(
        out,
        "{:<4} {:<16} {:>6}  {:<15} {:>9} {:>7}",
        "RANG", "PROFIL", "XP", "NIVEAU", "RÉUSSIS", "BADGES"
    )?;
    for (i, e) in board.iter().enumerate() {
        writeln!(
            out,
            "{:<4} {:<16} {:>6}  {:<15} {:>9} {:>7}",
            i + 1,
            e.name,
            e.xp,
            format!("{} {}", e.level, e.level_name),
            e.completed,
            e.badges
        )?;
    }
    if skipped > 0 {
        writeln!(
            out,
            "({skipped} fichier(s) ignoré(s) : ce ne sont pas des profils lisibles)"
        )?;
    }
    Ok(())
}

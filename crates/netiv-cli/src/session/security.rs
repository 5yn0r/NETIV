//! Commandes de sécurité : analyse de conception, captures, forensique, cyber range.

use std::path::Path;

use netiv_education::builtin;
use netiv_education::range::{RangeCheck, RangeRun, parse_behavior};
use netiv_education::run::{AnswerOutcome, RunError, Score};
use netiv_security::{Analysis, Capture, analyze, analyze_capture};

use super::*;

fn run_error(e: RunError) -> CliError {
    CliError(e.to_string())
}

fn difficulty_label(d: netiv_chaos::Difficulty) -> &'static str {
    match d {
        netiv_chaos::Difficulty::Easy => "facile",
        netiv_chaos::Difficulty::Medium => "moyen",
        netiv_chaos::Difficulty::Hard => "difficile",
    }
}

impl Session {
    pub(super) fn cmd_security(
        &mut self,
        args: &[&str],
        out: &mut dyn Write,
    ) -> Result<(), CliError> {
        match args {
            ["analyze"] => {
                let sim = self.sim()?;
                write!(out, "{}", analyze(sim).render())?;
                Ok(())
            }
            _ => err("usage : security analyze"),
        }
    }

    pub(super) fn cmd_capture(
        &mut self,
        args: &[&str],
        out: &mut dyn Write,
    ) -> Result<(), CliError> {
        match args {
            ["save", file] => {
                self.fs_allowed()?;
                let sim = self.sim()?;
                let cap = Capture::from_sim(sim, &self.name);
                cap.save(Path::new(file))?;
                writeln!(
                    out,
                    "Capture enregistrée dans {file} ({} événements, {} équipements).",
                    cap.events.len(),
                    cap.devices.len()
                )?;
                Ok(())
            }
            _ => err("usage : capture save <fichier.nlv>"),
        }
    }

    pub(super) fn cmd_forensic(
        &mut self,
        args: &[&str],
        out: &mut dyn Write,
    ) -> Result<(), CliError> {
        let usage = "usage : forensic analyze <fichier.nlv | live> [--packets]";
        let (what, packets) = match args {
            ["analyze", what] => (*what, false),
            ["analyze", what, "--packets"] | ["analyze", "--packets", what] => (*what, true),
            _ => return err(usage),
        };
        let cap = if what == "live" {
            Capture::from_sim(self.sim()?, &self.name)
        } else {
            self.fs_allowed()?;
            Capture::load(Path::new(what))?
        };
        if packets {
            write!(out, "{}", Analysis::render_packets(&cap))?;
        } else {
            write!(out, "{}", analyze_capture(&cap).render(&cap))?;
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // cyber range
    // ------------------------------------------------------------------

    pub(super) fn cmd_range(&mut self, args: &[&str], out: &mut dyn Write) -> Result<(), CliError> {
        match args {
            ["list"] => {
                writeln!(out, "{:<10} {:<10} {:<5} TITRE", "ID", "NIVEAU", "FAIT")?;
                for r in builtin::ranges() {
                    let done = self
                        .profile
                        .completed
                        .get(&r.id)
                        .map_or("·".to_string(), |c| c.score.to_string());
                    writeln!(
                        out,
                        "{:<10} {:<10} {:<5} {}",
                        r.id,
                        difficulty_label(r.difficulty),
                        done,
                        r.title
                    )?;
                }
                Ok(())
            }
            ["start", id] => self.range_start(id, out),
            ["show"] => {
                let Some(run) = &self.range_run else {
                    return err("aucun scénario en cours : « range start <id> »");
                };
                let now = (self.clock)();
                let s = run.scenario();
                writeln!(out, "{} — {}\n{}", s.id, s.title, s.description.trim_end())?;
                let left = run.remaining_secs(now);
                if left >= 0 {
                    writeln!(out, "Temps restant : {} min {:02} s", left / 60, left % 60)?;
                }
                writeln!(out, "Indices utilisés : {}", run.hints_used())?;
                Ok(())
            }
            ["check"] => self.range_check(out),
            ["finish"] => {
                let now = (self.clock)();
                let Some(run) = self.range_run.as_mut() else {
                    return err("aucun scénario en cours : « range start <id> »");
                };
                let score = run.finish(now).map_err(run_error)?;
                self.range_report(&score, out)
            }
            ["giveup"] => {
                let Some(run) = self.range_run.as_mut() else {
                    return err("aucun scénario en cours : « range start <id> »");
                };
                run.give_up().map_err(run_error)?;
                let s = run.scenario();
                writeln!(out, "Scénario abandonné — aucun point. Voici la solution :")?;
                writeln!(
                    out,
                    "Coupable : {} ({})",
                    s.culprit.device, s.culprit.behavior
                )?;
                writeln!(out, "{}", s.solution.explanation.trim_end())?;
                writeln!(out, "Commandes : {}", s.solution.commands.join(" ; "))?;
                Ok(())
            }
            _ => err("usage : range list | start <id> | show | check | finish | giveup"),
        }
    }

    fn range_start(&mut self, id: &str, out: &mut dyn Write) -> Result<(), CliError> {
        let scenario = builtin::range(id)
            .ok_or_else(|| CliError(format!("scénario inconnu : « {id} » (voir « range list »)")))?
            .clone();
        let now = (self.clock)();
        let (run, sim) = RangeRun::start(scenario.clone(), now).map_err(CliError)?;
        self.sim = Some(sim);
        self.layout = Layout::new();
        self.name = scenario.id.clone();
        self.chaos = None;
        self.baseline = None;
        self.last_cause = None;
        self.hint_level = 0;
        self.run = None;
        self.range_run = Some(run);
        self.profile.record_attempt(&scenario.id);
        let _ = self.save_profile_quiet();

        writeln!(
            out,
            "══ CYBER RANGE {} — {} ══",
            scenario.id, scenario.title
        )?;
        writeln!(
            out,
            "Difficulté : {}    Temps conseillé : {} min",
            difficulty_label(scenario.difficulty),
            scenario.time_limit_minutes
        )?;
        writeln!(out)?;
        writeln!(out, "{}", scenario.description.trim_end())?;
        writeln!(out)?;
        writeln!(
            out,
            "Barème : coupable +40 · correction +40 · temps +20 · indice −10"
        )?;
        writeln!(out)?;
        writeln!(
            out,
            "L'incident a déjà eu lieu : tout est dans la trace de la simulation."
        )?;
        writeln!(
            out,
            "Outils : forensic analyze live, capture save, security analyze, log, trace, show acl FW1…"
        )?;
        writeln!(
            out,
            "Puis : corrige (acl …), « range check », et désigne le coupable avec « answer <équipement> <comportement> »."
        )?;
        writeln!(
            out,
            "Comportements : port-scan, host-sweep, lateral-movement, beaconing, arp-spoofing, repeated-denials, rogue-dhcp."
        )?;
        Ok(())
    }

    fn range_check(&mut self, out: &mut dyn Write) -> Result<(), CliError> {
        let now = (self.clock)();
        let (Some(run), Some(sim)) = (self.range_run.as_mut(), self.sim.as_ref()) else {
            return err("aucun scénario en cours : « range start <id> »");
        };
        match run.check(sim, now).map_err(run_error)? {
            RangeCheck::Failing { still_open, broken } => {
                writeln!(out, "Pas encore :")?;
                for f in &still_open {
                    writeln!(out, "  ✗ ce flux illégitime passe toujours : {f}")?;
                }
                for f in &broken {
                    writeln!(out, "  ✗ ce flux légitime est cassé : {f}")?;
                }
                if !broken.is_empty() {
                    writeln!(
                        out,
                        "(Une bonne correction bloque l'attaque SANS couper le trafic légitime.)"
                    )?;
                }
            }
            RangeCheck::NeedsAnswer => {
                writeln!(
                    out,
                    "Brèche fermée ✓ — l'attaque est bloquée et le trafic légitime fonctionne."
                )?;
                writeln!(
                    out,
                    "Dernière étape : désigne le coupable avec « answer <équipement> <comportement> » (ou « range finish » : −40 points)."
                )?;
            }
            RangeCheck::Finished(score) => self.range_report(&score, out)?,
        }
        Ok(())
    }

    pub(super) fn range_answer(
        &mut self,
        device: &str,
        behavior: &str,
        now: u64,
        out: &mut dyn Write,
    ) -> Result<(), CliError> {
        if parse_behavior(behavior).is_none() {
            return err(format!(
                "comportement inconnu : « {behavior} » (port-scan, host-sweep, lateral-movement, beaconing, arp-spoofing, repeated-denials, rogue-dhcp)"
            ));
        }
        let Some(run) = self.range_run.as_mut() else {
            return err("aucun scénario en cours");
        };
        match run.answer(device, behavior, now).map_err(run_error)? {
            AnswerOutcome::Correct(Some(score)) => {
                writeln!(out, "Coupable identifié ✓")?;
                self.range_report(&score, out)?;
            }
            AnswerOutcome::Correct(None) => {
                writeln!(
                    out,
                    "Coupable identifié ✓ (+40). Il reste à fermer la brèche : « range check »."
                )?;
            }
            AnswerOutcome::Wrong {
                device_ok,
                cause_ok,
            } => {
                let hint = match (device_ok, cause_ok) {
                    (true, false) => " La bonne machine, mais pas ce comportement.",
                    (false, true) => " Le bon comportement, mais pas cette machine.",
                    _ => "",
                };
                writeln!(out, "Ce n'est pas ça (−10 points de diagnostic).{hint}")?;
            }
        }
        Ok(())
    }

    fn save_profile_quiet(&self) -> Result<(), CliError> {
        if let Some(path) = &self.profile_path {
            self.profile.save(path)?;
        }
        Ok(())
    }

    fn range_report(&mut self, score: &Score, out: &mut dyn Write) -> Result<(), CliError> {
        let Some(run) = &self.range_run else {
            return Ok(());
        };
        let s = run.scenario().clone();
        let rec = self
            .profile
            .record_result(&s.id, "security", &s.culprit.behavior, score);
        let save = self.save_profile_quiet();

        writeln!(out, "✓ SCÉNARIO TERMINÉ — {}", s.title)?;
        writeln!(
            out,
            "Score : {}/100   (coupable {} + correction {} + temps {} − indices {})",
            score.total, score.diagnosis, score.fix, score.time, score.hint_penalty
        )?;
        writeln!(
            out,
            "Temps : {} min {:02} s    Indices : {}",
            score.elapsed_secs / 60,
            score.elapsed_secs % 60,
            score.hints
        )?;
        if rec.improved {
            writeln!(
                out,
                "XP gagnés : +{}  → total {}",
                rec.xp_gained,
                self.profile.xp()
            )?;
        } else {
            writeln!(
                out,
                "Pas mieux que ton meilleur résultat : aucun XP supplémentaire."
            )?;
        }
        if rec.level_after > rec.level_before {
            let (n, name, _) = netiv_education::profile::level_for(self.profile.xp());
            writeln!(out, "★ Niveau {n} atteint : « {name} » !")?;
        }
        for b in &rec.new_badges {
            writeln!(out, "🏅 Nouveau badge : {} ({})", b.name, b.how)?;
        }
        writeln!(out)?;
        writeln!(out, "Explication : {}", s.solution.explanation.trim_end())?;
        save
    }
}

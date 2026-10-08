//! Commandes pédagogiques : challenges, progression, parcours, cours.

use std::fmt::Write as _;
use std::path::Path;

use netiv_education::builtin;
use netiv_education::learn::{self, LearningPath};
use netiv_education::run::{AnswerOutcome, CheckOutcome, RunError, Score, parse_cause};
use netiv_education::{Challenge, ChallengeRun, Course};

use super::*;

/// Résumé de l'épreuve en cours (challenge ou cyber range).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChallengeInfo {
    /// `challenge` ou `range`.
    pub kind: &'static str,
    /// Identifiant.
    pub id: String,
    /// Titre.
    pub title: String,
    /// Énoncé.
    pub description: String,
    /// Terminée (réussie ou abandonnée) ?
    pub finished: bool,
    /// Objectif atteint ?
    pub fixed: bool,
    /// Diagnostic donné ?
    pub answered: bool,
    /// Indices utilisés.
    pub hints_used: u32,
    /// Secondes restantes sur le temps conseillé (négatif si dépassé).
    pub remaining_secs: i64,
    /// Score final, s'il y en a un.
    pub score: Option<u32>,
}

const LOCKED: &str = "indisponible pendant un challenge : c'est à toi de trouver (« hint » si tu bloques, −10 points)";

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
    /// `true` tant qu'un challenge est en cours (non terminé).
    pub(super) fn challenge_active(&self) -> bool {
        self.run.as_ref().is_some_and(|r| !r.is_finished())
            || self.range_run.as_ref().is_some_and(|r| !r.is_finished())
    }

    /// État du challenge ou du scénario en cours, pour les interfaces.
    pub fn challenge_info(&self) -> Option<ChallengeInfo> {
        let now = (self.clock)();
        if let Some(r) = &self.run {
            let c = r.challenge();
            return Some(ChallengeInfo {
                kind: "challenge",
                id: c.id.clone(),
                title: c.title.clone(),
                description: c.description.trim_end().to_string(),
                finished: r.is_finished(),
                fixed: r.is_fixed(),
                answered: r.is_answered(),
                hints_used: r.hints_used(),
                remaining_secs: r.remaining_secs(now),
                score: r.score().map(|s| s.total),
            });
        }
        self.range_run.as_ref().map(|r| {
            let s = r.scenario();
            ChallengeInfo {
                kind: "range",
                id: s.id.clone(),
                title: s.title.clone(),
                description: s.description.trim_end().to_string(),
                finished: r.is_finished(),
                fixed: false,
                answered: false,
                hints_used: r.hints_used(),
                remaining_secs: r.remaining_secs(now),
                score: None,
            }
        })
    }

    /// Démarre un challenge décrit par un texte (YAML de challenge ou `.netiv`).
    pub fn start_challenge_text(
        &mut self,
        text: &str,
        out: &mut dyn Write,
    ) -> Result<(), CliError> {
        let challenge = if text.contains("format_version") {
            let bundle = netiv_topology::Bundle::from_yaml(text)?;
            Challenge::from_bundle(&bundle).map_err(CliError)?
        } else {
            Challenge::from_yaml(text).map_err(CliError)?
        };
        self.start_loaded(challenge, None, out)
    }

    /// Refuse une commande qui donnerait la réponse pendant un challenge.
    pub(super) fn deny_during_challenge(&self) -> Result<(), CliError> {
        if self.challenge_active() {
            return err(LOCKED);
        }
        Ok(())
    }

    fn save_profile(&self) -> Result<(), CliError> {
        if let Some(path) = &self.profile_path {
            self.profile.save(path)?;
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // challenge
    // ------------------------------------------------------------------

    pub(super) fn cmd_challenge(
        &mut self,
        args: &[&str],
        out: &mut dyn Write,
    ) -> Result<(), CliError> {
        match args {
            ["list", filter @ ..] => self.challenge_list(filter, out),
            ["start", what] => self.challenge_start(what, None, out),
            ["show"] => self.challenge_show(out),
            ["hint"] => self.cmd_hint(&[], out),
            ["check"] => self.challenge_check(out),
            ["finish"] => self.challenge_finish(out),
            ["giveup"] => self.challenge_giveup(out),
            ["causes"] => {
                writeln!(out, "Natures de panne reconnues par « answer » :")?;
                for k in netiv_diagnostics::CauseKind::ALL {
                    writeln!(out, "  {:<20} {}", k.slug(), k.label())?;
                }
                Ok(())
            }
            ["export", id, file] => self.challenge_export(id, file, out),
            ["generate", rest @ ..] => self.challenge_generate(rest, out),
            _ => err(
                "usage : challenge list [thème|facile|moyen|difficile] | start <id|fichier> | generate --topic T --difficulty D [--seed N] [--save f.netiv] | show | hint | check | finish | giveup | causes | export <id> <fichier.netiv>",
            ),
        }
    }

    fn challenge_list(&self, filter: &[&str], out: &mut dyn Write) -> Result<(), CliError> {
        let want = filter.first().copied();
        writeln!(
            out,
            "{:<17} {:<10} {:<12} {:<4} TITRE",
            "ID", "NIVEAU", "THÈME", "FAIT"
        )?;
        let mut shown = 0;
        for c in builtin::challenges() {
            let diff = netiv_chaos::Difficulty::from_slug(want.unwrap_or(""));
            let ok = match want {
                None => true,
                Some(w) => c.topic == w || diff == Some(c.difficulty),
            };
            if !ok {
                continue;
            }
            shown += 1;
            let done = self
                .profile
                .completed
                .get(&c.id)
                .map_or("·".to_string(), |d| format!("{}", d.score));
            writeln!(
                out,
                "{:<17} {:<10} {:<12} {:<4} {}",
                c.id,
                difficulty_label(c.difficulty),
                netiv_education::profile::topic_label(&c.topic),
                done,
                c.title
            )?;
        }
        if shown == 0 {
            writeln!(out, "(aucun challenge pour « {} »)", want.unwrap_or(""))?;
        }
        Ok(())
    }

    /// Charge un challenge livré (par identifiant) ou un fichier.
    fn load_challenge(&self, what: &str) -> Result<Challenge, CliError> {
        if let Some(c) = builtin::challenge(what) {
            return Ok(c.clone());
        }
        if !self.sandbox && Path::new(what).is_file() {
            let text = self.read_text(what)?;
            if text.contains("format_version") {
                let bundle = netiv_topology::Bundle::from_yaml(&text)?;
                return Challenge::from_bundle(&bundle).map_err(CliError);
            }
            return Challenge::from_yaml(&text).map_err(CliError);
        }
        err(format!(
            "challenge inconnu : « {what} » (ni un identifiant, ni un fichier — voir « challenge list »)"
        ))
    }

    pub(super) fn challenge_start(
        &mut self,
        what: &str,
        item: Option<&netiv_education::CourseItem>,
        out: &mut dyn Write,
    ) -> Result<(), CliError> {
        let challenge = self.load_challenge(what)?;
        self.start_loaded(challenge, item, out)
    }

    /// Démarre un challenge déjà chargé (livré, fichier ou généré).
    pub(super) fn start_loaded(
        &mut self,
        challenge: Challenge,
        item: Option<&netiv_education::CourseItem>,
        out: &mut dyn Write,
    ) -> Result<(), CliError> {
        let now = (self.clock)();
        let (mut run, sim) =
            ChallengeRun::start(challenge.clone(), now).map_err(|e| CliError(e.to_string()))?;
        let mut notes = String::new();
        if let Some(item) = item {
            if !item.hints_allowed() {
                run = run.without_hints();
                notes += " · indices interdits";
            }
            if item.minutes > 0 {
                run = run.with_time_limit(item.minutes);
                let _ = write!(notes, " · {} min imparties", item.minutes);
            }
        }
        self.sim = Some(sim);
        self.name = challenge.id.clone();
        self.chaos = None;
        self.baseline = None;
        self.last_cause = None;
        self.hint_level = 0;
        self.profile.record_attempt(&challenge.id);
        self.run = Some(run);
        let _ = self.save_profile();

        writeln!(
            out,
            "══ CHALLENGE {} — {} ══",
            challenge.id, challenge.title
        )?;
        writeln!(
            out,
            "Difficulté : {}    Thème : {}    Temps conseillé : {} min{notes}",
            difficulty_label(challenge.difficulty),
            netiv_education::profile::topic_label(&challenge.topic),
            challenge.time_limit_minutes
        )?;
        writeln!(out)?;
        writeln!(out, "{}", challenge.description.trim_end())?;
        writeln!(out)?;
        let how = match &challenge.objective.service {
            None => "ping".to_string(),
            Some(s) => format!("{}/{}", s.proto, s.port),
        };
        writeln!(
            out,
            "Objectif : {} doit joindre {} ({how}).",
            challenge.objective.from, challenge.objective.to
        )?;
        writeln!(
            out,
            "Barème   : diagnostic +40 · correction +40 · temps +20 · indice −10"
        )?;
        writeln!(out)?;
        writeln!(
            out,
            "Outils : ping, curl, trace, log, show …, puis « answer <équipement> <cause> » et « challenge check »."
        )?;
        writeln!(
            out,
            "(« challenge causes » liste les natures de panne ; « hint » donne un indice.)"
        )?;
        Ok(())
    }

    fn challenge_show(&self, out: &mut dyn Write) -> Result<(), CliError> {
        let Some(run) = &self.run else {
            return err("aucun challenge en cours : « challenge start <id> »");
        };
        let c = run.challenge();
        let now = (self.clock)();
        writeln!(out, "{} — {}", c.id, c.title)?;
        writeln!(out, "{}", c.description.trim_end())?;
        let left = run.remaining_secs(now);
        if left >= 0 {
            writeln!(out, "Temps restant : {} min {:02} s", left / 60, left % 60)?;
        } else {
            writeln!(
                out,
                "Temps conseillé dépassé de {} min {:02} s",
                -left / 60,
                -left % 60
            )?;
        }
        writeln!(
            out,
            "Indices utilisés : {}    Objectif atteint : {}    Diagnostic donné : {}",
            run.hints_used(),
            if run.is_fixed() { "oui" } else { "non" },
            if run.is_answered() { "oui" } else { "non" }
        )?;
        if run.is_finished() {
            writeln!(out, "(challenge terminé)")?;
        }
        Ok(())
    }

    fn challenge_check(&mut self, out: &mut dyn Write) -> Result<(), CliError> {
        let now = (self.clock)();
        let (Some(run), Some(sim)) = (self.run.as_mut(), self.sim.as_ref()) else {
            return err("aucun challenge en cours : « challenge start <id> »");
        };
        match run.check(sim, now).map_err(run_error)? {
            CheckOutcome::NotYet { replies, sent } => {
                writeln!(
                    out,
                    "Pas encore : l'objectif ne répond que {replies}/{sent} fois. Continue de chercher."
                )?;
            }
            CheckOutcome::NeedsAnswer => {
                writeln!(out, "Objectif atteint ✓ — le réseau fonctionne de nouveau.")?;
                writeln!(
                    out,
                    "Dernière étape : explique la cause avec « answer <équipement> <cause> » (ou « challenge finish » pour terminer sans diagnostic : −40 points)."
                )?;
            }
            CheckOutcome::Finished(score) => self.finish_report(&score, out)?,
        }
        Ok(())
    }

    fn challenge_finish(&mut self, out: &mut dyn Write) -> Result<(), CliError> {
        let now = (self.clock)();
        let Some(run) = self.run.as_mut() else {
            return err("aucun challenge en cours : « challenge start <id> »");
        };
        let score = run.finish(now).map_err(run_error)?;
        self.finish_report(&score, out)
    }

    fn challenge_giveup(&mut self, out: &mut dyn Write) -> Result<(), CliError> {
        let Some(run) = self.run.as_mut() else {
            return err("aucun challenge en cours : « challenge start <id> »");
        };
        run.give_up().map_err(run_error)?;
        let c = run.challenge();
        writeln!(
            out,
            "Challenge abandonné — aucun point. Voici la solution :"
        )?;
        writeln!(out, "{}", c.solution.explanation.trim_end())?;
        if !c.solution.commands.is_empty() {
            writeln!(out, "Commandes : {}", c.solution.commands.join(" ; "))?;
        }
        Ok(())
    }

    /// Enregistre le score dans le profil et l'affiche.
    fn finish_report(&mut self, score: &Score, out: &mut dyn Write) -> Result<(), CliError> {
        let Some(run) = &self.run else {
            return Ok(());
        };
        let challenge = run.challenge().clone();
        let rec = self.profile.record(&challenge, score);
        let save = self.save_profile();

        writeln!(out, "✓ CHALLENGE TERMINÉ — {}", challenge.title)?;
        writeln!(
            out,
            "Score : {}/100   (diagnostic {} + correction {} + temps {} − indices {})",
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
        writeln!(
            out,
            "Explication : {}",
            challenge.solution.explanation.trim_end()
        )?;
        save
    }

    fn challenge_export(&self, id: &str, file: &str, out: &mut dyn Write) -> Result<(), CliError> {
        let c = self.load_challenge(id)?;
        let bundle = c.to_bundle(&self.profile.name).map_err(CliError)?;
        self.write_text(file, &bundle.to_yaml()?)?;
        writeln!(
            out,
            "Challenge « {} » exporté dans {file} (réseau et pannes inclus).",
            c.id
        )?;
        Ok(())
    }

    /// `answer <équipement> <cause>`
    pub(super) fn cmd_answer(
        &mut self,
        args: &[&str],
        out: &mut dyn Write,
    ) -> Result<(), CliError> {
        let [device, cause @ ..] = args else {
            return err(
                "usage : answer <équipement> <cause>   (« challenge causes » liste les causes)",
            );
        };
        if cause.is_empty() {
            return err(
                "usage : answer <équipement> <cause>   (« challenge causes » liste les causes)",
            );
        }
        let cause = cause.join(" ");
        let now = (self.clock)();
        if self.range_run.as_ref().is_some_and(|r| !r.is_finished()) {
            return self.range_answer(device, &cause, now, out);
        }
        if parse_cause(&cause).is_none() {
            return err(format!(
                "cause inconnue : « {cause} » (« challenge causes » liste les natures de panne)"
            ));
        }
        let Some(run) = self.run.as_mut() else {
            return err(
                "aucun challenge ou scénario en cours : « challenge start <id> » / « range start <id> »",
            );
        };
        match run.answer(device, &cause, now).map_err(run_error)? {
            AnswerOutcome::Correct(Some(score)) => {
                writeln!(out, "Diagnostic correct ✓")?;
                self.finish_report(&score, out)?;
            }
            AnswerOutcome::Correct(None) => {
                writeln!(
                    out,
                    "Diagnostic correct ✓ (+40). Il reste à réparer : « challenge check » quand c'est fait."
                )?;
            }
            AnswerOutcome::Wrong {
                device_ok,
                cause_ok,
            } => {
                let hint = match (device_ok, cause_ok) {
                    (true, false) => " Le bon équipement, mais pas la bonne nature de panne.",
                    (false, true) => " La bonne nature de panne, mais pas sur cet équipement.",
                    _ => "",
                };
                writeln!(out, "Ce n'est pas ça (−10 points de diagnostic).{hint}")?;
            }
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // progression
    // ------------------------------------------------------------------

    pub(super) fn cmd_dashboard(&self, out: &mut dyn Write) -> Result<(), CliError> {
        write!(out, "{}", self.profile.dashboard(&builtin::catalog()))?;
        Ok(())
    }

    pub(super) fn cmd_badges(&self, out: &mut dyn Write) -> Result<(), CliError> {
        for b in netiv_education::BADGES {
            let mark = if self.profile.badges.contains(b.id) {
                "🏅"
            } else {
                "  "
            };
            writeln!(out, "{mark} {:<20} {}", b.name, b.how)?;
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // parcours
    // ------------------------------------------------------------------

    fn path(&self, key: Option<&str>) -> Result<&'static LearningPath, CliError> {
        let paths = learn::paths();
        match key {
            None => paths
                .first()
                .ok_or_else(|| CliError("aucun parcours disponible".into())),
            Some(k) => paths
                .iter()
                .find(|p| p.id == k)
                .ok_or_else(|| CliError(format!("parcours inconnu : « {k} »"))),
        }
    }

    pub(super) fn cmd_learn(&mut self, args: &[&str], out: &mut dyn Write) -> Result<(), CliError> {
        match args {
            [] => {
                for p in learn::paths() {
                    writeln!(
                        out,
                        "{:<14} {} — {}/{} leçons",
                        p.id,
                        p.title,
                        p.done_count(&self.profile),
                        p.lessons.len()
                    )?;
                }
                writeln!(out, "« learn <parcours> » affiche la liste à cocher.")?;
                Ok(())
            }
            ["next"] => {
                let p = self.path(None)?;
                match p.next_lesson(&self.profile) {
                    Some((n, l)) => write!(out, "{}", p.render_lesson(n, l))?,
                    None => writeln!(out, "Parcours terminé, bravo !")?,
                }
                Ok(())
            }
            ["read", key] => {
                let p = self.path(None)?;
                let Some((n, l)) = p.lesson(key) else {
                    return err(format!(
                        "leçon inconnue : « {key} » (numéro ou identifiant)"
                    ));
                };
                write!(out, "{}", p.render_lesson(n, l))?;
                if l.challenges.is_empty() {
                    self.profile.mark_lesson_read(&l.id);
                    self.save_profile()?;
                    writeln!(out, "\n✓ Leçon validée.")?;
                }
                Ok(())
            }
            [id] => {
                let p = self.path(Some(id))?;
                write!(out, "{}", p.render(&self.profile))?;
                Ok(())
            }
            _ => err("usage : learn | learn <parcours> | learn read <n°|id> | learn next"),
        }
    }

    // ------------------------------------------------------------------
    // cours (mode enseignant)
    // ------------------------------------------------------------------

    fn load_course(&self, file: &str) -> Result<Course, CliError> {
        Course::from_yaml(&self.read_text(file)?).map_err(CliError)
    }

    pub(super) fn cmd_course(
        &mut self,
        args: &[&str],
        out: &mut dyn Write,
    ) -> Result<(), CliError> {
        match args {
            ["new", id, rest @ ..] => {
                let yaml = Course::template(id).to_yaml().map_err(CliError)?;
                match rest {
                    [] => write!(out, "{yaml}")?,
                    [file] => {
                        self.fs_allowed()?;
                        if Path::new(file).exists() {
                            return err(format!("« {file} » existe déjà : je ne l'écrase pas"));
                        }
                        self.write_text(file, &yaml)?;
                        writeln!(
                            out,
                            "Modèle de cours écrit dans {file}. Modifie-le puis « course validate {file} »."
                        )?;
                    }
                    _ => return err("usage : course new <id> [fichier]"),
                }
                Ok(())
            }
            ["show", file] => {
                write!(out, "{}", self.load_course(file)?.render())?;
                Ok(())
            }
            ["validate", file] => {
                let course = self.load_course(file)?;
                let problems = course.problems();
                if problems.is_empty() {
                    writeln!(
                        out,
                        "Cours « {} » valide : {} élément(s), {} min, {} points.",
                        course.title,
                        course.items.len(),
                        course.total_minutes(),
                        course.total_points()
                    )?;
                    Ok(())
                } else {
                    let mut msg = format!("cours « {} » invalide :", course.title);
                    for p in problems {
                        let _ = write!(msg, "\n  - {p}");
                    }
                    err(msg)
                }
            }
            ["export", file, rest @ ..] => {
                let md = self.load_course(file)?.to_markdown();
                match rest {
                    [] => write!(out, "{md}")?,
                    [dest] => {
                        self.write_text(dest, &md)?;
                        writeln!(out, "Cours exporté dans {dest}.")?;
                    }
                    _ => return err("usage : course export <fichier> [sortie.md]"),
                }
                Ok(())
            }
            ["start", file, n] => {
                let course = self.load_course(file)?;
                let problems = course.problems();
                if !problems.is_empty() {
                    return err(format!("cours invalide : {}", problems.join(" ; ")));
                }
                let idx: usize = n
                    .parse::<usize>()
                    .ok()
                    .filter(|i| *i >= 1 && *i <= course.items.len())
                    .ok_or_else(|| {
                        CliError(format!(
                            "élément n°{n} inexistant (1 à {})",
                            course.items.len()
                        ))
                    })?;
                let item = course.items[idx - 1].clone();
                let Some(id) = item.challenge.clone() else {
                    writeln!(
                        out,
                        "{} : {}\n{}",
                        item.kind.label(),
                        item.title,
                        item.instructions.trim_end()
                    )?;
                    return Ok(());
                };
                writeln!(out, "{} {idx} — {}", item.kind.label(), item.title)?;
                self.challenge_start(&id, Some(&item), out)
            }
            ["grade", file] => {
                let course = self.load_course(file)?;
                write!(
                    out,
                    "{}",
                    course.grade(&self.profile).render(&self.profile.name)
                )?;
                Ok(())
            }
            _ => err(
                "usage : course new <id> [fichier] | show <f> | validate <f> | export <f> [md] | start <f> <n°> | grade <f>",
            ),
        }
    }
}

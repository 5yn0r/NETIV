//! Le déroulement d'un challenge : indices, réponse, vérification, score.
//!
//! Barème (cahier des charges §21) :
//!
//! | Élément | Points |
//! |---|---|
//! | Diagnostic juste (`answer`) | +40, −10 par mauvaise réponse |
//! | Correction (objectif atteint) | +40 |
//! | Temps | jusqu'à +20 (plein dans le premier quart du temps conseillé, nul au-delà) |
//! | Indice | −10 chacun |
//!
//! Le total est borné entre 0 et 100.

use netiv_core::Simulation;
use netiv_diagnostics::{CauseKind, Probe, RootCause, run_probe};

use crate::challenge::{Challenge, ChallengeError};

/// Points d'un diagnostic juste.
pub const DIAGNOSIS_POINTS: u32 = 40;
/// Points d'une correction réussie.
pub const FIX_POINTS: u32 = 40;
/// Points maximum pour le temps.
pub const TIME_POINTS: u32 = 20;
/// Pénalité par indice.
pub const HINT_PENALTY: u32 = 10;
/// Pénalité par mauvaise réponse.
pub const WRONG_ANSWER_PENALTY: u32 = 10;

/// Résultat chiffré d'un challenge.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Score {
    /// Points de diagnostic.
    pub diagnosis: u32,
    /// Points de correction.
    pub fix: u32,
    /// Points de temps.
    pub time: u32,
    /// Pénalité totale des indices.
    pub hint_penalty: u32,
    /// Total, entre 0 et 100.
    pub total: u32,
    /// XP gagnés (total × multiplicateur de difficulté).
    pub xp: u32,
    /// Temps écoulé.
    pub elapsed_secs: u64,
    /// Indices utilisés.
    pub hints: u32,
}

/// Points de temps : pleins dans le premier quart du temps conseillé,
/// décroissants jusqu'à 0 à la limite.
pub fn time_points(elapsed_secs: u64, limit_minutes: u32) -> u32 {
    let limit = u64::from(limit_minutes) * 60;
    if limit == 0 || elapsed_secs >= limit {
        return 0;
    }
    let full = limit / 4;
    if elapsed_secs <= full {
        return TIME_POINTS;
    }
    let left = limit - elapsed_secs;
    let span = limit - full;
    let pts = u64::from(TIME_POINTS) * left / span;
    u32::try_from(pts).unwrap_or(TIME_POINTS)
}

/// Opération impossible dans l'état actuel du challenge.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RunError {
    /// Le challenge est terminé.
    #[error("ce challenge est déjà terminé")]
    Finished,
    /// Les indices sont interdits (examen).
    #[error("les indices sont désactivés pour cette épreuve")]
    HintsDisabled,
    /// L'objectif n'est pas atteint.
    #[error("l'objectif n'est pas encore atteint : « challenge check » pour le vérifier")]
    NotFixed,
}

/// Réponse à `answer`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnswerOutcome {
    /// Bonne réponse. Le challenge se termine si l'objectif était déjà atteint.
    Correct(Option<Score>),
    /// Mauvaise réponse.
    Wrong {
        /// L'équipement était le bon.
        device_ok: bool,
        /// La nature de la panne était la bonne.
        cause_ok: bool,
    },
}

/// Résultat de `check`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckOutcome {
    /// L'objectif échoue encore.
    NotYet {
        /// Réponses reçues / envois.
        replies: u16,
        /// Envois.
        sent: u16,
    },
    /// L'objectif est atteint mais le diagnostic n'a pas été donné.
    NeedsAnswer,
    /// Challenge terminé.
    Finished(Score),
}

/// Un challenge en cours.
#[derive(Debug, Clone)]
pub struct ChallengeRun {
    challenge: Challenge,
    probe: Probe,
    dns_name: Option<String>,
    cause: RootCause,
    hints: Vec<String>,
    started_secs: u64,
    hints_used: u32,
    wrong_answers: u32,
    answered: bool,
    fixed: bool,
    finished: Option<Score>,
    gave_up: bool,
    allow_hints: bool,
    time_limit_minutes: u32,
}

impl ChallengeRun {
    /// Démarre un challenge. Retourne aussi le réseau à jouer (pannes cachées).
    pub fn start(
        challenge: Challenge,
        now_secs: u64,
    ) -> Result<(ChallengeRun, Simulation), ChallengeError> {
        let built = challenge.build()?;
        let time_limit_minutes = challenge.time_limit_minutes;
        Ok((
            ChallengeRun {
                challenge,
                probe: built.probe,
                dns_name: built.dns_name,
                cause: built.cause,
                hints: built.hints,
                started_secs: now_secs,
                hints_used: 0,
                wrong_answers: 0,
                answered: false,
                fixed: false,
                finished: None,
                gave_up: false,
                allow_hints: true,
                time_limit_minutes,
            },
            built.sim,
        ))
    }

    /// Interdit les indices (épreuve d'examen).
    pub fn without_hints(mut self) -> Self {
        self.allow_hints = false;
        self
    }

    /// Remplace la durée conseillée (examen avec temps imparti).
    pub fn with_time_limit(mut self, minutes: u32) -> Self {
        self.time_limit_minutes = minutes.max(1);
        self
    }

    /// Le challenge joué.
    pub fn challenge(&self) -> &Challenge {
        &self.challenge
    }

    /// Terminé (réussi ou abandonné) ?
    pub fn is_finished(&self) -> bool {
        self.finished.is_some() || self.gave_up
    }

    /// Score final, s'il y en a un.
    pub fn score(&self) -> Option<&Score> {
        self.finished.as_ref()
    }

    /// L'objectif a-t-il été atteint ?
    pub fn is_fixed(&self) -> bool {
        self.fixed
    }

    /// Le diagnostic a-t-il été donné ?
    pub fn is_answered(&self) -> bool {
        self.answered
    }

    /// Indices utilisés.
    pub fn hints_used(&self) -> u32 {
        self.hints_used
    }

    /// Indices restants avant la solution complète (le dernier niveau).
    pub fn hints_left(&self) -> usize {
        self.hints.len().saturating_sub(self.hints_used as usize)
    }

    /// Les indices sont-ils autorisés ?
    pub fn hints_allowed(&self) -> bool {
        self.allow_hints
    }

    /// Temps écoulé.
    pub fn elapsed_secs(&self, now_secs: u64) -> u64 {
        now_secs.saturating_sub(self.started_secs)
    }

    /// Secondes restantes sur le temps conseillé (négatif si dépassé).
    pub fn remaining_secs(&self, now_secs: u64) -> i64 {
        let limit = i64::from(self.time_limit_minutes) * 60;
        limit - i64::try_from(self.elapsed_secs(now_secs)).unwrap_or(i64::MAX)
    }

    /// Le diagnostic correct (pour l'écran de fin ou l'abandon).
    pub fn reveal(&self) -> &RootCause {
        &self.cause
    }

    /// Indice suivant. Chaque indice coûte 10 points, y compris la répétition
    /// de la solution.
    pub fn next_hint(&mut self) -> Result<String, RunError> {
        if self.is_finished() {
            return Err(RunError::Finished);
        }
        if !self.allow_hints {
            return Err(RunError::HintsDisabled);
        }
        let idx = (self.hints_used as usize).min(self.hints.len().saturating_sub(1));
        self.hints_used += 1;
        Ok(self.hints.get(idx).cloned().unwrap_or_default())
    }

    /// Propose un diagnostic : équipement + nature de la panne (nom court ou
    /// libellé, sans tenir compte de la casse).
    pub fn answer(
        &mut self,
        device: &str,
        cause: &str,
        now_secs: u64,
    ) -> Result<AnswerOutcome, RunError> {
        if self.is_finished() {
            return Err(RunError::Finished);
        }
        if self.answered {
            return Ok(AnswerOutcome::Correct(None));
        }
        let device_ok = device.eq_ignore_ascii_case(&self.challenge.solution.device);
        let cause_ok =
            parse_cause(cause).is_some_and(|k| k.slug() == self.challenge.solution.cause);
        if device_ok && cause_ok {
            self.answered = true;
            let score = (self.fixed).then(|| self.finalize(now_secs));
            return Ok(AnswerOutcome::Correct(score));
        }
        self.wrong_answers += 1;
        Ok(AnswerOutcome::Wrong {
            device_ok,
            cause_ok,
        })
    }

    /// Vérifie l'objectif sur le réseau `sim`.
    pub fn check(&mut self, sim: &Simulation, now_secs: u64) -> Result<CheckOutcome, RunError> {
        if self.is_finished() {
            return Err(RunError::Finished);
        }
        let result = run_probe(sim, &self.probe).map_err(|_| RunError::NotFixed)?;
        let reached =
            crate::challenge::objective_reached(sim, &self.probe, self.dns_name.as_deref())
                .map_err(|_| RunError::NotFixed)?;
        if !reached {
            return Ok(CheckOutcome::NotYet {
                // Avec un nom DNS qui ne se résout pas vers la bonne adresse,
                // rien n'arrive à destination, même si l'adresse répond.
                replies: if self.dns_name.is_some() && result.success {
                    0
                } else {
                    result.replies
                },
                sent: result.sent,
            });
        }
        self.fixed = true;
        if self.answered {
            Ok(CheckOutcome::Finished(self.finalize(now_secs)))
        } else {
            Ok(CheckOutcome::NeedsAnswer)
        }
    }

    /// Termine sans donner de diagnostic (0 point de diagnostic).
    pub fn finish(&mut self, now_secs: u64) -> Result<Score, RunError> {
        if self.is_finished() {
            return Err(RunError::Finished);
        }
        if !self.fixed {
            return Err(RunError::NotFixed);
        }
        Ok(self.finalize(now_secs))
    }

    /// Abandonne : la solution est révélée, aucun point n'est attribué.
    pub fn give_up(&mut self) -> Result<(), RunError> {
        if self.is_finished() {
            return Err(RunError::Finished);
        }
        self.gave_up = true;
        Ok(())
    }

    fn finalize(&mut self, now_secs: u64) -> Score {
        let score = compute_score(&ScoreInput {
            answered: self.answered,
            wrong_answers: self.wrong_answers,
            fixed: self.fixed,
            hints: self.hints_used,
            elapsed_secs: self.elapsed_secs(now_secs),
            limit_minutes: self.time_limit_minutes,
            xp_percent: self.challenge.xp_percent(),
        });
        self.finished = Some(score.clone());
        score
    }
}

/// Données d'un barème.
#[derive(Debug, Clone, Copy)]
pub struct ScoreInput {
    /// Le diagnostic a été donné et était juste.
    pub answered: bool,
    /// Mauvaises réponses données avant la bonne.
    pub wrong_answers: u32,
    /// L'objectif est atteint.
    pub fixed: bool,
    /// Indices utilisés.
    pub hints: u32,
    /// Temps écoulé.
    pub elapsed_secs: u64,
    /// Temps conseillé.
    pub limit_minutes: u32,
    /// Multiplicateur d'XP en centièmes (100 = ×1).
    pub xp_percent: u32,
}

/// Applique le barème du cahier des charges (voir le haut du module).
pub fn compute_score(i: &ScoreInput) -> Score {
    let diagnosis = if i.answered {
        DIAGNOSIS_POINTS.saturating_sub(WRONG_ANSWER_PENALTY * i.wrong_answers)
    } else {
        0
    };
    let fix = if i.fixed { FIX_POINTS } else { 0 };
    let time = if i.fixed {
        time_points(i.elapsed_secs, i.limit_minutes)
    } else {
        0
    };
    let hint_penalty = HINT_PENALTY * i.hints;
    let total = (diagnosis + fix + time)
        .saturating_sub(hint_penalty)
        .min(100);
    Score {
        diagnosis,
        fix,
        time,
        hint_penalty,
        total,
        xp: total * i.xp_percent / 100,
        elapsed_secs: i.elapsed_secs,
        hints: i.hints,
    }
}

/// Comprend un nom de panne saisi par l'utilisateur : nom court
/// (`wrong-gateway`) ou libellé français (« mauvaise passerelle »).
pub fn parse_cause(text: &str) -> Option<CauseKind> {
    let t = text.trim().to_lowercase().replace(['_', ' '], "-");
    CauseKind::from_slug(&t).or_else(|| {
        CauseKind::ALL
            .iter()
            .copied()
            .find(|k| k.label().to_lowercase().replace(' ', "-") == t)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn challenge(id: &str) -> Challenge {
        crate::builtin::challenge(id).unwrap().clone()
    }

    fn start(id: &str) -> (ChallengeRun, Simulation) {
        ChallengeRun::start(challenge(id), 1_000).unwrap()
    }

    /// Répare le réseau en annulant les pannes cachées.
    fn fix(sim: &mut Simulation) {
        sim.repair_all_faults(false);
    }

    #[test]
    fn time_points_curve() {
        // Limite 20 min = 1200 s ; plein jusqu'à 300 s (premier quart).
        assert_eq!(time_points(0, 20), 20);
        assert_eq!(time_points(300, 20), 20);
        assert_eq!(
            time_points(750, 20),
            10,
            "à mi-chemin entre 300 s et 1200 s"
        );
        assert_eq!(time_points(1_199, 20), 0);
        assert_eq!(time_points(1_200, 20), 0);
        assert_eq!(time_points(99_999, 20), 0);
        assert_eq!(time_points(10, 0), 0);
        // Décroissance monotone.
        let curve: Vec<u32> = (0..=1_200)
            .step_by(30)
            .map(|t| time_points(t, 20))
            .collect();
        assert!(curve.windows(2).all(|w| w[0] >= w[1]));
    }

    #[test]
    fn perfect_run_scores_a_hundred() {
        let (mut run, mut sim) = start("beginner-01");
        assert_eq!(
            run.answer("PC1", "wrong-gateway", 1_010).unwrap(),
            AnswerOutcome::Correct(None)
        );
        fix(&mut sim);
        let CheckOutcome::Finished(s) = run.check(&sim, 1_060).unwrap() else {
            panic!("le challenge devrait être terminé");
        };
        assert_eq!(
            (s.diagnosis, s.fix, s.time, s.hint_penalty, s.total),
            (40, 40, 20, 0, 100)
        );
        assert_eq!(s.xp, 100, "facile : ×1");
        assert_eq!(s.elapsed_secs, 60);
        assert!(run.is_finished());
    }

    #[test]
    fn the_order_of_answer_and_fix_does_not_matter() {
        let (mut run, mut sim) = start("beginner-01");
        fix(&mut sim);
        assert_eq!(run.check(&sim, 1_010).unwrap(), CheckOutcome::NeedsAnswer);
        assert!(run.is_fixed() && !run.is_finished());
        let AnswerOutcome::Correct(Some(s)) = run.answer("pc1", "WRONG-GATEWAY", 1_020).unwrap()
        else {
            panic!();
        };
        assert_eq!(s.total, 100);
    }

    #[test]
    fn hints_cost_ten_points_each_and_never_go_below_zero() {
        let (mut run, mut sim) = start("beginner-01");
        let texts: Vec<String> = (0..2).map(|_| run.next_hint().unwrap()).collect();
        assert!(texts[0].starts_with("Indice 1") && texts[1].starts_with("Indice 2"));
        run.answer("PC1", "wrong-gateway", 1_010).unwrap();
        fix(&mut sim);
        let CheckOutcome::Finished(s) = run.check(&sim, 1_020).unwrap() else {
            panic!()
        };
        assert_eq!((s.hints, s.hint_penalty, s.total), (2, 20, 80));

        // Beaucoup d'indices : le total reste borné à 0.
        let (mut run, mut sim) = start("beginner-01");
        for _ in 0..12 {
            run.next_hint().unwrap();
        }
        fix(&mut sim);
        run.check(&sim, 1_010).unwrap();
        let s = run.finish(1_010).unwrap();
        assert_eq!(s.total, 0);
        assert_eq!(s.xp, 0);
    }

    #[test]
    fn the_solution_hint_is_repeated_but_still_costs_points() {
        let (mut run, _) = start("beginner-01");
        let all: Vec<String> = (0..6).map(|_| run.next_hint().unwrap()).collect();
        assert!(all[3].starts_with("Solution"));
        assert_eq!(all[4], all[3]);
        assert_eq!(run.hints_used(), 6);
        assert_eq!(run.hints_left(), 0);
    }

    #[test]
    fn wrong_answers_reduce_the_diagnosis_points_and_give_partial_feedback() {
        let (mut run, mut sim) = start("beginner-01");
        assert_eq!(
            run.answer("PC1", "wrong-mask", 1_005).unwrap(),
            AnswerOutcome::Wrong {
                device_ok: true,
                cause_ok: false
            }
        );
        assert_eq!(
            run.answer("R1", "wrong-gateway", 1_006).unwrap(),
            AnswerOutcome::Wrong {
                device_ok: false,
                cause_ok: true
            }
        );
        assert_eq!(
            run.answer("R1", "link-down", 1_007).unwrap(),
            AnswerOutcome::Wrong {
                device_ok: false,
                cause_ok: false
            }
        );
        run.answer("PC1", "wrong-gateway", 1_008).unwrap();
        fix(&mut sim);
        let CheckOutcome::Finished(s) = run.check(&sim, 1_009).unwrap() else {
            panic!()
        };
        assert_eq!(s.diagnosis, 10, "40 − 3 × 10");
    }

    #[test]
    fn finishing_without_a_diagnosis_forfeits_its_points() {
        let (mut run, mut sim) = start("beginner-01");
        assert_eq!(run.finish(1_010), Err(RunError::NotFixed));
        fix(&mut sim);
        run.check(&sim, 1_010).unwrap();
        let s = run.finish(1_011).unwrap();
        assert_eq!((s.diagnosis, s.fix), (0, 40));
        assert_eq!(s.total, 60);
    }

    #[test]
    fn check_reports_progress_without_finishing() {
        let (mut run, sim) = start("beginner-01");
        let CheckOutcome::NotYet { replies, sent } = run.check(&sim, 1_010).unwrap() else {
            panic!()
        };
        assert_eq!((replies, sent), (0, 4));
        assert!(!run.is_fixed());
    }

    #[test]
    fn a_slow_solution_earns_less_time_points() {
        let (mut run, mut sim) = start("beginner-01"); // 15 min = 900 s
        run.answer("PC1", "wrong-gateway", 1_010).unwrap();
        fix(&mut sim);
        let CheckOutcome::Finished(s) = run.check(&sim, 1_000 + 900).unwrap() else {
            panic!()
        };
        assert_eq!(s.time, 0);
        assert_eq!(s.total, 80);
    }

    #[test]
    fn xp_scales_with_difficulty() {
        let (mut run, mut sim) = start("advanced-01"); // difficile : ×2
        run.answer("R3", "missing-route", 1_001).unwrap();
        fix(&mut sim);
        let CheckOutcome::Finished(s) = run.check(&sim, 1_002).unwrap() else {
            panic!()
        };
        assert_eq!(s.total, 100);
        assert_eq!(s.xp, 200);
    }

    #[test]
    fn giving_up_ends_the_run_without_a_score() {
        let (mut run, _) = start("beginner-01");
        run.give_up().unwrap();
        assert!(run.is_finished());
        assert!(run.score().is_none());
        assert_eq!(run.reveal().kind, CauseKind::WrongGateway);
        assert_eq!(run.next_hint(), Err(RunError::Finished));
        assert_eq!(run.give_up(), Err(RunError::Finished));
    }

    #[test]
    fn exam_mode_forbids_hints_and_overrides_the_time_limit() {
        let (run, _) = start("beginner-01");
        let mut run = run.without_hints().with_time_limit(5);
        assert_eq!(run.next_hint(), Err(RunError::HintsDisabled));
        assert!(!run.hints_allowed());
        assert_eq!(run.remaining_secs(1_000), 300);
        assert_eq!(run.remaining_secs(1_400), -100);
    }

    #[test]
    fn nothing_is_possible_after_the_end() {
        let (mut run, mut sim) = start("beginner-01");
        run.answer("PC1", "wrong-gateway", 1_001).unwrap();
        fix(&mut sim);
        run.check(&sim, 1_002).unwrap();
        assert_eq!(run.check(&sim, 1_003), Err(RunError::Finished));
        assert_eq!(run.answer("PC1", "x", 1_003), Err(RunError::Finished));
        assert_eq!(run.finish(1_003), Err(RunError::Finished));
        assert_eq!(run.next_hint(), Err(RunError::Finished));
    }

    #[test]
    fn causes_are_understood_by_slug_or_label() {
        assert_eq!(parse_cause("wrong-gateway"), Some(CauseKind::WrongGateway));
        assert_eq!(parse_cause("Wrong_Gateway"), Some(CauseKind::WrongGateway));
        assert_eq!(
            parse_cause("mauvaise passerelle par défaut"),
            Some(CauseKind::WrongGateway)
        );
        assert_eq!(parse_cause("câble coupé"), Some(CauseKind::LinkDown));
        assert_eq!(parse_cause("???"), None);
    }

    #[test]
    fn repeating_a_correct_answer_is_harmless() {
        let (mut run, _) = start("beginner-01");
        run.answer("PC1", "wrong-gateway", 1_001).unwrap();
        assert_eq!(
            run.answer("PC1", "wrong-gateway", 1_002).unwrap(),
            AnswerOutcome::Correct(None)
        );
        assert!(run.is_answered());
    }
}

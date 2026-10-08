#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Tout le contenu livré doit être valide et cohérent.

use std::collections::BTreeSet;

use netiv_diagnostics::CauseKind;
use netiv_education::Challenge;
use netiv_education::builtin::{self, TOPOLOGY_NAMES};

#[test]
fn every_builtin_topology_loads_without_warnings() {
    for name in TOPOLOGY_NAMES {
        let loaded = netiv_topology::load_yaml(builtin::topology(name).unwrap())
            .unwrap_or_else(|e| panic!("{name} : {e}"));
        assert!(loaded.warnings.is_empty(), "{name} : {:?}", loaded.warnings);
    }
    assert!(builtin::topology("nope").is_none());
}

#[test]
fn every_challenge_file_parses() {
    for (path, text) in builtin::challenge_sources() {
        Challenge::from_yaml(text).unwrap_or_else(|e| panic!("{path} : {e}"));
    }
    assert_eq!(
        builtin::challenges().len(),
        builtin::challenge_sources().len()
    );
}

#[test]
fn every_challenge_is_consistent_with_the_engine() {
    for c in builtin::challenges() {
        let problems = c.problems();
        assert!(problems.is_empty(), "{} : {problems:?}", c.id);
    }
}

#[test]
fn identifiers_are_unique_and_match_the_directory_difficulty() {
    let ids: BTreeSet<&str> = builtin::challenges()
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    assert_eq!(ids.len(), builtin::challenges().len());
    for ((path, _), c) in builtin::challenge_sources()
        .iter()
        .zip(builtin::challenges())
    {
        let dir = path.split('/').next().unwrap();
        assert_eq!(
            netiv_chaos::Difficulty::from_slug(dir),
            Some(c.difficulty),
            "{} : classé dans « {dir} »",
            c.id
        );
    }
}

#[test]
fn the_catalog_covers_every_topic_and_many_causes() {
    let topics: BTreeSet<&str> = builtin::challenges()
        .iter()
        .map(|c| c.topic.as_str())
        .collect();
    for t in [
        "physical",
        "addressing",
        "switching",
        "routing",
        "services",
        "security",
    ] {
        assert!(topics.contains(t), "aucun challenge sur « {t} »");
    }
    let causes: BTreeSet<CauseKind> = builtin::challenges()
        .iter()
        .map(|c| CauseKind::from_slug(&c.solution.cause).unwrap())
        .collect();
    assert!(causes.len() >= 12, "{causes:?}");
}

#[test]
fn statements_never_leak_the_answer() {
    for c in builtin::challenges() {
        let text = c.description.to_lowercase();
        // L'énoncé décrit un symptôme : il ne nomme ni la commande de réparation
        // ni le slug de la cause.
        assert!(
            !text.contains(&c.solution.cause),
            "{} : l'énoncé contient la cause",
            c.id
        );
        for cmd in &c.solution.commands {
            assert!(
                !c.description.contains(cmd.as_str()),
                "{} : l'énoncé contient la réponse",
                c.id
            );
        }
    }
}

#![allow(clippy::unwrap_used, clippy::expect_used)]

//! La documentation suit le code : chaque commande listée par `help` doit
//! figurer dans `docs/reference-commandes.md`.

use netiv_cli::Session;

const REFERENCE: &str = include_str!("../../../docs/reference-commandes.md");

/// Premier mot des lignes de commande de `help` (indentées de 4 espaces).
fn help_commands() -> Vec<String> {
    let mut out = Vec::new();
    Session::new().exec("help", &mut out).unwrap();
    let help = String::from_utf8(out).unwrap();
    let mut words: Vec<String> = help
        .lines()
        .filter_map(|l| l.strip_prefix("    "))
        .filter(|l| l.starts_with(|c: char| c.is_ascii_lowercase()))
        .filter_map(|l| l.split_whitespace().next())
        .map(str::to_string)
        .collect();
    words.sort();
    words.dedup();
    words
}

#[test]
fn every_help_command_is_documented() {
    let commands = help_commands();
    assert!(commands.len() >= 40, "aide incomplète ? {commands:?}");
    let missing: Vec<&String> = commands
        .iter()
        .filter(|c| !REFERENCE.contains(&format!("`{c}")))
        .collect();
    assert!(
        missing.is_empty(),
        "commandes absentes de docs/reference-commandes.md : {missing:?}"
    );
}

#[test]
fn every_cli_subcommand_is_documented() {
    // Sous-commandes du binaire `netiv` (crates/netiv-bin).
    for sub in ["run", "validate", "generate", "docs", "serve", "bench"] {
        assert!(
            REFERENCE.contains(&format!("`netiv {sub}")),
            "netiv {sub} non documenté"
        );
    }
}

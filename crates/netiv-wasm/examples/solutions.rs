//! Écrit les solutions des épreuves (JSON) sur la sortie standard :
//! `cargo run -p netiv-wasm --example solutions > solutions.json`.
//!
//! Réservé au serveur d'une plateforme qui intègre NETIV (tuteur, explication
//! après l'épreuve) : ce fichier ne doit jamais être servi au navigateur.

fn main() -> Result<(), serde_json::Error> {
    println!(
        "{}",
        serde_json::to_string_pretty(&netiv_service::solutions())?
    );
    Ok(())
}

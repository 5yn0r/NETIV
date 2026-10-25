//! Écrit le catalogue des épreuves (JSON) sur la sortie standard :
//! `cargo run -p netiv-wasm --example catalog > catalog.json`.
//!
//! Une plateforme qui intègre NETIV s'en sert pour ses pages de catalogue,
//! côté serveur, sans charger le module WebAssembly.

fn main() -> Result<(), serde_json::Error> {
    println!(
        "{}",
        serde_json::to_string_pretty(&netiv_service::catalog())?
    );
    Ok(())
}

# Contribuer

## Outils

- Rust stable (version fixée par [`rust-toolchain.toml`](../rust-toolchain.toml), édition 2024).
- Aucun outil JavaScript n'est nécessaire. `node --check crates/netiv-api/web/app.js` suffit
  pour vérifier la syntaxe du script de l'interface.

## Avant chaque commit

Les mêmes vérifications qu'en intégration continue ([`.github/workflows/ci.yml`](../.github/workflows/ci.yml)) :

```bash
cargo fmt --all --check
RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

## Tests

| Type | Où | Exemple |
|---|---|---|
| Scénarios du moteur | `crates/netiv-core/tests/` | Un ping traverse deux routeurs ; la trace contient les trois résolutions ARP. |
| Bout en bout CLI | `crates/netiv-cli/tests/` | Une session scriptée, avec sortie de référence (golden). |
| Contenu | `crates/netiv-education/tests/` | Chaque défi et chaque cyber range livrés sont résolus par leurs commandes de solution, pour 100 points. |
| Catalogue de pannes | `crates/netiv-chaos/tests/` | Chaque panne est retrouvée par `diagnose`, puis réparée. |
| Générateurs | `crates/netiv-topology/tests/`, `netiv-education` | Tout réseau généré est joignable ; tout défi généré est auto-validé. |
| API | `crates/netiv-api/tests/api.rs` | Requêtes en mémoire (`tower::ServiceExt::oneshot`), sans ouvrir de port. |
| WebAssembly | `crates/netiv-wasm/tests/engine.rs` (natif), `crates/netiv-wasm/tests/wasm.mjs` (vrai module, Node) | Le scénario golden donne la même sortie qu'en CLI ; défi, éditeur, erreurs `{ status, error }`, profil. |
| Service | `crates/netiv-service/tests/`, `crates/netiv-api/tests/parity.rs` | Opérations sans HTTP, stockage en mémoire ; le routeur `Service::request` répond comme l'API HTTP (mêmes codes, mêmes JSON). |
| Parallélisme | `crates/netiv-analysis/tests/analysis.rs` | Résultats identiques avec 1, 2, 5 et 16 threads. |
| Documentation | `crates/netiv-cli/tests/docs.rs` | Chaque commande de `help` figure dans [reference-commandes.md](reference-commandes.md). |

La sortie de référence de la session CLI se régénère avec :

```bash
UPDATE_GOLDEN=1 cargo test -p netiv-cli --test session
```

Il faut relire le diff de `tests/golden/mvp_session.txt` avant de le valider : un changement non
voulu de la trace est une régression.

## Conventions

- **Pas d'`unwrap`/`expect` hors des tests** (lint `clippy::unwrap_used`). Les erreurs
  d'utilisation remontent en `Result` avec un message en français qui dit quoi faire.
- **Les événements réseau ne sont pas des erreurs** : un paquet perdu, une route absente, un
  refus du pare-feu sont des événements de **trace**, jamais des `Err`.
- **Déterminisme** : pas d'horloge réelle, pas de `HashMap` dont on parcourt l'ordre pour produire
  une sortie (utiliser `BTreeMap`), le hasard passe uniquement par `SimRng`. Tout nouveau module
  qui produit une sortie a un test « même graine → même résultat ».
- **Les analyses travaillent sur des copies** (`fresh_clone`) et ne modifient jamais la
  simulation de l'utilisateur.
- **Documentation** : tout élément public est documenté (lint `missing_docs`). Les messages et
  commentaires sont en français, sans jargon inutile.
- **Interface web** : tout texte venant du serveur s'insère avec `textContent` (jamais
  `innerHTML`) ; aucune ressource externe (la CSP l'interdit de toute façon).
- **Un commit par sujet**, message en français à l'impératif ou au nominatif (« P1 : performances du moteur… »).

## Mesurer une optimisation

```bash
cargo build --release && ./target/release/netiv bench
```

Comparer avant et après sur la même machine, et reporter les chiffres dans
[performance.md](performance.md) si le changement est significatif.

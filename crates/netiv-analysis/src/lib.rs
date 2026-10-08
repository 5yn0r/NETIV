//! Analyse d'un réseau : qui joint qui, que se passe-t-il si… , à quel point
//! va-t-il bien, et à quoi ressemble sa documentation.
//!
//! Comme le diagnostic, tout se mesure sur des **copies** de la simulation.

pub mod docs;
pub mod health;
pub mod reach;
pub mod whatif;

pub use docs::{DocFormat, NetworkDoc, generate_docs};
pub use health::{Category, HealthReport, health};
pub use reach::{FlowId, Matrix};
pub use whatif::{Change, Impact, what_if};

//! Diagnostic réseau : sondes, analyse de la trace, cause racine, indices.
//!
//! Tout se fait sur des **copies** de la simulation (`fresh_clone`) : poser une
//! question au réseau ne le modifie jamais.

pub mod cause;
pub mod diagnose;
pub mod hints;
pub mod l2;
pub mod parallel;
pub mod probe;

pub use cause::{CauseKind, Layer, RootCause};
pub use diagnose::{Check, CheckStatus, Diagnosis, Request, Status, diagnose};
pub use hints::hints_for;
pub use probe::{Probe, ProbeKind, ProbeResult, run_probe};

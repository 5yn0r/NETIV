//! Sécurité : analyse d'une topologie (défauts de conception) et forensique
//! d'une capture (que s'est-il passé ?).
//!
//! Tout est **virtuel** : l'analyse s'appuie sur des sondes lancées dans des
//! copies de la simulation, jamais sur le réseau réel.

pub mod analyzer;
pub mod forensics;

pub use analyzer::{Finding, Report, Severity, analyze};
pub use forensics::{Analysis, Capture, ForensicFinding, analyze_capture};

//! Temps simulé. Il ne progresse que lorsque le moteur traite un événement.

use std::fmt;
use std::ops::{Add, AddAssign, Sub};

use serde::{Deserialize, Serialize};

/// Microsecondes écoulées depuis le début de la simulation.
#[derive(
    Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default, Serialize, Deserialize,
)]
pub struct SimTime(pub u64);

impl SimTime {
    /// Début de la simulation.
    pub const ZERO: SimTime = SimTime(0);

    /// Construit un temps à partir de microsecondes.
    pub const fn from_micros(us: u64) -> Self {
        SimTime(us)
    }

    /// Construit un temps à partir de millisecondes.
    pub const fn from_millis(ms: u64) -> Self {
        SimTime(ms * 1_000)
    }

    /// Valeur en microsecondes.
    pub const fn as_micros(self) -> u64 {
        self.0
    }

    /// Addition sans dépassement ; `None` en cas de débordement.
    pub fn checked_add(self, rhs: SimTime) -> Option<SimTime> {
        self.0.checked_add(rhs.0).map(SimTime)
    }

    /// Addition plafonnée à la date maximale (jamais de débordement).
    pub fn saturating_add(self, rhs: SimTime) -> SimTime {
        SimTime(self.0.saturating_add(rhs.0))
    }

    /// Durée écoulée depuis `earlier` (0 si `earlier` est postérieur).
    pub fn saturating_since(self, earlier: SimTime) -> SimTime {
        SimTime(self.0.saturating_sub(earlier.0))
    }
}

impl Add for SimTime {
    type Output = SimTime;
    fn add(self, rhs: SimTime) -> SimTime {
        SimTime(self.0 + rhs.0)
    }
}

impl AddAssign for SimTime {
    fn add_assign(&mut self, rhs: SimTime) {
        self.0 += rhs.0;
    }
}

impl Sub for SimTime {
    type Output = SimTime;
    fn sub(self, rhs: SimTime) -> SimTime {
        SimTime(self.0 - rhs.0)
    }
}

impl fmt::Display for SimTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 < 1_000 {
            f.pad(&format!("{} µs", self.0))
        } else {
            f.pad(&format!("{}.{:03} ms", self.0 / 1_000, self.0 % 1_000))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arithmetic() {
        let t = SimTime::from_millis(2) + SimTime::from_micros(500);
        assert_eq!(t.as_micros(), 2_500);
        assert_eq!(t - SimTime::from_millis(1), SimTime(1_500));
    }

    #[test]
    fn saturating_since_never_underflows() {
        assert_eq!(SimTime(5).saturating_since(SimTime(9)), SimTime::ZERO);
        assert_eq!(SimTime(9).saturating_since(SimTime(5)), SimTime(4));
    }

    #[test]
    fn checked_add_detects_overflow() {
        assert_eq!(SimTime(u64::MAX).checked_add(SimTime(1)), None);
    }

    #[test]
    fn display() {
        assert_eq!(SimTime(250).to_string(), "250 µs");
        assert_eq!(SimTime(1_500).to_string(), "1.500 ms");
    }
}

//! Générateur pseudo-aléatoire déterministe (SplitMix64).
//!
//! Écrit ici plutôt que tiré d'une dépendance : la séquence produite pour une
//! graine donnée est ainsi figée pour toujours, ce qui garantit que les
//! traces de référence (« golden traces ») ne changent pas lors d'une mise à
//! jour de dépendance.

/// Générateur SplitMix64.
#[derive(Clone, Debug)]
pub struct SimRng {
    state: u64,
}

impl SimRng {
    /// Crée un générateur à partir d'une graine.
    pub fn new(seed: u64) -> Self {
        SimRng { state: seed }
    }

    /// Prochain entier 64 bits.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Flottant uniforme dans `[0, 1)` (53 bits de précision).
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Vrai avec la probabilité `p` (bornée à `[0, 1]`).
    ///
    /// Consomme toujours un tirage, même pour `p = 0` ou `p = 1`, afin que
    /// la séquence ne dépende pas des valeurs de probabilité.
    pub fn chance(&mut self, p: f64) -> bool {
        let x = self.next_f64();
        x < p.clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_reference_splitmix64_vector() {
        // Vecteur de référence publié pour la graine 0.
        let mut r = SimRng::new(0);
        assert_eq!(r.next_u64(), 0xE220_A839_7B1D_CDAF);
        assert_eq!(r.next_u64(), 0x6E78_9E6A_A1B9_65F4);
    }

    #[test]
    fn same_seed_same_sequence() {
        let mut a = SimRng::new(42);
        let mut b = SimRng::new(42);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seed_different_sequence() {
        assert_ne!(SimRng::new(1).next_u64(), SimRng::new(2).next_u64());
    }

    #[test]
    fn f64_in_unit_interval() {
        let mut r = SimRng::new(7);
        for _ in 0..10_000 {
            let x = r.next_f64();
            assert!((0.0..1.0).contains(&x));
        }
    }

    #[test]
    fn chance_extremes() {
        let mut r = SimRng::new(3);
        assert!((0..1_000).all(|_| !r.chance(0.0)));
        assert!((0..1_000).all(|_| r.chance(1.0)));
    }

    #[test]
    fn chance_is_roughly_calibrated() {
        let mut r = SimRng::new(99);
        let hits = (0..100_000).filter(|_| r.chance(0.25)).count();
        assert!((24_000..26_000).contains(&hits), "hits = {hits}");
    }

    #[test]
    fn chance_always_consumes_one_draw() {
        let mut a = SimRng::new(5);
        let mut b = SimRng::new(5);
        a.chance(0.0);
        b.next_u64();
        assert_eq!(a.next_u64(), b.next_u64());
    }
}

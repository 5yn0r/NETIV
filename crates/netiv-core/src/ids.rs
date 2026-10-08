//! Identifiants typés. Des newtypes évitent de mélanger, par exemple,
//! un `DeviceId` et un `LinkId`.

use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! id_type {
    ($(#[$meta:meta])* $name:ident($inner:ty), $prefix:literal) => {
        $(#[$meta])*
        #[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
        pub struct $name(pub $inner);

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!($prefix, "{}"), self.0)
            }
        }
    };
}

id_type!(
    /// Identifiant d'un équipement.
    DeviceId(u32), "dev#"
);
id_type!(
    /// Identifiant d'une interface, local à un équipement.
    IfaceId(u32), "if#"
);
id_type!(
    /// Identifiant d'un lien.
    LinkId(u32), "link#"
);
id_type!(
    /// Identifiant d'un timer d'équipement.
    TimerId(u64), "timer#"
);
id_type!(
    /// Identité logique d'un « voyage » de paquet, conservée de saut en saut.
    TraceId(u64), "#"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_is_prefixed() {
        assert_eq!(DeviceId(3).to_string(), "dev#3");
        assert_eq!(TraceId(42).to_string(), "#42");
    }

    #[test]
    fn ids_are_ordered() {
        assert!(DeviceId(1) < DeviceId(2));
    }
}

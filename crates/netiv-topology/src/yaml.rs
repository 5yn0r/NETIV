//! Lecture et écriture YAML de NETIV.
//!
//! Seul point d'accès au moteur YAML (serde-saphyr, écrit en Rust sûr et
//! protégé par défaut contre les documents piégés : alias en cascade,
//! imbrication excessive). Les autres crates passent par ces fonctions.
//!
//! Les booléens sont stricts (`true`/`false` seulement, comme YAML 1.2) : un
//! nom d'équipement ou de domaine « no », « on » ou « y » reste du texte.

use std::fmt;

use serde::de::{self, DeserializeOwned, MapAccess, SeqAccess, Visitor};
use serde::ser::{SerializeMap, SerializeSeq};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Erreur de lecture (avec la position dans le texte).
pub type Error = serde_saphyr::Error;
/// Erreur d'écriture.
pub type SerializeError = serde_saphyr::SerializeError;

fn options() -> serde_saphyr::Options {
    serde_saphyr::options! { strict_booleans: true }
}

/// Lit un document YAML.
pub fn from_str<T: DeserializeOwned>(text: &str) -> Result<T, Error> {
    serde_saphyr::from_str_with_options(text, options())
}

/// Écrit un document YAML.
pub fn to_string<T: Serialize>(value: &T) -> Result<String, SerializeError> {
    serde_saphyr::to_string(value)
}

/// Valeur YAML quelconque, conservée telle quelle (ordre des clés compris) :
/// le scénario et le challenge d'un fichier `.netiv`, dont le format
/// appartient à d'autres crates.
#[derive(Debug, Clone, PartialEq)]
pub enum YamlValue {
    /// `null` ou valeur absente.
    Null,
    /// Booléen.
    Bool(bool),
    /// Entier signé.
    Int(i64),
    /// Entier non signé (au-delà de `i64`).
    UInt(u64),
    /// Nombre à virgule.
    Float(f64),
    /// Texte.
    String(String),
    /// Liste.
    Seq(Vec<YamlValue>),
    /// Table, dans l'ordre du document.
    Map(Vec<(YamlValue, YamlValue)>),
}

impl YamlValue {
    /// Vrai si c'est une table qui contient la clé texte `key`.
    pub fn has_key(&self, key: &str) -> bool {
        matches!(self, YamlValue::Map(entries)
            if entries.iter().any(|(k, _)| matches!(k, YamlValue::String(s) if s == key)))
    }
}

/// Convertit une valeur typée en [`YamlValue`] (en passant par le texte YAML).
pub fn to_value<T: Serialize>(value: &T) -> Result<YamlValue, String> {
    let text = to_string(value).map_err(|e| e.to_string())?;
    from_str(&text).map_err(|e| e.to_string())
}

/// Convertit une [`YamlValue`] en valeur typée (en passant par le texte YAML).
pub fn from_value<T: DeserializeOwned>(value: &YamlValue) -> Result<T, String> {
    let text = to_string(value).map_err(|e| e.to_string())?;
    from_str(&text).map_err(|e| e.to_string())
}

impl Serialize for YamlValue {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            YamlValue::Null => s.serialize_unit(),
            YamlValue::Bool(b) => s.serialize_bool(*b),
            YamlValue::Int(n) => s.serialize_i64(*n),
            YamlValue::UInt(n) => s.serialize_u64(*n),
            YamlValue::Float(f) => s.serialize_f64(*f),
            YamlValue::String(t) => s.serialize_str(t),
            YamlValue::Seq(items) => {
                let mut seq = s.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            YamlValue::Map(entries) => {
                let mut map = s.serialize_map(Some(entries.len()))?;
                for (k, v) in entries {
                    map.serialize_entry(k, v)?;
                }
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for YamlValue {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(ValueVisitor)
    }
}

struct ValueVisitor;

impl<'de> Visitor<'de> for ValueVisitor {
    type Value = YamlValue;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("une valeur YAML")
    }

    fn visit_bool<E: de::Error>(self, v: bool) -> Result<YamlValue, E> {
        Ok(YamlValue::Bool(v))
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<YamlValue, E> {
        Ok(YamlValue::Int(v))
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<YamlValue, E> {
        Ok(i64::try_from(v).map_or(YamlValue::UInt(v), YamlValue::Int))
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<YamlValue, E> {
        Ok(YamlValue::Float(v))
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<YamlValue, E> {
        Ok(YamlValue::String(v.to_string()))
    }

    fn visit_string<E: de::Error>(self, v: String) -> Result<YamlValue, E> {
        Ok(YamlValue::String(v))
    }

    fn visit_unit<E: de::Error>(self) -> Result<YamlValue, E> {
        Ok(YamlValue::Null)
    }

    fn visit_none<E: de::Error>(self) -> Result<YamlValue, E> {
        Ok(YamlValue::Null)
    }

    fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<YamlValue, D::Error> {
        YamlValue::deserialize(d)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<YamlValue, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element()? {
            items.push(item);
        }
        Ok(YamlValue::Seq(items))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<YamlValue, A::Error> {
        let mut entries = Vec::new();
        while let Some((k, v)) = map.next_entry()? {
            entries.push((k, v));
        }
        Ok(YamlValue::Map(entries))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn booleans_are_strict_names_stay_text() {
        let v: YamlValue = from_str("name: no\nflag: true\nport: \"80\"\nn: 80\n").unwrap();
        assert_eq!(
            v,
            YamlValue::Map(vec![
                (
                    YamlValue::String("name".into()),
                    YamlValue::String("no".into())
                ),
                (YamlValue::String("flag".into()), YamlValue::Bool(true)),
                (
                    YamlValue::String("port".into()),
                    YamlValue::String("80".into())
                ),
                (YamlValue::String("n".into()), YamlValue::Int(80)),
            ])
        );
    }

    #[test]
    fn values_round_trip_in_document_order() {
        let text = "zeta: 1\nalpha: [a, b]\n10: dix\n";
        let v: YamlValue = from_str(text).unwrap();
        assert!(v.has_key("zeta") && !v.has_key("10"));
        let back: YamlValue = from_str(&to_string(&v).unwrap()).unwrap();
        assert_eq!(back, v);
        let first = to_string(&v).unwrap();
        assert!(first.find("zeta") < first.find("alpha"), "{first}");
    }

    #[test]
    fn typed_values_convert_both_ways() {
        #[derive(Debug, PartialEq, Serialize, Deserialize)]
        struct S {
            name: String,
            port: u16,
        }
        let s = S {
            name: "on".into(),
            port: 443,
        };
        let v = to_value(&s).unwrap();
        assert_eq!(from_value::<S>(&v).unwrap(), s);
    }

    #[test]
    fn alias_bombs_are_refused() {
        let mut text = String::from("a0: &a0 [x, x, x, x, x, x, x, x, x, x]\n");
        for i in 1..12 {
            let p = i - 1;
            text.push_str(&format!(
                "a{i}: &a{i} [*a{p}, *a{p}, *a{p}, *a{p}, *a{p}, *a{p}, *a{p}, *a{p}, *a{p}, *a{p}]\n"
            ));
        }
        assert!(from_str::<YamlValue>(&text).is_err());
    }
}

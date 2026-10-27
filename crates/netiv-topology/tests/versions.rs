#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Versionnement des formats : les anciens fichiers restent lisibles, les
//! fichiers trop récents sont refusés clairement, et les exports de version 1
//! ne changent pas.

use netiv_topology::{
    Bundle, BundleMeta, TOPOLOGY_SCHEMA_VERSION, TopologyDef, export, load_any, load_yaml,
};

const MVP: &str = include_str!("../../../examples/mvp.yaml");

#[test]
fn a_topology_without_schema_version_is_version_1() {
    assert!(!MVP.contains("schema_version"));
    let def: TopologyDef = serde_yaml_ng::from_str(MVP).unwrap();
    assert_eq!(def.schema_version, 1);
    assert_eq!(TOPOLOGY_SCHEMA_VERSION, 1);
    load_yaml(MVP).unwrap();
    let explicit = format!("schema_version: 1\n{MVP}");
    load_yaml(&explicit).unwrap();
}

#[test]
fn a_newer_topology_is_refused_with_a_clear_message() {
    let future = format!("schema_version: 99\n{MVP}");
    let e = load_yaml(&future).unwrap_err().to_string();
    assert!(e.contains("format version 99 non supportée"), "{e}");
    assert!(load_any(&future).is_err());
}

#[test]
fn version_1_exports_are_unchanged() {
    let loaded = load_yaml(MVP).unwrap();
    let def = export(&loaded.sim, &loaded.name);
    let yaml = serde_yaml_ng::to_string(&def).unwrap();
    assert!(!yaml.contains("schema_version"), "{yaml}");
    let bundle = Bundle::new(BundleMeta::default(), def).to_yaml().unwrap();
    assert!(bundle.starts_with("format_version: 1"), "{bundle}");
    assert!(!bundle.contains("schema_version"), "{bundle}");
}

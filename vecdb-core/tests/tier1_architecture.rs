#[test]
fn test_no_vecq_dependency_in_core() {
    let cargo_toml_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");

    let content = std::fs::read_to_string(&cargo_toml_path).expect("Failed to read Cargo.toml");

    let cargo: toml::Value = toml::from_str(&content).expect("Failed to parse Cargo.toml");

    // check dependencies
    if let Some(deps) = cargo.get("dependencies").and_then(|d| d.as_table()) {
        if deps.contains_key("vecq") {
            panic!("ARCHITECTURE VIOLATION: vecdb-core MUST NOT depend on vecq!");
        }
    }

    // check workspace dependencies usage?
    // If it uses workspace = true, it might not show up here if I parse raw TOML,
    // but the key "vecq" would be present if defined.
}

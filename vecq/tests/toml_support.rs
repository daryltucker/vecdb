#[tokio::test]
async fn test_toml_data_querying() {
    // 1. Setup dummy TOML
    let content = "[package]\nversion = \"0.0.9\"";
    let file_type = vecq::types::FileType::Toml;

    // 2. Process (Data Mode)
    let options = vecq::FormatOptions::default();
    match vecq::process_file(content, file_type, ".package.version", "json", &options).await {
        Ok(res) => {
            println!("Result: {}", res);
            assert_eq!(res.trim(), "\"0.0.9\"");
        }
        Err(e) => panic!("Error: {}", e),
    }
}

#[tokio::test]
async fn test_toml_structural_parsing() {
    // 1. Setup dummy TOML
    let content = "[package]\nversion = \"0.0.9\"";
    let file_type = vecq::types::FileType::Toml;

    // 2. Parse explicitly (Structural Mode)
    let parsed = vecq::parse_file(content, file_type)
        .await
        .expect("Failed to parse");
    let json = vecq::convert_to_json(parsed).expect("Failed to convert");

    // 3. Verify structure (tables/entries)
    // We expect a 'tables' array containing the package table
    let tables = json
        .get("tables")
        .expect("Missing tables")
        .as_array()
        .expect("tables is not array");
    assert!(!tables.is_empty());

    // Find 'package' table
    let package = tables
        .iter()
        .find(|t| t["name"] == "package")
        .expect("Missing package table");

    // Check attributes inside
    let attributes = package.get("attributes").expect("Missing attributes");
    let value = attributes.get("value").expect("Missing value attribute");

    // Value should be object with version
    // Wait, convert_value for Table returns Value::Object(map)
    // So attributes.value should be that object
    assert_eq!(value["version"], "0.0.9");
}

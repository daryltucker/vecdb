//! YAML must be parsed as structured configuration, not as a wall of text.
//!
//! `.yaml` and `.yml` used to resolve to `FileType::Text`, so a `docker-compose.yaml`
//! or a CI workflow was stored as one undifferentiated blob: no keys, no nesting,
//! nothing to filter or route on. `vecdb-core` did contain a `YamlParser`, but it
//! was registered for `FileType::Toml` — a different format — behind a factory no
//! binary ever constructed, so it never ran on a YAML file in its life.
//!
//! These assertions are about *structure*, not merely "did it parse". A parser
//! that returns one element containing the whole document technically succeeds
//! and is exactly the state this replaces.

use vecq::parsers::create_parser;
use vecq::types::{DocumentElement, FileType};

async fn parse(content: &str) -> vecq::types::ParsedDocument {
    create_parser(FileType::Yaml)
        .expect("a YAML parser must be registered")
        .parse(content)
        .await
        .expect("valid YAML must parse")
}

fn walk<'a>(elements: &'a [DocumentElement], out: &mut Vec<&'a DocumentElement>) {
    for e in elements {
        out.push(e);
        walk(&e.children, out);
    }
}

fn all(doc: &vecq::types::ParsedDocument) -> Vec<&DocumentElement> {
    let mut v = Vec::new();
    walk(&doc.elements, &mut v);
    v
}

const COMPOSE: &str = r#"
version: "3.9"
services:
  qdrant:
    image: qdrant/qdrant:latest
    ports:
      - "6333:6333"
    environment:
      GRPC_PORT: 6334
"#;

#[tokio::test]
async fn yaml_is_detected_as_yaml_not_text() {
    // The bug was upstream of the parser: detection sent .yaml to Text, so no
    // YAML parser could ever have been reached.
    for path in ["compose.yaml", "workflow.yml", "/etc/thing/config.yaml"] {
        assert_eq!(
            vecdb_common::FileType::from_path(path),
            vecdb_common::FileType::Yaml,
            "{path} must resolve to Yaml; mapping it to Text is what stored these \
             files as one unstructured blob"
        );
    }
}

#[tokio::test]
async fn nested_keys_become_addressable_elements() {
    let doc = parse(COMPOSE).await;
    let elements = all(&doc);

    for key in ["version", "services", "qdrant", "image", "environment"] {
        assert!(
            elements.iter().any(|e| e.name.as_deref() == Some(key)),
            "no element named {key:?}. Found: {:?}\nA YAML document reduced to a \
             single blob is the regression this test exists for.",
            elements
                .iter()
                .filter_map(|e| e.name.as_deref())
                .collect::<Vec<_>>()
        );
    }
}

#[tokio::test]
async fn scalar_values_are_preserved_with_their_types() {
    let doc = parse("enabled: true\nreplicas: 3\nratio: 0.75\nname: vecdb\n").await;
    let elements = all(&doc);

    let value_of = |key: &str| {
        elements
            .iter()
            .find(|e| e.name.as_deref() == Some(key))
            .and_then(|e| e.attributes.get("value").cloned())
            .unwrap_or_else(|| panic!("no value attribute for {key}"))
    };

    // Types matter: a config value coerced to a string cannot be filtered on.
    assert_eq!(value_of("enabled"), serde_json::json!(true));
    assert_eq!(value_of("replicas"), serde_json::json!(3));
    assert_eq!(value_of("ratio"), serde_json::json!(0.75));
    assert_eq!(value_of("name"), serde_json::json!("vecdb"));
}

#[tokio::test]
async fn sequences_are_indexed_not_flattened() {
    let doc = parse("ports:\n  - 6333\n  - 6334\n  - 6335\n").await;
    let elements = all(&doc);

    for i in 0..3 {
        assert!(
            elements
                .iter()
                .any(|e| e.name.as_deref() == Some(&format!("ports[{i}]"))),
            "sequence entry ports[{i}] missing; list items must stay individually \
             addressable"
        );
    }
}

/// `---` separated documents are normal in Kubernetes manifests and CI config.
/// Reducing them to the first document loses most of the file silently.
#[tokio::test]
async fn multi_document_yaml_keeps_every_document() {
    let doc = parse("kind: First\n---\nkind: Second\n---\nkind: Third\n").await;
    let elements = all(&doc);

    let kinds: Vec<_> = elements
        .iter()
        .filter(|e| e.name.as_deref() == Some("kind"))
        .filter_map(|e| e.attributes.get("value"))
        .collect();

    assert_eq!(
        kinds.len(),
        3,
        "expected all three documents; got {kinds:?}. A multi-document YAML file \
         truncated to its first document loses the rest with no error."
    );
}

#[tokio::test]
async fn malformed_yaml_is_an_error_not_a_silent_blob() {
    let parser = create_parser(FileType::Yaml).expect("YAML parser");
    // Unclosed flow mapping.
    let result = parser.parse("services:\n  qdrant: {image: x\n").await;
    assert!(
        result.is_err(),
        "malformed YAML must error rather than degrade to unstructured text; \
         silent degradation is how the previous behaviour hid for so long"
    );
}

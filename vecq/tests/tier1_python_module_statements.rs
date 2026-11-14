//! Python `variable` (`variables`) is advertised and module-level
//! statements are no longer dropped.
//!
//! # What was wrong
//!
//! `elements py` did not list `variable` even though the Python parser has
//! always extracted assignments (`extract_variables`): with no
//! `Variable -> variable` schema mapping the elements landed in the generic
//! default group, invisible to the schema-driven `elements` command. Worse,
//! everything that is *not* a def/class/import/top-level-assignment was
//! silently dropped — a module's executable region (`if __name__ ==
//! "__main__":`, setup calls) simply did not exist as far as agents querying
//! with vecq were concerned.
//!
//! The `file_type` fixture `vecq/tests/fixtures/python/module_statements.py`
//! covers both halves:
//!
//! - `PAGE_SQL = "DELETE FROM ..."` — module-level assignment → Variable.
//! - `rebuild_table()` call + the `__main__` block — module statements → one
//!   synthetic `Module` element ("<module>") preserving source text.
//!
//! # Why a fixture, not inline source
//!
//! The reachability gate (`tier1_declaration_coverage.rs ::
//! every_element_mapping_is_reachable`) requires every schema mapping to be
//! produced by some fixture for that language; this file IS that proof, and
//! the detailed assertions below give it teeth.
//!
//! # Acceptance (BUG doc §6)
//!
//! - `.variables[]` populated, with `attributes.value` present (`.variable[]`
//!   until every list key plural).
//! - `.elements` (phantom key) gone from converted output.
//! - a `.module` element exists, contains the DELETE statement source,
//!   and its line range is honourable (end >= start).

use vecq::converter::UnifiedJsonConverter;
use vecq::types::ElementType;
use vecq::{FileType, JsonConverter, SchemaRegistry};

const FIXTURE: &str = "tests/fixtures/python/module_statements.py";

fn converted(fixture: &str) -> serde_json::Value {
    let content = std::fs::read_to_string(fixture).expect("read fixture");
    let parser = vecq::parsers::create_parser(FileType::Python).expect("python parser");
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let doc = rt.block_on(parser.parse(&content)).expect("parse");
    let converter = UnifiedJsonConverter::new(SchemaRegistry::new());
    converter.convert(doc).expect("convert")
}

#[test]
fn variable_mapping_is_advertised_by_schema() {
    let registry = SchemaRegistry::new();
    let schema = registry.get_schema(FileType::Python).expect("py schema");
    assert_eq!(
        schema.element_mappings.get(&ElementType::Variable),
        Some(&"variables".to_string()),
        "`elements py` must advertise Variable under `variables`"
    );
    // Each module-level statement is a `Block` (`blocks` since
    // with its `kind`); `module` is the module SCOPE object,
    // not an element mapping.
    assert_eq!(
        schema.element_mappings.get(&ElementType::Block),
        Some(&"blocks".to_string()),
        "module-level statements must be advertised under `blocks`"
    );
    assert_eq!(schema.element_mappings.get(&ElementType::Module), None);
    assert!(
        schema
            .attribute_definitions
            .get(&ElementType::Variable)
            .map(|attrs| attrs.contains(&"value".to_string()))
            .unwrap_or(false),
        "Variable must declare a `value` attribute (literal-preserving elision)"
    );
}

#[test]
fn phantom_elements_key_is_gone() {
    let json = converted(FIXTURE);
    let obj = json.as_object().expect("root object");
    assert!(
        !obj.contains_key("elements"),
        "converted output must not contain the phantom base key `elements`: {:?}",
        obj.keys().collect::<Vec<_>>()
    );
}

#[test]
fn variable_elements_present_with_value() {
    let json = converted(FIXTURE);
    let variables = json
        .get("variables")
        .and_then(|v| v.as_array())
        .expect("`variables` array present");
    assert!(
        !variables.is_empty(),
        "module-level assignments must be surfaced as Variable elements"
    );
    let page_sql = variables
        .iter()
        .find(|e| e.get("name").and_then(|n| n.as_str()) == Some("PAGE_SQL"))
        .expect("PAGE_SQL assignment missing");
    let value = page_sql
        .pointer("/attributes/value")
        .and_then(|v| v.as_str())
        .expect("PAGE_SQL must carry a `value` attribute");
    assert!(
        value.contains("DELETE FROM job_queue"),
        "value should keep its literal content, got: {:?}",
        value
    );
}

/// Each executable module statement is its own `blocks` element, as written,
/// with its grammar kind. Until 2026-314 they were ONE synthetic `<module>`
/// element spanning the first kept statement to the last — which, with a `def`
/// between, contained the `def` as well (two elements claiming its bytes;
/// `tier1_source_contract` check 7 now rejects that shape).
#[test]
fn module_statements_are_preserved() {
    let json = converted(FIXTURE);
    let blocks = json
        .get("blocks")
        .and_then(|v| v.as_array())
        .expect("`blocks` array present");
    let got: Vec<(&str, &str)> = blocks
        .iter()
        .map(|b| {
            (
                b.pointer("/attributes/kind")
                    .and_then(|k| k.as_str())
                    .unwrap(),
                b.get("content").and_then(|c| c.as_str()).unwrap(),
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            ("expression_statement", "rebuild_table()"),
            (
                "if_statement",
                "if __name__ == \"__main__\":\n    print(PAGE_SQL)"
            ),
        ],
        "one element per statement; the DELETE assignment is in `variables`"
    );
    for b in blocks {
        let start = b.get("line_start").and_then(|v| v.as_u64()).unwrap();
        let end = b.get("line_end").and_then(|v| v.as_u64()).unwrap();
        assert!(end >= start, "honourable line range: {start}..{end}");
    }
}

/// What those statements DO is the module scope: the
/// calls made outside every function, attributed to `<module>`.
#[test]
fn module_scope_holds_the_top_level_calls() {
    let json = converted(FIXTURE);
    let module = json.get("module").expect("`module` scope present");
    assert_eq!(
        module.get("name").and_then(|n| n.as_str()),
        Some("<module>")
    );
    let callees: Vec<&str> = module["calls"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["callee"].as_str().unwrap())
        .collect();
    assert_eq!(callees, ["rebuild_table", "print"]);
}

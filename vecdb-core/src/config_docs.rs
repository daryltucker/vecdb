//! Generate the configuration reference from the config structs themselves.
//!
//! `docs/CONFIG.md` used to be hand-written, with `tests/tier2_config_compliance.py`
//! checking after the fact that every field appeared *somewhere* in it. That
//! catches an undocumented field, and nothing else: a field could be documented
//! with the wrong type, the wrong default, or a description contradicting the
//! code, and the check would pass. It did — `respect_gitignore` was documented
//! as defaulting `false` while the code said `true`, and `truncate` was
//! documented as the opposite of what shipped.
//!
//! The tables are now derived from `schemars`, which reads the same `///`
//! comments a developer edits when changing behaviour. Drift is not detected;
//! it is impossible.
//!
//! Regenerate with `cargo run -p xtask -- gen-config-docs`. The suite runs the
//! same generator and fails if the checked-in file differs.

use schemars::JsonSchema;
use serde_json::Value;

/// One documented setting.
struct Field {
    name: String,
    ty: String,
    required: bool,
    description: String,
}

/// Render a struct's fields as a Markdown table.
fn table<T: JsonSchema>() -> String {
    let schema = serde_json::to_value(schemars::schema_for!(T)).unwrap_or(Value::Null);

    let required: Vec<String> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let mut fields: Vec<Field> = schema
        .get("properties")
        .and_then(Value::as_object)
        .map(|props| {
            props
                .iter()
                .map(|(name, prop)| Field {
                    name: name.clone(),
                    ty: type_name(prop),
                    required: required.contains(name),
                    description: describe(prop),
                })
                .collect()
        })
        .unwrap_or_default();

    // Stable order: required first, then alphabetical. A generated file that
    // reorders itself between runs produces noise diffs and stops being read.
    fields.sort_by(|a, b| b.required.cmp(&a.required).then(a.name.cmp(&b.name)));

    let mut out = String::from("| Key | Type | Required | Description |\n");
    out.push_str("|-----|------|----------|-------------|\n");
    for f in fields {
        out.push_str(&format!(
            "| `{}` | {} | {} | {} |\n",
            f.name,
            f.ty,
            if f.required { "**yes**" } else { "no" },
            f.description
        ));
    }
    out
}

/// A readable type name for the table.
fn type_name(prop: &Value) -> String {
    // `Option<T>` arrives as `"type": ["...", "null"]`; report the inner type.
    match prop.get("type") {
        Some(Value::String(t)) => return pretty_type(t),
        Some(Value::Array(ts)) => {
            if let Some(t) = ts.iter().filter_map(Value::as_str).find(|t| *t != "null") {
                return pretty_type(t);
            }
        }
        _ => {}
    }

    // A `$ref`, or an Option<enum> expressed as anyOf[{$ref}, {null}].
    if let Some(r) = prop.get("$ref").and_then(Value::as_str) {
        return format!("`{}`", r.rsplit('/').next().unwrap_or(r));
    }
    for key in ["anyOf", "oneOf", "allOf"] {
        if let Some(list) = prop.get(key).and_then(Value::as_array) {
            for entry in list {
                if let Some(r) = entry.get("$ref").and_then(Value::as_str) {
                    return format!("`{}`", r.rsplit('/').next().unwrap_or(r));
                }
                if let Some(Value::String(t)) = entry.get("type") {
                    if t != "null" {
                        return pretty_type(t);
                    }
                }
            }
        }
    }
    "—".to_string()
}

fn pretty_type(t: &str) -> String {
    match t {
        "object" => "table",
        other => other,
    }
    .to_string()
}

/// Flatten a doc comment into one Markdown table cell.
fn describe(prop: &Value) -> String {
    let raw = prop
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default();

    // Doc comments are written as prose across several lines; a table cell is
    // one line. Paragraph breaks become sentence breaks, and pipes would end
    // the cell early.
    raw.replace("\n\n", " — ")
        .replace('\n', " ")
        .replace('|', "\\|")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Fields the generator found with no doc comment.
///
/// A blank description cell is the generated equivalent of an undocumented
/// field: the table lists it, so the compliance question "is it documented?"
/// answers yes, while the reader learns nothing. Reported so the suite can
/// refuse it.
pub fn undocumented() -> Vec<String> {
    let mut missing = Vec::new();
    let mut check = |label: &str, rendered: &str| {
        for line in rendered.lines() {
            if line.starts_with("| `") && line.trim_end().ends_with("|  |") {
                let key = line.split('`').nth(1).unwrap_or("?");
                missing.push(format!("{label}.{key}"));
            }
        }
    };
    check("Config", &table::<crate::config::Config>());
    check("backend", &table::<crate::config::Backend>());
    check("embedder", &table::<crate::config::EmbedderSpec>());
    check("profiles", &table::<crate::config::Profile>());
    check("collections", &table::<crate::config::CollectionConfig>());
    check("ingestion", &table::<crate::config::IngestionConfig>());
    check("server", &table::<crate::config::ServerConfig>());
    missing
}

/// The full generated reference.
pub fn generate() -> String {
    let mut out = String::new();

    out.push_str(
        "<!-- BEGIN GENERATED CONFIG REFERENCE -->\n\
         <!-- Generated by `cargo run -p xtask -- gen-config-docs`. DO NOT EDIT BY HAND. -->\n\
         <!-- Source of truth: the doc comments in vecdb-core/src/config.rs -->\n\n",
    );

    out.push_str("### Top-Level Options\n\n");
    out.push_str(&table::<crate::config::Config>());

    out.push_str("\n#### Backend Options (`[backend.<name>]`)\n\n");
    out.push_str(&table::<crate::config::Backend>());

    out.push_str("\n#### Embedder Options (`[embedder.<name>]`)\n\n");
    out.push_str(&table::<crate::config::EmbedderSpec>());

    out.push_str("\n### Profile Options (`[profiles.<name>]`)\n\n");
    out.push_str(&table::<crate::config::Profile>());

    out.push_str("\n#### Collection Profile Options (`[collections.<name>]`)\n\n");
    out.push_str(&table::<crate::config::CollectionConfig>());

    out.push_str("\n### Ingestion Options (`[ingestion]`)\n\n");
    out.push_str(&table::<crate::config::IngestionConfig>());

    out.push_str("\n#### Server Options (`[server]`)\n\n");
    out.push_str(&table::<crate::config::ServerConfig>());

    out.push_str("\n<!-- END GENERATED CONFIG REFERENCE -->\n");
    out
}

/// Replace the generated block in an existing document, preserving hand-written
/// prose around it.
///
/// Prose and reference serve different purposes: the reference must track the
/// code exactly, while the surrounding explanation is written for a human and
/// has no field-by-field counterpart. Regenerating the whole file would delete
/// the latter every time.
pub fn splice(existing: &str) -> anyhow::Result<String> {
    const BEGIN: &str = "<!-- BEGIN GENERATED CONFIG REFERENCE -->";
    const END: &str = "<!-- END GENERATED CONFIG REFERENCE -->";

    let generated = generate();

    let (Some(start), Some(end)) = (existing.find(BEGIN), existing.find(END)) else {
        anyhow::bail!(
            "no generated block found in the document.\n\n  \
             Add these markers where the reference should go:\n    {BEGIN}\n    {END}"
        );
    };

    Ok(format!(
        "{}{}{}",
        &existing[..start],
        generated.trim_end_matches('\n'),
        &existing[end + END.len()..]
    ))
}

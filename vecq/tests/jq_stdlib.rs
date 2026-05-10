use assert_cmd::Command;
use serde_json::json;

/// --raw-json: treat .json files as plain data for native jq queries.
/// Default mode routes .json through the AST pipeline (.pairs[], .objects[]).
#[test]
fn test_jq_stdlib_keys() {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_vecq"));
    let input = json!({"a": 1, "b": 2}).to_string();
    let mut temp = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
    std::io::Write::write_all(&mut temp, input.as_bytes()).unwrap();

    let assert = cmd
        .arg("--raw-json")
        .arg("-q").arg("keys | sort")
        .arg(temp.path())
        .assert();

    assert.success().stdout("[\"a\",\"b\"]\n");
}

#[test]
fn test_jq_stdlib_length() {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_vecq"));
    let input = json!([1, 2, 3]).to_string();
    let mut temp = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
    std::io::Write::write_all(&mut temp, input.as_bytes()).unwrap();

    let assert = cmd
        .arg("--raw-json")
        .arg("-q").arg("length")
        .arg(temp.path())
        .assert();

    assert.success().stdout("3\n");
}

#[test]
fn test_jq_stdlib_to_entries() {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_vecq"));
    let input = json!({"a": 1}).to_string();
    let mut temp = tempfile::Builder::new().suffix(".json").tempfile().unwrap();
    std::io::Write::write_all(&mut temp, input.as_bytes()).unwrap();

    let assert = cmd
        .arg("--raw-json")
        .arg("-q")
        .arg("to_entries | .[0].key")
        .arg(temp.path())
        .assert();

    assert.success().stdout("\"a\"\n");
}

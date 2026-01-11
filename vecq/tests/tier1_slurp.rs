use assert_cmd::Command;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_slurp_classic_multiple_files() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = TempDir::new()?;
    let file1 = temp_dir.path().join("a.json");
    let file2 = temp_dir.path().join("b.json");
    
    fs::write(&file1, r#"{"name": "A"}"#)?;
    fs::write(&file2, r#"{"name": "B"}"#)?;

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_vecq"));
    cmd.arg("-s")
       .arg("-q")
       .arg(".")
       .arg(&file1)
       .arg(&file2);

    let output = cmd.output()?;
    let stdout = String::from_utf8(output.stdout)?;
    
    // Should be a list containing both objects
    let json: serde_json::Value = serde_json::from_str(&stdout)?;
    assert!(json.is_array());
    let arr = json.as_array().unwrap();
    if arr.len() != 2 {
        println!("Expected 2 items, got {}: {}", arr.len(), serde_json::to_string_pretty(&json).unwrap());
    }
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["name"], "A");
    assert_eq!(arr[1]["name"], "B");

    Ok(())
}

#[test]
fn test_slurp_ndjson_stream() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = TempDir::new()?;
    let file = temp_dir.path().join("stream.ndjson");
    
    fs::write(&file, r#"{"id": 1}
{"id": 2}
{"id": 3}"#)?;

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_vecq"));
    cmd.arg("-s")
       .arg("-q")
       .arg(".")
       .arg(&file);

    let output = cmd.output()?;
    let stdout = String::from_utf8(output.stdout)?;
    
    // Should be a list containing all 3 objects
    let json: serde_json::Value = serde_json::from_str(&stdout)?;
    assert!(json.is_array());
    let arr = json.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0]["id"], 1);
    assert_eq!(arr[2]["id"], 3);

    Ok(())
}

#[test]
fn test_slurp_mixed_inputs() -> Result<(), Box<dyn std::error::Error>> {
    let temp_dir = TempDir::new()?;
    let single = temp_dir.path().join("single.json");
    let stream = temp_dir.path().join("stream.ndjson");
    
    fs::write(&single, r#"{"type": "single"}"#)?;
    fs::write(&stream, r#"{"type": "stream_1"}
{"type": "stream_2"}"#)?;

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_vecq"));
    cmd.arg("-s")
       .arg("-q")
       .arg(".")
       .arg(&single)
       .arg(&stream);

    let output = cmd.output()?;
    let stdout = String::from_utf8(output.stdout)?;
    
    let json: serde_json::Value = serde_json::from_str(&stdout)?;
    assert!(json.is_array());
    let arr = json.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    
    let types: Vec<&str> = arr.iter()
        .map(|v| v["type"].as_str().unwrap())
        .collect();
        
    assert!(types.contains(&"single"));
    assert!(types.contains(&"stream_1"));
    assert!(types.contains(&"stream_2"));

    Ok(())
}

use vecq::{convert_to_json, parse_file, query_json, FileType};

#[tokio::test]
async fn test_basic_query_rust() {
    let content = r#"
    pub fn main() {
        println!("Hello");
    }
    
    fn private_helper() {}
    "#;
    let parsed = parse_file(content, FileType::Rust).await.unwrap();
    let json = convert_to_json(parsed).unwrap();
    println!(
        "Rust JSON: {}",
        serde_json::to_string_pretty(&json).unwrap()
    );

    // Test 1: Get all functions
    let results = query_json(&json, ".functions").unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].is_array());
    let funcs = results[0].as_array().unwrap();
    assert_eq!(funcs.len(), 2);

    // Test 2: Filter public functions
    let results = query_json(
        &json,
        ".functions[] | select(.attributes.visibility == \"pub\")",
    )
    .unwrap();
    println!("Filter Result: {:?}", results);
    assert_eq!(results.len(), 1);

    if let Some(obj) = results[0].as_object() {
        println!("Keys: {:?}", obj.keys());
    }
    // Should be a single object since there's only one match
    assert!(results[0].is_object());
    assert_eq!(
        results[0].get("name").and_then(|v| v.as_str()),
        Some("main")
    );

    // Test 3: Get names of all functions
    let results = query_json(&json, ".functions[] | .name").unwrap();
    println!("Names Result: {:?}", results);
    // results IS the array of outputs
    assert_eq!(results.len(), 2);

    let names: Vec<String> = results
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(names.contains(&"main".to_string()));
    assert!(names.contains(&"private_helper".to_string()));
}

#[tokio::test]
async fn test_python_classes() {
    let content = r#"
class Dog:
    def bark(self): pass

class Cat:
    def meow(self): pass
    "#;
    let parsed = parse_file(content, FileType::Python).await.unwrap();
    let json = convert_to_json(parsed).unwrap();
    println!(
        "Python JSON: {}",
        serde_json::to_string_pretty(&json).unwrap()
    );

    // Debug: Check literal query
    let results = query_json(&json, "1").unwrap();
    println!("Literal 1: {:?}", results);
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0],
        serde_json::Value::Number(serde_json::Number::from(1))
    );

    // Debug: Check identity
    let results = query_json(&json, ".").unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].is_object());

    // Query class names using .classes (plural) as seen in JSON output
    let results = query_json(&json, ".classes[] | .name").unwrap();
    println!("Python Names Result: {:?}", results);
    // Expecting ["Dog", "Cat"]
    assert_eq!(results.len(), 2);
    let names: Vec<&str> = results.iter().map(|v| v.as_str().unwrap()).collect();
    assert!(names.contains(&"Dog"));
    assert!(names.contains(&"Cat"));
}

#[tokio::test]
async fn test_empty_query() {
    let content = "fn foo() {}";
    let parsed = parse_file(content, FileType::Rust).await.unwrap();
    let json = convert_to_json(parsed).unwrap();

    let results = query_json(&json, ".").unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].is_object());
    assert!(results[0].get("functions").is_some());
}
